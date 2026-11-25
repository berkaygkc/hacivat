//! HV-SCRIPT-09: the "Go to script" index over names only (Task 10). Ranking, the I18N-09
//! search key (D-074 Turkish tolerance), environment scoping (R-M4b-6) and escaping
//! (R-M4b-23).

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use hacivat_l10n::search::{MatchMode, SearchOpts};
use hacivat_proto::EnvTag;
use hacivat_scripts::envscope::entry_scope;
use hacivat_scripts::index::{Hit, Rank, ScriptIndex};
use hacivat_scripts::testing::logical;
use hacivat_scripts::{
    EntryKind, LibraryId, LibraryRoot, LibrarySnapshot, LibraryState, LinkState, RelPath,
    ScriptEntry, WorkspaceSnapshot,
};

fn opts_en() -> SearchOpts {
    SearchOpts {
        turkish_tolerant: false,
        mode: MatchMode::Fuzzy,
    }
}

fn opts_tr() -> SearchOpts {
    SearchOpts {
        turkish_tolerant: true,
        mode: MatchMode::Fuzzy,
    }
}

fn opts_en_strict() -> SearchOpts {
    SearchOpts {
        turkish_tolerant: false,
        mode: MatchMode::Contains,
    }
}

fn entry(rel: &str, link: LinkState) -> ScriptEntry {
    let rel = RelPath::parse(rel).unwrap();
    let kind = if rel.name().ends_with(".md") {
        EntryKind::Markdown
    } else if rel.name().ends_with(".sql") {
        EntryKind::Sql
    } else {
        EntryKind::Folder
    };
    ScriptEntry {
        scope: entry_scope(&rel, kind),
        rel,
        kind,
        link,
        dataless: false,
        len: 0,
        mtime: SystemTime::UNIX_EPOCH,
        target: None,
    }
}

fn library(folder: &str, entries: Vec<ScriptEntry>) -> Arc<LibrarySnapshot> {
    let id = LibraryId::new();
    // A root that does not exist: the index never touches the disk (`index_reads_no_contents`).
    let path = std::env::temp_dir().join("hv-t10-no-such-dir").join(folder);
    Arc::new(LibrarySnapshot::new(
        LibraryRoot {
            id,
            path: path.clone(),
            canonical: path,
        },
        logical(folder),
        Vec::new(),
        entries,
        LibraryState::Watching,
    ))
}

fn snapshot(libs: Vec<Arc<LibrarySnapshot>>) -> WorkspaceSnapshot {
    WorkspaceSnapshot::new(1, libs)
}

/// Every environment a connection can have, untagged included.
fn every_env() -> Vec<Option<EnvTag>> {
    std::iter::once(None)
        .chain(EnvTag::ALL.into_iter().map(Some))
        .collect()
}

/// The query over every file some connection is shown (fix round 1, m2: there is no
/// unscoped query).
fn query(i: &ScriptIndex, q: &str, o: SearchOpts, limit: usize) -> Vec<Hit> {
    i.query_for_any(q, o, limit, &every_env())
}

fn idx(paths: &[&str]) -> ScriptIndex {
    let entries = paths.iter().map(|p| entry(p, LinkState::Plain)).collect();
    ScriptIndex::build(&snapshot(vec![library("orders", entries)]))
}

fn names(hits: &[Hit]) -> Vec<String> {
    hits.iter().map(|h| h.display_name.clone()).collect()
}

// --- plan Task 10 ----------------------------------------------------------------------------

#[test]
fn index_ranks_prefix_first() {
    let i = idx(&["reports/monthly.sql", "monthly_old.sql", "x/my_month.sql"]);
    assert_eq!(
        names(&query(&i, "month", opts_en(), 10)),
        ["monthly_old.sql", "monthly.sql", "my_month.sql"]
    );
}

#[test]
fn index_is_turkish_tolerant_for_tr() {
    let i = idx(&["müşteri_raporu.sql"]);
    assert_eq!(query(&i, "musteri", opts_tr(), 10).len(), 1);
    assert_eq!(query(&i, "musteri", opts_en_strict(), 10).len(), 0); // D-074
}

