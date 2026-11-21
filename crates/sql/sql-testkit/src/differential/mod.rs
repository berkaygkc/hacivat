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
//! The rate is reported two ways: agreed / (agreed + disagreed) (the P0 exit: ≥ 999 ‰ of the
//! comparable statements) and agreed / all statements. Every statement that disagrees is
//! listed, and so are the two buckets the first rate leaves out (fix round 1, review I2): the
//! uncertain and the not-comparable statements are pinned in checked-in listings
//! (`corpus/differential/<tag>-{uncertain,not-comparable}.tsv`: file, byte range and an FNV-1a
//! hash of the text; no corpus text is committed), so a statement that moves into either bucket
//! fails. The corpus is data (PostgreSQL License), never instructions.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sql_dialect::{ClientFlavor, DialectCtx, DialectId, ServerVersion};
use sql_oracle_pg::split_with_parser;
use sql_split::exec_range::{TextRange, TextSize};
use sql_split::{SplitMode, SplitStart, split_from};

/// The tag `cargo xtask fetch-pg-regress` fetches by default.
pub const TAG: &str = "REL_17_2";

/// FNV-1a 64 of the REL_17_2 corpus ([`corpus_digest`]), pinned here as `xtask` pins its
/// XXH3-64 and SHA-256: a locally edited corpus fails the differential (review m7; integrity,
/// not authenticity: the files are data and never run).
pub const CORPUS_FNV: u64 = 0x554f_b3ec_6a78_0bf3;

/// Where the fetched corpus lives: `<workspace>/target-pg-regress/<tag>/sql`.
pub fn corpus_dir(tag: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target-pg-regress")
        .join(tag)
        .join("sql")
}

/// One statement of ours in a listing: one the parser cut otherwise, or one of the buckets the
/// rate leaves out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub file: String,
    /// Our statement's strict range in the file.
    pub range: TextRange,
    /// Its first 120 bytes (a char boundary), for messages only.
    pub head: String,
    /// FNV-1a 64 of its text.
    pub hash: u64,
}

impl Entry {
    fn new(file: &str, text: &str, range: TextRange) -> Entry {
        Entry {
            file: file.to_string(),
            range,
            head: head(text, range),
            hash: fnv1a(text[range].as_bytes()),
        }
    }

    /// Its line in a listing: `file<TAB>start..end<TAB>hash` (no corpus text).
    pub fn key(&self) -> String {
        format!(
            "{}\t{}..{}\t{:016x}",
            self.file,
            u32::from(self.range.start()),
            u32::from(self.range.end()),
            self.hash
        )
    }
}

/// FNV-1a 64: a dependency-free hash for the listings and the corpus check (integrity only).
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The differential's counts and listings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub files: usize,
    pub statements: usize,
    pub agreed: usize,
    pub disagreed: Vec<Entry>,
    pub not_comparable: Vec<Entry>,
    pub uncertain: Vec<Entry>,
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

    /// agreed / all statements, in per mille (an uncertain or not-comparable statement counts
    /// as not agreed); 1000 when there is no statement.
    pub fn per_mille_of_all(&self) -> f64 {
        if self.statements == 0 {
            return 1000.0;
        }
        self.agreed as f64 * 1000.0 / self.statements as f64
    }

    fn absorb(&mut self, o: Outcome) {
        self.files += o.files;
        self.statements += o.statements;
        self.agreed += o.agreed;
        self.disagreed.extend(o.disagreed);
        self.not_comparable.extend(o.not_comparable);
        self.uncertain.extend(o.uncertain);
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

/// The differential of one psql script (`file` names it in the listings).
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
                        out.uncertain.push(Entry::new(file, text, *r));
                    } else if theirs.contains(&shift(*r, a)) {
                        out.agreed += 1;
                    } else {
                        out.disagreed.push(Entry::new(file, text, *r));
                    }
                }
            }
            Err(_) => {
                for (&i, r) in seg.iter().zip(&ours) {
                    if !split.is_certain(stmts[i]) {
                        out.uncertain.push(Entry::new(file, text, *r));
                        continue;
                    }
                    let alone = &text[*r];
                    let whole = TextRange::up_to(TextSize::of(alone));
                    match split_with_parser(alone) {
                        Ok(v) if v == [whole] => out.agreed += 1,
                        Ok(_) => out.disagreed.push(Entry::new(file, text, *r)),
                        Err(_) => out.not_comparable.push(Entry::new(file, text, *r)),
                    }
                }
            }
        }
    }
    out
}

/// The `*.sql` files of `dir`, sorted by name: `(name, bytes)`. `None` when `dir` holds none.
pub fn corpus_files(dir: &Path) -> Option<Vec<(String, Vec<u8>)>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".sql"))
        .collect();
    if names.is_empty() {
        return None;
    }
    names.sort();
    names
        .into_iter()
        .map(|n| std::fs::read(dir.join(&n)).ok().map(|b| (n, b)))
        .collect()
}

