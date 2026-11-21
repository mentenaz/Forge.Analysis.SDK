use crate::*;
use forge_analysis_schema::*;

fn rp(p: &str) -> RelPath {
    RelPath::new(p).unwrap()
}
fn sp(file: &str, line: u32, s: u32, e: u32) -> Span {
    Span { file: rp(file), start_line: line, end_line: line, start_byte: s, end_byte: e }
}
fn fact(kind: FactKind, file: &str, line: u32, s: u32, detail: &str, within: Option<u32>) -> StaticFact {
    let mut f = StaticFact::new(kind, sp(file, line, s, s + 5), Some(Quoted::new(detail)));
    f.within = within.map(FnId);
    f
}
fn effect(file: &str, line: u32, s: u32, e: Effect, within: Option<u32>) -> StaticFact {
    let mut f = fact(FactKind::Effect, file, line, s, "x", within);
    f.effect = Some(e);
    f
}
fn unresolved(file: &str, line: u32, s: u32, r: UnresolvedReason, within: Option<u32>) -> StaticFact {
    let mut f = fact(FactKind::Unresolved, file, line, s, "u", within);
    f.unresolved = Some(r);
    f
}
fn summary(file: &str, id: u32, name: Option<&str>, line: u32, lanes: Vec<Lane>) -> FunctionSummary {
    FunctionSummary {
        id: FnId(id),
        name: name.map(Quoted::new),
        at: sp(file, line, id * 100, id * 100 + 50),
        direct_effects: vec![],
        transitive_effects: vec![],
        unresolved: vec![],
        calls: vec![],
        callbacks: vec![],
        called_from: vec![],
        reachable_from_entry: !lanes.is_empty(),
        lanes,
        unfollowed_calls: 0,
        unfollowed_sites: vec![],
        unfollowed_callees: vec![],
        escapes: false,
    }
}
fn linked_fn(file: u32, func: u32, lanes: Vec<Lane>) -> LinkedFn {
    LinkedFn {
        at: FnRef { file, func: FnId(func) },
        transitive_effects: vec![],
        reachable_from_entry: !lanes.is_empty(),
        lanes,
    }
}
fn header() -> Header {
    Header { schema: 1, engine: "test".into(), catalog_sha256: Hash::of(b"c"), snapshot_sha256: Hash::of(b"s"), mode: Mode::StaticOnly, scope: Scope::Full }
}

struct Fixture {
    linked: Linked,
    facts_a: Vec<StaticFact>,
    sums_a: Vec<FunctionSummary>,
    facts_b: Vec<StaticFact>,
    sums_b: Vec<FunctionSummary>,
    pkg: Vec<StaticFact>,
    na: Vec<NotAnalysedFile>,
    unrecorded: u64,
}

