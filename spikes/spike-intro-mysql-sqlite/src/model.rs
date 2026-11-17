//! The introspectors' output: plain structs local to this spike. The mapping onto the
//! `hacivat-meta` model (HV-MODEL-01: `ObjectKind`, `Obj`, `KindProps`, `StateMark`) is in the
//! spike report ("Mapping note"); nothing here is the final model.
//!
//! Names are kept exactly as the server reports them (HV-MODEL-01, HV-INTRO-14): no folding,
//! no normalization. Every collection is sorted by a byte-wise key (`normalize`) so a full and
//! an incremental pass can be compared with `==`.

/// How much of a namespace a pass loads (HV-INTRO-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    /// Major object names (tables, views, routines) only.
    Names,
    /// Everything but source text.
    Structure,
    /// Structure plus view, routine, trigger and event bodies.
    Sources,
}

/// One namespace as the server lists it (a MySQL database, a SQLite `main`/`temp`/attached).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamespaceInfo {
    pub name: String,
    /// MySQL: `DEFAULT_CHARACTER_SET_NAME`; SQLite: the file path (empty for in-memory/temp).
    pub detail: Option<String>,
    /// MySQL: `DEFAULT_COLLATION_NAME`.
    pub collation: Option<String>,
    /// `mysql`, `information_schema`, `performance_schema`, `sys`; SQLite `temp`.
    pub system: bool,
}

