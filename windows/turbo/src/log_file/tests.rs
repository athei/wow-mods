use std::{fs, sync::atomic::AtomicU32, time::Duration};

use super::*;

pub struct TestDir(pub PathBuf);

impl TestDir {
    pub fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let primary = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|dir| dir.join(".git").is_dir())
            .expect("primary checkout ancestor");
        let path = primary
            .join(".codex/evidence/weirdutils-lua-inflate/log-file-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn executable_directory_does_not_depend_on_working_directory() {
    let root = TestDir::new();
    let exe = root.0.join("game with spaces and ünicode").join("WoW.exe");
    assert_eq!(
        directory_for(&exe).unwrap(),
        exe.parent().unwrap().join("Logs/wow_turbo")
    );
}

#[test]
fn repeated_pid_and_timestamp_create_distinct_unbuffered_sessions() {
    let root = TestDir::new();
    let dir = root.0.join("Logs/wow_turbo");
    let (mut first, first_path) = create_session(&dir, UNIX_EPOCH, 88).unwrap();
    first.write_all(b"first session\n").unwrap();
    let (mut second, second_path) = create_session(&dir, UNIX_EPOCH, 88).unwrap();
    second.write_all(b"second session\n").unwrap();
    assert_ne!(first_path, second_path);
    // Read before closing or flushing either writer.
    assert_eq!(fs::read(first_path).unwrap(), b"first session\n");
    assert_eq!(fs::read(second_path).unwrap(), b"second session\n");
}

#[test]
fn retention_keeps_ten_sessions_and_leaves_unrelated_entries_alone() {
    let root = TestDir::new();
    for age in 1..=15 {
        let (file, _) = create_session(&root.0, UNIX_EPOCH + Duration::from_secs(age), 88).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_secs(age))
            .unwrap();
    }
    let (_, active) = create_session(&root.0, SystemTime::now(), 88).unwrap();
    fs::write(root.0.join("CombatLog.txt"), b"unrelated").unwrap();
    fs::write(root.0.join("wow_turbo-not-a-session.log"), b"unrelated").unwrap();
    fs::create_dir(root.0.join("wow_turbo-0-0-0.log")).unwrap();
    prune(&active, KEEP);
    assert!(active.exists());
    for age in 1..=15 {
        assert_eq!(
            root.0
                .join(format!("wow_turbo-{}-88-0.log", age * 1000))
                .exists(),
            age >= 7
        );
    }
    assert_eq!(
        fs::read(root.0.join("CombatLog.txt")).unwrap(),
        b"unrelated"
    );
    assert!(root.0.join("wow_turbo-not-a-session.log").exists());
    assert!(root.0.join("wow_turbo-0-0-0.log").is_dir());
}

#[test]
fn unavailable_log_directory_is_an_error_without_touching_existing_data() {
    let root = TestDir::new();
    let blocked = root.0.join("Logs");
    fs::write(&blocked, b"keep this file").unwrap();
    assert!(create_session(&blocked.join("wow_turbo"), SystemTime::now(), 88).is_err());
    assert_eq!(fs::read(blocked).unwrap(), b"keep this file");
}