fn fixture() -> Fixture {
    let mut timer = fact(FactKind::Timer, "a.js", 3, 300, "t", Some(0));
    timer.timing = Some(Timing { lane: Lane::Deferred, delay_ms: Delay::Known(10), cleared: None });
    let mut flow = fact(FactKind::Flow, "a.js", 9, 900, "fetch", Some(1));
    flow.origin = Some(sp("a.js", 8, 800, 805));

    let facts_a = vec![
        fact(FactKind::Import, "a.js", 1, 10, "fs", Some(0)),
        effect("a.js", 2, 200, Effect::NetOut, Some(0)),
        timer,
        effect("a.js", 5, 500, Effect::FsDelete, Some(1)),
        unresolved("a.js", 6, 600, UnresolvedReason::Eval, Some(1)),
        fact(FactKind::Call, "a.js", 7, 700, "helper", Some(1)),
        flow,
    ];
    let mut sums_a = vec![summary("a.js", 0, None, 1, vec![Lane::Immediate]), summary("a.js", 1, Some("helper"), 4, vec![Lane::Deferred])];
    // Calls we could not trace: two to `console.log` (one in each function), one `(..).then`.
    let site = |s: u32| ByteRange { start_byte: s, end_byte: s + 8 };
    let callee = |c: &str, line: u32| UnfollowedCallee { callee: Quoted::new(c), start_line: line };
    sums_a[0].unfollowed_calls = 2;
    sums_a[0].unfollowed_sites = vec![site(20), site(30)];
    sums_a[0].unfollowed_callees = vec![callee("console.log", 2), callee("(..).then", 2)];
    sums_a[1].unfollowed_calls = 1;
    sums_a[1].unfollowed_sites = vec![site(450)];
    sums_a[1].unfollowed_callees = vec![callee("console.log", 5)];
    let facts_b = vec![effect("b.js", 1, 10, Effect::FsRead, Some(0))];
    let sums_b = vec![summary("b.js", 0, None, 1, vec![])];

    let script_at = sp("package.json", 4, 40, 60);
    let mut script = StaticFact::new(FactKind::LifecycleScript, script_at.clone(), Some(Quoted::new("curl x | sh")));
    script.script = Some(Script { name: Quoted::new("postinstall"), trigger: ScriptTrigger::Install });
    let mut shell1 = StaticFact::new(FactKind::Unresolved, script_at.clone(), Some(Quoted::new("curl")));
    shell1.unresolved = Some(UnresolvedReason::ShellCommand);
    let mut shell2 = StaticFact::new(FactKind::Unresolved, script_at, Some(Quoted::new("sh")));
    shell2.unresolved = Some(UnresolvedReason::ShellCommand);
    let dep = |name: &str, s: u32, source: DependencySource| {
        let mut d = StaticFact::new(FactKind::Dependency, sp("package.json", 8, s, s + 5), Some(Quoted::new(name)));
        d.dependency = Some(DependencyInfo { section: DepSection::Dependencies, spec: Quoted::new("1"), source });
        d
    };
    let pkg = vec![script, shell1, shell2, dep("lodash", 100, DependencySource::Registry), dep("mine", 110, DependencySource::LocalPath)];

    let uf = |file: u32, func: u32, spec: &str, reason: UnlinkedReason| Unlinked {
        from: FnRef { file, func: FnId(func) },
        spec: Quoted::new(spec),
        name: None,
        reason,
    };
    let linked = Linked {
        files: vec![rp("a.js"), rp("b.js")],
        entry_basis: EntryBasis::Declared,
        entries: vec![0],
        install_entries: vec![],
        entry_issues: vec![],
        fns: vec![linked_fn(0, 0, vec![Lane::Immediate]), linked_fn(0, 1, vec![Lane::Deferred]), linked_fn(1, 0, vec![])],
        followed_calls: vec![],
        unlinked: vec![
            uf(0, 1, "lodash", UnlinkedReason::ExternalPackage),
            uf(0, 0, "lodash", UnlinkedReason::ExternalPackage),
            uf(1, 0, "./gone", UnlinkedReason::MissingFile),
        ],
        links_made: 2,
    };
    let na = vec![
        NotAnalysedFile { path: Quoted::new("run.sh"), size: Some(10), kind: NotAnalysedKind::ShellScript },
        NotAnalysedFile { path: Quoted::new(".npmrc"), size: Some(5), kind: NotAnalysedKind::NpmConfig },
    ];
    Fixture { linked, facts_a, sums_a, facts_b, sums_b, pkg, na, unrecorded: 3 }
}

impl Fixture {
    fn run(&self) -> Report {
        let files = [
            FileInput { facts: &self.facts_a, summaries: &self.sums_a },
            FileInput { facts: &self.facts_b, summaries: &self.sums_b },
        ];
        build(&ReportInput {
            header: header(),
            linked: &self.linked,
            files: &files,
            package_facts: &self.pkg,
            not_analysed: &self.na,
            unrecorded_skips: self.unrecorded,
        })
        .unwrap()
    }
}

#[test]
fn a_mismatch_between_files_and_inputs_is_an_error_not_a_panic() {
    let fx = fixture();
    let files = [FileInput { facts: &fx.facts_a, summaries: &fx.sums_a }];
    let r = build(&ReportInput {
        header: header(),
        linked: &fx.linked,
        files: &files,
        package_facts: &[],
        not_analysed: &[],
        unrecorded_skips: 0,
    });
    assert_eq!(r.unwrap_err(), BuildError::FileCountMismatch { linked: 2, files: 1 });
}

