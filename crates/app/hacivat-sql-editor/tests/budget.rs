//! Budgets of the language service on the 50k-line fixture (R-M3-9 as ruled: M3a's keystroke
//! budget for the service is ≤ 0.5 ms, because nothing is lexed on the keystroke frame; the
//! preview and the decorations of one viewport stay within the resolve budget of 2 ms and the
//! map budget of 1 ms). Perf tests: `#[ignore = "perf"]`, gated on `HACIVAT_PERF=1`, run by the
//! gate's perf line in release.

mod support;

use std::time::Instant;

use gpui::TestAppContext;
use hacivat_ui_editor::{ExecAction, LanguageService};
use support::{DOC, Doc, caret, pg};

/// `hacivat_testkit::perf_enabled()`, inlined: an app crate's tests reach no core test kit.
fn perf_enabled() -> bool {
    std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1")
}

/// 50,000 lines: 12,500 four-line statements.
fn fixture() -> String {
    let mut text = String::with_capacity(2 << 20);
    for i in 0..12_500 {
        text.push_str(&format!(
            "SELECT id, name, 'müşteri {i}' AS label\n  FROM orders o\n  WHERE o.total > {i}\n  ORDER BY id;\n"
        ));
    }
    text
}

fn p95(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() * 95 / 100]
}

#[gpui::test]
#[ignore = "perf"]
fn change_p95_under_half_a_millisecond_on_50k_lines(cx: &mut TestAppContext) {
    if !perf_enabled() {
        return;
    }
    let mut d = Doc::analysed(cx, &fixture(), pg());
    let mut times = Vec::with_capacity(1_000);
    for i in 0..1_000 {
        let at = (i * 1_997) % d.model.buffer().len_bytes();
        let at = d.model.buffer().snap(at, hacivat_text::Bias::Before);
        let tr = hacivat_text::Transaction {
            changes: hacivat_text::ChangeSet::from_edits(
                d.model.buffer().len_bytes(),
                [(at..at, std::sync::Arc::from("x"))],
            )
            .unwrap(),
            selection: None,
            ann: hacivat_text::Annotations {
                user_event: hacivat_text::UserEvent::InputType,
                add_to_history: false,
                time: Instant::now(),
                origin: hacivat_text::Origin::User,
            },
        };
        let committed = d.model.dispatch(tr).unwrap();
        let snap = d.model.snapshot();
        let svc = d.svc.clone();
        times.push(cx.update(|cx| {
            let t = Instant::now();
            svc.change(DOC, &snap, &committed.desc, cx);
            t.elapsed().as_secs_f64() * 1e3
        }));
        if i % 50 == 49 {
            cx.run_until_parked();
        }
    }
    let p = p95(times);
    println!("change(): p95 {p:.4} ms");
    assert!(p < 0.5, "change() p95 {p:.4} ms");
}

#[gpui::test]
#[ignore = "perf"]
fn preview_p95_within_the_resolve_budget_on_50k_lines(cx: &mut TestAppContext) {
    if !perf_enabled() {
        return;
    }
    let text = fixture();
    let d = Doc::analysed(cx, &text, pg());
    let middle = text.len() / 2;
    for (name, action) in [
        ("Run (ask)", ExecAction::Execute1),
        ("Run 2", ExecAction::Execute2),
        ("Run 3", ExecAction::Execute3),
    ] {
        let mut times = Vec::with_capacity(200);
        for i in 0..200 {
            let at = caret(middle + i * 7);
            let t = Instant::now();
            let p = d.svc.exec_preview(DOC, d.version(), &at, action);
            times.push(t.elapsed().as_secs_f64() * 1e3);
            assert!(p.is_some());
        }
        let p = p95(times);
        println!("exec_preview {name}: p95 {p:.3} ms");
        assert!(p < 2.0, "exec_preview {name} p95 {p:.3} ms");
    }
}

/// One 60-line viewport's decorations, at the analysed version and rebased over three edits.
#[gpui::test]
#[ignore = "perf"]
fn viewport_decorations_p95_under_a_millisecond(cx: &mut TestAppContext) {
    if !perf_enabled() {
        return;
    }
    let text = fixture();
    let mut d = Doc::analysed(cx, &text, pg());
    let start = d.model.buffer().line_to_byte(25_000);
    let end = d.model.buffer().line_to_byte(25_060);
    let measure = |d: &Doc, shift: usize| {
        let sources = d.svc.decoration_sources(DOC);
        let snap = d.snap();
        let mut times = Vec::with_capacity(200);
        for i in 0..200 {
            // A new range each time, so no cached answer is measured.
            let r = start + shift + i..end + shift + i;
            let t = Instant::now();
            let set = sources[0].decorations(&snap, d.model.changelog(), r);
            times.push(t.elapsed().as_secs_f64() * 1e3);
            assert!(!set.is_empty());
        }
        p95(times)
    };
    let exact = measure(&d, 0);
    for _ in 0..3 {
        d.edit(cx, start - 10..start - 10, "x");
    }
    let rebased = measure(&d, 3);
    println!("decorations: exact p95 {exact:.3} ms, rebased over 3 edits p95 {rebased:.3} ms");
    assert!(exact < 1.0 && rebased < 1.0);
}
