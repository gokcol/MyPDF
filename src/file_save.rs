//! Atomic document saves, with a baseline to detect external changes.
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path};

pub type Fingerprint = [u8; 32];

pub fn fingerprint(bytes: &[u8]) -> Fingerprint {
    Sha256::digest(bytes).into()
}

/// Snapshot a Save As destination after the native dialog confirms replacement.
pub fn existing(path: &Path) -> Result<Option<Fingerprint>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_file() || meta.file_type().is_symlink() {
                bail!("Choose a regular PDF file, not a folder or symbolic link");
            }
            let bytes = crate::convert::read_bounded(path, crate::viewer::MAX_FILE_BYTES)?;
            Ok(Some(fingerprint(&bytes)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Replace only the version that was opened/last saved (or explicitly selected
/// for replacement). New paths keep the no-clobber guarantee of other exports.
pub fn write(path: &Path, bytes: &[u8], expected: Option<Fingerprint>) -> Result<Fingerprint> {
    let Some(expected) = expected else {
        crate::write_new(path, bytes)?;
        return Ok(fingerprint(bytes));
    };
    let check = || -> Result<std::fs::Permissions> {
        let meta = std::fs::symlink_metadata(path)
            .context("The file is no longer available. Use Save As to choose a destination")?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            bail!("The destination changed. Use Save As to choose a destination");
        }
        if meta.permissions().readonly() {
            bail!("The PDF is read-only. Use Save As to save elsewhere");
        }
        if existing(path)? != Some(expected) {
            bail!("The PDF changed outside MyPDF. Use Save As to preserve both versions");
        }
        Ok(meta.permissions())
    };
    let permissions = check()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().set_permissions(permissions)?;
    temporary.as_file().sync_all()?;
    // Recheck after export/write, just before the atomic replacement.
    check()?;
    temporary
        .persist(path)
        .with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(fingerprint(bytes))
}
