# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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
| `forge-analysis-cli` | Command line over the library (`--quick`, `--brief`, `--out`). `analyze_folder` is a thin wrapper kept for the tests. |

Planned and not built: a fetch crate (registry downloads) and a research crate.

## How a run works

intake (snapshot, hash) → per file: `analyze_package` for `package.json`, `analyze_file_full` for code →
`link_project` (follows imports across files) → `build` (report) → optional `brief`.
The time budget and cancel flag are checked between files only. One huge file is bounded by the size
limits, not by the flag. Files not reached are reported as `NotAnalysedKind::Stopped`.

## Analysis facts worth knowing

- **Flows** (`flow.rs`): sources are network calls (fetch, https.get, XHR, WebSocket). Sinks are code
  arguments (eval, Function, vm, exec/spawn, worker, timer string) and callee sinks (`FlowSink::Callee`:
  a call on, or of, a value that came from the network). Propagation follows oxc symbols. A function value is tainted only by what it returns, not by what its body does.
- **Known gap:** a remote object using a standard method name (`vuln.end()`, `send()`) is only reported
  as an untraced call, not as a flow. Typed receivers would fix it.
- **Call graph** (`callgraph.rs`): `Edges.seen_refs` keeps references unique. `Graph::owner` is O(functions)
  per call and may need an index later.
- A quadratic `Vec::contains` on nested function chains (`a=>a=>...`) was fixed in step 20. The slow test
  `long_chains_without_brackets` exercises 2 MiB chains.
- `REPORT_SCHEMA` is 7. **Bump rule** (also in the doc comment in `crates/forge-analysis/src/run.rs`): bump it when
  the JSON changes in a way a reader could meet (a new or renamed field, a new value of a written enum, a changed
  meaning; removals always). A change that only alters what the engine finds does not bump it. Goldens change with
  every bump and the commit says so. 1: first shape. 2: header `scope`, kind `out_of_scope`. 3: kind `auto_run_config`. 4: kind `archive`. 5: script triggers `on_open`, `on_git`, `on_build`. 6: the report's fixed `notice`. 7: the brief's `untraced` count and reasons `no_code_read`, `entry_point_missing`.
- **Quick scan** (`RunOptions.scope = Scope::Quick`, CLI `--quick`/`-q`): analyses every `package.json` (any depth), the
  files its install hooks run, and what those load (followed with `FileIndex`, same resolver as the linker). Other code
  is `NotAnalysedKind::OutOfScope`; non-code keeps its usual kind. Report and brief are always incomplete under quick
  (Decision A); `Review` still wins in the brief. `main`/`bin` are not followed: they do not run by themselves.

## CI

`.github/workflows/ci.yml`: fast tests on Ubuntu and Windows, `cargo deny` on core, and the full release suite on pushes
to master/main (not pull requests), plus the `notices` job. Only GitHub's own actions are used. Passed on its first run
(before the `notices` job existed: that job has not run on GitHub yet).

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

**Toolchain:** oxc needs Rust 1.98 or newer; CI pins 1.100.0 (`ci.yml`). Nothing enforces it locally: `rust-version` is
commented out in the root `Cargo.toml` and there is no `rust-toolchain.toml`. A sandbox with 1.97 needed
`--ignore-rust-version`.

## When you change X, also change Y

- **The report JSON in a way a reader could meet:** bump `REPORT_SCHEMA` (rule below), re-bless the goldens, say so in the commit.
- **A brief verdict, scope, reason kind or field:** update `docs/brief-contract.md` (a test fails if they drift).
- **A file the config reader handles:** add it to `CONFIG` in `autorun.rs` (a test checks it).
- **Any dependency:** regenerate `THIRD-PARTY-NOTICES.md` (command under Legal and notices) and run `cargo deny` on core.
- **The `NOTICE` wording:** keep it in step with section 3 of the Terms of Use; it is a code change plus re-blessed goldens.

## Status

