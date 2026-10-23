//! Compact application menus for Windows/Linux; tested on every platform.
use super::*;

#[derive(Clone, Copy)]
enum Entry {
    Command(&'static str, &'static str),
    Submenu(&'static str, &'static [Entry]),
    Separator,
    Recent,
}
use Entry::{Command as C, Recent as R, Separator as S, Submenu as M};
const MENUS: &[(&str, &[Entry])] = &[
    (
        "File",
        &[
            C("new", "New Blank PDF"),
            C("open", "Open…"),
            R,
            C("reopen_closed", "Reopen Closed Tab"),
            C("workspace", "Workspace…"),
            S,
            C("save", "Save"),
            C("save_as", "Save As…"),
            S,
            C("print", "Print…"),
            C("nup", "Multiple Pages per Sheet…"),
            C("merge_files", "Merge PDFs…"),
            S,
            C("protect", "Protect with Password…"),
            C("unprotect", "Remove Password…"),
            C("shrink", "Reduce File Size…"),
            S,
            C("settings", "Settings…"),
            C("close", "Close Tab"),
            C("quit", "Exit"),
        ],
    ),
    (
        "Edit",
        &[
            C("edit_undo", "Undo"),
            C("edit_redo", "Redo"),
            S,
            C("cut", "Cut"),
            C("copy", "Copy"),
            C("paste", "Paste"),
            C("paste_original", "Paste in Original Position"),
            C("select_all", "Select All"),
            S,
            C("context_copy_format", "Copy Object Style"),
            C("context_paste_format", "Paste Object Style"),
            S,
            C("undo", "Undo Document Change"),
            C("redo", "Redo Document Change"),
            C("find", "Find…"),
            M(
                "Translation",
                &[
                    C("translate_selection", "Translate Selected Text…"),
                    C("translate_page", "Translate Page…"),
                    C("translation_languages", "Translation Languages…"),
                ],
            ),
        ],
    ),
    (
        "View",
        &[
            C("home", "Home"),
            C("zen", "Zen Mode"),
            M(
                "Appearance",
                &[
                    C("theme_system", "Match System"),
                    C("theme_light", "Light"),
                    C("theme_dark", "Dark"),
                ],
            ),
            S,
            C("zoom_in", "Zoom In"),
            C("zoom_out", "Zoom Out"),
            C("actual", "Actual Size"),
            C("fit_width", "Fit Width"),
            C("fit_page", "Fit Page"),
            S,
            C("sidebar", "Show / Hide Left Panel"),
            C("inspector", "Show / Hide Right Panel"),
            C("guides", "Show / Hide Editing Guides"),
            C("history", "Document History…"),
            C("edit_navigation", "Bookmarks and Page Labels…"),
            C("review_comments", "Review Comments…"),
            C("recovery", "Document Recovery…"),
            C("page_text", "Accessible Page Text…"),
        ],
    ),
    (
        "Annotate",
        &[
            C("annotate_highlight", "Highlight"),
            C("annotate_underline", "Underline"),
            C("annotate_squiggly", "Squiggly underline"),
            C("annotate_strikeout", "Strikeout"),
            C("annotate_note", "Note"),
            C("annotate_callout", "Callout"),
            C("annotate_text", "Text Box"),
            S,
            C("annotate_pen", "Pen"),
            C("annotate_rectangle", "Rectangle"),
            C("annotate_ellipse", "Ellipse"),
            C("annotate_arrow", "Arrow"),
            C("objects", "Objects and Layers"),
        ],
    ),
    (
        "Edit PDF",
        &[
            C("edit_text", "Edit Text and Images"),
            C("add_image", "Image…"),
            C("edit_table", "Table…"),
            C("redact", "Redact"),
            M(
                "Header and Footer",
                &[
                    C("hf_page_numbers", "Add Page Numbers…"),
                    C("hf_bates", "Add Bates Numbers…"),
                    C("hf_text", "Add Text…"),
                    S,
                    C("hf_remove_numbers", "Remove Page Numbers…"),
                    C("hf_remove_all", "Remove All Text"),
                ],
            ),
            C("watermark", "Watermark…"),
            M(
                "Form",
                &[
                    C("form_detect", "Detect Form Fields"),
                    C("form_fill_details", "Fill with My Details…"),
                    C("my_details", "My Details…"),
                    S,
                    C("form_tab_rows", "Tab Order by Rows"),
                    C("form_tab_columns", "Tab Order by Columns"),
                    S,
                    C("form_import", "Import Form Data…"),
                    C("form_export", "Export Form Data…"),
                    S,
                    C("form_clear", "Clear Form…"),
                    C("form_flatten", "Flatten Form Fields…"),
                ],
            ),
            S,
            C("flatten", "Flatten PDF…"),
        ],
    ),
    ("Scan & OCR", &[C("ocr", "Recognize Text…")]),
    (
        "Sign",
        &[
            C("sign_ink", "Sign in Ink"),
            C("sign_digital", "Digital Sign"),
            C("sign_quick", "Quick Sign"),
            S,
            C("sign_manage", "Manage Signs…"),
            C("validate", "Validate Signatures…"),
        ],
    ),
    (
        "Tools",
        &[
            C("tool_search", "Find a Tool…"),
            C("compare", "Compare Documents…"),
            S,
            C("templates", "Document Templates…"),
            C("batch", "Batch Processing…"),
            C("sharing_copy", "Prepare Copy for Sharing…"),
            C("workflow_settings", "Workflow Settings…"),
        ],
    ),
    (
        "Window",
        &[
            C("window_minimize", "Minimize"),
            C("window_maximize", "Maximize / Restore"),
        ],
    ),
    (
        "Help",
        &[C("tool_search", "Find a Tool…"), C("about", "About MyPDF")],
    ),
];

