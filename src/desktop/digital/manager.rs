//! The Signatures window for everyday use: saved signatures (which digital ID
//! signs and how it looks), the digital IDs on this computer, and trusted
//! certificates. Written for people who have never handled a certificate.
use super::*;
use mypdf_milestone0::assets::Asset;
use mypdf_milestone0::signing::Background;
use std::hash::{Hash, Hasher};

/// A saved signature: the digital ID that signs and how it looks on the page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct SavedSignature {
    pub id: String,
    pub name: String,
    pub identity: Option<DefaultIdentity>,
    /// The name shown on the page; empty uses the name in the certificate.
    pub display_name: String,
    pub show_name: bool,
    pub show_date: bool,
    /// An optional line such as "Approved".
    pub note: String,
    /// A drawn, typed or imported image from the library.
    pub asset_id: Option<String>,
    pub background: Background,
}
impl Default for SavedSignature {
    fn default() -> Self {
        Self {
            id: format!("{:032x}", rand::random::<u128>()),
            name: "My signature".into(),
            identity: None,
            display_name: String::new(),
            show_name: true,
            show_date: true,
            note: String::new(),
            asset_id: None,
            background: Background::White,
        }
    }
}
impl SavedSignature {
    /// How the box is composed, or None when it would show nothing.
    pub fn layout(&self) -> Option<AppearanceLayout> {
        let details = self.show_name || self.show_date || !self.note.trim().is_empty();
        match (self.asset_id.is_some(), details) {
            (true, true) => Some(AppearanceLayout::ImageAndDetails),
            (true, false) => Some(AppearanceLayout::ImageOnly),
            (false, true) => Some(AppearanceLayout::DetailsOnly),
            (false, false) => None,
        }
    }
}

/// The look of the signature in use, applied to every signing request.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Look {
    pub display_name: String,
    pub show_name: bool,
    pub show_date: bool,
    pub note: String,
    pub background: Background,
}
impl Look {
    pub fn of(signature: &SavedSignature) -> Self {
        Self {
            display_name: signature.display_name.clone(),
            show_name: signature.show_name,
            show_date: signature.show_date,
            note: signature.note.clone(),
            background: signature.background,
        }
    }
    /// Apply to a request whose name is the certificate holder's.
    pub fn apply(&self, request: &mut DesktopSigningRequest) {
        if !self.display_name.trim().is_empty() {
            request.name = self.display_name.trim().to_owned();
        }
        request.show_name = self.show_name;
        request.show_date = self.show_date;
        request.background = self.background;
        if request.reason.trim().is_empty() {
            request.reason = self.note.trim().to_owned();
        }
    }
}

pub(super) type PreviewResult = Result<TextureHandle, String>;

pub(super) struct ManagerState {
    pub selected: Option<String>,
    /// Rendered previews by what they show, shared by cards and editors.
    previews: HashMap<u64, PreviewResult>,
    pending: HashSet<u64>,
    tx: mpsc::Sender<(u64, Result<Vec<u8>, String>)>,
    rx: mpsc::Receiver<(u64, Result<Vec<u8>, String>)>,
    /// The signature that receives the next image added to the library.
    pub adopt_into: Option<String>,
    /// The "Create my digital ID" form.
    pub new_id: Option<NewId>,
    pub confirm_delete: Option<String>,
    /// Edits to save once no field is being typed in.
    pub dirty: bool,
    /// The signature chosen in the Sign tab.
    pub sign_with: Option<String>,
}
pub(super) struct NewId {
    /// The signature that will sign with the new ID.
    pub signature: Option<String>,
    pub name: String,
    pub email: String,
    pub trust: bool,
}
impl ManagerState {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            selected: None,
            previews: HashMap::new(),
            pending: HashSet::new(),
            tx,
            rx,
            adopt_into: None,
            new_id: None,
            confirm_delete: None,
            dirty: false,
            sign_with: None,
        }
    }
    /// Turn finished previews into textures.
    pub fn receive(&mut self, ctx: &egui::Context) {
        for (key, result) in self.rx.try_iter().collect::<Vec<_>>() {
            self.pending.remove(&key);
            if self.previews.len() > 96 {
                self.previews.clear();
            }
            let texture = result.and_then(|png| {
                let image = mypdf_milestone0::assets::decode_png(&png)
                    .map_err(|e| format!("{e:#}"))?
                    .into_rgba8();
                Ok(ctx.load_texture(
                    format!("signature_preview_{key:016x}"),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        &image,
                    ),
                    egui::TextureOptions::LINEAR,
                ))
            });
            self.previews.insert(key, texture);
        }
    }
    /// Whether every requested preview has been drawn.
    #[cfg(test)]
    pub fn idle(&self) -> bool {
        self.pending.is_empty()
    }
    #[cfg(test)]
    pub fn drawn(&self) -> usize {
        self.previews.len()
    }
    #[cfg(test)]
    pub fn failures(&self) -> Vec<String> {
        self.previews
            .values()
            .filter_map(|p| p.as_ref().err().cloned())
            .collect()
    }
    /// The preview of `request`, drawn in the background the first time.
    pub fn preview(
        &mut self,
        ctx: &egui::Context,
        request: &DesktopSigningRequest,
    ) -> Option<PreviewResult> {
        let key = request_key(request);
        if let Some(found) = self.previews.get(&key) {
            return Some(found.clone());
        }
        if self.pending.insert(key) {
            let tx = self.tx.clone();
            let wake = ctx.clone();
            let request = request.clone();
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(|| signing::signature_artwork(&request))
                    .map_err(|_| "The preview could not be drawn.".to_string())
                    .and_then(|r| r.map_err(|e| format!("{e:#}")));
                let _ = tx.send((key, result));
                wake.request_repaint();
            });
        }
        None
    }
}

