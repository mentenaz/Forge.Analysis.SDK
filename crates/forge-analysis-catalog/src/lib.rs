//! Effect catalog: facts about APIs, loaded from TOML and validated.
//! Never loaded from the analysed folder. Severity and weakness mapping live elsewhere.

use forge_analysis_schema::{Effect, Hash};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

mod handles;
pub use handles::{handle_kind, is_platform_method, HandleKind};

const NODE_DEFAULTS: &str = include_str!("../data/node.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Source,
    Sink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Function,
    Global,
    Property,
    Constructor,
}

// ---- raw shape, exactly as written in TOML. Unknown fields are refused. ----

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    effect: Vec<RawEntry>,
    /// Calls known to have no effect outside the program (pure computation).
    #[serde(default)]
    known: Vec<RawKnown>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKnown {
    id: String,
    module: String,
    exports: Vec<String>,
}

/// A module function known to do nothing outside the program: no file, network, process or
/// environment access. Calling one is recorded as a call, not as something unseen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Known {
    pub id: String,
    pub module: String,
    pub exports: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    id: String,
    kind: Kind,
    category: Effect,
    module: Option<String>,
    exports: Option<Vec<String>>,
    global: Option<String>,
    object: Option<String>,
    property: Option<String>,
    path_arg: Option<u8>,
    url_arg: Option<u8>,
    command_arg: Option<u8>,
    delay_arg: Option<u8>,
    callback_arg: Option<u8>,
    role: Option<Role>,
    #[serde(default)]
    repeats: bool,
}

