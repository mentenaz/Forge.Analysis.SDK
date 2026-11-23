//! Input in, report out: intake, then every file, then the project link, then the report.

use forge_analysis_catalog::Catalog;
use forge_analysis_core::imports::{
    analyze_config, analyze_file_full, analyze_package, is_config_manifest, link_project, Analysis, EntryWant,
    FileIndex, LinkFile,
};
use forge_analysis_intake::{snapshot, snapshot_from_memory, IntakeError, MemFile, Options, SkipReason, Snapshot};
use forge_analysis_report::{
    build, classify_path, is_auto_run_code, is_auto_run_config, BuildError, FileInput, NotAnalysedFile,
    NotAnalysedKind, Report, ReportInput,
};
use forge_analysis_schema::{Header, Mode, Quoted, RelPath, Scope, StaticFact};
use std::collections::{BTreeSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Version of the report's shape.
///
/// Bump it whenever the JSON changes in a way a reader could meet: a new or renamed field, a new
/// value of an enum that is written out, or a changed meaning. Removing or renaming always bumps.
/// A change that only alters what the engine finds, and not the shape, does not bump it. Golden
/// files change with every bump, and the commit says so.
///
/// 1: first shape. 2: header gains `scope`; not-analysed kinds gain `out_of_scope`.
/// 3: not-analysed kinds gain `auto_run_config`. 4: not-analysed kinds gain `archive`.
/// 5: script triggers gain `on_open`, `on_git` and `on_build`. 6: the report gains `notice`.
/// 7: the brief gains `untraced` and the reasons `no_code_read` and `entry_point_missing`.
/// 8: `functions.untraced_by_callee[].sites` is removed (a copy of each function's `untraced_calls`) and
/// replaced by `functions` and `files` counts.
pub const REPORT_SCHEMA: u32 = 8;

/// Lets another thread stop a run. The run looks at it between files, so one very large file is
/// bounded by the size limits, not by this flag.
pub type CancelFlag = Arc<AtomicBool>;

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Size, count and depth limits for what is read.
    pub limits: Options,
    /// Stop reading new files after this long. Files not reached are listed as not analysed.
    pub time_budget: Option<Duration>,
    /// Stop reading new files once this is set to true.
    pub cancel: Option<CancelFlag>,
    /// `Full` analyses every file. `Quick` analyses only the files that run by themselves: every
    /// `package.json`, the files its install hooks run, tool configuration written as code (see
    /// `is_auto_run_code`), and what all of those load. Every other file is counted as not examined,
    /// and build and tool files we cannot read yet are listed as `AutoRunConfig`.
    pub scope: Scope,
}

impl RunOptions {
    fn stopped(&self, started: Instant) -> bool {
        if let Some(c) = &self.cancel {
            if c.load(Ordering::Relaxed) {
                return true;
            }
        }
        matches!(self.time_budget, Some(b) if started.elapsed() >= b)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    Intake(IntakeError),
    Report(BuildError),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Intake(IntakeError::Root(why)) => write!(f, "cannot open the folder: {why}"),
            RunError::Intake(IntakeError::NotADirectory) => write!(f, "that path is not a folder"),
            RunError::Report(e) => write!(f, "internal error while building the report: {e}"),
        }
    }
}

impl std::error::Error for RunError {}

fn skip_kind(r: &SkipReason) -> NotAnalysedKind {
    match r {
        SkipReason::Symlink { .. } => NotAnalysedKind::Symlink,
        SkipReason::IgnoredDir => NotAnalysedKind::IgnoredDir,
        SkipReason::TooDeep => NotAnalysedKind::TooDeep,
        SkipReason::TooLarge { .. } => NotAnalysedKind::TooLarge,
        SkipReason::TotalCapReached | SkipReason::FileCapReached => NotAnalysedKind::CapReached,
        SkipReason::NotRegular | SkipReason::OutsideRoot | SkipReason::BadPath | SkipReason::ReservedName => {
            NotAnalysedKind::Excluded
        }
        SkipReason::ReadError => NotAnalysedKind::Unreadable,
    }
}

/// Reads a folder and returns the report. Never writes anything and never runs the code.
pub fn analyze_folder(root: &Path, opts: &RunOptions) -> Result<Report, RunError> {
    let started = Instant::now();
    let snap = snapshot(root, &opts.limits).map_err(RunError::Intake)?;
    analyze_snapshot(&snap, opts, started)
}

