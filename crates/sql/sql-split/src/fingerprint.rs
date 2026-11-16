//! Statement fingerprints (SQL-PARAM-03; Task 6): `HvNorm1` replaces literals and `IN (…)`
//! lists with placeholders, collapses whitespace and comments, ASCII-uppercases keywords, keeps
//! identifiers and quoted text byte-exact and hashes with XXH3-64 (P-M3b-18).
//!
//! STUB until Task 6: [`hv_norm1`] hashes the raw text (no normalisation), `version` 0.

use std::sync::Arc;

use sql_dialect::DialectCtx;
use twox_hash::XxHash3_64;

/// Which normaliser produced a [`Fingerprint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FingerprintAlgo {
    /// Hacivat's lexer-based normaliser (every dialect).
    HvNorm1,
    /// libpg_query's fingerprint (PostgreSQL).
    PgQuery,
}

/// A statement's fingerprint: equal for statements that differ only in literals, whitespace,
/// comments and keyword case.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    pub algo: FingerprintAlgo,
    pub hash: u64,
    pub normalized: Arc<str>,
    /// The normaliser's version; `0` is Task 1a's stub.
    pub version: u16,
}

/// The `HvNorm1` fingerprint of `text` in `ctx`. STUB: XXH3-64 of the raw text, version 0.
pub fn hv_norm1(text: &str, ctx: &DialectCtx) -> Fingerprint {
    let _ = ctx;
    Fingerprint {
        algo: FingerprintAlgo::HvNorm1,
        hash: XxHash3_64::oneshot(text.as_bytes()),
        normalized: Arc::from(text),
        version: 0,
    }
}

#[cfg(test)]
mod tests {
    use sql_dialect::DialectId;

    use super::*;

    #[test]
    fn stub_hashes_the_raw_text() {
        let ctx = DialectCtx::new(DialectId::Postgres, None);
        let a = hv_norm1("SELECT 1", &ctx);
        assert_eq!(a.version, 0);
        assert_eq!(a.algo, FingerprintAlgo::HvNorm1);
        assert_eq!(&*a.normalized, "SELECT 1");
        assert_ne!(a.hash, hv_norm1("select 1", &ctx).hash);
    }
}
