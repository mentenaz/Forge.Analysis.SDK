//! Markdown writer. Same content as the JSON, laid out for a person: an index with a plain
//! explanation per section, then every section in full. Long groups are collapsed with
//! `<details>`, never cut.
//!
//! Text from the analysed code is only ever written inside a code span whose fence is longer
//! than any run of backticks in the text, so it cannot close the span, start a heading,
//! open a tag or end a collapsed block.

use crate::model::*;
use forge_analysis_schema::{
    DepSection, DependencySource, Delay, Effect, EntryBasis, FactKind, FlowSink, FnId, Lane, Mode, Quoted,
    ScriptTrigger, Span, UnlinkedReason, UnresolvedReason, UrlValue,
};
use std::fmt::Write;

/// A group with more items than this is collapsed.
pub const COLLAPSE_AT: usize = 25;

fn code(s: &str) -> String {
    if s.is_empty() {
        return "(empty)".to_string();
    }
    let mut longest = 0;
    let mut run = 0;
    for c in s.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(longest + 1);
    let pad = if s.starts_with('`') || s.ends_with('`') || s.starts_with(' ') || s.ends_with(' ') { " " } else { "" };
    format!("{fence}{pad}{s}{pad}{fence}")
}

fn q(x: &Quoted) -> String {
    let mut s = code(x.text());
    if x.truncated() {
        let _ = write!(s, " (cut; {} characters in all)", x.original_len());
    }
    s
}

fn oq(x: &Option<Quoted>) -> String {
    x.as_ref().map(q).unwrap_or_else(|| "(none)".to_string())
}

fn loc(at: &Span) -> String {
    code(&format!("{}:{}", at.file.as_quoted().text(), at.start_line))
}

fn fn_loc(f: &FnLoc) -> String {
    let line = f.line.map(|l| format!(":{l}")).unwrap_or_default();
    let file = f.file.as_quoted().text();
    match (&f.name, f.func) {
        (Some(n), _) => format!("{} in {}", q(n), code(&format!("{file}{line}"))),
        (None, FnId(0)) => format!("top level of {}", code(file)),
        (None, _) => format!("anonymous function in {}", code(&format!("{file}{line}"))),
    }
}

fn lane_word(l: Lane) -> &'static str {
    match l {
        Lane::Immediate => "immediate",
        Lane::Deferred => "deferred",
        Lane::Recurring => "recurring",
        Lane::EventDriven => "event-driven",
        Lane::Exit => "exit",
        Lane::Install => "install",
    }
}

fn lane_text(l: Lane) -> &'static str {
    match l {
        Lane::Immediate => "Runs right away when the program starts.",
        Lane::Deferred => "Runs once, later (a timer or a callback).",
        Lane::Recurring => "Runs again and again (an interval).",
        Lane::EventDriven => "Runs when something happens (a listener or a request).",
        Lane::Exit => "Runs when the program is shutting down.",
        Lane::Install => "Runs when the package is installed, before anything else of it.",
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn lanes_text(lanes: &[Lane]) -> String {
    lanes.iter().map(|l| lane_word(*l)).collect::<Vec<_>>().join(", ")
}

fn place_text(p: &Place) -> String {
    match (p.reachable, &p.in_fn) {
        (Some(true), _) => format!("runs: {}", lanes_text(&p.lanes)),
        (Some(false), _) => "not reached from any entry".to_string(),
        (None, Some(_)) => "reachability unknown".to_string(),
        (None, None) => String::new(),
    }
}

/// `where – in function – whether it runs`, skipping what is not there.
fn at_line(at: &Span, place: &Place) -> String {
    let mut s = loc(at);
    if let Some(f) = &place.in_fn {
        let _ = write!(s, " in {}", fn_loc(f));
    }
    let p = place_text(place);
    if !p.is_empty() {
        let _ = write!(s, " ({p})");
    }
    s
}

fn effect_word(e: Effect) -> &'static str {
    match e {
        Effect::FsRead => "reads files",
        Effect::FsWrite => "writes files",
        Effect::FsDelete => "deletes files",
        Effect::FsChmod => "changes file permissions",
        Effect::NetOut => "makes network requests",
        Effect::NetListen => "listens on the network",
        Effect::ProcessSpawn => "starts other programs",
        Effect::EnvRead => "reads environment variables",
        Effect::SystemInfo => "reads system information",
        Effect::DynamicLoad => "loads code by a computed name",
        Effect::CodeLoad => "runs code built at run time",
        Effect::Timer => "schedules work",
    }
}

