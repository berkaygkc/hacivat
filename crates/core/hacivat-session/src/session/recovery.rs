//! Recovery after a lost connection (HV-CONN-05, D-085, B4, B19; Task 17): the reconnect
//! schedule, which failures it retries, and what state a reconnect loses.
//!
//! - **Backoff** (B19): 0, 1, 2, 4 and 8 s, then every 30 s, ±20 % jitter, until 5 min after
//!   the loss; then the profile is `Lost` (Retry). The jitter draws from `getrandom` (Q30), or
//!   from the RNG a test passes to [`RecoveryPolicy::delay`].
//! - **What is lost** is read from the first keyword of each user statement (M3 replaces the
//!   scan with the classifier): temporary objects, session variables, session locks and
//!   prepared statements. Transaction-scoped state (`SET LOCAL`, `SET TRANSACTION`,
//!   `pg_advisory_xact_lock`) goes with the transaction and is not counted.

use std::time::Duration;

use hacivat_driver::TxStatus;
use hacivat_proto::{ErrClass, ErrFamily};

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
/// shutting down or starting, and the two Karagöz upstream losses.
pub fn is_recovery_case(class: ErrClass) -> bool {
    class.family() == ErrFamily::Network
        || matches!(
            class,
            ErrClass::ServerShutdown
                | ErrClass::ServerStarting
                | ErrClass::KaragozUpstreamUnreachable
                | ErrClass::KaragozResumeExpired
        )
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
    let words = words(sql);
    let word = |i: usize| words.get(i).map(String::as_str).unwrap_or_default();
    match first_keyword(sql).as_str() {
        "CREATE" => {
            // CREATE [GLOBAL | LOCAL] {TEMP | TEMPORARY} …
            let temp = words
                .iter()
                .skip(1)
                .take(2)
                .any(|w| w == "TEMP" || w == "TEMPORARY");
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

/// The words of `sql` outside comments and quoted text, ASCII upper-cased (INV-3).
fn words(sql: &str) -> Vec<String> {
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
            '\'' | '"' | '`' => {
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
            '#' => {
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
    fn words_skip_comments_and_quotes() {
        assert_eq!(
            words("select 'it''s -- x' /* y */ \"q\"\"r\" a_b # z\n c"),
            ["SELECT", "A_B", "C"]
        );
    }
}