#[test]
#[ignore = "perf"]
/// A release budget (`HACIVAT_PERF=1 cargo test --release -p hacivat-scripts --test index --
/// --ignored`), as the other perf tests.
fn query_over_10k_entries_under_5ms() {
    if !std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1") {
        return;
    }
    let paths: Vec<String> = (0..10_000)
        .map(|n| format!("folder_{}/report_{n}_müşteri.sql", n % 97))
        .collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let i = idx(&refs);
    // The keys are built once per tolerance, on the first query.
    let _ = query(&i, "warm", opts_tr(), 50);
    let mut best = u128::MAX;
    for _ in 0..5 {
        let t = Instant::now();
        let hits = query(&i, "rep42", opts_tr(), 50);
        best = best.min(t.elapsed().as_micros());
        assert!(!hits.is_empty());
    }
    println!("HV-FACT t10 query_10k_us={best}");
    assert!(best < 5_000, "best {best} µs");
}

// --- alg/28 HV-SCRIPT-09 ----------------------------------------------------------------------

/// alg/28 lists the expected order by kind: prefix, word-prefix, substring, subsequence.
#[test]
fn goto_ranking_table() {
    let i = idx(&["o_r_d.sql", "reorder.sql", "big_orders.sql", "orders.sql"]);
    let hits = query(&i, "ord", opts_en(), 10);
    assert_eq!(
        names(&hits),
        ["orders.sql", "big_orders.sql", "reorder.sql", "o_r_d.sql"]
    );
    let ranks: Vec<Rank> = hits.iter().map(|h| h.rank).collect();
    assert_eq!(
        ranks,
        [
            Rank::Prefix,
            Rank::WordPrefix,
            Rank::Substring,
            Rank::Subsequence
        ]
    );
}

#[test]
fn index_reads_no_contents() {
    // alg/28 names "a FakeFs that fails any read". The index has no file-system seam at all:
    // `index.rs` imports no I/O (the tripwire in `tests/service.rs` checks it), and it takes
    // only a snapshot. So the roots here are paths that do not exist; building and querying
    // the index touches no file system, so it can read no contents (R-M4b-15; review m10).
    let i = idx(&["a.sql", "notes/b.md"]);
    assert_eq!(i.len_for(None), 2);
    assert_eq!(names(&query(&i, "b", opts_en(), 10)), ["b.md"]);
}

// --- the brief: names, escaping, scoping -----------------------------------------------------

#[test]
fn musteri_finds_musteri_segment_also_when_the_name_is_nfd() {
    // NFC and the NFD spelling macOS hands out.
    let i = idx(&[
        "müşteri_segment.sql",
        "eski/mu\u{308}s\u{327}teri_segment.sql",
    ]);
    let hits = query(&i, "musteri", opts_tr(), 10);
    assert_eq!(hits.len(), 2);
    // The display form is NFC either way (R-M4b-23); the identity keeps the OS bytes.
    assert!(hits.iter().all(|h| h.display_name == "müşteri_segment.sql"));
    assert!(
        hits.iter()
            .any(|h| h.rel.as_str() == "eski/mu\u{308}s\u{327}teri_segment.sql")
    );
    assert_eq!(
        hits[0].rel.as_str(),
        "müşteri_segment.sql",
        "shorter path first"
    );
    assert!(query(&i, "musteri", opts_en_strict(), 10).is_empty());
}

#[test]
fn exact_name_outranks_prefix() {
    let i = idx(&["orders_old.sql", "deep/folder/orders.sql"]);
    let hits = query(&i, "orders", opts_en(), 10);
    assert_eq!(names(&hits), ["orders.sql", "orders_old.sql"]);
    assert_eq!(hits[0].rank, Rank::Exact);
    assert_eq!(hits[1].rank, Rank::Prefix);
    // Touching repeats are not an exact name.
    let i = idx(&["ordOrd.sql"]);
    assert_ne!(query(&i, "ord", opts_en(), 10)[0].rank, Rank::Exact);
}

