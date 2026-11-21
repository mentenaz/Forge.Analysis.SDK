//! The command line and the pipeline behind it: folder in, report out.
//! The analysed folder is only ever read. Reports go to a folder the caller names, and never
//! to one inside the analysed folder.

mod cli;
mod pipeline;

pub use cli::{
    parse_args, run, summary, usage, Command, ParseError, EXIT_CANNOT_RUN, EXIT_CANNOT_WRITE, EXIT_OK,
    EXIT_USAGE,
};
pub use pipeline::{analyze_folder, analyze_folder_scoped, RunError, REPORT_SCHEMA};
