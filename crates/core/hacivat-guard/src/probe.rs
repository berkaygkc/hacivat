//! The role probe: what the signed-in role could do if the guard failed (MCP-09 step 1).
//!
//! Every reading fails closed: an answer of an unexpected shape, a NULL, a name outside the
//! probe's fixed lists or a `GRANT` line the parser cannot read adds [`RoleWarning::Unparsed`]
//! and clears `least_privilege`.

use hacivat_proto::DbmsKind;

/// A privilege of the signed-in role worth a warning.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RoleWarning {
    Superuser,
    BypassRls,
    Replication,
    CreateRole,
    ServerFileRoles,
    ExecuteServerProgram,
    SignalBackend,
    WritePrivileges,
    TempPrivilege,
    /// An extension that can reach other servers (`dblink`, `postgres_fdw`, ...).
    EgressExtension(String),
    /// A MySQL role grant the probe does not expand (fails closed).
    RoleGrantNotExpanded,
    /// A probe answer the interpreter could not read (fails closed).
    Unparsed,
    // ## m2b-t23-f1
    /// PostgreSQL: it may execute a function whose effect a rollback does not undo (row 6;
    /// `PUBLIC` may run `pg_logical_emit_message` until the remediation's optional `REVOKE`).
    NonTransactionalWrite,
}

/// What the probe found.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct RoleFindings {
    pub least_privilege: bool,
    pub warnings: Vec<RoleWarning>,
}