/// What a preview depends on.
fn request_key(request: &DesktopSigningRequest) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    request.name.hash(&mut h);
    request.reason.hash(&mut h);
    request.location.hash(&mut h);
    request.show_name.hash(&mut h);
    request.show_date.hash(&mut h);
    format!("{:?}{:?}", request.background, request.layout).hash(&mut h);
    request
        .artwork
        .as_ref()
        .map(|a| (&a.id, a.aspect.to_bits()))
        .hash(&mut h);
    request
        .rect
        .map(|r| [(r[2] - r[0]).round() as i32, (r[3] - r[1]).round() as i32])
        .hash(&mut h);
    h.finish()
}

/// A box of a sensible size for showing a signature on its own.
fn sample_rect(layout: AppearanceLayout, artwork: Option<&Asset>) -> [f32; 4] {
    match layout {
        AppearanceLayout::ImageOnly => {
            let aspect = artwork.map_or(3., |a| a.aspect);
            [0., 0., 220., (220. / aspect).clamp(24., 110.)]
        }
        _ => [0., 0., 300., 90.],
    }
}

/// Where a digital ID lives, in plain words.
pub(super) fn place(entry: &Entry) -> String {
    match &entry.source {
        Source::Keychain { .. } if entry.token => "Smart card or USB token".into(),
        Source::Keychain { path: None, .. } => "Keychain on this Mac".into(),
        Source::Keychain { .. } => "Other keychain".into(),
        Source::WindowsStore { .. } if entry.token => "Smart card or USB token".into(),
        Source::WindowsStore { .. } => "Windows certificate store".into(),
        Source::File { alias, .. } => format!("File {alias}"),
        Source::Certificate => "Certificate only".into(),
    }
}

/// Days until the certificate expires, and the date it does.
fn expiry(der: &[u8]) -> Option<(i64, String)> {
    let (_, certificate) = x509_parser::parse_x509_certificate(der).ok()?;
    let at = certificate.validity().not_after.timestamp();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let date = time::OffsetDateTime::from_unix_timestamp(at).ok()?;
    let month = date.month().to_string();
    Some((
        (at - now).div_euclid(86_400),
        format!(
            "{} {} {}",
            date.day(),
            &month[..3.min(month.len())],
            date.year()
        ),
    ))
}

/// A digital ID's state, as a short phrase and its tone.
pub(super) fn status(entry: &Entry) -> (String, ui::Tone) {
    if matches!(entry.source, Source::Certificate) {
        return ("Can check signatures, cannot sign".into(), ui::Tone::Info);
    }
    match expiry(&entry.der) {
        Some((days, date)) if days < 0 => (format!("Expired on {date}"), ui::Tone::Danger),
        _ if !entry.eligible => ("Not available right now".into(), ui::Tone::Warning),
        _ if entry.purpose().other_purpose() => (
            format!(
                "{} · choose a document ID for PDFs",
                entry.purpose().label()
            ),
            ui::Tone::Warning,
        ),
        Some((days, date)) if days <= 30 => {
            (format!("Ready · expires soon ({date})"), ui::Tone::Warning)
        }
        Some((_, date)) => (format!("Ready · valid until {date}"), ui::Tone::Success),
        None => ("Ready".into(), ui::Tone::Success),
    }
}

/// Paint a preview texture inside `rect`, keeping its proportions.
pub(super) fn paint_preview(
    ui: &egui::Ui,
    rect: Rect,
    preview: Option<&PreviewResult>,
    clear: bool,
) {
    let painter = ui.painter();
    if clear {
        checkerboard(painter, rect);
    } else {
        // Signatures sit on white paper in both themes.
        painter.rect_filled(rect, 5, Color32::WHITE);
    }
    match preview {
        Some(Ok(texture)) => {
            let size = texture.size_vec2();
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            let image = Rect::from_center_size(rect.center(), size * scale);
            painter.image(
                texture.id(),
                image,
                Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
                Color32::WHITE,
            );
        }
        Some(Err(_)) => {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Nothing to show yet",
                FontId::proportional(11.),
                Color32::from_gray(120),
            );
        }
        None => {
            let spinner = Rect::from_center_size(rect.center(), vec2(16., 16.));
            egui::Spinner::new()
                .color(Color32::from_gray(150))
                .paint_at(ui, spinner);
        }
    }
}

