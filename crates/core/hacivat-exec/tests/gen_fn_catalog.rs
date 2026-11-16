//! M3b Task 3: the SAFE-03 catalog read (P-M3b-13). It proposes the candidate rows of
//! `crates/sql/sql-safety/data/<engine>/{allow,operators,types}.toml`; it never edits them.
//!
//! - PostgreSQL 14–18: `pg_proc`, `pg_operator` and `pg_type` of `pg_catalog` with OID < 16384
//!   (the builtins), read-only queries on the controller's Docker stack through
//!   `hacivat_testkit::targets`. A function name is a candidate when every overload on a line
//!   is a function, aggregate or window function, `IMMUTABLE` or `STABLE`, in language
//!   `internal` or `sql`, not `SECURITY DEFINER`, not on `sql-safety`'s deny list and not one
//!   of the names whose text argument is SQL or a server path; an operator when no
//!   implementing function is volatile or denied; a type when it is a base, domain, enum,
//!   range or multirange type outside the array category whose input function is not
//!   volatile. Lines where a name is missing set `since`/`until`.
//! - MySQL 8.0/8.4 and MariaDB 10.6/10.11/11.4: the names of the hand-curated
//!   `data/mysql/manual-index.txt` (never `mysql.help_*`), each prepared as `SELECT <name>()`
//!   (prepare only, nothing runs); a name is present on a line unless the server answers
//!   1305/1630 (no such function).
//! - SQLite: `pragma_function_list` of the bundled SQLite (`rusqlite`, safe API).
//!
//! Every candidate is written `reviewed = false` to the Docker artifacts
//! (`t03-<engine>-<file>.toml`, plus `t03-<engine>-excluded.tsv` with the reason for each name
//! left out); the implementer reviews each row and flips it in the data by hand.
//!
//! Run: `HACIVAT_GEN_FN_CATALOG=1 <main>/target/docker/run-docker-tests.sh --set full -p
//! hacivat-exec --test gen_fn_catalog`. Without `HACIVAT_GEN_FN_CATALOG=1` it does nothing,
//! so the crate's other Docker runs never write the artifacts.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

use common::matrix::{Line, Side, docker_enabled, must_exist, server_lines};
use hacivat_proto::DbmsKind;
use sql_dialect::DialectId;

fn wanted() -> bool {
    docker_enabled() && std::env::var("HACIVAT_GEN_FN_CATALOG").is_ok_and(|v| v == "1")
}

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sql/sql-safety/data")
}

// -----------------------------------------------------------------------------------------------
// The candidate rules (pure, tested without Docker)

/// Names whose `text` argument is SQL or a server path, left out whatever their volatility.
const SQL_OR_PATH_TEXT: [&str; 6] = [
    "query_to_",
    "cursor_to_",
    "database_to_",
    "schema_to_",
    "pg_read_",
    "pg_ls_",
];

/// One `pg_proc` row: `proname`, `prokind`, `provolatile`, `lanname`, `prosecdef`, the
/// argument types, tab-separated.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcRow {
    name: String,
    kind: char,
    volatile: char,
    language: String,
    secdef: bool,
}

fn parse_proc(line: &str) -> Option<ProcRow> {
    let mut f = line.splitn(6, '\t');
    Some(ProcRow {
        name: f.next()?.to_string(),
        kind: f.next()?.chars().next()?,
        volatile: f.next()?.chars().next()?,
        language: f.next()?.to_string(),
        secdef: f.next()? == "true",
    })
}

/// Why `row` keeps its name out of the candidates, if it does.
fn proc_exclusion(row: &ProcRow, denied: &BTreeSet<String>) -> Option<String> {
    if denied.contains(&row.name) {
        return Some("denied".into());
    }
    if SQL_OR_PATH_TEXT.iter().any(|p| row.name.starts_with(p)) || row.name == "pg_stat_file" {
        return Some("text argument is SQL or a path".into());
    }
    if !matches!(row.kind, 'f' | 'a' | 'w') {
        return Some(format!("prokind {}", row.kind));
    }
    if !matches!(row.volatile, 'i' | 's') {
        return Some(format!("provolatile {}", row.volatile));
    }
    if !matches!(row.language.as_str(), "internal" | "sql") {
        return Some(format!("language {}", row.language));
    }
    if row.secdef {
        return Some("security definer".into());
    }
    None
}

/// Per line (in line order), per name: `Ok(())` when a candidate there, `Err(reason)` when
/// present but left out; a name missing from a line has no entry for it.
type PerLine = BTreeMap<String, Vec<(usize, Result<(), String>)>>;