/// PostgreSQL's probe, one row set per statement. Everything a role can reach with `SET ROLE`
/// counts, so rows 1, 2 and 3 look at every role `current_user` is a `MEMBER` of (fix round 2,
/// I-1: role attributes are never inherited, and membership in a superuser role is one
/// `set_config('role', …)` away).
///
/// 1. The role flags, `bool_or` over those roles (always one row: a role is a member of
///    itself).
/// 2. The dangerous predefined roles among them. `pg_maintain` (PostgreSQL 17+: `ANALYZE`,
///    `LOCK … ACCESS EXCLUSIVE` on every relation) simply never matches on older servers.
/// 3. Whether any of them can write or create: owns a user relation; holds `INSERT`/`UPDATE`/
///    `DELETE`/`TRUNCATE` on one, or column-level `INSERT`/`UPDATE`
///    (`has_any_column_privilege`); holds `USAGE` or `UPDATE` on a sequence; holds `CREATE` on
///    a user schema (`public` on clusters created before PostgreSQL 15 or upgraded from one,
///    and the database owner's `public` on 15+); or has `CREATEDB` or `CREATE` on the database
///    (round 3, m-4: folded in here so row 1 keeps its four columns). Schemas are filtered
///    with `!~ '^pg_'`, which needs no escape and so does not depend on
///    `standard_conforming_strings`.
/// 4. TEMP on the database.
/// 5. The extensions that reach other servers.
/// 6. (Task 23 finding F1) The functions a rollback cannot undo that any of them may `EXECUTE`
///    ([`PG_EXECUTE_GATED`]): `PUBLIC` holds `EXECUTE` on `pg_logical_emit_message` on 13–18,
///    so a role reads least privilege only once the remediation's optional `REVOKE` ran. The
///    names are looked up in `pg_proc` (every overload, any version: the signatures differ from
///    17). The replication-origin functions count from 14, when `EXECUTE` became their gate;
///    before, they check for a superuser themselves.
///
/// T18 review m-3: every function, aggregate and relation is `pg_catalog.`-qualified, so a
/// `search_path` that names another schema before `pg_catalog` (an imported URL may set it)
/// cannot shadow them. Operators stay unqualified (ADR 0004 `m2b-t18`).
const PG_PROBE: [&str; 6] = [
    "SELECT pg_catalog.bool_or(rolsuper), pg_catalog.bool_or(rolbypassrls), pg_catalog.bool_or(rolreplication), pg_catalog.bool_or(rolcreaterole) FROM pg_catalog.pg_roles WHERE pg_catalog.pg_has_role(current_user, oid, 'MEMBER')",
    "SELECT r.rolname FROM pg_catalog.pg_roles r WHERE pg_catalog.pg_has_role(current_user, r.oid, 'MEMBER') AND r.rolname IN ('pg_read_server_files','pg_write_server_files','pg_execute_server_program','pg_signal_backend','pg_write_all_data','pg_maintain')",
    "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace JOIN pg_catalog.pg_roles r ON pg_catalog.pg_has_role(current_user, r.oid, 'MEMBER') WHERE c.relkind IN ('r','p','v','m','f','S') AND n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' AND (c.relowner = r.oid OR CASE WHEN c.relkind = 'S' THEN pg_catalog.has_sequence_privilege(r.oid, c.oid, 'USAGE, UPDATE') ELSE pg_catalog.has_table_privilege(r.oid, c.oid, 'INSERT, UPDATE, DELETE, TRUNCATE') OR pg_catalog.has_any_column_privilege(r.oid, c.oid, 'INSERT, UPDATE') END)) OR EXISTS (SELECT 1 FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_roles r ON pg_catalog.pg_has_role(current_user, r.oid, 'MEMBER') WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' AND pg_catalog.has_schema_privilege(r.oid, n.oid, 'CREATE')) OR EXISTS (SELECT 1 FROM pg_catalog.pg_roles r WHERE pg_catalog.pg_has_role(current_user, r.oid, 'MEMBER') AND (r.rolcreatedb OR pg_catalog.has_database_privilege(r.oid, pg_catalog.current_database(), 'CREATE')))",
    "SELECT pg_catalog.has_database_privilege(pg_catalog.current_database(), 'TEMP')",
    "SELECT extname FROM pg_catalog.pg_extension WHERE extname IN ('dblink','postgres_fdw','pg_net','http')",
    // ## m2b-t23-f1
    "SELECT DISTINCT p.proname FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace JOIN pg_catalog.pg_roles r ON pg_catalog.pg_has_role(current_user, r.oid, 'MEMBER') WHERE n.nspname = 'pg_catalog' AND p.proname IN ('pg_logical_emit_message','pg_switch_wal','pg_create_restore_point','pg_log_standby_snapshot','pg_backup_start','pg_backup_stop','pg_start_backup','pg_stop_backup','pg_promote','pg_wal_replay_pause','pg_wal_replay_resume','pg_replication_origin_create','pg_replication_origin_drop','pg_replication_origin_advance','pg_replication_origin_session_setup','pg_replication_origin_session_reset','pg_replication_origin_xact_setup','pg_replication_origin_xact_reset') AND (p.proname !~ '^pg_replication_origin_' OR pg_catalog.current_setting('server_version_num')::pg_catalog.int4 >= 140000) AND pg_catalog.has_function_privilege(r.oid, p.oid, 'EXECUTE')",
];

const MYSQL_PROBE: [&str; 1] = ["SHOW GRANTS"];

/// Row set 1's columns, in order, and the warning each `true` adds.
const PG_ROLE_FLAGS: [RoleWarning; 4] = [
    RoleWarning::Superuser,
    RoleWarning::BypassRls,
    RoleWarning::Replication,
    RoleWarning::CreateRole,
];

/// Row set 2's names (the probe's `IN` list) and the warning each adds.
const PG_PREDEFINED_ROLES: [(&str, RoleWarning); 6] = [
    ("pg_read_server_files", RoleWarning::ServerFileRoles),
    ("pg_write_server_files", RoleWarning::ServerFileRoles),
    (
        "pg_execute_server_program",
        RoleWarning::ExecuteServerProgram,
    ),
    ("pg_signal_backend", RoleWarning::SignalBackend),
    ("pg_write_all_data", RoleWarning::WritePrivileges),
    // Round 3, m-4: maintenance and `LOCK` on every relation, which a read-only transaction
    // does not stop.
    ("pg_maintain", RoleWarning::WritePrivileges),
];

/// Row set 5's names (the probe's `IN` list). A warning carries the name from this list, never
/// the server's text.
const PG_EGRESS_EXTENSIONS: [&str; 4] = ["dblink", "postgres_fdw", "pg_net", "http"];

