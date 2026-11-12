//! Status bar: environment, path, transaction, rows, cursor, encoding, dialect, language.

use hacivat_ui_core::gpui_surface::*;

use super::data::{Env, Lang};
use super::ui::{Ui, row};

pub fn statusbar(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    let seg = |left: bool| {
        let d = row().h_full().flex_none().px(px(10.0)).gap(px(6.0)).border_color(c.seam.to_hsla());
        if left { d.border_r_1() } else { d.border_l_1() }
    };
    let s12 = |col, s: &str| ui.txt(ui.sans(12.0, 400), col, s.to_string());
    let mono11 = |col, s: &str| ui.txt(ui.mono(11.0), col, s.to_string());
    let env = seg(true)
        .pl(px(12.0))
        .bg(c.red_tint.to_hsla())
        .child(ui.swatch(Env::Prod))
        .child(ui.i12("lock", c.red))
        .child(ui.txt(ui.sans(12.0, 600), c.red, ui.s("status.env")));
    let path = seg(true)
        .child(ui.i12("arrow-left-right", c.green))
        .child(s12(c.text_2, ui.s("path.directShort")))
        .child(mono11(c.text_2, "18 ms"))
        .child(s12(c.text_3, "· karagoz@eu-west-1a"));
    let commit = seg(true)
        .child(ui.i12("git-commit-horizontal", c.text_2))
        .child(s12(c.text_2, ui.s("autocommit")));
    let rows = seg(true)
        .child(mono11(c.text_2, "100"))
        .child(s12(c.text_2, ui.s("rowsWord")))
        .child(s12(c.text_2, "·"))
        .child(mono11(c.text_2, "42 ms"));
    let tr = ui.lang == Lang::Tr;
    let bold = |on: bool, s: &str| ui.txt(ui.sans(12.0, if on { 600 } else { 400 }), if on { c.text } else { c.text_2 }, s.to_string());
    let lang = seg(false)
        .child(ui.i12("languages", c.text_2))
        .child(row().child(bold(!tr, "EN")).child(s12(c.text_2, " / ")).child(bold(tr, "TR")));
    row()
        .w_full()
        .h(px(24.0))
        .flex_none()
        .border_t_1()
        .border_color(c.seam.to_hsla())
        .bg(c.ground.to_hsla())
        .child(env)
        .child(path)
        .child(commit)
        .child(rows)
        .child(div().flex_1())
        .child(seg(false).child(s12(c.text_2, ui.s("lncol"))))
        .child(seg(false).child(mono11(c.text_2, "UTF-8")))
        .child(seg(false).child(s12(c.text_2, "PostgreSQL")))
        .child(lang)
}
