# Forge analysis engine

A static-only code analyser written in Rust. It reads a project and reports what the code does from
start to exit, as facts. It never runs the code it analyses. Milestone 1 covers JavaScript and
TypeScript. The engine only flags and reports: Forge is not accountable for what the analysed code
does, and the engine is not a gate for any other part of Forge.

Owner: Francois (Mentenaz). Workspace on his machine: `E:\mentenaz\forge-analysis`.
Design document: "Forge Analysis Engine: Schema Draft" (claude.ai artifact 3f5a92ba-3e69-4c4a-b58a-50ddd80b2d33).
It still lacks steps 9 to 20 and a "Legal and notices" section.

## Standing rules (never break these)

- **Static only.** Nothing in the engine executes, evaluates or imports the target code. Intake is
  read-only and never follows symlinks or junctions.
- **Nothing is skipped silently.** Every item is a fact, an unresolved fact with a reason, or a line
  in the coverage counts. People will try to find loopholes, so anything we cannot trace must still
  be counted and listed.
- **Three layers:** facts, evidence, findings. The findings layer is not built yet.
- **`Quoted` is the only home for target text.** Anything copied from the analysed code is a
  `Quoted`. Everything Forge writes itself is a closed enum or a `&'static str`.
- **The schema is Serialize-only.** Nothing deserializes a forged `Quoted`.
- **Completeness:** a report is `Incomplete` if any coverage count is above zero.
- **No "safe" verdicts.** The brief says `nothing_found`, `review` or `incomplete`, never "safe".
- **`unsafe_code = "forbid"`** for the whole workspace. oxc crates are pinned with `=0.159.0`.

## Crates (`crates/`)

| Crate | Role |
|---|---|
| `forge-analysis-schema` | Facts, `Quoted`, `RelPath`, `FactId`, validation. Serialize-only. |
| `forge-analysis-catalog` | What known modules, calls and members do (Node defaults). |
| `forge-analysis-intake` | Folder or in-memory files in, hashed read-only snapshot out. Size, count and depth limits, skip reasons. `snapshot` for a folder, `snapshot_from_memory` for `MemFile`s. |
| `forge-analysis-core` | The analysis: parsing with oxc, `imports/` (visitor, call graph, flow, linker, package.json). Pure: no process, network or file access. |
| `forge-analysis-report` | Builds the 11-section report model, then JSON and Markdown. Never drops an item: long sections are grouped. |
| `forge-analysis` | **The library.** `analyze_folder`, `analyze_files`, `analyze_snapshot`, `RunOptions` (limits, time budget, cancel flag), `brief`. Owns `REPORT_SCHEMA`. |
| `forge-analysis-cli` | Command line over the library. `analyze_folder` is a thin wrapper kept for the tests. |

Planned and not built: a fetch crate (registry downloads) and a research crate.

## How a run works

intake (snapshot, hash) → per file: `analyze_package` for `package.json`, `analyze_file_full` for code →
`link_project` (follows imports across files) → `build` (report) → optional `brief`.
The time budget and cancel flag are checked between files only. One huge file is bounded by the size
limits, not by the flag. Files not reached are reported as `NotAnalysedKind::Stopped`.

## Analysis facts worth knowing

- **Flows** (`flow.rs`): sources are network calls (fetch, https.get, XHR, WebSocket). Sinks are code
  arguments (eval, Function, vm, exec/spawn, worker, timer string) and callee sinks (`FlowSink::Callee`:
  a call on, or of, a value that came from the network). Propagation follows oxc symbols.
- **Known gap:** a remote object using a standard method name (`vuln.end()`, `send()`) is only reported
  as an untraced call, not as a flow. Typed receivers would fix it.
- **Call graph** (`callgraph.rs`): `Edges.seen_refs` keeps references unique. `Graph::owner` is O(functions)
  per call and may need an index later.
- A quadratic `Vec::contains` on nested function chains (`a=>a=>...`) was fixed in step 20. The slow test
  `long_chains_without_brackets` exercises 2 MiB chains.