// ## m2b-t23-f1
/// Row set 6's names (the probe's `IN` list): the functions of
/// `functions::PG_NON_TRANSACTIONAL_FUNCTIONS` whose `EXECUTE` privilege is what lets a role
/// call them. The replication-slot functions are left out: `PUBLIC` may execute them, and they
/// refuse a role without `REPLICATION` at run time (row 1 reports it).
const PG_EXECUTE_GATED: [&str; 18] = [
    "pg_logical_emit_message",
    "pg_switch_wal",
    "pg_create_restore_point",
    "pg_log_standby_snapshot",
    "pg_backup_start",
    "pg_backup_stop",
    "pg_start_backup",
    "pg_stop_backup",
    "pg_promote",
    "pg_wal_replay_pause",
    "pg_wal_replay_resume",
    "pg_replication_origin_create",
    "pg_replication_origin_drop",
    "pg_replication_origin_advance",
    "pg_replication_origin_session_setup",
    "pg_replication_origin_session_reset",
    "pg_replication_origin_xact_setup",
    "pg_replication_origin_xact_reset",
];

/// Privileges a least-privilege MySQL role may hold.
const MYSQL_READ_PRIVILEGES: [&str; 3] = ["USAGE", "SELECT", "SHOW VIEW"];

/// MySQL privileges with a warning of their own. `ALL` (global or not) and
/// `CREATE TEMPORARY TABLES` are read apart.
const MYSQL_PRIVILEGE_WARNINGS: [(&str, RoleWarning); 13] = [
    ("SUPER", RoleWarning::Superuser),
    ("FILE", RoleWarning::ServerFileRoles),
    ("REPLICATION SLAVE", RoleWarning::Replication),
    ("REPLICATION CLIENT", RoleWarning::Replication),
    ("REPLICATION_SLAVE_ADMIN", RoleWarning::Replication),
    ("REPLICATION_APPLIER", RoleWarning::Replication),
    ("CREATE USER", RoleWarning::CreateRole),
    ("CREATE ROLE", RoleWarning::CreateRole),
    ("DROP ROLE", RoleWarning::CreateRole),
    ("ROLE_ADMIN", RoleWarning::CreateRole),
    ("GRANT OPTION", RoleWarning::CreateRole),
    ("PROXY", RoleWarning::CreateRole),
    ("CONNECTION_ADMIN", RoleWarning::SignalBackend),
];

/// MySQL privileges that change data or schema, or run code that may.
const MYSQL_WRITE_PRIVILEGES: [&str; 17] = [
    "INSERT",
    "UPDATE",
    "DELETE",
    "CREATE",
    "DROP",
    "ALTER",
    "INDEX",
    "REFERENCES",
    "CREATE VIEW",
    "CREATE ROUTINE",
    "ALTER ROUTINE",
    "EXECUTE",
    "TRIGGER",
    "EVENT",
    "LOCK TABLES",
    "CREATE TABLESPACE",
    "DELETE HISTORY",
];

/// The probe statements of `kind`, in order: six for PostgreSQL, `SHOW GRANTS` for MySQL and
/// MariaDB, none for any other engine.
pub fn role_probe_sql(kind: DbmsKind) -> &'static [&'static str] {
    match kind {
        DbmsKind::Postgres => &PG_PROBE,
        DbmsKind::MySql | DbmsKind::MariaDb => &MYSQL_PROBE,
        _ => &[],
    }
}

