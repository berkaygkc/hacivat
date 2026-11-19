//! The payload types of [`crate::view::CatalogView`] (M5 Task 1a's names, R-M4-27; field lists
//! A-1/A-2). Names are exact server text (`Name`), never folded; expressions and type texts are
//! raw, as the server printed them.

use std::sync::Arc;

/// An object name exactly as the server reported it.
pub type Name = Arc<str>;

/// One schema (PostgreSQL schema, MySQL/MariaDB database, SQLite `main`/`temp`/attached).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaInfo {
    pub name: Name,
    pub is_system: bool,
    pub comment: Option<Name>,
}

/// A relation by its exact schema and name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelRef {
    pub schema: Name,
    pub name: Name,
}

/// The kind of a relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelKind {
    Table,
    View,
    MaterializedView,
    Foreign,
    Partitioned,
}

/// One relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelInfo {
    pub name: RelRef,
    pub kind: RelKind,
    pub temp: bool,
    pub comment: Option<Name>,
}

/// A column's identity kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdentityKind {
    Always,
    ByDefault,
    AutoIncrement,
    RowidAlias,
}

/// A generated column's storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeneratedKind {
    Stored,
    Virtual,
}

/// One column of a relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: Name,
    /// The server's position number (PostgreSQL `attnum`, with gaps after `DROP COLUMN`).
    pub ordinal: u32,
    /// The declared type, raw.
    pub type_name: Name,
    pub nullable: bool,
    pub default_raw: Option<Name>,
    pub identity: Option<IdentityKind>,
    pub generated: Option<GeneratedKind>,
    pub collation: Option<Name>,
    pub comment: Option<Name>,
    pub in_pk: bool,
    pub in_fk: bool,
}

/// How an argument is passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArgMode {
    In,
    Out,
    InOut,
    Variadic,
    Table,
}

/// One routine argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnArg {
    pub name: Option<Name>,
    pub type_name: Name,
    pub mode: ArgMode,
    pub has_default: bool,
}

impl FnArg {
    /// An `IN` argument without a default.
    pub fn input(name: Option<Name>, type_name: Name) -> Self {
        FnArg {
            name,
            type_name,
            mode: ArgMode::In,
            has_default: false,
        }
    }
}

/// What kind of routine a signature belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FnKind {
    Function,
    Aggregate,
    Window,
    Procedure,
}

/// A routine's volatility (PostgreSQL), `Unknown` where the server does not say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Volatility {
    Unknown,
    Immutable,
    Stable,
    Volatile,
}

/// One routine signature (X-8, M4 review I5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnSig {
    pub schema: Name,
    pub name: Name,
    /// `None`: the name only is known (MySQL/MariaDB built-ins from the snapshots), which is not
    /// "takes no arguments" (`Some(vec![])`).
    pub args: Option<Vec<FnArg>>,
    /// The return type, raw; `None` for procedures and name-only signatures.
    pub returns: Option<Name>,
    pub returns_set: bool,
    pub kind: FnKind,
    pub volatility: Volatility,
    pub language: Option<Name>,
}

impl FnSig {
    /// A signature of which only the name is known.
    pub fn name_only(schema: Name, name: Name, kind: FnKind) -> Self {
        FnSig {
            schema,
            name,
            args: None,
            returns: None,
            returns_set: false,
            kind,
            volatility: Volatility::Unknown,
            language: None,
        }
    }

    /// Whether only the name is known.
    pub fn is_name_only(&self) -> bool {
        self.args.is_none()
    }
}

/// The body of a type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeKind {
    /// Labels in sort order (PostgreSQL enums, MySQL `ENUM`/`SET` column types).
    Enum {
        labels: Arc<[Name]>,
    },
    Domain {
        base: Name,
    },
    Composite,
    Range {
        subtype: Name,
    },
    Base,
}

/// One type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeInfo {
    pub name: Name,
    pub kind: TypeKind,
}

/// One foreign key, real or virtual; columns in FK order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FkEdge {
    pub from: RelRef,
    pub from_cols: Arc<[Name]>,
    pub to: RelRef,
    pub to_cols: Arc<[Name]>,
    pub name: Option<Name>,
    pub is_virtual: bool,
}

/// One server setting (PostgreSQL GUC, MySQL/MariaDB system variable) by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingInfo {
    pub name: Name,
    pub type_name: Option<Name>,
    pub context: Option<Name>,
}
