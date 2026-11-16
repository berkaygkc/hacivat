// ## m3b-t15-gate (fix round 1, I5)
//! Statements that make MySQL, MariaDB and TiDB commit an open transaction before they run,
//! read from the statement's words. This is the one list the gate uses; the session's T19
//! backstop (`TxError::ImplicitCommitUnconfirmed`) reuses it through `SessionEffects::DDL`,
//! which the gate sets for exactly these statements on those engines.
//!
//! Sources: MySQL 8.4 Reference Manual §15.3.3 "Statements That Cause an Implicit Commit"
//! (also 8.0 §13.3.3); MariaDB Knowledge Base "SQL statements That Cause an Implicit Commit".
//! The list, by the manual's groups:
//! - **DDL**: every `CREATE`, `ALTER`, `DROP`, `RENAME` and `TRUNCATE`, `INSTALL`/`UNINSTALL`
//!   (plugins, components), except `CREATE TEMPORARY TABLE` and `DROP TEMPORARY TABLE` (which do
//!   not commit; `ALTER TABLE` and `CREATE INDEX` on a temporary table still do);
//! - **the `mysql` schema**: `GRANT`, `REVOKE`, `CREATE`/`ALTER`/`DROP`/`RENAME USER`,
//!   `CREATE`/`DROP ROLE`, `SET PASSWORD`;
//! - **transactions and locking**: `BEGIN`, `START TRANSACTION`, `LOCK TABLES`, `UNLOCK TABLES`
//!   (when tables are locked; always counted: the gate cannot see the lock),
//!   `SET autocommit = 1` (any value but `0`/`OFF`/`FALSE`, counted when unreadable);
//! - **data loading**: `LOAD DATA` (on some engines; always counted), `LOAD XML`;
//! - **administration**: `ANALYZE`/`CHECK`/`OPTIMIZE`/`REPAIR TABLE`, `CACHE INDEX`,
//!   `LOAD INDEX INTO CACHE`, `FLUSH`, `RESET` (not `RESET PERSIST`);
//! - **replication**: `START`/`STOP REPLICA` (`SLAVE`, `GROUP_REPLICATION`), `RESET REPLICA`,
//!   `CHANGE REPLICATION SOURCE TO` / `CHANGE MASTER TO`.
//!
//! Over-counting is the safe direction: an extra "commits your open transaction" confirmation.

use sql_dialect::{DialectCtx, DialectId, lex_profile};
use sql_lexer::{LexState, TokenFlags, TokenKind, lex};

/// The statement's words and atoms in order, ASCII-uppercased: words, variables, numbers and
/// operators of the code, the words inside executable comments (they run), and, from the first
/// token the lexer could not read cleanly, every word of the rest of the text. Strings, quoted
/// names and plain comments are no words. Without a lexer for `dialect`, every ASCII word of
/// the text.
pub(crate) fn atoms(text: &str, dialect: DialectId) -> Vec<String> {
    let ctx = DialectCtx::new(dialect, None);
    let Ok(profile) = lex_profile(&ctx) else {
        return words(text);
    };
    let lexed = lex(text, profile, ctx.modes, &LexState::Initial);
    let mut out = Vec::new();
    for i in 0..lexed.len() {
        let r = lexed.range(i);
        let (s, e) = (usize::from(r.start()), usize::from(r.end()));
        let token = text.get(s..e).unwrap_or_default();
        let unclean = lexed
            .flags(i)
            .intersects(TokenFlags::UNTERMINATED | TokenFlags::ERROR)
            || lexed.kind(i) == TokenKind::Unknown;
        if unclean {
            out.extend(words(text.get(s..).unwrap_or_default()));
            break;
        }
        match lexed.kind(i) {
            TokenKind::ExecutableComment => out.extend(words(exec_body(token))),
            TokenKind::Word
            | TokenKind::SystemVar
            | TokenKind::UserVar
            | TokenKind::Number
            | TokenKind::Op => out.push(token.to_ascii_uppercase()),
            _ => {}
        }
    }
    out
}

/// The body of `/*!NNNNN … */` or `/*M!NNNNNN … */`.
fn exec_body(token: &str) -> &str {
    let t = token.trim_end_matches("*/");
    let t = t
        .strip_prefix("/*M!")
        .or_else(|| t.strip_prefix("/*!"))
        .unwrap_or(t);
    t.trim_start_matches(|c: char| c.is_ascii_digit())
}

/// Every word of `text` (ASCII letters, digits, `_`, `@`, `.`), uppercased.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '.')))
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_uppercase)
        .collect()
}

