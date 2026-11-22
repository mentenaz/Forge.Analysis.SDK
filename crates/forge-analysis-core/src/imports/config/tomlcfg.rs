//! The TOML files that name programs for tools to run: Cargo's config and manifest, `rust-toolchain`,
//! `pyproject.toml`, `mise.toml`, and the Codex agent's config.
//!
//! Read with `toml_edit`, which keeps spans and stops at its own nesting limit. TOML forbids a repeated
//! key, so a repeat is a parse error here and the file says `ParseFailed`: nothing can hide behind one.
//!
//! What gives a reason to review is a key that names a program that runs by itself: a compiler wrapper,
//! a linker or runner, a credential helper, a local build backend or build hook, a shell hook, an agent's
//! server or notify command. Where packages come from (source replacement, patches, git dependencies,
//! package indexes, a build backend or requirement that is not well known) is listed as not analysed.
//! `build.rs` and proc-macros are code in another language, and `build.rs` is already flagged unread.

use super::{Cf, Manifest};
use forge_analysis_schema::{ScriptTrigger, UnresolvedReason};
use toml_edit::{ImDocument, Item, Table, TableLike};

/// Build backends that ship with the usual Python build tools. Anything else is reported.
const KNOWN_BACKENDS: &[&str] = &[
    "setuptools.build_meta",
    "setuptools.build_meta:__legacy__",
    "hatchling.build",
    "flit_core.buildapi",
    "flit.buildapi",
    "poetry.core.masonry.api",
    "pdm.backend",
    "pdm.pep517.api",
    "maturin",
    "mesonpy",
    "scikit_build_core.build",
    "sipbuild.api",
    "uv_build",
    "whey",
];

/// Build requirements that are the usual tools (names as pip normalises them). Anything else is reported.
const KNOWN_REQUIRES: &[&str] = &[
    "setuptools",
    "wheel",
    "hatchling",
    "hatch-vcs",
    "hatch-fancy-pypi-readme",
    "flit-core",
    "flit",
    "poetry-core",
    "poetry",
    "pdm-backend",
    "pdm-pep517",
    "maturin",
    "meson-python",
    "meson",
    "ninja",
    "cmake",
    "scikit-build-core",
    "scikit-build",
    "setuptools-scm",
    "setuptools-rust",
    "cython",
    "numpy",
    "pybind11",
    "cffi",
    "versioneer",
    "pip",
    "build",
    "uv-build",
    "packaging",
    "tomli",
];

/// Environment variables that change what a build or a run executes.
fn dangerous_env(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    matches!(
        n.as_str(),
        "RUSTC" | "RUSTC_WRAPPER" | "RUSTC_WORKSPACE_WRAPPER" | "RUSTDOC" | "RUSTFLAGS" | "RUSTDOCFLAGS" | "LD_PRELOAD"
            | "LD_LIBRARY_PATH" | "PATH" | "PYTHONPATH" | "NODE_OPTIONS" | "BROWSER"
    ) || n.starts_with("CARGO_")
        || n.starts_with("DYLD_")
}

fn range(r: Option<std::ops::Range<usize>>, whole: u32) -> (u32, u32) {
    r.map(|r| (r.start as u32, r.end as u32)).unwrap_or((0, whole))
}

/// A string, or a list of strings joined by spaces.
fn words(it: &Item) -> Option<String> {
    if let Some(s) = it.as_str() {
        return Some(s.to_string());
    }
    let a = it.as_array()?;
    let mut parts = Vec::new();
    for v in a.iter() {
        parts.push(v.as_str()?.to_string());
    }
    Some(parts.join(" "))
}

/// The item at a path of keys, through tables and inline tables alike.
fn at<'a>(t: &'a Table, path: &[&str]) -> Option<&'a Item> {
    let mut cur = t.get(path.first()?)?;
    for k in &path[1..] {
        cur = cur.as_table_like()?.get(k)?;
    }
    Some(cur)
}

fn like<'a>(t: &'a Table, path: &[&str]) -> Option<&'a dyn TableLike> {
    at(t, path)?.as_table_like()
}

