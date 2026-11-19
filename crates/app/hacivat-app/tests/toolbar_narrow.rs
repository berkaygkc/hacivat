//! M3a follow-up F2, fix round 1 (I2): the console toolbar in a narrow pane. Trust signals
//! (the read-only lock chip, the environment mark) are never hidden or cut, at any width a
//! split pane can have (each side keeps 120 px, `hacivat_ui::panes`): the toolbar's last stage
//! wraps its row instead.
//!
//! ## m3b-t26-prod-unlock: the production read-only notice (and its short lock chip) is gone
//! (R-M3-6 REVISED ends). A production console's trust signal on the bar is its environment
//! swatch (the hairline and the capsule are drawn outside it); a production profile that is
//! read-only by its own flag also shows the lock chip, swept here like the staging one.

mod support;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    Bounds, Context, IntoElement, ParentElement as _, Pixels, Render, Styled as _, TestAppContext,
    Window, div, px,
};
use hacivat_app::console::toolbar::{RunUx, ToolbarActions, ToolbarModel, build};
use hacivat_ui::capsule::EnvKind;
use hacivat_ui::text::TextRegistry;
use support::shell;
// ## m3c-t41-chrome
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::console::toolbar::build_for;
use hacivat_app::console::{tx_badge, tx_chip};
use hacivat_app::results::strip::{self, StripActions, StripKind, StripModel};
use hacivat_driver::ExecOpts;
use hacivat_driver::fake::FakeDriver;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::LiveState;
use hacivat_session::session::{ExecOrigin, Request, TxMode};
use support::{
    add_profile, connect, live, profile_id, register_test_console, shell_with_fake,
    stop_core_runtime_test, test_profile, wait_until, with_ws,
};

const PROD: &str = "orders-prod";

fn pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres)
}

/// The narrowest console pane is 120 px (a split keeps each side at least that). Below 126 px
/// the English production chip ("read-only" with its red lock, 103.5 px, plus the bar's 22 px
/// of padding) was wider than the row itself (F2 fix round 1). ## m3b-t26-prod-unlock: that
/// chip is gone; the floors stay where they were (the lock chip is the widest trust item left).
const MIN_EN_PX: f32 = 126.0;
/// The same for Turkish ("salt okunur", about 119 px).
const MIN_TR_PX: f32 = 141.0;
/// The same for the pseudo-locales: their chip is about 186 px, and the waiting sentence keeps
/// too little room beside its marker and Cancel below about 190 px.
#[cfg(feature = "pseudo")]
const MIN_PSEUDO_PX: f32 = 219.0;
/// The bar's right padding (`spacing.lg`): the row's own edge.
const INNER_RIGHT: f32 = 10.0;

struct Host {
    width: Rc<Cell<f32>>,
    model: ToolbarModel,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let actions = ToolbarActions {
            run: Rc::new(|_, _| {}),
            stop: Rc::new(|_, _| {}),
            cancel_wait: Rc::new(|_, _| {}),
        };
        // The pane clips what runs past it, as the pane does.
        div()
            .w(px(self.width.get()))
            .overflow_hidden()
            .child(build(&self.model, &actions, cx))
    }
}

fn model(production: bool, read_only: bool, waiting: bool) -> ToolbarModel {
    ToolbarModel {
        run: RunUx::Idle,
        run_enabled: !waiting,
        name: "orders-prod".into(),
        env: Some(if production {
            EnvKind::Production
        } else {
            EnvKind::Staging
        }),
        waiting: waiting.then(|| ("orders-prod".into(), 4, 4)),
        read_only: read_only.then(|| "read-only".into()),
        pending_read_only: None,
    }
}

/// The trust items the toolbar of `model` must draw whole, by debug selector.
fn trust_ids(read_only: bool) -> Vec<&'static str> {
    let mut ids = vec!["console-env-swatch"];
    if read_only {
        ids.push("console-lock");
    }
    ids
}

fn inside(b: Bounds<Pixels>, bar: Bounds<Pixels>) -> bool {
    b.left() >= bar.left()
        && b.right() <= bar.right() - px(INNER_RIGHT) + px(0.5)
        && b.top() >= bar.top()
        && b.bottom() <= bar.bottom()
}

