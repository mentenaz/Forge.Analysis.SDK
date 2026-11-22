# Per-ecosystem lists: build history and detail

Moved out of `CLAUDE.md` on 22 Nov 2026 so that file stays short. This is the record of what slices A and B read and
why. The code is the source of truth: `crates/forge-analysis-report/src/autorun.rs` (the `CONFIG` and `CODE_PREFIXES`
tables), `crates/forge-analysis-core/src/imports/config.rs`, `imports/config/tomlcfg.rs` and `imports/xml.rs`.

## Slice A (list and flag, no parsing)

The list is broad on purpose: everything a tool, editor, AI agent, package manager or build system might act on by
itself (.NET, editors and agent hooks such as `.vscode/tasks.json`, `.claude/settings.json`, `.mcp.json`, git hooks,
Python, Rust, Ruby, PHP, JVM, Apple, C/C++ build files, `.env`, autorun/shortcut files). The one place to extend it is
`crates/forge-analysis-report/src/autorun.rs`: `CONFIG` (flagged, cannot parse yet: `NotAnalysedKind::AutoRunConfig`)
and `CODE_PREFIXES` (JS/TS tool config such as `webpack.config.js`, `.eslintrc.cjs`: **a quick scan reads these as
code**). Widening the table does not change the report shape, so it does not bump `REPORT_SCHEMA`. The brief reason
is `AutoRunNotAnalysed`; an unread auto-run file makes the brief `incomplete` in either scope (`review` still wins).
The list comes from tool documentation and known abuse; the entries say a file *can* act, not that it does. Project
files (`.csproj` etc.) and several others match by extension or folder, so "fixed list, not a pattern" is relaxed for
them.

Intake makes one exception to "ignored folders are not entered": inside a `.git` folder (any depth) it reads `config`
and the active files in `hooks` (not `*.sample`), because git runs them by itself. `.git` is still recorded once as
ignored, so the rest of it stays counted; links are never followed; the in-memory path makes the same exception.
Archives (`.zip`, `.tgz`, `.jar`, `.whl`, `.vsix` and more) get `NotAnalysedKind::Archive` and the brief reason
`ArchiveNotOpened` (brief incomplete); they are **listed, not opened**.

Known gaps: archives are not opened (needs size, zip-bomb and path-traversal limits); content inside a file (a `plugins`
entry in `tsconfig.json`, a key that config.rs does not list) is not read; `.git/modules` and `.git/info` are not read.

## Slice B, pieces 1 and 2 (`imports/config.rs`)

These files are *read*, not only listed: `.git/config`, `.vscode/tasks.json`, `.claude/settings.json` (+ `.local`),
`.mcp.json`, `.cursor/mcp.json`, `.vscode/mcp.json`, `devcontainer.json`, `composer.json`, `global.json`
(`msbuild-sdks`). Each command becomes a `LifecycleScript` fact with a trigger (`on_open`, `on_git`, `on_build`, or
`install`/`hook`/`manual`) plus an unresolved `ShellCommand` fact on the same text (quoted, not followed). Only commands
that hang on an automatic trigger are a brief reason (`AutomaticCommand`, gives `review`); a task that someone must
start is listed as `manual`. A read file with nothing automatic leaves the unread list; a file that does not parse gets
`ParseFailed` (brief incomplete). Repeated keys are flagged (`AmbiguousBinding`). Editor files that allow comments use
`json::parse_lenient`; the rest stay strict. Intake keeps the text of `.git/config` (`Options::source_path_endings`).
Files the reader handles must also be on the `CONFIG` table (a test checks it). Command text is quoted in the report,
so a secret written inside a command (a credential helper) would show.

## Piece 3: MSBuild XML and `nuget.config`

