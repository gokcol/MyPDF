//! Signed documents: a signature bar, an Acrobat-style details panel, click to
//! inspect a signature on the page, editing after an explicit unlock, and
//! removing a signature. Signed bytes are never rewritten: every check, save and
//! removal works on the signed original plus an incremental update.
use super::*;
use mypdf_milestone0::{
    annotations,
    signature_edit::{self, SignatureField},
    signing::{self, SignatureEvidence},
};

pub(super) struct SignatureView {
    pub field: SignatureField,
    pub evidence: Option<SignatureEvidence>,
    /// The signature's place in signing order (1 is the first signature).
    pub revision: usize,
}
type Overview = Result<Vec<SignatureView>, String>;

/// Per-document signature state.
#[derive(Default)]
pub(super) struct TabSignatures {
    /// Session revision and trusted-certificate count the overview was built for.
    key: Option<(u64, usize)>,
    job: Option<mpsc::Receiver<Overview>>,
    pub views: Option<Overview>,
    pub selected: Option<String>,
    pub panel: bool,
    certificates: HashSet<String>,
}
impl TabSignatures {
    fn signed(&self) -> Vec<&SignatureView> {
        match &self.views {
            Some(Ok(views)) => views.iter().filter(|v| v.field.signed).collect(),
            _ => vec![],
        }
    }
}

/// What a signature check concluded, worst first.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Verdict {
    Invalid,
    Unknown,
    Valid,
}
fn verdict(e: &SignatureEvidence) -> Verdict {
    if !(e.crypto_valid && e.digest_matches && e.byte_range_valid) {
        Verdict::Invalid
    } else if e.trusted && e.covers_current_file {
        Verdict::Valid
    } else {
        Verdict::Unknown
    }
}
fn verdict_style(v: Verdict) -> (Icon, Color32) {
    match v {
        Verdict::Valid => (Icon::Check, theme::success()),
        Verdict::Unknown => (Icon::Warning, theme::warning()),
        Verdict::Invalid => (Icon::Close, theme::danger()),
    }
}
/// The common name from a subject such as "CN=Orhan Gökçöl, O=TÜV NORD".
fn common_name(subject: &str) -> String {
    subject
        .split(", ")
        .find_map(|part| part.strip_prefix("CN="))
        .unwrap_or(subject)
        .to_owned()
}
/// "D:20261013162310+03'00'" as "2026/10/13 16:23:10 +03:00", like Acrobat.
fn pdf_date(raw: &str) -> String {
    let Some(d) = raw.strip_prefix("D:") else {
        return raw.to_owned();
    };
    let digits: String = d.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() < 8 {
        return raw.to_owned();
    }
    let part = |range: std::ops::Range<usize>, default: &'static str| {
        digits.get(range).unwrap_or(default).to_owned()
    };
    let zone = match &d[digits.len()..] {
        "" => String::new(),
        z if z.starts_with('Z') => " +00:00".into(),
        z if z.starts_with(['+', '-']) => {
            let n: String = z[1..].chars().filter(char::is_ascii_digit).collect();
            format!(
                " {}{}:{}",
                &z[..1],
                n.get(0..2).unwrap_or("00"),
                n.get(2..4).unwrap_or("00")
            )
        }
        _ => String::new(),
    };
    format!(
        "{}/{}/{} {}:{}:{}{zone}",
        &digits[0..4],
        &digits[4..6],
        &digits[6..8],
        part(8..10, "00"),
        part(10..12, "00"),
        part(12..14, "00"),
    )
}
fn signer_name(v: &SignatureView) -> String {
    v.evidence
        .as_ref()
        .and_then(|e| e.signer.as_ref())
        .map(|s| common_name(&s.subject))
        .unwrap_or_else(|| "an unknown signer".into())
}

/// Recognized events from background signature work.
pub(super) enum SignatureJob {
    /// Tab, session revision, signed original, and the document without the signature.
    Removed(u64, u64, Arc<[u8]>, Result<Vec<u8>, String>),
}
pub(super) enum Prompt {
    Unlock(u64),
    Remove(u64, String),
}

