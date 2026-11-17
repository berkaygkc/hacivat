//! Errors and notices. Codes only: every text the user reads is a Fluent message (`ddl-*`)
//! looked up at the edge; generated SQL never comes from Fluent.

use std::fmt;

use hacivat_meta::{ObjectKey, ObjectKind};
use sql_dialect::DialectId;

/// A renderer could not turn an op into text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// The op (or this form of it) is not rendered for this dialect; M7 fills the rest.
    Unsupported {
        op: &'static str,
        dialect: DialectId,
    },
    /// A text that would go into a string literal holds a NUL.
    Nul { what: &'static str },
    /// The op is malformed (an empty column list, a constraint that cannot be placed).
    Invalid(&'static str),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::Unsupported { op, dialect } => {
                write!(f, "{op} is not rendered for {dialect:?}")
            }
            RenderError::Nul { what } => write!(f, "NUL in {what}"),
            RenderError::Invalid(why) => write!(f, "invalid op: {why}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// A producer could not build a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DdlError {
    /// The object is not in the snapshot.
    NotFound(Box<ObjectKey>),
    /// The kind has no DDL producer for this dialect (`ObjectKind::Unknown` always,
    /// alg/02 HV-MODEL-01 edge 11).
    Unsupported {
        kind: ObjectKind,
        dialect: DialectId,
    },
    /// A name was stored lossily (`ObjFlags::LOSSY_NAME`): DDL from it would name another
    /// object.
    LossyName(Box<ObjectKey>),
    /// The object's source text is not loaded (the caller loads `Sources` level and retries).
    SourceNotLoaded(Box<ObjectKey>),
    /// A props variant the kind does not have (a corrupt or foreign model).
    BadProps(Box<ObjectKey>),
    Render(RenderError),
}

impl DdlError {
    pub fn not_found(k: ObjectKey) -> Self {
        DdlError::NotFound(Box::new(k))
    }

    pub fn lossy_name(k: ObjectKey) -> Self {
        DdlError::LossyName(Box::new(k))
    }

    pub fn source_not_loaded(k: ObjectKey) -> Self {
        DdlError::SourceNotLoaded(Box::new(k))
    }

    pub fn bad_props(k: ObjectKey) -> Self {
        DdlError::BadProps(Box::new(k))
    }
}

impl From<RenderError> for DdlError {
    fn from(e: RenderError) -> Self {
        DdlError::Render(e)
    }
}

impl fmt::Display for DdlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DdlError::NotFound(k) => write!(f, "object not found: {:?} {:?}", k.kind, k.path),
            DdlError::Unsupported { kind, dialect } => {
                write!(f, "no DDL producer for {kind:?} on {dialect:?}")
            }
            DdlError::LossyName(k) => write!(f, "lossy name: {:?}", k.path),
            DdlError::SourceNotLoaded(k) => write!(f, "source not loaded: {:?}", k.path),
            DdlError::BadProps(k) => write!(f, "unexpected properties: {:?}", k.path),
            DdlError::Render(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for DdlError {}

/// What a script wants the user to know (shown next to the DDL, never inside it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NoticeCode {
    /// SQLite constraints can only live inside the table (HV-DDL-09).
    SqlitePlacementForced,
    /// MySQL server text holds an executable comment `/*! … */` (it runs).
    WarnExecutableComment,
    /// An object of a kind without a producer was left out.
    UnsupportedKind,
    /// The grants facet is not loaded: the script has no `GRANT` statements (A-3).
    GrantsNotLoaded,
    /// The login could not read every grant: the `GRANT` statements may be incomplete.
    GrantsPartial,
    /// A requested edit of server text (skip `DEFINER`, `OR REPLACE`) was not made because
    /// the statement head was not the expected shape; the text is unchanged.
    HeadNotEdited,
    /// "Definitions provided by server" was asked for, but the model has no server text for
    /// this table: the definition is synthesized.
    ServerDefinitionUnavailable,
}

impl NoticeCode {
    /// The Fluent message id.
    pub const fn fluent_id(self) -> &'static str {
        match self {
            NoticeCode::SqlitePlacementForced => "ddl-sqlite-placement-forced",
            NoticeCode::WarnExecutableComment => "ddl-warn-executable-comment",
            NoticeCode::UnsupportedKind => "ddl-unsupported-kind",
            NoticeCode::GrantsNotLoaded => "ddl-grants-not-loaded",
            NoticeCode::GrantsPartial => "ddl-grants-partial",
            NoticeCode::HeadNotEdited => "ddl-head-not-edited",
            NoticeCode::ServerDefinitionUnavailable => "ddl-server-definition-unavailable",
        }
    }
}

/// One notice, about one object when it names one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub code: NoticeCode,
    pub kind: Option<ObjectKind>,
    pub object: Option<ObjectKey>,
}
