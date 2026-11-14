//! Splitter profiles (SQL-SPLIT-01, -02, -03; Task 8): the per-dialect tables the `sql-split`
//! engine reads. The tables are data; the engine holds the algorithm (`alg/10` SPLIT-02's single
//! forward loop over tokens with a block stack). Every row of every table has a fixture in
//! `data/split/<profile>.toml` (`covers = ["<row id>"]`); a meta-test in `sql-split` checks that
//! no row goes untested.
//!
//! - **Statement starts** (SQL-SPLIT-03 step 1, editor mode only): a start word that is the
//!   first significant token on its line, at block depth 0, ends the statement before it
//!   heuristically — unless the previous significant token is one of `prev_exceptions`
//!   (`UNION`↵`SELECT`, `FOR`↵`UPDATE`) or a `head_continuations` row applies (`INSERT …`↵`SELECT`).
//!   Words that often begin a continuation line (`SET`, `VALUES`, `FETCH`, `DO`, `BEGIN`,
//!   `END`, `START`, `RENAME`, `LOCK`, `DESC`) are deliberately not start words: a missed
//!   heuristic split sends a statement the server rejects, while a false one makes Run refuse a
//!   valid statement.
//! - **Blocks** (SPLIT-02 step 2): a routine head (`routine_head_skips`, `routine_kinds`) turns
//!   on the family's block rules for that statement — PostgreSQL `BEGIN ATOMIC … END`, MySQL and
//!   MariaDB stored programs, SQLite triggers. MySQL's `IF`, `CASE`, `LOOP`, `WHILE`, `REPEAT`
//!   (MariaDB also `FOR`) open blocks only in statement position (`statement_position_after`,
//!   `body_start_after`), so `REPEAT('a', 3)` and `IF(a, b, c)` stay functions (R-M3-5 erratum
//!   b). A top-level `BEGIN` is a transaction, except before a `top_level_block_begins` word
//!   (MariaDB `BEGIN NOT ATOMIC`, erratum c). `END` at statement start with an empty stack is
//!   PostgreSQL's and SQLite's `COMMIT` (erratum a); SQLite's `BEGIN DEFERRED|IMMEDIATE|
//!   EXCLUSIVE` stays a transaction (erratum d).
//! - **Client commands** ([`ClientProfile`], SPLIT-01): psql `\cmd`, `\;`, `\g`, `COPY … FROM
//!   STDIN` data; the mysql client's `DELIMITER`, `\G`, `\g`, `\c`, `\d`, `source`; sqlite3 `.cmd`.
//!
//! Words are matched ASCII case-insensitively against the token text (never through a locale),
//! so a table row needs no entry in the keyword tables (`DELIMITER`, `STDIN`, `ATOMIC`).

mod generic;
mod mysql;
mod pg;
mod sqlite;

use crate::id::{ClientFlavor, DialectCtx, DialectId};

/// Which block rules a dialect's statements follow.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockFamily {
    /// Parentheses only.
    Plain,
    /// PostgreSQL: in a `CREATE [OR REPLACE] FUNCTION|PROCEDURE`, `BEGIN ATOMIC … END` with
    /// `CASE … END` inside.
    Postgres,
    /// MySQL and MariaDB stored programs (`BEGIN … END`, `IF … END IF`, `CASE … END [CASE]`,
    /// loops, labels), and MariaDB's top-level `BEGIN NOT ATOMIC … END`.
    MySql,
    /// SQLite: in a `CREATE [TEMP|TEMPORARY] TRIGGER`, `BEGIN … END` with `CASE … END` inside.
    Sqlite,
}

/// A block on the splitter's stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    /// `(` … `)` or `[` … `]`.
    Paren,
    /// `BEGIN … END` (a stored-program or trigger body, MariaDB `BEGIN NOT ATOMIC`).
    BeginEnd,
    /// PostgreSQL `BEGIN ATOMIC … END`.
    BeginAtomic,
    /// `CASE … END [CASE]`.
    Case,
    /// MySQL `IF … END IF`.
    If,
    /// MySQL `LOOP … END LOOP`.
    Loop,
    /// MySQL `WHILE … END WHILE`.
    While,
    /// MySQL `REPEAT … END REPEAT`.
    Repeat,
    /// MariaDB `FOR … END FOR`.
    For,
}

