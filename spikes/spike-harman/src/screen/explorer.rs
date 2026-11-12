//! Left column: explorer tree and the connection sheet.

use hacivat_ui_core::gpui_surface::*;

use super::data::{Env, tree};
use super::ui::{Ui, col, row};

/// Panel header (`.ph`): 30 px, bottom seam, head fill.
pub fn panel_header(ui: &Ui) -> Div {
    row()
        .w_full()
        .h(px(30.0))
        .flex_none()
        .pl(px(12.0))
        .pr(px(10.0))
        .gap(px(10.0))
        .border_b_1()
        .border_color(ui.c.seam.to_hsla())
        .bg(ui.c.head.to_hsla())
}

pub fn explorer(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    let header = panel_header(ui)
        .child(ui.sc(13.5, 600, 0.075, c.text_2, ui.s("explorer")))
        .child(div().flex_1())
        .child(ui.txt(ui.mono(12.0), c.text_3, "6"))
        .child(ui.kbd("⌘1", c.text_3));
    let filter_bg = if ui.light() { c.panel.to_hsla() } else { hsla(0.0, 0.0, 0.0, 0.0) };
    let filter = row()
        .h(px(28.0))
        .flex_none()
        .mx(px(8.0))
        .mt(px(8.0))
        .mb(px(4.0))
        .pl(px(8.0))
        .pr(px(6.0))
        .gap(px(7.0))
        .border_1()
        .border_color(c.seam.to_hsla())
        .rounded(px(4.0))
        .bg(filter_bg)
        .child(ui.i12("search", c.text_3))
        .child(div().flex_1().child(ui.txt(ui.sans(12.5, 400), c.text_3, ui.s("filterConn"))))
        .child(ui.kbd("⌘F", c.text_3));
    let mut rows = col().flex_1().w_full().pt(px(2.0)).pb(px(8.0)).overflow_hidden();
    for r in tree() {
        rows = rows.child(if let Some((label, raw, n, agent)) = r.group {
            group(ui, if raw { label } else { ui.s(label) }, n, agent).into_any_element()
        } else {
            tree_row(ui, &r).into_any_element()
        });
    }
    col()
        .size_full()
        .bg(c.side.to_hsla())
        .child(header)
        .child(filter)
        .child(rows)
}

fn group(ui: &Ui, label: &str, n: &str, agent: bool) -> impl IntoElement {
    let c = &ui.c;
    let line = if ui.light() { c.seam } else { c.seam_soft };
    row()
        .w_full()
        .h(px(26.0))
        .flex_none()
        .pt(px(6.0))
        .pl(px(12.0))
        .pr(px(10.0))
        .gap(px(7.0))
        .child(ui.sc(13.0, 600, 0.075, c.text_3, label))
        .child(div().flex_1().h(px(1.0)).bg(line.to_hsla()))
        .children(agent.then(|| ui.i12("radio", c.text_3)))
        .child(ui.txt(ui.mono(11.0), c.text_3, n.to_string()))
}

fn tree_row(ui: &Ui, r: &super::data::TreeRow) -> impl IntoElement {
    let c = &ui.c;
    let label = if r.i18n { ui.s(r.label) } else { r.label };
    let label_color = if r.dim { c.text_2 } else { c.text };
    let weight = if r.strong { 600 } else { 400 };
    let chev = div().w(px(12.0)).flex_none().flex().justify_center().children(
        r.chev.map(|n| ui.i12(&format!("chevron-{n}"), c.text_3)),
    );
    let mut d = row()
        .relative()
        .w_full()
        .h(px(23.0))
        .flex_none()
        .pl(px(8.0 + r.depth as f32 * 14.0))
        .pr(px(10.0))
        .gap(px(6.0))
        .child(chev)
        .child(ui.i12(r.icon, if r.sel { c.blue } else { c.text_2 }))
        .child(ui.txt(ui.sans(13.0, weight), label_color, label));
    if r.sel {
        d = d.bg(c.blue_tint.to_hsla()).child(
            div().absolute().left_0().top_0().bottom_0().w(px(1.0)).bg(c.blue.to_hsla()),
        );
    }
    if let Some(env) = r.env {
        d = d.child(ui.swatch(env));
    }
    if r.ro || r.p2p {
        d = d.child(
            row()
                .gap(px(5.0))
                .children(r.ro.then(|| ui.i12("lock", c.text_2)))
                .children(r.p2p.then(|| ui.i12("arrow-left-right", c.green))),
        );
    }
    d = d.child(div().flex_1());
    if !r.cnt.is_empty() {
        d = d.child(ui.txt(ui.mono(11.5), c.text_3, r.cnt.to_string()));
    }
    if !r.meta.is_empty() {
        d = d.child(ui.txt(ui.mono(11.0), c.text_3, r.meta.to_string()));
    }
    d
}

/// Connection sheet: key/value rows for the active connection.
pub fn connection_sheet(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    let header = panel_header(ui)
        .child(ui.txt(ui.mono(12.5), c.text, "orders-prod"))
        .child(div().flex_1())
        .child(ui.txt(ui.sans(12.0, 400), c.text_3, ui.s("connection")))
        .child(ui.kbd("⌘I", c.text_3));
    let kv = |key: &str, value: Div| {
        row()
            .w_full()
            .h(px(22.0))
            .flex_none()
            .px(px(12.0))
            .gap(px(10.0))
            .child(
                div().w(px(92.0)).flex_none().child(ui.txt(ui.sans(12.5, 400), c.text_3, ui.s(key))),
            )
            .child(value.gap(px(6.0)))
    };
    let dd = || row().flex_1().min_w_0().overflow_hidden();
    let body = col()
        .pt(px(6.0))
        .child(kv(
            "kv.env",
            dd().child(ui.swatch(Env::Prod)).child(ui.txt(ui.sans(12.5, 600), c.red, ui.s("env.production"))),
        ))
        .child(kv(
            "kv.access",
            dd()
                .child(ui.i12("lock", c.red))
                .child(ui.txt(ui.sans(12.5, 400), c.text, ui.s("acc.ro")))
                .child(ui.txt(ui.mono(11.5), c.text_2, "app_ro")),
        ))
        .child(kv(
            "kv.path",
            dd()
                .child(ui.i12("arrow-left-right", c.green))
                .child(ui.txt(ui.sans(12.5, 400), c.text, ui.s("path.direct")))
                .child(ui.txt(ui.mono(11.5), c.text_2, "18 ms")),
        ))
        .child(kv("kv.agent", dd().child(ui.txt(ui.mono(11.5), c.text, "karagoz@eu-west-1a"))))
        .child(kv(
            "kv.engine",
            dd()
                .child(ui.txt(ui.sans(12.5, 400), c.text, "PostgreSQL"))
                .child(ui.txt(ui.mono(11.5), c.text, "16.4")),
        ))
        .child(kv("kv.granted", dd().child(ui.txt(ui.sans(12.5, 400), c.text, "Perde · Acme"))))
        .child(kv(
            "kv.audit",
            dd()
                .child(ui.i12("shield-check", c.text_2))
                .child(ui.txt(ui.sans(12.5, 400), c.text_2, ui.s("audit.val"))),
        ));
    col().size_full().bg(c.side.to_hsla()).child(header).child(body)
}
