//! Results panel: tabs, toolbar, the result grid with its scroll track and the next-page strip.

use hacivat_ui_core::gpui_surface::*;

use super::data::{C1, Env, R1, SEG1};
use super::explorer::panel_header;
use super::inspector::inspector;
use super::ui::{Ui, alpha, col, edge_b, edge_r, row};

const ROW_H: f32 = 21.0;
const HEAD_H: f32 = 36.0;

pub fn results(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    col()
        .size_full()
        .bg(c.panel.to_hsla())
        .child(header(ui))
        .child(toolbar(ui))
        .child(
            row()
                .w_full()
                .flex_1()
                .min_h_0()
                .items_start()
                .child(
                    col()
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .child(grid_wrap(ui))
                        .child(next_page_strip(ui)),
                )
                .child(inspector(ui)),
        )
}

fn header(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    let tab = |first: bool, on: bool| {
        let mut d = row()
            .h(px(30.0))
            .flex_none()
            .px(px(12.0))
            .gap(px(7.0))
            .border_r_1()
            .border_color(c.seam.to_hsla());
        if first {
            d = d.border_l_1();
        }
        if on {
            d = d.bg(c.raised.to_hsla());
        }
        d
    };
    let ink = |on: bool| if on { c.text } else { c.text_2 };
    let tabs = row()
        .h_full()
        .items_start()
        .child(
            tab(true, true)
                .child(ui.i12("table-2", ink(true)))
                .child(ui.txt(ui.sans(13.0, 400), ink(true), ui.s("result1")))
                .child(ui.txt(ui.mono(11.0), c.text_3, "stmt 1")),
        )
        .child(
            tab(false, false)
                .child(ui.i12("table-2", ink(false)))
                .child(ui.txt(ui.mono(12.0), ink(false), "orders"))
                .child(ui.swatch(Env::Stg))
                .child(ui.txt(ui.sans(13.0, 400), c.text_2, ui.s("env.stagingParen")))
                .child(
                    row()
                        .gap(px(4.0))
                        .child(div().size(px(7.0)).bg(c.amber.to_hsla()))
                        .child(ui.txt(ui.sans(11.5, 400), c.amber, "1")),
                ),
        );
    let meta = row()
        .gap(px(10.0))
        .child(
            row()
                .gap(px(6.0))
                .child(ui.i12("lock", c.red))
                .child(ui.txt(ui.sans(12.5, 400), c.text_2, ui.s("roNote"))),
        )
        .child(ui.vsep(12.0))
        .child(ui.txt(ui.mono(11.0), c.text_3, format!("100 {} · 42 ms", ui.s("rowsWord"))));
    panel_header(ui)
        .gap(px(12.0))
        .child(tabs)
        .child(div().flex_1())
        .child(meta)
        .child(ui.kbd("⌘3", c.text_3))
}

fn toolbar(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    let qbg = if ui.light() { c.ground } else { c.panel };
    let qbox = || {
        row()
            .h(px(24.0))
            .flex_none()
            .px(px(8.0))
            .gap(px(7.0))
            .border_1()
            .border_color(c.seam_strong.to_hsla())
            .rounded(px(4.0))
            .bg(qbg.to_hsla())
    };
    let where_box = qbox()
        .w(px(320.0))
        .child(ui.i12("filter", c.text_3))
        .child(ui.txt(ui.mono(11.5), c.syn_keyword, "WHERE"))
        // The mock's <input> carries ~2 px of default padding.
        .child(div().ml(px(2.0)).child(ui.txt(ui.mono(12.0), c.text_3, "city = 'İstanbul'")));
    let order_box = qbox()
        .child(ui.txt(ui.mono(11.5), c.syn_keyword, "ORDER BY"))
        .child(ui.txt(ui.mono(12.0), c.text, "revenue"))
        .child(ui.txt(ui.mono(11.5), c.syn_keyword, "DESC"))
        .child(ui.i12("arrow-down", c.blue));
    let transparent = hsla(0.0, 0.0, 0.0, 0.0);
    let ghost = |pad: f32| {
        row().h(px(24.0)).flex_none().px(px(pad)).gap(px(6.0)).border_1().border_color(transparent).rounded(px(4.0))
    };
    let pager = row()
        .ml(px(4.0))
        .gap(px(2.0))
        .child(ghost(6.0).child(ui.i12("chevron-left", c.text_3)))
        .child(
            row()
                .px(px(4.0))
                .child(ui.run(ui.mono(12.5), c.text.to_hsla()).font_weight(FontWeight(600.0)).child("1–100"))
                .child(ui.txt(ui.mono(12.5), c.text_2, format!(" {} {}", ui.s("of"), ui.nf(2481.0, 0)))),
        )
        .child(ghost(6.0).child(ui.i12("chevron-right", c.text_2)));
    let count = ghost(9.0)
        .child(ui.i12("sigma", c.text_2))
        .child(ui.txt(ui.sans(12.5, 500), c.text_2, ui.s("countRows")));
    let copy = ghost(6.0).child(ui.i12("copy", c.text_2));
    let export = row()
        .h(px(24.0))
        .flex_none()
        .px(px(9.0))
        .gap(px(6.0))
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .rounded(px(4.0))
        .child(ui.i12("download", c.text))
        .child(ui.txt(ui.sans(12.5, 500), c.text, ui.s("export")))
        .child(ui.i12("chevron-down", c.text_3));
    let danger = row()
        .h(px(20.0))
        .flex_none()
        .ml(px(28.0))
        .pl(px(12.0))
        .border_l_1()
        .border_color(c.seam.to_hsla())
        .child(
            row()
                .h(px(24.0))
                .px(px(9.0))
                .gap(px(6.0))
                .border_1()
                .border_color(c.seam_strong.to_hsla())
                .rounded(px(4.0))
                .child(ui.i12("trash-2", c.text_3))
                .child(ui.txt(ui.sans(12.5, 500), c.text_3, ui.s("deleteRows"))),
        );
    row()
        .w_full()
        .h(px(34.0))
        .flex_none()
        .pl(px(12.0))
        .pr(px(10.0))
        .gap(px(6.0))
        .border_b_1()
        .border_color(c.seam.to_hsla())
        .bg(c.raised.to_hsla())
        .child(where_box)
        .child(order_box)
        .child(pager)
        .child(div().w(px(1.0)).h(px(16.0)).mx(px(4.0)).flex_none().bg(c.seam.to_hsla()))
        .child(count)
        .child(copy)
        .child(export)
        .child(div().flex_1())
        .child(danger)
}