/// Under a statement whose head word is `head`, start words that continue the statement
/// instead of starting a new one: `words` (empty: every start word), the first one only
/// (`once`: `INSERT INTO t`↵`SELECT …`) or every one (`GRANT`↵`SELECT, INSERT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HeadContinuation {
    pub head: &'static str,
    pub words: &'static [&'static str],
    pub once: bool,
}

/// One dialect's splitter tables.
#[non_exhaustive]
#[derive(Debug, PartialEq, Eq)]
pub struct SplitProfile {
    /// The profile's name, for diagnostics and its fixture file (`data/split/<name>.toml`).
    pub name: &'static str,
    pub family: BlockFamily,
    /// Statement-start words for the editor's missing-semicolon heuristic.
    pub statement_starts: &'static [&'static str],
    /// A start word right after one of these (a word, or `,` `.`) continues the statement
    /// (`(` needs no row: an open parenthesis already holds the statement together).
    pub prev_exceptions: &'static [&'static str],
    pub head_continuations: &'static [HeadContinuation],
    /// Words between `CREATE`/`ALTER` and the routine kind (`OR REPLACE`, `DEFINER = …`,
    /// `TEMP`); strings, quoted identifiers, variables, parentheses and the value after a
    /// skip word's `=` (`DEFINER = admin@localhost`) are skipped too.
    pub routine_head_skips: &'static [&'static str],
    /// The routine kinds whose statements follow the family's block rules.
    pub routine_kinds: &'static [&'static str],
    /// A top-level `BEGIN` followed by one of these opens a block (MariaDB `BEGIN NOT ATOMIC`).
    pub top_level_block_begins: &'static [&'static str],
    /// A statement-position opener follows one of these (or a label's `:`).
    pub statement_position_after: &'static [&'static str],
    /// In a routine header (no block open yet), the body's first statement may follow one of
    /// these (`… FOR EACH ROW IF …`, `CREATE PROCEDURE p() IF …`), or a string (`COMMENT '…'`).
    pub body_start_after: &'static [&'static str],
    /// Openers that push a block only in statement position (MySQL `IF`, loops; a `CASE` in
    /// statement position is a CASE statement, which `END CASE` closes).
    pub statement_openers: &'static [(&'static str, BlockKind)],
    /// Words after `END` that name the block it closes (`END IF`, `END LOOP`).
    pub end_followers: &'static [(&'static str, BlockKind)],
}

impl SplitProfile {
    /// The profile's name, for diagnostics.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Every table row's id (`start:SELECT`, `prev:UNION`, `head:INSERT`, …), in table order:
    /// each has a fixture in `data/split/<name>.toml`.
    pub fn rows(&self) -> Vec<String> {
        let mut rows = Vec::new();
        let mut words = |table: &str, words: &[&str]| {
            rows.extend(words.iter().map(|w| format!("{table}:{w}")));
        };
        words("start", self.statement_starts);
        words("prev", self.prev_exceptions);
        words("skip", self.routine_head_skips);
        words("routine", self.routine_kinds);
        words("topblock", self.top_level_block_begins);
        words("pos", self.statement_position_after);
        words("body", self.body_start_after);
        rows.extend(
            self.head_continuations
                .iter()
                .map(|h| format!("head:{}", h.head)),
        );
        rows.extend(
            self.statement_openers
                .iter()
                .map(|(w, _)| format!("opener:{w}")),
        );
        rows.extend(self.end_followers.iter().map(|(w, _)| format!("end:{w}")));
        rows
    }
}

