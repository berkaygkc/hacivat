//! `cargo xtask gen-sql-tables [--check | --from-pg-query | --from-docker]` (M3a Task 3,
//! SQL-LEX-01): the keyword tables of `crates/sql/sql-dialect`.
//!
//! `crates/sql/sql-dialect/data/keywords/*.toml` hold one row per keyword per source, each with
//! its `source` and the server versions it holds for (`since` inclusive, `until` exclusive,
//! `""` unbounded). The plain command turns them into `phf` maps under
//! `crates/sql/sql-dialect/src/generated/`; `--check` fails when those files are stale (CI).
//!
//! The data comes only from licence-clean sources:
//! - PostgreSQL: `kwlist.h` inside the vendored `third_party/pg_query` (PostgreSQL License,
//!   attributed in `THIRD_PARTY_NOTICES.md`); `--from-pg-query` rewrites `pg.toml` from it.
//! - MySQL and MariaDB: what the servers report in `INFORMATION_SCHEMA.KEYWORDS`, never MySQL's
//!   `sql/lex.h` (GPL). `--from-docker` rewrites `mysql80.toml`, `mysql84.toml` and
//!   `mariadb.toml` with one read-only query per server of the running `hacivat-m2a` matrix
//!   (controller-owned; this never starts or stops it). CI never runs it.
//! - SQLite: the public-domain list at <https://sqlite.org/lang_keywords.html> (`sqlite.toml`,
//!   written once by hand).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result, anyhow, bail, ensure};

const DATA_DIR: &str = "crates/sql/sql-dialect/data/keywords";
const GEN_DIR: &str = "crates/sql/sql-dialect/src/generated";
const IDS_OUT: &str = "keywords_ids.rs";
const KWLIST: &str = "third_party/pg_query/libpg_query/src/postgres/include/parser/kwlist.h";
const PG_CONFIG: &str = "third_party/pg_query/libpg_query/src/postgres/include/pg_config.h";
const PG_QUERY_MANIFEST: &str = "third_party/pg_query/Cargo.toml";
const TARGETS_FILE: &str = "docker/m2a/targets.toml";
/// The matrix's test user (`docker/m2a/targets.toml` header); the password stays inside the
/// container's environment (`MYSQL_PASSWORD`/`MARIADB_PASSWORD` of `compose.yml`).
const DOCKER_USER: &str = "hv_test";
const KEYWORDS_QUERY: &str =
    "SELECT VERSION() AS V, K.* FROM INFORMATION_SCHEMA.KEYWORDS K ORDER BY K.WORD";

/// One generated table: a dialect family, its data files (oldest snapshot first) and its output.
#[derive(Debug)]
pub struct Family {
    pub name: &'static str,
    pub files: &'static [&'static str],
    pub out: &'static str,
}

pub const FAMILIES: [Family; 4] = [
    Family {
        name: "pg",
        files: &["pg.toml"],
        out: "keywords_pg.rs",
    },
    Family {
        name: "mysql",
        files: &["mysql80.toml", "mysql84.toml"],
        out: "keywords_mysql.rs",
    },
    Family {
        name: "mariadb",
        files: &["mariadb.toml"],
        out: "keywords_mariadb.rs",
    },
    Family {
        name: "sqlite",
        files: &["sqlite.toml"],
        out: "keywords_sqlite.rs",
    },
];

/// A server line, `major.minor` (`"18"` is `18.0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Line(pub u16, pub u16);

impl Line {
    fn parse(s: &str) -> Result<Line> {
        let mut parts = s.split('.');
        let num = |p: Option<&str>| -> Result<Option<u16>> {
            p.map(|p| {
                p.parse::<u16>()
                    .with_context(|| format!("`{s}` is not major.minor"))
            })
            .transpose()
        };
        let major = num(parts.next())?.ok_or_else(|| anyhow!("empty version"))?;
        let minor = num(parts.next())?.unwrap_or(0);
        ensure!(parts.next().is_none(), "`{s}` is not major.minor");
        Ok(Line(major, minor))
    }

    /// `""` is unbounded.
    fn parse_gate(s: &str) -> Result<Option<Line>> {
        if s.is_empty() {
            Ok(None)
        } else {
            Line::parse(s).map(Some)
        }
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.0, self.1)
    }
}

fn gate_text(g: Option<Line>) -> String {
    g.map(|l| l.to_string()).unwrap_or_default()
}

/// `KwCategory` of `crates/sql/sql-dialect` (PostgreSQL's four; the others use two).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Unreserved,
    ColName,
    TypeFuncName,
    Reserved,
}

impl Category {
    const ALL: [(Category, &'static str, &'static str); 4] = [
        (Category::Unreserved, "unreserved", "Unreserved"),
        (Category::ColName, "col_name", "ColName"),
        (Category::TypeFuncName, "type_func_name", "TypeFuncName"),
        (Category::Reserved, "reserved", "Reserved"),
    ];

    fn parse(s: &str) -> Result<Category> {
        Category::ALL
            .iter()
            .find(|c| c.1 == s)
            .map(|c| c.0)
            .ok_or_else(|| anyhow!("unknown category `{s}`"))
    }

    fn toml(self) -> &'static str {
        Category::ALL
            .iter()
            .find(|c| c.0 == self)
            .map_or("", |c| c.1)
    }

    fn rust(self) -> &'static str {
        Category::ALL
            .iter()
            .find(|c| c.0 == self)
            .map_or("", |c| c.2)
    }
}

/// A row: one keyword as one source reports it, and the versions the row holds for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub word: String,
    pub category: Category,
    pub source: String,
    pub since: Option<Line>,
    pub until: Option<Line>,
}

/// Where rows come from: the server line the source stands for and a description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub line: Line,
    pub text: String,
}