/// Sweeps the toolbar from a wide pane down to the narrowest one in `lang`; returns the
/// widths at which a trust item was not drawn whole inside the bar.
fn sweep(cx: &mut TestAppContext, lang: &str, floor: f32) -> Vec<String> {
    let _shell = shell(cx, lang);
    let mut failures = Vec::new();
    // ## m3b-t26-prod-unlock: production read-write, production read-only by its own flag,
    // and staging read-only (the old non-production case), each with and without waiting.
    for (production, read_only, waiting) in [
        (true, false, true),
        (true, false, false),
        (true, true, true),
        (true, true, false),
        (false, true, true),
        (false, true, false),
    ] {
        let width = Rc::new(Cell::new(700.0_f32));
        let w = width.clone();
        let (_, vcx) = cx.add_window_view(move |_, _| Host {
            width: w,
            model: model(production, read_only, waiting),
        });
        let mut widths: Vec<f32> = (0..)
            .map(|k| 700.0 - 13.0 * k as f32)
            .take_while(|w| *w > floor)
            .collect();
        widths.push(floor);
        for px_now in widths {
            width.set(px_now);
            vcx.update(|window, _| window.refresh());
            vcx.run_until_parked();
            let bar = vcx
                .debug_bounds("toolbar-console-toolbar")
                .expect("the console toolbar");
            // ## m3c-t41-chrome: below the old floors the bar takes its narrow stage, whose
            // padding is 8 px (the narrow Run draws no label).
            let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
            let narrow = !frame
                .iter()
                .any(|r| r.path.ends_with("#button-label-console-run") && r.visible);
            let inside = |b: Bounds<Pixels>, bar: Bounds<Pixels>| {
                if narrow {
                    b.left() >= bar.left()
                        && b.right() <= bar.right() - px(NARROW_RIGHT) + px(0.5)
                        && b.top() >= bar.top()
                        && b.bottom() <= bar.bottom()
                } else {
                    inside(b, bar)
                }
            };
            for id in trust_ids(read_only) {
                match vcx.debug_bounds(id) {
                    Some(b) if inside(b, bar) => {}
                    other => failures.push(format!(
                        "{lang} prod={production} ro={read_only} waiting={waiting} {px_now}: {id} {other:?} \
                         not inside {bar:?}"
                    )),
                }
            }
            let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
            for r in frame.iter().filter(|r| {
                r.path.ends_with("#console-lock-label")
                    || r.path.ends_with("#console-waiting-text")
                    // ## m3c-t41-chrome (F2 review N2): the wait's Cancel never hides.
                    || r.path.ends_with("#button-label-console-waiting-cancel")
            }) {
                if r.clipped || r.truncated || !r.visible || r.painted_px <= 0.0 {
                    failures.push(format!(
                        "{lang} prod={production} ro={read_only} waiting={waiting} {px_now}: {r:?} cut"
                    ));
                }
            }
        }
    }
    failures
}

