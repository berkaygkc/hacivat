//! Editor: tabs, query bar, gutter, highlighted SQL, execution band and the completion popup.

use hacivat_ui_core::gpui_surface::*;

use super::data::Env;
use super::explorer::panel_header;
use super::ui::{Ui, alpha, col, row};

const LINE: f32 = 18.0;
const CODE_TOP_PAD: f32 = 6.0;
const GUTTER_W: f32 = 58.0;
const LINES_PAD_LEFT: f32 = 8.0;
/// Source column (chars before the cursor) of the caret on line 14.
const CARET_COL: f32 = 48.0;
/// Popup placement from the Chromium capture: the popup's bottom edge sits 4.5 px above the top
/// of line 14, and its left edge 4 px left of the anchor character.
const POPUP_H: f32 = 172.0;
const POPUP_W: f32 = 538.0;

#[derive(Clone, Copy)]
enum Tk {
    Id,
    Kw,
    Fn,
    Str,
    Num,
    Com,
    Op,
    Al,
}

pub fn editor(ui: &Ui, advance: f32) -> impl IntoElement {
    let c = &ui.c;
    col()
        .relative()
        .size_full()
        .bg(c.panel.to_hsla())
        .overflow_hidden()
        .child(tabs_header(ui))
        .child(query_bar(ui))
        .child(code(ui, advance))
        // The 1 px production line across the very top of the editor.
        .child(div().absolute().left_0().right_0().top_0().h(px(1.0)).bg(c.red.to_hsla()))
}

fn tabs_header(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    let tab = |first: bool, on: bool, name: &str, tail: Vec<AnyElement>| {
        let mut d = row()
            .h(px(30.0))
            .flex_none()
            .px(px(12.0))
            .gap(px(7.0))
            .border_r_1()
            .border_color(c.seam.to_hsla())
            .child(ui.i12("file-code-2", if on { c.text } else { c.text_2 }))
            .child(ui.txt(ui.mono(12.0), if on { c.text } else { c.text_2 }, name.to_string()));
        if first {
            d = d.border_l_1();
        }
        if on {
            d = d.bg(c.raised.to_hsla());
        }
        d.children(tail)
    };
    let tabs = row()
        .h_full()
        .ml(px(2.0))
        .items_start()
        .child(tab(
            true,
            true,
            "orders-prod.sql",
            vec![ui.swatch(Env::Prod).into_any_element(), ui.i12("x", c.text_3).into_any_element()],
        ))
        .child(tab(false, false, "revenue-by-city.sql", vec![]))
        .child(tab(
            false,
            false,
            "scratch.sql",
            vec![div().size(px(6.0)).rounded_full().bg(c.text_2.to_hsla()).into_any_element()],
        ));
    panel_header(ui)
        .gap(px(12.0))
        .child(tabs)
        .child(div().flex_1())
        .child(
            row()
                .gap(px(3.0))
                .child(ui.txt(ui.mono(12.0), c.text_3, "2"))
                .child(ui.txt(ui.sans(12.0, 400), c.text_3, ui.s("statements"))),
        )
        .child(ui.kbd("⌘2", c.text_3))
}

fn query_bar(ui: &Ui) -> impl IntoElement {
    let c = &ui.c;
    let btn = |border: Hsla| {
        row().h(px(24.0)).flex_none().px(px(9.0)).gap(px(6.0)).border_1().border_color(border).rounded(px(4.0))
    };
    let transparent = hsla(0.0, 0.0, 0.0, 0.0);
    let run = btn(c.blue.to_hsla())
        .bg(c.blue.to_hsla())
        .child(ui.i12("play", c.on_accent))
        .child(ui.txt(ui.sans(12.5, 600), c.on_accent, ui.s("run")))
        .child(ui.run(ui.mono(11.0), alpha(c.on_accent, 0.7)).child("⌘↵"));
    let explain = btn(c.seam_strong.to_hsla())
        .child(ui.i12("list-tree", c.text))
        .child(ui.txt(ui.sans(12.5, 500), c.text, ui.s("explain")));
    let format = btn(transparent)
        .child(ui.i12("align-left", c.text_2))
        .child(ui.txt(ui.sans(12.5, 500), c.text_2, ui.s("format")));
    let crumb = row()
        .h(px(20.0))
        .flex_none()
        .ml(px(10.0))
        .pl(px(12.0))
        .gap(px(7.0))
        .border_l_1()
        .border_color(c.seam.to_hsla())
        .child(ui.swatch(Env::Prod))
        .child(ui.txt(ui.mono(11.5), c.text_2, "orders-prod"))
        .child(ui.i12("chevron-right", c.text_3))
        .child(ui.txt(ui.mono(11.5), c.text_2, "orders"))
        .child(ui.i12("chevron-right", c.text_3))
        .child(ui.txt(ui.mono(11.5), c.text, "public"));
    let chip = || {
        row().h(px(22.0)).flex_none().px(px(8.0)).gap(px(5.0)).border_1().border_color(c.seam.to_hsla()).rounded(px(4.0))
    };
    row()
        .w_full()
        .h(px(32.0))
        .flex_none()
        .pl(px(12.0))
        .pr(px(10.0))
        .gap(px(6.0))
        .border_b_1()
        .border_color(c.seam.to_hsla())
        .bg(c.raised.to_hsla())
        .child(run)
        .child(explain)
        .child(format)
        .child(crumb)
        .child(div().flex_1())
        .child(
            chip()
                .child(ui.i12("git-commit-horizontal", c.text_2))
                .child(ui.txt(ui.sans(12.5, 400), c.text_2, ui.s("autocommit"))),
        )
        .child(
            chip()
                .child(ui.txt(ui.sans(12.5, 400), c.text_2, ui.s("limit")))
                .child(ui.txt(ui.mono(11.0), c.text, "100")),
        )
}