Done: steps 1 to 21, up to the library crate with in-memory input, time budget, cancel and `brief`
(step 21). Step 20 fixed the call-graph slowdown. Step 19 added tainted-callee flows.
Steps 16 to 21 and the licence commits (third-party notices, GPL licence, report notice, this file's rewrite) are
committed and pushed (private repo `mentenaz/Forge.Analysis.SDK`, branch `master`). The fast workspace test passes
(22 Nov 2026, Rust 1.100.0). The notice wording change (see Legal and notices) is committed locally and not pushed. The
`notices` CI job had not been checked on GitHub when this was written.
**Milestone 1 signed off by Francois on 22 Nov 2026** (engineering sign-off, not legal sign-off), after the full workspace
test passed in release mode on Rust 1.100.0 (`cargo test --workspace --release`, slow robustness suite 232 s, no failures).
Scope as agreed: JS/TS static analysis plus the auto-run config readers, report, brief, library and CLI; the rules and known
limits in this file stand. Not confirmed at sign-off: the GitHub CI run (including the `notices` job) and a run on real
projects, not only fixtures.

## Next steps, in the agreed order

1. ~~Quick mode for JS/TS~~ done (first slice, no cache). The fixed list lives in `analyze_snapshot`.
2. **Per-ecosystem lists.** Slices A and B are built (history and detail: `docs/ecosystem-lists.md`). Rules a reader of the
   code cannot see:
   - The one place to extend the list of files that can act by themselves is `crates/forge-analysis-report/src/autorun.rs`:
     `CONFIG` (flagged, `NotAnalysedKind::AutoRunConfig`) and `CODE_PREFIXES` (JS/TS tool config; **a quick scan reads
     these as code**). Widening the table does not bump `REPORT_SCHEMA`.
   - A file the reader in `imports/config.rs` handles must also be on `CONFIG` (a test checks it). An unread auto-run
     file makes the brief `incomplete` in either scope (`review` still wins); only commands on an automatic trigger give
     `review`; archives are listed, not opened (`ArchiveNotOpened`).
   - Intake reads `config` and active hooks inside any `.git` folder; everything else in `.git` stays counted as ignored.
   - **Decided by Francois:** AI-agent *instruction* files (`CLAUDE.md`, `AGENTS.md`, `.cursorrules`, Copilot instructions)
     are deliberately excluded (prompt injection, not a command). Agent *config* files that name commands are read.
   - `node <file>` in a read command is followed like a `package.json` script. `npm run` inside these commands and
     imported `.targets` files are not.
   - Command text is quoted in the report, so a secret written inside a command would show.
   - Keep the canary fixture (`crates/forge-analysis-cli/tests/fixtures/canary`, test
     `every_vector_in_the_canary_test_is_reported`) passing. Possible gap: a real `requirements.txt` can name
     `--index-url`, `-e` or `git+` sources and is only listed.
3. **Restricted Mode integration (Forge side, not here):** show the quick-scan result in the Restricted
   Mode dialog. The panel trust gate itself is on Forge's `forge-panel-trust-gate` branch. **Our side is done:**
   `forge-analysis <folder> --brief [--quick]` prints exactly one line of JSON (the brief) on stdout, messages go to
   stderr, `--out` also writes `brief.json`. The brief carries `report_schema` and `snapshot_sha256` (a verdict
   belongs to that exact content). The contract for readers is `docs/brief-contract.md`; a test
   (`the_contract_document_names_every_verdict_scope_reason_and_field`) fails if the code and the document drift
   apart, so update the document whenever a verdict, scope, reason kind or brief field changes.
4. Then: the findings layer, lockfile and advisory matching (OSV), the fetch crate, typed receivers,
   catalog breadth, a command catalog, write-destination
   resolution, and a written bump rule for `REPORT_SCHEMA`.

Parked: behaviour diff between versions (needs fact IDs that stay stable across edits), a Python frontend.

**Phase 2 (parked, after phase 1 is hardened): running code in an isolated VM.** Discussed with Francois on 22 Nov 2026;
nothing is built and none of it is agreed beyond what is written here.
- It breaks the static-only rule on purpose, so it is a **separate crate with its own opt-in**, never a mode of the engine.
- **No safe verdict, ever.** A clean run only says what was observed in that one run and environment. Code can detect a VM,
  wait, or depend on the target. The report says which network level was used.
- The VM does not talk to Forge while it runs; it is destroyed afterwards and the report is built from what was logged.
  Logs are **untrusted text** (`Quoted`, never instructions), the hand-off out of the VM is an attack surface, a missing or
  truncated log is `incomplete`, and tracing from outside the VM (hypervisor) is more trustworthy than logs written inside.
- Network: not a bridge. NAT alone is not isolation (it still reaches the LAN and the host); egress rules must be enforced on
  the host side (block private ranges, link-local, the host and gateway, cloud metadata, SMB) and tested from inside the VM.
  Levels: none; simulated internet; filtered real internet. Proposed default: simulated, with real access as an explicit
  opt-in. Real access is traffic from the user's own IP address, so it belongs in the terms and in the lawyer brief.
- Open: who provides the VM (today the terms say the user does). Before release: updated terms and notice for observed
  behaviour, and a lawyer review of the phase 2 design.

## Decisions

**Decision A (decided by Francois):** when a quick scan did not examine some files, the brief says
`incomplete`. There is no separate quick state. The report carries `scope: quick` so the reader sees
the scan was deliberate, and every unread file is counted in `not_examined`. A quick scan can never say
`nothing_found`.

**Decision B (decided by Francois, 22 Nov 2026, after running real projects):** the brief's `complete` is narrower than the
report's completeness on purpose. Untraced places (a computed `require`, an unknown call target) do **not** make a brief
`incomplete`: nearly every real project has them (`Forge.Lite`: about 287,000 lines). They are counted in the brief's `untraced`
field and listed in the report, so they are never hidden. A scan that read no code (`no_code_read`) or has a declared entry point
matching no file (`entry_point_missing`) is `incomplete`.

**Real-project run (22 Nov 2026, release build, full scan):** `forge-demo` 1 s; `fnm/site` under 1 s; `gpui-component/website` 3 s;
`Forge.Lite` (VS Code scale, about 27,000 JS/TS files) 826 s, reached the intake limit (9,613 files skipped and listed), report.json
1.9 GB and report.md 259 MB, almost all of it the Unresolved section. Fixed since: the 5 `data_reaches_code` on the built vitepress bundle were a false positive, not XHR methods. A function
whose BODY contained `new XMLHttpRequest` counted as network data, and that spread to anything calling it. Now (`flow.rs`,
`Mentions`) a function value is network data only through what it returns, or when it is called on the spot; a helper that returns
what it fetched (`const load = () => fetch(u)`) still gives a flow (tests cover both). Grouping, first layer built (Markdown only, no schema change): an Unresolved group of more than 25 items is written by file,
then function, then line, so no path repeats; every item is still listed. Measured on `Forge.Lite` (813 s): Unresolved 83 MB ->
36 MB (2.3x, not the 10-40x first estimated: the code text of each item stays), report.md 259 -> 212 MB. Function index 169 MB
(about 516,000 functions; spread over many line types, so regrouping alone will not shrink it much). report.json is 1.89 GB:
functions list 987 MB, `functions.untraced_by_callee` 508 MB (a copy of each function's `untraced_calls`), unresolved 363 MB,
everything else about 30 MB. Open: the second layer (Function index wording, or a separate file for it), the JSON duplicate,
no `--time-budget` flag on the CLI.

## Legal and notices

Francois is not a lawyer and neither is Claude: nothing here has had legal review.

- **Forge Terms of Use (draft v0.1)** live in the main repo `E:\zed_Source` (`mentenaz/mentenaz.Forge.Super`), file
  `legal/forge-terms.md`, on branch `forge-terms-draft` (not merged yet):
  https://github.com/mentenaz/mentenaz.Forge.Super/blob/forge-terms-draft/legal/forge-terms.md . Link to it; do not copy
  it here. The other files in that `legal/` folder are Zed Industries' own and are left as upstream has them.
  **State on 22 Nov 2026:** `forge-terms-draft` is pushed to GitHub, with the licence statement, publisher,
  contact, "free and at your own risk" (section 5) and liability capped at what the user paid (section 6). The 8 crates
  that had no licence now say `GPL-3.0-or-later` (branch `forge-crate-licences`, merged into it). Counts: GPL-3.0-or-later
  222, Apache-2.0 75, MIT 1 (`gpui_flow`); two example crates (`gpui_base/examples/wasm`, `gpui_web/examples/hello_web`)
  have none, which section 7 says. The branch is not merged into `main`. Still in `[square brackets]`: governing law and
  courts, the sign-off choice (DCO or not), and two "[Lawyer to confirm]" notes (liability limit, licence statement in
  section 7). Open, not built: the Accept flow and acceptance record the terms promise; Zed's own `legal/` files sit next to
  Forge's; `script/check-licenses` failed on Windows only because symlinks are off (`core.symlinks=false`), not yet run on Linux.
  The engine repo is private but GPL: decide whether it goes public, stays a separate program, or the source is offered.
  A lawyer review is advised before a wide release and is needed before phase 2 (limited scope is enough for phase 1).
- **Third-party notices:** `THIRD-PARTY-NOTICES.md` is generated, never edited by hand:
  `cargo about generate --workspace about.hbs -o THIRD-PARTY-NOTICES.md` (cargo-about 0.8.2, settings in `about.toml`).
  CI (`notices` job) regenerates it and fails if it differs. `.gitattributes` stores it byte for byte (some licence texts
  carry CRLF). Only Apache-2.0, MIT, Unicode-3.0 and Unlicense are accepted; another licence stops the generation.
  `self_cell` is `Apache-2.0 OR GPL-2.0-only` and is taken under Apache-2.0. The `oxc_*` crates ship no licence file, so
  `about.hbs` carries their copyright notice by hand (from the oxc repo's `LICENSE`, read 22 Nov 2026): re-check it when oxc
  is updated.
- **Licence of this engine's code: GPL-3.0-or-later (decided by Francois, same as Forge).** `LICENSE` is the official
  GPL-3.0 text from gnu.org; `[workspace.package] license` and each crate's `license.workspace = true` say
  `GPL-3.0-or-later`. Per-file licence headers are not added. Not legal advice: check the dependencies' licences stay
  GPL-compatible (Apache-2.0, MIT, Unicode-3.0 and Unlicense are accepted in `about.toml`).
- **Fixed notice in every report (built):** `NOTICE` in `crates/forge-analysis-report/src/model.rs`, the report's `notice`
  field and a `> **Notice.**` block at the top of the Markdown. It follows section 3 of the Terms of Use. Since 22 Nov 2026
  it says the report comes from static analysis only (the code was read, never run), that "nothing found" means only that
  nothing was found in what could be analysed and does not mean the code is safe, and that running, installing or opening
  the code is at the reader's own risk. Changing the wording is a code change plus re-blessed goldens (it does not bump
  `REPORT_SCHEMA`: the field is the same, the text is not a written enum); keep it in step with the terms.
- **Open:** the "Legal and notices" section of the design document (a suggestion links the terms and this repo's
  notices); the `[square brackets]` in the terms; legal review of everything above.

## How to work with Francois

- He prefers a design discussion before implementation. Propose, wait for agreement, then build.
- Give commands as step-by-step PowerShell. He runs them and pastes the output.
- Say plainly what was not verified. If something could not be run or confirmed, name it.
- Keep a short list of what changed and how to test it at the end of each step.