/// Outline signed fields on the page; clicking one opens its details.
pub(super) fn page_overlay(
    tab: &mut Tab,
    ui: &mut egui::Ui,
    page: usize,
    page_rect: Rect,
    scale: f32,
) {
    let mut clicked = None;
    for v in tab.signatures.signed() {
        let (Some(p), Some(r)) = (v.field.page, v.field.rect) else {
            continue;
        };
        if p != page {
            continue;
        }
        let rect = Rect::from_min_max(
            page_rect.min + vec2(r[0], r[1]) * scale,
            page_rect.min + vec2(r[2], r[3]) * scale,
        );
        let response = ui
            .interact(
                rect,
                egui::Id::new((tab.id, page, "signature", &v.field.name)),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("Signed by {} · click for details", signer_name(v)));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("Signature: {}", v.field.name),
            )
        });
        let selected = tab.signatures.selected.as_deref() == Some(v.field.name.as_str());
        if response.hovered() || selected {
            let color = v
                .evidence
                .as_ref()
                .map_or(theme::warning(), |e| verdict_style(verdict(e)).1);
            ui::dashed_rect(ui.painter(), rect.expand(2.), color);
        }
        if response.clicked() {
            clicked = Some(v.field.name.clone());
        }
    }
    if let Some(name) = clicked {
        tab.signatures.selected = Some(name);
        tab.signatures.panel = true;
    }
}

/// A snapshot of a document that background work turns into saved bytes.
struct Inputs {
    source: Arc<[u8]>,
    annotations: Vec<annotations::Annotation>,
    revision: u64,
    original: Option<Arc<[u8]>>,
}
impl Inputs {
    fn of(tab: &Tab) -> Option<Self> {
        let s = tab.session.as_ref()?;
        Some(Self {
            source: s.source.clone(),
            annotations: s.annotations.clone(),
            revision: s.revision(),
            original: tab.signed_original.clone(),
        })
    }
    /// The document as it would be saved: the signed original plus edits.
    fn effective(&self) -> anyhow::Result<Vec<u8>> {
        let current = if self.revision == 0 || self.original.is_none() {
            self.source.to_vec()
        } else {
            annotations::export(&self.source, &self.annotations)?
        };
        match &self.original {
            Some(o) => signature_edit::incremental_update(o, &current),
            None => Ok(current),
        }
    }
}

