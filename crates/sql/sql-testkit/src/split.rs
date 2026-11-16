//! The splitter corpora (Task 8): `sql-dialect/data/split/<profile>.toml` (one fixture per
//! splitter-table row, `covers = ["<row id>"]`) and `corpus/split/<dialect>.toml` (the `alg/10`
//! SQL-SPLIT-01/-02/-03 cases, their R-M3-5 errata, and the `FX-SPLIT` fixtures). Expectations
//! come from Hacivat's specs (`alg/10`), never from another product (ALG-TEST-01).
//!
//! A case names its input (`input`, or `fixture = "FX-…"` from `corpus/fixtures/`), its dialect
//! and client flavor (file defaults), and what each mode gives:
//! - `editor`: the editor-mode statements' strict texts;
//! - `exec`: the execution-mode statements' strict texts (default: `editor`);
//! - `heuristic`: whether every editor statement is `Heuristic` (`true`) or `Exact` (`false`);
//!   default `false` when `ambiguous` is empty, unchecked otherwise; `confidence` instead names
//!   each one (`E`, `H`);
//! - `ambiguous`: the execution-mode ambiguity reasons, by `Debug` name, in text order;
//! - `client`: the client segments' texts; `empty`: the number of empty items;
//! - `diag`: the diagnostic ids (`split.…`), unchecked when absent;
//! - `copy`: the `COPY … FROM STDIN` data rows' text, `copy_end`: whether `\.` ends them;
//! - `crlf`: the case also holds with every line feed written `\r\n`.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use sql_dialect::{ClientFlavor, DialectCtx};
use sql_split::{Confidence, Split, SplitMode, SplitStart, split_from};

use crate::lex::{ModeOverrides, dialect_ctx};

/// One split case.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitCase {
    pub id: String,
    /// Where the expectation comes from.
    #[serde(default)]
    pub source: Option<String>,
    /// The splitter-table rows the case exercises.
    #[serde(default)]
    pub covers: Vec<String>,
    /// The dialects it runs under (default: the file's).
    #[serde(default)]
    pub dialects: Option<Vec<String>>,
    #[serde(default)]
    pub version: Option<String>,
    /// `psql`, `mysql`, `sqlite3` or `none` (default: the file's).
    #[serde(default)]
    pub flavor: Option<String>,
    #[serde(default)]
    pub modes: ModeOverrides,
    #[serde(default)]
    pub input: Option<String>,
    #[serde(default)]
    pub fixture: Option<String>,
    pub editor: Vec<String>,
    #[serde(default)]
    pub exec: Option<Vec<String>>,
    #[serde(default)]
    pub heuristic: Option<bool>,
    /// Each editor statement's confidence, `E` or `H` (instead of `heuristic`).
    #[serde(default)]
    pub confidence: Option<Vec<String>>,
    #[serde(default)]
    pub ambiguous: Vec<String>,
    #[serde(default)]
    pub client: Vec<String>,
    #[serde(default)]
    pub empty: Option<usize>,
    #[serde(default)]
    pub diag: Option<Vec<String>>,
    #[serde(default)]
    pub copy: Option<String>,
    #[serde(default)]
    pub copy_end: Option<bool>,
    #[serde(default)]
    pub crlf: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFile {
    dialects: Vec<String>,
    #[serde(default)]
    flavor: Option<String>,
    case: Vec<SplitCase>,
}

/// The `sql-testkit` crate directory.
fn testkit_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `corpus/split/<name>.toml`.
pub fn corpus_file(name: &str) -> PathBuf {
    testkit_dir()
        .join("corpus")
        .join("split")
        .join(format!("{name}.toml"))
}

/// `sql-dialect/data/split/<name>.toml`.
pub fn profile_fixture_file(name: &str) -> PathBuf {
    testkit_dir()
        .join("..")
        .join("sql-dialect")
        .join("data")
        .join("split")
        .join(format!("{name}.toml"))
}

