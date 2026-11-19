//! The PostgreSQL producer parts (M4 Task 19): table clauses from the model, and the
//! `ALTER SEQUENCE … OWNED BY` statement the IR has no variant for yet (T7 carry). Views,
//! functions and triggers are `pg_get_*def` server text; indexes with an expression, a
//! predicate or `INCLUDE` are `pg_get_indexdef` text when the `Sources` facet holds it
//! ([`super::DdlSources::server_text`]), else synthesized.

use hacivat_meta::name::Name;
use hacivat_meta::props::{Persistence, TableExt, TableProps};
use sql_ident::QualifiedName;

use crate::ir::PgTableDef;
use crate::render::{RenderCtx, RenderError, Stmt, TxnClass, stmt};

/// `ALTER SEQUENCE seq OWNED BY table.column`: a serial's sequence is created before its
/// table (the column default names it) and owned after it.
pub(crate) fn owned_by(
    ctx: &RenderCtx,
    seq: &QualifiedName,
    table: &QualifiedName,
    column: &Name,
) -> Result<Stmt, RenderError> {
    Ok(stmt(
        format!(
            "ALTER SEQUENCE {} OWNED BY {}.{}",
            ctx.qname(seq)?,
            ctx.qname(table)?,
            ctx.ident(column)?
        ),
        TxnClass::Transactional,
    ))
}

/// The table clauses of `tp`: persistence, `PARTITION OF … FOR VALUES`, `PARTITION BY`,
/// access method, storage parameters and tablespace, all raw. `INHERITS` needs a parent list
/// the model does not hold yet (the producer refuses an inheritance child instead).
pub(crate) fn table_ext(
    tp: &TableProps,
    partition_of: Option<(QualifiedName, Name)>,
) -> PgTableDef {
    let (access_method, with, tablespace) = match &tp.ext {
        TableExt::Pg(p) => (
            p.access_method.clone(),
            p.options.clone(),
            p.tablespace.clone(),
        ),
        _ => (None, Vec::new(), None),
    };
    PgTableDef {
        unlogged: tp.persistence == Persistence::Unlogged,
        temporary: tp.persistence == Persistence::Temporary,
        partition_of,
        partition_by: tp.partition_key.clone(),
        inherits: Vec::new(),
        access_method,
        with,
        tablespace,
    }
}
