//! Builds the report from the analysis. Pure: no files, no clock, no randomness, so the same
//! input always gives the same report.

use crate::model::*;
use forge_analysis_schema::{
    DependencySource, Effect, FactId, FactKind, FlowSink, FnId, FnRef, FunctionSummary, Header, Lane, Linked,
    LinkedFn, Quoted, ScriptTrigger, Span, StaticFact, UnlinkedReason, UnresolvedReason,
};
use std::collections::BTreeMap;

/// What the analysis found in one file. `Linked::files[i]` goes with `files[i]`.
pub struct FileInput<'a> {
    pub facts: &'a [StaticFact],
    pub summaries: &'a [FunctionSummary],
}

pub struct ReportInput<'a> {
    pub header: Header,
    pub linked: &'a Linked,
    pub files: &'a [FileInput<'a>],
    /// Facts from `package.json` files (scripts, dependencies, parse failures, shell commands).
    pub package_facts: &'a [StaticFact],
    pub not_analysed: &'a [NotAnalysedFile],
    pub unrecorded_skips: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// `files` and `linked.files` must have one entry per file, in the same order.
    FileCountMismatch { linked: usize, files: usize },
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for BuildError {}

/// How a file that was not read as code is classified, from its path alone.
pub fn classify_path(path: &str) -> NotAnalysedKind {
    let lower = path.to_ascii_lowercase().replace('\\', "/");
    let name = lower.rsplit('/').next().unwrap_or("");
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    if matches!(name, ".npmrc" | ".yarnrc" | ".yarnrc.yml" | ".pnpmrc") {
        NotAnalysedKind::NpmConfig
    } else if crate::autorun::is_auto_run_config(&lower) {
        NotAnalysedKind::AutoRunConfig
    } else if lower.contains(".github/workflows/")
        || lower.contains(".circleci/")
        || matches!(name, ".gitlab-ci.yml" | "azure-pipelines.yml" | "jenkinsfile")
    {
        NotAnalysedKind::CiWorkflow
    } else if name.starts_with("dockerfile")
        || ext == "dockerfile"
        || name.starts_with("docker-compose")
        || name == "containerfile"
    {
        NotAnalysedKind::Container
    } else if matches!(ext, "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat" | "cmd") {
        NotAnalysedKind::ShellScript
    } else if matches!(ext, "zip" | "tar" | "tgz" | "gz" | "bz2" | "xz" | "zst" | "lz" | "lzma" | "7z" | "rar" | "cab"
        | "jar" | "war" | "ear" | "whl" | "egg" | "nupkg" | "snupkg" | "gem" | "phar" | "apk" | "aab" | "ipa" | "deb"
        | "rpm" | "dmg" | "iso" | "pkg" | "msi" | "msix" | "appx" | "vsix" | "crx" | "xpi" | "asar" | "conda" | "tbz"
        | "tbz2" | "txz")
    {
        NotAnalysedKind::Archive
    } else if matches!(
        ext,
        "node" | "exe" | "dll" | "so" | "dylib" | "wasm" | "a" | "o" | "bin" | "pyd"
    ) {
        NotAnalysedKind::NativeBinary
    } else {
        NotAnalysedKind::OtherFile
    }
}

const FACT_KINDS: [FactKind; 9] = [
    FactKind::Entry,
    FactKind::Import,
    FactKind::Call,
    FactKind::Effect,
    FactKind::Timer,
    FactKind::LifecycleScript,
    FactKind::Dependency,
    FactKind::Unresolved,
    FactKind::Flow,
];

/// Most worrying first.
fn effect_rank(e: Effect) -> u8 {
    match e {
        Effect::CodeLoad => 0,
        Effect::DynamicLoad => 1,
        Effect::ProcessSpawn => 2,
        Effect::FsDelete => 3,
        Effect::NetOut => 4,
        Effect::NetListen => 5,
        Effect::FsWrite => 6,
        Effect::FsChmod => 7,
        Effect::FsRead => 8,
        Effect::EnvRead => 9,
        Effect::SystemInfo => 10,
        Effect::Timer => 11,
    }
}

