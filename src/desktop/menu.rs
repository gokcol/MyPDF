use eframe::egui;
use std::sync::mpsc;

#[cfg(all(target_os = "macos", not(test)))]
pub(super) const WORKFLOW_ITEMS: &[(&str, &str)] = &[
    ("templates", "Document Templates…"),
    ("batch", "Batch Processing…"),
    ("sharing_copy", "Prepare Copy for Sharing…"),
    ("workflow_settings", "Workflow Settings…"),
];

#[cfg(target_os = "macos")]
pub struct NativeMenu {
    menu: muda::Menu,
    rust_edit: muda::Submenu,
    dialog_edit: muda::Submenu,
    undo: muda::MenuItem,
    redo: muda::MenuItem,
    dialog_depth: DialogDepth,
}
#[cfg(target_os = "macos")]
#[derive(Default)]
struct DialogDepth(std::cell::Cell<usize>);
#[cfg(target_os = "macos")]
impl DialogDepth {
    /// Switch commands only on the outermost entry/exit of a native panel.
    fn transition(&self, entering: bool) -> bool {
        let old = self.0.get();
        let new = if entering {
            old.saturating_add(1)
        } else {
            old.saturating_sub(1)
        };
        self.0.set(new);
        if entering { old == 0 } else { old == 1 }
    }
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
        if !self.dialog_depth.transition(active) {
            return;
        }
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
        } else {
            #[cfg(not(test))]
            decorate_native_menu(&self.menu);
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
pub(super) const ABOUT: &str = "MyPDF was built with the help of Claude, Anthropic's AI assistant, to make everyday business paperwork easier. It is provided as is, without warranty of any kind: keep copies of important documents and use it at your own risk.";

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
                    credits: Some("Developed by Orhan Gökçöl".into()),
                    comments: Some(ABOUT.into()),
                    ..Default::default()
                }),
            ),
            &P::separator(),
            &item("settings", "Settings…", Code::Comma, Modifiers::empty()),
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
            &item("save", "Save", Code::KeyS, Modifiers::empty()),
            &item("save_as", "Save As…", Code::KeyS, Modifiers::SHIFT),
            &P::separator(),
            &item("print", "Print…", Code::KeyP, Modifiers::empty()),
            &MenuItem::with_id("nup", "Multiple Pages per Sheet…", true, None),
            &MenuItem::with_id("edit_navigation", "Bookmarks and Page Labels…", true, None),
            &MenuItem::with_id("review_comments", "Review Comments…", true, None),
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
    let translation = Submenu::with_items(
        "Translation",
        true,
        &[
            &MenuItem::with_id(
                "translate_selection",
                "Translate Selected Text…",
                true,
                None,
            ),
            &MenuItem::with_id("translate_page", "Translate Page…", true, None),
            &P::separator(),
            &MenuItem::with_id(
                "translation_languages",
                "Translation Languages…",
                true,
                None,
            ),
        ],
    );
    let Ok(translation) = translation else {
        return (None, rx);
    };
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
            &translation,
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
            &MenuItem::with_id("edit_text", "Edit Text and Images", true, None),
            &MenuItem::with_id("add_image", "Image…", true, None),
            &MenuItem::with_id("edit_table", "Table…", true, None),
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
    let tools = Submenu::new("Tools", true);
    if let Err(e) = tools.append_items(&[
        &MenuItem::with_id("compare", "Compare Documents…", true, None),
        &P::separator(),
    ]) {
        eprintln!("Could not install Tools menu: {e}");
        return (None, rx);
    }
    for (id, label) in WORKFLOW_ITEMS {
        if let Err(e) = tools.append(&MenuItem::with_id(*id, *label, true, None)) {
            eprintln!("Could not install Tools menu: {e}");
            return (None, rx);
        }
    }
    let window = Submenu::with_items("Window", true, &[&P::minimize(None), &P::maximize(None)]);
    let sign = Submenu::with_items(
        "Sign",
        true,
        &[
            &MenuItem::with_id("quick_sign", "Quick Sign", true, None),
            &MenuItem::with_id("settings", "Signature Settings…", true, None),
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
        menu.append_items(&[
            &app?, &file?, &edit, &edit_pdf?, &tools, &view?, &sign?, &window?,
        ])?;
        // Keep muda's action targets, accelerators and predefined Cocoa commands.
        // Only decorate each native row after the menu hierarchy exists.
        decorate_native_menu(&menu);
        menu.init_for_nsapp();
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("Could not install native menus: {e}");
        return (None, rx);
    }
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        if let Some(encoded) = event.id.0.strip_prefix("shell:")
            && let Some(action) = super::shell_menu::Action::decode(encoded)
        {
            super::open_with::queue_launch_actions(&super::open_with::Request {
                paths: vec![],
                actions: vec![action],
            });
            return;
        }
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
            dialog_depth: DialogDepth::default(),
        }),
        rx,
    )
}

