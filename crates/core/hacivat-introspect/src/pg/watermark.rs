// ## m4-t16-pg (spike origin 5a5ab08, `src/pg/watermark.rs`)
//! The incremental watermark and the full-or-incremental decision (HV-INTRO-05 steps 1–3;
//! R-M4-8, R-M4-9).
//!
//! The watermark `W` is the **snapshot xmin** of the pass's `REPEATABLE READ READ ONLY`
//! transaction (`pg_snapshot_xmin(pg_current_snapshot())`, 64-bit, epoch-extended): every
//! transaction below it had finished when the pass read the catalog, so the next pass re-reads
//! only rows written by transactions `>= W`. Snapshot functions never assign an XID
//! (ver/01 `#pg-canary`); `txid_current()` would, and no pack calls it.
//!
//! The change predicate is the server's: `age(x.xmin) BETWEEN 0 AND age(W::xid)` (finding 6).
//! The client keeps one check only, the 64-bit span `snap_xmax − W ≥ 2^31 − 2^20`
//! (`Full(wraparound)`): `age()` is 32-bit, and the 2^20 slack absorbs XIDs other sessions
//! assign between the probe and the change query.
//!
//! **Storage.** The model's `Watermark::PgXid { w, identity }` keeps `W`, the postmaster start
//! time (a restart) and one 64-bit identity word: the high 48 bits are an XXH3 of the system
//! identifier, the timeline and the database oid; the low 16 bits the server's major version.
//! A role that may not call `pg_control_system()` still has the start time and the database
//! oid (the spike's fallback).

use hacivat_meta::Watermark;
use hacivat_meta::namespace::PgIdentity;

use super::decode::IdentityRow;

/// Why a pass is full (stable codes, `intro-full-reason`; HV-INTRO-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FullReason {
    /// No previous watermark (a first load, or a watermark of another engine).
    NoWatermark,
    /// The server restarted since the last pass.
    Restart,
    /// Another server, database, timeline or cluster; or a watermark ahead of the server's
    /// next XID (a restore or `pg_resetwal`).
    Identity,
    /// The server's major version changed.
    Version,
    /// The server is a hot standby (incremental is not verified there, R-M4-9).
    Standby,
    /// The watermark is too old for 32-bit `age()` arithmetic.
    Wraparound,
    /// More than [`MASS_CHANGE_PCT`] % of the namespace's relations changed.
    MassChange,
    /// A schema whose types or routines the namespace references changed (a rename is two
    /// hops away, PG spike section 7 Q3).
    SchemaRename,
    /// The requested level is above the cached one.
    Level,
    /// A full pass was asked for (`Mode::Full`, `Brief`, a first load).
    Requested,
}

impl FullReason {
    pub fn code(self) -> &'static str {
        match self {
            FullReason::NoWatermark => "no-watermark",
            FullReason::Restart => "restart",
            FullReason::Identity => "identity",
            FullReason::Version => "version",
            FullReason::Standby => "standby",
            FullReason::Wraparound => "wraparound",
            FullReason::MassChange => "mass-change",
            FullReason::SchemaRename => "schema-rename",
            FullReason::Level => "level",
            FullReason::Requested => "requested",
        }
    }
}

/// Above this share (in percent) of the namespace's relations changed in one incremental pass,
/// a full pass is cheaper (PG spike section 7 Q5; R-M4-48).
pub const MASS_CHANGE_PCT: usize = 25;

/// The mass-change rule applies only to namespaces with at least this many relations: in a
/// small namespace a few re-reads cost less than the full pass, and the narrow-read
/// guarantees (HV-INTRO-05 tests 1–7) hold. Deviation recorded in ADR 0008 `m4-t16`.
pub const MASS_CHANGE_MIN_RELATIONS: usize = 20;

/// Whether `changed` of `relations` relations is a mass change.
pub fn is_mass_change(changed: usize, relations: usize) -> bool {
    relations >= MASS_CHANGE_MIN_RELATIONS
        && changed.saturating_mul(100) > relations.saturating_mul(MASS_CHANGE_PCT)
}

/// The largest span (in XIDs) from the previous watermark to the server's next XID that is
/// still incremental: `age()` is 32-bit and turns negative past 2^31; the 2^20 slack covers
/// XIDs assigned between the probe and the change query.
pub const SPAN_LIMIT: u64 = (1 << 31) - (1 << 20);

/// Who the server is, as the probe read it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerIdentity {
    pub system_identifier: Option<String>,
    pub timeline: Option<u32>,
    pub database: u32,
    /// `server_version_num`.
    pub version: u32,
    /// `pg_postmaster_start_time()`, microseconds since the Unix epoch.
    pub start_micros: i64,
}

impl ServerIdentity {
    /// The identity of an `Identity` probe row.
    pub fn of(row: &IdentityRow) -> ServerIdentity {
        ServerIdentity {
            system_identifier: row.system_identifier.clone(),
            timeline: row.timeline,
            database: row.db_oid,
            version: row.version,
            start_micros: epoch_micros(&row.start_epoch),
        }
    }