impl Viewer {
    pub(super) fn signature_events(&mut self, ctx: &egui::Context) {
        for job in self.signature_rx.try_iter().collect::<Vec<_>>() {
            match job {
                SignatureJob::Removed(id, revision, original, result) => {
                    self.signature_busy = false;
                    let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
                        continue;
                    };
                    // The removal is an ordinary edit, so Undo brings the signature back.
                    match result.and_then(|bytes| {
                        let session = tab.session.as_mut().ok_or("The document closed")?;
                        if session.revision() != revision {
                            return Err("The document changed while the signature was being removed. Please try again.".into());
                        }
                        signature_edit::unlock(&original).map_err(|e| format!("{e:#}"))?;
                        session.editable = true;
                        session.commit_document(bytes).map_err(|e| format!("{e:#}"))
                    }) {
                        Ok(()) => {
                            tab.signed_original = Some(original);
                            tab.signatures.selected = None;
                            tab.signatures.key = None;
                            tab.bytes = None;
                            tab.refresh_document(ctx);
                            self.status = Some((
                                "Signature removed • Save a Copy to keep the change; your original file is unchanged".into(),
                                false,
                            ));
                        }
                        Err(e) => self.status = Some((e, true)),
                    }
                }
            }
        }
        let trusted = self.trusted.clone();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if let Some(rx) = &tab.signatures.job
            && let Ok(overview) = rx.try_recv()
        {
            tab.signatures.views = Some(overview);
            tab.signatures.job = None;
        }
        if !(tab.signed || tab.signed_original.is_some()) || tab.signatures.job.is_some() {
            return;
        }
        let Some(inputs) = Inputs::of(tab) else {
            return;
        };
        let key = (inputs.revision, trusted.len());
        if tab.signatures.key == Some(key) {
            return;
        }
        tab.signatures.key = Some(key);
        let (tx, rx) = mpsc::channel();
        tab.signatures.job = Some(rx);
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let overview = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let effective = inputs.effective()?;
                let fields = signature_edit::signature_fields(&effective)?;
                let trusted: Vec<&[u8]> = trusted.iter().map(Vec::as_slice).collect();
                let mut evidence: HashMap<String, SignatureEvidence> =
                    signing::verify_trusting(&effective, &trusted)?
                        .into_iter()
                        .map(|e| (e.field.clone(), e))
                        .collect();
                // Signatures in the order they were applied, numbered like Acrobat.
                let mut views: Vec<(usize, SignatureView)> = fields
                    .into_iter()
                    .map(|field| {
                        let end = signature_edit::signed_revision(&effective, &field.name)
                            .map_or(usize::MAX, |bytes| bytes.len());
                        let view = SignatureView {
                            evidence: evidence.remove(&field.name),
                            field,
                            revision: 0,
                        };
                        (end, view)
                    })
                    .collect();
                views.sort_by_key(|(end, _)| *end);
                let mut revision = 0;
                Ok::<_, anyhow::Error>(
                    views
                        .into_iter()
                        .map(|(_, mut view)| {
                            if view.field.signed {
                                revision += 1;
                                view.revision = revision;
                            }
                            view
                        })
                        .collect(),
                )
            }))
            .map_err(|_| "Signatures could not be checked".to_owned())
            .and_then(|r| r.map_err(|e| format!("{e:#}")));
            let _ = tx.send(overview);
            wake.request_repaint();
        });
    }

    /// A slim bar under the ribbon of signed documents.
    pub(super) fn signature_bar(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if !(tab.signed || tab.signed_original.is_some()) {
            return;
        }
        let unlocked = tab.signed_original.is_some();
        let signed = tab.signatures.signed();
        let worst = signed
            .iter()
            .filter_map(|v| v.evidence.as_ref())
            .map(verdict)
            .fold(None, |a: Option<Verdict>, v| {
                Some(a.map_or(v, |a| if v < a { v } else { a }))
            });
        let (icon, color) = worst.map_or((Icon::Shield, theme::muted()), verdict_style);
        let message = match (&tab.signatures.views, signed.as_slice()) {
            (None, _) => "Checking signatures…".to_owned(),
            (Some(Err(e)), _) => format!("Signatures could not be checked: {e}"),
            (Some(Ok(_)), []) => "No applied signatures remain".to_owned(),
            (Some(Ok(_)), [one]) => format!(
                "Signed by {} · {}",
                signer_name(one),
                match worst {
                    Some(Verdict::Valid) => "signature is valid",
                    Some(Verdict::Invalid) => "signature is invalid",
                    _ if one
                        .evidence
                        .as_ref()
                        .is_some_and(|e| !e.covers_current_file) =>
                    {
                        "changed after signing"
                    }
                    _ => "signer not in your trusted certificates",
                }
            ),
            (Some(Ok(_)), many) => format!(
                "{} signatures · {}",
                many.len(),
                match worst {
                    Some(Verdict::Valid) => "all valid",
                    Some(Verdict::Invalid) => "at least one is invalid",
                    _ => "at least one needs attention",
                }
            ),
        };
        let mut toggle = false;
        let mut unlock = false;
        egui::TopBottomPanel::top("signature_bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::surface().lerp_to_gamma(color, 0.1))
                    .inner_margin(egui::Margin::symmetric(14, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(18., 18.), egui::Sense::hover());
                    ui::draw_icon(ui.painter(), r, icon, color);
                    ui.label(RichText::new(message).size(12.5).color(theme::ink()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        toggle = ui
                            .add(ActionButton::new(Icon::Shield, "Signature panel").selected(tab.signatures.panel))
                            .clicked();
                        if unlocked {
                            ui.label(
                                RichText::new("Editing · saves as a new revision")
                                    .size(11.5)
                                    .color(theme::muted()),
                            );
                        } else {
                            unlock = ui
                                .add(ActionButton::new(Icon::Pen, "Edit anyway…"))
                                .on_hover_text("Edit this signed PDF; signatures stay valid for what they signed")
                                .clicked();
                        }
                    });
                });
            });
        if toggle {
            tab.signatures.panel = !tab.signatures.panel;
        }
        if unlock {
            self.signature_prompt = Some(Prompt::Unlock(tab.id));
        }
    }

    pub(super) fn signature_panel_open(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|t| t.signatures.panel && (t.signed || t.signed_original.is_some()))
    }

    /// Acrobat-style signature details with actions.
    pub(super) fn signature_panel(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let mut close = false;
        let mut jump = None;
        let mut trust = None;
        let mut export = None;
        let mut view_signed = None;
        let mut remove = None;
        let tab_id = tab.id;
        egui::SidePanel::right("signature_panel")
            .default_width(340.)
            .width_range(300.0..=460.0)
            .resizable(true)
            .frame(egui::Frame::new().fill(theme::paper()).inner_margin(16.))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui::title(ui, "Signatures", 17.);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui
                            .add(ActionButton::new(Icon::Close, "Close signature panel").compact())
                            .clicked();
                    });
                });
                egui::ScrollArea::vertical().id_salt("signature_details").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.;
                    let state = &mut tab.signatures;
                    let views: Vec<&SignatureView> = match &state.views {
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Checking signatures…").color(theme::muted()));
                            });
                            return;
                        }
                        Some(Err(e)) => {
                            ui::callout(ui, ui::Tone::Danger, e);
                            return;
                        }
                        Some(Ok(views)) => views.iter().filter(|v| v.field.signed).collect(),
                    };
                    if views.is_empty() {
                        ui::callout(ui, ui::Tone::Info, "This document has no applied signatures.");
                    }
                    for v in views {
                        let open = state.selected.as_deref() == Some(v.field.name.as_str());
                        let (icon, color) = v
                            .evidence
                            .as_ref()
                            .map_or((Icon::Warning, theme::warning()), |e| verdict_style(verdict(e)));
                        egui::Frame::new()
                            .fill(theme::surface())
                            .stroke(Stroke::new(1., if open { theme::accent_line() } else { theme::line() }))
                            .corner_radius(12)
                            .inner_margin(12)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                let header = ui
                                    .horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(vec2(18., 18.), egui::Sense::hover());
                                        ui::draw_icon(ui.painter(), r, icon, color);
                                        let font = ui::strong(ui, 13.5);
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(format!(
                                                    "Rev. {}: Signed by {}",
                                                    v.revision,
                                                    signer_name(v)
                                                ))
                                                .font(font)
                                                .color(theme::ink()),
                                            )
                                            .sense(egui::Sense::click()),
                                        )
                                    })
                                    .inner;
                                if header.clicked() {
                                    state.selected = if open { None } else { Some(v.field.name.clone()) };
                                    if let Some(page) = v.field.page {
                                        jump = Some(page);
                                    }
                                }
                                if !open {
                                    return;
                                }
                                let Some(e) = &v.evidence else {
                                    ui::callout(ui, ui::Tone::Warning, "This signature could not be read.");
                                    return;
                                };
                                let verdict = verdict(e);
                                let font = ui::strong(ui, 12.5);
                                ui.label(
                                    RichText::new(match verdict {
                                        Verdict::Valid => "Signature is VALID:",
                                        Verdict::Unknown => "Signature validity is UNKNOWN:",
                                        Verdict::Invalid => "Signature is INVALID:",
                                    })
                                    .font(font)
                                    .color(color),
                                );
                                let integrity = if verdict == Verdict::Invalid {
                                    "The signed content was altered, or the signature is damaged."
                                } else if e.covers_current_file {
                                    "This document has not been modified since this signature was applied."
                                } else {
                                    "The document was changed after this signature was applied. View the signed version to see exactly what was signed."
                                };
                                let identity = if e.trusted {
                                    "The signer's certificate is trusted for this session."
                                } else {
                                    "The signer's identity is unknown because the certificate is not in your trusted certificates."
                                };
                                let time = if e.timestamp.is_some() {
                                    "Signing time comes from an embedded timestamp (not independently checked)."
                                } else {
                                    "Signing time is from the clock on the signer's computer."
                                };
                                for line in [integrity, identity, time] {
                                    ui.horizontal_top(|ui| {
                                        ui.label(RichText::new("•").color(theme::muted()));
                                        ui.add(egui::Label::new(RichText::new(line).size(12.5).color(theme::ink())).wrap());
                                    });
                                }
                                ui.add_space(2.);
                                ui::caption(ui, "Signature details");
                                egui::Grid::new((&v.field.name, "signature_grid"))
                                    .num_columns(2)
                                    .spacing(vec2(10., 5.))
                                    .show(ui, |ui| {
                                        let algorithm = e
                                            .signer_der
                                            .as_deref()
                                            .and_then(mypdf_milestone0::identity::key_description)
                                            .unwrap_or_else(|| "Unknown".into());
                                        let mut rows = vec![
                                            ("Signing time", e.claimed_signing_time.as_deref().map_or_else(|| "Not provided".into(), pdf_date)),
                                            ("Algorithm", algorithm),
                                        ];
                                        if let Some(s) = &e.signer {
                                            rows.push(("Signer", common_name(&s.subject)));
                                            rows.push(("Issued by", common_name(&s.issuer)));
                                            rows.push(("Valid from", s.not_before.clone()));
                                            rows.push(("Valid until", s.not_after.clone()));
                                        }
                                        for (label, value) in rows {
                                            ui.label(RichText::new(label).size(12.).color(theme::muted()));
                                            ui.add(egui::Label::new(RichText::new(value).size(12.5)).wrap());
                                            ui.end_row();
                                        }
                                        ui.label(RichText::new("Field").size(12.).color(theme::muted()));
                                        let field = match v.field.page {
                                            Some(page) => format!("{} on page {}", v.field.name, page + 1),
                                            None => format!("{} (invisible)", v.field.name),
                                        };
                                        if ui.link(field).clicked()
                                            && let Some(page) = v.field.page
                                        {
                                            jump = Some(page);
                                        }
                                        ui.end_row();
                                    });
                                if state.certificates.contains(&v.field.name)
                                    && let Some(s) = &e.signer
                                {
                                    ui::section(ui, "Certificate", |ui| {
                                        for (label, value) in [
                                            ("Subject", &s.subject),
                                            ("Issuer", &s.issuer),
                                            ("Valid from", &s.not_before),
                                            ("Valid until", &s.not_after),
                                            ("SHA-256", &s.sha256),
                                        ] {
                                            ui.label(RichText::new(label).size(11.5).color(theme::muted()));
                                            ui.add(egui::Label::new(RichText::new(value).size(12.)).wrap());
                                        }
                                        if s.self_issued {
                                            ui.label(RichText::new("Self-issued: trust is a decision for each recipient.").small().color(theme::muted()));
                                        }
                                    });
                                }
                                ui.add_space(2.);
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = vec2(4., 4.);
                                    if let Some(der) = &e.signer_der {
                                        if !e.trusted
                                            && ui
                                                .add(ActionButton::new(Icon::Shield, "Trust for this session"))
                                                .on_hover_text("Treat this signer's certificate as trusted until you quit")
                                                .clicked()
                                        {
                                            trust = Some(der.clone());
                                        }
                                        let showing = state.certificates.contains(&v.field.name);
                                        if ui
                                            .add(ActionButton::new(Icon::Document, if showing { "Hide certificate" } else { "Show certificate" }))
                                            .clicked()
                                        {
                                            if showing {
                                                state.certificates.remove(&v.field.name);
                                            } else {
                                                state.certificates.insert(v.field.name.clone());
                                            }
                                        }
                                        if ui.add(ActionButton::new(Icon::Export, "Export certificate…")).clicked() {
                                            export = Some((der.clone(), common_name(&e.signer.as_ref().map_or(String::new(), |s| s.subject.clone()))));
                                        }
                                    }
                                    if ui.add(ActionButton::new(Icon::Read, "View signed version")).clicked() {
                                        view_signed = Some(v.field.name.clone());
                                    }
                                    if ui.add(ActionButton::new(Icon::Trash, "Remove signature…")).clicked() {
                                        remove = Some(v.field.name.clone());
                                    }
                                });
                            });
                    }
                    ui.label(
                        RichText::new("Validation is offline: revocation is not checked and trusted roots are never changed.")
                            .size(11.)
                            .color(theme::muted()),
                    );
                });
            });
        if close {
            tab.signatures.panel = false;
        }
        if let Some(page) = jump {
            tab.jump(page);
        }
        if let Some(der) = trust
            && !self.trusted.contains(&der)
        {
            self.trusted.push(der);
        }
        if let Some((der, name)) = export
            && let Some(path) = self.native_save(
                rfd::FileDialog::new()
                    .set_title("Export certificate")
                    .add_filter("Certificate", &["cer"])
                    .set_file_name(format!(
                        "{}.cer",
                        if name.is_empty() {
                            "certificate".into()
                        } else {
                            name
                        }
                    )),
            )
        {
            self.status = Some(match mypdf_milestone0::write_new(&path, &der) {
                Ok(()) => (format!("Certificate exported to {}", path.display()), false),
                Err(e) => (format!("{e:#}"), true),
            });
        }
        if let Some(field) = view_signed {
            self.open_signed_version(tab_id, &field, ctx);
        }
        if let Some(field) = remove {
            self.signature_prompt = Some(Prompt::Remove(tab_id, field));
        }
    }

    /// Open the document exactly as it was signed, in a new read-only tab.
    fn open_signed_version(&mut self, tab: u64, field: &str, ctx: &egui::Context) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else {
            return;
        };
        let Some(inputs) = Inputs::of(t) else {
            return;
        };
        let result = inputs
            .effective()
            .and_then(|bytes| signature_edit::signed_revision(&bytes, field));
        match result {
            Ok(bytes) => {
                let stem = t
                    .path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                self.open_memory(
                    PathBuf::from(format!("{stem} (signed version {field}).pdf")),
                    bytes,
                    ctx,
                );
            }
            Err(e) => self.status = Some((format!("{e:#}"), true)),
        }
    }

    pub(super) fn signature_dialogs(&mut self, ctx: &egui::Context) {
        match self.signature_prompt.take() {
            Some(Prompt::Unlock(id)) => match ui::confirmation(
                ctx,
                "unlock_signed_pdf",
                "Edit this signed PDF?",
                "Your changes are saved after the signed version, so each signature stays valid for what it signed. Readers will show that the document changed after signing.",
                &[
                    ("Edit Anyway", ui::DialogTone::Primary),
                    ("Cancel", ui::DialogTone::Secondary),
                ],
                1,
            ) {
                Some(0) => self.unlock_signed(id, ctx),
                Some(_) => {}
                None => self.signature_prompt = Some(Prompt::Unlock(id)),
            },
            Some(Prompt::Remove(id, field)) => match ui::confirmation(
                ctx,
                "remove_signature",
                &format!("Remove the signature “{field}”?"),
                "The signature is cleared from its field, which can be signed again. Other signatures stay valid. Your original file is unchanged until you Save a Copy.",
                &[
                    ("Keep Signature", ui::DialogTone::Primary),
                    ("Remove Signature", ui::DialogTone::Destructive),
                ],
                0,
            ) {
                Some(1) => self.remove_signature(id, field, ctx),
                Some(_) => {}
                None => self.signature_prompt = Some(Prompt::Remove(id, field)),
            },
            None => {}
        }
    }

    fn unlock_signed(&mut self, id: u64, ctx: &egui::Context) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let Some(source) = tab.session.as_ref().map(|s| s.source.clone()) else {
            return;
        };
        match signature_edit::unlock(&source) {
            Ok(()) => {
                tab.signed_original = Some(source);
                if let Some(s) = &mut tab.session {
                    s.editable = true;
                }
                // Re-render without the managed annotations, which are now editable.
                tab.bytes = None;
                tab.refresh_document(ctx);
                self.status = Some((
                    "Editing a signed PDF • Save a Copy appends your changes after the signed version".into(),
                    false,
                ));
            }
            Err(e) => self.status = Some((format!("{e:#}"), true)),
        }
    }

    fn remove_signature(&mut self, id: u64, field: String, ctx: &egui::Context) {
        if self.signature_busy {
            return;
        }
        let Some(tab) = self.tabs.iter().find(|t| t.id == id) else {
            return;
        };
        let Some(inputs) = Inputs::of(tab) else {
            return;
        };
        let revision = inputs.revision;
        let original = inputs.original.clone().unwrap_or(inputs.source.clone());
        self.signature_busy = true;
        self.status = Some(("Removing signature…".into(), false));
        let tx = self.signature_tx.clone();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                signature_edit::remove_signature(&inputs.effective()?, &field)
            }))
            .map_err(|_| "The signature could not be removed".to_owned())
            .and_then(|r| r.map_err(|e| format!("{e:#}")));
            let _ = tx.send(SignatureJob::Removed(id, revision, original, result));
            wake.request_repaint();
        });
    }
}

