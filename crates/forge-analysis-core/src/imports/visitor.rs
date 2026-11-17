//! Whole-file analysis. Pass 1 collects every import, declaration and alias, anywhere in the
//! file, so a name is known before its uses. Pass 2 walks calls, `new` expressions and value
//! uses: `require()` and `import()`, API calls matched against the catalog, code-loading sinks.
//! Scope questions ("is this really the imported `fs`? the global `eval`? never reassigned?")
//! are answered by Oxc's semantic analysis, never by name alone.
//!
//! Policy: nothing is dropped. A call we cannot match is an unresolved fact with a reason.

use super::{normalize_module, Binding, BindingKind, Ctx, FileImports, Require};
use super::flow::{self, FlowHit, Site};
use super::handles::{self, Handles};
use forge_analysis_catalog::{Catalog, Entry, Role};
use forge_analysis_schema::{
    Delay, Effect, FactKind, Lane, Quoted, RelPath, StaticFact, Timing, UnresolvedReason,
};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, BindingPattern, CallExpression, ComputedMemberExpression, Expression,
    IdentifierReference, ImportDeclaration, ImportExpression, NewExpression, ObjectPattern,
    PropertyKey, StaticMemberExpression, VariableDeclarator,
};
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType, Span as OxcSpan};
use std::collections::BTreeMap;

/// Objects that stand for the global scope: `globalThis.eval(x)` is `eval(x)`.
const GLOBAL_OBJECTS: [&str; 4] = ["globalThis", "window", "self", "global"];

fn key(sp: OxcSpan) -> (u32, u32) {
    (sp.start, sp.end)
}

/// The catalog entry for a global we match: code loaders (`eval`, `Function`), network
/// globals (`fetch`, `WebSocket`, `XMLHttpRequest`) and timers (`setTimeout`, ...).
fn tracked<'c>(cat: &'c Catalog, name: &str) -> Option<&'c Entry> {
    cat.find_global(name)
        .filter(|e| matches!(e.category, Effect::CodeLoad | Effect::NetOut | Effect::Timer))
}

/// What Node does with a timer delay: anything outside 1..=2^31-1 (including NaN) becomes 1.
fn node_delay(v: f64) -> u64 {
    if (1.0..=2_147_483_647.0).contains(&v) {
        v.floor() as u64
    } else {
        1
    }
}

/// Everything the analysis of one file produces.
pub struct Analysis {
    pub file: FileImports,
    /// Places where network data may reach a code or command sink (see `flow`). Each one is
    /// also a `Flow` fact in `file.facts`.
    pub flows: Vec<FlowHit>,
}

/// Returns None if the file name is not a JS/TS source type.
pub fn analyze_file(file: &RelPath, source: &str, catalog: &Catalog) -> Option<FileImports> {
    analyze_file_full(file, source, catalog).map(|a| a.file)
}

/// Like `analyze_file`, and also returns the flow hits.
pub fn analyze_file_full(file: &RelPath, source: &str, catalog: &Catalog) -> Option<Analysis> {
    let source_type = SourceType::from_path(file.as_quoted().text()).ok()?;
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, source_type).parse();

    if !ret.diagnostics.is_empty() {
        let mut ctx = Ctx::new(file, source);
        ctx.parse_failed();
        return Some(Analysis { file: ctx.finish(), flows: Vec::new() });
    }

    let sem = SemanticBuilder::new().build(&ret.program);
    let ctx = Ctx::new(file, source).with_scoping(sem.semantic.scoping());

    let mut collector = Collector { ctx, catalog };
    collector.visit_program(&ret.program);

    let handles = handles::analyze_handles(&ret.program, sem.semantic.scoping());
    let mut matcher = Matcher { ctx: collector.ctx, catalog, sites: BTreeMap::new(), handles };
    matcher.visit_program(&ret.program);

    let flows = flow::find_flows(&ret.program, sem.semantic.scoping(), &matcher.sites);
    for hit in &flows {
        // Located at the sink; the detail is the quoted source expression; `origin` is where
        // the data entered. An unresolved fact for the same sink (computed code) stays beside it.
        let detail = Quoted::new(matcher.ctx.text(hit.origin));
        let mut fact = StaticFact::new(FactKind::Flow, matcher.ctx.span(hit.sink), Some(detail));
        fact.origin = Some(matcher.ctx.span(hit.origin));
        matcher.ctx.facts.push(fact);
    }

    let mut out = matcher.ctx.finish();
    out.facts.sort_by_key(|f| (f.at().start_byte, f.at().end_byte));
    Some(Analysis { file: out, flows })
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

/// Sees through `(f)` and `(0, f)`, which call `f` just the same.
fn unwrap_callee<'x, 'a>(mut e: &'x Expression<'a>) -> &'x Expression<'a> {
    loop {
        match e {
            Expression::ParenthesizedExpression(p) => e = &p.expression,
            Expression::SequenceExpression(s) => match s.expressions.last() {
                Some(last) => e = last,
                None => return e,
            },
            _ => return e,
        }
    }
}

impl Ctx<'_> {
    /// The binding an identifier refers to, per Oxc's scope analysis.
    fn resolve_binding(&self, id: &IdentifierReference<'_>) -> Option<Binding> {
        let scoping = self.scoping?;
        let rid = id.reference_id.get()?;
        let sym = scoping.get_reference(rid).symbol_id()?;
        self.table.lookup_by_decl(scoping.symbol_span(sym).start).cloned()
    }

    /// True if this name is not declared anywhere in the file (so it is the real global).
    /// With no scope information we cannot tell; treat it as the real thing, not hide it.
    fn is_global_ref(&self, id: &IdentifierReference<'_>) -> bool {
        match (self.scoping, id.reference_id.get()) {
            (Some(s), Some(rid)) => s.get_reference(rid).symbol_id().is_none(),
            _ => true,
        }
    }
}

// ---- pass 1: bindings and aliases ----

struct Collector<'s> {
    ctx: Ctx<'s>,
    catalog: &'s Catalog,
}