#[test]
fn every_fact_appears_somewhere() {
    let fx = fixture();
    let r = fx.run();
    let seen = r.fact_ids();
    let all = fx.facts_a.iter().chain(&fx.facts_b).chain(&fx.pkg);
    for f in all {
        assert!(seen.contains(f.id().as_str()), "missing {:?} at {:?}", f.kind(), f.at());
    }
    assert_eq!(r.overview.facts as usize, fx.facts_a.len() + fx.facts_b.len() + fx.pkg.len());
}

#[test]
fn a_fact_no_function_claims_is_listed_as_unplaced() {
    let mut fx = fixture();
    let loose = fact(FactKind::Call, "a.js", 50, 5000, "loose", None);
    let id = loose.id().clone();
    fx.facts_a.push(loose);
    let r = fx.run();
    assert_eq!(r.functions.unplaced_facts, vec![id.clone()]);
    assert!(r.fact_ids().contains(id.as_str()));
}

#[test]
fn the_index_has_eleven_numbered_sections_and_its_counts_match_the_sections() {
    let r = fixture().run();
    assert_eq!(r.index.len(), 11);
    assert_eq!(r.index.iter().map(|i| i.number).collect::<Vec<_>>(), (1..=11).collect::<Vec<u8>>());
    assert!(r.index.iter().all(|i| !i.explanation.is_empty() && !i.anchor.is_empty()));
    let count = |s: SectionId| r.index.iter().find(|i| i.section == s).unwrap().count;
    assert_eq!(count(SectionId::Overview), None);
    assert_eq!(count(SectionId::Effects), Some(3));
    assert_eq!(count(SectionId::Flows), Some(1));
    assert_eq!(count(SectionId::Scripts), Some(1));
    assert_eq!(count(SectionId::Dependencies), Some(2));
    assert_eq!(count(SectionId::Unresolved), Some(3));
    assert_eq!(count(SectionId::Unlinked), Some(3));
    assert_eq!(count(SectionId::NotAnalysed), Some(5), "two listed plus three unrecorded");
    assert_eq!(count(SectionId::Functions), Some(3));
    assert_eq!(count(SectionId::Timeline), Some(3), "two functions with a lane plus one timer");
}

#[test]
fn anchors_are_unique() {
    let mut a: Vec<_> = SectionId::ALL.iter().map(|s| s.anchor()).collect();
    a.sort();
    a.dedup();
    assert_eq!(a.len(), 11);
}

#[test]
fn effects_are_grouped_most_worrying_first_and_sorted_by_place() {
    let mut fx = fixture();
    fx.facts_a.push(effect("a.js", 20, 2000, Effect::NetOut, Some(1)));
    fx.facts_a.push(effect("a.js", 1, 1, Effect::NetOut, Some(0)));
    let r = fx.run();
    let order: Vec<Effect> = r.effects.iter().map(|g| g.effect).collect();
    assert_eq!(order, vec![Effect::FsDelete, Effect::NetOut, Effect::FsRead]);
    let lines: Vec<u32> = r.effects[1].items.iter().map(|i| i.at.start_line).collect();
    assert_eq!(lines, vec![1, 2, 20], "by file and position inside a group");
}

#[test]
fn an_effect_says_whether_it_can_run() {
    let r = fixture().run();
    let net = r.effects.iter().find(|g| g.effect == Effect::NetOut).unwrap();
    assert_eq!(net.items[0].place.lanes, vec![Lane::Immediate]);
    assert_eq!(net.items[0].place.reachable, Some(true));
    let read = r.effects.iter().find(|g| g.effect == Effect::FsRead).unwrap();
    assert_eq!(read.items[0].place.lanes, Vec::<Lane>::new());
    assert_eq!(read.items[0].place.reachable, Some(false));
    let del = r.effects.iter().find(|g| g.effect == Effect::FsDelete).unwrap();
    let f = del.items[0].place.in_fn.as_ref().unwrap();
    assert_eq!((f.func, f.line), (FnId(1), Some(4)));
    assert_eq!(f.name.as_ref().unwrap().text(), "helper");
}

