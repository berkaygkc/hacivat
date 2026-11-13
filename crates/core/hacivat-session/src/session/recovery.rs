//! Recovery after a lost connection (HV-CONN-05, D-085, B4, B19; Task 17): the reconnect
//! schedule, which failures it retries, and what state a reconnect loses.
//!
//! - **Backoff** (B19): 0, 1, 2, 4 and 8 s, then every 30 s, ±20 % jitter, until 5 min after
//!   the loss; then the profile is `Lost` (Retry). The jitter draws from `getrandom` (Q30), or
//!   from the RNG a test passes to [`RecoveryPolicy::delay`].
//! - **What is lost** is read from the first keyword of each user statement (M3 replaces the
//!   scan with the classifier): temporary objects, session variables, session locks and
//!   prepared statements. Transaction-scoped state (`SET LOCAL`, `SET TRANSACTION`,
//!   `pg_advisory_xact_lock`) goes with the transaction and is not counted. When the scan cannot
//!   be sure where a string ends (a backslash inside quotes: MySQL escapes, PostgreSQL does
//!   not), it also reads quoted text, so it errs toward reporting more (fix round 1, M5). `#` is
//!   not taken for a comment (it is an operator in PostgreSQL).

use std::time::Duration;

use hacivat_driver::{RawError, TxStatus};
use hacivat_proto::{DbmsKind, ErrClass, ErrFamily};

use crate::session::schema::first_keyword;

/// The reconnect schedule.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryPolicy {
    /// The first delays, in order.
    pub schedule: Vec<Duration>,
    /// The delay after the schedule runs out.
    pub steady: Duration,
    pub give_up_after: Duration,
    /// The jitter fraction (0.2 is ±20 %).
    pub jitter: f32,
}

impl Default for RecoveryPolicy {
    /// B19: `[0, 1, 2, 4, 8] s`, then every 30 s, giving up after 300 s, jitter 0.2.
    fn default() -> Self {
        RecoveryPolicy {
            schedule: [0, 1, 2, 4, 8].map(Duration::from_secs).to_vec(),
            steady: Duration::from_secs(30),
            give_up_after: Duration::from_secs(300),
            jitter: 0.2,
        }
    }
}

impl RecoveryPolicy {
    /// The delay before reconnect `attempt` (from 0: the first one waits `schedule[0]`),
    /// `elapsed` after the loss, with `rng` (in `[0, 1]`) spreading it by ±`jitter`. `None`
    /// gives up: `elapsed` reached `give_up_after`.
    pub fn delay(
        &self,
        attempt: u32,
        elapsed: Duration,
        rng: &mut dyn FnMut() -> f32,
    ) -> Option<Duration> {
        if elapsed >= self.give_up_after {
            return None;
        }
        let base = usize::try_from(attempt)
            .ok()
            .and_then(|i| self.schedule.get(i))
            .copied()
            .unwrap_or(self.steady);
        let r = rng().clamp(0.0, 1.0);
        let factor = (1.0 + f64::from(self.jitter) * (2.0 * f64::from(r) - 1.0)).max(0.0);
        // Whole microseconds: `0.2f32` is not exactly 0.2, and a delay must not lose its last
        // millisecond to that.
        let micros = (base.as_micros() as f64 * factor).round();
        Some(Duration::from_micros(micros as u64))
    }

    /// The production jitter source: a uniform value in `[0, 1)` from the OS (`getrandom`,
    /// Q30); 0.5 (no jitter) if the OS has none to give.
    pub fn jitter_source() -> impl FnMut() -> f32 {
        || {
            let mut bytes = [0u8; 4];
            if getrandom::fill(&mut bytes).is_err() {
                return 0.5;
            }
            // 24 bits: exactly representable in an `f32`.
            let bits = u32::from_le_bytes(bytes) >> 8;
            bits as f32 / (1u32 << 24) as f32
        }
    }
}

/// Whether a lost connection of `class` is worth reconnecting: the network, a server that is
/// shutting down or starting, the two Karagöz upstream losses, and (plan amendment, fix round
/// 1, I3) `ssh.channel_open`: a bastion that cannot reach the database while it restarts.
///
/// Only recovery asks: it reconnects a connection that worked, so a first connect that fails
/// with any of these is still a plain failure.
pub fn is_recovery_case(class: ErrClass) -> bool {
    class.family() == ErrFamily::Network
        || matches!(
            class,
            ErrClass::ServerShutdown
                | ErrClass::ServerStarting
                | ErrClass::KaragozUpstreamUnreachable
                | ErrClass::KaragozResumeExpired
                | ErrClass::SshChannelOpen
        )
}

