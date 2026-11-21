# Forge analysis report

- Engine: `forge-analysis 0.0.1`
- Schema: 4
- Mode: static only (no code was run)
- Scope: full (every file we can read as code)
- Snapshot: `d44f12d7976ed85c78472e2e037262cf742ebbc1ff3a7abd01cb3dfe62cd0884`
- Catalog: `661ff253d0997c290715b16d7de935041072e290e1efd30a82fa0c75eed0949b`

<a id="index"></a>

## Index

1. [Overview](#overview) — What was analysed, whether the picture is complete, and one count for everything we could not see.
2. [Entry points](#entry-points) (1) — What runs first: runtime and install entry files, and declared entries that do not match any analysed file.
3. [Execution timeline](#execution-timeline) (2) — When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.
4. [Effects](#effects) (0) — What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.
5. [Data flows](#data-flows) (0) — Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.
6. [Install scripts and shell commands](#install-scripts-and-shell-commands) (0) — Every package.json script, when it runs, and the commands inside it that we could not follow.
7. [Dependencies](#dependencies) (0) — Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.
8. [Unresolved](#unresolved) (0) — Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.
9. [Unlinked imports](#unlinked-imports) (2) — Imports we could not follow to their source: packages, missing files and names that are not exported.
10. [Not analysed](#not-analysed) (0) — Files and folders that were not read as code, by kind, and anything skipped by a limit.
11. [Function index](#function-index) (3) — Every function with its lanes, effects, callers and the facts inside it.

<a id="overview"></a>

## 1. Overview

What was analysed, whether the picture is complete, and one count for everything we could not see.

**Status: incomplete.** Some of the code could not be seen. Each line below says how much, and links to the section that lists every item.

| What we could not see | Count | Listed in |
|---|---|---|
| places where static analysis cannot say what happens | 0 | [Unresolved](#unresolved) |
| imports not followed to their source | 2 | [Unlinked imports](#unlinked-imports) |
| declared entry points that match no analysed file | 0 | [Entry points](#entry-points) |
| files or folders not read as code | 0 | [Not analysed](#not-analysed) |
| dependencies whose own code and install scripts were not analysed | 0 | [Dependencies](#dependencies) |
| calls whose target we could not see (no function followed, no fact) | 0 | [Function index](#function-index) |

- Files analysed: 1
- Files not analysed: 0
- Functions: 3
- Cross-file links made: 0
- Unresolved facts not listed as open because their call was followed to its source: 0
- Note: 1 of 3 functions are not reached from any entry point by a call, a callback or a name used as a value we could follow. They are listed in the function index and their facts are reported, but nothing says they run.
- Facts found: 7
- Facts by kind: 0 entry, 3 import, 1 call, 0 effect, 0 timer, 0 script, 0 dependency, 3 unresolved, 0 flow

[Back to the index](#index)

<a id="entry-points"></a>

## 2. Entry points

What runs first: runtime and install entry files, and declared entries that do not match any analysed file.

Entry points come from what the package declares (`main`, `bin`, `exports`, scripts).

### Runtime entry files (1)

- `app.js`

### Install entry files (0)

Run when the package is installed.

None found.

### Declared entries that match no analysed file (0)

None found.

[Back to the index](#index)

<a id="execution-timeline"></a>

## 3. Execution timeline

When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.

### Immediate (2)

Runs right away when the program starts.

- top level of `app.js`
- `view` in `app.js:4`

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

Every package.json script, when it runs, and the commands inside it that we could not follow.

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

### External package (2)

Code from a package. Its behaviour was not analysed. (A path alias from a bundler looks the same and may be a local file.)

- `ink` — imported by top level of `app.js`
- `react` — imported by top level of `app.js`

### Calls made into `react` (2)

2 calls from 2 functions. The package was not analysed, so what these calls do is not known. Each call is also listed in its function.

- `app.js:4` in `view` in `app.js:4` (runs: immediate): `React.createElement('div')`
- `app.js:5` in `state` in `app.js:5` (not reached from any entry): `React.useState(0)`

### Calls made into `ink` (1)

1 call from 1 function. The package was not analysed, so what these calls do is not known. Each call is also listed in its function.

- `app.js:6` in top level of `app.js` (runs: immediate): `ink.render(view())`

[Back to the index](#index)

<a id="not-analysed"></a>

## 10. Not analysed

Files and folders that were not read as code, by kind, and anything skipped by a limit.

None found.

[Back to the index](#index)

<a id="function-index"></a>

## 11. Function index

Every function with its lanes, effects, callers and the facts inside it.

### `app.js` (3)

- top level at line 1
  - runs: immediate
  - calls to something other than a function in this file: 2 (0 followed through an import or a prop, 2 matched to a fact, 0 not traced to any code)
  - calls into packages we did not analyse: `ink` ×1
  - facts: `fact:0e8974d2e99a81f2`, `fact:da328684252dd3ea`, `fact:aec0e42933347a43`, `fact:3e38faf0f57d911d`, `fact:aea167d1093d7fd1`
- `view` at line 4
  - runs: immediate
  - calls to something other than a function in this file: 1 (0 followed through an import or a prop, 1 matched to a fact, 0 not traced to any code)
  - calls into packages we did not analyse: `react` ×1
  - facts: `fact:ddc7df3b7874275e`
- `state` at line 5
  - runs: none (not reached from an entry)
  - calls to something other than a function in this file: 1 (0 followed through an import or a prop, 1 matched to a fact, 0 not traced to any code)
  - calls into packages we did not analyse: `react` ×1
  - facts: `fact:09cbef54f86a6978`

[Back to the index](#index)
