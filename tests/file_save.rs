use mypdf_milestone0::{convert, file_save};

#[test]
fn repeated_atomic_saves_and_save_as_replace_only_expected_version() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("document.pdf");
    let first = convert::blank(500., 700.).unwrap();
    let second = convert::blank(600., 800.).unwrap();
    let baseline = file_save::write(&path, &first, None).unwrap();
    let saved = file_save::write(&path, &second, Some(baseline)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), second);
    file_save::write(&path, &first, Some(saved)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), first);
    let other = root.path().join("save-as.pdf");
    std::fs::write(&other, &second).unwrap();
    file_save::write(&other, &first, file_save::existing(&other).unwrap()).unwrap();
    assert_eq!(std::fs::read(other).unwrap(), first);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn external_edits_missing_files_and_readonly_files_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("document.pdf");
    let baseline = file_save::write(&path, b"opened", None).unwrap();
    std::fs::write(&path, b"external edit").unwrap();
    assert!(
        file_save::write(&path, b"my edit", Some(baseline))
            .unwrap_err()
            .to_string()
            .contains("changed outside")
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
    assert!(file_save::write(&path, b"new file", None).is_err());
    let original_permissions = std::fs::metadata(&path).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&path, readonly).unwrap();
    assert!(file_save::write(&path, b"my edit", file_save::existing(&path).unwrap()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
    std::fs::set_permissions(&path, original_permissions).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(file_save::write(&path, b"my edit", Some(baseline)).is_err());
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn replacement_preserves_permissions_and_rejects_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("document.pdf");
    let baseline = file_save::write(&path, b"opened", None).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    file_save::write(&path, b"saved", Some(baseline)).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let link = root.path().join("link.pdf");
    symlink(&path, &link).unwrap();
    assert!(file_save::existing(&link).is_err());
    assert!(file_save::write(&link, b"wrong", Some(file_save::fingerprint(b"saved"))).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"saved");
}
