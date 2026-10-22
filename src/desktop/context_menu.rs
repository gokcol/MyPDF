//! Page menus resolve the clicked content once; commands keep that target and
//! revision rather than acting on whatever selection happens to be current.
use super::*;
use mypdf_milestone0::annotations::{Annotation, Kind, Markup};

#[derive(Default)]
pub(in crate::desktop) struct State {
    menu: Option<Menu>,
    group: Option<&'static str>,
    command: Option<(&'static str, Menu)>,
    pub cancel_text: bool,
    pub delete_text: bool,
    pub detect_table_area: bool,
    image_paste: Option<clipboard::Clip>,
    pub(in crate::desktop) select_after_text: Option<(u64, usize, [f32; 2])>,
    pub(in crate::desktop) area_after_table: Option<(u64, usize, [f32; 4])>,
}

#[derive(Clone)]
struct Menu {
    key: Key,
    point: [f32; 2],
    target: Target,
}
#[derive(Clone)]
enum Target {
    Page,
    PendingText,
    Artwork(Vec<Annotation>),
    Text(Box<InlineText>),
    SelectedText(text_selection::TextSelection),
    Image(content::ImageObject),
}
struct Item {
    label: &'static str,
    action: &'static str,
    enabled: bool,
}
fn item(label: &'static str, action: &'static str, enabled: bool) -> Item {
    Item {
        label,
        action,
        enabled,
    }
}
// The menu hints and event routing share these definitions. Command means Ctrl
// on Windows; no shortcut steals plain typing or macOS's Command-H Hide action.
fn shortcut(action: &str) -> Option<egui::KeyboardShortcut> {
    use egui::{Key, KeyboardShortcut, Modifiers as M};
    let markup = M::COMMAND
        | if cfg!(target_os = "macos") {
            M::CTRL
        } else {
            M::ALT
        };
    let (modifiers, key) = match action {
        "copy" | "copy_text" | "copy_image" | "object_copy" => (M::COMMAND, Key::C),
        "cut" | "object_cut" | "cut_image" => (M::COMMAND, Key::X),
        "paste" | "paste_here" => (M::COMMAND, Key::V),
        "paste_original" => (M::COMMAND | M::SHIFT, Key::V),
        "all" | "all_text" => (M::COMMAND, Key::A),
        "object_duplicate" => (M::COMMAND, Key::D),
        "group_objects" => (M::COMMAND, Key::G),
        "ungroup_objects" => (M::COMMAND | M::SHIFT, Key::G),
        "copy_format" => (M::COMMAND | M::ALT, Key::C),
        "paste_format" => (M::COMMAND | M::ALT, Key::V),
        "highlight" => (markup, Key::H),
        "underline" => (markup, Key::U),
        "strikeout" => (markup, Key::S),
        "delete" | "object_delete" | "delete_image" => (
            M::NONE,
            if cfg!(target_os = "macos") {
                Key::Backspace
            } else {
                Key::Delete
            },
        ),
        "done" | "finish_text" => (M::COMMAND, Key::Enter),
        "cancel" | "cancel_text" => (M::NONE, Key::Escape),
        _ => return None,
    };
    Some(KeyboardShortcut::new(modifiers, key))
}

pub(in crate::desktop) fn consume_action(ctx: &egui::Context, action: &str) -> bool {
    let Some(binding) = shortcut(action) else {
        return false;
    };
    ctx.input_mut(|i| {
        let mut consumed = false;
        // winit turns Command-C/X/V into clipboard events even with extra
        // modifiers, so match those modifiers for style/original-position keys.
        let clipboard_modifiers = i.modifiers;
        let clipboard_matches = clipboard_modifiers.matches_exact(binding.modifiers)
            || (clipboard_modifiers == egui::Modifiers::NONE
                && binding.modifiers == egui::Modifiers::COMMAND);
        i.events.retain(|event| {
            let matches = match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    modifiers.matches_exact(binding.modifiers)
                        && (*key == binding.logical_key
                            || (binding.modifiers == egui::Modifiers::NONE
                                && matches!(
                                    binding.logical_key,
                                    egui::Key::Delete | egui::Key::Backspace
                                )
                                && matches!(key, egui::Key::Delete | egui::Key::Backspace)))
                }
                egui::Event::Copy => clipboard_matches && binding.logical_key == egui::Key::C,
                egui::Event::Cut => clipboard_matches && binding.logical_key == egui::Key::X,
                egui::Event::Paste(_) => clipboard_matches && binding.logical_key == egui::Key::V,
                _ => false,
            };
            consumed |= matches;
            !matches
        });
        consumed
    })
}

pub(in crate::desktop) fn paste_requested(ctx: &egui::Context) -> bool {
    ctx.input(|i| {
        i.events.iter().any(|e| {
            matches!(e, egui::Event::Paste(_))
                || matches!(e, egui::Event::Key { key: egui::Key::V, pressed: true, modifiers, .. }
                if modifiers.matches_exact(egui::Modifiers::COMMAND))
        })
    })
}

fn items(target: &Target, editable: bool) -> Vec<Item> {
    let mut entries = match target {
        Target::PendingText => vec![
            item("Delete", "delete_text_box", editable),
            item("Finish text edit", "finish_text", true),
            item("Cancel text edit", "cancel_text", true),
        ],
        Target::SelectedText(_) => vec![
            item("Copy selected text", "copy_text", true),
            item("Translate selected text…", "translate_selection", true),
            item("Highlight", "highlight", editable),
            item("Underline", "underline", editable),
            item("Strikethrough", "strikeout", editable),
            item("Select all page text", "all_text", true),
        ],
        Target::Text(_) => vec![
            item("Edit text…", "edit_text_here", editable),
            item("Edit table…", "edit_table_here", editable),
            item("Detect table in selection", "table_area", editable),
            item("Copy text", "copy_text", true),
            item("Translate text…", "translate_text", true),
            item("Select area…", "text_area", editable),
        ],
        Target::Image(_) => vec![
            item("Image properties…", "image_properties", true),
            item("Copy image", "copy_image", true),
            item("Cut image", "cut_image", editable),
            item("Replace image…", "replace_image", editable),
            item("Remove background…", "image_background", editable),
            item("Delete image", "delete_image", editable),
        ],
        Target::Artwork(marks) => {
            let single = (marks.len() == 1).then(|| &marks[0]);
            let locked = marks.iter().any(|a| a.object.locked);
            let change = editable && !locked;
            let mut v = vec![];
            if single.is_some_and(|a| matches!(a.kind, Kind::Text { .. })) {
                v.push(item("Edit text…", "edit_text_here", change));
                v.push(item("Copy text", "copy_artwork_text", true));
                v.push(item("Translate text…", "translate_text", true));
            }
            v.push(item(
                match single.map(|a| &a.kind) {
                    Some(Kind::Image { .. }) => "Image properties…",
                    Some(Kind::Text { .. }) => "Text properties…",
                    Some(Kind::Note { .. }) => "Edit note…",
                    Some(Kind::Svg { .. } | Kind::Stamp { .. }) => "Stamp properties…",
                    Some(_) => "Drawing properties…",
                    None => "Selection properties…",
                },
                "object_properties",
                true,
            ));
            v.extend([
                item("Copy", "object_copy", true),
                item("Cut", "object_cut", change),
                item("Duplicate", "object_duplicate", change),
            ]);
            if single.is_some_and(|a| matches!(a.kind, Kind::Image { .. })) {
                v.extend([
                    item("Crop image…", "crop_object", change),
                    item("Remove background…", "object_background", change),
                ]);
            }
            if !single.is_some_and(|a| {
                matches!(
                    a.kind,
                    Kind::Text { .. } | Kind::Note { .. } | Kind::Markup { .. }
                )
            }) {
                v.extend([
                    item("Rotate 90° clockwise", "rotate_right", change),
                    item("Rotate 90° counterclockwise", "rotate_left", change),
                    item("Flip horizontal", "flip_horizontal", change),
                    item("Flip vertical", "flip_vertical", change),
                ]);
            }
            v.extend([
                item("Copy style", "copy_format", single.is_some()),
                item("Paste style", "paste_format", change),
                item("Group objects", "group_objects", change && marks.len() > 1),
                item(
                    "Ungroup",
                    "ungroup_objects",
                    change && marks.iter().any(|a| a.object.group.is_some()),
                ),
                item("Select Behind", "select_behind", true),
            ]);
            v.extend([
                item("Bring to front", "front", change),
                item("Send to back", "back", change),
                item(if locked { "Unlock" } else { "Lock" }, "lock", editable),
                item("Comments…", "review_comments", true),
                item("Repeat on pages…", "repeat_objects", change),
                item("Delete", "object_delete", change),
            ]);
            v
        }
        Target::Page => vec![
            item("Edit table…", "edit_table_here", editable),
            item("Detect table in selection", "table_area", editable),
            item("Select area…", "text_area", editable),
            item("Paste Here", "paste_here", editable),
            item("Paste in Original Position", "paste_original", editable),
            item("Add text here", "add_text_here", editable),
            item("Add image…", "add_image", editable),
            item("Copy region…", "copy_region", editable),
            item("Select all page text", "all_text", true),
        ],
    };
    if !matches!(target, Target::PendingText) && !entries.iter().any(|i| i.action == "text_area") {
        entries.push(item("Select area…", "text_area", editable));
    }
    entries
}

