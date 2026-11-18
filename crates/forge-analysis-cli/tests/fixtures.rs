//! End-to-end: real folders on disk through the whole pipeline.
//! Golden files pin the exact output. Rewrite them after an intended change with
//! `FORGE_BLESS=1 cargo test -p forge-analysis-cli --test fixtures`, then read the diff.

use forge_analysis_cli::analyze_folder;
use forge_analysis_intake::Options;
use forge_analysis_report::*;
use forge_analysis_schema::*;
use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

fn names() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(fixtures_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn report(name: &str) -> Report {
    analyze_folder(&fixtures_dir().join(name), &Options::default()).unwrap()
}

fn golden(name: &str, ext: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden").join(format!("{name}.{ext}"));
    if std::env::var("FORGE_BLESS").is_ok() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing {}; run with FORGE_BLESS=1", path.display()))
        .replace("\r\n", "\n");
    assert!(
        expected == actual,
        "{name}.{ext} differs from the golden file. If the change is intended, run with FORGE_BLESS=1 and review the diff."
    );
}

/// Removes inline code spans: a run of n backticks closes at the next run of exactly n.
fn outside_code(line: &str) -> String {
    let cs: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] != '`' {
            out.push(cs[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < cs.len() && cs[i] == '`' {
            i += 1;
        }
        let n = i - start;
        let mut j = i;
        let mut closed = None;
        while j < cs.len() {
            if cs[j] == '`' {
                let s = j;
                while j < cs.len() && cs[j] == '`' {
                    j += 1;
                }
                if j - s == n {
                    closed = Some(j);
                    break;
                }
            } else {
                j += 1;
            }
        }
        match closed {
            Some(end) => i = end,
            None => {
                // An unclosed run is literal text.
                out.extend(cs[start..i].iter());
            }
        }
    }
    out
}

fn json_strings(v: &serde_json::Value, key: &str, found: &mut Vec<(String, String)>) {
    match v {
        serde_json::Value::String(s) => found.push((key.to_string(), s.clone())),
        serde_json::Value::Array(a) => a.iter().for_each(|x| json_strings(x, key, found)),
        serde_json::Value::Object(m) => m.iter().for_each(|(k, x)| json_strings(x, k, found)),
        _ => {}
    }
}

#[test]
fn there_is_a_fixture_for_each_kind_of_project() {
    assert_eq!(names(), vec!["clean", "cross-file", "hostile", "install-script", "packages"]);
}

#[test]
fn every_fixture_accounts_for_every_fact_exactly_once() {
    for n in names() {
        let r = report(&n);
        assert_eq!(r.fact_ids().len() as u32, r.overview.facts, "{n}");
    }
}

#[test]
fn every_fixture_gives_the_same_report_twice() {
    for n in names() {
        assert_eq!(to_json(&report(&n)), to_json(&report(&n)), "{n}");
        assert_eq!(to_markdown(&report(&n)), to_markdown(&report(&n)), "{n}");
    }
}

#[test]
fn analysing_a_folder_changes_nothing_in_it() {
    fn listing(dir: &Path, out: &mut Vec<(PathBuf, u64)>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                listing(&p, out);
            } else {
                out.push((p.clone(), std::fs::metadata(&p).unwrap().len()));
            }
        }
    }
    for n in names() {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        listing(&fixtures_dir().join(&n), &mut a);
        let _ = report(&n);
        listing(&fixtures_dir().join(&n), &mut b);
        a.sort();
        b.sort();
        assert_eq!(a, b, "{n}");
    }
}

#[test]
fn markdown_is_safe_for_every_fixture() {
    for n in names() {
        let md = to_markdown(&report(&n));
        let opens = md.matches("<details>").count();
        let closes = md.lines().filter(|l| *l == "</details>").count();
        assert_eq!(opens, closes, "{n}: unbalanced details");
        for line in md.lines() {
            if line == "</details>" || line == "<details>" {
                continue;
            }
            let rest = outside_code(line);
            for bad in ["<script", "javascript:", "</details>", "pwned", "evil", "ignore all previous", "alert("] {
                assert!(!rest.to_lowercase().contains(bad), "{n}: {bad:?} outside a code span in: {line}");
            }
            if let Some(i) = rest.find("](") {
                assert!(rest[i..].starts_with("](#"), "{n}: a link that is not ours: {line}");
            }
            // Only our own headings may start a line with `#`.
            if rest.starts_with('#') {
                let t = rest.trim_start_matches('#');
                assert!(t.starts_with(' '), "{n}: {line}");
            }
        }
    }
}

