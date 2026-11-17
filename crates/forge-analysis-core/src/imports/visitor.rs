//! Whole-file walk: finds `require()` and `import()` anywhere, not just at the top level.
//! Scope questions ("is this really Node's require?") are answered by Oxc's semantic
//! analysis, not by our own guesswork.

use super::{Ctx, FileImports};
use forge_analysis_schema::{RelPath, UnresolvedReason};
use oxc_allocator::Allocator;
use oxc_ast::ast::{CallExpression, Expression, ImportExpression};
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::SourceType;

/// Returns None if the file name is not a JS/TS source type.
pub fn analyze_file(file: &RelPath, source: &str) -> Option<FileImports> {
    let source_type = SourceType::from_path(file.as_quoted().text()).ok()?;
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, source_type).parse();

    let mut ctx = Ctx::new(file, source);
    if !ret.diagnostics.is_empty() {
        ctx.parse_failed();
        return Some(ctx.finish());
    }

    let sem = SemanticBuilder::new().build(&ret.program);
    super::top_level(&mut ctx, &ret.program);

    let mut walker = Walker {
        ctx,
        scoping: sem.semantic.scoping(),
    };
    walker.visit_program(&ret.program);
    Some(walker.ctx.finish())
}

struct Walker<'s> {
    ctx: Ctx<'s>,
    scoping: &'s Scoping,
}

impl Walker<'_> {
    /// None: the callee is not called `require`. Some(true): it is Node's global `require`.
    /// Some(false): `require` is a local name (parameter, variable, function).
    fn require_scope(&self, call: &CallExpression<'_>) -> Option<bool> {
        let Expression::Identifier(callee) = &call.callee else {
            return None;
        };
        if self.ctx.text(callee.span) != "require" {
            return None;
        }
        // No reference id means we cannot tell; report it as the real thing rather than hide it.
        let global = callee.reference_id.get().map_or(true, |rid| {
            self.scoping.get_reference(rid).symbol_id().is_none()
        });
        Some(global)
    }
}

impl<'a> Visit<'a> for Walker<'_> {
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        match self.require_scope(it) {
            Some(true) => {
                if !self.ctx.handled.contains(&it.span.start) {
                    if let Some((outcome, sp)) = self.ctx.classify_call(it) {
                        self.ctx.emit_require(&outcome, sp);
                    }
                }
            }
            Some(false) => {
                let t = self.ctx.text(it.span);
                self.ctx
                    .unresolved(it.span, UnresolvedReason::AmbiguousBinding, t);
            }
            None => {}
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        let literal = match &it.source {
            Expression::StringLiteral(lit) => self.ctx.literal(lit),
            _ => None,
        };
        match literal {
            Some(m) => self.ctx.import_fact(it.span, &m),
            None => {
                let t = self.ctx.text(it.span);
                self.ctx
                    .unresolved(it.span, UnresolvedReason::DynamicRequire, t);
            }
        }
        walk::walk_import_expression(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_analysis_schema::FactKind;

    fn run(name: &str, src: &str) -> FileImports {
        analyze_file(&RelPath::new(name).unwrap(), src).unwrap()
    }

    fn count(f: &FileImports, kind: FactKind) -> usize {
        f.facts.iter().filter(|x| x.kind() == kind).count()
    }

    fn reasons(f: &FileImports) -> Vec<UnresolvedReason> {
        f.facts.iter().filter_map(|x| x.unresolved).collect()
    }

    #[test]
    fn require_inside_a_function_is_found() {
        let f = run(
            "a.js",
            r#"function f() { const fs = require("fs"); return fs; }"#,
        );
        assert_eq!(count(&f, FactKind::Import), 1);
        assert!(reasons(&f).is_empty());
    }

    #[test]
    fn require_as_an_argument_is_found() {
        let f = run("a.js", r#"app.use(require("./routes"));"#);
        assert_eq!(count(&f, FactKind::Import), 1);
    }

    #[test]
    fn require_deep_in_classes_and_arrows_is_found() {
        let f = run("a.js", r#"class A { m() { return () => require("os"); } }"#);
        assert_eq!(count(&f, FactKind::Import), 1);
    }

    #[test]
    fn top_level_requires_are_not_reported_twice() {
        let f = run("a.js", r#"const fs = require("fs"); const x = 1;"#);
        assert_eq!(count(&f, FactKind::Import), 1);
        assert!(f.table.lookup("fs").is_some());
    }

    #[test]
    fn a_local_require_is_reported_not_trusted() {
        let f = run("a.js", r#"function f(require) { return require("x"); }"#);
        assert_eq!(count(&f, FactKind::Import), 0);
        assert_eq!(reasons(&f), vec![UnresolvedReason::AmbiguousBinding]);
    }

    #[test]
    fn a_dynamic_nested_require_is_unresolved() {
        let f = run("a.js", "function f(n) { return require(n); }");
        assert_eq!(count(&f, FactKind::Import), 0);
        assert_eq!(reasons(&f), vec![UnresolvedReason::DynamicRequire]);
    }

    #[test]
    fn import_with_a_literal_is_an_import_fact() {
        let f = run(
            "a.js",
            r#"async function f() { return await import("./a.js"); }"#,
        );
        assert_eq!(count(&f, FactKind::Import), 1);
        assert!(reasons(&f).is_empty());
    }

    #[test]
    fn import_with_a_computed_source_is_unresolved() {
        let f = run("a.js", "async function f(n) { return import(n); }");
        assert_eq!(count(&f, FactKind::Import), 0);
        assert_eq!(reasons(&f), vec![UnresolvedReason::DynamicRequire]);
    }

    #[test]
    fn a_file_with_syntax_errors_gives_one_parse_failed_fact_only() {
        let f = run("a.js", "function (");
        assert_eq!(f.facts.len(), 1);
        assert_eq!(reasons(&f), vec![UnresolvedReason::ParseFailed]);
    }

    #[test]
    fn non_source_files_are_not_analysed() {
        assert!(analyze_file(&RelPath::new("notes.txt").unwrap(), "hello").is_none());
    }

    #[test]
    fn every_fact_is_valid_and_ids_are_distinct() {
        let f = run(
            "a.js",
            "function f() { require(\"a\"); require(\"b\"); }\nrequire(\"c\");",
        );
        assert_eq!(f.facts.len(), 3);
        let mut ids: Vec<_> = f.facts.iter().map(|x| x.id().clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3);
        assert!(f.facts.iter().all(|x| x.validate().is_empty()));
    }
}
