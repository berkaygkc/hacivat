//! Why the classifier doubts or flags a statement (`alg/13`; Task 4). Each code has a literal
//! Fluent id `safety-reason-<kebab>` (P24); the messages are Task 4's block in
//! `hacivat-core.ftl`.

use std::sync::Arc;

use sql_text::TextRange;

/// A reason code.
#[non_exhaustive]
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ReasonCode {
    NotImplemented,
    LexError,
    ClientCommand,
    TemplateVariable,
    NoStatement,
    MultiStatement,
    AmbiguousBoundary,
    ParseError,
    ParserDisagreement,
    TooComplex,
    SyntaxNewerThanAgent,
    UnsupportedDialect,
    FnDenylisted,
    FnUnknown,
    OperatorUnknown,
    CastUnknown,
    ExecutableComment,
    LexModeChange,
    PrivilegeChange,
    DataModifyingCte,
    LockingClause,
    ExplainAnalyze,
    DynamicSql,
    ProceduralCode,
    TxnControl,
    SessionState,
    ExternalFile,
    ExternalNetwork,
    ProcessExec,
    AdminControl,
    XidConsume,
    SequenceAdvance,
    WriteTemp,
    HintExecTime,
    HintSetVar,
    RoutineCall,
    SetNotAllowlisted,
    // ## m3b-t04-safety: the policy's own reasons, one per effect group it refuses or
    // confirms that had no code (`decide` names every refusal; Task 4).
    /// The statement changes table data (`WRITE_DATA`).
    WriteData,
    /// The statement changes the schema (`DDL`, `DDL_TEMP`).
    SchemaChange,
    /// The statement locks whole tables (`LOCK_TABLE`).
    TableLock,
    /// The statement takes an advisory lock (`ADVISORY_LOCK`).
    AdvisoryLock,
    /// The statement reaches files or the network outside the database (`EXTERNAL_IO`).
    ExternalIo,
    /// The statement prepares a statement for later (`PREPARE`).
    PreparedStatement,
    /// The statement sends a notification (`NOTIFY`).
    Notify,
    /// The statement's effects could not be determined (`UNKNOWN`, an empty or unnamed set).
    EffectsUnknown,
    /// The statement holds a destructive pattern (`DestructiveKind`; the report lists which).
    Destructive,
}

impl ReasonCode {
    /// Every code, in declaration order.
    pub const ALL: [ReasonCode; 46] = [
        ReasonCode::NotImplemented,
        ReasonCode::LexError,
        ReasonCode::ClientCommand,
        ReasonCode::TemplateVariable,
        ReasonCode::NoStatement,
        ReasonCode::MultiStatement,
        ReasonCode::AmbiguousBoundary,
        ReasonCode::ParseError,
        ReasonCode::ParserDisagreement,
        ReasonCode::TooComplex,
        ReasonCode::SyntaxNewerThanAgent,
        ReasonCode::UnsupportedDialect,
        ReasonCode::FnDenylisted,
        ReasonCode::FnUnknown,
        ReasonCode::OperatorUnknown,
        ReasonCode::CastUnknown,
        ReasonCode::ExecutableComment,
        ReasonCode::LexModeChange,
        ReasonCode::PrivilegeChange,
        ReasonCode::DataModifyingCte,
        ReasonCode::LockingClause,
        ReasonCode::ExplainAnalyze,
        ReasonCode::DynamicSql,
        ReasonCode::ProceduralCode,
        ReasonCode::TxnControl,
        ReasonCode::SessionState,
        ReasonCode::ExternalFile,
        ReasonCode::ExternalNetwork,
        ReasonCode::ProcessExec,
        ReasonCode::AdminControl,
        ReasonCode::XidConsume,
        ReasonCode::SequenceAdvance,
        ReasonCode::WriteTemp,
        ReasonCode::HintExecTime,
        ReasonCode::HintSetVar,
        ReasonCode::RoutineCall,
        ReasonCode::SetNotAllowlisted,
        ReasonCode::WriteData,
        ReasonCode::SchemaChange,
        ReasonCode::TableLock,
        ReasonCode::AdvisoryLock,
        ReasonCode::ExternalIo,
        ReasonCode::PreparedStatement,
        ReasonCode::Notify,
        ReasonCode::EffectsUnknown,
        ReasonCode::Destructive,
    ];

