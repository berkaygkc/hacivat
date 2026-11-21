//! `@<environment>/` scoping, fail closed (HV-SCRIPT-04; R-M4b-6).
//!
//! ## m4b-t01a-seams
//! Task 1a: the types ([`EnvScope`], [`EnvSet`], [`ScopeProblem`]) and the seams [`scope_of`]
//! and [`visible`].
//!
//! ## m4b-t04-scanner
//! Task 4 writes the rules (ADR 0009 `m4b-t04`; PO Q3 option (a)):
//! - A folder component whose first character is `@` is an environment folder. It scopes only
//!   when the rest equals one of the five tag ids (`local`, `dev`, `test`, `staging`,
//!   `production`; `EnvTag::id`) under ASCII case-insensitive comparison. Never locale casing:
//!   `@productİon` and `@productıon` are not `@production` in any locale.
//! - Any other `@name` is unknown and hides the file everywhere, with a "did you mean" tag
//!   ([`classify`]). So does a folder that a person would read as an environment folder but
//!   whose `@` is a full-width or small commercial at (`＠`, `﹫`), or that has an invisible
//!   character after its `@` (`@production\u{200B}`, a bidi override). Treating those as plain
//!   folders would show a "production-only" script on staging.
//! - ## m4b-t04-scanner fix round 1 (I1): characters a person does not see in front of the
//!   `@` (white space, NBSP, U+3000, default-ignorables, combining marks, blank glyphs) are
//!   skipped first, so ` @production` and `\u{200B}@production` scope like `@production`.
//! - ## m4b-t04-scanner fix round 1 (m7): a library root inside an environment folder is
//!   refused ([`root_env_folder`]), because scoping is relative to the root.
//! - The allowed set is the intersection over the path's environment folders; an empty
//!   intersection is a contradiction and hides the file. An untagged connection sees only
//!   files with no environment folder.
//! - Folder components only: for a file, every component but the last ([`scope_of`]); for a
//!   folder entry, every component ([`folder_scope`]). A file named `@production.sql` scopes
//!   nothing.
//! - [`visible`] recomputes the scope from the entry's path and also honours the scope stored
//!   in the entry (the scan narrows a file link's scope to its target's, [`combine`]); the
//!   stricter of the two wins, so an entry built by hand with `EnvScope::Any` cannot widen it.

use std::path::{Component, Path};
use std::sync::LazyLock;

use hacivat_proto::EnvTag;
use regex::Regex;

use crate::pathsafe::{RelPath, display_name};
use crate::scan::{EntryKind, ScriptEntry};

/// Where a file may be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvScope {
    /// No `@` folder on its path: every connection.
    Any,
    /// Only connections whose environment tag is in the set.
    Only(EnvSet),
    /// Never shown.
    Hidden(ScopeProblem),
}

/// A set of environment tags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct EnvSet(u8);

impl EnvSet {
    /// The empty set.
    pub const fn empty() -> Self {
        EnvSet(0)
    }

    /// The set holding `tag` only.
    pub fn of(tag: EnvTag) -> Self {
        EnvSet(bit(tag))
    }

    pub fn with(self, tag: EnvTag) -> Self {
        EnvSet(self.0 | bit(tag))
    }

    pub fn contains(self, tag: EnvTag) -> bool {
        self.0 & bit(tag) != 0
    }

    pub fn intersect(self, other: EnvSet) -> Self {
        EnvSet(self.0 & other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// ## m4b-t01a-fr1 (review M3): exhaustive, so a new tag fails to compile here instead of
/// silently sharing another tag's bit.
fn bit(tag: EnvTag) -> u8 {
    match tag {
        EnvTag::Local => 1 << 0,
        EnvTag::Dev => 1 << 1,
        EnvTag::Test => 1 << 2,
        EnvTag::Staging => 1 << 3,
        EnvTag::Production => 1 << 4,
    }
}

/// Why a path is hidden everywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeProblem {
    /// The first unknown environment folder on the path, as [`display_name`] shows it.
    Unknown(String),
    /// Two environment folders on the path allow no tag in common (`@production/@staging/`).
    Contradiction,
}

/// What one folder name means for scoping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvFolder {
    /// An ordinary folder.
    NotEnv,
    /// `@<tag>`: scopes to `tag`.
    Tag(EnvTag),
    /// An environment folder that names no tag: hides everything under it. `suggestion` is
    /// the tag a person most likely meant, for the "did you mean" diagnostic.
    Unknown { suggestion: Option<EnvTag> },
}

/// Characters that read as `@` (ASCII, FULLWIDTH COMMERCIAL AT, SMALL COMMERCIAL AT).
fn is_at_like(c: char) -> bool {
    matches!(c, '@' | '\u{FF20}' | '\u{FE6B}')
}

/// A run of characters a person does not see at the start of a name (fix round 1, I1):
/// Unicode `White_Space` (NBSP, U+3000, the en/em spaces), default-ignorable code points,
/// combining marks (`M*`), controls, format, private-use and unassigned code points, and the
/// blank glyphs (Braille blank U+2800, the Hangul fillers, the musical null notehead U+1D159).
static UNSEEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^[\p{White_Space}\p{Default_Ignorable_Code_Point}\p{M}\p{Cc}\p{Cf}\p{Co}\p{Cn}\u{2800}\u{3164}\u{FFA0}\u{115F}\u{1160}\u{1D159}]+",
    )
    .expect("a constant pattern")
});

