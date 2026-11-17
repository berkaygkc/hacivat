//! The generation target and the generator options (alg/03 HV-DDL-10, HV-DDL-11 v1 core
//! subset; plan-m4 Task 1a `GenOptions`, `DdlTarget`).
//!
//! Every option declares where it applies: categories, dialects and, where the server grew
//! the clause later, a minimum version. An option that does not apply is ignored by the
//! producers (and hidden by the DDL view), never turned into invalid SQL.

use hacivat_meta::{Name, ObjectKind};
use sql_dialect::{DialectId, ServerVersion};

use crate::quote::{LiteralCtx, QuoteCtx};

/// Where generated DDL goes and what the server there accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdlTarget {
    pub dialect: DialectId,
    /// The server version; `None` renders for the newest line Hacivat knows.
    pub version: Option<ServerVersion>,
    /// The session's current schema (PostgreSQL: the first `search_path` entry; MySQL: the
    /// current database). `Qualify::Auto` keeps names in it bare.
    pub current_schema: Option<Name>,
    /// PostgreSQL `standard_conforming_strings` (default on).
    pub pg_standard_conforming_strings: bool,
    /// MySQL / MariaDB `NO_BACKSLASH_ESCAPES` in the session `sql_mode` (default off).
    pub mysql_no_backslash_escapes: bool,
}

impl DdlTarget {
    /// A target of `dialect` with the servers' default settings and no current schema.
    pub fn new(dialect: DialectId) -> Self {
        DdlTarget {
            dialect,
            version: None,
            current_schema: None,
            pg_standard_conforming_strings: true,
            mysql_no_backslash_escapes: false,
        }
    }

    pub fn with_version(mut self, v: ServerVersion) -> Self {
        self.version = Some(v);
        self
    }

    pub fn with_current_schema(mut self, s: impl Into<Name>) -> Self {
        self.current_schema = Some(s.into());
        self
    }

    pub fn quote_ctx(&self) -> QuoteCtx {
        QuoteCtx::new(self.dialect, self.version)
    }

    pub fn literal_ctx(&self) -> LiteralCtx {
        LiteralCtx {
            dialect: self.dialect,
            pg_standard_conforming_strings: self.pg_standard_conforming_strings,
            mysql_no_backslash_escapes: self.mysql_no_backslash_escapes,
        }
    }

    /// Whether the target is at least `major.minor` (an unknown version counts as new).
    pub fn at_least(&self, major: u16, minor: u16) -> bool {
        self.version
            .is_none_or(|v| v >= ServerVersion::new(major, minor, 0))
    }
}

/// HV-DDL-10 schema qualification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Qualify {
    /// Bare when the object is in the current schema and its name is unambiguous in scope.
    #[default]
    Auto,
    Never,
    Always,
}

/// HV-DDL-09 constraint placement. Hacivat's default is `InsideTable` (R-M4-23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    InsideColumn,
    #[default]
    InsideTable,
    AfterTable,
}

/// The v1 core subset of HV-DDL-11 (defaults: alg/03 HV-DDL-11 "Decision", R-M4-23).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GenOptions {
    pub qualify: Qualify,
    pub placement: Placement,
    /// `CREATE OR REPLACE` where the dialect has it for that kind ([`or_replace_supported`]).
    pub or_replace: bool,
    /// `IF NOT EXISTS` where the dialect has it for that kind; `or_replace` wins where both
    /// apply.
    pub if_not_exists: bool,
    pub skip_grants: bool,
    pub skip_owner: bool,
    /// MySQL / MariaDB: drop the `DEFINER = …` clause from server text.
    pub skip_definer: bool,
    /// Emit server-generated constraint names too (`t_pkey`); off: they are left to the server.
    pub surrogate_names: bool,
    /// "Definitions provided by server": server text for tables where the engine has one.
    pub server_definitions: bool,
}

/// One option of [`GenOptions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OptionId {
    Qualify,
    Placement,
    OrReplace,
    IfNotExists,
    SkipGrants,
    SkipOwner,
    SkipDefiner,
    SurrogateNames,
    ServerDefinitions,
}

/// A generator category (alg/03 HV-DDL-11). M4 uses the first two; the others are M7's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    CreateScript,
    DefinitionsByServer,
    Drop,
    Truncate,
    Alter,
}

/// Which dialects an option applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialects {
    All,
    Pg,
    MySqlFamily,
    PgAndMySqlFamily,
}