    /// The Fluent message id, a literal per arm (P24).
    pub fn fluent_id(self) -> &'static str {
        match self {
            ReasonCode::NotImplemented => "safety-reason-not-implemented",
            ReasonCode::LexError => "safety-reason-lex-error",
            ReasonCode::ClientCommand => "safety-reason-client-command",
            ReasonCode::TemplateVariable => "safety-reason-template-variable",
            ReasonCode::NoStatement => "safety-reason-no-statement",
            ReasonCode::MultiStatement => "safety-reason-multi-statement",
            ReasonCode::AmbiguousBoundary => "safety-reason-ambiguous-boundary",
            ReasonCode::ParseError => "safety-reason-parse-error",
            ReasonCode::ParserDisagreement => "safety-reason-parser-disagreement",
            ReasonCode::TooComplex => "safety-reason-too-complex",
            ReasonCode::SyntaxNewerThanAgent => "safety-reason-syntax-newer-than-agent",
            ReasonCode::UnsupportedDialect => "safety-reason-unsupported-dialect",
            ReasonCode::FnDenylisted => "safety-reason-fn-denylisted",
            ReasonCode::FnUnknown => "safety-reason-fn-unknown",
            ReasonCode::OperatorUnknown => "safety-reason-operator-unknown",
            ReasonCode::CastUnknown => "safety-reason-cast-unknown",
            ReasonCode::ExecutableComment => "safety-reason-executable-comment",
            ReasonCode::LexModeChange => "safety-reason-lex-mode-change",
            ReasonCode::PrivilegeChange => "safety-reason-privilege-change",
            ReasonCode::DataModifyingCte => "safety-reason-data-modifying-cte",
            ReasonCode::LockingClause => "safety-reason-locking-clause",
            ReasonCode::ExplainAnalyze => "safety-reason-explain-analyze",
            ReasonCode::DynamicSql => "safety-reason-dynamic-sql",
            ReasonCode::ProceduralCode => "safety-reason-procedural-code",
            ReasonCode::TxnControl => "safety-reason-txn-control",
            ReasonCode::SessionState => "safety-reason-session-state",
            ReasonCode::ExternalFile => "safety-reason-external-file",
            ReasonCode::ExternalNetwork => "safety-reason-external-network",
            ReasonCode::ProcessExec => "safety-reason-process-exec",
            ReasonCode::AdminControl => "safety-reason-admin-control",
            ReasonCode::XidConsume => "safety-reason-xid-consume",
            ReasonCode::SequenceAdvance => "safety-reason-sequence-advance",
            ReasonCode::WriteTemp => "safety-reason-write-temp",
            ReasonCode::HintExecTime => "safety-reason-hint-exec-time",
            ReasonCode::HintSetVar => "safety-reason-hint-set-var",
            ReasonCode::RoutineCall => "safety-reason-routine-call",
            ReasonCode::SetNotAllowlisted => "safety-reason-set-not-allowlisted",
            ReasonCode::WriteData => "safety-reason-write-data",
            ReasonCode::SchemaChange => "safety-reason-schema-change",
            ReasonCode::TableLock => "safety-reason-table-lock",
            ReasonCode::AdvisoryLock => "safety-reason-advisory-lock",
            ReasonCode::ExternalIo => "safety-reason-external-io",
            ReasonCode::PreparedStatement => "safety-reason-prepared-statement",
            ReasonCode::Notify => "safety-reason-notify",
            ReasonCode::EffectsUnknown => "safety-reason-effects-unknown",
            ReasonCode::Destructive => "safety-reason-destructive",
        }
    }
}

/// One reason, with the range it points at and an argument (a function or variable name).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Reason {
    pub code: ReasonCode,
    pub range: Option<TextRange>,
    pub arg: Option<Arc<str>>,
}

impl Reason {
    /// A reason with no range and no argument.
    pub fn new(code: ReasonCode) -> Self {
        Reason {
            code,
            range: None,
            arg: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A compile-time proof that `ALL` lists every variant: adding one breaks this match.
    fn index(c: ReasonCode) -> usize {
        match c {
            ReasonCode::NotImplemented => 0,
            ReasonCode::LexError => 1,
            ReasonCode::ClientCommand => 2,
            ReasonCode::TemplateVariable => 3,
            ReasonCode::NoStatement => 4,
            ReasonCode::MultiStatement => 5,
            ReasonCode::AmbiguousBoundary => 6,
            ReasonCode::ParseError => 7,
            ReasonCode::ParserDisagreement => 8,
            ReasonCode::TooComplex => 9,
            ReasonCode::SyntaxNewerThanAgent => 10,
            ReasonCode::UnsupportedDialect => 11,
            ReasonCode::FnDenylisted => 12,
            ReasonCode::FnUnknown => 13,
            ReasonCode::OperatorUnknown => 14,
            ReasonCode::CastUnknown => 15,
            ReasonCode::ExecutableComment => 16,
            ReasonCode::LexModeChange => 17,
            ReasonCode::PrivilegeChange => 18,
            ReasonCode::DataModifyingCte => 19,
            ReasonCode::LockingClause => 20,
            ReasonCode::ExplainAnalyze => 21,
            ReasonCode::DynamicSql => 22,
            ReasonCode::ProceduralCode => 23,
            ReasonCode::TxnControl => 24,
            ReasonCode::SessionState => 25,
            ReasonCode::ExternalFile => 26,
            ReasonCode::ExternalNetwork => 27,
            ReasonCode::ProcessExec => 28,
            ReasonCode::AdminControl => 29,
            ReasonCode::XidConsume => 30,
            ReasonCode::SequenceAdvance => 31,
            ReasonCode::WriteTemp => 32,
            ReasonCode::HintExecTime => 33,
            ReasonCode::HintSetVar => 34,
            ReasonCode::RoutineCall => 35,
            ReasonCode::SetNotAllowlisted => 36,
            ReasonCode::WriteData => 37,
            ReasonCode::SchemaChange => 38,
            ReasonCode::TableLock => 39,
            ReasonCode::AdvisoryLock => 40,
            ReasonCode::ExternalIo => 41,
            ReasonCode::PreparedStatement => 42,
            ReasonCode::Notify => 43,
            ReasonCode::EffectsUnknown => 44,
            ReasonCode::Destructive => 45,
        }
    }

    #[test]
    fn all_lists_every_code_once_in_order() {
        for (i, c) in ReasonCode::ALL.iter().enumerate() {
            assert_eq!(index(*c), i, "{c:?}");
        }
    }
}