/// SF Symbols are native template images: AppKit supplies selected, disabled and
/// dark-mode colors. Keep standard MenuItem/Predefined objects so text panels,
/// Undo labels and Window commands retain their original responder routing.
#[cfg(all(target_os = "macos", not(test)))]
fn decorate_native_menu(menu: &muda::Menu) {
    use muda::ContextMenu;
    let (icons, rows) = decorate_native_rows(menu.ns_menu().cast(), &menu.items(), false);
    if std::env::var_os("MYPDF_MENU_ICON_REPORT").is_some() {
        eprintln!("MyPDF native menu icons: {icons}/{rows} rows");
    }
}

#[cfg(all(target_os = "macos", not(test)))]
fn decorate_native_rows(
    native: *mut objc2::runtime::AnyObject,
    items: &[muda::MenuItemKind],
    decorate_rows: bool,
) -> (usize, usize) {
    use muda::MenuItemKind;
    use objc2::{
        class, msg_send,
        runtime::{AnyObject, Bool},
    };
    // SAFETY: install() and native menu changes run on AppKit's main thread.
    // muda retains each menu/item; NSMenuItem retains the assigned NSImage.
    let (mut icons, mut rows) = (0, 0);
    unsafe {
        for (index, item) in items.iter().enumerate() {
            let row: *mut AnyObject = msg_send![native, itemAtIndex: index as isize];
            let separator: Bool = msg_send![row, isSeparatorItem];
            if separator.as_bool() {
                continue;
            }
            if decorate_rows {
                let label = match item {
                    MenuItemKind::MenuItem(i) => i.text(),
                    MenuItemKind::Submenu(i) => i.text(),
                    MenuItemKind::Predefined(i) => i.text(),
                    MenuItemKind::Check(i) => i.text(),
                    MenuItemKind::Icon(i) => i.text(),
                };
                let symbol = native_symbol(&item.id().0, &label);
                let name = std::ffi::CString::new(symbol).expect("static symbol name");
                let name: *mut AnyObject =
                    msg_send![class!(NSString), stringWithUTF8String: name.as_ptr()];
                let mut image: *mut AnyObject = msg_send![class!(NSImage),
                    imageWithSystemSymbolName: name, accessibilityDescription: std::ptr::null::<AnyObject>()];
                if image.is_null() {
                    // SF Symbols grow between macOS versions. Always leave a valid
                    // template glyph on an older supported system.
                    let name: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: c"NSActionTemplate".as_ptr()];
                    image = msg_send![class!(NSImage), imageNamed: name];
                } else {
                    let config: *mut AnyObject = msg_send![class!(NSImageSymbolConfiguration),
                        configurationWithPointSize: 15.0_f64, weight: 0.0_f64];
                    let sized: *mut AnyObject =
                        msg_send![image, imageWithSymbolConfiguration: config];
                    if !sized.is_null() {
                        image = sized;
                    }
                }
                if !image.is_null() {
                    let _: () = msg_send![image, setTemplate: Bool::YES];
                    let _: () = msg_send![row, setImage: image];
                }
                rows += 1;
                let installed: *mut AnyObject = msg_send![row, image];
                icons += usize::from(!installed.is_null());
            }
            if let Some(submenu) = item.as_submenu() {
                // Muda creates a distinct NSMenu for every attachment. Walk
                // this row's actual submenu, rather than its context-menu copy.
                let child: *mut AnyObject = msg_send![row, submenu];
                if !child.is_null() {
                    let (child_icons, child_rows) =
                        decorate_native_rows(child, &submenu.items(), true);
                    icons += child_icons;
                    rows += child_rows;
                }
            }
        }
    }
    (icons, rows)
}