/// The splitter profile for `ctx`: PostgreSQL (and the PostgreSQL-wire servers), MySQL,
/// MariaDB, SQLite; every other dialect splits with the generic profile (parentheses only).
pub fn split_profile(ctx: &DialectCtx) -> &'static SplitProfile {
    match ctx.id {
        DialectId::Postgres | DialectId::PgWire(_) => &pg::PG,
        DialectId::MySql => &mysql::MYSQL,
        DialectId::MariaDb => &mysql::MARIADB,
        DialectId::Sqlite => &sqlite::SQLITE,
        _ => &generic::GENERIC,
    }
}

/// What a client's script conventions recognise (SQL-SPLIT-01). Every field is a table row
/// with a fixture, like the splitter tables.
#[non_exhaustive]
#[derive(Debug, PartialEq, Eq)]
pub struct ClientProfile {
    pub name: &'static str,
    /// A `\` outside quotes and comments starts a client command (psql; the mysql client).
    pub backslash_commands: bool,
    /// psql: a backslash command runs to the end of its line (or to a `\\` separator); the
    /// mysql client: only the `\` and one character, except `backslash_to_eol`.
    pub backslash_line: bool,
    /// The mysql client's one-letter commands whose argument runs to the end of the line
    /// (`\.` = `source`, `\u` = `use`, …): every letter but the terminators and `\c`.
    pub backslash_to_eol: bool,
    /// `\name` commands that end the statement, and whether Hacivat runs the statement they
    /// end as written (`\g`; psql `\gset` stores into variables Hacivat does not have).
    pub backslash_terminators: &'static [(&'static str, bool)],
    /// `\name` commands that discard the statement typed so far (psql `\r`, mysql `\c`).
    pub backslash_clear: &'static [&'static str],
    /// `\name` commands that change the delimiter (mysql `\d`).
    pub backslash_delimiter: &'static [&'static str],
    /// psql `\;`: a `;` that does not end the statement.
    pub inline_semicolon: bool,
    /// The delimiter command at statement start (mysql `DELIMITER`).
    pub delimiter_command: Option<&'static str>,
    /// Other line commands at statement start that Hacivat does not run (mysql `source`).
    pub line_commands: &'static [&'static str],
    /// sqlite3: a line that starts with `.` at the left margin, at statement start.
    pub dot_commands: bool,
    /// psql: after `COPY … FROM STDIN` ends, the lines up to `\.` are its data.
    pub copy_from_stdin: bool,
}

impl ClientProfile {
    /// Every row's id (`client:backslash`, `client:\g`, `client:DELIMITER`, …): each has a
    /// fixture.
    pub fn rows(&self) -> Vec<String> {
        let mut rows = Vec::new();
        let flags = [
            ("backslash", self.backslash_commands),
            ("backslash-line", self.backslash_line),
            ("backslash-to-eol", self.backslash_to_eol),
            ("inline-semicolon", self.inline_semicolon),
            ("dot", self.dot_commands),
            ("copy", self.copy_from_stdin),
        ];
        rows.extend(
            flags
                .iter()
                .filter(|(_, on)| *on)
                .map(|(name, _)| format!("client:{name}")),
        );
        rows.extend(
            self.backslash_terminators
                .iter()
                .map(|(n, _)| format!("client:\\{n}")),
        );
        rows.extend(self.backslash_clear.iter().map(|n| format!("client:\\{n}")));
        rows.extend(
            self.backslash_delimiter
                .iter()
                .map(|n| format!("client:\\{n}")),
        );
        rows.extend(self.delimiter_command.map(|c| format!("client:{c}")));
        rows.extend(self.line_commands.iter().map(|c| format!("client:{c}")));
        rows
    }
}

static NO_CLIENT: ClientProfile = ClientProfile {
    name: "none",
    backslash_commands: false,
    backslash_line: false,
    backslash_to_eol: false,
    backslash_terminators: &[],
    backslash_clear: &[],
    backslash_delimiter: &[],
    inline_semicolon: false,
    delimiter_command: None,
    line_commands: &[],
    dot_commands: false,
    copy_from_stdin: false,
};