/// A saved signature as a card: its preview, its name and the Quick Sign star.
pub(super) fn signature_card(
    ui: &mut egui::Ui,
    signature: &SavedSignature,
    preview: Option<&PreviewResult>,
    default: bool,
    selected: bool,
    ready: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(148., 104.), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            true,
            selected,
            signature.name.as_str(),
        )
    });
    let painter = ui.painter();
    painter.rect(
        rect,
        11,
        if selected {
            theme::accent_soft()
        } else {
            theme::surface()
        },
        Stroke::new(
            if selected { 2. } else { 1. },
            if selected {
                theme::accent()
            } else if response.hovered() {
                theme::accent_line()
            } else {
                theme::line()
            },
        ),
        egui::StrokeKind::Inside,
    );
    let art = Rect::from_min_max(
        rect.min + vec2(10., 10.),
        pos2(rect.max.x - 10., rect.min.y + 70.),
    );
    paint_preview(ui, art, preview, signature.background == Background::Clear);
    let mut left = rect.left() + 12.;
    let baseline = rect.bottom() - 18.;
    if default {
        let star = Rect::from_center_size(pos2(left + 7., baseline), vec2(14., 14.));
        ui::draw_icon(ui.painter(), star, Icon::Star, theme::warning());
        left += 18.;
    }
    let right = if ready {
        rect.right() - 10.
    } else {
        let warn = Rect::from_center_size(pos2(rect.right() - 17., baseline), vec2(14., 14.));
        ui::draw_icon(ui.painter(), warn, Icon::Warning, theme::warning());
        rect.right() - 28.
    };
    let mut job = egui::text::LayoutJob::single_section(
        signature.name.clone(),
        egui::TextFormat::simple(ui::strong(ui, 12.5), theme::ink()),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - left).max(20.));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        pos2(left, baseline - galley.size().y / 2.),
        galley,
        theme::ink(),
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            11,
            Stroke::new(2.5, theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
    let hint = match (default, ready) {
        (true, true) => "Quick Sign uses this signature",
        (_, false) => "Choose a digital ID for this signature",
        _ => "Edit this signature",
    };
    response
        .on_hover_text(hint)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The dashed card that adds a signature.
fn new_card(ui: &mut egui::Ui, size: egui::Vec2, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let color = if response.hovered() {
        theme::accent()
    } else {
        theme::muted()
    };
    if response.hovered() {
        ui.painter().rect_filled(rect, 11, theme::accent_soft());
    }
    ui::dashed_rect(ui.painter(), rect.shrink(1.), color);
    let icon = Rect::from_center_size(rect.center() - vec2(0., 12.), vec2(22., 22.));
    ui::draw_icon(ui.painter(), icon, Icon::Plus, color);
    ui.painter().text(
        rect.center() + vec2(0., 16.),
        egui::Align2::CENTER_CENTER,
        label,
        ui::strong(ui, 12.),
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A background choice drawn as a small swatch.
fn background_tile(ui: &mut egui::Ui, background: Background, selected: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(92., 70.), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, background.label())
    });
    ui.painter().rect(
        rect,
        10,
        if selected {
            theme::accent_soft()
        } else {
            theme::surface()
        },
        Stroke::new(
            if selected { 2. } else { 1. },
            if selected {
                theme::accent()
            } else if response.hovered() {
                theme::accent_line()
            } else {
                theme::line()
            },
        ),
        egui::StrokeKind::Inside,
    );
    let swatch = Rect::from_min_max(
        rect.min + vec2(14., 10.),
        pos2(rect.max.x - 14., rect.min.y + 44.),
    );
    let rgb = |c: [f32; 3]| {
        Color32::from_rgb(
            (c[0] * 255.) as u8,
            (c[1] * 255.) as u8,
            (c[2] * 255.) as u8,
        )
    };
    match background.colors() {
        None => checkerboard(ui.painter(), swatch),
        Some((fill, frame)) => {
            ui.painter().rect_filled(swatch, 4, rgb(fill));
            match frame {
                Some(frame) => ui.painter().rect_stroke(
                    swatch.shrink(3.),
                    2,
                    Stroke::new(1.2, rgb(frame)),
                    egui::StrokeKind::Inside,
                ),
                None => ui.painter().rect_stroke(
                    swatch,
                    4,
                    Stroke::new(1., theme::line()),
                    egui::StrokeKind::Inside,
                ),
            };
        }
    }
    // A hint of handwriting on the swatch.
    let ink = Color32::from_rgb(28, 46, 120);
    let points: Vec<egui::Pos2> = (0..=16)
        .map(|i| {
            let t = i as f32 / 16.;
            pos2(
                swatch.left() + 12. + t * (swatch.width() - 24.),
                swatch.center().y + (t * 14.).sin() * 5. * (1. - t * 0.4),
            )
        })
        .collect();
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(1.4, ink)));
    ui.painter().text(
        pos2(rect.center().x, rect.bottom() - 13.),
        egui::Align2::CENTER_CENTER,
        background.label(),
        FontId::proportional(11.5),
        if selected {
            theme::accent()
        } else {
            theme::ink()
        },
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// The "no image" choice beside the library images.
fn no_image_card(ui: &mut egui::Ui, selected: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(112., 78.), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, "No image")
    });
    ui.painter().rect(
        rect,
        10,
        if selected {
            theme::accent_soft()
        } else {
            theme::surface()
        },
        Stroke::new(
            if selected { 2. } else { 1. },
            if selected {
                theme::accent()
            } else if response.hovered() {
                theme::accent_line()
            } else {
                theme::line()
            },
        ),
        egui::StrokeKind::Inside,
    );
    let icon = Rect::from_center_size(rect.center() - vec2(0., 8.), vec2(20., 20.));
    ui::draw_icon(ui.painter(), icon, Icon::Close, theme::muted());
    ui.painter().text(
        pos2(rect.center().x, rect.bottom() - 16.),
        egui::Align2::CENTER_CENTER,
        "No image",
        FontId::proportional(11.),
        theme::muted(),
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// A text field that reports whether it has focus.
fn text_field(ui: &mut egui::Ui, value: &mut String, hint: &str, limit: usize) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .margin(ui::FIELD_MARGIN)
            .desired_width(f32::INFINITY)
            .hint_text(hint)
            .char_limit(limit),
    )
}

