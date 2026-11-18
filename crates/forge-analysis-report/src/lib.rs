//! Builds the analysis report: a model of every section, then JSON and Markdown writers.
mod build;
mod markdown;
mod model;

pub use build::{build, classify_path, to_json, BuildError, FileInput, ReportInput};
pub use markdown::{to_markdown, COLLAPSE_AT};
pub use model::*;

#[cfg(test)]
mod tests;
