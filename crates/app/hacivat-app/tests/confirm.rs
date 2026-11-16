//! M3b Task 21: the gate's confirmation dialog and the confirm-in-transaction prompt
//! (`design/m3b-safe` surfaces 1 and 2; R-M3-55, R-M3-84, D-014), and the app's `ConfirmPort`.
//!
//! Every dialog is driven through a real shell with the keyboard and the pointer. Answers are
//! read from the dialog's own `oneshot`; every wait is a condition (`run_until_parked`,
//! `wait_until`), and the countdown is driven by the executor's clock, never by sleeping.

mod support;

use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, EntityInputHandler as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::prompt_kit::parts;
use hacivat_app::exec_ui::AppConfirmPort;
use hacivat_app::exec_ui::commit_prompt::{self, CommitContext, countdown_of, open_commit_views};
use hacivat_app::exec_ui::confirm::{
    self, ConfirmContext, PlanAnswers, PlanFlow, answer_plan, ask_plan, confirm_open,
    execute_all_covers, name_matches, offers_suppress, open_dialog_buttons, question_for,
};
use hacivat_exec::{
    CommitAnswer, CommitQuestion, ConfirmAnswer, ConfirmPort as _, ConfirmQuestion, GateClass,
    GateDecision, GateItem, GatePlan, GateReason,
};
use hacivat_proto::EnvTag;
use hacivat_session::session::{RunEvent, RunId, SessionEffects, UnitId, UnitVerdict};
use hacivat_ui::button::ButtonKind;
use hacivat_ui::input::TextInput;
use tokio::sync::{oneshot, watch};

use support::*;

// ---- fixtures and helpers -------------------------------------------------------------------

/// The table whose name a test types.
const NAME: &str = "müşteri";

fn reason(id: &'static str, args: &[(&'static str, &str)]) -> GateReason {
    GateReason {
        fluent_id: id,
        args: args.iter().map(|(k, v)| (*k, Arc::from(*v))).collect(),
    }
}

fn question(production: bool) -> ConfirmQuestion {
    ConfirmQuestion {
        run: RunId(1),
        unit: 0,
        index: 1,
        total: 1,
        sql_masked: Arc::from("DROP TABLE order_archive;"),
        reasons: vec![reason(
            "safety-destructive-drop",
            &[("table", "order_archive")],
        )],
        type_to_confirm: None,
        irreversible: false,
        suppressible: false,
        warn_only: false,
        production,
        connection: Arc::from(if production {
            "orders-prod"
        } else {
            "orders-staging"
        }),
    }
}

/// A production `DELETE` with no `WHERE` on [`NAME`]: the name must be typed.
fn typed_question() -> ConfirmQuestion {
    ConfirmQuestion {
        sql_masked: Arc::from("DELETE FROM müşteri;"),
        reasons: vec![reason("safety-destructive-no-where", &[("table", NAME)])],
        type_to_confirm: Some(Arc::from(NAME)),
        ..question(true)
    }
}

fn ctx(class: GateClass) -> ConfirmContext {
    ConfirmContext {
        class: Some(class),
        ..ConfirmContext::default()
    }
}

/// A shell whose window is the active one (a test may open several; dialogs open in the active
/// workspace window).
fn fresh(cx: &mut TestAppContext) -> &'static mut VisualTestContext {
    let vcx = shell(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    vcx.run_until_parked();
    vcx
}

/// An answer that may still come.
struct Asked<T> {
    rx: RefCell<oneshot::Receiver<T>>,
    got: RefCell<Option<T>>,
}

impl<T: Copy> Asked<T> {
    fn new(rx: oneshot::Receiver<T>) -> Self {
        Asked {
            rx: RefCell::new(rx),
            got: RefCell::new(None),
        }
    }

    /// The answer, if one came; panics when the asker's channel closed without one.
    fn get(&self) -> Option<T> {
        if let Some(a) = *self.got.borrow() {
            return Some(a);
        }
        match self.rx.borrow_mut().try_recv() {
            Ok(a) => {
                *self.got.borrow_mut() = Some(a);
                Some(a)
            }
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => panic!("the dialog dropped its reply"),
        }
    }
}

fn ask(vcx: &VisualTestContext, q: ConfirmQuestion, c: ConfirmContext) -> Asked<ConfirmAnswer> {
    let rx = vcx.cx.update(|cx| confirm::open_in(q, c, cx));
    vcx.run_until_parked();
    assert!(
        layer_ids(vcx).contains(&confirm::CONFIRM_LAYER_ID),
        "the dialog opened: {:?}",
        layer_ids(vcx)
    );
    Asked::new(rx)
}

fn commit_q(savepoint: bool, left: Duration) -> CommitQuestion {
    CommitQuestion {
        run: RunId(1),
        unit: UnitId {
            run: RunId(1),
            ordinal: 0,
        },
        question: 7,
        affected: Some(214),
        deadline: Instant::now() + left,
        savepoint,
        production: true,
        connection: Arc::from("orders-prod"),
    }
}

