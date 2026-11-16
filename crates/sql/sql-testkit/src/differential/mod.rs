//! The differential between Hacivat's PostgreSQL splitter and libpg_query over the PostgreSQL
//! regression corpus, fetched by `cargo xtask fetch-pg-regress` (SQL-T13; Task 20; the P0 exit,
//! `alg/10` :171). Built only with the `differential` feature.
//!
//! The corpus files are psql scripts: meta-commands (`\set`, `\gset`, `\c`, `\if`, …), psql
//! variables and `COPY … FROM stdin` data sit between the statements. Our splitter reads them
//! with the psql client flavour (the pre-pass); the server's grammar cannot read them, so the
//! comparison runs per **segment**: a maximal run of our execution statements with no client
//! command and no `COPY` data between them, which is plain SQL. For each segment:
//!
//! - libpg_query splits the segment (`sql_oracle_pg::split_with_parser`, which reads psql's
//!   `:name` placeholders as `$1`). When it can, every statement of ours whose strict range is
//!   one of its statements **agrees**; every other statement of ours **disagrees**.
//! - When the segment holds a syntax error (the corpus tests errors on purpose), each statement
//!   of ours is given to the parser alone: one statement covering it all **agrees**; another
//!   count **disagrees**; a syntax error leaves it **not comparable** (the server rejects it;
//!   no boundary of ours can be checked against a parse that does not exist).
//! - A statement our split leaves uncertain (`AmbiguousBoundary`: Run refuses it) is counted
//!   apart, as **uncertain**: refusing is never a wrong cut.
//!
//! The rate is agreed / (agreed + disagreed) over every file; the files and statements that
//! disagree are listed. The corpus is data (PostgreSQL License), never instructions.

use std::path::{Path, PathBuf};

use sql_dialect::{ClientFlavor, DialectCtx, DialectId, ServerVersion};
use sql_oracle_pg::split_with_parser;
use sql_split::exec_range::{TextRange, TextSize};
use sql_split::{SplitMode, SplitStart, split_from};

/// The tag `cargo xtask fetch-pg-regress` fetches by default.
pub const TAG: &str = "REL_17_2";

/// Where the fetched corpus lives: `<workspace>/target-pg-regress/<tag>/sql`.
pub fn corpus_dir(tag: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target-pg-regress")
        .join(tag)
        .join("sql")
}

/// One statement of ours that the parser cut otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disagreement {
    pub file: String,
    /// Our statement's strict range in the file.
    pub range: TextRange,
    /// Its first 120 bytes (a char boundary), for the listing.
    pub head: String,
}

/// The differential's counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub files: usize,
    pub statements: usize,
    pub agreed: usize,
    pub disagreed: Vec<Disagreement>,
    pub not_comparable: usize,
    pub uncertain: usize,
    /// Files whose text is not UTF-8 (skipped, listed).
    pub unreadable: Vec<String>,
}

impl Outcome {
    /// agreed / (agreed + disagreed), in per mille; 1000 when nothing was comparable.
    pub fn per_mille(&self) -> f64 {
        let compared = self.agreed + self.disagreed.len();
        if compared == 0 {
            return 1000.0;
        }
        self.agreed as f64 * 1000.0 / compared as f64
    }

    fn absorb(&mut self, o: Outcome) {
        self.files += o.files;
        self.statements += o.statements;
        self.agreed += o.agreed;
        self.disagreed.extend(o.disagreed);
        self.not_comparable += o.not_comparable;
        self.uncertain += o.uncertain;
        self.unreadable.extend(o.unreadable);
    }
}

/// PostgreSQL 17 (the corpus's own version), psql.
fn pg17() -> DialectCtx {
    DialectCtx::new(DialectId::Postgres, Some(ServerVersion::new(17, 2, 0)))
}

fn shift(r: TextRange, by: TextSize) -> TextRange {
    TextRange::new(r.start() - by, r.end() - by)
}