#[gpui::test]
fn trust_items_stay_whole_in_a_narrow_pane_in_english(cx: &mut TestAppContext) {
    // ## m3c-t41-chrome: down to the split floor (R-M3-75); the old floor stays as a name.
    let _ = MIN_EN_PX;
    let failures = sweep(cx, "en", SPLIT_FLOOR_PX);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[gpui::test]
fn trust_items_stay_whole_in_a_narrow_pane_in_turkish(cx: &mut TestAppContext) {
    // ## m3c-t41-chrome
    let _ = MIN_TR_PX;
    let failures = sweep(cx, "tr", SPLIT_FLOOR_PX);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[cfg(feature = "pseudo")]
#[gpui::test]
fn trust_items_stay_whole_in_a_narrow_pane_in_the_pseudo_locales(cx: &mut TestAppContext) {
    let mut failures = sweep(cx, "en-XA", MIN_PSEUDO_PX);
    failures.extend(sweep(cx, "tr-XT", MIN_PSEUDO_PX));
    assert!(failures.is_empty(), "{failures:#?}");
}

// ## m3c-t41-chrome: `ToolbarItem::narrow` (R-M3-75; design/m3b-safe surface 24). Below the
// per-locale minimums above, the bar takes its narrow stage, so every trust item stays whole
// down to the 120 px split floor in every locale: the environment mark, the read-only lock
// chip ("read-only (guarded)" whole), the pending read-only chip, the run state, the waiting
// sentence and its Cancel (F2 review N2), the transaction mode chip with its mode word (T2b
// C1, N4) and the transaction badge (N3); and the strip's "Cursor open" (F2 review N4).

/// The narrowest pane a split leaves (`hacivat_ui::panes`).
const SPLIT_FLOOR_PX: f32 = 120.0;
/// A narrow bar keeps 8 px on its right (NOTES "Narrow toolbar": padding 4/8).
const NARROW_RIGHT: f32 = 8.0;

/// What one sweep frame shows: the toolbar's states and, with a console, its chip and badge.
#[derive(Clone)]
struct Case {
    run: RunUx,
    waiting: bool,
    pending: bool,
    read_only: Option<&'static str>,
    console: Option<hacivat_app::console::ConsoleId>,
    // ## m3c-t41-chrome (fix round 1, m2)
    /// The console is in a user transaction (the badge and its Commit and Roll back).
    tx: bool,
    /// The console shares its session (the "Shared session" chip).
    shared: bool,
}

struct CaseHost {
    width: Rc<Cell<f32>>,
    case: Case,
}

impl Render for CaseHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let actions = ToolbarActions {
            run: Rc::new(|_, _| {}),
            stop: Rc::new(|_, _| {}),
            cancel_wait: Rc::new(|_, _| {}),
        };
        let c = &self.case;
        let model = ToolbarModel {
            run: c.run,
            run_enabled: !c.waiting,
            name: "orders-prod".into(),
            env: Some(EnvKind::Production),
            waiting: c.waiting.then(|| ("orders-prod".into(), 4, 4)),
            read_only: c.read_only.map(Into::into),
            pending_read_only: c.pending.then(|| "orders-prod".into()),
        };
        div()
            .w(px(self.width.get()))
            .overflow_hidden()
            .child(build_for(&model, &actions, c.console, cx))
    }
}