fn grid_wrap(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    let scroll = div()
        .absolute()
        .right_0()
        .top(px(37.0))
        .bottom_0()
        .w(px(9.0))
        .border_l_1()
        .border_color(c.seam.to_hsla())
        .bg(c.gutter.to_hsla())
        .child(
            div().absolute().left(px(2.0)).right(px(2.0)).top(px(2.0)).h(px(26.0)).rounded(px(3.0)).bg(alpha(c.text_3, 0.6)),
        )
        .child(div().absolute().left(px(3.0)).right(px(3.0)).top(px(30.0)).h(px(24.0)).bg(alpha(c.green_fill, 0.6)));
    div()
        .relative()
        .w_full()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .child(grid(ui))
        .child(scroll)
}

fn grid(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    let cell = |w: Option<f32>, d: Div| {
        let d = d
            .relative()
            .h(px(ROW_H))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .px(px(10.0))
            .child(edge_r(c.seam_soft.to_hsla()))
            .child(edge_b(c.seam_soft.to_hsla()));
        match w {
            Some(w) => d.w(px(w)),
            None => d.flex_1().min_w_0(),
        }
    };
    let th = |w: Option<f32>, name: &str, ty: &str, num: bool, sort: bool, last: bool| {
        let mut d = col()
            .relative()
            .h(px(HEAD_H))
            .flex_none()
            .justify_center()
            .px(px(10.0))
            .bg(c.head.to_hsla())
            .child(edge_b(c.seam_strong.to_hsla()));
        if !last {
            d = d.child(edge_r(c.seam.to_hsla()));
        }
        d = match w {
            Some(w) => d.w(px(w)),
            None => d.flex_1().min_w_0(),
        };
        if num {
            d = d.items_end();
        }
        if name.is_empty() {
            return d;
        }
        let mut hn = row().gap(px(6.0)).h(px(15.0));
        if sort {
            hn = hn.child(
                row()
                    .gap(px(1.0))
                    .child(ui.i12("arrow-down", c.blue))
                    .child(ui.txt(ui.mono(11.0), c.blue, "1")),
            );
        }
        hn = hn.child(ui.txt(ui.sans(13.0, 600), c.text, name.to_string()));
        d.child(hn).child(
            div().mt(px(1.0)).child(ui.txt(
                hacivat_ui_core::theme::TypeStyle { line_height: 13.0, ..ui.mono(11.0) },
                c.text_3,
                ty.to_string(),
            )),
        )
    };
    // Header: the row-number column has a seam border (not seam_soft) and the head fill.
    let head = row()
        .w_full()
        .h(px(HEAD_H))
        .flex_none()
        .items_start()
        .child(th(Some(C1[0]), "", "", false, false, false))
        .child(th(Some(C1[1]), "full_name", "text", false, false, false))
        .child(th(Some(C1[2]), "city", "text", false, false, false))
        .child(th(Some(C1[3]), "segment", "text", false, false, false))
        .child(th(Some(C1[4]), "orders", "bigint", true, false, false))
        .child(th(Some(C1[5]), "revenue", "numeric(14,2)", true, true, false))
        .child(th(None, "", "", false, false, true));
    let mut body = col().w_full();
    for (i, r) in R1.iter().enumerate() {
        let sel = (2..=4).contains(&i);
        let rn = cell(Some(C1[0]), div())
            .justify_end()
            .pr(px(9.0))
            .child(edge_r(c.seam.to_hsla()))
            .bg(c.gutter.to_hsla())
            .child(ui.txt(ui.sans(11.5, 400), if sel { c.blue } else { c.text_3 }, (i + 1).to_string()));
        let t = |s: &str| ui.txt(ui.sans(13.0, 400), c.text, s.to_string());
        let city = match r.1 {
            None => ui.sc(13.0, 600, 0.08, c.text_3, "null").into_any_element(),
            Some("") => ui.txt(ui.mono(11.5), c.text_3, "''").into_any_element(),
            Some(s) => t(s).into_any_element(),
        };
        let mut orders = cell(Some(C1[4]), div()).justify_end().child(t(&r.2.to_string()));
        let mut revenue = cell(Some(C1[5]), div()).justify_end().child(t(&ui.nf(r.3, 2)));
        if sel {
            orders = orders.bg(c.blue_tint.to_hsla());
            revenue = revenue.bg(if i == 2 { c.blue_tint_strong } else { c.blue_tint }.to_hsla());
        }
        body = body.child(
            row()
                .w_full()
                .h(px(ROW_H))
                .flex_none()
                .child(rn)
                .child(cell(Some(C1[1]), div()).child(t(r.0)))
                .child(cell(Some(C1[2]), div()).child(city))
                .child(cell(Some(C1[3]), div()).child(t(SEG1[i])))
                .child(orders)
                .child(revenue)
                .child(cell(None, div()).px(px(6.0))),
        );
    }
    // Selection box over cells (3 × 2): orders col starts at x = 46 + 232 + 180 + 132.
    let sel_x = C1[0] + C1[1] + C1[2] + C1[3];
    let sel_w = C1[4] + C1[5];
    let selbox = div()
        .absolute()
        .left(px(sel_x - 1.0))
        .top(px(HEAD_H + ROW_H * 2.0 - 1.0))
        .w(px(sel_w + 1.0))
        .h(px(ROW_H * 3.0 + 1.0))
        .border_1()
        .border_color(c.blue.to_hsla())
        .child(
            div()
                .absolute()
                .right(px(-4.0))
                .bottom(px(-4.0))
                .size(px(5.0))
                .bg(c.blue.to_hsla())
                .border_1()
                .border_color(c.panel.to_hsla()),
        );
    // `.rview` leaves 9 px on the right for the scroll track.
    div()
        .absolute()
        .left_0()
        .top_0()
        .bottom_0()
        .right(px(9.0))
        .overflow_hidden()
        .child(col().w_full().child(head).child(body))
        .child(selbox)
}

