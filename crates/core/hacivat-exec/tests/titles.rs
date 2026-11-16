//! M3b Task 16: result tab titles from the directly preceding comment (R-M3-66): user text kept
//! as written, ` (k)` for a unit's further result sets, cut after 40 graphemes for the tab
//! (the tooltip keeps the whole title).

mod common;

use common::m3b::{request, run_all};
use common::{fake_pg, generated, pg_doc, resolve_doc, service_over, units};
use hacivat_driver::fake::FakeQuery;
use hacivat_exec::{RunAction, TITLE_GRAPHEMES, comment_title, short_title};

/// The tab titles of running `script` whole (Execute 3) as an M3b run, where each `CALL`
/// returns two result sets and each `SELECT` one.
fn titles(script: &str) -> Vec<String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime");
    let script = script.to_string();
    rt.block_on(async move {
        // Each script answers one query.
        let call = FakeQuery {
            prefix: "CALL",
            sets: vec![generated(1), generated(2)],
            ..Default::default()
        };
        let select = FakeQuery {
            prefix: "SELECT",
            sets: vec![generated(1)],
            ..Default::default()
        };
        let fake = fake_pg().queries(vec![call.clone(), call, select.clone(), select]);
        let (b, s, _f) = service_over(fake).await;
        let _ = pg_doc(&script);
        let resolved = resolve_doc(&script, common::caret(0), RunAction::Execute3);
        let us = units(&resolved).to_vec();
        let n = us.len();
        let mut t = b.svc.run(&s, request(&b, us, run_all(n)));
        (&mut t.done).await.expect("the run ends");
        let tabs = t.results.borrow().clone();
        tabs.iter().map(|r| r.title.to_string()).collect()
    })
}

#[test]
fn title_from_preceding_comment_and_suffixes() {
    assert_eq!(
        titles("-- müşteriler\nCALL p(); -- two sets"),
        ["müşteriler", "müşteriler (2)"]
    );
    // No comment: `Result N`, N counting the units that opened a tab.
    assert_eq!(
        titles("SELECT 1;\n/* İKİNCİ */\nCALL p();\nSELECT 2;"),
        ["Result 1", "İKİNCİ", "İKİNCİ (2)", "Result 3"]
    );
}

#[test]
fn title_truncates_at_40_graphemes() {
    // 45 graphemes, Turkish letters with combining marks among them: never case-mapped.
    let long = format!("{}{}", "ı\u{307}ş".repeat(20), "ÇĞÖÜi");
    assert_eq!(
        comment_title(&format!("-- {long}")).as_deref(),
        Some(&*long)
    );
    let shown = short_title(&long);
    let expected = format!("{}…", "ı\u{307}ş".repeat(20));
    assert_eq!(&*shown, expected.as_str(), "40 graphemes and an ellipsis");
    assert_eq!(TITLE_GRAPHEMES, 40);
    // Exactly 40 graphemes stay whole.
    let forty = "a".repeat(40);
    assert_eq!(&*short_title(&forty), forty.as_str());
    // A flag and a ZWJ sequence are one grapheme each.
    let flags = format!("{}{}", "🇹🇷".repeat(39), "👨\u{200D}👩\u{200D}👧");
    assert_eq!(&*short_title(&flags), flags.as_str());
    let over = format!("{flags}x");
    assert_eq!(&*short_title(&over), format!("{flags}…").as_str());
}

#[test]
fn a_comment_title_is_user_text() {
    for (comment, title) in [
        ("-- İstanbul siparişleri", "İstanbul siparişleri"),
        ("# MySQL stili", "MySQL stili"),
        ("/* blok başlığı */", "blok başlığı"),
        ("-- eski\n-- yeni", "yeni"),
    ] {
        assert_eq!(
            comment_title(comment).as_deref(),
            Some(title),
            "{comment:?}"
        );
    }
    assert_eq!(comment_title("--   "), None);
}