// ## m3c-t41-chrome (fix round 1, m1)
/// The width of Cancel's word as its button draws it (the control style).
fn cancel_word_px(cx: &mut gpui::VisualTestContext) -> f32 {
    cx.update(|window, cx| {
        let word = hacivat_app::l10n_global::tr_id(cx, "hv-console-waiting-cancel");
        let style = hacivat_ui_core::theme::Theme::get(cx)
            .tokens
            .type_ramp
            .control;
        let run = gpui::TextRun {
            len: word.len(),
            font: hacivat_ui_core::fonts::font_for(&style),
            color: gpui::Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(
            window
                .text_system()
                .shape_line(word, px(style.size_px), &[run], None)
                .width,
        )
    })
}

/// The run states the sweep shows.
const RUNS: [RunUx; 3] = [RunUx::Idle, RunUx::Running(12), RunUx::Cancelling];

/// The words the mode chip of `console` may show in the current locale: its mode's word, and
/// for Auto-commit on a read-write session the narrow "Auto" (N4).
fn chip_words(cx: &gpui::App, console: hacivat_app::console::ConsoleId) -> Vec<String> {
    let view = tx_badge::view(console, cx).expect("the mirror");
    let mode = if view.read_only {
        TxMode::Auto
    } else {
        view.mode
    };
    let mut words = vec![tx_chip::mode_text(cx, mode).to_string()];
    if !view.read_only && mode == TxMode::Auto {
        words.push(hacivat_app::l10n_global::tr_id(cx, "hv-tx-chip-auto-short").to_string());
    }
    words
}

/// Sweeps `case` from a wide pane down to the split floor; the widths at which a trust item
/// was not drawn whole inside the bar, or the mode chip lost its word.
fn sweep_case(cx: &mut TestAppContext, label: &str, case: Case) -> Vec<String> {
    let width = Rc::new(Cell::new(700.0_f32));
    let w = width.clone();
    let c = case.clone();
    let (_, vcx) = cx.add_window_view(move |_, _| CaseHost { width: w, case: c });
    let mut failures = Vec::new();
    let mut widths: Vec<f32> = (0..)
        .map(|k| 700.0 - 11.0 * k as f32)
        .take_while(|w| *w > SPLIT_FLOOR_PX)
        .collect();
    widths.push(SPLIT_FLOOR_PX);
    for px_now in widths {
        width.set(px_now);
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let at = format!("{label} run={:?} {px_now}", case.run);
        let Some(bar) = vcx.debug_bounds("toolbar-console-toolbar") else {
            failures.push(format!("{at}: no console toolbar"));
            continue;
        };
        let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
        // N1: the narrow Run (or Stop) draws no label; the narrow bar keeps 8 px on its right.
        let narrow = !frame
            .iter()
            .any(|r| r.path.ends_with("#button-label-console-run") && r.visible);
        let right = if narrow { NARROW_RIGHT } else { INNER_RIGHT };
        let inside = |b: Bounds<Pixels>| {
            b.left() >= bar.left()
                && b.right() <= bar.right() - px(right) + px(0.5)
                && b.top() >= bar.top()
                && b.bottom() <= bar.bottom()
        };
        let mut boxes = vec!["console-env-swatch"];
        if case.read_only.is_some() && !case.pending {
            boxes.push("console-lock");
        }
        if case.pending {
            boxes.push("console-ro-pending");
        }
        if case.console.is_some() {
            boxes.push("tx-chip");
        }
        // ## m3c-t41-chrome (fix round 1, m2): the badge keeps Commit and Roll back (N3), and
        // the shared-session chip (a trust item, T22 fix round 1 I2) stays whole.
        if case.tx {
            boxes.extend([
                "tx-badge",
                "button-tx-badge-commit",
                "button-tx-badge-rollback",
            ]);
        }
        if case.shared {
            boxes.push("tx-shared");
        }
        for id in boxes {
            match vcx.debug_bounds(id) {
                Some(b) if inside(b) => {}
                other => failures.push(format!("{at}: {id} {other:?} not inside {bar:?}")),
            }
        }
        let mut texts: Vec<&str> = Vec::new();
        if case.read_only.is_some() && !case.pending {
            texts.push("#console-lock-label");
        }
        if case.pending {
            texts.push("#console-ro-pending-text");
        }
        if case.waiting {
            texts.push("#console-waiting-text");
            // F2 review N2: the wait's Cancel never hides: whole inside the bar, its word whole.
            // Only where its word is wider than the pane (a pseudo-locale at the split floor)
            // it is its glyph, the word its tooltip and name (as N1 draws Run and Stop): in a
            // real locale the word is always drawn.
            match vcx.debug_bounds("button-console-waiting-cancel") {
                Some(b) if inside(b) => {}
                other => failures.push(format!("{at}: Cancel {other:?} not inside {bar:?}")),
            }
            // ## m3c-t41-chrome (fix round 1, m1): the glyph only where the measured word (with
            // the button's 9 px padding a side and its border) does not fit the waiting item.
            let slot = vcx.debug_bounds("console-waiting");
            let word = cancel_word_px(vcx);
            let fits =
                !narrow || slot.is_some_and(|slot| word + 20.0 <= f32::from(slot.size.width));
            if fits {
                texts.push("#button-label-console-waiting-cancel");
            }
        }
        match case.run {
            RunUx::Running(_) => texts.push("#console-clock"),
            RunUx::Cancelling => texts.push("#console-cancelling"),
            RunUx::Idle => {}
        }
        if case.console.is_some() {
            texts.push("#tx-chip-mode");
            if case.tx {
                texts.push("#tx-badge-text");
            }
        }
        for id in texts {
            let found: Vec<_> = frame.iter().filter(|r| r.path.ends_with(id)).collect();
            if found.is_empty() {
                failures.push(format!("{at}: {id} not drawn"));
            }
            for r in found {
                if r.clipped || r.truncated || !r.visible || r.painted_px <= 0.0 {
                    failures.push(format!("{at}: {r:?} cut"));
                }
            }
        }
        // T2b C1 / N4: the mode chip keeps its mode word at every width.
        if let Some(console) = case.console {
            let words = vcx.read(|cx| chip_words(cx, console));
            let word = frame
                .iter()
                .find(|r| r.path.ends_with("#tx-chip-mode"))
                .map(|r| r.text.to_string());
            if !word.as_ref().is_some_and(|w| words.contains(w)) {
                failures.push(format!(
                    "{at}: the mode chip shows {word:?}, not one of {words:?}"
                ));
            }
        }
    }
    failures
}

/// Every state of the console toolbar in each of `langs`, down to the split floor, with a
/// console in a user transaction (the mode chip and the badge) and one whose session is
/// read-only (the disabled "Auto-commit" chip and the guarded lock chip).
fn sweep_locales(cx: &mut TestAppContext, langs: &[&str]) -> Vec<String> {
    let vcx = shell_with_fake(cx, langs[0], pg().tracks_transactions());
    add_profile(
        vcx,
        test_profile(PROD, Some(EnvTag::Production), "10.0.4.13"),
    );
    connect(vcx, PROD);
    wait_until(vcx, "the connect to finish", |vcx| {
        matches!(live(vcx, PROD), LiveState::Connected(_))
    });
    let id = profile_id(vcx, PROD);
    let profile = vcx.cx.read(|cx| {
        (*cx.global::<ConnectionsModel>()
            .get(id)
            .expect("the profile"))
        .clone()
    });
    let rt = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().core_runtime().handle());
    let session = |profile| {
        futures::executor::block_on(hacivat_session::testing::fake_session(
            rt.clone(),
            pg().tracks_transactions(),
            profile,
        ))
        .0
    };
    let rw = session(profile.clone());
    let rw_profile = profile.clone();
    let mut ro_profile = profile;
    ro_profile.read_only = true;
    let ro = session(ro_profile);
    let rw_console = register_test_console(vcx, id, rw.clone());
    let ro_console = register_test_console(vcx, id, ro);
    // ## m3c-t41-chrome (fix round 1, m2): two consoles over one session share it.
    let shared_session = session(rw_profile.clone());
    let shared_console = register_test_console(vcx, id, shared_session.clone());
    let _twin = register_test_console(vcx, id, shared_session);
    // The shell draws its Settings tab, so the sweep's window is the only console toolbar.
    with_ws(vcx, |ws, window, cx| {
        ws.open_registered_tab("settings", window, cx)
    });
    let ticket = rw.submit(Request::Exec {
        sql: "BEGIN".to_string(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::User,
    });
    let _ = futures::executor::block_on(ticket.reply).expect("the actor replies");
    wait_until(vcx, "the mirrors", |vcx| {
        vcx.cx.read(|cx| {
            tx_badge::user_tx(rw_console, cx).is_some()
                && tx_badge::view(ro_console, cx).is_some()
                && tx_badge::shares_session(shared_console, cx)
        })
    });
    let mut failures = Vec::new();
    for lang in langs {
        support::switch_to(vcx, lang);
        vcx.run_until_parked();
        let guarded = vcx
            .cx
            .read(|cx| hacivat_app::l10n_global::tr_id(cx, "hv-conn-access-ro-guarded"));
        let guarded: &'static str = Box::leak(guarded.to_string().into_boxed_str());
        for run in RUNS {
            for (waiting, pending) in [(false, false), (true, false), (false, true), (true, true)] {
                let case = Case {
                    run,
                    waiting,
                    pending,
                    read_only: None,
                    console: Some(rw_console),
                    tx: true,
                    shared: false,
                };
                failures.extend(sweep_case(cx, lang, case));
            }
            let case = Case {
                run,
                waiting: false,
                pending: false,
                read_only: Some(guarded),
                console: Some(ro_console),
                tx: false,
                shared: false,
            };
            failures.extend(sweep_case(cx, lang, case));
            // ## m3c-t41-chrome (fix round 1, m2)
            let case = Case {
                run,
                waiting: true,
                pending: false,
                read_only: None,
                console: Some(shared_console),
                tx: false,
                shared: true,
            };
            failures.extend(sweep_case(cx, lang, case));
        }
    }
    stop_core_runtime_test(cx);
    failures
}

#[gpui::test]
fn trust_items_whole_down_to_120px_in_every_locale(cx: &mut TestAppContext) {
    #[cfg(not(feature = "pseudo"))]
    let langs = ["en", "tr"];
    #[cfg(feature = "pseudo")]
    let langs = ["en", "tr", "en-XA", "tr-XT"];
    let failures = sweep_locales(cx, &langs);
    assert!(failures.is_empty(), "{failures:#?}");
}

// F2 review N4: the results strip's "Cursor open" (a NOTES #7 cursor signal) stays whole in a
// narrow pane, in every locale.

struct StripHost {
    width: Rc<Cell<f32>>,
}

impl Render for StripHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let none: hacivat_app::results::toolbar::Action = Rc::new(|_, _| {});
        let actions = StripActions {
            close_cursor: none.clone(),
            fetch_more: none.clone(),
            fetch_all: none.clone(),
            fetch_anyway: none.clone(),
            rerun: none,
        };
        let model = StripModel {
            kind: StripKind::Suspended { rows: 10_000 },
            more_sets: None,
        };
        div()
            .w(px(self.width.get()))
            .overflow_hidden()
            .child(strip::render(model, None, actions, cx))
    }
}

