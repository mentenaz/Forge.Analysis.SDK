//! What a file exports and which modules it loads, read from the syntax tree.
//!
//! Pure Oxc. Function identity comes from the call graph (by symbol or span), never by name.
//! Names are read from the source text by span.
//!
//! Anything we cannot enumerate sets `Exports::dynamic` (spread, computed keys, `module` or
//! `exports` used as a plain value), so a name we fail to find is reported as "unknown", not
//! as "absent".

use crate::imports::callgraph::Graph;
use crate::imports::link::{ExportName, ExportTarget, Exports, ExtCall, ImportEdge, ImportKind};
use oxc_ast::ast::{
    Argument, AssignmentExpression, AssignmentOperator, AssignmentTarget, BindingPattern,
    CallExpression, Declaration, ExportAllDeclaration, ExportDeclaration,
    ExportDefaultDeclaration, ExportDefaultDeclarationKind, ExportFromDeclaration,
    ExportNamedDeclaration, Expression, IdentifierReference, ImportDeclaration,
    ImportExpression, ModuleExportName, ObjectPropertyKind, PropertyKey, PropertyKind,
    Program, StringLiteral,
};
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Scoping;
use oxc_span::Span;
use std::collections::BTreeSet;

#[derive(Debug, Default)]
pub struct Extracted {
    pub exports: Exports,
    pub imports: Vec<ImportEdge>,
    /// Calls through `require("m").f()` and `require("m")()`. Calls through names bound to an
    /// import are found by the call graph and mapped by the caller.
    pub ext_calls: Vec<ExtCall>,
}

pub fn extract(program: &Program<'_>, scoping: &Scoping, graph: &Graph, src: &str) -> Extracted {
    let mut x = X { src, scoping, graph, out: Extracted::default(), consumed: BTreeSet::new() };
    x.visit_program(program);
    x.out
}

struct X<'s> {
    src: &'s str,
    scoping: &'s Scoping,
    graph: &'s Graph,
    out: Extracted,
    consumed: BTreeSet<(u32, u32)>,
}

fn peel<'e, 'a>(mut e: &'e Expression<'a>) -> &'e Expression<'a> {
    while let Expression::ParenthesizedExpression(p) = e {
        e = &p.expression;
    }
    e
}