Our own strict XML reader (`imports/xml.rs`: spans, 4 MiB / 64-deep limits; a DOCTYPE, unknown entity or repeated
attribute is refused; nothing expands). It reads `Directory.Build/Solution/Packages.*`,
`*.csproj/.fsproj/.vbproj/.vcxproj/.proj` and any `*.props`/`*.targets`: targets that run by themselves
(BeforeTargets/AfterTargets, InitialTargets/DefaultTargets, plus what they DependsOn or CallTarget), `Exec` (a command
fact), `DownloadFile`, tasks we cannot see into (`Unzip`, the `MSBuild` task, tasks from a `UsingTask`), inline task
code, Pre/PostBuildEvent, analyzers, SDKs from NuGet (Microsoft's are not reported), tool-path redirects
(`CscToolPath`...), restore-source properties and non-standard `Import`s. Plain file tasks (Copy, Delete,
WriteLinesToFile) and `PackageReference`s are NOT reported. `nuget.config`: any source other than nuget.org,
`repositoryPath`/`globalPackagesFolder`, and the presence (never the values) of credentials. No schema change.

## Piece 4: TOML (`imports/config/tomlcfg.rs`)

Read with `toml_edit`, pinned `=0.22.27`, no default features; core's only new dependency; `cargo deny` still passes;
TOML forbids repeated keys so they are a `ParseFailed`. Reads:

- `.cargo/config.toml`: rustc wrappers, linker/runner, credential helpers, link-arg rustflags, dangerous `[env]` names,
  aliases as manual; source replacement, registries and http proxy as "not analysed".
- `Cargo.toml`: only listed as not analysed (build script, proc-macro, `[patch]`/`[replace]`, git or alternative
  registry dependencies). A build script is not a command because `build.rs` is already flagged unread.
- `rust-toolchain(.toml)`: `toolchain.path`; a bare channel name is fine.
- `pyproject.toml`: `backend-path`, custom hatch hooks and pdm pre/post scripts are `on_build`; a build backend or
  requirement not on the fixed allowlists `KNOWN_BACKENDS`/`KNOWN_REQUIRES`, direct URL requirements, poetry/uv package
  indexes and pytest `-p` are "not analysed".
- `mise.toml`: hooks, `_.source`, `exec(` templates, plugins, tool postinstall; tasks are manual.
- `.codex/config.toml` (MCP server commands, `notify`) and `.gemini/settings.json` (MCP servers).

No schema change. Intake keeps the text of all `.toml` files.

## Decided by Francois: agent instruction files are excluded

AI-agent *instruction* files (`CLAUDE.md`, `AGENTS.md`, `.cursorrules`, Copilot instructions) are a prompt-injection
vector, not a command, and are deliberately **excluded**: not flagged, not listed as a kind. The agent *config* files
that name commands (`.claude/settings.json`, MCP files, `.codex/config.toml`, `.gemini/settings.json`) are read.

## `node <file>` in these commands is followed

Like a `package.json` script: `Cf::command` runs `interpret_command`, turns `Node` actions into `EntryWant`s
(`ConfigAnalysis::entries`) and `run.rs` feeds them to the linker and, in a quick scan, to the follower. Commands run
from the folder above a dot folder (`.vscode`, `.claude`, `.git`, `.cargo`, `.devcontainer`, `.cursor`, `.gemini`,
`.codex`), else from the file's own folder (`base_folder`); `${workspaceFolder}`, `$CLAUDE_PROJECT_DIR` and the like
count as that folder. A command that is only `node <file>` is not also "not followed"; anything else in it still is.
For MSBuild the file is a guess (`optional`). Not done: following imported `.targets` files; `npm run` inside these
commands. `config.rs` is still large; only the TOML part was split out.

## Checked against the canary test (`I:\canary-test`, 2026-11-22)

Its four vectors (npm `preinstall`, Cargo `rustc-wrapper`, MSBuild `InitialTargets` + `Exec`, a project-carried
`.venv\Scripts\python.exe` + `pyvenv.cfg`) are all reported, in a quick and a full scan. The fixture
`crates/forge-analysis-cli/tests/fixtures/canary` replicates that layout with relative paths and a stub `.exe` (test
`every_vector_in_the_canary_test_is_reported`); keep it passing. The canary's `requirements.txt` is empty: a real one
can name `--index-url`, `-e` or `git+` sources, and is only listed as an other file (a possible gap).

Still to do per ecosystem: new rows as Forge's panels learn to run more. Built from what Forge's panels really run and
what the canary test showed:

- .NET: `Directory.Build.props` and `.targets` (their targets run during a plain `dotnet list package` scan, high
  priority), `global.json`, `nuget.config`.
- Rust: `.cargo/config.toml`, `rust-toolchain.toml`, `build.rs`. `cargo metadata --no-deps` does not run a rustc
  wrapper, but `cargo check` and `cargo build` do.
- Node: `.npmrc`, install hooks. Python: a project-carried virtual-environment `python`.