/// Whether `engine` commits an open transaction before the statements of [`commits`].
pub fn engine_commits_implicitly(engine: hacivat_proto::DbmsKind) -> bool {
    use hacivat_proto::DbmsKind as K;
    matches!(engine, K::MySql | K::MariaDb | K::TiDb)
}

/// `CREATE [OR REPLACE] TEMPORARY TABLE` / `DROP TEMPORARY TABLE` (no implicit commit).
pub(crate) fn temporary_table(atoms: &[String]) -> bool {
    let w = |i: usize| atoms.get(i).map_or("", String::as_str);
    match w(0) {
        "CREATE" if w(1) == "OR" && w(2) == "REPLACE" => w(3) == "TEMPORARY" && w(4) == "TABLE",
        "CREATE" | "DROP" => w(1) == "TEMPORARY" && w(2) == "TABLE",
        _ => false,
    }
}

/// Whether a statement whose words are `atoms` makes MySQL/MariaDB/TiDB commit an open
/// transaction first (the module docs list them).
pub fn commits(atoms: &[String]) -> bool {
    let w = |i: usize| atoms.get(i).map_or("", String::as_str);
    match w(0) {
        "BEGIN" | "GRANT" | "REVOKE" | "RENAME" | "TRUNCATE" | "FLUSH" | "INSTALL"
        | "UNINSTALL" | "ALTER" => true,
        "CREATE" | "DROP" => !temporary_table(atoms),
        "START" => matches!(
            w(1),
            "TRANSACTION" | "REPLICA" | "SLAVE" | "GROUP_REPLICATION"
        ),
        "STOP" => matches!(w(1), "REPLICA" | "SLAVE" | "GROUP_REPLICATION"),
        "LOCK" | "UNLOCK" => matches!(w(1), "TABLE" | "TABLES"),
        "CACHE" => w(1) == "INDEX",
        "LOAD" => matches!(w(1), "INDEX" | "DATA" | "XML"),
        "RESET" => w(1) != "PERSIST",
        "ANALYZE" | "OPTIMIZE" | "REPAIR" | "CHECK" => {
            atoms.iter().take(4).any(|a| a == "TABLE" || a == "TABLES")
        }
        "CHANGE" => matches!(w(1), "MASTER" | "REPLICATION"),
        "SET" => set_commits(&atoms[1..]),
        _ => false,
    }
}

/// `SET PASSWORD …`, or an assignment of `autocommit` to anything but `0`/`OFF`/`FALSE`.
fn set_commits(rest: &[String]) -> bool {
    if rest.first().is_some_and(|w| w == "PASSWORD") {
        return true;
    }
    for (i, a) in rest.iter().enumerate() {
        let name = a.rsplit('.').next().unwrap_or(a).trim_start_matches('@');
        if name != "AUTOCOMMIT" {
            continue;
        }
        let value = rest
            .get(i + 1)
            .filter(|op| matches!(op.as_str(), "=" | ":=" | "TO"))
            .and_then(|_| rest.get(i + 2));
        match value.map(String::as_str) {
            Some("0" | "OFF" | "FALSE") => continue,
            _ => return true,
        }
    }
    false
}

/// The keywords that make an unanalysed statement (a pre-decided report) ask before it runs off
/// production (T15 fix round 1, I1): data and schema writes, privileges, routines, procedural
/// code, loads and copies, and their dialect equivalents.
const WRITE_WORDS: &[&str] = &[
    "INSERT",
    "UPDATE",
    "DELETE",
    "MERGE",
    "REPLACE",
    "UPSERT",
    "DROP",
    "TRUNCATE",
    "ALTER",
    "CREATE",
    "RENAME",
    "GRANT",
    "REVOKE",
    "CALL",
    "DO",
    "EXEC",
    "EXECUTE",
    "LOAD",
    "COPY",
    "IMPORT",
    "INTO",
    "HANDLER",
    "VACUUM",
    "REINDEX",
    "CLUSTER",
    "REFRESH",
    "ATTACH",
    "DETACH",
    "PRAGMA",
    "LOCK",
    "UNLOCK",
    "KILL",
    "SHUTDOWN",
    "FLUSH",
    "RESET",
    "PURGE",
    "OPTIMIZE",
    "REPAIR",
    "CHECKPOINT",
    "INSTALL",
    "UNINSTALL",
    "REASSIGN",
    "SECURITY",
    "PREPARE",
];

/// Whether `atoms` hold a word of [`WRITE_WORDS`] anywhere.
pub(crate) fn mentions_write(atoms: &[String]) -> bool {
    atoms.iter().any(|a| WRITE_WORDS.contains(&a.as_str()))
}