// ## m2b-t17-liveness (fix round 2)

/// MySQL and MariaDB server errors that come with the end of the connection (I6): the server
/// is shutting down (1053), the connection was killed (1927, 3169), or the client sat idle too
/// long (4031); and the client's own server-gone codes (2006, 2013), should a driver ever
/// report them as server errors.
const MYSQL_CONNECTION_ENDING: [(i64, ErrClass); 6] = [
    (1053, ErrClass::ServerShutdown),
    (1927, ErrClass::NetworkReset),
    (2006, ErrClass::NetworkReset),
    (2013, ErrClass::NetworkReset),
    (3169, ErrClass::NetworkReset),
    (4031, ErrClass::NetworkReset),
];

/// The class of the lost connection when the server error `raw` (classified `class`) ended it
/// (fix round 2, I6): a recovery case after connect (PostgreSQL's `57P01`/`57P02`
/// `server.shutdown`, `57P03` `server.starting`, `08006` `network.reset`), or one of MySQL's
/// connection-ending codes. `None`: an ordinary statement error.
///
/// Fix round 3 (I7): on PostgreSQL only a `FATAL` or `PANIC` ends the connection. An `ERROR`
/// with one of these SQLSTATEs is a remote server's failure relayed by `postgres_fdw` or
/// `dblink`, and the local session (and its transaction) is fine. The severity is the
/// server's `S` field: the non-localized `V` unless the server translates it; a translated
/// one that does not read `FATAL` is found closed at the next round trip instead (safe).
pub(crate) fn connection_ending(
    kind: DbmsKind,
    raw: &RawError,
    class: ErrClass,
) -> Option<ErrClass> {
    if kind == DbmsKind::Postgres {
        let severity = raw.server.as_ref()?.severity.as_deref()?;
        let fatal = ["FATAL", "PANIC"]
            .iter()
            .any(|s| severity.eq_ignore_ascii_case(s));
        return (fatal && is_recovery_case(class)).then_some(class);
    }
    if is_recovery_case(class) {
        return Some(class);
    }
    if !matches!(kind, DbmsKind::MySql | DbmsKind::MariaDb) {
        return None;
    }
    let code = raw.server.as_ref()?.vendor_code?;
    MYSQL_CONNECTION_ENDING
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, class)| *class)
}

/// Whether `sql` may open or end a transaction anywhere in it (fix round 2, review R1): any of
/// its `;`-separated statements starts with `BEGIN` or `START`, or sets `autocommit` (MySQL: an
/// implicit transaction). A `;` inside quotes splits too, which only errs toward "may".
pub(crate) fn touches_tx(sql: &str) -> bool {
    sql.split(';').any(|statement| {
        let words = words(statement, true);
        match words.first().map(String::as_str) {
            // `XA START` / `XA BEGIN` open an XA transaction (fix round 3, R3).
            Some("BEGIN" | "START" | "XA") => true,
            Some("SET") => words.iter().any(|w| w == "AUTOCOMMIT"),
            _ => false,
        }
    })
}

/// What a user statement leaves MySQL's `autocommit` at (fix round 3, I8): `Some(false)` after
/// `SET [SESSION | LOCAL | @@session. | @@local. | @@]autocommit = 0 | OFF | FALSE`, `Some(true)`
/// after `= 1 | ON | TRUE | DEFAULT`, the last one in a batch winning; `None` when it does not
/// set it. A `SET GLOBAL` changes new sessions only, not this one.
pub(crate) fn autocommit_set(sql: &str) -> Option<bool> {
    let mut last = None;
    for statement in sql.split(';') {
        let words = words(statement, true);
        if words.first().map(String::as_str) != Some("SET") {
            continue;
        }
        for (i, w) in words.iter().enumerate() {
            // Only a modifier right before it makes it global (`SET GLOBAL autocommit`,
            // `@@global.autocommit`); any doubt reads as this session's, which errs toward
            // "off" being remembered (the safe side).
            let global =
                i > 0 && matches!(words[i - 1].as_str(), "GLOBAL" | "PERSIST" | "PERSIST_ONLY");
            if w != "AUTOCOMMIT" || global {
                continue;
            }
            last = match words.get(i + 1).map(String::as_str) {
                Some("0" | "OFF" | "FALSE") => Some(false),
                Some("1" | "ON" | "TRUE" | "DEFAULT") => Some(true),
                _ => last,
            };
        }
    }
    last
}

