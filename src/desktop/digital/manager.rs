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
    pub transform: signing::AppearanceTransform,
    pub text_style: signing::SignatureTextStyle,
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
            transform: Default::default(),
            text_style: Default::default(),
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
    pub transform: signing::AppearanceTransform,
    pub text_style: signing::SignatureTextStyle,
}
impl Look {
    pub fn of(signature: &SavedSignature) -> Self {
        Self {
            display_name: signature.display_name.clone(),
            show_name: signature.show_name,
            show_date: signature.show_date,
            note: signature.note.clone(),
            background: signature.background,
            transform: signature.transform,
            text_style: signature.text_style.clone(),
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
        request.transform = self.transform;
        request.text_style = self.text_style.clone();
        if request.reason.trim().is_empty() {
            request.reason = self.note.trim().to_owned();
        }
    }
}

#[derive(Clone)]
pub(super) struct SignaturePreview {
    pub texture: TextureHandle,
    pub bounds: [f32; 4],
}
impl std::ops::Deref for SignaturePreview {
    type Target = TextureHandle;
    fn deref(&self) -> &Self::Target {
        &self.texture
    }
}
pub(super) type PreviewResult = Result<SignaturePreview, String>;

pub(super) struct ManagerState {
    pub selected: Option<String>,
    ink_selected: Option<String>,
    ink_name: String,
    ink_delete: bool,
    /// Rendered previews by what they show, shared by cards and editors.
    previews: HashMap<u64, PreviewResult>,
    pending: HashSet<u64>,
    queued: std::collections::VecDeque<(u64, DesktopSigningRequest, egui::Context)>,
    active: usize,
    last_previews: HashMap<String, SignaturePreview>,
    slot_keys: HashMap<String, u64>,
    tx: mpsc::Sender<(u64, Result<signing::SignatureAppearance, String>)>,
    rx: mpsc::Receiver<(u64, Result<signing::SignatureAppearance, String>)>,
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
            ink_selected: None,
            ink_name: String::new(),
            ink_delete: false,
            previews: HashMap::new(),
            pending: HashSet::new(),
            queued: Default::default(),
            active: 0,
            last_previews: HashMap::new(),
            slot_keys: HashMap::new(),
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
            self.active = self.active.saturating_sub(1);
            if self.previews.len() > 96 {
                self.previews.clear();
            }
            let texture = result.and_then(|appearance| {
                let image = mypdf_milestone0::assets::decode_png(&appearance.png)
                    .map_err(|e| format!("{e:#}"))?
                    .into_rgba8();
                Ok(SignaturePreview {
                    bounds: appearance.bounds,
                    texture: ctx.load_texture(
                        format!("signature_preview_{key:016x}"),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [image.width() as usize, image.height() as usize],
                            &image,
                        ),
                        egui::TextureOptions::LINEAR,
                    ),
                })
            });
            self.previews.insert(key, texture);
        }
        self.start_previews();
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
            if self.queued.len() >= 96
                && let Some((old, _, _)) = self.queued.pop_front()
            {
                self.pending.remove(&old);
            }
            self.queued.push_back((key, request.clone(), ctx.clone()));
            self.start_previews();
        }
        None
    }
    fn start_previews(&mut self) {
        while self.active < 2 {
            let Some((key, request, wake)) = self.queued.pop_front() else {
                break;
            };
            self.active += 1;
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(|| signing::signature_appearance(&request))
                    .map_err(|_| "The preview could not be drawn.".to_string())
                    .and_then(|r| r.map_err(|e| format!("{e:#}")));
                let _ = tx.send((key, result));
                wake.request_repaint();
            });
        }
    }
    pub fn preview_for(
        &mut self,
        ctx: &egui::Context,
        slot: &str,
        request: &DesktopSigningRequest,
    ) -> Option<PreviewResult> {
        let key = request_key(request);
        if let Some(old) = self.slot_keys.insert(slot.into(), key)
            && old != key
            && let Some(i) = self.queued.iter().position(|(k, _, _)| *k == old)
        {
            self.queued.remove(i);
            self.pending.remove(&old);
        }
        if self.slot_keys.len() > 128 {
            self.slot_keys.retain(|k, _| k == slot);
            self.last_previews.retain(|k, _| k == slot);
        }
        match self.preview(ctx, request) {
            Some(Ok(texture)) => {
                self.last_previews.insert(slot.into(), texture.clone());
                Some(Ok(texture))
            }
            Some(Err(error)) => Some(Err(error)),
            None => self.last_previews.get(slot).cloned().map(Ok),
        }
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
    serde_json::to_string(&request.text_style)
        .unwrap_or_default()
        .hash(&mut h);
    serde_json::to_string(&request.transform)
        .unwrap_or_default()
        .hash(&mut h);
    format!("{:?}{:?}", request.background, request.layout).hash(&mut h);
    request
        .artwork
        .as_ref()
        .map(|a| (&a.id, a.aspect.to_bits()))
        .hash(&mut h);
    request
        .rect
        .map(|r| [(r[2] - r[0]).to_bits(), (r[3] - r[1]).to_bits()])
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

struct IdentityChoice {
    reference: DefaultIdentity,
    title: String,
    detail: String,
    tooltip: String,
}
impl IdentityChoice {
    fn new(entry: &Entry) -> Self {
        let validity = expiry(&entry.der).map_or_else(
            || format!("Valid until {}", entry.info.not_after),
            |(days, date)| {
                format!(
                    "{} {date}",
                    if days < 0 { "Expired" } else { "Valid until" }
                )
            },
        );
        let purpose = if entry.purpose().other_purpose() {
            format!(" · {}", entry.purpose().label())
        } else {
            String::new()
        };
        let source_detail = match &entry.source {
            Source::File { path, .. } => format!("\nFile: {}", path.display()),
            Source::Keychain {
                path: Some(path), ..
            } => format!("\nKeychain: {}", path.display()),
            _ => String::new(),
        };
        Self {
            reference: entry.reference(),
            title: entry.selection_name(),
            detail: format!("{} · {validity}{purpose}", place(entry)),
            tooltip: format!("{}{source_detail}", entry.label()),
        }
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

/// Explicit spacing overrides the legacy layout only when the user changes it.
fn spacing_control(ui: &mut egui::Ui, label: &str, value: &mut Option<f32>, max: f32) {
    ui.push_id(label, |ui| {
        ui.horizontal(|ui| {
            ui.add_sized(vec2(110., 24.), egui::Label::new(label));
            ui.spacing_mut().slider_width = (ui.available_width() - 90.).clamp(30., 100.);
            let mut points = value.unwrap_or(8.);
            if ui
                .add(egui::Slider::new(&mut points, 0. ..=max).show_value(false))
                .on_hover_text(format!("{label} in PDF points; 0 removes this spacing"))
                .changed()
            {
                *value = Some(points);
            }
            egui::ComboBox::from_id_salt("presets")
                .width(76.)
                .selected_text(value.map_or_else(|| "Automatic".into(), |v| format!("{v:.1} pt")))
                .show_ui(ui, |ui| {
                    ui.selectable_value(value, None, "Automatic (original layout)");
                    for v in [0., 2., 4., 6., 8., 12., 16., 24., 32., 48.] {
                        if v <= max {
                            ui.selectable_value(value, Some(v), format!("{v} pt"));
                        }
                    }
                    ui.separator();
                    if ui
                        .add(
                            egui::DragValue::new(&mut points)
                                .range(0. ..=max)
                                .speed(0.25)
                                .suffix(" pt"),
                        )
                        .changed()
                    {
                        *value = Some(points);
                    }
                });
        });
    });
}

/// Four independent inside edges, stored as percentages so resizing keeps the composition.
fn edge_spacing(ui: &mut egui::Ui, id: &str, edges: &mut [f32; 4]) {
    ui.push_id(id, |ui| {
        let slider_width = ((ui.available_width() - 12.) / 2. - 120.).clamp(24., 64.);
        egui::Grid::new("edges")
            .num_columns(2)
            .spacing(vec2(12., 4.))
            .show(ui, |ui| {
                for (index, label) in [(0, "Left"), (2, "Right"), (1, "Top"), (3, "Bottom")] {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.;
                        ui.spacing_mut().slider_width = slider_width;
                        ui.add_sized(vec2(42., 24.), egui::Label::new(label));
                        ui.add(egui::Slider::new(&mut edges[index], 0. ..=45.).show_value(false))
                            .on_hover_text(format!("{label} edge spacing"));
                        egui::ComboBox::from_id_salt(index)
                            .width(56.)
                            .selected_text(format!("{:.1}%", edges[index]))
                            .show_ui(ui, |ui| {
                                for value in [0., 2., 5., 10., 15., 20., 25., 30., 40., 45.] {
                                    ui.selectable_value(
                                        &mut edges[index],
                                        value,
                                        format!("{value}%"),
                                    );
                                }
                                ui.separator();
                                ui.horizontal(|ui| {
                                    ui.label("Exact");
                                    ui.add(
                                        egui::DragValue::new(&mut edges[index])
                                            .range(0. ..=45.)
                                            .speed(0.25)
                                            .suffix("%"),
                                    );
                                });
                            });
                    });
                    if index == 2 || index == 3 {
                        ui.end_row();
                    }
                }
            });
    });
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
fn background_tile(ui: &mut egui::Ui, background: Background, selected: bool) -> egui::Response {
    let custom = matches!(background, Background::Tinted | Background::Custom { .. });
    let label = if custom {
        "Custom color"
    } else {
        background.label()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(92., 70.), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label)
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
    if custom {
        ui::paint_color_picker(
            ui,
            Rect::from_center_size(pos2(rect.right() - 13., rect.top() + 13.), vec2(22., 22.)),
            rgb(background.colors().unwrap().0),
        );
    }
    ui.painter().text(
        pos2(rect.center().x, rect.bottom() - 13.),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(11.5),
        if selected {
            theme::accent()
        } else {
            theme::ink()
        },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn background_choices(ui: &mut egui::Ui, current: &mut Background) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8., 8.);
        for preset in Background::CHOICES {
            let background =
                if preset == Background::Tinted && matches!(current, Background::Custom { .. }) {
                    *current
                } else {
                    preset
                };
            let response = background_tile(ui, background, *current == background);
            if response.clicked() {
                *current = background;
                ui::reset_color_picker(ui, response.id.with("fill_hex"));
                ui::reset_color_picker(ui, response.id.with("frame_hex"));
            }
            if preset == Background::Tinted {
                egui::Popup::from_toggle_button_response(&response)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        let (fill, frame) = current.colors().unwrap_or(([1.; 3], None));
                        let bytes = |rgb: [f32; 3]| rgb.map(|c| (c * 255.).round() as u8);
                        let channel = response.id.with("frame_channel");
                        let mut edit_frame =
                            ui.data(|d| d.get_temp::<bool>(channel)).unwrap_or(false);
                        let mut framed = frame.is_some();
                        let mut changed = false;
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut edit_frame, false, "Fill");
                            if ui
                                .selectable_value(&mut edit_frame, true, "Frame")
                                .clicked()
                                && !framed
                            {
                                framed = true;
                                changed = true;
                            }
                        });
                        let mut fill = bytes(fill);
                        let mut frame_color = bytes(frame.unwrap_or([0.11, 0.43, 0.35]));
                        let [r, g, b] = if edit_frame { frame_color } else { fill };
                        let mut color = Color32::from_rgb(r, g, b);
                        if ui::color_picker_contents(
                            ui,
                            &mut color,
                            if edit_frame {
                                "Frame color"
                            } else {
                                "Background color"
                            },
                            response
                                .id
                                .with(if edit_frame { "frame_hex" } else { "fill_hex" }),
                        ) {
                            let bytes = [color.r(), color.g(), color.b()];
                            if edit_frame {
                                frame_color = bytes;
                            } else {
                                fill = bytes;
                            }
                            changed = true;
                        }
                        if ui.checkbox(&mut framed, "Show frame").changed() {
                            changed = true;
                        }
                        ui.data_mut(|d| d.insert_temp(channel, edit_frame));
                        if changed {
                            *current = Background::Custom {
                                fill,
                                frame: framed.then_some(frame_color),
                            };
                        }
                    });
            }
        }
    });
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
        if self
            .manager
            .selected
            .as_ref()
            .is_none_or(|selected| self.signature(selected).is_none())
        {
            self.manager.selected = self
                .default_signature
                .clone()
                .filter(|selected| self.signature(selected).is_some())
                .or_else(|| self.signatures.first().map(|s| s.id.clone()));
        }
        self.manager.confirm_delete = None;
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

    /// Outside the scrolling form, so adjusting lower controls never hides the result.
    pub(super) fn pinned_preview(&mut self, ui: &mut egui::Ui, assets: &[Asset], placed: bool) {
        let signature = self
            .manager
            .selected
            .as_ref()
            .and_then(|id| self.signature(id))
            .or_else(|| self.default_signature())
            .or_else(|| self.signatures.first())
            .cloned();
        let Some(signature) = signature else {
            return;
        };
        let mut request = self.signature_request(&signature, assets);
        if placed {
            request.rect = self.request.rect;
            if !self.request.reason.trim().is_empty() {
                request.reason = self.request.reason.clone();
            }
            request.location = self.request.location.clone();
        }
        let preview = self
            .manager
            .preview_for(ui.ctx(), "pinned_signature", &request);
        ui.label(
            RichText::new("Live signature preview")
                .size(12.)
                .color(theme::muted()),
        );
        let height = (ui.ctx().content_rect().height() * 0.12).clamp(56., 96.);
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("signature_pinned_preview_rect"), rect)
        });
        paint_preview(
            ui,
            rect,
            preview.as_ref(),
            signature.background == Background::Clear,
        );
        if let Some(Err(error)) = preview {
            response.on_hover_text(error);
        }
        ui.separator();
    }

    /// My signatures: cards for each saved signature, then the editor.
    pub(super) fn signatures_panel(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut editing::EditorState,
        assets: &[Asset],
        action: &mut Option<Action>,
        compact: bool,
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
        let ctx = ui.ctx().clone();
        if compact {
            let selected_name = self
                .manager
                .selected
                .as_ref()
                .and_then(|id| self.signature(id))
                .map(|s| s.name.as_str())
                .unwrap_or("Signature");
            egui::ComboBox::from_id_salt("placed_signature_choice")
                .selected_text(selected_name)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for signature in self.signatures.clone() {
                        if ui
                            .selectable_label(
                                self.manager.selected.as_ref() == Some(&signature.id),
                                &signature.name,
                            )
                            .clicked()
                        {
                            self.save_if_dirty();
                            self.manager.selected = Some(signature.id);
                        }
                    }
                });
        } else {
            ui.label(
            RichText::new("A signature is your digital ID plus how it looks on the page. The one with a star is used by Quick Sign.")
                .color(theme::muted()),
        );
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(10., 10.);
                for signature in self.signatures.clone() {
                    let request = self.signature_request(&signature, assets);
                    let preview =
                        self.manager
                            .preview_for(&ctx, &format!("card:{}", signature.id), &request);
                    let selected = self.manager.selected.as_ref() == Some(&signature.id);
                    let default = self.default_signature.as_ref() == Some(&signature.id);
                    let ready =
                        self.signer_of(&signature).is_some() && signature.layout().is_some();
                    if signature_card(ui, &signature, preview.as_ref(), default, selected, ready)
                        .clicked()
                    {
                        self.save_if_dirty();
                        self.manager.selected = Some(signature.id.clone());
                    }
                }
                if new_card(ui, vec2(148., 104.), "New signature")
                    .on_hover_text(
                        "Create another signature, for example initials or a company stamp",
                    )
                    .clicked()
                {
                    self.save_if_dirty();
                    self.add_signature(None);
                }
            });
        }
        let Some(id) = self.manager.selected.clone() else {
            return;
        };
        let Some(before) = self.signature(&id).cloned() else {
            return;
        };
        let mut signature = before.clone();
        let mut typing = false;
        let mut make_default = false;
        ui::section(
            ui,
            if compact {
                "Appearance"
            } else {
                "Edit signature"
            },
            |ui| {
                ui.spacing_mut().item_spacing.y = 6.;
                ui.spacing_mut().slider_width = 120.;
                egui::CollapsingHeader::new("Saved signature details")
                    .default_open(!compact)
                    .show(ui, |ui| {
                        ui::field(ui, "Name", |ui| {
                            ui.horizontal(|ui| {
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut signature.name)
                                        .desired_width((ui.available_width() - 110.).max(80.))
                                        .hint_text("For example: Work, Initials, Approval")
                                        .char_limit(80),
                                );
                                typing |= response.has_focus();
                                if ui
                                    .add_enabled(
                                        !signature.name.trim().is_empty(),
                                        ActionButton::new(Icon::Pen, "Rename"),
                                    )
                                    .clicked()
                                {
                                    signature.name = signature.name.trim().to_owned();
                                    response.surrender_focus();
                                    typing = false;
                                }
                            });
                        });
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            if self.default_signature.as_ref() == Some(&id) {
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(16., 16.), egui::Sense::hover());
                                ui::draw_icon(ui.painter(), r, Icon::Star, theme::warning());
                                ui.label(RichText::new("Used by Quick Sign").color(theme::ink()));
                            } else {
                                make_default = ui
                                    .add(ActionButton::new(Icon::Star, "Use for Quick Sign"))
                                    .on_hover_text(
                                        "Quick Sign signs with this signature in one step",
                                    )
                                    .clicked();
                            }
                        });

                        self.signs_with(ui, &mut signature, action);
                    });
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
                            typing |=
                                text_field(ui, &mut signature.display_name, &certificate, 200)
                                    .on_hover_text("Leave empty to use the name in your digital ID")
                                    .has_focus();
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Note").color(theme::muted()));
                        typing |= text_field(
                            ui,
                            &mut signature.note,
                            "Optional, such as “Approved”",
                            300,
                        )
                        .has_focus();
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut signature.transform.fit_to_content, "Fit box to content")
                        .on_hover_text("Trim empty outer space without stretching the image or text. The page preview and saved signature use the same fitted box.");
                });
                spacing_control(
                    ui,
                    "Outer padding",
                    &mut signature.transform.outer_padding,
                    32.,
                );
                ui.add_enabled_ui(
                    signature.asset_id.is_some()
                        && !signature.transform.image_overlay
                        && (signature.show_name
                            || signature.show_date
                            || !signature.note.trim().is_empty()),
                    |ui| {
                        spacing_control(
                            ui,
                            "Image–text gap",
                            &mut signature.transform.image_text_gap,
                            48.,
                        );
                    },
                )
                .response
                .on_hover_text("The gap applies when the image is beside the details.");
                egui::CollapsingHeader::new("Text appearance & position")
                    .default_open(compact)
                    .show(ui, |ui| {
                        let style = &mut signature.text_style;
                        ui.spacing_mut().item_spacing = vec2(6., 4.);
                        ui.spacing_mut().interact_size.y = 24.;
                        ui.spacing_mut().button_padding.y = 3.;
                        ui.horizontal_wrapped(|ui| {
                            egui::ComboBox::from_id_salt("signature_text_family")
                                .width(130.)
                                .selected_text(if style.family.is_empty() {
                                    "Default font"
                                } else {
                                    &style.family
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut style.family,
                                        String::new(),
                                        "Default font",
                                    );
                                    let mut families = std::collections::BTreeSet::new();
                                    for font in mypdf_milestone0::text_style::fonts() {
                                        if families.insert(&font.family) {
                                            ui.selectable_value(
                                                &mut style.family,
                                                font.family.clone(),
                                                &font.family,
                                            );
                                        }
                                    }
                                });
                            ui.add(
                                egui::DragValue::new(&mut style.size)
                                    .range(6. ..=36.)
                                    .speed(0.25)
                                    .suffix(" pt"),
                            )
                            .on_hover_text("Font size: drag to adjust or click to type");
                            ui.menu_button("▾", |ui| {
                                for size in
                                    [6., 8., 9., 10., 11., 12., 14., 16., 18., 24., 30., 36.]
                                {
                                    if ui
                                        .selectable_label(style.size == size, format!("{size} pt"))
                                        .clicked()
                                    {
                                        style.size = size;
                                        ui.close();
                                    }
                                }
                            })
                            .response
                            .on_hover_text("Common font sizes");
                            ui::color_swatch(ui, &mut style.color, "Signature text color");
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.toggle_value(&mut style.bold, RichText::new("B").strong())
                                .on_hover_text("Bold");
                            ui.toggle_value(&mut style.italic, RichText::new("I").italics())
                                .on_hover_text("Italic");
                            ui.separator();
                            for (index, label) in ["Left", "Center", "Right"].iter().enumerate() {
                                ui.selectable_value(&mut style.alignment, index as u8, *label);
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().slider_width = 64.;
                            ui.label("X").on_hover_text("Horizontal position");
                            ui.add(
                                egui::Slider::new(&mut style.offset[0], -100. ..=100.).suffix("%"),
                            )
                            .on_hover_text("Horizontal position within the signature box");
                            ui.label("Y").on_hover_text("Vertical position");
                            ui.add(
                                egui::Slider::new(&mut style.offset[1], -100. ..=100.).suffix("%"),
                            )
                            .on_hover_text("Vertical position within the signature box");
                        });
                        ui.label("Text edge spacing");
                        edge_spacing(ui, "text_edges", &mut style.insets);
                    });
                egui::CollapsingHeader::new("Image artwork").default_open(!compact).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8., 8.);
                    if no_image_card(ui, signature.asset_id.is_none()) {
                        signature.asset_id = None;
                    }
                    // General library artwork is only offered when explicitly requested.
                    for asset in assets.iter().rev().filter(|a| self.is_signature_image(a)) {
                        let selected = signature.asset_id.as_ref() == Some(&asset.id);
                        if artwork_card(ui, editor, asset, selected) {
                            signature.asset_id = Some(asset.id.clone());
                        }
                    }
                });
                ui.weak("Saved signature artwork. Choose from library to use another image, stamp or drawing.");
                ui.horizontal_wrapped(|ui| {
                    ui.menu_button("Choose from library…", |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(240.)
                            .show(ui, |ui| {
                                for asset in assets {
                                    if ui
                                        .selectable_label(
                                            signature.asset_id.as_ref() == Some(&asset.id),
                                            &asset.name,
                                        )
                                        .clicked()
                                    {
                                        signature.asset_id = Some(asset.id.clone());
                                        ui.close();
                                    }
                                }
                                if assets.is_empty() {
                                    ui.weak("Your library is empty.");
                                }
                            });
                    });
                    if let Some(asset) = assets
                        .iter()
                        .find(|a| signature.asset_id.as_ref() == Some(&a.id))
                        && ui
                            .add(ActionButton::new(Icon::Settings, "Manage image…"))
                            .on_hover_text("Rename or delete this saved image in Ink signatures")
                            .clicked()
                    {
                        self.manage_signature_image(asset);
                    }
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
                if signature.asset_id.is_some() {
                    ui::field(ui, "Image layout", |ui| {
                        ui.add_enabled_ui(
                            signature.show_name
                                || signature.show_date
                                || !signature.note.trim().is_empty(),
                            |ui| {
                                let selected = usize::from(signature.transform.image_overlay);
                                if let Some(index) =
                                    ui::segmented(ui, &["Beside details", "Over details"], selected)
                                {
                                    signature.transform.image_overlay = index == 1;
                                }
                            },
                        );
                        ui.add(
                            egui::Slider::new(&mut signature.transform.image_opacity, 0.0..=1.0)
                                .text("Image opacity")
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.))
                                .custom_parser(|s| {
                                    s.trim()
                                        .trim_end_matches('%')
                                        .trim()
                                        .parse::<f64>()
                                        .ok()
                                        .map(|v| v / 100.)
                                }),
                        );
                    });
                }
                ui.add_enabled_ui(signature.asset_id.is_some(), |ui| {
                    ui.collapsing("Image rotation & edge spacing", |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add(ActionButton::new(Icon::RotateLeft, "−30°"))
                                .clicked()
                            {
                                signature.transform.rotation =
                                    (signature.transform.rotation + 150.).rem_euclid(360.) - 180.;
                            }
                            if ui
                                .add(ActionButton::new(Icon::RotateRight, "+30°"))
                                .clicked()
                            {
                                signature.transform.rotation =
                                    (signature.transform.rotation + 210.).rem_euclid(360.) - 180.;
                            }
                            ui.add(
                                egui::Slider::new(
                                    &mut signature.transform.rotation,
                                    -180.0..=180.0,
                                )
                                .show_value(false),
                            );
                            egui::ComboBox::from_id_salt("image_rotation_presets")
                                .width(66.)
                                .selected_text(format!("{:.0}°", signature.transform.rotation))
                                .show_ui(ui, |ui| {
                                    for angle in [
                                        -180., -90., -45., -30., -15., 0., 15., 30., 45., 90., 180.,
                                    ] {
                                        ui.selectable_value(
                                            &mut signature.transform.rotation,
                                            angle,
                                            format!("{angle}°"),
                                        );
                                    }
                                    ui.separator();
                                    ui.add(
                                        egui::DragValue::new(&mut signature.transform.rotation)
                                            .range(-180. ..=180.)
                                            .suffix("°"),
                                    );
                                });
                        });
                        edge_spacing(ui, "image_edges", &mut signature.transform.image_insets);
                    });
                });
                ui::field(ui, "Background", |ui| {
                    background_choices(ui, &mut signature.background);
                });
            },
        );
        editor.text_editing |= typing;
        if signature != before {
            if let Some(slot) = self.signatures.iter_mut().find(|s| s.id == id) {
                *slot = signature;
            }
            self.manager.dirty = true;
        }
        if self.manager.dirty && !typing && !ui.input(|i| i.pointer.any_down()) {
            self.commit_signatures();
        }
        if make_default {
            self.make_default(&id);
        }
    }

    pub(super) fn signature_actions(&mut self, ui: &mut egui::Ui) {
        let selected = self
            .manager
            .selected
            .clone()
            .filter(|id| self.signature(id).is_some())
            .or_else(|| self.signatures.first().map(|s| s.id.clone()));
        if let Some(id) = selected {
            if ui
                .add(ActionButton::new(Icon::Trash, "Delete…").tint(theme::danger()))
                .clicked()
            {
                self.manager.confirm_delete = Some(id.clone());
            }
            if ui.add(ActionButton::new(Icon::Copy, "Duplicate")).clicked() {
                self.save_if_dirty();
                self.duplicate_signature(&id);
            }
        }
    }

    pub(in crate::desktop) fn save_if_dirty(&mut self) {
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
            let ids: Vec<_> = self.signing_entries().map(IdentityChoice::new).collect();
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
                .find(|choice| Some(&choice.reference) == signature.identity.as_ref())
                .map(|choice| choice.title.clone());
            let missing = signature.identity.is_some() && current.is_none();
            let width = ui.available_width().min(460.);
            egui::ComboBox::from_id_salt(("signs_with", signature.id.as_str()))
                .selected_text(current.unwrap_or_else(|| {
                    if missing {
                        "Unavailable digital ID".into()
                    } else {
                        "Choose a digital ID".into()
                    }
                }))
                .width(width)
                .wrap_mode(egui::TextWrapMode::Truncate)
                .show_ui(ui, |ui| {
                    ui.set_width(width);
                    for choice in ids {
                        let mut text = egui::text::LayoutJob::default();
                        text.append(
                            &choice.title,
                            0.,
                            egui::TextFormat {
                                font_id: ui::strong(ui, 13.),
                                color: theme::ink(),
                                ..Default::default()
                            },
                        );
                        text.append(
                            &format!("\n{}", choice.detail),
                            0.,
                            egui::TextFormat {
                                font_id: FontId::proportional(11.5),
                                color: theme::muted(),
                                ..Default::default()
                            },
                        );
                        if ui
                            .add_sized(
                                vec2(width, 0.),
                                egui::Button::new(text)
                                    .selected(
                                        signature.identity.as_ref() == Some(&choice.reference),
                                    )
                                    .wrap(),
                            )
                            .on_hover_text(&choice.tooltip)
                            .clicked()
                        {
                            signature.identity = Some(choice.reference);
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
                let choice = IdentityChoice::new(entry);
                ui.add(
                    egui::Label::new(RichText::new(choice.detail).small().color(theme::muted()))
                        .wrap(),
                )
                .on_hover_text(choice.tooltip);
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

    pub(super) fn confirm_delete(&mut self, ctx: &egui::Context) {
        let Some(id) = self.manager.confirm_delete.clone() else {
            return;
        };
        let Some(signature) = self.signature(&id) else {
            self.manager.confirm_delete = None;
            return;
        };
        let name = signature.name.clone();
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

impl DigitalState {
    fn is_signature_image(&self, asset: &Asset) -> bool {
        asset.signature
            || self
                .signatures
                .iter()
                .any(|s| s.asset_id.as_deref() == Some(&asset.id))
    }
    fn manage_signature_image(&mut self, asset: &Asset) {
        self.manager.ink_selected = Some(asset.id.clone());
        self.manager.ink_name = asset.name.clone();
        self.manager.ink_delete = false;
        self.panel = Panel::Ink;
    }
    pub(in crate::desktop) fn ink_assets(&self, assets: &[Asset]) -> Vec<Asset> {
        let mut choices: Vec<_> = assets
            .iter()
            .filter(|a| {
                a.signature
                    || matches!(a.kind, mypdf_milestone0::annotations::Kind::Ink { .. })
                    || a.name.to_lowercase().contains("signature")
                    || self
                        .signatures
                        .iter()
                        .any(|s| s.asset_id.as_deref() == Some(&a.id))
            })
            .cloned()
            .collect();
        choices.sort_by_key(|a| self.default_ink.as_deref() != Some(&a.id));
        choices
    }
    pub(in crate::desktop) fn ink_is_default(&self, id: &str) -> bool {
        self.default_ink.as_deref() == Some(id)
    }

    fn delete_signature_image(
        &mut self,
        editor: &mut editing::EditorState,
        id: &str,
    ) -> Result<(), String> {
        let library = editor.library.as_mut().ok_or("Library is unavailable")?;
        let i = library
            .entries
            .iter()
            .position(|a| a.id == id)
            .ok_or("The image is no longer in the library")?;
        let removed = library.entries.remove(i);
        if let Err(error) = library.save() {
            library.entries.insert(i, removed);
            return Err(format!("Could not delete image: {error:#}"));
        }
        let previous = (
            self.signatures.clone(),
            self.default_ink.clone(),
            self.asset_id.clone(),
            self.layout,
        );
        for signature in &mut self.signatures {
            if signature.asset_id.as_deref() == Some(id) {
                signature.asset_id = None;
            }
        }
        if self.default_ink.as_deref() == Some(id) {
            self.default_ink = None;
        }
        if self.asset_id.as_deref() == Some(id) {
            self.asset_id = None;
            self.layout = AppearanceLayout::DetailsOnly;
        }
        if let Err(error) = self.save_catalogue() {
            (
                self.signatures,
                self.default_ink,
                self.asset_id,
                self.layout,
            ) = previous;
            library.entries.insert(i, removed);
            return match library.save() {
                Ok(()) => Err(format!(
                    "Could not update signature designs; image deletion was canceled: {error:#}"
                )),
                Err(restore) => Err(format!(
                    "Could not save signature designs ({error:#}) or restore the image library ({restore:#}). The image is still available in this session."
                )),
            };
        }
        editor.removed_asset = Some(removed);
        self.manager.ink_selected = None;
        Ok(())
    }

    pub(super) fn confirm_ink_delete(
        &mut self,
        ctx: &egui::Context,
        editor: &mut editing::EditorState,
    ) {
        if !self.manager.ink_delete {
            return;
        }
        let asset = editor
            .library
            .as_ref()
            .and_then(|l| {
                l.entries
                    .iter()
                    .find(|a| Some(&a.id) == self.manager.ink_selected.as_ref())
            })
            .cloned();
        let Some(asset) = asset else {
            self.manager.ink_delete = false;
            return;
        };
        let decision = ui::confirmation(
            ctx,
            "delete_ink_signature",
            &format!("Delete “{}”?", asset.name),
            "This removes the image from your library and from saved signature designs that use it. Copies already placed in PDFs stay unchanged.",
            &[
                ("Delete", ui::DialogTone::Destructive),
                ("Cancel", ui::DialogTone::Secondary),
            ],
            1,
        );
        if let Some(choice) = decision {
            self.manager.ink_delete = false;
            if choice == 0
                && let Err(error) = self.delete_signature_image(editor, &asset.id)
            {
                self.error = Some(error);
            }
        }
    }

    pub(super) fn ink_panel(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut editing::EditorState,
        assets: &[Asset],
        can_place: bool,
    ) -> Option<Asset> {
        ui.heading("Ink signatures");
        ui.weak("Drawn, typed or image signatures. These are visual marks, without a digital certificate.");
        let choices = self.ink_assets(assets);
        if self
            .manager
            .ink_selected
            .as_ref()
            .is_none_or(|id| !choices.iter().any(|a| &a.id == id))
        {
            self.manager.ink_selected = choices.first().map(|a| a.id.clone());
            self.manager.ink_name = choices.first().map(|a| a.name.clone()).unwrap_or_default();
            self.manager.ink_delete = false;
        }
        ui.horizontal_wrapped(|ui| {
            for asset in &choices {
                ui.vertical(|ui| {
                    if artwork_card(
                        ui,
                        editor,
                        asset,
                        self.manager.ink_selected.as_ref() == Some(&asset.id),
                    ) {
                        self.manager.ink_selected = Some(asset.id.clone());
                        self.manager.ink_name = asset.name.clone();
                        self.manager.ink_delete = false;
                    }
                    if self.ink_is_default(&asset.id) {
                        ui.weak("★ Default");
                    }
                });
            }
        });
        let mut place = None;
        if let Some(asset) = choices
            .iter()
            .find(|a| Some(&a.id) == self.manager.ink_selected.as_ref())
        {
            ui.separator();
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.manager.ink_name)
                        .hint_text("Signature name")
                        .char_limit(200),
                );
                editor.text_editing |= response.has_focus();
                if ui
                    .add_enabled(
                        !self.manager.ink_name.trim().is_empty(),
                        ActionButton::new(Icon::Save, "Rename"),
                    )
                    .clicked()
                {
                    let mut updated = asset.clone();
                    updated.name = self.manager.ink_name.trim().into();
                    updated.signature = true;
                    if let Err(e) = editor.add_asset(updated) {
                        self.error = Some(e);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        can_place,
                        ActionButton::new(Icon::Sign, "Place signature").primary(),
                    )
                    .clicked()
                {
                    place = Some(asset.clone());
                }
                if ui
                    .add_enabled(
                        !self.ink_is_default(&asset.id),
                        ActionButton::new(Icon::Star, "Make default"),
                    )
                    .clicked()
                {
                    self.default_ink = Some(asset.id.clone());
                    if let Err(e) = self.save_catalogue() {
                        self.error = Some(format!("{e:#}"));
                    }
                }
                if ui
                    .add(ActionButton::new(Icon::Trash, "Delete…").tint(theme::danger()))
                    .clicked()
                {
                    self.manager.ink_delete = true;
                }
            });
        } else {
            ui.weak(
                "Create your first signature below, or choose artwork already in your library.",
            );
        }
        ui.separator();
        if ui
            .add(ActionButton::new(Icon::Plus, "New ink signature…"))
            .clicked()
        {
            self.adopt_asset = false;
            editor.open_signature();
            editor.signature_manage_only = true;
        }
        ui.menu_button("Use existing artwork…", |ui| {
            for asset in assets {
                if ui.button(&asset.name).clicked() {
                    let mut asset = asset.clone();
                    asset.signature = true;
                    self.manager.ink_selected = Some(asset.id.clone());
                    self.manager.ink_name = asset.name.clone();
                    if let Err(e) = editor.add_asset(asset) {
                        self.error = Some(e);
                    }
                    ui.close();
                }
            }
        });
        place
    }
}

#[cfg(test)]
mod image_management_tests {
    use super::*;
    use mypdf_milestone0::annotations::{Kind, Style};

    #[test]
    fn image_choices_exclude_general_artwork_and_management_clears_references() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = DigitalState::at(temp.path().join("profile"));
        let mut editor = editing::EditorState::new();
        editor.library =
            Some(mypdf_milestone0::assets::Library::open(temp.path().join("library")).unwrap());
        let unrelated = Asset::new(
            "Arrow coral",
            Kind::Line {
                start: [0., 0.5],
                end: [1., 0.5],
                arrow: true,
            },
            Style::default(),
            3.,
        );
        let mut ink = Asset::new(
            "My signature",
            Kind::Ink {
                strokes: vec![vec![[0., 0.], [1., 1.]]],
            },
            Style::default(),
            3.,
        );
        ink.signature = true;
        let imported = Asset::new("Company image", Kind::Rectangle, Style::default(), 3.);
        for asset in [&unrelated, &ink, &imported] {
            editor.add_asset(asset.clone()).unwrap();
        }
        state.signatures = vec![
            SavedSignature {
                asset_id: Some(imported.id.clone()),
                ..Default::default()
            },
            SavedSignature {
                asset_id: Some(imported.id.clone()),
                ..Default::default()
            },
        ];
        state.default_ink = Some(imported.id.clone());
        state.asset_id = Some(imported.id.clone());
        state.save_catalogue().unwrap();
        assert!(!state.is_signature_image(&unrelated));
        assert!(state.is_signature_image(&ink));
        assert!(state.is_signature_image(&imported));
        state.manage_signature_image(&imported);
        assert!(state.panel == Panel::Ink);
        assert_eq!(
            state.manager.ink_selected.as_deref(),
            Some(imported.id.as_str())
        );
        assert_eq!(state.manager.ink_name, "Company image");
        let mut renamed = imported.clone();
        renamed.name = "Company logo".into();
        editor.add_asset(renamed).unwrap();
        assert!(
            state.is_signature_image(
                editor
                    .library
                    .as_ref()
                    .unwrap()
                    .entries
                    .iter()
                    .find(|a| a.id == imported.id)
                    .unwrap()
            )
        );
        state
            .delete_signature_image(&mut editor, &imported.id)
            .unwrap();
        let reloaded = DigitalState::at(temp.path().join("profile"));
        assert!(reloaded.signatures.iter().all(|s| s.asset_id.is_none()));
        assert!(reloaded.default_ink.is_none() && reloaded.asset_id.is_none());
        assert!(
            editor
                .library
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .any(|a| a.id == unrelated.id)
        );
        // A second deletion remains possible after the first selected image disappears.
        state.manage_signature_image(&ink);
        state.delete_signature_image(&mut editor, &ink.id).unwrap();
        assert_eq!(editor.library.as_ref().unwrap().entries.len(), 1);
    }

    #[test]
    fn image_deletion_rolls_back_if_signature_catalogue_changed_elsewhere() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = DigitalState::at(temp.path().join("profile"));
        let mut editor = editing::EditorState::new();
        editor.library =
            Some(mypdf_milestone0::assets::Library::open(temp.path().join("library")).unwrap());
        let asset = Asset::new("Saved ink", Kind::Rectangle, Style::default(), 2.);
        editor.add_asset(asset.clone()).unwrap();
        state.signatures = vec![SavedSignature {
            asset_id: Some(asset.id.clone()),
            ..Default::default()
        }];
        state.save_catalogue().unwrap();
        std::fs::write(&state.catalogue, b"{}").unwrap();
        assert!(
            state
                .delete_signature_image(&mut editor, &asset.id)
                .is_err()
        );
        assert_eq!(
            state.signatures[0].asset_id.as_deref(),
            Some(asset.id.as_str())
        );
        let library = mypdf_milestone0::assets::Library::open(temp.path().join("library")).unwrap();
        assert_eq!(library.entries, vec![asset]);
    }
}

#[cfg(test)]
mod preview_stability_tests {
    use super::*;
    #[test]
    fn edge_spacing_invalidates_both_preview_caches() {
        let mut request = DesktopSigningRequest::default();
        let original = request_key(&request);
        request.transform.image_insets[0] = 12.;
        assert_ne!(original, request_key(&request));
        let image = request_key(&request);
        request.text_style.insets[3] = 15.;
        assert_ne!(image, request_key(&request));
        let previous = request_key(&request);
        request.transform.outer_padding = Some(0.);
        assert_ne!(previous, request_key(&request));
        let previous = request_key(&request);
        request.transform.image_text_gap = Some(12.);
        assert_ne!(previous, request_key(&request));
        let previous = request_key(&request);
        request.transform.fit_to_content = true;
        assert_ne!(previous, request_key(&request));
    }
    #[test]
    fn rapid_changes_keep_last_preview_and_bound_render_workers() {
        let ctx = egui::Context::default();
        let mut manager = ManagerState::new();
        let mut request = DesktopSigningRequest {
            name: "Preview signer".into(),
            show_date: false,
            ..Default::default()
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let first = loop {
            manager.receive(&ctx);
            if let Some(Ok(texture)) = manager.preview_for(&ctx, "editor", &request) {
                break texture;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        for angle in 1..100 {
            request.transform.rotation = angle as f32;
            let Some(Ok(shown)) = manager.preview_for(&ctx, "editor", &request) else {
                panic!("Preview must never blank while updating")
            };
            assert_eq!(shown.id(), first.id());
            assert!(manager.active <= 2);
            assert!(
                manager.queued.len() <= 1,
                "only the latest unstarted preview should remain"
            );
        }
        while !manager.idle() {
            manager.receive(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let Some(Ok(latest)) = manager.preview_for(&ctx, "editor", &request) else {
            panic!("latest preview")
        };
        assert_ne!(latest.id(), first.id());
    }
}

#[cfg(test)]
mod identity_choice_tests {
    use super::*;

    fn same_name_identity(code_signing: bool) -> Entry {
        use rcgen::{
            CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyPair,
        };
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, "Same Person");
        if code_signing {
            params.extended_key_usages = vec![ExtendedKeyUsagePurpose::CodeSigning];
        }
        let der = params.self_signed(&key).unwrap().der().to_vec();
        let info = identity::info(&der).unwrap();
        Entry {
            id: info.sha256.clone(),
            source: Source::Keychain {
                path: None,
                fingerprint: info.sha256.clone(),
            },
            info,
            der,
            eligible: true,
            explanation: None,
            token: false,
        }
    }

    #[test]
    fn same_name_and_expiry_choices_are_distinct_and_keep_the_selected_certificate() {
        let first = same_name_identity(false);
        let second = same_name_identity(false);
        assert_eq!(first.signer_name(), second.signer_name());
        assert_eq!(first.info.not_after, second.info.not_after);
        let a = IdentityChoice::new(&first);
        let b = IdentityChoice::new(&second);
        assert_ne!(a.title, b.title);
        assert!(a.title.contains(&first.short_certificate_id()));
        assert!(a.detail.contains("Valid until"));
        assert!(a.tooltip.contains(&first.info.sha256));
        assert!(a.tooltip.contains(&first.info.issuer));
        let temp = tempfile::tempdir().unwrap();
        let mut state = DigitalState::at(temp.path().into());
        state.entries = vec![first, second.clone()];
        let id = state.add_signature(Some(b.reference.clone()));
        state.entries.reverse();
        let signature = state.signature(&id).unwrap();
        assert_eq!(
            state.signer_of(signature).unwrap().info.sha256,
            second.info.sha256
        );
        assert_eq!(signature.identity.as_ref(), Some(&b.reference));
    }

    #[test]
    fn choices_keep_source_details_and_software_signing_warning() {
        let mut entry = same_name_identity(true);
        let choice = IdentityChoice::new(&entry);
        assert!(choice.detail.contains("Code signing"));
        entry.source = Source::File {
            path: "/example/team.p12".into(),
            alias: "Team ID".into(),
        };
        let choice = IdentityChoice::new(&entry);
        assert!(choice.detail.contains("Team ID"));
        assert!(choice.tooltip.contains("/example/team.p12"));
    }
}
