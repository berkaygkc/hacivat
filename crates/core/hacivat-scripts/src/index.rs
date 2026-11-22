// ## m4b-t01a-seams
//! The "Go to script" index over names only, never contents (HV-SCRIPT-09; R-M4b-15), keyed by
//! the I18N-09 search key (`hacivat_l10n::search::build_key`). Task 10 writes it.
// ## m4b-t01a-seams
//!
//! ## m4b-t10-service
//! [`ScriptIndex::build`] takes a [`WorkspaceSnapshot`] and nothing else: it never touches a
//! file system, so it cannot read a script's contents (`index_reads_no_contents`).
//!
//! - **Entries.** Every listed script file (`.sql`, `.md`) of every library, except files hidden
//!   everywhere (an unknown `@name` folder or a contradictory chain, R-M4b-6) and disabled links
//!   (`OutsideLink`, `Loop`), which can never be opened. Folders are not offered.
//! - **Display.** A name, its folder and the library's folder name are shown through
//!   [`display_name`]: NFC (macOS hands out NFD), every control and invisible formatting
//!   character as `<U+XXXX>` (R-M4b-23). Identity stays the scan's exact [`RelPath`].
//! - **Keys.** The search keys are built from the display text with the caller's
//!   `turkish_tolerant` choice (D-074: `musteri` finds `müşteri_segment.sql` for a Turkish user,
//!   and does not otherwise). They are built once per choice, on the first query that needs them.
//! - **Rank** ([`Rank`], HV-SCRIPT-09 step 2): the exact name (with or without its extension) >
//!   a name prefix > a word prefix inside the name > a substring > a subsequence > a match in the
//!   library-relative folder or the library's name only. Ties: the shorter path, then the
//!   library's order in the snapshot, then the path's bytes. ("Recently opened" is the app's.)
//! - **Scope.** [`ScriptIndex::query_for`] lists only the files shown under a connection whose
//!   environment is `env` (`envscope::visible`, the same rule as the tree), so a `@production`
//!   file never appears for a staging connection; [`ScriptIndex::query_for_any`] those shown
//!   under at least one of several. Fix round 1 (m2): no public query or count spans every
//!   environment ([`ScriptIndex::len_for`]).
//! - **Cost** (m3): a query is cut to [`MAX_QUERY_CHARS`] characters. Keys are built on the
//!   first query of each tolerance, under a `OnceLock`: a second thread querying meanwhile waits
//!   for it (m4), so Task 15 queries one index from one thread (off the UI thread past 2,000
//!   entries).

use std::fmt;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use hacivat_l10n::search::{MatchKind, MatchMode, PreparedQuery, SearchKey, SearchOpts, build_key};
use hacivat_proto::EnvTag;

use crate::envscope::{self, EnvScope};
use crate::pathsafe::{RelPath, display_name};
use crate::registry::LibraryId;
use crate::scan::{EntryKind, LinkState, ScriptEntry};
use crate::service::WorkspaceSnapshot;

/// How a hit matched, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rank {
    /// The query is the whole name, or the name without its extension.
    Exact,
    /// The name starts with the query.
    Prefix,
    /// A word inside the name starts with the query (`big_orders` for `ord`).
    WordPrefix,
    /// The query is inside the name.
    Substring,
    /// The query's characters appear in order in the name (or as an acronym).
    Subsequence,
    /// The query matches only the folder or the library's name.
    Path,
}

/// One offered script.
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub library: LibraryId,
    pub entry: ScriptEntry,
    /// The file name as shown (NFC, escaped).
    pub display_name: String,
    /// The library-relative folder as shown (`""` at the root).
    pub display_folder: String,
    /// The path's length in characters (the first tie-break).
    path_len: usize,
}

/// One result of a query.
#[derive(Debug, Clone)]
pub struct Hit {
    pub library: LibraryId,
    pub rel: RelPath,
    pub display_name: String,
    pub display_folder: String,
    /// The library's folder name as shown.
    pub library_name: String,
    pub rank: Rank,
    /// Highlight ranges into [`Hit::display_name`] (empty for [`Rank::Path`] and an empty
    /// query).
    pub ranges: Vec<Range<usize>>,
}

// ## m4b-t10-service fix round 2 (m-b): counts only, never a path.
impl fmt::Debug for ScriptIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptIndex")
            .field("libraries", &self.libs.len())
            .finish_non_exhaustive()
    }
}

/// The keys of one entry.
#[derive(Debug)]
struct Keys {
    name: SearchKey,
    stem: Option<SearchKey>,
    path: SearchKey,
}

