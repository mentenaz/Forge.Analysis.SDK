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
//! A function value is network data only through what it returns (`const load = () => fetch(u)`),
//! not because its body contains a request or a tainted name (`const get = e => { new
//! XMLHttpRequest().send() }`). A function called on the spot counts in full.
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

use forge_analysis_catalog::{is_platform_method, HandleKind};
use oxc_ast::ast::{
    Argument, ArrowFunctionBody, ArrowFunctionExpression, AssignmentExpression, ExpressionStatement, Statement, VariableDeclaration, AssignmentTarget, BindingPattern,
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
    /// For a source: what kind of network object it gives back, which decides the platform's own methods.
    pub handle: HandleKind,
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
/// What a tainted value IS, which decides whether calling something on it means "the server decides what
/// runs". The order is the strength: a symbol only moves up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// Carries network data (a string, parsed JSON, an object built from it, the result of a helper). Calling
    /// something on it is ordinary: JSON cannot hold a function, and an object that merely holds data runs
    /// your own code.
    Data,
    /// A function that hands back a `Handle` (`() => fetch(u)`): calling it gives a `Handle`.
    Maker,
    /// The network object itself: a response, a socket, a request. Its own methods are the platform's API;
    /// any other method name is not, so the one who answered the request chose what runs.
    Handle,
    /// The result of running network data as code (`eval`, `new Function`, `vm`) or of a call the server
    /// chose. Anything called on it is remote code.
    Code,
}

#[derive(Debug, Clone, Copy)]
struct Taint {
    origin: Span,
    kind: Kind,
    /// For a `Handle` (or a `Maker` of one): what kind of object, so which methods are the platform's.
    ty: HandleKind,
}

type Tainted = BTreeMap<SymbolId, Taint>;
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
    /// The next function expression met is called on the spot, so its body counts.
    run_now: bool,
    /// Inside a function that is not run on the spot: only what it returns counts.
    returns_only: bool,
}

impl<'a> Visit<'a> for Mentions<'_> {
    // A function value is network data only through what it RETURNS: `const load = () => fetch(u)` is, so
    // `eval(await load())` is a flow; `const get = e => { new XMLHttpRequest().send() }` is not, because
    // nothing it does inside is handed back. A function called on the spot counts in full.
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        if self.run_now {
            self.run_now = false;
            walk::walk_function(self, it, flags);
        } else if !self.returns_only {
            self.returns_only = true;
            walk::walk_function(self, it, flags);
            self.returns_only = false;
        }
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        if self.run_now {
            self.run_now = false;
            walk::walk_arrow_function_expression(self, it);
        } else if !self.returns_only {
            if let Some(value) = it.body.as_expression() {
                // `() => value`: the expression is what is returned.
                self.visit_expression(value);
            } else {
                self.returns_only = true;
                walk::walk_arrow_function_expression(self, it);
                self.returns_only = false;
            }
        }
    }

    // Inside a function that is not run on the spot, statements whose value is dropped hand nothing back.
    fn visit_expression_statement(&mut self, it: &ExpressionStatement<'a>) {
        if !self.returns_only {
            walk::walk_expression_statement(self, it);
        }
    }

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        if !self.returns_only {
            walk::walk_variable_declaration(self, it);
        }
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if self.found.is_some() {
            return;
        }
        if let Some(sym) = symbol_of(self.scoping, it) {
            if let Some(t) = self.tainted.get(&sym) {
                self.found = Some(t.origin);
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
        // A function expression that is the callee runs now, so what its body fetches is the value.
        let now = matches!(
            it.callee.without_parentheses(),
            Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_)
        );
        if now {
            let outer = std::mem::replace(&mut self.returns_only, false);
            self.run_now = true;
            self.visit_expression(&it.callee);
            self.run_now = false;
            self.returns_only = outer;
            for arg in &it.arguments {
                self.visit_argument(arg);
            }
        } else {
            walk::walk_call_expression(self, it);
        }
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
    let mut m = Mentions { scoping, sites, tainted, found: None, run_now: false, returns_only: false };
    m.visit_expression(e);
    m.found
}

fn origin_of_arg(
    scoping: &Scoping,
    sites: &Sites,
    tainted: &Tainted,
    a: &Argument<'_>,
) -> Option<Span> {
    let mut m = Mentions { scoping, sites, tainted, found: None, run_now: false, returns_only: false };
    m.visit_argument(a);
    m.found
}

// ---- what kind of tainted value is this expression? ----

