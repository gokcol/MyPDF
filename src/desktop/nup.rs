use super::*;
use mypdf_milestone0::{annotations, nup::Layout};
use std::time::{Duration, Instant};

type RasterResult = Result<([usize; 2], Vec<u8>), String>;
enum Job {
    Preview(u64, RasterResult),
    Saved(Result<PathBuf, String>),
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct SavedLayout {
    name: String,
    layout: Layout,
}
pub(super) struct Draft {
    presets: Vec<SavedLayout>,
    preset_name: String,
    preset_error: Option<String>,
    preset_root: PathBuf,
    source: Arc<[u8]>,
    annotations: Option<Vec<annotations::Annotation>>,
    path: PathBuf,
    count: usize,
    layout: Layout,
    range: String,
    sheet: usize,
    generation: u64,
    pending: Instant,
    preview_busy: bool,
    saving: bool,
    preview: Option<TextureHandle>,
    error: Option<String>,
    tx: mpsc::Sender<Job>,
    rx: mpsc::Receiver<Job>,
}
fn prepare(source: &[u8], marks: Option<&[annotations::Annotation]>) -> anyhow::Result<Vec<u8>> {
    match marks {
        Some(marks) => annotations::export(source, marks),
        None => Ok(source.to_vec()),
    }
}
fn margins(ui: &mut egui::Ui, values: &mut [f32; 4]) {
    egui::Grid::new(ui.next_auto_id())
        .num_columns(4)
        .show(ui, |ui| {
            for (i, label) in ["Top", "Bottom", "Left", "Right"].into_iter().enumerate() {
                ui.label(label);
                ui::numeric_value(
                    ui,
                    egui::DragValue::new(&mut values[i])
                        .range(0. ..=2000.)
                        .speed(0.5)
                        .suffix(" pt"),
                );
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
}
impl Viewer {
    pub(super) fn open_nup(&mut self) {
        if self.unfinished_edit(None) || !self.apply_pending_annotation() {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(s) = &tab.session else {
            return;
        };
        if tab.pages.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let (presets, preset_error) = match preferences::load::<Vec<SavedLayout>>(
            &self.workspace.root,
            "export-presets.json",
        ) {
            Ok(v) => (v, None),
            Err(e) => (vec![], Some(e.to_string())),
        };
        self.nup = Some(Draft {
            presets,
            preset_error,
            preset_name: String::new(),
            preset_root: self.workspace.root.clone(),
            source: s.source.clone(),
            annotations: (s.revision() != 0).then(|| s.annotations.clone()),
            path: tab.path.clone(),
            count: tab.pages.len(),
            layout: Layout::default(),
            range: format!("1-{}", tab.pages.len()),
            sheet: 0,
            generation: 0,
            pending: Instant::now() - Duration::from_secs(1),
            preview_busy: false,
            saving: false,
            preview: None,
            error: None,
            tx,
            rx,
        });
    }
    #[cfg(test)]
    pub(super) fn preview_nup(&mut self) {
        self.open_nup();
        if let Some(d) = &mut self.nup {
            d.layout.header = "Document review".into();
            d.layout.footer = "Sheet {page} of {pages}".into();
            d.layout.watermark = "DRAFT".into();
        }
    }
    #[cfg(test)]
    pub(super) fn nup_preview_pending(&self) -> bool {
        self.nup
            .as_ref()
            .is_some_and(|d| d.preview.is_none() && d.error.is_none())
    }
    pub(super) fn nup_dialog(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.nup else {
            return;
        };
        let mut saved = None;
        for job in d.rx.try_iter() {
            match job {
                Job::Preview(generation, result) => {
                    d.preview_busy = false;
                    if generation == d.generation {
                        match result {
                            Ok((size, pixels)) => {
                                d.preview = Some(ctx.load_texture(
                                    "sheet_preview",
                                    egui::ColorImage::from_rgba_premultiplied(size, &pixels),
                                    egui::TextureOptions::LINEAR,
                                ));
                                d.error = None;
                            }
                            Err(error) => d.error = Some(error),
                        }
                    }
                }
                Job::Saved(result) => {
                    d.saving = false;
                    saved = Some(result);
                }
            }
        }
        if let Some(result) = saved {
            match result {
                Ok(path) => {
                    self.nup = None;
                    self.open(path.clone(), ctx);
                    self.status = Some((format!("Saved {}", path.display()), false));
                    return;
                }
                Err(error) => d.error = Some(error),
            }
        }
        let before = (d.layout.clone(), d.range.clone(), d.sheet);
        let mut close = false;
        let mut save = false;
        let height = (ctx.content_rect().height() - 200.).clamp(80., 480.);
        let modal=egui::Modal::new(egui::Id::new("multiple_pages_per_sheet"))
            .backdrop_color(Color32::from_black_alpha(64)).frame(ui::modal_frame()).show(ctx, |ui| {
                ui.set_width(ui::dialog_width(ctx, 900.));
                ui.heading("Multiple pages per sheet");
                ui.label("Save a new PDF with smaller pages arranged on each sheet.");
                ui.add_space(12.);
                let mut valid = d.layout.validate().and_then(|_| mypdf_milestone0::pages::parse_range(&d.range,d.count).map(|_| ()));
                ui::dialog_body(ui, "sheet_layout_body", 480., |ui| {
                ui.add_enabled_ui(!d.saving, |ui| {
                    ui.columns(2,|cols| {
                        cols[0].vertical(|ui| {
                            ui::caption(ui,"Saved export presets");
                            egui::ComboBox::from_id_salt("saved_export_layout").selected_text(if d.preset_name.is_empty(){"Choose a preset"}else{&d.preset_name}).width(ui.available_width()).show_ui(ui,|ui|{
                                for entry in &d.presets {
                                    if ui.selectable_label(entry.name==d.preset_name,&entry.name).clicked(){
                                        match entry.layout.validate(){Ok(())=>{d.layout=entry.layout.clone();d.preset_name=entry.name.clone();},Err(e)=>d.error=Some(e.to_string())}
                                    }
                                }
                            });
                            ui.horizontal(|ui|{
                                ui.add(egui::TextEdit::singleline(&mut d.preset_name).margin(ui::FIELD_MARGIN).hint_text("Preset name").char_limit(80).desired_width((ui.available_width()-96.).max(60.)));
                                if ui.add_enabled(!d.preset_name.trim().is_empty() && d.preset_error.is_none() && d.layout.validate().is_ok(),ActionButton::new(Icon::Save,"Save preset").compact()).clicked(){
                                    let name=d.preset_name.trim().to_owned();let mut entries=d.presets.clone();entries.retain(|p|p.name!=name);
                                    entries.push(SavedLayout{name,layout:d.layout.clone()});
                                    if entries.len()>50 {d.error=Some("Keep up to 50 export presets".into());}
                                    else {match preferences::save(&d.preset_root,"export-presets.json",&entries){Ok(())=>d.presets=entries,Err(e)=>d.preset_error=Some(e.to_string())}}
                                }
                                if ui.add_enabled(d.presets.iter().any(|p|p.name==d.preset_name),ActionButton::new(Icon::Trash,"Delete preset").compact()).clicked(){
                                    let mut entries=d.presets.clone();entries.retain(|p|p.name!=d.preset_name);
                                    match preferences::save(&d.preset_root,"export-presets.json",&entries){Ok(())=>{d.presets=entries;d.preset_name.clear();},Err(e)=>d.preset_error=Some(e.to_string())}
                                }
                            });
                            if let Some(e)=&d.preset_error {ui.colored_label(theme::danger(),e);}
                            ui.separator();
                            ui::caption(ui,"Layout · columns × rows");
                            ui.horizontal_wrapped(|ui| {
                                for (columns,rows) in mypdf_milestone0::nup::PRESETS {
                                    if ui.selectable_label((d.layout.columns,d.layout.rows)==(columns,rows),format!("{columns} × {rows}")).clicked() {
                                        d.layout.columns=columns; d.layout.rows=rows;
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Columns"); ui::numeric_value(ui, egui::DragValue::new(&mut d.layout.columns).range(1..=8));
                                ui.label("Rows"); ui::numeric_value(ui, egui::DragValue::new(&mut d.layout.rows).range(1..=8));
                            });
                            ui.add_space(10.); ui::caption(ui,"Source pages");
                            ui.add(egui::TextEdit::singleline(&mut d.range).margin(ui::FIELD_MARGIN).hint_text("1, 3-8").char_limit(2000).desired_width(f32::INFINITY));
                            ui.add_space(10.); ui::caption(ui,"Sheet size");
                            ui.horizontal(|ui| {
                                let paper = [("A4",[595.,842.]),("A3",[842.,1191.]),("Letter",[612.,792.]),("Legal",[612.,1008.])]
                                    .into_iter().find(|(_,size)| *size==d.layout.sheet || [size[1],size[0]]==d.layout.sheet).map_or("Custom",|(label,_)|label);
                                egui::ComboBox::from_id_salt("sheet_paper").selected_text(paper).show_ui(ui,|ui| {
                                    for (label, mut size) in [("A4",[595.,842.]),("A3",[842.,1191.]),("Letter",[612.,792.]),("Legal",[612.,1008.])] {
                                        if ui.button(label).clicked() {
                                            if d.layout.sheet[0]>d.layout.sheet[1] {size.swap(0,1);}
                                            d.layout.sheet=size; ui.close();
                                        }
                                    }
                                });
                                if ui.add(ActionButton::new(Icon::Swap,"Change orientation").compact()).clicked() { d.layout.sheet.swap(0,1); }
                            });
                            ui.horizontal(|ui| {
                                ui.label("W");ui::numeric_value(ui, egui::DragValue::new(&mut d.layout.sheet[0]).range(72. ..=14400.).suffix(" pt"));
                                ui.label("H");ui::numeric_value(ui, egui::DragValue::new(&mut d.layout.sheet[1]).range(72. ..=14400.).suffix(" pt"));
                            });
                            ui.add_space(10.);ui::caption(ui,"Sheet margins");
                            margins(ui,&mut d.layout.margins);
                            ui.horizontal(|ui| {
                                if ui.button("No margins").clicked(){d.layout.margins=[0.;4];}
                                ui.label("Gap");ui::numeric_value(ui, egui::DragValue::new(&mut d.layout.gap).range(0. ..=144.).suffix(" pt"));
                            });
                            egui::CollapsingHeader::new("Trim source page edges").show(ui,|ui| {
                                ui.weak("Remove whitespace before fitting each page. Content in these edges is clipped.");
                                margins(ui,&mut d.layout.trim);
                                if ui.button("No trimming").clicked(){d.layout.trim=[0.;4];}
                            });
                            ui.separator();
                            egui::CollapsingHeader::new("Header & footer").show(ui,|ui| {
                            ui.add(egui::TextEdit::singleline(&mut d.layout.header).margin(ui::FIELD_MARGIN).hint_text("Header (optional)").char_limit(500).desired_width(f32::INFINITY));
                            ui.add(egui::TextEdit::singleline(&mut d.layout.footer).margin(ui::FIELD_MARGIN).hint_text("Footer · e.g. Sheet {page} of {pages}").char_limit(500).desired_width(f32::INFINITY));
                            ui.weak("{page} and {pages} refer to output sheets.");
                            ui.horizontal(|ui| {ui.label("Text size");ui::numeric_value(ui, egui::DragValue::new(&mut d.layout.text_size).range(6. ..=48.).suffix(" pt"));});
                            });
                            ui.separator();
                            egui::CollapsingHeader::new("Watermark").show(ui,|ui| {
                            ui.add(egui::TextEdit::singleline(&mut d.layout.watermark).margin(ui::FIELD_MARGIN).hint_text("Optional · e.g. CONFIDENTIAL").char_limit(200).desired_width(f32::INFINITY));
                            if !d.layout.watermark.trim().is_empty() {
                                ui::numeric_slider(ui, egui::Slider::new(&mut d.layout.watermark_opacity,0.05..=1.).text("Opacity"));
                                ui::numeric_slider(ui, egui::Slider::new(&mut d.layout.watermark_angle,-180. ..=180.).text("Angle").suffix("°"));
                            }
                            });
                        });
                        cols[1].vertical_centered(|ui| {
                            egui::Frame::new().fill(theme::canvas()).corner_radius(10).inner_margin(12.).show(ui,|ui| {
                                if let Some(texture)=&d.preview {
                                    let size=texture.size_vec2();
                                    let scale=(ui.available_width()/size.x).min((height-80.).max(80.)/size.y);
                                    let response=ui.add(egui::Image::new(texture).fit_to_exact_size(size*scale));
                                    ui.painter().rect_stroke(response.rect,0.,Stroke::new(1.,theme::line()),egui::StrokeKind::Outside);
                                } else {
                                    ui.allocate_ui(vec2(ui.available_width(),(height-80.).max(80.)),|ui| {ui.centered_and_justified(|ui| {ui.weak("Preparing preview…");});});
                                }
                            });
                            if let Ok(pages)=mypdf_milestone0::pages::parse_range(&d.range,d.count) {
                                let sheets=pages.len().div_ceil(d.layout.capacity());
                                d.sheet=d.sheet.min(sheets.saturating_sub(1));
                                ui.horizontal(|ui| {
                                    if ui.add_enabled(d.sheet>0,ActionButton::new(Icon::Left,"Previous sheet").compact()).clicked(){d.sheet-=1;}
                                    ui.label(format!("Sheet {} of {sheets}",d.sheet+1));
                                    if ui.add_enabled(d.sheet+1<sheets,ActionButton::new(Icon::Right,"Next sheet").compact()).clicked(){d.sheet+=1;}
                                });
                                ui.weak(format!("{} pages → {sheets} {}",pages.len(),if sheets==1 {"sheet"} else {"sheets"}));
                            }
                        });
                    });
                });
                valid=d.layout.validate().and_then(|_| mypdf_milestone0::pages::parse_range(&d.range,d.count).map(|_| ()));
                if let Err(error)=&valid {ui.colored_label(theme::danger(),error.to_string());}
                if let Some(error)=&d.error {ui.colored_label(theme::danger(),error);}
                ui.add_space(8.);
                ui.weak("Creates flattened copies of annotations and form fields. Digital signatures remain with the original PDF.");
                });
                ui::dialog_footer(ui, |ui| {
                    save=ui.add_enabled(!d.saving && valid.is_ok() && d.preview.is_some() && before==(d.layout.clone(),d.range.clone(),d.sheet),ActionButton::new(Icon::Save,"Save as PDF…").primary()).clicked();
                    close=ui.add_enabled(!d.saving,ActionButton::new(Icon::Close,"Cancel")).clicked();
                    if d.saving {ui.spinner();ui.label("Saving PDF…");}
                });
            });
        if !d.saving && (close || modal.should_close()) {
            self.nup = None;
            return;
        }
        if before != (d.layout.clone(), d.range.clone(), d.sheet) {
            d.generation += 1;
            d.pending = Instant::now();
            d.preview = None;
            d.error = None;
        }
        if d.preview.is_none()
            && d.error.is_none()
            && !d.preview_busy
            && !d.saving
            && d.pending.elapsed() >= Duration::from_millis(250)
            && d.layout.validate().is_ok()
            && let Ok(pages) = mypdf_milestone0::pages::parse_range(&d.range, d.count)
        {
            d.preview_busy = true;
            let capacity = d.layout.capacity();
            let sheets = pages.len().div_ceil(capacity);
            d.sheet = d.sheet.min(sheets.saturating_sub(1));
            let pages = pages
                .into_iter()
                .skip(d.sheet * capacity)
                .take(capacity)
                .collect::<Vec<_>>();
            let mut layout = d.layout.clone();
            for text in [&mut layout.header, &mut layout.footer] {
                *text = text
                    .replace("{page}", &(d.sheet + 1).to_string())
                    .replace("{pages}", &sheets.to_string());
            }
            let (source, marks, tx, wake, generation) = (
                d.source.clone(),
                d.annotations.clone(),
                d.tx.clone(),
                ctx.clone(),
                d.generation,
            );
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> anyhow::Result<_> {
                        let source = prepare(&source, marks.as_deref())?;
                        let bytes = mypdf_milestone0::nup::export(&source, &pages, &layout)?;
                        viewer::render_page(&bytes, 0, 900. / layout.sheet[0].max(layout.sheet[1]))
                    },
                ))
                .map_err(|_| "Could not prepare the sheet preview".into())
                .and_then(|r| r.map_err(|e| format!("{e:#}")));
                let _ = tx.send(Job::Preview(generation, result));
                wake.request_repaint();
            });
        }
        if d.preview.is_none() && d.error.is_none() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if save {
            let dialog = rfd::FileDialog::new()
                .set_title("Save page layout as a new PDF")
                .set_file_name(format!(
                    "{}-sheets.pdf",
                    d.path.file_stem().unwrap_or_default().to_string_lossy()
                ))
                .add_filter("PDF", &["pdf"]);
            let Some(mut path) = self.native_save(dialog) else {
                return;
            };
            let d = self.nup.as_mut().unwrap();
            if path.extension().is_none() {
                path.set_extension("pdf");
            }
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
            {
                d.error = Some("Use a .pdf filename".into());
                return;
            }
            let Ok(pages) = mypdf_milestone0::pages::parse_range(&d.range, d.count) else {
                return;
            };
            d.saving = true;
            d.error = None;
            let (source, marks, layout, tx, wake) = (
                d.source.clone(),
                d.annotations.clone(),
                d.layout.clone(),
                d.tx.clone(),
                ctx.clone(),
            );
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> anyhow::Result<_> {
                        let source = prepare(&source, marks.as_deref())?;
                        let bytes = mypdf_milestone0::nup::export(&source, &pages, &layout)?;
                        mypdf_milestone0::write_new(&path, &bytes)?;
                        Ok(path)
                    },
                ))
                .map_err(|_| "Sheet export failed; the original is unchanged".into())
                .and_then(|r| r.map_err(|e| format!("{e:#}")));
                let _ = tx.send(Job::Saved(result));
                wake.request_repaint();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    #[test]
    fn sheet_dialog_refreshes_preview_after_layout_changes_without_changing_source() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let frame = |app: &mut Viewer| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            )
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            frame(&mut app);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let source = app.tabs[0].session.as_ref().unwrap().source.clone();
        app.preview_nup();
        while app.nup_preview_pending() {
            frame(&mut app);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.nup.as_ref().unwrap().error.is_none());
        let out = frame(&mut app);
        let nodes = out.platform_output.accesskit_update.unwrap().nodes;
        assert!(nodes.iter().any(|(_, n)| n.label() == Some("Save as PDF…")));
        let d = app.nup.as_mut().unwrap();
        d.layout.columns = 3;
        d.layout.rows = 2;
        d.generation += 1;
        d.preview = None;
        d.pending = Instant::now() - Duration::from_secs(1);
        while app.nup_preview_pending() {
            frame(&mut app);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.nup.as_ref().unwrap().error.is_none());
        for _ in 0..3 {
            let small = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(720., 480.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
            let nodes = small.platform_output.accesskit_update.unwrap().nodes;
            let bounds = nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Save as PDF…"))
                .unwrap()
                .1
                .bounds()
                .unwrap();
            assert!(
                bounds.y1 <= 480. && bounds.x1 <= 720.,
                "save button must remain reachable: {bounds:?}"
            );
        }
        app.nup = None;
        assert_eq!(app.tabs[0].session.as_ref().unwrap().source, source);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
    }
}