fn sweep_strip(cx: &mut TestAppContext, lang: &str) -> Vec<String> {
    let _shell = shell(cx, lang);
    let width = Rc::new(Cell::new(700.0_f32));
    let w = width.clone();
    let (_, vcx) = cx.add_window_view(move |_, _| StripHost { width: w });
    let mut failures = Vec::new();
    let mut widths: Vec<f32> = (0..)
        .map(|k| 700.0 - 11.0 * k as f32)
        .take_while(|w| *w > SPLIT_FLOOR_PX)
        .collect();
    widths.push(SPLIT_FLOOR_PX);
    for px_now in widths {
        width.set(px_now);
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let bar = vcx
            .debug_bounds("toolbar-results-strip")
            .expect("the strip");
        let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
        let cursor: Vec<_> = frame
            .iter()
            .filter(|r| r.path.ends_with("#strip-cursor"))
            .collect();
        if cursor.is_empty() {
            failures.push(format!("{lang} {px_now}: \"Cursor open\" not drawn"));
        }
        for r in cursor {
            if r.clipped || r.truncated || !r.visible || r.painted_px <= 0.0 {
                failures.push(format!("{lang} {px_now}: {r:?} cut in {bar:?}"));
            }
        }
    }
    failures
}

#[gpui::test]
fn the_strips_cursor_open_stays_whole_in_a_narrow_pane(cx: &mut TestAppContext) {
    let mut failures = sweep_strip(cx, "en");
    failures.extend(sweep_strip(cx, "tr"));
    #[cfg(feature = "pseudo")]
    {
        failures.extend(sweep_strip(cx, "en-XA"));
        failures.extend(sweep_strip(cx, "tr-XT"));
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ## m4-m3carry (U-6 a; T41 carry, M3c-34, design/m3b-safe surface 24): the strip's narrow
// form. "Fetch more" is never cut: in the strip's narrow stage it is its arrow-down icon, with
// its word as the tooltip and the accessible name.

/// Sweeps the suspended strip from 700 px down to the split floor in `lang`; returns the
/// failures and whether the icon form was reached.
fn sweep_fetch_more(cx: &mut TestAppContext, lang: &str) -> (Vec<String>, bool) {
    let _shell = shell(cx, lang);
    let width = Rc::new(Cell::new(700.0_f32));
    let w = width.clone();
    let (_, vcx) = cx.add_window_view(move |_, _| StripHost { width: w });
    let label = |vcx: &gpui::VisualTestContext| {
        let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
        frame
            .into_iter()
            .find(|r| r.path.ends_with("#button-label-strip-fetch-more"))
    };
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    // The wide strip draws the word; it names the icon form.
    let word = label(vcx)
        .expect("the wide strip draws \"Fetch more\"")
        .text;
    let mut failures = Vec::new();
    let mut iconified = false;
    let mut widths: Vec<f32> = (0..)
        .map(|k| 700.0 - 7.0 * k as f32)
        .take_while(|w| *w > SPLIT_FLOOR_PX)
        .collect();
    widths.push(SPLIT_FLOOR_PX);
    for px_now in widths {
        width.set(px_now);
        hacivat_ui::popover::a11y_record();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let decls = hacivat_ui::popover::a11y_take();
        let strip = vcx
            .debug_bounds("toolbar-results-strip")
            .expect("the strip");
        let Some(button) = vcx.debug_bounds("button-strip-fetch-more") else {
            failures.push(format!("{lang} {px_now}: Fetch more not drawn"));
            continue;
        };
        if button.left() < strip.left() || button.right() > strip.right() + px(0.5) {
            failures.push(format!(
                "{lang} {px_now}: Fetch more {button:?} past {strip:?}"
            ));
        }
        match label(vcx) {
            Some(r) => {
                if r.clipped || r.truncated || !r.visible || r.painted_px <= 0.0 {
                    failures.push(format!("{lang} {px_now}: {r:?} cut"));
                }
            }
            None => {
                iconified = true;
                let named = decls
                    .iter()
                    .any(|d| d.role == Some(gpui::Role::Button) && d.name == word);
                if !named {
                    failures.push(format!(
                        "{lang} {px_now}: the icon form is not named {word:?}: {decls:?}"
                    ));
                }
            }
        }
    }
    // ## m4-m3carry (fix round 1, m6): at the floor the icon's tooltip says its word. Hover it
    // and let the tooltip's delay run on the executor clock.
    if iconified && label(vcx).is_none() {
        let at = vcx
            .debug_bounds("button-strip-fetch-more")
            .expect("drawn")
            .center();
        vcx.simulate_mouse_move(at, None, gpui::Modifiers::none());
        let mut tip = false;
        for _ in 0..40 {
            vcx.executor()
                .advance_clock(std::time::Duration::from_millis(50));
            vcx.update(|window, _| window.refresh());
            vcx.run_until_parked();
            let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
            if frame
                .iter()
                .any(|r| r.path.contains("hacivat-ui/src/popover.rs") && r.text == word)
            {
                tip = true;
                break;
            }
        }
        if !tip {
            failures.push(format!("{lang}: the icon's tooltip does not say {word:?}"));
        }
    }
    (failures, iconified)
}

#[gpui::test]
fn the_strips_fetch_more_takes_its_icon_and_is_never_cut(cx: &mut TestAppContext) {
    let mut failures = Vec::new();
    let mut reached = Vec::new();
    let mut locales = vec!["en", "tr"];
    #[cfg(feature = "pseudo")]
    locales.extend(["en-XA", "tr-XT"]);
    for lang in locales {
        let (f, iconified) = sweep_fetch_more(cx, lang);
        failures.extend(f);
        if iconified {
            reached.push(lang);
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(
        !reached.is_empty(),
        "no locale reached the strip's narrow stage (the sweep proves nothing)"
    );
}

// ## m3c-t41-chrome (fix round 1; PO M3c-18): in a narrowing result toolbar the view items
// (Task 38's view switch and record toggle, here a pinned stand-in with a compact form at their
// priority) take their compact forms first; Count rows (priority 1) moves before Copy; Copy
// gives way last.

struct ResultBarHost {
    width: Rc<Cell<f32>>,
}

impl Render for ResultBarHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use hacivat_app::results::toolbar::{VIEW_ITEMS_PRIORITY, result_toolbar};
        use hacivat_ui::toolbar::ToolbarItem;
        let none: hacivat_app::results::toolbar::Action = Rc::new(|_, _| {});
        let word = |text: &'static str| -> hacivat_ui::toolbar::ItemFn {
            Rc::new(move |_, cx| {
                let t = hacivat_ui_core::theme::Theme::get(cx).tokens;
                hacivat_ui::text::Label::new(
                    text,
                    hacivat_ui::text::TextSource::Fluent,
                    t.type_ramp.body,
                    t.colors.text_2.to_hsla(),
                )
                .into_any_element()
            })
        };
        let count = ToolbarItem::new(1, word("Count rows"), Rc::new(|_| "Count rows".into()))
            .on_choose(|_, _| {});
        let view = ToolbarItem::pinned(
            word("Table · Transposed · Text"),
            Rc::new(|_| "View".into()),
        )
        .priority(VIEW_ITEMS_PRIORITY)
        .compact(word("▦"));
        div()
            .w(px(self.width.get()))
            .overflow_hidden()
            .child(result_toolbar(
                "10,000+ rows · 131 ms".into(),
                none,
                vec![count, view],
                cx,
            ))
    }
}

#[gpui::test]
fn copy_gives_way_last_in_a_narrow_result_toolbar(cx: &mut TestAppContext) {
    use hacivat_ui::toolbar::ToolbarProbe;
    let _shell = shell(cx, "en");
    let width = Rc::new(Cell::new(900.0_f32));
    let w = width.clone();
    let (_, vcx) = cx.add_window_view(move |_, _| ResultBarHost { width: w });
    vcx.update(|_, cx| cx.set_global(ToolbarProbe::default()));
    // Items: Copy 0, Count rows 1, the view items 2, rows · timing 3.
    let (mut count_moved, mut view_compact, mut copy_moved) = (false, false, false);
    for px_now in (180..=900).rev().step_by(4) {
        width.set(px_now as f32);
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let staged = vcx
            .read(|cx| cx.global::<ToolbarProbe>().staged.clone())
            .expect("a frame");
        let (count, view, copy) = (
            staged.fit.overflow.contains(&1),
            staged.compact.contains(&2),
            staged.fit.overflow.contains(&0),
        );
        assert!(
            !view || count,
            "{px_now}: the view items compacted before Count rows moved"
        );
        assert!(
            !copy || view,
            "{px_now}: Copy moved before the view items compacted"
        );
        count_moved |= count;
        view_compact |= view;
        copy_moved |= copy;
    }
    assert!(
        count_moved && view_compact && copy_moved,
        "the sweep reached every stage"
    );
}

// ## m3c-t41-chrome (fix round 1, m4): the narrow lock chip's lock is red on production (NOTES
// surface 24, Hue Is Meaning), `text-2` elsewhere.
#[gpui::test]
fn the_narrow_lock_is_red_on_production(cx: &mut TestAppContext) {
    use hacivat_ui::icon::IconName;
    let _shell = shell(cx, "en");
    for production in [true, false] {
        let width = Rc::new(Cell::new(SPLIT_FLOOR_PX));
        let w = width.clone();
        // The guarded label is wider than a 120 px pane's row: the chip takes its narrow form.
        let mut m = model(production, true, false);
        m.read_only = Some("read-only (guarded)".into());
        let (_, vcx) = cx.add_window_view(move |_, _| Host { width: w, model: m });
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("console-lock-glyph").is_some(),
            "the narrow lock chip draws its glyph"
        );
        hacivat_ui::icon::icon_record();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let locks: Vec<gpui::Hsla> = hacivat_ui::icon::icon_take()
            .into_iter()
            .filter(|i| i.name == IconName::Lock)
            .map(|i| i.color)
            .collect();
        let (red, text_2) = vcx.read(|cx| {
            let c = hacivat_ui_core::theme::Theme::get(cx).tokens.colors;
            (c.red.to_hsla(), c.text_2.to_hsla())
        });
        // The bar also lays out the chip's full form (its `text-2` lock) to measure it; the
        // narrow form drawn in the pane is the red one on production, and only there.
        assert!(locks.contains(&text_2), "{locks:?}");
        assert_eq!(
            locks.contains(&red),
            production,
            "production={production}: {locks:?}"
        );
        drop(width);
    }
}
