use forge_analysis_intake::{snapshot, IntakeError, Options, SkipReason};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Temp folder that deletes itself. Fixtures are written here, never by the crate under test.
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("forge-intake-{}-{tag}-{n}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn path(&self) -> &Path { &self.0 }
    fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.0.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn try_symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}
#[cfg(windows)]
fn try_symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}
#[cfg(not(any(unix, windows)))]
fn try_symlink_dir(_target: &Path, _link: &Path) -> bool {
    false
}

fn count(snap: &forge_analysis_intake::Snapshot, f: impl Fn(&SkipReason) -> bool) -> usize {
    snap.skipped().iter().filter(|s| f(&s.reason)).count()
}

#[test]
fn source_files_get_text_and_other_files_only_a_hash() {
    let t = Tmp::new("basic");
    t.write("a/b.js", b"let x = 1;");
    t.write("c.txt", b"notes");
    let snap = snapshot(t.path(), &Options::default()).unwrap();
    assert_eq!(snap.files().len(), 2);
    let b = &snap.files()[0];
    assert_eq!(b.path.as_quoted().text(), "a/b.js");
    assert_eq!(b.text.as_deref(), Some("let x = 1;"));
    let c = &snap.files()[1];
    assert_eq!(c.path.as_quoted().text(), "c.txt");
    assert!(c.text.is_none());
}

#[test]
fn same_content_gives_the_same_hash_in_any_folder() {
    let a = Tmp::new("same-a");
    let b = Tmp::new("same-b");
    for t in [&a, &b] {
        t.write("src/x.js", b"1");
        t.write("y.json", b"{}");
    }
    let sa = snapshot(a.path(), &Options::default()).unwrap();
    let sb = snapshot(b.path(), &Options::default()).unwrap();
    assert_eq!(sa.sha256(), sb.sha256());
}

#[test]
fn changing_a_file_changes_the_hash() {
    let t = Tmp::new("change");
    t.write("a.js", b"1");
    let h1 = snapshot(t.path(), &Options::default()).unwrap().sha256().clone();
    t.write("a.js", b"2");
    let h2 = snapshot(t.path(), &Options::default()).unwrap().sha256().clone();
    assert_ne!(h1, h2);
}

#[test]
fn node_modules_and_git_are_skipped_and_recorded() {
    let t = Tmp::new("ignored");
    t.write("node_modules/x.js", b"1");
    t.write(".git/config", b"1");
    t.write("src/a.js", b"1");
    let snap = snapshot(t.path(), &Options::default()).unwrap();
    assert_eq!(snap.files().len(), 1);
    assert_eq!(count(&snap, |r| matches!(r, SkipReason::IgnoredDir)), 2);
}

#[test]
fn file_count_cap_skips_and_records_the_rest() {
    let t = Tmp::new("filecap");
    for n in ["a.js", "b.js", "c.js"] {
        t.write(n, b"1");
    }
    let opts = Options { max_files: 1, ..Options::default() };
    let snap = snapshot(t.path(), &opts).unwrap();
    assert_eq!(snap.files().len(), 1);
    assert_eq!(count(&snap, |r| matches!(r, SkipReason::FileCapReached)), 2);
}

#[test]
fn oversized_file_is_skipped_with_its_size() {
    let t = Tmp::new("big");
    t.write("big.js", b"0123456789");
    let opts = Options { max_file_bytes: 4, ..Options::default() };
    let snap = snapshot(t.path(), &opts).unwrap();
    assert!(snap.files().is_empty());
    assert!(snap.skipped().iter().any(|s| s.reason == SkipReason::TooLarge { size: 10 }));
}

#[test]
fn total_size_cap_stops_further_files() {
    let t = Tmp::new("total");
    t.write("a.js", b"123456");
    t.write("b.js", b"123456");
    let opts = Options { max_total_bytes: 8, ..Options::default() };
    let snap = snapshot(t.path(), &opts).unwrap();
    assert_eq!(snap.files().len(), 1);
    assert_eq!(count(&snap, |r| matches!(r, SkipReason::TotalCapReached)), 1);
}

#[test]
fn depth_cap_skips_deeper_folders() {
    let t = Tmp::new("depth");
    t.write("a/x.js", b"1");
    t.write("a/b/c.js", b"1");
    let opts = Options { max_depth: 1, ..Options::default() };
    let snap = snapshot(t.path(), &opts).unwrap();
    assert_eq!(snap.files().len(), 1);
    assert_eq!(count(&snap, |r| matches!(r, SkipReason::TooDeep)), 1);
}

#[test]
fn invalid_utf8_is_flagged_not_hidden() {
    let t = Tmp::new("utf8");
    t.write("x.js", &[0xff, 0xfe, 0x41]);
    let snap = snapshot(t.path(), &Options::default()).unwrap();
    let f = &snap.files()[0];
    assert!(f.invalid_utf8);
    assert!(f.text.as_deref().unwrap().contains('A'));
}

#[test]
fn symlinks_are_recorded_and_never_followed() {
    let outside = Tmp::new("outside");
    outside.write("secret.js", b"steal");
    let root = Tmp::new("linkroot");
    root.write("ok.js", b"1");
    if !try_symlink_dir(outside.path(), &root.path().join("link")) {
        eprintln!("symlinks not permitted here; skipping this test");
        return;
    }
    let snap = snapshot(root.path(), &Options::default()).unwrap();
    assert_eq!(snap.files().len(), 1);
    assert_eq!(count(&snap, |r| matches!(r, SkipReason::Symlink { .. })), 1);
}

#[test]
fn a_file_is_not_a_valid_root() {
    let t = Tmp::new("notdir");
    t.write("f.js", b"1");
    let r = snapshot(&t.path().join("f.js"), &Options::default());
    assert_eq!(r.unwrap_err(), IntakeError::NotADirectory);
}

#[test]
fn a_missing_root_is_an_error() {
    let t = Tmp::new("missing");
    let r = snapshot(&t.path().join("nope"), &Options::default());
    assert!(matches!(r, Err(IntakeError::Root(_))));
}

#[test]
fn crate_source_contains_no_write_apis() {
    let src = include_str!("../src/lib.rs");
    for banned in [
        "File::create",
        "fs::write",
        "OpenOptions",
        "remove_file",
        "remove_dir",
        "create_dir",
        "fs::rename",
        "set_permissions",
        "fs::copy",
        "hard_link",
        "os::unix::fs::symlink",
        "os::windows::fs::symlink",
    ] {
        assert!(!src.contains(banned), "intake must stay read-only, found {banned}");
    }
}
