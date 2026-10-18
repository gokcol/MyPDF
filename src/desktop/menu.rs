use eframe::egui;
use std::sync::mpsc;

#[cfg(target_os = "macos")]
pub struct NativeMenu {
    menu: muda::Menu,
    rust_edit: muda::Submenu,
    dialog_edit: muda::Submenu,
    undo: muda::MenuItem,
    redo: muda::MenuItem,
}
#[cfg(target_os = "macos")]
impl NativeMenu {
    pub fn history_labels(&self, undo: Option<String>, redo: Option<String>, text: bool) {
        self.undo.set_text(if text {
            "Undo".into()
        } else {
            undo.map(|s| format!("Undo {s}"))
                .unwrap_or_else(|| "Undo".into())
        });
        self.redo.set_text(if text {
            "Redo".into()
        } else {
            redo.map(|s| format!("Redo {s}"))
                .unwrap_or_else(|| "Redo".into())
        });
    }

    fn dialog(&self, active: bool) {
        let (remove, insert) = if active {
            (&self.rust_edit, &self.dialog_edit)
        } else {
            (&self.dialog_edit, &self.rust_edit)
        };
        if let Err(e) = self
            .menu
            .remove(remove)
            .and_then(|_| self.menu.insert(insert, 2))
        {
            eprintln!("Could not switch native text commands: {e}");
        }
    }
}
#[cfg(not(target_os = "macos"))]
pub struct NativeMenu;
#[cfg(not(target_os = "macos"))]
impl NativeMenu {
    pub fn history_labels(&self, _: Option<String>, _: Option<String>, _: bool) {}
}

/// What MyPDF is, in the About window and the README.
#[cfg(all(target_os = "macos", not(test)))]
const ABOUT: &str = "MyPDF was built with the help of Claude, Anthropic's AI assistant, to make everyday business paperwork easier. It is provided as is, without warranty of any kind: keep copies of important documents and use it at your own risk.";

