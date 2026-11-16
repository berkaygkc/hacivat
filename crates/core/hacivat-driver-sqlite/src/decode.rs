//! SQLite values to `hacivat_values` (HV-RES-04 SQLite, HV-RES-01 SQLite mapping; Task 15):
//! the declared type's affinity sets a column's buffer, a value of another storage class is a
//! whole-value exception (alg/21:147), text that is not UTF-8 is kept as `Lossy` bytes, and the
//! column metadata is read inside `catch_unwind`. Public (hidden) so `tests/typed_hostile.rs`
//! reaches the decoders.
//!
//! **Affinity** ([`affinity_of`]): SQLite's five rules, in order, on the declared type with
//! ASCII case folding: it holds `INT` → INTEGER; `CHAR`, `CLOB` or `TEXT` → TEXT; `BLOB`, or it
//! is empty → BLOB; `REAL`, `FLOA` or `DOUB` → REAL; otherwise NUMERIC.
//!
//! **Buffers** ([`rule_of`]):
//!
//! | declared type | column kind |
//! |---|---|
//! | INTEGER affinity | `Int` (`I64`) |
//! | TEXT affinity | `Text` (`Var(Text)`) |
//! | REAL affinity | `Float` (`F64`) |
//! | `BOOL`, `BOOLEAN` | `Bool` while a value is 0 or 1 |
//! | `DATE`, `DATETIME`, `TIMESTAMP` | `Text`: stored text verbatim |
//! | other NUMERIC (`NUMERIC(10,2)`, `DECIMAL`) | the first value's class; `Int` when none |
//! | BLOB affinity, no declared type, expressions | the first value's class; `Bytes` when none |
//! | a declared type that is not UTF-8 | `Opaque` (whole values) |
//!
//! "The first value's class" is the storage class of the column's first non-NULL value in the
//! first chunk: INTEGER → `Int`, REAL → `Float`, TEXT → `Text`, BLOB → `Bytes`. SQLite has no
//! decimal type, and integers stay exact (an INTEGER value in a REAL column is an `Int`
//! exception, never widened).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use hacivat_values::{
    BufKind, ChunkBuilder, LogicalType, NativeType, OriginHint, SqliteAffinity, Value, VarKind,
};
use rusqlite::Statement;
use rusqlite::types::{Type, ValueRef};

use crate::exec::{UNREADABLE_NAME, column_name};

/// What a column's buffer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColKind {
    Int,
    Float,
    Text,
    Bytes,
    /// 0 and 1 as booleans; any other value is a whole-value exception.
    Bool,
    /// Whole values of any class: the declared type could not be read.
    Opaque,
}

impl ColKind {
    /// The column's buffer.
    pub fn buf(self) -> BufKind {
        match self {
            ColKind::Int => BufKind::I64,
            ColKind::Float => BufKind::F64 { f32: false },
            ColKind::Text => BufKind::Var(VarKind::Text),
            ColKind::Bytes => BufKind::Var(VarKind::Bytes),
            ColKind::Bool => BufKind::Bool,
            ColKind::Opaque => BufKind::Values,
        }
    }

    /// The column's logical type.
    pub fn logical(self) -> LogicalType {
        match self {
            ColKind::Int => LogicalType::Int {
                bits: 64,
                signed: true,
            },
            ColKind::Float => LogicalType::Float { bits: 64 },
            ColKind::Text => LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            ColKind::Bytes => LogicalType::Bytes { max: None },
            ColKind::Bool => LogicalType::Bool,
            ColKind::Opaque => LogicalType::Opaque {
                type_name: UNREADABLE_NAME.into(),
            },
        }
    }

    /// The kind that holds a storage class inline (`None` for NULL).
    pub fn of_class(class: Type) -> Option<ColKind> {
        match class {
            Type::Null => None,
            Type::Integer => Some(ColKind::Int),
            Type::Real => Some(ColKind::Float),
            Type::Text => Some(ColKind::Text),
            Type::Blob => Some(ColKind::Bytes),
        }
    }
}

