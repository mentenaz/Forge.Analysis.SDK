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
    pub completeness: Completeness,
    /// Always all lines, zero counts included, so a reader can see each was checked.
    pub coverage: Vec<CoverageLine>,
    pub fact_counts: Vec<FactCount>,
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
    pub calls: Vec<FnId>,
    pub callbacks: Vec<FnId>,
    pub called_from: Vec<FnId>,
    /// Every fact inside it, whatever its kind.
    pub facts: Vec<FactId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionsSection {
    pub functions: Vec<FunctionItem>,
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
    pub not_analysed: NotAnalysedSection,
    pub functions: FunctionsSection,
}