/// One merged candidate: name, `since`, `until`, the index of the line it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    name: String,
    since: Option<String>,
    until: Option<String>,
    source_line: usize,
}

/// Merges the lines (majors oldest first): a candidate on every line where it is present, on
/// a contiguous run of lines; otherwise excluded with the reason.
fn merge(
    per: &PerLine,
    majors: &[String],
    reference: usize,
) -> (Vec<Candidate>, Vec<(String, String)>) {
    let mut out = Vec::new();
    let mut excluded = Vec::new();
    for (name, seen) in per {
        if let Some((i, Err(why))) = seen.iter().find(|(_, s)| s.is_err()) {
            excluded.push((name.clone(), format!("{} on {}", why, majors[*i])));
            continue;
        }
        let lines: Vec<usize> = seen.iter().map(|(i, _)| *i).collect();
        let (first, last) = (lines[0], lines[lines.len() - 1]);
        if last - first + 1 != lines.len() {
            excluded.push((name.clone(), "not on a contiguous run of lines".into()));
            continue;
        }
        out.push(Candidate {
            name: name.clone(),
            since: (first > 0).then(|| majors[first].clone()),
            until: (last + 1 < majors.len()).then(|| majors[last + 1].clone()),
            source_line: if lines.contains(&reference) {
                reference
            } else {
                last
            },
        });
    }
    (out, excluded)
}

fn toml_rows(header: &str, rows: &[(Candidate, String)]) -> String {
    let mut s = String::from(header);
    s.push_str("rows = [\n");
    for (c, source) in rows {
        let _ = write!(
            s,
            "  {{ name = \"{}\"",
            c.name.replace('\\', "\\\\").replace('"', "\\\"")
        );
        if let Some(v) = &c.since {
            let _ = write!(s, ", since = \"{v}\"");
        }
        if let Some(v) = &c.until {
            let _ = write!(s, ", until = \"{v}\"");
        }
        let _ = writeln!(s, ", source = \"{source}\", reviewed = false }},");
    }
    s.push_str("]\n");
    s
}

fn write_artifact(name: &str, text: &str) {
    let path = hacivat_testkit::artifact(name);
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    eprintln!("gen_fn_catalog: wrote {}", path.display());
}

fn tsv(rows: &[(String, String)]) -> String {
    rows.iter().map(|(n, w)| format!("{n}\t{w}\n")).collect()
}

fn denied(d: DialectId) -> BTreeSet<String> {
    sql_safety::denied_function_names(d)
        .map(str::to_string)
        .collect()
}

#[test]
fn the_rules_drop_volatile_procedures_definers_and_denied_names() {
    let deny: BTreeSet<String> = ["pg_sleep".to_string()].into();
    let row = |line: &str| parse_proc(&line.replace('|', "\t")).unwrap();
    assert_eq!(
        proc_exclusion(&row("lower|f|i|internal|false|text"), &deny),
        None
    );
    assert_eq!(proc_exclusion(&row("now|f|s|internal|false|"), &deny), None);
    assert_eq!(
        proc_exclusion(&row("sum|a|i|internal|false|int4"), &deny),
        None
    );
    for (line, why) in [
        ("pg_sleep|f|v|internal|false|float8", "denied"),
        ("random|f|v|internal|false|", "provolatile v"),
        ("p|p|v|sql|false|", "prokind p"),
        ("f|f|i|plpgsql|false|", "language plpgsql"),
        ("f|f|i|c|false|", "language c"),
        ("f|f|s|sql|true|", "security definer"),
        (
            "query_to_xml|f|s|internal|false|text",
            "text argument is SQL or a path",
        ),
        (
            "pg_ls_dir|f|v|internal|false|text",
            "text argument is SQL or a path",
        ),
    ] {
        assert_eq!(
            proc_exclusion(&row(line), &deny).as_deref(),
            Some(why),
            "{line}"
        );
    }
}

