//! The command line: arguments, exit codes, and what gets written where.

use forge_analysis_cli::*;
use std::path::{Path, PathBuf};

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Temp {
        let p = std::env::temp_dir().join(format!("forge-analysis-cli-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Temp(p)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn go(a: &[&str]) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&args(a), &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name).to_string_lossy().into_owned()
}

#[test]
fn arguments_are_parsed() {
    let run = |f: &str, o: Option<&str>| Command::Run { folder: PathBuf::from(f), out: o.map(PathBuf::from), quick: false };
    assert_eq!(parse_args(&args(&["a"])), Ok(run("a", None)));
    assert_eq!(parse_args(&args(&["a", "--out", "o"])), Ok(run("a", Some("o"))));
    assert_eq!(parse_args(&args(&["--out", "o", "a"])), Ok(run("a", Some("o"))));
    assert_eq!(parse_args(&args(&["-o", "o", "a"])), Ok(run("a", Some("o"))));
    assert_eq!(parse_args(&args(&["a", "--out=o"])), Ok(run("a", Some("o"))));
    assert_eq!(parse_args(&args(&["--", "-odd-name"])), Ok(run("-odd-name", None)));
    let quick = Command::Run { folder: PathBuf::from("a"), out: None, quick: true };
    assert_eq!(parse_args(&args(&["a", "--quick"])), Ok(quick.clone()));
    assert_eq!(parse_args(&args(&["-q", "a"])), Ok(quick));
    assert_eq!(parse_args(&args(&["-h"])), Ok(Command::Help));
    assert_eq!(parse_args(&args(&["a", "--help"])), Ok(Command::Help));
    assert_eq!(parse_args(&args(&["--version"])), Ok(Command::Version));
    assert_eq!(parse_args(&args(&[])), Err(ParseError::NoFolder));
    assert_eq!(parse_args(&args(&["--out", "o"])), Err(ParseError::NoFolder));
    assert_eq!(parse_args(&args(&["a", "b"])), Err(ParseError::TwoFolders));
    assert_eq!(parse_args(&args(&["a", "--out"])), Err(ParseError::MissingValue("--out".into())));
    assert_eq!(parse_args(&args(&["a", "--out="])), Err(ParseError::MissingValue("--out".into())));
    assert_eq!(parse_args(&args(&["a", "--nope"])), Err(ParseError::UnknownFlag("--nope".into())));
}

#[test]
fn help_and_version_succeed() {
    let (code, out, _) = go(&["--help"]);
    assert_eq!((code, out.contains("Usage: forge-analysis")), (EXIT_OK, true));
    let (code, out, _) = go(&["--version"]);
    assert_eq!((code, out.starts_with("forge-analysis ")), (EXIT_OK, true));
}

#[test]
fn a_wrong_command_line_exits_2_with_the_usage() {
    for a in [&[][..], &["--nope"][..], &["a", "b"][..]] {
        let (code, out, err) = go(a);
        assert_eq!(code, EXIT_USAGE, "{a:?}");
        assert!(out.is_empty() && err.contains("Usage:"), "{a:?}");
    }
}

#[test]
fn a_folder_that_cannot_be_read_exits_3() {
    let t = Temp::new("missing");
    let (code, _, err) = go(&[t.0.join("nope").to_str().unwrap()]);
    assert_eq!(code, EXIT_CANNOT_RUN);
    assert!(err.contains("cannot open the folder"), "{err}");

    let file = t.0.join("a.txt");
    std::fs::write(&file, "x").unwrap();
    let (code, _, err) = go(&[file.to_str().unwrap()]);
    assert_eq!(code, EXIT_CANNOT_RUN);
    assert!(err.contains("not a folder"), "{err}");
}

#[test]
fn a_run_prints_a_summary_and_writes_nothing_without_out() {
    let (code, out, err) = go(&[&fixture("install-script")]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert!(out.contains("Status: incomplete"), "{out}");
    assert!(out.contains("places where static analysis cannot say what happens"));
    assert!(!out.contains("Report written"));
}

#[test]
fn quick_says_so_counts_what_it_skipped_and_is_never_complete() {
    let t = Temp::new("quick");
    let dest = t.0.join("q");
    let (code, out, err) = go(&[&fixture("install-script"), "--quick", "--out", dest.to_str().unwrap()]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert!(out.contains("Scope: quick"), "{out}");
    assert!(out.contains("Status: incomplete"), "{out}");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dest.join("report.json")).unwrap()).unwrap();
    assert_eq!(v["header"]["scope"], "quick");
    let md = std::fs::read_to_string(dest.join("report.md")).unwrap();
    assert!(md.contains("Scope: quick"));
    // A clean project is still incomplete under a quick scan.
    let (code, out, _) = go(&[&fixture("clean"), "--quick"]);
    assert_eq!(code, EXIT_OK);
    assert!(out.contains("Status: incomplete"), "{out}");
}

#[test]
fn with_out_it_writes_both_reports_and_creates_the_folder() {
    let t = Temp::new("out");
    let dest = t.0.join("deep").join("er");
    let (code, out, err) = go(&[&fixture("clean"), "--out", dest.to_str().unwrap()]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert!(out.contains("Report written to"));
    let json = std::fs::read_to_string(dest.join("report.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["header"]["schema"], 2);
    assert_eq!(v["header"]["mode"], "static_only");
    assert_eq!(v["header"]["scope"], "full");
    assert!(v["header"]["engine"].as_str().unwrap().starts_with("forge-analysis "));
    let md = std::fs::read_to_string(dest.join("report.md")).unwrap();
    assert!(md.starts_with("# Forge analysis report\n"));
}

#[test]
fn the_out_equals_form_works() {
    let t = Temp::new("eq");
    let (code, _, err) = go(&[&fixture("clean"), &format!("--out={}", t.0.display())]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert!(t.0.join("report.md").exists());
}

#[test]
fn an_output_folder_inside_the_analysed_folder_is_refused_and_nothing_is_created() {
    let t = Temp::new("inside");
    std::fs::write(t.0.join("a.js"), "1;\n").unwrap();
    for dest in [t.0.join("reports"), t.0.join("a").join("b"), t.0.clone()] {
        let (code, out, err) = go(&[t.0.to_str().unwrap(), "--out", dest.to_str().unwrap()]);
        assert_eq!(code, EXIT_USAGE, "{}", dest.display());
        assert!(err.contains("inside the analysed folder"), "{err}");
        assert!(out.is_empty(), "no analysis should run first");
    }
    assert!(!t.0.join("reports").exists() && !t.0.join("a").exists());
    assert!(!t.0.join("report.md").exists());
}

#[test]
fn a_sibling_whose_name_starts_the_same_is_not_inside() {
    let t = Temp::new("sibling");
    let src = t.0.join("proj");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.js"), "1;\n").unwrap();
    let dest = t.0.join("proj-reports");
    let (code, _, err) = go(&[src.to_str().unwrap(), "--out", dest.to_str().unwrap()]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert!(dest.join("report.json").exists());
}

#[test]
fn an_output_path_that_cannot_be_a_folder_exits_4() {
    let t = Temp::new("blocked");
    let blocker = t.0.join("file");
    std::fs::write(&blocker, "x").unwrap();
    let (code, out, err) = go(&[&fixture("clean"), "--out", blocker.to_str().unwrap()]);
    assert_eq!(code, EXIT_CANNOT_WRITE);
    assert!(err.contains("could not write the report"), "{err}");
    assert!(out.contains("Status:"), "the summary is still printed");
}

#[test]
fn an_empty_folder_is_a_complete_report_of_nothing() {
    let t = Temp::new("empty");
    let (code, out, err) = go(&[t.0.to_str().unwrap()]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert!(out.contains("Files analysed: 0"), "{out}");
}
