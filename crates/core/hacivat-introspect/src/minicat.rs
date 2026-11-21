// ## m4-t01c-core
//! The system-catalog snapshot read (HV-INTRO-16; Task 23): the introspectors' own system-mode
//! pack sections (`packs/<dialect>/40-system.hvq.sql`, R-M4-44) run on a `MetaConn`, one table
//! of rows per section in server row order, for `cargo xtask minicat`.
// ## m4-t23-minicat
//!
//! Only pack text runs here (R-M4-7): the `40-system` sections, plus, on PostgreSQL, the core
//! pack's `Identity` (the schema's oid, bound as `:ns`) and `Canary` (the snapshot assigned no
//! XID), all inside one read-only snapshot of the metadata connection (PostgreSQL pins
//! `search_path = pg_catalog, pg_temp`, R-M4-46). The schema travels as a bind and must be one
//! of the engine's system schemas: a fixture database or `hv_catalog_probe` is refused before
//! the connection is touched (T1c carry). The generator's built-in name probe is **not** here:
//! it lives only in `tests/minicat_gen.rs` (R-M4-44, ADR 0008 `m4-t23`).

use futures::future::BoxFuture;
use hacivat_meta::Name;
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaConn, Rows, SnapshotTxn};
use hacivat_values::{BindValue, LogicalType};
use sql_dialect::DialectId;

use crate::introspector::IntroError;
use crate::pack::{Mode, Pack, PackCtx};

/// The pack file of the system-mode sections.
pub const SYSTEM_FILE: &str = "40-system";

const PG_SYSTEM_SCHEMAS: [&str; 2] = ["information_schema", "pg_catalog"];
const MYSQL_SYSTEM_SCHEMAS: [&str; 4] =
    ["information_schema", "mysql", "performance_schema", "sys"];

/// The system schemas a snapshot reads for `engine` (none for an engine without snapshots,
/// R-M4-6).
pub fn system_schemas(engine: DbmsKind) -> &'static [&'static str] {
    match engine {
        DbmsKind::Postgres => &PG_SYSTEM_SCHEMAS,
        DbmsKind::MySql | DbmsKind::MariaDb => &MYSQL_SYSTEM_SCHEMAS,
        _ => &[],
    }
}

/// The sections of `pack`'s `40-system` file, in pack order.
pub fn system_sections(pack: &Pack) -> Vec<Name> {
    pack.sections()
        .iter()
        .filter(|s| pack.section(s).is_some_and(|sec| sec.file() == SYSTEM_FILE))
        .cloned()
        .collect()
}

fn protocol(code: &'static str) -> IntroError {
    IntroError::Protocol(Name::from(code))
}

fn pg_family(d: DialectId) -> bool {
    matches!(d, DialectId::Postgres | DialectId::PgWire(_))
}

fn mysql_family(d: DialectId) -> bool {
    matches!(d, DialectId::MySql | DialectId::MariaDb)
}

/// The text of a text bind named `name`.
fn text_param<'c>(ctx: &'c PackCtx, name: &str) -> Option<&'c str> {
    ctx.params
        .scalars
        .iter()
        .find(|(n, _)| &**n == name)
        .and_then(|(_, v)| match v {
            BindValue::Typed {
                hint: LogicalType::Text { .. },
                text,
            } => Some(&**text),
            _ => None,
        })
}

fn text_bind(s: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: s.into(),
    }
}

/// The cell `col` of the first row of `rows`.
fn first_cell<'r>(rows: &'r Rows, col: &str) -> Option<&'r str> {
    let i = rows.columns.iter().position(|c| &**c == col)?;
    rows.rows.first()?.get(i)?.as_deref()
}

/// One section rendered for `ctx` on `txn`, chunks concatenated.
#[expect(
    clippy::result_large_err,
    reason = "IntroError is the introspectors' error (Task 1c's seam); boxing it here alone \
              would not shrink it"
)]
async fn run_section(
    txn: &mut SnapshotTxn<'_>,
    pack: &Pack,
    section: &str,
    ctx: &PackCtx,
) -> Result<Rows, IntroError> {
    let qs = pack
        .render(section, ctx)
        .map_err(|_| IntroError::Protocol(Name::from(format!("minicat/{section}"))))?;
    let mut out = Rows::default();
    for q in &qs {
        let rows = txn.rows(q.as_meta_query()).await?;
        if out.columns.is_empty() {
            out.columns = rows.columns;
        }
        out.rows.extend(rows.rows);
    }
    Ok(out)
}

/// Runs every system-mode section of `pack` on `c` and answers each section's rows, in pack
/// order, chunks concatenated. `ctx` is `Mode::System` with the text bind `schema` naming one
/// of the engine's system schemas ([`system_schemas`] of `ctx.flavor`); on PostgreSQL the
/// schema's oid is read with `Identity` and bound as `ns`. Everything runs in one snapshot.
pub fn run_system_sections<'a>(
    c: &'a mut MetaConn,
    pack: &'a Pack,
    ctx: &'a PackCtx,
) -> BoxFuture<'a, Result<Vec<(Name, Rows)>, IntroError>> {
    Box::pin(async move {
        if ctx.mode != Mode::System {
            return Err(protocol("minicat/mode"));
        }
        let pg = pg_family(ctx.dialect);
        let same_family = (pg && pg_family(pack.dialect()))
            || (mysql_family(ctx.dialect) && mysql_family(pack.dialect()));
        if !same_family {
            return Err(protocol("minicat/dialect"));
        }
        let schema = text_param(ctx, "schema");
        if !schema.is_some_and(|s| system_schemas(ctx.flavor).contains(&s)) {
            return Err(protocol("minicat/schema"));
        }
        let sections = system_sections(pack);
        if sections.is_empty() {
            return Err(protocol("minicat/no-sections"));
        }
        let mut ctx = ctx.clone();
        let mut txn = c.begin_snapshot().await?;
        if pg {
            let mut full = ctx.clone();
            full.mode = Mode::Full;
            let id = run_section(&mut txn, pack, "Identity", &full).await?;
            let ns = first_cell(&id, "ns_oid")
                .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(|| protocol("minicat/schema"))?
                .to_string();
            ctx.params.scalars.retain(|(n, _)| &**n != "ns");
            ctx.params.scalars.push((Name::from("ns"), text_bind(&ns)));
        }
        let mut out = Vec::with_capacity(sections.len());
        for s in &sections {
            out.push((s.clone(), run_section(&mut txn, pack, s, &ctx).await?));
        }
        if pg {
            let mut full = ctx.clone();
            full.mode = Mode::Full;
            let canary = run_section(&mut txn, pack, "Canary", &full).await?;
            if !matches!(first_cell(&canary, "no_xid"), Some("t" | "true")) {
                return Err(protocol("minicat/canary"));
            }
        }
        txn.commit().await?;
        Ok(out)
    })
}