fn unresolved_text(r: UnresolvedReason) -> (&'static str, &'static str) {
    use UnresolvedReason::*;
    match r {
        Eval => ("Code built at run time", "Text is turned into code and run; what it does cannot be read from the source."),
        DynamicRequire => ("Module chosen at run time", "A module is loaded by a name computed while running."),
        Reflection => ("Reflection", "Code reaches functions through the language's own machinery (for example `constructor.constructor`)."),
        ComputedAccess => ("Computed property access", "A member is picked by a computed name, so which function runs is unknown."),
        IndirectUse => ("Indirect use", "A function is used as a value (passed on, stored or aliased), so where it runs is unknown."),
        UnknownReceiver => ("Unknown receiver", "A method with a known name is called on something we cannot identify."),
        EventHandler => ("Event handler we cannot see", "A callback is registered but its code is not visible here."),
        RuntimeDelay => ("Delay known only at run time", "A timer's delay is computed, so when it fires is unknown."),
        UncoveredModule => ("Module our catalog does not cover", "Code uses a built-in module we have no description of yet."),
        UncatalogedMember => ("Function our catalog does not cover", "A function of a covered module is not in the catalog yet."),
        ShellCommand => ("Shell command not followed", "A command in a script that is not `node <file>` or `npm run <script>`, or one built at run time."),
        AmbiguousBinding => ("Ambiguous name", "A name is bound more than once, so which one is used is unknown."),
        UnsupportedSyntax => ("Syntax not supported", "A form this version of the analysis does not read."),
        ParseFailed => ("File did not parse", "The file has syntax errors, so nothing in it is known."),
        TooDeeplyNested => ("File too deeply nested to read safely", "The file nests brackets so deeply, or the machine could not give the analysis enough room to read it safely, so it was not read and nothing in it is known."),
    }
}

fn unlinked_text(r: UnlinkedReason) -> (&'static str, &'static str) {
    use UnlinkedReason::*;
    match r {
        OpaqueExport => ("Export we cannot identify", "The name is exported, but not as a function we can identify, or the file builds its exports in ways we cannot follow."),
        NoSuchExport => ("Name that is not exported", "The file is analysed, its exports are fully known, and it does not export this name."),
        MissingFile => ("File that does not exist", "A relative path that matches no file."),
        OutsideRoot => ("Path outside the folder", "The path climbs out of the analysed folder."),
        ReexportCycle => ("Re-export cycle", "Re-exports lead back to themselves."),
        NotAnalysed => ("File that is not code we analyse", "The file exists but is JSON, native code or another kind we do not read as code."),
        SubpathImport => ("Package-internal alias", "A `#name` import mapped by `package.json`, which is not followed yet."),
        ExternalPackage => ("External package", "Code from a package. Its behaviour was not analysed. (A path alias from a bundler looks the same and may be a local file.)"),
    }
}

fn not_analysed_text(k: NotAnalysedKind) -> (&'static str, &'static str) {
    use NotAnalysedKind::*;
    match k {
        NpmConfig => ("Package manager settings", "`.npmrc` and similar: can change where packages come from and run hooks."),
        ShellScript => ("Shell and batch scripts", "Not read. They can run anything."),
        Container => ("Container definitions", "Dockerfiles and compose files: build steps and what runs in the container."),
        CiWorkflow => ("CI and workflow files", "Pipelines that run on a server when the code changes."),
        NativeBinary => ("Native binaries and WebAssembly", "Compiled code: not read."),
        InvalidUtf8 => ("Read with replaced bytes", "Not valid UTF-8. Analysed after replacing the bad bytes, so some names and strings may differ from the file."),
        OtherFile => ("Other files", "Not code we analyse (documents, images, data, unknown types). Listed so nothing is hidden."),
        TooLarge => ("Skipped: too large", "Over the size limit for one file."),
        Symlink => ("Skipped: links", "Links are never followed."),
        IgnoredDir => ("Skipped: ignored folders", "Folders skipped by rule (such as `node_modules`)."),
        TooDeep => ("Skipped: nested too deep", "Folders deeper than the depth limit."),
        CapReached => ("Skipped: a limit was reached", "The total size or file count limit stopped the intake."),
        Unreadable => ("Skipped: could not be read", "The operating system refused or failed to read it."),
        Excluded => ("Skipped: not a regular file or a bad path", "Devices, odd names or paths outside the folder."),
        Stopped => ("Not reached: the run was stopped", "The time budget ran out or the caller cancelled before these files were read."),
    }
}

