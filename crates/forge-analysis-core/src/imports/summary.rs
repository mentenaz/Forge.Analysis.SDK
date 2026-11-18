//! Turns the call graph plus the facts into one `FunctionSummary` per function, and records
//! on every fact which function it is inside.
//!
//! Pure: the caller supplies how to turn an Oxc span into a schema span and into text, so
//! this file knows nothing about `Ctx`.

use crate::imports::callgraph::Graph;
use crate::imports::exports::Extracted;
use crate::imports::link::{ExportName, ExtCall, LinkInfo, LocalEdges};
use forge_analysis_schema::{
    Effect, EffectUse, FactKind, FnId, FunctionSummary, Lane, Quoted, Span, StaticFact,
    UnresolvedReason,
};
use std::collections::{BTreeMap, BTreeSet};

pub fn summarize(
    graph: &Graph,
    facts: &mut [StaticFact],
    span_of: &dyn Fn(oxc_span::Span) -> Span,
    text_of: &dyn Fn(oxc_span::Span) -> String,
) -> Vec<FunctionSummary> {
    let n = graph.fns.len();
    let mut direct: Vec<Vec<EffectUse>> = vec![Vec::new(); n];
    let mut unresolved: Vec<BTreeSet<UnresolvedReason>> = vec![BTreeSet::new(); n];
    let mut timer_lane: BTreeMap<(u32, u32), Lane> = BTreeMap::new();

    for f in facts.iter_mut() {
        let at = oxc_span::Span::new(f.at().start_byte, f.at().end_byte);
        let owner = graph.owner(at);
        f.within = Some(FnId(owner));
        let i = owner as usize;
        match f.kind() {
            FactKind::Effect => {
                if let Some(effect) = f.effect {
                    direct[i].push(EffectUse { effect, fact: f.id().clone() });
                }
            }
            FactKind::Timer => {
                if let Some(t) = f.timing {
                    timer_lane.insert((f.at().start_byte, f.at().end_byte), t.lane);
                }
            }
            FactKind::Unresolved => {
                if let Some(r) = f.unresolved {
                    unresolved[i].insert(r);
                }
            }
            _ => {}
        }
    }

    let direct_sets: Vec<BTreeSet<Effect>> =
        direct.iter().map(|d| d.iter().map(|u| u.effect).collect()).collect();
    let transitive = graph.roll_up(&direct_sets);
    let lanes = graph.lanes(Lane::Immediate, &timer_lane);

    let mut calls: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); n];
    let mut callbacks: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); n];
    let mut called_from: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); n];
    for &(a, b) in &graph.calls {
        calls[a as usize].insert(b);
        called_from[b as usize].insert(a);
    }
    for c in &graph.callbacks {
        callbacks[c.from as usize].insert(c.to);
        called_from[c.to as usize].insert(c.from);
    }

    let ids = |s: &BTreeSet<u32>| s.iter().map(|&x| FnId(x)).collect::<Vec<_>>();
    graph
        .fns
        .iter()
        .enumerate()
        .map(|(i, f)| FunctionSummary {
            id: FnId(f.id),
            name: f.name.map(|sp| Quoted::new(&text_of(sp))),
            at: span_of(f.at),
            direct_effects: std::mem::take(&mut direct[i]),
            transitive_effects: transitive[i].iter().copied().collect(),
            unresolved: unresolved[i].iter().copied().collect(),
            calls: ids(&calls[i]),
            callbacks: ids(&callbacks[i]),
            called_from: ids(&called_from[i]),
            lanes: lanes[i].iter().copied().collect(),
            reachable_from_entry: !lanes[i].is_empty(),
            unfollowed_calls: f.unfollowed_calls,
            escapes: graph.escaped.contains(&f.id),
        })
        .collect()
}

/// How a local name is bound to a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bound {
    /// `import x from "m"`
    Default,
    /// `import { a as x } from "m"`, `const { a: x } = require("m")`
    Named(String),
    /// `import * as x from "m"`, `const x = require("m")`
    Namespace,
}

