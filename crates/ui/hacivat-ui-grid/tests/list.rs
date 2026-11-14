//! `VirtualList` (Task 20): only the visible rows are built, scrolling builds the rows it
//! reveals, appended rows are followed at the bottom. The target builds only with
//! `--features test-support`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    AnyElement, App, Entity, IntoElement, ParentElement, Styled, TestAppContext, VisualTestContext,
    Window, div, px, size,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_grid::{RowRenderer, VirtualList};

/// `len` rows; records which rows it was asked for.
struct Lines {
    len: u64,
    built: Rc<RefCell<Vec<u64>>>,
    calls: Rc<Cell<u64>>,
}

impl RowRenderer for Lines {
    fn len(&self) -> u64 {
        self.len
    }

    fn row(&self, ix: u64, _window: &mut Window, _cx: &mut App) -> AnyElement {
        self.calls.set(self.calls.get() + 1);
        self.built.borrow_mut().push(ix);
        div()
            .h(px(18.0))
            .w_full()
            .child(format!("line {ix}"))
            .into_any_element()
    }
}

type Built = Rc<RefCell<Vec<u64>>>;

fn open(
    cx: &mut TestAppContext,
    len: u64,
) -> (
    Entity<VirtualList<Lines>>,
    &mut VisualTestContext,
    Built,
    Rc<Cell<u64>>,
) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let built: Built = Rc::default();
    let calls: Rc<Cell<u64>> = Rc::default();
    let rows = Lines {
        len,
        built: built.clone(),
        calls: calls.clone(),
    };
    let (view, vcx) = cx.add_window_view(move |_, _| VirtualList::new(rows, 18.0));
    vcx.simulate_resize(size(px(400.0), px(180.0)));
    vcx.run_until_parked();
    (view, vcx, built, calls)
}

fn last_frame(built: &Built, vcx: &mut VisualTestContext) -> Vec<u64> {
    built.borrow_mut().clear();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    built.borrow().clone()
}

#[gpui::test]
fn a_million_rows_build_only_the_visible_ones(cx: &mut TestAppContext) {
    let (view, vcx, built, _) = open(cx, 1_000_000);
    assert_eq!(last_frame(&built, vcx), (0..10).collect::<Vec<_>>());
    assert_eq!(view.read_with(vcx, |l, _| l.visible_rows()), 0..10);
}

#[gpui::test]
fn scrolling_builds_the_rows_it_reveals(cx: &mut TestAppContext) {
    let (view, vcx, built, _) = open(cx, 1_000_000);
    vcx.update(|_, cx| view.update(cx, |l, cx| l.scroll_px(18.0 * 500_000.0 + 9.0, cx)));
    vcx.run_until_parked();
    let rows = view.read_with(vcx, |l, _| l.visible_rows());
    // Half a row into row 500,000: eleven rows intersect the 180 px viewport.
    assert_eq!(rows, 500_000..500_011);
    assert_eq!(
        last_frame(&built, vcx),
        (500_000..500_011).collect::<Vec<_>>()
    );
    vcx.update(|_, cx| view.update(cx, |l, cx| l.scroll_to_end(cx)));
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |l, _| l.visible_rows()),
        999_990..1_000_000
    );
}

#[gpui::test]
fn appended_rows_are_followed_at_the_bottom(cx: &mut TestAppContext) {
    let (view, vcx, built, _) = open(cx, 50);
    vcx.update(|_, cx| view.update(cx, |l, cx| l.scroll_to_end(cx)));
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |l, _| l.visible_rows()), 40..50);
    vcx.update(|_, cx| view.update(cx, |l, cx| l.rows_mut(cx).len = 75));
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |l, _| l.visible_rows()), 65..75);
    // Scrolled up, an append keeps the view where it is.
    vcx.update(|_, cx| view.update(cx, |l, cx| l.scroll_into_view(10, cx)));
    vcx.run_until_parked();
    let at = view.read_with(vcx, |l, _| l.visible_rows());
    assert!(at.contains(&10), "{at:?}");
    vcx.update(|_, cx| view.update(cx, |l, cx| l.rows_mut(cx).len = 100));
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |l, _| l.visible_rows()), at);
    assert!(!last_frame(&built, vcx).contains(&99));
}

#[gpui::test]
fn an_empty_list_builds_nothing(cx: &mut TestAppContext) {
    let (view, vcx, _, calls) = open(cx, 0);
    assert_eq!(calls.get(), 0);
    assert!(view.read_with(vcx, |l, _| l.rows().is_empty()));
}