fn ask_commit(vcx: &VisualTestContext, q: CommitQuestion, c: CommitContext) -> Asked<CommitAnswer> {
    let rx = vcx.cx.update(|cx| commit_prompt::open_in(q, c, cx));
    vcx.run_until_parked();
    assert!(
        layer_ids(vcx).contains(&commit_prompt::COMMIT_LAYER_ID),
        "the prompt opened: {:?}",
        layer_ids(vcx)
    );
    Asked::new(rx)
}

/// Presses each of `keys` down and up, as a keyboard sends it (GPUI turns a clean down-and-up
/// of Enter on a focused button into a click, so the key-up phase is exercised too).
fn press(vcx: &mut VisualTestContext, keys: &'static str) {
    for key in keys.split(' ') {
        let Ok(keystroke) = gpui::Keystroke::parse(key) else {
            panic!("not a keystroke: {key}");
        };
        vcx.simulate_keystrokes(key);
        vcx.simulate_event(gpui::KeyUpEvent { keystroke });
        vcx.run_until_parked();
    }
}

/// The close-window chord of this platform.
fn close_chord() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-w"
    } else {
        "ctrl-w"
    }
}

fn click_button(vcx: &mut VisualTestContext, id: &str) {
    let selector: &'static str = Box::leak(format!("button-{id}").into_boxed_str());
    click(vcx, selector);
    vcx.run_until_parked();
}

/// The registered name of the focused dialog part.
fn focused_id(vcx: &VisualTestContext) -> String {
    vcx.run_until_parked();
    in_window(vcx, |window, cx| parts::focused(window, cx)).unwrap_or_default()
}

fn input(vcx: &VisualTestContext, name: &str) -> Entity<TextInput> {
    vcx.cx
        .read(|cx| parts::input(cx, name))
        .unwrap_or_else(|| panic!("no field `{name}`"))
}

fn focus(vcx: &VisualTestContext, name: &str) {
    let handle = vcx
        .cx
        .read(|cx| parts::focus(cx, name))
        .unwrap_or_else(|| panic!("no part `{name}`"));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
}

