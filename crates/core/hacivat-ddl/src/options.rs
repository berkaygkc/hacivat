//! Generator options (HV-DDL-11 v1 core subset; R-M4-23 defaults). Defaults favour safety over
//! DataGrip's: nothing is replaced, nothing is skipped, constraints inside the table,
//! qualification `Auto` (display; executed scripts are always qualified, HV-DDL-10).
//!
//! Every option declares where it applies: categories and dialects, and, where a server grew
//! the clause later, a minimum version ([`or_replace_supported`]). An option that does not
//! apply is ignored by renderers and producers (and hidden by the DDL view), never turned into
//! invalid SQL. M4 Task 7 bodies (spike origin `da358e4`).

use hacivat_meta::kind::ObjectKind;
use sql_dialect::{DialectId, ServerVersion};

use crate::DdlTarget;

/// Qualification and placement are the DDL view's remembered options too, so they are
/// declared once in `hacivat_meta::userdata`.
pub use hacivat_meta::userdata::{Placement, Qualify};

/// The typed option set of one generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenOptions {
    pub qualify: Qualify,
    pub placement: Placement,
    pub or_replace: bool,
    pub if_not_exists: bool,
    pub skip_grants: bool,
    pub skip_owner: bool,
    /// MySQL/MariaDB only.
    pub skip_definer: bool,
    pub surrogate_names: bool,
    /// "Definitions provided by server" (R-M4-24).
    pub server_definitions: bool,
}

impl Default for GenOptions {
    fn default() -> Self {
        GenOptions {
            qualify: Qualify::Auto,
            placement: Placement::InsideTable,
            or_replace: false,
            if_not_exists: false,
            skip_grants: false,
            skip_owner: false,
            skip_definer: false,
            surrogate_names: false,
            server_definitions: false,
        }
    }
}

/// One option of [`GenOptions`], in declaration order.
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
    MySqlFamilyAndSqlite,
}

impl Dialects {
    fn contains(self, d: DialectId) -> bool {
        let pg = is_pg(d);
        let my = is_mysql(d);
        match self {
            Dialects::All => true,
            Dialects::Pg => pg,
            Dialects::MySqlFamily => my,
            Dialects::PgAndMySqlFamily => pg || my,
            Dialects::MySqlFamilyAndSqlite => my || d == DialectId::Sqlite,
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

/// Every option of the v1 core subset, in [`OptionId`] order.
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
    // Grants: PostgreSQL and MySQL/MariaDB; SQLite has none.
    OptionSpec {
        id: OptionId::SkipGrants,
        categories: CREATE,
        dialects: Dialects::PgAndMySqlFamily,
    },
    // Owners: PostgreSQL only (MySQL definers are `SkipDefiner`).
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
    // PostgreSQL has no table definition function, so the toggle is hidden there (R-M4-24).
    OptionSpec {
        id: OptionId::ServerDefinitions,
        categories: CREATE,
        dialects: Dialects::MySqlFamilyAndSqlite,
    },
];

/// The spec of `id`.
pub fn spec(id: OptionId) -> &'static OptionSpec {
    // `SPECS` is in `OptionId` declaration order (test `specs_follow_the_option_order`).
    &SPECS[id as usize]
}

/// Whether `spec` is shown for `category` on `target` (HV-DDL-11 step 1).
pub fn applicable(spec: &OptionSpec, category: Category, target: &DdlTarget) -> bool {
    spec.categories.contains(&category) && spec.dialects.contains(target.dialect)
}

fn is_pg(d: DialectId) -> bool {
    matches!(d, DialectId::Postgres | DialectId::PgWire(_))
}

fn is_mysql(d: DialectId) -> bool {
    matches!(d, DialectId::MySql | DialectId::MariaDb)
}

/// Whether the target server is at least `major.minor` (an unknown version, `0.0.0`, is not).
fn at_least(target: &DdlTarget, major: u16, minor: u16) -> bool {
    target.version >= ServerVersion::new(major, minor, 0)
}

/// Whether `CREATE OR REPLACE` exists for `kind` on `target` (alg/03 HV-DDL-11 matrix):
/// PostgreSQL views, functions, procedures and (14+) triggers, never tables or materialized
/// views; MySQL views; MariaDB views, functions, procedures, triggers and events; never SQLite.
pub fn or_replace_supported(target: &DdlTarget, kind: ObjectKind) -> bool {
    use ObjectKind::*;
    match target.dialect {
        d if is_pg(d) => match kind {
            View | Routine | Procedure => true,
            Trigger => at_least(target, 14, 0),
            _ => false,
        },
        DialectId::MySql => kind == View,
        DialectId::MariaDb => matches!(kind, View | Routine | Procedure | Trigger | Event),
        _ => false,
    }
}

/// Whether `CREATE … IF NOT EXISTS` exists for `kind` on `target`, for the statements Hacivat
/// synthesizes (server text is never edited for it).
pub fn if_not_exists_supported(target: &DdlTarget, kind: ObjectKind) -> bool {
    use ObjectKind::*;
    match target.dialect {
        d if is_pg(d) => matches!(kind, Schema | Table | Index | Sequence | MaterializedView),
        DialectId::MySql => matches!(kind, Schema | Table),
        DialectId::MariaDb => matches!(kind, Schema | Table | Index | Sequence),
        DialectId::Sqlite => matches!(kind, Table | Index),
        _ => false,
    }
}