fn action_icon(action: &str, label: &str) -> Icon {
    match action {
        "copy" | "copy_text" | "copy_artwork_text" | "copy_image" | "object_copy"
        | "object_duplicate" => Icon::Copy,
        "cut" | "object_cut" | "cut_image" => Icon::Cut,
        "paste" | "paste_here" | "paste_original" => Icon::Paste,
        "delete" | "delete_image" | "object_delete" | "delete_text_box" => Icon::Trash,
        "highlight" => Icon::Highlight,
        "underline" => Icon::Underline,
        "strikeout" => Icon::Strikeout,
        "all" | "all_text" | "text_area" => Icon::TextArea,
        "edit_table_here" | "table_area" => Icon::Table,
        "translate_selection" | "translate_text" => Icon::Translate,
        "done" | "finish_text" => Icon::Check,
        "cancel" | "cancel_text" => Icon::Close,
        "edit_text_here" | "add_text_here" => Icon::Text,
        "add_image" | "replace_image" | "image_properties" => Icon::FieldImage,
        "image_background" | "object_background" => Icon::Sparkle,
        "crop_object" | "copy_region" => Icon::Crop,
        "rotate_right" => Icon::RotateRight,
        "rotate_left" => Icon::RotateLeft,
        "flip_horizontal" => Icon::FlipHorizontal,
        "flip_vertical" => Icon::FlipVertical,
        "front" => Icon::BringForward,
        "back" | "select_behind" => Icon::SendBackward,
        "lock" if label == "Unlock" => Icon::Unlock,
        "lock" => Icon::Lock,
        "review_comments" => Icon::Note,
        "repeat_objects" => Icon::Library,
        "group_objects" => Icon::Group,
        "ungroup_objects" => Icon::Ungroup,
        "copy_format" | "paste_format" | "saved_styles" => Icon::Format,
        _ => Icon::Inspector,
    }
}
fn menu_layout(ui: &mut egui::Ui, width: f32) {
    ui.set_width(width);
    ui.spacing_mut().interact_size.y = 24.;
    ui.spacing_mut().item_spacing.y = 2.;
    ui.spacing_mut().button_padding = vec2(6., 2.);
}
fn submenu(action: &str) -> Option<(&'static str, Icon)> {
    match action {
        "rotate_right" | "rotate_left" | "flip_horizontal" | "flip_vertical" => {
            Some(("Transform", Icon::RotateRight))
        }
        "copy_format" | "paste_format" => Some(("Style", Icon::Format)),
        "group_objects" | "ungroup_objects" | "front" | "back" | "lock" => {
            Some(("Arrange", Icon::Group))
        }
        _ => None,
    }
}
fn action_row(ui: &mut egui::Ui, i: &Item, width: f32) -> bool {
    ui.add_enabled(
        i.enabled,
        ui::MenuItem::new(action_icon(i.action, i.label), i.label)
            .shortcut(shortcut(i.action))
            .width(width),
    )
    .clicked()
}
fn contains(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] - 2. && p[0] <= r[2] + 2. && p[1] >= r[1] - 2. && p[1] <= r[3] + 2.
}
fn candidates(state: &ContentState, tab: &Tab, page: usize) -> Vec<InlineText> {
    let Some(s) = &tab.session else { return vec![] };
    let key = (tab.id, s.revision(), page);
    let mut drafts = vec![];
    if let Some(info) = state.inspection.as_ref().filter(|i| i.key == key)
        && let Ok(runs) = &info.text
    {
        for (ordinal, run) in runs.iter().enumerate() {
            if let Some(rect) = run.rect {
                drafts.push(InlineText {
                    key,
                    target: InlineTarget::Run(ordinal),
                    geometry: run.layout.clone(),
                    original: run.text.clone(),
                    text: run.text.clone(),
                    rect,
                    size: run.size,
                    color: run.color,
                    font: run.base_font.clone(),
                    focus: true,
                });
            }
        }
    }
    for a in s
        .annotations
        .iter()
        .filter(|a| a.page == page && !a.object.hidden && !a.object.locked)
    {
        if let Kind::Text { text } = &a.kind {
            drafts.push(InlineText {
                key,
                target: InlineTarget::Annotation(a.id.clone()),
                geometry: None,
                original: text.clone(),
                text: text.clone(),
                rect: a.bounds(),
                size: a.style.font_size,
                color: a.style.color,
                font: "Text box".into(),
                focus: true,
            });
        }
    }
    drafts
}
fn target_at(
    state: &ContentState,
    editor: &mut editing::EditorState,
    tab: &mut Tab,
    page: usize,
    p: [f32; 2],
    scale: f32,
) -> Target {
    if state.pending_text().is_some()
        || (state.new_placed && state.add_target == Some((tab.id, page)))
    {
        return Target::PendingText;
    }
    if let Some(a) = editing::hit_annotation(tab, page, p, scale) {
        if !selection::selected_ids(tab).contains(&a.id) {
            selection::set(tab, editor, vec![a.id]);
        }
        let ids = selection::selected_ids(tab);
        return Target::Artwork(
            tab.session
                .as_ref()
                .unwrap()
                .annotations
                .iter()
                .filter(|a| ids.contains(&a.id))
                .cloned()
                .collect(),
        );
    }
    selection::set(tab, editor, vec![]);
    if let Some(s) = editor
        .text_selection
        .filter(|s| s.tab == tab.id && s.page == page && s.contains(tab, p))
    {
        return Target::SelectedText(s);
    }
    if editor.mode == editing::Mode::Edit {
        let texts = candidates(state, tab, page);
        if let Some(d) = texts.iter().rev().find(|d| contains(d.rect, p)) {
            return Target::Text(Box::new(line_at(d.clone(), &texts, tab.pages[page])));
        }
        if let Some(info) = state
            .inspection
            .as_ref()
            .filter(|i| Some(i.key) == tab.session.as_ref().map(|s| (tab.id, s.revision(), page)))
            && let Ok(images) = &info.images
            && let Some(image) = images.iter().rev().find(|i| contains(i.rect, p))
        {
            return Target::Image(image.clone());
        }
    }
    if let Some(s) = text_selection::TextSelection::word_at(tab, page, p) {
        editor.text_selection = Some(s);
        return Target::SelectedText(s);
    }
    editor.text_selection = None;
    Target::Page
}
/// Selecting supported content chooses its editor; Read retains ordinary text selection.
#[allow(clippy::too_many_arguments)]
pub(in crate::desktop) fn smart_select(
    state: &mut ContentState,
    editor: &mut editing::EditorState,
    tab: &mut Tab,
    ui: &egui::Ui,
    page: usize,
    rect: Rect,
    scale: f32,
    inspector: &mut bool,
) {
    if state.busy
        || state.add_target.is_some()
        || state.region.active
        || state.area_mode
        || editor.library_open
        || editor.is_building_path()
        || !matches!(
            editor.mode,
            editing::Mode::Edit | editing::Mode::Draw | editing::Mode::Annotate
        )
        || (editor.mode != editing::Mode::Edit && editor.tool != editing::Tool::Select)
        || egui::Popup::is_any_open(ui.ctx())
    {
        return;
    }
    let pressed = ui.input(|i| i.pointer.primary_pressed());
    let clicked = ui.input(|i| i.pointer.primary_clicked());
    if !pressed && !clicked {
        return;
    }
    let Some(pos) = ui.ctx().pointer_interact_pos().filter(|p| {
        rect.intersect(ui.clip_rect()).contains(*p)
            && ui.ctx().layer_id_at(*p) == Some(ui.layer_id())
    }) else {
        return;
    };
    let p = (pos - rect.min) / scale;
    let point = [p.x, p.y];
    let Some(session) = &tab.session else { return };
    let key = (tab.id, session.revision(), page);
    if let Some(draft) = &state.existing {
        let active = Rect::from_min_max(
            rect.min + vec2(draft.rect[0], draft.rect[1]) * scale,
            rect.min + vec2(draft.rect[2], draft.rect[3]) * scale,
        );
        // Typing and its move/resize handles keep their input. A click on
        // another object commits the draft first, then resolves that point
        // against the new revision so old image/text indices cannot leak.
        if clicked
            && draft.key.0 == tab.id
            && !(draft.key.2 == page
                && (active.expand(6.).contains(pos)
                    || text_move_handle(active, rect).expand(6.).contains(pos)))
            && !candidates(state, tab, page)
                .iter()
                .any(|text| contains(text.rect, point))
            && (editing::hit_annotation(tab, page, point, scale).is_some()
                || state.inspection.as_ref().is_some_and(|info| {
                    info.key == key
                        && info.images.as_ref().is_ok_and(|images| {
                            images.iter().any(|image| contains(image.rect, point))
                        })
                }))
        {
            state.after_text = Some(AfterText::SelectAt(tab.id, page, point));
            state.new_commit = true;
        }
        return;
    }
    if let Some(a) = editing::hit_annotation(tab, page, point, scale) {
        // A note is a comment, not an empty drawing or underlying PDF text.
        // Keep the pointer press for page gestures, and open its body on click.
        if matches!(a.kind, Kind::Note { .. }) && !ui.input(|i| i.modifiers.shift) {
            if clicked {
                selection::set(tab, editor, vec![a.id.clone()]);
                editor.note_requested = Some((tab.id, a.id));
            }
            return;
        }
        // Keep the text widget alive through press/release so it receives focus
        // before the next keystroke. Edit already handles these boxes in place.
        if matches!(a.kind, Kind::Text { .. })
            && editor.mode == editing::Mode::Edit
            && !state.image_mode
        {
            if clicked {
                *inspector = true;
            }
            return;
        }
        if clicked
            && editor.mode == editing::Mode::Edit
            && matches!(a.kind, Kind::Text { .. })
            && !a.object.locked
            && session.editable
        {
            state.context.command = Some((
                "edit_text_here",
                Menu {
                    key,
                    point,
                    target: Target::Artwork(vec![a]),
                },
            ));
        } else if pressed {
            if !ui.input(|i| i.modifiers.shift) && !selection::selected_ids(tab).contains(&a.id) {
                selection::set(tab, editor, vec![a.id]);
            }
            *inspector = true;
        }
        return;
    }
    if editor.mode != editing::Mode::Edit {
        return;
    }
    // Active move/resize handles are outside the artwork's painted area.
    if !selection::selected_ids(tab).is_empty() && editor.mode == editing::Mode::Draw && pressed {
        return;
    }
    if let Some(info) = state.inspection.as_ref().filter(|i| i.key == key) {
        let texts = candidates(state, tab, page);
        if let Some(d) = texts.iter().rev().find(|d| contains(d.rect, point)) {
            if clicked && session.editable {
                let draft = line_at(d.clone(), &texts, tab.pages[page]);
                state.context.command = Some((
                    "edit_text_here",
                    Menu {
                        key,
                        point,
                        target: Target::Text(Box::new(draft)),
                    },
                ));
            }
            return;
        }
        if pressed
            && let Ok(images) = &info.images
            && let Some((i, image)) = images
                .iter()
                .enumerate()
                .rev()
                .find(|(_, im)| contains(im.rect, point))
        {
            let r = image.rect;
            state.image_mode = true;
            state.selected = Some(i);
            state.rect = [r[0], r[1], r[2] - r[0], r[3] - r[1]];
            selection::set(tab, editor, vec![]);
            editor.mode = editing::Mode::Edit;
            *inspector = true;
        }
    }
}

