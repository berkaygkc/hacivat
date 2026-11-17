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
                r.path.ends_with("#console-lock-label") || r.path.ends_with("#console-waiting-text")
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
    let failures = sweep(cx, "en", MIN_EN_PX);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[gpui::test]
fn trust_items_stay_whole_in_a_narrow_pane_in_turkish(cx: &mut TestAppContext) {
    let failures = sweep(cx, "tr", MIN_TR_PX);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[cfg(feature = "pseudo")]
#[gpui::test]
fn trust_items_stay_whole_in_a_narrow_pane_in_the_pseudo_locales(cx: &mut TestAppContext) {
    let mut failures = sweep(cx, "en-XA", MIN_PSEUDO_PX);
    failures.extend(sweep(cx, "tr-XT", MIN_PSEUDO_PX));
    assert!(failures.is_empty(), "{failures:#?}");
}
