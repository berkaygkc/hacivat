//! The session environment (HV-SESS-06; Task 7): the statements a session replays after a reset
//! or a reconnect (client identity, read-only settings, time zone, the last schema), and the
//! quoting functions every control statement uses.
//!
//! A fresh session runs none of it (Q27): connect already set the identity and read-only. A
//! read-only session's guard replays the plan without the PostgreSQL client identity after each
//! statement (Q15), since `DISCARD ALL` restores the startup `application_name`.
//!
//! ## m3b-t12-session-carries (ADR 0004 `m3b-t12`)
//! - **C11:** a PostgreSQL session with a time zone from the settings (`ui.time_zone`, through
//!   `ManagerConfig::time_zone`) gets `SET TIME ZONE` in its plan, so the guard's replay and a
//!   reset or reconnect restore it; never on MySQL or MariaDB (R5: their named zones need the
//!   server's zone tables). Connect cannot set it, so the session sends it once after connect
//!   (`EnvPlan::after_connect`).
//! - **C3:** a read-write session runs the profile's startup script after connect and after
//!   every reset or reconnect, after the time zone and before the schema restore (the user's
//!   own switch wins over the script's starting context; fix round 1, review I3a). The
//!   script is split in execution mode ([`startup_statements`]); one uncertain boundary or one
//!   client command refuses it whole, and nothing of it is sent. A read-only session never
//!   gets it: the guard's reset after each statement would wipe it, and its replay is trusted
//!   SQL, which a user's script is not.

use hacivat_guard::plan_for;
use hacivat_proto::DbmsKind;
// ## m3b-t12-session-carries
use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_split::{SplitMode, split_from};

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
    /// PostgreSQL: the whole `search_path` setting, element by element (M11 ruling).
    RestoreSearchPath(Vec<String>),
    // ## m3b-t12-session-carries (C3)
    /// The profile's startup script, split in execution mode ([`startup_statements`]); read-write
    /// sessions only, last in the plan.
    StartupScript(Vec<String>),
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

    // ## m3b-t12-session-carries
    /// The statements of one of this plan's steps.
    pub fn step_statements(&self, step: &EnvStep) -> Vec<String> {
        step_statements(self.kind, step)
    }

    /// What the session sends once after connect, which connect itself did not set: the time
    /// zone (C11) and the startup script (C3), in plan order. The identity, the read-only
    /// setup and the schema came with the connect (Q27).
    pub fn after_connect(&self) -> Vec<String> {
        self.steps
            .iter()
            .filter(|step| matches!(step, EnvStep::TimeZone(_) | EnvStep::StartupScript(_)))
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
        // ## m3b-t12-session-carries (C3)
        (EnvStep::StartupScript(statements), _) => statements.clone(),
        (EnvStep::TimeZone(zone), Family::Postgres) => {
            vec![format!("SET TIME ZONE {}", quote_pg_literal(zone))]
        }
        (EnvStep::TimeZone(zone), Family::MySql) => {
            vec![format!("SET time_zone = {}", quote_my_literal(zone))]
        }
        (EnvStep::RestoreSearchPath(path), Family::Postgres) => {
            search_path_sql(path).into_iter().collect()
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
/// 3. the time zone: none here ([`build_env_plan_with`] takes one);
/// 4. ## m3b-t12-session-carries: the profile's startup script, on a read-write session whose
///    script splits with certainty (C3);
/// 5. the last schema, only when one is known (Q27).
pub fn build_env_plan(
    kind: DbmsKind,
    profile: &Profile,
    read_only: bool,
    last_schema: Option<&SchemaCtx>,
) -> EnvPlan {
    build_env_plan_with(kind, profile, read_only, last_schema, None)
}

// ## m3b-t12-session-carries
/// [`build_env_plan`] with the settings' time zone (an IANA name, C11): a PostgreSQL session
/// gets `SET TIME ZONE` after the read-only setup; MySQL and MariaDB never do (R5).
pub fn build_env_plan_with(
    kind: DbmsKind,
    profile: &Profile,
    read_only: bool,
    last_schema: Option<&SchemaCtx>,
    time_zone: Option<&str>,
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
    // ## m3b-t12-session-carries (C11): PostgreSQL only; an empty name or a NUL is never sent.
    if family(kind) == Family::Postgres
        && let Some(zone) = time_zone.filter(|z| !z.is_empty() && !z.contains('\0'))
    {
        steps.push(EnvStep::TimeZone(zone.to_string()));
    }
    // ## m3b-t12-session-carries (C3): read-write only, and only a script that splits with
    // certainty (the session refuses the other kind at connect, `StartupScriptRefused`).
    // Fix round 1 (review I3a): before the schema restore, so the script sets the starting
    // context and the user's own later switch wins on a reset or reconnect.
    if !read_only
        && let Some(Ok(statements)) = profile
            .startup_script
            .as_deref()
            .map(|script| startup_statements(kind, script))
        && !statements.is_empty()
    {
        steps.push(EnvStep::StartupScript(statements));
    }
    // PostgreSQL restores the whole setting when it is known (M11 ruling), else the schema.
    let path = last_schema.map(|s| &s.search_path_setting);
    if family(kind) == Family::Postgres
        && let Some(path) = path
        && search_path_sql(path).is_some()
    {
        steps.push(EnvStep::RestoreSearchPath(path.clone()));
    } else if let Some(name) = last_schema.and_then(|s| s.schema.as_ref())
        && !switch_schema_sql(kind, name).is_empty()
    {
        steps.push(EnvStep::RestoreSchema(name.clone()));
    }
    EnvPlan { kind, steps }
}

// ## m3b-t12-session-carries (C3)
/// Why a startup script is not run: one of its boundaries is uncertain, or it holds a client
/// command (`\set`, `DELIMITER`): execution mode never sends either (D-06, R-M3-24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StartupScriptRefused;

/// The statements of `script` for `kind`, split in execution mode, each its strict text
/// (without the terminator); empty items are dropped. Refused whole when any boundary is
/// uncertain or the script holds a client command: none of it is sent.
pub fn startup_statements(
    kind: DbmsKind,
    script: &str,
) -> Result<Vec<String>, StartupScriptRefused> {
    // The engine's client flavor, so its client commands are seen (and refused).
    let (id, flavor) = match family(kind) {
        Family::Postgres => (DialectId::Postgres, ClientFlavor::Psql),
        Family::MySql if kind == DbmsKind::MariaDb => (DialectId::MariaDb, ClientFlavor::MySqlCli),
        Family::MySql => (DialectId::MySql, ClientFlavor::MySqlCli),
        Family::Sqlite => (DialectId::Sqlite, ClientFlavor::Sqlite3),
        Family::Other => (DialectId::Generic, ClientFlavor::None),
    };
    let split = split_from(
        script,
        &DialectCtx::new(id, None),
        flavor,
        SplitMode::Execution,
        &Default::default(),
    );
    if !split.ambiguous.is_empty() || !split.client.is_empty() {
        return Err(StartupScriptRefused);
    }
    let mut out = Vec::new();
    for stmt in split.statements() {
        if !split.is_certain(stmt) {
            return Err(StartupScriptRefused);
        }
        let text = script[stmt.strict].trim();
        if !text.is_empty() {
            out.push(text.to_string());
        }
    }
    Ok(out)
}

/// `SET search_path TO "a", "b"`, each element quoted exactly (`"$user"` is PostgreSQL's
/// documented spelling of the token). `None` for an empty list or one with an empty element or
/// a NUL, which is never sent.
fn search_path_sql(path: &[String]) -> Option<String> {
    if path.is_empty() || path.iter().any(|e| e.is_empty() || e.contains('\0')) {
        return None;
    }
    let list: Vec<String> = path.iter().map(|e| quote_pg_ident(e)).collect();
    Some(format!("SET search_path TO {}", list.join(", ")))
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
/// `NO_BACKSLASH_ESCAPES`: the quote can never be escaped away). Under `NO_BACKSLASH_ESCAPES`
/// a value with a backslash reads with the backslash doubled: safe, but not the value meant
/// (only the M3 time zone uses it, and zone names have no backslash).
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
                // ## m3b-t12-session-carries: the script's statements render as they are.
                EnvStep::StartupScript(vec!["SET work_mem = '64MB'".into()]),
            ],
        };
        assert_eq!(
            tz(DbmsKind::Postgres).statements(),
            ["SET TIME ZONE 'Europe/Istanbul'", "SET work_mem = '64MB'"]
        );
        assert_eq!(
            tz(DbmsKind::MySql).statements(),
            ["SET time_zone = 'Europe/Istanbul'", "SET work_mem = '64MB'"]
        );
        assert_eq!(tz(DbmsKind::Sqlite).statements(), ["SET work_mem = '64MB'"]);
        assert_eq!(quote_my_literal("a\\'b"), "'a\\\\''b'");
    }
}

