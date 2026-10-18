//! Small local preferences: bounded reads, atomic replacement and no symlink following.
use anyhow::{Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use std::{io::Write, path::Path};
pub(super) fn load<T: DeserializeOwned + Default>(root: &Path, name: &str) -> Result<T> {
    let path = root.join(name);
    ensure!(!path.is_symlink(), "Preference file cannot be a symlink");
    if !path.exists() {
        return Ok(T::default());
    }
    let bytes = mypdf_milestone0::convert::read_bounded(&path, 2 * 1024 * 1024)?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub(super) fn save<T: Serialize>(root: &Path, name: &str, value: &T) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let path = root.join(name);
    ensure!(!path.is_symlink(), "Preference file cannot be a symlink");
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "Preferences exceed their size limit"
    );
    let mut temp = tempfile::NamedTempFile::new_in(root)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}