impl Cf<'_> {
    pub(super) fn toml_file(&mut self, kind: Manifest) {
        let src = self.src;
        let doc = match ImDocument::parse(src) {
            Ok(d) => d,
            Err(_) => {
                // `rust-toolchain` may be a bare channel name, which is not TOML.
                let plain = kind == Manifest::RustToolchain
                    && src.lines().filter(|l| !l.trim().is_empty()).count() == 1
                    && !src.contains('=');
                if !plain {
                    self.unresolved(0, self.len(), UnresolvedReason::ParseFailed, None);
                }
                return;
            }
        };
        let t = doc.as_table();
        match kind {
            Manifest::CargoConfig => self.cargo_config(t),
            Manifest::CargoToml => self.cargo_toml(t),
            Manifest::RustToolchain => self.rust_toolchain(t),
            Manifest::Pyproject => self.pyproject(t),
            Manifest::Mise => self.mise(t),
            Manifest::Codex => self.codex(t),
            _ => {}
        }
    }

    /// A command at `it`: a script fact and a not-followed fact. A value that is not text is flagged.
    fn toml_command(&mut self, name: &str, it: &Item, trigger: ScriptTrigger) {
        let (s, e) = range(it.span(), self.len());
        match words(it) {
            Some(line) => self.command(name, &line, s, e, trigger),
            None => self.unresolved(s, e, UnresolvedReason::UnsupportedSyntax, None),
        }
    }

    /// Something we do not follow, reported with what it says.
    fn toml_info(&mut self, it: &Item, detail: &str) {
        let (s, e) = range(it.span(), self.len());
        self.unresolved(s, e, UnresolvedReason::UnsupportedSyntax, Some(detail));
    }

    // ---- .cargo/config.toml ----

    fn cargo_config(&mut self, t: &Table) {
        use ScriptTrigger::{Manual, OnBuild};
        for key in ["rustc-wrapper", "rustc-workspace-wrapper", "rustc", "rustdoc"] {
            if let Some(it) = at(t, &["build", key]) {
                self.toml_command(&format!("build.{key}"), it, OnBuild);
            }
        }
        for key in ["rustflags", "rustdocflags"] {
            if let Some(it) = at(t, &["build", key]) {
                self.rustflags(&format!("build.{key}"), it);
            }
        }
        if let Some(targets) = like(t, &["target"]) {
            for (triple, tv) in targets.iter() {
                let Some(tt) = tv.as_table_like() else { continue };
                for key in ["linker", "runner"] {
                    if let Some(it) = tt.get(key) {
                        self.toml_command(&format!("target.{triple}.{key}"), it, OnBuild);
                    }
                }
                for key in ["rustflags", "rustdocflags"] {
                    if let Some(it) = tt.get(key) {
                        self.rustflags(&format!("target.{triple}.{key}"), it);
                    }
                }
            }
        }
        // Credential helpers run whenever cargo needs a token. `cargo:` ones are built in.
        let mut providers: Vec<(String, &Item)> = Vec::new();
        for key in ["credential-provider", "global-credential-providers", "credential-process"] {
            if let Some(it) = at(t, &["registry", key]) {
                providers.push((format!("registry.{key}"), it));
            }
        }
        if let Some(regs) = like(t, &["registries"]) {
            for (name, rv) in regs.iter() {
                for key in ["credential-provider", "credential-process"] {
                    if let Some(it) = rv.as_table_like().and_then(|r| r.get(key)) {
                        providers.push((format!("registries.{name}.{key}"), it));
                    }
                }
            }
        }
        for (name, it) in providers {
            let builtin = it.as_str().is_some_and(|s| s.starts_with("cargo:"));
            if !builtin {
                self.toml_command(&name, it, OnBuild);
            }
        }
        if let Some(al) = like(t, &["alias"]) {
            for (name, it) in al.iter() {
                self.toml_command(&format!("alias.{name}"), it, Manual);
            }
        }
        if let Some(env) = like(t, &["env"]) {
            for (name, it) in env.iter() {
                if dangerous_env(name) {
                    let value = it
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| it.as_table_like().and_then(|x| x.get("value")).and_then(|v| v.as_str()).map(str::to_string))
                        .unwrap_or_default();
                    let (s, e) = range(it.span(), self.len());
                    self.script(&format!("env.{name}"), &format!("{name} = {value}"), s, e, OnBuild);
                }
            }
        }
        // Where crates come from.
        if let Some(src) = like(t, &["source"]) {
            for (name, it) in src.iter() {
                let mut what = String::new();
                if let Some(x) = it.as_table_like() {
                    for key in ["replace-with", "registry", "directory", "local-registry", "git"] {
                        if let Some(v) = x.get(key).and_then(|v| v.as_str()) {
                            what.push_str(&format!(" {key}={v}"));
                        }
                    }
                }
                self.toml_info(it, &format!("crate source {name}:{what}"));
            }
        }
        if let Some(regs) = like(t, &["registries"]) {
            for (name, it) in regs.iter() {
                let index = it.as_table_like().and_then(|r| r.get("index")).and_then(|v| v.as_str()).unwrap_or("");
                self.toml_info(it, &format!("registry {name}: {index}"));
            }
        }
        for key in ["proxy", "cainfo"] {
            if let Some(it) = at(t, &["http", key]) {
                self.toml_info(it, &format!("http.{key} = {}", it.as_str().unwrap_or("?")));
            }
        }
        if let Some(it) = t.get("include") {
            self.toml_info(it, "include of another config file");
        }
    }

    /// Flags that set a linker or link arguments run a program the project chose.
    fn rustflags(&mut self, name: &str, it: &Item) {
        let (s, e) = range(it.span(), self.len());
        let Some(text) = words(it) else {
            self.unresolved(s, e, UnresolvedReason::UnsupportedSyntax, None);
            return;
        };
        let low = text.to_ascii_lowercase();
        if low.contains("linker") || low.contains("link-arg") || low.contains("pre-link") || low.contains("-zpre") {
            self.script(name, &text, s, e, ScriptTrigger::OnBuild);
        }
    }

    // ---- Cargo.toml ----

    fn cargo_toml(&mut self, t: &Table) {
        if let Some(it) = at(t, &["package", "build"]) {
            if let Some(path) = it.as_str() {
                self.toml_info(it, &format!("build script {path}"));
            }
        }
        if at(t, &["lib", "proc-macro"]).and_then(|v| v.as_bool()) == Some(true) || at(t, &["lib", "proc_macro"]).and_then(|v| v.as_bool()) == Some(true) {
            if let Some(it) = t.get("lib") {
                self.toml_info(it, "proc-macro crate: runs at compile time");
            }
        }
        for key in ["patch", "replace"] {
            if let Some(it) = t.get(key) {
                self.toml_info(it, &format!("[{key}] changes what dependencies resolve to"));
            }
        }
        for sect in ["dependencies", "dev-dependencies", "build-dependencies"] {
            self.cargo_deps(&format!("{sect}"), like(t, &[sect]));
        }
        if let Some(ws) = like(t, &["workspace"]) {
            self.cargo_deps("workspace.dependencies", ws.get("dependencies").and_then(|d| d.as_table_like()));
        }
        if let Some(targets) = like(t, &["target"]) {
            for (triple, tv) in targets.iter() {
                if let Some(tt) = tv.as_table_like() {
                    for sect in ["dependencies", "dev-dependencies", "build-dependencies"] {
                        self.cargo_deps(&format!("target.{triple}.{sect}"), tt.get(sect).and_then(|d| d.as_table_like()));
                    }
                }
            }
        }
    }

    /// Dependencies from git or an alternative registry. Plain versions and local paths are not listed.
    fn cargo_deps(&mut self, section: &str, deps: Option<&dyn TableLike>) {
        let Some(deps) = deps else { return };
        for (name, it) in deps.iter() {
            let Some(d) = it.as_table_like() else { continue };
            if let Some(g) = d.get("git").and_then(|v| v.as_str()) {
                self.toml_info(it, &format!("{section}: {name} from git {g}"));
            } else if let Some(r) = d.get("registry").and_then(|v| v.as_str()) {
                self.toml_info(it, &format!("{section}: {name} from registry {r}"));
            }
        }
    }

    // ---- rust-toolchain.toml ----

    fn rust_toolchain(&mut self, t: &Table) {
        if let Some(it) = at(t, &["toolchain", "path"]) {
            let (s, e) = range(it.span(), self.len());
            // Every cargo and rustc call goes through the programs in this folder.
            self.script("toolchain.path", it.as_str().unwrap_or("?"), s, e, ScriptTrigger::OnBuild);
        }
    }

    // ---- pyproject.toml ----

    fn pyproject(&mut self, t: &Table) {
        use ScriptTrigger::{Manual, OnBuild};
        if let Some(bp) = at(t, &["build-system", "backend-path"]) {
            // Code in the project that the build imports and runs.
            let (s, e) = range(bp.span(), self.len());
            let paths = words(bp).unwrap_or_default();
            self.script("build-system.backend-path", &paths, s, e, OnBuild);
        }
        if let Some(it) = at(t, &["build-system", "build-backend"]) {
            if let Some(b) = it.as_str() {
                if !KNOWN_BACKENDS.contains(&b) {
                    self.toml_info(it, &format!("build backend {b} is not a well-known one"));
                }
            }
        }
        if let Some(reqs) = at(t, &["build-system", "requires"]).and_then(|r| r.as_array()) {
            for r in reqs.iter() {
                let Some(spec) = r.as_str() else { continue };
                let (s, e) = range(r.span(), self.len());
                let direct = spec.contains(" @ ") || spec.contains("://") || spec.contains("git+");
                let name = spec
                    .split(|c: char| matches!(c, ' ' | '<' | '>' | '=' | '!' | '~' | ';' | '[' | '@' | '('))
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .replace(['_', '.'], "-");
                if direct || !KNOWN_REQUIRES.contains(&name.as_str()) {
                    self.unresolved(s, e, UnresolvedReason::UnsupportedSyntax, Some(&format!("build requirement {spec}")));
                }
            }
        }
        // A custom build hook is a Python file the build runs.
        let mut hooks: Vec<(String, &Item)> = Vec::new();
        if let Some(it) = at(t, &["tool", "hatch", "build", "hooks", "custom"]) {
            hooks.push(("tool.hatch.build.hooks.custom".to_string(), it));
        }
        if let Some(targets) = like(t, &["tool", "hatch", "build", "targets"]) {
            for (name, tv) in targets.iter() {
                if let Some(it) = tv.as_table_like().and_then(|x| x.get("hooks")).and_then(|h| h.as_table_like()).and_then(|h| h.get("custom")) {
                    hooks.push((format!("tool.hatch.build.targets.{name}.hooks.custom"), it));
                }
            }
        }
        for (name, it) in hooks {
            let path = it.as_table_like().and_then(|x| x.get("path")).and_then(|v| v.as_str()).unwrap_or("hatch_build.py");
            let (s, e) = range(it.span(), self.len());
            self.script(&name, path, s, e, OnBuild);
        }
        // `pdm` runs `pre_*` and `post_*` scripts around its own commands.
        if let Some(sc) = like(t, &["tool", "pdm", "scripts"]) {
            for (name, it) in sc.iter() {
                let lifecycle = ["pre_", "post_"].iter().any(|p| {
                    name.strip_prefix(p).is_some_and(|r| matches!(r, "install" | "lock" | "build" | "publish" | "run" | "script"))
                });
                let cmd = it
                    .as_table_like()
                    .and_then(|x| x.get("cmd").or_else(|| x.get("shell")).or_else(|| x.get("call")))
                    .unwrap_or(it);
                self.toml_command(&format!("tool.pdm.scripts.{name}"), cmd, if lifecycle { OnBuild } else { Manual });
            }
        }
        // Where packages come from.
        if let Some(srcs) = at(t, &["tool", "poetry", "source"]) {
            match srcs.as_array_of_tables() {
                Some(a) => {
                    for tb in a.iter() {
                        let name = tb.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let url = tb.get("url").and_then(|v| v.as_str()).unwrap_or("");
                        let (s, e) = range(tb.span(), self.len());
                        self.unresolved(s, e, UnresolvedReason::UnsupportedSyntax, Some(&format!("package source {name}: {url}")));
                    }
                }
                None => self.toml_info(srcs, "package sources for poetry"),
            }
        }
        for key in ["index-url", "extra-index-url", "find-links"] {
            if let Some(it) = at(t, &["tool", "uv", key]) {
                self.toml_info(it, &format!("uv {key}: {}", words(it).unwrap_or_default()));
            }
        }
        if let Some(idx) = at(t, &["tool", "uv", "index"]).and_then(|i| i.as_array_of_tables()) {
            for tb in idx.iter() {
                let url = tb.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let (s, e) = range(tb.span(), self.len());
                self.unresolved(s, e, UnresolvedReason::UnsupportedSyntax, Some(&format!("uv package index {url}")));
            }
        }
        if let Some(it) = at(t, &["tool", "uv", "sources"]) {
            self.toml_info(it, "uv sources redirect where named packages come from");
        }
        // pytest loads a plugin named with `-p`, and runs its code.
        if let Some(it) = at(t, &["tool", "pytest", "ini_options", "addopts"]) {
            let text = words(it).unwrap_or_default();
            if text.split_whitespace().any(|w| w == "-p" || w.starts_with("-p") && w.len() > 2 || w == "--pyargs") {
                self.toml_info(it, &format!("pytest addopts: {text}"));
            }
        }
    }

    // ---- mise.toml ----

    fn mise(&mut self, t: &Table) {
        use ScriptTrigger::{Manual, OnBuild, OnOpen};
        if let Some(hooks) = like(t, &["hooks"]) {
            for (name, it) in hooks.iter() {
                let trigger = if matches!(name, "preinstall" | "postinstall") { OnBuild } else { OnOpen };
                let cmd = it.as_table_like().and_then(|x| x.get("script")).unwrap_or(it);
                self.toml_command(&format!("hooks.{name}"), cmd, trigger);
            }
        }
        if let Some(env) = like(t, &["env"]) {
            for (name, it) in env.iter() {
                if name == "_" {
                    // `_.source` runs a shell script every time the folder is entered.
                    if let Some(src) = it.as_table_like().and_then(|x| x.get("source")) {
                        self.toml_command("env._.source", src, OnOpen);
                    }
                } else if let Some(v) = it.as_str() {
                    // A template that runs a command to compute a value.
                    if v.contains("exec(") {
                        let (s, e) = range(it.span(), self.len());
                        self.command(&format!("env.{name}"), v, s, e, OnOpen);
                    }
                }
            }
        }
        if let Some(tasks) = like(t, &["tasks"]) {
            for (name, it) in tasks.iter() {
                let cmd = it.as_table_like().and_then(|x| x.get("run")).unwrap_or(it);
                self.toml_command(&format!("tasks.{name}"), cmd, Manual);
            }
        }
        if let Some(pl) = like(t, &["plugins"]) {
            for (name, it) in pl.iter() {
                let (s, e) = range(it.span(), self.len());
                // A plugin is code fetched and run when tools are installed.
                self.script(&format!("plugins.{name}"), it.as_str().unwrap_or("?"), s, e, OnBuild);
            }
        }
        if let Some(tools) = like(t, &["tools"]) {
            for (name, it) in tools.iter() {
                if let Some(post) = it.as_table_like().and_then(|x| x.get("postinstall")) {
                    self.toml_command(&format!("tools.{name}.postinstall"), post, OnBuild);
                }
            }
        }
    }

    // ---- .codex/config.toml ----

    fn codex(&mut self, t: &Table) {
        use ScriptTrigger::OnOpen;
        if let Some(servers) = like(t, &["mcp_servers"]) {
            for (name, it) in servers.iter() {
                let Some(sv) = it.as_table_like() else {
                    self.toml_info(it, "MCP server that is not a table");
                    continue;
                };
                if let Some(cmd) = sv.get("command").and_then(|c| c.as_str()) {
                    let args = sv.get("args").and_then(words).unwrap_or_default();
                    let line = if args.is_empty() { cmd.to_string() } else { format!("{cmd} {args}") };
                    let (s, e) = range(it.span(), self.len());
                    self.command(&format!("mcp_servers.{name}"), &line, s, e, OnOpen);
                } else if let Some(url) = sv.get("url").and_then(|u| u.as_str()) {
                    self.toml_info(it, &format!("remote MCP server {url}"));
                } else {
                    self.toml_info(it, "MCP server with no command");
                }
            }
        }
        // Run after each agent turn.
        if let Some(n) = t.get("notify") {
            self.toml_command("notify", n, OnOpen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{analyze_config, is_config_manifest};
    use forge_analysis_schema::{RelPath, ScriptTrigger, StaticFact, UnresolvedReason};

    fn run(path: &str, src: &str) -> Vec<StaticFact> {
        analyze_config(&RelPath::new(path).unwrap(), src).unwrap().facts
    }

    fn scripts(f: &[StaticFact]) -> Vec<(String, String, ScriptTrigger)> {
        f.iter()
            .filter_map(|x| {
                let s = x.script.as_ref()?;
                Some((s.name.text().to_string(), x.detail().map(|d| d.text().to_string()).unwrap_or_default(), s.trigger))
            })
            .collect()
    }

    fn infos(f: &[StaticFact]) -> Vec<String> {
        f.iter()
            .filter(|x| x.unresolved == Some(UnresolvedReason::UnsupportedSyntax))
            .filter_map(|x| x.detail().map(|d| d.text().to_string()))
            .collect()
    }

    fn reasons(f: &[StaticFact]) -> Vec<UnresolvedReason> {
        f.iter().filter_map(|x| x.unresolved).collect()
    }

    fn has(f: &[StaticFact], name: &str, t: ScriptTrigger) -> bool {
        scripts(f).iter().any(|(n, _, x)| n == name && *x == t)
    }

    #[test]
    fn the_toml_files_are_recognised() {
        for p in [
            ".cargo/config.toml",
            "a/.cargo/config",
            "Cargo.toml",
            "crates/x/Cargo.toml",
            "rust-toolchain",
            "rust-toolchain.toml",
            "pyproject.toml",
            "mise.toml",
            ".mise.toml",
            ".codex/config.toml",
            ".gemini/settings.json",
        ] {
            assert!(is_config_manifest(p), "{p}");
        }
        for p in ["config.toml", "cargo.lock", "pyproject.toml.bak", "codex/config.toml", "tools.toml"] {
            assert!(!is_config_manifest(p), "{p}");
        }
    }

    #[test]
    fn cargo_config_programs_that_run_on_every_build_are_found() {
        let f = run(
            ".cargo/config.toml",
            r#"
[build]
rustc-wrapper = "tools/wrap.sh"
rustflags = ["-C", "target-cpu=native"]
[target.x86_64-unknown-linux-gnu]
linker = "tools/ld.sh"
runner = ["tools/run.sh", "--x"]
rustflags = ["-C", "link-arg=-fuse-ld=evil"]
[registry]
credential-provider = ["cargo:token", "tools/cred"]
[registries.corp]
index = "https://corp.example/index"
credential-process = "tools/cred2"
[alias]
b = "build --release"
[env]
RUSTC_WRAPPER = "x.sh"
HARMLESS = "1"
[source.crates-io]
replace-with = "mirror"
[source.mirror]
registry = "https://mirror.example/index"
[http]
proxy = "http://p.example:8080"
"#,
        );
        assert!(has(&f, "build.rustc-wrapper", ScriptTrigger::OnBuild));
        assert!(has(&f, "target.x86_64-unknown-linux-gnu.linker", ScriptTrigger::OnBuild));
        let s = scripts(&f);
        assert!(s.iter().any(|(n, c, _)| n.ends_with(".runner") && c == "tools/run.sh --x"));
        // Only flags that set a linker are a program to run; target-cpu is not.
        assert!(has(&f, "target.x86_64-unknown-linux-gnu.rustflags", ScriptTrigger::OnBuild));
        assert!(!s.iter().any(|(n, _, _)| n == "build.rustflags"));
        // A built-in credential provider is not a program, a path is. So is the legacy process key.
        assert!(has(&f, "registry.credential-provider", ScriptTrigger::OnBuild));
        assert!(has(&f, "registries.corp.credential-process", ScriptTrigger::OnBuild));
        assert!(has(&f, "alias.b", ScriptTrigger::Manual));
        assert!(has(&f, "env.RUSTC_WRAPPER", ScriptTrigger::OnBuild));
        assert!(!s.iter().any(|(n, _, _)| n == "env.HARMLESS"));
        let i = infos(&f);
        assert!(i.iter().any(|d| d.starts_with("crate source crates-io: replace-with=mirror")), "{i:?}");
        assert!(i.iter().any(|d| d.contains("mirror") && d.contains("mirror.example")), "{i:?}");
        assert!(i.iter().any(|d| d == "registry corp: https://corp.example/index"), "{i:?}");
        assert!(i.iter().any(|d| d.starts_with("http.proxy = ")), "{i:?}");
    }

    #[test]
    fn a_plain_cargo_config_holds_nothing_automatic() {
        assert!(run(".cargo/config.toml", "[build]\ntarget-dir = \"t\"\n[term]\ncolor = \"always\"\n").is_empty());
    }

    #[test]
    fn cargo_toml_lists_where_dependencies_come_from_but_a_build_script_is_not_a_command() {
        let f = run(
            "Cargo.toml",
            r#"
[package]
name = "x"
build = "build.rs"
[lib]
proc-macro = true
[dependencies]
serde = "1"
local = { path = "../local" }
fromgit = { git = "https://x.example/r.git" }
other = { version = "1", registry = "corp" }
[build-dependencies]
cc = "1"
[patch.crates-io]
serde = { git = "https://x.example/serde" }
"#,
        );
        assert!(scripts(&f).is_empty());
        let i = infos(&f);
        assert!(i.contains(&"build script build.rs".to_string()), "{i:?}");
        assert!(i.iter().any(|d| d.starts_with("proc-macro crate")), "{i:?}");
        assert!(i.iter().any(|d| d == "dependencies: fromgit from git https://x.example/r.git"), "{i:?}");
        assert!(i.iter().any(|d| d == "dependencies: other from registry corp"), "{i:?}");
        assert!(i.iter().any(|d| d.starts_with("[patch]")), "{i:?}");
        // A plain version and a local path are not listed.
        assert!(!i.iter().any(|d| d.contains("serde from") || d.contains("local")));
        assert!(run("Cargo.toml", "[package]\nname = \"x\"\n[dependencies]\nserde = \"1\"\n").is_empty());
    }

    #[test]
    fn a_toolchain_path_runs_a_chosen_compiler_and_a_bare_channel_name_runs_nothing() {
        let f = run("rust-toolchain.toml", "[toolchain]\nchannel = \"nightly\"\npath = \"tools/rust\"\n");
        assert!(has(&f, "toolchain.path", ScriptTrigger::OnBuild));
        assert!(run("rust-toolchain.toml", "[toolchain]\nchannel = \"stable\"\ncomponents = [\"clippy\"]\n").is_empty());
        assert!(run("rust-toolchain", "1.100.0\n").is_empty());
        assert_eq!(reasons(&run("rust-toolchain", "a = = b\nc\n")), vec![UnresolvedReason::ParseFailed]);
    }

    #[test]
    fn pyproject_reports_local_backends_hooks_and_what_is_not_well_known() {
        let f = run(
            "pyproject.toml",
            r#"
[build-system]
requires = ["setuptools>=61", "wheel", "Evil_Pkg", "tool @ https://x.example/t.whl", "Hatch.VCS ; python_version > '3'"]
build-backend = "evil_backend"
backend-path = ["tools/backend"]
[tool.hatch.build.hooks.custom]
path = "hooks/build.py"
[tool.pdm.scripts]
post_install = "python setup_extra.py"
lint = { cmd = "ruff check" }
[[tool.poetry.source]]
name = "corp"
url = "https://corp.example/simple"
[tool.uv]
extra-index-url = ["https://corp.example/simple"]
[tool.pytest.ini_options]
addopts = "-q -p evilplugin"
"#,
        );
        assert!(has(&f, "build-system.backend-path", ScriptTrigger::OnBuild));
        assert!(scripts(&f).iter().any(|(n, c, _)| n == "tool.hatch.build.hooks.custom" && c == "hooks/build.py"));
        assert!(has(&f, "tool.pdm.scripts.post_install", ScriptTrigger::OnBuild));
        assert!(has(&f, "tool.pdm.scripts.lint", ScriptTrigger::Manual));
        let i = infos(&f);
        assert!(i.contains(&"build backend evil_backend is not a well-known one".to_string()), "{i:?}");
        assert!(i.iter().any(|d| d == "build requirement Evil_Pkg"), "{i:?}");
        assert!(i.iter().any(|d| d.starts_with("build requirement tool @ https://")), "{i:?}");
        // The usual tools, with version bounds, extras, markers and odd spelling, are not reported.
        assert!(!i.iter().any(|d| d.contains("setuptools") || d.contains("wheel") || d.contains("Hatch")), "{i:?}");
        assert!(i.iter().any(|d| d == "package source corp: https://corp.example/simple"), "{i:?}");
        assert!(i.iter().any(|d| d.starts_with("uv extra-index-url")), "{i:?}");
        assert!(i.iter().any(|d| d.starts_with("pytest addopts")), "{i:?}");
    }

    #[test]
    fn an_ordinary_pyproject_holds_nothing_to_report() {
        let src = "[build-system]\nrequires = [\"setuptools>=61\", \"wheel\"]\nbuild-backend = \"setuptools.build_meta\"\n[project]\nname = \"x\"\n[tool.pytest.ini_options]\naddopts = \"-q\"\n";
        assert!(run("pyproject.toml", src).is_empty());
    }

    #[test]
    fn mise_hooks_env_scripts_tasks_and_plugins() {
        let f = run(
            "mise.toml",
            r#"
[hooks]
enter = "echo entered"
postinstall = ["sh", "post.sh"]
[env]
_.source = "./env.sh"
TOKEN = "{{ exec(command='cat ~/.secret') }}"
PLAIN = "1"
[tasks]
build = "make"
test = { run = ["cargo", "test"] }
[plugins]
evil = "https://x.example/plugin.git"
[tools]
node = { version = "20", postinstall = "npm i -g x" }
python = "3.12"
"#,
        );
        assert!(has(&f, "hooks.enter", ScriptTrigger::OnOpen));
        assert!(has(&f, "hooks.postinstall", ScriptTrigger::OnBuild));
        assert!(has(&f, "env._.source", ScriptTrigger::OnOpen));
        assert!(has(&f, "env.TOKEN", ScriptTrigger::OnOpen));
        assert!(!scripts(&f).iter().any(|(n, _, _)| n == "env.PLAIN"));
        assert!(has(&f, "tasks.build", ScriptTrigger::Manual));
        assert!(scripts(&f).iter().any(|(n, c, t)| n == "tasks.test" && c == "cargo test" && *t == ScriptTrigger::Manual));
        assert!(has(&f, "plugins.evil", ScriptTrigger::OnBuild));
        assert!(has(&f, "tools.node.postinstall", ScriptTrigger::OnBuild));
    }

    #[test]
    fn codex_servers_and_notify_commands_run_on_open() {
        let f = run(
            ".codex/config.toml",
            "notify = [\"sh\", \"n.sh\"]\n[mcp_servers.a]\ncommand = \"npx\"\nargs = [\"-y\", \"p\"]\n[mcp_servers.b]\nurl = \"https://x.example/m\"\n[mcp_servers.c]\n",
        );
        let s = scripts(&f);
        assert!(s.iter().any(|(n, c, t)| n == "mcp_servers.a" && c == "npx -y p" && *t == ScriptTrigger::OnOpen));
        assert!(s.iter().any(|(n, c, t)| n == "notify" && c == "sh n.sh" && *t == ScriptTrigger::OnOpen));
        let i = infos(&f);
        assert!(i.contains(&"remote MCP server https://x.example/m".to_string()), "{i:?}");
        assert!(i.contains(&"MCP server with no command".to_string()), "{i:?}");
    }

    #[test]
    fn the_gemini_settings_file_is_read_like_the_other_agent_files() {
        let f = run(".gemini/settings.json", r#"{ // c
 "mcpServers": { "a": { "command": "npx", "args": ["x"] } } }"#);
        assert!(has(&f, "a", ScriptTrigger::OnOpen));
    }

    #[test]
    fn toml_that_does_not_parse_says_so_and_a_repeated_key_cannot_hide() {
        for (p, s) in [
            (".cargo/config.toml", "[build\nrustc-wrapper = \"x\""),
            ("pyproject.toml", "a = 1\na = 2\n"),
            ("Cargo.toml", "[package]\nname = \"x\"\n[package]\nname = \"y\"\n"),
            ("mise.toml", "= x"),
            (".codex/config.toml", ""),
        ] {
            let f = run(p, s);
            if s.is_empty() {
                // An empty file is valid TOML and holds nothing.
                assert!(f.is_empty());
            } else {
                assert_eq!(reasons(&f), vec![UnresolvedReason::ParseFailed], "{p}");
            }
        }
    }

    #[test]
    fn hostile_shapes_do_not_panic_and_are_not_dropped() {
        for (p, s) in [
            (".cargo/config.toml", "build = 5\n"),
            (".cargo/config.toml", "[build]\nrustc-wrapper = 5\n"),
            (".cargo/config.toml", "[build]\nrustc-wrapper = [1, 2]\n"),
            (".cargo/config.toml", "target = { a = 1 }\n"),
            ("mise.toml", "hooks = 5\n[tasks]\na = 5\n"),
            ("mise.toml", "[hooks]\nenter = 5\n"),
            (".codex/config.toml", "mcp_servers = { a = 5 }\n"),
            (".codex/config.toml", "notify = 5\n"),
            ("pyproject.toml", "[tool.pdm.scripts]\na = 5\n"),
        ] {
            let _ = run(p, s); // must not panic
        }
        assert!(!run(".cargo/config.toml", "[build]\nrustc-wrapper = 5\n").is_empty());
        assert!(!run("mise.toml", "[hooks]\nenter = 5\n").is_empty());
        assert!(!run(".codex/config.toml", "notify = 5\n").is_empty());
    }
}