/// Reads the probe's answers, one row set per probe statement (rows of text columns, `None` for
/// SQL NULL).
///
/// - PostgreSQL: least privilege is no `true` in row set 1, no rows in 2, `false` in 3 and no
///   rows in 5 or 6. A `true` in 4 adds [`RoleWarning::TempPrivilege`] only and keeps least
///   privilege (Q21: the read-only transaction already blocks temporary tables). A row in 6 is
///   [`RoleWarning::NonTransactionalWrite`] (Task 23 finding F1).
/// - MySQL and MariaDB: every `GRANT` line grants only `USAGE`, `SELECT` and `SHOW VIEW`, with
///   no grant option and no `PROXY`; a role grant is [`RoleWarning::RoleGrantNotExpanded`].
/// - Any other engine has no probe, so its answer is [`RoleWarning::Unparsed`].
pub fn interpret_probe(kind: DbmsKind, results: &[Vec<Vec<Option<String>>>]) -> RoleFindings {
    let mut f = Findings::default();
    match kind {
        DbmsKind::Postgres => read_pg(results, &mut f),
        DbmsKind::MySql | DbmsKind::MariaDb => read_mysql(results, &mut f),
        _ => f.deny(RoleWarning::Unparsed),
    }
    RoleFindings {
        least_privilege: f.least_privilege,
        warnings: f.warnings,
    }
}

/// Findings under construction: least privilege until a warning denies it.
struct Findings {
    least_privilege: bool,
    warnings: Vec<RoleWarning>,
}

impl Default for Findings {
    fn default() -> Self {
        Findings {
            least_privilege: true,
            warnings: Vec::new(),
        }
    }
}