impl DigitalState {
    /// The digital ID a signature signs with, if it is available now.
    pub(super) fn signer_of(&self, signature: &SavedSignature) -> Option<&Entry> {
        let reference = signature.identity.as_ref()?;
        self.entries
            .iter()
            .find(|e| e.eligible && e.reference() == *reference)
    }
    /// Digital IDs that can sign.
    pub(super) fn signing_entries(&self) -> impl Iterator<Item = &Entry> {
        // Keep unrelated certificate purposes available for explicit inspection
        // and selection, but never put them ahead of ordinary signing IDs.
        let available = |e: &&Entry| e.eligible && !matches!(e.source, Source::Certificate);
        self.entries
            .iter()
            .filter(available)
            .filter(|e| !e.purpose().other_purpose())
            .chain(
                self.entries
                    .iter()
                    .filter(available)
                    .filter(|e| e.purpose().other_purpose()),
            )
    }
    pub(super) fn signature(&self, id: &str) -> Option<&SavedSignature> {
        self.signatures.iter().find(|s| s.id == id)
    }
    /// The request that draws `signature` on its own, as a preview.
    pub(super) fn signature_request(
        &self,
        signature: &SavedSignature,
        assets: &[Asset],
    ) -> DesktopSigningRequest {
        let artwork = signature
            .asset_id
            .as_ref()
            .and_then(|id| assets.iter().find(|a| &a.id == id))
            .cloned();
        let mut shown = signature.clone();
        if artwork.is_none() {
            shown.asset_id = None;
        }
        let layout = shown.layout().unwrap_or(AppearanceLayout::DetailsOnly);
        let mut request = DesktopSigningRequest {
            rect: Some(sample_rect(layout, artwork.as_ref())),
            name: self
                .signer_of(signature)
                .map_or_else(|| "Your Name".to_owned(), Entry::signer_name),
            artwork,
            layout,
            ..Default::default()
        };
        Look::of(signature).apply(&mut request);
        request
    }
    /// A name for a new signature that no other signature uses.
    fn fresh_name(&self, base: &str) -> String {
        (1..)
            .map(|n| {
                if n == 1 {
                    base.to_owned()
                } else {
                    format!("{base} {n}")
                }
            })
            .find(|name| !self.signatures.iter().any(|s| &s.name == name))
            .unwrap()
    }
    /// Add a signature that signs with `identity`, or with the ID most likely
    /// meant: the Quick Sign one, or the only one there is. Returns its id.
    pub(super) fn add_signature(&mut self, identity: Option<DefaultIdentity>) -> String {
        let identity = identity
            .or_else(|| {
                self.default_signature()
                    .filter(|s| self.signer_of(s).is_some())
                    .and_then(|s| s.identity.clone())
            })
            .or_else(|| {
                let mut ids = self.signing_entries();
                match (ids.next(), ids.next()) {
                    (Some(only), None) => Some(only.reference()),
                    _ => None,
                }
            });
        let signature = SavedSignature {
            name: self.fresh_name("My signature"),
            identity,
            ..Default::default()
        };
        let id = signature.id.clone();
        self.signatures.push(signature);
        if self.default_signature().is_none() {
            self.default_signature = Some(id.clone());
        }
        self.manager.selected = Some(id.clone());
        self.commit_signatures();
        id
    }
    /// Make a signature the one Quick Sign uses.
    pub(super) fn make_default(&mut self, id: &str) {
        if self.signature(id).is_some() {
            self.default_signature = Some(id.to_owned());
            self.commit_signatures();
        }
    }
    pub(super) fn delete_signature(&mut self, id: &str) {
        self.signatures.retain(|s| s.id != id);
        if self.default_signature.as_deref() == Some(id) {
            // Quick Sign keeps working with the next signature.
            self.default_signature = self.signatures.first().map(|s| s.id.clone());
        }
        if self.manager.selected.as_deref() == Some(id) {
            self.manager.selected = self.default_signature.clone();
        }
        if self.manager.sign_with.as_deref() == Some(id) {
            self.manager.sign_with = None;
        }
        self.commit_signatures();
    }
    fn duplicate_signature(&mut self, id: &str) {
        let Some(original) = self.signature(id).cloned() else {
            return;
        };
        let copy = SavedSignature {
            id: SavedSignature::default().id,
            name: self.fresh_name(&format!("{} copy", original.name)),
            ..original
        };
        self.manager.selected = Some(copy.id.clone());
        self.signatures.push(copy);
        self.commit_signatures();
    }
    /// Save the signatures and use the default one for signing.
    pub(super) fn commit_signatures(&mut self) {
        self.manager.dirty = false;
        self.apply_default_signature();
        self.select_default();
        if let Err(e) = self.save_catalogue() {
            self.error = Some(format!(
                "Your signatures apply to this session, but could not be saved: {e:#}"
            ));
        }
    }
    /// Use a signature for the next signing without changing the default.
    pub(super) fn use_signature(&mut self, id: &str) {
        let Some(signature) = self.signature(id).cloned() else {
            return;
        };
        let entry = self
            .signer_of(&signature)
            .map(|e| (e.id.clone(), e.signer_name()));
        if self.selected != entry.as_ref().map(|e| e.0.clone()) {
            self.sign_password = Zeroizing::new(String::new());
        }
        self.selected = entry.as_ref().map(|e| e.0.clone());
        if let Some((_, name)) = entry {
            self.request.name = name;
        }
        self.asset_id = signature.asset_id.clone();
        self.layout = signature.layout().unwrap_or(AppearanceLayout::DetailsOnly);
        self.look = Look::of(&signature);
        self.manager.sign_with = Some(signature.id);
    }

