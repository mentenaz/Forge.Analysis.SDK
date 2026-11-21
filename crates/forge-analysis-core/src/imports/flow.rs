//! Flow check: does data that came from the network reach the code argument of a sink?
//!
//! This is a deliberate over-approximation inside ONE file and tracked by Oxc symbols, not
//! names. It answers "network data MAY reach this sink", never "this is malicious".
//!
//! What it follows:
//!   * a symbol initialised or assigned from an expression that contains a source call or a
//!     tainted symbol (concatenation, templates, `await`, member access and method calls on a
//!     tainted value all count);
//!   * the parameters of a callback handed to a source call, or to a method called on a
//!     tainted value (`https.get(u, res => ..)`, `res.on("data", d => ..)`, `p.then(t => ..)`),
//!     including a callback passed by name;
//!   * `x.onmessage = e => ..` where `x` is tainted;
//!   * destructuring.
//!
//! A second kind of hit: the thing CALLED is network data (`const r = await fetch(u);
//! r.scan()`, `fn()` where `fn` came from a response). Calling a method every response,
//! string, array, stream or promise has (`json`, `text`, `on`, `map`, `then`, ..) is not a hit;
//! any other name is, because the server decides what it runs. A remote object that happens to
//! use one of the standard names is the known gap; those calls stay in the named untraced list.
//!
//! What it does NOT follow (reported in the coverage line, never silently dropped):
//!   * flows across function calls, across files, through object properties or arrays used
//!     as storage, or through storage (download, write to disk, load later).
//!
//! The matcher decides what a source and a sink are (from the catalog) and hands this module
//! a map from call/new span to `Site`. This module knows nothing about the catalog.

use oxc_ast::ast::{
    Argument, AssignmentExpression, AssignmentTarget, BindingPattern,
    CallExpression, Expression, FormalParameters, Function, IdentifierReference, NewExpression,
    VariableDeclarator,
};
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::{Scoping, ScopeFlags, SymbolId};
use oxc_span::Span;
use std::collections::BTreeMap;

/// What the matcher knows about one call or `new` expression.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Site {
    /// Its result (or its callbacks' arguments) is network data.
    pub source: bool,
    /// Its arguments are executed as code or as a command.
    pub sink: bool,
    /// When set, only this argument is the executed one (`setTimeout`'s callback); the
    /// other arguments (the delay) are not.
    pub arg: Option<u8>,
}

/// Network data may reach `sink`; `origin` is where the data entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowHit {
    pub sink: Span,
    pub origin: Span,
    /// The data is not an argument but the thing being called: `r.scan()` where `r` holds
    /// network data. What runs is decided by whoever answered the request.
    pub callee: bool,
}

type Sites = BTreeMap<(u32, u32), Site>;
type Tainted = BTreeMap<SymbolId, Span>;
type FnParams = BTreeMap<SymbolId, Vec<SymbolId>>;

/// Hard stop for the propagation loop. It always terminates (the tainted set only grows and
/// is bounded by the number of symbols); this just bounds the work on adversarial input.
const MAX_ROUNDS: usize = 64;

pub fn find_flows(
    program: &oxc_ast::ast::Program<'_>,
    scoping: &Scoping,
    sites: &Sites,
) -> Vec<FlowHit> {
    // No sources means no flows (a call on network data needs a source too): skip all the work.
    if !sites.values().any(|s| s.source) {
        return Vec::new();
    }

    let mut collect = FnCollector { params: FnParams::new() };
    collect.visit_program(program);
    let fn_params = collect.params;

    let mut tainted = Tainted::new();
    for _ in 0..MAX_ROUNDS {
        let mut p = Propagate {
            scoping,
            sites,
            fn_params: &fn_params,
            tainted: std::mem::take(&mut tainted),
            changed: false,
        };
        p.visit_program(program);
        tainted = p.tainted;
        if !p.changed {
            break;
        }
    }

    let mut finder = SinkFinder { scoping, sites, tainted: &tainted, hits: Vec::new() };
    finder.visit_program(program);
    finder.hits.sort_by_key(|h| (h.sink.start, h.sink.end, h.callee));
    finder.hits
}

