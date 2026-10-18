//! Getting around: the outline in the sidebar, links on the page, and opening
//! web links after asking.
use super::*;
use mypdf_milestone0::navigation::{self, Link, OutlineItem, Target};

pub(super) struct NavData {
    labels: Vec<String>,
    pub outline: Result<Vec<OutlineItem>, String>,
    pub links: Vec<Link>,
}
/// The outline and links of one version of a document.
pub(super) struct Nav {
    source: Arc<[u8]>,
    rx: Option<mpsc::Receiver<NavData>>,
    data: Option<NavData>,
}

impl Nav {
    pub(super) fn label(&self, page: usize) -> Option<&str> {
        self.data.as_ref()?.labels.get(page).map(String::as_str)
    }
}
impl Tab {
    /// The document's outline and links, worked out in the background. While a
    /// changed document is read again, the previous ones stay.
    pub(super) fn navigation(&mut self, ctx: &egui::Context) -> Option<&NavData> {
        let source = self.bytes.clone()?;
        if self
            .nav
            .as_ref()
            .is_none_or(|n| !Arc::ptr_eq(&n.source, &source))
        {
            let (tx, rx) = mpsc::channel();
            let bytes = source.clone();
            let count = self.pages.len();
            let wake = ctx.clone();
            std::thread::spawn(move || {
                let data = std::panic::catch_unwind(|| NavData {
                    labels: mypdf_milestone0::navigation_edit::labels(&bytes, count)
                        .unwrap_or_default(),
                    outline: navigation::outline(&bytes).map_err(|e| format!("{e:#}")),
                    links: navigation::links(&bytes).unwrap_or_default(),
                })
                .unwrap_or_else(|_| NavData {
                    labels: vec![],
                    outline: Err("The outline could not be read".into()),
                    links: vec![],
                });
                let _ = tx.send(data);
                wake.request_repaint();
            });
            let previous = self.nav.take().and_then(|n| n.data);
            self.nav = Some(Nav {
                source,
                rx: Some(rx),
                data: previous,
            });
        }
        let nav = self.nav.as_mut()?;
        if let Some(rx) = &nav.rx
            && let Ok(data) = rx.try_recv()
        {
            nav.data = Some(data);
            nav.rx = None;
        }
        nav.data.as_ref()
    }
}

/// The Outline sidebar tab: the document's headings, the current one marked.
pub(super) fn outline_list(tab: &mut Tab, ui: &mut egui::Ui, ctx: &egui::Context) {
    let current = tab.page;
    let id = tab.id;
    let mut jump = None;
    match tab.navigation(ctx).map(|n| &n.outline) {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Reading the outline…").color(theme::muted()));
            });
        }
        Some(Err(e)) => {
            ui.label(RichText::new(e).size(12.).color(theme::muted()));
        }
        Some(Ok(items)) if items.is_empty() => {
            ui.label(
                RichText::new("This PDF has no outline. Documents made from Word or with headings usually have one.")
                    .size(12.)
                    .color(theme::muted()),
            );
        }
        Some(Ok(items)) => {
            // The section you are reading: the last heading at or before this page.
            let here = items
                .iter()
                .enumerate()
                .filter(|(_, i)| i.page.is_some_and(|p| p <= current))
                .max_by_key(|(n, i)| (i.page, *n))
                .map(|(n, _)| n);
            egui::ScrollArea::vertical()
                .id_salt((id, "outline"))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.;
                    for (n, item) in items.iter().enumerate() {
                        let selected = here == Some(n);
                        // The section being read sits on a soft highlight.
                        let response = egui::Frame::new()
                            .fill(if selected {
                                theme::accent_soft()
                            } else {
                                Color32::TRANSPARENT
                            })
                            .corner_radius(6)
                            .inner_margin(egui::Margin::symmetric(4, 2))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_space(item.level.min(6) as f32 * 12.);
                                    let title = if item.title.is_empty() {
                                        "Untitled"
                                    } else {
                                        &item.title
                                    };
                                    let color = if selected {
                                        theme::accent()
                                    } else if item.page.is_some() {
                                        theme::ink()
                                    } else {
                                        theme::muted()
                                    };
                                    let text = RichText::new(title).size(12.5).color(color);
                                    let label = ui.add(
                                        egui::Button::new(if item.level == 0 {
                                            text.strong()
                                        } else {
                                            text
                                        })
                                        .wrap()
                                        .frame(false),
                                    );
                                    if let Some(page) = item.page {
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(format!("{}", page + 1))
                                                        .size(11.)
                                                        .color(theme::muted()),
                                                );
                                            },
                                        );
                                    }
                                    label
                                })
                                .inner
                            })
                            .inner;
                        if response.clicked()
                            && let Some(page) = item.page
                        {
                            jump = Some(page);
                        }
                    }
                });
        }
    }
    if let Some(page) = jump {
        tab.jump(page);
    }
}