#[test]
fn hostile_text_in_json_stays_in_quoted_objects() {
    for n in names() {
        let v: serde_json::Value = serde_json::from_str(&to_json(&report(&n))).unwrap();
        let mut found = Vec::new();
        json_strings(&v, "", &mut found);
        for (key, s) in found {
            let l = s.to_lowercase();
            if l.contains("pwned") || l.contains("evil") || l.contains("ignore all previous") {
                assert_eq!(key, "text", "{n}: target text outside a quoted object: {s}");
            }
        }
    }
}

#[test]
fn hostile_control_characters_are_gone() {
    let md = to_markdown(&report("hostile"));
    assert!(!md.contains('\u{202e}'));
    assert!(!md.chars().any(|c| c.is_control() && c != '\n'));
    let json = to_json(&report("hostile"));
    assert!(!json.contains('\u{202e}'));
}

#[test]
fn a_very_long_name_is_cut_and_says_how_long_it_was() {
    let md = to_markdown(&report("hostile"));
    assert!(md.contains("characters in all)"));
}

/// `index.js` has three calls to something other than a function in its own file:
/// `require('./util')` (sits on an import fact), `add(1, 2)` (followed to `util.js`) and
/// `console.log(...)` (nothing says what it runs). The first two are accounted for and the
/// third is counted on the coverage line, so the project is Incomplete for that one reason only.
#[test]
fn clean_accounts_for_each_call_and_counts_the_one_it_cannot_trace() {
    let r = report("clean");
    assert_eq!(r.entry_points.basis, EntryBasis::Declared);
    assert_eq!(r.overview.files_analysed, 2);
    assert!(r.unresolved.is_empty(), "{:?}", r.unresolved);
    assert_eq!(r.overview.links_made, 2, "the import and the call were both followed");
    assert_eq!(r.overview.unresolved_cleared_by_links, 1);
    let by = |file: &str, name: Option<&str>| {
        r.functions
            .functions
            .iter()
            .find(|f| f.file.as_quoted().text() == file && f.name.as_ref().map(|n| n.text()) == name)
            .unwrap()
    };
    let top = by("index.js", None);
    assert_eq!(top.unfollowed_calls, 3);
    assert_eq!(
        (top.calls_followed_across_files, top.calls_matched_to_a_fact, top.calls_not_traced),
        (1, 1, 1),
        "add is followed, require sits on an import fact, console.log is not traced"
    );
    assert_eq!(top.followed_calls.len(), 1);
    assert_eq!(top.followed_calls[0].to.file.as_quoted().text(), "util.js");
    assert_eq!(top.followed_calls[0].line, Some(2), "the call is on line 2 of index.js");
    assert!(top.unresolved.is_empty(), "the function no longer claims an open module alarm");
    let add = by("util.js", Some("add"));
    assert_eq!(add.followed_called_from.len(), 1);
    assert_eq!(add.followed_called_from[0].file.as_quoted().text(), "index.js");
    assert_eq!(r.fact_ids().len() as u32, r.overview.facts, "the cleared fact is still listed, in its function");
    let c = |s: SectionId| r.overview.coverage.iter().find(|c| c.section == s).unwrap().count;
    assert_eq!((c(SectionId::Unresolved), c(SectionId::Unlinked), c(SectionId::NotAnalysed), c(SectionId::EntryPoints), c(SectionId::Dependencies)), (0, 0, 0, 0, 0));
    assert_eq!(c(SectionId::Functions), 1, "the one call we cannot trace");
    assert_eq!(r.overview.completeness, Completeness::Incomplete);
}

/// Calls into external packages are grouped by package under Unlinked imports and are not open
/// unresolved items; `path.join` is known to have no effect, so it is a call fact and nothing is
/// left open. Every call is still a fact in its function.
#[test]
fn calls_into_packages_are_grouped_by_package_not_listed_as_unresolved() {
    let r = report("packages");
    let pkgs: Vec<(String, u32, u32)> =
        r.package_calls.iter().map(|p| (p.spec.text().to_string(), p.calls, p.functions)).collect();
    assert_eq!(pkgs, vec![("react".to_string(), 2, 2), ("ink".to_string(), 1, 1)]);
    let open: Vec<(UnresolvedReason, usize)> = r.unresolved.iter().map(|g| (g.reason, g.items.len())).collect();
    assert!(open.is_empty(), "path.join is a known call, so nothing stays open: {open:?}");
    let view = r.functions.functions.iter().find(|f| f.name.as_ref().map(|n| n.text()) == Some("view")).unwrap();
    assert_eq!(view.calls_into_packages.len(), 1);
    assert!(view.unresolved.is_empty(), "a function whose only flag is a package call has no open flag");
    assert_eq!(r.fact_ids().len() as u32, r.overview.facts);
}

