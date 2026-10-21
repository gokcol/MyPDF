//! AppKit asks for the current menu on each Dock right-click. Add only the
//! optional Dock callback to winit's delegate; never replace its delegate.
#![cfg(not(test))]
use super::*;
use muda::{CheckMenuItem, ContextMenu, Menu, MenuItem, PredefinedMenuItem, Submenu};
use objc2::{
    class, msg_send,
    runtime::{AnyObject, Imp, Sel},
    sel,
};
use std::cell::RefCell;

thread_local! {
    static SNAPSHOT: RefCell<Snapshot> = RefCell::new(Snapshot::default());
    // The returned NSMenu stays retained until the next Dock request.
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
}

pub(super) fn publish(snapshot: Snapshot) {
    SNAPSHOT.with(|s| *s.borrow_mut() = snapshot);
}

pub(super) fn install() {
    // SAFETY: Viewer is created on AppKit's main thread after winit has set its
    // delegate. We add an absent optional method, preserving every winit method.
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let delegate: *mut AnyObject = msg_send![app, delegate];
        let Some(delegate) = delegate.as_ref() else {
            return;
        };
        let class = delegate.class();
        let selector = sel!(applicationDockMenu:);
        if class.instance_method(selector).is_some() {
            return;
        }
        let implementation = std::mem::transmute::<
            unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> *mut AnyObject,
            Imp,
        >(dock_menu);
        if !objc2::ffi::class_addMethod(
            (class as *const objc2::runtime::AnyClass).cast_mut(),
            selector,
            implementation,
            c"@@:@".as_ptr(),
        )
        .as_bool()
        {
            eprintln!("Could not install the MyPDF Dock menu");
        }
    }
}

unsafe extern "C-unwind" fn dock_menu(
    _: *mut AnyObject,
    _: Sel,
    _: *mut AnyObject,
) -> *mut AnyObject {
    // Never unwind through AppKit, including during process shutdown.
    std::panic::catch_unwind(|| {
        let snapshot = SNAPSHOT.with(|s| s.borrow().clone());
        let Ok(menu) = build(&snapshot) else {
            return std::ptr::null_mut();
        };
        let pointer = menu.ns_menu().cast();
        MENU.with(|m| *m.borrow_mut() = Some(menu));
        pointer
    })
    .unwrap_or(std::ptr::null_mut())
}

fn item(entry: &Entry) -> CheckMenuItem {
    CheckMenuItem::with_id(
        format!("shell:{}", entry.action.encode()),
        &entry.title,
        true,
        entry.active,
        None,
    )
}

fn build(snapshot: &Snapshot) -> muda::Result<Menu> {
    let menu = Menu::new();
    let mut has_documents = false;
    for group in &snapshot.groups {
        if snapshot.grouped {
            let sub = Submenu::new(compact_title(&group.name), true);
            if group.entries.is_empty() {
                sub.append(&MenuItem::with_id(
                    format!("shell:{}", Action::Group(group.id).encode()),
                    "Show group",
                    true,
                    None,
                ))?;
            } else {
                for entry in &group.entries {
                    sub.append(&item(entry))?;
                }
            }
            menu.append(&sub)?;
            if group.active {
                // SAFETY: muda owns the NSMenu and its last NSMenuItem. All
                // access occurs synchronously on AppKit's main thread.
                unsafe {
                    let native = menu.ns_menu().cast::<AnyObject>();
                    let count: isize = msg_send![native, numberOfItems];
                    let last: *mut AnyObject = msg_send![native, itemAtIndex: count - 1];
                    let _: () = msg_send![last, setState: 1_isize];
                }
            }
            has_documents = true;
        } else {
            for entry in &group.entries {
                menu.append(&item(entry))?;
                has_documents = true;
            }
        }
    }
    if has_documents {
        menu.append(&PredefinedMenuItem::separator())?;
    }
    let recent = Submenu::new("Recent PDFs", true);
    let mut count = 0;
    for entry in &snapshot.recent {
        if let Action::Document(path) = &entry.action
            && path.is_file()
        {
            recent.append(&item(entry))?;
            count += 1;
        }
    }
    if count > 0 {
        menu.append(&recent)?;
    }
    menu.append(&MenuItem::with_id(
        format!("shell:{}", Action::Open.encode()),
        "Open PDF…",
        true,
        None,
    ))?;
    menu.append(&MenuItem::with_id(
        format!("shell:{}", Action::New.encode()),
        "New Blank PDF",
        true,
        None,
    ))?;
    Ok(menu)
}