fn type_into(vcx: &mut VisualTestContext, name: &str, text: &str) {
    focus(vcx, name);
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

/// Replaces the field's text with `text` through the platform input path (a paste lands so).
fn set_field(vcx: &VisualTestContext, name: &str, text: &str) {
    let field = input(vcx, name);
    in_window(vcx, |window, cx| {
        field.update(cx, |i, cx| {
            i.set_value("");
            i.replace_text_in_range(None, text, window, cx);
        });
    });
    vcx.run_until_parked();
}

/// The open dialog's button `id`: (kind, enabled), when drawn.
fn button(vcx: &VisualTestContext, id: &str) -> Option<(ButtonKind, bool)> {
    vcx.cx.read(|cx| {
        open_dialog_buttons(cx)
            .into_iter()
            .find(|b| b.id == id)
            .map(|b| (b.kind, b.enabled))
    })
}

fn frame_texts(vcx: &VisualTestContext) -> Vec<String> {
    registry_frame(vcx)
        .into_iter()
        .map(|r| r.text.to_string())
        .collect()
}

fn shows(vcx: &VisualTestContext, text: &str) -> bool {
    frame_texts(vcx).iter().any(|t| t == text)
}

fn advance(vcx: &VisualTestContext, d: Duration) {
    vcx.executor().advance_clock(d);
    vcx.run_until_parked();
}

// ---- the plan (R-M3-84, RC-1) -------------------------------------------------------------------

fn item(unit: u32, decision: GateDecision, class: GateClass) -> GateItem {
    GateItem {
        unit,
        decision,
        verdict: UnitVerdict::Write,
        effects: SessionEffects::empty(),
        class,
        fingerprint: 1_000 + u64::from(unit),
    }
}

fn confirm_item(unit: u32, name: Option<&str>, class: GateClass) -> GateItem {
    item(
        unit,
        GateDecision::Confirm {
            reasons: vec![reason("safety-reason-schema-change", &[])],
            type_to_confirm: name.map(Arc::from),
            irreversible: false,
            suppressible: name.is_none(),
        },
        class,
    )
}

fn warn_item(unit: u32) -> GateItem {
    item(
        unit,
        GateDecision::Warn(vec![reason("safety-reason-locking-clause", &[])]),
        GateClass::Lock,
    )
}

/// Five units: a confirm, a confirm whose name is typed, a confirm, a confirm-in-transaction
/// unit and a lock warning.
fn mixed_plan() -> GatePlan {
    GatePlan {
        items: vec![
            confirm_item(0, None, GateClass::DdlOther),
            confirm_item(1, Some(NAME), GateClass::DmlUnsafe),
            confirm_item(2, None, GateClass::DdlOther),
            item(
                3,
                GateDecision::RunInTxThenConfirm {
                    reasons: Vec::new(),
                },
                GateClass::Dml,
            ),
            warn_item(4),
        ],
    }
}

#[test]
fn execute_all_never_answers_a_type_to_confirm_unit() {
    let mut plan = mixed_plan();
    assert_eq!(execute_all_covers(&plan, 0), vec![2, 4]);
    assert_eq!(
        answer_plan(&mut plan, 0, ConfirmAnswer::ExecuteAll, false),
        PlanFlow::Go(None)
    );
    assert_eq!(plan.items[0].decision, GateDecision::Run);
    assert!(
        matches!(
            &plan.items[1].decision,
            GateDecision::Confirm { type_to_confirm: Some(n), .. } if &**n == NAME
        ),
        "a unit that needs its name typed still asks"
    );
    assert_eq!(plan.items[2].decision, GateDecision::Run);
    assert!(
        matches!(
            plan.items[3].decision,
            GateDecision::RunInTxThenConfirm { .. }
        ),
        "Execute all never pre-answers confirm-in-transaction"
    );
    assert_eq!(plan.items[4].decision, GateDecision::Run);
}

#[test]
fn skip_all_skips_every_later_unit_that_asks_and_cancel_sends_nothing() {
    let mut plan = mixed_plan();
    assert_eq!(
        answer_plan(&mut plan, 0, ConfirmAnswer::SkipAll, false),
        PlanFlow::Go(None)
    );
    for i in [0, 1, 2, 4] {
        assert!(
            matches!(&plan.items[i].decision, GateDecision::Deny(r) if r.fluent_id == hacivat_exec::SKIPPED_BY_USER),
            "unit {i} skipped"
        );
    }
    assert!(matches!(
        plan.items[3].decision,
        GateDecision::RunInTxThenConfirm { .. }
    ));
    let mut plan = mixed_plan();
    assert_eq!(
        answer_plan(&mut plan, 0, ConfirmAnswer::Cancel, false),
        PlanFlow::Cancelled
    );
}

#[test]
fn an_answered_confirm_keeps_the_items_verdict_and_suppress_returns_its_fingerprint() {
    let mut explain = item(
        0,
        GateDecision::Confirm {
            reasons: vec![reason("exec-explain-analyze-not-undone", &[])],
            type_to_confirm: None,
            irreversible: false,
            suppressible: false,
        },
        GateClass::Unknown,
    );
    explain.verdict = UnitVerdict::ExplainAnalyzeRollback;
    explain.effects = SessionEffects::OPENS_WRITE;
    let mut plan = GatePlan {
        items: vec![explain.clone(), confirm_item(1, None, GateClass::DdlOther)],
    };
    assert_eq!(
        answer_plan(&mut plan, 0, ConfirmAnswer::Execute, false),
        PlanFlow::Go(None)
    );
    let kept = &plan.items[0];
    assert_eq!(kept.decision, GateDecision::Run);
    assert_eq!(kept.verdict, UnitVerdict::ExplainAnalyzeRollback);
    assert_eq!(
        (kept.effects, kept.class, kept.fingerprint),
        (explain.effects, explain.class, explain.fingerprint)
    );
    // "Don't ask again" hands back the item's key only when the dialog offered it.
    assert_eq!(
        answer_plan(
            &mut plan.clone(),
            1,
            ConfirmAnswer::ExecuteAndSuppress,
            false
        ),
        PlanFlow::Go(None)
    );
    assert_eq!(
        answer_plan(&mut plan, 1, ConfirmAnswer::ExecuteAndSuppress, true),
        PlanFlow::Go(Some(1_001))
    );
    assert_eq!(plan.items[1].decision, GateDecision::Run);
}

#[test]
fn the_question_carries_the_items_facts_unchanged() {
    let typed = confirm_item(1, Some("Müşteri"), GateClass::DmlUnsafe);
    let q = question_for(
        RunId(3),
        &typed,
        4,
        Arc::from("DELETE FROM \"Müşteri\""),
        true,
        Arc::from("p"),
    )
    .expect("a Confirm asks");
    assert_eq!(q.type_to_confirm.as_deref(), Some("Müşteri"), "exact bytes");
    assert_eq!((q.unit, q.index, q.total), (1, 2, 4));
    assert!(q.production && !q.warn_only);
    let warn = question_for(
        RunId(3),
        &warn_item(0),
        1,
        Arc::from("x"),
        true,
        Arc::from("p"),
    )
    .expect("a Warn asks");
    assert!(warn.warn_only && warn.suppressible);
    let tx = item(
        0,
        GateDecision::RunInTxThenConfirm {
            reasons: Vec::new(),
        },
        GateClass::Dml,
    );
    assert!(question_for(RunId(3), &tx, 1, Arc::from("x"), true, Arc::from("p")).is_none());
}

#[test]
fn dont_ask_again_is_never_offered_where_the_rules_forbid_it() {
    let mut q = question(true);
    q.suppressible = true;
    for class in [
        GateClass::DdlDestructive,
        GateClass::Admin,
        GateClass::Unknown,
        GateClass::Routine,
    ] {
        assert!(!offers_suppress(&q, Some(class)), "{class:?} on production");
    }
    assert!(!offers_suppress(&q, None), "an unknown class on production");
    assert!(offers_suppress(&q, Some(GateClass::DdlOther)));
    assert!(offers_suppress(&q, Some(GateClass::Lock)));
    // Off production the question's flag decides (the gate already refused multi-statement
    // writes and typed names).
    let mut calm = question(false);
    calm.suppressible = true;
    assert!(offers_suppress(&calm, Some(GateClass::DmlUnsafe)));
    calm.suppressible = false;
    assert!(!offers_suppress(&calm, Some(GateClass::DmlUnsafe)));
    // Never with a typed name, never for a statement not analysed or an implicit commit.
    let mut typed = typed_question();
    typed.suppressible = true;
    assert!(!offers_suppress(&typed, Some(GateClass::DmlUnsafe)));
    for id in ["exec-gate-not-analysed", "exec-gate-implicit-commit"] {
        let mut q = question(false);
        q.suppressible = true;
        q.reasons.push(reason(id, &[]));
        assert!(!offers_suppress(&q, Some(GateClass::Unknown)), "{id}");
    }
}

#[test]
fn the_typed_name_compares_exact_bytes() {
    assert!(name_matches(NAME, NAME));
    for wrong in [
        "MÜŞTERİ",
        "musteri",
        "müşteri ",
        " müşteri",
        "mu\u{308}şteri",
        "MÜŞTERI",
    ] {
        assert!(!name_matches(wrong, NAME), "{wrong:?}");
    }
}

// ---- the confirmation dialog --------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Close {
    Escape,
    CmdW,
    XButton,
    Window,
    Quit,
}

