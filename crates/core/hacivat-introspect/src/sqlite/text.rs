// ## m4-t18-sqlite
//! What only `sqlite_schema.sql` says (F10): a small tokenizer over the server's own `CREATE`
//! text, and the facts read from it — `WITHOUT ROWID` and `STRICT` (when `pragma_table_list`
//! is not available: the user's connection, R-M4-38), a virtual table's module, a column's
//! declared collation and generated expression, `AUTOINCREMENT`, each foreign key's deferral,
//! and a trigger's timing, events and `WHEN`.
//!
//! The text is the server's (it parsed it when it stored it), so the scan only has to find
//! structure, never to validate it: strings (`'…'`), quoted names (`"…"`, `` `…` ``, `[…]`),
//! comments (`--`, `/* */`) and nesting are skipped so a word or a parenthesis inside them is
//! never read as code. Keywords compare ASCII-caselessly (SQLite's own rule). What the scan
//! cannot place is reported as unknown (`None`, the default), never guessed.
//!
//! `table_options` and `virtual_module` keep the behaviour of the MySQL/SQLite spike's scanners
//! (spike origin `f690f45`, `sqlite.rs`), rebuilt on the shared tokenizer.

use std::ops::Range;

/// One token of SQL text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tok {
    /// A bare word: a keyword or an unquoted name.
    Word,
    /// `"…"`, `` `…` `` or `[…]`.
    Quoted,
    /// `'…'`.
    Str,
    LParen,
    RParen,
    Comma,
    Dot,
    Semi,
    /// Anything else (operators, numbers' digits, `?`).
    Other,
}

/// The tokens of `sql` with their byte ranges, comments and white space left out.
pub(crate) fn tokens(sql: &str) -> Vec<(Tok, Range<usize>)> {
    let b = sql.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let start = i;
        let tok = match c {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0c => {
                i += 1;
                continue;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                i = b[i..]
                    .iter()
                    .position(|&x| x == b'\n')
                    .map_or(b.len(), |p| i + p);
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = find(b, i + 2, b"*/").map_or(b.len(), |p| p + 2);
                continue;
            }
            b'\'' => {
                i = close_quote(b, i, b'\'');
                Tok::Str
            }
            b'"' | b'`' => {
                i = close_quote(b, i, c);
                Tok::Quoted
            }
            b'[' => {
                i = b[i..]
                    .iter()
                    .position(|&x| x == b']')
                    .map_or(b.len(), |p| i + p + 1);
                Tok::Quoted
            }
            b'(' => {
                i += 1;
                Tok::LParen
            }
            b')' => {
                i += 1;
                Tok::RParen
            }
            b',' => {
                i += 1;
                Tok::Comma
            }
            b'.' => {
                i += 1;
                Tok::Dot
            }
            b';' => {
                i += 1;
                Tok::Semi
            }
            c if is_word_byte(c) => {
                while i < b.len() && is_word_byte(b[i]) {
                    i += 1;
                }
                Tok::Word
            }
            _ => {
                // One whole UTF-8 character (a non-ASCII byte is a word byte above).
                i += 1;
                Tok::Other
            }
        };
        out.push((tok, start..i));
    }
    out
}

/// A byte that continues a bare word (SQLite: letters, digits, `_`, `$`, any non-ASCII byte).
fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

