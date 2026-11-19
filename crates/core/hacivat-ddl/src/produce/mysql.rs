//! The MySQL and MariaDB producer parts (M4 Task 19). Views, routines and triggers are their
//! `SHOW CREATE` text (R-M4-37), shown verbatim; the renderer makes the head edits
//! `skip_definer` and `or_replace` ask for. Text holding an executable comment (`/*!…*/`,
//! MariaDB `/*M!…*/`) raises `ddl-warn-executable-comment`. Column defaults and `ON UPDATE`
//! texts are kept as each flavor prints them (F9); `DEFAULT` text arrives quoted from the
//! introspector (T7 carry). Secondary indexes are `CREATE INDEX` after the table (the IR has
//! no `KEY` table element, T7 carry).

use hacivat_meta::name::Name;
use hacivat_meta::props::{TableExt, TableProps};
use sql_dialect::DialectId;

use sql_ident::QualifiedName;

use crate::ir::{IndexPartDef, MySqlTableDef};
use crate::render::{RenderCtx, RenderError, Stmt, TxnClass, index_part, stmt};

pub use crate::render::mysql::has_executable_comment;

/// The expression of the `EXTRA` clause `on update …` (`DEFAULT_GENERATED on update
/// CURRENT_TIMESTAMP(3)` → `CURRENT_TIMESTAMP(3)`). The expression is one token (MySQL allows
/// only `CURRENT_TIMESTAMP` and its synonyms there), so words after it (`INVISIBLE`, MySQL
/// 8.0.23+) are not part of it.
pub fn on_update_of(extra: &str) -> Option<Name> {
    const KEY: &[u8] = b"on update ";
    let b = extra.as_bytes();
    (0..b.len())
        .find(|&i| b.len() - i >= KEY.len() && b[i..i + KEY.len()].eq_ignore_ascii_case(KEY))
        .and_then(|i| extra[i + KEY.len()..].split_ascii_whitespace().next())
        .map(Name::from)
}

/// `ALTER TABLE table ADD PRIMARY KEY (parts) [USING HASH] [COMMENT '…']`: a primary key
/// whose index options have no constraint form (descending parts, prefixes, a method, a
/// comment). `BTREE` is the default and is not written; any other method is refused.
pub(crate) fn add_primary_key(
    ctx: &RenderCtx,
    table: &QualifiedName,
    parts: &[IndexPartDef],
    method: Option<&str>,
    comment: Option<&str>,
) -> Result<Stmt, RenderError> {
    let parts: Result<Vec<String>, _> = parts.iter().map(|p| index_part(ctx, p)).collect();
    let mut s = format!(
        "ALTER TABLE {} ADD PRIMARY KEY ({})",
        ctx.qname(table)?,
        parts?.join(", ")
    );
    match method {
        Some(m) if m.eq_ignore_ascii_case("hash") => s.push_str(" USING HASH"),
        Some(m) if !m.eq_ignore_ascii_case("btree") => {
            return Err(RenderError::Invalid(
                "a primary key with a method other than BTREE or HASH",
            ));
        }
        _ => {}
    }
    if let Some(c) = comment {
        s.push_str(" COMMENT ");
        s.push_str(&ctx.literal(c)?);
    }
    Ok(stmt(s, TxnClass::ImplicitCommit))
}

/// Whether an index method is not the default `BTREE` (none means the default).
pub(crate) fn non_default_method(method: Option<&str>) -> bool {
    method.is_some_and(|m| !m.eq_ignore_ascii_case("btree"))
}

/// The tokens of `CREATE_OPTIONS` (space-separated `name=value` words).
fn option_tokens(options: &str) -> impl Iterator<Item = &str> {
    options.split_ascii_whitespace()
}

/// `ROW_FORMAT` when `CREATE_OPTIONS` states it explicitly (`row_format=DYNAMIC`).
pub fn row_format_of(options: &str) -> Option<Name> {
    const KEY: &str = "row_format=";
    option_tokens(options).find_map(|t| {
        let head = t.get(..KEY.len())?;
        head.eq_ignore_ascii_case(KEY)
            .then(|| Name::from(&t[KEY.len()..]))
            .filter(|v| !v.is_empty())
    })
}

/// Whether `CREATE_OPTIONS` says the table is partitioned (the partitioning clause is not
/// introspected in M4: the producer says so with `ddl-unsupported-kind`).
pub fn is_partitioned(options: &str) -> bool {
    option_tokens(options).any(|t| t.eq_ignore_ascii_case("partitioned"))
}

/// The table options of `tp`: engine, explicit row format and collation (the collation
/// implies the character set, so `DEFAULT CHARSET` is not written).
pub(crate) fn table_ext(tp: &TableProps) -> MySqlTableDef {
    match &tp.ext {
        TableExt::MySql {
            engine,
            collation,
            options,
        } => MySqlTableDef {
            engine: engine.clone(),
            row_format: options.as_deref().and_then(row_format_of),
            charset: None,
            collation: collation.clone(),
        },
        _ => MySqlTableDef::default(),
    }
}

/// The index option that hides an index from the optimizer: MySQL `INVISIBLE`, MariaDB
/// `IGNORED`.
pub(crate) fn invisible_word(d: DialectId) -> &'static str {
    if d == DialectId::MariaDb {
        "IGNORED"
    } else {
        "INVISIBLE"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_update_is_read_from_extra() {
        assert_eq!(
            on_update_of("DEFAULT_GENERATED on update CURRENT_TIMESTAMP(3)").as_deref(),
            Some("CURRENT_TIMESTAMP(3)")
        );
        assert_eq!(
            on_update_of("on update current_timestamp()").as_deref(),
            Some("current_timestamp()")
        );
        assert_eq!(
            on_update_of("DEFAULT_GENERATED on update CURRENT_TIMESTAMP INVISIBLE").as_deref(),
            Some("CURRENT_TIMESTAMP"),
            "a trailing EXTRA word is not part of the expression"
        );
        assert_eq!(on_update_of("auto_increment"), None);
        assert_eq!(on_update_of("on update "), None);
        assert_eq!(on_update_of(""), None);
    }

    #[test]
    fn create_options_are_tokens() {
        assert_eq!(
            row_format_of("stats_persistent=0 row_format=DYNAMIC").as_deref(),
            Some("DYNAMIC")
        );
        assert_eq!(row_format_of("row_format="), None);
        assert_eq!(row_format_of("partitioned"), None);
        assert!(is_partitioned("partitioned"));
        assert!(!is_partitioned("row_format=DYNAMIC"));
    }
}