/// Only an exact followed call clears the flag. In `cross-file`, `run('ls')` is followed and
/// `missing()` is not (no such export), so exactly that one stays open, next to its unlinked entry.
#[test]
fn a_call_that_was_not_followed_keeps_its_alarm() {
    let r = report("cross-file");
    let open: Vec<String> = r
        .unresolved
        .iter()
        .filter(|g| g.reason == UnresolvedReason::UncoveredModule)
        .flat_map(|g| g.items.iter().map(|i| i.detail.as_ref().map(|d| d.text().to_string()).unwrap_or_default()))
        .collect();
    assert_eq!(open, vec!["missing()".to_string()]);
    assert_eq!(r.overview.completeness, Completeness::Incomplete);
}

#[test]
fn golden_reports() {
    for n in names() {
        let r = report(&n);
        golden(&n, "json", &(to_json(&r) + "\n"));
        golden(&n, "md", &to_markdown(&r));
    }
}

#[test]
fn an_install_script_shows_up_as_install_work() {
    let r = report("install-script");
    assert_eq!(r.entry_points.install, vec![RelPath::new("setup.js").unwrap()]);
    let post = r.scripts.iter().find(|s| s.name.text() == "postinstall").unwrap();
    assert_eq!(post.trigger, ScriptTrigger::Install);
    let un: Vec<&str> = post.unfollowed.iter().map(|q| q.text()).collect();
    assert_eq!(un, vec!["curl", "sh"]);
    let effects: Vec<Effect> = r.effects.iter().map(|g| g.effect).collect();
    assert!(effects.contains(&Effect::FsDelete) && effects.contains(&Effect::NetOut) && effects.contains(&Effect::CodeLoad), "{effects:?}");
    assert!(!r.flows.is_empty(), "network text reaching eval is a flow");
    let del = r.effects.iter().find(|g| g.effect == Effect::FsDelete).unwrap();
    assert!(del.items[0].place.lanes.contains(&Lane::Install));
    let deps: Vec<(&str, DependencySource)> =
        r.dependencies.iter().map(|d| (d.name.as_ref().unwrap().text(), d.info.source)).collect();
    assert!(deps.contains(&("evil", DependencySource::Git)));
    assert!(deps.contains(&("local", DependencySource::LocalPath)));
    let kinds: Vec<NotAnalysedKind> = r.not_analysed.groups.iter().map(|g| g.kind).collect();
    assert!(kinds.contains(&NotAnalysedKind::NpmConfig) && kinds.contains(&NotAnalysedKind::ShellScript) && kinds.contains(&NotAnalysedKind::OtherFile), "{kinds:?}");
    assert_eq!(r.overview.completeness, Completeness::Incomplete);
}

#[test]
fn cross_file_reports_what_it_could_not_follow() {
    let r = report("cross-file");
    let find = |reason: UnlinkedReason| r.unlinked.iter().find(|g| g.reason == reason);
    assert!(find(UnlinkedReason::ExternalPackage).unwrap().items.iter().any(|i| i.spec.text() == "lodash"));
    assert!(find(UnlinkedReason::MissingFile).unwrap().items.iter().any(|i| i.spec.text() == "./gone.js"));
    assert!(find(UnlinkedReason::NoSuchExport).unwrap().items.iter().any(|i| i.name.as_ref().map(|n| n.text()) == Some("missing")));
    assert!(r.unresolved.iter().any(|g| g.reason == UnresolvedReason::ParseFailed), "broken.js");
    let spawn = r.effects.iter().find(|g| g.effect == Effect::ProcessSpawn).unwrap();
    assert_eq!(spawn.items[0].place.reachable, Some(true), "run() is called from main.js");
    let del = r.effects.iter().find(|g| g.effect == Effect::FsDelete).unwrap();
    assert_eq!(del.items[0].place.reachable, Some(false), "orphan.js is not reached from the declared entry");
}