impl Findings {
    /// Adds `w` once, keeping least privilege.
    fn note(&mut self, w: RoleWarning) {
        if !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    /// Adds `w` once and clears least privilege.
    fn deny(&mut self, w: RoleWarning) {
        self.least_privilege = false;
        self.note(w);
    }
}

type RowSet = Vec<Vec<Option<String>>>;

fn read_pg(results: &[RowSet], f: &mut Findings) {
    let [flags, roles, write, temp, extensions, functions] = results else {
        f.deny(RoleWarning::Unparsed);
        return;
    };

    match single_row(flags, PG_ROLE_FLAGS.len()) {
        Some(row) => {
            for (cell, w) in row.iter().zip(PG_ROLE_FLAGS) {
                match pg_bool(cell.as_deref()) {
                    Some(true) => f.deny(w),
                    Some(false) => {}
                    None => f.deny(RoleWarning::Unparsed),
                }
            }
        }
        None => f.deny(RoleWarning::Unparsed),
    }

    for row in roles {
        let known = single_cell(row).and_then(|name| {
            PG_PREDEFINED_ROLES
                .iter()
                .find(|(r, _)| name.eq_ignore_ascii_case(r))
        });
        match known {
            Some((_, w)) => f.deny(w.clone()),
            None => f.deny(RoleWarning::Unparsed),
        }
    }

    match single_bool(write) {
        Some(true) => f.deny(RoleWarning::WritePrivileges),
        Some(false) => {}
        None => f.deny(RoleWarning::Unparsed),
    }

    // Q21: TEMP alone is a finding, not a reason to drop the enforced label.
    match single_bool(temp) {
        Some(true) => f.note(RoleWarning::TempPrivilege),
        Some(false) => {}
        None => f.deny(RoleWarning::Unparsed),
    }

    for row in extensions {
        let known = single_cell(row).and_then(|name| {
            PG_EGRESS_EXTENSIONS
                .iter()
                .find(|e| name.eq_ignore_ascii_case(e))
        });
        match known {
            Some(e) => f.deny(RoleWarning::EgressExtension((*e).to_owned())),
            None => f.deny(RoleWarning::Unparsed),
        }
    }

    // ## m2b-t23-f1: a role that may run a function no rollback undoes is not read-only,
    // whatever the guard's text layer refuses (SQL built at run time names no function).
    for row in functions {
        let known = single_cell(row).is_some_and(|name| {
            PG_EXECUTE_GATED
                .iter()
                .any(|g| name.eq_ignore_ascii_case(g))
        });
        f.deny(if known {
            RoleWarning::NonTransactionalWrite
        } else {
            RoleWarning::Unparsed
        });
    }
}

/// The only row of `set`, when it has exactly `columns` columns.
fn single_row(set: &RowSet, columns: usize) -> Option<&[Option<String>]> {
    match set.as_slice() {
        [row] if row.len() == columns => Some(row),
        _ => None,
    }
}

/// The boolean of a one-row, one-column row set.
fn single_bool(set: &RowSet) -> Option<bool> {
    single_row(set, 1).and_then(|row| pg_bool(row[0].as_deref()))
}

/// The text of a one-column, non-NULL row.
fn single_cell(row: &[Option<String>]) -> Option<&str> {
    match row {
        [Some(v)] => Some(v),
        _ => None,
    }
}

/// A PostgreSQL boolean in text form (`t`/`f` on the wire; `true`/`false` accepted).
fn pg_bool(cell: Option<&str>) -> Option<bool> {
    let v = cell?.trim();
    if v.eq_ignore_ascii_case("t") || v.eq_ignore_ascii_case("true") {
        Some(true)
    } else if v.eq_ignore_ascii_case("f") || v.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

/// `SHOW GRANTS`: one row set of one-column rows, at least one line.
fn read_mysql(results: &[RowSet], f: &mut Findings) {
    let [lines] = results else {
        f.deny(RoleWarning::Unparsed);
        return;
    };
    if lines.is_empty() {
        f.deny(RoleWarning::Unparsed);
    }
    for row in lines {
        match single_cell(row) {
            Some(line) => read_grant_line(line, f),
            None => f.deny(RoleWarning::Unparsed),
        }
    }
}

/// One `SHOW GRANTS` line: `GRANT <privileges> ON <object> TO <user> [...]` or
/// `GRANT <role>[, <role>] TO <user> [...]`. `REVOKE` lines (MySQL's partial revokes) only take
/// privileges away, so they are skipped. Anything else is `Unparsed`.
fn read_grant_line(line: &str, f: &mut Findings) {
    let Some(tokens) = tokenize(line) else {
        f.deny(RoleWarning::Unparsed);
        return;
    };
    match tokens.first() {
        Some(t) if t.is_word("REVOKE") => return,
        Some(t) if t.is_word("GRANT") => {}
        _ => {
            f.deny(RoleWarning::Unparsed);
            return;
        }
    }
    let top_word = |word: &str| tokens.iter().position(|t| t.depth == 0 && t.is_word(word));
    let Some(to) = top_word("TO") else {
        f.deny(RoleWarning::Unparsed);
        return;
    };
    let after_to = &tokens[to + 1..];
    // A second `ON` or `TO` after the grantee is a line the parser does not know.
    if after_to
        .iter()
        .any(|t| t.depth == 0 && (t.is_word("ON") || t.is_word("TO")))
    {
        f.deny(RoleWarning::Unparsed);
        return;
    }
    if with_option(after_to, "GRANT") || with_option(after_to, "ADMIN") {
        f.deny(RoleWarning::CreateRole);
    }
    // The first top-level `ON` is before `TO` here: any later one was refused above.
    match top_word("ON") {
        Some(on) => {
            let global = tokens.get(on + 1).is_some_and(|t| t.is_word("*.*"));
            read_privileges(&tokens[1..on], global, f);
        }
        // A role grant: the roles' own privileges are not shown, so they are not judged.
        None if to > 1 => f.deny(RoleWarning::RoleGrantNotExpanded),
        None => f.deny(RoleWarning::Unparsed),
    }
}

/// Whether `WITH <word> OPTION` appears at the top level of `tokens`.
fn with_option(tokens: &[Token<'_>], word: &str) -> bool {
    tokens.windows(3).any(|w| {
        w.iter().all(|t| t.depth == 0)
            && w[0].is_word("WITH")
            && w[1].is_word(word)
            && w[2].is_word("OPTION")
    })
}

/// The comma-separated privilege list between `GRANT` and `ON`. A column list in parentheses
/// is dropped (`SELECT (a, b)` is `SELECT`); what is left of each privilege must be plain
/// words (a quoted privilege is not one).
fn read_privileges(tokens: &[Token<'_>], global: bool, f: &mut Findings) {
    if tokens.is_empty() {
        f.deny(RoleWarning::Unparsed);
        return;
    }
    for group in tokens.split(|t| t.depth == 0 && t.is_punct(b',')) {
        let words: Vec<&Token<'_>> = group
            .iter()
            .filter(|t| t.depth == 0 && !t.is_punct(b'(') && !t.is_punct(b')'))
            .collect();
        if words.is_empty() || words.iter().any(|t| !t.plain_word()) {
            f.deny(RoleWarning::Unparsed);
            continue;
        }
        let name = words.iter().map(|t| t.text).collect::<Vec<_>>().join(" ");
        read_privilege(&name, global, f);
    }
}

fn read_privilege(name: &str, global: bool, f: &mut Findings) {
    let is = |p: &str| name.eq_ignore_ascii_case(p);
    if MYSQL_READ_PRIVILEGES.iter().any(|p| is(p)) {
        return;
    }
    let warning = if is("ALL") || is("ALL PRIVILEGES") {
        if global {
            RoleWarning::Superuser
        } else {
            RoleWarning::WritePrivileges
        }
    } else if is("CREATE TEMPORARY TABLES") {
        // Not least privilege on MySQL (unlike PostgreSQL's TEMP, Q21), but this is what it
        // allows.
        RoleWarning::TempPrivilege
    } else if let Some((_, w)) = MYSQL_PRIVILEGE_WARNINGS.iter().find(|(p, _)| is(p)) {
        w.clone()
    } else if MYSQL_WRITE_PRIVILEGES.iter().any(|p| is(p)) {
        RoleWarning::WritePrivileges
    } else {
        // A privilege without a row here (`PROCESS`, dynamic `*_ADMIN` privileges, ...).
        RoleWarning::Unparsed
    };
    f.deny(warning);
}

/// A token of a `SHOW GRANTS` line, with its parenthesis depth (a parenthesis has the depth
/// outside it).
struct Token<'a> {
    text: &'a str,
    quoted: bool,
    depth: u32,
}

impl Token<'_> {
    fn is_word(&self, word: &str) -> bool {
        !self.quoted && self.text.eq_ignore_ascii_case(word)
    }

    fn is_punct(&self, p: u8) -> bool {
        !self.quoted && self.text.as_bytes() == [p]
    }

    /// An unquoted privilege word: ASCII letters and `_` only.
    fn plain_word(&self) -> bool {
        !self.quoted
            && !self.text.is_empty()
            && self
                .text
                .bytes()
                .all(|b| b.is_ascii_alphabetic() || b == b'_')
    }
}

/// Splits a grant line into words, quoted names (`` ` ``, `'`, `"`; a doubled quote escapes
/// itself, and `\` escapes inside `'` and `"`), commas and parentheses. `None` for an
/// unterminated quote, unbalanced parentheses, a `;` outside quotes (a second statement) or a
/// non-ASCII character outside quotes.
fn tokenize(line: &str) -> Option<Vec<Token<'_>>> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut depth: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let start = i;
        let quoted = matches!(b, b'`' | b'\'' | b'"');
        match b {
            _ if b.is_ascii_whitespace() => {
                i += 1;
                continue;
            }
            b';' => return None,
            b',' | b'(' => i += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                i += 1;
            }
            _ if quoted => {
                i += 1;
                loop {
                    let c = *bytes.get(i)?;
                    let backslash = c == b'\\' && b != b'`';
                    let doubled = c == b && bytes.get(i + 1) == Some(&b);
                    if backslash || doubled {
                        i += 2;
                    } else {
                        i += 1;
                        if c == b {
                            break;
                        }
                    }
                }
            }
            _ => {
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(bytes[i], b';' | b',' | b'(' | b')' | b'`' | b'\'' | b'"')
                {
                    i += 1;
                }
            }
        }
        // `i` only ever stops on an ASCII byte or the end, so the slice is on char boundaries.
        let text = line.get(start..i)?;
        // Servers print keywords, `*.*` and `@` in ASCII; anything else outside quotes (a
        // no-break space, a look-alike letter) is a line the parser does not know (M-2).
        if !quoted && !text.is_ascii() {
            return None;
        }
        tokens.push(Token {
            text,
            quoted,
            depth,
        });
        if b == b'(' {
            depth += 1;
        }
    }
    (depth == 0).then_some(tokens)
}
