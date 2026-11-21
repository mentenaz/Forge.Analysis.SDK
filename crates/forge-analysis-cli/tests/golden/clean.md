# Forge analysis report

- Engine: `forge-analysis 0.0.1`
- Schema: 5
- Mode: static only (no code was run)
- Scope: full (every file we can read as code)
- Snapshot: `cba6ce1d98dfe0b5012bf70cc0e5f73700b73588bc6c75ae16a5b38c530e52b5`
- Catalog: `661ff253d0997c290715b16d7de935041072e290e1efd30a82fa0c75eed0949b`

<a id="index"></a>

## Index

1. [Overview](#overview) — What was analysed, whether the picture is complete, and one count for everything we could not see.
2. [Entry points](#entry-points) (1) — What runs first: runtime and install entry files, and declared entries that do not match any analysed file.
3. [Execution timeline](#execution-timeline) (3) — When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.
4. [Effects](#effects) (0) — What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.
5. [Data flows](#data-flows) (0) — Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.
6. [Install scripts and shell commands](#install-scripts-and-shell-commands) (0) — Every command a project file asks a tool or package manager to run (package.json scripts, editor tasks, agent hooks, git settings and more): when it runs, and the commands inside it that we could not follow.
7. [Dependencies](#dependencies) (0) — Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.
8. [Unresolved](#unresolved) (0) — Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.
9. [Unlinked imports](#unlinked-imports) (0) — Imports we could not follow to their source: packages, missing files and names that are not exported.
10. [Not analysed](#not-analysed) (0) — Files and folders that were not read as code, by kind, and anything skipped by a limit.
11. [Function index](#function-index) (3) — Every function with its lanes, effects, callers and the facts inside it.

<a id="overview"></a>

## 1. Overview

What was analysed, whether the picture is complete, and one count for everything we could not see.

**Status: complete.** Nothing was left unseen.

| What we could not see | Count | Listed in |
|---|---|---|
| places where static analysis cannot say what happens | 0 | [Unresolved](#unresolved) |
| imports not followed to their source | 0 | [Unlinked imports](#unlinked-imports) |
| declared entry points that match no analysed file | 0 | [Entry points](#entry-points) |
| files or folders not read as code | 0 | [Not analysed](#not-analysed) |
| dependencies whose own code and install scripts were not analysed | 0 | [Dependencies](#dependencies) |
| calls whose target we could not see (no function followed, no fact) | 0 | [Function index](#function-index) |

- Files analysed: 2
- Files not analysed: 0
- Functions: 3
- Cross-file links made: 2
- Unresolved facts not listed as open because their call was followed to its source: 1
- Facts found: 3
- Facts by kind: 0 entry, 1 import, 1 call, 0 effect, 0 timer, 0 script, 0 dependency, 1 unresolved, 0 flow

[Back to the index](#index)

<a id="entry-points"></a>

## 2. Entry points

What runs first: runtime and install entry files, and declared entries that do not match any analysed file.

Entry points come from what the package declares (`main`, `bin`, `exports`, scripts).

### Runtime entry files (1)

- `index.js`

### Install entry files (0)

Run when the package is installed.

None found.

### Declared entries that match no analysed file (0)

None found.

[Back to the index](#index)

<a id="execution-timeline"></a>

## 3. Execution timeline

When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.

### Immediate (3)

Runs right away when the program starts.

- top level of `index.js`
- top level of `util.js`
- `add` in `util.js:1`

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

None found.

[Back to the index](#index)

<a id="dependencies"></a>

## 7. Dependencies

Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.

None found.

[Back to the index](#index)

<a id="unresolved"></a>

## 8. Unresolved

Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.

None found.

[Back to the index](#index)

<a id="unlinked-imports"></a>

## 9. Unlinked imports

Imports we could not follow to their source: packages, missing files and names that are not exported.

None found.

[Back to the index](#index)

<a id="not-analysed"></a>

## 10. Not analysed

Files and folders that were not read as code, by kind, and anything skipped by a limit.

None found.

[Back to the index](#index)

<a id="function-index"></a>

## 11. Function index

Every function with its lanes, effects, callers and the facts inside it.

### `index.js` (1)

- top level at line 1
  - runs: immediate
  - calls to something other than a function in this file: 3 (1 followed through an import or a prop, 2 matched to a fact, 0 not traced to any code)
  - followed call at line 2 to `add` in `util.js` line 1 (through `./util`, export `add`)
  - facts: `fact:20fc86e92cbfc568`, `fact:69c0146d02a28261`, `fact:85fc4adfeb497561`

### `util.js` (2)

- top level at line 1
  - runs: immediate
- `add` at line 1
  - runs: immediate; also used as a value, so it may run from elsewhere
  - called through an import or a prop from: top level in `index.js`

[Back to the index](#index)