/// How a declared type sets its column's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rule {
    Fixed(ColKind),
    /// The first non-NULL value of the first chunk decides; `fallback` when there is none.
    FirstValue {
        fallback: ColKind,
    },
}

impl Rule {
    /// The kind once the first chunk's first non-NULL storage class is known.
    pub fn kind(self, first: Option<Type>) -> ColKind {
        match self {
            Rule::Fixed(kind) => kind,
            Rule::FirstValue { fallback } => first.and_then(ColKind::of_class).unwrap_or(fallback),
        }
    }
}

/// Whether `hay` holds `needle` (ASCII), ignoring ASCII case.
fn holds(hay: &str, needle: &str) -> bool {
    hay.as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// The affinity of a declared type: SQLite's rules (§3.1 of its datatype documentation), in
/// order.
pub fn affinity_of(decl: &str) -> SqliteAffinity {
    if holds(decl, "INT") {
        SqliteAffinity::Integer
    } else if ["CHAR", "CLOB", "TEXT"].iter().any(|n| holds(decl, n)) {
        SqliteAffinity::Text
    } else if decl.is_empty() || holds(decl, "BLOB") {
        SqliteAffinity::Blob
    } else if ["REAL", "FLOA", "DOUB"].iter().any(|n| holds(decl, n)) {
        SqliteAffinity::Real
    } else {
        SqliteAffinity::Numeric
    }
}

/// The type name of a declared type: its leading word (`DATETIME` of `DATETIME(6)`).
fn type_word(decl: &str) -> &str {
    let decl = decl.trim_start();
    let end = decl
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(decl.len());
    &decl[..end]
}

/// A declared type's affinity and how it sets its column's kind (`None`: no declared type, an
/// expression column).
pub fn rule_of(decl: Option<&str>) -> (SqliteAffinity, Rule) {
    let Some(decl) = decl else {
        return (
            SqliteAffinity::Blob,
            Rule::FirstValue {
                fallback: ColKind::Bytes,
            },
        );
    };
    let affinity = affinity_of(decl);
    let rule = match affinity {
        SqliteAffinity::Integer => Rule::Fixed(ColKind::Int),
        SqliteAffinity::Text => Rule::Fixed(ColKind::Text),
        SqliteAffinity::Real => Rule::Fixed(ColKind::Float),
        SqliteAffinity::Blob => Rule::FirstValue {
            fallback: ColKind::Bytes,
        },
        SqliteAffinity::Numeric => {
            let word = type_word(decl);
            let is = |names: &[&str]| names.iter().any(|n| word.eq_ignore_ascii_case(n));
            if is(&["BOOL", "BOOLEAN"]) {
                Rule::Fixed(ColKind::Bool)
            } else if is(&["DATE", "DATETIME", "TIMESTAMP"]) {
                Rule::Fixed(ColKind::Text)
            } else {
                Rule::FirstValue {
                    fallback: ColKind::Int,
                }
            }
        }
    };
    (affinity, rule)
}

/// The buffer of a column declared `decl` whose first chunk's first non-NULL value has storage
/// class `first` (`None`: every value was NULL, or there were none).
pub fn buf_kind_of(decl: Option<&str>, first: Option<Type>) -> BufKind {
    rule_of(decl).1.kind(first).buf()
}

/// An SQLite value as a whole [`Value`]: text and blobs cut at `cap` bytes with their total
/// length, text that is not UTF-8 as `Lossy` bytes ([`ChunkBuilder::capped_value`]).
pub fn value_of(v: ValueRef<'_>, cap: u32) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Int(i),
        ValueRef::Real(f) => Value::F64(f),
        ValueRef::Text(t) => ChunkBuilder::capped_value(VarKind::Text, t, cap),
        ValueRef::Blob(b) => ChunkBuilder::capped_value(VarKind::Bytes, b, cap),
    }
}