impl Collector<'_> {
    /// What a member of a tracked thing is, if we can say.
    fn member_of(&self, kind: &BindingKind, module: &str, name: &str) -> Option<(BindingKind, String)> {
        match kind {
            BindingKind::Namespace | BindingKind::Default => Some((
                BindingKind::Named { imported: name.to_string() },
                module.to_string(),
            )),
            BindingKind::Named { .. } | BindingKind::Opaque => {
                Some((BindingKind::Opaque, module.to_string()))
            }
            BindingKind::GlobalObject => tracked(self.catalog, name)
                .map(|_| (BindingKind::GlobalAlias { name: name.to_string() }, String::new())),
            BindingKind::GlobalAlias { .. } => None,
        }
    }

    /// What an expression stands for, if it is something we track.
    fn alias_target(&self, e: &Expression<'_>) -> Option<(BindingKind, String)> {
        match unwrap_callee(e) {
            Expression::Identifier(id) => {
                if let Some(b) = self.ctx.resolve_binding(id) {
                    return Some((b.kind, b.module));
                }
                if !self.ctx.is_global_ref(id) {
                    return None;
                }
                let name = self.ctx.text(id.span);
                if GLOBAL_OBJECTS.contains(&name) {
                    Some((BindingKind::GlobalObject, String::new()))
                } else if tracked(self.catalog, name).is_some() {
                    Some((BindingKind::GlobalAlias { name: name.to_string() }, String::new()))
                } else {
                    None
                }
            }
            Expression::StaticMemberExpression(m) => {
                let name = self.ctx.text(m.property.span);
                let (kind, module) = self.alias_target(&m.object)?;
                self.member_of(&kind, &module, name)
            }
            _ => None,
        }
    }

    /// `obj[key]`: a literal key is the same as a static member; anything else is reported.
    fn computed_target(
        &mut self,
        d: &VariableDeclarator<'_>,
        m: &ComputedMemberExpression<'_>,
    ) -> Option<(BindingKind, String)> {
        let (kind, module) = self.alias_target(&m.object)?;
        let literal = match &m.expression {
            Expression::StringLiteral(l) => self.ctx.literal(l),
            _ => None,
        };
        match (&kind, literal) {
            (BindingKind::GlobalAlias { .. }, _) => None,
            (_, Some(k)) => self.member_of(&kind, &module, &k),
            (BindingKind::GlobalObject, None) => {
                let t = self.ctx.text(d.span);
                self.ctx.unresolved(d.span, UnresolvedReason::ComputedAccess, t);
                None
            }
            (_, None) => {
                let t = self.ctx.text(d.span);
                self.ctx.unresolved(d.span, UnresolvedReason::ComputedAccess, t);
                Some((BindingKind::Opaque, module))
            }
        }
    }

    fn alias_declarator(&mut self, d: &VariableDeclarator<'_>, init: &Expression<'_>) {
        let target = match unwrap_callee(init) {
            Expression::ComputedMemberExpression(m) => self.computed_target(d, m),
            other => self.alias_target(other),
        };
        let Some((kind, module)) = target else { return };
        match &d.id {
            BindingPattern::BindingIdentifier(id) => {
                let local = self.ctx.text(id.span).to_string();
                self.ctx.bind(id.span, id, local, &module, kind);
            }
            BindingPattern::ObjectPattern(p) => self.alias_pattern(p, &kind, &module),
            _ => {
                let t = self.ctx.text(d.span);
                self.ctx.unresolved(d.span, UnresolvedReason::UnsupportedSyntax, t);
            }
        }
    }

    /// `const { a, b: c } = <tracked thing>`
    fn alias_pattern(&mut self, p: &ObjectPattern<'_>, kind: &BindingKind, module: &str) {
        for prop in &p.properties {
            if prop.computed {
                let t = self.ctx.text(prop.span);
                self.ctx.unresolved(prop.span, UnresolvedReason::ComputedAccess, t);
                continue;
            }
            let key_name = match &prop.key {
                PropertyKey::StaticIdentifier(k) => Some(self.ctx.text(k.span).to_string()),
                PropertyKey::StringLiteral(l) => self.ctx.literal(l),
                _ => None,
            };
            let target = match &prop.value {
                BindingPattern::AssignmentPattern(a) => &a.left,
                other => other,
            };
            match (key_name, target) {
                (Some(name), BindingPattern::BindingIdentifier(id)) => {
                    if let Some((k, m)) = self.member_of(kind, module, &name) {
                        let local = self.ctx.text(id.span).to_string();
                        self.ctx.bind(id.span, id, local, &m, k);
                    }
                }
                _ => {
                    let t = self.ctx.text(prop.span);
                    self.ctx.unresolved(prop.span, UnresolvedReason::UnsupportedSyntax, t);
                }
            }
        }
        if p.rest.is_some() {
            let t = self.ctx.text(p.span);
            self.ctx.unresolved(p.span, UnresolvedReason::UnsupportedSyntax, t);
        }
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        self.ctx.import_declaration(it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        self.ctx.declarator(it);
        if let Some(init) = &it.init {
            // A declaration's initialiser is handled here (alias or not); it is not a stray use.
            self.ctx.consumed.insert(key(init.span()));
            self.ctx.consumed.insert(key(unwrap_callee(init).span()));
            self.alias_declarator(it, init);
        }
        walk::walk_variable_declarator(self, it);
    }
}

// ---- pass 2: calls and uses ----

struct Matcher<'s> {
    ctx: Ctx<'s>,
    catalog: &'s Catalog,
    /// Every matched call or `new`, by span: is it a network source, a code/command sink?
    sites: BTreeMap<(u32, u32), Site>,
    /// Which timer handles are cleared, thrown away, or escape.
    handles: Handles,
}

impl<'s> Matcher<'s> {
    fn import_binding(&self, id: &IdentifierReference<'_>) -> Option<Binding> {
        self.ctx.resolve_binding(id)
    }

    fn is_global(&self, id: &IdentifierReference<'_>) -> bool {
        self.ctx.is_global_ref(id)
    }

    /// None: not called `require`. Some(true): Node's global `require`.
    /// Some(false): `require` is a local name (parameter, variable, function).
    fn require_scope(&self, call: &CallExpression<'_>) -> Option<bool> {
        let Expression::Identifier(callee) = &call.callee else { return None };
        if self.ctx.text(callee.span) != "require" {
            return None;
        }
        Some(self.is_global(callee))
    }