fn answer_after(cx: &mut TestAppContext, close: Close) -> ConfirmAnswer {
    let vcx = fresh(cx);
    let a = ask(vcx, question(true), ctx(GateClass::DdlDestructive));
    match close {
        Close::Escape => press(vcx, "escape"),
        Close::CmdW => press(vcx, close_chord()),
        Close::XButton => {
            click(vcx, "prompt-close");
            vcx.run_until_parked();
        }
        Close::Window => close_all_windows(vcx),
        Close::Quit => assert!(run(vcx, "app.quit")),
    }
    vcx.run_until_parked();
    let answer = a
        .get()
        .unwrap_or_else(|| panic!("{close:?} answered nothing"));
    if !matches!(close, Close::Window) {
        assert!(
            !layer_ids(vcx).contains(&confirm::CONFIRM_LAYER_ID),
            "{close:?} closed the dialog"
        );
    }
    answer
}

#[gpui::test]
fn any_close_answers_cancel(cx: &mut TestAppContext) {
    for close in [
        Close::Escape,
        Close::CmdW,
        Close::XButton,
        Close::Window,
        Close::Quit,
    ] {
        assert_eq!(answer_after(cx, close), ConfirmAnswer::Cancel, "{close:?}");
    }
}

#[gpui::test]
fn enter_in_type_field_never_executes(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let a = ask(vcx, typed_question(), ctx(GateClass::DmlUnsafe));
    type_into(vcx, "confirm-type", NAME);
    assert_eq!(
        button(vcx, "confirm-execute"),
        Some((ButtonKind::Danger, true))
    );
    assert_eq!(focused_id(vcx), "confirm-type");
    press(vcx, "enter");
    assert!(a.get().is_none(), "Enter in the field sends nothing");
    assert!(layer_ids(vcx).contains(&confirm::CONFIRM_LAYER_ID));
    // A mismatched name and Enter: still nothing; the hint shows.
    set_field(vcx, "confirm-type", "Müşteri");
    press(vcx, "enter");
    assert!(a.get().is_none());
    assert!(shows(
        vcx,
        "Type it exactly as shown, letter case included."
    ));
    // Escape in the field answers Cancel.
    focus(vcx, "confirm-type");
    press(vcx, "escape");
    assert_eq!(a.get(), Some(ConfirmAnswer::Cancel));
}

#[gpui::test]
fn execute_disabled_until_exact_bytes(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let a = ask(vcx, typed_question(), ctx(GateClass::DmlUnsafe));
    assert_eq!(
        button(vcx, "confirm-execute"),
        Some((ButtonKind::Danger, false))
    );
    for wrong in ["MÜŞTERİ", "musteri", "müşteri ", "mu\u{308}şteri"] {
        set_field(vcx, "confirm-type", wrong);
        assert_eq!(
            button(vcx, "confirm-execute"),
            Some((ButtonKind::Danger, false)),
            "{wrong:?} keeps Execute disabled"
        );
        click_button(vcx, "confirm-execute");
        assert!(
            a.get().is_none(),
            "a disabled Execute sends nothing ({wrong:?})"
        );
    }
    // A name the keyboard can't reproduce can be pasted: Copy writes the exact bytes.
    click_button(vcx, "confirm-copy");
    let copied = vcx.cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some(NAME));
    set_field(vcx, "confirm-type", NAME);
    assert_eq!(
        button(vcx, "confirm-execute"),
        Some((ButtonKind::Danger, true))
    );
    click_button(vcx, "confirm-execute");
    assert_eq!(a.get(), Some(ConfirmAnswer::Execute));
    assert!(!layer_ids(vcx).contains(&confirm::CONFIRM_LAYER_ID));
}

