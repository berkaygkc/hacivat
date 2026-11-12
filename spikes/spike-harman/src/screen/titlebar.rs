//! Title bar: native traffic lights (inset), brand, the connection capsule, palette, controls.

use hacivat_ui_core::gpui_surface::*;

use super::data::Lang;
use super::ui::{Ui, row};

/// Width reserved for the native traffic lights: the mock's 3 × 12 px lights, 2 × 8 px gaps and
/// its 18 px margin.
pub const LIGHTS_INSET: f32 = 52.0 + 18.0;

pub fn titlebar(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    row()
        .w_full()
        .h(px(40.0))
        .flex_none()
        .pl(px(14.0))
        .pr(px(10.0))
        .border_b_1()
        .border_color(c.seam.to_hsla())
        .bg(c.ground.to_hsla())
        .child(div().w(px(LIGHTS_INSET)).flex_none())
        .child(brand(ui))
        .child(ui.vsep(18.0))
        .child(capsule(ui).ml(px(12.0)))
        .child(div().flex_1())
        .child(palette(ui))
        .child(div().flex_1())
        .child(controls(ui))
}

fn brand(ui: &Ui) -> impl IntoElement {
    row()
        .gap(px(8.0))
        .pr(px(14.0))
        .child(ui.icon("hv-mark", 16.0, ui.c.text_2.to_hsla()))
        .child(ui.sc(14.5, 600, 0.09, ui.c.text, "Hacivat"))
}

fn capsule(ui: &Ui) -> Div {
    let c = &ui.c;
    let cell = |d: Div| {
        d.flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(9.0))
            .h_full()
            .flex_none()
            .border_l_1()
            .border_color(c.seam_strong.to_hsla())
    };
    let env = ui
        .cp_fill_quad()
        .flex()
        .flex_row()
        .items_center()
        .h_full()
        .flex_none()
        .pl(px(9.0))
        .pr(px(10.0))
        .rounded_tl(px(3.0))
        .rounded_bl(px(3.0))
        .child(ui.sc(13.5, 700, 0.09, c.on_cp_fill, ui.s("env.prodCap")));
    let name = cell(div())
        .child(ui.txt(ui.sans(13.0, 600), c.text, "orders-prod"))
        .child(ui.txt(ui.sans(13.0, 400), c.text_2, "PostgreSQL 16"));
    let ro = cell(div())
        .child(ui.i12("lock", c.red))
        .child(ui.txt(ui.sans(13.0, 400), c.text, ui.s("acc.ro")));
    let path = cell(div())
        .child(ui.i12("arrow-left-right", c.green))
        .child(ui.txt(ui.sans(13.0, 400), c.text, ui.s("path.directCap")));
    let chev = cell(div()).px(px(6.0)).child(ui.i12("chevrons-up-down", c.text_3));
    row()
        .h(px(26.0))
        .flex_none()
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .rounded(px(4.0))
        .overflow_hidden()
        .bg(c.panel.to_hsla())
        .child(env)
        .child(name)
        .child(ro)
        .child(path)
        .child(chev)
}

fn palette(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    row()
        .w(px(290.0))
        .h(px(26.0))
        .flex_none()
        .mr(px(14.0))
        .pl(px(9.0))
        .pr(px(6.0))
        .gap(px(8.0))
        .border_1()
        .border_color(c.seam.to_hsla())
        .rounded(px(4.0))
        .child(ui.i12("search", c.text_3))
        .child(div().flex_1().child(ui.txt(ui.sans(13.0, 400), c.text_3, ui.s("palette"))))
        .child(ui.kbd("⌘K", c.text_3))
}

fn controls(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    let seg = || {
        row()
            .h(px(26.0))
            .flex_none()
            .border_1()
            .border_color(c.seam_strong.to_hsla())
            .rounded(px(4.0))
            .overflow_hidden()
    };
    // A segmented-control button; `on` is the pressed state.
    let btn = |first: bool, on: bool, pad: f32| {
        let mut d = row().h_full().px(px(pad)).gap(px(6.0)).flex_none();
        if !first {
            d = d.border_l_1().border_color(c.seam_strong.to_hsla());
        }
        if on {
            d = d.bg(c.raised.to_hsla());
        }
        d
    };
    let ink = |on: bool| if on { c.text } else { c.text_2 };
    let view = seg()
        .child(
            btn(true, true, 9.0)
                .child(ui.i12("layout-panel-left", ink(true)))
                .child(ui.txt(ui.sans(13.0, 400), ink(true), ui.s("view.workspace"))),
        )
        .child(
            btn(false, false, 9.0)
                .child(ui.txt(ui.sans(13.0, 400), ink(false), ui.s("view.splash")))
                .child(ui.kbd("S", c.text_3)),
        );
    let tr = ui.lang == Lang::Tr;
    let mono_btn = |s: &str, on: bool| {
        let mut st = ui.mono(11.5);
        st.weight = if on { 600 } else { 400 };
        ui.txt(st, ink(on), s.to_string())
    };
    let lang = seg()
        .child(btn(true, !tr, 8.0).child(mono_btn("EN", !tr)))
        .child(btn(false, tr, 8.0).child(mono_btn("TR", tr)));
    let theme_dark = !ui.light();
    let theme = seg()
        .child(btn(true, theme_dark, 7.0).child(ui.i12("moon", ink(theme_dark))))
        .child(btn(false, !theme_dark, 7.0).child(ui.i12("sun", ink(!theme_dark))));
    row().gap(px(8.0)).child(view).child(lang).child(theme)
}
