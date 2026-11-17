//! Timer handles: is the value a timer call returns ever cleared?
//!
//! `cleared` on a timer fact is a three-way answer, never a guess:
//!   * `Some(true)`  a `clearTimeout` / `clearInterval` / `clearImmediate` call exists for it;
//!   * `Some(false)` the handle is thrown away, or kept and never cleared or passed on;
//!   * `None`        the handle escapes (returned, passed to another call, stored in an object,
//!                   exported, ...), so a clear may happen where we cannot see it.
//! "Cleared" means a clear call exists, not that it definitely runs.
//!
//! A clear is recognised by name: `clearTimeout(h)` where `clearTimeout` is the unshadowed
//! global, or `x.clearTimeout(h)` on any object (we cannot know what `x` is, so we accept it
//! rather than wrongly report the timer as never cleared).

use oxc_ast::ast::{
    Argument, AssignmentExpression, AssignmentTarget, BindingPattern, CallExpression, Expression,
    ExpressionStatement, IdentifierReference, VariableDeclarator,
};
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::{ReferenceId, Scoping, SymbolId};
use oxc_span::Span;
use std::collections::{BTreeMap, BTreeSet};

const CLEARS: [&str; 3] = ["clearTimeout", "clearInterval", "clearImmediate"];
/// Methods on a timer object that neither clear nor leak it.
const BENIGN: [&str; 4] = ["unref", "ref", "hasRef", "refresh"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Use {
    Discarded,
    Stored(SymbolId),
}

#[derive(Debug, Default)]
pub struct Handles {
    uses: BTreeMap<(u32, u32), Use>,
    cleared: BTreeSet<SymbolId>,
    escaping: BTreeSet<SymbolId>,
}

impl Handles {
    /// The `cleared` value for the timer call at `call`.
    pub fn cleared(&self, call: Span) -> Option<bool> {
        match self.uses.get(&(call.start, call.end)) {
            Some(Use::Discarded) => Some(false),
            Some(Use::Stored(sym)) => {
                if self.cleared.contains(sym) {
                    Some(true)
                } else if self.escaping.contains(sym) {
                    None
                } else {
                    Some(false)
                }
            }
            None => None,
        }
    }
}

pub fn analyze_handles(program: &oxc_ast::ast::Program<'_>, scoping: &Scoping) -> Handles {
    let mut v = Collect {
        scoping,
        uses: BTreeMap::new(),
        cleared: BTreeSet::new(),
        benign: BTreeSet::new(),
    };
    v.visit_program(program);

    // A stored handle escapes if it is read anywhere other than a clear or a benign method.
    let mut escaping = BTreeSet::new();
    for u in v.uses.values() {
        if let Use::Stored(sym) = u {
            let leaks = scoping.get_resolved_reference_ids(*sym).iter().any(|rid| {
                let r = scoping.get_reference(*rid);
                r.is_read() && !v.benign.contains(rid)
            });
            if leaks {
                escaping.insert(*sym);
            }
        }
    }
    Handles { uses: v.uses, cleared: v.cleared, escaping }
}

fn key(sp: Span) -> (u32, u32) {
    (sp.start, sp.end)
}

fn unparen<'x, 'a>(mut e: &'x Expression<'a>) -> &'x Expression<'a> {
    while let Expression::ParenthesizedExpression(p) = e {
        e = &p.expression;
    }
    e
}

struct Collect<'s> {
    scoping: &'s Scoping,
    uses: BTreeMap<(u32, u32), Use>,
    cleared: BTreeSet<SymbolId>,
    /// References that are a clear's argument or the object of a benign method.
    benign: BTreeSet<ReferenceId>,
}

impl Collect<'_> {
    fn symbol(&self, id: &IdentifierReference<'_>) -> Option<(SymbolId, ReferenceId)> {
        let rid = id.reference_id.get()?;
        let sym = self.scoping.get_reference(rid).symbol_id()?;
        Some((sym, rid))
    }

    fn is_unshadowed_global(&self, id: &IdentifierReference<'_>) -> bool {
        match id.reference_id.get() {
            Some(rid) => self.scoping.get_reference(rid).symbol_id().is_none(),
            None => true,
        }
    }

    fn store(&mut self, value: &Expression<'_>, sym: Option<SymbolId>) {
        if let Expression::CallExpression(c) = unparen(value) {
            if let Some(sym) = sym {
                self.uses.insert(key(c.span), Use::Stored(sym));
            }
        }
    }
}

