//! Native document shortcuts. The UI, Dock and Jump List share the same actions.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "windows")]
mod windows;

const RECENT_LIMIT: usize = 8;
pub(super) const ARGUMENT: &str = "--shell-action";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Action {
    Document(PathBuf),
    Tab { session: String, id: u64 },
    Group(u64),
    Open,
    New,
}
impl Action {
    pub(super) fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("serializable shell action"))
    }
    pub(super) fn decode(value: &str) -> Option<Self> {
        if value.len() > 24_000 {
            return None;
        }
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(value).ok()?).ok()
    }
    #[cfg(any(target_os = "windows", test))]
    fn arguments(&self) -> String {
        // Only a fixed flag and URL-safe ASCII reach the Windows command line.
        format!("{ARGUMENT} {}", self.encode())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    title: String,
    detail: String,
    action: Action,
    active: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Group {
    id: u64,
    name: String,
    active: bool,
    entries: Vec<Entry>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Snapshot {
    grouped: bool,
    groups: Vec<Group>,
    recent: Vec<Entry>,
}

pub(super) struct State {
    session: String,
    last: Option<Snapshot>,
    #[cfg(all(target_os = "windows", not(test)))]
    publisher: windows::Publisher,
}
impl State {
    pub(super) fn new() -> Self {
        #[cfg(all(target_os = "macos", not(test)))]
        mac::install();
        Self {
            session: format!("{:032x}", rand::random::<u128>()),
            last: None,
            #[cfg(all(target_os = "windows", not(test)))]
            publisher: windows::Publisher::new(),
        }
    }
    fn publish(&mut self, snapshot: Snapshot) {
        if self.last.as_ref() == Some(&snapshot) {
            return;
        }
        #[cfg(all(target_os = "macos", not(test)))]
        mac::publish(snapshot.clone());
        #[cfg(all(target_os = "windows", not(test)))]
        self.publisher.publish(snapshot.clone());
        self.last = Some(snapshot);
    }
}

impl Viewer {
    pub(super) fn shell_action(&mut self, action: Action, ctx: &egui::Context) {
        open_with::Arrivals {
            focus: true,
            ..Default::default()
        }
        .activate(ctx);
        match action {
            Action::Document(path) => {
                let path = path.canonicalize().unwrap_or(path);
                if let Some(index) = self.tabs.iter().position(|t| t.path == path) {
                    self.activate_shell_tab(index);
                } else if path.is_file() {
                    self.open(path, ctx);
                    self.sync_tab_group();
                } else {
                    self.status = Some((
                        "This recent PDF is no longer available. Use Open to locate it.".into(),
                        true,
                    ));
                }
            }
            Action::Tab { session, id } => {
                if session == self.shell_menu.session
                    && let Some(index) = self.tabs.iter().position(|t| t.id == id)
                {
                    self.activate_shell_tab(index);
                } else {
                    self.status = Some(("That unsaved tab is no longer open. Check Document Recovery for unsaved work.".into(), false));
                }
            }
            Action::Group(id) => self.switch_group(id),
            Action::Open => self.open_dialog(ctx),
            Action::New => self.new_blank(ctx),
        }
    }

    fn activate_shell_tab(&mut self, index: usize) {
        self.switch_group(self.tabs[index].document_group);
        self.active = index;
        self.home.visible = false;
        self.tab_revealed = None;
        self.sync_tab_group();
        self.workspace.invalidate();
    }

    fn shell_snapshot(&self) -> Snapshot {
        let grouped = self.tab_controls.groups.iter().any(|g| g.id != 0);
        let mut groups: Vec<_> = self
            .tab_controls
            .groups
            .iter()
            .map(|g| Group {
                id: g.id,
                name: g.name.clone(),
                active: g.id == self.tab_controls.current_group,
                entries: self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.document_group == g.id)
                    .map(|(index, tab)| {
                        let name = tab.name();
                        let duplicate = self.tabs.iter().filter(|t| t.name() == name).count() > 1;
                        let title = if duplicate {
                            format!(
                                "{name} — {}",
                                tab.path
                                    .parent()
                                    .unwrap_or_else(|| std::path::Path::new(""))
                                    .display()
                            )
                        } else {
                            name
                        };
                        Entry {
                            title: compact_title(&title),
                            detail: tab.path.display().to_string(),
                            action: if tab.has_destination && tab.path.is_absolute() {
                                Action::Document(tab.path.clone())
                            } else {
                                Action::Tab {
                                    session: self.shell_menu.session.clone(),
                                    id: tab.id,
                                }
                            },
                            active: !self.home.visible && index == self.active,
                        }
                    })
                    .collect(),
            })
            .collect();
        // Ungrouped is implicit until it contains documents (or is selected).
        groups.retain(|g| g.id != 0 || !g.entries.is_empty() || g.active);
        let paths = self.home.recent_paths();
        let recent = paths
            .iter()
            .take(RECENT_LIMIT)
            .map(|path| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let duplicate = paths
                    .iter()
                    .filter(|p| p.file_name() == path.file_name())
                    .count()
                    > 1;
                let title = if duplicate {
                    format!(
                        "{name} — {}",
                        path.parent()
                            .unwrap_or_else(|| std::path::Path::new(""))
                            .display()
                    )
                } else {
                    name.into_owned()
                };
                Entry {
                    title: compact_title(&title),
                    detail: path.display().to_string(),
                    action: Action::Document(path.clone()),
                    active: false,
                }
            })
            .collect();
        Snapshot {
            grouped,
            groups,
            recent,
        }
    }

    pub(super) fn publish_shell_menu(&mut self) {
        self.shell_menu.publish(self.shell_snapshot());
    }
}

