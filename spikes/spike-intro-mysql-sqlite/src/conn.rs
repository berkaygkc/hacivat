//! The catalog connection: catalog SQL with bound names in, text rows out.
//!
//! [`TypedCatalog`] runs over any [`RawConn`] through its typed path (`query`/`next_event`),
//! because that is the existing API that takes binds (HV-INTRO-14 rule 1: names are always
//! bound, never spliced). `execute` has no binds. Every value is read back as text; catalog SQL
//! casts temporal values to `CHAR` so their text is the server's.

use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_driver::{Protocol, QueryEvent, QuerySpec, RawConn, RawError};
use hacivat_values::{BindValue, LogicalType, ValueRef};

/// An introspection failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntroError {
    /// The driver failed (`RawError` kept for its server code).
    Driver { sql: String, error: Box<RawError> },
    /// A catalog row did not have the shape the query declares.
    Shape { sql: String, detail: String },
    /// A value was cut at the cell cap (never expected: the cap is 256 MiB).
    Truncated { sql: String },
}

impl IntroError {
    /// The server's vendor code (MySQL error number), when the server answered.
    pub fn vendor_code(&self) -> Option<i64> {
        match self {
            IntroError::Driver { error, .. } => error.server.as_ref().and_then(|s| s.vendor_code),
            _ => None,
        }
    }

    /// The server's message text, when it sent one.
    pub fn server_text(&self) -> Option<&str> {
        match self {
            IntroError::Driver { error, .. } => error.server.as_ref().map(|s| s.text.as_str()),
            _ => None,
        }
    }
}

/// One catalog row: SQL NULL is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row(pub Vec<Option<String>>);

impl Row {
    pub fn opt(&self, i: usize) -> Option<&str> {
        self.0.get(i).and_then(|v| v.as_deref())
    }

    /// The owned text of column `i` (`None` for NULL).
    pub fn text(&self, i: usize) -> Option<String> {
        self.opt(i).map(str::to_owned)
    }

    /// Column `i`, NULL as the empty string.
    pub fn str(&self, i: usize) -> &str {
        self.opt(i).unwrap_or("")
    }

    pub fn u32(&self, i: usize) -> Option<u32> {
        self.opt(i).and_then(|v| v.trim().parse().ok())
    }

    pub fn i64(&self, i: usize) -> Option<i64> {
        self.opt(i).and_then(|v| v.trim().parse().ok())
    }

    /// `YES`/`1`/`ON` (ASCII, any case) → true.
    pub fn flag(&self, i: usize) -> bool {
        matches!(self.opt(i), Some(v) if v.eq_ignore_ascii_case("yes") || v == "1" || v.eq_ignore_ascii_case("on"))
    }
}

/// What the introspectors need from a connection.
pub trait CatalogConn: Send {
    /// Runs one catalog query with `binds` (text) for its `?` placeholders.
    fn rows<'a>(
        &'a mut self,
        sql: &'a str,
        binds: &'a [&'a str],
    ) -> BoxFuture<'a, Result<Vec<Row>, IntroError>>;

    /// Runs trusted control text (`SET …`, `START TRANSACTION …`, `PRAGMA …`).
    fn control<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), IntroError>>;
}

/// A [`CatalogConn`] over a [`RawConn`]'s typed path, logging every statement it sends (the
/// tests assert on the log, e.g. "an INSERT re-reads no table", HV-INTRO-07 test 4).
pub struct TypedCatalog<'c> {
    conn: &'c mut dyn RawConn,
    pub log: Vec<String>,
    pub rows_read: u64,
}

impl<'c> TypedCatalog<'c> {
    pub fn new(conn: &'c mut dyn RawConn) -> Self {
        TypedCatalog {
            conn,
            log: Vec::new(),
            rows_read: 0,
        }
    }

    /// Statements of the log that contain `needle`.
    pub fn sent(&self, needle: &str) -> usize {
        self.log.iter().filter(|s| s.contains(needle)).count()
    }
}

const CELL_CAP: u32 = 256 << 20;