#[test]
fn the_timeline_lists_functions_by_lane_and_every_timer() {
    let r = fixture().run();
    let lanes: Vec<(Lane, usize)> = r.timeline.lanes.iter().map(|l| (l.lane, l.functions.len())).collect();
    assert_eq!(lanes, vec![(Lane::Immediate, 1), (Lane::Deferred, 1)]);
    assert_eq!(r.timeline.timers.len(), 1);
    assert_eq!(r.timeline.timers[0].timing.lane, Lane::Deferred);
}

#[test]
fn unresolved_items_are_grouped_by_reason_most_worrying_first() {
    let r = fixture().run();
    let reasons: Vec<UnresolvedReason> = r.unresolved.iter().map(|g| g.reason).collect();
    assert_eq!(reasons, vec![UnresolvedReason::Eval, UnresolvedReason::ShellCommand]);
    assert_eq!(r.unresolved[1].items.len(), 2);
    assert!(r.unresolved[1].items.iter().all(|i| i.place.in_fn.is_none() && i.place.reachable.is_none()));
}

#[test]
fn a_script_lists_the_commands_we_did_not_follow() {
    let r = fixture().run();
    let s = &r.scripts[0];
    assert_eq!(s.name.text(), "postinstall");
    assert_eq!(s.trigger, ScriptTrigger::Install);
    let un: Vec<&str> = s.unfollowed.iter().map(|q| q.text()).collect();
    assert_eq!(un, vec!["curl", "sh"], "in the order they were found");
}

#[test]
fn scripts_are_ordered_install_first() {
    let mut fx = fixture();
    let mut manual = StaticFact::new(FactKind::LifecycleScript, sp("package.json", 2, 5, 9), Some(Quoted::new("x")));
    manual.script = Some(Script { name: Quoted::new("start"), trigger: ScriptTrigger::Manual });
    fx.pkg.push(manual);
    let r = fx.run();
    let t: Vec<ScriptTrigger> = r.scripts.iter().map(|s| s.trigger).collect();
    assert_eq!(t, vec![ScriptTrigger::Install, ScriptTrigger::Manual]);
}

#[test]
fn the_same_import_from_several_functions_is_one_item_with_every_location() {
    let fx = fixture();
    let r = fx.run();
    let ext = r.unlinked.iter().find(|g| g.reason == UnlinkedReason::ExternalPackage).unwrap();
    assert_eq!(ext.items.len(), 1);
    let from: Vec<(FnId, Option<u32>)> = ext.items[0].from.iter().map(|f| (f.func, f.line)).collect();
    assert_eq!(from, vec![(FnId(0), Some(1)), (FnId(1), Some(4))]);
    let total: usize = r.unlinked.iter().flat_map(|g| &g.items).map(|i| i.from.len()).sum();
    assert_eq!(total, fx.linked.unlinked.len(), "merging must not lose a single link");
}

#[test]
fn unlinked_groups_put_the_worrying_reasons_first() {
    let r = fixture().run();
    let order: Vec<UnlinkedReason> = r.unlinked.iter().map(|g| g.reason).collect();
    assert_eq!(order, vec![UnlinkedReason::MissingFile, UnlinkedReason::ExternalPackage]);
}

#[test]
fn different_names_from_one_package_stay_separate() {
    let mut fx = fixture();
    fx.linked.unlinked.push(Unlinked {
        from: FnRef { file: 0, func: FnId(0) },
        spec: Quoted::new("lodash"),
        name: Some(Quoted::new("merge")),
        reason: UnlinkedReason::ExternalPackage,
    });
    let r = fx.run();
    let ext = r.unlinked.iter().find(|g| g.reason == UnlinkedReason::ExternalPackage).unwrap();
    assert_eq!(ext.items.len(), 2);
}

