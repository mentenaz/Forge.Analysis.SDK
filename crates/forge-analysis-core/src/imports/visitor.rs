//! Whole-file analysis. Pass 1 collects every import and declaration, anywhere in the file,
//! so a name is known before its uses. Pass 2 walks calls: `require()` and `import()`,
//! then API calls matched against the catalog. Scope questions ("is this really the
//! imported `fs`?") are answered by Oxc's semantic analysis, never by name alone.
//!
//! Policy: a call we cannot match is recorded as unresolved with a reason, never dropped.
//! The one deliberate silence is a call on a module the catalog does not cover at all.

use super::{normalize_module, Binding, BindingKind, Ctx, FileImports, Require};
use forge_analysis_catalog::{Catalog, Entry};
use forge_analysis_schema::{FactKind, Quoted, RelPath, StaticFact, UnresolvedReason};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, CallExpression, Expression, IdentifierReference, ImportDeclaration, ImportExpression,
    StaticMemberExpression, VariableDeclarator,
};
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType, Span as OxcSpan};

/// Returns None if the file name is not a JS/TS source type.
pub fn analyze_file(file: &RelPath, source: &str, catalog: &Catalog) -> Option<FileImports> {
    let source_type = SourceType::from_path(file.as_quoted().text()).ok()?;
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, source_type).parse();

    if !ret.diagnostics.is_empty() {
        let mut ctx = Ctx::new(file, source);
        ctx.parse_failed();
        return Some(ctx.finish());
    }

    let sem = SemanticBuilder::new().build(&ret.program);
    let ctx = Ctx::new(file, source).with_scoping(sem.semantic.scoping());

    let mut collector = Collector { ctx };
    collector.visit_program(&ret.program);

    let mut matcher = Matcher { ctx: collector.ctx, catalog };
    matcher.visit_program(&ret.program);

    let mut out = matcher.ctx.finish();
    out.facts.sort_by_key(|f| (f.at().start_byte, f.at().end_byte));
    Some(out)
}

// ---- pass 1: bindings ----

struct Collector<'s> {
    ctx: Ctx<'s>,
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        self.ctx.import_declaration(it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        self.ctx.declarator(it);
        walk::walk_variable_declarator(self, it);
    }
}

// ---- pass 2: calls ----

struct Matcher<'s> {
    ctx: Ctx<'s>,
    catalog: &'s Catalog,
}

/// The identifier at the root of `a`, `a.b`, `a.b.c`. None for anything else.
fn root_ident<'x, 'a>(mut e: &'x Expression<'a>) -> Option<&'x IdentifierReference<'a>> {
    loop {
        match e {
            Expression::Identifier(id) => return Some(&**id),
            Expression::StaticMemberExpression(m) => e = &m.object,
            _ => return None,
        }
    }
}

impl<'s> Matcher<'s> {
    /// The import this identifier refers to, per Oxc's scope analysis.
    fn import_binding(&self, id: &IdentifierReference<'_>) -> Option<Binding> {
        let scoping = self.ctx.scoping?;
        let rid = id.reference_id.get()?;
        let sym = scoping.get_reference(rid).symbol_id()?;
        self.ctx.table.lookup_by_decl(scoping.symbol_span(sym).start).cloned()
    }

    /// None: not called `require`. Some(true): Node's global `require`.
    /// Some(false): `require` is a local name (parameter, variable, function).
    fn require_scope(&self, call: &CallExpression<'_>) -> Option<bool> {
        let Expression::Identifier(callee) = &call.callee else { return None };
        if self.ctx.text(callee.span) != "require" {
            return None;
        }
        // No scope information means we cannot tell; report it as the real thing, not hide it.
        let global = match (self.ctx.scoping, callee.reference_id.get()) {
            (Some(s), Some(rid)) => s.get_reference(rid).symbol_id().is_none(),
            _ => true,
        };
        Some(global)
    }

    fn flag(&mut self, call: &CallExpression<'_>, reason: UnresolvedReason) {
        let t = self.ctx.text(call.span);
        self.ctx.unresolved(call.span, reason, t);
    }

    /// `module.name(...)` through any route: namespace, alias, or `require("m").name`.
    fn resolve(&mut self, call: &CallExpression<'_>, module: &str, name: &str) {
        let cat = self.catalog;
        if let Some(entry) = cat.find_function(module, name) {
            self.emit_effect(call, entry);
        } else if cat.knows_module(module) {
            self.flag(call, UnresolvedReason::UncatalogedMember);
        }
    }

    /// A deeper path or a non-function member of a module we cover (`fs.promises.readFile`).
    fn chain(&mut self, call: &CallExpression<'_>, module: &str) {
        if self.catalog.knows_module(module) {
            self.flag(call, UnresolvedReason::UncatalogedMember);
        }
    }

    /// A method call on something we cannot trace, whose name matches a catalogued API.
    fn maybe_unknown(&mut self, call: &CallExpression<'_>, name: &str) {
        if self.catalog.exports_name(name) {
            self.flag(call, UnresolvedReason::UnknownReceiver);
        }
    }

