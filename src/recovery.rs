//! Private, atomic recovery copies. Never opens an original document for writing.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 8] = b"MYPDFRC1";
const LIMIT: u64 = crate::viewer::MAX_FILE_BYTES;
const BUDGET: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Metadata {
    pub original: PathBuf,
    pub revision: u64,
    pub page: usize,
    pub updated: u64,
    pub sha256: String,
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub file: PathBuf,
    pub metadata: Metadata,
}
pub struct Store {
    root: PathBuf,
    own: PathBuf,
    // Locks live for the whole session, including recovery decisions. Another
    // instance must never offer, delete, or replace a live instance's copies.
    locks: Vec<(PathBuf, File)>,
}
fn directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    ensure!(
        !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "Recovery directory is a symbolic link"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn regular(path: &Path) -> Result<()> {
    ensure!(
        fs::symlink_metadata(path)?.file_type().is_file(),
        "Recovery item is not a regular file"
    );
    Ok(())
}
fn header(file: &mut File) -> Result<Metadata> {
    let mut magic = [0; 8];
    file.read_exact(&mut magic)?;
    ensure!(&magic == MAGIC, "Unknown recovery format");
    let mut size = [0; 4];
    file.read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    ensure!(size <= 64 * 1024, "Recovery header is too large");
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
impl Store {
    pub fn new(root: &Path) -> Result<Self> {
        directory(root)?;
        let own = root.join(format!("session-{:032x}", rand::random::<u128>()));
        directory(&own)?;
        let lock = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(own.join("lock"))?;
        lock.try_lock()?;
        Ok(Self {
            root: root.into(),
            own: own.clone(),
            locks: vec![(own, lock)],
        })
    }
    pub fn scan(&mut self) -> Result<(Vec<Entry>, Vec<String>)> {
        let mut entries = vec![];
        let mut warnings = vec![];
        for dir in fs::read_dir(&self.root)?.take(512) {
            let dir = dir?;
            let name = dir.file_name().to_string_lossy().into_owned();
            if dir.path() == self.own || !name.starts_with("session-") || !dir.file_type()?.is_dir()
            {
                continue;
            }
            let lockpath = dir.path().join("lock");
            if regular(&lockpath).is_err() {
                continue;
            }
            let lock = OpenOptions::new().read(true).write(true).open(&lockpath)?;
            if lock.try_lock().is_err() {
                continue;
            }
            self.locks.push((dir.path(), lock));
            for item in fs::read_dir(dir.path())?.take(64) {
                let item = item?;
                if item.path().extension().is_none_or(|s| s != "recovery") {
                    continue;
                }
                let result = (|| -> Result<Entry> {
                    regular(&item.path())?;
                    let mut file = File::open(item.path())?;
                    ensure!(
                        file.metadata()?.len() <= LIMIT + 65548,
                        "Recovery copy is too large"
                    );
                    Ok(Entry {
                        file: item.path(),
                        metadata: header(&mut file)?,
                    })
                })();
                match result {
                    Ok(e) => entries.push(e),
                    Err(e) => {
                        warnings.push(format!("Recovery copy retained but unavailable: {e:#}"))
                    }
                }
            }
        }
        entries.sort_by_key(|e| std::cmp::Reverse(e.metadata.updated));
        Ok((entries, warnings))
    }
    fn owned(&self, path: &Path) -> Result<()> {
        ensure!(
            path.extension().is_some_and(|s| s == "recovery")
                && self
                    .locks
                    .iter()
                    .any(|(dir, _)| path.parent() == Some(dir.as_path())),
            "Recovery path is not owned by this session"
        );
        Ok(())
    }
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>> {
        self.owned(&entry.file)?;
        regular(&entry.file)?;
        let mut file = File::open(&entry.file)?;
        let metadata = header(&mut file)?;
        let mut bytes = vec![];
        file.take(LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= LIMIT && bytes.starts_with(b"%PDF-"),
            "Invalid recovery PDF"
        );
        ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == metadata.sha256,
            "Recovery checksum failed; the copy was preserved"
        );
        // An unlocked signed PDF is saved here as an incremental update. The
        // editing permission is process-local, so it is not present after a
        // restart. Reading a recovery copy must not require that permission or
        // rewrite the signed bytes; the desktop restores its editing state.
        let doc = lopdf::Document::load_mem(&bytes).context("Invalid recovery PDF")?;
        ensure!(
            !doc.is_encrypted(),
            "Encrypted recovery PDFs cannot be restored; the copy was retained"
        );
        Ok(bytes)
    }
    pub fn write(
        &self,
        id: u64,
        original: PathBuf,
        revision: u64,
        page: usize,
        bytes: &[u8],
    ) -> Result<()> {
        ensure!(
            bytes.len() as u64 <= LIMIT && bytes.starts_with(b"%PDF-"),
            "Invalid recovery PDF"
        );
        let path = self.own.join(format!("{id}.recovery"));
        if path.exists() {
            regular(&path)?;
        }
        let used: u64 = fs::read_dir(&self.own)?
            .filter_map(Result::ok)
            .filter(|e| e.path() != path)
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        ensure!(
            used + bytes.len() as u64 <= BUDGET,
            "Recovery storage limit reached (2 GB). Save a copy of your work."
        );
        let metadata = Metadata {
            original,
            revision,
            page,
            updated: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
        };
        let header = serde_json::to_vec(&metadata)?;
        ensure!(header.len() <= 64 * 1024, "Recovery metadata too large");
        let mut tmp = tempfile::NamedTempFile::new_in(&self.own)?;
        tmp.write_all(MAGIC)?;
        tmp.write_all(&(header.len() as u32).to_le_bytes())?;
        tmp.write_all(&header)?;
        tmp.write_all(bytes)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&path)?;
        #[cfg(unix)]
        File::open(&self.own)?.sync_all()?;
        Ok(())
    }
    pub fn remove(&self, entry: &Entry) -> Result<()> {
        self.remove_path(&entry.file)
    }
    pub fn clear(&self, id: u64) -> Result<()> {
        self.remove_path(&self.own.join(format!("{id}.recovery")))
    }
    fn remove_path(&self, path: &Path) -> Result<()> {
        self.owned(path)?;
        if path.exists() {
            regular(path)?;
            fs::remove_file(path)?;
        }
        Ok(())
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        // Only remove empty owned session directories. Recoverable work survives.
        for (dir, _) in &self.locks {
            if fs::read_dir(dir)
                .is_ok_and(|mut items| items.all(|e| e.is_ok_and(|e| e.file_name() == "lock")))
            {
                let _ = fs::remove_file(dir.join("lock"));
                let _ = fs::remove_dir(dir);
            }
        }
    }
}