#[gpui::test]
fn execute_all_disabled_until_exact_bytes(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let mut q = typed_question();
    q.index = 2;
    q.total = 3;
    let c = ConfirmContext {
        covered: 1,
        later_asks: 1,
        ..ctx(GateClass::DmlUnsafe)
    };
    let a = ask(vcx, q, c);
    assert_eq!(
        button(vcx, "confirm-execute-all"),
        Some((ButtonKind::Danger, false))
    );
    assert!(shows(vcx, "statement 2 of 3"));
    set_field(vcx, "confirm-type", "MÜŞTERİ");
    click_button(vcx, "confirm-execute-all");
    assert!(a.get().is_none());
    set_field(vcx, "confirm-type", NAME);
    assert_eq!(
        button(vcx, "confirm-execute-all"),
        Some((ButtonKind::Danger, true))
    );
    click_button(vcx, "confirm-execute-all");
    assert_eq!(a.get(), Some(ConfirmAnswer::ExecuteAll));
}

#[gpui::test]
fn cancel_has_initial_focus_and_execute_is_danger_except_lock(cx: &mut TestAppContext) {
    let cases: [(ConfirmQuestion, GateClass, ButtonKind); 5] = [
        (
            question(true),
            GateClass::DdlDestructive,
            ButtonKind::Danger,
        ),
        (question(true), GateClass::Unknown, ButtonKind::Danger),
        (typed_question(), GateClass::DmlUnsafe, ButtonKind::Danger),
        // The calm variant off production.
        (
            ConfirmQuestion {
                suppressible: true,
                ..question(false)
            },
            GateClass::DmlUnsafe,
            ButtonKind::Danger,
        ),
        // A row lock changes no data: Default (RC-2), still never focused.
        (
            ConfirmQuestion {
                warn_only: true,
                suppressible: true,
                ..question(true)
            },
            GateClass::Lock,
            ButtonKind::Default,
        ),
    ];
    for (q, class, kind) in cases {
        let vcx = fresh(cx);
        let suppressible = q.suppressible;
        let a = ask(vcx, q, ctx(class));
        assert_eq!(focused_id(vcx), "confirm-cancel", "{class:?}");
        assert_eq!(
            button(vcx, "confirm-cancel"),
            Some((ButtonKind::Primary, true))
        );
        assert_eq!(
            button(vcx, "confirm-execute").map(|b| b.0),
            Some(kind),
            "{class:?}"
        );
        if suppressible {
            assert_eq!(button(vcx, "confirm-suppress").map(|b| b.0), Some(kind));
        }
        // Enter on the focused Cancel sends nothing.
        press(vcx, "enter");
        assert_eq!(a.get(), Some(ConfirmAnswer::Cancel), "{class:?}");
    }
}

#[gpui::test]
fn no_suppress_button_with_type_to_confirm(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let mut q = typed_question();
    // Even a question that claims it is suppressible.
    q.suppressible = true;
    let _a = ask(vcx, q, ctx(GateClass::DmlUnsafe));
    assert_eq!(button(vcx, "confirm-suppress"), None);
    assert!(!button_labels(vcx).iter().any(|l| l.contains("ask again")));
    // On production, a destructive class never offers it either.
    let vcx = fresh(cx);
    let mut q = question(true);
    q.suppressible = true;
    let _a = ask(vcx, q, ctx(GateClass::DdlDestructive));
    assert_eq!(button(vcx, "confirm-suppress"), None);
    // Where it is offered on production, its label names the scope.
    let vcx = fresh(cx);
    let mut q = question(true);
    q.suppressible = true;
    let a = ask(vcx, q, ctx(GateClass::DdlOther));
    assert!(button_labels(vcx).contains(&"Execute; don't ask again in this console".to_string()));
    assert!(shows(
        vcx,
        "Not asked again for statements of the same shape in orders-prod until it closes."
    ));
    click_button(vcx, "confirm-suppress");
    assert_eq!(a.get(), Some(ConfirmAnswer::ExecuteAndSuppress));
}

#[gpui::test]
fn tab_is_trapped_in_visual_order(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let mut q = typed_question();
    q.index = 2;
    q.total = 3;
    let c = ConfirmContext {
        covered: 1,
        later_asks: 1,
        ..ctx(GateClass::DmlUnsafe)
    };
    let _a = ask(vcx, q, c);
    assert_eq!(focused_id(vcx), "confirm-cancel");
    // Execute and Execute all are disabled: Tab skips them.
    let mut seen = Vec::new();
    for _ in 0..5 {
        press(vcx, "tab");
        seen.push(focused_id(vcx));
    }
    assert_eq!(
        seen,
        [
            "confirm-type",
            "confirm-copy",
            "confirm-skip",
            "confirm-skip-all",
            "confirm-cancel"
        ]
    );
    set_field(vcx, "confirm-type", NAME);
    focus(vcx, "confirm-copy");
    press(vcx, "tab");
    assert_eq!(focused_id(vcx), "confirm-execute");
    press(vcx, "shift-tab");
    assert_eq!(focused_id(vcx), "confirm-copy");
}

