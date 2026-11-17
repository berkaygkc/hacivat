// ## m3b-t26-prod-unlock (new module; the T12 ruling carried to Task 26 as MUST 3)
//! A production profile's startup script (Task 12, C3) may run only statements the classifier
//! rates as harmless: a `Read` or a `Session` statement on the harmless-`SET` list, exactly what
//! the read-only check allows (`StatementClassifier::check_read_only_in`, P-M3b-6). Until Task 26
//! a production session was read-only and never ran its script; since then it is read-write, so
//! the script would otherwise reach a production database unconfirmed, on every connect, reset
//! and reconnect.
//!
//! - **Through the pool, in the session's context.** Each statement is checked by the session's
//!   classifier (`ManagerConfig::classifier`: the app's `ClassifyPool`) in the context the
//!   session reads now (its lexing-mode setting, search path or current database, version); a
//!   statement the classifier says may change that context leaves the later ones checked in an
//!   unknown context (every mode, unqualified builtins unknown: fail closed).
//! - **Fail closed.** A session without a classifier runs no production script; one statement
//!   refused refuses all of it (none is sent) and the session (`config.invalid`). The warning
//!   names the statement's number in the script, never its text.
//! - **Either tag.** A script is checked when the profile the session opened with or the
//!   profile's live tag (`ProfileActivity::production`, set by `SessionManager::update_profile`)
//!   is production (fix round 1, A-I1): a session opened before a retag checks the script it
//!   replays on its own reset or reconnect, and the retag closes the idle pooled sessions
//!   (ADR 0004 `m3b-t26`).

use std::sync::Arc;

use hacivat_proto::EnvTag;

use crate::session::actor::Core;
use crate::session::verdict::{ClassifierContext, RoCheck};

impl Core {
    /// The number (from 1) of the first statement of `statements` a production profile's
    /// startup script may not run (the module docs); `None` when every one may, or neither the
    /// opening nor the live tag is production.
    pub(crate) async fn refused_startup_statement(
        &mut self,
        statements: &[String],
    ) -> Option<usize> {
        let production = self.profile.environment == Some(EnvTag::Production)
            || self.live_env.as_ref().is_some_and(|a| a.production());
        if !production || statements.is_empty() {
            return None;
        }
        let Some(classifier) = self.classifier.clone() else {
            // Nothing can check it: nothing of it runs.
            return Some(1);
        };
        let allow: Arc<[Arc<str>]> = self
            .profile
            .read_only_fn_allowlist
            .iter()
            .map(|name| Arc::from(name.as_str()))
            .collect();
        let mut ctx = self.read_write_context().await;
        for (i, sql) in statements.iter().enumerate() {
            let answer = classifier
                .check_read_only_in(
                    self.kind,
                    Arc::from(sql.as_str()),
                    allow.clone(),
                    ctx.clone(),
                )
                .await;
            match answer.check {
                RoCheck::Allow => {
                    if answer.changes_context {
                        ctx = ClassifierContext {
                            lex_modes: None,
                            search_path: None,
                            server_version: ctx.server_version.clone(),
                        };
                    }
                }
                RoCheck::Deny(_) => return Some(i + 1),
            }
        }
        None
    }
}