    /// My signatures: cards for each saved signature, then the editor.
    pub(super) fn signatures_panel(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut editing::EditorState,
        assets: &[Asset],
        action: &mut Option<Action>,
    ) {
        if self.signatures.is_empty() {
            self.first_signature(ui);
            return;
        }
        if self
            .manager
            .selected
            .as_ref()
            .is_none_or(|id| self.signature(id).is_none())
        {
            self.manager.selected = self
                .default_signature
                .clone()
                .or_else(|| self.signatures.first().map(|s| s.id.clone()));
        }
        ui.label(
            RichText::new("A signature is your digital ID plus how it looks on the page. The one with a star is used by Quick Sign.")
                .color(theme::muted()),
        );
        let ctx = ui.ctx().clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(10., 10.);
            for signature in self.signatures.clone() {
                let request = self.signature_request(&signature, assets);
                let preview = self.manager.preview(&ctx, &request);
                let selected = self.manager.selected.as_ref() == Some(&signature.id);
                let default = self.default_signature.as_ref() == Some(&signature.id);
                let ready = self.signer_of(&signature).is_some() && signature.layout().is_some();
                if signature_card(ui, &signature, preview.as_ref(), default, selected, ready)
                    .clicked()
                {
                    self.save_if_dirty();
                    self.manager.selected = Some(signature.id.clone());
                }
            }
            if new_card(ui, vec2(148., 104.), "New signature")
                .on_hover_text("Create another signature, for example initials or a company stamp")
                .clicked()
            {
                self.save_if_dirty();
                self.add_signature(None);
            }
        });
        let Some(id) = self.manager.selected.clone() else {
            return;
        };
        let Some(before) = self.signature(&id).cloned() else {
            return;
        };
        let mut signature = before.clone();
        let mut typing = false;
        let mut remove = false;
        let mut duplicate = false;
        let mut make_default = false;
        ui::section(ui, "Edit signature", |ui| {
            ui.spacing_mut().item_spacing.y = 12.;
            typing |= ui::field(ui, "Name", |ui| {
                text_field(
                    ui,
                    &mut signature.name,
                    "For example: Work, Initials, Approval",
                    80,
                )
            })
            .has_focus();
            self.signs_with(ui, &mut signature, action);
            ui::field(ui, "Show on the signature", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut signature.show_name, "Your name");
                    ui.checkbox(&mut signature.show_date, "Date and time");
                });
                if signature.show_name {
                    let certificate = self.signer_of(&signature).map_or_else(
                        || "The name in your digital ID".to_owned(),
                        Entry::signer_name,
                    );
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Name shown").color(theme::muted()));
                        typing |= text_field(ui, &mut signature.display_name, &certificate, 200)
                            .on_hover_text("Leave empty to use the name in your digital ID")
                            .has_focus();
                    });
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Note").color(theme::muted()));
                    typing |=
                        text_field(ui, &mut signature.note, "Optional, such as “Approved”", 300)
                            .has_focus();
                });
            });
            ui::field(ui, "Image", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8., 8.);
                    if no_image_card(ui, signature.asset_id.is_none()) {
                        signature.asset_id = None;
                    }
                    // Newest first: a just-drawn signature is easy to find.
                    for asset in assets.iter().rev().take(11) {
                        let selected = signature.asset_id.as_ref() == Some(&asset.id);
                        if artwork_card(ui, editor, asset, selected) {
                            signature.asset_id = Some(asset.id.clone());
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add(ActionButton::new(Icon::Pen, "Draw or type…"))
                        .on_hover_text("Draw your handwritten signature, or type it")
                        .clicked()
                    {
                        self.manager.adopt_into = Some(id.clone());
                        *action = Some(Action::DrawArtwork);
                    }
                    if ui
                        .add(ActionButton::new(Icon::Import, "Use a picture…"))
                        .on_hover_text(
                            "A scan or photo of your signature, or a logo (PNG, JPEG or SVG)",
                        )
                        .clicked()
                    {
                        self.manager.adopt_into = Some(id.clone());
                        *action = Some(Action::ImportArtwork);
                    }
                    if let Some(asset) = assets
                        .iter()
                        .find(|a| Some(&a.id) == signature.asset_id.as_ref())
                        && matches!(
                            asset.kind,
                            mypdf_milestone0::annotations::Kind::Image { .. }
                        )
                        && ui
                            .add(ActionButton::new(Icon::FieldImage, "Remove background…"))
                            .clicked()
                    {
                        *action = Some(Action::RemoveArtworkBackground(id.clone(), asset.clone()));
                    }
                });
            });
            ui::field(ui, "Background", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8., 8.);
                    for background in Background::CHOICES {
                        if background_tile(ui, background, signature.background == background) {
                            signature.background = background;
                        }
                    }
                });
            });
            ui::field(ui, "Preview", |ui| {
                if signature.layout().is_none() {
                    ui::callout(
                        ui,
                        ui::Tone::Warning,
                        "Choose what the signature shows: your name, the date, a note or an image.",
                    );
                    return;
                }
                let request = self.signature_request(&signature, assets);
                let preview = self.manager.preview(&ctx, &request);
                let [_, _, w, h] = request.rect.unwrap_or([0., 0., 300., 90.]);
                let width = ui.available_width().min(w * 1.4);
                let (rect, _) =
                    ui.allocate_exact_size(vec2(width, width * h / w), egui::Sense::hover());
                ui.painter().rect_stroke(
                    rect.expand(1.),
                    6,
                    Stroke::new(1., theme::line()),
                    egui::StrokeKind::Outside,
                );
                paint_preview(
                    ui,
                    rect,
                    preview.as_ref(),
                    signature.background == Background::Clear,
                );
                ui.label(
                    RichText::new("Signing adds the real date and time. You can resize the box when you place it.")
                        .size(11.5)
                        .color(theme::muted()),
                );
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if self.default_signature.as_ref() == Some(&id) {
                    let (r, _) = ui.allocate_exact_size(vec2(16., 16.), egui::Sense::hover());
                    ui::draw_icon(ui.painter(), r, Icon::Star, theme::warning());
                    ui.label(RichText::new("Used by Quick Sign").color(theme::ink()));
                } else {
                    make_default = ui
                        .add(ActionButton::new(Icon::Star, "Use for Quick Sign"))
                        .on_hover_text("Quick Sign signs with this signature in one step")
                        .clicked();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    remove = ui
                        .add(ActionButton::new(Icon::Trash, "Delete…").tint(theme::danger()))
                        .clicked();
                    duplicate = ui
                        .add(ActionButton::new(Icon::Copy, "Duplicate"))
                        .on_hover_text("Start a new signature from this one")
                        .clicked();
                });
            });
        });
        editor.text_editing |= typing;
        if signature != before {
            if let Some(slot) = self.signatures.iter_mut().find(|s| s.id == id) {
                *slot = signature;
            }
            self.manager.dirty = true;
        }
        if self.manager.dirty && !typing {
            self.commit_signatures();
        }
        if make_default {
            self.make_default(&id);
        }
        if duplicate {
            self.duplicate_signature(&id);
        }
        if remove {
            self.manager.confirm_delete = Some(id.clone());
        }
        self.confirm_delete(ui.ctx());
    }

    pub(super) fn save_if_dirty(&mut self) {
        if self.manager.dirty {
            self.commit_signatures();
        }
    }

    /// The first-run card: what a signature is, and one button to start.
    fn first_signature(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(theme::paper())
            .stroke(Stroke::new(1., theme::line()))
            .corner_radius(14)
            .inner_margin(24)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(56., 56.), egui::Sense::hover());
                    ui::badge(ui.painter(), r, Icon::Sign, ui::Fill::Brand, Color32::WHITE);
                    ui.add_space(10.);
                    ui::title(ui, "Create your signature", 17.);
                    ui.add_space(4.);
                    ui.label(
                        RichText::new("A signature joins your digital ID, which proves the signature is yours, with how it looks on the page: your name, the date and, if you like, your handwriting.")
                            .size(13.)
                            .color(theme::muted()),
                    );
                    if self.signing_entries().next().is_none() {
                        ui.add_space(4.);
                        ui.label(
                            RichText::new("No digital ID yet? MyPDF can create one for you in the next step.")
                                .size(12.)
                                .color(theme::muted()),
                        );
                    }
                    ui.add_space(14.);
                    if ui
                        .add(ActionButton::new(Icon::Plus, "Create a signature").primary())
                        .clicked()
                    {
                        self.add_signature(None);
                    }
                });
            });
    }

    /// "Signs with": the digital ID, or the ways to get one.
    fn signs_with(
        &mut self,
        ui: &mut egui::Ui,
        signature: &mut SavedSignature,
        action: &mut Option<Action>,
    ) {
        ui::field(ui, "Signs with", |ui| {
            let ids: Vec<(DefaultIdentity, String)> = self
                .signing_entries()
                .map(|e| {
                    (
                        e.reference(),
                        format!(
                            "{} · {}{}",
                            e.signer_name(),
                            place(e),
                            if e.purpose().other_purpose() {
                                format!(" · {}", e.purpose().label())
                            } else {
                                String::new()
                            }
                        ),
                    )
                })
                .collect();
            if ids.is_empty() {
                ui::callout(
                    ui,
                    ui::Tone::Info,
                    "You need a digital ID to sign. It proves that the signature is yours.",
                );
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add(ActionButton::new(Icon::Shield, "Create my digital ID…").primary())
                        .clicked()
                    {
                        *action = Some(Action::NewId(Some(signature.id.clone())));
                    }
                    if ui
                        .add(ActionButton::new(Icon::Import, "Use a .p12 / .pfx file…"))
                        .on_hover_text("A digital ID you received as a file")
                        .clicked()
                    {
                        *action = Some(Action::Import);
                    }
                });
                ui.label(
                    RichText::new("Using an e-signature smart card or USB token? Insert it and choose Refresh on the Digital IDs tab.")
                        .size(11.5)
                        .color(theme::muted()),
                );
                return;
            }
            let current = ids
                .iter()
                .find(|(r, _)| Some(r) == signature.identity.as_ref())
                .map(|(_, label)| label.clone());
            let missing = signature.identity.is_some() && current.is_none();
            egui::ComboBox::from_id_salt(("signs_with", signature.id.as_str()))
                .selected_text(current.unwrap_or_else(|| {
                    if missing {
                        "Unavailable digital ID".into()
                    } else {
                        "Choose a digital ID".into()
                    }
                }))
                .width(ui.available_width().min(460.))
                .show_ui(ui, |ui| {
                    for (reference, label) in ids {
                        if ui
                            .selectable_label(
                                signature.identity.as_ref() == Some(&reference),
                                label,
                            )
                            .clicked()
                        {
                            signature.identity = Some(reference);
                            ui.close();
                        }
                    }
                });
            if missing {
                ui::callout(
                    ui,
                    ui::Tone::Warning,
                    "The digital ID for this signature is not available. Insert its card or token, or choose another ID.",
                );
            } else if signature.identity.is_none() {
                ui::callout(ui, ui::Tone::Warning, "Choose the digital ID that signs.");
            }
            if let Some(entry) = self.signer_of(signature) {
                let (text, tone) = status(entry);
                if !matches!(tone, ui::Tone::Success) {
                    ui::callout(ui, tone, &text);
                }
                if entry.purpose().other_purpose() {
                    ui.label(
                        RichText::new(entry.purpose().explanation())
                            .small()
                            .color(theme::muted()),
                    );
                }
                if entry.info.self_issued {
                    ui.label(
                        RichText::new("This is a personal (self-signed) ID. People who receive your PDF see it as trusted after they trust your certificate.")
                            .size(11.5)
                            .color(theme::muted()),
                    );
                }
            }
        });
    }

    fn confirm_delete(&mut self, ctx: &egui::Context) {
        let Some(id) = self.manager.confirm_delete.clone() else {
            return;
        };
        let name = self
            .signature(&id)
            .map_or_else(String::new, |s| s.name.clone());
        match ui::confirmation(
            ctx,
            "delete_signature",
            &format!("Delete “{name}”?"),
            "Only this signature's look is removed. Your digital ID stays on this computer, and documents you signed keep their signatures.",
            &[
                ("Delete", ui::DialogTone::Destructive),
                ("Cancel", ui::DialogTone::Secondary),
            ],
            1,
        ) {
            Some(0) => {
                self.manager.confirm_delete = None;
                self.delete_signature(&id);
                self.message = format!("“{name}” deleted.");
            }
            Some(_) => self.manager.confirm_delete = None,
            None => {}
        }
    }

    /// Digital IDs grouped by where they live, with what to do next.
    pub(super) fn identities_panel(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut editing::EditorState,
        action: &mut Option<Action>,
    ) {
        ui.label(
            RichText::new("A digital ID proves who signed. It holds a certificate with your name and a private key that never leaves your computer, card or token.")
                .color(theme::muted()),
        );
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(ActionButton::new(Icon::Shield, "Create my digital ID…").primary())
                .clicked()
            {
                *action = Some(Action::NewId(None));
            }
            if ui
                .add(ActionButton::new(Icon::Import, "Add a .p12 / .pfx file…"))
                .clicked()
            {
                *action = Some(Action::Import);
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            if ui
                .add_enabled(!self.scanning, ActionButton::new(Icon::Reload, "Refresh"))
                .on_hover_text("Look again for digital IDs, cards and tokens")
                .clicked()
            {
                *action = Some(Action::Keychain);
            }
        });
        let quick = self.default_signature().and_then(|s| s.identity.clone());
        let system = |e: &&Entry| {
            matches!(
                e.source,
                Source::Keychain { .. } | Source::WindowsStore { .. }
            )
        };
        let groups: [(&str, Vec<Entry>, &str); 5] = [
            (
                if cfg!(target_os = "windows") {
                    "On this computer"
                } else {
                    "On this Mac (Keychain)"
                },
                self.entries
                    .iter()
                    .filter(|e| system(e) && !e.token && !e.purpose().other_purpose())
                    .cloned()
                    .collect(),
                "",
            ),
            (
                "Smart cards and USB tokens",
                self.entries
                    .iter()
                    .filter(|e| system(e) && e.token && !e.purpose().other_purpose())
                    .cloned()
                    .collect(),
                "Insert your e-signature card or token and choose Refresh. It appears here once its driver is installed.",
            ),
            (
                "Files",
                self.entries
                    .iter()
                    .filter(|e| {
                        matches!(e.source, Source::File { .. }) && !e.purpose().other_purpose()
                    })
                    .cloned()
                    .collect(),
                "",
            ),
            (
                "Other purposes · software signing and authentication",
                self.entries
                    .iter()
                    .filter(|e| {
                        !matches!(e.source, Source::Certificate) && e.purpose().other_purpose()
                    })
                    .cloned()
                    .collect(),
                "",
            ),
            (
                "Certificates for checking signatures",
                self.entries
                    .iter()
                    .filter(|e| matches!(e.source, Source::Certificate))
                    .cloned()
                    .collect(),
                "",
            ),
        ];
        if self.entries.is_empty() && !self.scanning {
            ui::callout(
                ui,
                ui::Tone::Info,
                "No digital IDs yet. Create one, add a file, or insert your smart card or token.",
            );
        }
        ui.spacing_mut().item_spacing.y = 6.;
        for (title, entries, empty) in groups {
            let tokens = title.starts_with("Smart");
            if entries.is_empty()
                && !(tokens && cfg!(any(target_os = "macos", target_os = "windows")))
            {
                continue;
            }
            ui.add_space(6.);
            ui::caption(ui, title);
            if entries.is_empty() {
                ui.label(RichText::new(empty).size(12.).color(theme::muted()));
            }
            for entry in &entries {
                let selected = self.selected.as_ref() == Some(&entry.id);
                let used = quick.as_ref() == Some(&entry.reference());
                match identity_row(ui, entry, selected, used) {
                    Some(RowClick::Select) => self.selected = Some(entry.id.clone()),
                    Some(RowClick::Forget) => *action = Some(Action::Forget(entry.id.clone())),
                    None => {}
                }
            }
        }
        ui.spacing_mut().item_spacing.y = 10.;
        if let Some(entry) = self
            .entries
            .iter()
            .find(|e| Some(&e.id) == self.selected.as_ref())
            .cloned()
        {
            ui::section(ui, &entry.signer_name(), |ui| {
                let (text, tone) = status(&entry);
                ui::callout(ui, tone, &format!("{} · {text}", place(&entry)));
                ui.label(RichText::new(format!("Purpose: {}", entry.purpose().label())).strong());
                ui.label(
                    RichText::new(entry.purpose().explanation())
                        .small()
                        .color(theme::muted()),
                );
                if let Some(explanation) = &entry.explanation {
                    ui.label(RichText::new(explanation).small().color(theme::muted()));
                }
                ui.horizontal_wrapped(|ui| {
                    if entry.eligible
                        && !matches!(entry.source, Source::Certificate)
                        && ui
                            .add(ActionButton::new(Icon::Plus, "Use in a new signature").primary())
                            .clicked()
                    {
                        *action = Some(Action::NewSignature(entry.reference()));
                    }
                    if ui
                        .add(ActionButton::new(Icon::Export, "Share certificate…"))
                        .on_hover_text("Save the public part (.cer) to send to people who check your signatures")
                        .clicked()
                    {
                        *action = Some(Action::ExportPublic);
                    }
                    if matches!(entry.source, Source::File { .. })
                        && ui
                            .add(ActionButton::new(Icon::Save, "Back up…"))
                            .on_hover_text("Save an encrypted copy with a new passphrase")
                            .clicked()
                    {
                        *action = Some(Action::ExportP12);
                    }
                });
                egui::CollapsingHeader::new("Trust")
                    .id_salt(("identity_trust", &entry.id))
                    .show(ui, |ui| {
                        trust_controls(ui, &mut self.system_trust, &self.trust, &entry.der, action);
                    });
                egui::CollapsingHeader::new("Certificate details")
                    .id_salt(("identity_details", &entry.id))
                    .show(ui, |ui| certificate_info(ui, &entry.info));
            });
        }
        egui::CollapsingHeader::new("More options")
            .id_salt("identity_more")
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add(ActionButton::new(
                            Icon::Save,
                            "Create a .p12 file with a passphrase…",
                        ))
                        .clicked()
                    {
                        *action = Some(Action::Create);
                    }
                    if ui
                        .add(ActionButton::new(
                            Icon::Open,
                            "Add a certificate (.cer / .crt / .pem)…",
                        ))
                        .on_hover_text("A public certificate checks signatures; it cannot sign")
                        .clicked()
                    {
                        *action = Some(Action::ReadCertificate);
                    }
                });
                #[cfg(target_os = "macos")]
                {
                    let r = ui::field(ui, "Other keychain file", |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.keychain_path)
                                .margin(ui::FIELD_MARGIN)
                                .desired_width(f32::INFINITY)
                                .hint_text("Empty uses your usual keychains"),
                        )
                    });
                    editor.text_editing |= r.has_focus();
                }
                if !self.store_message.is_empty() {
                    ui.label(
                        RichText::new(&self.store_message)
                            .size(11.5)
                            .color(theme::muted()),
                    );
                }
            });
        let _ = &editor;
    }

    /// The "Create my digital ID" dialog: a name, an optional email, done.
    pub(super) fn new_id_dialog(
        &mut self,
        ctx: &egui::Context,
        editor: &mut editing::EditorState,
    ) -> Option<Action> {
        let draft = self.manager.new_id.as_mut()?;
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("create_digital_id"))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(ui::dialog_width(ctx, 360.));
                ui::title(ui, "Create your digital ID", 16.);
                ui::dialog_body(ui, "create_identity_body", 420., |ui| {
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(54., 54.), egui::Sense::hover());
                    ui::badge(ui.painter(), r, Icon::Shield, ui::Fill::Brand, Color32::WHITE);
                    ui.add_space(12.);
                    ui.label(
                        RichText::new(if cfg!(target_os = "windows") {
                            "MyPDF creates a personal digital ID and keeps it in Windows. There is no password to remember."
                        } else {
                            "MyPDF creates a personal digital ID and keeps it in your Keychain. There is no password to remember."
                        })
                        .size(13.)
                        .color(theme::muted()),
                    );
                });
                ui.add_space(12.);
                let mut focus = false;
                focus |= ui::field(ui, "Your full name", |ui| {
                    let r = text_field(ui, &mut draft.name, "As it should appear on documents", 200);
                    if draft.name.is_empty() && !r.has_focus() && !ui.memory(|m| m.focused().is_some()) {
                        r.request_focus();
                    }
                    r
                })
                .has_focus();
                focus |= ui::field(ui, "Email (optional)", |ui| {
                    text_field(ui, &mut draft.email, "name@example.com", 254)
                })
                .has_focus();
                editor.text_editing |= focus;
                ui.checkbox(
                    &mut draft.trust,
                    if cfg!(target_os = "windows") {
                        "Trust it on this computer (Windows asks you to confirm)"
                    } else {
                        "Trust it on this Mac (macOS asks for your password)"
                    },
                );
                ui.label(
                    RichText::new("Other people's apps, such as Adobe Acrobat, show it as trusted once they trust your certificate. For legally qualified signatures, use the card or token from a certificate provider.")
                        .size(11.5)
                        .color(theme::muted()),
                );
                if let Some(error) = &self.error {
                    ui::callout(ui, ui::Tone::Danger, error);
                }
                });
                ui.add_space(12.);
                let half = (ui.available_width() - 10.) / 2.;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.;
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        if ui::dialog_button(ui, "Cancel", ui::DialogTone::Secondary).clicked() {
                            decision = Some(false);
                        }
                    });
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        if ui::dialog_button(ui, "Create", ui::DialogTone::Primary).clicked() {
                            decision = Some(true);
                        }
                    });
                });
            });
        if modal.should_close() {
            decision = decision.or(Some(false));
        }
        match decision {
            Some(true) if draft.name.trim().is_empty() => {
                self.error = Some("Enter your full name for the digital ID.".into());
                None
            }
            Some(true) => Some(Action::CreateInStore),
            Some(false) => {
                self.manager.new_id = None;
                None
            }
            None => None,
        }
    }
}

