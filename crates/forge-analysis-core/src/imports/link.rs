//! Joins the per-file results into one project view.
//!
//! Pure: it works on plain data (`LinkInfo` per file plus the list of files in the folder)
//! and never touches the disk or the parser.
//!
//! What it links:
//!   * an import or `require` of a file in the folder: the importer's function reaches that
//!     file's top level, which runs when it is first loaded, so its effects and lanes follow;
//!   * a call to an imported name that is a function in that file (through re-exports);
//! and for everything else it says why not (`UnlinkedReason`), never dropping it.

use forge_analysis_schema::{
    Effect, EntryBasis, EntryIssue, FnId, FnRef, FollowedCall, Lane, Linked, LinkedFn, Quoted, RelPath,
    Unlinked, UnlinkedReason,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExportName {
    Default,
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportTarget {
    /// A function in this file.
    Func(u32),
    /// `export { a as b } from "m"`: another module's export, under a new name.
    Reexport { spec: String, name: ExportName },
    /// Exported, but not as a function we can identify (a value, a class, a namespace).
    Opaque,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exports {
    pub names: BTreeMap<ExportName, ExportTarget>,
    /// `export * from "m"`
    pub stars: Vec<String>,
    /// Something exports in a way we cannot enumerate (spread, computed keys, `module` used
    /// as a value), so a missing name is "unknown", not "absent".
    pub dynamic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    Static,
    Dynamic,
    Require,
}

/// "Function `from` loads module `spec`."
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportEdge {
    pub from: u32,
    pub spec: String,
    pub kind: ImportKind,
}

/// "Function `from` calls the export `name` of module `spec`."
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtCall {
    pub from: u32,
    /// Byte range of the call expression in its file.
    pub site: (u32, u32),
    pub spec: String,
    pub name: ExportName,
}

/// The call graph inside one file, in the shape the linker needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalEdges {
    pub calls: Vec<(u32, u32)>,
    /// `(registrar, callback, lane)`: the lane is the timer's, when the receiving call is one.
    pub callbacks: Vec<(u32, u32, Option<Lane>)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkInfo {
    pub exports: Exports,
    pub imports: Vec<ImportEdge>,
    pub ext_calls: Vec<ExtCall>,
    pub edges: LocalEdges,
    /// Per function: the effects it performs directly.
    pub direct: Vec<BTreeSet<Effect>>,
}

pub struct LinkFile<'a> {
    pub path: &'a RelPath,
    pub info: &'a LinkInfo,
}

const NODE_BUILTINS: &[&str] = &[
    "assert", "async_hooks", "buffer", "child_process", "cluster", "console", "constants",
    "crypto", "dgram", "diagnostics_channel", "dns", "domain", "events", "fs", "http", "http2",
    "https", "inspector", "module", "net", "os", "path", "perf_hooks", "process", "punycode",
    "querystring", "readline", "repl", "stream", "string_decoder", "sys", "timers", "tls",
    "trace_events", "tty", "url", "util", "v8", "vm", "wasi", "worker_threads", "zlib", "test",
];

const EXTS: &[&str] =
    &[".js", ".mjs", ".cjs", ".ts", ".tsx", ".jsx", ".mts", ".cts", ".json", ".node"];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Res {
    File(usize),
    NotAnalysed,
    Missing,
    Outside,
    Package,
    Subpath,
    Builtin,
}

fn norm(path: &str) -> String {
    path.replace('\\', "/")
}

fn is_builtin(spec: &str) -> bool {
    let s = spec.strip_prefix("node:").unwrap_or(spec);
    let first = s.split('/').next().unwrap_or(s);
    spec.starts_with("node:") || NODE_BUILTINS.contains(&first)
}

struct Resolver<'a> {
    analysed: BTreeMap<String, usize>,
    all: &'a BTreeSet<String>,
}

impl Resolver<'_> {
    fn hit(&self, p: &str) -> Option<Res> {
        if let Some(&i) = self.analysed.get(p) {
            return Some(Res::File(i));
        }
        if self.all.contains(p) {
            return Some(Res::NotAnalysed);
        }
        None
    }

    fn resolve(&self, from: &str, spec: &str) -> Res {
        if !(spec.starts_with("./") || spec.starts_with("../") || spec == "." || spec == "..") {
            if spec.starts_with('/') {
                return Res::Outside;
            }
            if spec.starts_with('#') {
                return Res::Subpath;
            }
            return if is_builtin(spec) { Res::Builtin } else { Res::Package };
        }
        let mut parts: Vec<&str> = match from.rfind('/') {
            Some(i) => from[..i].split('/').collect(),
            None => Vec::new(),
        };
        for seg in spec.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Res::Outside;
                    }
                }
                s => parts.push(s),
            }
        }
        let base = parts.join("/");
        if base.is_empty() {
            return Res::Missing;
        }
        if let Some(r) = self.hit(&base) {
            return r;
        }
        // TypeScript writes the compiled extension in imports of its own sources.
        for (js, ts) in [(".js", [".ts", ".tsx"].as_slice()), (".mjs", &[".mts"]), (".cjs", &[".cts"]), (".jsx", &[".tsx"])] {
            if let Some(stem) = base.strip_suffix(js) {
                for t in ts {
                    if let Some(r) = self.hit(&format!("{stem}{t}")) {
                        return r;
                    }
                }
            }
        }
        for e in EXTS {
            if let Some(r) = self.hit(&format!("{base}{e}")) {
                return r;
            }
        }
        for e in EXTS {
            if let Some(r) = self.hit(&format!("{base}/index{e}")) {
                return r;
            }
        }
        Res::Missing
    }
}