#[test]
fn folder_path_match_ranks_last_and_limit_applies() {
    let i = idx(&["reports/x.sql", "reports_2026.sql", "y.sql"]);
    let hits = query(&i, "reports", opts_en(), 10);
    assert_eq!(names(&hits), ["reports_2026.sql", "x.sql"]);
    assert_eq!(hits[1].rank, Rank::Path);
    assert_eq!(hits[1].display_folder, "reports");
    assert!(hits[1].ranges.is_empty());
    assert_eq!(query(&i, "reports", opts_en(), 1).len(), 1);
    // An empty query lists every file, shortest path first.
    assert_eq!(
        names(&query(&i, "  ", opts_en(), 10)),
        ["y.sql", "x.sql", "reports_2026.sql"]
    );
}

#[test]
fn highlight_ranges_index_the_display_name() {
    let i = idx(&["big_orders.sql"]);
    let h = &query(&i, "ord", opts_en(), 10)[0];
    assert_eq!(&h.display_name[h.ranges[0].clone()], "ord");
}

#[test]
fn index_never_lists_hidden_or_out_of_scope_files() {
    let entries = vec![
        entry("a.sql", LinkState::Plain),
        entry("@production", LinkState::Plain),
        entry("@production/p.sql", LinkState::Plain),
        entry("@staging/s.sql", LinkState::Plain),
        // Hidden everywhere: an unknown tag and a contradiction.
        entry("@prod/x.sql", LinkState::Plain),
        entry("@production/@staging/c.sql", LinkState::Plain),
        // Disabled links never open, so they are never offered.
        entry("out.sql", LinkState::OutsideLink),
        entry("loop.sql", LinkState::Loop),
    ];
    let i = ScriptIndex::build(&snapshot(vec![library("orders", entries)]));
    let all = |hits: Vec<Hit>| {
        let mut v: Vec<String> = hits.iter().map(|h| h.rel.as_str().to_owned()).collect();
        v.sort();
        v
    };
    // Folders are not offered either; `@prod` and the contradiction never are.
    assert_eq!(
        all(query(&i, "", opts_en(), 100)),
        ["@production/p.sql", "@staging/s.sql", "a.sql"]
    );
    assert_eq!(
        all(i.query_for("", opts_en(), 100, Some(EnvTag::Staging))),
        ["@staging/s.sql", "a.sql"]
    );
    assert_eq!(
        all(i.query_for("p", opts_en(), 100, Some(EnvTag::Staging))),
        Vec::<String>::new()
    );
    assert_eq!(
        all(i.query_for("", opts_en(), 100, Some(EnvTag::Production))),
        ["@production/p.sql", "a.sql"]
    );
    // An untagged connection sees only unscoped files.
    assert_eq!(all(i.query_for("", opts_en(), 100, None)), ["a.sql"]);
    assert!(
        query(&i, "x", opts_en(), 100)
            .iter()
            .all(|h| h.rel.as_str() != "@prod/x.sql")
    );
}

#[test]
fn a_stored_scope_narrower_than_the_path_hides_the_file() {
    // The scan narrows a file link to its target's scope; the index honours the stored one.
    let mut link = entry("a.sql", LinkState::InsideLink);
    link.target = Some(RelPath::parse("@staging/t.sql").unwrap());
    link.scope = entry_scope(&RelPath::parse("@staging/t.sql").unwrap(), EntryKind::Sql);
    let i = ScriptIndex::build(&snapshot(vec![library("orders", vec![link])]));
    assert_eq!(
        i.query_for("a", opts_en(), 10, Some(EnvTag::Staging)).len(),
        1
    );
    assert!(
        i.query_for("a", opts_en(), 10, Some(EnvTag::Production))
            .is_empty()
    );
    assert!(i.query_for("a", opts_en(), 10, None).is_empty());
}