- `REPORT_SCHEMA` is 5. **Bump rule** (also in the doc comment in `crates/forge-analysis/src/run.rs`): bump it when
  the JSON changes in a way a reader could meet (a new or renamed field, a new value of a written enum, a changed
  meaning; removals always). A change that only alters what the engine finds does not bump it. Goldens change with
  every bump and the commit says so. 1: first shape. 2: header `scope`, kind `out_of_scope`. 3: kind `auto_run_config`. 4: kind `archive`. 5: script triggers `on_open`, `on_git`, `on_build`.
- **Quick scan** (`RunOptions.scope = Scope::Quick`, CLI `--quick`/`-q`): analyses every `package.json` (any depth), the
  files its install hooks run, and what those load (followed with `FileIndex`, same resolver as the linker). Other code
  is `NotAnalysedKind::OutOfScope`; non-code keeps its usual kind. Report and brief are always incomplete under quick
  (Decision A); `Review` still wins in the brief. `main`/`bin` are not followed: they do not run by themselves.

## CI

`.github/workflows/ci.yml`: fast tests on Ubuntu and Windows, `cargo deny` on core, and the full release suite on pushes
to master/main (not pull requests). Rust is pinned to 1.100.0. Only GitHub's own actions are used. Passed on its first run.

## Commands (PowerShell, from the workspace folder)

```powershell
cargo test --workspace -- --skip long_chains_without_brackets   # fast check
cargo test --workspace                                          # full, includes the slow robustness test
cargo test -p forge-analysis-core --test robustness long_chains -- --nocapture
cargo run --release -p forge-analysis-cli -- <folder> --out <report-folder>   # binary name: forge-analysis; add --quick for a quick scan
cargo test -p forge-analysis-report <test_name>                  # one test (name filter works across crates)
cargo test -p forge-analysis-cli --test fixtures                 # CLI golden tests only
$env:FORGE_BLESS = "1"; cargo test -p forge-analysis-cli --test fixtures   # re-bless goldens, then read the diff
$env:FORGE_FUZZ_ITERS = "200"; $env:FORGE_FUZZ_SEED = "1"        # tune/reproduce the robustness fuzz (default 1500 iters)
cargo deny --manifest-path crates/forge-analysis-core/Cargo.toml check bans   # core must stay free of network/process/OS crates
```

`FORGE_BLESS` also works for the report crate's golden tests (`crates/forge-analysis-report/src/tests.rs`).
`deny.toml` at the root exists to enforce "core is pure": do not add a dependency that it bans.

The full run is slow in debug (about 75 minutes at the last measure; 4 minutes in release). Golden files
live in `crates/forge-analysis-cli/tests/golden` and `crates/forge-analysis-report/tests/golden`. Only
re-bless them when the report is meant to change, and say so in the commit.

The required toolchain is Rust 1.98 (oxc needs it). `rust-version` is commented out in the root `Cargo.toml`
and there is no `rust-toolchain.toml`, so nothing enforces it. The sandbox used for steps 19 to 21 had 1.97 and
needed `--ignore-rust-version`.

## Status

Done: steps 1 to 21, up to the library crate with in-memory input, time budget, cancel and `brief`
(step 21). Step 20 fixed the call-graph slowdown. Step 19 added tainted-callee flows.
Steps 16 to 21 are committed and pushed (private repo `mentenaz/Forge.Analysis.SDK`, branch `master`).
The stray `visitor.rs.bak` is gone. The fast workspace test passes on the owner's machine.
Waiting on the owner: sign off Milestone 1, and the full (slow) test run if he wants it confirmed.

## Next steps, in the agreed order