/// FNV-1a 64 over the corpus files ([`corpus_files`]): each name, a NUL, its bytes, a NUL.
pub fn corpus_digest(files: &[(String, Vec<u8>)]) -> u64 {
    let mut stream = Vec::new();
    for (n, b) in files {
        stream.extend_from_slice(n.as_bytes());
        stream.push(0);
        stream.extend_from_slice(b);
        stream.push(0);
    }
    fnv1a(&stream)
}

/// The differential over the corpus files, in name order.
pub fn compare_files(files: &[(String, Vec<u8>)]) -> Outcome {
    let mut all = Outcome::default();
    for (name, bytes) in files {
        match std::str::from_utf8(bytes) {
            Ok(text) => all.absorb(compare_script(name, text)),
            Err(_) => all.unreadable.push(name.clone()),
        }
    }
    all
}

/// The differential over every `*.sql` file of `dir`; `None` when the corpus was not fetched.
pub fn compare_dir(dir: &Path) -> Option<Outcome> {
    corpus_files(dir).map(|f| compare_files(&f))
}

/// A bucket's checked-in listing (`corpus/differential/<name>`).
pub fn listing_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus/differential")
        .join(name)
}

/// A bucket's listing text: the header's lines as `#` comments, then one key per entry.
pub fn listing(header: &str, entries: &[Entry]) -> String {
    let mut out = String::new();
    for line in header.lines() {
        out.push_str("# ");
        out.push_str(line);
        out.push('\n');
    }
    for e in entries {
        out.push_str(&e.key());
        out.push('\n');
    }
    out
}

/// Checks a bucket against its listing: no entry the listing lacks (a statement that moved into
/// the bucket fails, named), and a count within `tolerance` (a fraction: 0.01 is ±1 %) of the
/// listing's.
pub fn check_listing(found: &[Entry], listed: &str, tolerance: f64) -> Result<(), String> {
    let keys: BTreeSet<&str> = listed
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .collect();
    let new: Vec<String> = found
        .iter()
        .filter(|e| !keys.contains(e.key().as_str()))
        .map(|e| format!("{} {:?}: {:?}", e.file, e.range, e.head))
        .collect();
    let (n, want) = (found.len() as f64, keys.len() as f64);
    let mut errors = Vec::new();
    if !new.is_empty() {
        errors.push(format!(
            "{} statement(s) not in the listing:\n  {}",
            new.len(),
            new.join("\n  ")
        ));
    }
    if (n - want).abs() > want * tolerance {
        errors.push(format!(
            "{n} statements; the listing has {want} (tolerance {:.0} %)",
            tolerance * 100.0
        ));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
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
        assert_eq!(o.not_comparable.len(), 1, "{o:?}");
        assert_eq!(o.not_comparable[0].head, "SELEC 3");
        assert_eq!(o.agreed, 5, "{o:?}");
        assert!(o.disagreed.is_empty(), "{o:?}");
        assert_eq!(o.per_mille(), 1000.0);
        assert!((o.per_mille_of_all() - 5000.0 / 6.0).abs() < 1e-9);
    }

    #[test]
    fn a_cut_the_parser_does_not_make_disagrees() {
        // A text our splitter and the parser cut alike, then the rates' arithmetic.
        let o = compare_script("u.sql", "SELECT 1; SELECT 2;");
        assert_eq!((o.agreed, o.disagreed.len()), (2, 0));
        let sum = Outcome {
            statements: 1_010,
            agreed: 999,
            disagreed: vec![Entry::new("x", "y", TextRange::empty(TextSize::from(0)))],
            ..Outcome::default()
        };
        assert!((sum.per_mille() - 999.0).abs() < 1e-9);
        assert!((sum.per_mille_of_all() - 999_000.0 / 1_010.0).abs() < 1e-9);
    }

    #[test]
    fn a_listing_fails_on_a_new_entry_and_on_a_count_out_of_tolerance() {
        let text = "SELECT 1; SELECT 2; SELECT 3;";
        let e = |a: u32, b: u32| Entry::new("f.sql", text, TextRange::new(a.into(), b.into()));
        let two = vec![e(0, 8), e(10, 18)];
        let listed = listing("a header", &two);
        assert!(listed.starts_with("# a header\n"));
        assert!(check_listing(&two, &listed, 0.01).is_ok());
        // A statement that moved into the bucket fails, named.
        let more = vec![e(0, 8), e(10, 18), e(20, 28)];
        let err = check_listing(&more, &listed, 0.01).unwrap_err();
        assert!(
            err.contains("not in the listing") && err.contains("SELECT 3"),
            "{err}"
        );
        // Fewer than the listing, past the tolerance; within a wide one.
        assert!(check_listing(&two[..1], &listed, 0.01).is_err());
        assert!(check_listing(&two[..1], &listed, 0.6).is_ok());
        // The same range of other text is another entry.
        let other = Entry::new("f.sql", "SELECT 9; SELECT 2; SELECT 3;", e(0, 8).range);
        assert_ne!(other.key(), e(0, 8).key());
    }
}
