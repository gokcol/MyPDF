//! Local document shortcuts. Clearing history never touches a PDF.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

const MAX_BYTES: usize = 1024 * 1024;
const MAX_RECENTS: usize = 50;
const MAX_FAVORITES: usize = 100;

/// Local view preferences; they never alter a PDF's opening instructions.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ReadingState {
    pub page: usize,
    pub top: f32,
    pub scale: Option<f32>,
    pub fit_page: bool,
    pub layout: u8,
    pub sidebar: bool,
    pub inspector: bool,
}
impl Default for ReadingState {
    fn default() -> Self {
        Self {
            page: 0,
            top: 0.,
            scale: None,
            fit_page: false,
            layout: 0,
            sidebar: true,
            inspector: true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub opened: i64,
    pub favorite: bool,
    #[serde(default)]
    pub reading: Option<ReadingState>,
}
impl Entry {
    pub fn name(&self) -> String {
        self.path
            .file_stem()
            .unwrap_or(self.path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
    pub fn date(&self) -> String {
        time::OffsetDateTime::from_unix_timestamp(self.opened / 1000)
            .map(|d| format!("{:02}.{:02}.{}", d.day(), u8::from(d.month()), d.year()))
            .unwrap_or_default()
    }
}

pub struct History {
    root: PathBuf,
    pub entries: Vec<Entry>,
}
impl History {
    pub fn load(root: PathBuf) -> Result<Self> {
        let entries = Self::read(&root)?;
        Ok(Self { root, entries })
    }
    fn read(root: &Path) -> Result<Vec<Entry>> {
        let path = root.join("home.json");
        ensure!(!path.is_symlink(), "Home history cannot be a symlink");
        let bytes = match crate::convert::read_bounded(&path, MAX_BYTES as u64) {
            Ok(bytes) => bytes,
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(vec![]);
            }
            Err(e) => return Err(e),
        };
        let entries: Vec<Entry> = serde_json::from_slice(&bytes)
            .context("Could not read Home history; its file is preserved")?;
        ensure!(
            entries.len() <= MAX_RECENTS + MAX_FAVORITES,
            "Home history exceeds its entry limit"
        );
        ensure!(
            entries.iter().all(|e| e.path.is_absolute()),
            "Home history contains an invalid path"
        );
        Ok(entries)
    }
    fn change(&mut self, apply: impl FnOnce(&mut Vec<Entry>)) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let lock_path = self.root.join("home.lock");
        ensure!(!lock_path.is_symlink(), "Home lock cannot be a symlink");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        lock.try_lock()
            .context("Home history is being updated by another MyPDF window")?;
        // Re-read under the lock so another window's favorites are never overwritten.
        let mut entries = Self::read(&self.root)?;
        apply(&mut entries);
        entries.sort_by(|a, b| b.opened.cmp(&a.opened).then_with(|| a.path.cmp(&b.path)));
        entries.dedup_by(|a, b| a.path == b.path);
        ensure!(
            entries.iter().filter(|e| e.favorite).count() <= MAX_FAVORITES,
            "The Favorites limit is 100 documents"
        );
        let mut recent = 0;
        for e in &mut entries {
            if e.opened > 0 {
                recent += 1;
                if recent > MAX_RECENTS {
                    e.opened = 0;
                }
            }
        }
        entries.retain(|e| e.opened > 0 || e.favorite);
        let raw = serde_json::to_vec(&entries)?;
        ensure!(
            raw.len() <= MAX_BYTES,
            "Home history exceeds its size limit"
        );
        let mut tmp = tempfile::NamedTempFile::new_in(&self.root)?;
        tmp.write_all(&raw)?;
        tmp.as_file().sync_all()?;
        tmp.persist(self.root.join("home.json"))
            .context("Could not save Home history")?;
        self.entries = entries;
        Ok(())
    }
    pub fn record(&mut self, path: &Path) -> Result<()> {
        let path = path.canonicalize()?;
        ensure!(path.is_file(), "Document is no longer available");
        let opened = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
        self.change(|entries| {
            let opened = opened.max(
                entries
                    .iter()
                    .map(|e| e.opened)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1),
            );
            if let Some(e) = entries.iter_mut().find(|e| e.path == path) {
                e.opened = opened;
            } else {
                entries.push(Entry {
                    path,
                    opened,
                    favorite: false,
                    reading: None,
                });
            }
        })
    }
    pub fn reading(&self, path: &Path) -> Option<ReadingState> {
        self.entries
            .iter()
            .find(|e| e.path == path)
            .and_then(|e| e.reading.clone())
    }
    pub fn save_reading(&mut self, states: &[(PathBuf, ReadingState)]) -> Result<()> {
        ensure!(
            states.iter().all(|(_, s)| s.top.is_finite()
                && s.top >= 0.
                && s.scale
                    .is_none_or(|v| v.is_finite() && (0.1..=6.).contains(&v))
                && s.layout <= 2),
            "Invalid reading position"
        );
        self.change(|entries| {
            for (path, state) in states {
                if let Some(e) = entries.iter_mut().find(|e| &e.path == path) {
                    e.reading = Some(state.clone());
                }
            }
        })
    }
    pub fn favorite(&mut self, path: &Path, favorite: bool) -> Result<()> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.change(|entries| {
            if let Some(e) = entries.iter_mut().find(|e| e.path == path) {
                e.favorite = favorite;
            }
        })
    }
    pub fn clear_recents(&mut self) -> Result<()> {
        self.change(|entries| {
            for e in entries {
                e.opened = 0;
            }
        })
    }
    pub fn remove_recent(&mut self, path: &Path) -> Result<()> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.change(|entries| {
            for e in entries {
                if e.path == path {
                    e.opened = 0;
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recents_and_favorites_survive_restart_and_clear_never_removes_files() {
        let temp = tempfile::tempdir().unwrap();
        let pdf = temp.path().join("Test.pdf");
        std::fs::write(&pdf, b"source untouched").unwrap();
        let mut a = History::load(temp.path().into()).unwrap();
        let mut b = History::load(temp.path().into()).unwrap();
        a.record(&pdf).unwrap();
        a.favorite(&pdf, true).unwrap();
        b.record(&pdf).unwrap();
        assert_eq!(b.entries.len(), 1);
        assert!(b.entries[0].favorite);
        let second = temp.path().join("Another.pdf");
        std::fs::write(&second, b"another source").unwrap();
        b.record(&second).unwrap();
        b.record(&pdf).unwrap();
        assert_eq!(b.entries[0].path, pdf.canonicalize().unwrap());
        assert!(b.entries[0].opened > b.entries[1].opened);
        b.clear_recents().unwrap();
        let mut loaded = History::load(temp.path().into()).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].opened, 0);
        loaded.favorite(&pdf, false).unwrap();
        assert!(loaded.entries.is_empty());
        assert_eq!(std::fs::read(pdf).unwrap(), b"source untouched");
    }
    #[test]
    fn corrupt_or_locked_history_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let mut history = History::load(temp.path().into()).unwrap();
        let path = temp.path().join("home.json");
        std::fs::write(&path, b"corrupted").unwrap();
        assert!(history.clear_recents().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"corrupted");
        std::fs::write(&path, b"[]").unwrap();
        let lock = std::fs::File::create(temp.path().join("home.lock")).unwrap();
        lock.lock().unwrap();
        assert!(history.clear_recents().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"[]");
    }
}