fn trigger_text(t: ScriptTrigger) -> &'static str {
    match t {
        ScriptTrigger::Install => "runs at install",
        ScriptTrigger::Prepare => "runs on a local or git install",
        ScriptTrigger::Hook => "runs around another command",
        ScriptTrigger::Manual => "runs only when someone asks",
    }
}

fn dep_section(s: DepSection) -> &'static str {
    match s {
        DepSection::Dependencies => "dependencies",
        DepSection::Dev => "devDependencies",
        DepSection::Optional => "optionalDependencies",
        DepSection::Peer => "peerDependencies",
        DepSection::Bundled => "bundledDependencies",
        DepSection::Overrides => "overrides and resolutions",
    }
}

fn dep_source(s: DependencySource) -> &'static str {
    match s {
        DependencySource::Registry => "package registry",
        DependencySource::Alias => "registry package under another name",
        DependencySource::Git => "git repository",
        DependencySource::Url => "tarball from a URL",
        DependencySource::LocalPath => "local path",
        DependencySource::Workspace => "another package of this repository",
        DependencySource::Bundled => "shipped inside the package",
        DependencySource::Other => "form we do not recognise",
    }
}

fn kind_word(k: FactKind) -> &'static str {
    match k {
        FactKind::Entry => "entry",
        FactKind::Import => "import",
        FactKind::Call => "call",
        FactKind::Effect => "effect",
        FactKind::Timer => "timer",
        FactKind::LifecycleScript => "script",
        FactKind::Dependency => "dependency",
        FactKind::Unresolved => "unresolved",
        FactKind::Flow => "flow",
    }
}

fn url_text(u: &UrlValue) -> String {
    use forge_analysis_schema::Scheme::*;
    let scheme = match u.scheme {
        Http => "http",
        Https => "https",
        Ws => "ws",
        Wss => "wss",
        Other => "(other scheme)",
    };
    let port = u.port.map(|p| format!(":{p}")).unwrap_or_default();
    let mut s = code(&format!("{scheme}://{}{port}{}", u.host.text(), u.path.text()));
    if u.has_userinfo {
        s.push_str(" (contains a user name or password)");
    }
    if !u.query_keys.is_empty() {
        let keys: Vec<String> = u.query_keys.iter().map(q).collect();
        let _ = write!(s, " (query keys: {})", keys.join(", "));
    }
    s
}

/// For text that sits inside raw HTML (`<summary>`), where Markdown is not read but tags are.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn size_text(n: Option<u64>) -> String {
    match n {
        Some(b) => format!("{b} bytes"),
        None => "size unknown".to_string(),
    }
}

struct W {
    out: String,
}

impl W {
    fn line(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }
    fn blank(&mut self) {
        self.out.push('\n');
    }
    fn item(&mut self, s: &str) {
        let _ = writeln!(self.out, "- {s}");
    }
    fn heading(&mut self, s: &SectionId) {
        self.blank();
        let _ = writeln!(self.out, "<a id=\"{}\"></a>", s.anchor());
        self.blank();
        let _ = writeln!(self.out, "## {}. {}", s.number(), s.title());
        self.blank();
        self.line(s.explanation());
        self.blank();
    }
    fn back(&mut self) {
        if !self.out.ends_with("\n\n") {
            self.blank();
        }
        self.line("[Back to the index](#index)");
    }
    /// A group of items, collapsed when long. `body` writes the items.
    fn group(&mut self, title: &str, note: &str, count: usize, body: impl FnOnce(&mut W)) {
        if count > COLLAPSE_AT {
            let _ = writeln!(self.out, "<details>");
            let _ = writeln!(self.out, "<summary>{} ({count})</summary>", html_escape(title));
            self.blank();
        } else {
            let _ = writeln!(self.out, "### {title} ({count})");
            self.blank();
        }
        if !note.is_empty() {
            self.line(note);
            self.blank();
        }
        body(self);
        if count > COLLAPSE_AT {
            self.blank();
            self.line("</details>");
        }
        self.blank();
    }
    fn none(&mut self) {
        self.line("None found.");
    }
}

