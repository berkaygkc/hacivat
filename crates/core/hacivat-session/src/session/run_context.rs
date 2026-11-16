// ## m3b-t24-exec-ui (new module, the T24 I1/I2 rulings)
//! The context Run classifies a session's statements in (plan amendment C2(b)/I1; Task 24):
//! its lexing-mode setting, its search path (MySQL/MariaDB: the current database) and the
//! server version, kept **per session** and answered by the session itself
//! ([`SessionHandle::classifier_context`](crate::session::SessionHandle::classifier_context)).
//!
//! - **One source, one parser.** A read-only session answers what its guard's classifier is
//!   told (`Core::classifier_context`, Task 18). A read-write session reads the same constant
//!   text T18 reads (`readonly::read_context_again`: PostgreSQL
//!   `current_setting('standard_conforming_strings')` and `current_setting('search_path')`,
//!   split by `schema::split_search_path`; MySQL/MariaDB `@@SESSION.sql_mode` and
//!   `DATABASE()`), a read that cannot fail inside an open transaction.
//! - **Never stale.** The question is queued like any request, so it is answered after every
//!   request sent before it, and every request that may have changed the context marks it
//!   stale before the next request is taken ([`changes_run_context`]): a run with any unit not
//!   proved a read, any user statement, any transaction command, a schema switch, a reset, a
//!   reconnect. A stale context is read again before it is answered.
//! - **Fail closed, never harmful.** In a failed PostgreSQL transaction, or while a suspended
//!   cursor holds the connection, nothing is read (a read there would fail, or release the
//!   cursor): the answer is an unknown context (`ClassifierContext::default()`), which the
//!   classifier reads under every mode, with unqualified builtins unknown. The read is
//!   Hacivat's own (`observe(false)`): it never counts as a statement of the user's
//!   transaction.

use std::sync::Arc;

use hacivat_driver::{ExecSummary, SemVerLoose, SessionSettings, TxStatus};

use crate::session::actor::Core;
use crate::session::readonly::RoContextRead;
use crate::session::schema::{Family, family, split_search_path};
use crate::session::verdict::{ClassifierContext, UnitVerdict};
use crate::session::{Reply, Request};

/// A read-write session's context, as Run last read it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum RunContext {
    /// Read it again before answering (at connect, after a request that may have changed it).
    #[default]
    Stale,
    Known(SessionSettings),
}

/// Whether `request` may change the context Run classifies in: anything but Hacivat's own
/// reads, pings, cursor fetches and a run whose every unit is a proved read.
pub(crate) fn changes_run_context(request: &Request) -> bool {
    match request {
        Request::Exec { origin, .. } => *origin == crate::session::ExecOrigin::User,
        Request::Run(spec) => spec.units.iter().any(|u| u.verdict != UnitVerdict::Read),
        Request::Tx(_) | Request::SwitchSchema(_) | Request::Reset => true,
        Request::Ping
        | Request::FetchMore { .. }
        | Request::FetchAll { .. }
        | Request::CloseCursor { .. }
        | Request::ReadContext => false,
    }
}

impl Core {
    /// The session's context now (the module docs), as a [`Reply`] that [`decode`] reads.
    pub(crate) async fn answer_context(&mut self) -> Reply {
        let server_version = self
            .live
            .as_ref()
            .and_then(|live| live.server.version.clone());
        let unknown = ClassifierContext {
            server_version: server_version.clone(),
            ..ClassifierContext::default()
        };
        let busy = self.cursor.is_some() || self.tx.status == TxStatus::Failed;
        let ctx = if self.read_only {
            if busy && self.ro_context == RoContextRead::Stale {
                unknown
            } else {
                self.classifier_context().await
            }
        } else {
            if self.run_context == RunContext::Stale
                && !busy
                && let Some(read) = self.read_context_again().await
            {
                self.run_context = RunContext::Known(read);
            }
            match &self.run_context {
                RunContext::Known(read) => self.context_of(read, server_version),
                RunContext::Stale => unknown,
            }
        };
        Ok(encode(&ctx))
    }

