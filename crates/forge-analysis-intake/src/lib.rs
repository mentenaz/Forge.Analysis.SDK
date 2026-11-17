//! Intake: turns a folder into a hashed, read-only snapshot.
//! Read-only by construction: nothing here opens a file for writing.
//! Symlinks and junctions are never followed. Everything skipped is recorded with a reason.

use forge_analysis_schema::{Hash, Quoted, RelPath};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Hard bound on how many skipped items are recorded; the rest are only counted.
pub const MAX_SKIPPED_RECORDS: usize = 10_000;

#[derive(Debug, Clone)]
pub struct Options {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_files: usize,
    pub max_depth: usize,
    pub ignored_dirs: Vec<String>,
    pub source_extensions: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_file_bytes: 2 * 1024 * 1024,
            max_total_bytes: 200 * 1024 * 1024,
            max_files: 50_000,
            max_depth: 64,
            ignored_dirs: vec!["node_modules".into(), ".git".into()],
            source_extensions: ["js", "mjs", "cjs", "ts", "tsx", "jsx", "json"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    Symlink { target: Quoted },
    IgnoredDir,
    TooDeep,
    TooLarge { size: u64 },
    TotalCapReached,
    FileCapReached,
    NotRegular,
    OutsideRoot,
    BadPath,
    ReservedName,
    ReadError,
}

impl SkipReason {
    fn tag(&self) -> &'static str {
        match self {
            SkipReason::Symlink { .. } => "symlink",
            SkipReason::IgnoredDir => "ignored_dir",
            SkipReason::TooDeep => "too_deep",
            SkipReason::TooLarge { .. } => "too_large",
            SkipReason::TotalCapReached => "total_cap",
            SkipReason::FileCapReached => "file_cap",
            SkipReason::NotRegular => "not_regular",
            SkipReason::OutsideRoot => "outside_root",
            SkipReason::BadPath => "bad_path",
            SkipReason::ReservedName => "reserved_name",
            SkipReason::ReadError => "read_error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub path: Quoted,
    pub reason: SkipReason,
}

#[derive(Debug, Clone)]
pub struct SnapshotFile {
    pub path: RelPath,
    pub sha256: Hash,
    pub size: u64,
    /// Only set for files with a source extension.
    pub text: Option<String>,
    /// True if the bytes were not valid UTF-8 (text was decoded with replacement).
    pub invalid_utf8: bool,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    files: Vec<SnapshotFile>,
    skipped: Vec<Skipped>,
    skipped_dropped: u64,
    total_bytes: u64,
    sha256: Hash,
}

impl Snapshot {
    pub fn files(&self) -> &[SnapshotFile] {
        &self.files
    }
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }
    pub fn skipped_dropped(&self) -> u64 {
        self.skipped_dropped
    }
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
    pub fn sha256(&self) -> &Hash {
        &self.sha256
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntakeError {
    Root(String),
    NotADirectory,
}

impl std::fmt::Display for IntakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for IntakeError {}

pub fn snapshot(root: &Path, opts: &Options) -> Result<Snapshot, IntakeError> {
    let root_canon = fs::canonicalize(root).map_err(|e| IntakeError::Root(e.to_string()))?;
    if !root_canon.is_dir() {
        return Err(IntakeError::NotADirectory);
    }

    let mut files: Vec<(String, SnapshotFile)> = Vec::new();
    let mut skipped: Vec<(String, Skipped)> = Vec::new();
    let mut dropped = 0u64;
    let mut total = 0u64;
    let mut stack: Vec<(PathBuf, String, usize)> = vec![(root_canon.clone(), String::new(), 0)];

    while let Some((dir, rel_dir, depth)) = stack.pop() {
        let reader = match fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => {
                record(&mut skipped, &mut dropped, &rel_dir, SkipReason::ReadError);
                continue;
            }
        };
        let mut entries = Vec::new();
        for item in reader {
            match item {
                Ok(e) => entries.push(e),
                Err(_) => record(&mut skipped, &mut dropped, &rel_dir, SkipReason::ReadError),
            }
        }
        // Sorted so the walk order, and therefore the cap behaviour, is deterministic.
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if rel_dir.is_empty() {
                name.clone()
            } else {
                format!("{rel_dir}/{name}")
            };
            let path = entry.path();

            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => {
                    record(&mut skipped, &mut dropped, &rel, SkipReason::ReadError);
                    continue;
                }
            };

            // Never follow links. On Windows this also covers junctions.
            if ft.is_symlink() {
                let target = fs::read_link(&path)
                    .map(|t| t.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let reason = SkipReason::Symlink {
                    target: Quoted::new(&target),
                };
                record(&mut skipped, &mut dropped, &rel, reason);
                continue;
            }
            if is_reserved_name(&name) {
                record(&mut skipped, &mut dropped, &rel, SkipReason::ReservedName);
                continue;
            }

            if ft.is_dir() {
                if opts.ignored_dirs.iter().any(|d| d == &name) {
                    record(&mut skipped, &mut dropped, &rel, SkipReason::IgnoredDir);
                    continue;
                }
                if depth + 1 > opts.max_depth {
                    record(&mut skipped, &mut dropped, &rel, SkipReason::TooDeep);
                    continue;
                }
                if !inside(&root_canon, &path) {
                    record(&mut skipped, &mut dropped, &rel, SkipReason::OutsideRoot);
                    continue;
                }
                stack.push((path, rel, depth + 1));
                continue;
            }

            if !ft.is_file() {
                record(&mut skipped, &mut dropped, &rel, SkipReason::NotRegular);
                continue;
            }
            let rel_path = match RelPath::new(&rel) {
                Some(p) => p,
                None => {
                    record(&mut skipped, &mut dropped, &rel, SkipReason::BadPath);
                    continue;
                }
            };
            if !inside(&root_canon, &path) {
                record(&mut skipped, &mut dropped, &rel, SkipReason::OutsideRoot);
                continue;
            }
            if files.len() >= opts.max_files {
                record(&mut skipped, &mut dropped, &rel, SkipReason::FileCapReached);
                continue;
            }
            let size = match entry.metadata() {
                Ok(m) => m.len(),
                Err(_) => {
                    record(&mut skipped, &mut dropped, &rel, SkipReason::ReadError);
                    continue;
                }
            };
            if size > opts.max_file_bytes {
                record(
                    &mut skipped,
                    &mut dropped,
                    &rel,
                    SkipReason::TooLarge { size },
                );
                continue;
            }
            if total.saturating_add(size) > opts.max_total_bytes {
                record(
                    &mut skipped,
                    &mut dropped,
                    &rel,
                    SkipReason::TotalCapReached,
                );
                continue;
            }

            // Read once, hash the exact bytes read, never read the file again.
            let bytes = match read_capped(&path, opts.max_file_bytes) {
                Ok(Some(b)) => b,
                Ok(None) => {
                    // grew past the cap between the size check and the read
                    record(
                        &mut skipped,
                        &mut dropped,
                        &rel,
                        SkipReason::TooLarge {
                            size: size.max(opts.max_file_bytes + 1),
                        },
                    );
                    continue;
                }
                Err(_) => {
                    record(&mut skipped, &mut dropped, &rel, SkipReason::ReadError);
                    continue;
                }
            };
            let read_len = bytes.len() as u64;
            total += read_len;
            let sha256 = Hash::of(&bytes);

            let (text, invalid_utf8) = if is_source(&name, &opts.source_extensions) {
                match String::from_utf8(bytes) {
                    Ok(s) => (Some(s), false),
                    Err(e) => (
                        Some(String::from_utf8_lossy(e.as_bytes()).into_owned()),
                        true,
                    ),
                }
            } else {
                (None, false)
            };

            files.push((
                rel,
                SnapshotFile {
                    path: rel_path,
                    sha256,
                    size: read_len,
                    text,
                    invalid_utf8,
                },
            ));
        }
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));
    skipped.sort_by(|a, b| a.0.cmp(&b.0));

