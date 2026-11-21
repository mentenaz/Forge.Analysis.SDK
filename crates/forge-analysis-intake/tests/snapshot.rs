use forge_analysis_intake::{snapshot, snapshot_from_memory, IntakeError, MemFile, Options, SkipReason};
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
    fn path(&self) -> &Path {
        &self.0
    }
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
    let h1 = snapshot(t.path(), &Options::default())
        .unwrap()
        .sha256()
        .clone();
    t.write("a.js", b"2");
    let h2 = snapshot(t.path(), &Options::default())
        .unwrap()
        .sha256()
        .clone();
    assert_ne!(h1, h2);
}

#[test]
fn node_modules_and_git_are_skipped_and_recorded() {
    let t = Tmp::new("ignored");
    t.write("node_modules/x.js", b"1");
    t.write(".git/HEAD", b"1"); // not one of the two files intake reads inside .git
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
    let opts = Options {
        max_files: 1,
        ..Options::default()
    };
    let snap = snapshot(t.path(), &opts).unwrap();
    assert_eq!(snap.files().len(), 1);
    assert_eq!(count(&snap, |r| matches!(r, SkipReason::FileCapReached)), 2);
}

#[test]
fn oversized_file_is_skipped_with_its_size() {
    let t = Tmp::new("big");
    t.write("big.js", b"0123456789");
    let opts = Options {
        max_file_bytes: 4,
        ..Options::default()
    };
    let snap = snapshot(t.path(), &opts).unwrap();
    assert!(snap.files().is_empty());
    assert!(snap
        .skipped()
        .iter()
        .any(|s| s.reason == SkipReason::TooLarge { size: 10 }));
}

#[test]
fn total_size_cap_stops_further_files() {
    let t = Tmp::new("total");
    t.write("a.js", b"123456");
    t.write("b.js", b"123456");
    let opts = Options {
        max_total_bytes: 8,
        ..Options::default()
    };
    let snap = snapshot(t.path(), &opts).unwrap();
    assert_eq!(snap.files().len(), 1);
    assert_eq!(
        count(&snap, |r| matches!(r, SkipReason::TotalCapReached)),
        1
    );
}

#[test]
fn depth_cap_skips_deeper_folders() {
    let t = Tmp::new("depth");
    t.write("a/x.js", b"1");
    t.write("a/b/c.js", b"1");
    let opts = Options {
        max_depth: 1,
        ..Options::default()
    };
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
        assert!(
            !src.contains(banned),
            "intake must stay read-only, found {banned}"
        );
    }
}

fn mem(files: &[(&str, &[u8])]) -> Vec<MemFile> {
    files.iter().map(|(p, b)| MemFile { path: p.to_string(), bytes: b.to_vec() }).collect()
}

fn paths(snap: &forge_analysis_intake::Snapshot) -> Vec<String> {
    snap.files().iter().map(|f| f.path.as_quoted().text().to_string()).collect()
}

#[test]
fn memory_and_disk_give_the_same_snapshot() {
    let files: Vec<(&str, &[u8])> = vec![
        ("package.json", b"{\"name\":\"a\"}"),
        ("src/index.js", b"console.log(1)"),
        ("src/util.ts", b"export const x = 1"),
        ("README.md", b"# hi"),
        ("node_modules/dep/index.js", b"module.exports = 1"),
    ];
    let t = Tmp::new("parity");
    for (p, b) in &files {
        t.write(p, b);
    }
    let disk = snapshot(t.path(), &Options::default()).unwrap();
    let mut reversed = mem(&files);
    reversed.reverse();
    let from_mem = snapshot_from_memory(reversed, &Options::default());
    // The disk walk records an ignored folder once; memory records each file in it. So the
    // included files and sizes are compared here, and the hash on a run with no ignored folder.
    assert_eq!(paths(&disk), paths(&from_mem));
    assert_eq!(disk.total_bytes(), from_mem.total_bytes());

    let plain = &files[..4];
    let t = Tmp::new("parity2");
    for (p, b) in plain {
        t.write(p, b);
    }
    let disk = snapshot(t.path(), &Options::default()).unwrap();
    let from_mem = snapshot_from_memory(mem(plain), &Options::default());
    assert_eq!(disk.sha256(), from_mem.sha256());
}