/// What a session did that a reconnect would lose (a first-keyword scan, B19).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Tracked {
    pub tx: TxStatus,
    pub temp_created: bool,
    pub set_seen: bool,
    pub prepared: u32,
    pub locks_seen: bool,
}

/// Notes what the user statement `sql` does to the session's state (the module docs).
pub fn track_statement(t: &mut Tracked, sql: &str) {
    // A backslash in quoted text leaves the end of the string unsure: read the quotes too.
    let words = words(sql, !sql.contains('\\'));
    let word = |i: usize| words.get(i).map(String::as_str).unwrap_or_default();
    match first_keyword(sql).as_str() {
        "CREATE" => {
            // CREATE [OR REPLACE] [GLOBAL | LOCAL] {TEMP | TEMPORARY} …
            let temp = words
                .iter()
                .skip(1)
                .find(|w| !matches!(w.as_str(), "OR" | "REPLACE" | "GLOBAL" | "LOCAL"))
                .is_some_and(|w| w == "TEMP" || w == "TEMPORARY");
            t.temp_created |= temp;
        }
        "SET" => {
            if !matches!(word(1), "LOCAL" | "TRANSACTION") {
                t.set_seen = true;
            }
        }
        "LOCK" => t.locks_seen = true,
        "PREPARE" => t.prepared = t.prepared.saturating_add(1),
        "DEALLOCATE" => {
            let all = word(1) == "ALL" || (word(1) == "PREPARE" && word(2) == "ALL");
            t.prepared = if all { 0 } else { t.prepared.saturating_sub(1) };
        }
        _ => {}
    }
    if words.iter().any(|w| SESSION_LOCKS.contains(&w.as_str())) {
        t.locks_seen = true;
    }
}

/// Functions that take a lock held until the session ends (not the `_xact_` ones).
const SESSION_LOCKS: [&str; 6] = [
    "PG_ADVISORY_LOCK",
    "PG_ADVISORY_LOCK_SHARED",
    "PG_TRY_ADVISORY_LOCK",
    "PG_TRY_ADVISORY_LOCK_SHARED",
    "GET_LOCK",
    "SP_GETAPPLOCK",
];

