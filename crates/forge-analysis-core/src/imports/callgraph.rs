//! Function table and same-file call graph, built from Oxc symbols (never names).
//!
//! Knows nothing about the catalog or the schema. The glue gives it each fact's span to find
//! the function that owns it, and the lane of each timer call, and gets back who calls whom.
//!
//! What it follows:
//!   * a call to a function declared in this file, by declaration or by `const f = () => ..`;
//!   * an immediately invoked function expression;
//!   * a function handed to any call or `new` as an argument, by name or inline. That is a
//!     CALLBACK edge: the callee may run it, now or later, so its effects count for the caller.
//!
//! What it does NOT follow, and counts instead of dropping:
//!   * calls to methods, parameters, imports, built-ins, or anything else not resolved to a
//!     function in this file (`unfollowed_calls` on the calling function);
//!   * functions used as plain values (returned, stored, exported, assigned): `escaped`.

use oxc_ast::ast::{
    Argument, ArrowFunctionExpression, BindingPattern, CallExpression, Declaration,
    ExportDeclaration, ExportDefaultDeclaration, ExportDefaultDeclarationKind, Expression,
    Function, IdentifierReference, NewExpression, Program, VariableDeclarator,
};
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::{ScopeFlags, Scoping, SymbolId};
use oxc_span::{GetSpan, Span};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnNode {
    /// Index in source order; 0 is the top level.
    pub id: u32,
    pub at: Span,
    pub name: Option<Span>,
    pub unfollowed_calls: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallbackEdge {
    pub from: u32,
    pub to: u32,
    /// The call or `new` that received the function.
    pub call: (u32, u32),
}

/// A call whose callee is a local name that is not a function in this file (an import, a
/// parameter, a variable), or a member of one (`ns.f()`). The glue decides what it refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawExtCall {
    pub from: u32,
    /// Byte offset of the declaration of the called name (or of the object, for `ns.f()`).
    pub decl_start: u32,
    /// The member name, for `ns.f()`.
    pub member: Option<Span>,
}

#[derive(Debug, Default)]
pub struct Graph {
    pub fns: Vec<FnNode>,
    pub sym_fn: BTreeMap<SymbolId, u32>,
    pub ext_calls: Vec<RawExtCall>,
    pub calls: Vec<(u32, u32)>,
    pub callbacks: Vec<CallbackEdge>,
    pub escaped: BTreeSet<u32>,
}

type SymFn = BTreeMap<SymbolId, u32>;
type Key = (u32, u32);

fn key(s: Span) -> Key {
    (s.start, s.end)
}

fn peel<'e, 'a>(mut e: &'e Expression<'a>) -> &'e Expression<'a> {
    while let Expression::ParenthesizedExpression(p) = e {
        e = &p.expression;
    }
    e
}

pub fn build(program: &Program<'_>, scoping: &Scoping) -> Graph {
    let mut p1 = Table::default();
    p1.fns.push(FnNode { id: 0, at: program.span, name: None, unfollowed_calls: 0 });
    p1.visit_program(program);
    let by_span: BTreeMap<Key, u32> = p1.fns.iter().skip(1).map(|f| (key(f.at), f.id)).collect();
    let mut p2 = Edges {
        scoping,
        sym_fn: p1.sym_fn,
        by_span,
        stack: vec![0],
        consumed: BTreeSet::new(),
        g: Graph { fns: p1.fns, ..Graph::default() },
    };
    p2.visit_program(program);
    p2.g.sym_fn = p2.sym_fn;
    p2.g
}

impl Graph {
    /// The function whose own span is exactly `at`.
    pub fn id_of(&self, at: Span) -> Option<u32> {
        self.fns.iter().skip(1).find(|f| f.at == at).map(|f| f.id)
    }

    /// The innermost function containing `at`. On equal size the later (inner) one wins, so a
    /// file that is one single function is not mistaken for its own top level.
    pub fn owner(&self, at: Span) -> u32 {
        self.fns
            .iter()
            .filter(|f| f.at.start <= at.start && at.end <= f.at.end)
            .min_by_key(|f| (f.at.end - f.at.start, std::cmp::Reverse(f.id)))
            .map_or(0, |f| f.id)
    }