/// One data file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub file: String,
    pub dialect: String,
    pub header: Vec<String>,
    pub sources: BTreeMap<String, Source>,
    pub rows: Vec<Row>,
}

/// A keyword is upper-case ASCII that is also a Rust constant name (`kw::SELECT`).
fn valid_word(w: &str) -> bool {
    let b = w.as_bytes();
    !b.is_empty()
        && w != "_"
        && (b[0].is_ascii_uppercase() || b[0] == b'_')
        && b.iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_')
}

fn str_field<'a>(t: &'a toml::Table, key: &str, what: &str) -> Result<&'a str> {
    t.get(key)
        .and_then(toml::Value::as_str)
        .ok_or_else(|| anyhow!("{what}: `{key}` is missing or not a string"))
}

/// Parses and validates one data file.
pub fn parse_table(file: &str, text: &str) -> Result<Table> {
    let text = normalize_eol(text);
    let doc: toml::Table =
        toml::from_str(&text).with_context(|| format!("{file} does not parse"))?;
    let dialect = str_field(&doc, "dialect", file)?.to_string();
    let header = text
        .lines()
        .take_while(|l| l.starts_with('#'))
        .map(|l| l.trim_start_matches('#').trim_start().to_string())
        .collect();
    let mut sources = BTreeMap::new();
    for (key, v) in doc
        .get("sources")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| anyhow!("{file}: no [sources]"))?
    {
        let t = v
            .as_table()
            .ok_or_else(|| anyhow!("{file}: source `{key}` is not a table"))?;
        let what = format!("{file}: source `{key}`");
        let source = Source {
            line: Line::parse(str_field(t, "line", &what)?).with_context(|| what.clone())?,
            text: str_field(t, "text", &what)?.to_string(),
        };
        ensure!(!source.text.is_empty(), "{what}: empty text");
        sources.insert(key.clone(), source);
    }
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for v in doc
        .get("keywords")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| anyhow!("{file}: no `keywords` array"))?
    {
        let t = v
            .as_table()
            .ok_or_else(|| anyhow!("{file}: a keyword row is not a table"))?;
        let word = str_field(t, "word", file)?.to_string();
        let what = format!("{file}: `{word}`");
        ensure!(valid_word(&word), "{what}: not an upper-case ASCII keyword");
        let row = Row {
            category: Category::parse(str_field(t, "category", &what)?)
                .with_context(|| what.clone())?,
            source: str_field(t, "source", &what)?.to_string(),
            since: Line::parse_gate(str_field(t, "since", &what)?).with_context(|| what.clone())?,
            until: Line::parse_gate(str_field(t, "until", &what)?).with_context(|| what.clone())?,
            word,
        };
        let source = sources
            .get(&row.source)
            .ok_or_else(|| anyhow!("{what}: source `{}` is not in [sources]", row.source))?;
        ensure!(
            row.since.is_none_or(|s| s <= source.line) && row.until.is_none_or(|u| source.line < u),
            "{what}: the gates [{}, {}) exclude its source's line {}",
            gate_text(row.since),
            gate_text(row.until),
            source.line
        );
        ensure!(
            seen.insert((row.word.clone(), row.source.clone())),
            "{what}: twice from `{}`",
            row.source
        );
        rows.push(row);
    }
    Ok(Table {
        file: file.to_string(),
        dialect,
        header,
        sources,
        rows,
    })
}

fn data_path(root: &Path, file: &str) -> PathBuf {
    root.join(DATA_DIR).join(file)
}

/// Every data file, in [`FAMILIES`] order.
pub fn all_tables(root: &Path) -> Result<Vec<Table>> {
    let mut out = Vec::new();
    for f in &FAMILIES {
        for file in f.files {
            let path = data_path(root, file);
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let table = parse_table(file, &text)?;
            ensure!(
                table.dialect == f.name,
                "{file}: dialect `{}`, expected `{}`",
                table.dialect,
                f.name
            );
            out.push(table);
        }
    }
    Ok(out)
}

/// One generated row: a category over a version range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gate {
    category: Category,
    since: Option<Line>,
    until: Option<Line>,
}

/// The rows of a family per word, oldest first, adjacent rows with the same category joined.
/// Rows of one word must not overlap.
fn merge(tables: &[&Table]) -> Result<BTreeMap<String, Vec<Gate>>> {
    let mut by_word: BTreeMap<String, Vec<Gate>> = BTreeMap::new();
    for t in tables {
        for r in &t.rows {
            by_word.entry(r.word.clone()).or_default().push(Gate {
                category: r.category,
                since: r.since,
                until: r.until,
            });
        }
    }
    for (word, gates) in &mut by_word {
        // `None` (unbounded below) sorts first.
        gates.sort_by_key(|g| g.since);
        let mut joined: Vec<Gate> = Vec::new();
        for g in gates.drain(..) {
            match joined.last_mut() {
                None => joined.push(g),
                Some(prev) => {
                    let (Some(end), Some(start)) = (prev.until, g.since) else {
                        bail!("{word}: rows overlap");
                    };
                    ensure!(end <= start, "{word}: rows overlap");
                    if end == start && prev.category == g.category {
                        prev.until = g.until;
                    } else {
                        joined.push(g);
                    }
                }
            }
        }
        *gates = joined;
    }
    Ok(by_word)
}

/// FNV-1a 64 of a family's data files (LF line ends), in file order.
fn data_hash(texts: &[String]) -> u64 {
    let mut h: u64 = 14_695_981_039_346_656_037;
    for t in texts {
        for b in normalize_eol(t).bytes().chain(std::iter::once(0)) {
            h ^= u64::from(b);
            h = h.wrapping_mul(1_099_511_628_211);
        }
    }
    h
}