/// Analyses files held in memory. Same limits, same report, no disk access.
pub fn analyze_files(files: Vec<MemFile>, opts: &RunOptions) -> Result<Report, RunError> {
    let started = Instant::now();
    let snap = snapshot_from_memory(files, &opts.limits);
    analyze_snapshot(&snap, opts, started)
}

/// Analyses a snapshot. `started` is when the run began, so the time budget counts intake too.
pub fn analyze_snapshot(snap: &Snapshot, opts: &RunOptions, started: Instant) -> Result<Report, RunError> {
    let catalog = Catalog::node_defaults();
    let all_files: BTreeSet<String> = snap.files().iter().map(|f| f.path.as_quoted().text().to_string()).collect();

    let mut analysed: Vec<(RelPath, Analysis)> = Vec::new();
    let mut package_facts: Vec<StaticFact> = Vec::new();
    let mut wants: Vec<EntryWant> = Vec::new();
    let mut not_analysed: Vec<NotAnalysedFile> = Vec::new();

    let index = FileIndex::new(&all_files);
    let quick = opts.scope == Scope::Quick;
    let by_name: std::collections::BTreeMap<&str, &forge_analysis_intake::SnapshotFile> =
        snap.files().iter().map(|f| (f.path.as_quoted().text(), f)).collect();
    // A full run visits every file in path order. A quick run starts from the `package.json` files
    // and adds what the install hooks run, and what those files load, as they are found.
    let mut work: VecDeque<&forge_analysis_intake::SnapshotFile> = if quick {
        snap.files()
            .iter()
            .filter(|f| {
                let name = f.path.as_quoted().text();
                name.rsplit('/').next() == Some("package.json") || is_auto_run_code(name) || is_config_manifest(name)
            })
            .collect()
    } else {
        snap.files().iter().collect()
    };
    let mut visited: BTreeSet<&str> = BTreeSet::new();

    while let Some(f) = work.pop_front() {
        let name = f.path.as_quoted().text();
        if !visited.insert(name) {
            continue;
        }
        // Checked before each file, so a file that has started is always finished.
        if opts.stopped(started) {
            not_analysed.push(NotAnalysedFile {
                path: f.path.as_quoted().clone(),
                size: Some(f.size),
                kind: NotAnalysedKind::Stopped,
            });
            continue;
        }
        let (dir, base) = match name.rfind('/') {
            Some(i) => (&name[..=i], &name[i + 1..]),
            None => ("", name),
        };
        let mut done = false;
        if let Some(text) = &f.text {
            if base == "package.json" {
                let gyp = all_files.contains(&format!("{dir}binding.gyp"));
                if let Some(p) = analyze_package(&f.path, text, gyp) {
                    package_facts.extend(p.facts);
                    for w in p.entries {
                        if quick && !w.install {
                            continue; // `main`, `bin` and the like do not run by themselves
                        }
                        if quick {
                            let spec = if w.spec.starts_with("./") || w.spec.starts_with("../") || w.spec.starts_with('/') {
                                w.spec.clone()
                            } else {
                                format!("./{}", w.spec)
                            };
                            if let Some(target) = index.resolve(name, &spec).and_then(|t| by_name.get(t)) {
                                work.push_back(target);
                            }
                        }
                        wants.push(w);
                    }
                    done = true;
                }
            } else if let Some(c) = analyze_config(&f.path, text) {
                // Editor, agent, git and build files whose commands run by themselves.
                package_facts.extend(c.facts);
                for w in c.entries {
                    if quick && !w.install {
                        continue; // someone has to start it
                    }
                    if quick {
                        let spec = if w.spec.starts_with("./") || w.spec.starts_with("../") || w.spec.starts_with('/') {
                            w.spec.clone()
                        } else {
                            format!("./{}", w.spec)
                        };
                        if let Some(target) = index.resolve(w.from.as_quoted().text(), &spec).and_then(|t| by_name.get(t)) {
                            work.push_back(target);
                        }
                    }
                    wants.push(w);
                }
                done = true;
            } else if let Some(a) = analyze_file_full(&f.path, text, &catalog) {
                if quick {
                    for edge in &a.link.imports {
                        if let Some(target) = index.resolve(name, &edge.spec).and_then(|t| by_name.get(t)) {
                            work.push_back(target);
                        }
                    }
                }
                analysed.push((f.path.clone(), a));
                done = true;
            }
        }
        if done {
            if f.invalid_utf8 {
                not_analysed.push(NotAnalysedFile {
                    path: f.path.as_quoted().clone(),
                    size: Some(f.size),
                    kind: NotAnalysedKind::InvalidUtf8,
                });
            }
        } else {
            not_analysed.push(NotAnalysedFile {
                path: f.path.as_quoted().clone(),
                size: Some(f.size),
                kind: classify_path(name),
            });
        }
    }
    if quick {
        // Everything the quick scan did not visit is still listed. Code a full run would read is
        // `OutOfScope`; other files keep the kind they would have had in a full run.
        for f in snap.files() {
            let name = f.path.as_quoted().text();
            if visited.contains(name) {
                continue;
            }
            // JSON is read as text but never analysed as code, in a full run either. A file a tool
            // acts on by itself keeps that label even when it happens to be code.
            let is_code = f.text.is_some() && !name.to_ascii_lowercase().ends_with(".json");
            let kind = if is_auto_run_config(name) {
                NotAnalysedKind::AutoRunConfig
            } else if is_code {
                NotAnalysedKind::OutOfScope
            } else {
                classify_path(name)
            };
            not_analysed.push(NotAnalysedFile { path: f.path.as_quoted().clone(), size: Some(f.size), kind });
        }
    }
    for s in snap.skipped() {
        let (text, size) = match &s.reason {
            SkipReason::Symlink { target } => (format!("{} -> {}", s.path.text(), target.text()), None),
            SkipReason::TooLarge { size } => (s.path.text().to_string(), Some(*size)),
            _ => (s.path.text().to_string(), None),
        };
        not_analysed.push(NotAnalysedFile { path: Quoted::new(&text), size, kind: skip_kind(&s.reason) });
    }

    let link_files: Vec<LinkFile<'_>> = analysed.iter().map(|(p, a)| LinkFile { path: p, info: &a.link }).collect();
    let linked = link_project(&link_files, &all_files, &wants);
    let inputs: Vec<FileInput<'_>> =
        analysed.iter().map(|(_, a)| FileInput { facts: &a.file.facts, summaries: &a.summaries }).collect();

    let header = Header {
        schema: REPORT_SCHEMA,
        engine: format!("forge-analysis {}", env!("CARGO_PKG_VERSION")),
        catalog_sha256: catalog.version_hash().clone(),
        snapshot_sha256: snap.sha256().clone(),
        mode: Mode::StaticOnly,
        scope: opts.scope,
    };
    build(&ReportInput {
        header,
        linked: &linked,
        files: &inputs,
        package_facts: &package_facts,
        not_analysed: &not_analysed,
        unrecorded_skips: snap.skipped_dropped(),
    })
    .map_err(RunError::Report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The config reader and the auto-run table must agree: a file that is read must also be one a
    /// tool acts on by itself, or it could drop off the list when its parser fails.
    #[test]
    fn every_file_the_config_reader_handles_is_on_the_auto_run_table() {
        for p in [
            ".git/config",
            "a/.git/config",
            ".vscode/tasks.json",
            ".claude/settings.json",
            ".claude/settings.local.json",
            ".mcp.json",
            ".cursor/mcp.json",
            ".vscode/mcp.json",
            ".devcontainer/devcontainer.json",
            ".devcontainer/x/devcontainer.json",
            ".devcontainer.json",
            "composer.json",
            "global.json",
            "Directory.Build.props",
            "Directory.Build.targets",
            "Directory.Packages.props",
            "src/App/App.csproj",
            "build/common.props",
            "build/tasks.targets",
            "nuget.config",
            ".cargo/config.toml",
            "a/.cargo/config",
            "Cargo.toml",
            "rust-toolchain",
            "rust-toolchain.toml",
            "pyproject.toml",
            "mise.toml",
            ".mise.local.toml",
            ".codex/config.toml",
            ".gemini/settings.json",
        ] {
            assert!(is_config_manifest(p), "{p}");
            assert!(is_auto_run_config(p), "{p} is read but not on the auto-run table");
        }
    }
}