// ## m3b-t12-session-carries (C11; fix round 1, review m7)
/// The settings' time zone as the sessions of one profile share it. A name that is no IANA
/// shape, or one a server refused, is never sent again: the first refusal breaks its session
/// with `config.invalid` and says which setting it was; every later session keeps the
/// server's zone instead of failing too.
#[derive(Debug)]
pub struct SessionZone {
    name: String,
    refused: std::sync::atomic::AtomicBool,
    // ## m3b-t22-tx-ui (named exception, review I4)
    /// A server refused it (not merely a name that is no zone).
    by_server: std::sync::atomic::AtomicBool,
}

impl SessionZone {
    /// The zone `name`; one that is not an IANA-shaped name is refused at once (logged once).
    pub fn new(name: &str) -> SessionZone {
        let valid = valid_zone_name(name);
        if !valid {
            tracing::warn!(
                setting = "ui.time_zone",
                "the time zone setting is not a time zone name; sessions keep the server's zone"
            );
        }
        SessionZone {
            name: name.to_string(),
            refused: std::sync::atomic::AtomicBool::new(!valid),
            by_server: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The name to send, unless it was refused.
    pub fn usable(&self) -> Option<&str> {
        (!self.refused.load(std::sync::atomic::Ordering::SeqCst)).then_some(self.name.as_str())
    }

    /// A server refused it: no session sends it again. True the first time (the caller says
    /// so once).
    pub fn refuse(&self) -> bool {
        // ## m3b-t22-tx-ui (named exception, review I4)
        self.by_server
            .store(true, std::sync::atomic::Ordering::SeqCst);
        !self.refused.swap(true, std::sync::atomic::Ordering::SeqCst)
    }

    // ## m3b-t22-tx-ui (named exception, review I4)
    /// The zone's name once a server refused it (the UI says so; a retry connects without
    /// it).
    pub fn refused_by_server(&self) -> Option<&str> {
        self.by_server
            .load(std::sync::atomic::Ordering::SeqCst)
            .then_some(self.name.as_str())
    }
}

/// An IANA-shaped zone name (`Area/Location`), as the settings check it: 1 to 64 characters
/// of ASCII letters, digits, `/`, `_`, `-`, `+`, no leading, trailing or doubled `/`, no `..`.
pub fn valid_zone_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.contains("//")
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
}