    // Snapshot hash: every included file (path, content hash, size), every skipped
    // item (path, reason) and the overflow count. Nothing about the root folder's own
    // name or location, so the same content gives the same hash anywhere.
    let mut lines: Vec<String> = Vec::new();
    for (rel, f) in &files {
        lines.push(format!("F\0{rel}\0{}\0{}", f.sha256.as_str(), f.size));
    }
    for (rel, s) in &skipped {
        lines.push(format!("S\0{rel}\0{}", s.reason.tag()));
    }
    lines.push(format!("D\0{dropped}"));
    lines.sort();
    let sha256 = Hash::of(lines.join("\n").as_bytes());

    Ok(Snapshot {
        files: files.into_iter().map(|(_, f)| f).collect(),
        skipped: skipped.into_iter().map(|(_, s)| s).collect(),
        skipped_dropped: dropped,
        total_bytes: total,
        sha256,
    })
}

fn record(skipped: &mut Vec<(String, Skipped)>, dropped: &mut u64, rel: &str, reason: SkipReason) {
    if skipped.len() >= MAX_SKIPPED_RECORDS {
        *dropped += 1;
        return;
    }
    let shown = if rel.is_empty() { "." } else { rel };
    skipped.push((
        shown.to_string(),
        Skipped {
            path: Quoted::new(shown),
            reason,
        },
    ));
}

/// The jail check: the resolved location must be inside the canonical root.
fn inside(root_canon: &Path, p: &Path) -> bool {
    fs::canonicalize(p)
        .map(|c| c.starts_with(root_canon))
        .unwrap_or(false)
}

/// Reads at most `max` bytes. Returns None if the file holds more than that.
fn read_capped(path: &Path, max: u64) -> std::io::Result<Option<Vec<u8>>> {
    let f = fs::File::open(path)?;
    let mut buf = Vec::new();
    f.take(max + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > max {
        Ok(None)
    } else {
        Ok(Some(buf))
    }
}

fn is_source(name: &str, exts: &[String]) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .map(|e| exts.iter().any(|s| *s == e))
        .unwrap_or(false)
}

/// Windows device names (CON, NUL, COM1 ...) can hang or misbehave when opened.
fn is_reserved_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    if stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) {
        return matches!(stem.chars().last(), Some('1'..='9'));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_device_names_are_caught() {
        for n in ["CON", "con.txt", "NUL", "aux.js", "COM1", "lpt9.ts", "Prn "] {
            assert!(is_reserved_name(n), "{n}");
        }
        for n in ["console.js", "auxiliary.js", "COM10", "com0", "index.ts"] {
            assert!(!is_reserved_name(n), "{n}");
        }
    }

    #[test]
    fn source_extension_check_is_case_insensitive() {
        let exts = vec!["js".to_string(), "ts".to_string()];
        assert!(is_source("A.JS", &exts));
        assert!(is_source("x.ts", &exts));
        assert!(!is_source("x.txt", &exts));
        assert!(!is_source("Makefile", &exts));
    }
}