    /// A read-write session's context from what it read, in the classifier's form (the
    /// read-only guard's rules, `readonly.rs`, without its pin).
    fn context_of(
        &self,
        read: &SessionSettings,
        server_version: Option<SemVerLoose>,
    ) -> ClassifierContext {
        match family(self.kind) {
            Family::Postgres => ClassifierContext {
                lex_modes: read.lex_modes.clone(),
                search_path: read
                    .search_path
                    .as_deref()
                    .map(|s| split_search_path(s).into_iter().map(Arc::from).collect()),
                server_version,
            },
            Family::MySql => ClassifierContext {
                lex_modes: read.lex_modes.clone(),
                search_path: read.search_path.clone().map(|db| Arc::from(vec![db])),
                server_version,
            },
            Family::Sqlite | Family::Other => ClassifierContext {
                lex_modes: None,
                search_path: None,
                server_version,
            },
        }
    }
}

/// `ctx` as a reply: row 0 holds the mode setting, whether a search path is known, and the
/// version (major, minor, patch, raw); each later row one element of the path.
fn encode(ctx: &ClassifierContext) -> ExecSummary {
    let v = ctx.server_version.as_ref();
    let mut rows = vec![vec![
        ctx.lex_modes.as_deref().map(str::to_string),
        ctx.search_path.as_ref().map(|_| "1".to_string()),
        v.map(|v| v.major.to_string()),
        v.map(|v| v.minor.to_string()),
        v.map(|v| v.patch.to_string()),
        v.map(|v| v.raw.clone()),
    ]];
    for element in ctx.search_path.iter().flat_map(|p| p.iter()) {
        rows.push(vec![Some(element.to_string())]);
    }
    ExecSummary {
        row_count: rows.len() as u64,
        rows,
        ..ExecSummary::default()
    }
}

/// The context an [`encode`]d reply holds; an unreadable one is unknown.
pub(crate) fn decode(s: &ExecSummary) -> ClassifierContext {
    let Some(head) = s.rows.first() else {
        return ClassifierContext::default();
    };
    let cell = |i: usize| head.get(i).cloned().flatten();
    let num = |i: usize| cell(i).and_then(|n| n.parse::<u32>().ok());
    let server_version = match (num(2), num(3), num(4), cell(5)) {
        (Some(major), Some(minor), Some(patch), Some(raw)) => Some(SemVerLoose {
            major,
            minor,
            patch,
            raw,
        }),
        _ => None,
    };
    let search_path = cell(1).map(|_| {
        s.rows[1..]
            .iter()
            .map(|r| Arc::from(r.first().cloned().flatten().unwrap_or_default()))
            .collect()
    });
    ClassifierContext {
        lex_modes: cell(0).map(Arc::from),
        search_path,
        server_version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_context_round_trips_through_its_reply() {
        let ctx = ClassifierContext {
            lex_modes: Some(Arc::from("on")),
            search_path: Some(Arc::from(vec![Arc::from("$user"), Arc::from("public")])),
            server_version: Some(SemVerLoose {
                major: 16,
                minor: 4,
                patch: 0,
                raw: "16.4".into(),
            }),
        };
        assert_eq!(decode(&encode(&ctx)), ctx);
        let empty = ClassifierContext {
            search_path: Some(Arc::from(Vec::<Arc<str>>::new())),
            ..ClassifierContext::default()
        };
        assert_eq!(
            decode(&encode(&empty)),
            empty,
            "an empty path stays known and empty"
        );
        assert_eq!(
            decode(&encode(&ClassifierContext::default())),
            ClassifierContext::default()
        );
        assert_eq!(
            decode(&ExecSummary::default()),
            ClassifierContext::default()
        );
    }
}