fn code(ui: &Ui, advance: f32) -> impl IntoElement {
    let c = &ui.c;
    let gutter = {
        let mut g = col().relative().w(px(GUTTER_W)).flex_none();
        for i in 1..=14u32 {
            let ink = if i == 14 { c.text } else { c.text_3 };
            let mut ln = div()
                .relative()
                .w_full()
                .h(px(LINE))
                .flex_none()
                .flex()
                .justify_end()
                .pr(px(14.0))
                .child(ui.txt(ui.mono(12.5), ink, i.to_string()));
            if i == 2 {
                ln = ln.child(div().absolute().left(px(8.0)).top(px(3.0)).child(ui.i12("check", c.green)));
            }
            g = g.child(ln);
        }
        g.child(div().absolute().left(px(4.0)).top(px(LINE)).w(px(1.0)).h(px(LINE * 11.0)).bg(c.green.to_hsla()))
            .child(div().absolute().left(px(4.0)).top(px(LINE * 13.0)).w(px(1.0)).h(px(LINE)).bg(c.blue.to_hsla()))
    };
    use Tk::*;
    let lines: Vec<Vec<(Tk, &str)>> = vec![
        vec![(Com, "-- Top customers by revenue, last 30 days")],
        vec![(Kw, "SELECT"), (Id, " c.full_name"), (Op, ",")],
        vec![(Id, "       c.city"), (Op, ","), (Id, " c.segment"), (Op, ",")],
        vec![(Id, "       "), (Fn, "count"), (Op, "("), (Id, "o.id"), (Op, ")"), (Id, "          "), (Kw, "AS"), (Id, " "), (Al, "orders"), (Op, ",")],
        vec![(Id, "       "), (Fn, "sum"), (Op, "("), (Id, "oi.line_total"), (Op, ")"), (Id, "   "), (Kw, "AS"), (Id, " "), (Al, "revenue")],
        vec![(Kw, "FROM"), (Id, " customers c")],
        vec![(Kw, "JOIN"), (Id, " orders o "), (Kw, "ON"), (Id, " o.customer_id "), (Op, "="), (Id, " c.id")],
        vec![(Kw, "JOIN"), (Id, " order_items oi "), (Kw, "ON"), (Id, " oi.order_id "), (Op, "="), (Id, " o.id")],
        vec![
            (Kw, "WHERE"), (Id, " o.ordered_at "), (Op, ">"), (Id, " "), (Fn, "now"), (Op, "()"), (Id, " "), (Op, "-"), (Id, " "),
            (Kw, "interval"), (Id, " "), (Str, "'30 days'"),
        ],
        vec![(Kw, "GROUP BY"), (Id, " c.full_name"), (Op, ","), (Id, " c.city"), (Op, ","), (Id, " c.segment")],
        vec![(Kw, "ORDER BY"), (Id, " revenue "), (Kw, "DESC")],
        vec![(Kw, "LIMIT"), (Id, " "), (Num, "100"), (Op, ";")],
        vec![(Id, " ")],
        vec![
            (Kw, "SELECT"), (Id, " "), (Op, "*"), (Id, " "), (Kw, "FROM"), (Id, " orders "), (Kw, "WHERE"), (Id, " status "), (Op, "="),
            (Id, " "), (Str, "'paid'"), (Id, " "), (Kw, "AND"), (Id, " c"),
        ],
    ];
    let ink = |t: Tk| match t {
        Id => c.syn_identifier,
        Kw => c.syn_keyword,
        Fn => c.syn_function,
        Str => c.syn_string,
        Num => c.syn_number,
        Com => c.syn_comment,
        Op => c.syn_operator,
        Al => c.syn_alias,
    };
    let mut body = col().relative().flex_1().h_full().pl(px(LINES_PAD_LEFT)).min_w_0();
    for (i, toks) in lines.into_iter().enumerate() {
        let mut l = row().relative().w_full().h(px(LINE)).flex_none();
        // One shaped run per line: separate token elements accumulate layout rounding.
        let mut text = String::new();
        let mut marks: Vec<(std::ops::Range<usize>, HighlightStyle)> = vec![];
        for (t, s) in toks {
            let range = text.len()..text.len() + s.len();
            text.push_str(s);
            let mut hl = HighlightStyle { color: Some(ink(t).to_hsla()), ..Default::default() };
            if matches!(t, Com) {
                hl.font_style = Some(FontStyle::Italic);
            }
            if matches!(t, Al) && ui.light() {
                hl.font_weight = Some(FontWeight(500.0));
            }
            marks.push((range, hl));
        }
        l = l.child(
            ui.run(ui.mono(12.5), c.syn_identifier.to_hsla()).child(StyledText::new(text).with_highlights(marks)),
        );
        match i {
            1 => {
                // Per-statement status, right-aligned at the end of line 2.
                let t3 = ui.sans(11.0, 400);
                l = l.child(
                    row().absolute().right(px(14.0)).top_0().h(px(LINE)).gap(px(6.0))
                        .child(
                            row().gap(px(4.0))
                                .child(ui.i12("check", c.green))
                                .child(ui.txt(t3, c.green, ui.s("executed"))),
                        )
                        .child(ui.txt(t3, c.text_3, "42 ms"))
                        .child(ui.txt(t3, c.text_3, "·"))
                        .child(ui.txt(t3, c.text_3, format!("100 {}", ui.s("rowsWord")))),
                );
            }
            3 | 4 => {
                let ty = if i == 3 { "bigint" } else { "numeric" };
                l = l.child(
                    div().ml(px(10.0)).mt(px(-1.0)).px(px(4.0)).h(px(14.0)).rounded(px(3.0)).bg(c.seam_soft.to_hsla())
                        .child(ui.txt(TypeStyleLh::lh(ui.sans(11.0, 500), 14.0), c.text_3, ty)),
                );
            }
            13 => {
                // Execution-range band behind the current statement.
                let k = if ui.light() { 0.8 } else { 0.55 };
                l = l.child(
                    div().absolute().left(px(-LINES_PAD_LEFT)).right_0().top_0().bottom_0().bg(alpha(c.blue_tint, k)),
                );
            }
            _ => {}
        }
        body = body.child(l);
    }
    // Caret and completion popup are placed from measured Chromium geometry.
    let caret_x = LINES_PAD_LEFT + CARET_COL * advance;
    let line14_top = LINE * 13.0;
    body = body
        .child(
            div().absolute().left(px(caret_x)).top(px(line14_top + 0.5)).w(px(2.0)).h(px(16.0)).bg(c.blue.to_hsla()),
        )
        .child(
            completion(ui)
                .absolute()
                .left(px(LINES_PAD_LEFT + (CARET_COL - 1.0) * advance - 4.0))
                .top(px(line14_top - 4.5 - POPUP_H)),
        );
    row()
        .w_full()
        .flex_1()
        .min_h_0()
        .pt(px(CODE_TOP_PAD))
        .child(gutter)
        .child(body)
}

