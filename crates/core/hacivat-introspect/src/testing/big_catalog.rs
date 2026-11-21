// ## m4-t01c-core (seam), filled by m4-t16-pg
//! The large PostgreSQL fixture `hv_m4_big` (Task 16; reused by Tasks 28–29, dropped at the
//! gate, C-9): `tables` tables with `columns` columns in all (15,000 and 160,000 for the
//! gate), in the schema `hv_m4_big` of the connection's database (`hv_test` on `pg17`).
//! Every table has an `id int` primary key; every 10th table also references the previous
//! table and carries a comment (the PG spike's 10k fixture, section 5).
//!
//! Created and dropped idempotently in **batches of 250 tables per server-side `DO` block**
//! (one transaction for all of them runs out of lock-table slots, PG spike section 5): every
//! statement is `IF NOT EXISTS` / `IF EXISTS`, so a run interrupted half-way resumes. It runs
//! on a writable test connection, never on a metadata connection; its text holds only
//! numbers this module computes (no user text).

use futures::future::BoxFuture;
use hacivat_session::pipeline::LiveConnection;

/// The fixture schema.
pub const SCHEMA: &str = "hv_m4_big";

/// Tables per `DO` block.
pub const BATCH: u32 = 250;

/// Why the fixture could not be built or dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BigCatalogError {
    /// Not written yet (M4 Task 1c stub; no longer returned).
    NotImplemented,
    /// `tables` is 0 or `columns` is fewer than `tables` (each table has its key column).
    BadShape,
    /// A batch failed: its first table number and the server's code (no text: the batch is
    /// fixture SQL, and a server message may quote it).
    Server {
        first_table: u32,
        code: Option<String>,
    },
}

/// The number of columns of table `i` (1-based) when `columns` are spread over `tables`.
pub fn columns_of(i: u32, tables: u32, columns: u32) -> u32 {
    let base = columns / tables;
    let extra = columns % tables;
    base + u32::from(i <= extra)
}

/// The `DO` block that creates tables `first..=last`.
pub fn create_batch(first: u32, last: u32, tables: u32, columns: u32) -> String {
    let mut body = String::new();
    for i in first..=last {
        let n = columns_of(i, tables, columns);
        let mut cols = String::from("id int PRIMARY KEY");
        let mut extra = n.saturating_sub(1);
        if i % 10 == 0 && i > 1 && extra > 0 {
            cols.push_str(&format!(", ref int REFERENCES {SCHEMA}.t{} (id)", i - 1));
            extra -= 1;
        }
        for c in 1..=extra {
            cols.push_str(&format!(", c{c} int"));
        }
        body.push_str(&format!(
            "CREATE TABLE IF NOT EXISTS {SCHEMA}.t{i} ({cols});\n"
        ));
        if i % 10 == 0 {
            body.push_str(&format!("COMMENT ON TABLE {SCHEMA}.t{i} IS 'big {i}';\n"));
        }
    }
    format!("DO $hv_big$ BEGIN\n{body}END $hv_big$")
}

/// The `DO` block that drops tables `t<first>..=t<last>` of `schema` (a test schema name:
/// ASCII letters, digits and `_` only, checked by [`drop_schema_tables`]).
pub fn drop_batch(schema: &str, first: u32, last: u32) -> String {
    let mut body = String::new();
    // In reverse, so a table's referencing neighbour goes first.
    for i in (first..=last).rev() {
        body.push_str(&format!("DROP TABLE IF EXISTS {schema}.t{i} CASCADE;\n"));
    }
    format!("DO $hv_big$ BEGIN\n{body}END $hv_big$")
}

/// Drops the tables `t1..=t<tables>` of `schema` in batches of [`BATCH`] (the highest first),
/// then the schema: never one `DROP SCHEMA … CASCADE` over thousands of tables, which runs
/// out of lock-table slots (PG spike section 5; review I-5). `schema` must be a plain
/// lower-case ASCII identifier (the test schemas' `hvm4_*` names).
pub fn drop_schema_tables<'a>(
    conn: &'a mut LiveConnection,
    schema: &'a str,
    tables: u32,
) -> BoxFuture<'a, Result<(), BigCatalogError>> {
    Box::pin(async move {
        let plain = !schema.is_empty()
            && schema
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !plain {
            return Err(BigCatalogError::BadShape);
        }
        for (first, last) in batches(tables).into_iter().rev() {
            run(conn, &drop_batch(schema, first, last), first).await?;
        }
        run(conn, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"), 0).await
    })
}

/// The batches `[(first, last)]` of `tables` tables.
pub fn batches(tables: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut first = 1;
    while first <= tables {
        let last = (first + BATCH - 1).min(tables);
        out.push((first, last));
        first = last + 1;
    }
    out
}

async fn run(
    conn: &mut LiveConnection,
    sql: &str,
    first_table: u32,
) -> Result<(), BigCatalogError> {
    conn.raw()
        .batch(sql)
        .await
        .map_err(|e| BigCatalogError::Server {
            first_table,
            code: e.server.as_ref().and_then(|m| m.sqlstate.clone()),
        })
}

/// Creates `hv_m4_big` with `tables` tables and `columns` columns in all, unless it is there
/// (every batch is idempotent).
pub fn ensure_pg(
    conn: &mut LiveConnection,
    tables: u32,
    columns: u32,
) -> BoxFuture<'_, Result<(), BigCatalogError>> {
    Box::pin(async move {
        if tables == 0 || columns < tables {
            return Err(BigCatalogError::BadShape);
        }
        run(conn, &format!("CREATE SCHEMA IF NOT EXISTS {SCHEMA}"), 0).await?;
        for (first, last) in batches(tables) {
            run(conn, &create_batch(first, last, tables, columns), first).await?;
        }
        Ok(())
    })
}

/// Drops `hv_m4_big`: its tables in batches of [`BATCH`] (the highest first), then the schema.
pub fn drop_pg(
    conn: &mut LiveConnection,
    tables: u32,
) -> BoxFuture<'_, Result<(), BigCatalogError>> {
    drop_schema_tables(conn, SCHEMA, tables)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_shape_spreads_160k_columns_over_15k_tables_in_250_table_batches() {
        let (t, c) = (15_000, 160_000);
        assert_eq!((1..=t).map(|i| columns_of(i, t, c)).sum::<u32>(), c);
        let b = batches(t);
        assert_eq!(b.len(), 60);
        assert!(b.iter().all(|(f, l)| l - f < BATCH));
        assert_eq!(b.last(), Some(&(14_751, 15_000)));
        let sql = create_batch(1, 250, t, c);
        assert_eq!(sql.matches("CREATE TABLE IF NOT EXISTS").count(), 250);
        assert!(sql.contains("REFERENCES hv_m4_big.t9 (id)"));
        assert!(
            drop_batch(SCHEMA, 1, 250)
                .starts_with("DO $hv_big$ BEGIN\nDROP TABLE IF EXISTS hv_m4_big.t250")
        );
    }
}