fn next_page_strip(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    // `.gfoot` sits on a raised surface: tertiary ink steps up to text-2.
    let d = c.text_2;
    let mut pages = row()
        .h(px(8.0))
        .flex_none()
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .rounded(px(3.0))
        .overflow_hidden();
    for i in 0..25 {
        let mut p = div().w(px(6.0)).h_full().flex_none();
        if i > 0 {
            p = p.border_l_1().border_color(c.seam.to_hsla());
        }
        pages = pages.child(match i {
            0 => p.bg(c.blue.to_hsla()),
            1 => p.bg(c.green_fill.to_hsla()),
            _ => p,
        });
    }
    row()
        .w_full()
        .h(px(26.0))
        .flex_none()
        .pl(px(12.0))
        .pr(px(10.0))
        .gap(px(8.0))
        .border_t_1()
        .border_color(c.seam.to_hsla())
        .bg(c.raised.to_hsla())
        .child(pages)
        .child(
            row()
                .gap(px(5.0))
                .child(ui.i12("check", c.green))
                .child(ui.txt(ui.sans(12.0, 400), c.green, ui.s("nextReady"))),
        )
        .child(ui.txt(ui.mono(11.5), c.text, "101–200"))
        .child(ui.txt(ui.sans(12.0, 400), d, ui.s("prefetched")))
        .child(div().flex_1())
        .child(ui.txt(ui.sans(12.0, 400), d, ui.s("selection")))
        .child(ui.txt(ui.mono(11.5), c.text, "3 × 2"))
        .child(ui.vsep(12.0))
        .child(ui.txt(ui.sans(12.0, 400), d, "Σ"))
        .child(ui.txt(ui.mono(11.5), c.text_2, "revenue"))
        .child(ui.run(ui.mono(11.5), c.text.to_hsla()).font_weight(FontWeight(600.0)).child(ui.nf(110190.85, 2)))
}
