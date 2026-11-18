use forge_analysis_catalog::Catalog;
use forge_analysis_schema::Effect;

#[test]
fn finds_a_function_by_module_and_export() {
    let c = Catalog::node_defaults();
    let e = c.find_function("fs", "readFile").expect("fs.readFile is catalogued");
    assert_eq!(e.category, Effect::FsRead);
    assert_eq!(e.args.path, Some(0));
}

#[test]
fn a_missing_export_or_module_finds_nothing() {
    let c = Catalog::node_defaults();
    assert!(c.find_function("fs", "watch").is_none());
    assert!(c.find_function("path", "join").is_none());
}

#[test]
fn knows_which_modules_it_covers() {
    let c = Catalog::node_defaults();
    assert!(c.knows_module("child_process"));
    assert!(!c.knows_module("zlib"));
}

#[test]
fn knows_which_names_it_exports() {
    let c = Catalog::node_defaults();
    assert!(c.exports_name("exec"));
    assert!(!c.exports_name("definitelyNotAnApi"));
}

#[test]
fn finds_code_loading_globals() {
    let c = Catalog::node_defaults();
    let e = c.find_global("eval").expect("eval is catalogued");
    assert_eq!(e.category, Effect::CodeLoad);
    assert!(c.find_global("Function").is_some());
    assert!(c.find_global("notAGlobal").is_none());
}

#[test]
fn finds_constructors_by_module() {
    let c = Catalog::node_defaults();
    let e = c.find_constructor("vm", "Script").expect("vm.Script is catalogued");
    assert_eq!(e.category, Effect::CodeLoad);
    assert!(c.find_constructor("vm", "Nope").is_none());
}

#[test]
fn vm_functions_are_code_load() {
    let c = Catalog::node_defaults();
    let e = c.find_function("vm", "runInThisContext").expect("catalogued");
    assert_eq!(e.category, Effect::CodeLoad);
}