    /// The 64-bit identity word (module docs).
    pub fn word(&self) -> u64 {
        let mut text = String::new();
        text.push_str(self.system_identifier.as_deref().unwrap_or("-"));
        text.push('|');
        text.push_str(
            &self
                .timeline
                .map_or_else(|| "-".to_string(), |t| t.to_string()),
        );
        text.push('|');
        text.push_str(&self.database.to_string());
        let h = twox_hash::XxHash3_64::oneshot(text.as_bytes());
        (h & !0xFFFF) | u64::from((self.version / 10_000) & 0xFFFF)
    }

    /// The model watermark of a pass whose snapshot xmin is `w`.
    pub fn watermark(&self, w: u64) -> Watermark {
        Watermark::PgXid {
            w,
            identity: PgIdentity {
                system_identifier: self.word(),
                start_time_micros: self.start_micros,
            },
        }
    }
}

/// `extract(epoch FROM …)` text (`1763300000.123456`) as microseconds; a text that does not
/// parse is 0 (it then never equals a real start time, so the pass is full).
pub fn epoch_micros(s: &str) -> i64 {
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    let Ok(secs) = whole.parse::<i64>() else {
        return 0;
    };
    let mut digits: String = frac.chars().take(6).collect();
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    while digits.len() < 6 {
        digits.push('0');
    }
    let micros: i64 = digits.parse().unwrap_or(0);
    secs.saturating_mul(1_000_000)
        .saturating_add(if secs < 0 { -micros } else { micros })
}

/// What the probe of a pass saw, for the decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Now {
    pub identity: ServerIdentity,
    pub in_recovery: bool,
    pub snap_xmin: u64,
    pub snap_xmax: u64,
}

impl Now {
    pub fn of(row: &IdentityRow) -> Now {
        Now {
            identity: ServerIdentity::of(row),
            in_recovery: row.in_recovery,
            snap_xmin: row.snap_xmin,
            snap_xmax: row.snap_xmax,
        }
    }

    /// The watermark this pass records.
    pub fn watermark(&self) -> Watermark {
        self.identity.watermark(self.snap_xmin)
    }
}

/// Full or incremental.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Plan {
    Full(FullReason),
    /// Re-read rows with `age(xmin) BETWEEN 0 AND age(since)`: `since` is the previous
    /// watermark's low 32 bits (an `xid`).
    Incremental {
        since: u32,
    },
}

/// HV-INTRO-05 steps 2–3 against the previous namespace's watermark.
pub fn plan(prev: &Watermark, now: &Now) -> Plan {
    let Watermark::PgXid { w, identity } = prev else {
        return Plan::Full(FullReason::NoWatermark);
    };
    if now.in_recovery {
        return Plan::Full(FullReason::Standby);
    }
    let word = now.identity.word();
    if identity.system_identifier & !0xFFFF != word & !0xFFFF {
        return Plan::Full(FullReason::Identity);
    }
    if identity.system_identifier & 0xFFFF != word & 0xFFFF {
        return Plan::Full(FullReason::Version);
    }
    if identity.start_time_micros != now.identity.start_micros {
        return Plan::Full(FullReason::Restart);
    }
    // A watermark ahead of the server's next XID: the horizon went backwards (a restore or
    // `pg_resetwal` the identity did not reveal).
    let Some(span) = now.snap_xmax.checked_sub(*w) else {
        return Plan::Full(FullReason::Identity);
    };
    if span >= SPAN_LIMIT {
        return Plan::Full(FullReason::Wraparound);
    }
    Plan::Incremental {
        // The low 32 bits are the `xid` that `age()` takes; the span check above keeps it
        // within age()'s positive range.
        since: (*w & 0xFFFF_FFFF) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now(xmax: u64) -> Now {
        Now {
            identity: ServerIdentity {
                system_identifier: Some("7708".into()),
                timeline: Some(1),
                database: 16384,
                version: 170_012,
                start_micros: 1_763_300_000_123_456,
            },
            in_recovery: false,
            snap_xmin: xmax,
            snap_xmax: xmax,
        }
    }

    #[test]
    fn epoch_text_parses_to_micros() {
        assert_eq!(epoch_micros("1763300000.123456"), 1_763_300_000_123_456);
        assert_eq!(epoch_micros("1763300000.5"), 1_763_300_000_500_000);
        assert_eq!(epoch_micros("1763300000"), 1_763_300_000_000_000);
        assert_eq!(epoch_micros("x"), 0);
    }

    #[test]
    fn first_pass_and_foreign_watermarks_are_full() {
        assert_eq!(
            plan(&Watermark::None, &now(100)),
            Plan::Full(FullReason::NoWatermark)
        );
    }

    #[test]
    fn incremental_reads_from_the_previous_snapshot_xmin() {
        let prev = now(1000).watermark();
        assert_eq!(plan(&prev, &now(1010)), Plan::Incremental { since: 1000 });
        let prev = now((3u64 << 32) + 77).watermark();
        assert_eq!(
            plan(&prev, &now((3u64 << 32) + 90)),
            Plan::Incremental { since: 77 }
        );
    }

    #[test]
    fn a_minor_version_keeps_the_word_a_major_changes_it() {
        let a = now(1).identity;
        let mut b = a.clone();
        b.version = 170_013;
        assert_eq!(a.word(), b.word());
        b.version = 180_000;
        assert_ne!(a.word() & 0xFFFF, b.word() & 0xFFFF);
        assert_eq!(a.word() & !0xFFFF, b.word() & !0xFFFF);
    }
}
