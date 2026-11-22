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