// ---- validated shape ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Function {
        module: String,
        exports: Vec<String>,
    },
    Global {
        name: String,
    },
    Property {
        object: String,
        property: String,
    },
    Constructor {
        module: String,
        exports: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ArgRoles {
    pub path: Option<u8>,
    pub url: Option<u8>,
    pub command: Option<u8>,
    pub delay: Option<u8>,
    pub callback: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub id: String,
    pub category: Effect,
    pub target: Target,
    pub args: ArgRoles,
    pub role: Option<Role>,
    pub repeats: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    Parse(String),
    BadId { id: String },
    KnownOverlapsEffect { id: String, module: String, name: String },
    DuplicateId { id: String },
    MissingField { id: String, field: &'static str },
    ForbiddenField { id: String, field: &'static str },
    EmptyName { id: String, field: &'static str },
    EmptyExports { id: String },
    DuplicateExport { id: String, name: String },
    ArgRoleNotAllowed { id: String, field: &'static str },
    RepeatsWithoutTimer { id: String },
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CatalogError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    entries: Vec<Entry>,
    known: Vec<Known>,
    version: Hash,
}

impl Catalog {
    pub fn from_toml(src: &str) -> Result<Self, CatalogError> {
        let raw: RawFile = toml::from_str(src).map_err(|e| CatalogError::Parse(e.to_string()))?;

        let mut seen = BTreeSet::new();
        let mut entries = Vec::with_capacity(raw.effect.len());
        for r in raw.effect {
            let entry = build(r)?;
            if !seen.insert(entry.id.clone()) {
                return Err(CatalogError::DuplicateId { id: entry.id });
            }
            entries.push(entry);
        }
        entries.sort_by(|a, b| a.id.cmp(&b.id));

        let mut known = Vec::with_capacity(raw.known.len());
        for k in raw.known {
            if !valid_id(&k.id) {
                return Err(CatalogError::BadId { id: k.id });
            }
            if !seen.insert(k.id.clone()) {
                return Err(CatalogError::DuplicateId { id: k.id });
            }
            let module = non_empty(&k.id, k.module, "module")?;
            let exports = exports_list(&k.id, k.exports)?;
            for name in &exports {
                let clash = entries.iter().any(|e| {
                    matches!(&e.target, Target::Function { module: m, exports: x }
                        if *m == module && x.contains(name))
                });
                if clash {
                    return Err(CatalogError::KnownOverlapsEffect {
                        id: k.id.clone(),
                        module: module.clone(),
                        name: name.clone(),
                    });
                }
            }
            known.push(Known { id: k.id, module, exports });
        }
        known.sort_by(|a, b| a.id.cmp(&b.id));

        // Hash of the canonical form (sorted, validated), not of the raw file text:
        // comments, whitespace and entry order don't change it, real content does.
        let canonical =
            serde_json::to_vec(&(&entries, &known)).map_err(|e| CatalogError::Parse(e.to_string()))?;
        let version = Hash::of(&canonical);
        Ok(Self { entries, known, version })
    }

    /// The Node core catalog compiled into the binary.
    pub fn node_defaults() -> Self {
        Self::from_toml(NODE_DEFAULTS)
            .expect("embedded node catalog must be valid; checked by tests")
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn known(&self) -> &[Known] {
        &self.known
    }

    /// True if `module.export` is listed as having no effect outside the program.
    pub fn find_known(&self, module: &str, export: &str) -> Option<&Known> {
        self.known
            .iter()
            .find(|k| k.module == module && k.exports.iter().any(|x| x == export))
    }

    pub fn version_hash(&self) -> &Hash {
        &self.version
    }

    /// The function entry for `module.export`, if the catalog has one.
    pub fn find_function(&self, module: &str, export: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| {
            matches!(&e.target, Target::Function { module: m, exports }
                if m == module && exports.iter().any(|x| x == export))
        })
    }

    /// True if any function entry belongs to this module.
    pub fn knows_module(&self, module: &str) -> bool {
        self.entries
            .iter()
            .any(|e| matches!(&e.target, Target::Function { module: m, .. } if m == module))
            || self.known.iter().any(|k| k.module == module)
    }

    /// True if some function entry exports this name, in any module.
    pub fn exports_name(&self, export: &str) -> bool {
        self.entries.iter().any(|e| {
            matches!(&e.target, Target::Function { exports, .. }
                if exports.iter().any(|x| x == export))
        })
    }

    /// The global-function entry for `name` (`eval`, `setTimeout`, ...).
    pub fn find_global(&self, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| matches!(&e.target, Target::Global { name: n } if n == name))
    }

    /// The constructor entry for `new module.export(...)`.
    pub fn find_constructor(&self, module: &str, export: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| {
            matches!(&e.target, Target::Constructor { module: m, exports }
                if m == module && exports.iter().any(|x| x == export))
        })
    }
}

// ---- validation helpers ----

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

fn forbid<T>(id: &str, v: &Option<T>, field: &'static str) -> Result<(), CatalogError> {
    if v.is_some() {
        Err(CatalogError::ForbiddenField {
            id: id.to_string(),
            field,
        })
    } else {
        Ok(())
    }
}

fn require<T>(id: &str, v: Option<T>, field: &'static str) -> Result<T, CatalogError> {
    v.ok_or(CatalogError::MissingField {
        id: id.to_string(),
        field,
    })
}

fn non_empty(id: &str, v: String, field: &'static str) -> Result<String, CatalogError> {
    if v.is_empty() {
        Err(CatalogError::EmptyName {
            id: id.to_string(),
            field,
        })
    } else {
        Ok(v)
    }
}

fn exports_list(id: &str, mut list: Vec<String>) -> Result<Vec<String>, CatalogError> {
    if list.is_empty() {
        return Err(CatalogError::EmptyExports { id: id.to_string() });
    }
    if list.iter().any(|s| s.is_empty()) {
        return Err(CatalogError::EmptyName {
            id: id.to_string(),
            field: "exports",
        });
    }
    list.sort();
    for w in list.windows(2) {
        if w[0] == w[1] {
            return Err(CatalogError::DuplicateExport {
                id: id.to_string(),
                name: w[0].clone(),
            });
        }
    }
    Ok(list)
}

fn build(raw: RawEntry) -> Result<Entry, CatalogError> {
    let id = raw.id;
    if !valid_id(&id) {
        return Err(CatalogError::BadId { id });
    }

    let target = match raw.kind {
        Kind::Function | Kind::Constructor => {
            forbid(&id, &raw.global, "global")?;
            forbid(&id, &raw.object, "object")?;
            forbid(&id, &raw.property, "property")?;
            let module = non_empty(&id, require(&id, raw.module, "module")?, "module")?;
            let exports = exports_list(&id, require(&id, raw.exports, "exports")?)?;
            if raw.kind == Kind::Function {
                Target::Function { module, exports }
            } else {
                Target::Constructor { module, exports }
            }
        }
        Kind::Global => {
            forbid(&id, &raw.module, "module")?;
            forbid(&id, &raw.exports, "exports")?;
            forbid(&id, &raw.object, "object")?;
            forbid(&id, &raw.property, "property")?;
            Target::Global {
                name: non_empty(&id, require(&id, raw.global, "global")?, "global")?,
            }
        }
        Kind::Property => {
            forbid(&id, &raw.module, "module")?;
            forbid(&id, &raw.exports, "exports")?;
            forbid(&id, &raw.global, "global")?;
            Target::Property {
                object: non_empty(&id, require(&id, raw.object, "object")?, "object")?,
                property: non_empty(&id, require(&id, raw.property, "property")?, "property")?,
            }
        }
    };

    let args = ArgRoles {
        path: raw.path_arg,
        url: raw.url_arg,
        command: raw.command_arg,
        delay: raw.delay_arg,
        callback: raw.callback_arg,
    };

    if matches!(target, Target::Property { .. }) {
        for (v, field) in [
            (args.path, "path_arg"),
            (args.url, "url_arg"),
            (args.command, "command_arg"),
            (args.delay, "delay_arg"),
            (args.callback, "callback_arg"),
        ] {
            if v.is_some() {
                return Err(CatalogError::ArgRoleNotAllowed { id, field });
            }
        }
    }
    if args.delay.is_some() && raw.category != Effect::Timer {
        return Err(CatalogError::ArgRoleNotAllowed {
            id,
            field: "delay_arg",
        });
    }
    if raw.repeats && raw.category != Effect::Timer {
        return Err(CatalogError::RepeatsWithoutTimer { id });
    }

    Ok(Entry {
        id,
        category: raw.category,
        target,
        args,
        role: raw.role,
        repeats: raw.repeats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(extra: &str) -> String {
        format!("[[effect]]\nid = \"t.x\"\n{extra}\n")
    }

    #[test]
    fn a_known_call_is_found_and_makes_its_module_known() {
        let c = Catalog::node_defaults();
        assert!(c.find_known("path", "join").is_some());
        assert!(c.find_known("path", "nope").is_none());
        assert!(c.knows_module("path"));
    }

    #[test]
    fn a_name_cannot_be_both_an_effect_and_known() {
        let src = "[[effect]]\nid = \"t.a\"\nkind = \"function\"\nmodule = \"m\"\nexports = [\"f\"]\ncategory = \"fs_read\"\n\n[[known]]\nid = \"t.b\"\nmodule = \"m\"\nexports = [\"f\"]\n";
        assert!(matches!(Catalog::from_toml(src), Err(CatalogError::KnownOverlapsEffect { .. })));
    }

    #[test]
    fn embedded_node_catalog_loads() {
        let c = Catalog::node_defaults();
        assert!(c.entries().len() >= 14);
        assert_eq!(c.version_hash().as_str().len(), 64);
    }

    #[test]
    fn hash_ignores_comments_whitespace_and_entry_order() {
        let a = r#"
[[effect]]
id = "t.a"
kind = "global"
global = "setInterval"
category = "timer"

[[effect]]
id = "t.b"
kind = "global"
global = "setTimeout"
category = "timer"
"#;
        let b = r#"
# same entries, other order, with a comment
[[effect]]
id       = "t.b"
kind     = "global"
global   = "setTimeout"
category = "timer"

[[effect]]
id       = "t.a"
kind     = "global"
global   = "setInterval"
category = "timer"
"#;
        assert_eq!(
            Catalog::from_toml(a).unwrap().version_hash(),
            Catalog::from_toml(b).unwrap().version_hash()
        );
    }

    #[test]
    fn export_order_does_not_change_the_hash() {
        let a = one("kind = \"function\"\nmodule = \"fs\"\nexports = [\"a\", \"b\"]\ncategory = \"fs_read\"");
        let b = one("kind = \"function\"\nmodule = \"fs\"\nexports = [\"b\", \"a\"]\ncategory = \"fs_read\"");
        assert_eq!(
            Catalog::from_toml(&a).unwrap().version_hash(),
            Catalog::from_toml(&b).unwrap().version_hash()
        );
    }

    #[test]
    fn hash_changes_when_content_changes() {
        let a =
            one("kind = \"function\"\nmodule = \"fs\"\nexports = [\"a\"]\ncategory = \"fs_read\"");
        let b = one("kind = \"function\"\nmodule = \"fs\"\nexports = [\"a\", \"b\"]\ncategory = \"fs_read\"");
        assert_ne!(
            Catalog::from_toml(&a).unwrap().version_hash(),
            Catalog::from_toml(&b).unwrap().version_hash()
        );
    }

    #[test]
    fn unknown_field_is_refused() {
        let s = one("kind = \"global\"\nglobal = \"x\"\ncategory = \"timer\"\npath_agr = 0");
        assert!(matches!(
            Catalog::from_toml(&s),
            Err(CatalogError::Parse(_))
        ));
    }

    #[test]
    fn unknown_category_is_refused() {
        let s = one("kind = \"global\"\nglobal = \"x\"\ncategory = \"fs_reed\"");
        assert!(matches!(
            Catalog::from_toml(&s),
            Err(CatalogError::Parse(_))
        ));
    }

    #[test]
    fn duplicate_id_is_refused() {
        let e =
            "[[effect]]\nid = \"t.x\"\nkind = \"global\"\nglobal = \"x\"\ncategory = \"timer\"\n";
        let s = format!("{e}\n{e}");
        assert_eq!(
            Catalog::from_toml(&s),
            Err(CatalogError::DuplicateId { id: "t.x".into() })
        );
    }

    #[test]
    fn bad_id_is_refused() {
        let s = "[[effect]]\nid = \"Bad Id\"\nkind = \"global\"\nglobal = \"x\"\ncategory = \"timer\"\n";
        assert_eq!(
            Catalog::from_toml(s),
            Err(CatalogError::BadId {
                id: "Bad Id".into()
            })
        );
    }

    #[test]
    fn function_without_module_is_refused() {
        let s = one("kind = \"function\"\nexports = [\"a\"]\ncategory = \"fs_read\"");
        assert_eq!(
            Catalog::from_toml(&s),
            Err(CatalogError::MissingField {
                id: "t.x".into(),
                field: "module"
            })
        );
    }

    #[test]
    fn global_with_module_is_refused() {
        let s = one("kind = \"global\"\nglobal = \"x\"\nmodule = \"fs\"\ncategory = \"timer\"");
        assert_eq!(
            Catalog::from_toml(&s),
            Err(CatalogError::ForbiddenField {
                id: "t.x".into(),
                field: "module"
            })
        );
    }

    #[test]
    fn property_with_an_arg_role_is_refused() {
        let s = one("kind = \"property\"\nobject = \"process\"\nproperty = \"env\"\ncategory = \"env_read\"\npath_arg = 0");
        assert_eq!(
            Catalog::from_toml(&s),
            Err(CatalogError::ArgRoleNotAllowed {
                id: "t.x".into(),
                field: "path_arg"
            })
        );
    }

    #[test]
    fn empty_exports_are_refused() {
        let s = one("kind = \"function\"\nmodule = \"fs\"\nexports = []\ncategory = \"fs_read\"");
        assert_eq!(
            Catalog::from_toml(&s),
            Err(CatalogError::EmptyExports { id: "t.x".into() })
        );
    }

    #[test]
    fn duplicate_exports_are_refused() {
        let s = one("kind = \"function\"\nmodule = \"fs\"\nexports = [\"a\", \"a\"]\ncategory = \"fs_read\"");
        assert_eq!(
            Catalog::from_toml(&s),
            Err(CatalogError::DuplicateExport {
                id: "t.x".into(),
                name: "a".into()
            })
        );
    }

    #[test]
    fn repeats_needs_a_timer() {
        let s = one("kind = \"global\"\nglobal = \"x\"\ncategory = \"fs_read\"\nrepeats = true");
        assert_eq!(
            Catalog::from_toml(&s),
            Err(CatalogError::RepeatsWithoutTimer { id: "t.x".into() })
        );
    }

    #[test]
    fn delay_arg_needs_a_timer() {
        let s = one("kind = \"global\"\nglobal = \"x\"\ncategory = \"fs_read\"\ndelay_arg = 1");
        assert_eq!(
            Catalog::from_toml(&s),
            Err(CatalogError::ArgRoleNotAllowed {
                id: "t.x".into(),
                field: "delay_arg"
            })
        );
    }

    #[test]
    fn interval_entry_is_recurring_with_its_roles() {
        let c = Catalog::node_defaults();
        let e = c
            .entries()
            .iter()
            .find(|e| e.id == "node.timers.interval")
            .unwrap();
        assert!(e.repeats);
        assert_eq!(e.args.delay, Some(1));
        assert_eq!(e.args.callback, Some(0));
    }
}
