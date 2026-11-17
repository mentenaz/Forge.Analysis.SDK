pub mod imports;

pub enum UnresolvedReason {
    DynamicRequire,
    Eval,
    Reflection,
    ComputedAccess,
    RuntimeDelay,
    EventHandler,
    ParseFailed,
    UnsupportedSyntax,
    AmbiguousBinding,
}