pub fn to_markdown(r: &Report) -> String {
    let mut w = W { out: String::new() };
    w.line("# Forge analysis report");
    w.blank();
    let mode = match r.header.mode {
        Mode::StaticOnly => "static only (no code was run)",
        Mode::WithRun => "static analysis plus a sandboxed run",
    };
    w.item(&format!("Engine: {}", code(&r.header.engine)));
    w.item(&format!("Schema: {}", r.header.schema));
    w.item(&format!("Mode: {mode}"));
    w.item(&format!("Snapshot: {}", code(r.header.snapshot_sha256.as_str())));
    w.item(&format!("Catalog: {}", code(r.header.catalog_sha256.as_str())));

    w.blank();
    w.line("<a id=\"index\"></a>");
    w.blank();
    w.line("## Index");
    w.blank();
    for e in &r.index {
        let count = e.count.map(|c| format!(" ({c})")).unwrap_or_default();
        w.line(&format!("{}. [{}](#{}){} — {}", e.number, e.title, e.anchor, count, e.explanation));
    }

    overview(&mut w, r);
    entry_points(&mut w, r);
    timeline(&mut w, r);
    effects(&mut w, r);
    flows(&mut w, r);
    scripts(&mut w, r);
    dependencies(&mut w, r);
    unresolved(&mut w, r);
    unlinked(&mut w, r);
    not_analysed(&mut w, r);
    functions(&mut w, r);
    w.out
}

fn overview(w: &mut W, r: &Report) {
    let o = &r.overview;
    w.heading(&SectionId::Overview);
    match o.completeness {
        Completeness::Complete => w.line("**Status: complete.** Nothing was left unseen."),
        Completeness::Incomplete => w.line(
            "**Status: incomplete.** Some of the code could not be seen. Each line below says how much, and links to the section that lists every item.",
        ),
    }
    w.blank();
    w.line("| What we could not see | Count | Listed in |");
    w.line("|---|---|---|");
    for c in &o.coverage {
        w.line(&format!("| {} | {} | [{}](#{}) |", c.label, c.count, c.section.title(), c.section.anchor()));
    }
    w.blank();
    w.item(&format!("Files analysed: {}", o.files_analysed));
    w.item(&format!("Files not analysed: {}", o.files_not_analysed));
    w.item(&format!("Functions: {}", o.functions));
    w.item(&format!("Cross-file links made: {}", o.links_made));
    w.item(&format!(
        "Unresolved facts not listed as open because their call was followed to its source: {}",
        o.unresolved_cleared_by_links
    ));
    for n in &o.mirrored_output {
        w.item(&format!(
            "Note: {} looks like built output of {} ({} of {} files share a name). Both were analysed and both are reported, so the same behaviour can appear twice.",
            q(&n.output_dir),
            q(&n.source_dir),
            n.matching_files,
            n.output_files
        ));
    }
    if o.unreached_functions > 0 {
        w.item(&format!(
            "Note: {} of {} functions are not reached from any entry point by a call, a callback or a name used as a value we could follow. They are listed in the function index and their facts are reported, but nothing says they run.",
            o.unreached_functions, o.functions
        ));
    }
    w.item(&format!("Facts found: {}", o.facts));
    let kinds: Vec<String> = o.fact_counts.iter().map(|f| format!("{} {}", f.count, kind_word(f.kind))).collect();
    w.item(&format!("Facts by kind: {}", kinds.join(", ")));
    w.back();
}

