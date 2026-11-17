//! The incremental watermark and the full-or-incremental decision (HV-INTRO-05 steps 1–3).
//!
//! The watermark `w` is the **snapshot xmin** of the pass's `REPEATABLE READ READ ONLY`
//! transaction (`pg_snapshot_xmin(pg_current_snapshot())`, 64-bit, epoch-extended): every
//! transaction below it had finished when the pass read the catalog, so the next pass needs
//! to re-read only rows written by transactions `>= w`. Snapshot functions never assign an XID
//! (ver/01 #pg-canary); `txid_current()` would.

use super::IntroError;
use super::decode::Row;
use super::model::Oid;
use super::sql::PgVersion;

/// Who the server is: when any of it changes, the XID horizon of the old watermark cannot be
/// trusted (restore, `pg_resetwal`, failover, PITR, another server), so the pass is full.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PgIdentity {
    /// `pg_control_system().system_identifier`, when the role may call it.
    pub system_identifier: Option<String>,
    /// `pg_control_checkpoint().timeline_id`, when the role may call it.
    pub timeline: Option<u32>,
    /// `pg_postmaster_start_time()` as epoch seconds (text, exact).
    pub postmaster_start: String,
    pub database: Oid,
}

/// A namespace's incremental watermark (`Watermark::PgXid`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PgWatermark {
    /// The snapshot xmin of the pass that produced the snapshot.
    pub w: u64,
    pub version: PgVersion,
    pub identity: PgIdentity,
}

/// What the probe of a pass read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub version: PgVersion,
    pub in_recovery: bool,
    pub identity: PgIdentity,
    pub snap_xmin: u64,
    pub snap_xmax: u64,
    pub namespace_exists: bool,
}

impl Probe {
    pub fn decode(r: &Row<'_>) -> Result<Probe, IntroError> {
        Ok(Probe {
            version: PgVersion(r.num(0)?),
            in_recovery: r.bool(1)?,
            identity: PgIdentity {
                postmaster_start: r.string(2)?,
                database: r.num(3)?,
                system_identifier: r.opt_string(4),
                timeline: r.opt_num(5)?,
            },
            snap_xmin: r.num(6)?,
            snap_xmax: r.num(7)?,
            namespace_exists: r.bool(8)?,
        })
    }

    /// The watermark a pass with this probe records.
    pub fn watermark(&self) -> PgWatermark {
        PgWatermark {
            w: self.snap_xmin,
            version: self.version,
            identity: self.identity.clone(),
        }
    }
}

/// Why a pass is full (stable codes for the UI's `intro-full-reason`, HV-INTRO-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FullReason {
    /// No previous watermark (first load).
    NoWatermark,
    /// The server restarted since the last pass.
    Restart,
    /// Another server, database, timeline or cluster.
    Identity,
    /// The server's version changed.
    Version,
    /// The server is a hot standby (incremental is not verified there).
    Standby,
    /// The watermark is too old for 32-bit `age()` arithmetic.
    Wraparound,
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
        }
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

/// The largest span (in XIDs) from the previous watermark to the server's next XID that is
/// still incremental: `age()` is 32-bit and turns negative past 2^31, so beyond it a pass is
/// full. The 2^20 slack covers XIDs assigned between the probe and the change query.
pub const SPAN_LIMIT: u64 = (1 << 31) - (1 << 20);