impl Dialects {
    fn contains(self, d: DialectId) -> bool {
        let pg = matches!(d, DialectId::Postgres | DialectId::PgWire(_));
        let my = matches!(d, DialectId::MySql | DialectId::MariaDb);
        match self {
            Dialects::All => true,
            Dialects::Pg => pg,
            Dialects::MySqlFamily => my,
            Dialects::PgAndMySqlFamily => pg || my,
        }
    }
}

/// An option's applicability (alg/03 HV-DDL-11 `OptionSpec`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionSpec {
    pub id: OptionId,
    pub categories: &'static [Category],
    pub dialects: Dialects,
}

const CREATE: &[Category] = &[Category::CreateScript];
const CREATE_AND_SERVER: &[Category] = &[Category::CreateScript, Category::DefinitionsByServer];

/// Every option of the v1 core subset.
pub const SPECS: [OptionSpec; 9] = [
    OptionSpec {
        id: OptionId::Qualify,
        categories: CREATE,
        dialects: Dialects::All,
    },
    OptionSpec {
        id: OptionId::Placement,
        categories: CREATE,
        dialects: Dialects::All,
    },
    OptionSpec {
        id: OptionId::OrReplace,
        categories: CREATE_AND_SERVER,
        dialects: Dialects::All,
    },
    OptionSpec {
        id: OptionId::IfNotExists,
        categories: CREATE,
        dialects: Dialects::All,
    },
    // Grants and owners: PostgreSQL and MySQL/MariaDB grants; SQLite has neither.
    OptionSpec {
        id: OptionId::SkipGrants,
        categories: CREATE,
        dialects: Dialects::PgAndMySqlFamily,
    },
    OptionSpec {
        id: OptionId::SkipOwner,
        categories: CREATE,
        dialects: Dialects::Pg,
    },
    OptionSpec {
        id: OptionId::SkipDefiner,
        categories: CREATE_AND_SERVER,
        dialects: Dialects::MySqlFamily,
    },
    OptionSpec {
        id: OptionId::SurrogateNames,
        categories: CREATE,
        dialects: Dialects::All,
    },
    // PostgreSQL has no table definition function (R-M4-24): the toggle is hidden there.
    OptionSpec {
        id: OptionId::ServerDefinitions,
        categories: &[Category::CreateScript],
        dialects: Dialects::MySqlFamily,
    },
];

/// The spec of `id`.
pub fn spec(id: OptionId) -> &'static OptionSpec {
    // `SPECS` is in `OptionId` declaration order (`specs_follow_the_option_order`).
    &SPECS[id as usize]
}

/// Whether `spec` is shown for `category` on `target` (HV-DDL-11 step 1).
pub fn applicable(spec: &OptionSpec, category: Category, target: &DdlTarget) -> bool {
    spec.categories.contains(&category) && spec.dialects.contains(target.dialect)
}

/// Whether `CREATE OR REPLACE` exists for `kind` on `target` (alg/03 HV-DDL-11 matrix):
/// PostgreSQL views, functions, procedures and (14+) triggers, never tables or materialized
/// views; MySQL views; MariaDB views, functions, procedures, triggers and events; never SQLite.
pub fn or_replace_supported(target: &DdlTarget, kind: ObjectKind) -> bool {
    use ObjectKind::*;
    match target.dialect {
        DialectId::Postgres | DialectId::PgWire(_) => match kind {
            View | Routine | Procedure => true,
            Trigger => target.at_least(14, 0),
            _ => false,
        },
        DialectId::MySql => kind == View,
        DialectId::MariaDb => matches!(kind, View | Routine | Procedure | Trigger | Event),
        _ => false,
    }
}

/// Whether `CREATE … IF NOT EXISTS` exists for `kind` on `target`, for the statements
/// Hacivat synthesizes (server text is never edited for it).
pub fn if_not_exists_supported(target: &DdlTarget, kind: ObjectKind) -> bool {
    use ObjectKind::*;
    match target.dialect {
        DialectId::Postgres | DialectId::PgWire(_) => {
            matches!(kind, Schema | Table | Index | Sequence | MaterializedView)
        }
        DialectId::MySql => matches!(kind, Schema | Table),
        DialectId::MariaDb => matches!(kind, Schema | Table | Index | Sequence),
        DialectId::Sqlite => matches!(kind, Table | Index),
        _ => false,
    }
}