/// Fixes a run's line height without touching its font.
struct TypeStyleLh;
impl TypeStyleLh {
    fn lh(mut s: hacivat_ui_core::theme::TypeStyle, lh: f32) -> hacivat_ui_core::theme::TypeStyle {
        s.line_height = lh;
        s
    }
}

fn completion(ui: &Ui) -> Div {
    let c = &ui.c;
    // Raised surface: tertiary ink steps up to text-2.
    let t3 = c.text_2;
    let item = |on: bool, out: bool, icon: &str, rest: &str, ty: Option<(&str, bool)>, tys: &str| {
        let kind_ink = if on { c.blue } else if out { t3 } else { c.text_2 };
        let name_ink = if out { t3 } else { c.text };
        let mut ty_el = match ty {
            Some((s, true)) => ui.txt(ui.mono(11.5), t3, s.to_string()),
            _ => {
                let mut e = ui.txt(ui.sans(11.5, 400), t3, tys.to_string());
                if out {
                    e = e.italic();
                }
                e
            }
        };
        if matches!(ty, Some((_, false))) {
            ty_el = ui.txt(ui.sans(11.5, 400), t3, tys.to_string());
        }
        let mut li = row()
            .w_full()
            .h(px(23.0))
            .flex_none()
            .pl(px(5.0))
            .pr(px(7.0))
            .gap(px(6.0))
            .rounded(px(3.0))
            .child(div().w(px(18.0)).flex_none().flex().justify_center().child(ui.i12(icon, kind_ink)))
            .child(
                row()
                    .flex_1()
                    .min_w_0()
                    .child(ui.run(ui.mono(12.5), c.blue.to_hsla()).child("c"))
                    .child(ui.txt(ui.mono(12.5), name_ink, rest.to_string())),
            )
            .child(ty_el);
        if on {
            li = li.bg(c.blue_tint_strong.to_hsla());
        }
        li
    };
    let kw = ui.s("kind.keyword");
    let list = col()
        .w(px(300.0))
        .flex_none()
        .p(px(4.0))
        .border_r_1()
        .border_color(c.seam.to_hsla())
        .child(item(true, false, "columns-2", "ustomer_id", Some(("bigint · orders", true)), ""))
        .child(item(false, true, "columns-2", "ity", None, ui.s("notInScope")))
        .child(item(false, false, "columns-2", "ancelled_at", Some(("timestamptz · orders", true)), ""))
        .child(item(false, false, "square-function", "ount()", None, ui.s("kind.function")))
        .child(item(false, false, "calendar", "urrent_date", None, ui.s("kind.kwfn")))
        .child(item(false, false, "brackets", "ase", None, kw));
    let dk = |k: &str, v: AnyElement| {
        row()
            .w_full()
            .h(px(19.0))
            .flex_none()
            .child(div().w(px(78.0)).flex_none().child(ui.txt(ui.sans(12.0, 400), t3, k.to_string())))
            .child(v)
    };
    let det = col()
        .w(px(236.0))
        .flex_none()
        .pt(px(9.0))
        .px(px(12.0))
        .pb(px(8.0))
        .child(
            row()
                .w_full()
                .mb(px(8.0))
                .child(ui.txt(ui.mono(12.5), c.text, "orders.customer_id"))
                .child(div().flex_1())
                .child(ui.txt(ui.sans(12.0, 400), t3, ui.s("d.column"))),
        )
        .child(dk(ui.s("d.type"), ui.txt(ui.mono(11.5), c.text, "bigint").into_any_element()))
        .child(dk(ui.s("d.nullable"), ui.txt(ui.sans(12.0, 400), c.text, ui.s("d.no")).into_any_element()))
        .child(dk(ui.s("d.table"), ui.txt(ui.mono(11.5), c.text, "public.orders").into_any_element()))
        .child(dk(
            ui.s("d.refs"),
            row()
                .gap(px(5.0))
                .child(ui.i12("key-round", c.blue))
                .child(ui.txt(ui.mono(11.5), c.blue, "FK → customers.id"))
                .into_any_element(),
        ));
    let hint = |k: &str, s: &str| {
        row().gap(px(5.0)).child(ui.kbd_on(k, t3, c.seam)).child(ui.txt(ui.sans(11.5, 400), t3, s.to_string()))
    };
    let foot = row()
        .w_full()
        .h(px(24.0))
        .flex_none()
        .px(px(10.0))
        .gap(px(12.0))
        .border_t_1()
        .border_color(c.seam.to_hsla())
        .bg(c.head.to_hsla())
        .child(hint("↵", ui.s("hint.insert")))
        .child(hint("⇥", ui.s("hint.replace")))
        .child(hint("⌘.", ui.s("hint.docs")))
        .child(div().flex_1())
        .child(ui.txt(ui.mono(11.5), t3, "6 / 41"));
    let shadows: Vec<BoxShadow> = ui
        .shadow
        .iter()
        .map(|l| BoxShadow {
            color: l.color.to_hsla(),
            offset: point(px(l.x), px(l.y)),
            blur_radius: px(l.blur),
            spread_radius: px(l.spread),
            inset: false,
        })
        .collect();
    col()
        .w(px(POPUP_W))
        .h(px(POPUP_H))
        .flex_none()
        .bg(c.raised.to_hsla())
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .rounded(px(6.0))
        .shadow(shadows)
        .overflow_hidden()
        .child(row().w_full().items_start().child(list).child(det))
        .child(foot)
}
