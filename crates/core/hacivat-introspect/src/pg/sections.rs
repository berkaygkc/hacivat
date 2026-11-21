// ## m4-t16-pg
//! Running the `packs/pg` sections on a snapshot: render (the pack's text only, every name and
//! oid a bind, R-M4-3, R-M4-7), run through `SnapshotTxn::rows`, check the server's columns
//! against the section's declared `returns`, decode, and record what ran (the pass report and
//! the Docker tests' query log).
//!
//! A server error `XX000` ("cache lookup failed": an object dropped while a deparser read it,
//! PG spike finding 8) is [`IntroError::CatalogRace`], which Task 15 retries once; a
//! permission error `42501` is [`IntroError::Permission`]; every other server error is
//! [`IntroError::Meta`].

use std::time::{Duration, Instant};

use hacivat_meta::{Level, Name, ObjectKind};
use hacivat_session::meta::{MetaError, SnapshotTxn};
use hacivat_values::{BindValue, LogicalType};
use sql_dialect::DialectId;
use tokio_util::sync::CancellationToken;

use super::decode::Row;
use crate::engine::Phase;
use crate::filter::SqlPredicate;
use crate::introspector::IntroError;
use crate::pack::{Mode, Pack, PackCtx, PackFlags, Params};

/// The section names of `packs/pg` (the pack files are the source of truth; these are the
/// names the introspector asks for).
pub mod name {
    pub const IDENTITY: &str = "Identity";
    pub const LIST_NAMESPACES: &str = "ListNamespaces";
    pub const LIST_DATABASES: &str = "ListDatabases";
    pub const RETRIEVE_NAMESPACE: &str = "RetrieveNamespace";
    pub const LIST_EXISTENT: &str = "ListExistent";
    pub const LIST_EXISTENT_COMMENTS: &str = "ListExistentComments";
    pub const CHANGED_PARENTS: &str = "ChangedParents";
    pub const CHANGED_DEPENDENTS: &str = "ChangedDependents";
    pub const RETRIEVE_RELATIONS: &str = "RetrieveRelations";
    pub const RETRIEVE_COLUMNS: &str = "RetrieveColumns";
    pub const RETRIEVE_INDEXES: &str = "RetrieveIndexes";
    pub const RETRIEVE_CONSTRAINTS: &str = "RetrieveConstraints";
    pub const RETRIEVE_TRIGGERS: &str = "RetrieveTriggers";
    pub const RETRIEVE_SEQUENCES: &str = "RetrieveSequences";
    pub const RETRIEVE_ROUTINES: &str = "RetrieveRoutines";
    pub const RETRIEVE_TYPES: &str = "RetrieveTypes";
    pub const RETRIEVE_TYPE_ATTRIBUTES: &str = "RetrieveTypeAttributes";
    pub const RETRIEVE_DOMAIN_CONSTRAINTS: &str = "RetrieveDomainConstraints";
    pub const COMMENTS: &str = "Comments";
    pub const CANARY: &str = "Canary";
    pub const STATE_OF_RELATION: &str = "StateOfRelation";
    pub const STATE_OF_ROUTINE: &str = "StateOfRoutine";
    pub const STATE_OF_TYPE: &str = "StateOfType";
    pub const STATE_OF_CONSTRAINT: &str = "StateOfConstraint";
    pub const STATE_OF_TRIGGER: &str = "StateOfTrigger";
    pub const SYSTEM_RELATIONS: &str = "SystemRelations";
    pub const SYSTEM_COLUMNS: &str = "SystemColumns";
    pub const SYSTEM_ROUTINES: &str = "SystemRoutines";
    pub const SYSTEM_OPERATORS: &str = "SystemOperators";
    pub const SYSTEM_TYPES: &str = "SystemTypes";
    pub const LIST_SETTINGS: &str = "ListSettings";
}

/// The PostgreSQL pack (parsed once per process).
pub fn pack() -> Result<Pack, IntroError> {
    Pack::builtin(DialectId::Postgres).map_err(|e| {
        IntroError::Protocol(format!("pg/pack: {:?} at {}:{}", e.kind, e.file, e.line).into())
    })
}

/// A text bind (names, oids and the watermark all travel as text parameters; the server
/// parses each with the parameter's inferred type).
pub fn text_bind(s: impl Into<Name>) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: s.into(),
    }
}

/// An oid bind.
pub fn oid_bind(oid: u32) -> BindValue {
    text_bind(oid.to_string())
}

/// The binds of one rendering.
#[derive(Debug, Clone, Default)]
pub struct Binds {
    pub ns: Option<u32>,
    pub w: Option<u32>,
    pub schema: Option<Name>,
    pub oids: Option<Vec<u32>>,
}

impl Binds {
    pub fn ns(ns: u32) -> Self {
        Binds {
            ns: Some(ns),
            ..Binds::default()
        }
    }