#[test]
fn not_analysed_files_are_grouped_by_kind_and_unrecorded_skips_are_counted() {
    let r = fixture().run();
    let kinds: Vec<NotAnalysedKind> = r.not_analysed.groups.iter().map(|g| g.kind).collect();
    assert_eq!(kinds, vec![NotAnalysedKind::NpmConfig, NotAnalysedKind::ShellScript]);
    assert_eq!(r.not_analysed.unrecorded, 3);
}

#[test]
fn classify_path_knows_the_usual_suspects() {
    use NotAnalysedKind::*;
    for (p, k) in [
        (".npmrc", NpmConfig),
        ("web/.yarnrc.yml", NpmConfig),
        ("scripts/setup.sh", ShellScript),
        ("install.PS1", ShellScript),
        ("build.bat", ShellScript),
        ("Dockerfile", Container),
        ("docker/Dockerfile.prod", Container),
        ("docker-compose.yml", Container),
        (".github/workflows/ci.yml", CiWorkflow),
        ("a\\.github\\workflows\\x.yaml", CiWorkflow),
        (".gitlab-ci.yml", CiWorkflow),
        ("Jenkinsfile", CiWorkflow),
        ("build/addon.node", NativeBinary),
        ("bin/tool.exe", NativeBinary),
        ("pkg/core.wasm", NativeBinary),
        ("README.md", OtherFile),
        ("img/logo.png", OtherFile),
    ] {
        assert_eq!(classify_path(p), k, "{p}");
    }
}

#[test]
fn coverage_always_lists_every_line_and_the_status_follows_it() {
    let r = fixture().run();
    assert_eq!(r.overview.coverage.len(), 6);
    let c = |s: SectionId| r.overview.coverage.iter().find(|c| c.section == s).unwrap().count;
    assert_eq!(c(SectionId::Unresolved), 3);
    assert_eq!(c(SectionId::Unlinked), 3);
    assert_eq!(c(SectionId::NotAnalysed), 5);
    assert_eq!(c(SectionId::EntryPoints), 0);
    assert_eq!(c(SectionId::Dependencies), 1, "only the registry one; a local path is ours");
    assert_eq!(c(SectionId::Functions), 3, "the calls we could not trace");
    assert_eq!(r.overview.completeness, Completeness::Incomplete);
}

#[test]
fn nothing_unseen_means_complete() {
    let mut fx = fixture();
    fx.facts_a.retain(|f| f.kind() != FactKind::Unresolved);
    fx.pkg.clear();
    fx.linked.unlinked.clear();
    fx.na.clear();
    fx.unrecorded = 0;
    for s in fx.sums_a.iter_mut() {
        s.unfollowed_calls = 0;
        s.unfollowed_sites.clear();
        s.unfollowed_callees.clear();
    }
    let r = fx.run();
    assert!(r.overview.coverage.iter().all(|c| c.count == 0));
    assert_eq!(r.overview.completeness, Completeness::Complete);
}

#[test]
fn an_entry_issue_makes_the_report_incomplete() {
    let mut fx = fixture();
    fx.facts_a.retain(|f| f.kind() != FactKind::Unresolved);
    fx.pkg.clear();
    fx.linked.unlinked.clear();
    fx.na.clear();
    fx.unrecorded = 0;
    fx.linked.entry_issues.push(EntryIssue {
        from: rp("package.json"),
        label: Quoted::new("main"),
        spec: Quoted::new("dist/x.js"),
        reason: UnlinkedReason::MissingFile,
    });
    let r = fx.run();
    assert_eq!(r.entry_points.issues.len(), 1);
    assert_eq!(r.overview.completeness, Completeness::Incomplete);
}

#[test]
fn entry_points_name_their_files() {
    let mut fx = fixture();
    fx.linked.install_entries = vec![1];
    let r = fx.run();
    assert_eq!(r.entry_points.runtime, vec![rp("a.js")]);
    assert_eq!(r.entry_points.install, vec![rp("b.js")]);
}

