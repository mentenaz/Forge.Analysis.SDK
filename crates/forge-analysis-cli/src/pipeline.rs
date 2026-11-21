//! Folder in, report out. The work lives in the `forge-analysis` library; this keeps the old
//! entry point so the command line and its tests do not change.

use forge_analysis::RunOptions;
use forge_analysis_intake::Options;
use forge_analysis_report::Report;
use std::path::Path;

pub use forge_analysis::{RunError, REPORT_SCHEMA};

/// Reads a folder and returns the report. Never writes anything and never runs the code.
pub fn analyze_folder(root: &Path, opts: &Options) -> Result<Report, RunError> {
    forge_analysis::analyze_folder(root, &RunOptions { limits: opts.clone(), ..RunOptions::default() })
}