pub(in crate::desktop) fn page_menu(
    state: &mut ContentState,
    editor: &mut editing::EditorState,
    tab: &mut Tab,
    ui: &mut egui::Ui,
    page: usize,
    rect: Rect,
    scale: f32,
) {
    if !matches!(
        editor.mode,
        editing::Mode::Read | editing::Mode::Annotate | editing::Mode::Draw | editing::Mode::Edit
    ) || state.busy
    {
        return;
    }
    let Some(s) = &tab.session else { return };
    let key = (tab.id, s.revision(), page);
    let editable = s.editable;
    let ctx = ui.ctx();
    let id = egui::Id::new((tab.id, page, "contextual_page_menu"));
    let click = ctx
        .input(|i| i.pointer.secondary_clicked())
        .then(|| ctx.pointer_interact_pos())
        .flatten()
        .filter(|p| {
            rect.intersect(ui.clip_rect()).contains(*p)
                && ctx.layer_id_at(*p) == Some(ui.layer_id())
        });
    let mut open = None;
    if let Some(pos) = click {
        let p = (pos - rect.min) / scale;
        // The live TextEdit owns its menu, preserving its character selection.
        let in_editor = state
            .existing
            .as_ref()
            .is_some_and(|d| d.key == key && contains(d.rect, [p.x, p.y]))
            || (state.new_placed
                && state.add_target == Some((tab.id, page))
                && contains(
                    [
                        state.new_rect[0],
                        state.new_rect[1],
                        state.new_rect[0] + state.new_rect[2],
                        state.new_rect[1] + state.new_rect[3],
                    ],
                    [p.x, p.y],
                ));
        if !in_editor {
            let target = target_at(state, editor, tab, page, [p.x, p.y], scale);
            state.context.menu = Some(Menu {
                key,
                point: [p.x, p.y],
                target,
            });
            state.context.group = None;
            open = Some(egui::SetOpenCommand::Bool(true));
        }
    }
    let Some(menu) = state
        .context
        .menu
        .as_ref()
        .filter(|m| m.key == key)
        .cloned()
    else {
        egui::Popup::close_id(ctx, id);
        return;
    };
    egui::Popup::new(id, ctx.clone(), rect, ui.layer_id())
        .kind(egui::PopupKind::Menu)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .frame(egui::Frame::popup(ui.style()).fill(theme::paper()))
        .style(egui::containers::menu::menu_style)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .open_memory(open)
        .at_pointer_fixed()
        .show(|ui| {
            let mut entries = items(&menu.target, editable);
            if editor.mode != editing::Mode::Edit
                || state.table_selection.is_none_or(|(k, _)| k != key)
            {
                entries.retain(|e| e.action != "table_area");
            }
            let mut labels = entries
                .iter()
                .filter_map(|entry| match (state.context.group, submenu(entry.action)) {
                    (Some(group), Some((name, _))) if group == name => {
                        Some((entry.label, shortcut(entry.action)))
                    }
                    (None, Some((name, _))) => Some((name, None)),
                    (None, None) => Some((entry.label, shortcut(entry.action))),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if state.context.group.is_some() {
                labels.push(("Back to actions", None));
            }
            let width = ui::menu_width(ui, labels);
            menu_layout(ui, width);
            egui::ScrollArea::vertical()
                .id_salt("context_actions_scroll")
                .max_height((ctx.content_rect().height() - 40.).max(80.))
                .show(ui, |ui| {
                    if let Some(group) = state.context.group {
                        if ui
                            .add(ui::MenuItem::new(Icon::Left, "Back to actions").width(width))
                            .clicked()
                        {
                            state.context.group = None;
                        }
                        ui.separator();
                        ui.weak(group);
                        for entry in entries
                            .iter()
                            .filter(|e| submenu(e.action).is_some_and(|g| g.0 == group))
                        {
                            if action_row(ui, entry, width) {
                                state.context.command = Some((entry.action, menu.clone()));
                                state.context.group = None;
                                ui.close();
                            }
                        }
                        return;
                    }
                    let mut shown_groups = std::collections::HashSet::new();
                    for i in &entries {
                        if let Some((label, icon)) = submenu(i.action) {
                            if !shown_groups.insert(label) {
                                continue;
                            }
                            let response = ui.add(ui::MenuItem::new(icon, label).width(width));
                            ui::draw_icon(
                                ui.painter(),
                                Rect::from_center_size(
                                    pos2(response.rect.right() - 12., response.rect.center().y),
                                    vec2(12., 12.),
                                ),
                                Icon::Right,
                                ui.visuals().text_color(),
                            );
                            if response.clicked() {
                                state.context.group = Some(label);
                            }
                            continue;
                        }
                        if matches!(
                            i.action,
                            "object_copy"
                                | "rotate_right"
                                | "front"
                                | "object_delete"
                                | "delete_image"
                                | "add_text_here"
                        ) {
                            ui.separator();
                        }
                        if action_row(ui, i, width) {
                            state.context.command = Some((i.action, menu.clone()));
                            ui.close();
                        }
                    }
                });
        });
}

// egui's text cursor treats any pointer press as a caret move. Keep the
// pre-click selection on secondary press so menu actions affect those words.
pub(super) fn secondary_selection(
    ctx: &egui::Context,
    id: egui::Id,
) -> Option<egui::text::CCursorRange> {
    ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary))
        .then(|| egui::TextEdit::load_state(ctx, id)?.cursor.char_range())
        .flatten()
}
pub(super) fn restore_secondary_selection(
    output: &mut egui::text_edit::TextEditOutput,
    range: Option<egui::text::CCursorRange>,
) {
    if let Some(range) = range {
        output.cursor_range = Some(range);
        output.state.cursor.set_char_range(Some(range));
        output
            .state
            .clone()
            .store(&output.response.ctx, output.response.id);
    }
}

/// Inline editors cannot borrow the Viewer while laying out their TextEdit.
/// Capture the chosen characters for the translation panel without committing
/// the draft, changing focus, or retaining the document beyond the next frame.
pub(crate) fn take_translation_text(ctx: &egui::Context) -> Option<String> {
    ctx.data_mut(|data| {
        let id = egui::Id::new("inline_translation_request");
        let text = data.get_temp::<String>(id);
        data.remove::<String>(id);
        text
    })
}

