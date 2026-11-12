//! Grid inspector: the selected record and range statistics.

use hacivat_ui_core::gpui_surface::*;
use hacivat_ui_core::theme::TypeStyle;

use super::ui::{Ui, col, row};

pub fn inspector(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    let ih = || {
        row()
            .w_full()
            .h(px(30.0))
            .flex_none()
            .px(px(12.0))
            .gap(px(8.0))
            .border_b_1()
            .border_color(c.seam.to_hsla())
    };
    let field = |name: &str, ty: &str, value: String, num: bool| {
        let fk = TypeStyle { line_height: 14.0, ..ui.mono(11.0) };
        let mut v = div().w_full().child(ui.txt(TypeStyle { line_height: 18.0, ..ui.sans(13.5, 400) }, c.text, value));
        if num {
            v = v.flex().justify_end();
        }
        col()
            .relative()
            .w_full()
            .flex_none()
            .pt(px(6.0))
            .pb(px(7.0))
            .px(px(12.0))
            .gap(px(2.0))
            .child(
                row()
                    .w_full()
                    .justify_between()
                    .child(ui.txt(fk, c.text_2, name.to_string()))
                    .child(ui.txt(fk, c.text_3, ty.to_string())),
            )
            .child(v)
            .child(super::ui::edge_b(if ui.light() { c.seam } else { c.seam_soft }.to_hsla()))
    };
    let stat = |label_prefix: &str, mono: &str, value: String| {
        let lb = row()
            .gap(px(3.0))
            .child(ui.txt(ui.sans(11.5, 400), c.text_3, label_prefix.to_string()))
            .child(ui.txt(TypeStyle { line_height: 15.0, ..ui.mono(11.0) }, c.text_3, mono.to_string()));
        col()
            .flex_1()
            .min_w_0()
            .gap(px(2.0))
            .child(lb)
            .child(ui.txt(TypeStyle { line_height: 20.0, ..ui.sans(15.0, 600) }, c.text, value))
    };
    let stat_plain = |label: &str, value: String| {
        col()
            .flex_1()
            .min_w_0()
            .gap(px(2.0))
            .child(ui.txt(ui.sans(11.5, 400), c.text_3, label.to_string()))
            .child(ui.txt(TypeStyle { line_height: 20.0, ..ui.sans(15.0, 600) }, c.text, value))
    };
    col()
        .w(px(252.0))
        .h_full()
        .flex_none()
        .border_l_1()
        .border_color(c.seam.to_hsla())
        .bg(c.gutter.to_hsla())
        .child(
            ih().child(ui.i12("panel-right", c.text_3))
                .child(ui.sc(13.5, 600, 0.075, c.text_2, ui.s("record")))
                .child(div().flex_1())
                .child(
                    row()
                        .gap(px(5.0))
                        .child(ui.txt(ui.sans(12.0, 400), c.text_3, ui.s("rowWord")))
                        .child(ui.txt(ui.mono(11.5), c.text, "3")),
                ),
        )
        .child(field("full_name", "text", "Cem Öztürk".into(), false))
        .child(field("city", "text", "İzmir".into(), false))
        .child(field("segment", "text", "enterprise".into(), false))
        .child(field("orders", "bigint", "11".into(), true))
        .child(field("revenue", "numeric(14,2)", ui.nf(39104.75, 2), true))
        .child(
            ih().border_t_1()
                .child(ui.sc(13.5, 600, 0.075, c.text_2, ui.s("selRange")))
                .child(div().flex_1())
                .child(
                    row()
                        .gap(px(5.0))
                        .child(ui.txt(ui.mono(11.5), c.text, "6"))
                        .child(ui.txt(ui.sans(12.0, 400), c.text_3, ui.s("cells"))),
                ),
        )
        .child(
            col()
                .w_full()
                .pt(px(10.0))
                .px(px(12.0))
                .pb(px(12.0))
                .gap(px(12.0))
                .child(
                    row()
                        .w_full()
                        .items_start()
                        .gap(px(10.0))
                        .child(stat("Σ", "revenue", ui.nf(110190.85, 2)))
                        .child(stat(ui.s("avg"), "revenue", ui.nf(36730.28, 2))),
                )
                .child(
                    row()
                        .w_full()
                        .items_start()
                        .gap(px(10.0))
                        .child(stat("Σ", "orders", "30".into()))
                        .child(stat_plain(ui.s("avgPerOrder"), ui.nf(3673.03, 2))),
                ),
        )
}