    pub fn with_oids(mut self, oids: Vec<u32>) -> Self {
        self.oids = Some(oids);
        self
    }

    pub fn with_w(mut self, w: u32) -> Self {
        self.w = Some(w);
        self
    }

    fn params(&self) -> Params {
        let mut p = Params::default();
        if let Some(ns) = self.ns {
            p.scalars.push((Name::from("ns"), oid_bind(ns)));
        }
        if let Some(w) = self.w {
            p.scalars.push((Name::from("w"), oid_bind(w)));
        }
        if let Some(s) = &self.schema {
            p.scalars.push((Name::from("schema"), text_bind(s.clone())));
        }
        if let Some(oids) = &self.oids {
            p.lists.push((
                Name::from("oids"),
                oids.iter().copied().map(oid_bind).collect(),
            ));
        }
        p
    }
}

/// The context a section is rendered for (`server_version_num`, the pass's mode and level).
pub fn ctx(version: u32, mode: Mode, level: Level, binds: &Binds) -> PackCtx {
    ctx_like(version, mode, level, binds, None)
}

/// [`ctx`] with one pushed-down name pattern (R-W3-2): `like` is one run of
/// `engine::bounded_pushdown`'s `SqlPredicate` (`relname LIKE :name_like ESCAPE
/// :like_escape`, the `C` collation of `name`); `None` binds SQL NULL, which the sections read
/// as "no predicate".
pub fn ctx_like(
    version: u32,
    mode: Mode,
    level: Level,
    binds: &Binds,
    like: Option<&BindValue>,
) -> PackCtx {
    let mut params = binds.params();
    params.scalars.push((
        Name::from(SqlPredicate::PATTERN_BIND),
        like.cloned().unwrap_or(BindValue::Null),
    ));
    params.scalars.push((
        Name::from(SqlPredicate::ESCAPE_BIND),
        if like.is_some() {
            SqlPredicate::escape()
        } else {
            BindValue::Null
        },
    ));
    PackCtx {
        dialect: DialectId::Postgres,
        version,
        flavor: hacivat_proto::DbmsKind::Postgres,
        mode,
        level,
        flags: PackFlags::empty(),
        params,
    }
}

/// The runs of a section that takes the relation-name pushdown: one per pattern, or one run
/// without a predicate.
pub fn like_runs(pushed: Option<&SqlPredicate>) -> Vec<Option<BindValue>> {
    match pushed {
        Some(p) if !p.binds.is_empty() => p.binds.iter().cloned().map(Some).collect(),
        _ => vec![None],
    }
}

/// What one section cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionStat {
    pub name: &'static str,
    /// Statements sent (a chunked list bind sends several; an empty list none).
    pub queries: usize,
    pub rows: usize,
    pub elapsed: Duration,
}

/// The sections a pass ran, in order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryLog {
    pub sections: Vec<SectionStat>,
}

impl QueryLog {
    /// Whether a section of this name sent a statement.
    pub fn ran(&self, name: &str) -> bool {
        self.sections
            .iter()
            .any(|s| s.name == name && s.queries > 0)
    }

    /// The names of the sections that sent a statement, in order.
    pub fn names(&self) -> Vec<&'static str> {
        self.sections
            .iter()
            .filter(|s| s.queries > 0)
            .map(|s| s.name)
            .collect()
    }
}

/// Reads sections inside one snapshot.
pub struct Reader<'r, 'c> {
    pub txn: &'r mut SnapshotTxn<'c>,
    pub pack: Pack,
    pub version: u32,
    pub cancel: &'r CancellationToken,
    pub log: &'r mut QueryLog,
    /// Told the phase and the row count of every section as it finishes.
    pub progress: Option<ProgressSink>,
}

/// A pass's progress sink: the phase of each finished section and the rows it read (the
/// engine's closed `Phase` set; determinate counts, no ambient motion).
pub type ProgressSink = std::sync::Arc<dyn Fn(Phase, u64) + Send + Sync>;

/// The engine phase a section belongs to.
pub fn phase_of(section: &str) -> Phase {
    match section {
        name::LIST_NAMESPACES | name::LIST_DATABASES => Phase::ListNamespaces,
        name::RETRIEVE_RELATIONS | name::RETRIEVE_NAMESPACE | name::SYSTEM_RELATIONS => {
            Phase::Tables
        }
        name::RETRIEVE_COLUMNS | name::SYSTEM_COLUMNS => Phase::Columns,
        name::RETRIEVE_INDEXES => Phase::Indexes,
        name::RETRIEVE_CONSTRAINTS => Phase::Constraints,
        name::RETRIEVE_TRIGGERS => Phase::Triggers,
        name::RETRIEVE_ROUTINES | name::SYSTEM_ROUTINES | name::SYSTEM_OPERATORS => Phase::Routines,
        name::RETRIEVE_TYPES
        | name::RETRIEVE_TYPE_ATTRIBUTES
        | name::RETRIEVE_DOMAIN_CONSTRAINTS
        | name::SYSTEM_TYPES => Phase::Types,
        name::RETRIEVE_SEQUENCES => Phase::Sequences,
        name::COMMENTS => Phase::Comments,
        name::LIST_EXISTENT | name::LIST_EXISTENT_COMMENTS => Phase::Existence,
        _ => Phase::Validate,
    }
}