    fn tracked_global(&self, name: &str) -> Option<&'s Entry> {
        let cat: &'s Catalog = self.catalog;
        tracked(cat, name)
    }

    fn flag_at(&mut self, sp: OxcSpan, reason: UnresolvedReason) {
        let t = self.ctx.text(sp);
        self.ctx.unresolved(sp, reason, t);
    }

    fn flag(&mut self, call: &CallExpression<'_>, reason: UnresolvedReason) {
        self.flag_at(call.span, reason);
    }

    /// The call reaches a module we know nothing, or not enough, about.
    fn unmatched(&mut self, call: &CallExpression<'_>, module: &str) {
        let reason = if self.catalog.knows_module(module) {
            UnresolvedReason::UncatalogedMember
        } else {
            UnresolvedReason::UncoveredModule
        };
        self.flag(call, reason);
    }

    /// `module.name(...)` through any route: namespace, alias, or `require("m").name`.
    fn resolve(&mut self, call: &CallExpression<'_>, module: &str, name: &str) {
        let cat = self.catalog;
        if let Some(entry) = cat.find_function(module, name) {
            self.emit_effect(call, entry);
        } else {
            self.unmatched(call, module);
        }
    }

    /// A method call on something we cannot trace, whose name matches a catalogued API.
    fn maybe_unknown(&mut self, call: &CallExpression<'_>, name: &str) {
        if self.catalog.exports_name(name) {
            self.flag(call, UnresolvedReason::UnknownReceiver);
        }
    }

    fn emit_effect(&mut self, call: &CallExpression<'_>, entry: &Entry) {
        self.emit_parts(call.span, &call.callee, &call.arguments, entry);
    }

    /// One Call fact and one Effect fact for a matched call or `new` expression.
    fn emit_parts(
        &mut self,
        at: OxcSpan,
        callee: &Expression<'_>,
        args: &[Argument<'_>],
        entry: &Entry,
    ) {
        let callee_text = self.ctx.text(callee.span());
        let call_fact = StaticFact::new(
            FactKind::Call,
            self.ctx.span(at),
            Some(Quoted::new(callee_text)),
        );
        self.ctx.facts.push(call_fact);

        if entry.category == Effect::Timer {
            self.emit_timer(at, callee, args, entry);
            return;
        }

        let site = Site {
            source: entry.category == Effect::NetOut && entry.role == Some(Role::Source),
            sink: matches!(
                entry.category,
                Effect::CodeLoad | Effect::ProcessSpawn | Effect::DynamicLoad
            ),
            arg: None,
        };
        if site.source || site.sink {
            self.sites.insert(key(at), site);
        }

        // The argument that carries the path, command, URL or code, if the catalog names one.
        let code_load = entry.category == Effect::CodeLoad;
        let role = entry
            .args
            .path
            .or(entry.args.command)
            .or(entry.args.url)
            .or(if code_load { Some(0) } else { None });
        let mut detail: Option<String> = None;
        let mut problem: Option<(OxcSpan, UnresolvedReason)> = None;
        if let Some(i) = role {
            match args.get(usize::from(i)) {
                None => {}
                Some(Argument::StringLiteral(lit)) => match self.ctx.literal(lit) {
                    Some(v) => detail = Some(v),
                    None => problem = Some((lit.span, UnresolvedReason::UnsupportedSyntax)),
                },
                Some(other) => {
                    let reason = if code_load {
                        UnresolvedReason::Eval
                    } else {
                        UnresolvedReason::ComputedAccess
                    };
                    problem = Some((other.span(), reason));
                }
            }
        }

        let mut effect = StaticFact::new(
            FactKind::Effect,
            self.ctx.span(at),
            detail.as_deref().map(Quoted::new),
        );
        effect.effect = Some(entry.category);
        self.ctx.facts.push(effect);

        if let Some((sp, reason)) = problem {
            self.flag_at(sp, reason);
        }
    }

    /// A timer call: a Timer fact (lane, delay, cleared). A callback that is text is code
    /// being loaded, so it is also a code-load effect and a flow sink.
    fn emit_timer(
        &mut self,
        at: OxcSpan,
        callee: &Expression<'_>,
        args: &[Argument<'_>],
        entry: &Entry,
    ) {
        let mut problems: Vec<(OxcSpan, UnresolvedReason)> = Vec::new();

        let lane = if entry.repeats {
            Lane::Recurring
        } else if entry.args.delay.is_none() {
            Lane::Immediate
        } else {
            Lane::Deferred
        };
        let delay_ms = match entry.args.delay {
            None => Delay::Known(0),
            Some(i) => match args.get(usize::from(i)) {
                None => Delay::Known(1),
                Some(Argument::NumericLiteral(n)) => Delay::Known(node_delay(n.value)),
                Some(other) => {
                    problems.push((other.span(), UnresolvedReason::RuntimeDelay));
                    Delay::Unresolved
                }
            },
        };

        // `Some(detail)`: the callback is text that will be run as code.
        let mut code: Option<Option<String>> = None;
        let mut sink = false;
        let callback = entry.args.callback;
        if let Some(arg) = callback.and_then(|i| args.get(usize::from(i))) {
            match arg {
                Argument::ArrowFunctionExpression(_) | Argument::FunctionExpression(_) => {}
                // a function we can name, or a value we cannot tell apart from one; if it is
                // network data the flow check reports it
                Argument::Identifier(_) => sink = true,
                Argument::StringLiteral(lit) => {
                    sink = true;
                    match self.ctx.literal(lit) {
                        Some(v) => code = Some(Some(v)),
                        None => {
                            code = Some(None);
                            problems.push((lit.span, UnresolvedReason::UnsupportedSyntax));
                        }
                    }
                }
                Argument::TemplateLiteral(_) | Argument::BinaryExpression(_) => {
                    sink = true;
                    code = Some(None);
                    problems.push((arg.span(), UnresolvedReason::Eval));
                }
                other => {
                    sink = true;
                    problems.push((other.span(), UnresolvedReason::EventHandler));
                }
            }
        }
        self.sites.insert(key(at), Site { source: false, sink, arg: callback });

        let callee_text = self.ctx.text(callee.span());
        let mut timer = StaticFact::new(
            FactKind::Timer,
            self.ctx.span(at),
            Some(Quoted::new(callee_text)),
        );
        timer.timing = Some(Timing { lane, delay_ms, cleared: self.handles.cleared(at) });
        self.ctx.facts.push(timer);

        if let Some(detail) = code {
            let mut effect = StaticFact::new(
                FactKind::Effect,
                self.ctx.span(at),
                detail.as_deref().map(Quoted::new),
            );
            effect.effect = Some(Effect::CodeLoad);
            self.ctx.facts.push(effect);
        }

        for (sp, reason) in problems {
            self.flag_at(sp, reason);
        }
    }

    /// `eval(...)`, `Function(...)`: the unshadowed global, or a local we cannot trust.
    fn global_call(&mut self, call: &CallExpression<'_>, id: &IdentifierReference<'_>) {
        let name = self.ctx.text(id.span);
        let Some(entry) = self.tracked_global(name) else { return };
        if self.is_global(id) {
            self.emit_effect(call, entry);
        } else {
            self.flag(call, UnresolvedReason::AmbiguousBinding);
        }
    }

    /// `globalThis.eval(...)`. Returns true if it was one.
    fn global_object_call(
        &mut self,
        call: &CallExpression<'_>,
        id: &IdentifierReference<'_>,
        name: &str,
    ) -> bool {
        if !GLOBAL_OBJECTS.contains(&self.ctx.text(id.span)) || !self.is_global(id) {
            return false;
        }
        match self.tracked_global(name) {
            Some(entry) => {
                self.emit_effect(call, entry);
                true
            }
            None => false,
        }
    }

    /// `<global object>["eval"](...)` or `<global object>[name](...)`.
    fn computed_on_global(&mut self, call: &CallExpression<'_>, key_expr: &Expression<'_>) {
        let literal = match key_expr {
            Expression::StringLiteral(l) => self.ctx.literal(l),
            _ => None,
        };
        match literal {
            Some(k) => {
                if let Some(entry) = self.tracked_global(&k) {
                    self.emit_effect(call, entry);
                }
            }
            None => self.flag(call, UnresolvedReason::ComputedAccess),
        }
    }

    fn member_call(&mut self, call: &CallExpression<'_>, m: &StaticMemberExpression<'_>) {
        let name = self.ctx.text(m.property.span);

        // x.constructor.constructor("code"): the classic way to reach Function without naming it.
        if name == "constructor" {
            if let Expression::StaticMemberExpression(inner) = &m.object {
                if self.ctx.text(inner.property.span) == "constructor" {
                    self.flag(call, UnresolvedReason::Reflection);
                    return;
                }
            }
        }

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
                    BindingKind::Named { .. } | BindingKind::Opaque => {
                        self.unmatched(call, &b.module)
                    }
                    BindingKind::GlobalObject => {
                        if let Some(entry) = self.tracked_global(name) {
                            self.emit_effect(call, entry);
                        }
                    }
                    BindingKind::GlobalAlias { .. } => {}
                },
                None => {
                    if !self.global_object_call(call, id, name) {
                        self.maybe_unknown(call, name);
                    }
                }
            },
            other => match root_ident(other).and_then(|id| self.import_binding(id)) {
                Some(b) => match &b.kind {
                    BindingKind::GlobalObject | BindingKind::GlobalAlias { .. } => {}
                    _ => self.unmatched(call, &b.module),
                },
                None => self.maybe_unknown(call, name),
            },
        }
    }

    fn match_call(&mut self, call: &CallExpression<'_>) {
        match unwrap_callee(&call.callee) {
            Expression::Identifier(id) => match self.import_binding(id) {
                Some(b) => match &b.kind {
                    BindingKind::Named { imported } => self.resolve(call, &b.module, imported),
                    // the module itself, or an unnamed member of it, called as a function
                    BindingKind::Namespace | BindingKind::Default | BindingKind::Opaque => {
                        self.unmatched(call, &b.module)
                    }
                    BindingKind::GlobalAlias { name } => {
                        if let Some(entry) = self.tracked_global(name) {
                            self.emit_effect(call, entry);
                        }
                    }
                    BindingKind::GlobalObject => {}
                },
                None => self.global_call(call, id),
            },
            Expression::StaticMemberExpression(m) => self.member_call(call, m),
            // x[name](...): the member is chosen at runtime
            Expression::ComputedMemberExpression(m) => {
                match root_ident(&m.object).and_then(|id| self.import_binding(id)) {
                    Some(b) => match &b.kind {
                        BindingKind::GlobalObject => self.computed_on_global(call, &m.expression),
                        BindingKind::GlobalAlias { .. } => {}
                        _ => self.flag(call, UnresolvedReason::ComputedAccess),
                    },
                    None => {
                        if let Expression::Identifier(id) = &m.object {
                            if GLOBAL_OBJECTS.contains(&self.ctx.text(id.span)) && self.is_global(id) {
                                self.computed_on_global(call, &m.expression);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// `new Function(...)`, `new vm.Script(...)`.
    fn match_new(&mut self, it: &NewExpression<'_>) {
        let cat = self.catalog;
        let mut found: Option<&Entry> = None;
        // Set when the callee names a member of a module, to report a miss.
        let mut module: Option<String> = None;

        match unwrap_callee(&it.callee) {
            Expression::Identifier(id) => match self.import_binding(id) {
                Some(b) => match &b.kind {
                    BindingKind::Named { imported } => {
                        found = cat.find_constructor(&b.module, imported);
                        module = Some(b.module.clone());
                    }
                    BindingKind::GlobalAlias { name } => found = self.tracked_global(name),
                    _ => {}
                },
                None => {
                    let name = self.ctx.text(id.span);
                    if let Some(entry) = self.tracked_global(name) {
                        if self.is_global(id) {
                            found = Some(entry);
                        } else {
                            self.flag_at(it.span, UnresolvedReason::AmbiguousBinding);
                        }
                    }
                }
            },
            Expression::StaticMemberExpression(m) => {
                let name = self.ctx.text(m.property.span);
                match &m.object {
                    Expression::Identifier(id) => match self.import_binding(id) {
                        Some(b) => match &b.kind {
                            BindingKind::Namespace | BindingKind::Default => {
                                found = cat.find_constructor(&b.module, name);
                                module = Some(b.module.clone());
                            }
                            BindingKind::Named { .. } | BindingKind::Opaque => {
                                module = Some(b.module.clone());
                            }
                            BindingKind::GlobalObject => found = self.tracked_global(name),
                            BindingKind::GlobalAlias { .. } => {}
                        },
                        None => {
                            if GLOBAL_OBJECTS.contains(&self.ctx.text(id.span)) && self.is_global(id) {
                                found = self.tracked_global(name);
                            }
                        }
                    },
                    Expression::CallExpression(inner) => {
                        if let Some((Require::Literal(lit), _)) = self.ctx.classify_call(inner) {
                            let m = normalize_module(&lit);
                            found = cat.find_constructor(&m, name);
                            module = Some(m);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        if let Some(entry) = found {
            self.emit_parts(it.span, &it.callee, &it.arguments, entry);
        } else if let Some(m) = module {
            let reason = if cat.knows_module(&m) {
                UnresolvedReason::UncatalogedMember
            } else {
                UnresolvedReason::UncoveredModule
            };
            self.flag_at(it.span, reason);
        }
    }

    /// A catalogued API used as a value, not called: `list.forEach(fs.readFile)`.
    fn indirect_member(&mut self, it: &StaticMemberExpression<'_>) {
        let name = self.ctx.text(it.property.span);
        let cat = self.catalog;
        let known = |module: &str| {
            cat.find_function(module, name).is_some() || cat.find_constructor(module, name).is_some()
        };
        let hit = match &it.object {
            Expression::Identifier(id) => match self.import_binding(id) {
                Some(b) => match &b.kind {
                    BindingKind::Namespace | BindingKind::Default => known(&b.module),
                    BindingKind::GlobalObject => self.tracked_global(name).is_some(),
                    _ => false,
                },
                None => {
                    GLOBAL_OBJECTS.contains(&self.ctx.text(id.span))
                        && self.is_global(id)
                        && self.tracked_global(name).is_some()
                }
            },
            Expression::CallExpression(inner) => match self.ctx.classify_call(inner) {
                Some((Require::Literal(lit), _)) => known(&normalize_module(&lit)),
                _ => false,
            },
            _ => false,
        };
        if hit {
            self.flag_at(it.span, UnresolvedReason::IndirectUse);
        }
    }

    /// A catalogued name used as a value: `run(eval)`, `setup(exec)`.
    fn indirect_ident(&mut self, it: &IdentifierReference<'_>) {
        let cat = self.catalog;
        let hit = match self.import_binding(it) {
            Some(b) => match &b.kind {
                BindingKind::Named { imported } => {
                    cat.find_function(&b.module, imported).is_some()
                        || cat.find_constructor(&b.module, imported).is_some()
                }
                BindingKind::GlobalAlias { .. } => true,
                _ => false,
            },
            None => {
                let name = self.ctx.text(it.span);
                self.is_global(it) && self.tracked_global(name).is_some()
            }
        };
        if hit {
            self.flag_at(it.span, UnresolvedReason::IndirectUse);
        }
    }
}

impl<'a> Visit<'a> for Matcher<'_> {
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        self.ctx.consumed.insert(key(unwrap_callee(&it.callee).span()));
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

    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        self.ctx.consumed.insert(key(unwrap_callee(&it.callee).span()));
        self.match_new(it);
        walk::walk_new_expression(self, it);
    }

    fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'a>) {
        if !self.ctx.consumed.contains(&key(it.span)) {
            self.indirect_member(it);
        }
        walk::walk_static_member_expression(self, it);
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if !self.ctx.consumed.contains(&key(it.span)) {
            self.indirect_ident(it);
        }
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

    // ---- code-load sinks ----

    #[test]
    fn eval_with_a_literal_is_a_code_load_fact() {
        let f = run("a.js", r#"eval("1+1");"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, Some("1+1".into()))));
        assert_eq!(count(&f, FactKind::Call), 1);
        assert!(reasons(&f).is_empty());
    }

    #[test]
    fn eval_of_a_variable_reports_computed_code() {
        let f = run("a.js", "eval(code);");
        assert_eq!(effect(&f), Some((Effect::CodeLoad, None)));
        assert_eq!(reasons(&f), vec![UnresolvedReason::Eval]);
    }

    #[test]
    fn new_function_is_a_code_load() {
        let f = run("a.js", r#"new Function("return 1");"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, Some("return 1".into()))));
        assert!(reasons(&f).is_empty());
    }

    #[test]
    fn function_called_without_new_is_a_code_load() {
        let f = run("a.js", "Function(body)();");
        assert_eq!(effect(&f), Some((Effect::CodeLoad, None)));
        assert_eq!(reasons(&f), vec![UnresolvedReason::Eval]);
    }

    #[test]
    fn vm_run_calls_are_code_loads() {
        let f = run("a.js", r#"const vm = require("vm"); vm.runInThisContext(src);"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, None)));
        assert_eq!(reasons(&f), vec![UnresolvedReason::Eval]);
    }

    #[test]
    fn new_vm_script_is_a_code_load() {
        let f = run("a.js", r#"const vm = require("vm"); new vm.Script("1");"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, Some("1".into()))));
    }

    #[test]
    fn a_named_import_of_the_vm_constructor_matches() {
        let f = run("a.js", r#"import { Script } from "vm"; new Script(s);"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, None)));
        assert_eq!(reasons(&f), vec![UnresolvedReason::Eval]);
    }

    #[test]
    fn a_local_function_named_like_a_sink_is_not_trusted() {
        let f = run("a.js", r#"function f() { function Function(s) { return s; } return Function("x"); }"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::AmbiguousBinding]);
    }

    #[test]
    fn eval_through_the_global_object_is_found() {
        let f = run("a.js", "globalThis.eval(x);");
        assert_eq!(effect(&f), Some((Effect::CodeLoad, None)));
        assert_eq!(reasons(&f), vec![UnresolvedReason::Eval]);
    }

    #[test]
    fn indirect_eval_is_found() {
        let f = run("a.js", r#"(0, eval)("1");"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, Some("1".into()))));
    }

    #[test]
    fn a_literal_computed_key_on_the_global_object_is_found() {
        let f = run("a.js", r#"window["eval"]("1");"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, Some("1".into()))));
    }

    #[test]
    fn a_computed_key_on_the_global_object_is_reported() {
        let f = run("a.js", r#"globalThis[name]("1");"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::ComputedAccess]);
    }

    #[test]
    fn constructor_dot_constructor_is_reported_as_reflection() {
        let f = run("a.js", r#"[].constructor.constructor("return 1")();"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::Reflection]);
    }

    #[test]
    fn a_remote_dynamic_import_is_a_dynamic_load() {
        let f = run("a.js", r#"async function f() { await import("https://example.com/x.js"); }"#);
        assert_eq!(count(&f, FactKind::Import), 1);
        assert_eq!(effect(&f), Some((Effect::DynamicLoad, Some("https://example.com/x.js".into()))));
        assert!(reasons(&f).is_empty());
    }

    #[test]
    fn a_remote_static_import_is_a_dynamic_load() {
        let f = run("a.js", r#"import x from "https://example.com/x.js";"#);
        assert_eq!(effect(&f), Some((Effect::DynamicLoad, Some("https://example.com/x.js".into()))));
    }

    #[test]
    fn similar_names_are_not_sinks() {
        let f = run("a.js", r#"evaluate(x); myeval("1"); obj.eval("1");"#);
        assert!(effect(&f).is_none());
        assert!(reasons(&f).is_empty());
    }


    // ---- silences closed ----

    #[test]
    fn a_module_the_catalog_does_not_cover_is_reported() {
        let f = run("a.js", r#"const path = require("path"); path.join("a", "b");"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::UncoveredModule]);
    }

    #[test]
    fn calling_an_uncovered_module_itself_is_reported() {
        let f = run("a.js", r#"const express = require("express"); express();"#);
        assert_eq!(reasons(&f), vec![UnresolvedReason::UncoveredModule]);
    }

    #[test]
    fn an_uncatalogued_constructor_of_a_covered_module_is_reported() {
        let f = run("a.js", r#"const http = require("http"); new http.Agent();"#);
        assert_eq!(reasons(&f), vec![UnresolvedReason::UncatalogedMember]);
    }

    #[test]
    fn a_constructor_of_an_uncovered_module_is_reported() {
        let f = run("a.js", r#"const ws = require("ws"); new ws.Server();"#);
        assert_eq!(reasons(&f), vec![UnresolvedReason::UncoveredModule]);
    }

    // ---- aliases ----

    #[test]
    fn an_alias_of_a_member_is_followed() {
        let f = run("a.js", r#"const fs = require("fs"); const r = fs.readFile; r("/etc/passwd");"#);
        assert_eq!(effect(&f), Some((Effect::FsRead, Some("/etc/passwd".into()))));
        assert!(reasons(&f).is_empty());
    }

    #[test]
    fn destructuring_from_a_namespace_is_followed() {
        let f = run("a.js", r#"import fs from "fs"; const { readFile } = fs; readFile("a");"#);
        assert_eq!(effect(&f), Some((Effect::FsRead, Some("a".into()))));
    }

    #[test]
    fn an_alias_of_a_namespace_is_followed() {
        let f = run("a.js", r#"const fs = require("fs"); const a = fs; a.unlinkSync("x");"#);
        assert_eq!(effect(&f), Some((Effect::FsDelete, Some("x".into()))));
    }

    #[test]
    fn aliases_of_aliases_are_followed() {
        let f = run("a.js", r#"const fs = require("fs"); const a = fs; const r = a.readFile; r("x");"#);
        assert_eq!(effect(&f), Some((Effect::FsRead, Some("x".into()))));
    }

    #[test]
    fn an_alias_of_eval_is_a_code_load() {
        let f = run("a.js", r#"const e = eval; e("1");"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, Some("1".into()))));
        assert!(reasons(&f).is_empty());
    }

    #[test]
    fn an_alias_of_the_global_object_is_followed() {
        let f = run("a.js", r#"const g = globalThis; g.eval("1");"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, Some("1".into()))));
    }

    #[test]
    fn destructuring_eval_from_the_global_object_is_followed() {
        let f = run("a.js", r#"const { eval: e } = globalThis; e("1");"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, Some("1".into()))));
    }

    #[test]
    fn an_alias_of_function_can_be_constructed() {
        let f = run("a.js", r#"const F = Function; new F("return 1");"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, Some("return 1".into()))));
    }

    #[test]
    fn a_reassigned_alias_is_not_trusted() {
        let f = run("a.js", r#"const fs = require("fs"); let r = fs.readFile; r = other; r("x");"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::AmbiguousBinding]);
    }

    #[test]
    fn an_alias_of_an_uncovered_member_is_reported_when_called() {
        let f = run("a.js", r#"const path = require("path"); const j = path.join; j("a");"#);
        assert_eq!(reasons(&f), vec![UnresolvedReason::UncoveredModule]);
    }

    #[test]
    fn an_alias_of_a_deeper_path_is_reported_when_called() {
        let f = run("a.js", r#"const fs = require("fs"); const r = fs.promises.readFile; r("x");"#);
        assert_eq!(reasons(&f), vec![UnresolvedReason::UncatalogedMember]);
    }

    #[test]
    fn an_alias_through_a_computed_key_is_reported() {
        let f = run("a.js", r#"const fs = require("fs"); const r = fs[name]; r("x");"#);
        let r = reasons(&f);
        assert!(r.contains(&UnresolvedReason::ComputedAccess));
        assert!(r.contains(&UnresolvedReason::UncatalogedMember));
    }

    // ---- catalogued APIs used as values ----

    #[test]
    fn passing_a_catalogued_member_as_a_value_is_reported() {
        let f = run("a.js", r#"const fs = require("fs"); [1].forEach(fs.readFile);"#);
        assert_eq!(reasons(&f), vec![UnresolvedReason::IndirectUse]);
    }

    #[test]
    fn passing_eval_as_a_value_is_reported() {
        let f = run("a.js", "run(eval);");
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::IndirectUse]);
    }

    #[test]
    fn calling_through_call_or_apply_is_reported() {
        let f = run("a.js", r#"const fs = require("fs"); fs.readFile.call(null, "x");"#);
        let r = reasons(&f);
        assert!(r.contains(&UnresolvedReason::IndirectUse));
        assert!(r.contains(&UnresolvedReason::UncatalogedMember));
    }

    #[test]
    fn passing_a_named_import_as_a_value_is_reported() {
        let f = run("a.js", r#"import { exec } from "child_process"; setup(exec);"#);
        assert_eq!(reasons(&f), vec![UnresolvedReason::IndirectUse]);
    }

    // ---- network globals ----

    #[test]
    fn fetch_with_a_literal_url_is_a_network_effect() {
        let f = run("a.js", r#"fetch("https://example.com/x");"#);
        assert_eq!(effect(&f), Some((Effect::NetOut, Some("https://example.com/x".into()))));
        assert_eq!(count(&f, FactKind::Call), 1);
        assert!(reasons(&f).is_empty());
    }

    #[test]
    fn fetch_with_a_computed_url_is_reported() {
        let f = run("a.js", "fetch(u);");
        assert_eq!(effect(&f), Some((Effect::NetOut, None)));
        assert_eq!(reasons(&f), vec![UnresolvedReason::ComputedAccess]);
    }

    #[test]
    fn fetch_through_the_global_object_or_an_alias_is_found() {
        let f = run("a.js", r#"globalThis.fetch("a");"#);
        assert_eq!(effect(&f), Some((Effect::NetOut, Some("a".into()))));
        let f = run("a.js", r#"const g = fetch; g("b");"#);
        assert_eq!(effect(&f), Some((Effect::NetOut, Some("b".into()))));
    }

    #[test]
    fn a_local_fetch_is_not_trusted() {
        let f = run("a.js", r#"function f(fetch) { return fetch("x"); }"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::AmbiguousBinding]);
    }

    #[test]
    fn websocket_and_xhr_constructors_are_network_effects() {
        let f = run("a.js", r#"new WebSocket("wss://example.com/s");"#);
        assert_eq!(effect(&f), Some((Effect::NetOut, Some("wss://example.com/s".into()))));
        let f = run("a.js", "new XMLHttpRequest();");
        assert_eq!(effect(&f), Some((Effect::NetOut, None)));
        assert!(reasons(&f).is_empty());
    }

    // ---- network data reaching a sink ----

    fn flows(name: &str, src: &str) -> Vec<(String, String)> {
        let a = analyze_file_full(&RelPath::new(name).unwrap(), src, &Catalog::node_defaults())
            .unwrap();
        let text = |s: OxcSpan| src[s.start as usize..s.end as usize].to_string();
        a.flows.iter().map(|h| (text(h.sink), text(h.origin))).collect()
    }

    #[test]
    fn fetched_text_reaching_eval_is_a_flow() {
        let f = flows(
            "a.js",
            "async function f() { const t = await (await fetch(u)).text(); eval(t); }",
        );
        assert_eq!(f, vec![("eval(t)".to_string(), "fetch(u)".to_string())]);
    }

    #[test]
    fn an_aliased_fetch_still_counts_as_a_source() {
        let f = flows("a.js", "const g = fetch; g(u).then(t => eval(t));");
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].0, "eval(t)");
    }

    #[test]
    fn http_response_data_reaching_new_function_is_a_flow() {
        let f = flows(
            "a.js",
            r#"const https = require("https");
               https.get(u, res => { res.on("data", d => { new Function(d)(); }); });"#,
        );
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].0.starts_with("new Function(d)"));
        assert!(f[0].1.starts_with("https.get("));
    }

    #[test]
    fn a_named_import_source_counts() {
        let f = flows("a.js", r#"import { get } from "https"; get(u, r => r.on("d", x => eval(x)));"#);
        assert_eq!(f.len(), 1, "{f:?}");
    }

    #[test]
    fn network_data_reaching_a_command_is_a_flow() {
        let f = flows(
            "a.js",
            r#"const cp = require("child_process");
               async function f() { const t = await (await fetch(u)).text(); cp.exec(t); }"#,
        );
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].0, "cp.exec(t)");
    }

    #[test]
    fn websocket_messages_reaching_vm_are_a_flow() {
        let f = flows(
            "a.js",
            r#"const vm = require("vm");
               const ws = new WebSocket(u);
               ws.onmessage = e => { vm.runInThisContext(e.data); };"#,
        );
        assert_eq!(f.len(), 1, "{f:?}");
    }

    #[test]
    fn local_data_reaching_a_sink_is_not_a_flow() {
        let f = flows("a.js", r#"const vm = require("vm"); vm.runInThisContext("1");"#);
        assert!(f.is_empty());
        let f = flows("a.js", "const c = 'x'; eval(c); fetch(u);");
        assert!(f.is_empty());
    }

    #[test]
    fn file_data_is_not_network_data() {
        let f = flows("a.js", r#"const fs = require("fs"); fs.readFile(p, (e, d) => eval(d));"#);
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn a_flow_does_not_replace_the_unresolved_code_fact() {
        let a = analyze_file_full(
            &RelPath::new("a.js").unwrap(),
            "async function f() { const t = await (await fetch(u)).text(); eval(t); }",
            &Catalog::node_defaults(),
        )
        .unwrap();
        assert_eq!(a.flows.len(), 1);
        assert!(reasons(&a.file).contains(&UnresolvedReason::Eval));
    }

    #[test]
    fn a_file_with_syntax_errors_has_no_flows() {
        let a = analyze_file_full(
            &RelPath::new("a.js").unwrap(),
            "fetch(u).then(t => eval(t)",
            &Catalog::node_defaults(),
        )
        .unwrap();
        assert!(a.flows.is_empty());
    }

    #[test]
    fn a_flow_is_a_valid_fact_at_the_sink_with_its_origin() {
        let src = "async function f() { const t = await (await fetch(u)).text(); eval(t); }";
        let f = run("a.js", src);
        let flow: Vec<_> = f.facts.iter().filter(|x| x.kind() == FactKind::Flow).collect();
        assert_eq!(flow.len(), 1);
        let fact = flow[0];
        assert!(fact.validate().is_empty());
        assert_eq!(fact.detail().map(|d| d.text()), Some("fetch(u)"));
        let at = fact.at();
        assert_eq!(&src[at.start_byte as usize..at.end_byte as usize], "eval(t)");
        let o = fact.origin.as_ref().unwrap();
        assert_eq!(&src[o.start_byte as usize..o.end_byte as usize], "fetch(u)");
    }

    #[test]
    fn flow_facts_have_distinct_ids_and_sit_beside_the_other_facts() {
        let f = run(
            "a.js",
            "async function f() { const t = await (await fetch(u)).text(); eval(t); eval(t); }",
        );
        assert_eq!(count(&f, FactKind::Flow), 2);
        assert_eq!(count(&f, FactKind::Effect), 3); // fetch + two evals
        let mut ids: Vec<_> = f.facts.iter().map(|x| x.id().clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), f.facts.len());
        assert!(f.facts.iter().all(|x| x.validate().is_empty()));
    }

    #[test]
    fn no_flow_means_no_flow_facts() {
        let f = run("a.js", "const c = 'x'; eval(c); fetch('https://example.com');");
        assert_eq!(count(&f, FactKind::Flow), 0);
    }

    // ---- timers ----

    fn timer(f: &FileImports) -> Option<Timing> {
        f.facts.iter().find(|x| x.kind() == FactKind::Timer).and_then(|x| x.timing)
    }

    fn timing(lane: Lane, delay_ms: Delay, cleared: Option<bool>) -> Option<Timing> {
        Some(Timing { lane, delay_ms, cleared })
    }

    #[test]
    fn set_timeout_is_a_deferred_timer_with_a_known_delay() {
        let f = run("a.js", "setTimeout(() => {}, 1000);");
        assert_eq!(timer(&f), timing(Lane::Deferred, Delay::Known(1000), Some(false)));
        assert_eq!(count(&f, FactKind::Call), 1);
        assert!(effect(&f).is_none());
        assert!(reasons(&f).is_empty());
        assert!(f.facts.iter().all(|x| x.validate().is_empty()));
    }

    #[test]
    fn set_interval_is_recurring_and_a_cleared_handle_is_noted() {
        let f = run("a.js", "const h = setInterval(f, 5000); clearInterval(h);");
        assert_eq!(timer(&f), timing(Lane::Recurring, Delay::Known(5000), Some(true)));
    }

    #[test]
    fn set_immediate_is_the_immediate_lane() {
        let f = run("a.js", "setImmediate(f);");
        assert_eq!(timer(&f), timing(Lane::Immediate, Delay::Known(0), Some(false)));
    }

    #[test]
    fn delays_follow_what_node_does() {
        assert_eq!(timer(&run("a.js", "setTimeout(f);")).unwrap().delay_ms, Delay::Known(1));
        assert_eq!(timer(&run("a.js", "setTimeout(f, 0);")).unwrap().delay_ms, Delay::Known(1));
        assert_eq!(timer(&run("a.js", "setTimeout(f, 5.7);")).unwrap().delay_ms, Delay::Known(5));
        assert_eq!(
            timer(&run("a.js", "setTimeout(f, 3000000000);")).unwrap().delay_ms,
            Delay::Known(1)
        );
    }

    #[test]
    fn a_computed_delay_is_unresolved_and_reported() {
        let f = run("a.js", "setTimeout(f, n);");
        assert_eq!(timer(&f).unwrap().delay_ms, Delay::Unresolved);
        assert_eq!(reasons(&f), vec![UnresolvedReason::RuntimeDelay]);
    }

    #[test]
    fn an_escaping_handle_is_unknown_and_an_unused_one_is_not_cleared() {
        let f = run("a.js", "const h = setTimeout(f, 1); register(h);");
        assert_eq!(timer(&f).unwrap().cleared, None);
        let f = run("a.js", "const h = setTimeout(f, 1);");
        assert_eq!(timer(&f).unwrap().cleared, Some(false));
        let f = run("a.js", "function g() { return setInterval(f, 1); }");
        assert_eq!(timer(&f).unwrap().cleared, None);
    }

    #[test]
    fn a_string_callback_is_a_code_load() {
        let f = run("a.js", r#"setTimeout("go()", 10);"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, Some("go()".into()))));
        assert_eq!(timer(&f).unwrap().lane, Lane::Deferred);
        assert!(reasons(&f).is_empty());
    }

    #[test]
    fn a_built_string_callback_is_a_code_load_reported_as_computed() {
        let f = run("a.js", "setTimeout(`go(${x})`, 10);");
        assert_eq!(effect(&f), Some((Effect::CodeLoad, None)));
        assert_eq!(reasons(&f), vec![UnresolvedReason::Eval]);
        let f = run("a.js", r#"setTimeout("a" + b, 10);"#);
        assert_eq!(effect(&f), Some((Effect::CodeLoad, None)));
    }

    #[test]
    fn a_callback_we_cannot_see_is_reported() {
        let f = run("a.js", "setTimeout(getCb(), 10);");
        assert_eq!(reasons(&f), vec![UnresolvedReason::EventHandler]);
        assert!(effect(&f).is_none());
    }

    #[test]
    fn timers_are_found_through_every_route() {
        assert!(timer(&run("a.js", "globalThis.setTimeout(f, 5);")).is_some());
        assert!(timer(&run("a.js", "const t = setTimeout; t(f, 5);")).is_some());
        assert!(timer(&run("a.js", r#"require("timers").setTimeout(f, 5);"#)).is_some());
        assert!(timer(&run("a.js", r#"import { setInterval as si } from "timers"; si(f, 5);"#))
            .is_some());
        assert!(timer(&run("a.js", r#"const { setImmediate: si } = require("timers"); si(f);"#))
            .is_some());
    }

    #[test]
    fn a_local_set_timeout_is_not_trusted() {
        let f = run("a.js", "function g(setTimeout) { setTimeout(f, 1); }");
        assert!(timer(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::AmbiguousBinding]);
    }

    #[test]
    fn a_timer_used_as_a_value_is_reported() {
        let f = run("a.js", "run(setTimeout);");
        assert_eq!(reasons(&f), vec![UnresolvedReason::IndirectUse]);
    }

    #[test]
    fn network_text_run_by_a_timer_is_a_flow() {
        let f = flows("a.js", "fetch(u).then(t => setTimeout(t, 0));");
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].0.starts_with("setTimeout(t"));
    }

    #[test]
    fn a_function_callback_or_a_tainted_delay_is_not_a_flow() {
        let f = flows("a.js", "fetch(u).then(t => setTimeout(() => go(), t));");
        assert!(f.is_empty(), "{f:?}");
        let f = flows("a.js", "fetch(u); setTimeout(go, 10);");
        assert!(f.is_empty(), "{f:?}");
    }

    // ---- more catalog coverage ----

    #[test]
    fn fs_promises_calls_match_the_catalog() {
        let f = run("a.js", r#"import { readFile } from "fs/promises"; readFile("/etc/passwd");"#);
        assert_eq!(effect(&f), Some((Effect::FsRead, Some("/etc/passwd".into()))));
        let f = run(
            "a.js",
            r#"const fsp = require("fs/promises"); async function f() { await fsp.unlink("x"); }"#,
        );
        assert_eq!(effect(&f), Some((Effect::FsDelete, Some("x".into()))));
        assert!(reasons(&f).is_empty());
    }

    #[test]
    fn the_node_prefix_reaches_the_same_entries() {
        let f = run("a.js", r#"import { writeFile } from "node:fs/promises"; writeFile("a", "b");"#);
        assert_eq!(effect(&f), Some((Effect::FsWrite, Some("a".into()))));
    }

    #[test]
    fn an_fs_promises_member_we_do_not_catalogue_is_reported() {
        let f = run("a.js", r#"import { open } from "fs/promises"; open("x");"#);
        assert!(effect(&f).is_none());
        assert_eq!(reasons(&f), vec![UnresolvedReason::UncatalogedMember]);
    }

    #[test]
    fn a_worker_runs_code_from_a_path() {
        let f = run("a.js", r#"const { Worker } = require("worker_threads"); new Worker("./w.js");"#);
        assert_eq!(effect(&f), Some((Effect::DynamicLoad, Some("./w.js".into()))));
        let f = run("a.js", r#"const { Worker } = require("worker_threads"); new Worker(p);"#);
        assert_eq!(effect(&f), Some((Effect::DynamicLoad, None)));
        assert_eq!(reasons(&f), vec![UnresolvedReason::ComputedAccess]);
    }

    #[test]
    fn network_data_reaching_a_worker_is_a_flow() {
        let f = flows(
            "a.js",
            r#"const { Worker } = require("worker_threads");
               async function f() { const t = await (await fetch(u)).text(); new Worker(t); }"#,
        );
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].0.starts_with("new Worker(t)"));
    }

    #[test]
    fn http2_and_tls_connections_are_network_effects() {
        let f = run("a.js", r#"const h2 = require("http2"); h2.connect("https://example.com");"#);
        assert_eq!(effect(&f), Some((Effect::NetOut, Some("https://example.com".into()))));
        let f = run("a.js", r#"const tls = require("tls"); tls.connect(443, "example.com");"#);
        assert_eq!(effect(&f).map(|e| e.0), Some(Effect::NetOut));
    }
}