// ---- helpers ----

fn key(sp: Span) -> (u32, u32) {
    (sp.start, sp.end)
}

fn symbol_of(scoping: &Scoping, id: &IdentifierReference<'_>) -> Option<SymbolId> {
    let rid = id.reference_id.get()?;
    scoping.get_reference(rid).symbol_id()
}

fn collect_binding(p: &BindingPattern<'_>, out: &mut Vec<SymbolId>) {
    match p {
        BindingPattern::BindingIdentifier(id) => {
            if let Some(s) = id.symbol_id.get() {
                out.push(s);
            }
        }
        BindingPattern::ObjectPattern(o) => {
            for prop in &o.properties {
                collect_binding(&prop.value, out);
            }
            if let Some(rest) = &o.rest {
                collect_binding(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(a) => {
            for el in a.elements.iter().flatten() {
                collect_binding(el, out);
            }
            if let Some(rest) = &a.rest {
                collect_binding(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(a) => collect_binding(&a.left, out),
    }
}

fn param_symbols(params: &FormalParameters<'_>) -> Vec<SymbolId> {
    let mut out = Vec::new();
    for item in &params.items {
        collect_binding(&item.pattern, &mut out);
    }
    if let Some(rest) = &params.rest {
        collect_binding(&rest.rest.argument, &mut out);
    }
    out
}

/// The identifier at the root of `a`, `a.b`, `a[b]`, `(a).b`.
fn root_ident<'x, 'a>(mut e: &'x Expression<'a>) -> Option<&'x IdentifierReference<'a>> {
    loop {
        match e {
            Expression::Identifier(id) => return Some(&**id),
            Expression::StaticMemberExpression(m) => e = &m.object,
            Expression::ComputedMemberExpression(m) => e = &m.object,
            Expression::ParenthesizedExpression(p) => e = &p.expression,
            _ => return None,
        }
    }
}

fn target_root<'x, 'a>(t: &'x AssignmentTarget<'a>) -> Option<&'x IdentifierReference<'a>> {
    match t {
        AssignmentTarget::AssignmentTargetIdentifier(id) => Some(&**id),
        AssignmentTarget::StaticMemberExpression(m) => root_ident(&m.object),
        AssignmentTarget::ComputedMemberExpression(m) => root_ident(&m.object),
        _ => None,
    }
}

/// Parameters of a function literal, seeing through parentheses.
fn literal_params<'x, 'a>(e: &'x Expression<'a>) -> Option<&'x FormalParameters<'a>> {
    match e {
        Expression::ArrowFunctionExpression(f) => Some(&f.params),
        Expression::FunctionExpression(f) => Some(&f.params),
        Expression::ParenthesizedExpression(p) => literal_params(&p.expression),
        _ => None,
    }
}

// ---- pre-pass: which names stand for which function's parameters ----

struct FnCollector {
    params: FnParams,
}

impl<'a> Visit<'a> for FnCollector {
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        if let Some(sym) = it.id.as_ref().and_then(|id| id.symbol_id.get()) {
            self.params.insert(sym, param_symbols(&it.params));
        }
        walk::walk_function(self, it, flags);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let (BindingPattern::BindingIdentifier(id), Some(init)) = (&it.id, &it.init) {
            if let (Some(sym), Some(params)) = (id.symbol_id.get(), literal_params(init)) {
                self.params.insert(sym, param_symbols(params));
            }
        }
        walk::walk_variable_declarator(self, it);
    }
}

// ---- "does this expression carry network data, and where from?" ----

struct Mentions<'m> {
    scoping: &'m Scoping,
    sites: &'m Sites,
    tainted: &'m Tainted,
    found: Option<Span>,
}

impl<'a> Visit<'a> for Mentions<'_> {
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if self.found.is_some() {
            return;
        }
        if let Some(sym) = symbol_of(self.scoping, it) {
            if let Some(origin) = self.tainted.get(&sym) {
                self.found = Some(*origin);
            }
        }
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if self.found.is_some() {
            return;
        }
        if self.sites.get(&key(it.span)).is_some_and(|s| s.source) {
            self.found = Some(it.span);
            return;
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        if self.found.is_some() {
            return;
        }
        if self.sites.get(&key(it.span)).is_some_and(|s| s.source) {
            self.found = Some(it.span);
            return;
        }
        walk::walk_new_expression(self, it);
    }
}

fn origin_of_expr(
    scoping: &Scoping,
    sites: &Sites,
    tainted: &Tainted,
    e: &Expression<'_>,
) -> Option<Span> {
    let mut m = Mentions { scoping, sites, tainted, found: None };
    m.visit_expression(e);
    m.found
}

fn origin_of_arg(
    scoping: &Scoping,
    sites: &Sites,
    tainted: &Tainted,
    a: &Argument<'_>,
) -> Option<Span> {
    let mut m = Mentions { scoping, sites, tainted, found: None };
    m.visit_argument(a);
    m.found
}

// ---- propagation, repeated until nothing new is tainted ----

struct Propagate<'s> {
    scoping: &'s Scoping,
    sites: &'s Sites,
    fn_params: &'s FnParams,
    tainted: Tainted,
    changed: bool,
}

impl Propagate<'_> {
    fn origin(&self, e: &Expression<'_>) -> Option<Span> {
        origin_of_expr(self.scoping, self.sites, &self.tainted, e)
    }

    fn taint(&mut self, sym: SymbolId, origin: Span) {
        if !self.tainted.contains_key(&sym) {
            self.tainted.insert(sym, origin);
            self.changed = true;
        }
    }

    fn taint_all(&mut self, syms: Vec<SymbolId>, origin: Span) {
        for s in syms {
            self.taint(s, origin);
        }
    }

    /// The parameters of a callback argument: a function literal, or a function named here.
    fn taint_callback_arg(&mut self, arg: &Argument<'_>, origin: Span) {
        let syms = match arg {
            Argument::ArrowFunctionExpression(f) => param_symbols(&f.params),
            Argument::FunctionExpression(f) => param_symbols(&f.params),
            Argument::Identifier(id) => symbol_of(self.scoping, id)
                .and_then(|s| self.fn_params.get(&s).cloned())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        self.taint_all(syms, origin);
    }
}

impl<'a> Visit<'a> for Propagate<'_> {
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let Some(init) = &it.init {
            if let Some(origin) = self.origin(init) {
                let mut syms = Vec::new();
                collect_binding(&it.id, &mut syms);
                self.taint_all(syms, origin);
            }
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        // `a = <tainted>` and `a.b = <tainted>` taint `a`.
        if let Some(origin) = self.origin(&it.right) {
            if let Some(sym) = target_root(&it.left).and_then(|id| symbol_of(self.scoping, id)) {
                self.taint(sym, origin);
            }
        }
        // `x.onmessage = e => ..` where `x` is tainted: the handler's parameters are tainted.
        if let Some(params) = literal_params(&it.right) {
            let owner = target_root(&it.left)
                .and_then(|id| symbol_of(self.scoping, id))
                .and_then(|s| self.tainted.get(&s).copied());
            if let Some(origin) = owner {
                let syms = param_symbols(params);
                self.taint_all(syms, origin);
            }
        }
        walk::walk_assignment_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        // A source call, or a method called on tainted data, hands tainted values to callbacks.
        let carrier = if self.sites.get(&key(it.span)).is_some_and(|s| s.source) {
            Some(it.span)
        } else {
            self.origin(&it.callee)
        };
        if let Some(origin) = carrier {
            for arg in &it.arguments {
                self.taint_callback_arg(arg, origin);
            }
        }
        walk::walk_call_expression(self, it);
    }
}

// ---- finding the sinks that receive tainted data ----

struct SinkFinder<'s> {
    scoping: &'s Scoping,
    sites: &'s Sites,
    tainted: &'s Tainted,
    hits: Vec<FlowHit>,
}

