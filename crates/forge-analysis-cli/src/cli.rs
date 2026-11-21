//! Argument handling, the summary and the two output files.

use crate::pipeline::{analyze_folder_scoped, RunError};
use forge_analysis_intake::Options;
use forge_analysis_report::{to_json, to_markdown, Completeness, Report};
use forge_analysis_schema::Scope;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The run finished (whatever the report's status says: incomplete is the normal result).
pub const EXIT_OK: i32 = 0;
/// The command line was wrong, or the output folder is not allowed.
pub const EXIT_USAGE: i32 = 2;
/// The folder could not be analysed at all.
pub const EXIT_CANNOT_RUN: i32 = 3;
/// The analysis ran but the report could not be written.
pub const EXIT_CANNOT_WRITE: i32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Run { folder: PathBuf, out: Option<PathBuf>, quick: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    NoFolder,
    TwoFolders,
    MissingValue(String),
    UnknownFlag(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::NoFolder => write!(f, "no folder given"),
            ParseError::TwoFolders => write!(f, "only one folder can be analysed at a time"),
            ParseError::MissingValue(flag) => write!(f, "{flag} needs a value"),
            ParseError::UnknownFlag(flag) => write!(f, "unknown option {flag}"),
        }
    }
}

pub fn usage() -> String {
    "Usage: forge-analysis <folder> [--quick] [--out <dir>]\n\
     \n\
     Reads <folder> and reports what its code does, without running any of it.\n\
     \n\
     Options:\n\
     \x20 -o, --out <dir>   write report.json and report.md into <dir>\n\
     \x20                   (never inside the analysed folder)\n\
     \x20 -q, --quick      read only the files that run by themselves (install hooks and what\n\
     \x20                   they load). Every other file is counted as not examined, so a\n\
     \x20                   quick scan is always reported as incomplete.\n\
     \x20 -h, --help        show this help\n\
     \x20 -V, --version     show the version\n\
     \n\
     Without --out only a summary is printed.\n\
     Exit codes: 0 done, 2 wrong command line, 3 could not analyse, 4 could not write the report.\n"
        .to_string()
}

pub fn parse_args(args: &[String]) -> Result<Command, ParseError> {
    let mut folder: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut quick = false;
    let mut only_positional = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if only_positional || !a.starts_with('-') || a == "-" {
            if folder.replace(PathBuf::from(a)).is_some() {
                return Err(ParseError::TwoFolders);
            }
            continue;
        }
        match a.as_str() {
            "--" => only_positional = true,
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "-q" | "--quick" => quick = true,
            "-o" | "--out" => match args.get(i) {
                Some(v) => {
                    out = Some(PathBuf::from(v));
                    i += 1;
                }
                None => return Err(ParseError::MissingValue(a.clone())),
            },
            other => match other.strip_prefix("--out=") {
                Some("") => return Err(ParseError::MissingValue("--out".to_string())),
                Some(v) => out = Some(PathBuf::from(v)),
                None => return Err(ParseError::UnknownFlag(other.to_string())),
            },
        }
    }
    match folder {
        Some(folder) => Ok(Command::Run { folder, out, quick }),
        None => Err(ParseError::NoFolder),
    }
}

/// The nearest folder that exists, made absolute, with the missing tail put back.
fn resolve(path: &Path) -> Option<PathBuf> {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(c) = std::fs::canonicalize(&cur) {
            let mut full = c;
            for part in tail.iter().rev() {
                full.push(part);
            }
            return Some(full);
        }
        let name = cur.file_name()?.to_os_string();
        tail.push(name);
        cur = match cur.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => std::env::current_dir().ok()?,
        };
    }
}

fn out_is_inside(folder: &Path, out: &Path) -> bool {
    match (std::fs::canonicalize(folder), resolve(out)) {
        (Ok(f), Some(o)) => o.starts_with(&f),
        _ => false,
    }
}

pub fn summary(r: &Report) -> String {
    let o = &r.overview;
    let mut s = String::new();
    s.push_str("Forge analysis (static only: no code was run)\n");
    if r.header.scope == Scope::Quick {
        s.push_str("Scope: quick (only files that run by themselves)\n");
    }
    s.push_str(&format!(
        "Files analysed: {}   not analysed: {}   functions: {}   facts: {}\n",
        o.files_analysed, o.files_not_analysed, o.functions, o.facts
    ));
    match o.completeness {
        Completeness::Complete => s.push_str("Status: complete. Nothing was left unseen.\n"),
        Completeness::Incomplete => {
            s.push_str("Status: incomplete. What we could not see:\n");
            for c in &o.coverage {
                s.push_str(&format!("  {:>6}  {}\n", c.count, c.label));
            }
        }
    }
    s
}

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match parse_args(args) {
        Err(e) => {
            let _ = writeln!(err, "error: {e}\n\n{}", usage());
            EXIT_USAGE
        }
        Ok(Command::Help) => {
            let _ = write!(out, "{}", usage());
            EXIT_OK
        }
        Ok(Command::Version) => {
            let _ = writeln!(out, "forge-analysis {}", env!("CARGO_PKG_VERSION"));
            EXIT_OK
        }
        Ok(Command::Run { folder, out: out_dir, quick }) => {
            if let Some(o) = &out_dir {
                if out_is_inside(&folder, o) {
                    let _ = writeln!(err, "error: the output folder is inside the analysed folder; choose one outside it");
                    return EXIT_USAGE;
                }
            }
            let report = match analyze_folder_scoped(&folder, &Options::default(), if quick { Scope::Quick } else { Scope::Full }) {
                Ok(r) => r,
                Err(e @ RunError::Intake(_)) | Err(e @ RunError::Report(_)) => {
                    let _ = writeln!(err, "error: {e}");
                    return EXIT_CANNOT_RUN;
                }
            };
            let _ = write!(out, "{}", summary(&report));
            if let Some(dir) = out_dir {
                let write_all = || -> std::io::Result<()> {
                    std::fs::create_dir_all(&dir)?;
                    std::fs::write(dir.join("report.json"), to_json(&report) + "\n")?;
                    std::fs::write(dir.join("report.md"), to_markdown(&report))?;
                    Ok(())
                };
                if let Err(e) = write_all() {
                    let _ = writeln!(err, "error: could not write the report: {e}");
                    return EXIT_CANNOT_WRITE;
                }
                let _ = writeln!(out, "Report written to {}", dir.display());
            }
            EXIT_OK
        }
    }
}
