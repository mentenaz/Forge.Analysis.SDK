//! Import table: which local names refer to which modules.
//! Top-level statements only in this cut. Nested scopes come with the visitor step.
//! Names and string literals are read from source text by span, so nothing depends on
//! Oxc's string types, and anything with an escape sequence is reported, not interpreted.

use forge_analysis_schema::{FactKind, Quoted, RelPath, Span, StaticFact, UnresolvedReason,Effect};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, BindingPattern,BindingIdentifier, CallExpression, Expression, ImportDeclaration,
    ImportDeclarationSpecifier, ModuleExportName, ObjectPattern, Program, PropertyKey, Statement,
    StringLiteral, VariableDeclarator,
};
use oxc_parser::Parser;
use oxc_span::{SourceType, Span as OxcSpan};
use oxc_semantic::Scoping;
use std::collections::BTreeSet;
mod flow;
mod handles;
mod visitor;
pub use flow::FlowHit;
pub use visitor::{analyze_file, analyze_file_full, max_bracket_depth, Analysis, MAX_BRACKET_DEPTH};
mod callgraph;
mod summary;
mod exports;
mod link;
pub use link::{link as link_files, ExportName, ExportTarget, Exports, ExtCall, ExtRef, ImportEdge, ImportKind, LinkFile, LinkInfo, LocalEdges};
mod json;
mod package;

pub use link::{link_project, link_seeded, EntryWant};
pub use package::{analyze_package, classify_dependency, interpret_command, Action, PackageAnalysis};


#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingKind {
    /// `import * as x`, or `const x = require("m")`
    Namespace,
    /// `import x from "m"`
    Default,
    /// `import { a as x }`, `const { a: x } = require("m")`, `require("m").a`
    Named { imported: String },
    /// An alias of a code-loading global: `const e = eval`
       GlobalAlias { name: String },
       /// The global object: `const g = globalThis`
       GlobalObject,
       /// A member of a module that we could not name: `fs.promises.readFile`, `fs[name]`
       Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub local: String,
    /// Module name without a `node:` prefix.
    pub module: String,
    pub kind: BindingKind,
    /// Byte offset of the declaring identifier; lets Oxc's scope analysis point back to us.
    pub decl_start: u32,
}

#[derive(Debug, Clone, Default)]
pub struct ImportTable {
    bindings: Vec<Binding>,
    ambiguous: BTreeSet<String>,
}

impl ImportTable {
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// A name bound more than once is ambiguous, so it resolves to nothing.
    pub fn lookup(&self, local: &str) -> Option<&Binding> {
        if self.ambiguous.contains(local) {
            return None;
        }
        self.bindings.iter().find(|b| b.local == local)
    }

    /// The binding declared at this byte offset, unless its name is ambiguous.
    pub fn lookup_by_decl(&self, decl_start: u32) -> Option<&Binding> {
        let b = self.bindings.iter().find(|b| b.decl_start == decl_start)?;
        if self.ambiguous.contains(&b.local) {
            None
        } else {
            Some(b)
        }
    }

    fn add(&mut self, b: Binding) -> bool {
        if self.bindings.iter().any(|x| x.local == b.local) {
            self.ambiguous.insert(b.local);
            return false;
        }
        self.bindings.push(b);
        true
    }
}

#[derive(Debug, Clone)]
pub struct FileImports {
    pub table: ImportTable,
    pub facts: Vec<StaticFact>,
}

pub fn normalize_module(name: &str) -> String {
    name.strip_prefix("node:").unwrap_or(name).to_string()
}

/// Returns None if the file name is not a JS/TS source type.
/// Top-level statements only; `analyze_file` is the whole-file version.
pub fn analyze_imports(file: &RelPath, source: &str) -> Option<FileImports> {
    let source_type = SourceType::from_path(file.as_quoted().text()).ok()?;
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, source_type).parse();

    let mut ctx = Ctx::new(file, source);

    // A file that did not parse cleanly gets one fact and nothing else: facts taken
    // from a partly recovered tree could be wrong.
    if !ret.diagnostics.is_empty() {
        ctx.parse_failed();
        return Some(ctx.finish());
    }

    top_level(&mut ctx, &ret.program);
    Some(ctx.finish())
}