/// `text` as `prefix` comment lines of at most 100 columns (rustfmt does not wrap comments).
fn comment(prefix: &str, text: &str) -> String {
    let mut out = String::new();
    let mut line = String::from(prefix);
    for word in text.split(' ') {
        if line.len() > prefix.len() && line.len() + 1 + word.len() > 100 {
            out.push_str(&line);
            out.push('\n');
            line = String::from(prefix);
        }
        line.push(' ');
        line.push_str(word);
    }
    out.push_str(&line);
    out.push('\n');
    out
}

fn generated_by(from: &str) -> String {
    comment(
        "//",
        &format!(
            "Generated by `cargo xtask gen-sql-tables` from {from} (Task 3). Do not edit: change the data, then run `cargo xtask gen-sql-tables`."
        ),
    )
}

fn gate_rust(g: Option<Line>) -> String {
    g.map_or_else(|| "None".to_string(), |l| format!("at({}, {})", l.0, l.1))
}

fn render_family(
    f: &Family,
    merged: &BTreeMap<String, Vec<Gate>>,
    snapshots: &[Line],
    hash: u64,
) -> Result<String> {
    let mut o = String::new();
    let files = f
        .files
        .iter()
        .map(|f| format!("`data/keywords/{f}`"))
        .collect::<Vec<_>>()
        .join(" and ");
    writeln!(o, "{}", generated_by(&files))?;
    let gated = merged
        .values()
        .flatten()
        .any(|g| g.since.is_some() || g.until.is_some());
    let at = if gated { ", at" } else { "" };
    writeln!(o, "use crate::keywords::{{Gated, KwCategory{at}, kw}};\n")?;
    writeln!(
        o,
        "/// FNV-1a 64 of the data files (LF line ends): `version.rs`'s tests tie it to the profile release.\n#[cfg(test)]\npub(crate) const DATA_HASH: u64 = {hash};\n"
    )?;
    let lines = snapshots
        .iter()
        .map(|l| format!("({}, {})", l.0, l.1))
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(
        o,
        "/// The first version (major, minor) of each snapshot line, oldest first: `version.rs`'s tests\n/// tie it to the profile lines.\n#[cfg(test)]\npub(crate) const SNAPSHOTS: &[(u16, u16)] = &[{lines}];\n"
    )?;
    let mut map = phf_codegen::Map::<&str>::new();
    for (word, gates) in merged {
        let rows = gates
            .iter()
            .map(|g| {
                format!(
                    "Gated::new(kw::{word}, KwCategory::{}, {}, {})",
                    g.category.rust(),
                    gate_rust(g.since),
                    gate_rust(g.until)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        map.entry(word.as_str(), format!("&[{rows}]"));
    }
    writeln!(
        o,
        "/// Each keyword (upper-case ASCII) with its rows, oldest first.\npub(crate) static KEYWORDS: phf::Map<&'static str, &'static [Gated]> = {};",
        map.build()
    )?;
    rustfmt(&o)
}

fn render_ids(words: &BTreeSet<String>) -> Result<String> {
    let mut o = String::new();
    writeln!(o, "{}", generated_by("every file in `data/keywords/`"))?;
    writeln!(
        o,
        "//! The shared keyword ids, one per word of any table, and their canonical text.\n"
    )?;
    let max = words.iter().map(String::len).max().unwrap_or(0);
    writeln!(
        o,
        "/// The longest keyword, in bytes.\npub(super) const MAX_LEN: usize = {max};\n"
    )?;
    writeln!(
        o,
        "/// The canonical text of each keyword, indexed by its id.\npub(super) static TEXT: [&str; {}] = [",
        words.len()
    )?;
    for w in words {
        writeln!(o, "\"{w}\",")?;
    }
    writeln!(o, "];\n")?;
    write!(
        o,
        "{}",
        comment(
            "///",
            "Every keyword of every table. Ids follow the words' byte order and change when a table gains a word: never persist them."
        )
    )?;
    writeln!(o, "pub mod kw {{\nuse crate::keywords::KeywordId;\n")?;
    for (i, w) in words.iter().enumerate() {
        writeln!(o, "pub const {w}: KeywordId = KeywordId({i});")?;
    }
    writeln!(o, "}}")?;
    rustfmt(&o)
}

/// Every generated file and its contents, from the data under `root`.
pub fn render_all(root: &Path) -> Result<Vec<(PathBuf, String)>> {
    let tables = all_tables(root)?;
    let mut words = BTreeSet::new();
    let mut out = Vec::new();
    for f in &FAMILIES {
        let mine: Vec<&Table> = tables.iter().filter(|t| t.dialect == f.name).collect();
        let texts = f
            .files
            .iter()
            .map(|file| std::fs::read_to_string(data_path(root, file)))
            .collect::<std::io::Result<Vec<_>>>()?;
        let merged = merge(&mine)?;
        let snapshots: BTreeSet<Line> = mine
            .iter()
            .flat_map(|t| t.sources.values().map(|s| s.line))
            .collect();
        words.extend(merged.keys().cloned());
        let snapshots: Vec<Line> = snapshots.into_iter().collect();
        out.push((
            root.join(GEN_DIR).join(f.out),
            render_family(f, &merged, &snapshots, data_hash(&texts))?,
        ));
    }
    ensure!(
        words.len() <= usize::from(u16::MAX),
        "{} keywords do not fit a u16 id",
        words.len()
    );
    out.push((root.join(GEN_DIR).join(IDS_OUT), render_ids(&words)?));
    Ok(out)
}

/// `Ok` when every generated file matches the data; otherwise the stale files.
pub fn check_fresh(root: &Path) -> Result<()> {
    let mut stale = Vec::new();
    for (path, want) in render_all(root)? {
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        if normalize_eol(&have) != normalize_eol(&want) {
            stale.push(path.display().to_string());
        }
    }
    ensure!(
        stale.is_empty(),
        "stale (run `cargo xtask gen-sql-tables`): {}",
        stale.join(", ")
    );
    Ok(())
}

fn write_generated(root: &Path) -> Result<()> {
    for (path, text) in render_all(root)? {
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        if normalize_eol(&have) != normalize_eol(&text) {
            std::fs::write(&path, text)?;
            println!("wrote {}", path.display());
        }
    }
    Ok(())
}

/// A data file in its canonical form.
pub fn render_toml(t: &Table) -> String {
    let q = |s: &str| toml::Value::String(s.to_string()).to_string();
    let mut o = String::new();
    for h in &t.header {
        if h.is_empty() {
            o.push_str("#\n");
        } else {
            o.push_str(&format!("# {h}\n"));
        }
    }
    o.push_str(&format!("\ndialect = {}\n\nkeywords = [\n", q(&t.dialect)));
    for r in &t.rows {
        o.push_str(&format!(
            "  {{ word = {}, category = {}, source = {}, since = {}, until = {} }},\n",
            q(&r.word),
            q(r.category.toml()),
            q(&r.source),
            q(&gate_text(r.since)),
            q(&gate_text(r.until)),
        ));
    }
    o.push_str("]\n");
    for (key, s) in &t.sources {
        o.push_str(&format!(
            "\n[sources.{}]\nline = {}\ntext = {}\n",
            q(key),
            q(&s.line.to_string()),
            q(&s.text)
        ));
    }
    o
}

/// Validates `t` by parsing its rendering, then writes it.
fn write_table(root: &Path, t: &Table) -> Result<()> {
    let text = render_toml(t);
    let back = parse_table(&t.file, &text)?;
    ensure!(back.rows == t.rows, "{}: rows do not round-trip", t.file);
    let path = data_path(root, &t.file);
    std::fs::write(&path, text)?;
    println!("wrote {} ({} rows)", path.display(), t.rows.len());
    Ok(())
}

fn header(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|s| (*s).to_string()).collect()
}

/// The `PG_KEYWORD("abort", ABORT_P, UNRESERVED_KEYWORD, BARE_LABEL)` rows of `kwlist.h`.
pub fn parse_kwlist(text: &str) -> Result<Vec<(String, Category)>> {
    let mut out = Vec::new();
    for line in text.lines().filter(|l| l.starts_with("PG_KEYWORD(")) {
        // A row may end in a comment: `PG_KEYWORD("analyse", …)  /* British spelling */`.
        let code = line.split("/*").next().unwrap_or(line).trim_end();
        let args = code
            .strip_prefix("PG_KEYWORD(")
            .and_then(|l| l.strip_suffix(')'))
            .ok_or_else(|| anyhow!("kwlist.h: `{line}`"))?;
        let parts: Vec<&str> = args.split(',').map(str::trim).collect();
        let [name, _token, category, _label] = parts[..] else {
            bail!("kwlist.h: `{line}` has not four arguments");
        };
        let word = name
            .strip_prefix('"')
            .and_then(|n| n.strip_suffix('"'))
            .ok_or_else(|| anyhow!("kwlist.h: `{line}`"))?
            .to_ascii_uppercase();
        let category = match category {
            "UNRESERVED_KEYWORD" => Category::Unreserved,
            "COL_NAME_KEYWORD" => Category::ColName,
            "TYPE_FUNC_NAME_KEYWORD" => Category::TypeFuncName,
            "RESERVED_KEYWORD" => Category::Reserved,
            other => bail!("kwlist.h: unknown category `{other}`"),
        };
        out.push((word, category));
    }
    ensure!(!out.is_empty(), "kwlist.h has no PG_KEYWORD rows");
    Ok(out)
}

fn c_define<'a>(text: &'a str, name: &str) -> Result<&'a str> {
    let prefix = format!("#define {name} ");
    text.lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .map(|v| v.trim().trim_matches('"'))
        .ok_or_else(|| anyhow!("pg_config.h has no {name}"))
}