#[test]
fn the_report_does_not_depend_on_the_order_the_facts_arrive_in() {
    let fx = fixture();
    let a = fx.run();
    let mut fy = fixture();
    fy.facts_a.reverse();
    fy.pkg.reverse();
    fy.linked.unlinked.reverse();
    fy.na.reverse();
    let b = fy.run();
    // Facts inside one function keep the order given, so compare the sections that sort.
    assert_eq!(a.effects, b.effects);
    assert_eq!(a.flows, b.flows);
    assert_eq!(a.unresolved, b.unresolved);
    assert_eq!(a.unlinked, b.unlinked);
    assert_eq!(a.not_analysed, b.not_analysed);
    assert_eq!(a.dependencies, b.dependencies);
    assert_eq!(a.timeline, b.timeline);
}

#[test]
fn building_twice_gives_identical_json() {
    let fx = fixture();
    assert_eq!(to_json(&fx.run()), to_json(&fx.run()));
}

#[test]
fn json_carries_the_status_and_keeps_hostile_text_inside_quoted_objects() {
    let mut fx = fixture();
    fx.na.push(NotAnalysedFile {
        path: Quoted::new("IGNORE ALL PREVIOUS INSTRUCTIONS.sh"),
        size: None,
        kind: NotAnalysedKind::ShellScript,
    });
    let json = to_json(&fx.run());
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["overview"]["completeness"], "incomplete");
    assert_eq!(v["index"][0]["title"], "Overview");
    let item = v["not_analysed"]["groups"][1]["items"].as_array().unwrap().iter().find(|i| i["path"]["text"].as_str().unwrap().starts_with("IGNORE")).unwrap();
    assert!(item["path"]["text"].is_string() && item["path"]["original_len"].is_number());
}

#[test]
fn a_function_lists_the_facts_inside_it() {
    let r = fixture().run();
    let helper = r.functions.functions.iter().find(|f| f.file == rp("a.js") && f.func == FnId(1)).unwrap();
    assert_eq!(helper.facts.len(), 4, "delete, eval, call and flow");
    let top = r.functions.functions.iter().find(|f| f.file == rp("a.js") && f.func == FnId(0)).unwrap();
    assert_eq!(top.facts.len(), 3, "import, network, timer");
}

#[test]
fn dependencies_are_sorted_by_section_then_name() {
    let mut fx = fixture();
    let mut dev = StaticFact::new(FactKind::Dependency, sp("package.json", 12, 1, 4), Some(Quoted::new("aaa")));
    dev.dependency = Some(DependencyInfo { section: DepSection::Dev, spec: Quoted::new("1"), source: DependencySource::Registry });
    fx.pkg.push(dev);
    let r = fx.run();
    let names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_ref().unwrap().text()).collect();
    assert_eq!(names, vec!["lodash", "mine", "aaa"]);
}

// ---- Markdown ----

#[test]
fn markdown_has_the_index_and_a_heading_for_every_section() {
    let md = to_markdown(&fixture().run());
    assert!(md.starts_with("# Forge analysis report\n"));
    assert!(md.contains("<a id=\"index\"></a>\n\n## Index\n"));
    for s in SectionId::ALL {
        assert!(md.contains(&format!("[{}](#{})", s.title(), s.anchor())), "index link for {:?}", s);
        assert!(md.contains(&format!("<a id=\"{}\"></a>\n\n## {}. {}\n", s.anchor(), s.number(), s.title())), "heading for {:?}", s);
        assert!(md.contains(s.explanation()));
    }
}

#[test]
fn markdown_says_the_status_and_lists_every_coverage_line() {
    let md = to_markdown(&fixture().run());
    assert!(md.contains("**Status: incomplete.**"));
    assert_eq!(md.matches("| [").count(), 6, "one table row per coverage line");
}

#[test]
fn markdown_shows_what_matters_from_every_section() {
    let md = to_markdown(&fixture().run());
    for needle in [
        "lodash", "./gone", "`curl`", "`sh`", "postinstall", "run.sh", ".npmrc", "Deletes files", "Makes network requests",
        "Code built at run time", "External package", "File that does not exist", "Another 3 skipped items",
        "not followed: `curl`",
    ] {
        assert!(md.contains(needle), "missing {needle:?}");
    }
}