1. ~~Quick mode for JS/TS~~ done (first slice, no cache). The fixed list lives in `analyze_snapshot`.
2. **Per-ecosystem lists.** Slice A (list and flag, no parsing) is built, and the list is broad on purpose: everything
   a tool, editor, AI agent, package manager or build system might act on by itself (.NET, editors and agent hooks
   such as `.vscode/tasks.json`, `.claude/settings.json`, `.mcp.json`, git hooks, Python, Rust, Ruby, PHP, JVM,
   Apple, C/C++ build files, `.env`, autorun/shortcut files). The one place to extend it is
   `crates/forge-analysis-report/src/autorun.rs`: `CONFIG` (flagged, cannot parse yet: `NotAnalysedKind::AutoRunConfig`)
   and `CODE_PREFIXES` (JS/TS tool config such as `webpack.config.js`, `.eslintrc.cjs`: **a quick scan reads these as
   code**). Widening the table does not change the report shape, so it does not bump `REPORT_SCHEMA`. The brief reason
   is `AutoRunNotAnalysed`; an unread auto-run file makes the brief `incomplete` in either scope (`review` still wins).
   The list comes from tool documentation and known abuse and is **not yet checked against the canary test**; the
   entries say a file *can* act, not that it does. Project files (`.csproj` etc.) and several others match by
   extension or folder, so "fixed list, not a pattern" is relaxed for them.
   Intake makes one exception to "ignored folders are not entered": inside a `.git` folder (any depth) it reads
   `config` and the active files in `hooks` (not `*.sample`), because git runs them by itself. `.git` is still
   recorded once as ignored, so the rest of it stays counted; links are never followed; the in-memory path makes the
   same exception. Archives (`.zip`, `.tgz`, `.jar`, `.whl`, `.vsix` and more) get `NotAnalysedKind::Archive` and the
   brief reason `ArchiveNotOpened` (brief incomplete); they are **listed, not opened**. Known gaps: archives are not
   opened (needs size, zip-bomb and path-traversal limits); content inside a file (a `plugins` entry in
   `tsconfig.json`, a key that config.rs does not list) is not read; `.git/modules` and
   `.git/info` are not read. Slice B is partly built (see above).
   **Slice B, pieces 1 and 2 are built** (`crates/forge-analysis-core/src/imports/config.rs`): these files are now
   *read*, not only listed: `.git/config`, `.vscode/tasks.json`, `.claude/settings.json` (+ `.local`), `.mcp.json`,
   `.cursor/mcp.json`, `.vscode/mcp.json`, `devcontainer.json`, `composer.json`, `global.json` (`msbuild-sdks`). Each
   command becomes a `LifecycleScript` fact with a trigger (`on_open`, `on_git`, `on_build`, or `install`/`hook`/`manual`)
   plus an unresolved `ShellCommand` fact on the same text (quoted, not followed). Only commands that hang on an
   automatic trigger are a brief reason (`AutomaticCommand`, gives `review`); a task that someone must start is listed
   as `manual`. A read file with nothing automatic leaves the unread list; a file that does not parse gets
   `ParseFailed` (brief incomplete). Repeated keys are flagged (`AmbiguousBinding`). Editor files that allow comments
   use `json::parse_lenient`; the rest stay strict. Intake keeps the text of `.git/config`
   (`Options::source_path_endings`). Files the reader handles must also be on the `CONFIG` table (a test checks it).
   Command text is quoted in the report, so a secret written inside a command (a credential helper) would show.
   **Piece 3 (MSBuild XML and `nuget.config`) is built too**, with our own strict XML reader (`imports/xml.rs`: spans,
   4 MiB / 64-deep limits; a DOCTYPE, unknown entity or repeated attribute is refused; nothing expands). It reads
   `Directory.Build/Solution/Packages.*`, `*.csproj/.fsproj/.vbproj/.vcxproj/.proj` and any `*.props`/`*.targets`:
   targets that run by themselves (BeforeTargets/AfterTargets, InitialTargets/DefaultTargets, plus what they
   DependsOn or CallTarget), `Exec` (a command fact), `DownloadFile`, tasks we cannot see into (`Unzip`, the `MSBuild`
   task, tasks from a `UsingTask`), inline task code, Pre/PostBuildEvent, analyzers, SDKs from NuGet (Microsoft's are
   not reported), tool-path redirects (`CscToolPath`...), restore-source properties and non-standard `Import`s. Plain
   file tasks (Copy, Delete, WriteLinesToFile) and `PackageReference`s are NOT reported. `nuget.config`: any source
   other than nuget.org, `repositoryPath`/`globalPackagesFolder`, and the presence (never the values) of
   credentials. No schema change, so `REPORT_SCHEMA` stays 5.
   **Piece 4 (TOML) is built too** (`imports/config/tomlcfg.rs`, read with `toml_edit`, pinned `=0.22.27`, no default
   features; core's only new dependency; `cargo deny` still passes; TOML forbids repeated keys so they are a
   `ParseFailed`). Reads `.cargo/config.toml` (rustc wrappers, linker/runner, credential helpers, link-arg rustflags,
   dangerous `[env]` names, aliases as manual; source replacement, registries, http proxy as "not analysed"),
   `Cargo.toml` (only listed as not analysed: build script, proc-macro, `[patch]`/`[replace]`, git or alternative
   registry dependencies; a build script is not a command because `build.rs` is already flagged unread),
   `rust-toolchain(.toml)` (`toolchain.path`; a bare channel name is fine), `pyproject.toml` (`backend-path`, custom
   hatch hooks, pdm pre/post scripts are `on_build`; a build backend or requirement not on the fixed allowlists
   `KNOWN_BACKENDS`/`KNOWN_REQUIRES`, direct URL requirements, poetry/uv package indexes and pytest `-p` are "not
   analysed"), `mise.toml` (hooks, `_.source`, `exec(` templates, plugins, tool postinstall; tasks are manual),
   `.codex/config.toml` (MCP server commands, `notify`) and `.gemini/settings.json` (MCP servers, like the other
   agent files). No schema change. Intake keeps the text of all `.toml` files.
   **Decided by Francois: AI-agent *instruction* files** (`CLAUDE.md`, `AGENTS.md`, `.cursorrules`, Copilot
   instructions) are a prompt-injection vector, not a command, and are deliberately **excluded**: not flagged, not
   listed as a kind. (The agent *config* files that name commands, `.claude/settings.json`, MCP files,
   `.codex/config.toml`, `.gemini/settings.json`, are read.) Following `node x.js` inside these commands, and
   following imported `.targets` files, is not done. `config.rs` is still large; only the TOML part was split out.
   Still to do per ecosystem: confirm the list against the canary test. Built from what Forge's panels really run and what the canary test showed:
   - .NET: `Directory.Build.props` and `.targets` (their targets run during a plain `dotnet list package`
     scan, high priority), `global.json`, `nuget.config`.
   - Rust: `.cargo/config.toml`, `rust-toolchain.toml`, `build.rs`. Note that `cargo metadata --no-deps`
     does not run a rustc wrapper, but `cargo check` and `cargo build` do.
   - Node: `.npmrc`, install hooks. Python: a project-carried virtual-environment `python`.
3. **Restricted Mode integration (Forge side, not here):** show the quick-scan result in the Restricted
   Mode dialog. The panel trust gate itself is on Forge's `forge-panel-trust-gate` branch.
4. Then: the findings layer, lockfile and advisory matching (OSV), the fetch crate, typed receivers,
   catalog breadth, a command catalog, write-destination
   resolution, and a written bump rule for `REPORT_SCHEMA`.

Parked: behaviour diff between versions (needs fact IDs that stay stable across edits), a Python frontend.

## Decisions

**Decision A (decided by Francois):** when a quick scan did not examine some files, the brief says
`incomplete`. There is no separate quick state. The report carries `scope: quick` so the reader sees
the scan was deliberate, and every unread file is counted in `not_examined`. A quick scan can never say
`nothing_found`.

## How to work with Francois

- He prefers a design discussion before implementation. Propose, wait for agreement, then build.
- Give commands as step-by-step PowerShell. He runs them and pastes the output.
- Say plainly what was not verified. If something could not be run or confirmed, name it.
- Keep a short list of what changed and how to test it at the end of each step.