fn from_pg_query(root: &Path) -> Result<()> {
    let kwlist = std::fs::read_to_string(root.join(KWLIST)).context("reading kwlist.h")?;
    let config = std::fs::read_to_string(root.join(PG_CONFIG)).context("reading pg_config.h")?;
    let manifest =
        std::fs::read_to_string(root.join(PG_QUERY_MANIFEST)).context("reading pg_query")?;
    let manifest: toml::Table = toml::from_str(&manifest)?;
    let crate_version = manifest
        .get("package")
        .and_then(|p| p.get("version"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| anyhow!("pg_query manifest has no version"))?;
    let (version, major) = (
        c_define(&config, "PG_VERSION")?,
        c_define(&config, "PG_MAJORVERSION")?,
    );
    let key = format!("pg-{version}-kwlist");
    let rows = parse_kwlist(&kwlist)?
        .into_iter()
        .map(|(word, category)| Row {
            word,
            category,
            source: key.clone(),
            since: None,
            until: None,
        })
        .collect();
    let table = Table {
        file: "pg.toml".into(),
        dialect: "pg".into(),
        header: header(&[
            "PostgreSQL keywords (SQL-LEX-01). Written by `cargo xtask gen-sql-tables --from-pg-query`",
            "from the vendored libpg_query's `kwlist.h`; `cargo xtask gen-sql-tables` turns it into",
            "`src/generated/keywords_pg.rs`. Never edit by hand.",
            "",
            "Row: `word` (ASCII upper case), `category` (kwlist.h's), `source` (a key of [sources]),",
            "`since`/`until`: the server versions the row holds for (inclusive/exclusive, \"\" =",
            "unbounded). One snapshot only, so no row is gated: older servers lack a few of these",
            "words, which then lex as keywords there (harmless for highlighting and splitting).",
        ]),
        sources: BTreeMap::from([(
            key,
            Source {
                line: Line::parse(major)?,
                text: format!(
                    "{KWLIST} of the vendored pg_query {crate_version} (PostgreSQL {version}); PostgreSQL License, attributed in THIRD_PARTY_NOTICES.md"
                ),
            },
        )]),
        rows,
    };
    write_table(root, &table)
}

/// A `[[target]]` of `docker/m2a/targets.toml` that reports keywords.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordTarget {
    pub engine: String,
    pub service: String,
    pub line: Line,
}