/// The index after the closing `q` of the quoted run starting at `at` (`qq` is an escaped
/// quote); the end of the text when it never closes.
fn close_quote(b: &[u8], at: usize, q: u8) -> usize {
    let mut i = at + 1;
    while i < b.len() {
        if b[i] == q {
            if b.get(i + 1) == Some(&q) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    b.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// The tokens of one text, with helpers.
struct Toks<'a> {
    sql: &'a str,
    t: Vec<(Tok, Range<usize>)>,
}

impl<'a> Toks<'a> {
    fn new(sql: &'a str) -> Self {
        Toks {
            sql,
            t: tokens(sql),
        }
    }

    fn len(&self) -> usize {
        self.t.len()
    }

    fn kind(&self, i: usize) -> Option<Tok> {
        self.t.get(i).map(|t| t.0)
    }

    fn text(&self, i: usize) -> &'a str {
        self.t.get(i).map_or("", |t| &self.sql[t.1.clone()])
    }

    /// Whether token `i` is the bare word `w` (ASCII-caseless).
    fn is(&self, i: usize, w: &str) -> bool {
        self.kind(i) == Some(Tok::Word) && self.text(i).eq_ignore_ascii_case(w)
    }

    /// The name token `i` spells: a bare word as written, a quoted name unquoted.
    fn name(&self, i: usize) -> Option<String> {
        match self.kind(i)? {
            Tok::Word => Some(self.text(i).to_string()),
            Tok::Quoted | Tok::Str => Some(unquote(self.text(i))),
            _ => None,
        }
    }

    /// The text from the start of token `a` to the end of token `b` (inclusive), as written.
    fn span(&self, a: usize, b: usize) -> &'a str {
        match (self.t.get(a), self.t.get(b)) {
            (Some(x), Some(y)) if x.1.start <= y.1.end => &self.sql[x.1.start..y.1.end],
            _ => "",
        }
    }

    /// The index of the `)` closing the `(` at `open`.
    fn matching(&self, open: usize) -> Option<usize> {
        let mut depth = 0usize;
        for i in open..self.len() {
            match self.kind(i)? {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The first `(` at depth 0 and its `)`: a `CREATE TABLE`'s definition list.
    fn first_group(&self) -> Option<(usize, usize)> {
        let open = (0..self.len()).find(|&i| self.kind(i) == Some(Tok::LParen))?;
        Some((open, self.matching(open)?))
    }

    /// The elements of the group `(open, close)`: token ranges split at its depth-1 commas.
    fn elements(&self, open: usize, close: usize) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut depth = 0usize;
        let mut start = open + 1;
        for i in open + 1..close {
            match self.kind(i) {
                Some(Tok::LParen) => depth += 1,
                Some(Tok::RParen) => depth = depth.saturating_sub(1),
                Some(Tok::Comma) if depth == 0 => {
                    out.push(start..i);
                    start = i + 1;
                }
                _ => {}
            }
        }
        if start < close {
            out.push(start..close);
        }
        out
    }

    /// The indices of `range` at its own depth 0 (outside every parenthesis).
    fn top(&self, range: Range<usize>) -> Vec<usize> {
        let mut out = Vec::new();
        let mut depth = 0usize;
        for i in range {
            match self.kind(i) {
                Some(Tok::LParen) => {
                    if depth == 0 {
                        out.push(i);
                    }
                    depth += 1;
                }
                Some(Tok::RParen) => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        out.push(i);
                    }
                }
                _ if depth == 0 => out.push(i),
                _ => {}
            }
        }
        out
    }
}

/// A quoted name's text: the quotes removed, a doubled quote made one.
pub(crate) fn unquote(t: &str) -> String {
    let Some(first) = t.chars().next() else {
        return String::new();
    };
    match first {
        '[' => t[1..].strip_suffix(']').unwrap_or(&t[1..]).to_string(),
        '"' | '`' | '\'' => {
            let inner = &t[1..];
            let inner = inner.strip_suffix(first).unwrap_or(inner);
            let doubled: String = [first, first].iter().collect();
            inner.replace(&doubled, &first.to_string())
        }
        _ => t.to_string(),
    }
}

/// `(WITHOUT ROWID, STRICT)` of a `CREATE TABLE` text: the words after the definition list's
/// closing parenthesis.
pub(crate) fn table_options(sql: &str) -> (bool, bool) {
    let t = Toks::new(sql);
    let Some((_, close)) = t.first_group() else {
        return (false, false);
    };
    let mut without_rowid = false;
    let mut strict = false;
    for i in close + 1..t.len() {
        if t.is(i, "WITHOUT") && t.is(i + 1, "ROWID") {
            without_rowid = true;
        }
        if t.is(i, "STRICT") {
            strict = true;
        }
    }
    (without_rowid, strict)
}

/// `CREATE VIRTUAL TABLE x USING fts5(…)` → `fts5`.
pub(crate) fn virtual_module(sql: &str) -> Option<String> {
    let t = Toks::new(sql);
    if !(t.is(0, "CREATE") && t.is(1, "VIRTUAL") && t.is(2, "TABLE")) {
        return None;
    }
    let using = (3..t.len()).find(|&i| t.is(i, "USING"))?;
    t.name(using + 1)
}

/// What a column definition of a `CREATE TABLE` says beyond the pragmas.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ColumnDef {
    pub name: String,
    /// The last `COLLATE` name, as written.
    pub collation: Option<String>,
    /// The `AS ( … )` expression of a generated column, as written.
    pub generated: Option<String>,
    pub autoincrement: bool,
}