#[cfg(target_os = "macos")]
fn native_symbol(id: &str, label: &str) -> &'static str {
    match id {
        "new" => "doc.badge.plus",
        "open" => "folder",
        "reopen_closed" | "recovery" => "arrow.clockwise",
        "workspace" => "rectangle.3.group",
        "save" => "square.and.arrow.down",
        "save_as" | "sharing_copy" => "square.and.arrow.down.on.square",
        "print" | "nup" => "printer",
        "edit_navigation" => "list.bullet.indent",
        "review_comments" => "text.bubble",
        "compare" => "rectangle.split.2x1",
        "protect" => "lock",
        "unprotect" => "lock.open",
        "shrink" => "arrow.down.right.and.arrow.up.left",
        "close" | "quit" => "xmark",
        "edit_undo" | "undo" => "arrow.uturn.backward",
        "edit_redo" | "redo" => "arrow.uturn.forward",
        "cut" => "scissors",
        "copy" => "doc.on.doc",
        "paste" | "paste_original" => "doc.on.clipboard",
        "select_all" => "checkmark.square",
        "context_copy_format" | "context_paste_format" => "paintbrush",
        "find" | "tool_search" => "magnifyingglass",
        "translate_selection" | "translate_page" | "translation_languages" => "character.bubble",
        "hf_page_numbers" | "hf_bates" | "hf_text" => "text.append",
        "hf_remove_numbers" | "hf_remove_all" | "form_clear" => "eraser",
        "form_detect" => "viewfinder",
        "form_fill_details" | "my_details" => "person.crop.rectangle",
        "form_tab_rows" | "form_tab_columns" => "list.number",
        "form_import" => "square.and.arrow.down",
        "form_export" => "square.and.arrow.up",
        "form_flatten" | "flatten" => "square.3.layers.3d.down.right",
        "edit_text" | "page_text" => "text.cursor",
        "add_image" => "photo",
        "edit_table" => "tablecells",
        "redact" => "rectangle.fill",
        "watermark" => "drop",
        "ocr" => "text.viewfinder",
        "theme_system" => "circle.lefthalf.filled",
        "theme_light" => "sun.max",
        "theme_dark" => "moon",
        "home" => "house",
        "zen" => "viewfinder",
        "zoom_in" => "plus.magnifyingglass",
        "zoom_out" => "minus.magnifyingglass",
        "actual" => "1.magnifyingglass",
        "fit_width" => "arrow.left.and.right",
        "fit_page" => "arrow.up.left.and.arrow.down.right",
        "sidebar" => "sidebar.left",
        "inspector" => "sidebar.right",
        "guides" => "ruler",
        "history" => "clock.arrow.circlepath",
        "templates" => "doc.text",
        "batch" => "square.stack.3d.up",
        "workflow_settings" | "settings" => "gearshape",
        "quick_sign" | "sign" | "validate" => "signature",
        _ => match label {
            "Translation" => "character.bubble",
            "Header and Footer" => "text.append",
            "Form" => "list.bullet.rectangle",
            "Appearance" => "circle.lefthalf.filled",
            "Undo" => "arrow.uturn.backward",
            "Redo" => "arrow.uturn.forward",
            "Cut" => "scissors",
            "Copy" => "doc.on.doc",
            "Paste" => "doc.on.clipboard",
            "Select All" => "checkmark.square",
            "Minimize" => "minus",
            "Zoom" => "arrow.up.left.and.arrow.down.right",
            "Hide MyPDF" | "Hide Others" => "eye.slash",
            "Show All" => "eye",
            "About MyPDF" => "info.circle",
            _ => "doc",
        },
    }
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn native_icons_cover_edit_panels_submenus_and_standard_window_actions() {
        assert_eq!(
            native_symbol("save_as", "Save As…"),
            "square.and.arrow.down.on.square"
        );
        assert_eq!(
            native_symbol("compare", "Compare Documents…"),
            "rectangle.split.2x1"
        );
        assert_eq!(
            native_symbol("generated", "Undo"),
            native_symbol("edit_undo", "Undo")
        );
        assert_eq!(
            native_symbol("generated", "Copy"),
            native_symbol("copy", "Copy")
        );
        assert_eq!(native_symbol("generated", "Form"), "list.bullet.rectangle");
        assert_eq!(native_symbol("generated", "Minimize"), "minus");
        assert_eq!(
            native_symbol("generated", "Zoom"),
            "arrow.up.left.and.arrow.down.right"
        );
    }

    #[test]
    fn native_commands_restore_only_after_outermost_panel_closes() {
        let depth = DialogDepth::default();
        assert!(depth.transition(true));
        assert!(!depth.transition(true));
        assert!(!depth.transition(false));
        assert!(depth.transition(false));
        assert!(!depth.transition(false));
        assert!(depth.transition(true));
        assert!(depth.transition(false));
    }
}
