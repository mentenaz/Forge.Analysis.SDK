# Forge analysis report

- Engine: `forge-analysis 0.0.1`
- Schema: 1
- Mode: static only (no code was run)
- Snapshot: `9ce6978007195f638eeee5bbb73f818d77aa1370dbd1aa0b2d698984b2b370ce`
- Catalog: `3964b56eb1f291b361d5245379fe30f34f914abad7446a37f6ac4c56aa0c11e0`

<a id="index"></a>

## Index

1. [Overview](#overview) — What was analysed, whether the picture is complete, and one count for everything we could not see.
2. [Entry points](#entry-points) (1) — What runs first: runtime and install entry files, and declared entries that do not match any analysed file.
3. [Execution timeline](#execution-timeline) (1) — When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer.
4. [Effects](#effects) (4) — What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.
5. [Data flows](#data-flows) (0) — Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.
6. [Install scripts and shell commands](#install-scripts-and-shell-commands) (2) — Every package.json script, when it runs, and the commands inside it that we could not follow.
7. [Dependencies](#dependencies) (2) — Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.
8. [Unresolved](#unresolved) (3) — Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.
9. [Unlinked imports](#unlinked-imports) (4) — Imports we could not follow to their source: packages, missing files and names that are not exported.
10. [Not analysed](#not-analysed) (0) — Files and folders that were not read as code, by kind, and anything skipped by a limit.
11. [Function index](#function-index) (5) — Every function with its lanes, effects, callers and the facts inside it.

<a id="overview"></a>

## 1. Overview

What was analysed, whether the picture is complete, and one count for everything we could not see.

**Status: incomplete.** Some of the code could not be seen. Each line below says how much, and links to the section that lists every item.

| What we could not see | Count | Listed in |
|---|---|---|
| places where static analysis cannot say what happens | 3 | [Unresolved](#unresolved) |
| imports not followed to their source | 4 | [Unlinked imports](#unlinked-imports) |
| declared entry points that match no analysed file | 0 | [Entry points](#entry-points) |
| files or folders not read as code | 0 | [Not analysed](#not-analysed) |
| dependencies whose own code and install scripts were not analysed | 2 | [Dependencies](#dependencies) |

- Files analysed: 5
- Files not analysed: 0
- Functions: 5
- Cross-file links made: 0
- Facts found: 21
- Facts by kind: 0 entry, 6 import, 4 call, 4 effect, 0 timer, 2 script, 2 dependency, 3 unresolved, 0 flow

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

### Immediate (1)

Runs right away when the program starts.

- top level of `index.js`

### Timers (0)

None found.

[Back to the index](#index)

<a id="effects"></a>

## 4. Effects

What the code does to the outside world: files, network, processes, environment and loading of code, each with its location.

### Runs code built at run time (1)

- `index.js:6` in top level of `index.js` (runs: immediate): `1`

### Starts other programs (1)

- `[x](y).js:1` in top level of `[x](y).js` (not reached from any entry): `x`

### Deletes files (1)

- ````a`b``c```d.js:1```` in top level of ````a`b``c```d.js```` (not reached from any entry): `x`

### Makes network requests (1)

- `index.js:7` in top level of `index.js` (runs: immediate): ``https://user:pass@evil.example/p`a`th?token=1&`</details>`=2``

[Back to the index](#index)

<a id="data-flows"></a>

## 5. Data flows

Data from the network or a file that can reach code execution or a command, from where it enters to where it is used.

None found.

[Back to the index](#index)

<a id="install-scripts-and-shell-commands"></a>

## 6. Install scripts and shell commands

Every package.json script, when it runs, and the commands inside it that we could not follow.

### Scripts (2)

- `postinstall` (runs at install) at `package.json:4`: ``node `echo evil`.js <script>alert(1)</script> | sh``
  - not followed: `subshell`
- `build` (runs only when someone asks) at `package.json:5`: `echo "# Heading" </details> [x](javascript:alert(1))`
  - not followed: `subshell`

[Back to the index](#index)

<a id="dependencies"></a>

## 7. Dependencies

Every declared dependency with where its code comes from: registry, git, URL, local path or bundled.

### dependencies (2)

- `[click](http://evil.example)` `</details> # pwned`: package registry (at `package.json:8`)
- `normal` `^1.0.0`: package registry (at `package.json:9`)

[Back to the index](#index)

<a id="unresolved"></a>

## 8. Unresolved

Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour.

### Module chosen at run time (1)

A module is loaded by a name computed while running.

- `index.js:2` in top level of `index.js` (runs: immediate): ``require("`</details>\n# pwned")``

### Shell command not followed (2)

A command in a script that is not `node <file>` or `npm run <script>`, or one built at run time.

- `package.json:4`: `subshell`
- `package.json:5`: `subshell`

[Back to the index](#index)

<a id="unlinked-imports"></a>

## 9. Unlinked imports

Imports we could not follow to their source: packages, missing files and names that are not exported.

### File that does not exist (2)

A relative path that matches no file.

- `./gone` — imported by top level of `# Heading.js`
- `./xevil` — imported by top level of `index.js`

### External package (2)

Code from a package. Its behaviour was not analysed. (A path alias from a bundler looks the same and may be a local file.)

- ````a``b```c```` — imported by top level of `index.js`
- `mmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmm` (cut; 400 characters in all) — imported by top level of `index.js`

[Back to the index](#index)

<a id="not-analysed"></a>

## 10. Not analysed

Files and folders that were not read as code, by kind, and anything skipped by a limit.

None found.

[Back to the index](#index)

<a id="function-index"></a>

## 11. Function index

Every function with its lanes, effects, callers and the facts inside it.

### `![img](http).js` (1)

- top level at line 1
  - runs: none (not reached from an entry)

### `# Heading.js` (1)

- top level at line 1
  - runs: none (not reached from an entry)
  - calls not followed to a function in this file: 1
  - facts: `fact:54b2f2f39b301dbf`

### `[x](y).js` (1)

- top level at line 1
  - runs: none (not reached from an entry)
  - can reach: starts other programs
  - calls not followed to a function in this file: 2
  - facts: `fact:e078c00e036c3ef6`, `fact:d41303d2534a00e2`, `fact:a7ffc016b06b3842`

### ````a`b``c```d.js```` (1)

- top level at line 1
  - runs: none (not reached from an entry)
  - can reach: deletes files
  - calls not followed to a function in this file: 2
  - facts: `fact:622a1b670134a722`, `fact:c72d9f3694dc5dad`, `fact:6b22e68cfa1643ee`

### `index.js` (1)

- top level at line 1
  - runs: immediate
  - can reach: makes network requests, runs code built at run time
  - unresolved inside: Module chosen at run time
  - calls not followed to a function in this file: 6
  - facts: `fact:8dfc76ab0865fc05`, `fact:2604a0f4518977d8`, `fact:0eb079624129b4ca`, `fact:191e204c3cc8349d`, `fact:111dd675953354fd`, `fact:ecb9dce3f19f33a4`, `fact:97b7e410f9bf1d04`, `fact:b10f00e92ba8f090`

[Back to the index](#index)