    /// Everything each function can reach: its own items plus those of every function it
    /// calls or hands off as a callback, to any depth.
    pub fn roll_up<T: Copy + Ord>(&self, direct: &[BTreeSet<T>]) -> Vec<BTreeSet<T>> {
        let mut out: Vec<BTreeSet<T>> = (0..self.fns.len())
            .map(|i| direct.get(i).cloned().unwrap_or_default())
            .collect();
        let edges: Vec<(u32, u32)> = self
            .calls
            .iter()
            .copied()
            .chain(self.callbacks.iter().map(|c| (c.from, c.to)))
            .collect();
        // Monotone, so it settles in at most one round per function.
        for _ in 0..=self.fns.len() {
            let mut changed = false;
            for &(from, to) in &edges {
                let add: Vec<T> = out[to as usize].iter().copied().collect();
                for t in add {
                    changed |= out[from as usize].insert(t);
                }
            }
            if !changed {
                break;
            }
        }
        out
    }

    /// When each function can run. The top level runs at `root`. A callee inherits its
    /// caller's lanes. A callback handed to a call listed in `callback_lane` (a timer) takes
    /// that lane; any other callback inherits its registrar's lanes, which is the one thing
    /// we can say without knowing what the receiving call does with it.
    pub fn lanes<L: Copy + Ord>(&self, root: L, callback_lane: &BTreeMap<Key, L>) -> Vec<BTreeSet<L>> {
        let mut out: Vec<BTreeSet<L>> = vec![BTreeSet::new(); self.fns.len()];
        out[0].insert(root);
        for _ in 0..=self.fns.len() {
            let mut changed = false;
            for &(from, to) in &self.calls {
                let add: Vec<L> = out[from as usize].iter().copied().collect();
                for l in add {
                    changed |= out[to as usize].insert(l);
                }
            }
            for c in &self.callbacks {
                if let Some(&l) = callback_lane.get(&c.call) {
                    if !out[c.from as usize].is_empty() {
                        changed |= out[c.to as usize].insert(l);
                    }
                } else {
                    let add: Vec<L> = out[c.from as usize].iter().copied().collect();
                    for l in add {
                        changed |= out[c.to as usize].insert(l);
                    }
                }
            }
            if !changed {
                break;
            }
        }
        out
    }
}

// ---- pass 1: the function table ----

#[derive(Default)]
struct Table {
    fns: Vec<FnNode>,
    sym_fn: SymFn,
    /// initialiser span -> (variable symbol, variable name span)
    decl_names: BTreeMap<Key, (SymbolId, Span)>,
}

impl Table {
    fn add(&mut self, at: Span, own: Option<(SymbolId, Span)>) {
        let id = self.fns.len() as u32;
        let mut name = None;
        if let Some((sym, sp)) = own {
            self.sym_fn.insert(sym, id);
            name = Some(sp);
        }
        self.fns.push(FnNode { id, at, name, unfollowed_calls: 0 });
    }
}

impl<'a> Visit<'a> for Table {
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let (BindingPattern::BindingIdentifier(b), Some(init)) = (&it.id, &it.init) {
            let init = peel(init);
            if matches!(init, Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_)) {
                self.decl_names.insert(key(init.span()), (b.symbol_id(), b.span));
            }
        }
        walk::walk_variable_declarator(self, it);
    }
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let own = self
            .decl_names
            .get(&key(it.span))
            .copied()
            .or_else(|| it.id.as_ref().map(|i| (i.symbol_id(), i.span)));
        self.add(it.span, own);
        walk::walk_function(self, it, flags);
    }
    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let own = self.decl_names.get(&key(it.span)).copied();
        self.add(it.span, own);
        walk::walk_arrow_function_expression(self, it);
    }
}

// ---- pass 2: the edges ----

struct Edges<'s> {
    scoping: &'s Scoping,
    sym_fn: SymFn,
    by_span: BTreeMap<Key, u32>,
    stack: Vec<u32>,
    consumed: BTreeSet<Key>,
    g: Graph,
}

