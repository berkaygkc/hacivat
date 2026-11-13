//! The paint-level One Filled Badge check counts `cp_fill` quads in the rendered scene (Task 8,
//! R12). Needs `--features test-support,scene-check`.
#![cfg(feature = "scene-check")]

use gpui::{
    Context, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext,
    VisualTestContext, Window, div, px,
};
use hacivat_ui_core::named_rules::{assert_one_filled_badge, count_solid_quads};
use hacivat_ui_core::theme::DARK;

/// Paints `blocks` solid `cp_fill` quads on a ground that is not `cp_fill`.
struct CpFillProbe {
    blocks: usize,
}

impl Render for CpFillProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let c = &DARK.colors;
        div()
            .size_full()
            .flex()
            .gap(px(4.))
            .bg(c.ground.to_hsla())
            .children((0..self.blocks).map(|_| div().w(px(40.)).h(px(26.)).bg(c.cp_fill.to_hsla())))
    }
}

fn draw<V: Render + 'static>(
    cx: &mut TestAppContext,
    build: impl FnOnce(&mut Window) -> V,
) -> (gpui::Entity<V>, &mut VisualTestContext) {
    cx.add_window_view(|window, _| build(window))
}

#[gpui::test]
fn one_cp_fill_quad_counts_one(cx: &mut TestAppContext) {
    let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 1 });
    vcx.update(|w, _| assert_eq!(count_solid_quads(w, DARK.colors.cp_fill.to_hsla()), 1));
}

#[gpui::test]
#[should_panic(expected = "cp-fill quads: 2")]
fn two_cp_fill_quads_fail(cx: &mut TestAppContext) {
    let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 2 });
    vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), true));
}

#[gpui::test]
fn no_production_means_zero_quads(cx: &mut TestAppContext) {
    let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 0 });
    vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), false));
}

#[gpui::test]
fn one_cp_fill_quad_with_production_holds(cx: &mut TestAppContext) {
    let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 1 });
    vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), true));
}

#[gpui::test]
#[should_panic(expected = "cp-fill quads: 1")]
fn a_cp_fill_quad_without_production_fails(cx: &mut TestAppContext) {
    let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 1 });
    vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), false));
}

#[gpui::test]
#[should_panic(expected = "cp-fill quads: 0")]
fn production_without_a_cp_fill_quad_fails(cx: &mut TestAppContext) {
    let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 0 });
    vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), true));
}

#[gpui::test]
fn other_colours_are_not_counted(cx: &mut TestAppContext) {
    let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 2 });
    vcx.update(|w, _| {
        assert_eq!(count_solid_quads(w, DARK.colors.ground.to_hsla()), 1);
        assert_eq!(count_solid_quads(w, DARK.colors.red.to_hsla()), 0);
    });
}

// ## m2a-t21-shots

use gpui::{Bounds, point, size};
use hacivat_ui_core::named_rules::{assert_one_filled_badge_in, filled_badges};

/// A 40 x 26 `cp_fill` block at the top left (the capsule's production block), then `marks`
/// 7 px `cp_fill` squares (the light theme's production marks) and `extra` more blocks.
struct ScopedProbe {
    marks: usize,
    extra: usize,
}

impl Render for ScopedProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let c = &DARK.colors;
        let block = || div().w(px(40.)).h(px(26.)).bg(c.cp_fill.to_hsla());
        div()
            .size_full()
            .flex()
            .items_start()
            .gap(px(4.))
            .bg(c.ground.to_hsla())
            .child(block())
            .children((0..self.marks).map(|_| div().size(px(7.)).bg(c.cp_fill.to_hsla())))
            .children((0..self.extra).map(|_| block()))
    }
}

/// The region of the first block.
fn capsule() -> Option<Bounds<gpui::Pixels>> {
    Some(Bounds::new(point(px(0.), px(0.)), size(px(40.), px(26.))))
}

#[gpui::test]
fn only_the_block_inside_the_capsule_counts(cx: &mut TestAppContext) {
    let (_, vcx) = draw(cx, |_| ScopedProbe { marks: 3, extra: 0 });
    vcx.update(|w, _| {
        assert_eq!(
            filled_badges(w, DARK.colors.cp_fill.to_hsla(), capsule()),
            (1, 0)
        );
        assert_one_filled_badge_in(w, DARK.colors.cp_fill.to_hsla(), true, capsule());
    });
}

#[gpui::test]
#[should_panic(expected = "badge-sized cp-fill quads outside it: 1")]
fn a_second_badge_outside_the_capsule_fails(cx: &mut TestAppContext) {
    let (_, vcx) = draw(cx, |_| ScopedProbe { marks: 0, extra: 1 });
    vcx.update(|w, _| {
        assert_one_filled_badge_in(w, DARK.colors.cp_fill.to_hsla(), true, capsule())
    });
}

#[gpui::test]
#[should_panic(expected = "cp-fill quads in the capsule: 0 (expected 1)")]
fn no_capsule_drawn_fails_when_production_is_expected(cx: &mut TestAppContext) {
    let (_, vcx) = draw(cx, |_| ScopedProbe { marks: 1, extra: 0 });
    vcx.update(|w, _| assert_one_filled_badge_in(w, DARK.colors.cp_fill.to_hsla(), true, None));
}