fn compact_title(title: &str) -> String {
    let chars: Vec<_> = title.chars().filter(|c| !c.is_control()).collect();
    if chars.len() <= 64 {
        chars.into_iter().collect()
    } else {
        chars[..42]
            .iter()
            .chain(std::iter::once(&'…'))
            .chain(&chars[chars.len() - 21..])
            .collect()
    }
}

// Windows has a shared slot budget. Reserve room for recents, then distribute
// the remaining slots across groups so one large group cannot hide all others.
#[cfg(any(target_os = "windows", test))]
fn jump_categories(
    snapshot: &Snapshot,
    slots: usize,
    removed: &HashSet<String>,
) -> Vec<(String, Vec<Entry>)> {
    let slots = slots.min(12);
    let mut groups = snapshot.groups.clone();
    groups.sort_by_key(|g| !g.active);
    let mut used = HashSet::new();
    let mut categories: Vec<(String, Vec<Entry>)> = groups
        .into_iter()
        .filter_map(|g| {
            let mut entries = g.entries;
            entries.sort_by_key(|e| !e.active);
            if entries.is_empty() && snapshot.grouped {
                entries.push(Entry {
                    title: format!("Open {}", g.name),
                    detail: String::new(),
                    action: Action::Group(g.id),
                    active: false,
                });
            }
            entries.retain(|e| {
                !removed.contains(&e.action.arguments()) && used.insert(e.action.arguments())
            });
            (!entries.is_empty()).then_some((
                if snapshot.grouped {
                    format!("Group · {}", g.name)
                } else {
                    "Open PDFs".into()
                },
                entries,
            ))
        })
        .collect();
    let recent: Vec<_> = snapshot
        .recent
        .iter()
        .filter(|e| !removed.contains(&e.action.arguments()) && used.insert(e.action.arguments()))
        .cloned()
        .collect();
    let group_budget = slots.saturating_sub(recent.len().min(3).min(slots.saturating_sub(1)));
    let mut lengths = vec![0; categories.len()];
    let mut remaining = group_budget;
    loop {
        let before = remaining;
        for ((_, entries), count) in categories.iter().zip(&mut lengths) {
            if remaining > 0 && *count < entries.len() {
                *count += 1;
                remaining -= 1;
            }
        }
        if remaining == 0 || before == remaining {
            break;
        }
    }
    let mut names = HashSet::new();
    for ((name, entries), count) in categories.iter_mut().zip(lengths) {
        entries.truncate(count);
        let original = name.clone();
        let mut suffix = 2;
        while !names.insert(name.clone()) {
            *name = format!("{original} ({suffix})");
            suffix += 1;
        }
    }
    categories.retain(|(_, entries)| !entries.is_empty());
    let used_slots: usize = categories.iter().map(|(_, e)| e.len()).sum();
    let recent: Vec<_> = recent.into_iter().take(slots - used_slots).collect();
    if !recent.is_empty() {
        categories.push(("Recent PDFs".into(), recent));
    }
    categories
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::convert;

    fn fixture() -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        for name in ["Personal.pdf", "Work.pdf", "Notes.pdf"] {
            app.open_memory(name.into(), convert::blank(500., 700.).unwrap(), &ctx);
        }
        app.tab_controls.groups.push(tabs::Group {
            id: 1,
            name: "Work".into(),
            active: None,
        });
        app.tabs[1].document_group = 1;
        app.tabs[2].document_group = 1;
        app.switch_group(0);
        (app, ctx)
    }

    #[test]
    fn native_shortcut_switches_group_and_existing_tab_without_touching_edits() {
        let (mut app, ctx) = fixture();
        let before: Vec<_> = app.tabs.iter().map(|t| (t.id, t.unsaved_on_load)).collect();
        let work = app
            .shell_snapshot()
            .groups
            .into_iter()
            .find(|g| g.id == 1)
            .unwrap();
        app.shell_action(work.entries[1].action.clone(), &ctx);
        assert_eq!(app.tab_controls.current_group, 1);
        assert_eq!(app.active, 2);
        assert!(!app.home.visible);
        assert_eq!(
            app.tabs
                .iter()
                .map(|t| (t.id, t.unsaved_on_load))
                .collect::<Vec<_>>(),
            before
        );
        let snapshot = app.shell_snapshot();
        assert_eq!(snapshot.groups.iter().filter(|g| g.active).count(), 1);
        assert_eq!(
            snapshot
                .groups
                .iter()
                .flat_map(|g| &g.entries)
                .filter(|e| e.active)
                .count(),
            1
        );
        let active = app.active;
        app.shell_action(
            Action::Tab {
                session: "previous app launch".into(),
                id: app.tabs[0].id,
            },
            &ctx,
        );
        assert_eq!(
            app.active, active,
            "a stale unsaved shortcut cannot select a reused tab ID"
        );
    }

    #[test]
    fn recent_pdf_selects_its_existing_group_and_missing_file_does_not_create_tab() {
        let (mut app, ctx) = fixture();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("Signed Ünicode.pdf");
        std::fs::write(&path, convert::blank(500., 700.).unwrap()).unwrap();
        app.open(path.clone(), &ctx);
        let index = app.active;
        app.tabs[index].document_group = 1;
        app.switch_group(0);
        app.shell_action(Action::Document(path), &ctx);
        assert_eq!(app.tab_controls.current_group, 1);
        assert_eq!(app.active, index);
        assert_eq!(app.tabs.len(), 4);
        app.shell_action(Action::Document(folder.path().join("deleted.pdf")), &ctx);
        assert_eq!(app.tabs.len(), 4);
        assert_eq!(app.active, index);
    }

    #[test]
    fn native_menu_is_flat_without_groups_and_limits_recent_files() {
        let (mut app, _) = fixture();
        app.tab_controls.groups.truncate(1);
        app.tabs.iter_mut().for_each(|t| t.document_group = 0);
        let folder = tempfile::tempdir().unwrap();
        for index in 0..12 {
            let path = folder.path().join(format!("recent-{index}.pdf"));
            std::fs::write(&path, b"fixture").unwrap();
            app.home.remember(&path);
        }
        let snapshot = app.shell_snapshot();
        assert!(!snapshot.grouped);
        assert_eq!(snapshot.groups.len(), 1);
        assert_eq!(snapshot.groups[0].entries.len(), 3);
        assert_eq!(snapshot.recent.len(), RECENT_LIMIT);
        assert!(compact_title(&"Ü".repeat(100)).chars().count() <= 64);
    }

    #[test]
    fn jump_list_keeps_groups_and_recents_within_budget_and_honors_removed_items() {
        let (app, _) = fixture();
        let mut snapshot = app.shell_snapshot();
        snapshot.recent = (0..8)
            .map(|i| Entry {
                title: format!("Recent {i}"),
                detail: String::new(),
                action: Action::Document(format!("recent-{i}.pdf").into()),
                active: false,
            })
            .collect();
        // Recents already shown in a group must not occupy another slot.
        snapshot
            .recent
            .insert(0, snapshot.groups[0].entries[0].clone());
        let removed = HashSet::from([snapshot.groups[1].entries[0].action.arguments()]);
        for slots in 0..=20 {
            let categories = jump_categories(&snapshot, slots, &removed);
            let entries: Vec<_> = categories.iter().flat_map(|(_, e)| e).collect();
            assert!(entries.len() <= slots.min(12));
            assert!(
                entries
                    .iter()
                    .all(|e| !removed.contains(&e.action.arguments()))
            );
            assert_eq!(
                entries
                    .iter()
                    .map(|e| e.action.arguments())
                    .collect::<HashSet<_>>()
                    .len(),
                entries.len()
            );
            if slots >= 5 {
                assert_eq!(categories.len(), 3);
                assert_eq!(categories.last().unwrap().0, "Recent PDFs");
            }
        }
        snapshot.groups[1].name = snapshot.groups[0].name.clone();
        let categories = jump_categories(&snapshot, 12, &removed);
        assert_eq!(
            categories
                .iter()
                .map(|(name, _)| name)
                .collect::<HashSet<_>>()
                .len(),
            categories.len()
        );
    }

    #[test]
    fn shell_arguments_round_trip_spaces_quotes_unicode_and_do_not_treat_flags_as_files() {
        for action in [
            Action::Document(PathBuf::from("C:\\Work & notes\\Örhan's \"PDF\".pdf")),
            Action::Open,
            Action::New,
            Action::Group(4),
        ] {
            let args = action.arguments();
            assert_eq!(args.split_whitespace().count(), 2);
            let request =
                open_with::launch_request(args.split_whitespace().map(Into::into).collect())
                    .unwrap();
            assert!(request.paths.is_empty());
            assert_eq!(request.actions, vec![action]);
        }
        assert!(open_with::launch_request(vec![ARGUMENT.into(), "broken".into()]).is_err());
        assert_eq!(
            open_with::launch_request(vec!["--".into(), "--shell-action".into()])
                .unwrap()
                .paths,
            vec![PathBuf::from("--shell-action")]
        );
    }
}