#[test]
fn merging_sets_version_gates_and_drops_mixed_or_gapped_names() {
    let majors: Vec<String> = ["14", "15", "16", "17", "18"].map(String::from).to_vec();
    let mut per = PerLine::new();
    per.insert("lower".into(), (0..5).map(|i| (i, Ok(()))).collect());
    per.insert("new16".into(), (2..5).map(|i| (i, Ok(()))).collect());
    per.insert("gone16".into(), (0..2).map(|i| (i, Ok(()))).collect());
    per.insert("only18".into(), vec![(4, Ok(()))]);
    per.insert(
        "flipped".into(),
        vec![(0, Ok(())), (1, Err("provolatile v".into()))],
    );
    per.insert("gap".into(), vec![(0, Ok(())), (2, Ok(()))]);
    let (c, ex) = merge(&per, &majors, 3);
    let got: Vec<(&str, Option<&str>, Option<&str>, usize)> = c
        .iter()
        .map(|c| {
            (
                c.name.as_str(),
                c.since.as_deref(),
                c.until.as_deref(),
                c.source_line,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("gone16", None, Some("16"), 1),
            ("lower", None, None, 3),
            ("new16", Some("16"), None, 3),
            ("only18", Some("18"), None, 4),
        ]
    );
    let ex: Vec<&str> = ex.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(ex, ["flipped", "gap"]);
}

// -----------------------------------------------------------------------------------------------
// The catalog read (Docker)

/// The rows are tab-separated: operator names hold `|`.
const PROCS: &str = "SELECT p.proname::text || chr(9) || p.prokind::text || chr(9) || \
     p.provolatile::text || chr(9) || l.lanname::text || chr(9) || p.prosecdef::text || chr(9) || \
     pg_catalog.oidvectortypes(p.proargtypes) \
     FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_language l ON l.oid = p.prolang \
     WHERE p.oid < 16384 AND p.pronamespace = 'pg_catalog'::regnamespace ORDER BY 1";

const OPERATORS: &str = "SELECT o.oprname::text || chr(9) || \
     coalesce(string_agg(DISTINCT p.proname::text, ',') FILTER (WHERE p.provolatile = 'v'), '') \
     || chr(9) || string_agg(DISTINCT p.proname::text, ',') \
     FROM pg_catalog.pg_operator o JOIN pg_catalog.pg_proc p ON p.oid = o.oprcode::oid \
     WHERE o.oid < 16384 AND o.oprnamespace = 'pg_catalog'::regnamespace \
     GROUP BY o.oprname ORDER BY 1";

const TYPES: &str = "SELECT t.typname::text || chr(9) || t.typtype::text || chr(9) || \
     t.typcategory::text || chr(9) || p.provolatile::text \
     FROM pg_catalog.pg_type t JOIN pg_catalog.pg_proc p ON p.oid = t.typinput::oid \
     WHERE t.oid < 16384 AND t.typnamespace = 'pg_catalog'::regnamespace ORDER BY 1";

/// The server major of `line` as the data writes it (`"17"`, `"8.4"`, `"10.11"`).
fn major(line: &Line) -> String {
    line.target().major.clone()
}

fn sorted_by_major(mut lines: Vec<Line>) -> Vec<Line> {
    let key = |l: &Line| -> Vec<u32> {
        major(l)
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    lines.sort_by_key(key);
    lines
}

async fn read_pg(lines: &[Line]) {
    let majors: Vec<String> = lines.iter().map(major).collect();
    let reference = majors.iter().position(|m| m == "17").expect("a pg17 line");
    let deny = denied(DialectId::Postgres);
    let manual: BTreeSet<String> = manual_names("pg/allow.toml");
    let (mut fns, mut ops, mut types) = (PerLine::new(), PerLine::new(), PerLine::new());
    for (i, line) in lines.iter().enumerate() {
        let mut side = Side::open(line, None).await;
        let mut by_name: BTreeMap<String, Result<(), String>> = BTreeMap::new();
        for r in side.column(PROCS).await {
            let row = parse_proc(&r).unwrap_or_else(|| panic!("{}: bad row {r:?}", line.id));
            let verdict = match proc_exclusion(&row, &deny) {
                Some(why) => Err(why),
                None => Ok(()),
            };
            let slot = by_name.entry(row.name).or_insert(Ok(()));
            if slot.is_ok() {
                *slot = verdict;
            }
        }
        for (name, v) in by_name {
            fns.entry(name).or_default().push((i, v));
        }
        for r in side.column(OPERATORS).await {
            let mut f = r.splitn(3, '\t');
            let (op, volatile, codes) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
            let v = if !volatile.is_empty() {
                Err(format!("volatile implementation {volatile}"))
            } else if let Some(c) = codes.split(',').find(|c| deny.contains(*c)) {
                Err(format!("implemented by denied {c}"))
            } else {
                Ok(())
            };
            ops.entry(op.to_string()).or_default().push((i, v));
        }
        for r in side.column(TYPES).await {
            let f: Vec<&str> = r.split('\t').collect();
            let v = if !matches!(f[1], "b" | "d" | "e" | "r" | "m") {
                Err(format!("typtype {}", f[1]))
            } else if f[2] == "A" {
                Err("array".to_string())
            } else if f[3] == "v" {
                Err("volatile input function".to_string())
            } else {
                Ok(())
            };
            types.entry(f[0].to_string()).or_default().push((i, v));
        }
        side.close().await;
    }
    let src = |catalog: &str, c: &Candidate| format!("{catalog}@pg{}", majors[c.source_line]);
    let header = |what: &str| {
        format!(
            "# Candidates from {what} of pg_catalog (OID < 16384) on PostgreSQL {}; generated by \
             hacivat-exec/tests/gen_fn_catalog.rs, every row unreviewed.\n",
            majors.join(", ")
        )
    };
    let (c, mut ex) = merge(&fns, &majors, reference);
    let rows: Vec<(Candidate, String)> = c
        .into_iter()
        .filter(|c| !manual.contains(&c.name))
        .map(|c| {
            let s = src("pg_proc", &c);
            (c, s)
        })
        .collect();
    write_artifact("t03-pg-allow.toml", &toml_rows(&header("pg_proc"), &rows));
    let (c, ex_ops) = merge(&ops, &majors, reference);
    let rows: Vec<(Candidate, String)> = c
        .into_iter()
        .map(|c| {
            let s = src("pg_operator", &c);
            (c, s)
        })
        .collect();
    write_artifact(
        "t03-pg-operators.toml",
        &toml_rows(&header("pg_operator"), &rows),
    );
    let (c, ex_types) = merge(&types, &majors, reference);
    let rows: Vec<(Candidate, String)> = c
        .into_iter()
        .map(|c| {
            let s = src("pg_type", &c);
            (c, s)
        })
        .collect();
    write_artifact("t03-pg-types.toml", &toml_rows(&header("pg_type"), &rows));
    ex.extend(
        ex_ops
            .into_iter()
            .map(|(n, w)| (format!("operator {n}"), w)),
    );
    ex.extend(ex_types.into_iter().map(|(n, w)| (format!("type {n}"), w)));
    write_artifact("t03-pg-excluded.tsv", &tsv(&ex));
}

/// The names of the `source = "manual"` rows of `data/<file>` (the generator never proposes
/// them again).
fn manual_names(file: &str) -> BTreeSet<String> {
    let text = std::fs::read_to_string(data_dir().join(file)).unwrap_or_default();
    let doc: toml::Table = toml::from_str(&text).unwrap_or_default();
    doc.get("rows")
        .and_then(|r| r.as_array())
        .into_iter()
        .flatten()
        .filter_map(|r| r.as_table())
        .filter(|r| r.get("source").and_then(|s| s.as_str()) == Some("manual"))
        .filter_map(|r| r.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect()
}

/// The names of `data/mysql/manual-index.txt`, in order.
fn manual_index() -> Vec<String> {
    let text = std::fs::read_to_string(data_dir().join("mysql/manual-index.txt"))
        .expect("data/mysql/manual-index.txt");
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for l in text.lines() {
        let l = l.split('#').next().unwrap_or_default().trim();
        if l.is_empty() {
            continue;
        }
        assert!(
            l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "manual-index: {l:?}"
        );
        assert!(
            seen.insert(l.to_ascii_lowercase()),
            "manual-index: {l} twice"
        );
        out.push(l.to_string());
    }
    out
}

/// Whether `name` is a function on the line (`PREPARE` only; nothing runs), with the server's
/// answer: `ok`, or the error code (1582 wrong argument count and 1064 special syntax mean
/// present; 1305/1630 mean no such function).
async fn probe(side: &mut Side, name: &str) -> (bool, String) {
    match side
        .try_batch(&format!("PREPARE hv_t03 FROM 'SELECT {name}()'"))
        .await
    {
        Ok(()) => {
            side.batch("DEALLOCATE PREPARE hv_t03").await;
            (true, "ok".to_string())
        }
        Err(code) => (code != "1305" && code != "1630", code),
    }
}

async fn read_mysql_family(engine: DbmsKind, file_engine: &str, lines: &[Line]) {
    let d = if engine == DbmsKind::MariaDb {
        DialectId::MariaDb
    } else {
        DialectId::MySql
    };
    let deny = denied(d);
    let manual = manual_names(&format!("{file_engine}/allow.toml"));
    let majors: Vec<String> = lines.iter().map(major).collect();
    let names = manual_index();
    let mut per = PerLine::new();
    let mut codes: Vec<(String, String)> = Vec::new();
    let mut probes: Vec<(String, String)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let mut side = Side::open(line, None).await;
        // The probe must tell a missing function from a present one.
        let missing = probe(&mut side, "hv_t03_no_such_fn").await;
        assert!(
            !missing.0,
            "{}: the probe cannot see a missing function: {missing:?}",
            line.id
        );
        let abs = probe(&mut side, "ABS").await;
        assert!(abs.0, "{}: ABS: {abs:?}", line.id);
        for n in &names {
            if deny.contains(&n.to_ascii_lowercase()) {
                continue;
            }
            let (found, code) = probe(&mut side, n).await;
            probes.push((n.clone(), format!("{}\t{code}", majors[i])));
            if found {
                per.entry(n.to_ascii_uppercase())
                    .or_default()
                    .push((i, Ok(())));
            } else {
                codes.push((n.clone(), format!("missing on {}", majors[i])));
            }
        }
        side.close().await;
    }
    let reference = majors.len() - 1;
    let (c, mut ex) = merge(&per, &majors, reference);
    let rows: Vec<(Candidate, String)> = c
        .into_iter()
        .filter(|c| !manual.iter().any(|m| m.eq_ignore_ascii_case(&c.name)))
        .map(|c| (c, "manual-index".to_string()))
        .collect();
    let header = format!(
        "# Candidates from data/mysql/manual-index.txt present on {file_engine} {}; generated by \
         hacivat-exec/tests/gen_fn_catalog.rs, every row unreviewed.\n",
        majors.join(", ")
    );
    write_artifact(
        &format!("t03-{file_engine}-allow.toml"),
        &toml_rows(&header, &rows),
    );
    ex.extend(codes);
    write_artifact(&format!("t03-{file_engine}-excluded.tsv"), &tsv(&ex));
    write_artifact(&format!("t03-{file_engine}-probe.tsv"), &tsv(&probes));
}

fn read_sqlite() {
    let conn = rusqlite::Connection::open_in_memory().expect("an in-memory SQLite");
    let mut stmt = conn
        .prepare("SELECT DISTINCT name, builtin, type FROM pragma_function_list ORDER BY name")
        .expect("pragma_function_list");
    let rows: Vec<(String, i64, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("pragma_function_list rows")
        .collect::<Result<_, _>>()
        .expect("pragma_function_list rows");
    let deny = denied(DialectId::Sqlite);
    let manual = manual_names("sqlite/allow.toml");
    let mut names = BTreeSet::new();
    let mut ex = Vec::new();
    for (name, builtin, kind) in rows {
        let key = name.to_ascii_lowercase();
        if deny.contains(&key) {
            ex.push((name, "denied".to_string()));
        } else if builtin != 1 {
            ex.push((name, "not builtin".to_string()));
        } else if !matches!(kind.as_str(), "s" | "a" | "w") {
            ex.push((name, format!("type {kind}")));
        } else if !manual.contains(&key) {
            names.insert(key);
        }
    }
    let rows: Vec<(Candidate, String)> = names
        .into_iter()
        .map(|name| {
            (
                Candidate {
                    name,
                    since: None,
                    until: None,
                    source_line: 0,
                },
                "pragma_function_list".to_string(),
            )
        })
        .collect();
    let version = rusqlite::version();
    let header = format!(
        "# Candidates from pragma_function_list of the bundled SQLite {version}; generated by \
         hacivat-exec/tests/gen_fn_catalog.rs, every row unreviewed.\n"
    );
    write_artifact("t03-sqlite-allow.toml", &toml_rows(&header, &rows));
    ex.sort();
    ex.dedup();
    write_artifact("t03-sqlite-excluded.tsv", &tsv(&ex));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn gen_fn_catalog() {
    if !wanted() {
        eprintln!("gen_fn_catalog: set HACIVAT_GEN_FN_CATALOG=1 to read the catalogs");
        return;
    }
    assert!(
        must_exist(),
        "the catalog read needs every line: run it with --set full"
    );
    let lines = server_lines();
    let of =
        |e: DbmsKind| sorted_by_major(lines.iter().filter(|l| l.engine == e).cloned().collect());
    read_pg(&of(DbmsKind::Postgres)).await;
    read_mysql_family(DbmsKind::MySql, "mysql", &of(DbmsKind::MySql)).await;
    read_mysql_family(DbmsKind::MariaDb, "mariadb", &of(DbmsKind::MariaDb)).await;
    read_sqlite();
}