#[test]
fn display_names_escape_invisible_and_bidi_characters() {
    let i = idx(&["a\u{202E}lqs.sql", "zero\u{200B}width.sql"]);
    let hits = query(&i, "", opts_en(), 10);
    let shown = names(&hits);
    assert!(shown.contains(&"a<U+202E>lqs.sql".to_owned()), "{shown:?}");
    assert!(
        shown.contains(&"zero<U+200B>width.sql".to_owned()),
        "{shown:?}"
    );
    // The library name is escaped the same way.
    let lib = library("lib\u{202E}x", vec![entry("a.sql", LinkState::Plain)]);
    let i = ScriptIndex::build(&snapshot(vec![lib]));
    assert_eq!(
        query(&i, "a", opts_en(), 10)[0].library_name,
        "lib<U+202E>x"
    );
}

#[test]
fn ties_follow_library_order() {
    let a = library("first", vec![entry("same.sql", LinkState::Plain)]);
    let b = library("second", vec![entry("same.sql", LinkState::Plain)]);
    let (ida, idb) = (a.id, b.id);
    let i = ScriptIndex::build(&snapshot(vec![a, b]));
    let hits = query(&i, "same", opts_en(), 10);
    assert_eq!(
        hits.iter().map(|h| h.library).collect::<Vec<_>>(),
        [ida, idb]
    );
}

// --- fix round 1 ------------------------------------------------------------------------------

/// m3: a pasted page of text is cut to `MAX_QUERY_CHARS` (H5 measured 2 s at 64 KiB).
#[test]
fn a_huge_query_is_cut_to_the_cap() {
    let i = idx(&["report_müşteri.sql"]);
    let huge: String = "rprt".chars().cycle().take(65_536).collect();
    let capped: String = huge
        .chars()
        .take(hacivat_scripts::index::MAX_QUERY_CHARS)
        .collect();
    assert_eq!(
        names(&query(&i, &huge, opts_tr(), 10)),
        names(&query(&i, &capped, opts_tr(), 10))
    );
    assert_eq!(
        names(&query(&i, "rprt", opts_tr(), 10)),
        ["report_müşteri.sql"]
    );
}

/// The reviewer's H5, kept as a measurement: the capped cost of a huge query.
#[test]
fn h5_huge_query_cost() {
    let paths: Vec<String> = (0..2_000)
        .map(|n| format!("f{}/report_{n}_müşteri.sql", n % 31))
        .collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let i = idx(&refs);
    let _ = query(&i, "warm", opts_tr(), 50);
    for len in [64usize, 4_096, 65_536] {
        let q: String = "rprt".chars().cycle().take(len).collect();
        let t = Instant::now();
        let hits = query(&i, &q, opts_tr(), 50);
        println!(
            "HV-FACT t10 huge_query len={len} us={} hits={}",
            t.elapsed().as_micros(),
            hits.len()
        );
    }
}

/// m2: counts are per environment.
#[test]
fn counts_are_per_environment() {
    let i = idx(&[
        "a.sql",
        "@production/p.sql",
        "@staging/s.sql",
        "@prod/x.sql",
    ]);
    assert_eq!(i.len_for(None), 1);
    assert_eq!(i.len_for(Some(EnvTag::Staging)), 2);
    assert_eq!(i.len_for(Some(EnvTag::Production)), 2);
    let staging = i.query_for_any("", opts_en(), 10, &[None, Some(EnvTag::Staging)]);
    assert_eq!(staging.len(), 2);
    assert!(
        staging
            .iter()
            .all(|h| !h.rel.as_str().starts_with("@production"))
    );
}