    fn emit_effect(&mut self, call: &CallExpression<'_>, entry: &Entry) {
        let callee_text = self.ctx.text(call.callee.span());
        let call_fact = StaticFact::new(
            FactKind::Call,
            self.ctx.span(call.span),
            Some(Quoted::new(callee_text)),
        );
        self.ctx.facts.push(call_fact);

        // The argument that carries the path, command or URL, if the catalog names one.
        let role = entry.args.path.or(entry.args.command).or(entry.args.url);
        let mut detail: Option<String> = None;
        let mut problem: Option<(OxcSpan, UnresolvedReason)> = None;
        if let Some(i) = role {
            match call.arguments.get(usize::from(i)) {
                None => {}
                Some(Argument::StringLiteral(lit)) => match self.ctx.literal(lit) {
                    Some(v) => detail = Some(v),
                    None => problem = Some((lit.span, UnresolvedReason::UnsupportedSyntax)),
                },
                Some(other) => problem = Some((other.span(), UnresolvedReason::ComputedAccess)),
            }
        }

        let mut effect = StaticFact::new(
            FactKind::Effect,
            self.ctx.span(call.span),
            detail.as_deref().map(Quoted::new),
        );
        effect.effect = Some(entry.category);
        self.ctx.facts.push(effect);

        if let Some((sp, reason)) = problem {
            let t = self.ctx.text(sp);
            self.ctx.unresolved(sp, reason, t);
        }
    }

    fn member_call(&mut self, call: &CallExpression<'_>, m: &StaticMemberExpression<'_>) {
        let name = self.ctx.text(m.property.span);

        // require("m").name(...)
        if let Expression::CallExpression(inner) = &m.object {
            if let Some((outcome, _)) = self.ctx.classify_call(inner) {
                // A dynamic require was already reported when the inner call was visited.
                if let Require::Literal(lit) = outcome {
                    let module = normalize_module(&lit);
                    self.resolve(call, &module, name);
                }
                return;
            }
        }

        match &m.object {
            Expression::Identifier(id) => match self.import_binding(id) {
                Some(b) => match &b.kind {
                    BindingKind::Namespace | BindingKind::Default => {
                        self.resolve(call, &b.module, name)
                    }
                    BindingKind::Named { .. } => self.chain(call, &b.module),
                },
                None => self.maybe_unknown(call, name),
            },
            other => match root_ident(other).and_then(|id| self.import_binding(id)) {
                Some(b) => self.chain(call, &b.module),
                None => self.maybe_unknown(call, name),
            },
        }
    }

    fn match_call(&mut self, call: &CallExpression<'_>) {
        match &call.callee {
            // rf(...) where rf was imported by name
            Expression::Identifier(id) => {
                if let Some(b) = self.import_binding(id) {
                    if let BindingKind::Named { imported } = &b.kind {
                        self.resolve(call, &b.module, imported);
                    }
                }
            }
            Expression::StaticMemberExpression(m) => self.member_call(call, m),
            // fs[name](...): the member is chosen at runtime
            Expression::ComputedMemberExpression(m) => {
                if let Some(b) = root_ident(&m.object).and_then(|id| self.import_binding(id)) {
                    if self.catalog.knows_module(&b.module) {
                        self.flag(call, UnresolvedReason::ComputedAccess);
                    }
                }
            }
            _ => {}
        }
    }
}