impl X<'_> {
    fn text(&self, s: Span) -> &str {
        &self.src[s.start as usize..s.end as usize]
    }

    /// The text of a string literal, unless it has escapes (those are reported by the
    /// per-file pass as unresolved, not interpreted here).
    fn lit(&self, l: &StringLiteral<'_>) -> Option<String> {
        let raw = self.text(l.span);
        let q = raw.chars().next()?;
        if raw.len() < 2 || !(q == '"' || q == '\'') || !raw.ends_with(q) || raw.contains('\\') {
            return None;
        }
        Some(raw[1..raw.len() - 1].to_string())
    }

    fn is_global(&self, r: &IdentifierReference<'_>, name: &str) -> bool {
        if self.text(r.span) != name {
            return false;
        }
        match r.reference_id.get() {
            Some(rid) => self.scoping.get_reference(rid).symbol_id().is_none(),
            None => true,
        }
    }

    fn fn_of(&self, e: &Expression<'_>) -> Option<u32> {
        match peel(e) {
            Expression::Identifier(r) => {
                let sym = self.scoping.get_reference(r.reference_id.get()?).symbol_id()?;
                self.graph.sym_fn.get(&sym).copied()
            }
            Expression::ArrowFunctionExpression(f) => self.graph.id_of(f.span),
            Expression::FunctionExpression(f) => self.graph.id_of(f.span),
            _ => None,
        }
    }

    fn name_of(&self, n: &str) -> ExportName {
        if n == "default" {
            ExportName::Default
        } else {
            ExportName::Named(n.to_string())
        }
    }

    fn add(&mut self, name: ExportName, target: ExportTarget) {
        match self.out.exports.names.get(&name) {
            Some(old) if *old != target => {
                self.out.exports.names.insert(name, ExportTarget::Opaque);
            }
            Some(_) => {}
            None => {
                self.out.exports.names.insert(name, target);
            }
        }
    }

    fn module_export_name(&self, n: &ModuleExportName<'_>) -> Option<String> {
        match n {
            ModuleExportName::IdentifierName(i) => Some(self.text(i.span).to_string()),
            ModuleExportName::IdentifierReference(i) => Some(self.text(i.span).to_string()),
            ModuleExportName::StringLiteral(l) => self.lit(l),
        }
    }

    fn import(&mut self, from: u32, l: &StringLiteral<'_>, kind: ImportKind) {
        if let Some(spec) = self.lit(l) {
            self.out.imports.push(ImportEdge { from, spec, kind });
        }
    }

    /// `require("literal")` -> the literal, when this call is a global `require`.
    fn require_spec(&self, c: &CallExpression<'_>) -> Option<String> {
        match peel(&c.callee) {
            Expression::Identifier(r) if self.is_global(r, "require") => {}
            _ => return None,
        }
        match c.arguments.first() {
            Some(Argument::StringLiteral(l)) => self.lit(l),
            _ => None,
        }
    }

    /// `module.exports` as an expression: the span of its `module` identifier.
    fn module_exports_obj(&self, e: &Expression<'_>) -> Option<Span> {
        if let Expression::StaticMemberExpression(m) = peel(e) {
            if self.text(m.property.span) == "exports" {
                if let Expression::Identifier(r) = peel(&m.object) {
                    if self.is_global(r, "module") {
                        return Some(r.span);
                    }
                }
            }
        }
        None
    }

    fn object_exports(&mut self, o: &oxc_ast::ast::ObjectExpression<'_>) {
        for p in &o.properties {
            match p {
                ObjectPropertyKind::SpreadProperty(_) => self.out.exports.dynamic = true,
                ObjectPropertyKind::ObjectProperty(p) => {
                    let name = match &p.key {
                        PropertyKey::StaticIdentifier(i) if !p.computed => {
                            Some(self.text(i.span).to_string())
                        }
                        PropertyKey::StringLiteral(l) if !p.computed => self.lit(l),
                        _ => None,
                    };
                    let Some(name) = name else {
                        self.out.exports.dynamic = true;
                        continue;
                    };
                    let target = match (p.kind == PropertyKind::Init, self.fn_of(&p.value)) {
                        (true, Some(f)) => ExportTarget::Func(f),
                        _ => ExportTarget::Opaque,
                    };
                    self.add(self.name_of(&name), target);
                }
            }
        }
    }
}