#[derive(Debug)]
struct Lib {
    name: String,
    entries: Vec<IndexEntry>,
    /// Keys for `turkish_tolerant` false (0) and true (1), built on first use.
    keys: [OnceLock<Vec<Keys>>; 2],
}

/// The "Go to script" index of one snapshot. Cheap to share (`Arc` inside); querying it from
/// another thread needs no lock.
#[derive(Clone, Default)]
pub struct ScriptIndex {
    libs: Arc<[Lib]>,
}

impl ScriptIndex {
    /// The index of every offered script in `snapshot` (names and metadata only).
    pub fn build(snapshot: &WorkspaceSnapshot) -> Self {
        let libs = snapshot
            .libraries
            .iter()
            .map(|l| {
                let folder = l
                    .root
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let entries = l
                    .entries
                    .iter()
                    .filter(|e| offered(e))
                    .map(|e| IndexEntry {
                        library: l.id,
                        display_name: display_name(e.rel.name()),
                        display_folder: e
                            .rel
                            .as_str()
                            .rsplit_once('/')
                            .map(|(folder, _)| display_name(folder))
                            .unwrap_or_default(),
                        path_len: e.rel.as_str().chars().count(),
                        entry: e.clone(),
                    })
                    .collect();
                Lib {
                    name: display_name(&folder),
                    entries,
                    keys: [OnceLock::new(), OnceLock::new()],
                }
            })
            .collect();
        ScriptIndex { libs }
    }

    // ## m4b-t10-service fix round 1 (m2): counts and hits are per environment.
    /// How many scripts a connection whose environment is `env` is offered.
    pub fn len_for(&self, env: Option<EnvTag>) -> usize {
        self.libs
            .iter()
            .flat_map(|l| &l.entries)
            .filter(|e| envscope::visible(&e.entry, env))
            .count()
    }

    /// Whether a connection whose environment is `env` is offered no script (fix round 2, m-e:
    /// there is no all-environment answer).
    pub fn is_empty_for(&self, env: Option<EnvTag>) -> bool {
        self.len_for(env) == 0
    }

    /// The best `limit` hits for `q` over the scripts shown under a connection whose
    /// environment is `env` (`None`: an untagged connection, which sees only unscoped files),
    /// ranked (module docs).
    pub fn query_for(
        &self,
        q: &str,
        opts: SearchOpts,
        limit: usize,
        env: Option<EnvTag>,
    ) -> Vec<Hit> {
        self.query_where(q, opts, limit, &|e| envscope::visible(&e.entry, env))
    }

    /// [`query_for`](Self::query_for) over the scripts shown under at least one of `envs` (Task 15:
    /// the environments of the connections a file could open under).
    pub fn query_for_any(
        &self,
        q: &str,
        opts: SearchOpts,
        limit: usize,
        envs: &[Option<EnvTag>],
    ) -> Vec<Hit> {
        self.query_where(q, opts, limit, &|e| {
            envs.iter().any(|env| envscope::visible(&e.entry, *env))
        })
    }

    /// The ranked hits over the entries `keep` accepts.
    pub(crate) fn query_where(
        &self,
        q: &str,
        opts: SearchOpts,
        limit: usize,
        keep: &dyn Fn(&IndexEntry) -> bool,
    ) -> Vec<Hit> {
        // m3: a pasted page of text is not a script name; matching it whole costs seconds.
        let q = cap(q.trim());
        let matcher = Matcher::new(q, opts);
        // Candidates hold indexes only; the hits' strings are copied for the `limit` best.
        let mut found: Vec<(Sort, &IndexEntry, Vec<Range<usize>>)> = Vec::new();
        for (order, lib) in self.libs.iter().enumerate() {
            let keys = lib.keys[usize::from(opts.turkish_tolerant)]
                .get_or_init(|| lib.entries.iter().map(|e| keys_of(lib, e, opts)).collect());
            for (entry, keys) in lib.entries.iter().zip(keys) {
                if !keep(entry) {
                    continue;
                }
                let Some((rank, ranges)) = matcher.rank(keys) else {
                    continue;
                };
                let sort = Sort {
                    rank,
                    path_len: entry.path_len,
                    order,
                };
                found.push((sort, entry, ranges));
            }
        }
        let by = |(a, ea, _): &(Sort, &IndexEntry, _), (b, eb, _): &(Sort, &IndexEntry, _)| {
            a.cmp(b).then_with(|| {
                let (x, y) = (ea.entry.rel.as_str(), eb.entry.rel.as_str());
                x.as_bytes().cmp(y.as_bytes())
            })
        };
        if found.len() > limit && limit > 0 {
            found.select_nth_unstable_by(limit - 1, by);
            found.truncate(limit);
        }
        found.sort_by(by);
        found.truncate(limit);
        found
            .into_iter()
            .map(|(sort, entry, ranges)| Hit {
                library: entry.library,
                rel: entry.entry.rel.clone(),
                display_name: entry.display_name.clone(),
                display_folder: entry.display_folder.clone(),
                library_name: self.libs[sort.order].name.clone(),
                rank: sort.rank,
                ranges,
            })
            .collect()
    }
}

