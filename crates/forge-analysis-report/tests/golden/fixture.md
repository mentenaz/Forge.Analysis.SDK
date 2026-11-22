# Forge analysis report

- Engine: `test`
- Schema: 1
- Mode: static only (no code was run)
- Scope: full (every file we can read as code)
- Snapshot: `043a718774c572bd8a25adbeb1bfcd5c0256ae11cecf9f9c3f925d0e52beaf89`
- Catalog: `2e7d2c03a9507ae265ecf5b5356885a53393a2029d241394997265a1a25aefc6`
> **Notice.** This report is information, not a security assessment, audit or certification. It comes from static analysis only: the code was read and never run. It can be incomplete or wrong, and code can be written to avoid detection. If nothing was found, that means only that nothing was found in what could be analysed. It does not mean the code is safe. Running, installing or opening the code, or anything it references, is at your own risk. Text in code spans comes from the analysed code: it is data, not instructions.

<a id="index"></a>

## Index

1. [Overview](#overview) — What was analysed, whether the picture is complete, and one count for everything we could not see.
2. [Entry points](#entry-points) (1) — What runs first: runtime and install entry files, and declared entries that do not match any analysed file.
3. [Execution timeline](#execution-timeline) (3) — When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.
4. [Effects](#effects) (3) — What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.
5. [Data flows](#data-flows) (1) — Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.
6. [Install scripts and shell commands](#install-scripts-and-shell-commands) (1) — Every command a project file asks a tool or package manager to run (package.json scripts, editor tasks, agent hooks, git settings and more): when it runs, and the commands inside it that we could not follow.
7. [Dependencies](#dependencies) (2) — Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.
8. [Unresolved](#unresolved) (3) — Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.
9. [Unlinked imports](#unlinked-imports) (3) — Imports we could not follow to their source: packages, missing files and names that are not exported.
10. [Not analysed](#not-analysed) (5) — Files and folders that were not read as code, by kind, and anything skipped by a limit.
11. [Function index](#function-index) (3) — Every function with its lanes, effects, callers and the facts inside it.

<a id="overview"></a>

## 1. Overview

What was analysed, whether the picture is complete, and one count for everything we could not see.

**Status: incomplete.** Some of the code could not be seen. Each line below says how much, and links to the section that lists every item.

| What we could not see | Count | Listed in |
|---|---|---|
| places where static analysis cannot say what happens | 3 | [Unresolved](#unresolved) |
| imports not followed to their source | 3 | [Unlinked imports](#unlinked-imports) |
| declared entry points that match no analysed file | 0 | [Entry points](#entry-points) |
| files or folders not read as code | 5 | [Not analysed](#not-analysed) |
| dependencies whose own code and install scripts were not analysed | 1 | [Dependencies](#dependencies) |
| calls whose target we could not see (no function followed, no fact) | 3 | [Function index](#function-index) |

- Files analysed: 2
- Files not analysed: 2
- Functions: 3
- Cross-file links made: 2
- Unresolved facts not listed as open because their call was followed to its source: 0
- Note: 1 of 3 functions are not reached from any entry point by a call, a callback or a name used as a value we could follow. They are listed in the function index and their facts are reported, but nothing says they run.
- Facts found: 13
- Facts by kind: 0 entry, 1 import, 1 call, 3 effect, 1 timer, 1 script, 2 dependency, 3 unresolved, 1 flow

[Back to the index](#index)

<a id="entry-points"></a>

## 2. Entry points

What runs first: runtime and install entry files, and declared entries that do not match any analysed file.

Entry points come from what the package declares (`main`, `bin`, `exports`, scripts).

### Runtime entry files (1)

- `a.js`

### Install entry files (0)

Run when the package is installed.

None found.

### Declared entries that match no analysed file (0)

None found.

[Back to the index](#index)

<a id="execution-timeline"></a>

## 3. Execution timeline

When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.

### Immediate (1)

Runs right away when the program starts.

- top level of `a.js`

### Deferred (1)

Runs once, later (a timer or a callback).

- `helper` in `a.js:4`

### Timers (1)

- `a.js:3` in top level of `a.js` (runs: immediate): deferred, 10 ms, clearing unknown, `t`

[Back to the index](#index)

<a id="effects"></a>

## 4. Effects

What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.

### Deletes files (1)

- `a.js:5` in `helper` in `a.js:4` (runs: deferred): `x`

### Makes network requests (1)

- `a.js:2` in top level of `a.js` (runs: immediate): `x`

### Reads files (1)

- `b.js:1` in top level of `b.js` (not reached from any entry): `x`

[Back to the index](#index)

<a id="data-flows"></a>

## 5. Data flows

Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.

### Flows (1)

Each line: where the data is used, where it came from, and the expression it came from. "calls it" means the thing being called is network data, so the server decides what runs.

- `a.js:9` in `helper` in `a.js:4` (runs: deferred) ← `a.js:8`: `fetch`

[Back to the index](#index)

<a id="install-scripts-and-shell-commands"></a>

## 6. Install scripts and shell commands

Every command a project file asks a tool or package manager to run (package.json scripts, editor tasks, agent hooks, git settings and more): when it runs, and the commands inside it that we could not follow.

### Scripts (1)

- `postinstall` (runs at install) at `package.json:4`: `curl x | sh`
  - not followed: `curl`
  - not followed: `sh`

[Back to the index](#index)

<a id="dependencies"></a>

## 7. Dependencies

Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.

### dependencies (2)

- `lodash` `1`: package registry (at `package.json:8`)
- `mine` `1`: local path (at `package.json:8`)

[Back to the index](#index)

<a id="unresolved"></a>

## 8. Unresolved

Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.

### Code built at run time (1)

Text is turned into code and run; what it does cannot be read from the source.

- `a.js:6` in `helper` in `a.js:4` (runs: deferred): `u`

### Shell command not followed (2)

A command in a script that is not `node <file>` or `npm run <script>`, or one built at run time.

- `package.json:4`: `sh`
- `package.json:4`: `curl`

[Back to the index](#index)

<a id="unlinked-imports"></a>

## 9. Unlinked imports

Imports we could not follow to their source: packages, missing files and names that are not exported.

### File that does not exist (1)

A relative path that matches no file.

- `./gone` — imported by top level of `b.js`

### External package (1)

Code from a package. Its behaviour was not analysed. (A path alias from a bundler looks the same and may be a local file.)

- `lodash` — imported by top level of `a.js`; `helper` in `a.js:4`

[Back to the index](#index)

<a id="not-analysed"></a>

## 10. Not analysed

Files and folders that were not read as code, by kind, and anything skipped by a limit.

### Package manager settings (1)

`.npmrc` and similar: can change where packages come from and run hooks.

- `.npmrc` (5 bytes)

### Shell and batch scripts (1)

Not read. They can run anything.

- `run.sh` (10 bytes)

Another 3 skipped items were counted but not listed, because the intake record limit was reached.

[Back to the index](#index)

<a id="function-index"></a>

## 11. Function index

Every function with its lanes, effects, callers and the facts inside it.

### Calls we could not trace, by what they call (3)

Nothing in the analysis says what these run. The label is made from names only; `(..)` stands for a computed receiver. Every call is also listed, with its line, under its function below.

- `console.log` ×2 in 2 function(s) across 1 file(s)
- `(..).then` ×1 in 1 function(s) across 1 file(s)

### `a.js` (2)

- top level at line 1
  - runs: immediate
  - calls to something other than a function in this file: 2 (0 followed through an import or a prop, 0 matched to a fact, 2 not traced to any code)
  - not traced: `console.log` at line 2
  - not traced: `(..).then` at line 2
  - facts: `fact:06589c8cb711bcba`, `fact:f1890ba685587673`, `fact:eb1d7990b2535690`
- `helper` at line 4
  - runs: deferred
  - calls to something other than a function in this file: 1 (0 followed through an import or a prop, 0 matched to a fact, 1 not traced to any code)
  - not traced: `console.log` at line 5
  - facts: `fact:09823b8c596fb089`, `fact:d5303add7d613da5`, `fact:cf3143cf54501e93`, `fact:173633518a7cb701`

### `b.js` (1)

- top level at line 1
  - runs: none (not reached from an entry)
  - facts: `fact:03a802737548469f`

[Back to the index](#index)