fn top_level(ctx: &mut Ctx<'_>, program: &Program<'_>) {
    for stmt in &program.body {
        match stmt {
            Statement::ImportDeclaration(decl) => ctx.import_declaration(decl),
            Statement::VariableDeclaration(vd) => {
                for d in &vd.declarations {
                    ctx.declarator(d);
                }
            }
            Statement::ExpressionStatement(es) => {
                if let Some((outcome, sp)) = ctx.classify_require(&es.expression) {
                    ctx.emit_require(&outcome, sp);
                }
            }
            _ => {}
        }
    }
}

enum Require {
    Literal(String),
    Dynamic,
}

struct LineIndex {
    starts: Vec<u32>,
}

impl LineIndex {
    fn new(src: &str) -> Self {
        let mut starts = vec![0u32];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(u32::try_from(i + 1).unwrap_or(u32::MAX));
            }
        }
        Self { starts }
    }

    /// 1-based line of a byte offset.
    fn line(&self, byte: u32) -> u32 {
        self.starts.partition_point(|&s| s <= byte) as u32
    }
}

struct Ctx<'s> {
    src: &'s str,
    file: &'s RelPath,
    lines: LineIndex,
    table: ImportTable,
    facts: Vec<StaticFact>,
    /// Start offsets of require calls the top-level pass already reported.
    handled: BTreeSet<u32>,
    scoping: Option<&'s Scoping>,
    consumed: BTreeSet<(u32, u32)>,
    /// Declaration offsets of names that hold React's state setter or dispatcher
    /// (`const [x, setX] = useState()`): calling one only updates React's own state.
    setters: BTreeSet<u32>,
}

impl<'s> Ctx<'s> {
    fn new(file: &'s RelPath, src: &'s str) -> Self {
        Self {
            src,
            file,
            lines: LineIndex::new(src),
            table: ImportTable::default(),
            facts: Vec::new(),
            handled: BTreeSet::new(),
            scoping: None,
            consumed: BTreeSet::new(),
            setters: BTreeSet::new(),
        }
    }

    fn with_scoping(mut self, scoping: &'s Scoping) -> Self {
        self.scoping = Some(scoping);
        self
    }

    fn parse_failed(&mut self) {
        let end = u32::try_from(self.src.len()).unwrap_or(u32::MAX);
        self.unresolved(OxcSpan::new(0, end), UnresolvedReason::ParseFailed, "");
    }

    fn too_deep(&mut self) {
        let end = u32::try_from(self.src.len()).unwrap_or(u32::MAX);
        self.unresolved(OxcSpan::new(0, end), UnresolvedReason::TooDeeplyNested, "");
    }

    fn finish(self) -> FileImports {
        FileImports {
            table: self.table,
            facts: self.facts,
        }
    }