/// One introspected namespace.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchemaSnapshot {
    pub name: String,
    pub level: Option<Level>,
    /// Tables, views, sequences (MariaDB), virtual tables (SQLite), by name.
    pub relations: Vec<Relation>,
    pub routines: Vec<Routine>,
    pub triggers: Vec<Trigger>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RelationKind {
    Table,
    View,
    /// MariaDB `SEQUENCE` (10.3+).
    Sequence,
    /// MariaDB `SYSTEM VERSIONED` base table.
    SystemVersioned,
    /// SQLite `CREATE VIRTUAL TABLE`.
    VirtualTable,
    /// SQLite shadow tables of a virtual table (FTS5 `_data`, …) as `pragma_table_list` says.
    Shadow,
    /// A `TABLE_TYPE` this spike does not know, kept verbatim (HV-MODEL-01 edge 11).
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Relation {
    pub name: String,
    pub kind: Option<RelationKind>,
    /// MySQL `ENGINE`; SQLite the virtual-table module.
    pub engine: Option<String>,
    pub comment: Option<String>,
    pub collation: Option<String>,
    /// MySQL `CREATE_OPTIONS` (row format, partitioned, …).
    pub options: Option<String>,
    /// SQLite `WITHOUT ROWID`.
    pub without_rowid: bool,
    /// SQLite `STRICT` (3.37+).
    pub strict: bool,
    /// The change marker (HV-INTRO-07/08/15): MySQL `CREATE_TIME` (UTC text) for tables, the
    /// definition hash for views; SQLite none (schema-level only).
    pub marker: Option<String>,
    pub view: Option<ViewDef>,
    /// SQLite: the `CREATE …` text from `sqlite_schema` (needed for CHECK, generated
    /// expressions and partial-index predicates, which no pragma reports).
    pub sql: Option<String>,
    pub columns: Vec<Column>,
    pub indexes: Vec<Index>,
    pub constraints: Vec<Constraint>,
    /// The relation's columns could not be read (SQLite: a view over a dropped table, a
    /// virtual table whose module is not loaded). Kept, with no columns (HV-INTRO-13 broken).
    pub broken: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewDef {
    /// `None` at `Structure` level or without `SHOW VIEW` (MySQL shows an empty text then).
    pub definition: Option<String>,
    pub check_option: Option<String>,
    pub updatable: Option<bool>,
    pub security_type: Option<String>,
    pub definer: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeneratedKind {
    Virtual,
    Stored,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Column {
    pub name: String,
    /// 1-based (MySQL `ORDINAL_POSITION`; SQLite `cid + 1`).
    pub ordinal: u32,
    /// The declared type as the server prints it (MySQL `COLUMN_TYPE`, SQLite `type`).
    pub data_type: String,
    pub nullable: bool,
    /// The default expression as the server reports it, never re-quoted (HV-MODEL-01).
    /// MySQL shows a literal unquoted, MariaDB quoted: kept verbatim, `default_quoted` says.
    pub default: Option<String>,
    pub generated: Option<(GeneratedKind, Option<String>)>,
    /// MySQL 8.0.23+ / MariaDB 10.3+ `INVISIBLE`; SQLite hidden = 1 (virtual-table column).
    pub hidden: bool,
    pub auto_increment: bool,
    /// MySQL `on update CURRENT_TIMESTAMP` (from `EXTRA`).
    pub on_update: Option<String>,
    pub charset: Option<String>,
    pub collation: Option<String>,
    pub comment: Option<String>,
    /// SQLite: 1-based position in the primary key (0 = not part of it).
    pub pk_position: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Index {
    pub name: String,
    pub unique: bool,
    pub primary: bool,
    /// MySQL `INDEX_TYPE` (BTREE, HASH, FULLTEXT, SPATIAL).
    pub index_type: Option<String>,
    pub columns: Vec<IndexColumn>,
    pub comment: Option<String>,
    /// MySQL `IS_VISIBLE = NO` / MariaDB `IGNORED = YES` → false.
    pub visible: bool,
    /// SQLite `origin`: `c` (CREATE INDEX), `u` (UNIQUE), `pk`.
    pub origin: Option<String>,
    /// SQLite partial index (predicate in `sql`).
    pub partial: bool,
    /// SQLite `CREATE INDEX` text (`None` for automatic indexes).
    pub sql: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexColumn {
    /// `None` for an expression key part.
    pub name: Option<String>,
    /// MySQL 8.0.13+ functional key part (`STATISTICS.EXPRESSION`); SQLite reports only that
    /// a key part is an expression (`cid = -2`), the text lives in `Index::sql`.
    pub expression: Option<String>,
    pub descending: bool,
    /// MySQL prefix length.
    pub sub_part: Option<u32>,
    pub collation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintKind {
    PrimaryKey,
    Unique,
    ForeignKey(ForeignKey),
    Check { clause: String },
}

impl ConstraintKind {
    fn rank(&self) -> u8 {
        match self {
            ConstraintKind::PrimaryKey => 0,
            ConstraintKind::Unique => 1,
            ConstraintKind::ForeignKey(_) => 2,
            ConstraintKind::Check { .. } => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constraint {
    /// `None` for SQLite's unnamed constraints.
    pub name: Option<String>,
    pub kind: ConstraintKind,
    /// Key columns in key order (PK/UK/FK).
    pub columns: Vec<String>,
    /// MySQL 8.0.16+ `ENFORCED`; true elsewhere.
    pub enforced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ForeignKey {
    pub ref_schema: Option<String>,
    pub ref_table: String,
    /// In key order; SQLite `None` when the FK names no columns (references the PK).
    pub ref_columns: Vec<Option<String>>,
    pub on_update: Option<String>,
    pub on_delete: Option<String>,
    pub match_option: Option<String>,
    /// MySQL `REFERENTIAL_CONSTRAINTS.UNIQUE_CONSTRAINT_NAME` (the parent key).
    pub ref_constraint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RoutineKind {
    Function,
    Procedure,
    /// MariaDB `PACKAGE` / `PACKAGE BODY` (sql_mode=ORACLE).
    Package,
    PackageBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Routine {
    /// MySQL `SPECIFIC_NAME` (no overloading: equal to the name).
    pub specific_name: String,
    pub name: String,
    pub kind: Option<RoutineKind>,
    /// Functions: `DTD_IDENTIFIER` of the result.
    pub returns: Option<String>,
    pub params: Vec<Param>,
    pub deterministic: Option<bool>,
    pub data_access: Option<String>,
    pub security_type: Option<String>,
    pub definer: Option<String>,
    pub comment: Option<String>,
    /// `Sources` level; `None` without the privilege to see it (HV-INTRO-07 edge 12).
    pub definition: Option<String>,
    /// `LAST_ALTERED` (UTC text): the HV-INTRO-07 marker.
    pub marker: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Param {
    pub ordinal: u32,
    pub mode: Option<String>,
    pub name: Option<String>,
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Trigger {
    pub name: String,
    pub table: String,
    pub timing: Option<String>,
    pub event: Option<String>,
    /// MySQL `ACTION_ORDER` (FOLLOWS/PRECEDES).
    pub order: Option<u32>,
    /// `Sources` level: MySQL `ACTION_STATEMENT`, SQLite the `CREATE TRIGGER` text.
    pub body: Option<String>,
    /// MySQL `CREATED` (UTC text, centiseconds): the HV-INTRO-07 marker.
    pub marker: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Event {
    pub name: String,
    pub event_type: Option<String>,
    pub execute_at: Option<String>,
    pub interval_value: Option<String>,
    pub interval_field: Option<String>,
    pub status: Option<String>,
    pub on_completion: Option<String>,
    pub comment: Option<String>,
    pub definer: Option<String>,
    /// `Sources` level.
    pub body: Option<String>,
    /// `LAST_ALTERED` (UTC text).
    pub marker: Option<String>,
}

impl SchemaSnapshot {
    /// Sorts every collection by a byte-wise key, so two passes compare with `==`.
    pub fn normalize(&mut self) {
        self.relations.sort_by(|a, b| a.name.cmp(&b.name));
        for r in &mut self.relations {
            r.normalize();
        }
        self.routines
            .sort_by(|a, b| (a.kind, &a.specific_name).cmp(&(b.kind, &b.specific_name)));
        for r in &mut self.routines {
            r.params.sort_by_key(|p| p.ordinal);
        }
        self.triggers.sort_by(|a, b| a.name.cmp(&b.name));
        self.events.sort_by(|a, b| a.name.cmp(&b.name));
    }

    pub fn relation(&self, name: &str) -> Option<&Relation> {
        self.relations.iter().find(|r| r.name == name)
    }

    pub fn routine(&self, name: &str) -> Option<&Routine> {
        self.routines.iter().find(|r| r.name == name)
    }

    pub fn trigger(&self, name: &str) -> Option<&Trigger> {
        self.triggers.iter().find(|r| r.name == name)
    }

    pub fn event(&self, name: &str) -> Option<&Event> {
        self.events.iter().find(|r| r.name == name)
    }

    /// Columns of every relation.
    pub fn column_count(&self) -> usize {
        self.relations.iter().map(|r| r.columns.len()).sum()
    }
}

impl Relation {
    pub fn normalize(&mut self) {
        self.columns.sort_by_key(|c| c.ordinal);
        self.indexes.sort_by(|a, b| a.name.cmp(&b.name));
        self.constraints.sort_by(|a, b| {
            (a.kind.rank(), &a.name, &a.columns).cmp(&(b.kind.rank(), &b.name, &b.columns))
        });
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    pub fn index(&self, name: &str) -> Option<&Index> {
        self.indexes.iter().find(|i| i.name == name)
    }

    pub fn constraint(&self, name: &str) -> Option<&Constraint> {
        self.constraints
            .iter()
            .find(|c| c.name.as_deref() == Some(name))
    }
}