#[test]
fn every_fact_in_a_function_is_named_in_the_markdown() {
    let fx = fixture();
    let md = to_markdown(&fx.run());
    for f in fx.facts_a.iter().chain(&fx.facts_b) {
        assert!(md.contains(f.id().as_str()), "{:?}", f.id());
    }
}

#[test]
fn hostile_text_cannot_leave_its_code_span() {
    let mut fx = fixture();
    let nasty = "x`` ``` </details>\n# Heading\n[a](javascript:alert(1)) | a | b";
    fx.na.push(NotAnalysedFile { path: Quoted::new(nasty), size: Some(1), kind: NotAnalysedKind::OtherFile });
    fx.linked.unlinked.push(Unlinked {
        from: FnRef { file: 0, func: FnId(0) },
        spec: Quoted::new("`<script>alert(1)</script>`"),
        name: None,
        reason: UnlinkedReason::ExternalPackage,
    });
    let md = to_markdown(&fx.run());
    // The newline is stripped by Quoted, so the "heading" is mid-line inside the span.
    assert!(!md.lines().any(|l| l.starts_with("# Heading")));
    // The run of three backticks in the text means the span is fenced with four.
    assert!(md.contains("````"), "fence must be longer than any run inside");
    // Details blocks stay balanced: the text's own closing tag sits inside a code span.
    let opens = md.matches("<details>").count();
    let closes = md.lines().filter(|l| *l == "</details>").count();
    assert_eq!(opens, closes);
    // No raw tag from the target outside a span: every `<script>` is inside backticks.
    for line in md.lines().filter(|l| l.contains("<script>")) {
        let before = &line[..line.find("<script>").unwrap()];
        assert!(before.matches('`').count() % 2 == 1, "tag outside a span: {line}");
    }
}

#[test]
fn a_cut_text_says_how_long_it_really_was() {
    let mut fx = fixture();
    fx.na.push(NotAnalysedFile { path: Quoted::new(&"a".repeat(1000)), size: None, kind: NotAnalysedKind::OtherFile });
    let md = to_markdown(&fx.run());
    assert!(md.contains("(cut; 1000 characters in all)"));
    assert!(md.contains("size unknown"));
}

#[test]
fn long_groups_are_collapsed_and_short_ones_are_not() {
    let fx = fixture();
    let short = to_markdown(&fx.run());
    assert_eq!(short.matches("<details>").count(), 0);

    let mut big = fixture();
    for i in 0..(COLLAPSE_AT as u32 + 5) {
        big.facts_b.push(effect("b.js", 10 + i, 1000 + i * 10, Effect::FsRead, Some(0)));
    }
    let md = to_markdown(&big.run());
    assert!(md.contains("<summary>Reads files (31)</summary>"));
    assert_eq!(md.matches("<details>").count(), md.lines().filter(|l| *l == "</details>").count());
    let r = big.run();
    let listed: usize = r.effects.iter().find(|g| g.effect == Effect::FsRead).unwrap().items.len();
    let in_md = md.lines().filter(|l| l.starts_with("- `b.js:")).count();
    assert!(in_md >= listed, "collapsing must not cut items: {in_md} < {listed}");
}

#[test]
fn an_empty_section_says_so() {
    let mut fx = fixture();
    fx.facts_a.retain(|f| f.kind() != FactKind::Flow);
    let md = to_markdown(&fx.run());
    let flows = md.split("## 5. Data flows").nth(1).unwrap().split("## 6.").next().unwrap();
    assert!(flows.contains("None found."));
}

#[test]
fn markdown_is_identical_on_every_build() {
    let fx = fixture();
    assert_eq!(to_markdown(&fx.run()), to_markdown(&fx.run()));
}

// ---- golden files ----
// Run with FORGE_BLESS=1 to rewrite them after an intended change, then read the diff.

fn golden(name: &str, actual: &str) {
    let path = format!("{}/tests/golden/{}", env!("CARGO_MANIFEST_DIR"), name);
    if std::env::var("FORGE_BLESS").is_ok() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing {path}; run with FORGE_BLESS=1"));
    // Git may check files out with Windows line endings.
    let expected = expected.replace("\r\n", "\n");
    assert!(expected == actual, "{name} differs from the golden file. If the change is intended, run with FORGE_BLESS=1 and review the diff.");
}