#[gpui::test]
fn the_dialog_shows_what_the_gate_said(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let mut q = question(true);
    q.irreversible = true;
    q.reasons = vec![
        reason("safety-destructive-drop", &[("table", "evil\u{202E}txt")]),
        reason("exec-explain-analyze-not-undone", &[]),
    ];
    let _a = ask(vcx, q, ctx(GateClass::DdlDestructive));
    let texts = frame_texts(vcx);
    assert!(texts.contains(&"Run this statement on orders-prod?".to_string()));
    assert!(texts.contains(&"Removes schema objects or their data".to_string()));
    assert!(
        texts.contains(&"MySQL commits DDL at once, so this can't be rolled back.".to_string())
    );
    assert!(texts.contains(&"Nothing is sent until you choose.".to_string()));
    // A bidi control in a name is drawn visibly, never applied.
    assert!(
        texts.iter().any(|t| t == "Removes evil\\u{202E}txt."),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("EXPLAIN ANALYZE runs the statement"))
    );
    assert!(!texts.iter().any(|t| t.contains('\u{202E}')));
}

#[gpui::test]
fn the_calm_variant_says_so_and_the_unanalysed_one_does_not(cx: &mut TestAppContext) {
    let calm = "Off production, Hacivat asks only before statements like this one. Settings › Execution can turn it off.";
    let vcx = fresh(cx);
    let mut q = question(false);
    q.suppressible = true;
    let _a = ask(vcx, q, ctx(GateClass::DmlUnsafe));
    assert!(shows(vcx, calm));
    assert!(button_labels(vcx).contains(&"Execute and don't ask again".to_string()));
    // The text registry records every window's frame: close this one first.
    press(vcx, "escape");
    let vcx = fresh(cx);
    let mut q = question(false);
    q.reasons = vec![reason("exec-gate-not-analysed", &[])];
    let _a = ask(vcx, q, ctx(GateClass::Unknown));
    assert!(!shows(vcx, calm));
    assert!(shows(vcx, "Nothing is sent until you choose."));
    assert_eq!(button(vcx, "confirm-suppress"), None);
}

#[gpui::test]
fn ask_plan_asks_in_order_and_execute_all_still_asks_for_a_typed_name(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let plan = mixed_plan();
    let sql: Vec<Arc<str>> = (0..5).map(|i| Arc::from(format!("S{i}"))).collect();
    let task = vcx.cx.update(|cx| {
        ask_plan(
            RunId(9),
            plan,
            sql,
            true,
            Arc::from("orders-prod"),
            ConfirmContext::default(),
            cx,
        )
    });
    let out: std::rc::Rc<RefCell<Option<Option<PlanAnswers>>>> = Default::default();
    let slot = out.clone();
    vcx.cx.update(|cx| {
        cx.spawn(async move |_| {
            *slot.borrow_mut() = Some(task.await);
        })
        .detach();
    });
    vcx.run_until_parked();
    assert!(shows(vcx, "statement 1 of 5"));
    assert!(shows(
        vcx,
        "Execute all also runs the remaining 2 statements that would ask, without asking. A statement that needs its name typed still asks, and a production change still waits for your answer after it runs."
    ));
    click_button(vcx, "confirm-execute-all");
    // Unit 1 still asks for its name.
    assert!(vcx.cx.read(confirm_open));
    assert!(shows(vcx, "statement 2 of 5"));
    set_field(vcx, "confirm-type", NAME);
    click_button(vcx, "confirm-execute");
    wait_until(vcx, "the plan", |_| out.borrow().is_some());
    let answers = out.borrow_mut().take().flatten().expect("not cancelled");
    let decisions: Vec<_> = answers
        .plan
        .items
        .iter()
        .map(|i| i.decision.clone())
        .collect();
    assert_eq!(decisions[0], GateDecision::Run);
    assert_eq!(decisions[1], GateDecision::Run);
    assert_eq!(decisions[2], GateDecision::Run);
    assert!(matches!(
        decisions[3],
        GateDecision::RunInTxThenConfirm { .. }
    ));
    assert_eq!(decisions[4], GateDecision::Run);
    assert!(answers.suppress.is_empty());
    assert!(!vcx.cx.read(confirm_open));
}

#[gpui::test]
fn labels_found_with_access_key_runs(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let mut q = question(true);
    q.index = 1;
    q.total = 3;
    let c = ConfirmContext {
        covered: 2,
        later_asks: 2,
        ..ctx(GateClass::DdlDestructive)
    };
    let _a = ask(vcx, q, c);
    let frame = registry_frame(vcx);
    for label in ["Execute", "Execute all", "Skip", "Skip all", "Cancel"] {
        assert!(find_label_runs(&frame, label).is_some(), "{label}");
    }
    press(vcx, "escape");
    let _c = ask_commit(
        vcx,
        commit_q(false, Duration::from_secs(300)),
        CommitContext::default(),
    );
    let frame = registry_frame(vcx);
    for label in ["Commit", "Roll back"] {
        assert!(find_label_runs(&frame, label).is_some(), "{label}");
    }
    press(vcx, "escape");
    let _s = ask_commit(
        vcx,
        commit_q(true, Duration::from_secs(300)),
        CommitContext::default(),
    );
    let frame = registry_frame(vcx);
    for label in ["Keep", "Undo"] {
        assert!(find_label_runs(&frame, label).is_some(), "{label}");
    }
}

// ---- the confirm-in-transaction prompt ---------------------------------------------------------