fn reason_of(r: &Res) -> Option<UnlinkedReason> {
    match r {
        Res::File(_) | Res::Builtin => None,
        Res::NotAnalysed => Some(UnlinkedReason::NotAnalysed),
        Res::Missing => Some(UnlinkedReason::MissingFile),
        Res::Outside => Some(UnlinkedReason::OutsideRoot),
        Res::Package => Some(UnlinkedReason::ExternalPackage),
        Res::Subpath => Some(UnlinkedReason::SubpathImport),
    }
}

/// Follows an export name to the function it ends at.
fn find_export(
    files: &[LinkFile<'_>],
    paths: &[String],
    rs: &Resolver<'_>,
    file: usize,
    name: &ExportName,
    seen: &mut BTreeSet<(usize, ExportName)>,
) -> Result<(usize, u32), UnlinkedReason> {
    if !seen.insert((file, name.clone())) {
        return Err(UnlinkedReason::ReexportCycle);
    }
    let ex = &files[file].info.exports;
    match ex.names.get(name) {
        Some(ExportTarget::Func(id)) => return Ok((file, *id)),
        Some(ExportTarget::Opaque) => return Err(UnlinkedReason::OpaqueExport),
        Some(ExportTarget::Reexport { spec, name: inner }) => {
            return match rs.resolve(&paths[file], spec) {
                Res::File(j) => find_export(files, paths, rs, j, inner, seen),
                Res::Builtin => Err(UnlinkedReason::OpaqueExport),
                other => Err(reason_of(&other).unwrap_or(UnlinkedReason::OpaqueExport)),
            };
        }
        None => {}
    }
    let mut unknown = ex.dynamic;
    if matches!(name, ExportName::Named(_)) {
        for spec in &ex.stars {
            match rs.resolve(&paths[file], spec) {
                Res::File(j) => match find_export(files, paths, rs, j, name, seen) {
                    Ok(hit) => return Ok(hit),
                    Err(UnlinkedReason::NoSuchExport) => {}
                    Err(UnlinkedReason::ReexportCycle) => {}
                    Err(_) => unknown = true,
                },
                _ => unknown = true,
            }
        }
    }
    Err(if unknown { UnlinkedReason::OpaqueExport } else { UnlinkedReason::NoSuchExport })
}

#[derive(Clone, Copy)]
struct Out {
    to: usize,
    lane: Option<Lane>,
    callback: bool,
}

pub fn link(files: &[LinkFile<'_>], all_files: &BTreeSet<String>, entries: &[usize]) -> Linked {
    link_seeded(files, all_files, entries, &[])
}

/// A declared entry point: a file named by a `package.json` field or script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryWant {
    /// The `package.json` that declares it; the path is read relative to its folder.
    pub from: RelPath,
    /// Which declaration, for the report: `main`, `bin.tool`, `scripts.postinstall`.
    pub label: String,
    pub spec: String,
    /// Runs at install time rather than when the package is used.
    pub install: bool,
    /// A guess that need not exist (Node's default `index.js`): no issue when it is absent.
    pub optional: bool,
}

/// `link` with entry points taken from `package.json` files. A declaration that does not tie
/// to an analysed file is reported in `entry_issues`; if none tie at all the entries fall back
/// to roots, exactly as with no declarations.
pub fn link_project(
    files: &[LinkFile<'_>],
    all_files: &BTreeSet<String>,
    wants: &[EntryWant],
) -> Linked {
    let paths: Vec<String> = files.iter().map(|f| norm(f.path.as_quoted().text())).collect();
    let rs = Resolver {
        analysed: paths.iter().cloned().enumerate().map(|(i, p)| (p, i)).collect(),
        all: all_files,
    };
    let (mut run, mut install) = (BTreeSet::new(), BTreeSet::new());
    let mut issues: Vec<(String, String, String, UnlinkedReason)> = Vec::new();
    let mut seen_issue = BTreeSet::new();
    for w in wants {
        let from = norm(w.from.as_quoted().text());
        let spec = if w.spec.starts_with("./") || w.spec.starts_with("../") || w.spec.starts_with('/') {
            w.spec.clone()
        } else {
            format!("./{}", w.spec)
        };
        match rs.resolve(&from, &spec) {
            Res::File(i) => {
                if w.install { install.insert(i) } else { run.insert(i) };
            }
            _ if w.optional => {}
            other => {
                let reason = reason_of(&other).unwrap_or(UnlinkedReason::MissingFile);
                let key = (from.clone(), w.label.clone(), w.spec.clone(), reason);
                if seen_issue.insert(key.clone()) {
                    issues.push(key);
                }
            }
        }
    }
    let run: Vec<usize> = run.into_iter().collect();
    let install: Vec<usize> = install.into_iter().collect();
    let mut linked = link_seeded(files, all_files, &run, &install);
    issues.sort();
    linked.entry_issues = issues
        .into_iter()
        .filter_map(|(from, label, spec, reason)| {
            let from = wants.iter().find(|w| norm(w.from.as_quoted().text()) == from)?.from.clone();
            Some(EntryIssue { from, label: Quoted::new(&label), spec: Quoted::new(&spec), reason })
        })
        .collect();
    linked
}

/// `link` with a second set of entries that run at install time (`Lane::Install`).
pub fn link_seeded(
    files: &[LinkFile<'_>],
    all_files: &BTreeSet<String>,
    entries: &[usize],
    install_entries: &[usize],
) -> Linked {
    let paths: Vec<String> = files.iter().map(|f| norm(f.path.as_quoted().text())).collect();
    let rs = Resolver {
        analysed: paths.iter().cloned().enumerate().map(|(i, p)| (p, i)).collect(),
        all: all_files,
    };

    let mut offset = Vec::with_capacity(files.len() + 1);
    let mut total = 0usize;
    for f in files {
        offset.push(total);
        total += f.info.direct.len().max(1);
    }
    offset.push(total);

    let node = |file: usize, func: u32| -> Option<usize> {
        let n = offset[file + 1] - offset[file];
        ((func as usize) < n).then(|| offset[file] + func as usize)
    };

    let mut out: Vec<Vec<Out>> = vec![Vec::new(); total];
    let mut unlinked: Vec<(UnlinkedReason, usize, u32, String, Option<String>)> = Vec::new();
    let mut links_made = 0u32;
    let mut followed: Vec<FollowedCall> = Vec::new();
    let mut file_edges: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); files.len()];
    let mut seen_edge: BTreeSet<(usize, usize, bool)> = BTreeSet::new();
    let mut add = |out: &mut Vec<Vec<Out>>, a: usize, o: Out| {
        if seen_edge.insert((a, o.to, o.callback)) || o.lane.is_some() {
            out[a].push(o);
        }
    };

    for (fi, f) in files.iter().enumerate() {
        for &(a, b) in &f.info.edges.calls {
            if let (Some(x), Some(y)) = (node(fi, a), node(fi, b)) {
                add(&mut out, x, Out { to: y, lane: None, callback: false });
            }
        }
        for &(a, b, lane) in &f.info.edges.callbacks {
            if let (Some(x), Some(y)) = (node(fi, a), node(fi, b)) {
                out[x].push(Out { to: y, lane, callback: true });
            }
        }
        for imp in &f.info.imports {
            let res = rs.resolve(&paths[fi], &imp.spec);
            match res {
                Res::File(j) => {
                    if let (Some(x), Some(y)) = (node(fi, imp.from), node(j, 0)) {
                        add(&mut out, x, Out { to: y, lane: None, callback: false });
                        links_made += 1;
                        if j != fi {
                            file_edges[fi].insert(j);
                        }
                    }
                }
                ref other => {
                    if let Some(r) = reason_of(other) {
                        unlinked.push((r, fi, imp.from, imp.spec.clone(), None));
                    }
                }
            }
        }
        for c in &f.info.ext_calls {
            let Res::File(j) = rs.resolve(&paths[fi], &c.spec) else { continue };
            let mut seen = BTreeSet::new();
            match find_export(files, &paths, &rs, j, &c.name, &mut seen) {
                Ok((tf, tfn)) => {
                    if let (Some(x), Some(y)) = (node(fi, c.from), node(tf, tfn)) {
                        add(&mut out, x, Out { to: y, lane: None, callback: false });
                        links_made += 1;
                        followed.push(FollowedCall {
                            from: FnRef { file: fi as u32, func: FnId(c.from) },
                            to: FnRef { file: tf as u32, func: FnId(tfn) },
                            start_byte: c.site.0,
                            end_byte: c.site.1,
                            spec: Quoted::new(&c.spec),
                            name: Quoted::new(&match &c.name {
                                ExportName::Default => "default".to_string(),
                                ExportName::Named(n) => n.clone(),
                            }),
                        });
                    }
                }
                Err(r) => {
                    let name = match &c.name {
                        ExportName::Default => "default".to_string(),
                        ExportName::Named(n) => n.clone(),
                    };
                    unlinked.push((r, fi, c.from, c.spec.clone(), Some(name)));
                }
            }
        }
    }

    // Entry points: the ones named, else roots.
    let (basis, entries): (EntryBasis, Vec<usize>) = if entries.is_empty() {
        (EntryBasis::Roots, roots(files.len(), &file_edges))
    } else {
        let mut e: Vec<usize> = entries.iter().copied().filter(|&i| i < files.len()).collect();
        e.sort_unstable();
        e.dedup();
        (EntryBasis::Declared, e)
    };

    // Lanes: a worklist over the project graph.
    let mut lanes: Vec<BTreeSet<Lane>> = vec![BTreeSet::new(); total];
    let mut queue: VecDeque<usize> = VecDeque::new();
    for &e in &entries {
        let n = offset[e];
        if lanes[n].insert(Lane::Immediate) {
            queue.push_back(n);
        }
    }
    let mut install: Vec<usize> = install_entries.iter().copied().filter(|&i| i < files.len()).collect();
    install.sort_unstable();
    install.dedup();
    for &e in &install {
        let n = offset[e];
        if lanes[n].insert(Lane::Install) {
            queue.push_back(n);
        }
    }
    while let Some(from) = queue.pop_front() {
        let here: Vec<Lane> = lanes[from].iter().copied().collect();
        for o in out[from].clone() {
            let add: Vec<Lane> = match (o.callback, o.lane) {
                (true, Some(l)) => vec![l],
                _ => here.clone(),
            };
            let mut changed = false;
            for l in add {
                changed |= lanes[o.to].insert(l);
            }
            if changed {
                queue.push_back(o.to);
            }
        }
    }

    // Effects: each function gets what the functions it reaches do.
    let mut effects: Vec<BTreeSet<Effect>> = vec![BTreeSet::new(); total];
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); total];
    for (a, outs) in out.iter().enumerate() {
        for o in outs {
            preds[o.to].push(a);
        }
    }
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (fi, f) in files.iter().enumerate() {
        for (k, d) in f.info.direct.iter().enumerate() {
            let n = offset[fi] + k;
            effects[n] = d.clone();
            if !d.is_empty() {
                queue.push_back(n);
            }
        }
    }
    while let Some(n) = queue.pop_front() {
        let eff: Vec<Effect> = effects[n].iter().copied().collect();
        for &p in &preds[n] {
            let mut changed = false;
            for e in &eff {
                changed |= effects[p].insert(*e);
            }
            if changed {
                queue.push_back(p);
            }
        }
    }

    let mut fns = Vec::with_capacity(total);
    for (fi, f) in files.iter().enumerate() {
        let n = f.info.direct.len().max(1);
        for k in 0..n {
            let g = offset[fi] + k;
            fns.push(LinkedFn {
                at: FnRef { file: fi as u32, func: FnId(k as u32) },
                transitive_effects: effects[g].iter().copied().collect(),
                lanes: lanes[g].iter().copied().collect(),
                reachable_from_entry: !lanes[g].is_empty(),
            });
        }
    }

    unlinked.sort();
    let unlinked = unlinked
        .into_iter()
        .map(|(reason, fi, func, spec, name)| Unlinked {
            from: FnRef { file: fi as u32, func: FnId(func) },
            spec: Quoted::new(&spec),
            name: name.map(|n| Quoted::new(&n)),
            reason,
        })
        .collect();

    Linked {
        files: files.iter().map(|f| f.path.clone()).collect(),
        entry_basis: basis,
        entries: entries.iter().map(|&e| e as u32).collect(),
        install_entries: install.iter().map(|&e| e as u32).collect(),
        entry_issues: Vec::new(),
        fns,
        unlinked,
        followed_calls: followed,
        links_made,
    }
}