/// The MySQL and MariaDB targets, one per server line (the first target of a line wins:
/// `mysql84-x509` repeats `mysql84`), oldest first.
pub fn keyword_targets(targets_toml: &str) -> Result<Vec<KeywordTarget>> {
    let doc: toml::Table = toml::from_str(targets_toml).context("targets.toml does not parse")?;
    let mut out: Vec<KeywordTarget> = Vec::new();
    for t in doc
        .get("target")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| anyhow!("targets.toml has no [[target]]"))?
    {
        let t = t
            .as_table()
            .ok_or_else(|| anyhow!("a [[target]] is not a table"))?;
        let engine = str_field(t, "engine", "target")?;
        if engine != "mysql" && engine != "mariadb" {
            continue;
        }
        let target = KeywordTarget {
            engine: engine.to_string(),
            service: str_field(t, "service", "target")?.to_string(),
            line: Line::parse(str_field(t, "major", "target")?)?,
        };
        if !out
            .iter()
            .any(|o| o.engine == target.engine && o.line == target.line)
        {
            out.push(target);
        }
    }
    out.sort_by(|a, b| (&a.engine, a.line).cmp(&(&b.engine, b.line)));
    Ok(out)
}

/// What one server reported: its version and its keywords.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub version: String,
    /// `None` when the server has no `RESERVED` column (MariaDB).
    pub words: Vec<(String, Option<bool>)>,
    /// Entries that are no keyword name (operators), left out.
    pub skipped: Vec<String>,
}

/// Parses `mysql -B` output of [`KEYWORDS_QUERY`]: a header row, then tab-separated rows.
pub fn parse_keywords_output(out: &str) -> Result<Snapshot> {
    let mut lines = out.lines();
    let head: Vec<&str> = lines
        .next()
        .ok_or_else(|| anyhow!("no header row"))?
        .split('\t')
        .collect();
    let col = |name: &str| head.iter().position(|h| h.eq_ignore_ascii_case(name));
    let (v, w) = (
        col("V").ok_or_else(|| anyhow!("no V column"))?,
        col("WORD").ok_or_else(|| anyhow!("no WORD column"))?,
    );
    let reserved = col("RESERVED");
    let mut version = None;
    let (mut words, mut skipped) = (Vec::new(), Vec::new());
    for line in lines.filter(|l| !l.is_empty()) {
        let cells: Vec<&str> = line.split('\t').collect();
        let cell = |i: usize| {
            cells
                .get(i)
                .copied()
                .ok_or_else(|| anyhow!("short row `{line}`"))
        };
        let row_version = cell(v)?;
        ensure!(
            *version.get_or_insert(row_version) == row_version,
            "the version changed mid-query"
        );
        let word = cell(w)?.to_ascii_uppercase();
        if !valid_word(&word) {
            skipped.push(word);
            continue;
        }
        let flag = match reserved {
            None => None,
            Some(r) => Some(match cell(r)? {
                "1" => true,
                "0" => false,
                other => bail!("RESERVED is `{other}` for {word}"),
            }),
        };
        words.push((word, flag));
    }
    ensure!(!words.is_empty(), "the server reported no keywords");
    Ok(Snapshot {
        version: version.unwrap_or_default().to_string(),
        words,
        skipped,
    })
}

/// The leading `digits.digits.digits` of a server version string (`11.4.8-MariaDB-ubu2404`).
fn numeric_version(v: &str) -> &str {
    let end = v
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(v.len());
    v[..end].trim_end_matches('.')
}

