# The brief: what Forge can rely on

The brief is the short answer the engine gives about a folder: one verdict, the reasons, and what was not
looked at. It is built only from a finished report, so it never says more than the report does. This document
is the contract for programs that read it (the Restricted Mode dialog in Forge, scripts, CI). The report
itself is described by the design document; this is only the brief.

The engine only flags and reports. It is not a gate, and Forge is not accountable for what the analysed code
does. A brief never says "safe".

## Getting it

```powershell
forge-analysis <folder> --brief                 # full scan; stdout is the brief
forge-analysis <folder> --brief --quick         # quick scan of the files that run by themselves
forge-analysis <folder> --brief --out <dir>     # also writes report.json, report.md and brief.json into <dir>
```

- With `--brief`, **stdout holds exactly one line: the brief as JSON.** Human messages go to stderr.
- From Rust: `forge_analysis::analyze_folder(path, &RunOptions { scope, ..Default::default() })`, then
  `forge_analysis::brief(&report)`. `analyze_files` takes files held in memory.
- The engine never runs the code it reads, never writes into the analysed folder, and never follows links.
  `--out` must be outside the analysed folder.
- Exit codes: `0` the scan finished (whatever the verdict says: `review` and `incomplete` are normal results),
  `2` the command line was wrong, `3` the folder could not be analysed (no brief is printed), `4` the report could
  not be written.

## The shape

```json
{
  "report_schema": 7,
  "snapshot_sha256": "<64 hex characters>",
  "verdict": "review",
  "scope": "full",
  "complete": false,
  "reasons": [ { "kind": "install_script", "count": 1 }, { "kind": "automatic_command", "count": 2 } ],
  "not_examined": 7,
  "files_analysed": 0,
  "untraced": 0
}
```

| Field | Meaning |
|---|---|
| `report_schema` | The `REPORT_SCHEMA` of the report this brief came from. See Versioning. |
| `snapshot_sha256` | The hash of the folder as it was read. **The verdict belongs to exactly this content.** Hash the folder again before acting on an old verdict; if it differs, the verdict no longer applies. |
| `verdict` | `nothing_found`, `review` or `incomplete`. See below. |
| `scope` | `full` (every file we can read as code) or `quick` (only the files that run by themselves). |
| `complete` | True only when nothing that should have been read was left unread. Never true for a quick scan. |
| `reasons` | At most 11, most serious first. Each is a `kind` and a `count`. A kind appears only when its count is above zero. |
| `not_examined` | Files that were not read as code (documents, images, binaries, archives, build files not read yet, and in a quick scan every code file left out). Always present, even when zero. |
| `files_analysed` | Code files that were read. |
| `untraced` | Places where static analysis cannot say what happens (a computed `require`, a call whose target is unknown). Always present, even when zero. **It does not change the verdict**: almost every real project has some, and every one is listed in the report. Read it as a size: 3 and 300,000 are different projects. |

## The verdict

- `review`: something on the review list was found. Look at the report before running the code. This wins over
  `incomplete`, and `complete` is still reported alongside it.
- `incomplete`: nothing on the review list was found, but part of what should have been read was not (see the
  reasons). **Not a clean result.**
- `nothing_found`: nothing on the review list was found in what was read, and all of it was read. It means what
  it says. It does not mean the code is safe: the code was never run, and running it is at the reader's own risk.
  A quick scan can never give it.

A program that does not know a verdict value must treat it as `review`.

## The reasons

| `kind` | What it counts | Gives `review` |
|---|---|---|
| `data_reaches_code` | Data from the network that can reach code execution, or a call on it. | yes |
| `install_script` | A package script that runs by itself at install. | yes |
| `install_time_effect` | Code on the install path that spawns processes, uses the network, changes files or loads code. | yes |
| `automatic_command` | A command a tool runs without anyone asking: an editor task set to run on folder open, an AI-agent hook or server, a dev container command, a git setting that runs a program, a compiler wrapper, a build target or SDK, a package-manager script event. | yes |
| `auto_run_not_analysed` | Files a tool acts on by itself that were not read (build and tool files, package-manager settings). | no, `incomplete` |
| `archive_not_opened` | Archives and packages whose contents are unseen. | no, `incomplete` |
| `run_stopped` | The run was stopped (time budget or cancel) before every file was read. | no, `incomplete` |
| `quick_scope` | A quick scan: code a full run would read was left unread on purpose. | no, `incomplete` |
| `no_code_read` | Not a quick scan, and no code file was read at all, so there was nothing to find anything in. | no, `incomplete` |
| `entry_point_missing` | Entry points the project declares (`main`, `bin`, `exports`, a script's file) that match no file we read. | no, `incomplete` |
| `not_fully_read` | Files skipped by a limit, unreadable files, and code that could not be parsed. Places the analysis cannot follow are counted in `untraced`, not here. | no, `incomplete` |

Counts are of items, not of files. The reasons list is a summary: the report has every item, with its place.
A program that does not know a `kind` should show it generically and not drop it.

## Rules that do not change

- Nothing is skipped silently: every item is a fact, an unresolved fact with a reason, or a line in the
  `not_examined` and coverage counts.
- The brief never says "safe" in any form.
- The same input gives the same brief, whatever order the files were given in.
- A quick scan reports `scope: "quick"`, `complete: false`, and a verdict of `review` or `incomplete`.

## Versioning

- `report_schema` is the version of the report's shape. It goes up whenever a field, or a value of an enum that
  is written out, is added, renamed or removed, or changes meaning. A program should read `report_schema` and
  refuse, or ask the user, when it is higher than the version it was written for.
- New `reasons[].kind` values and new fields can appear in a later version. Reading code must ignore fields it
  does not know and show reasons it does not know.
- Adding a verdict value would be a breaking change and would raise the version.
- Changing what the engine finds, without changing the shape, does not change `report_schema`. The same folder
  can therefore get a different brief after an engine update. The report's header carries the engine version
  and the catalog hash for that reason.

This document is checked by a test (`crates/forge-analysis/src/brief.rs`): every verdict, scope and reason kind in
the code must appear here.