impl<'a> Visit<'a> for Matcher<'_> {
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        match self.require_scope(it) {
            Some(true) => {
                if !self.ctx.handled.contains(&it.span.start) {
                    if let Some((outcome, sp)) = self.ctx.classify_call(it) {
                        self.ctx.emit_require(&outcome, sp);
                    }
                }
            }
            Some(false) => self.flag(it, UnresolvedReason::AmbiguousBinding),
            None => {}
        }
        self.match_call(it);
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
                self.ctx.unresolved(it.span, UnresolvedReason::DynamicRequire, t);
            }
        }
        walk::walk_import_expression(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_analysis_schema::Effect;

    fn run(name: &str, src: &str) -> FileImports {
        analyze_file(&RelPath::new(name).unwrap(), src, &Catalog::node_defaults()).unwrap()
    }

    fn count(f: &FileImports, kind: FactKind) -> usize {
        f.facts.iter().filter(|x| x.kind() == kind).count()
    }

    fn reasons(f: &FileImports) -> Vec<UnresolvedReason> {
        f.facts.iter().filter_map(|x| x.unresolved).collect()
    }

    fn effect(f: &FileImports) -> Option<(Effect, Option<String>)> {
        f.facts.iter().find(|x| x.kind() == FactKind::Effect).map(|x| {
            (x.effect.unwrap(), x.detail().map(|d| d.text().to_string()))
        })
    }

    // ---- require() and import() anywhere ----

    #[test]
    fn require_inside_a_function_is_found() {
        let f = run("a.js", r#"function f() { const fs = require("fs"); return fs; }"#);
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
        let f = run("a.js", r#"async function f() { return await import("./a.js"); }"#);
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
        let c = Catalog::node_defaults();
        assert!(analyze_file(&RelPath::new("notes.txt").unwrap(), "hello", &c).is_none());
    }

    #[test]
    fn every_fact_is_valid_and_ids_are_distinct() {
        let f = run("a.js", "function f() { require(\"a\"); require(\"b\"); }\nrequire(\"c\");");
        assert_eq!(f.facts.len(), 3);
        let mut ids: Vec<_> = f.facts.iter().map(|x| x.id().clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3);
        assert!(f.facts.iter().all(|x| x.validate().is_empty()));
    }

    // ---- catalog matching ----

    #[test]
    fn a_namespace_call_matches_the_catalog() {
        let f = run("a.js", r#"const fs = require("fs"); fs.readFile("/etc/passwd");"#);
        assert_eq!(effect(&f), Some((Effect::FsRead, Some("/etc/passwd".into()))));
        assert_eq!(count(&f, FactKind::Call), 1);
        assert!(f.facts.iter().all(|x| x.validate().is_empty()));
    }

    #[test]
    fn a_computed_path_is_reported_beside_the_effect() {
        let f = run("a.js", r#"import fs from "fs"; fs.writeFileSync(p, d);"#);
        assert_eq!(effect(&f), Some((Effect::FsWrite, None)));
        assert_eq!(reasons(&f), vec![UnresolvedReason::ComputedAccess]);
    }

    #[test]
    fn a_destructured_alias_matches() {
        let f = run("a.js", r#"const { readFile: rf } = require("fs"); rf("a");"#);
        assert_eq!(effect(&f), Some((Effect::FsRead, Some("a".into()))));
    }

    #[test]
    fn a_named_es_import_matches() {
        let f = run("a.js", r#"import { exec } from "child_process"; exec("ls -la");"#);
        assert_eq!(effect(&f), Some((Effect::ProcessSpawn, Some("ls -la".into()))));
    }

    #[test]
    fn require_then_member_call_matches() {
        let f = run("a.js", r#"require("fs").unlinkSync("x");"#);
        assert_eq!(effect(&f), Some((Effect::FsDelete, Some("x".into()))));
        assert_eq!(count(&f, FactKind::Import), 1);
    }

    #[test]
    fn a_shadowed_name_is_not_the_import() {
        let f = run("a.js", r#"const fs = require("fs"); function f(fs) { fs.readFile("x"); }"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::UnknownReceiver]);
    }

    #[test]
    fn an_untraceable_receiver_with_a_catalogued_name_is_unresolved() {
        let f = run("a.js", r#"function f(x) { x.readFile("a"); }"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::UnknownReceiver]);
    }

    #[test]
    fn an_uncatalogued_member_of_a_covered_module_is_reported() {
        let f = run("a.js", r#"const fs = require("fs"); fs.existsSync("a");"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::UncatalogedMember]);
    }

    #[test]
    fn a_module_the_catalog_does_not_cover_is_silent() {
        let f = run("a.js", r#"const path = require("path"); path.join("a", "b");"#);
        assert!(effect(&f).is_none());
        assert!(reasons(&f).is_empty());
    }

    #[test]
    fn a_binding_made_inside_a_function_is_used_there() {
        let f = run("a.js", r#"function f() { const fs = require("fs"); fs.readFile("x"); }"#);
        assert_eq!(effect(&f), Some((Effect::FsRead, Some("x".into()))));
        assert_eq!(count(&f, FactKind::Import), 1);
    }

    #[test]
    fn a_function_may_use_a_binding_declared_after_it() {
        let f = run("a.js", r#"function f() { fs.readFile("x"); } const fs = require("fs");"#);
        assert_eq!(effect(&f), Some((Effect::FsRead, Some("x".into()))));
    }

    #[test]
    fn a_computed_member_call_on_a_covered_module_is_reported() {
        let f = run("a.js", r#"const fs = require("fs"); fs[name]("x");"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::ComputedAccess]);
    }

    #[test]
    fn an_escaped_path_literal_is_not_interpreted() {
        let f = run("a.js", r#"const fs = require("fs"); fs.readFile("a\u0062");"#);
        assert_eq!(effect(&f), Some((Effect::FsRead, None)));
        assert_eq!(reasons(&f), vec![UnresolvedReason::UnsupportedSyntax]);
    }

    #[test]
    fn a_deeper_path_in_a_covered_module_is_reported() {
        let f = run("a.js", r#"const fs = require("fs"); fs.promises.readFile("x");"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::UncatalogedMember]);
    }

    #[test]
    fn a_member_call_on_a_dynamic_require_reports_only_the_require() {
        let f = run("a.js", r#"require(name).readFile("x");"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::DynamicRequire]);
    }
}