/// One `REFERENCES` clause, in text order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FkClause {
    pub deferrable: bool,
    pub initially_deferred: bool,
}

/// The column definitions and foreign-key clauses of a `CREATE TABLE` text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TableDef {
    pub columns: Vec<ColumnDef>,
    pub fks: Vec<FkClause>,
}

impl TableDef {
    /// The definition of column `name` (SQLite compares column names ASCII-caselessly).
    pub fn column(&self, name: &str) -> Option<&ColumnDef> {
        self.columns
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }
}

/// Words that start a table constraint (anything else starts a column definition).
const TABLE_CONSTRAINT: [&str; 5] = ["CONSTRAINT", "PRIMARY", "UNIQUE", "CHECK", "FOREIGN"];

/// Parses the definition list of a `CREATE TABLE` text; empty for any other shape (`AS SELECT`).
pub(crate) fn table_def(sql: &str) -> TableDef {
    let t = Toks::new(sql);
    let mut def = TableDef::default();
    // `CREATE [TEMP|TEMPORARY] TABLE`; a virtual table's list is its module's arguments.
    if !(t.is(0, "CREATE") && (1..3).any(|i| t.is(i, "TABLE"))) || t.is(1, "VIRTUAL") {
        return def;
    }
    let Some((open, close)) = t.first_group() else {
        return def;
    };
    for el in t.elements(open, close) {
        let top = t.top(el.clone());
        let Some(&first) = top.first() else { continue };
        let constraint = TABLE_CONSTRAINT.iter().any(|w| t.is(first, w));
        if !constraint && let Some(name) = t.name(first) {
            let mut col = ColumnDef {
                name,
                ..ColumnDef::default()
            };
            for (k, &i) in top.iter().enumerate() {
                if t.is(i, "COLLATE")
                    && let Some(&n) = top.get(k + 1)
                {
                    col.collation = t.name(n);
                } else if t.is(i, "AS")
                    && let Some(&open) = top.get(k + 1)
                    && t.kind(open) == Some(Tok::LParen)
                    && let Some(close) = t.matching(open)
                    && close > open + 1
                {
                    col.generated = Some(t.span(open + 1, close - 1).trim().to_string());
                } else if t.is(i, "AUTOINCREMENT") {
                    col.autoincrement = true;
                }
            }
            def.columns.push(col);
        }
        fk_clauses(&t, &top, &mut def.fks);
    }
    def
}

/// Every `REFERENCES` clause among `top` (one element's depth-0 tokens) with its deferral.
/// SQLite applies a `[NOT] DEFERRABLE [INITIALLY DEFERRED | INITIALLY IMMEDIATE]` clause to the
/// most recent foreign key of the table, wherever it stands — after its `REFERENCES`, or alone
/// in a later column (`sqlite3DeferForeignKey`; review m1) — so the scan walks the elements in
/// order and updates the last clause pushed. The element's first token (a column's name) is
/// never a clause word.
fn fk_clauses(t: &Toks<'_>, top: &[usize], out: &mut Vec<FkClause>) {
    for (k, &i) in top.iter().enumerate().skip(1) {
        if t.is(i, "REFERENCES") {
            out.push(FkClause::default());
        } else if t.is(i, "DEFERRABLE")
            && let Some(fk) = out.last_mut()
        {
            let negated = k > 1 && t.is(top[k - 1], "NOT");
            fk.deferrable = !negated;
            fk.initially_deferred = !negated
                && top
                    .get(k + 2)
                    .is_some_and(|&d| t.is(top[k + 1], "INITIALLY") && t.is(d, "DEFERRED"));
        }
    }
}

/// When a trigger fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Timing {
    #[default]
    Before,
    After,
    InsteadOf,
}

/// What a trigger fires on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
    Insert,
    Update,
    Delete,
}

/// The shape of a `CREATE TRIGGER` text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TriggerDef {
    pub timing: Timing,
    pub event: Event,
    /// The `WHEN` expression, as written.
    pub when: Option<String>,
}

/// The words a trigger body's statement starts with.
const BODY_STARTS: [&str; 6] = ["SELECT", "INSERT", "UPDATE", "DELETE", "REPLACE", "WITH"];