impl<'a> Visit<'a> for X<'_> {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        if !it.import_kind.is_type() {
            self.import(0, &it.source, ImportKind::Static);
        }
        walk::walk_import_declaration(self, it);
    }

    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        if !it.export_kind.is_type() {
            self.import(0, &it.source, ImportKind::Static);
            if let Some(spec) = self.lit(&it.source) {
                for s in &it.specifiers {
                    let (Some(local), Some(exported)) =
                        (self.module_export_name(&s.local), self.module_export_name(&s.exported))
                    else {
                        self.out.exports.dynamic = true;
                        continue;
                    };
                    let name = self.name_of(&local);
                    self.add(self.name_of(&exported), ExportTarget::Reexport { spec: spec.clone(), name });
                }
            }
        }
        walk::walk_export_from_declaration(self, it);
    }

    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        if !it.export_kind.is_type() {
            self.import(0, &it.source, ImportKind::Static);
            match &it.exported {
                None => {
                    if let Some(spec) = self.lit(&it.source) {
                        self.out.exports.stars.push(spec);
                    }
                }
                Some(n) => match self.module_export_name(n) {
                    Some(n) => self.add(self.name_of(&n), ExportTarget::Opaque),
                    None => self.out.exports.dynamic = true,
                },
            }
        }
        walk::walk_export_all_declaration(self, it);
    }

    fn visit_export_declaration(&mut self, it: &ExportDeclaration<'a>) {
        match &it.declaration {
            Declaration::FunctionDeclaration(f) => {
                if let (Some(id), Some(func)) = (&f.id, self.graph.id_of(f.span)) {
                    let n = self.text(id.span).to_string();
                    self.add(ExportName::Named(n), ExportTarget::Func(func));
                }
            }
            Declaration::ClassDeclaration(c) => {
                if let Some(id) = &c.id {
                    let n = self.text(id.span).to_string();
                    self.add(ExportName::Named(n), ExportTarget::Opaque);
                }
            }
            Declaration::VariableDeclaration(v) => {
                for d in &v.declarations {
                    match &d.id {
                        BindingPattern::BindingIdentifier(b) => {
                            let n = self.text(b.span).to_string();
                            let target = match d.init.as_ref().and_then(|e| self.fn_of(e)) {
                                Some(f) => ExportTarget::Func(f),
                                None => ExportTarget::Opaque,
                            };
                            self.add(ExportName::Named(n), target);
                        }
                        _ => self.out.exports.dynamic = true,
                    }
                }
            }
            _ => {}
        }
        walk::walk_export_declaration(self, it);
    }

    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        if !it.export_kind.is_type() {
            for s in &it.specifiers {
                let Some(exported) = self.module_export_name(&s.exported) else {
                    self.out.exports.dynamic = true;
                    continue;
                };
                let target = match &s.local {
                    ModuleExportName::IdentifierReference(r) => {
                        let f = r
                            .reference_id
                            .get()
                            .and_then(|rid| self.scoping.get_reference(rid).symbol_id())
                            .and_then(|sym| self.graph.sym_fn.get(&sym).copied());
                        f.map_or(ExportTarget::Opaque, ExportTarget::Func)
                    }
                    _ => ExportTarget::Opaque,
                };
                self.add(self.name_of(&exported), target);
            }
        }
        walk::walk_export_named_declaration(self, it);
    }

    fn visit_export_default_declaration(&mut self, it: &ExportDefaultDeclaration<'a>) {
        let target = match &it.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(f) => self
                .graph
                .id_of(f.span)
                .map_or(ExportTarget::Opaque, ExportTarget::Func),
            ExportDefaultDeclarationKind::ClassDeclaration(_) => ExportTarget::Opaque,
            other => other
                .as_expression()
                .and_then(|e| self.fn_of(e))
                .map_or(ExportTarget::Opaque, ExportTarget::Func),
        };
        self.add(ExportName::Default, target);
        walk::walk_export_default_declaration(self, it);
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        if let Expression::StringLiteral(l) = peel(&it.source) {
            let from = self.graph.owner(it.span);
            self.import(from, l, ImportKind::Dynamic);
        }
        walk::walk_import_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let from = self.graph.owner(it.span);
        if let Some(spec) = self.require_spec(it) {
            self.out.imports.push(ImportEdge { from, spec, kind: ImportKind::Require });
        }
        match peel(&it.callee) {
            // require("m")()
            Expression::CallExpression(inner) => {
                if let Some(spec) = self.require_spec(inner) {
                    self.out.ext_calls.push(ExtCall { from, spec, name: ExportName::Default });
                }
            }
            // require("m").f()
            Expression::StaticMemberExpression(m) => {
                if let Expression::CallExpression(inner) = peel(&m.object) {
                    if let Some(spec) = self.require_spec(inner) {
                        let n = self.text(m.property.span).to_string();
                        self.out.ext_calls.push(ExtCall { from, spec, name: self.name_of(&n) });
                    }
                }
            }
            _ => {}
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        if let AssignmentTarget::StaticMemberExpression(m) = &it.left {
            let plain = it.operator == AssignmentOperator::Assign;
            let prop = self.text(m.property.span).to_string();
            // module.exports = X
            if let (Expression::Identifier(r), "exports") = (peel(&m.object), prop.as_str()) {
                if self.is_global(r, "module") {
                    self.consumed.insert((r.span.start, r.span.end));
                    if !plain {
                        self.out.exports.dynamic = true;
                    } else if let Expression::ObjectExpression(o) = peel(&it.right) {
                        self.object_exports(o);
                        self.add(ExportName::Default, ExportTarget::Opaque);
                    } else {
                        let t = self.fn_of(&it.right).map_or(ExportTarget::Opaque, ExportTarget::Func);
                        self.add(ExportName::Default, t);
                    }
                }
            }
            // module.exports.n = X
            if let Some(module_span) = self.module_exports_obj(&m.object) {
                self.consumed.insert((module_span.start, module_span.end));
                if plain {
                    let t = self.fn_of(&it.right).map_or(ExportTarget::Opaque, ExportTarget::Func);
                    self.add(self.name_of(&prop), t);
                } else {
                    self.out.exports.dynamic = true;
                }
            }
            // exports.n = X
            if let Expression::Identifier(r) = peel(&m.object) {
                if self.is_global(r, "exports") {
                    self.consumed.insert((r.span.start, r.span.end));
                    if plain {
                        let t = self.fn_of(&it.right).map_or(ExportTarget::Opaque, ExportTarget::Func);
                        self.add(self.name_of(&prop), t);
                    } else {
                        self.out.exports.dynamic = true;
                    }
                }
            }
        }
        walk::walk_assignment_expression(self, it);
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        let t = self.text(it.span);
        if (t == "module" || t == "exports")
            && !self.consumed.contains(&(it.span.start, it.span.end))
            && self.is_global(it, t)
        {
            self.out.exports.dynamic = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imports::callgraph;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;

    fn run(src: &str) -> (Extracted, Graph) {
        let alloc = Allocator::default();
        let ret = Parser::new(&alloc, src, SourceType::from_path("a.ts").unwrap()).parse();
        assert!(ret.diagnostics.is_empty(), "{:?}", ret.diagnostics);
        let sem = SemanticBuilder::new().build(&ret.program);
        let g = callgraph::build(&ret.program, sem.semantic.scoping());
        let e = extract(&ret.program, sem.semantic.scoping(), &g, src);
        (e, g)
    }

    fn named(n: &str) -> ExportName {
        ExportName::Named(n.to_string())
    }

    fn fn_name(src: &str, g: &Graph, id: u32) -> String {
        g.fns[id as usize].name.map_or("<anon>".into(), |s| src[s.start as usize..s.end as usize].to_string())
    }

    #[test]
    fn exported_function_declarations_are_function_exports() {
        let src = "export function wipe(){} export class K {} export const v = 1; export const f = () => 1;";
        let (e, g) = run(src);
        let n = &e.exports.names;
        match n.get(&named("wipe")) {
            Some(ExportTarget::Func(id)) => assert_eq!(fn_name(src, &g, *id), "wipe"),
            other => panic!("{other:?}"),
        }
        assert_eq!(n.get(&named("K")), Some(&ExportTarget::Opaque));
        assert_eq!(n.get(&named("v")), Some(&ExportTarget::Opaque));
        match n.get(&named("f")) {
            Some(ExportTarget::Func(id)) => assert_eq!(fn_name(src, &g, *id), "f"),
            other => panic!("{other:?}"),
        }
        assert!(!e.exports.dynamic);
    }

    #[test]
    fn export_lists_and_renames_resolve_by_symbol() {
        let src = "function a(){} const b = 1; export { a as x, b }; export { a as default };";
        let (e, _) = run(src);
        assert!(matches!(e.exports.names.get(&named("x")), Some(ExportTarget::Func(_))));
        assert_eq!(e.exports.names.get(&named("b")), Some(&ExportTarget::Opaque));
        assert!(matches!(e.exports.names.get(&ExportName::Default), Some(ExportTarget::Func(_))));
    }

    #[test]
    fn default_exports_cover_declarations_and_expressions() {
        let (e, _) = run("export default function main(){}");
        assert!(matches!(e.exports.names.get(&ExportName::Default), Some(ExportTarget::Func(_))));
        let (e, _) = run("function f(){} export default f;");
        assert!(matches!(e.exports.names.get(&ExportName::Default), Some(ExportTarget::Func(_))));
        let (e, _) = run("export default 5;");
        assert_eq!(e.exports.names.get(&ExportName::Default), Some(&ExportTarget::Opaque));
    }

    #[test]
    fn reexports_and_stars_are_recorded() {
        let (e, _) = run("export { a as b } from './m'; export * from './n'; export * as ns from './o';");
        assert_eq!(
            e.exports.names.get(&named("b")),
            Some(&ExportTarget::Reexport { spec: "./m".into(), name: named("a") })
        );
        assert_eq!(e.exports.stars, vec!["./n".to_string()]);
        assert_eq!(e.exports.names.get(&named("ns")), Some(&ExportTarget::Opaque));
        let specs: Vec<_> = e.imports.iter().map(|i| i.spec.as_str()).collect();
        assert_eq!(specs, vec!["./m", "./n", "./o"]);
    }

    #[test]
    fn commonjs_assignments_are_exports() {
        let src = "function a(){} exports.a = a; module.exports.b = function(){}; exports.c = 5;";
        let (e, _) = run(src);
        assert!(matches!(e.exports.names.get(&named("a")), Some(ExportTarget::Func(_))));
        assert!(matches!(e.exports.names.get(&named("b")), Some(ExportTarget::Func(_))));
        assert_eq!(e.exports.names.get(&named("c")), Some(&ExportTarget::Opaque));
        assert!(!e.exports.dynamic);
    }

    #[test]
    fn module_exports_equals_a_function_is_the_default() {
        let (e, _) = run("function run(){} module.exports = run;");
        assert!(matches!(e.exports.names.get(&ExportName::Default), Some(ExportTarget::Func(_))));
    }

    #[test]
    fn module_exports_equals_an_object_gives_named_exports() {
        let src = "function a(){} const x = 1; module.exports = { a, b: () => 1, x, m(){ } };";
        let (e, _) = run(src);
        let n = &e.exports.names;
        assert!(matches!(n.get(&named("a")), Some(ExportTarget::Func(_))));
        assert!(matches!(n.get(&named("b")), Some(ExportTarget::Func(_))));
        assert!(matches!(n.get(&named("m")), Some(ExportTarget::Func(_))));
        assert_eq!(n.get(&named("x")), Some(&ExportTarget::Opaque));
        assert_eq!(n.get(&ExportName::Default), Some(&ExportTarget::Opaque));
        assert!(!e.exports.dynamic);
    }

    #[test]
    fn spread_computed_and_unrecognised_uses_make_exports_dynamic() {
        assert!(run("module.exports = { ...other };").0.exports.dynamic);
        assert!(run("const k='a'; module.exports = { [k]: 1 };").0.exports.dynamic);
        assert!(run("Object.assign(module.exports, { a: 1 });").0.exports.dynamic);
        assert!(run("exports[name] = 1;").0.exports.dynamic);
        assert!(run("module.exports ||= {};").0.exports.dynamic);
    }

    #[test]
    fn a_local_variable_named_module_is_not_commonjs() {
        let (e, _) = run("const module = { exports: 1 }; module.exports = 2;");
        assert!(e.exports.names.is_empty());
        assert!(!e.exports.dynamic);
    }

    #[test]
    fn conflicting_assignments_become_opaque() {
        let (e, _) = run("function a(){} exports.a = a; exports.a = 5;");
        assert_eq!(e.exports.names.get(&named("a")), Some(&ExportTarget::Opaque));
    }

    #[test]
    fn imports_are_found_with_the_function_that_loads_them() {
        let src = "import x from './s'; function f(){ require('./r'); import('./d'); } f();";
        let (e, g) = run(src);
        let by = |s: &str| e.imports.iter().find(|i| i.spec == s).unwrap();
        assert_eq!((by("./s").from, by("./s").kind), (0, ImportKind::Static));
        assert_eq!(fn_name(src, &g, by("./r").from), "f");
        assert_eq!(by("./r").kind, ImportKind::Require);
        assert_eq!(by("./d").kind, ImportKind::Dynamic);
    }

    #[test]
    fn type_only_imports_load_nothing() {
        let (e, _) = run("import type { T } from './types'; export type { U } from './u';");
        assert!(e.imports.is_empty());
    }

    #[test]
    fn a_shadowed_require_is_not_a_require() {
        let (e, _) = run("function f(require){ require('./x'); }");
        assert!(e.imports.is_empty());
    }

    #[test]
    fn computed_and_escaped_specifiers_make_no_edge() {
        let (e, _) = run("require(name); import(name); require('a\\u0062');");
        assert!(e.imports.is_empty());
    }

    #[test]
    fn require_member_calls_are_ext_calls() {
        let (e, _) = run("require('./u').wipe(); require('./v')();");
        assert_eq!(e.ext_calls.len(), 2);
        assert_eq!(e.ext_calls[0].name, named("wipe"));
        assert_eq!(e.ext_calls[1].name, ExportName::Default);
    }
}