fn entry_points(w: &mut W, r: &Report) {
    let e = &r.entry_points;
    w.heading(&SectionId::EntryPoints);
    match e.basis {
        EntryBasis::Declared => w.line("Entry points come from what the package declares (`main`, `bin`, `exports`, scripts)."),
        EntryBasis::Roots => w.line("Nothing usable was declared, so every file that no other file imports is treated as an entry."),
    }
    w.blank();
    let files = |w: &mut W, files: &[forge_analysis_schema::RelPath]| {
        if files.is_empty() {
            w.none();
        }
        for f in files {
            w.item(&q(f.as_quoted()));
        }
    };
    w.group("Runtime entry files", "", e.runtime.len(), |w| files(w, &e.runtime));
    w.group("Install entry files", "Run when the package is installed.", e.install.len(), |w| files(w, &e.install));
    w.group("Declared entries that match no analysed file", "", e.issues.len(), |w| {
        if e.issues.is_empty() {
            w.none();
        }
        for i in &e.issues {
            w.item(&format!(
                "{} in {} points at {}: {}",
                q(&i.label),
                q(i.from.as_quoted()),
                q(&i.spec),
                unlinked_text(i.reason).0.to_lowercase()
            ));
        }
    });
    w.back();
}

fn timeline(w: &mut W, r: &Report) {
    let t = &r.timeline;
    w.heading(&SectionId::Timeline);
    if t.lanes.is_empty() {
        w.line("No function is reached from an entry.");
        w.blank();
    }
    for g in &t.lanes {
        w.group(&cap(lane_word(g.lane)), lane_text(g.lane), g.functions.len(), |w| {
            for f in &g.functions {
                w.item(&fn_loc(f));
            }
        });
    }
    w.group("Timers", "", t.timers.len(), |w| {
        if t.timers.is_empty() {
            w.none();
        }
        for x in &t.timers {
            let delay = match x.timing.delay_ms {
                Delay::Known(ms) => format!("{ms} ms"),
                Delay::Unresolved => "delay known only at run time".to_string(),
            };
            let cleared = match x.timing.cleared {
                Some(true) => "cleared",
                Some(false) => "never cleared",
                None => "clearing unknown",
            };
            w.item(&format!("{}: {}, {}, {}, {}", at_line(&x.at, &x.place), lane_word(x.timing.lane), delay, cleared, oq(&x.detail)));
        }
    });
    w.back();
}

fn effects(w: &mut W, r: &Report) {
    w.heading(&SectionId::Effects);
    if r.effects.is_empty() {
        w.none();
    }
    for g in &r.effects {
        w.group(&cap(effect_word(g.effect)), "", g.items.len(), |w| {
            for i in &g.items {
                let mut s = at_line(&i.at, &i.place);
                if let Some(u) = &i.url {
                    let _ = write!(s, ": {}", url_text(u));
                } else if i.detail.is_some() {
                    let _ = write!(s, ": {}", oq(&i.detail));
                }
                w.item(&s);
            }
        });
    }
    w.back();
}

fn flows(w: &mut W, r: &Report) {
    w.heading(&SectionId::Flows);
    if r.flows.is_empty() {
        w.none();
    } else {
        w.group("Flows", "Each line: where the data is used, where it came from, and the expression it came from. \"calls it\" means the thing being called is network data, so the server decides what runs.", r.flows.len(), |w| {
            for f in &r.flows {
                let how = match f.sink {
                    FlowSink::Code => "",
                    FlowSink::Callee => " (calls it)",
                };
                w.item(&format!("{}{} ← {}: {}", at_line(&f.at, &f.place), how, loc(&f.origin), oq(&f.detail)));
            }
        });
    }
    w.back();
}

fn scripts(w: &mut W, r: &Report) {
    w.heading(&SectionId::Scripts);
    if r.scripts.is_empty() {
        w.none();
    } else {
        w.group("Scripts", "", r.scripts.len(), |w| {
            for s in &r.scripts {
                w.item(&format!("{} ({}) at {}: {}", q(&s.name), trigger_text(s.trigger), loc(&s.at), oq(&s.command)));
                for u in &s.unfollowed {
                    w.line(&format!("  - not followed: {}", q(u)));
                }
            }
        });
    }
    w.back();
}

fn dependencies(w: &mut W, r: &Report) {
    w.heading(&SectionId::Dependencies);
    if r.dependencies.is_empty() {
        w.none();
    }
    let mut sections: Vec<DepSection> = r.dependencies.iter().map(|d| d.info.section).collect();
    sections.dedup();
    for s in sections {
        let items: Vec<&DependencyItem> = r.dependencies.iter().filter(|d| d.info.section == s).collect();
        w.group(dep_section(s), "", items.len(), |w| {
            for d in &items {
                w.item(&format!("{} {}: {} (at {})", oq(&d.name), q(&d.info.spec), dep_source(d.info.source), loc(&d.at)));
            }
        });
    }
    w.back();
}

