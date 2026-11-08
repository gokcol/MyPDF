//! In-app Windows preview. Opening or configuring this view never starts a job.
use super::*;

type Raster = Result<([usize; 2], Vec<u8>), String>;
pub(super) enum Action {
    None,
    Settings,
    Cancel,
    Print,
}

pub(super) struct Preview {
    pub bytes: Arc<[u8]>,
    pub pages: Vec<viewer::PageSize>,
    pub name: String,
    pub job: windows::Job,
    paper: windows::Paper,
    printer: String,
    page: usize,
    texture: Option<TextureHandle>,
    pending: Option<mpsc::Receiver<(usize, Raster)>>,
    error: Option<String>,
}

impl Preview {
    pub fn new(
        bytes: Vec<u8>,
        pages: Vec<viewer::PageSize>,
        name: String,
        job: windows::Job,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !job.pages().is_empty() && job.pages().end <= pages.len(),
            "The selected pages are unavailable."
        );
        let paper = job.paper()?;
        for page in job.pages() {
            paper.page_rect(pages[page])?;
        }
        Ok(Self {
            bytes: bytes.into(),
            pages,
            name,
            paper,
            printer: job.printer_name(),
            page: job.pages().start,
            job,
            texture: None,
            pending: None,
            error: None,
        })
    }

    pub fn configure(&mut self, owner: isize) {
        let result = windows::choose_printer(owner, self.pages.len(), Some(&mut self.job), false)
            .and_then(|job| {
                job.map(|job| {
                    let paper = job.paper()?;
                    for page in job.pages() {
                        paper.page_rect(self.pages[page])?;
                    }
                    Ok((job, paper))
                })
                .transpose()
            });
        match result {
            Ok(Some((job, paper))) => {
                let page = self.page.clamp(job.pages().start, job.pages().end - 1);
                if page != self.page {
                    self.texture = None;
                }
                self.page = page;
                self.printer = job.printer_name();
                self.job = job;
                self.paper = paper;
                self.error = None;
            }
            Ok(None) => {}
            Err(error) => {
                self.error = Some(format!("Could not change printer settings: {error:#}"))
            }
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok((page, result)) => {
                    self.pending = None;
                    if page == self.page {
                        match result {
                            Ok((size, pixels)) => {
                                self.texture = Some(ctx.load_texture(
                                    "print_preview",
                                    egui::ColorImage::from_rgba_premultiplied(size, &pixels),
                                    egui::TextureOptions::LINEAR,
                                ))
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.error = Some("The preview worker stopped unexpectedly.".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    fn render(&mut self, ctx: &egui::Context) {
        if self.texture.is_some() || self.pending.is_some() || self.error.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let (bytes, page, size, wake) = (
            self.bytes.clone(),
            self.page,
            self.pages[self.page],
            ctx.clone(),
        );
        match std::thread::Builder::new()
            .name("MyPDF print preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    viewer::render_page(&bytes, page, 1400. / size.width.max(size.height))
                }))
                .map_err(|_| "Could not render the print preview.".to_owned())
                .and_then(|result| {
                    result
                        .map_err(|error| format!("Could not preview page {}: {error:#}", page + 1))
                });
                let _ = tx.send((page, result));
                wake.request_repaint();
            }) {
            Ok(_) => self.pending = Some(rx),
            Err(error) => self.error = Some(format!("Could not prepare preview: {error}")),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Action {
        self.poll(ctx);
        let mut action = Action::None;
        let before = self.page;
        let modal = egui::Modal::new(egui::Id::new("windows_print_preview"))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(ui::dialog_width(ctx, 780.));
                ui.heading("Print preview");
                ui.add(egui::Label::new(&self.name).truncate());
                ui::dialog_body(ui, "print_preview_body", 760., |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("Printer: {}", self.printer));
                        if ui.button("Printer and page settings…").clicked() {
                            action = Action::Settings;
                        }
                    });
                    let points = self.paper.points();
                    let range = self.job.pages();
                    ui.label(format!(
                        "Paper: {:.0} × {:.0} mm · Pages {}–{} · Shrink to fit",
                        points[0] * 25.4 / 72.,
                        points[1] * 25.4 / 72.,
                        range.start + 1,
                        range.end
                    ));
                    ui.weak(
                        "Settings return here for review. Only Print below sends the document.",
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.page > range.start,
                                egui::Button::new("Previous page"),
                            )
                            .clicked()
                        {
                            self.page -= 1;
                        }
                        ui.label(format!("Page {} of {}", self.page + 1, self.pages.len()));
                        if ui
                            .add_enabled(self.page + 1 < range.end, egui::Button::new("Next page"))
                            .clicked()
                        {
                            self.page += 1;
                        }
                    });
                    if before != self.page {
                        self.texture = None;
                        self.error = None;
                    }
                    let height = (ctx.content_rect().height() - 380.).clamp(100., 580.);
                    let paper_size = vec2(points[0], points[1]);
                    let scale = ((ui.available_width() - 24.).max(40.) / paper_size.x)
                        .min(height / paper_size.y);
                    egui::Frame::new()
                        .fill(theme::canvas())
                        .corner_radius(8)
                        .inner_margin(12.)
                        .show(ui, |ui| {
                            ui.set_width((ui.available_width()).max(paper_size.x * scale));
                            ui.vertical_centered(|ui| {
                                let (rect, _) = ui
                                    .allocate_exact_size(paper_size * scale, egui::Sense::hover());
                                ui.painter().rect_filled(rect, 0., Color32::WHITE);
                                if let Some(texture) = &self.texture {
                                    if let Ok([x, y, w, h]) =
                                        self.paper.page_rect(self.pages[self.page])
                                    {
                                        let page = egui::Rect::from_min_size(
                                            rect.min + vec2(x * rect.width(), y * rect.height()),
                                            vec2(w * rect.width(), h * rect.height()),
                                        );
                                        ui.painter()
                                            .with_clip_rect(rect.intersect(ui.clip_rect()))
                                            .image(
                                                texture.id(),
                                                page,
                                                egui::Rect::from_min_max(
                                                    egui::Pos2::ZERO,
                                                    egui::pos2(1., 1.),
                                                ),
                                                Color32::WHITE,
                                            );
                                    }
                                } else {
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "Preparing preview…",
                                        FontId::proportional(14.),
                                        Color32::DARK_GRAY,
                                    );
                                }
                                ui.painter().rect_stroke(
                                    rect,
                                    0.,
                                    Stroke::new(1_f32, theme::line()),
                                    egui::StrokeKind::Outside,
                                );
                            });
                        });
                    if let Some(error) = &self.error {
                        ui.colored_label(theme::danger(), error);
                        if ui.button("Retry preview").clicked() {
                            self.error = None;
                        }
                    }
                });
                ui::dialog_footer(ui, |ui| {
                    if ui
                        .add_enabled(
                            self.texture.is_some() && self.error.is_none(),
                            ActionButton::new(Icon::Print, "Print").primary(),
                        )
                        .clicked()
                    {
                        action = Action::Print;
                    }
                    if ui.button("Cancel").clicked() {
                        action = Action::Cancel;
                    }
                });
            });
        if modal.should_close() {
            action = Action::Cancel;
        }
        if matches!(action, Action::None) {
            self.render(ctx);
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        action
    }
}
