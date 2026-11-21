//! Files that tools read and act on without anyone asking.
//!
//! This is the one place that says what "runs by itself" means for each ecosystem. It is a fixed
//! table, not a heuristic: a file is on it or it is not. Extend it here and nowhere else.
//!
//! Two lists:
//! - `is_auto_run_config`: files we cannot parse yet. They are listed in the report as
//!   `AutoRunConfig` and make the brief incomplete, so a hidden hook cannot pass unseen.
//! - `is_auto_run_code`: JavaScript and TypeScript files that tools load and run (build, lint and
//!   test configuration). A quick scan reads these as code, so what they do is reported as facts.
//!
//! The lists come from the documentation of each tool and from known abuse of editors, package
//! managers and build systems. They are **not yet checked against Forge's canary test**. They only
//! say that a file *can* act by itself, never that it does: most of these files are harmless.
//! Names match without regard to case, as they do on Windows and macOS.
//!
//! Known gaps: `.git/hooks` and `.git/config` are listed below but intake skips `.git`, so they are
//! never seen. Config *inside* a file (a `plugins` entry in `tsconfig.json`, a `[scripts]` table in
//! a manifest) cannot be told apart from harmless content until those files are parsed.

#[derive(Clone, Copy)]
enum Pat {
    /// The file name is exactly this.
    Name(&'static str),
    /// The file name ends in `.` and this.
    Ext(&'static str),
    /// The file name starts with this.
    Prefix(&'static str),
    /// The file is anywhere below a folder (or a run of folders) with this path.
    Dir(&'static str),
    /// The path is, or ends in `/` and, this. For a folder and a file name together.
    Suffix(&'static str),
}

use Pat::*;

const CONFIG: &[Pat] = &[
    // ---- .NET: MSBuild imports these into every project below them; NuGet and the SDK read the rest.
    Name("directory.build.props"),
    Name("directory.build.targets"),
    Name("directory.build.rsp"),
    Name("directory.solution.props"),
    Name("directory.solution.targets"),
    Name("directory.packages.props"),
    Name("global.json"),
    Name("nuget.config"),
    Suffix(".config/dotnet-tools.json"),
    Ext("csproj"),
    Ext("fsproj"),
    Ext("vbproj"),
    Ext("sln"),
    Ext("slnx"),
    // ---- Editors and AI coding agents: acted on when a folder is opened or a session starts.
    Suffix(".vscode/tasks.json"), // tasks can run when the folder opens
    Suffix(".vscode/settings.json"),
    Suffix(".vscode/launch.json"),
    Suffix(".vscode/extensions.json"),
    Suffix(".vscode/mcp.json"), // MCP servers are started as commands
    Name(".mcp.json"),
    Suffix(".cursor/mcp.json"),
    Suffix(".gemini/settings.json"),
    Suffix(".codex/config.toml"),
    Suffix(".claude/settings.json"), // hooks run commands
    Suffix(".claude/settings.local.json"),
    Dir(".claude/hooks"),
    Dir(".devcontainer"), // `initializeCommand` runs on the host
    Name(".devcontainer.json"),
    Name(".gitpod.yml"),
    Name(".replit"),
    // ---- Shells and environment managers: loaded when someone enters the folder.
    Name(".envrc"), // direnv
    Name(".env"),
    Prefix(".env."), // loaded by many tools; `NODE_OPTIONS` can preload code
    Name("mise.toml"),
    Name(".mise.toml"),
    Name("flake.nix"),
    Name("shell.nix"),
    // ---- Git hooks and the tools that install them: run on commit, push and checkout.
    Dir(".husky"),
    Dir(".githooks"),
    Dir(".git-hooks"),
    Dir(".git/hooks"),
    Suffix(".git/config"),
    Name("lefthook.yml"),
    Name("lefthook.yaml"),
    Name(".lefthook.yml"),
    Name(".pre-commit-config.yaml"),
    Name(".huskyrc"),
    Name(".lintstagedrc"),
    Name(".lintstagedrc.json"),
    Name(".lintstagedrc.yml"),
    Name(".lintstagedrc.yaml"),
    // ---- Node and JavaScript package managers.
    Dir(".yarn"), // `yarnPath` and plugins are JavaScript the package manager runs
    Name("bunfig.toml"), // can preload scripts
    // ---- Python: run when a package is built or installed, when tests start, or when the
    // interpreter starts.
    Name("setup.py"),
    Name("setup.cfg"),
    Name("pyproject.toml"), // the build backend, which can be a local path
    Name("conftest.py"), // imported by pytest on its own
    Name("sitecustomize.py"),
    Name("usercustomize.py"),
    Ext("pth"), // lines starting with `import` are executed at interpreter start
    Name("tox.ini"),
    Name("noxfile.py"),
    Name("pipfile"),
    Name("poetry.toml"),
    Name("pip.conf"),
    Name("pip.ini"),
    Name(".pypirc"),
    Name("pyvenv.cfg"), // names the interpreter a project-carried environment uses
    Suffix("bin/python"),
    Suffix("bin/python3"),
    Suffix("scripts/python.exe"),
    Suffix("scripts/pythonw.exe"),
    // ---- Rust: `build.rs` and proc macros run at build time; cargo config can set a rustc wrapper.
    Name("build.rs"),
    Dir(".cargo"),
    Name("cargo.toml"),
    Name("rust-toolchain"),
    Name("rust-toolchain.toml"),
    // ---- Ruby.
    Name("gemfile"),
    Ext("gemspec"),
    Name("extconf.rb"), // native extension build, run at gem install
    Name("rakefile"),
    Name("podfile"),
    Ext("podspec"),
    Name("vagrantfile"),
    Suffix(".bundle/config"),
    // ---- PHP.
    Name("composer.json"), // `scripts` and plugins
    // ---- Java and Kotlin: build scripts are programs.
    Name("pom.xml"),
    Dir(".mvn"),
    Name("mvnw"),
    Name("mvnw.cmd"),
    Name("build.gradle"),
    Name("build.gradle.kts"),
    Name("settings.gradle"),
    Name("settings.gradle.kts"),
    Name("init.gradle"),
    Ext("gradle"),
    Name("gradle.properties"),
    Name("gradlew"),
    Name("gradlew.bat"),
    Name("gradle-wrapper.properties"),
    Name("gradle-wrapper.jar"), // a program the wrapper script runs
    Name("maven-wrapper.jar"),
    Dir("buildsrc"),
    Name("build.sbt"),
    Ext("sbt"),
    Name("build.xml"),
    // ---- Apple and Swift.
    Name("package.swift"),
    Name("project.pbxproj"), // Run Script build phases
    Ext("xcscheme"), // pre- and post-actions
    // ---- C and C++, and the build systems around them.
    Name("makefile"),
    Name("gnumakefile"),
    Name("makefile.am"),
    Name("makefile.in"),
    Name("justfile"),
    Name("taskfile.yml"),
    Name("taskfile.yaml"),
    Name("cmakelists.txt"),
    Ext("cmake"),
    Name("meson.build"),
    Name("configure"),
    Name("configure.ac"),
    Name(".bazelrc"),
    Name("build.bazel"),
    Name("workspace"),
    Name("module.bazel"),
    // ---- Windows: literal autorun and shortcut files.
    Name("autorun.inf"),
    Ext("lnk"),
];

/// Names that tools load as JavaScript or TypeScript. The file name starts with one of these and
/// ends in a JavaScript or TypeScript extension, so `webpack.config.prod.js` and `vite.config.ts`
/// both match.
const CODE_PREFIXES: &[&str] = &[
    ".pnpmfile",
    ".babelrc",
    "babel.config",
    ".eslintrc",
    "eslint.config",
    ".prettierrc",
    "prettier.config",
    ".stylelintrc",
    "stylelint.config",
    "webpack.config",
    "rspack.config",
    "rollup.config",
    "vite.config",
    "vitest.config",
    "jest.config",
    ".mocharc",
    "karma.conf",
    "playwright.config",
    "cypress.config",
    "postcss.config",
    "tailwind.config",
    "next.config",
    "nuxt.config",
    "svelte.config",
    "astro.config",
    "metro.config",
    "craco.config",
    "tsup.config",
    "esbuild.config",
    "gulpfile",
    "gruntfile",
    "lint-staged.config",
    ".lintstagedrc",
    "commitlint.config",
    "release.config",
    ".releaserc",
    ".huskyrc",
    ".sequelizerc",
    "knexfile",
    "ecosystem.config",
];

const CODE_EXTENSIONS: &[&str] = &["js", "cjs", "mjs", "ts", "cts", "mts"];

fn normalise(path: &str) -> String {
    path.to_ascii_lowercase().replace('\\', "/")
}

fn matches(p: &Pat, path: &str, name: &str, ext: Option<&str>) -> bool {
    match *p {
        Name(n) => name == n,
        Ext(e) => ext == Some(e),
        Prefix(x) => name.starts_with(x),
        Dir(d) => format!("/{path}").contains(&format!("/{d}/")),
        Suffix(s) => path == s || path.ends_with(&format!("/{s}")),
    }
}

fn parts(path: &str) -> (&str, Option<&str>) {
    let name = path.rsplit('/').next().unwrap_or("");
    (name, name.rsplit_once('.').map(|(_, e)| e))
}

/// Can a tool read this file and act on it by itself, in a way we cannot yet read?
pub fn is_auto_run_config(path: &str) -> bool {
    let path = normalise(path);
    let (name, ext) = parts(&path);
    CONFIG.iter().any(|p| matches(p, &path, name, ext))
}

/// Is this a JavaScript or TypeScript file that a tool loads and runs (build, lint or test
/// configuration)? A quick scan reads these as code.
pub fn is_auto_run_code(path: &str) -> bool {
    let path = normalise(path);
    let (name, ext) = parts(&path);
    ext.is_some_and(|e| CODE_EXTENSIONS.contains(&e)) && CODE_PREFIXES.iter().any(|p| name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path that must match the pattern, built from the pattern itself.
    fn sample(p: &Pat) -> String {
        match *p {
            Name(n) => format!("x/y/{n}"),
            Ext(e) => format!("x/thing.{e}"),
            Prefix(x) => format!("x/{x}rest"),
            Dir(d) => format!("x/{d}/inner/file"),
            Suffix(s) => format!("x/{s}"),
        }
    }

    #[test]
    fn every_entry_matches_a_path_built_from_it_in_any_case_and_with_either_slash() {
        for p in CONFIG {
            let s = sample(p);
            assert!(is_auto_run_config(&s), "{s}");
            assert!(is_auto_run_config(&s.to_ascii_uppercase()), "{s}");
            assert!(is_auto_run_config(&s.replace('/', "\\")), "{s}");
        }
    }

    #[test]
    fn common_files_that_do_not_run_by_themselves_are_not_on_the_list() {
        for p in [
            "README.md",
            "src/index.js",
            "src/main.rs",
            "Program.cs",
            "package-lock.json",
            "tsconfig.json",
            "src/makefile.txt",
            "docs/setup.py.md",
            "environment.ts",
            "src/.environment",
            "notes/.vscode-notes.md",
            "vscode/tasks.json",
            "src/claude/settings.json",
            "mycargo.toml",
            "a/b/gemfile.lock",
            "mvnw.txt",
        ] {
            assert!(!is_auto_run_config(p), "{p}");
        }
    }

    #[test]
    fn folders_match_whole_names_only() {
        assert!(is_auto_run_config(".husky/pre-commit"));
        assert!(is_auto_run_config("a/.husky/_/husky.sh"));
        assert!(!is_auto_run_config("a/not.husky/pre-commit"));
        assert!(!is_auto_run_config("a/.huskyx/pre-commit"));
        assert!(is_auto_run_config("pkg/.cargo/config.toml"));
        assert!(is_auto_run_config(".claude/hooks/guard.sh"));
        assert!(!is_auto_run_config(".claude/commands/review.md"));
        assert!(is_auto_run_config("venv/bin/python"));
        assert!(!is_auto_run_config("venv/bin/pythonista"));
    }

    #[test]
    fn tool_configuration_written_as_code_is_found_by_name_and_extension() {
        for p in [
            "webpack.config.js",
            "a/vite.config.ts",
            ".eslintrc.cjs",
            "babel.config.mjs",
            ".pnpmfile.cjs",
            "webpack.config.prod.js",
            "gulpfile.js",
            "Jest.Config.TS",
        ] {
            assert!(is_auto_run_code(p), "{p}");
        }
        for p in [".eslintrc", ".eslintrc.json", "webpack.config.json", "src/webpack.js", "index.js", "vite.config.md"] {
            assert!(!is_auto_run_code(p), "{p}");
        }
    }

    #[test]
    fn the_two_lists_do_not_overlap() {
        for p in CODE_PREFIXES {
            for e in CODE_EXTENSIONS {
                let path = format!("{p}.{e}");
                assert!(!is_auto_run_config(&path), "{path} is on both lists");
            }
        }
    }
}