// ## m4b-t15-goto (named exception: Task 15's per-library scope)
/// Go to script offers each library's files under the environments of the connections that
/// library is shown under, and only unscoped files for a library no connection shows.
#[test]
fn scoped_query_filters_each_library_by_its_own_connections() {
    let staging_lib = library(
        "orders",
        vec![
            entry("a.sql", LinkState::Plain),
            entry("@production/p.sql", LinkState::Plain),
            entry("@staging/s.sql", LinkState::Plain),
        ],
    );
    let prod_lib = library(
        "billing",
        vec![
            entry("b.sql", LinkState::Plain),
            entry("@production/q.sql", LinkState::Plain),
        ],
    );
    let unbound_lib = library(
        "misc",
        vec![
            entry("m.sql", LinkState::Plain),
            entry("@production/r.sql", LinkState::Plain),
        ],
    );
    let (s, p) = (staging_lib.id, prod_lib.id);
    let i = ScriptIndex::build(&snapshot(vec![staging_lib, prod_lib, unbound_lib]));
    let mut scopes = std::collections::HashMap::new();
    scopes.insert(s, vec![Some(EnvTag::Staging)]);
    scopes.insert(p, vec![Some(EnvTag::Production)]);
    let mut got: Vec<String> = i
        .query_scoped("", opts_en(), 100, &scopes)
        .iter()
        .map(|h| format!("{}:{}", h.library_name, h.rel.as_str()))
        .collect();
    got.sort();
    // `@production/p.sql` is in a library only staging shows: never offered, although another
    // library is shown under production; the unbound library offers its unscoped file only.
    assert_eq!(
        got,
        [
            "billing:@production/q.sql",
            "billing:b.sql",
            "misc:m.sql",
            "orders:@staging/s.sql",
            "orders:a.sql",
        ]
    );
    // An empty environment list is the unbound view too.
    scopes.insert(p, Vec::new());
    let billing: Vec<String> = i
        .query_scoped("", opts_en(), 100, &scopes)
        .iter()
        .filter(|h| h.library_name == "billing")
        .map(|h| h.rel.as_str().to_owned())
        .collect();
    assert_eq!(billing, ["b.sql"]);
}
// ## m4b-t15-goto

// ## m4b-gatefix-1 (E12 Go to script)

/// The ranking as `ScriptIndex` did it before gatefix-1 (`query_where` and its `Matcher`,
/// copied, over the public search API and `envscope::visible`), kept as the reference the
/// faster index must equal: same hits, order, ranks and highlights.
mod reference {
    use std::ops::Range;

    use hacivat_l10n::search::{
        MatchKind, MatchMode, PreparedQuery, SearchKey, SearchOpts, build_key,
    };
    use hacivat_proto::EnvTag;
    use hacivat_scripts::envscope;
    use hacivat_scripts::index::Rank;
    use hacivat_scripts::pathsafe::display_name;
    use hacivat_scripts::{EntryKind, LibraryId, LinkState, ScriptEntry};

    /// A library as the reference sees it: its id, its root and its scanned entries.
    pub type Lib = (LibraryId, std::path::PathBuf, Vec<ScriptEntry>);

    /// `(library, rel, display_name, display_folder, library_name, rank, ranges)`.
    pub type Row = (
        LibraryId,
        String,
        String,
        String,
        String,
        Rank,
        Vec<Range<usize>>,
    );

    fn offered(e: &ScriptEntry) -> bool {
        let file = matches!(e.kind, EntryKind::Sql | EntryKind::Markdown);
        let opens = matches!(e.link, LinkState::Plain | LinkState::InsideLink);
        let hidden = |s: &envscope::EnvScope| matches!(s, envscope::EnvScope::Hidden(_));
        file && opens && !hidden(&e.scope) && !hidden(&envscope::entry_scope(&e.rel, e.kind))
    }

    struct Keys {
        name: SearchKey,
        stem: Option<SearchKey>,
        path: SearchKey,
    }

    fn cap(q: &str) -> &str {
        q.char_indices().nth(256).map_or(q, |(i, _)| &q[..i])
    }

    fn is_exact(keys: &Keys, name_q: &PreparedQuery, key_q: &SearchKey, opts: SearchOpts) -> bool {
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
            .any(|t| whole(t, name_q) && whole(key_q, &PreparedQuery::new(t.text(), &opts)))
    }

