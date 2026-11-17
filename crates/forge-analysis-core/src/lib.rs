//! Core analyzer: source text in, facts out. No file, process or network access.

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseSummary {
    pub statements: usize,
    pub had_errors: bool,
}

/// Parses one file. Returns None if the file name isn't a JS/TS source type.
/// Spike: proves the pinned Oxc API compiles; replaced by the real frontend next.
pub fn parse_summary(source: &str, file_name: &str) -> Option<ParseSummary> {
    let source_type = SourceType::from_path(file_name).ok()?;
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, source_type).parse();
    Some(ParseSummary {
        statements: ret.program.body.len(),
        had_errors: !ret.diagnostics.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typescript() {
        let s = parse_summary("import fs from 'fs'; fs.readFile('a');", "a.ts").unwrap();
        assert_eq!(
            s,
            ParseSummary {
                statements: 2,
                had_errors: false
            }
        );
    }

    #[test]
    fn syntax_errors_are_reported_not_fatal() {
        let s = parse_summary("const = ;", "a.js").unwrap();
        assert!(s.had_errors);
    }

    #[test]
    fn non_source_extensions_are_not_parsed() {
        assert!(parse_summary("hello", "notes.txt").is_none());
    }
}