/// Everything the project linker needs from one file. `bind` maps the declaration offset of a
/// local name to the module it was imported from and how.
pub fn link_info(
    graph: &Graph,
    facts: &[StaticFact],
    extracted: Extracted,
    bind: &dyn Fn(u32) -> Option<(String, Bound)>,
    text_of: &dyn Fn(oxc_span::Span) -> String,
) -> LinkInfo {
    let mut timer_lane: BTreeMap<(u32, u32), Lane> = BTreeMap::new();
    let mut direct: Vec<BTreeSet<Effect>> = vec![BTreeSet::new(); graph.fns.len()];
    for f in facts {
        match f.kind() {
            FactKind::Timer => {
                if let Some(t) = f.timing {
                    timer_lane.insert((f.at().start_byte, f.at().end_byte), t.lane);
                }
            }
            FactKind::Effect => {
                if let Some(e) = f.effect {
                    let at = oxc_span::Span::new(f.at().start_byte, f.at().end_byte);
                    direct[graph.owner(at) as usize].insert(e);
                }
            }
            _ => {}
        }
    }
    let edges = LocalEdges {
        calls: graph.calls.clone(),
        callbacks: graph
            .callbacks
            .iter()
            .map(|c| (c.from, c.to, timer_lane.get(&c.call).copied()))
            .collect(),
    };
    let mut ext_calls = extracted.ext_calls;
    for raw in &graph.ext_calls {
        let Some((spec, bound)) = bind(raw.decl_start) else { continue };
        let name = match (&raw.member, bound) {
            (None, Bound::Default) | (None, Bound::Namespace) => ExportName::Default,
            (None, Bound::Named(n)) => ExportName::Named(n),
            (Some(m), Bound::Namespace) => ExportName::Named(text_of(*m)),
            // `d.f()` on a default or named import: we do not know the shape of `d`.
            (Some(_), _) => continue,
        };
        ext_calls.push(ExtCall { from: raw.from, spec, name });
    }
    LinkInfo { exports: extracted.exports, imports: extracted.imports, ext_calls, edges, direct }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imports::callgraph;
    use forge_analysis_schema::{Delay, RelPath, Timing};
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;

    fn rel() -> RelPath {
        RelPath::new("a.js").expect("path")
    }

    fn sp(src: &str, s: oxc_span::Span) -> Span {
        let line = |o: u32| src[..o as usize].matches('\n').count() as u32 + 1;
        Span { file: rel(), start_line: line(s.start), end_line: line(s.end), start_byte: s.start, end_byte: s.end }
    }

    /// Facts are placed by the text they sit on: `DEL` is an FsDelete effect, `TIMER(` ...`)` a
    /// deferred timer call, `BAD` an unresolved ComputedAccess.
    fn run(src: &str) -> (Vec<FunctionSummary>, Vec<StaticFact>) {
        let alloc = Allocator::default();
        let ret = Parser::new(&alloc, src, SourceType::mjs()).parse();
        assert!(ret.diagnostics.is_empty());
        let sem = SemanticBuilder::new().build(&ret.program);
        let g = callgraph::build(&ret.program, sem.semantic.scoping());
        let mut facts = Vec::new();
        let mut find = |needle: &str, f: &dyn Fn(oxc_span::Span) -> StaticFact| {
            let mut from = 0;
            while let Some(i) = src[from..].find(needle) {
                let s = (from + i) as u32;
                facts.push(f(oxc_span::Span::new(s, s + needle.len() as u32)));
                from += i + needle.len();
            }
        };
        find("DEL", &|s| {
            let mut f = StaticFact::new(FactKind::Effect, sp(src, s), None);
            f.effect = Some(Effect::FsDelete);
            f
        });
        find("BAD", &|s| {
            let mut f = StaticFact::new(FactKind::Unresolved, sp(src, s), None);
            f.unresolved = Some(UnresolvedReason::ComputedAccess);
            f
        });
        // a timer fact covers the whole call expression, like the real matcher's
        let mut from = 0;
        while let Some(i) = src[from..].find("setTimeout(") {
            let s = from + i;
            let mut depth = 0;
            let mut e = s;
            for (k, c) in src[s..].char_indices() {
                if c == '(' { depth += 1 }
                if c == ')' { depth -= 1; if depth == 0 { e = s + k + 1; break } }
            }
            let mut f = StaticFact::new(FactKind::Timer, sp(src, oxc_span::Span::new(s as u32, e as u32)), None);
            f.timing = Some(Timing { lane: Lane::Deferred, delay_ms: Delay::Known(5), cleared: None });
            facts.push(f);
            from = e;
        }
        let out = summarize(
            &g,
            &mut facts,
            &|s| sp(src, s),
            &|s| src[s.start as usize..s.end as usize].to_string(),
        );
        for s in &out {
            assert!(s.validate().is_empty(), "{:?}", s.validate());
        }
        (out, facts)
    }

    fn by_name<'a>(v: &'a [FunctionSummary], n: &str) -> &'a FunctionSummary {
        v.iter().find(|s| s.name.as_ref().is_some_and(|q| q.text() == n)).expect("named fn")
    }

    #[test]
    fn a_fact_knows_its_function() {
        let (s, facts) = run("function a(){ DEL; } a();");
        let a = by_name(&s, "a");
        assert_eq!(a.direct_effects.len(), 1);
        assert_eq!(facts[0].within, Some(a.id));
    }

    #[test]
    fn top_level_facts_belong_to_function_zero() {
        let (s, facts) = run("DEL;");
        assert_eq!(facts[0].within, Some(FnId(0)));
        assert_eq!(s[0].direct_effects.len(), 1);
    }

    #[test]
    fn effects_roll_up_to_callers_but_stay_direct_only_where_they_happen() {
        let (s, _) = run("function a(){ b(); } function b(){ DEL; } a();");
        assert!(by_name(&s, "a").direct_effects.is_empty());
        assert_eq!(by_name(&s, "a").transitive_effects, vec![Effect::FsDelete]);
        assert_eq!(s[0].transitive_effects, vec![Effect::FsDelete]);
    }

    #[test]
    fn an_effect_in_a_timer_callback_runs_deferred_not_immediate() {
        let (s, _) = run("setTimeout(() => { DEL; }, 5);");
        let cb = &s[1];
        assert_eq!(cb.lanes, vec![Lane::Deferred]);
        assert_eq!(cb.direct_effects.len(), 1);
        // the top level still reaches it, so the roll-up includes it
        assert_eq!(s[0].transitive_effects, vec![Effect::FsDelete]);
        assert!(s[0].direct_effects.is_empty());
    }

    #[test]
    fn a_function_nothing_calls_is_not_reachable_but_still_listed() {
        let (s, _) = run("function lonely(){ DEL; }");
        let l = by_name(&s, "lonely");
        assert!(!l.reachable_from_entry);
        assert!(l.lanes.is_empty());
        assert_eq!(l.transitive_effects, vec![Effect::FsDelete]);
    }

    #[test]
    fn an_exported_function_is_flagged_as_escaping() {
        let (s, _) = run("export function run(){ DEL; }");
        assert!(by_name(&s, "run").escapes);
    }

    #[test]
    fn unresolved_reasons_are_per_function() {
        let (s, _) = run("function a(){ BAD; } a();");
        assert_eq!(by_name(&s, "a").unresolved, vec![UnresolvedReason::ComputedAccess]);
        assert!(s[0].unresolved.is_empty());
    }

    #[test]
    fn called_from_is_the_reverse_of_calls() {
        let (s, _) = run("function a(){} a();");
        assert_eq!(by_name(&s, "a").called_from, vec![FnId(0)]);
        assert_eq!(s[0].calls, vec![by_name(&s, "a").id]);
    }

    #[test]
    fn unfollowed_calls_are_counted_per_function() {
        let (s, _) = run("function a(){ x.y(); z(); } a();");
        assert_eq!(by_name(&s, "a").unfollowed_calls, 2);
    }
}