fn query_keywords(t: &KeywordTarget) -> Result<Snapshot> {
    let (client, password_var) = if t.engine == "mariadb" {
        ("mariadb", "MARIADB_PASSWORD")
    } else {
        ("mysql", "MYSQL_PASSWORD")
    };
    // One read-only query; the password never leaves the container.
    let script = format!(
        "MYSQL_PWD=\"${password_var}\" exec {client} -h127.0.0.1 -P3306 -u{DOCKER_USER} -B -e \"{KEYWORDS_QUERY}\""
    );
    let container = crate::docker::container_name(&t.service);
    let out = Command::new("docker")
        .args(["exec", &container, "sh", "-c", &script])
        .output()
        .context("running docker")?;
    ensure!(
        out.status.success(),
        "{container}: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    let snap = parse_keywords_output(&String::from_utf8(out.stdout)?)
        .with_context(|| container.clone())?;
    let reported = Line::parse(
        &numeric_version(&snap.version)
            .split('.')
            .take(2)
            .collect::<Vec<_>>()
            .join("."),
    )?;
    ensure!(
        reported == t.line,
        "{container} reports {}, targets.toml says {}",
        snap.version,
        t.line
    );
    Ok(snap)
}

/// The rows of each snapshot of one engine, oldest first: a snapshot stands for every version
/// from its line up to the next snapshot's line (the newest for every later version).
pub fn snapshot_rows(
    engine: &str,
    snaps: &[(KeywordTarget, Snapshot)],
) -> Result<Vec<(String, Source, Vec<Row>)>> {
    let mut out = Vec::new();
    for (i, (t, s)) in snaps.iter().enumerate() {
        let since = (i > 0).then_some(t.line);
        let until = snaps.get(i + 1).map(|n| n.0.line);
        let version = numeric_version(&s.version);
        let key = format!("{engine}-{version}");
        let classified = s.words.iter().all(|w| w.1.is_some());
        ensure!(
            classified || s.words.iter().all(|w| w.1.is_none()),
            "{key}: RESERVED is missing on some rows"
        );
        let name = if engine == "mariadb" {
            "MariaDB"
        } else {
            "MySQL"
        };
        let mut text = format!(
            "INFORMATION_SCHEMA.KEYWORDS of the {name} {} server (compose service {}), read by `cargo xtask gen-sql-tables --from-docker`",
            s.version, t.service
        );
        if !classified {
            text.push_str(
                "; the server reports no RESERVED column, so every row is recorded reserved (an identifier spelled like one is quoted)",
            );
        }
        if !s.skipped.is_empty() {
            text.push_str(&format!("; {} non-word entries left out", s.skipped.len()));
        }
        let rows = s
            .words
            .iter()
            .map(|(word, reserved)| Row {
                word: word.clone(),
                category: if reserved.unwrap_or(true) {
                    Category::Reserved
                } else {
                    Category::Unreserved
                },
                source: key.clone(),
                since,
                until,
            })
            .collect();
        out.push((key, Source { line: t.line, text }, rows));
    }
    Ok(out)
}

const SERVER_HEADER: [&str; 7] = [
    "Row: `word` (ASCII upper case), `category`, `source` (a key of [sources]), `since`/`until`:",
    "the server versions the row holds for (inclusive/exclusive, \"\" = unbounded). Each source",
    "is one server's snapshot and stands for every version from its line up to the next",
    "snapshot's line; the newest also stands for later versions.",
    "",
    "Never copied from MySQL's `sql/lex.h` (GPL): the rows are what the servers report.",
    "Never edit by hand.",
];

fn from_docker(root: &Path) -> Result<()> {
    let targets = keyword_targets(&std::fs::read_to_string(root.join(TARGETS_FILE))?)?;
    let mut by_engine: BTreeMap<String, Vec<(KeywordTarget, Snapshot)>> = BTreeMap::new();
    for t in targets {
        let snap = query_keywords(&t)?;
        println!(
            "{}: {} keywords ({} skipped)",
            t.service,
            snap.words.len(),
            snap.skipped.len()
        );
        by_engine
            .entry(t.engine.clone())
            .or_default()
            .push((t, snap));
    }
    for (engine, snaps) in &by_engine {
        let snapshots = snapshot_rows(engine, snaps)?;
        let what = format!(
            "{} keywords (SQL-LEX-01). Written by `cargo xtask gen-sql-tables --from-docker`; `cargo",
            if engine == "mariadb" {
                "MariaDB"
            } else {
                "MySQL"
            }
        );
        let mut lines = vec![
            what.as_str(),
            "xtask gen-sql-tables` turns it into the generated table under `src/generated/`.",
            "",
        ];
        lines.extend(SERVER_HEADER);
        if engine == "mariadb" {
            let mut table = Table {
                file: "mariadb.toml".into(),
                dialect: "mariadb".into(),
                header: header(&lines),
                sources: BTreeMap::new(),
                rows: Vec::new(),
            };
            for (key, source, rows) in snapshots {
                table.sources.insert(key, source);
                table.rows.extend(rows);
            }
            write_table(root, &table)?;
        } else {
            for (key, source, rows) in snapshots {
                let file = format!("mysql{}{}.toml", source.line.0, source.line.1);
                ensure!(
                    FAMILIES[1].files.contains(&file.as_str()),
                    "{file} is not a data file of the mysql family: add it to FAMILIES"
                );
                let table = Table {
                    file,
                    dialect: "mysql".into(),
                    header: header(&lines),
                    sources: BTreeMap::from([(key, source)]),
                    rows,
                };
                write_table(root, &table)?;
            }
        }
    }
    Ok(())
}

fn rustfmt(src: &str) -> Result<String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning rustfmt")?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("no rustfmt stdin"))?
        .write_all(src.as_bytes())?;
    let out = child.wait_with_output()?;
    ensure!(
        out.status.success(),
        "rustfmt failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?)
}

/// CRLF -> LF, so a Windows checkout compares and hashes like a Unix one.
fn normalize_eol(s: &str) -> std::borrow::Cow<'_, str> {
    if s.contains("\r\n") {
        std::borrow::Cow::Owned(s.replace("\r\n", "\n"))
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

fn run_inner(args: &[String]) -> Result<bool> {
    let root = crate::workspace_root();
    match args.first().map(String::as_str) {
        None => write_generated(&root)?,
        Some("--check") => {
            if let Err(e) = check_fresh(&root) {
                eprintln!("gen-sql-tables: {e:#}");
                return Ok(false);
            }
            println!("gen-sql-tables: the generated keyword tables are fresh");
        }
        Some("--from-pg-query") => {
            from_pg_query(&root)?;
            write_generated(&root)?;
        }
        Some("--from-docker") => {
            from_docker(&root)?;
            write_generated(&root)?;
        }
        Some(other) => bail!(
            "unknown argument `{other}`\nusage: cargo xtask gen-sql-tables [--check | --from-pg-query | --from-docker]"
        ),
    }
    Ok(true)
}

pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    match run_inner(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("gen-sql-tables: {e:#}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_root;

    const SAMPLE: &str = r#"# A sample.
dialect = "mysql"
keywords = [
  { word = "SELECT", category = "reserved", source = "a", since = "", until = "8.4" },
  { word = "SELECT", category = "reserved", source = "b", since = "8.4", until = "" },
  { word = "STATUS", category = "unreserved", source = "a", since = "", until = "8.4" },
  { word = "STATUS", category = "reserved", source = "b", since = "8.4", until = "" },
  { word = "OLD", category = "unreserved", source = "a", since = "", until = "8.4" },
]

[sources.a]
line = "8.0"
text = "server a"

[sources.b]
line = "8.4"
text = "server b"
"#;

    fn sample() -> Table {
        parse_table("sample.toml", SAMPLE).unwrap()
    }

    #[test]
    fn generated_tables_are_fresh() {
        let fresh = check_fresh(&workspace_root());
        assert!(fresh.is_ok(), "{fresh:?}");
    }

    #[test]
    fn every_row_names_its_source() {
        let tables = all_tables(&workspace_root()).unwrap();
        assert_eq!(tables.len(), 5);
        for t in &tables {
            assert!(!t.rows.is_empty(), "{}", t.file);
            for r in &t.rows {
                assert!(!r.source.is_empty(), "{}: {}", t.file, r.word);
                assert!(t.sources.contains_key(&r.source), "{}: {}", t.file, r.word);
            }
            for s in t.sources.values() {
                assert!(!s.text.is_empty(), "{}", t.file);
            }
        }
    }

    #[test]
    fn no_source_is_gpl_lex_h() {
        // MySQL's and MariaDB's `sql/lex.h` are GPL: rows come from servers, kwlist.h and the
        // SQLite page only.
        for t in all_tables(&workspace_root()).unwrap() {
            for s in t.sources.values() {
                assert!(!s.text.contains("lex.h"), "{}: {}", t.file, s.text);
                let clean = s.text.contains("INFORMATION_SCHEMA.KEYWORDS")
                    || s.text.contains("kwlist.h")
                    || s.text.contains("sqlite.org/lang_keywords.html");
                assert!(clean, "{}: {}", t.file, s.text);
            }
        }
    }

    #[test]
    fn data_files_are_canonical() {
        // Every data file is what `render_toml` writes, so importer output and hand edits stay
        // in one format.
        for t in all_tables(&workspace_root()).unwrap() {
            let path = data_path(&workspace_root(), &t.file);
            let text = std::fs::read_to_string(path).unwrap();
            assert_eq!(normalize_eol(&text), render_toml(&t), "{}", t.file);
        }
    }

    #[test]
    fn rows_join_across_snapshots() {
        let t = sample();
        let merged = merge(&[&t]).unwrap();
        let all = Gate {
            category: Category::Reserved,
            since: None,
            until: None,
        };
        assert_eq!(merged["SELECT"], [all]);
        assert_eq!(
            merged["STATUS"],
            [
                Gate {
                    category: Category::Unreserved,
                    since: None,
                    until: Some(Line(8, 4)),
                },
                Gate {
                    category: Category::Reserved,
                    since: Some(Line(8, 4)),
                    until: None,
                },
            ]
        );
        assert_eq!(merged["OLD"][0].until, Some(Line(8, 4)));
    }

    #[test]
    fn duplicate_and_overlapping_rows_are_refused() {
        let text = SAMPLE.replace(
            r#"source = "b", since = "8.4", until = "" },
  { word = "STATUS""#,
            r#"source = "b", since = "8.4", until = "" },
  { word = "SELECT", category = "reserved", source = "a", since = "8.0", until = "" },
  { word = "STATUS""#,
        );
        assert_ne!(text, SAMPLE);
        // `SELECT` twice from source `a`.
        assert!(parse_table("x.toml", &text).is_err());
        let mut t = sample();
        t.rows.push(Row {
            word: "OLD".into(),
            category: Category::Unreserved,
            source: "b".into(),
            since: None,
            until: None,
        });
        assert!(merge(&[&t]).is_err());
    }

    #[test]
    fn bad_rows_are_refused() {
        for (from, to) in [
            (r#"word = "OLD""#, r#"word = "old""#),
            (r#"word = "OLD""#, r#"word = "LİMİT""#),
            (r#"word = "OLD""#, r#"word = "&&""#),
            (
                r#"category = "unreserved", source = "a", since = "", until = "8.4" },
]"#,
                r#"category = "keyword", source = "a", since = "", until = "8.4" },
]"#,
            ),
            (
                r#"word = "OLD", category = "unreserved", source = "a""#,
                r#"word = "OLD", category = "unreserved", source = "z""#,
            ),
            (
                r#"word = "OLD", category = "unreserved", source = "a", since = """#,
                r#"word = "OLD", category = "unreserved", source = "a", since = "x.y""#,
            ),
            // Gates that exclude the row's own source line (8.0).
            (
                r#"word = "OLD", category = "unreserved", source = "a", since = """#,
                r#"word = "OLD", category = "unreserved", source = "a", since = "8.1""#,
            ),
            (
                r#"source = "a", since = "", until = "8.4" },
]"#,
                r#"source = "a", since = "", until = "8.0" },
]"#,
            ),
        ] {
            let text = SAMPLE.replace(from, to);
            assert_ne!(text, SAMPLE, "{to}");
            assert!(parse_table("x.toml", &text).is_err(), "{to}");
        }
    }

    #[test]
    fn data_round_trips_through_render_toml() {
        let t = sample();
        assert_eq!(parse_table("sample.toml", &render_toml(&t)).unwrap(), t);
    }

    #[test]
    fn kwlist_rows_parse() {
        let text = "/* x */\nPG_KEYWORD(\"abort\", ABORT_P, UNRESERVED_KEYWORD, BARE_LABEL)\nPG_KEYWORD(\"all\", ALL, RESERVED_KEYWORD, BARE_LABEL)\t\t/* a comment */\nPG_KEYWORD(\"between\", BETWEEN, COL_NAME_KEYWORD, BARE_LABEL)\nPG_KEYWORD(\"binary\", BINARY, TYPE_FUNC_NAME_KEYWORD, BARE_LABEL)\n";
        assert_eq!(
            parse_kwlist(text).unwrap(),
            [
                ("ABORT".to_string(), Category::Unreserved),
                ("ALL".to_string(), Category::Reserved),
                ("BETWEEN".to_string(), Category::ColName),
                ("BINARY".to_string(), Category::TypeFuncName),
            ]
        );
        assert!(parse_kwlist("PG_KEYWORD(\"x\", X, ODD_KEYWORD, BARE_LABEL)").is_err());
        assert!(parse_kwlist("nothing").is_err());
    }

    #[test]
    fn vendored_kwlist_matches_pg_toml() {
        let root = workspace_root();
        let kwlist = parse_kwlist(&std::fs::read_to_string(root.join(KWLIST)).unwrap()).unwrap();
        let pg = all_tables(&root).unwrap().remove(0);
        let rows: Vec<(String, Category)> = pg
            .rows
            .iter()
            .map(|r| (r.word.clone(), r.category))
            .collect();
        assert_eq!(rows, kwlist);
    }

    #[test]
    fn server_output_parses_with_and_without_reserved() {
        let mysql = "V\tWORD\tRESERVED\n8.0.44\tACCESSIBLE\t1\n8.0.44\taction\t0\n";
        let s = parse_keywords_output(mysql).unwrap();
        assert_eq!(s.version, "8.0.44");
        assert_eq!(
            s.words,
            [
                ("ACCESSIBLE".to_string(), Some(true)),
                ("ACTION".to_string(), Some(false))
            ]
        );
        let mariadb = "V\tWORD\n11.4.8-MariaDB-ubu2404\t&&\n11.4.8-MariaDB-ubu2404\tACCESSIBLE\n";
        let s = parse_keywords_output(mariadb).unwrap();
        assert_eq!(s.words, [("ACCESSIBLE".to_string(), None)]);
        assert_eq!(s.skipped, ["&&"]);
        assert_eq!(numeric_version(&s.version), "11.4.8");
        assert!(parse_keywords_output("V\tWORD\tRESERVED\n8.0.1\tX\t2\n").is_err());
        assert!(parse_keywords_output("V\tWORD\n8.0.1\tX\n8.0.2\tY\n").is_err());
    }

    #[test]
    fn snapshots_stand_for_their_lines() {
        let target = |line| KeywordTarget {
            engine: "mysql".into(),
            service: "s".into(),
            line,
        };
        let snap = |v: &str, w: &[(&str, bool)]| Snapshot {
            version: v.into(),
            words: w
                .iter()
                .map(|(a, b)| ((*a).to_string(), Some(*b)))
                .collect(),
            skipped: Vec::new(),
        };
        let rows = snapshot_rows(
            "mysql",
            &[
                (target(Line(8, 0)), snap("8.0.44", &[("A", true)])),
                (target(Line(8, 4)), snap("8.4.7", &[("A", false)])),
            ],
        )
        .unwrap();
        assert_eq!(rows[0].0, "mysql-8.0.44");
        assert_eq!(
            (rows[0].2[0].since, rows[0].2[0].until),
            (None, Some(Line(8, 4)))
        );
        assert_eq!(
            (rows[1].2[0].since, rows[1].2[0].until),
            (Some(Line(8, 4)), None)
        );
        assert_eq!(rows[1].2[0].category, Category::Unreserved);
    }

    #[test]
    fn keyword_targets_take_one_server_per_line() {
        let toml = r#"
[[target]]
id = "pg17"
service = "pg17"
engine = "postgres"
major = "17"
[[target]]
id = "mysql84"
service = "mysql84"
engine = "mysql"
major = "8.4"
[[target]]
id = "mysql80"
service = "mysql80"
engine = "mysql"
major = "8.0"
[[target]]
id = "mysql84-x509"
service = "mysql84-x509"
engine = "mysql"
major = "8.4"
[[target]]
id = "mariadb106"
service = "mariadb106"
engine = "mariadb"
major = "10.6"
"#;
        let services: Vec<String> = keyword_targets(toml)
            .unwrap()
            .into_iter()
            .map(|t| t.service)
            .collect();
        assert_eq!(services, ["mariadb106", "mysql80", "mysql84"]);
    }

    #[test]
    fn repo_targets_cover_the_data_files() {
        let root = workspace_root();
        let targets =
            keyword_targets(&std::fs::read_to_string(root.join(TARGETS_FILE)).unwrap()).unwrap();
        let mysql: Vec<String> = targets
            .iter()
            .filter(|t| t.engine == "mysql")
            .map(|t| format!("mysql{}{}.toml", t.line.0, t.line.1))
            .collect();
        assert_eq!(mysql, FAMILIES[1].files);
        assert!(targets.iter().any(|t| t.engine == "mariadb"));
    }

    #[test]
    fn lines_parse_and_order() {
        assert_eq!(Line::parse("18").unwrap(), Line(18, 0));
        assert_eq!(Line::parse("10.11").unwrap(), Line(10, 11));
        assert!(Line(10, 11) > Line(10, 6));
        assert!(Line::parse("8.4.1").is_err());
        assert!(Line::parse("").is_err());
        assert_eq!(Line::parse_gate("").unwrap(), None);
    }

    #[test]
    fn data_hash_ignores_line_endings() {
        let lf = vec!["a\nb\n".to_string()];
        let crlf = vec!["a\r\nb\r\n".to_string()];
        assert_eq!(data_hash(&lf), data_hash(&crlf));
        assert_ne!(data_hash(&lf), data_hash(&["a\nc\n".to_string()]));
    }
}