/// Pushes one SQLite value into column `col` of kind `kind`: inline when the storage class is
/// the column's (text and blobs cut at `cap`), else a whole-value exception of its own class.
/// Never panics on any value.
pub fn push(b: &mut ChunkBuilder, col: u32, kind: ColKind, v: ValueRef<'_>, cap: u32) {
    match (kind, v) {
        (_, ValueRef::Null) => b.push_null(col),
        (ColKind::Int, ValueRef::Integer(i)) => b.push_i64(col, i),
        (ColKind::Float, ValueRef::Real(f)) => b.push_f64(col, f),
        (ColKind::Text, ValueRef::Text(t)) | (ColKind::Bytes, ValueRef::Blob(t)) => {
            b.push_bytes(col, t, cap)
        }
        (ColKind::Bool, ValueRef::Integer(i @ (0 | 1))) => b.push_bool(col, i == 1),
        _ => b.push_value(col, value_of(v, cap)),
    }
}

/// Pushes a value [`value_of`] made (a row of the first chunk, read before the column's kind
/// was known): the same cell [`push`] would have made of the SQLite value.
pub(crate) fn push_owned(b: &mut ChunkBuilder, col: u32, kind: ColKind, v: Value) {
    match (kind, v) {
        (ColKind::Bool, Value::Int(i @ (0 | 1))) => b.push_bool(col, i == 1),
        (_, v) => b.push_value(col, v),
    }
}

/// A declared type as read from the statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decl {
    /// No declared type (an expression, or a column declared without one).
    None,
    Declared(String),
    /// Not UTF-8 (or not reachable, because the column's name is not).
    Unreadable,
}

/// One result column as read from the prepared statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Column {
    /// The column's name; `?` when it is not UTF-8.
    pub name: Arc<str>,
    pub decl: Decl,
    pub origin: Option<OriginHint>,
}

impl Column {
    /// The column's native type, its rule, and so its kind.
    pub fn native_and_rule(&self) -> (NativeType, Rule) {
        match &self.decl {
            Decl::Unreadable => (NativeType::Unknown, Rule::Fixed(ColKind::Opaque)),
            Decl::None => {
                let (affinity, rule) = rule_of(None);
                (
                    NativeType::Sqlite {
                        decl: None,
                        affinity,
                    },
                    rule,
                )
            }
            Decl::Declared(d) => {
                let (affinity, rule) = rule_of(Some(d));
                (
                    NativeType::Sqlite {
                        decl: Some(d.as_str().into()),
                        affinity,
                    },
                    rule,
                )
            }
        }
    }
}

/// The result columns of `stmt`. rusqlite `expect`s on metadata that is not UTF-8
/// (`column.rs:181-212`), so each read runs inside `catch_unwind` on the connection's thread
/// (R12, P28): a column keeps its valid name when only its declared type is unreadable (it is
/// then `Opaque`); only an unreadable name becomes `?`, and then the declared type is
/// unreadable too (rusqlite reads the name first). An origin with any unreadable part is left
/// out. Each read is O(1): `ColumnsIter::nth` maps only the column asked for.
pub(crate) fn columns(stmt: &Statement<'_>) -> Vec<Column> {
    (0..stmt.column_count())
        .map(|i| {
            let Some(name) = column_name(stmt, i) else {
                return Column {
                    name: UNREADABLE_NAME.into(),
                    decl: Decl::Unreadable,
                    origin: None,
                };
            };
            let decl = catch_unwind(AssertUnwindSafe(|| {
                stmt.columns()
                    .nth(i)
                    .and_then(|c| c.decl_type().map(str::to_string))
            }));
            let decl = match decl {
                Ok(Some(d)) => Decl::Declared(d),
                Ok(None) => Decl::None,
                Err(_) => Decl::Unreadable,
            };
            let origin = catch_unwind(AssertUnwindSafe(|| {
                let m = stmt.columns_with_metadata().nth(i)?;
                Some(OriginHint::Sqlite {
                    database: m.database_name()?.into(),
                    table: m.table_name()?.into(),
                    column: m.origin_name()?.into(),
                })
            }))
            .unwrap_or(None);
            Column {
                name: name.into(),
                decl,
                origin,
            }
        })
        .collect()
}