/// A PDF signed in a visible "Approval" field by a fresh synthetic identity.
#[cfg(test)]
pub(super) fn signed_fixture(source: &[u8], name: &str) -> Vec<u8> {
    let password = "synthetic-signature-panel";
    let created = mypdf_milestone0::identity::create(name, password, true).unwrap();
    let signer =
        mypdf_milestone0::identity::FileSigner::load(&created.pkcs12, password, None).unwrap();
    let request = signing::DesktopSigningRequest {
        field: "Approval".into(),
        rect: Some([72., 600., 330., 680.]),
        name: name.into(),
        reason: "Approved".into(),
        ..Default::default()
    };
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(signing::sign_custom(source, &signer, &request))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    use mypdf_milestone0::{content, document};

    #[test]
    fn signing_times_and_names_read_like_acrobat() {
        assert_eq!(pdf_date("D:20261013162310Z"), "2026/10/13 16:23:10 +00:00");
        assert_eq!(
            pdf_date("D:20240102030405+03'00'"),
            "2024/01/02 03:04:05 +03:00"
        );
        assert_eq!(pdf_date("D:20240102"), "2024/01/02 00:00:00");
        assert_eq!(pdf_date("yesterday"), "yesterday");
        assert_eq!(pdf_date("D:20240102ü"), "2024/01/02 00:00:00");
        assert_eq!(common_name("CN=Orhan Gökçöl, O=Example"), "Orhan Gökçöl");
        assert_eq!(common_name("O=No Name"), "O=No Name");
    }

    #[test]
    fn signatures_are_inspected_on_click_then_edited_and_removed_without_breaking_signed_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let font = document::default_font().unwrap();
        let signed = signed_fixture(&document::fixture(&font).unwrap(), "Panel Signer");
        let pdf = temp.path().join("Signed.pdf");
        std::fs::write(&pdf, &signed).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![pdf],
        );
        let frame = |app: &mut Viewer, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 760.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let until = |app: &mut Viewer, ready: &dyn Fn(&Viewer) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            while !ready(app) {
                assert!(std::time::Instant::now() < deadline, "timed out");
                frame(app, vec![]);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        let first = |a: &Viewer| -> Option<(bool, bool, bool)> {
            let views = a.tabs[0].signatures.signed();
            let e = views.first()?.evidence.as_ref()?;
            Some((
                e.crypto_valid && e.digest_matches,
                e.trusted,
                e.covers_current_file,
            ))
        };
        until(&mut app, &|a| {
            a.tabs[0].signatures.views.is_some() && !a.tabs[0].pages.is_empty()
        });
        let tab_id = app.tabs[0].id;
        assert!(app.tabs[0].signed && !app.tabs[0].session.as_ref().unwrap().editable);
        {
            let views = app.tabs[0].signatures.signed();
            assert_eq!(views.len(), 1);
            assert_eq!(views[0].revision, 1);
            assert_eq!(signer_name(views[0]), "Panel Signer");
            let e = views[0].evidence.as_ref().unwrap();
            assert!(
                verdict(e) == Verdict::Unknown,
                "valid, but the signer is not trusted"
            );
            assert!(e.covers_current_file && !e.trusted);
            assert_eq!(
                e.signer_der
                    .as_deref()
                    .and_then(mypdf_milestone0::identity::key_description)
                    .as_deref(),
                Some("ECDSA P-256")
            );
        }

        // Clicking the signature on the page opens its details.
        app.tabs[0].zoom = Zoom::Page;
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let target = ctx
            .read_response(egui::Id::new((tab_id, 0_usize, "signature", "Approval")))
            .expect("the signed field is clickable")
            .rect
            .center();
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(target),
                    egui::Event::PointerButton {
                        pos: target,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(app.tabs[0].signatures.panel);
        assert_eq!(app.tabs[0].signatures.selected.as_deref(), Some("Approval"));
        assert!(app.signature_panel_open());
        frame(&mut app, vec![]);

        // Trusting the certificate for the session makes the signature valid.
        let der = app.tabs[0].signatures.signed()[0]
            .evidence
            .as_ref()
            .and_then(|e| e.signer_der.clone())
            .unwrap();
        app.trusted.push(der);
        until(&mut app, &|a| first(a) == Some((true, true, true)));

        // Editing needs an explicit unlock, then saves after the signed bytes.
        app.signature_prompt = Some(Prompt::Unlock(tab_id));
        frame(&mut app, vec![]);
        assert!(
            app.signature_prompt.is_some(),
            "the dialog waits for an answer"
        );
        app.signature_prompt = None;
        app.unlock_signed(tab_id, &ctx);
        until(&mut app, &|a| {
            a.tabs[0].session.as_ref().unwrap().editable && a.tabs[0].bytes.is_some()
        });
        let session = app.tabs[0].session.as_mut().unwrap();
        let edited = content::add_text(
            &session.source,
            1,
            "Approved with changes",
            [50., 300., 400., 330.],
            14.,
            [0.; 3],
        )
        .unwrap();
        session.commit_document(edited).unwrap();
        until(&mut app, &|a| first(a) == Some((true, true, false)));
        let saved = Inputs::of(&app.tabs[0]).unwrap().effective().unwrap();
        assert!(saved.starts_with(&signed), "the signed bytes are kept");
        let report = signing::verify(&saved, None).unwrap();
        assert!(
            report[0].crypto_valid && report[0].digest_matches && !report[0].covers_current_file
        );
        let text = |bytes: &[u8]| -> Vec<String> {
            document::text_runs(bytes, 1)
                .unwrap()
                .into_iter()
                .map(|r| r.text)
                .collect()
        };
        assert!(text(&saved).iter().any(|t| t == "Approved with changes"));

        // "View signed version" opens exactly what was signed.
        app.open_signed_version(tab_id, "Approval", &ctx);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tabs[1].bytes.as_deref(), Some(&signed[..]));
        app.close(1);
        app.active = 0;

        // Removing the signature keeps the edits; Undo brings it back.
        app.remove_signature(tab_id, "Approval".into(), &ctx);
        until(&mut app, &|a| {
            !a.signature_busy
                && a.tabs[0].signatures.job.is_none()
                && matches!(&a.tabs[0].signatures.views, Some(Ok(v)) if v.iter().all(|v| !v.field.signed))
        });
        let saved = Inputs::of(&app.tabs[0]).unwrap().effective().unwrap();
        assert!(saved.starts_with(&signed));
        assert!(signing::verify(&saved, None).unwrap().is_empty());
        assert!(text(&saved).iter().any(|t| t == "Approved with changes"));
        app.editor_undo(false);
        until(&mut app, &|a| first(a) == Some((true, true, false)));
    }
}