    fn text(&self, sp: OxcSpan) -> &'s str {
        let src: &'s str = self.src;
        src.get(sp.start as usize..sp.end as usize).unwrap_or("")
    }

    fn span(&self, sp: OxcSpan) -> Span {
        Span {
            file: self.file.clone(),
            start_line: self.lines.line(sp.start),
            end_line: self.lines.line(sp.end.saturating_sub(1).max(sp.start)),
            start_byte: sp.start,
            end_byte: sp.end,
        }
    }

    fn import_fact(&mut self, sp: OxcSpan, module_as_written: &str) {
        let f = StaticFact::new(FactKind::Import, self.span(sp), Some(Quoted::new(module_as_written)));
        self.facts.push(f);

        // A module fetched from a URL is code we cannot see.
        let lower = module_as_written.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("data:") {
            let mut e = StaticFact::new(
                FactKind::Effect,
                self.span(sp),
                Some(Quoted::new(module_as_written)),
            );
            e.effect = Some(Effect::DynamicLoad);
            self.facts.push(e);
        }
    }

    fn unresolved(&mut self, sp: OxcSpan, reason: UnresolvedReason, detail: &str) {
        self.unresolved_in(sp, reason, detail, None);
    }

    /// An unresolved fact for a call into `module` (normalised), which the report groups by.
    fn unresolved_in(&mut self, sp: OxcSpan, reason: UnresolvedReason, detail: &str, module: Option<&str>) {
        let mut f = StaticFact::new(
            FactKind::Unresolved,
            self.span(sp),
            Some(Quoted::new(detail)),
        );
        f.unresolved = Some(reason);
        f.module = module.map(Quoted::new);
        self.facts.push(f);
    }

    /// The text of a plain string literal, or None if it has an escape sequence.
    fn literal(&self, lit: &StringLiteral<'_>) -> Option<String> {
        let raw = self.text(lit.span);
        let bytes = raw.as_bytes();
        let q = *bytes.first()?;
        if raw.len() < 2 || (q != b'"' && q != b'\'') || bytes[raw.len() - 1] != q {
            return None;
        }
        let inner = &raw[1..raw.len() - 1];
        if inner.contains('\\') {
            return None;
        }
        Some(inner.to_string())
    }

    fn bind(
        &mut self,
        sp: OxcSpan,
        decl: &BindingIdentifier<'_>,
        local: String,
        module: &str,
        kind: BindingKind,
    ) {
        // A name that is written again later cannot be trusted to keep pointing at one thing.
        if self.is_rewritten(decl) {
            self.unresolved(sp, UnresolvedReason::AmbiguousBinding, &local);
            return;
        }
        let b = Binding {
            local: local.clone(),
            module: module.to_string(),
            kind,
            decl_start: decl.span.start,
        };
        if !self.table.add(b) {
            self.unresolved(sp, UnresolvedReason::AmbiguousBinding, &local);
        }
    }

    fn is_rewritten(&self, id: &BindingIdentifier<'_>) -> bool {
        let Some(scoping) = self.scoping else { return false };
        let Some(sym) = id.symbol_id.get() else { return false };
        scoping
            .get_resolved_reference_ids(sym)
            .iter()
            .any(|r| scoping.get_reference(*r).is_write())
    }

    fn classify_require(&self, expr: &Expression<'_>) -> Option<(Require, OxcSpan)> {
        let Expression::CallExpression(call) = expr else {
            return None;
        };
        self.classify_call(call)
    }

    fn classify_call(&self, call: &CallExpression<'_>) -> Option<(Require, OxcSpan)> {
        let Expression::Identifier(callee) = &call.callee else {
            return None;
        };
        if self.text(callee.span) != "require" {
            return None;
        }
        if let (Some(scoping), Some(rid)) = (self.scoping, callee.reference_id.get()) {
            if scoping.get_reference(rid).symbol_id().is_some() {
                return None; // a local named `require`, not Node's
            }
        }
        let outcome = match (call.arguments.len(), call.arguments.first()) {
            (1, Some(Argument::StringLiteral(lit))) => match self.literal(lit) {
                Some(v) => Require::Literal(v),
                None => Require::Dynamic,
            },
            _ => Require::Dynamic,
        };
        Some((outcome, call.span))
    }

    /// Records the fact for a require call. Returns the module if it was a plain literal.
    fn emit_require(&mut self, outcome: &Require, sp: OxcSpan) -> Option<String> {
        self.handled.insert(sp.start);
        match outcome {
            Require::Literal(m) => {
                self.import_fact(sp, m);
                Some(normalize_module(m))
            }
            Require::Dynamic => {
                let t = self.text(sp);
                self.unresolved(sp, UnresolvedReason::DynamicRequire, t);
                None
            }
        }
    }

    fn import_declaration(&mut self, decl: &ImportDeclaration<'_>) {
        // Type-only imports are erased at runtime: no fact, no binding.
        if decl.import_kind.is_type() {
            return;
        }
        let Some(written) = self.literal(&decl.source) else {
            let t = self.text(decl.source.span);
            self.unresolved(decl.span, UnresolvedReason::UnsupportedSyntax, t);
            return;
        };
        self.import_fact(decl.span, &written);
        let module = normalize_module(&written);

        let Some(specs) = &decl.specifiers else {
            return;
        };
        for spec in specs {
            match spec {
                ImportDeclarationSpecifier::ImportSpecifier(s) => {
                    let imported = match &s.imported {
                        ModuleExportName::IdentifierName(n) => Some(self.text(n.span).to_string()),
                        ModuleExportName::IdentifierReference(r) => {
                            Some(self.text(r.span).to_string())
                        }
                        ModuleExportName::StringLiteral(l) => self.literal(l),
                    };
                    let local = self.text(s.local.span).to_string();
                    match imported {
                        Some(imported) => {
                            self.bind(s.span, &s.local, local, &module, BindingKind::Named { imported })
                        }
                        None => {
                            let t = self.text(s.span);
                            self.unresolved(s.span, UnresolvedReason::UnsupportedSyntax, t);
                        }
                    }
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                    let local = self.text(s.local.span).to_string();
                    self.bind(s.span, &s.local, local, &module, BindingKind::Default);
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                    let local = self.text(s.local.span).to_string();
                    self.bind(s.span, &s.local, local, &module, BindingKind::Namespace);
                }
            }
        }
    }

    fn declarator(&mut self, d: &VariableDeclarator<'_>) {
        let Some(init) = &d.init else { return };

        // const x = require("m")   /   const { a, b: c } = require("m")
        if let Some((outcome, sp)) = self.classify_require(init) {
            let Some(module) = self.emit_require(&outcome, sp) else {
                return;
            };
            match &d.id {
                BindingPattern::BindingIdentifier(id) => {
                    let local = self.text(id.span).to_string();
                    self.bind(id.span, id, local, &module, BindingKind::Namespace);
                }
                BindingPattern::ObjectPattern(p) => self.object_pattern(p, &module),
                _ => {
                    let t = self.text(d.span);
                    self.unresolved(d.span, UnresolvedReason::UnsupportedSyntax, t);
                }
            }
            return;
        }

        // const f = require("m").name
        if let Expression::StaticMemberExpression(m) = init {
            if let Some((outcome, sp)) = self.classify_require(&m.object) {
                let module = self.emit_require(&outcome, sp);
                match (module, &d.id) {
                    (Some(module), BindingPattern::BindingIdentifier(id)) => {
                        let imported = self.text(m.property.span).to_string();
                        let local = self.text(id.span).to_string();
                        self.bind(id.span, id, local, &module, BindingKind::Named { imported });
                    }
                    (Some(_), _) => {
                        let t = self.text(d.span);
                        self.unresolved(d.span, UnresolvedReason::UnsupportedSyntax, t);
                    }
                    (None, _) => {}
                }
            }
        }
    }

    fn object_pattern(&mut self, p: &ObjectPattern<'_>, module: &str) {
        for prop in &p.properties {
            if prop.computed {
                let t = self.text(prop.span);
                self.unresolved(prop.span, UnresolvedReason::ComputedAccess, t);
                continue;
            }
            let imported = match &prop.key {
                PropertyKey::StaticIdentifier(k) => Some(self.text(k.span).to_string()),
                PropertyKey::StringLiteral(l) => self.literal(l),
                _ => None,
            };
            let target = match &prop.value {
                BindingPattern::AssignmentPattern(a) => &a.left,
                other => other,
            };
            match (imported, target) {
                (Some(imported), BindingPattern::BindingIdentifier(id)) => {
                    let local = self.text(id.span).to_string();
                    self.bind(id.span, id, local, module, BindingKind::Named { imported });
                }
                _ => {
                    let t = self.text(prop.span);
                    self.unresolved(prop.span, UnresolvedReason::UnsupportedSyntax, t);
                }
            }
        }
        if p.rest.is_some() {
            let t = self.text(p.span);
            self.unresolved(p.span, UnresolvedReason::UnsupportedSyntax, t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(name: &str, src: &str) -> FileImports {
        analyze_imports(&RelPath::new(name).unwrap(), src).unwrap()
    }

    fn count(f: &FileImports, kind: FactKind) -> usize {
        f.facts.iter().filter(|x| x.kind() == kind).count()
    }

    fn unresolved_reasons(f: &FileImports) -> Vec<UnresolvedReason> {
        f.facts.iter().filter_map(|x| x.unresolved).collect()
    }

    #[test]
    fn es_imports_default_named_and_namespace() {
        let f = run(
            "a.ts",
            r#"import fs from "fs"; import { readFile as rf, writeFile } from "node:fs"; import * as os from "os";"#,
        );
        assert_eq!(f.table.lookup("fs").unwrap().kind, BindingKind::Default);
        let rf = f.table.lookup("rf").unwrap();
        assert_eq!(rf.module, "fs");
        assert_eq!(
            rf.kind,
            BindingKind::Named {
                imported: "readFile".into()
            }
        );
        assert_eq!(f.table.lookup("writeFile").unwrap().module, "fs");
        assert_eq!(f.table.lookup("os").unwrap().kind, BindingKind::Namespace);
        assert_eq!(count(&f, FactKind::Import), 3);
    }

    #[test]
    fn require_of_a_whole_module() {
        let f = run("a.js", r#"const cp = require("child_process");"#);
        let b = f.table.lookup("cp").unwrap();
        assert_eq!(b.module, "child_process");
        assert_eq!(b.kind, BindingKind::Namespace);
        assert_eq!(count(&f, FactKind::Import), 1);
    }

    #[test]
    fn require_destructuring_with_alias_and_default() {
        let f = run(
            "a.js",
            r#"const { exec, spawn: sp, x = 1 } = require("node:child_process");"#,
        );
        assert_eq!(
            f.table.lookup("exec").unwrap().kind,
            BindingKind::Named {
                imported: "exec".into()
            }
        );
        assert_eq!(
            f.table.lookup("sp").unwrap().kind,
            BindingKind::Named {
                imported: "spawn".into()
            }
        );
        assert_eq!(f.table.lookup("x").unwrap().module, "child_process");
        assert!(unresolved_reasons(&f).is_empty());
    }

    #[test]
    fn require_with_member_access() {
        let f = run("a.js", r#"const rf = require("fs").readFile;"#);
        let b = f.table.lookup("rf").unwrap();
        assert_eq!(
            b.kind,
            BindingKind::Named {
                imported: "readFile".into()
            }
        );
    }

    #[test]
    fn dynamic_require_is_unresolved_and_binds_nothing() {
        let f = run("a.js", r#"const name = "fs"; const m = require(name);"#);
        assert!(f.table.lookup("m").is_none());
        assert_eq!(
            unresolved_reasons(&f),
            vec![UnresolvedReason::DynamicRequire]
        );
        assert_eq!(count(&f, FactKind::Import), 0);
    }

    #[test]
    fn side_effect_imports_have_facts_but_no_bindings() {
        let f = run("a.js", r#"import "x"; require("y");"#);
        assert_eq!(count(&f, FactKind::Import), 2);
        assert!(f.table.bindings().is_empty());
    }

    #[test]
    fn a_name_bound_twice_is_ambiguous() {
        let f = run("a.js", r#"var a = require("x"); var a = require("y");"#);
        assert!(f.table.lookup("a").is_none());
        assert_eq!(
            unresolved_reasons(&f),
            vec![UnresolvedReason::AmbiguousBinding]
        );
    }

    #[test]
    fn type_only_imports_are_ignored() {
        let f = run("a.ts", r#"import type { T } from "./t";"#);
        assert!(f.facts.is_empty());
        assert!(f.table.bindings().is_empty());
    }

    #[test]
    fn a_file_with_syntax_errors_gives_one_parse_failed_fact_only() {
        let f = run("a.js", "const = ;");
        assert_eq!(f.facts.len(), 1);
        assert_eq!(unresolved_reasons(&f), vec![UnresolvedReason::ParseFailed]);
        assert!(f.table.bindings().is_empty());
    }

    #[test]
    fn non_source_files_are_not_analysed() {
        assert!(analyze_imports(&RelPath::new("notes.txt").unwrap(), "hello").is_none());
    }

    #[test]
    fn facts_carry_lines_and_distinct_ids_even_on_one_line() {
        let f = run(
            "a.js",
            "const a = require(\"a\"); const b = require(\"b\");\nconst c = require(\"c\");\n",
        );
        assert_eq!(f.facts.len(), 3);
        assert_eq!(f.facts[0].at().start_line, 1);
        assert_eq!(f.facts[1].at().start_line, 1);
        assert_eq!(f.facts[2].at().start_line, 2);
        let mut ids: Vec<_> = f.facts.iter().map(|x| x.id().clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3);
        assert!(f.facts.iter().all(|x| x.validate().is_empty()));
    }

    #[test]
    fn computed_destructuring_key_is_reported() {
        let f = run("a.js", r#"const k = "a"; const { [k]: v } = require("m");"#);
        assert!(f.table.lookup("v").is_none());
        assert_eq!(
            unresolved_reasons(&f),
            vec![UnresolvedReason::ComputedAccess]
        );
    }

    #[test]
    fn rest_in_destructuring_is_reported_but_named_parts_still_bind() {
        let f = run("a.js", r#"const { a, ...rest } = require("m");"#);
        assert!(f.table.lookup("a").is_some());
        assert!(f.table.lookup("rest").is_none());
        assert_eq!(
            unresolved_reasons(&f),
            vec![UnresolvedReason::UnsupportedSyntax]
        );
    }

    #[test]
    fn escape_sequences_in_module_names_are_not_interpreted() {
        let f = run("a.js", r#"import x from "a\u0062";"#);
        assert!(f.table.lookup("x").is_none());
        assert_eq!(
            unresolved_reasons(&f),
            vec![UnresolvedReason::UnsupportedSyntax]
        );
    }
}
