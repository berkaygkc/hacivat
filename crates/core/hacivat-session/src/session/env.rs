//! The session environment (HV-SESS-06; Task 7): the statements a session replays after a reset
//! or a reconnect (client identity, read-only settings, time zone, the last schema), and the
//! quoting functions every control statement uses.
//!
//! A fresh session runs none of it (Q27): connect already set the identity and read-only. A
//! read-only session's guard replays the plan without the PostgreSQL client identity after each
//! statement (Q15), since `DISCARD ALL` restores the startup `application_name`.

use hacivat_guard::plan_for;
use hacivat_proto::DbmsKind;

use crate::profile::Profile;
use crate::session::SchemaCtx;
use crate::session::schema::{Family, family, switch_schema_sql};

/// One environment step.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EnvStep {
    /// The application name the server shows (PostgreSQL `application_name`).
    ClientIdentity(String),
    /// The engine's read-only session settings.
    ReadOnly(Vec<String>),
    TimeZone(String),
    RestoreSchema(String),
    /// A user startup script (M3).
    StartupScript,
}

/// A session's environment, in order, for the engine it runs on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnvPlan {
    /// The engine whose SQL the steps become.
    pub kind: DbmsKind,
    pub steps: Vec<EnvStep>,
}

impl Default for EnvPlan {
    /// No steps.
    fn default() -> Self {
        EnvPlan {
            kind: DbmsKind::Unknown,
            steps: Vec::new(),
        }
    }
}

impl EnvPlan {
    /// The plan as statements, in order; each literal goes through its quoting function.
    pub fn statements(&self) -> Vec<String> {
        self.steps
            .iter()
            .flat_map(|step| step_statements(self.kind, step))
            .collect()
    }

    /// What a read-only statement's guard replays after its reset (Q15): every statement but
    /// the client identity, which the startup parameter restores.
    pub fn replay(&self) -> Vec<String> {
        self.steps
            .iter()
            .filter(|step| !matches!(step, EnvStep::ClientIdentity(_)))
            .flat_map(|step| step_statements(self.kind, step))
            .collect()
    }
}

fn step_statements(kind: DbmsKind, step: &EnvStep) -> Vec<String> {
    match (step, family(kind)) {
        (EnvStep::ClientIdentity(name), Family::Postgres) => {
            vec![format!("SET application_name = {}", quote_pg_literal(name))]
        }
        (EnvStep::ReadOnly(statements), _) => statements.clone(),
        (EnvStep::TimeZone(zone), Family::Postgres) => {
            vec![format!("SET TIME ZONE {}", quote_pg_literal(zone))]
        }
        (EnvStep::TimeZone(zone), Family::MySql) => {
            vec![format!("SET time_zone = {}", quote_my_literal(zone))]
        }
        (EnvStep::RestoreSchema(name), _) => {
            let sql = switch_schema_sql(kind, name);
            if sql.is_empty() {
                Vec::new()
            } else {
                vec![sql]
            }
        }
        _ => Vec::new(),
    }
}

/// The environment of a `kind` session for `profile`:
/// 1. the client identity (PostgreSQL; MySQL's connect attribute already carries it);
/// 2. the read-only setup of `kind`'s guard plan, when `read_only`;
/// 3. the time zone: none until the settings model (M3);
/// 4. the last schema, only when one is known (Q27).
pub fn build_env_plan(
    kind: DbmsKind,
    profile: &Profile,
    read_only: bool,
    last_schema: Option<&SchemaCtx>,
) -> EnvPlan {
    let mut steps = Vec::new();
    if family(kind) == Family::Postgres {
        steps.push(EnvStep::ClientIdentity(profile.application_name.clone()));
    }
    if read_only && let Some(plan) = plan_for(kind) {
        let setup: Vec<String> = plan.session_setup.iter().map(|s| s.to_string()).collect();
        if !setup.is_empty() {
            steps.push(EnvStep::ReadOnly(setup));
        }
    }
    if let Some(name) = last_schema.and_then(|s| s.schema.as_ref())
        && !switch_schema_sql(kind, name).is_empty()
    {
        steps.push(EnvStep::RestoreSchema(name.clone()));
    }
    EnvPlan { kind, steps }
}

/// `s` without NUL characters: no engine stores one in a name or a setting, and the server
/// would end its C string at it.
fn without_nul(s: &str) -> String {
    s.replace('\0', "")
}

/// A PostgreSQL identifier: `"`, with `"` doubled.
pub fn quote_pg_ident(s: &str) -> String {
    format!("\"{}\"", without_nul(s).replace('"', "\"\""))
}

/// A MySQL identifier: `` ` ``, with `` ` `` doubled.
pub fn quote_my_ident(s: &str) -> String {
    format!("`{}`", without_nul(s).replace('`', "``"))
}

/// A PostgreSQL string literal: `'`, with `'` doubled. With a backslash it becomes an `E''`
/// literal with backslashes doubled, so it reads the same whatever
/// `standard_conforming_strings` is.
pub fn quote_pg_literal(s: &str) -> String {
    let s = without_nul(s);
    let body = s.replace('\'', "''");
    if s.contains('\\') {
        format!("E'{}'", body.replace('\\', "\\\\"))
    } else {
        format!("'{body}'")
    }
}

/// A MySQL string literal: `'`, with `'` doubled and backslashes doubled (safe with and without
/// `NO_BACKSLASH_ESCAPES`: the quote can never be escaped away).
pub fn quote_my_literal(s: &str) -> String {
    format!(
        "'{}'",
        without_nul(s).replace('\\', "\\\\").replace('\'', "''")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_zone_and_startup_steps_render_per_engine() {
        let tz = |kind| EnvPlan {
            kind,
            steps: vec![
                EnvStep::TimeZone("Europe/Istanbul".into()),
                EnvStep::StartupScript,
            ],
        };
        assert_eq!(
            tz(DbmsKind::Postgres).statements(),
            ["SET TIME ZONE 'Europe/Istanbul'"]
        );
        assert_eq!(
            tz(DbmsKind::MySql).statements(),
            ["SET time_zone = 'Europe/Istanbul'"]
        );
        assert!(tz(DbmsKind::Sqlite).statements().is_empty());
        assert_eq!(quote_my_literal("a\\'b"), "'a\\\\''b'");
    }
}