/// Links on a page. In Read they follow with a click; elsewhere with ⌘-click,
/// so tools keep working over them. Returns the link to follow.
pub(super) fn link_overlay(
    tab: &Tab,
    mode: editing::Mode,
    ui: &mut egui::Ui,
    page: usize,
    page_rect: Rect,
    scale: f32,
) -> Option<Target> {
    let data = tab.nav.as_ref()?.data.as_ref()?;
    let read = mode == editing::Mode::Read;
    if !read && !ui.input(|i| i.modifiers.command) {
        return None;
    }
    let mut follow = None;
    for (i, link) in data
        .links
        .iter()
        .enumerate()
        .filter(|(_, l)| l.page == page)
    {
        let r = Rect::from_min_max(
            page_rect.min + vec2(link.rect[0], link.rect[1]) * scale,
            page_rect.min + vec2(link.rect[2], link.rect[3]) * scale,
        );
        let hint = match &link.target {
            Target::Uri(uri) => uri.clone(),
            Target::Page(p) => format!("Go to page {}", p + 1),
        };
        let response = ui
            .interact(
                r.intersect(page_rect),
                egui::Id::new((tab.id, page, "link", i)),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(hint);
        if response.hovered() {
            ui.painter().with_clip_rect(page_rect).rect_filled(
                r,
                2.,
                theme::accent().gamma_multiply(0.1),
            );
        }
        if response.clicked() {
            follow = Some(link.target.clone());
        }
    }
    follow
}

/// Open a web or mail address in the system's browser or mail app.
pub(super) fn open_external(target: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("/usr/bin/open")
        .arg(target)
        .spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", target])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = std::process::Command::new("xdg-open").arg(target).spawn();
}

impl Viewer {
    pub(super) fn link_dialog(&mut self, ctx: &egui::Context) {
        let Some(uri) = self.link_confirm.clone() else {
            return;
        };
        let mail = uri.to_ascii_lowercase().starts_with("mailto:");
        match ui::confirmation(
            ctx,
            "open_link",
            if mail {
                "Write an email?"
            } else {
                "Open this link?"
            },
            &format!(
                "{uri}\n\n{}",
                if mail {
                    "It opens in your mail app."
                } else {
                    "It opens in your web browser. Only continue if you trust this document."
                }
            ),
            &[
                ("Open", ui::DialogTone::Primary),
                ("Cancel", ui::DialogTone::Secondary),
            ],
            1,
        ) {
            Some(0) => {
                self.link_confirm = None;
                open_external(&uri);
            }
            Some(_) => self.link_confirm = None,
            None => {}
        }
    }
}

#[cfg(test)]
impl Viewer {
    /// Visual QA: give the first tab's document an outline.
    pub(super) fn preview_outline(&mut self) {
        use lopdf::{Object, dictionary};
        let s = self.tabs[0].session.as_mut().unwrap();
        let mut doc = lopdf::Document::load_mem(&s.source).unwrap();
        let pages: Vec<_> = doc.get_pages().into_values().collect();
        let outlines = doc.new_object_id();
        let titles = [
            (0, "Introduction", 0),
            (1, "Invoice and totals", 0),
            (1, "İstanbul review", 0),
            (0, "Appendix: rotated page", 1),
        ];
        let ids: Vec<_> = titles.iter().map(|_| doc.new_object_id()).collect();
        let (top, children) = ([ids[0], ids[3]], [ids[1], ids[2]]);
        for (i, (level, title, page)) in titles.iter().enumerate() {
            let mut d = dictionary! {
                "Title" => lopdf::text_string(title),
                "Dest" => vec![Object::Reference(pages[*page]), "Fit".into()],
            };
            if *level == 0 {
                d.set("Parent", outlines);
                if ids[i] == top[0] {
                    d.set("Next", top[1]);
                    d.set("First", children[0]);
                    d.set("Last", children[1]);
                }
            } else {
                d.set("Parent", top[0]);
                if ids[i] == children[0] {
                    d.set("Next", children[1]);
                }
            }
            doc.objects.insert(ids[i], d.into());
        }
        doc.objects.insert(
            outlines,
            dictionary! { "Type" => "Outlines", "First" => top[0], "Last" => top[1] }.into(),
        );
        doc.catalog_mut().unwrap().set("Outlines", outlines);
        let mut bytes = vec![];
        doc.save_to(&mut bytes).unwrap();
        s.commit_document(bytes).unwrap();
        self.outline = true;
        self.search = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    use lopdf::{Document, Object, Stream, dictionary};

    /// Three pages; page 1 links to page 3 and to a web page; an outline entry for page 2.
    fn fixture() -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let mut kids = vec![];
        for _ in 0..3 {
            let content = doc.add_object(Stream::new(dictionary! {}, vec![]));
            kids.push(doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
                "Contents" => content,
            }));
        }
        let to_page = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => vec![100.into(), 600.into(), 300.into(), 640.into()],
            "Dest" => vec![Object::Reference(kids[2]), "Fit".into()],
        });
        let web = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => vec![100.into(), 500.into(), 300.into(), 540.into()],
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com/") },
        });
        doc.get_dictionary_mut(kids[0]).unwrap().set(
            "Annots",
            vec![Object::Reference(to_page), Object::Reference(web)],
        );
        doc.objects.insert(
            pages_id,
            dictionary! { "Type" => "Pages", "Kids" => kids.iter().map(|k| Object::Reference(*k)).collect::<Vec<_>>(), "Count" => 3 }.into(),
        );
        let outlines = doc.new_object_id();
        let entry = doc.add_object(dictionary! { "Title" => Object::string_literal("Chapter two"), "Parent" => outlines, "Dest" => vec![Object::Reference(kids[1]), "Fit".into()] });
        doc.objects.insert(
            outlines,
            dictionary! { "Type" => "Outlines", "First" => entry, "Last" => entry }.into(),
        );
        let catalog = doc.add_object(
            dictionary! { "Type" => "Catalog", "Pages" => pages_id, "Outlines" => outlines },
        );
        doc.trailer.set("Root", catalog);
        let mut out = vec![];
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn the_outline_and_links_take_you_to_pages_and_ask_before_the_web() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(PathBuf::from("Guide.pdf"), fixture(), &ctx);
        app.editor.mode = editing::Mode::Read;
        app.tabs[0].zoom = Zoom::Page;
        let frame = |app: &mut Viewer, events: Vec<egui::Event>| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 900.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            )
        };
        let click = |app: &mut Viewer, at: egui::Pos2| {
            for pressed in [true, false] {
                let _ = frame(
                    app,
                    vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            let _ = frame(app, vec![]);
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.tabs[0].nav.as_ref().is_none_or(|n| n.data.is_none())
            || app.tabs[0].pages.is_empty()
        {
            assert!(std::time::Instant::now() < deadline);
            let _ = frame(&mut app, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        for _ in 0..3 {
            let _ = frame(&mut app, vec![]);
        }
        // The outline tab lists the chapter; choosing it goes to page 2.
        app.outline = true;
        let output = frame(&mut app, vec![]);
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let r = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Chapter two"))
            .expect("the outline is listed")
            .1
            .bounds()
            .unwrap();
        click(
            &mut app,
            pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32),
        );
        for _ in 0..4 {
            let _ = frame(&mut app, vec![]);
        }
        assert_eq!(app.tabs[0].page, 1, "the outline went to page 2");

        // Back on page 1, a click on the link goes to page 3.
        app.tabs[0].jump(0);
        for _ in 0..4 {
            let _ = frame(&mut app, vec![]);
        }
        let page_rect = |app: &Viewer| {
            ctx.read_response(egui::Id::new((app.tabs[0].id, 0_usize, "link", 0_usize)))
                .map(|r| r.rect)
        };
        let link = page_rect(&app).expect("the link is on the page");
        click(&mut app, link.center());
        for _ in 0..4 {
            let _ = frame(&mut app, vec![]);
        }
        assert_eq!(app.tabs[0].page, 2, "the link went to page 3");
        // A web link asks first.
        app.tabs[0].jump(0);
        for _ in 0..4 {
            let _ = frame(&mut app, vec![]);
        }
        let web = ctx
            .read_response(egui::Id::new((app.tabs[0].id, 0_usize, "link", 1_usize)))
            .unwrap()
            .rect;
        click(&mut app, web.center());
        assert_eq!(app.link_confirm.as_deref(), Some("https://example.com/"));
    }
}
