//! The Forge analysis engine as a library: files in, report or brief out.
//!
//! Nothing here runs the analysed code, and nothing is written. Input is either a folder (read
//! only) or files held in memory, so an editor can hand over what it has open without touching
//! the disk. The core crate stays pure; reading folders and the clock live here.

mod brief;
mod run;

pub use brief::{brief, Brief, Reason, ReasonKind, Scope, Verdict, MAX_REASONS};
pub use run::{
    analyze_files, analyze_folder, analyze_snapshot, CancelFlag, RunError, RunOptions, REPORT_SCHEMA,
};

pub use forge_analysis_intake::{MemFile, Options as Limits};
pub use forge_analysis_report::{to_json, to_markdown, to_markdown_parts, Completeness, MarkdownParts, Report, FUNCTIONS_FILE};