impl Edges<'_> {
    fn cur(&self) -> u32 {
        *self.stack.last().unwrap_or(&0)
    }
    fn resolve(&self, r: &IdentifierReference<'_>) -> Option<u32> {
        let rid = r.reference_id.get()?;
        let sym = self.scoping.get_reference(rid).symbol_id()?;
        self.sym_fn.get(&sym).copied()
    }
    fn symbol_of(&self, r: &IdentifierReference<'_>) -> Option<SymbolId> {
        let rid = r.reference_id.get()?;
        self.scoping.get_reference(rid).symbol_id()
    }
    fn note_ext(&mut self, callee: &Expression<'_>) {
        let from = self.cur();
        match callee {
            Expression::Identifier(r) => {
                if let Some(sym) = self.symbol_of(r) {
                    let decl_start = self.scoping.symbol_span(sym).start;
                    self.g.ext_calls.push(RawExtCall { from, decl_start, member: None });
                }
            }
            Expression::StaticMemberExpression(m) => {
                if let Expression::Identifier(r) = peel(&m.object) {
                    if let Some(sym) = self.symbol_of(r) {
                        let decl_start = self.scoping.symbol_span(sym).start;
                        self.g.ext_calls.push(RawExtCall {
                            from,
                            decl_start,
                            member: Some(m.property.span),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    fn inline_fn(&self, e: &Expression<'_>) -> Option<u32> {
        match peel(e) {
            Expression::ArrowFunctionExpression(f) => self.by_span.get(&key(f.span)).copied(),
            Expression::FunctionExpression(f) => self.by_span.get(&key(f.span)).copied(),
            _ => None,
        }
    }
    fn arguments(&mut self, args: &[Argument<'_>], call: Key) {
        let from = self.cur();
        for a in args {
            let to = match a {
                Argument::Identifier(r) => {
                    let t = self.resolve(r);
                    if t.is_some() {
                        self.consumed.insert(key(r.span));
                    }
                    t
                }
                Argument::ArrowFunctionExpression(f) => self.by_span.get(&key(f.span)).copied(),
                Argument::FunctionExpression(f) => self.by_span.get(&key(f.span)).copied(),
                _ => None,
            };
            if let Some(to) = to {
                self.g.callbacks.push(CallbackEdge { from, to, call });
            }
        }
    }
    fn unfollowed(&mut self) {
        let c = self.cur() as usize;
        self.g.fns[c].unfollowed_calls += 1;
    }
    fn enter(&mut self, at: Span) {
        if let Some(&id) = self.by_span.get(&key(at)) {
            self.stack.push(id);
        }
    }
    fn leave(&mut self, at: Span) {
        if self.by_span.contains_key(&key(at)) {
            self.stack.pop();
        }
    }
}

impl<'a> Visit<'a> for Edges<'_> {
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        self.enter(it.span);
        walk::walk_function(self, it, flags);
        self.leave(it.span);
    }
    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.enter(it.span);
        walk::walk_arrow_function_expression(self, it);
        self.leave(it.span);
    }
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let from = self.cur();
        let target = match peel(&it.callee) {
            Expression::Identifier(r) => {
                let t = self.resolve(r);
                if t.is_some() {
                    self.consumed.insert(key(r.span));
                }
                t
            }
            other => self.inline_fn(other),
        };
        match target {
            Some(to) => self.g.calls.push((from, to)),
            None => {
                self.unfollowed();
                self.note_ext(peel(&it.callee));
            }
        }
        self.arguments(&it.arguments, key(it.span));
        walk::walk_call_expression(self, it);
    }
    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        self.unfollowed();
        self.arguments(&it.arguments, key(it.span));
        walk::walk_new_expression(self, it);
    }
    fn visit_export_declaration(&mut self, it: &ExportDeclaration<'a>) {
        match &it.declaration {
            Declaration::FunctionDeclaration(f) => {
                if let Some(&id) = self.by_span.get(&key(f.span)) {
                    self.g.escaped.insert(id);
                }
            }
            Declaration::VariableDeclaration(v) => {
                for d in &v.declarations {
                    if let Some(init) = &d.init {
                        if let Some(id) = self.inline_fn(init) {
                            self.g.escaped.insert(id);
                        }
                    }
                }
            }
            _ => {}
        }
        walk::walk_export_declaration(self, it);
    }
    fn visit_export_default_declaration(&mut self, it: &ExportDefaultDeclaration<'a>) {
        if let ExportDefaultDeclarationKind::FunctionDeclaration(f) = &it.declaration {
            if let Some(&id) = self.by_span.get(&key(f.span)) {
                self.g.escaped.insert(id);
            }
        }
        walk::walk_export_default_declaration(self, it);
    }
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if !self.consumed.contains(&key(it.span)) {
            if let Some(f) = self.resolve(it) {
                self.g.escaped.insert(f);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;

    fn graph(src: &str) -> Graph {
        let alloc = Allocator::default();
        let ret = Parser::new(&alloc, src, SourceType::mjs()).parse();
        assert!(ret.diagnostics.is_empty(), "parse: {:?}", ret.diagnostics);
        let sem = SemanticBuilder::new().build(&ret.program);
        build(&ret.program, sem.semantic.scoping())
    }

    fn name_of(src: &str, g: &Graph, id: u32) -> String {
        g.fns[id as usize]
            .name
            .map_or("<anon>".to_string(), |s| src[s.start as usize..s.end as usize].to_string())
    }

    fn edges(src: &str) -> Vec<(String, String)> {
        let g = graph(src);
        g.calls.iter().map(|&(a, b)| (name_of(src, &g, a), name_of(src, &g, b))).collect()
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    #[test]
    fn the_top_level_is_function_zero() {
        let g = graph("1;");
        assert_eq!(g.fns.len(), 1);
        assert_eq!(g.fns[0].id, 0);
    }

    #[test]
    fn a_call_to_a_declared_function_is_an_edge() {
        assert_eq!(edges("function a(){} a();"), vec![pair("<anon>", "a")]);
    }

    #[test]
    fn calls_resolve_before_the_declaration_is_seen() {
        assert_eq!(edges("a(); function a(){}"), vec![pair("<anon>", "a")]);
    }

    #[test]
    fn arrow_and_function_expressions_bound_to_variables_are_named() {
        let src = "const f = () => 1; const g = function(){}; f(); g();";
        assert_eq!(edges(src), vec![pair("<anon>", "f"), pair("<anon>", "g")]);
    }

    #[test]
    fn nested_calls_belong_to_the_inner_function() {
        let src = "function a(){ b(); } function b(){} a();";
        let e = edges(src);
        assert!(e.contains(&pair("a", "b")));
        assert!(e.contains(&pair("<anon>", "a")));
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn a_named_function_expression_is_named() {
        let src = "setInterval(function tick(){ }, 1);";
        let g = graph(src);
        assert_eq!(name_of(src, &g, 1), "tick");
    }

    #[test]
    fn recursion_is_an_edge_to_itself() {
        assert!(edges("function a(){ a(); }").contains(&pair("a", "a")));
    }

    #[test]
    fn an_iife_is_a_call() {
        let g = graph("(function(){ })(); (() => {})();");
        assert_eq!(g.calls, vec![(0, 1), (0, 2)]);
    }

    #[test]
    fn a_shadowing_local_is_not_the_outer_function() {
        let src = "function a(){} function f(a){ a(); } f(1);";
        let g = graph(src);
        // the inner `a()` calls a parameter, so it is unfollowed, not an edge to `a`
        let f = g.fns.iter().find(|n| name_of(src, &g, n.id) == "f").unwrap();
        assert_eq!(f.unfollowed_calls, 1);
        assert!(!g.calls.iter().any(|&(from, to)| from == f.id && name_of(src, &g, to) == "a"));
    }

    #[test]
    fn methods_and_imports_are_counted_not_followed() {
        let g = graph("import x from 'y'; x(); obj.m(); new Thing();");
        assert!(g.calls.is_empty());
        assert_eq!(g.fns[0].unfollowed_calls, 3);
    }

    #[test]
    fn an_inline_callback_is_a_callback_edge() {
        let g = graph("setTimeout(() => { }, 5);");
        assert_eq!(g.callbacks.len(), 1);
        assert_eq!(g.callbacks[0].from, 0);
        assert_eq!(g.callbacks[0].to, 1);
        assert_eq!(g.fns[0].unfollowed_calls, 1);
    }

    #[test]
    fn a_named_callback_is_a_callback_edge_and_does_not_escape() {
        let g = graph("function h(){} setTimeout(h, 5);");
        assert_eq!(g.callbacks.len(), 1);
        assert!(g.escaped.is_empty());
    }

    #[test]
    fn a_function_used_as_a_value_escapes() {
        let g = graph("function h(){} const x = h; export { h };");
        assert!(g.escaped.contains(&1));
    }

    #[test]
    fn calls_to_unknown_names_are_recorded_for_linking() {
        let src = "import { a } from 'm'; import * as ns from 'n'; a(); ns.f(); local();";
        let g = graph(src);
        assert_eq!(g.ext_calls.len(), 2, "unresolved globals like local() are not recorded");
        assert!(g.ext_calls[0].member.is_none());
        let m = g.ext_calls[1].member.unwrap();
        assert_eq!(&src[m.start as usize..m.end as usize], "f");
    }

    #[test]
    fn id_of_finds_a_function_by_its_span() {
        let src = "function a(){}";
        let g = graph(src);
        assert_eq!(g.id_of(g.fns[1].at), Some(1));
        assert_eq!(g.id_of(Span::new(0, 1)), None);
    }

    #[test]
    fn exported_declarations_escape() {
        assert!(graph("export function a(){}").escaped.contains(&1));
        assert!(graph("export default function(){}").escaped.contains(&1));
        assert!(graph("export const f = () => 1;").escaped.contains(&1));
    }

    #[test]
    fn a_called_function_does_not_escape() {
        assert!(graph("function h(){} h();").escaped.is_empty());
    }

    #[test]
    fn owner_is_the_innermost_function() {
        let src = "function a(){ function b(){ X; } Y; } Z;";
        let g = graph(src);
        let at = |t: &str| {
            let s = src.find(t).unwrap() as u32;
            Span::new(s, s + 1)
        };
        assert_eq!(name_of(src, &g, g.owner(at("X"))), "b");
        assert_eq!(name_of(src, &g, g.owner(at("Y"))), "a");
        assert_eq!(g.owner(at("Z")), 0);
    }

    #[test]
    fn a_file_that_is_exactly_one_function_still_has_it_as_owner() {
        let src = "function only(){ X; }";
        let g = graph(src);
        let s = src.find('X').unwrap() as u32;
        assert_eq!(name_of(src, &g, g.owner(Span::new(s, s + 1))), "only");
    }

    #[test]
    fn roll_up_reaches_through_calls_and_callbacks() {
        let src = "function a(){ b(); } function b(){ setTimeout(() => { c(); }, 1); } function c(){ DEL; } a();";
        let g = graph(src);
        let mut direct: Vec<BTreeSet<&str>> = vec![BTreeSet::new(); g.fns.len()];
        let c = g.fns.iter().find(|n| name_of(src, &g, n.id) == "c").unwrap().id;
        direct[c as usize].insert("fs_delete");
        let up = g.roll_up(&direct);
        let a = g.fns.iter().find(|n| name_of(src, &g, n.id) == "a").unwrap().id;
        assert!(up[a as usize].contains("fs_delete"));
        assert!(up[0].contains("fs_delete"));
    }

    #[test]
    fn roll_up_terminates_on_mutual_recursion() {
        let g = graph("function a(){ b(); } function b(){ a(); } a();");
        let mut direct: Vec<BTreeSet<u8>> = vec![BTreeSet::new(); g.fns.len()];
        direct[2].insert(7);
        let up = g.roll_up(&direct);
        assert!(up[1].contains(&7) && up[2].contains(&7) && up[0].contains(&7));
    }

    #[test]
    fn lanes_follow_calls_and_timer_callbacks() {
        let src = "function a(){} function b(){} a(); setTimeout(() => { b(); }, 5);";
        let g = graph(src);
        let call = g.callbacks[0].call;
        let mut cl = BTreeMap::new();
        cl.insert(call, "deferred");
        let lanes = g.lanes("immediate", &cl);
        let id = |n: &str| g.fns.iter().find(|f| name_of(src, &g, f.id) == n).unwrap().id as usize;
        assert_eq!(lanes[0], BTreeSet::from(["immediate"]));
        assert_eq!(lanes[id("a")], BTreeSet::from(["immediate"]));
        // b is only called from inside the timer callback
        assert_eq!(lanes[id("b")], BTreeSet::from(["deferred"]));
    }

    #[test]
    fn a_function_nothing_reaches_has_no_lane() {
        let g = graph("function lonely(){} ");
        let lanes = g.lanes("immediate", &BTreeMap::new());
        assert!(lanes[1].is_empty());
    }

    #[test]
    fn a_callback_to_an_unknown_call_inherits_the_registrars_lanes() {
        let g = graph("[1].map(() => { });");
        let lanes = g.lanes("immediate", &BTreeMap::new());
        assert_eq!(lanes[1], BTreeSet::from(["immediate"]));
    }

    #[test]
    fn a_function_called_from_two_lanes_has_both() {
        let src = "function w(){} w(); setInterval(() => { w(); }, 9);";
        let g = graph(src);
        let mut cl = BTreeMap::new();
        cl.insert(g.callbacks[0].call, "recurring");
        let lanes = g.lanes("immediate", &cl);
        assert_eq!(lanes[1], BTreeSet::from(["immediate", "recurring"]));
    }

    #[test]
    fn class_methods_and_object_methods_are_functions() {
        let g = graph("class A { m(){ } } const o = { n(){ } };");
        assert_eq!(g.fns.len(), 3);
    }
}