impl SinkFinder<'_> {
    fn check(&mut self, at: Span, args: &[Argument<'_>]) {
        let Some(site) = self.sites.get(&key(at)).filter(|s| s.sink) else { return };
        for (i, a) in args.iter().enumerate() {
            if site.arg.is_some_and(|only| usize::from(only) != i) {
                continue;
            }
            if let Some(origin) = origin_of_arg(self.scoping, self.sites, self.tainted, a) {
                self.hits.push(FlowHit { sink: at, origin, callee: false });
                return;
            }
        }
    }
}

impl SinkFinder<'_> {
    /// Is the function being called network data, or looked up on network data?
    fn check_callee(&mut self, it: &CallExpression<'_>) {
        let mut callee = &it.callee;
        while let Expression::ParenthesizedExpression(p) = callee {
            callee = &p.expression;
        }
        let origin = match callee {
            Expression::Identifier(id) => {
                symbol_of(self.scoping, id).and_then(|s| self.tainted.get(&s).copied())
            }
            Expression::StaticMemberExpression(m) => {
                if is_standard_method(m.property.name.as_str()) {
                    None
                } else {
                    origin_of_expr(self.scoping, self.sites, self.tainted, &m.object)
                }
            }
            Expression::ComputedMemberExpression(m) => {
                origin_of_expr(self.scoping, self.sites, self.tainted, &m.object)
            }
            // `new Function(d)()` and `eval(t)()`: the sink itself is already reported.
            Expression::NewExpression(n) if self.sites.get(&key(n.span)).is_some_and(|x| x.sink) => None,
            Expression::CallExpression(c) if self.sites.get(&key(c.span)).is_some_and(|x| x.sink) => None,
            other => origin_of_expr(self.scoping, self.sites, self.tainted, other),
        };
        if let Some(origin) = origin {
            self.hits.push(FlowHit { sink: it.span, origin, callee: true });
        }
    }
}

