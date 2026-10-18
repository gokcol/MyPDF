//! Restore file-backed tabs without storing document bytes, search terms or passwords.
use super::*;
use mypdf_milestone0::home::ReadingState;
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct SavedTab {
    pub path: PathBuf,
    pub view: ReadingState,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct Saved {
    enabled: bool,
    tabs: Vec<SavedTab>,
    active: Option<PathBuf>,
    home: bool,
    sidebar: bool,
    inspector: bool,
    outline: bool,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            enabled: true,
            tabs: vec![],
            active: None,
            home: true,
            sidebar: true,
            inspector: true,
            outline: false,
        }
    }
}
pub(super) struct State {
    pub root: PathBuf,
    saved: Saved,
    last: Option<std::time::Instant>,
    pub closed: Vec<SavedTab>,
    pub options: bool,
    error: Option<String>,
    #[cfg(test)]
    _temporary: tempfile::TempDir,
}
impl State {
    pub(super) fn invalidate(&mut self) {
        self.last = None;
    }
    pub fn new() -> Self {
        #[cfg(test)]
        let temporary = tempfile::tempdir().unwrap();
        #[cfg(test)]
        let root = temporary.path().to_path_buf();
        #[cfg(not(test))]
        let root = mypdf_milestone0::assets::data_directory();
        let (saved, error) = match preferences::load::<Saved>(&root, "workspace.json") {
            Ok(mut s) => {
                s.tabs.truncate(16);
                s.tabs.retain(|t| {
                    t.path.is_absolute()
                        && t.view.top.is_finite()
                        && t.view
                            .scale
                            .is_none_or(|v| v.is_finite() && (0.1..=6.).contains(&v))
                });
                (s, None)
            }
            Err(e) => (Saved::default(), Some(e.to_string())),
        };
        Self {
            root,
            saved,
            last: None,
            closed: vec![],
            options: false,
            error,
            #[cfg(test)]
            _temporary: temporary,
        }
    }
}
impl Viewer {
    pub(super) fn restore_workspace(&mut self, ctx: &egui::Context) {
        if !self.workspace.saved.enabled {
            return;
        }
        let saved = self.workspace.saved.clone();
        let mut missing = 0;
        for entry in saved.tabs {
            if entry.path.is_file() {
                self.restore_tab(entry, ctx);
            } else {
                missing += 1;
            }
        }
        if let Some(i) = self
            .tabs
            .iter()
            .position(|t| Some(&t.path) == saved.active.as_ref())
        {
            self.active = i;
        }
        self.home.visible = saved.home || self.tabs.is_empty();
        self.sidebar = saved.sidebar;
        self.inspector = saved.inspector;
        self.outline = saved.outline;
        if missing > 0 {
            self.status = Some((
                format!("Restored workspace; {missing} unavailable file(s) were skipped"),
                false,
            ));
        }
    }
    fn restore_tab(&mut self, entry: SavedTab, ctx: &egui::Context) {
        self.open(entry.path.clone(), ctx);
        if let Some(t) = self.tabs.iter_mut().find(|t| t.path == entry.path) {
            t.view_preferences = entry.view;
            t.page_layout = reading::PageLayout::from_code(t.view_preferences.layout);
            t.restore_view = true;
        }
    }
    pub(super) fn reopen_closed(&mut self, ctx: &egui::Context) {
        if self.tabs.len() >= 16 {
            self.status = Some(("Close a tab before reopening another".into(), true));
            return;
        }
        if let Some(entry) = self.workspace.closed.pop() {
            if entry.path.is_file() {
                self.restore_tab(entry, ctx);
            } else {
                self.status = Some(("The closed document is no longer available".into(), true));
            }
        }
    }
    pub(super) fn remember_workspace(&mut self, ctx: &egui::Context) {
        if !self.exit_approved
            && self
                .workspace
                .last
                .is_some_and(|t| t.elapsed().as_secs() < 2)
        {
            ctx.request_repaint_after(std::time::Duration::from_secs(2));
            return;
        }
        self.workspace.last = Some(std::time::Instant::now());
        let next = Saved {
            enabled: self.workspace.saved.enabled,
            tabs: self
                .tabs
                .iter()
                .filter(|t| t.path.is_absolute() && t.path.is_file())
                .map(|t| SavedTab {
                    path: t.path.clone(),
                    view: t.view_preferences.clone(),
                })
                .collect(),
            active: self.tabs.get(self.active).map(|t| t.path.clone()),
            home: self.home.visible,
            sidebar: self.sidebar,
            inspector: self.inspector,
            outline: self.outline,
        };
        if next != self.workspace.saved && self.workspace.error.is_none() {
            match preferences::save(&self.workspace.root, "workspace.json", &next) {
                Ok(()) => self.workspace.saved = next,
                Err(e) => self.workspace.error = Some(e.to_string()),
            }
        }
    }
    pub(super) fn workspace_window(&mut self, ctx: &egui::Context) {
        if !self.workspace.options {
            return;
        }
        let mut open = true;
        egui::Window::new("Workspace").open(&mut open).resizable(false).show(ctx,|ui|{
            let mut enabled=self.workspace.saved.enabled;
            if ui.checkbox(&mut enabled,"Reopen my documents when MyPDF starts").changed() {
                let mut saved=self.workspace.saved.clone();saved.enabled=enabled;
                match preferences::save(&self.workspace.root,"workspace.json",&saved) {Ok(())=>self.workspace.saved=saved,Err(e)=>self.workspace.error=Some(e.to_string())}
            }
            ui.weak("Remembers tabs, reading positions and panels locally.\nUnsaved changes remain in Document Recovery.");
            if ui.add_enabled(!self.workspace.closed.is_empty(),ActionButton::new(Icon::Undo,"Reopen Closed Tab")).clicked(){self.reopen_closed(ctx);}
            if let Some(error)=&self.workspace.error {ui.colored_label(theme::danger(),error);}
        });
        self.workspace.options = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_restores_order_active_tab_views_and_closed_tabs_without_capturing_content() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("one.pdf");
        let second = root.path().join("two.pdf");
        let bytes = mypdf_milestone0::convert::blank(500., 700.).unwrap();
        std::fs::write(&first, &bytes).unwrap();
        std::fs::write(&second, &bytes).unwrap();
        let first = first.canonicalize().unwrap();
        let second = second.canonicalize().unwrap();
        app.open(first.clone(), &ctx);
        app.open(second.clone(), &ctx);
        app.active = 0;
        app.tabs[0].view_preferences.scale = Some(1.75);
        app.sidebar = false;
        app.inspector = true;
        app.home.visible = false;
        app.remember_workspace(&ctx);
        let raw = std::fs::read_to_string(app.workspace.root.join("workspace.json")).unwrap();
        assert!(!raw.contains("%PDF"));
        assert!(!raw.contains("password"));
        let saved: Saved = serde_json::from_str(&raw).unwrap();
        assert_eq!(saved.active, Some(first.clone()));
        assert_eq!(saved.tabs.len(), 2);
        app.tabs.clear();
        app.workspace.saved = saved;
        app.restore_workspace(&ctx);
        assert_eq!(app.tabs[0].path, first);
        assert_eq!(app.tabs[1].path, second);
        assert_eq!(app.active, 0);
        assert_eq!(app.tabs[0].view_preferences.scale, Some(1.75));
        assert!(!app.sidebar);
        app.close(1);
        assert_eq!(app.tabs.len(), 1);
        app.reopen_closed(&ctx);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tabs[1].path, second);
    }
}