/// psql (PostgreSQL's command-line client).
static PSQL: ClientProfile = ClientProfile {
    name: "psql",
    backslash_commands: true,
    backslash_line: true,
    backslash_to_eol: true,
    // `\g` and `\gx` send the buffer; the others send it too but do something Hacivat cannot
    // (store into variables, execute the result, describe without running).
    backslash_terminators: &[
        ("g", true),
        ("gx", true),
        ("gset", false),
        ("gexec", false),
        ("gdesc", false),
    ],
    backslash_clear: &["r", "reset"],
    backslash_delimiter: &[],
    inline_semicolon: true,
    delimiter_command: None,
    line_commands: &[],
    dot_commands: false,
    copy_from_stdin: true,
};

/// The mysql (and mariadb) command-line client.
static MYSQL_CLI: ClientProfile = ClientProfile {
    name: "mysql",
    backslash_commands: true,
    backslash_line: false,
    backslash_to_eol: true,
    backslash_terminators: &[("g", true), ("G", true)],
    backslash_clear: &["c"],
    backslash_delimiter: &["d"],
    inline_semicolon: false,
    delimiter_command: Some("DELIMITER"),
    line_commands: &["source"],
    dot_commands: false,
    copy_from_stdin: false,
};

/// The sqlite3 shell.
static SQLITE3: ClientProfile = ClientProfile {
    name: "sqlite3",
    backslash_commands: false,
    backslash_line: false,
    backslash_to_eol: false,
    backslash_terminators: &[],
    backslash_clear: &[],
    backslash_delimiter: &[],
    inline_semicolon: false,
    delimiter_command: None,
    line_commands: &[],
    dot_commands: true,
    copy_from_stdin: false,
};

/// The client conventions of `flavor` (`ClientFlavor::None`: none, every line is SQL).
pub fn client_profile(flavor: ClientFlavor) -> &'static ClientProfile {
    match flavor {
        ClientFlavor::Psql => &PSQL,
        ClientFlavor::MySqlCli => &MYSQL_CLI,
        ClientFlavor::Sqlite3 => &SQLITE3,
        _ => &NO_CLIENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::LexModes;

    fn profile(id: DialectId) -> &'static SplitProfile {
        split_profile(&DialectCtx {
            id,
            version: None,
            modes: LexModes::default_for(id),
        })
    }

    #[test]
    fn each_dialect_has_its_profile() {
        assert_eq!(profile(DialectId::Postgres).name(), "pg");
        assert_eq!(
            profile(DialectId::PgWire(crate::id::PgWireFlavor::Cockroach)).name(),
            "pg"
        );
        assert_eq!(profile(DialectId::MySql).name(), "mysql");
        assert_eq!(profile(DialectId::MariaDb).name(), "mariadb");
        assert_eq!(profile(DialectId::Sqlite).name(), "sqlite");
        for id in [
            DialectId::Generic,
            DialectId::SqlServer,
            DialectId::ClickHouse,
            DialectId::Oracle,
        ] {
            assert_eq!(profile(id).name(), "generic");
        }
    }

    #[test]
    fn row_ids_are_unique_within_a_profile() {
        for id in [
            DialectId::Postgres,
            DialectId::MySql,
            DialectId::MariaDb,
            DialectId::Sqlite,
            DialectId::Generic,
        ] {
            let rows = profile(id).rows();
            let mut sorted = rows.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), rows.len(), "{}", profile(id).name());
        }
        for flavor in [
            ClientFlavor::Psql,
            ClientFlavor::MySqlCli,
            ClientFlavor::Sqlite3,
        ] {
            let rows = client_profile(flavor).rows();
            let mut sorted = rows.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), rows.len(), "{flavor:?}");
        }
        assert!(client_profile(ClientFlavor::None).rows().is_empty());
    }

    #[test]
    fn mariadb_adds_for_loops_to_mysql() {
        let (my, maria) = (profile(DialectId::MySql), profile(DialectId::MariaDb));
        assert!(!my.statement_openers.iter().any(|(w, _)| *w == "FOR"));
        assert!(maria.statement_openers.contains(&("FOR", BlockKind::For)));
        assert!(maria.end_followers.contains(&("FOR", BlockKind::For)));
        assert_eq!(my.statement_starts, maria.statement_starts);
    }
}