/// Whether `c` is one of the [`UNSEEN`] characters.
fn is_unseen(c: char) -> bool {
    if c.is_ascii_graphic() {
        return false;
    }
    let mut buf = [0u8; 4];
    UNSEEN.is_match(c.encode_utf8(&mut buf))
}

/// Classifies one folder name.
///
/// Leading characters a person does not see are skipped (fix round 1, I1, controller
/// ruling): a name whose first visible character is `@` is an environment folder. It scopes
/// when that `@` is the ASCII one and the rest is exactly a tag (ASCII case-insensitive);
/// anything else (a lookalike `@`, a misspelt tag, an invisible character after the `@`) is
/// unknown and hides everything under it.
pub fn classify(name: &str) -> EnvFolder {
    // Fast path: a name that starts with an ASCII letter, digit or punctuation other than `@`
    // starts with a visible character that is not `@`.
    if name
        .as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_graphic() && *b != b'@')
    {
        return EnvFolder::NotEnv;
    }
    let lead = if name.starts_with('@') {
        0
    } else {
        UNSEEN.find(name).map_or(0, |m| m.end())
    };
    let mut chars = name[lead..].chars();
    if !chars.next().is_some_and(is_at_like) {
        return EnvFolder::NotEnv;
    }
    let after = chars.as_str();
    if name[lead..].starts_with('@')
        && let Some(tag) = EnvTag::ALL
            .into_iter()
            .find(|t| after.eq_ignore_ascii_case(t.id()))
    {
        return EnvFolder::Tag(tag);
    }
    let seen: String = after.chars().filter(|c| !is_unseen(*c)).collect();
    EnvFolder::Unknown {
        suggestion: suggest(&seen),
    }
}

/// The first component of a library root's canonical path (its own name included) that is an
/// environment folder, as [`display_name`] shows it (fix round 1, m7). Scoping is relative to
/// the root, so such a root would show `@production/reports/purge.sql` everywhere: the scan
/// and `join` refuse it instead, and attach (Tasks 3, 16) can say so before it is added.
pub fn root_env_folder(canonical: &Path) -> Option<String> {
    canonical.components().find_map(|c| match c {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            (classify(&name) != EnvFolder::NotEnv).then(|| display_name(&name))
        }
        _ => None,
    })
}

/// The tag a misspelt `rest` most likely means: an ASCII case-insensitive match, else the
/// closest tag by optimal string alignment distance (at most 3, and at most half the tag's
/// length, so `@x` suggests nothing), else a tag that `rest` starts (3 characters or more,
/// `@prod`) or that starts `rest` (`@develop`).
fn suggest(rest: &str) -> Option<EnvTag> {
    let folded: Vec<char> = rest.chars().map(|c| c.to_ascii_lowercase()).collect();
    if folded.is_empty() {
        return None;
    }
    let near = EnvTag::ALL
        .into_iter()
        .filter_map(|t| {
            let id: Vec<char> = t.id().chars().collect();
            let d = osa_distance(&folded, &id);
            (d <= 3 && d <= id.len() / 2).then_some((d, t.severity_rank(), t))
        })
        .min();
    if let Some((_, _, tag)) = near {
        return Some(tag);
    }
    let text: String = folded.iter().collect();
    EnvTag::ALL
        .into_iter()
        .find(|t| (folded.len() >= 3 && t.id().starts_with(&text)) || text.starts_with(t.id()))
}