/// Steps through the wrappers that do not change what a value is: parentheses, `await`, type casts.
fn strip<'x, 'a>(mut e: &'x Expression<'a>) -> &'x Expression<'a> {
    loop {
        e = match e {
            Expression::ParenthesizedExpression(p) => &p.expression,
            Expression::AwaitExpression(a) => &a.argument,
            Expression::TSAsExpression(a) => &a.expression,
            Expression::TSNonNullExpression(a) => &a.expression,
            Expression::TSSatisfiesExpression(a) => &a.expression,
            Expression::TSTypeAssertion(a) => &a.expression,
            _ => return e,
        }
    }
}

/// The values a function literal hands back: its expression body, or its `return` arguments (not those of
/// functions inside it).
fn returned<'x, 'a>(f: &'x Expression<'a>) -> Option<Vec<&'x Expression<'a>>> {
    let mut out = Vec::new();
    match f {
        Expression::ArrowFunctionExpression(a) => {
            if let Some(v) = a.body.as_expression() {
                out.push(v);
            } else if let ArrowFunctionBody::FunctionBody(b) = &a.body {
                collect_returns(&b.statements, &mut out);
            }
        }
        Expression::FunctionExpression(fe) => {
            if let Some(b) = &fe.body {
                collect_returns(&b.statements, &mut out);
            }
        }
        _ => return None,
    }
    Some(out)
}

fn collect_returns<'x, 'a>(stmts: &'x [Statement<'a>], out: &mut Vec<&'x Expression<'a>>) {
    for st in stmts {
        match st {
            Statement::ReturnStatement(r) => {
                if let Some(a) = &r.argument {
                    out.push(a);
                }
            }
            Statement::BlockStatement(b) => collect_returns(&b.body, out),
            Statement::IfStatement(i) => {
                collect_returns(std::slice::from_ref(&i.consequent), out);
                if let Some(alt) = &i.alternate {
                    collect_returns(std::slice::from_ref(alt), out);
                }
            }
            Statement::TryStatement(t) => {
                collect_returns(&t.block.body, out);
                if let Some(h) = &t.handler {
                    collect_returns(&h.body.body, out);
                }
                if let Some(f) = &t.finalizer {
                    collect_returns(&f.body, out);
                }
            }
            Statement::ForStatement(f) => collect_returns(std::slice::from_ref(&f.body), out),
            Statement::ForOfStatement(f) => collect_returns(std::slice::from_ref(&f.body), out),
            Statement::ForInStatement(f) => collect_returns(std::slice::from_ref(&f.body), out),
            Statement::WhileStatement(w) => collect_returns(std::slice::from_ref(&w.body), out),
            Statement::DoWhileStatement(w) => collect_returns(std::slice::from_ref(&w.body), out),
            Statement::LabeledStatement(l) => collect_returns(std::slice::from_ref(&l.body), out),
            Statement::SwitchStatement(sw) => {
                for c in &sw.cases {
                    collect_returns(&c.consequent, out);
                }
            }
            _ => {}
        }
    }
}

/// What the function literal `f` hands back, as a taint: `Maker` when it hands back a `Handle`, `Data` when it
/// hands back anything else tainted.
fn literal_returns(scoping: &Scoping, sites: &Sites, tainted: &Tainted, f: &Expression<'_>) -> Option<Taint> {
    let mut best: Option<Taint> = None;
    for r in returned(f)? {
        if let Some(t) = classify(scoping, sites, tainted, r) {
            let k = if t.kind >= Kind::Maker { Kind::Maker } else { Kind::Data };
            if best.map_or(true, |b| k > b.kind) {
                best = Some(Taint { origin: t.origin, kind: k, ty: t.ty });
            }
        }
    }
    best
}

/// A taint with no particular object type (data, code, or a handle whose type is not known).
fn plain(origin: Span, kind: Kind) -> Taint {
    Taint { origin, kind, ty: HandleKind::Unknown }
}

/// The network object a source call gives back.
fn handle_at(sites: &Sites, at: Span) -> Taint {
    Taint { origin: at, kind: Kind::Handle, ty: sites.get(&key(at)).map_or(HandleKind::Unknown, |s| s.handle) }
}

fn is_source(sites: &Sites, at: Span) -> bool {
    sites.get(&key(at)).is_some_and(|s| s.source)
}

fn is_sink(sites: &Sites, at: Span) -> bool {
    sites.get(&key(at)).is_some_and(|s| s.sink)
}