fn git_project(t: &Tmp) {
    t.write("package.json", b"{}");
    t.write(".git/config", b"[core]\n\tfsmonitor = x\n");
    t.write(".git/hooks/pre-commit", b"#!/bin/sh\n");
    t.write(".git/hooks/pre-commit.sample", b"#!/bin/sh\n");
    t.write(".git/HEAD", b"ref: refs/heads/main\n");
    t.write(".git/objects/aa/bb", b"x");
    t.write("sub/.git/config", b"[core]\n");
}

#[test]
fn git_config_and_active_hooks_are_read_and_the_rest_of_git_is_still_skipped() {
    let t = Tmp::new("git");
    git_project(&t);
    let snap = snapshot(t.path(), &Options::default()).unwrap();
    assert_eq!(paths(&snap), vec![".git/config", ".git/hooks/pre-commit", "package.json", "sub/.git/config"]);
    // Each `.git` folder is still recorded once as ignored, so the rest of it is counted.
    let ignored: Vec<_> = snap
        .skipped()
        .iter()
        .filter(|s| matches!(s.reason, SkipReason::IgnoredDir))
        .map(|s| s.path.text().to_string())
        .collect();
    assert_eq!(ignored, vec![".git", "sub/.git"]);
    // Changing a hook changes the snapshot hash.
    let before = snap.sha256().clone();
    t.write(".git/hooks/pre-commit", b"#!/bin/sh\nid\n");
    assert_ne!(snapshot(t.path(), &Options::default()).unwrap().sha256(), &before);
    // Changing something intake does not read does not.
    let before = snapshot(t.path(), &Options::default()).unwrap().sha256().clone();
    t.write(".git/HEAD", b"ref: refs/heads/other\n");
    assert_eq!(snapshot(t.path(), &Options::default()).unwrap().sha256(), &before);
}

#[test]
fn memory_input_makes_the_same_git_exception() {
    let t = Tmp::new("gitmem");
    git_project(&t);
    let disk = snapshot(t.path(), &Options::default()).unwrap();
    let from_mem = snapshot_from_memory(
        mem(&[
            ("package.json", b"{}"),
            (".git/config", b"[core]\n\tfsmonitor = x\n"),
            (".git/hooks/pre-commit", b"#!/bin/sh\n"),
            (".git/hooks/pre-commit.sample", b"#!/bin/sh\n"),
            (".git/HEAD", b"ref: refs/heads/main\n"),
            (".git/objects/aa/bb", b"x"),
            ("sub/.git/config", b"[core]\n"),
            // Not let through: a folder deeper in `.git`, and `.git` inside another ignored folder.
            (".git/hooks/nested/x", b"x"),
            ("node_modules/p/.git/config", b"x"),
        ]),
        &Options::default(),
    );
    assert_eq!(paths(&disk), paths(&from_mem));
}

#[test]
fn a_huge_git_config_is_recorded_as_too_large_not_hidden() {
    let t = Tmp::new("gitbig");
    t.write(".git/config", &vec![b'x'; 200]);
    let opts = Options { max_file_bytes: 100, ..Options::default() };
    let snap = snapshot(t.path(), &opts).unwrap();
    assert!(paths(&snap).is_empty());
    assert!(snap
        .skipped()
        .iter()
        .any(|s| matches!(s.reason, SkipReason::TooLarge { size: 200 }) && s.path.text() == ".git/config"));
}

#[test]
fn a_linked_hooks_folder_is_recorded_and_never_followed() {
    let t = Tmp::new("gitlink");
    let outside = Tmp::new("gitlink-outside");
    outside.write("pre-commit", b"#!/bin/sh\n");
    t.write(".git/config", b"[core]\n");
    if !try_symlink_dir(outside.path(), &t.path().join(".git").join("hooks")) {
        return; // links need privileges on some machines
    }
    let snap = snapshot(t.path(), &Options::default()).unwrap();
    assert_eq!(paths(&snap), vec![".git/config"]);
    assert!(snap
        .skipped()
        .iter()
        .any(|s| matches!(s.reason, SkipReason::Symlink { .. }) && s.path.text() == ".git/hooks"));
}