fn unresolved_rank(r: UnresolvedReason) -> u8 {
    use UnresolvedReason::*;
    match r {
        Eval => 0,
        DynamicRequire => 1,
        Reflection => 2,
        ComputedAccess => 3,
        IndirectUse => 4,
        UnknownReceiver => 5,
        EventHandler => 6,
        RuntimeDelay => 7,
        UncoveredModule => 8,
        UncatalogedMember => 9,
        ShellCommand => 10,
        AmbiguousBinding => 11,
        UnsupportedSyntax => 12,
        ParseFailed => 13,
        TooDeeplyNested => 14,
    }
}

fn unlinked_rank(r: UnlinkedReason) -> u8 {
    use UnlinkedReason::*;
    match r {
        OpaqueExport => 0,
        NoSuchExport => 1,
        MissingFile => 2,
        OutsideRoot => 3,
        ReexportCycle => 4,
        NotAnalysed => 5,
        SubpathImport => 6,
        ExternalPackage => 7,
    }
}

fn trigger_rank(t: ScriptTrigger) -> u8 {
    match t {
        ScriptTrigger::Install => 0,
        ScriptTrigger::Prepare => 1,
        ScriptTrigger::OnOpen => 2,
        ScriptTrigger::OnGit => 3,
        ScriptTrigger::OnBuild => 4,
        ScriptTrigger::Hook => 5,
        ScriptTrigger::Manual => 6,
    }
}

fn span_key(at: &Span, id: &FactId) -> (String, u32, u32, String) {
    (
        at.file.as_quoted().text().to_string(),
        at.start_byte,
        at.end_byte,
        id.as_str().to_string(),
    )
}

fn quoted_key(q: &Quoted) -> (String, String) {
    (q.text().to_string(), q.sha256().to_string())
}

fn opt_key(q: &Option<Quoted>) -> (String, String) {
    q.as_ref().map(quoted_key).unwrap_or_default()
}

struct Ctx<'a> {
    input: &'a ReportInput<'a>,
    summaries: BTreeMap<FnRef, &'a FunctionSummary>,
    linked_fns: BTreeMap<FnRef, &'a LinkedFn>,
}

impl<'a> Ctx<'a> {
    fn fn_loc(&self, file: u32, func: FnId) -> FnLoc {
        let r = FnRef { file, func };
        let s = self.summaries.get(&r);
        FnLoc {
            file: self.input.linked.files[file as usize].clone(),
            func,
            name: s.and_then(|s| s.name.clone()),
            line: s.map(|s| s.at.start_line),
        }
    }

    fn place(&self, file: u32, fact: &StaticFact) -> Place {
        match fact.within {
            Some(func) => {
                let linked = self.linked_fns.get(&FnRef { file, func });
                Place {
                    in_fn: Some(self.fn_loc(file, func)),
                    lanes: linked.map(|l| l.lanes.clone()).unwrap_or_default(),
                    reachable: linked.map(|l| l.reachable_from_entry),
                }
            }
            None => Place { in_fn: None, lanes: Vec::new(), reachable: None },
        }
    }
}

/// All facts with the index of the analysed file they came from; package facts have none.
fn all_facts<'a>(input: &'a ReportInput<'a>) -> Vec<(Option<u32>, &'a StaticFact)> {
    let mut v: Vec<(Option<u32>, &StaticFact)> = Vec::new();
    for (i, f) in input.files.iter().enumerate() {
        v.extend(f.facts.iter().map(|x| (Some(i as u32), x)));
    }
    v.extend(input.package_facts.iter().map(|x| (None, x)));
    v
}