#[gpui::test]
fn commit_prompt_focuses_roll_back_and_counts_down_once_a_second(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let a = ask_commit(
        vcx,
        commit_q(false, Duration::from_secs(272)),
        CommitContext::default(),
    );
    assert_eq!(focused_id(vcx), "commit-safe");
    assert!(shows(vcx, "214 rows changed on orders-prod"));
    let count = || vcx.cx.read(countdown_of).expect("the prompt is open");
    assert_eq!(count(), ("4:32".to_string(), 0));
    advance(vcx, Duration::from_millis(500));
    assert_eq!(
        count(),
        ("4:32".to_string(), 0),
        "no repaint within the second"
    );
    advance(vcx, Duration::from_millis(500));
    assert_eq!(count(), ("4:31".to_string(), 1));
    advance(vcx, Duration::from_secs(1));
    assert_eq!(count(), ("4:30".to_string(), 2));
    advance(vcx, Duration::from_secs(10));
    assert_eq!(
        count(),
        ("4:20".to_string(), 12),
        "once a second, never more"
    );
    // Enter on the focused Roll back answers it.
    press(vcx, "enter");
    assert_eq!(a.get(), Some(CommitAnswer::RollBack));
    assert!(vcx.cx.read(open_commit_views).is_empty());
}

#[gpui::test]
fn savepoint_prompt_says_the_transaction_stays_open(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let mut q = commit_q(true, Duration::from_secs(291));
    q.affected = Some(1);
    let a = ask_commit(vcx, q, CommitContext::default());
    let texts = frame_texts(vcx);
    assert!(texts.contains(&"1 row changed inside your transaction".to_string()));
    assert!(texts.contains(
        &"Undo: removes this statement's change; your transaction stays open.".to_string()
    ));
    assert!(texts.contains(
        &"Keep: the change joins your open transaction. Nothing is committed yet.".to_string()
    ));
    let labels = button_labels(vcx);
    assert!(labels.contains(&"Keep".to_string()) && labels.contains(&"Undo".to_string()));
    assert!(
        !labels.iter().any(|l| l == "Commit" || l == "Roll back"),
        "the savepoint form never says Commit: {labels:?}"
    );
    assert!(texts.iter().any(|t| t == "Undoes this statement in "));
    assert!(texts.iter().any(|t| t == "4:51"));
    assert_eq!(focused_id(vcx), "commit-safe");
    // Tab reaches Keep; Enter keeps the change in the transaction.
    press(vcx, "tab");
    assert_eq!(focused_id(vcx), "commit-keep");
    press(vcx, "enter");
    assert_eq!(a.get(), Some(CommitAnswer::Commit));
}

#[gpui::test]
fn commit_prompt_closes_answer_roll_back(cx: &mut TestAppContext) {
    for keys in ["escape", close_chord()] {
        let vcx = fresh(cx);
        let a = ask_commit(
            vcx,
            commit_q(false, Duration::from_secs(300)),
            CommitContext::default(),
        );
        press(vcx, keys);
        assert_eq!(a.get(), Some(CommitAnswer::RollBack), "{keys}");
    }
    let vcx = fresh(cx);
    let a = ask_commit(
        vcx,
        commit_q(true, Duration::from_secs(300)),
        CommitContext::default(),
    );
    close_all_windows(vcx);
    assert_eq!(a.get(), Some(CommitAnswer::RollBack), "closing the window");
}

#[gpui::test]
fn quit_with_an_open_commit_prompt_answers_roll_back(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let a = ask_commit(
        vcx,
        commit_q(false, Duration::from_secs(300)),
        CommitContext::default(),
    );
    assert!(run(vcx, "app.quit"));
    vcx.run_until_parked();
    assert_eq!(a.get(), Some(CommitAnswer::RollBack));
    assert!(!layer_ids(vcx).contains(&commit_prompt::COMMIT_LAYER_ID));
}

#[gpui::test]
fn the_deadline_answers_roll_back(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let a = ask_commit(
        vcx,
        commit_q(false, Duration::from_secs(3)),
        CommitContext::default(),
    );
    advance(vcx, Duration::from_secs(2));
    assert!(a.get().is_none());
    advance(vcx, Duration::from_secs(2));
    assert_eq!(a.get(), Some(CommitAnswer::RollBack));
    assert!(!layer_ids(vcx).contains(&commit_prompt::COMMIT_LAYER_ID));
}

// ---- the port -----------------------------------------------------------------------------------

/// A shell with a console bound to a profile `name` tagged `env`.
fn console(
    cx: &mut TestAppContext,
    name: &str,
    env: EnvTag,
) -> (
    &'static mut VisualTestContext,
    hacivat_app::console::ConsoleId,
) {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let p = test_profile(name, Some(env), "10.0.0.1");
    let profile = add_profile(vcx, p.clone());
    let (session, _fake) = test_session(vcx, p);
    let id = register_test_console(vcx, profile, session);
    (vcx, id)
}

