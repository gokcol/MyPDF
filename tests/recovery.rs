use mypdf_milestone0::{convert, recovery::Store};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

#[test]
fn atomic_latest_copy_live_instance_exclusion_and_claims() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original.pdf");
    let blank = convert::blank(595., 842.).unwrap();
    fs::write(&original, &blank).unwrap();
    let root = temp.path().join("recovery");
    let first = Store::new(&root).unwrap();
    first.write(1, original.clone(), 1, 0, &blank).unwrap();
    let second_pdf = convert::blank(400., 500.).unwrap();
    first.write(1, original.clone(), 2, 1, &second_pdf).unwrap();
    let mut second = Store::new(&root).unwrap();
    assert!(
        second.scan().unwrap().0.is_empty(),
        "Live sessions must be excluded"
    );
    drop(first);
    let (entries, warnings) = second.scan().unwrap();
    assert!(warnings.is_empty());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].metadata.revision, 2);
    assert_eq!(second.read(&entries[0]).unwrap(), second_pdf);
    let mut third = Store::new(&root).unwrap();
    assert!(
        third.scan().unwrap().0.is_empty(),
        "Recovery claims must be exclusive"
    );
    second.remove(&entries[0]).unwrap();
    assert!(!entries[0].file.exists());
    assert_eq!(fs::read(original).unwrap(), blank);
}

#[test]
fn damaged_snapshots_are_retained_and_unowned_paths_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path()).unwrap();
    store
        .write(
            4,
            PathBuf::from("Untitled.pdf"),
            3,
            0,
            &convert::blank(200., 300.).unwrap(),
        )
        .unwrap();
    drop(store);
    let mut reader = Store::new(temp.path()).unwrap();
    let entry = reader.scan().unwrap().0.remove(0);
    let mut bytes = fs::read(&entry.file).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&entry.file, bytes).unwrap();
    assert!(
        reader
            .read(&entry)
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );
    assert!(entry.file.exists());
    let mut fake = entry.clone();
    fake.file = temp.path().join("outside.recovery");
    fs::write(&fake.file, b"preserve").unwrap();
    assert!(reader.read(&fake).is_err());
    assert!(reader.remove(&fake).is_err());
    assert_eq!(fs::read(fake.file).unwrap(), b"preserve");
}

#[cfg(unix)]
#[test]
fn recovery_refuses_symlinks_and_uses_private_permissions() {
    use std::os::unix::{fs::PermissionsExt, fs::symlink};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let store = Store::new(&root).unwrap();
    store
        .write(
            1,
            "Untitled.pdf".into(),
            1,
            0,
            &convert::blank(200., 200.).unwrap(),
        )
        .unwrap();
    drop(store);
    let mut reader = Store::new(&root).unwrap();
    let entry = reader.scan().unwrap().0.remove(0);
    assert_eq!(
        fs::metadata(&entry.file).unwrap().permissions().mode() & 0o077,
        0
    );
    let outside = temp.path().join("outside");
    fs::write(&outside, b"original").unwrap();
    fs::remove_file(&entry.file).unwrap();
    symlink(&outside, &entry.file).unwrap();
    assert!(reader.read(&entry).is_err());
    assert!(reader.remove(&entry).is_err());
    assert_eq!(fs::read(outside).unwrap(), b"original");
}

#[test]
fn recovery_child() {
    let Some(root) = std::env::var_os("MYPDF_RECOVERY_CHILD") else {
        return;
    };
    let root = PathBuf::from(root);
    let store = Store::new(&root.join("copies")).unwrap();
    store
        .write(
            7,
            "Crash proof.pdf".into(),
            9,
            0,
            &convert::blank(321., 456.).unwrap(),
        )
        .unwrap();
    fs::write(root.join("ready"), b"ready").unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
fn real_process_kill_releases_lock_and_recovers_complete_pdf() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "recovery_child", "--nocapture"])
        .env("MYPDF_RECOVERY_CHILD", temp.path())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !temp.path().join("ready").exists() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("Child did not save in time");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    let mut reader = Store::new(&temp.path().join("copies")).unwrap();
    let entries = reader.scan().unwrap().0;
    assert_eq!(entries.len(), 1);
    let pdf = reader.read(&entries[0]).unwrap();
    assert_eq!(entries[0].metadata.revision, 9);
    let geometry = mypdf_milestone0::annotations::page_geometry(&pdf).unwrap();
    assert_eq!(geometry[0].0, [0., 0., 321., 456.]);
}