impl Reader<'_, '_> {
    /// Renders `section` for `mode`/`level` with `binds`, runs every chunk and decodes each
    /// row with `f`.
    pub async fn read<T>(
        &mut self,
        section: &'static str,
        mode: Mode,
        level: Level,
        binds: &Binds,
        f: impl Fn(&Row<'_>) -> Result<T, IntroError>,
    ) -> Result<Vec<T>, IntroError> {
        self.read_like(section, mode, level, binds, &[None], f)
            .await
    }

    /// [`Reader::read`] once per pushed-down pattern run ([`like_runs`]), the rows of every
    /// run concatenated (the caller deduplicates by oid where runs overlap).
    pub async fn read_like<T>(
        &mut self,
        section: &'static str,
        mode: Mode,
        level: Level,
        binds: &Binds,
        runs: &[Option<BindValue>],
        f: impl Fn(&Row<'_>) -> Result<T, IntroError>,
    ) -> Result<Vec<T>, IntroError> {
        let mut out = Vec::new();
        for like in runs {
            out.extend(
                self.read_one(section, mode, level, binds, like.as_ref(), &f)
                    .await?,
            );
        }
        if let Some(p) = self.progress.as_ref() {
            p(phase_of(section), out.len() as u64);
        }
        Ok(out)
    }

    async fn read_one<T>(
        &mut self,
        section: &'static str,
        mode: Mode,
        level: Level,
        binds: &Binds,
        like: Option<&BindValue>,
        f: &impl Fn(&Row<'_>) -> Result<T, IntroError>,
    ) -> Result<Vec<T>, IntroError> {
        if self.cancel.is_cancelled() {
            return Err(IntroError::Cancelled);
        }
        let t0 = Instant::now();
        let ctx = ctx_like(self.version, mode, level, binds, like);
        let queries = self
            .pack
            .render(section, &ctx)
            .map_err(|e| IntroError::Protocol(format!("pg/{section}: {e:?}").into()))?;
        let declared = self.pack.section(section).map_or(0, |s| s.returns().len());
        let mut out = Vec::new();
        let mut rows = 0;
        for q in &queries {
            let got = tokio::select! {
                biased;
                () = self.cancel.cancelled() => return Err(IntroError::Cancelled),
                got = self.txn.rows(q.as_meta_query()) => got,
            };
            let got = got.map_err(|e| error(e, section))?;
            if got.columns.len() != declared {
                return Err(IntroError::Protocol(
                    format!(
                        "pg/{section}: {} columns, {declared} declared",
                        got.columns.len()
                    )
                    .into(),
                ));
            }
            rows += got.rows.len();
            for cells in &got.rows {
                out.push(f(&Row { section, cells })?);
            }
        }
        self.log.sections.push(SectionStat {
            name: section,
            queries: queries.len(),
            rows,
            elapsed: t0.elapsed(),
        });
        Ok(out)
    }
}

/// The kind a permission error is reported for, per section.
fn kind_of(section: &str) -> ObjectKind {
    match section {
        name::RETRIEVE_ROUTINES | name::STATE_OF_ROUTINE => ObjectKind::Routine,
        name::RETRIEVE_SEQUENCES => ObjectKind::Sequence,
        name::RETRIEVE_TYPES | name::RETRIEVE_TYPE_ATTRIBUTES | name::STATE_OF_TYPE => {
            ObjectKind::Domain
        }
        name::RETRIEVE_TRIGGERS | name::STATE_OF_TRIGGER => ObjectKind::Trigger,
        name::RETRIEVE_INDEXES => ObjectKind::Index,
        name::RETRIEVE_CONSTRAINTS | name::STATE_OF_CONSTRAINT => ObjectKind::Check,
        name::RETRIEVE_COLUMNS => ObjectKind::Column,
        _ => ObjectKind::Table,
    }
}

/// A metadata error as an introspection error (module docs).
pub fn error(e: MetaError, section: &str) -> IntroError {
    match &e {
        MetaError::Server { sqlstate, .. } => match sqlstate.as_deref() {
            Some("XX000") => IntroError::CatalogRace,
            Some("42501") => IntroError::Permission {
                kind: kind_of(section),
            },
            _ => IntroError::Meta(e),
        },
        MetaError::Timeout => IntroError::Timeout,
        MetaError::Cancelled => IntroError::Cancelled,
        _ => IntroError::Meta(e),
    }
}
