pub mod annotations;
pub mod assets;
pub mod content;
pub mod convert;
pub mod document;
pub mod forms;
pub mod header_footer;
pub mod home;
pub mod identity;
#[cfg(target_os = "macos")]
pub mod keychain;
pub mod pages;
pub mod pdf_font;
pub mod proof;
pub mod recovery;
pub mod redact;
pub mod signature_edit;
pub mod signing;
pub mod stroke;
pub mod svg;
pub mod text_block;
pub mod text_geometry;
pub mod text_style;
pub mod unicode_text;
pub mod viewer;
#[cfg(target_os = "windows")]
pub mod windows_store;

use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

/// Create a complete output beside its destination, then publish without replacing files.
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(path).with_context(|| {
        format!(
            "cannot create {}; existing files are preserved",
            path.display()
        )
    })?;
    Ok(())
}