    pub fn query(
        libs: &[Lib],
        q: &str,
        opts: SearchOpts,
        limit: usize,
        envs: &[Option<EnvTag>],
    ) -> Vec<Row> {
        let q = cap(q.trim());
        let path_opts = SearchOpts {
            mode: match opts.mode {
                MatchMode::Fuzzy => MatchMode::Contains,
                other => other,
            },
            ..opts
        };
        let name_q = PreparedQuery::new(q, &opts);
        let path_q = PreparedQuery::new(q, &path_opts);
        let key_q = build_key(q, &opts);
        let rank = |keys: &Keys| -> Option<(Rank, Vec<Range<usize>>)> {
            if q.is_empty() {
                return Some((Rank::Path, Vec::new()));
            }
            if let Some(m) = name_q.matches(&keys.name) {
                let at_start = m.ranges.first().is_some_and(|r| r.start == 0);
                let rank = match m.kind {
                    MatchKind::ExactWord | MatchKind::Prefix if at_start => {
                        if is_exact(keys, &name_q, &key_q, opts) {
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
            path_q.matches(&keys.path).map(|_| (Rank::Path, Vec::new()))
        };
        let mut found: Vec<((Rank, usize, usize), Row)> = Vec::new();
        for (order, (id, root, entries)) in libs.iter().enumerate() {
            let folder = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let lib_name = display_name(&folder);
            for e in entries.iter().filter(|e| offered(e)) {
                if !envs.iter().any(|env| envscope::visible(e, *env)) {
                    continue;
                }
                let name = display_name(e.rel.name());
                let dir = e
                    .rel
                    .as_str()
                    .rsplit_once('/')
                    .map(|(f, _)| display_name(f))
                    .unwrap_or_default();
                let path = if dir.is_empty() {
                    lib_name.clone()
                } else {
                    format!("{lib_name}/{dir}")
                };
                let keys = Keys {
                    stem: name
                        .rsplit_once('.')
                        .map(|(s, _)| s)
                        .filter(|s| !s.is_empty())
                        .map(|s| build_key(s, &opts)),
                    name: build_key(&name, &opts),
                    path: build_key(&path, &opts),
                };
                let Some((rank, ranges)) = rank(&keys) else {
                    continue;
                };
                let sort = (rank, e.rel.as_str().chars().count(), order);
                let row = (
                    *id,
                    e.rel.as_str().to_owned(),
                    name,
                    dir,
                    lib_name.clone(),
                    rank,
                    ranges,
                );
                found.push((sort, row));
            }
        }
        found.sort_by(|(a, ra), (b, rb)| {
            a.cmp(b)
                .then_with(|| ra.1.as_bytes().cmp(rb.1.as_bytes()))
        });
        found.truncate(limit);
        found.into_iter().map(|(_, row)| row).collect()
    }
}

fn rows(hits: &[Hit]) -> Vec<reference::Row> {
    hits.iter()
        .map(|h| {
            (
                h.library,
                h.rel.as_str().to_owned(),
                h.display_name.clone(),
                h.display_folder.clone(),
                h.library_name.clone(),
                h.rank,
                h.ranges.clone(),
            )
        })
        .collect()
}

/// A generated corpus: three libraries, environment folders (known, unknown and
/// contradictory), Turkish and accented names, an invisible character, links that do not
/// open, another extension and many ties on length.
fn generated_corpus() -> (WorkspaceSnapshot, Vec<reference::Lib>) {
    let folders = [
        "",
        "reports",
        "@production",
        "@staging/müşteri",
        "@production/@staging",
        "@nowhere",
        "İstanbul/q4",
        "a_b",
    ];
    let stems = [
        "report_42",
        "rep_4_2",
        "müşteri_segment",
        "MUSTERI",
        "orders",
        "big_orders",
        "o_r_d",
        "reorder",
        "ISTANBUL_ıi",
        "straße",
        "x\u{200B}y",
        "e10",
        "events_2026_10",
        "report",
    ];
    let exts = [".sql", ".md", ".txt"];
    let (mut libs, mut plain) = (Vec::new(), Vec::new());
    for (l, lib) in ["orders", "müşteri_ops", "Reports"].into_iter().enumerate() {
        let mut entries = Vec::new();
        for (f, folder) in folders.iter().enumerate() {
            for (s, stem) in stems.iter().enumerate() {
                let ext = exts[(l + f + s) % exts.len()];
                let rel = if folder.is_empty() {
                    format!("{stem}{ext}")
                } else {
                    format!("{folder}/{stem}{ext}")
                };
                let link = match (l * 7 + f * 3 + s) % 11 {
                    0 => LinkState::InsideLink,
                    1 => LinkState::OutsideLink,
                    _ => LinkState::Plain,
                };
                entries.push(entry(&rel, link));
            }
        }
        let snap = library(lib, entries.clone());
        plain.push((snap.id, snap.root.path.clone(), entries));
        libs.push(snap);
    }
    (snapshot(libs), plain)
}

#[test]
fn ranking_equals_the_pre_gatefix_reference_on_a_generated_corpus() {
    let (snap, libs) = generated_corpus();
    let i = ScriptIndex::build(&snap);
    let queries = [
        "",
        " ",
        "rep42",
        "report",
        "report_42",
        "report_42.sql",
        "musteri",
        "MÜŞTERİ",
        "müş",
        "istanbul",
        "ISTANBUL",
        "ı",
        "i",
        "ord",
        "orders",
        "e10",
        "strasse",
        "xy",
        "q4",
        "@prod",
        "reports/",
        "zzz",
        "r",
        "s",
        "md",
        ".sql",
        "o_r",
        "big ord",
    ];
    let envs: Vec<Vec<Option<EnvTag>>> = std::iter::once(every_env())
        .chain(every_env().into_iter().map(|e| vec![e]))
        .collect();
    let (mut compared, mut nonempty) = (0, 0);
    for tolerant in [false, true] {
        for mode in [MatchMode::Contains, MatchMode::WordPrefix, MatchMode::Fuzzy] {
            let o = SearchOpts {
                turkish_tolerant: tolerant,
                mode,
            };
            for q in queries {
                for env in &envs {
                    // The reference sorts everything, so each limit is a prefix of the whole.
                    let all = reference::query(&libs, q, o, usize::MAX, env);
                    for limit in [0, 1, 3, 50, 10_000] {
                        let want = all[..limit.min(all.len())].to_vec();
                        let got = rows(&i.query_for_any(q, o, limit, env));
                        assert_eq!(got, want, "{q:?} {o:?} limit {limit} envs {env:?}");
                        if let [one] = env.as_slice() {
                            let got = rows(&i.query_for(q, o, limit, *one));
                            assert_eq!(got, want, "{q:?} {o:?} limit {limit} env {one:?}");
                        }
                        compared += 1;
                        nonempty += usize::from(!want.is_empty());
                    }
                }
            }
        }
    }
    assert!(compared > 5_000 && nonempty > 2_000, "{compared} {nonempty}");
}

/// Go to script on 10,000 entries for query shapes beyond `rep42`: one every entry matches as
/// a prefix (the most hits), a Turkish-tolerant one, a single letter, a folder-only one, a
/// miss and the empty query (best of five each, after a warm-up).
#[test]
#[ignore = "perf"]
fn query_shapes_over_10k_entries_under_5ms() {
    if !std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1") {
        return;
    }
    let paths: Vec<String> = (0..10_000)
        .map(|n| format!("folder_{}/report_{n}_müşteri.sql", n % 97))
        .collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let i = idx(&refs);
    let _ = query(&i, "warm", opts_tr(), 50);
    let mut worst = 0;
    for q in ["rep42", "report", "musteri", "r", "folder_42", "zzz", ""] {
        let mut best = u128::MAX;
        for _ in 0..5 {
            let t = Instant::now();
            let _ = query(&i, q, opts_tr(), 50);
            best = best.min(t.elapsed().as_micros());
        }
        println!("HV-FACT gatefix1 query_10k q={q:?} us={best}");
        worst = worst.max(best);
    }
    assert!(worst < 5_000, "worst {worst} µs");
}
// ## m4b-gatefix-1