pub fn build(input: &ReportInput<'_>) -> Result<Report, BuildError> {
    let linked = input.linked;
    if input.files.len() != linked.files.len() {
        return Err(BuildError::FileCountMismatch { linked: linked.files.len(), files: input.files.len() });
    }
    let mut summaries = BTreeMap::new();
    for (i, f) in input.files.iter().enumerate() {
        for s in f.summaries {
            summaries.insert(FnRef { file: i as u32, func: s.id }, s);
        }
    }
    let mut linked_fns = BTreeMap::new();
    for l in &linked.fns {
        linked_fns.insert(l.at, l);
    }
    let cx = Ctx { input, summaries, linked_fns };
    let facts = all_facts(input);

    // A call through an import that the linker followed is not a place where "static analysis
    // cannot say what happens": the per-file pass flags it only because it cannot see the other
    // file. Only an exact match (same file, function and byte range) clears the flag.
    let followed_sites: std::collections::BTreeSet<(u32, FnId, u32, u32)> = linked
        .followed_calls
        .iter()
        .map(|c| (c.from.file, c.from.func, c.start_byte, c.end_byte))
        .collect();
    // A call into an external package (one we saw imported and did not follow) is reported with
    // its package, not as an open unresolved item: the import already carries "not analysed".
    let package_specs: std::collections::BTreeSet<String> = linked
        .unlinked
        .iter()
        .filter(|u| u.reason == UnlinkedReason::ExternalPackage)
        .map(|u| u.spec.text().to_string())
        .collect();
    let package_of = |f: &StaticFact| -> Option<Quoted> {
        match (f.unresolved, &f.module) {
            (Some(UnresolvedReason::UncoveredModule), Some(m)) if package_specs.contains(m.text()) => Some(m.clone()),
            _ => None,
        }
    };
    let is_followed = |file: &Option<u32>, f: &StaticFact| -> bool {
        match (file, f.unresolved, f.within) {
            (Some(i), Some(UnresolvedReason::UncoveredModule), Some(func)) => {
                followed_sites.contains(&(*i, func, f.at().start_byte, f.at().end_byte))
            }
            _ => false,
        }
    };

    // ---- entry points ----
    let path_of = |i: &u32| linked.files[*i as usize].clone();
    let entry_points = EntryPoints {
        basis: linked.entry_basis,
        runtime: linked.entries.iter().filter(|i| (**i as usize) < linked.files.len()).map(path_of).collect(),
        install: linked.install_entries.iter().filter(|i| (**i as usize) < linked.files.len()).map(path_of).collect(),
        issues: linked.entry_issues.clone(),
    };

    // ---- timeline ----
    let mut lanes: BTreeMap<Lane, Vec<FnLoc>> = BTreeMap::new();
    for l in &linked.fns {
        for lane in &l.lanes {
            lanes.entry(*lane).or_default().push(cx.fn_loc(l.at.file, l.at.func));
        }
    }
    let mut timers: Vec<TimerItem> = Vec::new();
    // ---- effects, flows, unresolved, scripts, dependencies ----
    let mut effects: BTreeMap<u8, (Effect, Vec<EffectItem>)> = BTreeMap::new();
    let mut flows: Vec<FlowItem> = Vec::new();
    let mut scripts: Vec<ScriptItem> = Vec::new();
    let mut dependencies: Vec<DependencyItem> = Vec::new();
    let mut unresolved: BTreeMap<u8, (UnresolvedReason, Vec<UnresolvedItem>)> = BTreeMap::new();
    let mut shell: Vec<&StaticFact> = Vec::new();
    let mut package_map: BTreeMap<(String, String), (Quoted, Vec<UnresolvedItem>)> = BTreeMap::new();

    for (file, f) in &facts {
        let place = match file {
            Some(i) => cx.place(*i, f),
            None => Place { in_fn: None, lanes: Vec::new(), reachable: None },
        };
        match f.kind() {
            FactKind::Timer => {
                if let Some(timing) = f.timing {
                    timers.push(TimerItem { id: f.id().clone(), at: f.at().clone(), detail: f.detail().cloned(), timing, place: place.clone() });
                }
            }
            FactKind::Effect => {
                if let Some(e) = f.effect {
                    effects.entry(effect_rank(e)).or_insert_with(|| (e, Vec::new())).1.push(EffectItem {
                        id: f.id().clone(),
                        at: f.at().clone(),
                        detail: f.detail().cloned(),
                        url: f.url.clone(),
                        place: place.clone(),
                    });
                }
            }
            FactKind::Flow => {
                if let Some(origin) = &f.origin {
                    flows.push(FlowItem { id: f.id().clone(), sink: f.flow_sink.unwrap_or(FlowSink::Code), at: f.at().clone(), origin: origin.clone(), detail: f.detail().cloned(), place: place.clone() });
                }
            }
            FactKind::LifecycleScript => {
                if let Some(s) = &f.script {
                    scripts.push(ScriptItem {
                        id: f.id().clone(),
                        at: f.at().clone(),
                        name: s.name.clone(),
                        trigger: s.trigger,
                        command: f.detail().cloned(),
                        unfollowed: Vec::new(),
                    });
                }
            }
            FactKind::Dependency => {
                if let Some(d) = &f.dependency {
                    dependencies.push(DependencyItem { id: f.id().clone(), at: f.at().clone(), name: f.detail().cloned(), info: d.clone() });
                }
            }
            _ => {}
        }
        if let Some(pkg) = package_of(f).filter(|_| !is_followed(file, f)) {
            let items = package_map.entry(quoted_key(&pkg)).or_insert_with(|| (pkg.clone(), Vec::new()));
            items.1.push(UnresolvedItem { id: f.id().clone(), at: f.at().clone(), detail: f.detail().cloned(), place: place.clone() });
        } else if let Some(reason) = f.unresolved.filter(|_| !is_followed(file, f)) {
            if reason == UnresolvedReason::ShellCommand {
                shell.push(*f);
            }
            unresolved.entry(unresolved_rank(reason)).or_insert_with(|| (reason, Vec::new())).1.push(UnresolvedItem {
                id: f.id().clone(),
                at: f.at().clone(),
                detail: f.detail().cloned(),
                place,
            });
        }
    }

    // A script's unfollowed commands are the shell-command facts on its own value span.
    for s in &mut scripts {
        let mut mine: Vec<&&StaticFact> = shell
            .iter()
            .filter(|f| f.at().file == s.at.file && f.at().start_byte == s.at.start_byte && f.at().end_byte == s.at.end_byte)
            .collect();
        mine.sort_by_key(|f| f.id().as_str().to_string());
        // Source order is the order the facts were made in; keep that, not the id order.
        let mut ordered: Vec<&StaticFact> = Vec::new();
        for (_, f) in &facts {
            if mine.iter().any(|m| m.id() == f.id()) {
                ordered.push(*f);
            }
        }
        s.unfollowed = ordered.iter().filter_map(|f| f.detail().cloned()).collect();
    }

    timers.sort_by_key(|t| span_key(&t.at, &t.id));
    let mut effect_groups: Vec<EffectGroup> = Vec::new();
    for (_, (effect, mut items)) in effects {
        items.sort_by_key(|i| span_key(&i.at, &i.id));
        effect_groups.push(EffectGroup { effect, items });
    }
    flows.sort_by_key(|f| span_key(&f.at, &f.id));
    scripts.sort_by_key(|s| {
        let k = span_key(&s.at, &s.id);
        (trigger_rank(s.trigger), k)
    });
    dependencies.sort_by_key(|d| (d.info.section, opt_key(&d.name), span_key(&d.at, &d.id)));
    let mut unresolved_groups: Vec<UnresolvedGroup> = Vec::new();
    for (_, (reason, mut items)) in unresolved {
        items.sort_by_key(|i| span_key(&i.at, &i.id));
        unresolved_groups.push(UnresolvedGroup { reason, items });
    }

    let mut package_calls: Vec<PackageCalls> = Vec::new();
    for (_, (spec, mut items)) in package_map {
        items.sort_by_key(|i| span_key(&i.at, &i.id));
        let functions: std::collections::BTreeSet<(String, u32)> = items
            .iter()
            .filter_map(|i| i.place.in_fn.as_ref().map(|f| (f.file.as_quoted().text().to_string(), f.func.0)))
            .collect();
        package_calls.push(PackageCalls { spec, calls: items.len() as u32, functions: functions.len() as u32, items });
    }
    // Most calls first: that is the package the code leans on most.
    package_calls.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| quoted_key(&a.spec).cmp(&quoted_key(&b.spec))));

    // ---- unlinked (merged by what is imported, every importing function kept) ----
    type Key = (String, String, String, String);
    let mut merged: BTreeMap<u8, BTreeMap<Key, UnlinkedItem>> = BTreeMap::new();
    let mut reasons: BTreeMap<u8, UnlinkedReason> = BTreeMap::new();
    for u in &linked.unlinked {
        let rank = unlinked_rank(u.reason);
        reasons.insert(rank, u.reason);
        let (n0, n1) = opt_key(&u.name);
        let (s0, s1) = quoted_key(&u.spec);
        let item = merged.entry(rank).or_default().entry((s0, s1, n0, n1)).or_insert_with(|| UnlinkedItem {
            spec: u.spec.clone(),
            name: u.name.clone(),
            from: Vec::new(),
        });
        item.from.push(cx.fn_loc(u.from.file, u.from.func));
    }
    let mut unlinked_groups: Vec<UnlinkedGroup> = Vec::new();
    for (rank, items) in merged {
        let mut items: Vec<UnlinkedItem> = items.into_values().collect();
        for it in &mut items {
            it.from.sort_by_key(|f| (f.file.as_quoted().text().to_string(), f.func));
            it.from.dedup();
        }
        unlinked_groups.push(UnlinkedGroup { reason: reasons[&rank], items });
    }

    // ---- not analysed ----
    let mut by_kind: BTreeMap<NotAnalysedKind, Vec<NotAnalysedFile>> = BTreeMap::new();
    for n in input.not_analysed {
        by_kind.entry(n.kind).or_default().push(n.clone());
    }
    let mut not_groups: Vec<NotAnalysedGroup> = Vec::new();
    for (kind, mut items) in by_kind {
        items.sort_by_key(|i| quoted_key(&i.path));
        not_groups.push(NotAnalysedGroup { kind, items });
    }
    let not_analysed = NotAnalysedSection { groups: not_groups, unrecorded: input.unrecorded_skips };

    // ---- function index ----
    let mut placed: BTreeMap<FnRef, Vec<FactId>> = BTreeMap::new();
    let mut unplaced: Vec<FactId> = Vec::new();
    for (i, f) in input.files.iter().enumerate() {
        for fact in f.facts {
            match fact.within {
                Some(func) if cx.summaries.contains_key(&FnRef { file: i as u32, func }) => {
                    placed.entry(FnRef { file: i as u32, func }).or_default().push(fact.id().clone());
                }
                _ => unplaced.push(fact.id().clone()),
            }
        }
    }
    let mut still_open: BTreeMap<FnRef, Vec<UnresolvedReason>> = BTreeMap::new();
    for (file, f) in &facts {
        if let (Some(i), Some(reason), Some(func)) = (file, f.unresolved, f.within) {
            if !is_followed(file, f) && package_of(f).is_none() {
                still_open.entry(FnRef { file: *i, func }).or_default().push(reason);
            }
        }
    }
    let mut call_lines: BTreeMap<(u32, FnId, u32, u32), u32> = BTreeMap::new();
    let mut cleared = 0u32;
    for (file, f) in &facts {
        if is_followed(file, f) {
            cleared += 1;
            if let (Some(i), Some(func)) = (file, f.within) {
                call_lines.insert((*i, func, f.at().start_byte, f.at().end_byte), f.at().start_line);
            }
        }
    }
    let mut pkg_counts: BTreeMap<FnRef, BTreeMap<(String, String), (Quoted, u32)>> = BTreeMap::new();
    for (file, f) in &facts {
        if let (Some(i), Some(func), Some(pkg)) = (file, f.within, package_of(f)) {
            if !is_followed(file, f) {
                pkg_counts.entry(FnRef { file: *i, func }).or_default().entry(quoted_key(&pkg)).or_insert_with(|| (pkg.clone(), 0)).1 += 1;
            }
        }
    }
    let mut out_calls: BTreeMap<FnRef, Vec<CrossCall>> = BTreeMap::new();
    let mut in_calls: BTreeMap<FnRef, Vec<FnLoc>> = BTreeMap::new();
    for c in &linked.followed_calls {
        out_calls.entry(c.from).or_default().push(CrossCall {
            to: cx.fn_loc(c.to.file, c.to.func),
            spec: c.spec.clone(),
            name: c.name.clone(),
            via_prop: c.via_prop,
            line: call_lines.get(&(c.from.file, c.from.func, c.start_byte, c.end_byte)).copied(),
            start_byte: c.start_byte,
            end_byte: c.end_byte,
        });
        in_calls.entry(c.to).or_default().push(cx.fn_loc(c.from.file, c.from.func));
    }
    for v in out_calls.values_mut() {
        v.sort_by_key(|c| (c.start_byte, c.end_byte));
    }
    for v in in_calls.values_mut() {
        v.sort_by_key(|f| (f.file.as_quoted().text().to_string(), f.func));
        v.dedup();
    }
    // A call that is not to a function in its own file is one of: followed across files, sitting
    // exactly on a fact, or not traced at all. The last kind is what the coverage line counts.
    let mut fact_sites: std::collections::BTreeSet<(u32, FnId, u32, u32)> = std::collections::BTreeSet::new();
    for (file, f) in &facts {
        if let (Some(i), Some(func)) = (file, f.within) {
            fact_sites.insert((*i, func, f.at().start_byte, f.at().end_byte));
        }
    }
    let mut not_traced_total = 0u32;
    let mut untraced_groups: BTreeMap<(String, String), UntracedGroup> = BTreeMap::new();
    let mut function_items: Vec<FunctionItem> = Vec::new();
    for (r, s) in &cx.summaries {
        let l = cx.linked_fns.get(r);
        let (mut by_link, mut by_fact, mut untraced) = (0u32, 0u32, 0u32);
        let mut untraced_calls: Vec<UntracedCall> = Vec::new();
        for (idx, site) in s.unfollowed_sites.iter().enumerate() {
            let k = (r.file, r.func, site.start_byte, site.end_byte);
            if followed_sites.contains(&k) {
                by_link += 1;
            } else if fact_sites.contains(&k) {
                by_fact += 1;
            } else {
                untraced += 1;
                // `validate` guarantees one label per site; a missing one is still counted.
                let (callee, line) = match s.unfollowed_callees.get(idx) {
                    Some(c) => (c.callee.clone(), c.start_line),
                    None => (Quoted::new("(unknown)"), s.at.start_line),
                };
                untraced_calls.push(UntracedCall {
                    callee: callee.clone(),
                    line,
                    start_byte: site.start_byte,
                    end_byte: site.end_byte,
                });
                let g = untraced_groups
                    .entry(quoted_key(&callee))
                    .or_insert_with(|| UntracedGroup { callee, count: 0, sites: Vec::new() });
                g.count += 1;
                g.sites.push(UntracedSite {
                    in_fn: cx.fn_loc(r.file, r.func),
                    line,
                    start_byte: site.start_byte,
                    end_byte: site.end_byte,
                });
            }
        }
        not_traced_total += untraced;
        function_items.push(FunctionItem {
            at: s.at.clone(),
            file: linked.files[r.file as usize].clone(),
            func: r.func,
            name: s.name.clone(),
            lanes: l.map(|l| l.lanes.clone()).unwrap_or_else(|| s.lanes.clone()),
            reachable_from_entry: l.map(|l| l.reachable_from_entry).unwrap_or(s.reachable_from_entry),
            escapes: s.escapes,
            direct_effects: s.direct_effects.clone(),
            transitive_effects: l.map(|l| l.transitive_effects.clone()).unwrap_or_else(|| s.transitive_effects.clone()),
            // The summary lists a reason once per function; drop one only when every fact that
            // gave it was followed, so a reason with an open fact stays.
            unresolved: s
                .unresolved
                .iter()
                .copied()
                .filter(|u| *u != UnresolvedReason::UncoveredModule || still_open.get(r).is_some_and(|o| o.contains(u)))
                .collect(),
            unfollowed_calls: s.unfollowed_calls,
            calls_followed_across_files: by_link,
            calls_matched_to_a_fact: by_fact,
            calls_not_traced: untraced,
            untraced_calls,
            calls_into_packages: pkg_counts
                .remove(r)
                .map(|m| m.into_values().map(|(spec, count)| PackageCount { spec, count }).collect())
                .unwrap_or_default(),
            calls: s.calls.clone(),
            callbacks: s.callbacks.clone(),
            called_from: s.called_from.clone(),
            followed_calls: out_calls.remove(r).unwrap_or_default(),
            followed_called_from: in_calls.remove(r).unwrap_or_default(),
            facts: placed.remove(r).unwrap_or_default(),
        });
    }
    unplaced.sort();
    let mut untraced_by_callee: Vec<UntracedGroup> = untraced_groups.into_values().collect();
    for g in &mut untraced_by_callee {
        g.sites.sort_by_key(|x| (x.in_fn.file.as_quoted().text().to_string(), x.in_fn.func, x.start_byte, x.end_byte));
    }
    untraced_by_callee.sort_by(|a, c| c.count.cmp(&a.count).then_with(|| quoted_key(&a.callee).cmp(&quoted_key(&c.callee))));
    let functions = FunctionsSection { functions: function_items, untraced_by_callee, unplaced_facts: unplaced };

    // ---- counts, coverage, index ----
    let count = |n: usize| n as u32;
    let unresolved_total: usize = unresolved_groups.iter().map(|g| g.items.len()).sum();
    let effects_total: usize = effect_groups.iter().map(|g| g.items.len()).sum();
    let not_analysed_total: u64 = not_analysed.groups.iter().map(|g| g.items.len() as u64).sum::<u64>() + not_analysed.unrecorded;
    let outside_dependencies = dependencies
        .iter()
        .filter(|d| !matches!(d.info.source, DependencySource::LocalPath | DependencySource::Workspace | DependencySource::Bundled))
        .count();
    let functions_with_lane = linked.fns.iter().filter(|l| !l.lanes.is_empty()).count();

    let coverage = vec![
        CoverageLine { section: SectionId::Unresolved, label: "places where static analysis cannot say what happens", count: count(unresolved_total) },
        CoverageLine { section: SectionId::Unlinked, label: "imports not followed to their source", count: count(linked.unlinked.len()) },
        CoverageLine { section: SectionId::EntryPoints, label: "declared entry points that match no analysed file", count: count(linked.entry_issues.len()) },
        CoverageLine { section: SectionId::NotAnalysed, label: "files or folders not read as code", count: not_analysed_total.min(u32::MAX as u64) as u32 },
        CoverageLine { section: SectionId::Dependencies, label: "dependencies whose own code and install scripts were not analysed", count: count(outside_dependencies) },
        CoverageLine { section: SectionId::Functions, label: "calls whose target we could not see (no function followed, no fact)", count: not_traced_total },
    ];
    // A quick scan is never complete: it left code unread on purpose, even when no file is listed.
    let quick = input.header.scope == forge_analysis_schema::Scope::Quick;
    let completeness = if quick || coverage.iter().any(|c| c.count > 0) { Completeness::Incomplete } else { Completeness::Complete };

    let mirrored_output = mirror_notes(&linked.files);
    let unreached_functions = count(functions.functions.iter().filter(|f| !f.reachable_from_entry).count());
    let overview = Overview {
        files_analysed: count(linked.files.len()),
        files_not_analysed: count(not_analysed.groups.iter().map(|g| g.items.len()).sum()),
        functions: count(functions.functions.len()),
        facts: count(facts.len()),
        links_made: linked.links_made,
        unresolved_cleared_by_links: cleared,
        completeness,
        coverage,
        fact_counts: FACT_KINDS
            .iter()
            .map(|k| FactCount { kind: *k, count: count(facts.iter().filter(|(_, f)| f.kind() == *k).count()) })
            .collect(),
        mirrored_output,
        unreached_functions,
    };

    let counts: [Option<u32>; 11] = [
        None,
        Some(count(entry_points.runtime.len() + entry_points.install.len() + entry_points.issues.len())),
        Some(count(functions_with_lane + timers.len())),
        Some(count(effects_total)),
        Some(count(flows.len())),
        Some(count(scripts.len())),
        Some(count(dependencies.len())),
        Some(count(unresolved_total)),
        Some(count(linked.unlinked.len())),
        Some(not_analysed_total.min(u32::MAX as u64) as u32),
        Some(count(functions.functions.len())),
    ];
    let index = SectionId::ALL
        .iter()
        .zip(counts)
        .map(|(s, c)| IndexEntry { section: *s, number: s.number(), title: s.title(), explanation: s.explanation(), count: c, anchor: s.anchor() })
        .collect();

    let timeline = Timeline {
        lanes: lanes.into_iter().map(|(lane, functions)| LaneGroup { lane, functions }).collect(),
        timers,
    };

    Ok(Report {
        header: input.header.clone(),
        index,
        overview,
        entry_points,
        timeline,
        effects: effect_groups,
        flows,
        scripts,
        dependencies,
        unresolved: unresolved_groups,
        unlinked: unlinked_groups,
        package_calls,
        not_analysed,
        functions,
    })
}