/// The optimal string alignment distance (Levenshtein plus adjacent transpositions).
fn osa_distance(a: &[char], b: &[char]) -> usize {
    let w = b.len() + 1;
    let mut d = vec![0usize; (a.len() + 1) * w];
    for (i, row) in d.chunks_mut(w).enumerate() {
        row[0] = i;
    }
    for (j, cell) in d.iter_mut().take(w).enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[(i - 1) * w + j] + 1)
                .min(d[i * w + j - 1] + 1)
                .min(d[(i - 1) * w + j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[(i - 2) * w + j - 2] + 1);
            }
            d[i * w + j] = best;
        }
    }
    d[a.len() * w + b.len()]
}

/// The scope of a file at `rel` from its folders (every component but the last).
pub fn scope_of(rel: &RelPath) -> EnvScope {
    let parts: Vec<&str> = rel.components().collect();
    let folders = parts.split_last().map_or(&[][..], |(_, f)| f);
    scope_over(folders)
}

/// The scope of a folder at `rel` (every component, its own name included).
pub fn folder_scope(rel: &RelPath) -> EnvScope {
    let parts: Vec<&str> = rel.components().collect();
    scope_over(&parts)
}

/// The scope of an entry: [`folder_scope`] for a folder, [`scope_of`] for a file.
pub fn entry_scope(rel: &RelPath, kind: EntryKind) -> EnvScope {
    match kind {
        EntryKind::Folder => folder_scope(rel),
        EntryKind::Sql | EntryKind::Markdown => scope_of(rel),
    }
}

fn scope_over(folders: &[&str]) -> EnvScope {
    let mut allowed: Option<EnvSet> = None;
    for name in folders {
        match classify(name) {
            EnvFolder::NotEnv => {}
            EnvFolder::Tag(tag) => {
                let set = EnvSet::of(tag);
                allowed = Some(allowed.map_or(set, |a| a.intersect(set)));
            }
            EnvFolder::Unknown { .. } => {
                return EnvScope::Hidden(ScopeProblem::Unknown(display_name(name)));
            }
        }
    }
    match allowed {
        None => EnvScope::Any,
        Some(set) if set.is_empty() => EnvScope::Hidden(ScopeProblem::Contradiction),
        Some(set) => EnvScope::Only(set),
    }
}

/// The intersection of two scopes: where both allow (a file link and its target, the scan).
/// A hidden scope wins (the first one's problem when both are hidden).
pub fn combine(a: &EnvScope, b: &EnvScope) -> EnvScope {
    match (a, b) {
        (EnvScope::Hidden(_), _) => a.clone(),
        (_, EnvScope::Hidden(_)) => b.clone(),
        (EnvScope::Any, _) => b.clone(),
        (_, EnvScope::Any) => a.clone(),
        (EnvScope::Only(x), EnvScope::Only(y)) => {
            let both = x.intersect(*y);
            if both.is_empty() {
                EnvScope::Hidden(ScopeProblem::Contradiction)
            } else {
                EnvScope::Only(both)
            }
        }
    }
}

/// Whether `scope` admits a connection whose environment is `env` (an untagged connection is
/// admitted only by [`EnvScope::Any`]).
pub fn admits(scope: &EnvScope, env: Option<EnvTag>) -> bool {
    match scope {
        EnvScope::Any => true,
        EnvScope::Only(set) => env.is_some_and(|t| set.contains(t)),
        EnvScope::Hidden(_) => false,
    }
}

/// Whether `entry` is shown under a connection whose environment is `env`: both the scope its
/// path gives it now and the scope the scan stored must admit `env`.
pub fn visible(entry: &ScriptEntry, env: Option<EnvTag>) -> bool {
    admits(&entry_scope(&entry.rel, entry.kind), env) && admits(&entry.scope, env)
}

/// Whether what [`pathsafe::join`](crate::pathsafe::join) resolved for `rel` may be opened
/// under a connection whose environment is `env` (fix round 1, I2): the scope of `rel` and, when
/// `rel` was a link, the scope of the target it resolved to now. The open (Task 5) and the tab
/// (Task 14) call this on `join`'s answer before they read a byte.
pub fn visible_at(
    rel: &RelPath,
    kind: EntryKind,
    target: Option<&RelPath>,
    env: Option<EnvTag>,
) -> bool {
    let here = entry_scope(rel, kind);
    let scope = match target {
        Some(t) => combine(&here, &scope_of(t)),
        None => here,
    };
    admits(&scope, env)
}
// ## m4b-t04-scanner