/// Create a personal ID in the system store; the passphrase only carries the
/// key into the store and is then forgotten.
pub(super) fn create_in_store(
    name: &str,
    email: &str,
    trust: bool,
    signature: Option<String>,
) -> anyhow::Result<Event> {
    let password = Zeroizing::new(format!("{:032x}", rand::random::<u128>()));
    let created = identity::create_personal(
        &PersonalId {
            name: name.trim().into(),
            organization: String::new(),
            email: email.trim().into(),
            validity_days: 3 * 365,
            ecdsa: false,
        },
        &password,
    )?;
    let info = identity::info(&created.certificate_der)?;
    platform_import(&created.pkcs12, &password)?;
    // The ID works without system trust; a declined password prompt is not a failure.
    let trusted = if trust {
        platform_trust(&created.certificate_der, true).map_err(|e| format!("{e:#}"))
    } else {
        Err(String::new())
    };
    #[cfg(target_os = "windows")]
    let source = Source::WindowsStore {
        fingerprint: info.sha256.clone(),
    };
    #[cfg(not(target_os = "windows"))]
    let source = Source::Keychain {
        path: None,
        fingerprint: info.sha256.clone(),
    };
    Ok(Event::Stored {
        entry: Entry {
            id: format!("{:032x}", rand::random::<u128>()),
            source,
            info,
            der: created.certificate_der,
            eligible: true,
            explanation: None,
            token: false,
        },
        signature,
        trusted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_identity_keeps_actions_visible_with_a_long_error() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.digital.manager.new_id = Some(NewId {
            signature: None,
            name: "Preview Person".into(),
            email: String::new(),
            trust: false,
        });
        app.digital.error = Some(
            "This example error explains why the digital ID could not be created. ".repeat(20),
        );
        let mut bounds = HashMap::new();
        for _ in 0..4 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(720., 480.))),
                    ..Default::default()
                },
                |ctx| {
                    app.digital.new_id_dialog(ctx, &mut app.editor);
                },
            );
            for (_, node) in output.platform_output.accesskit_update.unwrap().nodes {
                if let (Some(label), Some(rect)) = (node.label(), node.bounds()) {
                    bounds.insert(label.to_owned(), rect);
                }
            }
        }
        for label in ["Create", "Cancel"] {
            let rect = bounds
                .get(label)
                .unwrap_or_else(|| panic!("Missing {label}"));
            assert!(
                rect.x0 >= 0. && rect.y0 >= 0. && rect.x1 <= 720. && rect.y1 <= 480.,
                "{label} is outside the viewport: {rect:?}"
            );
        }
    }
}