#[test]
fn golden_json() {
    golden("fixture.json", &(to_json(&fixture().run()) + "\n"));
}

#[test]
fn golden_markdown() {
    golden("fixture.md", &to_markdown(&fixture().run()));
}

#[test]
fn a_hostile_file_name_cannot_inject_into_the_function_index_heading_or_summary() {
    for functions in [1u32, COLLAPSE_AT as u32 + 3] {
        let name = "![img](http)<script>alert(1)</script>.js";
        let mut fx = fixture();
        fx.linked.files[1] = rp(name);
        fx.facts_b.clear();
        fx.sums_b = (0..functions).map(|i| summary(name, i, None, 1 + i, vec![])).collect();
        fx.linked.fns.retain(|f| f.at.file != 1);
        for i in 0..functions {
            fx.linked.fns.push(linked_fn(1, i, vec![]));
        }
        fx.linked.unlinked.retain(|u| u.from.file != 1);
        let md = to_markdown(&fx.run());
        let lines: Vec<&str> = md.lines().filter(|l| l.contains("alert(1)")).collect();
        assert!(!lines.is_empty());
        for l in lines {
            if l.starts_with("<summary>") {
                assert!(!l.contains("<script>") && l.contains("&lt;script&gt;"), "{l}");
            } else {
                let before = &l[..l.find("<script>").unwrap()];
                assert!(before.matches('`').count() % 2 == 1, "tag outside a code span: {l}");
            }
        }
    }
}

#[test]
fn built_output_that_mirrors_source_is_flagged_and_nothing_is_dropped() {
    let files = vec![
        rp("src/a.ts"),
        rp("src/ui/b.tsx"),
        rp("dist/a.js"),
        rp("dist/ui/b.js"),
        rp("tools/x.js"),
    ];
    let notes = crate::build::mirror_notes(&files);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].output_dir.text(), "dist");
    assert_eq!(notes[0].source_dir.text(), "src");
    assert_eq!(notes[0].matching_files, 2);
    assert_eq!(notes[0].output_files, 2);
}

#[test]
fn an_unrelated_dist_folder_is_not_flagged() {
    let files = vec![rp("src/a.ts"), rp("dist/other.js")];
    assert!(crate::build::mirror_notes(&files).is_empty());
}

#[test]
fn the_overview_counts_functions_no_entry_reaches_and_does_not_change_the_status() {
    let r = fixture().run();
    let unreached = r.functions.functions.iter().filter(|f| !f.reachable_from_entry).count() as u32;
    assert_eq!(r.overview.unreached_functions, unreached);
    assert!(r.overview.coverage.iter().all(|c| !c.label.contains("reach")), "a note, not a coverage line");
    assert!(crate::markdown::to_markdown(&r).contains("not reached from any entry point") == (unreached > 0));
}

#[test]
fn every_call_we_could_not_trace_is_named_once_per_function_and_once_per_group() {
    let r = fixture().run();
    let in_functions: u32 = r.functions.functions.iter().map(|f| f.untraced_calls.len() as u32).sum();
    let in_groups: u32 = r.functions.untraced_by_callee.iter().map(|g| g.count).sum();
    let line = r.overview.coverage.iter().find(|c| c.section == SectionId::Functions).unwrap().count;
    assert_eq!((in_functions, in_groups), (line, line));
    assert!(line >= 3, "the fixture has three untraced calls");
    for f in &r.functions.functions {
        assert_eq!(f.untraced_calls.len() as u32, f.calls_not_traced);
    }
    let first = &r.functions.untraced_by_callee[0];
    assert_eq!((first.callee.text(), first.count, first.sites.len()), ("console.log", 2, 2), "most common first");
    let md = to_markdown(&r);
    assert!(md.contains("Calls we could not trace, by what they call"));
    assert!(md.contains("not traced: `console.log` at line 2"));
}