/// Methods that strings, arrays, objects, maps, promises, responses, headers, streams, event
/// emitters, sockets and requests all have. Calling one on network data is ordinary handling
/// of it, not the server choosing what runs.
fn is_standard_method(name: &str) -> bool {
    const STANDARD: &[&str] = &[
        // strings
        "at", "charAt", "charCodeAt", "codePointAt", "concat", "endsWith", "includes", "indexOf",
        "lastIndexOf", "localeCompare", "match", "matchAll", "normalize", "padEnd", "padStart",
        "repeat", "replace", "replaceAll", "search", "slice", "split", "startsWith", "substring",
        "substr", "toLowerCase", "toUpperCase", "toLocaleLowerCase", "toLocaleUpperCase", "trim",
        "trimStart", "trimEnd",
        // arrays
        "map", "filter", "forEach", "reduce", "reduceRight", "find", "findIndex", "findLast",
        "findLastIndex", "some", "every", "flat", "flatMap", "join", "push", "pop", "shift",
        "unshift", "splice", "sort", "reverse", "fill", "keys", "values", "entries", "copyWithin",
        "toSorted", "toReversed", "toSpliced", "with",
        // every object
        "toString", "valueOf", "toJSON", "toLocaleString", "toFixed", "toPrecision",
        "hasOwnProperty", "isPrototypeOf", "propertyIsEnumerable",
        // maps, sets, headers
        "get", "set", "has", "add", "delete", "clear", "append",
        // promises
        "then", "catch", "finally",
        // fetch responses
        "json", "text", "blob", "arrayBuffer", "formData", "bytes", "clone",
        // streams, emitters, sockets, requests
        "on", "once", "off", "addListener", "removeListener", "removeAllListeners", "emit",
        "addEventListener", "removeEventListener", "pipe", "unpipe", "setEncoding", "setTimeout",
        "pause", "resume", "destroy", "end", "write", "abort", "close", "send", "read", "cancel",
        "getReader", "setHeader", "getHeader", "open", "setRequestHeader", "getResponseHeader",
        "getAllResponseHeaders",
    ];
    STANDARD.contains(&name)
}