fn unresolved(w: &mut W, r: &Report) {
    w.heading(&SectionId::Unresolved);
    if r.unresolved.is_empty() {
        w.none();
    }
    for g in &r.unresolved {
        let (title, note) = unresolved_text(g.reason);
        w.group(title, note, g.items.len(), |w| {
            for i in &g.items {
                let mut s = at_line(&i.at, &i.place);
                if i.detail.is_some() {
                    let _ = write!(s, ": {}", oq(&i.detail));
                }
                w.item(&s);
            }
        });
    }
    w.back();
}

fn unlinked(w: &mut W, r: &Report) {
    w.heading(&SectionId::Unlinked);
    if r.unlinked.is_empty() {
        w.none();
    }
    for g in &r.unlinked {
        let (title, note) = unlinked_text(g.reason);
        w.group(title, note, g.items.len(), |w| {
            for i in &g.items {
                let name = i.name.as_ref().map(|n| format!(" → {}", q(n))).unwrap_or_default();
                let from: Vec<String> = i.from.iter().map(fn_loc).collect();
                w.item(&format!("{}{} — imported by {}", q(&i.spec), name, from.join("; ")));
            }
        });
    }
    for p in &r.package_calls {
        let title = format!("Calls made into {}", q(&p.spec));
        let note = format!(
            "{} {} from {} {}. The package was not analysed, so what these calls do is not known. Each call is also listed in its function.",
            p.calls,
            if p.calls == 1 { "call" } else { "calls" },
            p.functions,
            if p.functions == 1 { "function" } else { "functions" },
        );
        w.group(&title, &note, p.items.len(), |w| {
            for i in &p.items {
                let mut s = at_line(&i.at, &i.place);
                if i.detail.is_some() {
                    let _ = write!(s, ": {}", oq(&i.detail));
                }
                w.item(&s);
            }
        });
    }
    w.back();
}

fn not_analysed(w: &mut W, r: &Report) {
    w.heading(&SectionId::NotAnalysed);
    if r.not_analysed.groups.is_empty() && r.not_analysed.unrecorded == 0 {
        w.none();
    }
    for g in &r.not_analysed.groups {
        let (title, note) = not_analysed_text(g.kind);
        w.group(title, note, g.items.len(), |w| {
            for i in &g.items {
                w.item(&format!("{} ({})", q(&i.path), size_text(i.size)));
            }
        });
    }
    if r.not_analysed.unrecorded > 0 {
        w.line(&format!(
            "Another {} skipped items were counted but not listed, because the intake record limit was reached.",
            r.not_analysed.unrecorded
        ));
        w.blank();
    }
    w.back();
}

