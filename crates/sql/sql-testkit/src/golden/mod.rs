//! The D-011 goldens (Task 13): the CARET, SEL and CHOOSE families of
//! `docs/research/datagrip/13-behavioral-test-corpus.md` §3.6–§3.8 as `corpus/datagrip/*.toml`.
//!
//! Clean room (ALG-TEST-01): a case takes its *scenario* (input, caret, policy) from that
//! Hacivat-authored document and its *expectation* from Hacivat's specs (`alg/10` SQL-SPLIT-04
//! aligned to D-011, `alg/20` HV-EXEC-01/-02, the M3a rulings), never from the document's
//! "Expected hint" column, which is guesswork about another product. `datagrip_observed` stays
//! empty unless a black-box run is ever recorded.
//!
//! A case:
//! - `id`: the corpus ID (`CARET-07`); every ID of `corpus/datagrip/ids/<family>.txt` must be
//!   present, run or skipped with a reason (the meta-test in `sql-split/tests/d011_goldens.rs`);
//! - `dialect` (default `postgres`), `version`, `flavor` (default: the dialect's client);
//! - the text: `input` with `┃` carets and `«…»` selections, or `fixture = "FX-…"` (verbatim
//!   from `corpus/fixtures/`) with `caret = { after = "…", occurrence = n }` (or `before`),
//!   `carets = [..]`, or `select = { text = "…" }` / `{ all = true }`;
//! - `policy = { inside, outside, selection }` (defaults: D-011) and `remembered` (a kind);
//! - `requires`: `cst` (the case needs a parse tree: M3a runs `expect_without_cst`, M5 runs
//!   `expect`), or a capability M3a's runner never has, which then needs `skip = "<the same>"`;
//! - `expect` / `expect_without_cst`: one of `units_text` (each unit's exact bytes),
//!   `candidates = [{ kind, units_text }]`, `resolution = "nothing" | "ambiguous" |
//!   "unsupported_client_command"`; plus `unit_kinds`,
//!   `titles` (each unit's title comment, `""` for none), `preselect`, `labels`,
//!   `label_starts` (the preselected candidate's label);
//! - `sources`: where the expectation comes from.

pub mod markers;
mod runner;

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub use markers::{Anchor, Marked, Select};
pub use runner::{Outcome, run};

/// The policy a case runs under; a missing part is D-011's default.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicySpec {
    #[serde(default)]
    pub inside: Option<String>,
    #[serde(default)]
    pub outside: Option<String>,
    #[serde(default)]
    pub selection: Option<String>,
}

/// One expected chooser candidate: its kind and the exact text of each unit it runs.
///
/// Expectations are texts, never indices into the split under test: a case says what runs
/// in the corpus's own words, so a wrong split cannot agree with itself.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateSpec {
    pub kind: String,
    pub units_text: Vec<String>,
}

/// What a case expects.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    #[serde(default)]
    pub units_text: Option<Vec<String>>,
    #[serde(default)]
    pub candidates: Option<Vec<CandidateSpec>>,
    #[serde(default)]
    pub resolution: Option<String>,
    /// Each unit's kind: `statement`, `subquery`, `batch`, `verbatim`, `copy_in`.
    #[serde(default)]
    pub unit_kinds: Option<Vec<String>>,
    /// Each unit's title comment (`""`: none).
    #[serde(default)]
    pub titles: Option<Vec<String>>,
    /// The preselected candidate's kind.
    #[serde(default)]
    pub preselect: Option<String>,
    /// Every candidate's label.
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    /// How the preselected candidate's label starts.
    #[serde(default)]
    pub label_starts: Option<String>,
}

/// One golden case.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    #[serde(default)]
    pub dialect: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub flavor: Option<String>,
    #[serde(default)]
    pub input: Option<String>,
    #[serde(default)]
    pub fixture: Option<String>,
    #[serde(default)]
    pub caret: Option<Anchor>,
    #[serde(default)]
    pub carets: Vec<Anchor>,
    #[serde(default)]
    pub select: Option<Select>,
    #[serde(default)]
    pub policy: PolicySpec,
    #[serde(default)]
    pub remembered: Option<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    /// Why the case does not run in this runner: one of its `requires`.
    #[serde(default)]
    pub skip: Option<String>,
    #[serde(default)]
    pub expect: Option<Expect>,
    #[serde(default)]
    pub expect_without_cst: Option<Expect>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Empty unless a black-box observation is ever recorded (ALG-TEST-01); never an
    /// expectation.
    #[serde(default)]
    pub datagrip_observed: Option<String>,
}

impl Case {
    /// The case's family: its ID without the number (`CARET`).
    pub fn family(&self) -> &str {
        self.id.rsplit_once('-').map_or(&self.id, |(f, _)| f)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFile {
    case: Vec<Case>,
}

/// One entry of the divergence register (`corpus/datagrip/divergences.toml`, ALG-TEST-01 step
/// 5): a case where Hacivat's M3a answer differs on purpose from the corpus scenario's
/// premise or from the catalogue's later answer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Divergence {
    pub id: String,
    pub what: String,
    pub reason: String,
    pub sources: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DivergenceFile {
    divergence: Vec<Divergence>,
}

/// `corpus/<rel>` in this crate (`datagrip/caret.toml`).
pub fn corpus(rel: &str) -> PathBuf {
    let mut path = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    path.extend(rel.split('/'));
    path
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The cases of the corpus file at `path`.
///
/// # Panics
/// When the file is missing, malformed or holds no case, naming it.
pub fn load(path: &Path) -> Vec<Case> {
    let file: CaseFile =
        toml::from_str(&read(path)).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(!file.case.is_empty(), "{}: no case", path.display());
    file.case
}

/// The IDs of `corpus/datagrip/ids/<family>.txt` (`caret`), one per line, taken once from the
/// research document.
///
/// # Panics
/// When the file is missing.
pub fn ids(family: &str) -> Vec<String> {
    read(&corpus(&format!("datagrip/ids/{family}.txt")))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The divergence register.
///
/// # Panics
/// When the file is missing or malformed.
pub fn divergences() -> Vec<Divergence> {
    let path = corpus("datagrip/divergences.toml");
    let file: DivergenceFile =
        toml::from_str(&read(&path)).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    file.divergence
}
