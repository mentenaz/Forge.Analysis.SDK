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
- `REPORT_SCHEMA` is 1. There is no written rule yet for when it must change. Writing one is open work.

## Commands (PowerShell, from the workspace folder)

```powershell
cargo test --workspace -- --skip long_chains_without_brackets   # fast check
cargo test --workspace                                          # full, includes the slow robustness test
cargo test -p forge-analysis-core --test robustness long_chains -- --nocapture
cargo run --release -p forge-analysis-cli -- <folder> --out <report-folder>   # binary name: forge-analysis
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
Waiting on the owner: confirm steps 20 and 21 on his machine, delete the stray
`crates/forge-analysis-core/src/imports/visitor.rs.bak` (it is tracked in git, so use `git rm`), commit steps 16 to 21, and sign off Milestone 1.

## Next steps, in the agreed order

1. **Quick mode for JS/TS:** read only the files that run by themselves, using a fixed list (not a
   pattern). Starts with `package.json` install hooks. Adds `scope: quick`. Counts every file it did not
   read in `not_examined`. No cache in the first slice.
2. **Per-ecosystem lists**, built from what Forge's panels really run and what the canary test showed:
   - .NET: `Directory.Build.props` and `.targets` (their targets run during a plain `dotnet list package`
     scan, high priority), `global.json`, `nuget.config`.
   - Rust: `.cargo/config.toml`, `rust-toolchain.toml`, `build.rs`. Note that `cargo metadata --no-deps`
     does not run a rustc wrapper, but `cargo check` and `cargo build` do.
   - Node: `.npmrc`, install hooks. Python: a project-carried virtual-environment `python`.
3. **Restricted Mode integration (Forge side, not here):** show the quick-scan result in the Restricted
   Mode dialog. The panel trust gate itself is on Forge's `forge-panel-trust-gate` branch.
4. Then: the findings layer, lockfile and advisory matching (OSV), the fetch crate, typed receivers,
   CI (there is no `.github` folder yet), catalog breadth, a command catalog, write-destination
   resolution, and a written bump rule for `REPORT_SCHEMA`.

Parked: behaviour diff between versions (needs fact IDs that stay stable across edits), a Python frontend.

## Open decision

**Decision A:** when a quick scan did not examine some files, does the brief say `incomplete`, or does it
keep a separate state? The brief currently covers full analyses only. Ask before building quick mode.

## How to work with Francois

- He prefers a design discussion before implementation. Propose, wait for agreement, then build.
- Give commands as step-by-step PowerShell. He runs them and pastes the output.
- Say plainly what was not verified. If something could not be run or confirmed, name it.
- Keep a short list of what changed and how to test it at the end of each step.