impl Report {
    /// Every fact id mentioned anywhere in the report.
    pub fn fact_ids(&self) -> std::collections::BTreeSet<String> {
        let mut s = std::collections::BTreeSet::new();
        let mut add = |id: &FactId| {
            s.insert(id.as_str().to_string());
        };
        self.timeline.timers.iter().for_each(|t| add(&t.id));
        self.effects.iter().flat_map(|g| &g.items).for_each(|i| add(&i.id));
        self.flows.iter().for_each(|f| add(&f.id));
        self.scripts.iter().for_each(|x| add(&x.id));
        self.dependencies.iter().for_each(|d| add(&d.id));
        self.unresolved.iter().flat_map(|g| &g.items).for_each(|i| add(&i.id));
        self.package_calls.iter().flat_map(|g| &g.items).for_each(|i| add(&i.id));
        self.functions.functions.iter().flat_map(|f| &f.facts).for_each(&mut add);
        self.functions.unplaced_facts.iter().for_each(&mut add);
        s
    }
}

pub fn to_json(report: &Report) -> String {
    serde_json::to_string_pretty(report).expect("a report is always serialisable")
}

/// Output-like folders (`dist`, `build`, `out`) whose files mostly share names with files under
/// `src`. Looks only at paths; nothing is dropped or merged.
pub(crate) fn mirror_notes(files: &[forge_analysis_schema::RelPath]) -> Vec<MirrorNote> {
    use std::collections::{BTreeMap, BTreeSet};
    fn stem(p: &str) -> String {
        let p = p.replace('\\', "/");
        match p.rfind('.') {
            Some(i) if i > p.rfind('/').map(|j| j + 1).unwrap_or(0) => p[..i].to_string(),
            _ => p,
        }
    }
    let mut src: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut out: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for f in files {
        let t = f.as_quoted().text().replace('\\', "/");
        let parts: Vec<&str> = t.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            let prefix = parts[..i].join("/");
            let rest = stem(&parts[i + 1..].join("/"));
            if parts.len() <= i + 1 {
                break;
            }
            if *part == "src" {
                src.entry(prefix.clone()).or_default().insert(rest);
            } else if matches!(*part, "dist" | "build" | "out") {
                out.entry((prefix, part.to_string())).or_default().insert(rest);
            }
        }
    }
    let mut notes = Vec::new();
    for ((prefix, dir), rests) in &out {
        let Some(s) = src.get(prefix) else { continue };
        let matching = rests.iter().filter(|r| s.contains(*r)).count();
        if matching > 0 && matching * 2 >= rests.len() {
            let join = |d: &str| if prefix.is_empty() { d.to_string() } else { format!("{prefix}/{d}") };
            notes.push(MirrorNote {
                output_dir: Quoted::new(&join(dir)),
                source_dir: Quoted::new(&join("src")),
                matching_files: matching as u32,
                output_files: rests.len() as u32,
            });
        }
    }
    notes
}