#[gpui::test]
fn the_port_names_the_console_s_connection_and_production(cx: &mut TestAppContext) {
    let (vcx, id) = console(cx, "billing-prod", EnvTag::Production);
    let port = vcx.cx.update(|cx| AppConfirmPort::for_console(id, cx));
    // The run service's question carries an empty name (T16 m13).
    let mut q = commit_q(false, Duration::from_secs(300));
    q.connection = Arc::from("");
    let a = Asked::new(port.commit_decision(q));
    wait_until(vcx, "the prompt", |vcx| {
        !vcx.cx.read(open_commit_views).is_empty()
    });
    assert!(shows(vcx, "214 rows changed on billing-prod"));
    press(vcx, "escape");
    assert_eq!(a.get(), Some(CommitAnswer::RollBack));
    // A confirmation that claims "not production" for a production console gets the
    // production wording.
    let mut q = question(false);
    q.connection = Arc::from("");
    let a = Asked::new(port.confirm(q));
    wait_until(vcx, "the dialog", |vcx| vcx.cx.read(confirm_open));
    assert!(shows(vcx, "Run this statement on billing-prod?"));
    assert!(shows(vcx, "Nothing is sent until you choose."));
    press(vcx, "escape");
    assert_eq!(a.get(), Some(ConfirmAnswer::Cancel));
}

#[gpui::test]
fn the_port_of_a_staging_console_draws_no_production(cx: &mut TestAppContext) {
    let (vcx, id) = console(cx, "billing-staging", EnvTag::Staging);
    let port = vcx.cx.update(|cx| AppConfirmPort::for_console(id, cx));
    let mut q = question(true);
    q.connection = Arc::from("");
    q.suppressible = true;
    let a = Asked::new(port.confirm(q));
    wait_until(vcx, "the dialog", |vcx| vcx.cx.read(confirm_open));
    assert!(shows(vcx, "Run this statement on billing-staging?"));
    assert!(button_labels(vcx).contains(&"Execute and don't ask again".to_string()));
    press(vcx, "escape");
    assert_eq!(a.get(), Some(ConfirmAnswer::Cancel));
}

#[gpui::test]
fn a_port_whose_console_is_gone_or_unbound_answers_the_safe_value(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let port = vcx
        .cx
        .update(|cx| AppConfirmPort::for_console(hacivat_app::console::ConsoleId(4242), cx));
    let c = Asked::new(port.commit_decision(commit_q(false, Duration::from_secs(300))));
    let q = Asked::new(port.confirm(question(true)));
    vcx.run_until_parked();
    assert_eq!(c.get(), Some(CommitAnswer::RollBack));
    assert_eq!(q.get(), Some(ConfirmAnswer::Cancel));
    assert!(layer_ids(vcx).is_empty(), "nothing opened");
    let detached = AppConfirmPort::default();
    let mut rx = detached.commit_decision(commit_q(true, Duration::from_secs(300)));
    assert!(matches!(rx.try_recv(), Ok(CommitAnswer::RollBack)));
}

#[gpui::test]
fn the_port_follows_commit_decided(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let port = vcx.cx.update(AppConfirmPort::unbound);
    let q = commit_q(false, Duration::from_secs(300));
    let waiting = RunEvent::NeedsCommit {
        unit: q.unit,
        question: q.question,
        affected: q.affected,
        deadline: q.deadline,
        savepoint: q.savepoint,
    };
    let (tx, rx) = watch::channel(Some((11, waiting)));
    let sql: Arc<[Arc<str>]> = Arc::from(vec![Arc::from("UPDATE orders SET archived = true")]);
    port.follow(rx, sql);
    let a = Asked::new(port.commit_decision(q));
    wait_until(vcx, "the prompt", |vcx| {
        !vcx.cx.read(open_commit_views).is_empty()
    });
    assert!(
        shows(vcx, "UPDATE orders SET archived = true"),
        "the statement is shown"
    );
    // The session decided (by its deadline, say): `CommitDecided` clears the question.
    tx.send_replace(None);
    wait_until(vcx, "the prompt to close", |vcx| {
        vcx.cx.read(open_commit_views).is_empty()
    });
    assert_eq!(a.get(), Some(CommitAnswer::RollBack));
    assert!(!layer_ids(vcx).contains(&commit_prompt::COMMIT_LAYER_ID));
}

#[gpui::test]
fn a_lost_connection_turns_the_prompt_into_a_notice(cx: &mut TestAppContext) {
    let vcx = fresh(cx);
    let port = vcx.cx.update(AppConfirmPort::unbound);
    let a = Asked::new(port.commit_decision(commit_q(true, Duration::from_secs(300))));
    wait_until(vcx, "the prompt", |vcx| {
        !vcx.cx.read(open_commit_views).is_empty()
    });
    port.connection_lost(Some(3));
    wait_until(vcx, "the lost form", |vcx| {
        shows(
            vcx,
            "The server rolled back your open transaction: this statement and the 3 before it. Nothing was committed.",
        )
    });
    assert_eq!(a.get(), Some(CommitAnswer::RollBack));
    assert!(shows(vcx, "The connection to orders-prod was lost"));
    assert_eq!(button_labels(vcx), ["Close"]);
    assert_eq!(focused_id(vcx), "commit-close");
    // No countdown runs any more; the user closes it.
    advance(vcx, Duration::from_secs(400));
    assert!(layer_ids(vcx).contains(&commit_prompt::COMMIT_LAYER_ID));
    press(vcx, "enter");
    assert!(!layer_ids(vcx).contains(&commit_prompt::COMMIT_LAYER_ID));
}