fn functions(w: &mut W, r: &Report) {
    w.heading(&SectionId::Functions);
    let mut files: Vec<&forge_analysis_schema::RelPath> = Vec::new();
    for f in &r.functions.functions {
        if files.last() != Some(&&f.file) {
            files.push(&f.file);
        }
    }
    if !r.functions.untraced_by_callee.is_empty() {
        let total: usize = r.functions.untraced_by_callee.iter().map(|g| g.count as usize).sum();
        w.group(
            "Calls we could not trace, by what they call",
            "Nothing in the analysis says what these run. The label is made from names only; `(..)` stands for a computed receiver. Every call is also listed, with its line, under its function below.",
            total,
            |w| {
                for g in &r.functions.untraced_by_callee {
                    let mut fns: Vec<(String, u32)> = g.sites.iter().map(|s| (q(s.in_fn.file.as_quoted()), s.in_fn.func.0)).collect();
                    fns.sort();
                    fns.dedup();
                    let files: std::collections::BTreeSet<&String> = fns.iter().map(|(f, _)| f).collect();
                    w.item(&format!("{} ×{} in {} function(s) across {} file(s)", q(&g.callee), g.count, fns.len(), files.len()));
                }
            },
        );
    }
    for file in files {
        let fns: Vec<&FunctionItem> = r.functions.functions.iter().filter(|f| &f.file == file).collect();
        w.group(&q(file.as_quoted()), "", fns.len(), |w| {
            for f in &fns {
                let name = match (&f.name, f.func) {
                    (Some(n), _) => q(n),
                    (None, FnId(0)) => "top level".to_string(),
                    (None, _) => "anonymous".to_string(),
                };
                w.item(&format!("{} at line {}", name, f.at.start_line));
                let lanes = if f.lanes.is_empty() { "none (not reached from an entry)".to_string() } else { lanes_text(&f.lanes) };
                w.line(&format!("  - runs: {lanes}{}", if f.escapes { "; also used as a value, so it may run from elsewhere" } else { "" }));
                if !f.transitive_effects.is_empty() {
                    let e: Vec<&str> = f.transitive_effects.iter().map(|e| effect_word(*e)).collect();
                    w.line(&format!("  - can reach: {}", e.join(", ")));
                }
                if !f.unresolved.is_empty() {
                    let u: Vec<&str> = f.unresolved.iter().map(|u| unresolved_text(*u).0).collect();
                    w.line(&format!("  - unresolved inside: {}", u.join(", ")));
                }
                if f.unfollowed_calls > 0 {
                    w.line(&format!(
                        "  - calls to something other than a function in this file: {} ({} followed through an import or a prop, {} matched to a fact, {} not traced to any code)",
                        f.unfollowed_calls, f.calls_followed_across_files, f.calls_matched_to_a_fact, f.calls_not_traced
                    ));
                }
                if !f.untraced_calls.is_empty() {
                    let mut by_label: Vec<(String, Vec<u32>)> = Vec::new();
                    for u in &f.untraced_calls {
                        let l = q(&u.callee);
                        match by_label.iter_mut().find(|(k, _)| *k == l) {
                            Some((_, lines)) => lines.push(u.line),
                            None => by_label.push((l, vec![u.line])),
                        }
                    }
                    for (label, lines) in by_label {
                        let ls: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
                        w.line(&format!("  - not traced: {} at line {}", label, ls.join(", ")));
                    }
                }
                for c in &f.followed_calls {
                    let target = match (&c.to.name, c.to.func) {
                        (Some(n), _) => q(n),
                        (None, FnId(0)) => "top level".to_string(),
                        (None, _) => "an anonymous function".to_string(),
                    };
                    let site = match c.line {
                        Some(l) => format!("line {l}"),
                        None => format!("bytes {}-{}", c.start_byte, c.end_byte),
                    };
                    let how = if c.via_prop {
                        format!("through the function handed over as prop {}", q(&c.name))
                    } else {
                        format!("through {}, export {}", q(&c.spec), q(&c.name))
                    };
                    w.line(&format!(
                        "  - followed call at {} to {} in {}{} ({})",
                        site,
                        target,
                        q(c.to.file.as_quoted()),
                        c.to.line.map(|l| format!(" line {l}")).unwrap_or_default(),
                        how,
                    ));
                }
                if !f.calls_into_packages.is_empty() {
                    let p: Vec<String> = f.calls_into_packages.iter().map(|c| format!("{} ×{}", q(&c.spec), c.count)).collect();
                    w.line(&format!("  - calls into packages we did not analyse: {}", p.join(", ")));
                }
                if !f.followed_called_from.is_empty() {
                    let from: Vec<String> = f
                        .followed_called_from
                        .iter()
                        .map(|c| {
                            let n = match &c.name {
                                Some(n) => q(n),
                                None => "top level".to_string(),
                            };
                            format!("{} in {}", n, q(c.file.as_quoted()))
                        })
                        .collect();
                    w.line(&format!("  - called through an import or a prop from: {}", from.join(", ")));
                }
                if !f.facts.is_empty() {
                    let ids: Vec<String> = f.facts.iter().map(|i| code(i.as_str())).collect();
                    w.line(&format!("  - facts: {}", ids.join(", ")));
                }
            }
        });
    }
    if !r.functions.unplaced_facts.is_empty() {
        let ids: Vec<String> = r.functions.unplaced_facts.iter().map(|i| code(i.as_str())).collect();
        w.group("Facts not placed in any function", "", ids.len(), |w| {
            for i in &ids {
                w.item(i);
            }
        });
    }
    w.back();
}