/// Files nothing else imports; then, for any import cycle nothing outside reaches, its
/// lowest-numbered file. Deterministic.
fn roots(n: usize, edges: &[BTreeSet<usize>]) -> Vec<usize> {
    let mut indeg = vec![0u32; n];
    for e in edges {
        for &t in e {
            indeg[t] += 1;
        }
    }
    let mut roots: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    loop {
        let mut reached = vec![false; n];
        let mut stack = roots.clone();
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut reached[i], true) {
                continue;
            }
            stack.extend(edges[i].iter().copied());
        }
        match (0..n).find(|&i| !reached[i]) {
            Some(i) => {
                roots.push(i);
                roots.sort_unstable();
            }
            None => return roots,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::imports::callgraph;
    use crate::imports::exports;
    use crate::imports::summary::{link_info, Bound};
    use forge_analysis_schema::{Delay, FactKind, StaticFact, Timing};
    use oxc_allocator::Allocator;
    use oxc_ast::ast::{
        BindingPattern, Expression, ImportDeclarationSpecifier, Program, PropertyKey, Statement,
    };
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::{GetSpan, SourceType};

    /// Stand-in for the per-file import table: top-level `import` and `require` bindings.
    fn binder(src: &str, program: &Program<'_>) -> BTreeMap<u32, (String, Bound)> {
        let lit = |s: oxc_span::Span| src[s.start as usize + 1..s.end as usize - 1].to_string();
        let mut m = BTreeMap::new();
        for st in &program.body {
            match st {
                Statement::ImportDeclaration(d) => {
                    let spec = lit(d.source.span);
                    for s in d.specifiers.iter().flatten() {
                        match s {
                            ImportDeclarationSpecifier::ImportSpecifier(i) => {
                                let n = src[i.imported.span().start as usize..i.imported.span().end as usize].to_string();
                                m.insert(i.local.span.start, (spec.clone(), Bound::Named(n)));
                            }
                            ImportDeclarationSpecifier::ImportDefaultSpecifier(i) => {
                                m.insert(i.local.span.start, (spec.clone(), Bound::Default));
                            }
                            ImportDeclarationSpecifier::ImportNamespaceSpecifier(i) => {
                                m.insert(i.local.span.start, (spec.clone(), Bound::Namespace));
                            }
                        }
                    }
                }
                Statement::VariableDeclaration(v) => {
                    for d in &v.declarations {
                        let Some(Expression::CallExpression(c)) = &d.init else { continue };
                        let Some(oxc_ast::ast::Argument::StringLiteral(l)) = c.arguments.first() else { continue };
                        let spec = lit(l.span);
                        match &d.id {
                            BindingPattern::BindingIdentifier(b) => {
                                m.insert(b.span.start, (spec, Bound::Namespace));
                            }
                            BindingPattern::ObjectPattern(o) => {
                                for p in &o.properties {
                                    if let (PropertyKey::StaticIdentifier(k), BindingPattern::BindingIdentifier(b)) = (&p.key, &p.value) {
                                        let n = src[k.span.start as usize..k.span.end as usize].to_string();
                                        m.insert(b.span.start, (spec.clone(), Bound::Named(n)));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        m
    }

    /// Facts are placed by text: `DEL` is an FsDelete effect; a `setTimeout(...)` call is a
    /// deferred timer.
    pub(crate) fn info(path: &str, src: &str) -> LinkInfo {
        let alloc = Allocator::default();
        let ret = Parser::new(&alloc, src, SourceType::from_path(path).unwrap()).parse();
        assert!(ret.diagnostics.is_empty(), "{path}: {:?}", ret.diagnostics);
        let sem = SemanticBuilder::new().build(&ret.program);
        let g = callgraph::build(&ret.program, sem.semantic.scoping());
        let ex = exports::extract(&ret.program, sem.semantic.scoping(), &g, src);
        let rel = RelPath::new(path).unwrap();
        let sp = |s: oxc_span::Span| forge_analysis_schema::Span {
            file: rel.clone(),
            start_line: 1,
            end_line: 1,
            start_byte: s.start,
            end_byte: s.end,
        };
        let mut facts = Vec::new();
        let mut from = 0;
        while let Some(i) = src[from..].find("DEL") {
            let s = (from + i) as u32;
            let mut f = StaticFact::new(FactKind::Effect, sp(oxc_span::Span::new(s, s + 3)), None);
            f.effect = Some(Effect::FsDelete);
            facts.push(f);
            from = from + i + 3;
        }
        let mut from = 0;
        while let Some(i) = src[from..].find("setTimeout(") {
            let s = from + i;
            let (mut depth, mut e) = (0, s);
            for (k, c) in src[s..].char_indices() {
                if c == '(' { depth += 1 }
                if c == ')' { depth -= 1; if depth == 0 { e = s + k + 1; break } }
            }
            let mut f = StaticFact::new(FactKind::Timer, sp(oxc_span::Span::new(s as u32, e as u32)), None);
            f.timing = Some(Timing { lane: Lane::Deferred, delay_ms: Delay::Known(5), cleared: None });
            facts.push(f);
            from = e;
        }
        let b = binder(src, &ret.program);
        link_info(
            &g,
            &facts,
            ex,
            &|d| b.get(&d).cloned(),
            &|s| src[s.start as usize..s.end as usize].to_string(),
        )
    }

    fn project(files: &[(&str, &str)], extra: &[&str], entries: &[usize]) -> Linked {
        let rels: Vec<RelPath> = files.iter().map(|(p, _)| RelPath::new(p).unwrap()).collect();
        let infos: Vec<LinkInfo> = files.iter().map(|(p, s)| info(p, s)).collect();
        let lf: Vec<LinkFile<'_>> =
            rels.iter().zip(&infos).map(|(path, info)| LinkFile { path, info }).collect();
        let mut all: BTreeSet<String> = files.iter().map(|(p, _)| p.to_string()).collect();
        all.extend(extra.iter().map(|s| s.to_string()));
        link(&lf, &all, entries)
    }

    fn top(l: &Linked, file: u32) -> &LinkedFn {
        l.fns.iter().find(|f| f.at.file == file && f.at.func == FnId(0)).unwrap()
    }

    fn reasons(l: &Linked) -> Vec<UnlinkedReason> {
        l.unlinked.iter().map(|u| u.reason).collect()
    }

    const DELETE_IN_B: &str = "export function wipe(){ DEL; } export function other(){ DEL; }";

    #[test]
    fn a_call_to_an_imported_function_brings_its_effects() {
        let l = project(
            &[("a.js", "import { wipe } from './b.js'; wipe();"), ("b.js", "export function wipe(){ DEL; }")],
            &[],
            &[],
        );
        assert_eq!(top(&l, 0).transitive_effects, vec![Effect::FsDelete]);
        assert!(l.unlinked.is_empty(), "{:?}", l.unlinked);
    }

    #[test]
    fn importing_a_file_runs_its_top_level() {
        let l = project(&[("a.js", "import './b.js';"), ("b.js", "DEL;")], &[], &[]);
        assert_eq!(top(&l, 0).transitive_effects, vec![Effect::FsDelete]);
        assert_eq!(top(&l, 1).lanes, vec![Lane::Immediate]);
    }

    #[test]
    fn an_import_that_is_never_called_does_not_pull_in_the_files_functions() {
        let l = project(&[("a.js", "import './b.js';"), ("b.js", DELETE_IN_B)], &[], &[]);
        assert!(top(&l, 0).transitive_effects.is_empty());
    }

    #[test]
    fn a_require_inside_a_timer_callback_loads_the_file_deferred() {
        let l = project(
            &[("a.js", "setTimeout(() => { require('./b.js'); }, 5);"), ("b.js", "DEL;")],
            &[],
            &[],
        );
        assert_eq!(top(&l, 1).lanes, vec![Lane::Deferred]);
        assert_eq!(top(&l, 0).transitive_effects, vec![Effect::FsDelete]);
    }

    #[test]
    fn a_function_called_from_two_lanes_in_another_file_has_both() {
        let l = project(
            &[
                ("a.js", "import { w } from './b.js'; w(); setTimeout(() => { w(); }, 5);"),
                ("b.js", "export function w(){ DEL; }"),
            ],
            &[],
            &[],
        );
        let w = l.fns.iter().find(|f| f.at.file == 1 && f.at.func != FnId(0)).unwrap();
        assert_eq!(w.lanes, vec![Lane::Immediate, Lane::Deferred]);
    }

    #[test]
    fn roots_are_files_nothing_imports() {
        let l = project(
            &[("a.js", "import './b.js';"), ("b.js", ""), ("c.js", "")],
            &[],
            &[],
        );
        assert_eq!(l.entry_basis, EntryBasis::Roots);
        assert_eq!(l.entries, vec![0, 2]);
    }

    #[test]
    fn an_import_cycle_nothing_reaches_still_gets_a_root() {
        let l = project(&[("a.js", "import './b.js';"), ("b.js", "import './a.js'; DEL;")], &[], &[]);
        assert_eq!(l.entries, vec![0]);
        assert_eq!(top(&l, 0).transitive_effects, vec![Effect::FsDelete]);
        assert!(top(&l, 1).reachable_from_entry);
    }

    #[test]
    fn declared_entries_decide_what_is_reachable() {
        let l = project(&[("a.js", ""), ("b.js", "DEL;")], &[], &[0]);
        assert_eq!(l.entry_basis, EntryBasis::Declared);
        assert!(!top(&l, 1).reachable_from_entry);
        assert!(top(&l, 1).lanes.is_empty());
        assert_eq!(top(&l, 1).transitive_effects, vec![Effect::FsDelete]);
    }

    #[test]
    fn packages_are_reported_and_builtins_are_not() {
        let l = project(
            &[("a.js", "import fs from 'fs'; import p from 'node:path'; import _ from 'lodash'; import s from '@scope/pkg/sub';")],
            &[],
            &[],
        );
        assert_eq!(reasons(&l), vec![UnlinkedReason::ExternalPackage; 2]);
    }

    #[test]
    fn missing_outside_and_unanalysed_files_are_reported() {
        let l = project(
            &[(
                "src/a.js",
                "import './gone.js'; import '../../up.js'; import '/abs.js'; import './data.json';",
            )],
            &["src/data.json"],
            &[],
        );
        let mut r = reasons(&l);
        r.sort();
        assert_eq!(
            r,
            vec![
                UnlinkedReason::MissingFile,
                UnlinkedReason::OutsideRoot,
                UnlinkedReason::OutsideRoot,
                UnlinkedReason::NotAnalysed,
            ]
        );
    }

    #[test]
    fn a_followed_call_records_where_the_call_is_and_what_it_ran() {
        let a = "const { add } = require('./b');\nadd(1, 2);\nconst x = 1;";
        let l = project(&[("a.js", a), ("b.js", "exports.add = function add(x, y) { return x + y; };")], &[], &[]);
        assert_eq!(l.followed_calls.len(), 1, "{:?}", l.followed_calls);
        let c = &l.followed_calls[0];
        assert_eq!((c.from.file, c.to.file), (0, 1));
        assert_eq!(&a[c.start_byte as usize..c.end_byte as usize], "add(1, 2)");
        assert_eq!((c.spec.text(), c.name.text()), ("./b", "add"));
    }

    #[test]
    fn a_call_that_cannot_be_followed_is_not_recorded_as_followed() {
        let l = project(&[("a.js", "const { nope } = require('./b'); nope();"), ("b.js", "exports.other = 1;")], &[], &[]);
        assert!(l.followed_calls.is_empty(), "{:?}", l.followed_calls);
        assert!(l.unlinked.iter().any(|u| u.reason == UnlinkedReason::NoSuchExport), "{:?}", l.unlinked);
    }

    #[test]
    fn extensions_and_index_files_are_probed() {
        let l = project(
            &[
                ("src/a.ts", "import './b'; import './c.js'; import './d'; import '../top';"),
                ("src/b.js", ""),
                ("src/c.ts", ""),
                ("src/d/index.mjs", ""),
                ("top.cjs", ""),
            ],
            &[],
            &[],
        );
        assert!(l.unlinked.iter().all(|u| u.reason != UnlinkedReason::MissingFile), "{:?}", l.unlinked);
        assert_eq!(l.links_made, 4);
        assert_eq!(l.entries, vec![0]);
    }

    #[test]
    fn re_exports_are_followed_to_the_function() {
        let l = project(
            &[
                ("a.js", "import { f } from './idx.js'; f();"),
                ("idx.js", "export { f } from './impl.js';"),
                ("impl.js", "export function f(){ DEL; }"),
            ],
            &[],
            &[],
        );
        assert_eq!(top(&l, 0).transitive_effects, vec![Effect::FsDelete]);
        assert!(l.unlinked.is_empty(), "{:?}", l.unlinked);
    }

    #[test]
    fn star_exports_are_searched() {
        let l = project(
            &[
                ("a.js", "import { f } from './idx.js'; f();"),
                ("idx.js", "export * from './impl.js';"),
                ("impl.js", "export function f(){ DEL; }"),
            ],
            &[],
            &[],
        );
        assert_eq!(top(&l, 0).transitive_effects, vec![Effect::FsDelete]);
    }

    #[test]
    fn a_missing_name_is_no_such_export_unless_exports_are_dynamic() {
        let l = project(
            &[("a.js", "import { g } from './b.js'; g();"), ("b.js", "export function f(){}")],
            &[],
            &[],
        );
        assert_eq!(reasons(&l), vec![UnlinkedReason::NoSuchExport]);
        let l = project(
            &[("a.js", "import { g } from './b.js'; g();"), ("b.js", "module.exports = { ...x };")],
            &[],
            &[],
        );
        assert_eq!(reasons(&l), vec![UnlinkedReason::OpaqueExport]);
    }

    #[test]
    fn a_non_function_export_is_opaque() {
        let l = project(
            &[("a.js", "import { v } from './b.js'; v();"), ("b.js", "export const v = 1;")],
            &[],
            &[],
        );
        assert_eq!(reasons(&l), vec![UnlinkedReason::OpaqueExport]);
        assert_eq!(l.unlinked[0].name.as_ref().unwrap().text(), "v");
    }

    #[test]
    fn re_export_cycles_terminate() {
        let l = project(
            &[
                ("a.js", "import { f } from './b.js'; f();"),
                ("b.js", "export { f } from './c.js';"),
                ("c.js", "export { f } from './b.js';"),
            ],
            &[],
            &[],
        );
        assert_eq!(reasons(&l), vec![UnlinkedReason::ReexportCycle]);
    }

    #[test]
    fn namespace_members_default_imports_and_require_forms_link() {
        let b = "export function wipe(){ DEL; } export default function main(){ DEL; }";
        for a in [
            "import * as u from './b.js'; u.wipe();",
            "import main from './b.js'; main();",
            "const u = require('./b.js'); u.wipe();",
            "const { wipe } = require('./b.js'); wipe();",
            "require('./b.js').wipe();",
        ] {
            let l = project(&[("a.js", a), ("b.js", b)], &[], &[]);
            assert_eq!(top(&l, 0).transitive_effects, vec![Effect::FsDelete], "{a}");
            assert!(l.unlinked.is_empty(), "{a}: {:?}", l.unlinked);
        }
    }

    #[test]
    fn commonjs_module_exports_function_links_through_a_plain_require() {
        let l = project(
            &[("a.js", "const run = require('./b.js'); run();"), ("b.js", "module.exports = function(){ DEL; };")],
            &[],
            &[],
        );
        let run = l.fns.iter().find(|f| f.at.file == 1 && f.at.func != FnId(0)).unwrap();
        assert_eq!(run.lanes, vec![Lane::Immediate]);
        assert!(run.transitive_effects.contains(&Effect::FsDelete));
    }

    #[test]
    fn a_called_function_in_another_file_does_not_make_its_siblings_run() {
        let l = project(
            &[("a.js", "import { wipe } from './b.js'; wipe();"), ("b.js", "export function wipe(){} export function other(){ DEL; }")],
            &[],
            &[],
        );
        assert!(top(&l, 0).transitive_effects.is_empty());
        let other = l.fns.iter().find(|f| f.at.file == 1 && f.at.func == FnId(2)).unwrap();
        assert!(!other.reachable_from_entry);
    }

    #[test]
    fn mutual_calls_across_files_terminate_and_share_effects() {
        let l = project(
            &[
                ("a.js", "import { b } from './b.js'; export function a(){ b(); DEL; } a();"),
                ("b.js", "import { a } from './a.js'; export function b(){ a(); }"),
            ],
            &[],
            &[],
        );
        assert_eq!(top(&l, 0).transitive_effects, vec![Effect::FsDelete]);
        let b = l.fns.iter().find(|f| f.at.file == 1 && f.at.func == FnId(1)).unwrap();
        assert_eq!(b.transitive_effects, vec![Effect::FsDelete]);
    }

    #[test]
    fn unlinked_entries_are_sorted_and_quote_their_text() {
        let l = project(&[("a.js", "import 'zeta'; import 'alpha';")], &[], &[]);
        assert_eq!(l.unlinked.len(), 2);
        assert_eq!(l.unlinked[0].spec.text(), "alpha");
        assert_eq!(l.unlinked[1].spec.text(), "zeta");
    }

    #[test]
    fn every_function_of_every_file_is_listed_once() {
        let l = project(&[("a.js", "function f(){} function g(){}"), ("b.js", "")], &[], &[]);
        assert_eq!(l.fns.len(), 4);
        let mut keys: Vec<_> = l.fns.iter().map(|f| f.at).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), 4);
    }

    #[test]
    fn declared_entries_out_of_range_are_ignored_and_duplicates_collapse() {
        let l = project(&[("a.js", ""), ("b.js", "")], &[], &[1, 1, 9]);
        assert_eq!(l.entries, vec![1]);
    }
}