/// The words of `sql` outside comments (and outside quoted text when `skip_quotes`), ASCII
/// upper-cased (INV-3).
fn words(sql: &str, skip_quotes: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut chars = sql.chars().peekable();
    let flush = |word: &mut String, out: &mut Vec<String>| {
        if !word.is_empty() {
            out.push(std::mem::take(word).to_ascii_uppercase());
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' | '`' if skip_quotes => {
                flush(&mut word, &mut out);
                // To the closing quote; a doubled quote stays inside.
                while let Some(q) = chars.next() {
                    if q == c {
                        if chars.peek() == Some(&c) {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                flush(&mut word, &mut out);
                for n in chars.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                flush(&mut word, &mut out);
                chars.next();
                let mut star = false;
                for n in chars.by_ref() {
                    if star && n == '/' {
                        break;
                    }
                    star = n == '*';
                }
            }
            c if c.is_ascii_alphanumeric() || c == '_' => word.push(c),
            _ => flush(&mut word, &mut out),
        }
    }
    flush(&mut word, &mut out);
    out
}

/// What a reconnect loses, by what the session did.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct LostState {
    pub open_tx: bool,
    pub temp_objects: bool,
    pub session_vars: bool,
    pub prepared: u32,
    pub locks: bool,
}

impl LostState {
    /// Nothing is lost.
    pub fn is_empty(&self) -> bool {
        *self == LostState::default()
    }
}

/// What a reconnect after `t` loses.
pub fn lost_state(t: &Tracked) -> LostState {
    LostState {
        open_tx: t.tx != TxStatus::Idle,
        temp_objects: t.temp_created,
        session_vars: t.set_seen,
        prepared: t.prepared,
        locks: t.locks_seen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_state_follows_what_was_tracked() {
        assert!(lost_state(&Tracked::default()).is_empty());
        let t = Tracked {
            tx: TxStatus::InTx,
            prepared: 2,
            ..Tracked::default()
        };
        let lost = lost_state(&t);
        assert!(lost.open_tx && lost.prepared == 2 && !lost.is_empty());
    }

    #[test]
    fn a_users_autocommit_setting_is_read_in_every_spelling() {
        for off in [
            "SET autocommit = 0",
            "set autocommit=OFF",
            "SET SESSION autocommit = 0",
            "SET @@session.autocommit = 0",
            "SET @@autocommit := FALSE",
            "SET LOCAL autocommit = 0",
            "SET @x = 1, autocommit = 0",
            "SET autocommit = 1; SET autocommit = 0",
        ] {
            assert_eq!(autocommit_set(off), Some(false), "{off}");
        }
        for on in [
            "SET autocommit = 1",
            "SET autocommit = ON",
            "SET @@session.autocommit = DEFAULT",
            "SET autocommit = 0; SET autocommit = 1",
        ] {
            assert_eq!(autocommit_set(on), Some(true), "{on}");
        }
        for neither in [
            "SET GLOBAL autocommit = 0",
            "SET @@global.autocommit = 0",
            "SET work_mem = '1MB'",
            "SELECT 'SET autocommit = 0'",
            "UPDATE t SET autocommit = 0",
        ] {
            assert_eq!(autocommit_set(neither), None, "{neither}");
        }
    }

    #[test]
    fn a_transaction_anywhere_in_a_batch_is_seen() {
        assert!(touches_tx("XA START 'x'"));
        assert!(touches_tx("xa begin 'x'"));
        assert!(touches_tx("BEGIN"));
        assert!(touches_tx(
            "insert into t values (1); begin; update t set a = 2"
        ));
        assert!(touches_tx("/* go */ start transaction"));
        assert!(touches_tx("SET autocommit = 0"));
        assert!(touches_tx("set @@session.autocommit=0"));
        assert!(!touches_tx("SELECT start, begin FROM t"));
        assert!(!touches_tx("SET work_mem = '1MB'"));
        assert!(!touches_tx("UPDATE t SET a = 2"));
    }

    #[test]
    fn connection_ending_server_errors() {
        let mysql = |code: i64| RawError {
            kind: hacivat_driver::RawErrorKind::Db,
            server: Some(hacivat_driver::ServerMessage {
                sqlstate: None,
                vendor_code: Some(code),
                severity: None,
                text: String::new(),
                detail: None,
                hint: None,
                lang_hint: None,
            }),
        };
        let internal = ErrClass::Internal;
        assert_eq!(
            connection_ending(DbmsKind::MySql, &mysql(4031), internal),
            Some(ErrClass::NetworkReset)
        );
        assert_eq!(
            connection_ending(DbmsKind::MariaDb, &mysql(1053), internal),
            Some(ErrClass::ServerShutdown)
        );
        assert_eq!(
            connection_ending(DbmsKind::MySql, &mysql(1062), internal),
            None
        );
        // PostgreSQL decides by class (57P01 is `server.shutdown`) and severity (I7); a vendor
        // code means nothing.
        let pg = |severity: Option<&str>| RawError {
            kind: hacivat_driver::RawErrorKind::Db,
            server: Some(hacivat_driver::ServerMessage {
                sqlstate: Some("57P01".into()),
                vendor_code: Some(4031),
                severity: severity.map(str::to_string),
                text: String::new(),
                detail: None,
                hint: None,
                lang_hint: None,
            }),
        };
        let shutdown = ErrClass::ServerShutdown;
        for fatal in ["FATAL", "PANIC", "fatal"] {
            assert_eq!(
                connection_ending(DbmsKind::Postgres, &pg(Some(fatal)), shutdown),
                Some(shutdown)
            );
        }
        // A relayed remote failure (`postgres_fdw`, `dblink`), a translated severity, none.
        for not_fatal in [Some("ERROR"), Some("ÖLÜMCÜL"), None] {
            assert_eq!(
                connection_ending(DbmsKind::Postgres, &pg(not_fatal), shutdown),
                None
            );
        }
        assert_eq!(
            connection_ending(DbmsKind::Postgres, &pg(Some("FATAL")), internal),
            None
        );
    }

    #[test]
    fn words_skip_comments_and_quotes() {
        assert_eq!(
            words("select 'it''s -- x' /* y */ \"q\"\"r\" a_b # z\n c", true),
            ["SELECT", "A_B", "Z", "C"]
        );
        assert_eq!(words("select 'x' y", false), ["SELECT", "X", "Y"]);
    }
}