#[cfg(all(target_os = "macos", not(test)))]
pub fn install(ctx: egui::Context) -> (Option<NativeMenu>, mpsc::Receiver<String>) {
    use muda::{
        Menu, MenuItem, PredefinedMenuItem as P, Submenu,
        accelerator::{Accelerator, Code, Modifiers},
    };
    let (tx, rx) = mpsc::channel();
    let menu = Menu::new();
    let item = |id: &str, label: &str, code: Code, extra: Modifiers| {
        MenuItem::with_id(
            id,
            label,
            true,
            Some(Accelerator::new(Some(Modifiers::SUPER | extra), code)),
        )
    };
    let app = Submenu::with_items(
        "MyPDF",
        true,
        &[
            &P::about(
                Some("About MyPDF"),
                Some(muda::AboutMetadata {
                    name: Some("MyPDF".into()),
                    version: Some(format!("{} · Native PDF editor", env!("CARGO_PKG_VERSION"))),
                    comments: Some(ABOUT.into()),
                    ..Default::default()
                }),
            ),
            &P::separator(),
            &P::hide(None),
            &P::hide_others(None),
            &P::show_all(None),
            &P::separator(),
            &item("quit", "Quit MyPDF", Code::KeyQ, Modifiers::empty()),
        ],
    );
    let file = Submenu::with_items(
        "File",
        true,
        &[
            &item("new", "New Blank PDF", Code::KeyN, Modifiers::empty()),
            &item("open", "Open…", Code::KeyO, Modifiers::empty()),
            &item(
                "reopen_closed",
                "Reopen Closed Tab",
                Code::KeyT,
                Modifiers::SHIFT,
            ),
            &MenuItem::with_id("workspace", "Workspace…", true, None),
            &item("save", "Save a Copy…", Code::KeyS, Modifiers::empty()),
            &item("save", "Save As…", Code::KeyS, Modifiers::SHIFT),
            &P::separator(),
            &item("print", "Print…", Code::KeyP, Modifiers::empty()),
            &MenuItem::with_id("nup", "Multiple Pages per Sheet…", true, None),
            &MenuItem::with_id("edit_navigation", "Bookmarks and Page Labels…", true, None),
            &MenuItem::with_id("review_comments", "Review Comments…", true, None),
            &MenuItem::with_id("compare", "Compare Documents…", true, None),
            &P::separator(),
            &MenuItem::with_id("protect", "Protect with Password…", true, None),
            &MenuItem::with_id("unprotect", "Remove Password…", true, None),
            &MenuItem::with_id("shrink", "Reduce File Size…", true, None),
            &P::separator(),
            &item("close", "Close Tab", Code::KeyW, Modifiers::empty()),
        ],
    );
    let undo = item("edit_undo", "Undo", Code::KeyZ, Modifiers::empty());
    let redo = item("edit_redo", "Redo", Code::KeyZ, Modifiers::SHIFT);
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &undo,
            &redo,
            &P::separator(),
            &item("cut", "Cut", Code::KeyX, Modifiers::empty()),
            &item("copy", "Copy", Code::KeyC, Modifiers::empty()),
            &item("paste", "Paste", Code::KeyV, Modifiers::empty()),
            &item(
                "paste_original",
                "Paste in Original Position",
                Code::KeyV,
                Modifiers::SHIFT,
            ),
            &item("select_all", "Select All", Code::KeyA, Modifiers::empty()),
            &P::separator(),
            &item(
                "context_copy_format",
                "Copy Object Style",
                Code::KeyC,
                Modifiers::ALT,
            ),
            &item(
                "context_paste_format",
                "Paste Object Style",
                Code::KeyV,
                Modifiers::ALT,
            ),
            &P::separator(),
            &MenuItem::with_id("undo", "Undo Document Change", true, None),
            &MenuItem::with_id("redo", "Redo Document Change", true, None),
            &P::separator(),
            &item("find", "Find…", Code::KeyF, Modifiers::empty()),
        ],
    );
    let header_footer = Submenu::with_items(
        "Header and Footer",
        true,
        &[
            &MenuItem::with_id("hf_page_numbers", "Add Page Numbers…", true, None),
            &MenuItem::with_id("hf_bates", "Add Bates Numbers…", true, None),
            &MenuItem::with_id("hf_text", "Add Text…", true, None),
            &P::separator(),
            &MenuItem::with_id("hf_remove_numbers", "Remove Page Numbers…", true, None),
            &MenuItem::with_id("hf_remove_all", "Remove All Text", true, None),
        ],
    );
    let Ok(header_footer) = header_footer else {
        return (None, rx);
    };
    let form = Submenu::with_items(
        "Form",
        true,
        &[
            &MenuItem::with_id("form_detect", "Detect Form Fields", true, None),
            &P::separator(),
            &MenuItem::with_id("form_fill_details", "Fill with My Details…", true, None),
            &MenuItem::with_id("my_details", "My Details…", true, None),
            &P::separator(),
            &MenuItem::with_id("form_tab_rows", "Tab Order by Rows", true, None),
            &MenuItem::with_id("form_tab_columns", "Tab Order by Columns", true, None),
            &P::separator(),
            &MenuItem::with_id("form_import", "Import Form Data…", true, None),
            &MenuItem::with_id("form_export", "Export Form Data…", true, None),
            &P::separator(),
            &MenuItem::with_id("form_clear", "Clear Form…", true, None),
            &MenuItem::with_id("form_flatten", "Flatten Form Fields…", true, None),
        ],
    );
    let Ok(form) = form else {
        return (None, rx);
    };
    let edit_pdf = Submenu::with_items(
        "Edit PDF",
        true,
        &[
            &MenuItem::with_id("edit_text", "Text", true, None),
            &MenuItem::with_id("edit_images", "Image", true, None),
            &MenuItem::with_id("add_image", "Add Image…", true, None),
            &MenuItem::with_id("redact", "Redact", true, None),
            &header_footer,
            &MenuItem::with_id("watermark", "Watermark…", true, None),
            &form,
            &MenuItem::with_id("ocr", "Recognize Text…", true, None),
            &P::separator(),
            &MenuItem::with_id("flatten", "Flatten PDF…", true, None),
        ],
    );
    let appearance = Submenu::with_items(
        "Appearance",
        true,
        &[
            &MenuItem::with_id("theme_system", "Match System", true, None),
            &MenuItem::with_id("theme_light", "Light", true, None),
            &MenuItem::with_id("theme_dark", "Dark", true, None),
        ],
    );
    let Ok(appearance) = appearance else {
        return (None, rx);
    };
    let view = Submenu::with_items(
        "View",
        true,
        &[
            &MenuItem::with_id("home", "Home (⇧⌘H)", true, None),
            &MenuItem::with_id("zen", "Zen Mode (⇧⌘F)", true, None),
            &appearance,
            &P::separator(),
            &item("zoom_in", "Zoom In", Code::Equal, Modifiers::empty()),
            &item("zoom_out", "Zoom Out", Code::Minus, Modifiers::empty()),
            &item("actual", "Actual Size", Code::Digit0, Modifiers::empty()),
            &MenuItem::with_id("fit_width", "Fit Width", true, None),
            &MenuItem::with_id("fit_page", "Fit Page", true, None),
            &MenuItem::with_id("sidebar", "Show / Hide Left Panel (⇧⌘L)", true, None),
            &MenuItem::with_id("inspector", "Show / Hide Right Panel (⇧⌘R)", true, None),
            &MenuItem::with_id("guides", "Show / Hide Editing Guides", true, None),
            &MenuItem::with_id("history", "Document History…", true, None),
            &item(
                "tool_search",
                "Find a Tool…",
                Code::KeyK,
                Modifiers::empty(),
            ),
            &MenuItem::with_id("recovery", "Document Recovery…", true, None),
            &MenuItem::with_id("page_text", "Accessible Page Text…", true, None),
        ],
    );
    let window = Submenu::with_items("Window", true, &[&P::minimize(None), &P::maximize(None)]);
    let sign = Submenu::with_items(
        "Sign",
        true,
        &[
            &MenuItem::with_id("quick_sign", "Quick Sign", true, None),
            &MenuItem::with_id("appearance", "My Signatures…", true, None),
            &MenuItem::with_id("identities", "Digital IDs…", true, None),
            &MenuItem::with_id("sign", "Sign with Options…", true, None),
            &MenuItem::with_id("validate", "Validate Signatures…", true, None),
        ],
    );
    let Ok(edit) = edit else {
        return (None, rx);
    };
    let dialog_edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &P::undo(None),
            &P::redo(None),
            &P::separator(),
            &P::cut(None),
            &P::copy(None),
            &P::paste(None),
            &P::select_all(None),
        ],
    );
    let Ok(dialog_edit) = dialog_edit else {
        return (None, rx);
    };
    let result = (|| -> muda::Result<()> {
        menu.append_items(&[&app?, &file?, &edit, &edit_pdf?, &view?, &sign?, &window?])?;
        menu.init_for_nsapp();
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("Could not install native menus: {e}");
        return (None, rx);
    }
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        let _ = tx.send(event.id.0);
        ctx.request_repaint();
    }));
    (
        Some(NativeMenu {
            menu,
            rust_edit: edit,
            dialog_edit,
            undo,
            redo,
        }),
        rx,
    )
}

#[cfg(any(not(target_os = "macos"), test))]
pub fn install(_: egui::Context) -> (Option<NativeMenu>, mpsc::Receiver<String>) {
    let (_, rx) = mpsc::channel();
    (None, rx)
}

// File panels own their Cocoa text fields; restore the app's commands even if a panel unwinds.
pub fn with_file_dialog<T>(menu: Option<&NativeMenu>, show: impl FnOnce() -> T) -> T {
    #[cfg(target_os = "macos")]
    struct Restore<'a>(Option<&'a NativeMenu>);
    #[cfg(target_os = "macos")]
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            if let Some(menu) = self.0 {
                menu.dialog(false);
            }
        }
    }
    #[cfg(target_os = "macos")]
    let _restore = {
        if let Some(menu) = menu {
            menu.dialog(true);
        }
        Restore(menu)
    };
    #[cfg(not(target_os = "macos"))]
    let _ = menu;
    show()
}