impl<'a> Visit<'a> for SinkFinder<'_> {
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        self.check_callee(it);
        self.check(it.span, &it.arguments);
        walk::walk_call_expression(self, it);
    }

    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        self.check(it.span, &it.arguments);
        walk::walk_new_expression(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_ast::ast::Program;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::{GetSpan, SourceType};

    /// Stand-in for the real matcher: names decide sources and sinks.
    struct Stub<'s> {
        src: &'s str,
        sites: Sites,
    }

    impl Stub<'_> {
        fn callee_text(&self, e: &Expression<'_>) -> &str {
            let sp = e.span();
            &self.src[sp.start as usize..sp.end as usize]
        }
        fn classify(&self, callee: &str) -> Site {
            let source = ["fetch", "https.get", "https.request", "XMLHttpRequest", "WebSocket"]
                .contains(&callee);
            let sink = ["eval", "Function", "vm.runInThisContext", "exec", "spawn"]
                .contains(&callee);
            Site { source, sink, arg: None }
        }
    }

    impl<'a> Visit<'a> for Stub<'_> {
        fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
            let site = self.classify(self.callee_text(&it.callee));
            if site.source || site.sink {
                self.sites.insert(key(it.span), site);
            }
            walk::walk_call_expression(self, it);
        }
        fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
            let site = self.classify(self.callee_text(&it.callee));
            if site.source || site.sink {
                self.sites.insert(key(it.span), site);
            }
            walk::walk_new_expression(self, it);
        }
    }

    fn flows(src: &str) -> Vec<(String, String)> {
        let alloc = Allocator::default();
        let ret = Parser::new(&alloc, src, SourceType::mjs()).parse();
        assert!(ret.diagnostics.is_empty(), "parse: {:?}", ret.diagnostics);
        let program: &Program<'_> = &ret.program;
        let sem = SemanticBuilder::new().build(program);
        let mut stub = Stub { src, sites: Sites::new() };
        stub.visit_program(program);
        find_flows(program, sem.semantic.scoping(), &stub.sites)
            .into_iter()
            .filter(|h| !h.callee)
            .map(|h| {
                let t = |s: Span| src[s.start as usize..s.end as usize].to_string();
                (t(h.sink), t(h.origin))
            })
            .collect()
    }

    /// The calls whose callee is network data: (the call, where the data came from).
    fn callee_flows(src: &str) -> Vec<(String, String)> {
        let alloc = Allocator::default();
        let ret = Parser::new(&alloc, src, SourceType::mjs()).parse();
        assert!(ret.diagnostics.is_empty(), "parse: {:?}", ret.diagnostics);
        let program: &Program<'_> = &ret.program;
        let sem = SemanticBuilder::new().build(program);
        let mut stub = Stub { src, sites: Sites::new() };
        stub.visit_program(program);
        find_flows(program, sem.semantic.scoping(), &stub.sites)
            .into_iter()
            .filter(|h| h.callee)
            .map(|h| {
                let t = |s: Span| src[s.start as usize..s.end as usize].to_string();
                (t(h.sink), t(h.origin))
            })
            .collect()
    }

    #[test]
    fn a_method_called_on_network_data_is_a_callee_flow() {
        let f = callee_flows("const vuln = await fetch(server); await vuln.scan();");
        assert_eq!(f, vec![("vuln.scan()".to_string(), "fetch(server)".to_string())]);
    }

    #[test]
    fn handling_the_response_the_normal_way_is_not() {
        let f = callee_flows(
            "const r = await fetch(u); const t = await r.text(); const j = await r.json(); \
             t.trim().split(',').map(x => x.toString()); r.headers.get('a');",
        );
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn calling_a_function_that_came_from_the_response_is_a_callee_flow() {
        let f = callee_flows("const { run } = await (await fetch(u)).json(); run(1);");
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].0, "run(1)");
    }

    #[test]
    fn a_computed_method_on_network_data_is_a_callee_flow() {
        let f = callee_flows("const r = await fetch(u); r[name]();");
        assert_eq!(f.len(), 1, "{f:?}");
    }

    #[test]
    fn a_chain_after_network_data_reports_each_unknown_call() {
        let f = callee_flows("const r = await fetch(u); r.a().b();");
        assert_eq!(f.len(), 2, "{f:?}");
    }

    #[test]
    fn a_method_on_local_data_is_not_a_callee_flow() {
        let f = callee_flows("const r = {scan(){}}; r.scan(); const s = await fetch(u); other.scan();");
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn a_callback_parameter_of_a_network_call_may_be_called() {
        let f = callee_flows("https.get(u, res => { res.on('data', d => d.toString()); res.exec2(); });");
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].0, "res.exec2()");
    }

    fn one(src: &str) -> (String, String) {
        let f = flows(src);
        assert_eq!(f.len(), 1, "{f:?}");
        f.into_iter().next().unwrap()
    }

    #[test]
    fn fetch_text_into_eval() {
        let (sink, origin) = one("const r = await fetch(u); const t = await r.text(); eval(t);");
        assert_eq!(sink, "eval(t)");
        assert_eq!(origin, "fetch(u)");
    }

    #[test]
    fn nested_awaits_into_eval() {
        let (sink, _) = one("const code = await (await fetch(u)).text(); eval(code);");
        assert_eq!(sink, "eval(code)");
    }

    #[test]
    fn http_callback_data_into_new_function() {
        let (sink, origin) = one(
            r#"https.get(u, res => { res.on("data", d => { new Function(d)(); }); });"#,
        );
        assert!(sink.starts_with("new Function(d)"), "{sink}");
        assert!(origin.starts_with("https.get("), "{origin}");
    }

    #[test]
    fn promise_chain_into_vm() {
        let (sink, _) =
            one("fetch(u).then(r => r.text()).then(t => vm.runInThisContext(t));");
        assert_eq!(sink, "vm.runInThisContext(t)");
    }

    #[test]
    fn xhr_response_into_eval() {
        let (sink, origin) =
            one("const x = new XMLHttpRequest(); x.onload = () => { eval(x.responseText); };");
        assert_eq!(sink, "eval(x.responseText)");
        assert_eq!(origin, "new XMLHttpRequest()");
    }

    #[test]
    fn websocket_handler_into_eval() {
        let (sink, _) =
            one("const ws = new WebSocket(u); ws.onmessage = e => { eval(e.data); };");
        assert_eq!(sink, "eval(e.data)");
    }

    #[test]
    fn a_callback_passed_by_name_is_followed() {
        let (sink, _) = one(
            "https.get(u, handler); function handler(res) { res.on('data', d => eval(d)); }",
        );
        assert_eq!(sink, "eval(d)");
    }

    #[test]
    fn a_callback_held_in_a_variable_is_followed() {
        let (sink, _) = one("const h = res => eval(res); https.get(u, h);");
        assert_eq!(sink, "eval(res)");
    }

    #[test]
    fn concatenation_and_templates_still_count() {
        assert_eq!(flows("const t = await fetch(u); eval('x' + t);").len(), 1);
        assert_eq!(flows("const t = await fetch(u); eval(`x${t}`);").len(), 1);
    }

    #[test]
    fn a_tainted_value_in_the_argument_list_of_a_spawn_counts() {
        let (sink, _) = one("const t = await fetch(u); spawn('sh', ['-c', t]);");
        assert!(sink.starts_with("spawn("), "{sink}");
    }

    #[test]
    fn assignment_carries_taint() {
        let (sink, _) = one("let a; const t = await fetch(u); a = t; eval(a);");
        assert_eq!(sink, "eval(a)");
    }

    #[test]
    fn destructuring_carries_taint() {
        let (sink, _) =
            one("const r = await fetch(u); const { code } = await r.json(); eval(code);");
        assert_eq!(sink, "eval(code)");
    }

    #[test]
    fn a_chain_of_variables_carries_taint() {
        let (sink, _) = one("const a = await fetch(u); const b = a; const c = b; eval(c);");
        assert_eq!(sink, "eval(c)");
    }

    #[test]
    fn a_local_string_is_not_a_flow() {
        assert!(flows("const code = 'x'; eval(code); fetch(u);").is_empty());
    }

    #[test]
    fn a_source_with_no_sink_is_not_a_flow() {
        assert!(flows("const r = await fetch(u); console.log(r);").is_empty());
    }

    #[test]
    fn taint_is_per_symbol_not_per_name() {
        // `t` inside g is a different symbol from the callback's `t`.
        let f = flows("function g(t) { eval(t); } fetch(u).then(t => use(t));");
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn a_callback_of_untainted_data_is_not_tainted() {
        assert!(flows("[1].forEach(x => eval(x)); fetch(u);").is_empty());
    }

    #[test]
    fn two_sinks_two_hits_in_order() {
        let f = flows("const t = await fetch(u); eval(t); eval(t + 1);");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].0, "eval(t)");
        assert_eq!(f[1].0, "eval(t + 1)");
    }

    #[test]
    fn deep_variable_chains_terminate() {
        let mut s = String::from("const v0 = await fetch(u);");
        for i in 1..200 {
            s.push_str(&format!("const v{i} = v{};", i - 1));
        }
        s.push_str("eval(v199);");
        assert_eq!(flows(&s).len(), 1);
    }

    #[test]
    fn reverse_order_declarations_still_converge() {
        // The later statement is visited first in source order, so one pass is not enough.
        let (sink, _) = one(
            "function f() { eval(b); } let b; let a; function g() { b = a; } function h() { a = fetch(u); }",
        );
        assert_eq!(sink, "eval(b)");
    }

    #[test]
    fn a_site_with_one_executed_argument_ignores_the_others() {
        // setTimeout-style: argument 0 is executed, argument 1 (the delay) is not.
        let src = "const t = await fetch(u); T(f, t); T(t, 1);";
        let alloc = Allocator::default();
        let ret = Parser::new(&alloc, src, SourceType::mjs()).parse();
        let sem = SemanticBuilder::new().build(&ret.program);
        let mut stub = Stub { src, sites: Sites::new() };
        stub.visit_program(&ret.program);
        // Mark every T(...) call as a sink on argument 0 only.
        struct Mark<'s>(&'s str, Sites);
        impl<'a> Visit<'a> for Mark<'_> {
            fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
                let sp = it.callee.span();
                if &self.0[sp.start as usize..sp.end as usize] == "T" {
                    self.1.insert(key(it.span), Site { source: false, sink: true, arg: Some(0) });
                }
                walk::walk_call_expression(self, it);
            }
        }
        let mut mark = Mark(src, stub.sites.clone());
        mark.visit_program(&ret.program);
        let hits = find_flows(&ret.program, sem.semantic.scoping(), &mark.1);
        let sinks: Vec<_> =
            hits.iter().map(|h| &src[h.sink.start as usize..h.sink.end as usize]).collect();
        assert_eq!(sinks, vec!["T(t, 1)"]);
    }
}