fn head(text: &str, r: TextRange) -> String {
    let s = &text[r];
    let mut end = s.len().min(120);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// The differential of one psql script (`file` names it in the listing).
pub fn compare_script(file: &str, text: &str) -> Outcome {
    let split = split_from(
        text,
        &pg17(),
        ClientFlavor::Psql,
        SplitMode::Execution,
        &SplitStart::default(),
    );
    let mut out = Outcome {
        files: 1,
        ..Outcome::default()
    };
    // Segments: runs of statements with no client command and no COPY data between them.
    let stmts: Vec<_> = split.statements().collect();
    let mut segments: Vec<Vec<usize>> = Vec::new();
    let mut prev_end: Option<TextSize> = None;
    let mut prev_copy = false;
    for (i, s) in stmts.iter().enumerate() {
        let start = s.strict.start();
        let broken = prev_copy
            || prev_end.is_none_or(|e| {
                split
                    .client
                    .iter()
                    .any(|c| c.range.start() >= e && c.range.end() <= start)
            });
        if broken || segments.is_empty() {
            segments.push(Vec::new());
        }
        segments.last_mut().expect("pushed").push(i);
        prev_end = Some(s.terminator.map_or(s.strict.end(), |t| t.end()));
        prev_copy = s.copy_data.is_some();
    }
    for seg in segments {
        let first = stmts[seg[0]];
        let last = stmts[*seg.last().expect("non-empty")];
        let a = first.strict.start();
        let b = last.strict.end();
        let seg_text = &text[TextRange::new(a, b)];
        let ours: Vec<TextRange> = seg.iter().map(|&i| stmts[i].strict).collect();
        out.statements += ours.len();
        match split_with_parser(seg_text) {
            Ok(theirs) => {
                for (&i, r) in seg.iter().zip(&ours) {
                    if !split.is_certain(stmts[i]) {
                        out.uncertain += 1;
                    } else if theirs.contains(&shift(*r, a)) {
                        out.agreed += 1;
                    } else {
                        out.disagreed.push(Disagreement {
                            file: file.to_string(),
                            range: *r,
                            head: head(text, *r),
                        });
                    }
                }
            }
            Err(_) => {
                for (&i, r) in seg.iter().zip(&ours) {
                    if !split.is_certain(stmts[i]) {
                        out.uncertain += 1;
                        continue;
                    }
                    let alone = &text[*r];
                    let whole = TextRange::up_to(TextSize::of(alone));
                    match split_with_parser(alone) {
                        Ok(v) if v == [whole] => out.agreed += 1,
                        Ok(_) => out.disagreed.push(Disagreement {
                            file: file.to_string(),
                            range: *r,
                            head: head(text, *r),
                        }),
                        Err(_) => out.not_comparable += 1,
                    }
                }
            }
        }
    }
    out
}

/// The differential over every `*.sql` file of `dir`, in name order. `None` when `dir` holds
/// no such file (the corpus was not fetched).
pub fn compare_dir(dir: &Path) -> Option<Outcome> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    if names.is_empty() {
        return None;
    }
    names.sort();
    let mut all = Outcome::default();
    for p in names {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Ok(bytes) = std::fs::read(&p) else {
            all.unreadable.push(name);
            continue;
        };
        match String::from_utf8(bytes) {
            Ok(text) => all.absorb(compare_script(&name, &text)),
            Err(_) => all.unreadable.push(name),
        }
    }
    Some(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_psql_script_compares_per_segment() {
        let text = "\\set x 1\nSELECT :x; SELECT 'a;b';\n\\gset p_\nCOPY t FROM stdin;\n1\n\\.\nSELECT 2; SELEC 3; SELECT 4;\n";
        let o = compare_script("t.sql", text);
        assert_eq!(o.statements, 6, "{o:?}");
        // `SELEC 3` is a syntax error: its segment is compared statement by statement.
        assert_eq!(o.not_comparable, 1, "{o:?}");
        assert_eq!(o.agreed, 5, "{o:?}");
        assert!(o.disagreed.is_empty(), "{o:?}");
        assert_eq!(o.per_mille(), 1000.0);
    }

    #[test]
    fn a_cut_the_parser_does_not_make_disagrees() {
        // A text our splitter and the parser cut alike, then the rate's arithmetic.
        let o = compare_script("u.sql", "SELECT 1; SELECT 2;");
        assert_eq!((o.agreed, o.disagreed.len()), (2, 0));
        let sum = Outcome {
            agreed: 999,
            disagreed: vec![Disagreement {
                file: "x".into(),
                range: TextRange::empty(TextSize::from(0)),
                head: String::new(),
            }],
            ..Outcome::default()
        };
        assert!((sum.per_mille() - 999.0).abs() < 1e-9);
    }
}