/// The kind of tainted value `e` is, or None when it holds no network data. The forms that keep a value's
/// identity (a source call, a name, `await`, a cast) keep its kind; a call on a `Handle` that is not the
/// platform's own API gives `Code`; running network data as code gives `Code`; everything else that mentions
/// network data is `Data`.
fn classify(scoping: &Scoping, sites: &Sites, tainted: &Tainted, e: &Expression<'_>) -> Option<Taint> {
    let e = strip(e);
    match e {
        Expression::Identifier(id) => symbol_of(scoping, id).and_then(|s| tainted.get(&s).copied()),
        Expression::NewExpression(n) if is_source(sites, n.span) => Some(handle_at(sites, n.span)),
        Expression::CallExpression(c) if is_source(sites, c.span) => Some(handle_at(sites, c.span)),
        Expression::NewExpression(n) if is_sink(sites, n.span) => n
            .arguments
            .iter()
            .find_map(|a| origin_of_arg(scoping, sites, tainted, a))
            .map(|origin| plain(origin, Kind::Code)),
        Expression::CallExpression(c) => {
            if is_sink(sites, c.span) {
                if let Some(origin) = c.arguments.iter().find_map(|a| origin_of_arg(scoping, sites, tainted, a)) {
                    return Some(plain(origin, Kind::Code));
                }
            }
            let callee = strip(&c.callee);
            // A function literal called on the spot gives what it returns.
            if let Some(t) = literal_returns(scoping, sites, tainted, callee) {
                return Some(Taint { origin: t.origin, kind: if t.kind == Kind::Maker { Kind::Handle } else { Kind::Data }, ty: t.ty });
            }
            match callee {
                Expression::Identifier(id) => {
                    if let Some(t) = symbol_of(scoping, id).and_then(|s| tainted.get(&s).copied()) {
                        match t.kind {
                            Kind::Maker => return Some(Taint { kind: Kind::Handle, ..t }),
                            Kind::Code => return Some(Taint { kind: Kind::Code, ..t }),
                            _ => {}
                        }
                    }
                }
                Expression::StaticMemberExpression(m) => {
                    if let Some(o) = classify(scoping, sites, tainted, &m.object) {
                        let platform = is_platform_method(o.ty, m.property.name.as_str());
                        if o.kind == Kind::Code || (o.kind == Kind::Handle && !platform) {
                            return Some(plain(o.origin, Kind::Code));
                        }
                    }
                }
                Expression::ComputedMemberExpression(m) => {
                    if let Some(o) = classify(scoping, sites, tainted, &m.object) {
                        if o.kind >= Kind::Handle {
                            return Some(plain(o.origin, Kind::Code));
                        }
                    }
                }
                _ => {}
            }
            origin_of_expr(scoping, sites, tainted, e).map(|origin| plain(origin, Kind::Data))
        }
        Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {
            literal_returns(scoping, sites, tainted, e)
        }
        other => origin_of_expr(scoping, sites, tainted, other).map(|origin| plain(origin, Kind::Data)),
    }
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

    fn classify(&self, e: &Expression<'_>) -> Option<Taint> {
        classify(self.scoping, self.sites, &self.tainted, e)
    }

    /// A symbol only moves up in kind, so the loop ends.
    fn taint(&mut self, sym: SymbolId, t: Taint) {
        match self.tainted.get(&sym) {
            Some(old) if old.kind >= t.kind => {}
            _ => {
                self.tainted.insert(sym, t);
                self.changed = true;
            }
        }
    }

    fn taint_all(&mut self, syms: Vec<SymbolId>, t: Taint) {
        for s in syms {
            self.taint(s, t);
        }
    }

    /// The parameters of a callback argument: a function literal, or a function named here.
    fn taint_callback_arg(&mut self, arg: &Argument<'_>, t: Taint) {
        let syms = match arg {
            Argument::ArrowFunctionExpression(f) => param_symbols(&f.params),
            Argument::FunctionExpression(f) => param_symbols(&f.params),
            Argument::Identifier(id) => symbol_of(self.scoping, id)
                .and_then(|s| self.fn_params.get(&s).cloned())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        self.taint_all(syms, t);
    }
}

impl<'a> Visit<'a> for Propagate<'_> {
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let Some(init) = &it.init {
            if let Some(t) = self.classify(init) {
                let mut syms = Vec::new();
                collect_binding(&it.id, &mut syms);
                // `const { a } = handle` takes pieces of it: those are data, not the object itself.
                let t = if matches!(it.id, BindingPattern::BindingIdentifier(_)) { t } else { Taint { kind: Kind::Data, ..t } };
                self.taint_all(syms, t);
            }
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        // `a = <tainted>` and `a.b = <tainted>` taint `a`.
        if let Some(t) = self.classify(&it.right) {
            if let Some(sym) = target_root(&it.left).and_then(|id| symbol_of(self.scoping, id)) {
                // Only `a = <value>` makes `a` that value; `a.b = <value>` makes `a` an object that holds data.
                let direct = matches!(it.left, AssignmentTarget::AssignmentTargetIdentifier(_));
                self.taint(sym, if direct { t } else { Taint { kind: Kind::Data, ..t } });
            }
        }
        // `x.onmessage = e => ..` where `x` is tainted: the handler's parameters are data.
        if let Some(params) = literal_params(&it.right) {
            let owner = target_root(&it.left)
                .and_then(|id| symbol_of(self.scoping, id))
                .and_then(|s| self.tainted.get(&s).copied());
            if let Some(t) = owner {
                let syms = param_symbols(params);
                self.taint_all(syms, Taint { kind: Kind::Data, ..t });
            }
        }
        walk::walk_assignment_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        // A source call, or a method called on tainted data, hands tainted values to callbacks.
        // Callbacks of a source call get the network object itself (`https.get(u, res => ..)`); so does the
        // `then` of a response (`fetch(u).then(r => ..)`). Any other callback gets data.
        let carrier = if is_source(self.sites, it.span) {
            Some(handle_at(self.sites, it.span))
        } else if let Expression::StaticMemberExpression(m) = strip(&it.callee) {
            match self.classify(&m.object) {
                Some(o) if m.property.name == "then" && o.kind == Kind::Handle => Some(o),
                Some(o) => Some(Taint { kind: Kind::Data, ..o }),
                None => None,
            }
        } else {
            self.origin(&it.callee).map(|origin| plain(origin, Kind::Data))
        };
        if let Some(t) = carrier {
            for arg in &it.arguments {
                self.taint_callback_arg(arg, t);
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
    /// Is what is being called remote code? That is: anything called on a `Code` value, and a method on a
    /// `Handle` that is not the platform's own API. Data, and objects that merely hold data, are called
    /// freely: JSON cannot hold a function, and such an object runs your own code.
    fn check_callee(&mut self, it: &CallExpression<'_>) {
        let callee = strip(&it.callee);
        let origin = match callee {
            Expression::Identifier(id) => symbol_of(self.scoping, id)
                .and_then(|s| self.tainted.get(&s).copied())
                .filter(|t| t.kind == Kind::Code)
                .map(|t| t.origin),
            Expression::StaticMemberExpression(m) => classify(self.scoping, self.sites, self.tainted, &m.object)
                .filter(|o| o.kind == Kind::Code || (o.kind == Kind::Handle && !is_platform_method(o.ty, m.property.name.as_str())))
                .map(|o| o.origin),
            Expression::ComputedMemberExpression(m) => classify(self.scoping, self.sites, self.tainted, &m.object)
                .filter(|o| o.kind >= Kind::Handle)
                .map(|o| o.origin),
            // `new Function(d)()` and `eval(t)()`: the sink itself is already reported.
            Expression::NewExpression(n) if is_sink(self.sites, n.span) => None,
            Expression::CallExpression(c) if is_sink(self.sites, c.span) => None,
            other => classify(self.scoping, self.sites, self.tainted, other).filter(|t| t.kind == Kind::Code).map(|t| t.origin),
        };
        if let Some(origin) = origin {
            self.hits.push(FlowHit { sink: it.span, origin, callee: true });
        }
    }
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
            let source = ["fetch", "https.get", "https.request", "XMLHttpRequest", "WebSocket", "net.connect"]
                .contains(&callee);
            let handle = match callee {
                "fetch" => HandleKind::Response,
                "https.get" | "https.request" => HandleKind::Http,
                "XMLHttpRequest" => HandleKind::Xhr,
                "WebSocket" => HandleKind::WebSocket,
                "net.connect" => HandleKind::Socket,
                _ => HandleKind::Unknown,
            };
            let sink = ["eval", "Function", "vm.runInThisContext", "exec", "spawn"]
                .contains(&callee);
            Site { source, sink, arg: None, handle }
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
    fn a_function_that_contains_a_request_is_not_network_data() {
        // Found on a real built bundle (vitepress): a prefetch helper that makes an XHR inside its body, picked by a
        // condition and then called. The function is not the response, so calling it is not a callee flow.
        let f = callee_flows(
            "const xf = e => { const t = new XMLHttpRequest(); t.open('GET', e); t.send(); };
             const ef = ok ? other : xf; ef(u);",
        );
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn an_object_built_from_a_callback_that_makes_a_request_is_not_network_data() {
        let f = callee_flows(
            "const get = e => { const t = new XMLHttpRequest(); t.send(); };
             const n = new Observer(i => { get(i); }); n.observe(1); n.disconnect();",
        );
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn a_helper_that_returns_what_it_fetched_is_still_network_data() {
        // Wrapping the request in a helper must not hide the flow.
        let a = flows("const load = () => fetch(u).then(r => r.text()); eval(await load());");
        assert_eq!(a.len(), 1, "{a:?}");
        let b = flows("const load = async () => { const r = await fetch(u); return r.text(); }; eval(await load());");
        assert_eq!(b.len(), 1, "{b:?}");
        let c = callee_flows("const make = () => fetch(u); const r = make(); r.scan();");
        assert!(c.contains(&("r.scan()".to_string(), "fetch(u)".to_string())), "{c:?}");
    }

    #[test]
    fn a_function_called_on_the_spot_still_hands_back_what_its_body_fetched() {
        // The body of a function that is called right away does produce the value.
        let f = callee_flows("const r = (() => fetch(server))(); r.scan();");
        assert!(f.contains(&("r.scan()".to_string(), "fetch(server)".to_string())), "{f:?}");
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
    fn calling_what_network_text_was_run_as_is_a_callee_flow() {
        // The text became code, so what it hands back is remote code. (The `eval` itself is a code flow.)
        let f = callee_flows("const run = eval(await (await fetch(u)).text()); run(1);");
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].0, "run(1)");
    }

    #[test]
    fn parsed_json_cannot_hold_a_function_so_calling_into_it_is_not_a_callee_flow() {
        let f = callee_flows("const { run } = await (await fetch(u)).json(); run(1);");
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn calling_a_helper_that_returns_fetched_data_is_not_a_callee_flow_but_its_data_still_flows() {
        // Found on a real project: `readPolicy()` was reported as "calls network data".
        let src = "const read = async () => (await fetch(u)).json(); const first = await read(); read();";
        assert!(callee_flows(src).is_empty());
        let d = flows("const read = async () => (await fetch(u)).text(); eval(await read());");
        assert_eq!(d.len(), 1, "{d:?}");
    }

    #[test]
    fn an_object_built_from_a_request_helper_is_not_network_data() {
        // Found on a real project: `new Client({ requestHandler })` where the handler used `fetch`, then
        // every `client.createSession()` was reported.
        let f = callee_flows(
            "const fetchFn = (i) => fetch(i); const handler = new Handler({ fetchFn });              const client = new Client({ requestHandler: handler }); client.createSession(); client.disconnect();",
        );
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn a_socket_has_its_own_methods() {
        // Found on a real project: `socket.setNoDelay(true)` and `client.getPeerCertificate()`.
        let f = callee_flows("const s = net.connect(o); s.setNoDelay(true); s.getPeerCertificate(); s.write(x); s.on('data', d => d);");
        assert!(f.is_empty(), "{f:?}");
        // ...and not a response's.
        let g = callee_flows("const s = net.connect(o); s.json();");
        assert_eq!(g.len(), 1, "{g:?}");
    }

    #[test]
    fn a_response_with_a_standard_name_that_is_not_its_own_is_a_callee_flow() {
        // The known gap, closed: `end()` is a method of streams, not of a response.
        let f = callee_flows("const vuln = await fetch(server); vuln.end(); vuln.send(1);");
        assert_eq!(f.len(), 2, "{f:?}");
        let ok = callee_flows("const r = await fetch(u); await r.json(); await r.text(); r.clone();");
        assert!(ok.is_empty(), "{ok:?}");
    }

    #[test]
    fn a_websocket_and_a_request_object_have_their_own_methods() {
        assert!(callee_flows("const ws = new WebSocket(u); ws.send(x); ws.close(); ws.addEventListener('message', m => m);").is_empty());
        assert_eq!(callee_flows("const ws = new WebSocket(u); ws.open();").len(), 1);
        assert!(callee_flows("const x = new XMLHttpRequest(); x.open('GET', u); x.send(); x.setRequestHeader('a', 'b');").is_empty());
        assert!(callee_flows("https.get(u, res => { res.setEncoding('utf8'); res.on('data', d => d); res.pipe(out); });").is_empty());
    }

    #[test]
    fn a_helper_that_returns_the_network_object_still_gives_one() {
        let f = callee_flows("const make = () => fetch(u); const r = make(); r.scan();");
        assert!(f.contains(&("r.scan()".to_string(), "fetch(u)".to_string())), "{f:?}");
    }

    #[test]
    fn what_a_non_platform_method_on_the_network_object_returns_is_remote_code() {
        let f = callee_flows("const r = await fetch(u); const x = r.fetchPlugin(); x.run();");
        assert_eq!(f.len(), 2, "{f:?}");
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
                    self.1.insert(key(it.span), Site { source: false, sink: true, arg: Some(0), handle: HandleKind::Unknown });
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