impl<'a> Visit<'a> for Collect<'_> {
    fn visit_expression_statement(&mut self, it: &ExpressionStatement<'a>) {
        if let Expression::CallExpression(c) = unparen(&it.expression) {
            // `setInterval(..).unref();` still throws the handle away.
            let inner = match &c.callee {
                Expression::StaticMemberExpression(m)
                    if BENIGN.contains(&m.property.name.as_str()) =>
                {
                    match unparen(&m.object) {
                        Expression::CallExpression(t) => Some(t.span),
                        _ => None,
                    }
                }
                _ => None,
            };
            self.uses.insert(key(inner.unwrap_or(c.span)), Use::Discarded);
        }
        walk::walk_expression_statement(self, it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let (BindingPattern::BindingIdentifier(id), Some(init)) = (&it.id, &it.init) {
            self.store(init, id.symbol_id.get());
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        if let AssignmentTarget::AssignmentTargetIdentifier(id) = &it.left {
            let sym = self.symbol(id).map(|(s, _)| s);
            self.store(&it.right, sym);
        }
        walk::walk_assignment_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let named_clear = match &it.callee {
            Expression::Identifier(id) => {
                CLEARS.contains(&id.name.as_str()) && self.is_unshadowed_global(id)
            }
            Expression::StaticMemberExpression(m) => CLEARS.contains(&m.property.name.as_str()),
            _ => false,
        };
        if named_clear {
            if let Some(Argument::Identifier(id)) = it.arguments.first() {
                if let Some((sym, rid)) = self.symbol(id) {
                    self.cleared.insert(sym);
                    self.benign.insert(rid);
                }
            }
        }
        // `h.unref()` and friends read `h` without leaking it.
        if let Expression::StaticMemberExpression(m) = &it.callee {
            if BENIGN.contains(&m.property.name.as_str()) {
                if let Expression::Identifier(id) = unparen(&m.object) {
                    if let Some((_, rid)) = self.symbol(id) {
                        self.benign.insert(rid);
                    }
                }
            }
        }
        walk::walk_call_expression(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;

    /// `cleared` for every call to `T(...)` in the source, in order.
    fn cleared(src: &str) -> Vec<Option<bool>> {
        let alloc = Allocator::default();
        let ret = Parser::new(&alloc, src, SourceType::mjs()).parse();
        assert!(ret.diagnostics.is_empty(), "{:?}", ret.diagnostics);
        let sem = SemanticBuilder::new().build(&ret.program);
        let h = analyze_handles(&ret.program, sem.semantic.scoping());

        struct Calls<'h>(&'h Handles, Vec<Option<bool>>);
        impl<'a, 'h> Visit<'a> for Calls<'h> {
            fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
                if let Expression::Identifier(id) = &it.callee {
                    if id.name == "T" {
                        self.1.push(self.0.cleared(it.span));
                    }
                }
                walk::walk_call_expression(self, it);
            }
        }
        let mut c = Calls(&h, Vec::new());
        c.visit_program(&ret.program);
        c.1
    }

    #[test]
    fn a_discarded_handle_is_not_cleared() {
        assert_eq!(cleared("T(f, 1);"), vec![Some(false)]);
    }

    #[test]
    fn a_handle_cleared_by_its_variable_is_cleared() {
        assert_eq!(cleared("const h = T(f, 1); clearTimeout(h);"), vec![Some(true)]);
        assert_eq!(cleared("let h; h = T(f, 1); clearInterval(h);"), vec![Some(true)]);
    }

    #[test]
    fn a_clear_may_come_before_the_timer_in_the_file() {
        assert_eq!(
            cleared("function stop() { clearTimeout(h); } const h = T(f, 1);"),
            vec![Some(true)]
        );
    }

    #[test]
    fn a_stored_handle_that_is_never_used_is_not_cleared() {
        assert_eq!(cleared("const h = T(f, 1);"), vec![Some(false)]);
    }

    #[test]
    fn unref_and_ref_do_not_leak_the_handle() {
        assert_eq!(cleared("const h = T(f, 1); h.unref();"), vec![Some(false)]);
        assert_eq!(cleared("T(f, 1).unref();"), vec![Some(false)]);
    }

    #[test]
    fn a_handle_passed_on_or_returned_escapes() {
        assert_eq!(cleared("const h = T(f, 1); register(h);"), vec![None]);
        assert_eq!(cleared("function g() { const h = T(f, 1); return h; }"), vec![None]);
        assert_eq!(cleared("function g() { return T(f, 1); }"), vec![None]);
        assert_eq!(cleared("const o = { h: T(f, 1) };"), vec![None]);
        assert_eq!(cleared("const h = T(f, 1); store.push(h);"), vec![None]);
    }

    #[test]
    fn a_handle_stored_on_a_property_escapes() {
        assert_eq!(cleared("this.h = T(f, 1);"), vec![None]);
    }

    #[test]
    fn a_clear_on_another_object_is_accepted_by_name() {
        assert_eq!(cleared("const h = T(f, 1); timers.clearTimeout(h);"), vec![Some(true)]);
    }

    #[test]
    fn a_shadowed_clear_function_is_not_the_global() {
        // `clearTimeout` here is a parameter, not the global: it does not count as a clear,
        // and `h` is read by it, so the handle is reported as escaping.
        assert_eq!(
            cleared("function g(clearTimeout) { const h = T(f, 1); clearTimeout(h); }"),
            vec![None]
        );
    }

    #[test]
    fn a_clear_of_a_different_handle_does_not_count() {
        assert_eq!(
            cleared("const a = T(f, 1); const b = T(f, 2); clearTimeout(b);"),
            vec![Some(false), Some(true)]
        );
    }

    #[test]
    fn two_timers_in_one_variable_are_both_cleared_by_one_clear() {
        assert_eq!(
            cleared("let h = T(f, 1); h = T(f, 2); clearTimeout(h);"),
            vec![Some(true), Some(true)]
        );
    }

    #[test]
    fn a_handle_inside_another_expression_is_unknown() {
        assert_eq!(cleared("const h = (T(f, 1), 5);"), vec![None]);
    }
}
