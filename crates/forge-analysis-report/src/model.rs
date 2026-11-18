//! The report as data. Everything in here is Serialize-only: text from the analysed code is
//! always a `Quoted`, and everything Forge wrote itself is a closed enum or a `&'static str`.
//! No section ever drops an item: a long section is grouped, never cut.

use forge_analysis_schema::{
    DependencyInfo, Effect, EffectUse, EntryBasis, EntryIssue, FactId, FactKind, FnId, Header, Lane,
    Quoted, RelPath, ScriptTrigger, Span, Timing, UnlinkedReason, UnresolvedReason, UrlValue,
};
use serde::Serialize;

/// The eleven sections, in report order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionId {
    Overview,
    EntryPoints,
    Timeline,
    Effects,
    Flows,
    Scripts,
    Dependencies,
    Unresolved,
    Unlinked,
    NotAnalysed,
    Functions,
}

impl SectionId {
    pub const ALL: [SectionId; 11] = [
        SectionId::Overview,
        SectionId::EntryPoints,
        SectionId::Timeline,
        SectionId::Effects,
        SectionId::Flows,
        SectionId::Scripts,
        SectionId::Dependencies,
        SectionId::Unresolved,
        SectionId::Unlinked,
        SectionId::NotAnalysed,
        SectionId::Functions,
    ];

    pub fn number(self) -> u8 {
        self as u8 + 1
    }

    pub fn anchor(self) -> &'static str {
        match self {
            SectionId::Overview => "overview",
            SectionId::EntryPoints => "entry-points",
            SectionId::Timeline => "execution-timeline",
            SectionId::Effects => "effects",
            SectionId::Flows => "data-flows",
            SectionId::Scripts => "install-scripts-and-shell-commands",
            SectionId::Dependencies => "dependencies",
            SectionId::Unresolved => "unresolved",
            SectionId::Unlinked => "unlinked-imports",
            SectionId::NotAnalysed => "not-analysed",
            SectionId::Functions => "function-index",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            SectionId::Overview => "Overview",
            SectionId::EntryPoints => "Entry points",
            SectionId::Timeline => "Execution timeline",
            SectionId::Effects => "Effects",
            SectionId::Flows => "Data flows",
            SectionId::Scripts => "Install scripts and shell commands",
            SectionId::Dependencies => "Dependencies",
            SectionId::Unresolved => "Unresolved",
            SectionId::Unlinked => "Unlinked imports",
            SectionId::NotAnalysed => "Not analysed",
            SectionId::Functions => "Function index",
        }
    }

    /// One plain-language sentence for the index.
    pub fn explanation(self) -> &'static str {
        match self {
            SectionId::Overview => {
                "What was analysed, whether the picture is complete, and one count for everything we could not see."
            }
            SectionId::EntryPoints => {
                "What runs first: runtime and install entry files, and declared entries that do not match any analysed file."
            }
            SectionId::Timeline => {
                "When code runs: which functions run immediately, later, repeatedly, on events, at exit or at install, and every timer."
            }
            SectionId::Effects => {
                "What the code does to the outside world: files, network, processes, environment and loading of code, each with its location."
            }
            SectionId::Flows => {
                "Data from the network or a file that can reach code execution or a command, from where it enters to where it is used."
            }
            SectionId::Scripts => {
                "Every package.json script, when it runs, and the commands inside it that we could not follow."
            }
            SectionId::Dependencies => {
                "Every declared dependency with where its code comes from: registry, git, URL, local path or bundled."
            }
            SectionId::Unresolved => {
                "Places where static analysis cannot say what happens (computed code, dynamic loads, shell commands). Each one hides behaviour."
            }
            SectionId::Unlinked => {
                "Imports we could not follow to their source: packages, missing files and names that are not exported."
            }
            SectionId::NotAnalysed => {
                "Files and folders that were not read as code, by kind, and anything skipped by a limit."
            }
            SectionId::Functions => {
                "Every function with its lanes, effects, callers and the facts inside it."
            }
        }
    }
}

/// A function, located for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FnLoc {
    pub file: RelPath,
    pub func: FnId,
    pub name: Option<Quoted>,
    /// Where the function starts. None when the file has no summary for it.
    pub line: Option<u32>,
}

