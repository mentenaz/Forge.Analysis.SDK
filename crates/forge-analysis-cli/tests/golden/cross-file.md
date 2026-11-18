# Forge analysis report

- Engine: `forge-analysis 0.0.1`
- Schema: 1
- Mode: static only (no code was run)
- Snapshot: `286395761548dfa7a3ee02e6d3ca3534abb33f5af2b50ba052982489a296c0d9`
- Catalog: `3964b56eb1f291b361d5245379fe30f34f914abad7446a37f6ac4c56aa0c11e0`

<a id="index"></a>

## Index

1. [Overview](#overview) — What was analysed, whether the picture is complete, and one count for everything we could not see.
2. [Entry points](#entry-points) (1) — What runs first: runtime and install entry files, and declared entries that do not match any analysed file.
3. [Execution timeline](#execution-timeline) (6) — When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.
4. [Effects](#effects) (2) — What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.
5. [Data flows](#data-flows) (0) — Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.
6. [Install scripts and shell commands](#install-scripts-and-shell-commands) (0) — Every package.json script, when it runs, and the commands inside it that we could not follow.
7. [Dependencies](#dependencies) (0) — Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.
8. [Unresolved](#unresolved) (3) — Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.
9. [Unlinked imports](#unlinked-imports) (3) — Imports we could not follow to their source: packages, missing files and names that are not exported.
10. [Not analysed](#not-analysed) (0) — Files and folders that were not read as code, by kind, and anything skipped by a limit.
11. [Function index](#function-index) (7) — Every function with its lanes, effects, callers and the facts inside it.

<a id="overview"></a>

## 1. Overview

What was analysed, whether the picture is complete, and one count for everything we could not see.

**Status: incomplete.** Some of the code could not be seen. Each line below says how much, and links to the section that lists every item.

| What we could not see | Count | Listed in |
|---|---|---|
| places where static analysis cannot say what happens | 3 | [Unresolved](#unresolved) |
| imports not followed to their source | 3 | [Unlinked imports](#unlinked-imports) |
| declared entry points that match no analysed file | 0 | [Entry points](#entry-points) |
| files or folders not read as code | 0 | [Not analysed](#not-analysed) |
| dependencies whose own code and install scripts were not analysed | 0 | [Dependencies](#dependencies) |

- Files analysed: 5
- Files not analysed: 0
- Functions: 7
- Cross-file links made: 4
- Facts found: 16
- Facts by kind: 0 entry, 6 import, 3 call, 2 effect, 1 timer, 0 script, 0 dependency, 4 unresolved, 0 flow

[Back to the index](#index)

<a id="entry-points"></a>

## 2. Entry points

What runs first: runtime and install entry files, and declared entries that do not match any analysed file.

Entry points come from what the package declares (`main`, `bin`, `exports`, scripts).

### Runtime entry files (1)

- `main.js`

### Install entry files (0)

Run when the package is installed.

None found.

### Declared entries that match no analysed file (0)

None found.

[Back to the index](#index)

<a id="execution-timeline"></a>

## 3. Execution timeline

When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.

### Immediate (4)

Runs right away when the program starts.

- top level of `helper.js`
- top level of `main.js`
- top level of `runner.js`
- `run` in `runner.js:2`

### Deferred (1)

Runs once, later (a timer or a callback).

- anonymous function in `main.js:6`

### Timers (1)

- `main.js:6` in top level of `main.js` (runs: immediate): deferred, 100 ms, never cleared, `setTimeout`

[Back to the index](#index)

<a id="effects"></a>

## 4. Effects

What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.

### Starts other programs (1)

- `runner.js:2` in `run` in `runner.js:2` (runs: immediate)

### Deletes files (1)

- `orphan.js:1` in top level of `orphan.js` (not reached from any entry): `/etc/x`

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

### Computed property access (1)

A member is picked by a computed name, so which function runs is unknown.

- `runner.js:2` in `run` in `runner.js:2` (runs: immediate): `cmd`

### Module our catalog does not cover (1)

Code uses a built-in module we have no description of yet.

- `main.js:6` in anonymous function in `main.js:6` (runs: deferred): `missing()`

### File did not parse (1)

The file has syntax errors, so nothing in it is known.

- `broken.js:1`: (empty)

[Back to the index](#index)

<a id="unlinked-imports"></a>

## 9. Unlinked imports

Imports we could not follow to their source: packages, missing files and names that are not exported.

### Name that is not exported (1)

The file is analysed, its exports are fully known, and it does not export this name.

- `./runner.js` → `missing` — imported by anonymous function in `main.js:6`

### File that does not exist (1)

A relative path that matches no file.

- `./gone.js` — imported by top level of `main.js`

### External package (1)

Code from a package. Its behaviour was not analysed. (A path alias from a bundler looks the same and may be a local file.)

- `lodash` — imported by top level of `main.js`

[Back to the index](#index)

<a id="not-analysed"></a>

## 10. Not analysed

Files and folders that were not read as code, by kind, and anything skipped by a limit.

None found.

[Back to the index](#index)

<a id="function-index"></a>

## 11. Function index

Every function with its lanes, effects, callers and the facts inside it.

### `helper.js` (2)

- top level at line 1
  - runs: immediate
- `helper` at line 1
  - runs: none (not reached from an entry); also used as a value, so it may run from elsewhere

### `main.js` (2)

- top level at line 1
  - runs: immediate
  - can reach: starts other programs
  - calls not followed to a function in this file: 2
  - followed call to `run` in `runner.js`:2 (through `./runner.js`, export `run`, bytes 132-141)
  - facts: `fact:9e3950ebcee45faf`, `fact:e5ba01ae63ea9ccd`, `fact:3aad1bf36a45fb26`, `fact:da767f241ea70a1d`, `fact:efab61581da7098d`, `fact:b16b8df71b81b888`, `fact:0aaa5afc057b9479`
- anonymous at line 6
  - runs: deferred
  - unresolved inside: Module our catalog does not cover
  - calls not followed to a function in this file: 1
  - facts: `fact:72428e957c1e45ae`

### `orphan.js` (1)

- top level at line 1
  - runs: none (not reached from an entry)
  - can reach: deletes files
  - calls not followed to a function in this file: 2
  - facts: `fact:6f61be9e7537cc5e`, `fact:604213d2ac79fa54`, `fact:1764ad4094257cb8`

### `runner.js` (2)

- top level at line 1
  - runs: immediate
  - facts: `fact:923cd4856da564c7`
- `run` at line 2
  - runs: immediate; also used as a value, so it may run from elsewhere
  - can reach: starts other programs
  - unresolved inside: Computed property access
  - calls not followed to a function in this file: 1
  - called through an import from: top level in `main.js`
  - facts: `fact:5d6d958f4743937b`, `fact:0bac39e12220c3da`, `fact:4258b73ee550cc1a`

### Facts not placed in any function (1)

- `fact:5dccd2a31f00f049`

[Back to the index](#index)