fn shortcut(id: &str) -> Option<egui::KeyboardShortcut> {
    use egui::{Key as K, Modifiers as M};
    let (key, extra) = match id {
        "new" => (K::N, M::NONE),
        "open" => (K::O, M::NONE),
        "save" => (K::S, M::NONE),
        "save_as" => (K::S, M::SHIFT),
        "close" => (K::W, M::NONE),
        "print" => (K::P, M::NONE),
        "edit_undo" => (K::Z, M::NONE),
        "edit_redo" => (K::Z, M::SHIFT),
        "cut" => (K::X, M::NONE),
        "copy" => (K::C, M::NONE),
        "paste" => (K::V, M::NONE),
        "paste_original" => (K::V, M::SHIFT),
        "select_all" => (K::A, M::NONE),
        "find" => (K::F, M::NONE),
        "tool_search" => (K::K, M::NONE),
        "settings" => (K::Comma, M::NONE),
        "reopen_closed" => (K::T, M::SHIFT),
        "home" => (K::H, M::SHIFT),
        "zen" => (K::F, M::SHIFT),
        "zoom_in" => (K::Equals, M::NONE),
        "zoom_out" => (K::Minus, M::NONE),
        "actual" => (K::Num0, M::NONE),
        "sidebar" => (K::L, M::SHIFT),
        "inspector" => (K::R, M::SHIFT),
        "context_copy_format" => (K::C, M::ALT),
        "context_paste_format" => (K::V, M::ALT),
        _ => return None,
    };
    Some(egui::KeyboardShortcut::new(M::COMMAND | extra, key))
}
fn icon(id: &str) -> Icon {
    match id {
        "new" => Icon::Blank,
        "open" => Icon::Open,
        "reopen_closed" | "recovery" => Icon::Reload,
        "workspace" => Icon::Group,
        "save" => Icon::Save,
        "save_as" | "sharing_copy" => Icon::SaveAs,
        "print" | "nup" => Icon::Print,
        "merge_files" | "batch" => Icon::Library,
        "compare" => Icon::Compare,
        "settings" | "workflow_settings" | "sign_manage" => Icon::Settings,
        "copy" | "context_copy_format" => Icon::Copy,
        "cut" => Icon::Cut,
        "paste" | "paste_original" | "context_paste_format" => Icon::Paste,
        "select_all" => Icon::Select,
        "undo" | "edit_undo" => Icon::Undo,
        "redo" | "edit_redo" => Icon::Redo,
        "find" | "tool_search" => Icon::Search,
        "translate_selection" | "translate_page" | "translation_languages" => Icon::Translate,
        "home" => Icon::Home,
        "zen" => Icon::Zen,
        "theme_system" | "theme_light" | "theme_dark" => Icon::Theme,
        "zoom_in" => Icon::Plus,
        "zoom_out" | "window_minimize" => Icon::Minus,
        "actual" | "fit_page" | "window_maximize" => Icon::Rectangle,
        "fit_width" => Icon::FlipHorizontal,
        "sidebar" => Icon::Sidebar,
        "inspector" => Icon::Inspector,
        "guides" => Icon::Guides,
        "history" => Icon::History,
        "edit_navigation" => Icon::Outline,
        "review_comments" | "annotate_note" => Icon::Note,
        "page_text" | "edit_text" | "annotate_text" => Icon::Text,
        "annotate_highlight" => Icon::Highlight,
        "annotate_underline" => Icon::Underline,
        "annotate_squiggly" => Icon::Squiggly,
        "annotate_strikeout" => Icon::Strikeout,
        "annotate_pen" | "sign_ink" => Icon::Pen,
        "annotate_rectangle" => Icon::Rectangle,
        "annotate_ellipse" => Icon::Circle,
        "annotate_arrow" => Icon::Arrow,
        "annotate_callout" => Icon::Callout,
        "objects" => Icon::Library,
        "sign_quick" | "sign_digital" | "validate" => Icon::Sign,
        "protect" => Icon::Lock,
        "unprotect" => Icon::Unlock,
        "shrink" => Icon::Crop,
        "add_image" => Icon::FieldImage,
        "edit_table" => Icon::Table,
        "redact" => Icon::Redact,
        "hf_page_numbers" | "hf_bates" | "hf_text" => Icon::HeaderFooter,
        "hf_remove_numbers" | "hf_remove_all" | "form_clear" => Icon::Eraser,
        "watermark" => Icon::Watermark,
        "form_detect" => Icon::Search,
        "form_fill_details" | "my_details" => Icon::Person,
        "form_tab_rows" | "form_tab_columns" => Icon::TabOrder,
        "form_import" => Icon::Import,
        "form_export" => Icon::Export,
        "form_flatten" | "flatten" => Icon::Flatten,
        "ocr" => Icon::Ocr,
        "templates" => Icon::Document,
        "close" | "quit" => Icon::Close,
        "about" => Icon::Info,
        _ => Icon::Document,
    }
}
fn submenu_icon(label: &str) -> Icon {
    match label {
        "Translation" => Icon::Translate,
        "Appearance" => Icon::Theme,
        "Header and Footer" => Icon::HeaderFooter,
        "Form" => Icon::Form,
        _ => Icon::Library,
    }
}
fn enabled(id: &str, document: bool, saving: bool) -> bool {
    if id == "ocr" && !mypdf_milestone0::ocr::available() {
        return false;
    }
    if matches!(id, "save" | "save_as") {
        return document && !saving;
    }
    document
        || matches!(
            id,
            "new"
                | "open"
                | "reopen_closed"
                | "workspace"
                | "settings"
                | "quit"
                | "home"
                | "zen"
                | "theme_system"
                | "theme_light"
                | "theme_dark"
                | "tool_search"
                | "recovery"
                | "sign_manage"
                | "templates"
                | "batch"
                | "sharing_copy"
                | "workflow_settings"
                | "my_details"
                | "window_minimize"
                | "window_maximize"
                | "about"
        )
}
fn entries(
    ui: &mut egui::Ui,
    items: &[Entry],
    document: bool,
    saving: bool,
    recent: &[recent_menu::Recent],
) -> Option<recent_menu::Action> {
    egui::ScrollArea::vertical()
        .max_height((ui.ctx().content_rect().height() - 64.).max(120.))
        .show(ui, |ui| {
            entries_contents(ui, items, document, saving, recent)
        })
        .inner
}
fn entries_contents(
    ui: &mut egui::Ui,
    items: &[Entry],
    document: bool,
    saving: bool,
    recent: &[recent_menu::Recent],
) -> Option<recent_menu::Action> {
    let width = ui::menu_width(
        ui,
        items.iter().filter_map(|e| match e {
            C(id, label) => Some((*label, shortcut(id))),
            M(label, _) => Some((*label, None)),
            R => Some(("Open Recent", None)),
            S => None,
        }),
    );
    let mut action = None;
    for entry in items {
        match entry {
            C(id, label) => {
                if ui
                    .add_enabled(
                        enabled(id, document, saving),
                        ui::MenuItem::new(icon(id), label)
                            .shortcut(shortcut(id))
                            .width(width),
                    )
                    .clicked()
                {
                    action = Some(recent_menu::Action::Command(id));
                    ui.close();
                }
            }
            M(label, children) => {
                let response = ui.add(ui::MenuItem::new(submenu_icon(label), label).width(width));
                ui::draw_icon(
                    ui.painter(),
                    Rect::from_center_size(
                        pos2(response.rect.right() - 12., response.rect.center().y),
                        vec2(12., 12.),
                    ),
                    Icon::Right,
                    ui.visuals().text_color(),
                );
                let popup_id = egui::containers::menu::SubMenu::id_from_widget_id(response.id);
                let needs_measurement = egui::AreaState::load(ui.ctx(), popup_id)
                    .is_none_or(|state| state.size.is_none());
                let popup = egui::containers::menu::SubMenu::new()
                    .config(
                        egui::containers::menu::MenuConfig::new()
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                    )
                    .show(ui, &response, |ui| {
                        if let Some(id) = entries(ui, children, document, saving, recent) {
                            action = Some(id);
                        }
                    });
                if needs_measurement && popup.is_some() {
                    ui.ctx()
                        .request_discard("Measure the application submenu before displaying it");
                }
            }
            R => {
                if let (Some(choice), _) = recent_menu::submenu(ui, recent, width) {
                    action = Some(choice);
                }
            }
            S => {
                ui.separator();
            }
        }
    }
    action
}
pub(super) fn bar(
    ui: &mut egui::Ui,
    document: bool,
    saving: bool,
    recent: &[recent_menu::Recent],
) -> Option<recent_menu::Action> {
    let mut action = None;
    egui::MenuBar::new().ui(ui, |ui| {
        for (name, children) in MENUS {
            let response = ui.menu_button(*name, |ui| {
                if let Some(id) = entries(ui, children, document, saving, recent) {
                    action = Some(id);
                }
            });
            let key = match *name {
                "File" => egui::Key::F,
                "Edit" => egui::Key::E,
                "View" => egui::Key::V,
                "Annotate" => egui::Key::A,
                "Edit PDF" => egui::Key::P,
                "Scan & OCR" => egui::Key::O,
                "Sign" => egui::Key::S,
                "Tools" => egui::Key::T,
                "Window" => egui::Key::W,
                _ => egui::Key::H,
            };
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, key)) {
                egui::Popup::open_id(
                    ui.ctx(),
                    egui::Popup::default_response_id(&response.response),
                );
                ui.ctx().request_repaint();
            }
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compare_lives_in_tools_and_every_command_has_a_contextual_icon() {
        fn check(entries: &[Entry]) {
            for entry in entries {
                match entry {
                    C(id, _) => assert!(
                        !matches!(icon(id), Icon::Document) || *id == "templates",
                        "Missing contextual menu icon for {id}"
                    ),
                    M(label, children) => {
                        assert!(!matches!(submenu_icon(label), Icon::Library));
                        check(children);
                    }
                    R => {}
                    S => {}
                }
            }
        }
        for (name, entries) in MENUS {
            check(entries);
            let compare = entries
                .iter()
                .filter(|e| matches!(e, C("compare", _)))
                .count();
            assert_eq!(compare, usize::from(*name == "Tools"));
        }
    }

    #[test]
    fn windows_menu_includes_all_main_menus_and_save_shortcuts() {
        assert_eq!(
            MENUS.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            [
                "File",
                "Edit",
                "View",
                "Annotate",
                "Edit PDF",
                "Scan & OCR",
                "Sign",
                "Tools",
                "Window",
                "Help"
            ]
        );
        assert_ne!(shortcut("save"), shortcut("save_as"));
        assert!(!enabled("save", false, false));
        assert!(!enabled("save_as", true, true));
        assert!(enabled("settings", false, false));
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(bar(ui, true, false, &[]).is_none());
                for (_, items) in MENUS {
                    assert!(entries(ui, items, true, false, &[]).is_none());
                }
            });
        });
    }
}
