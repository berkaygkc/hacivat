//! Hashes the classifier attaches to a report (Task 14): the request's fingerprint and the
//! hash of the exact bytes classified (P-M3b-2), which the exec gate compares with the text it
//! is about to send.
//!
//! **Fingerprint** (`alg/13` SQL-SAFE-02 step 9, SQL-PARAM-03): on PostgreSQL (and the PG-wire
//! flavours) libpg_query's own fingerprint (`sql_oracle_pg::tree::fingerprint`, on the
//! oracle's serial parser thread) with `algo = PgQuery`; when libpg_query cannot fingerprint
//! the text (a syntax error, a limit, the parser thread gone), and on every other dialect,
//! Hacivat's `HvNorm1` with `algo = HvNorm1`. `Fingerprint.normalized` is always `HvNorm1`'s
//! text (literals masked: no secret survives it), so a reader has one readable form whichever
//! hash was taken. A text the classifier refused before analysis never reaches the parser:
//! it is fingerprinted with `HvNorm1` only.

use sql_dialect::{DialectCtx, DialectId};
use sql_split::{Fingerprint, FingerprintAlgo, hv_norm1};
use twox_hash::XxHash3_64;

/// XXH3-64 of `text`'s bytes: `SafetyReport.classified_hash`.
pub fn classified_hash(text: &str) -> u64 {
    XxHash3_64::oneshot(text.as_bytes())
}

// ## m3b-t14-orchestrator

/// The version recorded with a libpg_query fingerprint: libpg_query's fingerprint algorithm 3
/// (the one every libpg_query since PostgreSQL 13 computes).
pub const PG_QUERY_FINGERPRINT_VERSION: u16 = 3;

/// The fingerprint of a request the classifier analysed (the module docs).
pub(crate) fn request_fingerprint(sql: &str, ctx: &DialectCtx) -> Fingerprint {
    let norm = hv_norm1(sql, ctx);
    if !matches!(ctx.id, DialectId::Postgres | DialectId::PgWire(_)) {
        return norm;
    }
    match sql_oracle_pg::tree::fingerprint(sql) {
        Ok(hash) => Fingerprint {
            algo: FingerprintAlgo::PgQuery,
            hash,
            normalized: norm.normalized,
            version: PG_QUERY_FINGERPRINT_VERSION,
        },
        Err(_) => norm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg() -> DialectCtx {
        DialectCtx::new(DialectId::Postgres, None)
    }

    #[test]
    fn pg_takes_the_parsers_fingerprint_and_keeps_the_readable_form() {
        let a = request_fingerprint("SELECT a FROM t WHERE id = 1", &pg());
        let b = request_fingerprint("select a\n from t where id = 42 -- x", &pg());
        assert_eq!(a.algo, FingerprintAlgo::PgQuery);
        assert_eq!(a.version, PG_QUERY_FINGERPRINT_VERSION);
        assert_eq!(a.hash, b.hash);
        assert_eq!(
            &*a.normalized,
            &*hv_norm1("SELECT a FROM t WHERE id = 1", &pg()).normalized
        );
        assert!(!a.normalized.contains('1'));
    }

    #[test]
    fn a_parse_error_falls_back_to_hv_norm1() {
        let f = request_fingerprint("SELEC 1", &pg());
        assert_eq!(f, hv_norm1("SELEC 1", &pg()));
        assert_eq!(f.algo, FingerprintAlgo::HvNorm1);
    }

    #[test]
    fn other_dialects_use_hv_norm1() {
        for d in [DialectId::MySql, DialectId::MariaDb, DialectId::Sqlite] {
            let ctx = DialectCtx::new(d, None);
            assert_eq!(
                request_fingerprint("SELECT 1", &ctx),
                hv_norm1("SELECT 1", &ctx)
            );
        }
    }
}