/// Where in the program a fact sits, so a reader can tell whether it can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Place {
    pub in_fn: Option<FnLoc>,
    pub lanes: Vec<Lane>,
    /// None for facts outside any function (package.json) or in a function we have no summary for.
    pub reachable: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    /// Nothing was left unseen. Rare: any external package or unparsed file prevents it.
    Complete,
    /// At least one coverage line has a non-zero count.
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoverageLine {
    /// The section that lists every item counted here.
    pub section: SectionId,
    pub label: &'static str,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FactCount {
    pub kind: FactKind,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Overview {
    pub files_analysed: u32,
    pub files_not_analysed: u32,
    pub functions: u32,
    pub facts: u32,
    pub links_made: u32,
    /// Unresolved facts not listed as open because the linker followed their call. Each is
    /// still a fact, listed in its function.
    pub unresolved_cleared_by_links: u32,
    pub completeness: Completeness,
    /// Always all lines, zero counts included, so a reader can see each was checked.
    pub coverage: Vec<CoverageLine>,
    pub fact_counts: Vec<FactCount>,
    /// Folders of built output that look like copies of source folders. Flag only: both
    /// copies are still analysed and every fact in them is still reported.
    pub mirrored_output: Vec<MirrorNote>,
    /// Functions no entry point reaches by any edge we can follow (call, callback or named as a
    /// value). A note, not a coverage line: they are still listed in the function index and
    /// their facts are still reported.
    pub unreached_functions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MirrorNote {
    /// e.g. `dist`
    pub output_dir: Quoted,
    /// e.g. `src`
    pub source_dir: Quoted,
    /// How many files in the output folder have a same-named file (ignoring extension)
    /// under the source folder.
    pub matching_files: u32,
    pub output_files: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexEntry {
    pub section: SectionId,
    pub number: u8,
    pub title: &'static str,
    pub explanation: &'static str,
    /// Items in the section. None for the overview.
    pub count: Option<u32>,
    pub anchor: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryPoints {
    pub basis: EntryBasis,
    pub runtime: Vec<RelPath>,
    pub install: Vec<RelPath>,
    pub issues: Vec<EntryIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaneGroup {
    pub lane: Lane,
    pub functions: Vec<FnLoc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimerItem {
    pub id: FactId,
    pub at: Span,
    pub detail: Option<Quoted>,
    pub timing: Timing,
    #[serde(flatten)]
    pub place: Place,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Timeline {
    pub lanes: Vec<LaneGroup>,
    pub timers: Vec<TimerItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectItem {
    pub id: FactId,
    pub at: Span,
    pub detail: Option<Quoted>,
    pub url: Option<UrlValue>,
    #[serde(flatten)]
    pub place: Place,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectGroup {
    pub effect: Effect,
    pub items: Vec<EffectItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlowItem {
    pub id: FactId,
    /// The sink.
    pub at: Span,
    pub origin: Span,
    /// The quoted source expression.
    pub detail: Option<Quoted>,
    #[serde(flatten)]
    pub place: Place,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptItem {
    pub id: FactId,
    pub at: Span,
    pub name: Quoted,
    pub trigger: ScriptTrigger,
    pub command: Option<Quoted>,
    /// Commands inside it we did not follow (the text of each), in source order.
    pub unfollowed: Vec<Quoted>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DependencyItem {
    pub id: FactId,
    pub at: Span,
    pub name: Option<Quoted>,
    pub info: DependencyInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnresolvedItem {
    pub id: FactId,
    pub at: Span,
    pub detail: Option<Quoted>,
    #[serde(flatten)]
    pub place: Place,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnresolvedGroup {
    pub reason: UnresolvedReason,
    pub items: Vec<UnresolvedItem>,
}

/// One import we could not follow, with every function that makes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnlinkedItem {
    pub spec: Quoted,
    pub name: Option<Quoted>,
    pub from: Vec<FnLoc>,
}

/// Calls made into one external package we did not analyse. Each call is still a fact (listed
/// here and in its function); they are grouped by package so they do not bury the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageCalls {
    /// The package specifier as imported (`react`, `react/jsx-runtime`).
    pub spec: Quoted,
    pub calls: u32,
    /// How many different functions make these calls.
    pub functions: u32,
    pub items: Vec<UnresolvedItem>,
}

/// How many calls one function makes into one external package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageCount {
    pub spec: Quoted,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnlinkedGroup {
    pub reason: UnlinkedReason,
    pub items: Vec<UnlinkedItem>,
}

/// Why a file was not read as code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotAnalysedKind {
    NpmConfig,
    ShellScript,
    Container,
    CiWorkflow,
    NativeBinary,
    /// Not valid UTF-8: analysed after replacing the bad bytes, so some text may differ.
    InvalidUtf8,
    OtherFile,
    TooLarge,
    Symlink,
    IgnoredDir,
    TooDeep,
    CapReached,
    Unreadable,
    Excluded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotAnalysedFile {
    pub path: Quoted,
    pub size: Option<u64>,
    pub kind: NotAnalysedKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotAnalysedGroup {
    pub kind: NotAnalysedKind,
    pub items: Vec<NotAnalysedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotAnalysedSection {
    pub groups: Vec<NotAnalysedGroup>,
    /// Skips beyond intake's record limit: counted, not listed.
    pub unrecorded: u64,
}

/// A call that was followed to the function it runs, in this or another file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrossCall {
    pub to: FnLoc,
    /// The import specifier the call went through.
    pub spec: Quoted,
    /// The export that was called (`default` for the default export).
    pub name: Quoted,
    /// True when the call was followed through a JSX prop: `name` is then the prop and `spec`
    /// is empty.
    pub via_prop: bool,
    /// Line of the call, when a fact at the call says so; the byte range is always there.
    pub line: Option<u32>,
    pub start_byte: u32,
    pub end_byte: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionItem {
    pub at: Span,
    pub file: RelPath,
    pub func: FnId,
    pub name: Option<Quoted>,
    pub lanes: Vec<Lane>,
    pub reachable_from_entry: bool,
    pub escapes: bool,
    pub direct_effects: Vec<EffectUse>,
    /// Across the whole project, after linking.
    pub transitive_effects: Vec<Effect>,
    pub unresolved: Vec<UnresolvedReason>,
    pub unfollowed_calls: u32,
    /// Of those calls: followed to a function through an import (listed in `followed_calls`).
    pub calls_followed_across_files: u32,
    /// Of those calls: sitting exactly on a fact (effect, import, timer, unresolved...).
    pub calls_matched_to_a_fact: u32,
    /// Of those calls: nothing in the analysis says what they run. Counted, not hidden.
    pub calls_not_traced: u32,
    /// Every one of those not-traced calls, named: what it calls, and where. Exactly
    /// `calls_not_traced` entries, in source order.
    pub untraced_calls: Vec<UntracedCall>,
    /// Calls this function makes into external packages (also facts in `facts`).
    pub calls_into_packages: Vec<PackageCount>,
    pub calls: Vec<FnId>,
    pub callbacks: Vec<FnId>,
    pub called_from: Vec<FnId>,
    /// Calls made through an import that were followed to the function they run.
    pub followed_calls: Vec<CrossCall>,
    /// Functions elsewhere that call this one through an import.
    pub followed_called_from: Vec<FnLoc>,
    /// Every fact inside it, whatever its kind.
    pub facts: Vec<FactId>,
}

/// A call whose target the analysis could not see. `callee` is a short label made from names
/// only (`console.log`, `(..).then`), so calls that differ only in arguments group together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UntracedCall {
    pub callee: Quoted,
    pub line: u32,
    pub start_byte: u32,
    pub end_byte: u32,
}

/// Every untraced call with the same callee label, across the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UntracedGroup {
    pub callee: Quoted,
    pub count: u32,
    pub sites: Vec<UntracedSite>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UntracedSite {
    pub in_fn: FnLoc,
    pub line: u32,
    pub start_byte: u32,
    pub end_byte: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionsSection {
    pub functions: Vec<FunctionItem>,
    /// The same untraced calls as in each function's `untraced_calls`, grouped by what they
    /// call, most common first. Their total is the coverage line's count.
    pub untraced_by_callee: Vec<UntracedGroup>,
    /// Facts the analysis did not place in any function. Listed, never dropped.
    pub unplaced_facts: Vec<FactId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub header: Header,
    pub index: Vec<IndexEntry>,
    pub overview: Overview,
    pub entry_points: EntryPoints,
    pub timeline: Timeline,
    pub effects: Vec<EffectGroup>,
    pub flows: Vec<FlowItem>,
    pub scripts: Vec<ScriptItem>,
    pub dependencies: Vec<DependencyItem>,
    pub unresolved: Vec<UnresolvedGroup>,
    pub unlinked: Vec<UnlinkedGroup>,
    pub package_calls: Vec<PackageCalls>,
    pub not_analysed: NotAnalysedSection,
    pub functions: FunctionsSection,
}
