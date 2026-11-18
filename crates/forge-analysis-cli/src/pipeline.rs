//! Folder in, report out: intake, then every file, then the project link, then the report.

use forge_analysis_catalog::Catalog;
use forge_analysis_core::imports::{analyze_file_full, analyze_package, link_project, Analysis, EntryWant, LinkFile};
use forge_analysis_intake::{snapshot, IntakeError, Options, SkipReason};
use forge_analysis_report::{
    build, classify_path, BuildError, FileInput, NotAnalysedFile, NotAnalysedKind, Report, ReportInput,
};
use forge_analysis_schema::{Header, Mode, Quoted, RelPath, StaticFact};
use std::collections::BTreeSet;
use std::path::Path;

/// Version of the report's shape. Bump it when the JSON changes in a way readers must notice.
pub const REPORT_SCHEMA: u32 = 1;

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
pub fn analyze_folder(root: &Path, opts: &Options) -> Result<Report, RunError> {
    let snap = snapshot(root, opts).map_err(RunError::Intake)?;
    let catalog = Catalog::node_defaults();
    let all_files: BTreeSet<String> = snap.files().iter().map(|f| f.path.as_quoted().text().to_string()).collect();

    let mut analysed: Vec<(RelPath, Analysis)> = Vec::new();
    let mut package_facts: Vec<StaticFact> = Vec::new();
    let mut wants: Vec<EntryWant> = Vec::new();
    let mut not_analysed: Vec<NotAnalysedFile> = Vec::new();

    for f in snap.files() {
        let name = f.path.as_quoted().text();
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
                    wants.extend(p.entries);
                    done = true;
                }
            } else if let Some(a) = analyze_file_full(&f.path, text, &catalog) {
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