/// `CREATE [TEMP] TRIGGER [IF NOT EXISTS] [schema.]name [BEFORE|AFTER|INSTEAD OF]
/// DELETE|INSERT|UPDATE [OF cols] ON table [FOR EACH ROW] [WHEN expr] BEGIN …`; `None` for
/// another shape.
pub(crate) fn trigger_def(sql: &str) -> Option<TriggerDef> {
    let t = Toks::new(sql);
    let mut i = 0;
    if !t.is(i, "CREATE") {
        return None;
    }
    i += 1;
    if t.is(i, "TEMP") || t.is(i, "TEMPORARY") {
        i += 1;
    }
    if !t.is(i, "TRIGGER") {
        return None;
    }
    i += 1;
    if t.is(i, "IF") && t.is(i + 1, "NOT") && t.is(i + 2, "EXISTS") {
        i += 3;
    }
    // The name (possibly `schema.name`).
    t.name(i)?;
    i += 1;
    if t.kind(i) == Some(Tok::Dot) {
        t.name(i + 1)?;
        i += 2;
    }
    let timing = if t.is(i, "BEFORE") {
        i += 1;
        Timing::Before
    } else if t.is(i, "AFTER") {
        i += 1;
        Timing::After
    } else if t.is(i, "INSTEAD") && t.is(i + 1, "OF") {
        i += 2;
        Timing::InsteadOf
    } else {
        Timing::Before
    };
    let event = if t.is(i, "DELETE") {
        Event::Delete
    } else if t.is(i, "INSERT") {
        Event::Insert
    } else if t.is(i, "UPDATE") {
        Event::Update
    } else {
        return None;
    };
    let on = (i + 1..t.len()).find(|&k| t.is(k, "ON"))?;
    // The body's `BEGIN`: a bare word at the top level, not a qualified name's part, and
    // followed by a statement (`BEGIN` is also a fallback identifier, so `WHEN new.begin > 0`
    // or a column named `begin` must not end the `WHEN` text; review m2).
    let mut depth = 0usize;
    let begin = (on + 1..t.len()).find(|&k| {
        match t.kind(k) {
            Some(Tok::LParen) => depth += 1,
            Some(Tok::RParen) => depth = depth.saturating_sub(1),
            _ => {}
        }
        depth == 0
            && t.is(k, "BEGIN")
            && t.kind(k - 1) != Some(Tok::Dot)
            && t.kind(k + 1) != Some(Tok::Dot)
            && BODY_STARTS.iter().any(|w| t.is(k + 1, w))
    })?;
    let when = (on + 1..begin)
        .find(|&k| t.is(k, "WHEN"))
        .filter(|&k| k + 1 < begin)
        .map(|k| t.span(k + 1, begin - 1).trim().to_string());
    Some(TriggerDef {
        timing,
        event,
        when,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The spike's cases (`f690f45`).
    #[test]
    fn table_options_scan_skips_strings_comments_and_nesting() {
        assert_eq!(table_options("CREATE TABLE t(a)"), (false, false));
        assert_eq!(
            table_options("CREATE TABLE t(a PRIMARY KEY) WITHOUT ROWID"),
            (true, false)
        );
        assert_eq!(
            table_options("CREATE TABLE t(a INT, CHECK (a > 0)) strict, without  rowid"),
            (true, true)
        );
        assert_eq!(
            table_options("CREATE TABLE t(a DEFAULT ')', b TEXT /* ) STRICT */) "),
            (false, false)
        );
        assert_eq!(
            table_options("CREATE TABLE \"x)\"(a) -- WITHOUT ROWID\n"),
            (false, false)
        );
        assert_eq!(
            table_options("CREATE TABLE [we(ird](a, b AS (a*2)) STRICT"),
            (false, true)
        );
    }

    #[test]
    fn virtual_module_is_read_from_using() {
        assert_eq!(
            virtual_module("CREATE VIRTUAL TABLE f USING fts5(a, b)").as_deref(),
            Some("fts5")
        );
        assert_eq!(
            virtual_module("CREATE VIRTUAL TABLE r USING rtree (id, x0, x1)").as_deref(),
            Some("rtree")
        );
        assert_eq!(virtual_module("CREATE TABLE t(a)"), None);
        assert_eq!(
            virtual_module("create virtual table \"a USING b\" using \"My Mod\"(x)").as_deref(),
            Some("My Mod")
        );
    }

    #[test]
    fn column_definitions_give_collation_generation_and_autoincrement() {
        let d = table_def(
            "CREATE TABLE t(id integer primary key autoincrement, \"b, c\" TEXT COLLATE \"NoCase\", \
             g AS (a + ')' || \"x(\") STORED, d DEFAULT (cast(1 AS int)), \
             [e] TEXT COLLATE rtrim NOT NULL COLLATE binary, CONSTRAINT k UNIQUE (d))",
        );
        let names: Vec<&str> = d.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["id", "b, c", "g", "d", "e"]);
        assert!(d.columns[0].autoincrement);
        assert_eq!(
            d.column("B, C").unwrap().collation.as_deref(),
            Some("NoCase")
        );
        assert_eq!(
            d.column("g").unwrap().generated.as_deref(),
            Some("a + ')' || \"x(\"")
        );
        assert_eq!(
            d.column("d").unwrap().generated,
            None,
            "AS inside DEFAULT ( … )"
        );
        assert_eq!(d.column("e").unwrap().collation.as_deref(), Some("binary"));
        assert_eq!(
            table_def("CREATE TABLE t AS SELECT 1 AS a"),
            TableDef::default()
        );
    }

    #[test]
    fn fk_deferral_per_clause_in_text_order() {
        let d = table_def(
            "CREATE TABLE c(a REFERENCES p DEFERRABLE INITIALLY DEFERRED, \
             b REFERENCES p(x) NOT DEFERRABLE INITIALLY DEFERRED NOT NULL, \
             c CHECK (c <> 'REFERENCES q DEFERRABLE'), \
             FOREIGN KEY (c) REFERENCES q(y) ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE, \
             FOREIGN KEY (a) REFERENCES r DEFERRABLE)",
        );
        assert_eq!(
            d.fks,
            vec![
                FkClause {
                    deferrable: true,
                    initially_deferred: true
                },
                FkClause::default(),
                FkClause {
                    deferrable: true,
                    initially_deferred: false
                },
                FkClause {
                    deferrable: true,
                    initially_deferred: false
                },
            ]
        );
    }

    #[test]
    fn a_stray_deferrable_applies_to_the_previous_fk() {
        let d = table_def(
            "CREATE TABLE c(a REFERENCES p(id), b DEFERRABLE INITIALLY DEFERRED, \
             e REFERENCES q, f NOT DEFERRABLE, deferrable INT)",
        );
        assert_eq!(
            d.fks,
            vec![
                FkClause {
                    deferrable: true,
                    initially_deferred: true
                },
                FkClause::default(),
            ]
        );
        // Before any FK, a stray clause has nothing to apply to.
        assert!(table_def("CREATE TABLE c(a DEFERRABLE, b)").fks.is_empty());
    }

    #[test]
    fn trigger_when_is_not_cut_at_a_begin_identifier() {
        let t = trigger_def(
            "CREATE TRIGGER t1 AFTER INSERT ON t WHEN new.begin > 0 AND (begin + 1) > begin \
             BEGIN UPDATE t SET a = 1; END",
        )
        .unwrap();
        assert_eq!(
            t.when.as_deref(),
            Some("new.begin > 0 AND (begin + 1) > begin")
        );
    }

    #[test]
    fn trigger_shapes() {
        let t = trigger_def(
            "CREATE TRIGGER IF NOT EXISTS main.\"after\" AFTER UPDATE OF a, b ON t \
             FOR EACH ROW WHEN new.a > 0 AND old.b <> 'BEGIN' BEGIN SELECT 1; END",
        )
        .unwrap();
        assert_eq!((t.timing, t.event), (Timing::After, Event::Update));
        assert_eq!(t.when.as_deref(), Some("new.a > 0 AND old.b <> 'BEGIN'"));
        let t = trigger_def("create temp trigger x instead of delete on v begin select 2; end")
            .unwrap();
        assert_eq!(
            (t.timing, t.event, t.when),
            (Timing::InsteadOf, Event::Delete, None)
        );
        let t = trigger_def("CREATE TRIGGER y INSERT ON t BEGIN SELECT 3; END").unwrap();
        assert_eq!((t.timing, t.event), (Timing::Before, Event::Insert));
        assert_eq!(trigger_def("CREATE TABLE t(a)"), None);
    }

    #[test]
    fn unterminated_text_never_panics() {
        for sql in [
            "CREATE TABLE t(a '",
            "CREATE TABLE t(\"a",
            "CREATE TABLE [t(a",
            "CREATE TABLE t(a /*",
            "CREATE TRIGGER",
            "CREATE TABLE t(a COLLATE",
            "CREATE TABLE t(a AS (",
            "",
        ] {
            let _ = (
                table_def(sql),
                table_options(sql),
                trigger_def(sql),
                virtual_module(sql),
            );
        }
    }
}