/// Text commands operate on the TextEdit's captured Unicode character range.
/// Secondary clicks do not replace that range with the whole block.
pub(super) fn text_menu(
    output: &mut egui::text_edit::TextEditOutput,
    text: &mut String,
    done: &mut bool,
    properties: &mut bool,
    context: &mut State,
) {
    use egui::text::{CCursor, CCursorRange};
    let ctx = output.response.ctx.clone();
    let range = output
        .cursor_range
        .or_else(|| output.state.cursor.char_range())
        .unwrap_or_default();
    let [lo, hi] = range.sorted_cursors();
    let selected = lo.index < hi.index;
    let mut action = None;
    egui::Popup::context_menu(&output.response)
        .frame(egui::Frame::popup(&ctx.style()).fill(theme::paper()))
        .show(|ui| {
            let entries = [
                ("Cut", "cut", selected),
                ("Copy", "copy", selected),
                ("Translate selected text…", "translate_selection", selected),
                ("Paste", "paste", true),
                ("Delete selected text", "delete", selected),
                ("Delete text box", "delete_text_box", true),
                ("Select all text", "all", !text.is_empty()),
                ("Text properties…", "properties", true),
                ("Detect table in selection", "table_area", true),
                ("Finish text edit", "done", true),
                ("Cancel text edit", "cancel", true),
            ];
            let width = ui::menu_width(
                ui,
                entries
                    .iter()
                    .map(|(label, name, _)| (*label, shortcut(name))),
            );
            menu_layout(ui, width);
            egui::ScrollArea::vertical()
                .id_salt("text_context_actions_scroll")
                .max_height((ctx.content_rect().height() - 40.).max(80.))
                .show(ui, |ui| {
                    for (label, name, enabled) in entries {
                        if ui
                            .add_enabled(
                                enabled,
                                ui::MenuItem::new(action_icon(name, label), label)
                                    .shortcut(shortcut(name))
                                    .width(width),
                            )
                            .clicked()
                        {
                            action = Some(name);
                            ui.close();
                        }
                    }
                });
        });
    let Some(action) = action else { return };
    output.response.request_focus();
    output.cursor_range = Some(range);
    match action {
        "table_area" => context.detect_table_area = true,
        "done" => *done = true,
        "cancel" => context.cancel_text = true,
        "delete_text_box" => context.delete_text = true,
        "properties" => *properties = true,
        "translate_selection" => {
            let selection: String = text
                .chars()
                .skip(lo.index)
                .take(hi.index - lo.index)
                .collect();
            if !selection.trim().is_empty() {
                ctx.data_mut(|data| {
                    data.insert_temp(egui::Id::new("inline_translation_request"), selection);
                });
            }
        }
        "all" => {
            output.cursor_range = Some(CCursorRange::two(
                CCursor::new(0),
                CCursor::new(text.chars().count()),
            ));
        }
        "copy" | "cut" | "delete" | "paste" => {
            if action == "copy" || action == "cut" {
                ctx.copy_text(
                    text.chars()
                        .skip(lo.index)
                        .take(hi.index - lo.index)
                        .collect(),
                );
            }
            let replacement = match action {
                "cut" | "delete" => Some(String::new()),
                "paste" => match clipboard::read() {
                    clipboard::Clip::Text(t) => Some(t),
                    clip @ clipboard::Clip::Image(..) => {
                        context.image_paste = Some(clip);
                        None
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some(replacement) = replacement {
                replace_selection(output, text, range, &replacement);
            }
        }
        _ => {}
    }
    output.state.cursor.set_char_range(output.cursor_range);
    output.state.clone().store(&ctx, output.response.id);
    ctx.request_repaint();
}
fn replace_selection(
    output: &mut egui::text_edit::TextEditOutput,
    text: &mut String,
    range: egui::text::CCursorRange,
    replacement: &str,
) {
    use egui::text::{CCursor, CCursorRange};
    let [lo, hi] = range.sorted_cursors();
    let byte = |n| text.char_indices().nth(n).map_or(text.len(), |(b, _)| b);
    let (start, end) = (byte(lo.index), byte(hi.index));
    let available = 32768usize.saturating_sub(text.len() - (end - start));
    let mut replacement = replacement.to_owned();
    let mut end_byte = replacement.len().min(available);
    while !replacement.is_char_boundary(end_byte) {
        end_byte -= 1;
    }
    replacement.truncate(end_byte);
    let mut undo = output.state.undoer();
    undo.add_undo(&(range, text.clone()));
    text.replace_range(start..end, &replacement);
    output.cursor_range = Some(CCursorRange::one(CCursor::new(
        lo.index + replacement.chars().count(),
    )));
    undo.add_undo(&(output.cursor_range.unwrap(), text.clone()));
    output.state.set_undoer(undo);
    output.response.mark_changed();
}

impl Viewer {
    fn shortcut_context(&self, ctx: &egui::Context) -> Option<(Menu, bool)> {
        if self.home.visible
            || self.content.busy
            || self.organizer.busy
            || self.digital.open
            || self.editor.signature_open
            || self.editor.library_open
            || self.recovery.open
            || self.close_intent.is_some()
            || self.accessible_text
            || self.review.open
            || self.navigation_editor.is_some()
            || self.workspace.options
            || self.workbench_owns_shortcuts()
            || ctx.memory(|m| m.top_modal_layer().is_some())
        {
            return None;
        }
        let tab = self.tabs.get(self.active)?;
        let session = tab.session.as_ref()?;
        if let Some(menu) = self.content.context.menu.as_ref()
            && menu.key.0 == tab.id
            && menu.key.1 == session.revision()
            && egui::Popup::is_id_open(
                ctx,
                egui::Id::new((tab.id, menu.key.2, "contextual_page_menu")),
            )
        {
            return Some((menu.clone(), true));
        }
        if ctx.wants_keyboard_input()
            || self.editor.text_editing
            || self.content.pending_text().is_some()
            || egui::Popup::is_any_open(ctx)
            || self.editor.interaction_active()
        {
            return None;
        }
        let target = if let Some(text) = self.editor.text_selection
            && matches!(
                self.editor.mode,
                editing::Mode::Read | editing::Mode::Annotate
            )
            && text.tab == tab.id
            && text.page == tab.page
            && text.text(tab).is_some()
        {
            Target::SelectedText(text)
        } else if matches!(
            self.editor.mode,
            editing::Mode::Draw | editing::Mode::Annotate | editing::Mode::Edit
        ) {
            let ids = selection::selected_ids(tab);
            let marks: Vec<_> = session
                .annotations
                .iter()
                .filter(|a| ids.contains(&a.id) && a.page == tab.page && !a.object.hidden)
                .cloned()
                .collect();
            if !marks.is_empty() {
                Target::Artwork(marks)
            } else if self.editor.mode == editing::Mode::Edit && self.content.image_mode {
                let inspection = self
                    .content
                    .inspection
                    .as_ref()
                    .filter(|i| i.key == (tab.id, session.revision(), tab.page))?;
                Target::Image(
                    inspection
                        .images
                        .as_ref()
                        .ok()?
                        .get(self.content.selected?)?
                        .clone(),
                )
            } else {
                return None;
            }
        } else if self.editor.mode == editing::Mode::Read {
            Target::Page
        } else {
            return None;
        };
        Some((
            Menu {
                key: (tab.id, session.revision(), tab.page),
                point: [0., 0.],
                target,
            },
            false,
        ))
    }

    fn run_context_shortcut(
        &mut self,
        ctx: &egui::Context,
        menu: Menu,
        action: &'static str,
        enabled: bool,
    ) {
        egui::Popup::close_id(
            ctx,
            egui::Id::new((menu.key.0, menu.key.2, "contextual_page_menu")),
        );
        if enabled {
            self.content.context.command = Some((action, menu));
            self.context_commands(ctx);
        }
    }

    /// Native Edit-menu accelerators use the same captured target as a context
    /// menu click, including when the pointer is on a different object.
    pub(in crate::desktop) fn context_native_action(
        &mut self,
        action: &str,
        ctx: &egui::Context,
    ) -> bool {
        let aliases: &[&str] = match action {
            "copy" => &["object_copy", "copy_text", "copy_image"],
            "cut" => &["object_cut", "cut_image"],
            "select_all" => &["all_text"],
            "paste" => &["paste_here"],
            "paste_original" => &["paste_original"],
            "context_copy_format" => &["copy_format"],
            "context_paste_format" => &["paste_format"],
            _ => return false,
        };
        let Some((menu, popup)) = self.shortcut_context(ctx) else {
            return false;
        };
        if !popup && matches!(action, "paste" | "paste_original") {
            return false;
        }
        let editable = self.tabs[self.active].session.as_ref().unwrap().editable;
        if let Some(entry) = items(&menu.target, editable)
            .into_iter()
            .find(|e| aliases.contains(&e.action))
        {
            consume_action(ctx, entry.action);
            self.run_context_shortcut(ctx, menu, entry.action, entry.enabled);
            return true;
        }
        false
    }

    pub(in crate::desktop) fn context_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        let Some((menu, popup)) = self.shortcut_context(ctx) else {
            return false;
        };
        let editable = self.tabs[self.active].session.as_ref().unwrap().editable;
        for entry in items(&menu.target, editable) {
            // Ordinary paste still starts its placement preview. The menu's
            // Paste Here shortcut applies only while that menu is open.
            if !popup && matches!(entry.action, "paste_here" | "paste_original") {
                continue;
            }
            if consume_action(ctx, entry.action) {
                self.run_context_shortcut(ctx, menu, entry.action, entry.enabled);
                return true;
            }
        }
        false
    }

    pub(in crate::desktop) fn context_commands(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.content.context.detect_table_area) {
            self.table_from_selection();
        }

        if let Some((id, page, area)) = self.content.context.area_after_table {
            let active = self
                .tabs
                .get(self.active)
                .and_then(|t| t.session.as_ref().map(|s| (t.id, s.revision(), t.page)));
            if self.editor.mode != editing::Mode::Edit
                || active.is_none_or(|k| k.0 != id || k.2 != page)
            {
                self.content.context.area_after_table = None;
            } else if !self.content.busy
                && self
                    .content
                    .inspection
                    .as_ref()
                    .is_some_and(|i| Some(i.key) == active)
            {
                self.content.context.area_after_table = None;
                let tab = &self.tabs[self.active];
                let rect = Rect::from_min_max(pos2(area[0], area[1]), pos2(area[2], area[3]));
                let selected: Vec<_> = candidates(&self.content, tab, page)
                    .into_iter()
                    .filter(|d| {
                        rect.contains(pos2(
                            (d.rect[0] + d.rect[2]) / 2.,
                            (d.rect[1] + d.rect[3]) / 2.,
                        ))
                    })
                    .collect();
                if selected.len() > 2000
                    || selected.iter().map(|d| d.text.len() + 1).sum::<usize>() > 32_768
                {
                    self.content.selection_hint = Some(
                        "Select a smaller block (up to 2,000 objects or 32 KB of text).".into(),
                    );
                } else {
                    self.content.existing = selected_block(selected, area, tab.pages[page]);
                }
                self.content.table_selection = Some((active.unwrap(), area));
                self.content.area_mode = false;
                self.inspector = true;
            }
        }
        if let Some((id, page, point)) = self.content.context.select_after_text {
            let current = self.tabs.get(self.active).and_then(|tab| {
                tab.session
                    .as_ref()
                    .map(|session| (tab.id, session.revision(), tab.page))
            });
            if current.is_none_or(|key| key.0 != id || key.2 != page) {
                self.content.context.select_after_text = None;
            } else if !self.content.busy
                && self
                    .content
                    .inspection
                    .as_ref()
                    .is_some_and(|info| Some(info.key) == current)
            {
                self.content.context.select_after_text = None;
                let target = target_at(
                    &self.content,
                    &mut self.editor,
                    &mut self.tabs[self.active],
                    page,
                    point,
                    1.,
                );
                let action = match &target {
                    Target::Text(_) => Some("edit_text_here"),
                    Target::Image(_) => Some("image_properties"),
                    Target::Artwork(_) => Some("object_properties"),
                    _ => None,
                };
                if let Some(action) = action {
                    self.content.context.command = Some((
                        action,
                        Menu {
                            key: current.unwrap(),
                            point,
                            target,
                        },
                    ));
                }
            }
        }
        if let Some(clip) = self.content.context.image_paste.take() {
            self.paste_image_over_text(ctx, clip);
        }
        if std::mem::take(&mut self.content.context.delete_text) {
            self.delete_live_text(ctx);
        }
        if std::mem::take(&mut self.content.context.cancel_text) {
            self.content.existing = None;
            self.content.next_existing = None;
            self.content.clear_new_text();
            self.content.new_placed = false;
            self.content.next_text = None;
            self.content.new_commit = false;
            self.content.after_text = None;
        }
        let Some((action, menu)) = self.content.context.command.take() else {
            return;
        };
        let Some(index) = self.tabs.iter().position(|t| {
            t.id == menu.key.0
                && t.session
                    .as_ref()
                    .is_some_and(|s| s.revision() == menu.key.1)
        }) else {
            return;
        };
        if self.content.busy || self.organizer.busy {
            return;
        }
        self.active = index;
        self.tabs[index].page = menu.key.2;
        if !items(
            &menu.target,
            self.tabs[index].session.as_ref().unwrap().editable,
        )
        .iter()
        .any(|i| i.action == action && i.enabled)
        {
            return;
        }
        if action == "finish_text" {
            self.content.new_commit = true;
            return;
        }
        if action == "cancel_text" {
            self.content.context.cancel_text = true;
            return;
        }
        if action == "delete_text_box" {
            self.delete_live_text(ctx);
            return;
        }
        if self.content.pending_text().is_some() {
            return;
        }
        if matches!(
            action,
            "edit_text_here"
                | "text_area"
                | "add_text_here"
                | "add_image"
                | "copy_region"
                | "image_properties"
                | "replace_image"
                | "image_background"
                | "delete_image"
                | "cut_image"
                | "object_properties"
        ) {
            self.redaction.active = false;
            self.content.region.active = false;
            self.content.region.drag = None;
            self.content.add_target = None;
            self.content.new_placed = false;
            self.content.marquee = None;
        }
        if let Target::Artwork(marks) = &menu.target {
            selection::set(
                &mut self.tabs[index],
                &mut self.editor,
                marks.iter().map(|a| a.id.clone()).collect(),
            );
        }
        match action {
            "table_area" => self.table_from_selection(),
            "edit_table_here" => {
                self.editor.mode = editing::Mode::Edit;
                self.open_table_at(menu.point);
            }
            "translate_selection" => {
                if let Target::SelectedText(selection) = menu.target {
                    self.editor.text_selection = Some(selection);
                    if let Some(text) = selection.text(&self.tabs[index]) {
                        let anchor = selection.rects(&self.tabs[index]).first().copied();
                        self.translate_text(ctx, text, "Selected text".into());
                        if let Some(anchor) = anchor {
                            self.set_translation_anchor(anchor, None);
                        }
                    }
                }
            }
            "translate_text" => {
                let text = match &menu.target {
                    Target::Text(draft) => Some(draft.text.clone()),
                    Target::Artwork(marks) => {
                        marks.first().and_then(|a| a.kind.text().map(str::to_owned))
                    }
                    _ => None,
                };
                if let Some(text) = text {
                    let anchor = match &menu.target {
                        Target::Text(draft) => Some((
                            draft.rect,
                            match &draft.target {
                                InlineTarget::Annotation(id) => Some(id.clone()),
                                _ => None,
                            },
                        )),
                        Target::Artwork(marks) => {
                            marks.first().map(|a| (a.bounds(), Some(a.id.clone())))
                        }
                        _ => None,
                    };
                    self.translate_text(ctx, text, "Selected text".into());
                    if let Some((anchor, object)) = anchor {
                        self.set_translation_anchor(anchor, object);
                    }
                }
            }
            "review_comments" => {
                self.review.open = true;
                if let Target::Artwork(m) = &menu.target {
                    self.review.selected = m.first().map(|a| a.id.clone());
                }
            }
            "select_behind" => {
                let t = &mut self.tabs[index];
                let s = t.session.as_ref().unwrap();
                let hits: Vec<_> = s
                    .annotations
                    .iter()
                    .rev()
                    .filter(|a| {
                        a.page == menu.key.2
                            && !a.object.hidden
                            && a.hit_test(menu.point, 5. / t.scale.max(0.1))
                    })
                    .collect();
                let selected = selection::selected_ids(t);
                let next = hits
                    .iter()
                    .position(|a| selected.contains(&a.id))
                    .map_or(0, |i| (i + 1) % hits.len().max(1));
                if let Some(a) = hits.get(next) {
                    let id = a.id.clone();
                    selection::set(t, &mut self.editor, vec![id]);
                    self.editor.mode = editing::Mode::Draw;
                    self.inspector = true;
                }
            }
            "copy_text" | "copy_artwork_text" => {
                let text = match &menu.target {
                    Target::SelectedText(s) => s.text(&self.tabs[index]),
                    Target::Text(d) => Some(d.text.clone()),
                    Target::Artwork(m) => m.first().and_then(|a| a.kind.text().map(str::to_owned)),
                    _ => None,
                };
                if let Some(text) = text {
                    ctx.copy_text(text);
                }
            }
            "all_text" => {
                self.editor.text_selection =
                    text_selection::TextSelection::all(&self.tabs[index], menu.key.2);
                self.editor.mode = editing::Mode::Read;
            }
            "highlight" | "underline" | "strikeout" => {
                if let Target::SelectedText(s) = &menu.target {
                    let quads = s.rects(&self.tabs[index]);
                    if !quads.is_empty() {
                        let rect = quads.iter().fold(
                            [
                                f32::INFINITY,
                                f32::INFINITY,
                                f32::NEG_INFINITY,
                                f32::NEG_INFINITY,
                            ],
                            |a, b| {
                                [
                                    a[0].min(b[0]),
                                    a[1].min(b[1]),
                                    a[2].max(b[2]),
                                    a[3].max(b[3]),
                                ]
                            },
                        );
                        let quads = quads
                            .into_iter()
                            .map(|q| {
                                [
                                    (q[0] - rect[0]) / (rect[2] - rect[0]).max(0.01),
                                    (q[1] - rect[1]) / (rect[3] - rect[1]).max(0.01),
                                    (q[2] - rect[0]) / (rect[2] - rect[0]).max(0.01),
                                    (q[3] - rect[1]) / (rect[3] - rect[1]).max(0.01),
                                ]
                            })
                            .collect();
                        let mut style = self.editor.style.clone();
                        if action == "highlight" {
                            style.color = [1., 0.85, 0.2];
                            style.opacity = 0.35;
                        }
                        let mark = Annotation::new(
                            menu.key.2,
                            rect,
                            Kind::Markup {
                                kind: match action {
                                    "highlight" => Markup::Highlight,
                                    "underline" => Markup::Underline,
                                    _ => Markup::Strikeout,
                                },
                                quads,
                            },
                            style,
                        );
                        let t = &mut self.tabs[index];
                        let session = t.session.as_mut().unwrap();
                        let mut entries = session.annotations.clone();
                        entries.push(mark.clone());
                        match session.commit(entries) {
                            Ok(()) => {
                                selection::set(t, &mut self.editor, vec![mark.id]);
                                self.editor.text_selection = None;
                            }
                            Err(e) => self.status = Some((e.to_string(), true)),
                        }
                    }
                }
            }
            "edit_text_here" => {
                let draft = match &menu.target {
                    Target::Text(d) => Some((**d).clone()),
                    Target::Artwork(m) => {
                        let texts = candidates(&self.content, &self.tabs[index], menu.key.2);
                        texts.into_iter().find(
                            |d| matches!(&d.target,InlineTarget::Annotation(id) if id==&m[0].id),
                        )
                    }
                    _ => None,
                };
                if let Some(draft) = draft {
                    self.content.existing = Some(draft);
                    self.content.image_mode = false;
                    self.content.selected = None;
                    self.content.area_mode = false;
                    self.content.add_target = None;
                    self.editor.mode = editing::Mode::Edit;
                    self.content.show_text_inspector = true;
                    self.inspector = true;
                }
            }
            "text_area" => {
                self.editor.mode = editing::Mode::Edit;
                self.content.image_mode = false;
                self.content.area_mode = true;
                self.inspector = true;
            }
            "add_text_here" => {
                self.editor.mode = editing::Mode::Edit;
                self.content.image_mode = false;
                self.content.area_mode = false;
                let t = &self.tabs[index];
                self.content
                    .place_text(t.id, menu.key.2, menu.point, t.pages[menu.key.2]);
                self.inspector = true;
            }
            "copy_region" => {
                self.editor.mode = editing::Mode::Edit;
                self.content.image_mode = true;
                self.content.add_target = None;
                self.content.region.active = true;
                self.content.region.drag = None;
                self.content.region.key = Some(menu.key);
                self.inspector = true;
            }
            "paste_here" => {
                self.paste(ctx);
                if let Some(marks) = self.workbench.paste.take() {
                    let t = &mut self.tabs[index];
                    let copies =
                        workbench::positioned(&marks, menu.key.2, menu.point, t.pages[menu.key.2]);
                    workbench::place(&mut self.editor, t, copies, &mut self.status);
                } else if self.content.new_placed
                    && self.content.add_target == Some((menu.key.0, menu.key.2))
                {
                    self.content.new_rect[0] = menu.point[0].min(
                        (self.tabs[index].pages[menu.key.2].width - self.content.new_rect[2])
                            .max(0.),
                    );
                    self.content.new_rect[1] = menu.point[1].min(
                        (self.tabs[index].pages[menu.key.2].height - self.content.new_rect[3])
                            .max(0.),
                    );
                }
            }
            "object_properties" => {
                if let Target::Artwork(marks) = &menu.target
                    && marks.len() == 1
                    && matches!(marks[0].kind, Kind::Note { .. })
                {
                    self.open_note_reader(menu.key.0, &marks[0].id);
                } else {
                    self.editor.mode = editing::Mode::Draw;
                    self.editor.tool = editing::Tool::Select;
                    self.inspector = true;
                }
            }
            "rotate_right" | "rotate_left" | "flip_horizontal" | "flip_vertical" | "front"
            | "back" | "lock" => {
                if let Target::Artwork(marks) = &menu.target {
                    let t = &mut self.tabs[index];
                    let s = t.session.as_mut().unwrap();
                    let ids: Vec<_> = marks.iter().map(|a| a.id.clone()).collect();
                    let mut entries = s.annotations.clone();
                    if action == "lock" {
                        let lock = !marks.iter().any(|a| a.object.locked);
                        for a in entries.iter_mut().filter(|a| ids.contains(&a.id)) {
                            a.object.locked = lock;
                        }
                    } else if action == "front" || action == "back" {
                        entries.retain(|a| !ids.contains(&a.id));
                        if action == "front" {
                            entries.extend(marks.clone());
                        } else {
                            entries.splice(0..0, marks.clone());
                        }
                    } else {
                        let transform = match action {
                            "rotate_left" => selection::Transform::Rotate(-90.),
                            "flip_horizontal" => selection::Transform::FlipHorizontal,
                            "flip_vertical" => selection::Transform::FlipVertical,
                            _ => selection::Transform::Rotate(90.),
                        };
                        let transformed = selection::transformed(marks, transform);
                        for a in &mut entries {
                            if let Some(replacement) = transformed.iter().find(|m| m.id == a.id) {
                                *a = replacement.clone();
                            }
                        }
                    }
                    match s.commit(entries) {
                        Ok(()) => selection::set(t, &mut self.editor, ids),
                        Err(e) => self.status = Some((e.to_string(), true)),
                    }
                }
            }
            "image_properties" | "copy_image" | "cut_image" | "replace_image"
            | "image_background" | "delete_image" => {
                if let Target::Image(image) = menu.target {
                    self.editor.mode = editing::Mode::Edit;
                    self.content.image_mode = true;
                    self.content.area_mode = false;
                    self.inspector = true;
                    self.content.selected = self
                        .content
                        .inspection
                        .as_ref()
                        .and_then(|i| i.images.as_ref().ok())
                        .and_then(|images| images.iter().position(|i| i.index == image.index));
                    let r = image.rect;
                    self.content.rect = [r[0], r[1], r[2] - r[0], r[3] - r[1]];
                    match action {
                        "delete_image" => self.content_change(
                            ctx,
                            Edit::Image {
                                page: menu.key.2 as u32 + 1,
                                index: image.index,
                                edit: content::ImageEdit::Delete,
                            },
                        ),
                        "image_background" => self.remove_content_image_background(image.index),
                        "replace_image" => {
                            if let Some(path) = self.native_pick(
                                rfd::FileDialog::new()
                                    .set_title("Replace image")
                                    .add_filter("Images", &["png", "jpg", "jpeg"]),
                            ) {
                                match convert::read_bounded(&path, 32 * 1024 * 1024) {
                                    Ok(bytes) => self.content_change(
                                        ctx,
                                        Edit::Image {
                                            page: menu.key.2 as u32 + 1,
                                            index: image.index,
                                            edit: content::ImageEdit::Replace(bytes),
                                        },
                                    ),
                                    Err(e) => self.status = Some((e.to_string(), true)),
                                }
                            }
                        }
                        "copy_image" | "cut_image" => {
                            match content::image_png(
                                &self.tabs[index].session.as_ref().unwrap().source,
                                menu.key.2 as u32 + 1,
                                image.index,
                            ) {
                                Ok(png) => {
                                    let copied = self.copy_artwork(
                                        ctx,
                                        vec![Annotation::new(
                                            menu.key.2,
                                            image.rect,
                                            Kind::Image { png: Arc::new(png) },
                                            Default::default(),
                                        )],
                                    );
                                    if copied.is_some() && action == "cut_image" {
                                        self.content_change(
                                            ctx,
                                            Edit::Image {
                                                page: menu.key.2 as u32 + 1,
                                                index: image.index,
                                                edit: content::ImageEdit::Delete,
                                            },
                                        );
                                    } else if copied.is_some() {
                                        self.status = Some((
                                            format!(
                                                "Copied selected image · {} × {} pixels",
                                                image.pixels[0], image.pixels[1]
                                            ),
                                            false,
                                        ));
                                    }
                                }
                                Err(e) => {
                                    self.status = Some((
                                        format!(
                                            "Could not copy the selected image: {e}. The clipboard was not changed."
                                        ),
                                        true,
                                    ))
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => self.action(action, ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::annotations::Style;
    fn press_shortcut(
        app: &mut Viewer,
        ctx: &egui::Context,
        action: &str,
        clipboard: bool,
    ) -> egui::FullOutput {
        let key = shortcut(action).unwrap();
        let event = if clipboard {
            match key.logical_key {
                egui::Key::C => egui::Event::Copy,
                egui::Key::V => egui::Event::Paste(String::new()),
                _ => unreachable!(),
            }
        } else {
            egui::Event::Key {
                key: key.logical_key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: key.modifiers,
            }
        };
        ctx.run(
            egui::RawInput {
                modifiers: key.modifiers,
                events: vec![event],
                ..Default::default()
            },
            |ctx| app.shortcuts(ctx),
        )
    }

    #[test]
    fn shortcuts_copy_select_all_and_mark_up_page_text() {
        use mypdf_milestone0::viewer::{GlyphBox, PageText};
        let (mut app, ctx) = fixture();
        app.editor.mode = editing::Mode::Read;
        app.tabs[0].text[0] = Some(Arc::new(PageText {
            text: "One two".into(),
            limited: false,
            glyphs: (0..7)
                .map(|i| GlyphBox {
                    range: i..i + 1,
                    rect: [20. + i as f32 * 8., 50., 28. + i as f32 * 8., 62.],
                })
                .collect(),
        }));
        press_shortcut(&mut app, &ctx, "all_text", false);
        assert_eq!(
            app.editor
                .text_selection
                .unwrap()
                .text(&app.tabs[0])
                .as_deref(),
            Some("One two")
        );
        let out = press_shortcut(&mut app, &ctx, "copy_text", true);
        assert!(
            out.platform_output
                .commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t=="One two"))
        );
        for (action, expected) in [
            ("highlight", Markup::Highlight),
            ("underline", Markup::Underline),
            ("strikeout", Markup::Strikeout),
        ] {
            app.editor.text_selection =
                text_selection::TextSelection::word_at(&app.tabs[0], 0, [25., 55.]);
            press_shortcut(&mut app, &ctx, action, false);
            let mark = app.tabs[0]
                .session
                .as_ref()
                .unwrap()
                .annotations
                .last()
                .unwrap();
            assert!(matches!(&mark.kind,Kind::Markup {kind,..} if *kind==expected));
            assert_eq!(mark.rect, [20., 50., 44., 62.]);
        }
        // A Read selection must not take Copy/Duplicate away from artwork after
        // switching modes and clicking an object.
        app.editor.text_selection =
            text_selection::TextSelection::word_at(&app.tabs[0], 0, [25., 55.]);
        app.editor.mode = editing::Mode::Draw;
        let id = app.tabs[0].session.as_ref().unwrap().annotations[0]
            .id
            .clone();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![id]);
        assert!(app.editor.text_selection.is_none());
        press_shortcut(&mut app, &ctx, "object_duplicate", false);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 4);
    }

    #[test]
    fn shortcuts_group_ungroup_duplicate_and_protect_locked_objects() {
        let (mut app, ctx) = fixture();
        app.editor.mode = editing::Mode::Draw;
        let marks = vec![
            Annotation::new(0, [20., 20., 80., 80.], Kind::Rectangle, Default::default()),
            Annotation::new(0, [120., 20., 180., 80.], Kind::Ellipse, Default::default()),
        ];
        let ids = marks.iter().map(|a| a.id.clone()).collect();
        app.tabs[0].session.as_mut().unwrap().commit(marks).unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, ids);
        press_shortcut(&mut app, &ctx, "group_objects", false);
        assert!(
            app.tabs[0]
                .session
                .as_ref()
                .unwrap()
                .annotations
                .iter()
                .all(|a| a.object.group.is_some())
        );
        press_shortcut(&mut app, &ctx, "ungroup_objects", false);
        assert!(
            app.tabs[0]
                .session
                .as_ref()
                .unwrap()
                .annotations
                .iter()
                .all(|a| a.object.group.is_none())
        );
        press_shortcut(&mut app, &ctx, "object_duplicate", false);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 4);
        let session = app.tabs[0].session.as_mut().unwrap();
        let mut marks = session.annotations.clone();
        marks[0].object.locked = true;
        let id = marks[0].id.clone();
        session.commit(marks).unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![id]);
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        press_shortcut(&mut app, &ctx, "object_delete", false);
        press_shortcut(&mut app, &ctx, "object_duplicate", false);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
        app.tabs[0].session.as_mut().unwrap().editable = false;
        press_shortcut(&mut app, &ctx, "group_objects", false);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
    }

    #[test]
    fn shortcuts_style_clipboard_events_keep_geometry_and_respect_text_focus() {
        let (mut app, ctx) = fixture();
        app.editor.mode = editing::Mode::Draw;
        let a = Annotation::new(
            0,
            [20., 20., 80., 80.],
            Kind::Rectangle,
            Style {
                width: 6.,
                color: [1., 0., 0.],
                ..Default::default()
            },
        );
        let b = Annotation::new(0, [120., 20., 180., 80.], Kind::Ellipse, Default::default());
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![a.clone(), b.clone()])
            .unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![a.id]);
        press_shortcut(&mut app, &ctx, "copy_format", true);
        selection::set(&mut app.tabs[0], &mut app.editor, vec![b.id]);
        press_shortcut(&mut app, &ctx, "paste_format", true);
        let updated = &app.tabs[0].session.as_ref().unwrap().annotations[1];
        assert_eq!(updated.style.width, 6.);
        assert_eq!(updated.style.color, [1., 0., 0.]);
        assert_eq!(updated.rect, b.rect);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 2);
        app.editor.text_editing = true;
        press_shortcut(&mut app, &ctx, "object_duplicate", false);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 2);
        app.editor.text_editing = false;
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.text_edit_singleline(&mut String::new()).request_focus();
            });
        });
        assert!(ctx.wants_keyboard_input());
        press_shortcut(&mut app, &ctx, "object_delete", false);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 2);
    }

    #[test]
    fn shortcuts_in_a_context_menu_act_on_its_target_and_close_it() {
        let (mut app, ctx) = fixture();
        let a = Annotation::new(0, [20., 20., 80., 80.], Kind::Rectangle, Default::default());
        let b = Annotation::new(
            0,
            [100., 100., 180., 180.],
            Kind::Ellipse,
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![a.clone(), b])
            .unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![a.id]);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        click(
            &mut app,
            &ctx,
            pos2(120., 150.),
            egui::PointerButton::Secondary,
        );
        assert!(egui::Popup::is_any_open(&ctx));
        press_shortcut(&mut app, &ctx, "object_duplicate", false);
        let marks = &app.tabs[0].session.as_ref().unwrap().annotations;
        assert_eq!(marks.len(), 3);
        assert!(matches!(marks.last().unwrap().kind, Kind::Ellipse));
        assert!(!egui::Popup::is_any_open(&ctx));
    }

    fn fixture() -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(
            PathBuf::from("Context menu test.pdf"),
            convert::blank(500., 500.).unwrap(),
            &ctx,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.home.visible = false;
        app.editor.mode = editing::Mode::Edit;
        (app, ctx)
    }
    fn commands(target: &Target, editable: bool) -> Vec<&'static str> {
        items(target, editable)
            .into_iter()
            .filter(|i| i.enabled)
            .map(|i| i.action)
            .collect()
    }
    fn run_command(app: &mut Viewer, ctx: &egui::Context, target: Target, action: &'static str) {
        let t = &app.tabs[0];
        app.content.context.command = Some((
            action,
            Menu {
                key: (t.id, t.session.as_ref().unwrap().revision(), 0),
                point: [240., 240.],
                target,
            },
        ));
        app.context_commands(ctx);
    }
    fn frame(app: &mut Viewer, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(800., 800.))),
                events,
                ..Default::default()
            },
            |ctx| {
                theme::sync(ctx.theme());
                egui::CentralPanel::default().show(ctx, |ui| {
                    let r = Rect::from_min_size(pos2(20., 20.), vec2(500., 500.));
                    overlay(
                        &mut app.content,
                        app.editor.mode,
                        &app.tabs[0],
                        ui,
                        0,
                        r,
                        1.,
                    );
                    page_menu(
                        &mut app.content,
                        &mut app.editor,
                        &mut app.tabs[0],
                        ui,
                        0,
                        r,
                        1.,
                    );
                });
            },
        );
        if let Some(update) = &output.platform_output.accesskit_update {
            ctx.data_mut(|data| {
                let bounds = data.get_temp_mut_or_default::<HashMap<String, Rect>>(egui::Id::new(
                    "context_test_bounds",
                ));
                for (_, node) in &update.nodes {
                    if let (Some(label), Some(b)) = (node.label(), node.bounds()) {
                        bounds.insert(
                            label.to_string(),
                            Rect::from_min_max(
                                pos2(b.x0 as f32, b.y0 as f32),
                                pos2(b.x1 as f32, b.y1 as f32),
                            ),
                        );
                    }
                }
            });
        }
        output
    }
    fn click(app: &mut Viewer, ctx: &egui::Context, p: egui::Pos2, button: egui::PointerButton) {
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(p),
                    egui::Event::PointerButton {
                        pos: p,
                        button,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    fn menu_click(app: &mut Viewer, ctx: &egui::Context, label: &str) {
        frame(app, ctx, vec![]);
        assert!(
            egui::Popup::is_any_open(ctx),
            "No context menu open for {label}"
        );
        let b = ctx
            .data(|d| {
                d.get_temp::<HashMap<String, Rect>>(egui::Id::new("context_test_bounds"))
                    .and_then(|b| b.get(label).copied())
            })
            .unwrap_or_else(|| panic!("Missing menu {label}"));
        click(app, ctx, b.center(), egui::PointerButton::Primary);
        app.context_commands(ctx);
        frame(app, ctx, vec![]);
    }
    #[test]
    fn empty_text_box_handle_menu_can_delete_placeholder() {
        let (mut app, ctx) = fixture();
        let tab = &app.tabs[0];
        app.content.place_text(tab.id, 0, [50., 60.], tab.pages[0]);
        let target = target_at(
            &app.content,
            &mut app.editor,
            &mut app.tabs[0],
            0,
            [80., 51.],
            1.,
        );
        assert!(matches!(target, Target::PendingText));
        assert!(commands(&target, true).contains(&"delete_text_box"));
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let r = Rect::from_min_size(
            pos2(20. + app.content.new_rect[0], 20. + app.content.new_rect[1]),
            vec2(app.content.new_rect[2], app.content.new_rect[3]),
        );
        let handle = text_move_handle(r, Rect::from_min_size(pos2(20., 20.), vec2(500., 500.)));
        click(
            &mut app,
            &ctx,
            handle.center(),
            egui::PointerButton::Secondary,
        );
        menu_click(&mut app, &ctx, "Delete");
        assert!(!app.content.new_placed);
        assert!(app.content.add_target.is_none());
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
    }

    #[test]
    fn image_paste_from_live_text_menu_preserves_typed_text_and_is_undoable() {
        for text in ["", "Keep this text"] {
            let (mut app, ctx) = fixture();
            let tab = &app.tabs[0];
            app.content.place_text(tab.id, 0, [50., 60.], tab.pages[0]);
            app.content.new_text = text.into();
            frame(&mut app, &ctx, vec![]);
            frame(&mut app, &ctx, vec![]);
            clipboard::set_for_test(clipboard::Clip::Image(
                clipboard::png(200, 80, &[0, 0, 200, 255].repeat(200 * 80)).unwrap(),
                [200, 80],
            ));
            click(
                &mut app,
                &ctx,
                pos2(110., 85.),
                egui::PointerButton::Secondary,
            );
            menu_click(&mut app, &ctx, "Paste");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while app.content.busy {
                app.content_events(&ctx);
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(app.content.pending_text().is_none());
            assert!(!app.content.new_placed);
            assert!(app.workbench.paste.is_some());
            app.place_pending_original();
            let session = app.tabs[0].session.as_mut().unwrap();
            assert_eq!(session.annotations.len(), 1);
            session.undo();
            assert!(session.annotations.is_empty());
            if !text.is_empty() {
                assert!(
                    mypdf_milestone0::text_geometry::runs(&session.source, 1)
                        .unwrap()
                        .iter()
                        .any(|run| run.text == text)
                );
            }
            session.redo();
            assert_eq!(session.annotations.len(), 1);
        }
    }

    #[test]
    fn deleting_live_text_annotation_supports_undo_redo() {
        let (mut app, ctx) = fixture();
        let mark = Annotation::new(
            0,
            [50., 60., 250., 100.],
            Kind::Text {
                text: "Keep with Undo".into(),
            },
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![mark.clone()])
            .unwrap();
        run_command(
            &mut app,
            &ctx,
            Target::Artwork(vec![mark.clone()]),
            "edit_text_here",
        );
        assert!(app.content.existing.is_some());
        run_command(&mut app, &ctx, Target::PendingText, "delete_text_box");
        let session = app.tabs[0].session.as_mut().unwrap();
        assert!(session.annotations.is_empty());
        session.undo();
        assert_eq!(session.annotations, vec![mark]);
        session.redo();
        assert!(session.annotations.is_empty());
    }

    #[test]
    fn signing_actions_are_absent_from_context_menus() {
        let drawing = Annotation::new(0, [10., 10., 20., 20.], Kind::Rectangle, Default::default());
        for target in [
            Target::Page,
            Target::PendingText,
            Target::Artwork(vec![drawing]),
        ] {
            for entry in items(&target, true) {
                assert!(!entry.label.contains("Sign"));
                assert!(!entry.action.contains("sign"));
            }
        }
    }

    #[test]
    fn drawing_menu_stays_compact_and_transform_submenu_remains_actionable() {
        for dark in [false, true] {
            let (mut app, ctx) = fixture();
            ctx.set_theme(if dark {
                egui::ThemePreference::Dark
            } else {
                egui::ThemePreference::Light
            });
            let shape = Annotation::new(
                0,
                [100., 100., 180., 180.],
                Kind::Rectangle,
                Default::default(),
            );
            app.tabs[0]
                .session
                .as_mut()
                .unwrap()
                .commit(vec![shape])
                .unwrap();
            frame(&mut app, &ctx, vec![]);
            click(
                &mut app,
                &ctx,
                pos2(120., 150.),
                egui::PointerButton::Secondary,
            );
            // Width must not expand as egui remembers the preceding popup size.
            for _ in 0..12 {
                frame(&mut app, &ctx, vec![]);
            }
            let bounds = ctx
                .data(|d| d.get_temp::<HashMap<String, Rect>>(egui::Id::new("context_test_bounds")))
                .unwrap();
            let property = bounds["Drawing properties…"];
            assert!(property.width() <= 300., "oversized menu: {property:?}");
            for label in ["Copy", "Cut", "Duplicate", "Transform", "Style", "Delete"] {
                assert_eq!(
                    bounds[label].width(),
                    property.width(),
                    "misaligned {label}"
                );
                assert_eq!(
                    bounds[label].right(),
                    property.right(),
                    "shortcut gutter for {label}"
                );
            }
            menu_click(&mut app, &ctx, "Transform");
            menu_click(&mut app, &ctx, "Rotate 90° clockwise");
            assert_eq!(
                app.tabs[0].session.as_ref().unwrap().annotations[0].rotation,
                90.
            );
            assert!(!egui::Popup::is_any_open(&ctx));
        }
    }
    #[test]
    fn target_follows_right_click_and_only_offers_relevant_actions_in_edit_mode() {
        let (mut app, ctx) = fixture();
        let image = Annotation::new(
            0,
            [10., 10., 60., 60.],
            Kind::Image {
                png: Arc::new(clipboard::png(1, 1, &[0, 0, 0, 255]).unwrap()),
            },
            Default::default(),
        );
        let shape = Annotation::new(
            0,
            [100., 100., 180., 180.],
            Kind::Rectangle,
            Default::default(),
        );
        let text = Annotation::new(
            0,
            [200., 50., 400., 100.],
            Kind::Text {
                text: "Selected words".into(),
            },
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![image.clone(), shape.clone(), text.clone()])
            .unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![shape.id.clone()]);
        let target = target_at(
            &app.content,
            &mut app.editor,
            &mut app.tabs[0],
            0,
            [30., 30.],
            1.,
        );
        assert_eq!(selection::selected_ids(&app.tabs[0]), vec![image.id]);
        assert!(commands(&target, true).contains(&"crop_object"));
        let target = target_at(
            &app.content,
            &mut app.editor,
            &mut app.tabs[0],
            0,
            [100., 130.],
            1.,
        );
        assert!(commands(&target, true).contains(&"flip_horizontal"));
        assert!(!commands(&target, true).contains(&"crop_object"));
        let target = target_at(
            &app.content,
            &mut app.editor,
            &mut app.tabs[0],
            0,
            [210., 60.],
            1.,
        );
        assert!(commands(&target, true).contains(&"edit_text_here"));
        assert!(!commands(&target, true).contains(&"flip_horizontal"));
        run_command(&mut app, &ctx, target, "edit_text_here");
        assert_eq!(
            app.content.existing.as_ref().unwrap().text,
            "Selected words"
        );
        app.content.existing = None;
        let target = target_at(
            &app.content,
            &mut app.editor,
            &mut app.tabs[0],
            0,
            [450., 450.],
            1.,
        );
        assert!(matches!(target, Target::Page));
        assert!(commands(&target, true).contains(&"add_text_here"));
        assert!(!commands(&target, true).contains(&"object_delete"));
        assert!(selection::selected_ids(&app.tabs[0]).is_empty());
    }
    #[test]
    fn edit_mode_secondary_click_opens_drawing_menu_and_executes_on_clicked_object() {
        let (mut app, ctx) = fixture();
        let shape = Annotation::new(
            0,
            [100., 100., 180., 180.],
            Kind::Rectangle,
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![shape])
            .unwrap();
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        click(
            &mut app,
            &ctx,
            pos2(120., 150.),
            egui::PointerButton::Secondary,
        );
        assert!(matches!(
            app.content.context.menu.as_ref().unwrap().target,
            Target::Artwork(_)
        ));
        menu_click(&mut app, &ctx, "Transform");
        menu_click(&mut app, &ctx, "Rotate 90° clockwise");
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations[0].rotation,
            90.
        );
        app.tabs[0].session.as_mut().unwrap().undo();
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations[0].rotation,
            0.
        );
    }
    #[test]
    fn locked_and_read_only_objects_cannot_be_changed_and_stale_commands_are_ignored() {
        let (mut app, ctx) = fixture();
        let mut shape = Annotation::new(
            0,
            [100., 100., 180., 180.],
            Kind::Rectangle,
            Default::default(),
        );
        shape.object.locked = true;
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![shape.clone()])
            .unwrap();
        let target = Target::Artwork(vec![shape.clone()]);
        assert!(commands(&target, true).contains(&"object_copy"));
        assert!(!commands(&target, true).contains(&"object_delete"));
        assert!(!commands(&target, false).contains(&"lock"));
        run_command(&mut app, &ctx, target.clone(), "object_delete");
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 1);
        run_command(&mut app, &ctx, target, "lock");
        assert!(
            !app.tabs[0].session.as_ref().unwrap().annotations[0]
                .object
                .locked
        );
        let t = &app.tabs[0];
        let old = Menu {
            key: (t.id, 0, 0),
            point: [100., 100.],
            target: Target::Artwork(vec![shape]),
        };
        app.content.context.command = Some(("object_delete", old));
        app.context_commands(&ctx);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 1);
    }
    #[test]
    fn base_pdf_text_and_image_have_different_menus_and_image_copy_keeps_pixels() {
        let (mut app, ctx) = fixture();
        let source = content::add_text(
            &app.tabs[0].session.as_ref().unwrap().source,
            1,
            "Original PDF text",
            [30., 40., 300., 80.],
            16.,
            [0.; 3],
        )
        .unwrap();
        let png = clipboard::png(2, 2, &[0, 90, 240, 255].repeat(4)).unwrap();
        let source = content::add_image(&source, 1, [100., 200., 200., 300.], &png).unwrap();
        let mut pdf = lopdf::Document::load_mem(&source).unwrap();
        for object in pdf.objects.values_mut() {
            if let Ok(stream) = object.as_stream_mut()
                && stream
                    .dict
                    .get(b"Subtype")
                    .and_then(lopdf::Object::as_name)
                    .ok()
                    == Some(b"Image")
            {
                stream.dict.set(
                    "Decode",
                    vec![0.into(), 1.into(), 0.into(), 1.into(), 0.into(), 1.into()],
                );
            }
        }
        let source = content::serialize(&mut pdf).unwrap();
        let runs = mypdf_milestone0::text_geometry::runs(&source, 1).unwrap();
        let r = runs[0].rect.unwrap();
        let images = content::images(&source, 1).unwrap();
        app.tabs[0].session = Some(mypdf_milestone0::annotations::Session::new(
            source.into(),
            vec![],
            true,
        ));
        let key = (
            app.tabs[0].id,
            app.tabs[0].session.as_ref().unwrap().revision(),
            0,
        );
        app.content.inspection = Some(Inspection {
            guides: Default::default(),
            key,
            text: Ok(runs),
            images: Ok(images.clone()),
            fields: Ok(vec![]),
            designed: Ok(vec![]),
        });
        let target = target_at(
            &app.content,
            &mut app.editor,
            &mut app.tabs[0],
            0,
            [(r[0] + r[2]) / 2., (r[1] + r[3]) / 2.],
            1.,
        );
        assert!(matches!(target, Target::Text(_)));
        assert!(commands(&target, true).contains(&"edit_text_here"));
        let target = target_at(
            &app.content,
            &mut app.editor,
            &mut app.tabs[0],
            0,
            [150., 250.],
            1.,
        );
        assert!(matches!(target, Target::Image(_)));
        assert!(commands(&target, true).contains(&"replace_image"));
        assert!(!commands(&target, true).contains(&"crop_object"));
        // A previous screenshot must be replaced even for PDF Decode images.
        clipboard::set_for_test(clipboard::Clip::Image(
            clipboard::png(100, 80, &[255, 0, 0, 255].repeat(100 * 80)).unwrap(),
            [100, 80],
        ));
        run_command(&mut app, &ctx, target, "copy_image");
        assert!(app.editor.marks_clip.is_some());
        let clipboard::Clip::Image(copied, size) = clipboard::read() else {
            panic!("Copy image did not publish pixels")
        };
        assert_eq!(size, [2, 2]);
        assert_eq!(
            content::decode_image(&copied).unwrap().get_pixel(0, 0).0,
            [0, 90, 240, 255]
        );
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
        assert!(!commands(&Target::Image(images[0].clone()), false).contains(&"cut_image"));
        run_command(
            &mut app,
            &ctx,
            Target::Image(images[0].clone()),
            "cut_image",
        );
        assert!(matches!(clipboard::read(), clipboard::Clip::Image(..)));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.content.busy {
            app.content_events(&ctx);
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let s = app.tabs[0].session.as_mut().unwrap();
        assert!(content::images(&s.source, 1).unwrap().is_empty());
        s.undo();
        assert_eq!(content::images(&s.source, 1).unwrap().len(), 1);
    }
    #[test]
    fn context_translation_stays_with_text_and_omits_page_and_zoom_actions() {
        use mypdf_milestone0::viewer::{GlyphBox, PageText};
        let (mut app, _) = fixture();
        app.tabs[0].text[0] = Some(Arc::new(PageText {
            text: "Hello".into(),
            limited: false,
            glyphs: (0..5)
                .map(|i| GlyphBox {
                    range: i..i + 1,
                    rect: [20. + i as f32 * 8., 50., 28. + i as f32 * 8., 62.],
                })
                .collect(),
        }));
        let selection = text_selection::TextSelection::all(&app.tabs[0], 0).unwrap();
        let selected = commands(&Target::SelectedText(selection), false);
        assert!(selected.contains(&"translate_selection"));
        assert!(!selected.contains(&"translate_page"));
        assert!(!selected.contains(&"highlight"));
        let page = commands(&Target::Page, false);
        for removed in ["translate_page", "fit_width", "fit_page"] {
            assert!(!page.contains(&removed));
        }
        assert!(!page.contains(&"translate_selection"));
        let drawing = Annotation::new(
            0,
            [100., 100., 180., 180.],
            Kind::Rectangle,
            Default::default(),
        );
        assert!(
            !commands(&Target::Artwork(vec![drawing]), true)
                .iter()
                .any(|action| action.starts_with("translate"))
        );
        let text = Annotation::new(
            0,
            [100., 100., 180., 180.],
            Kind::Text {
                text: "Merhaba".into(),
            },
            Default::default(),
        );
        assert!(commands(&Target::Artwork(vec![text]), false).contains(&"translate_text"));
        for action in ["translate_selection", "translate_text"] {
            assert!(matches!(action_icon(action, ""), Icon::Translate));
            assert!(
                shortcut(action).is_none(),
                "translation must not steal a shortcut"
            );
        }
    }

    #[test]
    fn inline_translation_snapshots_unicode_selection_without_changing_the_draft() {
        use egui::text::{CCursor, CCursorRange};
        let (mut app, ctx) = fixture();
        app.content
            .place_text(app.tabs[0].id, 0, [50., 60.], app.tabs[0].pages[0]);
        app.content.new_text = "First İstanbul last".into();
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let id = egui::Id::new((app.tabs[0].id, 0usize, "inline_new_text"));
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(6), CCursor::new(14))));
        state.store(&ctx, id);
        frame(&mut app, &ctx, vec![]);
        click(
            &mut app,
            &ctx,
            pos2(110., 85.),
            egui::PointerButton::Secondary,
        );
        menu_click(&mut app, &ctx, "Translate selected text…");
        assert_eq!(take_translation_text(&ctx).as_deref(), Some("İstanbul"));
        assert!(
            take_translation_text(&ctx).is_none(),
            "requests are consumed once"
        );
        assert_eq!(app.content.new_text, "First İstanbul last");
        assert_eq!(app.content.new_selection, Some(6..15));
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
        assert!(app.content.pending_text().is_some());
        assert!(!egui::Popup::is_any_open(&ctx));
    }

    #[test]
    fn live_text_context_menu_preserves_unicode_selection_cut_paste_and_cancel() {
        use egui::text::{CCursor, CCursorRange};
        let (mut app, ctx) = fixture();
        app.content
            .place_text(app.tabs[0].id, 0, [50., 60.], app.tabs[0].pages[0]);
        app.content.new_text = "First İstanbul last".into();
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let id = egui::Id::new((app.tabs[0].id, 0usize, "inline_new_text"));
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(6), CCursor::new(14))));
        state.store(&ctx, id);
        frame(&mut app, &ctx, vec![]);
        click(
            &mut app,
            &ctx,
            pos2(110., 85.),
            egui::PointerButton::Secondary,
        );
        assert_eq!(
            app.content.new_selection,
            Some(6..15),
            "right click must preserve the UTF-8 byte selection"
        );
        menu_click(&mut app, &ctx, "Cut");
        assert_eq!(app.content.new_text, "First  last");
        for (shift, expected) in [(false, "First İstanbul last"), (true, "First  last")] {
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::Key {
                    key: egui::Key::Z,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        shift,
                        ..egui::Modifiers::COMMAND
                    },
                }],
            );
            assert_eq!(
                app.content.new_text, expected,
                "Text menu changes must support Undo/Redo"
            );
        }
        clipboard::set_for_test(clipboard::Clip::Text("İzmir".into()));
        click(
            &mut app,
            &ctx,
            pos2(110., 85.),
            egui::PointerButton::Secondary,
        );
        menu_click(&mut app, &ctx, "Paste");
        assert_eq!(app.content.new_text, "First İzmir last");
        click(
            &mut app,
            &ctx,
            pos2(110., 85.),
            egui::PointerButton::Secondary,
        );
        menu_click(&mut app, &ctx, "Cancel text edit");
        assert!(app.content.new_text.is_empty());
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
    }
    #[test]
    fn reading_selection_keeps_its_words_and_markup_uses_normalized_geometry() {
        use mypdf_milestone0::viewer::{GlyphBox, PageText};
        let (mut app, ctx) = fixture();
        app.editor.mode = editing::Mode::Read;
        app.tabs[0].text[0] = Some(Arc::new(PageText {
            text: "One two".into(),
            glyphs: (0..7)
                .map(|i| GlyphBox {
                    range: i..i + 1,
                    rect: [20. + i as f32 * 8., 50., 28. + i as f32 * 8., 62.],
                })
                .collect(),
            limited: false,
        }));
        let selected = text_selection::TextSelection::word_at(&app.tabs[0], 0, [25., 55.]).unwrap();
        assert_eq!(selected.text(&app.tabs[0]).as_deref(), Some("One"));
        app.editor.text_selection = Some(selected);
        let target = target_at(
            &app.content,
            &mut app.editor,
            &mut app.tabs[0],
            0,
            [40., 55.],
            1.,
        );
        assert!(matches!(target, Target::SelectedText(_)));
        assert!(!commands(&target, true).contains(&"object_rotate"));
        run_command(&mut app, &ctx, target, "highlight");
        let marks = &app.tabs[0].session.as_ref().unwrap().annotations;
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].rect, [20., 50., 44., 62.]);
        let Kind::Markup { quads, .. } = &marks[0].kind else {
            panic!()
        };
        assert_eq!(quads, &vec![[0., 0., 1., 1.]]);
    }
    fn auto_edit_fixture(image_rect: [f32; 4]) -> (Viewer, egui::Context, egui::Pos2) {
        let (mut app, ctx) = fixture();
        let source = content::add_text(
            &app.tabs[0].session.as_ref().unwrap().source,
            1,
            "Original words",
            [30., 40., 300., 80.],
            16.,
            [0.; 3],
        )
        .unwrap();
        let png = clipboard::png(2, 2, &[0, 90, 240, 255].repeat(4)).unwrap();
        let source = content::add_image(&source, 1, image_rect, &png).unwrap();
        let runs = mypdf_milestone0::text_geometry::runs(&source, 1).unwrap();
        let r = runs[0].rect.unwrap();
        let images = content::images(&source, 1).unwrap();
        app.tabs[0].session = Some(mypdf_milestone0::annotations::Session::new(
            source.into(),
            vec![],
            true,
        ));
        let key = (app.tabs[0].id, 0, 0);
        app.content.key = Some(key);
        app.content.inspection = Some(Inspection {
            key,
            guides: Default::default(),
            text: Ok(runs),
            images: Ok(images),
            fields: Ok(vec![]),
            designed: Ok(vec![]),
        });
        (
            app,
            ctx,
            pos2(20. + (r[0] + r[2]) / 2., 20. + (r[1] + r[3]) / 2.),
        )
    }

    fn auto_edit_frame(app: &mut Viewer, ctx: &egui::Context, events: Vec<egui::Event>) {
        app.content_events(ctx);
        app.inspect_content(ctx);
        app.context_commands(ctx);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(700., 700.))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = Rect::from_min_size(pos2(20., 20.), vec2(500., 500.));
                    smart_select(
                        &mut app.content,
                        &mut app.editor,
                        &mut app.tabs[0],
                        ui,
                        0,
                        rect,
                        1.,
                        &mut app.inspector,
                    );
                    overlay(
                        &mut app.content,
                        app.editor.mode,
                        &app.tabs[0],
                        ui,
                        0,
                        rect,
                        1.,
                    );
                });
            },
        );
        app.context_commands(ctx);
        app.finish_new_text(ctx);
    }

    fn auto_edit_click(app: &mut Viewer, ctx: &egui::Context, pos: egui::Pos2) {
        for pressed in [true, false] {
            auto_edit_frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        auto_edit_frame(app, ctx, vec![]);
    }

    #[test]
    fn automatic_edit_switches_text_and_images_and_keeps_committed_typing() {
        let (mut app, ctx, text_point) = auto_edit_fixture([100., 200., 200., 300.]);
        // A tool remembered from Draw must not prevent selection in Edit.
        app.editor.tool = editing::Tool::Arrow;
        for _ in 0..2 {
            auto_edit_frame(&mut app, &ctx, vec![]);
        }
        auto_edit_click(&mut app, &ctx, pos2(170., 270.));
        assert!(app.content.image_mode);
        assert_eq!(app.content.selected, Some(0));
        auto_edit_click(&mut app, &ctx, text_point);
        assert!(!app.content.image_mode);
        assert!(app.content.existing.is_some());
        app.content.existing.as_mut().unwrap().text = "Edited words".into();
        auto_edit_click(&mut app, &ctx, pos2(170., 270.));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !app.content.image_mode || app.content.selected != Some(0) {
            assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
            auto_edit_frame(&mut app, &ctx, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.content.existing.is_none());
        let session = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(session.revision(), 1);
        assert!(
            mypdf_milestone0::text_geometry::runs(&session.source, 1)
                .unwrap()
                .iter()
                .any(|r| r.text == "Edited words")
        );
        session.undo();
        assert!(
            mypdf_milestone0::text_geometry::runs(&session.source, 1)
                .unwrap()
                .iter()
                .any(|r| r.text == "Original words")
        );
    }

    #[test]
    fn escape_leaves_explicit_edit_tools_ready_for_automatic_selection() {
        let (mut app, ctx, _) = auto_edit_fixture([100., 200., 200., 300.]);
        for _ in 0..2 {
            auto_edit_frame(&mut app, &ctx, vec![]);
        }
        for add in [true, false] {
            app.content.image_mode = false;
            app.content.area_mode = !add;
            app.content.add_target = add.then_some((app.tabs[0].id, 0));
            auto_edit_frame(
                &mut app,
                &ctx,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
            );
            assert!(app.content.add_target.is_none());
            assert!(!app.content.area_mode);
            auto_edit_click(&mut app, &ctx, pos2(170., 270.));
            assert!(app.content.image_mode);
            assert_eq!(app.content.selected, Some(0));
        }
    }

    #[test]
    fn automatic_edit_prioritizes_text_over_background_images_and_retains_failed_drafts() {
        let (mut app, ctx, text_point) = auto_edit_fixture([0., 0., 500., 500.]);
        for _ in 0..2 {
            auto_edit_frame(&mut app, &ctx, vec![]);
        }
        auto_edit_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(text_point),
                egui::Event::PointerButton {
                    pos: text_point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        assert!(
            !app.content.image_mode,
            "pressing text must not flash the background image inspector"
        );
        auto_edit_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: text_point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        let draft = app.content.existing.as_mut().unwrap();
        draft.text = "Keep these unsaved words".into();
        draft.key.1 += 1; // Simulate a revision conflict during an edit.
        auto_edit_click(&mut app, &ctx, pos2(170., 270.));
        assert!(!app.content.image_mode);
        assert_eq!(
            app.content.existing.as_ref().unwrap().text,
            "Keep these unsaved words"
        );
        assert!(app.content.after_text.is_none());
        assert!(app.content.context.select_after_text.is_none());
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
    }

    #[test]
    fn a_coalesced_area_drag_keeps_its_bounds_for_table_detection() {
        let (mut app, ctx, _) = auto_edit_fixture([100., 200., 200., 300.]);
        app.content.area_mode = true;
        for _ in 0..3 {
            auto_edit_frame(&mut app, &ctx, vec![]);
        }
        let (start, end) = (pos2(30., 30.), pos2(350., 120.));
        auto_edit_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerMoved(end),
                egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        assert_eq!(
            app.content.table_selection,
            Some(((app.tabs[0].id, 0, 0), [10., 10., 330., 100.]))
        );
        assert!(!app.content.area_mode);
        assert!(app.content.existing.is_some());
        app.content.existing = None;
        app.content.table_selection = None;
        auto_edit_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        assert!(
            app.content.table_selection.is_none(),
            "A plain click must not create an empty area selection"
        );
    }

    #[test]
    fn annotate_clicks_do_not_enter_pdf_text_or_image_editing() {
        let (mut app, ctx, text_point) = auto_edit_fixture([100., 200., 200., 300.]);
        app.editor.mode = editing::Mode::Annotate;
        auto_edit_frame(&mut app, &ctx, vec![]);
        auto_edit_frame(&mut app, &ctx, vec![]);
        for point in [text_point, pos2(170., 270.)] {
            auto_edit_click(&mut app, &ctx, point);
            assert!(app.editor.mode == editing::Mode::Annotate);
            assert!(app.content.existing.is_none());
            assert!(!app.content.image_mode);
            assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
        }
    }

    #[test]
    fn smart_selection_opens_artwork_inspector_without_changing_modes() {
        let (mut app, ctx) = fixture();
        let shape = Annotation::new(
            0,
            [80., 80., 180., 180.],
            Kind::Rectangle,
            Default::default(),
        );
        let id = shape.id.clone();
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![shape])
            .unwrap();
        let draw = |app: &mut Viewer, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(700., 700.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        smart_select(
                            &mut app.content,
                            &mut app.editor,
                            &mut app.tabs[0],
                            ui,
                            0,
                            Rect::from_min_size(pos2(20., 20.), vec2(500., 500.)),
                            1.,
                            &mut app.inspector,
                        )
                    });
                },
            );
        };
        draw(&mut app, vec![]);
        draw(&mut app, vec![]);
        let point = pos2(100., 150.);
        draw(
            &mut app,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        assert!(app.editor.mode == editing::Mode::Edit);
        assert_eq!(app.tabs[0].selected_annotation, Some(id));
        assert!(app.inspector);
        draw(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        app.editor.mode = editing::Mode::Read;
        draw(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        assert!(app.editor.mode == editing::Mode::Read);
    }
    #[test]
    fn contextual_group_format_and_select_behind_commands_use_clicked_objects() {
        let (mut app, ctx) = fixture();
        let a = Annotation::new(
            0,
            [200., 200., 300., 300.],
            Kind::Rectangle,
            Default::default(),
        );
        let mut b = a.duplicate();
        b.rect = a.rect;
        b.style.color = [1., 0., 0.];
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![a.clone(), b.clone()])
            .unwrap();
        run_command(
            &mut app,
            &ctx,
            Target::Artwork(vec![a.clone(), b.clone()]),
            "group_objects",
        );
        let marks = app.tabs[0].session.as_ref().unwrap().annotations.clone();
        assert!(marks[0].object.group.is_some());
        assert_eq!(marks[0].object.group, marks[1].object.group);
        run_command(&mut app, &ctx, Target::Artwork(marks), "ungroup_objects");
        assert!(
            app.tabs[0]
                .session
                .as_ref()
                .unwrap()
                .annotations
                .iter()
                .all(|a| a.object.group.is_none())
        );
        run_command(
            &mut app,
            &ctx,
            Target::Artwork(vec![b.clone()]),
            "copy_format",
        );
        run_command(&mut app, &ctx, Target::Artwork(vec![a]), "paste_format");
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations[0]
                .style
                .color,
            [1., 0., 0.]
        );
        run_command(&mut app, &ctx, Target::Artwork(vec![b]), "select_behind");
        assert_eq!(
            app.tabs[0].selected_annotation,
            Some(
                app.tabs[0].session.as_ref().unwrap().annotations[0]
                    .id
                    .clone()
            )
        );
    }
}
