//! Builds the analysis report: a model of every section, then JSON and Markdown writers.
mod autorun;
mod build;
mod markdown;
mod model;

pub use autorun::{is_auto_run_code, is_auto_run_config};
pub use build::{build, classify_path, to_json, BuildError, FileInput, ReportInput};
pub use markdown::{to_markdown, COLLAPSE_AT};
pub use model::*;

#[cfg(test)]
mod tests;