/// HV-INTRO-05 steps 2–3.
pub fn plan(prev: Option<&PgWatermark>, now: &Probe) -> Plan {
    let Some(prev) = prev else {
        return Plan::Full(FullReason::NoWatermark);
    };
    if now.in_recovery {
        return Plan::Full(FullReason::Standby);
    }
    if prev.version != now.version {
        return Plan::Full(FullReason::Version);
    }
    let (a, b) = (&prev.identity, &now.identity);
    if a.database != b.database
        || a.system_identifier != b.system_identifier
        || a.timeline != b.timeline
    {
        return Plan::Full(FullReason::Identity);
    }
    if a.postmaster_start != b.postmaster_start {
        return Plan::Full(FullReason::Restart);
    }
    // A watermark ahead of the server's next XID: the horizon went backwards (a restore the
    // identity did not reveal).
    let Some(span) = now.snap_xmax.checked_sub(prev.w) else {
        return Plan::Full(FullReason::Identity);
    };
    if span >= SPAN_LIMIT {
        return Plan::Full(FullReason::Wraparound);
    }
    Plan::Incremental {
        // The low 32 bits are the `xid` that `age()` takes; the span check above keeps it
        // within age()'s positive range.
        since: (prev.w & 0xFFFF_FFFF) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(xmax: u64) -> Probe {
        Probe {
            version: PgVersion(170_012),
            in_recovery: false,
            identity: PgIdentity {
                system_identifier: Some("7708".into()),
                timeline: Some(1),
                postmaster_start: "1763300000.123456".into(),
                database: 16384,
            },
            snap_xmin: xmax,
            snap_xmax: xmax,
            namespace_exists: true,
        }
    }

    #[test]
    fn first_pass_is_full() {
        assert_eq!(plan(None, &probe(100)), Plan::Full(FullReason::NoWatermark));
    }

    #[test]
    fn incremental_reads_from_the_previous_snapshot_xmin() {
        let prev = probe(1000).watermark();
        assert_eq!(
            plan(Some(&prev), &probe(1010)),
            Plan::Incremental { since: 1000 }
        );
        assert_eq!(
            plan(Some(&prev), &probe(1000)),
            Plan::Incremental { since: 1000 }
        );
        // An epoch-extended watermark passes its low 32 bits (the xid `age()` takes).
        let prev = probe((3u64 << 32) + 77).watermark();
        assert_eq!(
            plan(Some(&prev), &probe((3u64 << 32) + 90)),
            Plan::Incremental { since: 77 }
        );
    }

    #[test]
    fn identity_restart_version_and_standby_force_full() {
        let prev = probe(1000).watermark();
        let mut p = probe(1010);
        p.identity.postmaster_start = "1763300999.0".into();
        assert_eq!(plan(Some(&prev), &p), Plan::Full(FullReason::Restart));
        let mut p = probe(1010);
        p.identity.system_identifier = Some("other".into());
        assert_eq!(plan(Some(&prev), &p), Plan::Full(FullReason::Identity));
        let mut p = probe(1010);
        p.identity.timeline = Some(2);
        assert_eq!(plan(Some(&prev), &p), Plan::Full(FullReason::Identity));
        let mut p = probe(1010);
        p.identity.database = 1;
        assert_eq!(plan(Some(&prev), &p), Plan::Full(FullReason::Identity));
        let mut p = probe(1010);
        p.version = PgVersion(170_013);
        assert_eq!(plan(Some(&prev), &p), Plan::Full(FullReason::Version));
        let mut p = probe(1010);
        p.in_recovery = true;
        assert_eq!(plan(Some(&prev), &p), Plan::Full(FullReason::Standby));
    }

    #[test]
    fn a_horizon_that_went_backwards_is_an_identity_change() {
        // pg_resetwal -x to a lower XID, or a restore the identity fields could not reveal
        // (e.g. a role without pg_control_system()).
        let prev = probe(5000).watermark();
        assert_eq!(
            plan(Some(&prev), &probe(4000)),
            Plan::Full(FullReason::Identity)
        );
    }

    #[test]
    fn a_watermark_older_than_the_age_range_forces_full() {
        // ver/01 #pg-xmin-freeze: pg_resetwal -x 2^31+2000 jumps the XID counter past the
        // 32-bit age range; the next pass cannot use age() against the old watermark.
        let prev = probe(738).watermark();
        let jumped = probe((1u64 << 31) + 2000);
        assert_eq!(
            plan(Some(&prev), &jumped),
            Plan::Full(FullReason::Wraparound)
        );
        // Just inside the limit is still incremental.
        assert_eq!(
            plan(Some(&prev), &probe(738 + SPAN_LIMIT - 1)),
            Plan::Incremental { since: 738 }
        );
        assert_eq!(
            plan(Some(&prev), &probe(738 + SPAN_LIMIT)),
            Plan::Full(FullReason::Wraparound)
        );
    }

    #[test]
    fn reason_codes_are_stable() {
        assert_eq!(FullReason::Wraparound.code(), "wraparound");
        assert_eq!(FullReason::NoWatermark.code(), "no-watermark");
    }
}