/// The longest query matched, in characters (fix round 1, m3).
pub const MAX_QUERY_CHARS: usize = 256;

fn cap(q: &str) -> &str {
    q.char_indices()
        .nth(MAX_QUERY_CHARS)
        .map_or(q, |(i, _)| &q[..i])
}
// ## m4b-t10-service

/// Whether a scanned entry is offered: a script file that some connection can be shown, and a
/// link that opens.
fn offered(e: &ScriptEntry) -> bool {
    let file = matches!(e.kind, EntryKind::Sql | EntryKind::Markdown);
    let opens = matches!(e.link, LinkState::Plain | LinkState::InsideLink);
    let hidden = |s: &EnvScope| matches!(s, EnvScope::Hidden(_));
    file && opens && !hidden(&e.scope) && !hidden(&envscope::entry_scope(&e.rel, e.kind))
}

fn keys_of(lib: &Lib, e: &IndexEntry, opts: SearchOpts) -> Keys {
    let stem = e
        .display_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .filter(|s| !s.is_empty())
        .map(|s| build_key(s, &opts));
    let path = if e.display_folder.is_empty() {
        lib.name.clone()
    } else {
        format!("{}/{}", lib.name, e.display_folder)
    };
    Keys {
        name: build_key(&e.display_name, &opts),
        stem,
        path: build_key(&path, &opts),
    }
}

/// The sort key of a hit (the path's bytes break the last tie).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Sort {
    rank: Rank,
    path_len: usize,
    order: usize,
}

/// One folded query.
struct Matcher<'q> {
    text: &'q str,
    opts: SearchOpts,
    name: PreparedQuery,
    /// Folders and library names match as a plain substring (no subsequence over a long path).
    path: PreparedQuery,
    key: SearchKey,
}

impl<'q> Matcher<'q> {
    fn new(text: &'q str, opts: SearchOpts) -> Self {
        let path_opts = SearchOpts {
            mode: match opts.mode {
                MatchMode::Fuzzy => MatchMode::Contains,
                other => other,
            },
            ..opts
        };
        Matcher {
            text,
            opts,
            name: PreparedQuery::new(text, &opts),
            path: PreparedQuery::new(text, &path_opts),
            key: build_key(text, &opts),
        }
    }

    /// The rank and highlight of `keys`, if the query matches them.
    fn rank(&self, keys: &Keys) -> Option<(Rank, Vec<Range<usize>>)> {
        if self.text.is_empty() {
            return Some((Rank::Path, Vec::new()));
        }
        if let Some(m) = self.name.matches(&keys.name) {
            let at_start = m.ranges.first().is_some_and(|r| r.start == 0);
            let rank = match m.kind {
                MatchKind::ExactWord | MatchKind::Prefix if at_start => {
                    if self.is_exact(keys) {
                        Rank::Exact
                    } else {
                        Rank::Prefix
                    }
                }
                MatchKind::ExactWord | MatchKind::Prefix => Rank::WordPrefix,
                MatchKind::Substring => Rank::Substring,
                MatchKind::Fuzzy => Rank::Subsequence,
            };
            return Some((rank, m.ranges));
        }
        self.path
            .matches(&keys.path)
            .map(|_| (Rank::Path, Vec::new()))
    }

    /// The folded query equals the folded name or stem: each covers the other whole.
    fn is_exact(&self, keys: &Keys) -> bool {
        let whole = |key: &SearchKey, query: &PreparedQuery| {
            query.matches(key).is_some_and(|m| {
                matches!(m.kind, MatchKind::ExactWord)
                    && m.ranges.len() == 1
                    && m.ranges[0] == (0..key.text().len())
            })
        };
        [Some(&keys.name), keys.stem.as_ref()]
            .into_iter()
            .flatten()
            .any(|target| {
                whole(target, &self.name)
                    && whole(&self.key, &PreparedQuery::new(target.text(), &self.opts))
            })
    }
}
// ## m4b-t10-service