fn spec<'a>(sql: &'a str, binds: &'a [BindValue]) -> QuerySpec<'a> {
    QuerySpec {
        sql,
        binds,
        protocol: Protocol::Extended,
        suspendable: false,
        first_rows: 8_192,
        chunk_rows: 8_192,
        chunk_bytes: 16 << 20,
        cell_cap: CELL_CAP,
        max_result_sets: 1,
    }
}

/// A cell's text. Integers print in decimal; bytes are read as UTF-8 (lossy: MySQL sends a
/// `_bin` string with the binary flag on some versions).
fn cell_text(v: ValueRef<'_>) -> Result<Option<String>, ()> {
    Ok(match v {
        ValueRef::Null => None,
        ValueRef::Text(s) | ValueRef::Json(s) | ValueRef::Xml(s) => Some(s.to_owned()),
        ValueRef::Int(i) => Some(i.to_string()),
        ValueRef::UInt(u) => Some(u.to_string()),
        ValueRef::Bool(b) => Some(if b { "1" } else { "0" }.to_owned()),
        ValueRef::F64(f) => Some(f.to_string()),
        ValueRef::F32(f) => Some(f.to_string()),
        ValueRef::Bytes(b) | ValueRef::Lossy(b) => Some(String::from_utf8_lossy(b).into_owned()),
        ValueRef::Truncated(_) => return Err(()),
        // Catalog SQL casts every other type to CHAR; anything else is a shape error.
        other => Some(format!("{other:?}")),
    })
}

impl CatalogConn for TypedCatalog<'_> {
    fn rows<'a>(
        &'a mut self,
        sql: &'a str,
        binds: &'a [&'a str],
    ) -> BoxFuture<'a, Result<Vec<Row>, IntroError>> {
        Box::pin(async move {
            self.log.push(sql.to_owned());
            let bv: Vec<BindValue> = binds
                .iter()
                .map(|b| BindValue::Typed {
                    hint: LogicalType::Text {
                        max_chars: None,
                        fixed: false,
                    },
                    text: Arc::from(*b),
                })
                .collect();
            let err = |error: RawError| IntroError::Driver {
                sql: sql.to_owned(),
                error: Box::new(error),
            };
            self.conn.query(spec(sql, &bv)).await.map_err(err)?;
            let mut out = Vec::new();
            let mut width = 0usize;
            loop {
                match self.conn.next_event().await.map_err(err)? {
                    QueryEvent::ResultStart(schema) => width = schema.columns.len(),
                    QueryEvent::Rows(chunk) => {
                        for r in 0..chunk.rows() {
                            let mut row = Vec::with_capacity(width);
                            for c in 0..width {
                                let v = cell_text(chunk.cell(r, c as u32)).map_err(|()| {
                                    IntroError::Truncated {
                                        sql: sql.to_owned(),
                                    }
                                })?;
                                row.push(v);
                            }
                            out.push(Row(row));
                        }
                    }
                    QueryEvent::Done(_) => break,
                    QueryEvent::ResultEnd(_) | QueryEvent::Notice(_) => {}
                    QueryEvent::Suspended => {
                        return Err(IntroError::Shape {
                            sql: sql.to_owned(),
                            detail: "suspended".into(),
                        });
                    }
                }
            }
            self.rows_read += out.len() as u64;
            Ok(out)
        })
    }

    fn control<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), IntroError>> {
        Box::pin(async move {
            self.log.push(sql.to_owned());
            self.conn
                .batch(sql)
                .await
                .map_err(|error| IntroError::Driver {
                    sql: sql.to_owned(),
                    error: Box::new(error),
                })
        })
    }
}

/// `?, ?, …` for `n` binds.
pub fn placeholders(n: usize) -> String {
    let mut s = String::with_capacity(n * 3);
    for i in 0..n {
        if i > 0 {
            s.push_str(", ");
        }
        s.push('?');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_join_with_commas() {
        assert_eq!(placeholders(0), "");
        assert_eq!(placeholders(1), "?");
        assert_eq!(placeholders(3), "?, ?, ?");
    }

    #[test]
    fn row_flag_reads_yes_one_on() {
        let r = Row(vec![
            Some("YES".into()),
            Some("1".into()),
            Some("NO".into()),
            None,
        ]);
        assert!(r.flag(0) && r.flag(1) && !r.flag(2) && !r.flag(3));
    }
}