/// The text of `corpus/fixtures/<id>.sql`.
pub fn fixture(id: &str) -> String {
    let path = testkit_dir()
        .join("corpus")
        .join("fixtures")
        .join(format!("{id}.sql"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The cases of `path`, with the file's dialects and flavor filled in.
pub fn load(path: &Path) -> Vec<SplitCase> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let file: CaseFile =
        toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    file.case
        .into_iter()
        .map(|mut c| {
            c.dialects.get_or_insert_with(|| file.dialects.clone());
            if c.flavor.is_none() {
                c.flavor.clone_from(&file.flavor);
            }
            c
        })
        .collect()
}

/// The client flavor named `name`.
pub fn flavor(name: Option<&str>) -> ClientFlavor {
    match name {
        None | Some("none") => ClientFlavor::None,
        Some("psql") => ClientFlavor::Psql,
        Some("mysql") => ClientFlavor::MySqlCli,
        Some("sqlite3") => ClientFlavor::Sqlite3,
        Some(other) => panic!("unknown client flavor {other:?}"),
    }
}

impl SplitCase {
    /// The case's input text.
    pub fn text(&self) -> String {
        match (&self.input, &self.fixture) {
            (Some(i), None) => i.clone(),
            (None, Some(f)) => fixture(f),
            _ => panic!("{}: exactly one of `input` and `fixture`", self.id),
        }
    }

    /// The contexts the case runs in.
    pub fn contexts(&self) -> Vec<DialectCtx> {
        let m = self.modes;
        self.dialects
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|d| {
                let mut ctx = dialect_ctx(d, self.version.as_deref());
                let modes = &mut ctx.modes;
                modes.standard_conforming_strings = m
                    .standard_conforming_strings
                    .unwrap_or(modes.standard_conforming_strings);
                modes.ansi_quotes = m.ansi_quotes.unwrap_or(modes.ansi_quotes);
                modes.no_backslash_escapes =
                    m.no_backslash_escapes.unwrap_or(modes.no_backslash_escapes);
                ctx
            })
            .collect()
    }

    /// Checks the case in every context (and with CRLF line ends when `crlf`): `Err` says
    /// what differs.
    pub fn check(&self) -> Result<(), String> {
        let lf = self.text();
        let mut inputs = vec![(lf.clone(), false)];
        if self.crlf {
            inputs.push((lf.replace('\n', "\r\n"), true));
        }
        for ctx in self.contexts() {
            for (text, crlf) in &inputs {
                self.check_one(text, &ctx, *crlf).map_err(|e| {
                    format!(
                        "{} [{:?}{}]: {e}",
                        self.id,
                        ctx.id,
                        if *crlf { ", CRLF" } else { "" }
                    )
                })?;
            }
        }
        Ok(())
    }

    fn check_one(&self, text: &str, ctx: &DialectCtx, crlf: bool) -> Result<(), String> {
        let fl = flavor(self.flavor.as_deref());
        let start = SplitStart::default();
        let editor = split_from(text, ctx, fl, SplitMode::Editor, &start);
        let exec = split_from(text, ctx, fl, SplitMode::Execution, &start);
        let fix = |v: &[String]| -> Vec<String> {
            v.iter()
                .map(|s| {
                    if crlf {
                        s.replace('\n', "\r\n")
                    } else {
                        s.clone()
                    }
                })
                .collect()
        };
        let want_editor = fix(&self.editor);
        let want_exec = fix(self.exec.as_deref().unwrap_or(&self.editor));
        expect(
            "editor statements",
            &strict_texts(text, &editor),
            &want_editor,
        )?;
        expect(
            "execution statements",
            &strict_texts(text, &exec),
            &want_exec,
        )?;
        // An execution statement is `Exact` exactly when it is certain, and every ambiguity
        // lies inside one execution statement.
        for s in exec.statements() {
            let overlapped = exec.ambiguities_in(s.strict).next().is_some();
            if (s.confidence == Confidence::Exact) == overlapped || exec.is_certain(s) == overlapped
            {
                return Err(format!(
                    "execution statement {:?} is {:?}; an ambiguity overlaps it: {overlapped}",
                    &text[s.strict], s.confidence
                ));
            }
        }
        if let Some(a) = exec
            .ambiguous
            .iter()
            .find(|a| !exec.statements().any(|s| s.strict.contains_range(a.at)))
        {
            return Err(format!("ambiguity {a:?} lies inside no statement"));
        }
        let reasons: Vec<String> = exec
            .ambiguous
            .iter()
            .map(|a| format!("{:?}", a.reason))
            .collect();
        expect("ambiguous", &reasons, &self.ambiguous)?;
        if let Some(want) = &self.confidence {
            let got: Vec<String> = editor
                .statements()
                .map(|s| match s.confidence {
                    Confidence::Exact => "E".to_string(),
                    Confidence::Heuristic => "H".to_string(),
                })
                .collect();
            expect("editor confidence", &got, want)?;
        }
        let heuristic = self
            .heuristic
            .or((self.ambiguous.is_empty() && self.confidence.is_none()).then_some(false));
        if let Some(h) = heuristic {
            let want = if h {
                Confidence::Heuristic
            } else {
                Confidence::Exact
            };
            if let Some(s) = editor.statements().find(|s| s.confidence != want) {
                return Err(format!(
                    "editor statement {:?} is {:?}, expected {want:?}",
                    &text[s.strict], s.confidence
                ));
            }
        }
        let client: Vec<String> = exec
            .client
            .iter()
            .map(|c| text[c.range].to_string())
            .collect();
        expect("client segments", &client, &fix(&self.client))?;
        if let Some(n) = self.empty {
            let got = exec.items.len() - exec.statements().count();
            if got != n {
                return Err(format!("{got} empty items, expected {n}"));
            }
        }
        if let Some(diag) = &self.diag {
            let got: Vec<String> = exec
                .diagnostics
                .iter()
                .map(|d| d.code.id().to_string())
                .collect();
            expect("diagnostics", &got, diag)?;
        }
        if let Some(rows) = &self.copy {
            let data = exec
                .statements()
                .find_map(|s| s.copy_data)
                .ok_or("no COPY data")?;
            expect(
                "COPY rows",
                &[text[data.rows].to_string()],
                &fix(std::slice::from_ref(rows)),
            )?;
            if let Some(end) = self.copy_end
                && data.end.is_some() != end
            {
                return Err(format!("COPY end marker present: {}", data.end.is_some()));
            }
        }
        Ok(())
    }
}

fn expect(what: &str, got: &[String], want: &[String]) -> Result<(), String> {
    if got == want {
        Ok(())
    } else {
        Err(format!("{what}: got {got:?}, expected {want:?}"))
    }
}

/// The statements' strict texts.
pub fn strict_texts(text: &str, split: &Split) -> Vec<String> {
    split
        .statements()
        .map(|s| text[s.strict].to_string())
        .collect()
}
