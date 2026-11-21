//! The history dialog's rows (M3b Task 27): one `HistoryHit` per row (masked statement,
//! connection, outcome glyph, time, run count, pin), virtualized.
//!
//! ## m3b-t01b-seams: the module line (Task 1b). Task 27 fills it.
//!
//! ## m3b-t27-history
//!
//! The NOTES "History row" (`design/m3b-safe` surface 11): a 24 px row of time (Fragment Mono
//! 11 `text-2`, the format locale's medium time, the instant in ISO 8601 UTC in its tooltip),
//! the 7 px environment mark, the connection (mono 11.5, 96 px, cut with an ellipsis: data),
//! the outcome glyph (green check, ink triangle-alert, `text-2` ban), the statement's first
//! line (mono 11.5 `text-2`, ellipsized: data), the duration (mono 11, 64 px, right), the rows
//! (mono 11, 76 px, right) and the pin glyph when pinned; the selected row is
//! `blue-tint-strong` with its statement in `text`. Rows are grouped by day ("Today",
//! "Yesterday", then the date) in the format locale's time zone. The list holds at most the
//! store's 200 hits, so it is drawn whole (not virtualized).
//!
//! Every text a row shows is the store's: the redacted statement, never a bind value.

use std::sync::Arc;

use hacivat_exec::{HistoryHit, HistoryRecord, Outcome};
use hacivat_l10n::{ArgValue, WhenValue};
use hacivat_proto::EnvTag;
use hacivat_ui::capsule::env_swatch;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::Tip;
// ## m3b-t29-shots (named exception)
use hacivat_ui::popover::{A11yDecl, Declare as _};
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    App, Div, FluentBuilder as _, Hsla, InteractiveElement, ParentElement, Role, SharedString,
    Stateful, StatefulInteractiveElement, Styled, TextRun, Window, div, hsla, px,
};
use hacivat_ui_core::theme::{Theme, TypeStyle};

use crate::connections::error_view::env_kind;
use crate::connections::prompt_kit::visible;
use crate::exec_ui::marks::{duration_text, message};

/// The row's height (NOTES "History row").
pub(crate) const ROW_PX: f32 = 24.0;
/// A day's group header.
pub(crate) const GROUP_PX: f32 = 22.0;
/// The time, duration and rows columns are at least NOTES' widths and else as wide as their
/// widest text in the list ([`Widths`]): a 12-hour clock (`12:32:08 PM`), a Turkish `sn` and
/// the pseudo-locales are wider than NOTES' `14:32:08`, and a chrome text is never cut.
const TIME_PX: f32 = 58.0;
const MARK_PX: f32 = 7.0;
const CONN_PX: f32 = 96.0;
/// The narrowest the connection column gets when the row is short of room (it ellipsizes:
/// data).
const CONN_MIN_PX: f32 = 40.0;
const GLYPH_PX: f32 = 12.0;
const DURATION_PX: f32 = 64.0;
const ROWS_PX: f32 = 76.0;
/// The most characters of a statement's first line a row lays out (it ellipsizes anyway).
const LINE_MAX_CHARS: usize = 400;
/// The most characters of a connection name shown.
pub(crate) const NAME_MAX_CHARS: usize = 120;
const DAY_MS: i64 = 86_400_000;

/// What a row shows of the connection an entry ran on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Conn {
    pub(crate) name: Arc<str>,
    pub(crate) env: Option<EnvTag>,
}

/// The statement's first non-blank line, trimmed and made visible (controls, bidi and
/// invisible characters as escapes), at most [`LINE_MAX_CHARS`] characters.
pub fn first_line(sql: &str) -> String {
    let line = sql
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    visible(&line.replace('\t', " "), LINE_MAX_CHARS)
}

/// `ms` (Unix milliseconds) as ISO 8601 in UTC, `2026-11-16T11:32:08Z` (the tooltip and the
/// preview: one unambiguous form whatever the format locale).
pub fn iso_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's `civil_from_days`.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// The rows an entry reports: fetched, else affected.
pub fn rows_of(r: &HistoryRecord) -> Option<u64> {
    r.rows_fetched.or(r.rows_affected)
}

/// The entry's time in the format locale's medium time style.
pub(crate) fn time_text(cx: &App, ms: i64) -> SharedString {
    message(
        cx,
        "hv-history-time",
        vec![("t", ArgValue::When(WhenValue::Instant { unix_ms: ms }))],
    )
}

/// The entry's day in the format locale's medium date style (the group key and header).
pub(crate) fn day_text(cx: &App, ms: i64) -> SharedString {
    message(
        cx,
        "hv-history-day",
        vec![("t", ArgValue::When(WhenValue::Instant { unix_ms: ms }))],
    )
}

/// The group header of `day` (a [`day_text`]) seen at `now_ms`: "Today", "Yesterday", else the
/// date itself.
pub(crate) fn group_title(cx: &App, day: &SharedString, now_ms: i64) -> SharedString {
    if *day == day_text(cx, now_ms) {
        message(cx, "hv-history-today", vec![])
    } else if *day == day_text(cx, now_ms - DAY_MS) {
        message(cx, "hv-history-yesterday", vec![])
    } else {
        day.clone()
    }
}

/// The outcome's word.
pub(crate) fn outcome_text(cx: &App, o: Outcome) -> SharedString {
    let id = match o {
        Outcome::Ok => "hv-history-outcome-ok",
        Outcome::Failed => "hv-history-outcome-failed",
        Outcome::Cancelled => "hv-history-outcome-cancelled",
        Outcome::Denied => "hv-history-outcome-denied",
    };
    message(cx, id, vec![])
}

/// The outcome's glyph and ink (Hue Is Meaning: green for a success, ink for a failure, text-2
/// for a stop or a refusal).
pub(crate) fn outcome_glyph(theme: &Theme, o: Outcome) -> Icon {
    let c = &theme.tokens.colors;
    let (icon, ink) = match o {
        Outcome::Ok => (IconName::Check, c.green),
        Outcome::Failed => (IconName::TriangleAlert, c.text),
        Outcome::Cancelled => (IconName::Ban, c.text_2),
        Outcome::Denied => (IconName::CircleX, c.text_2),
    };
    Icon::new(icon).size(IconSize::Chrome).color(ink.to_hsla())
}

/// The rows text ("12 rows"), when the entry reports any.
pub(crate) fn rows_text(cx: &App, r: &HistoryRecord) -> Option<SharedString> {
    rows_of(r).map(|n| message(cx, "hv-history-rows", vec![("rows", ArgValue::Count(n))]))
}

/// A day's group header (12 px `text-2`; the list is a field well, so `text-2` is its quiet
/// ink).
pub(crate) fn group_header(theme: &Theme, title: SharedString) -> Div {
    let t = theme.tokens;
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .h(px(GROUP_PX))
        .px(px(t.spacing.lg))
        .child(
            Label::new(
                title,
                TextSource::Fluent,
                t.type_ramp.caption,
                t.colors.text_2.to_hsla(),
            )
            .id("history-group"),
        )
}

/// A fixed-width cell.
fn cell(w: f32) -> Div {
    div().flex().flex_row().items_center().flex_none().w(px(w))
}

/// The widths of the list's measured columns: every row draws its time, duration and rows in
/// columns of these widths, so the columns align and no text is cut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Widths {
    time: f32,
    duration: f32,
    rows: f32,
}

/// Fragment Mono 11 (NOTES: the time, duration and rows columns, the preview's values): the
/// data style at 11 px, its line kept.
pub(crate) fn mono11(theme: &Theme) -> TypeStyle {
    let mut style = theme.tokens.type_ramp.mono_data;
    style.size_px = 11.0;
    style
}

/// The width `text` takes in `style`.
fn measure(text: &SharedString, style: &TypeStyle, ink: Hsla, window: &mut Window) -> f32 {
    let run = TextRun {
        len: text.len(),
        font: font_for(style),
        color: ink,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let width: f32 = window
        .text_system()
        .shape_line(text.clone(), px(style.size_px), &[run], None)
        .width
        .into();
    // A pixel of slack against rounding between this shaping and the label's.
    width.ceil() + 1.0
}

/// The column widths for `hits` (at least NOTES' widths).
pub(crate) fn widths(hits: &[HistoryHit], theme: &Theme, window: &mut Window, cx: &App) -> Widths {
    let t = theme.tokens;
    let style = mono11(theme);
    let ink = t.colors.text_2.to_hsla();
    let mut w = Widths {
        time: TIME_PX,
        duration: DURATION_PX,
        rows: ROWS_PX,
    };
    for hit in hits {
        let r = &hit.record;
        w.time = w
            .time
            .max(measure(&time_text(cx, r.ts_start_ms), &style, ink, window));
        let duration = SharedString::from(duration_text(cx, u64::from(r.duration_ms)));
        w.duration = w.duration.max(measure(&duration, &style, ink, window));
        if let Some(rows) = rows_text(cx, r) {
            w.rows = w.rows.max(measure(&rows, &style, ink, window));
        }
    }
    w
}

/// Row `index` of the list's `total` for `hit`: its connection `conn` (`None`: the connection
/// is gone), selected or not.
pub(crate) fn row(
    theme: &Theme,
    // ## m3b-t29-shots (named exception): the list's size too, for the option's position.
    (index, total): (usize, usize),
    hit: &HistoryHit,
    conn: Option<&Conn>,
    selected: bool,
    w: Widths,
    cx: &App,
) -> Stateful<Div> {
    let t = theme.tokens;
    let c = &t.colors;
    let r = &hit.record;
    let ink2 = c.text_2.to_hsla();
    let sql_ink: Hsla = if selected { c.text } else { c.text_2 }.to_hsla();
    let iso: SharedString = iso_utc(r.ts_start_ms).into();
    let time = cell(w.time)
        .id(("history-time", index))
        .tooltip(move |_, cx| Tip::view(iso.clone(), cx))
        .child(
            Label::new(
                time_text(cx, r.ts_start_ms),
                TextSource::Fluent,
                mono11(theme),
                ink2,
            )
            .id("history-row-time"),
        );
    let mark = cell(MARK_PX).children(
        conn.and_then(|k| k.env)
            .map(|e| env_swatch(env_kind(e), theme)),
    );
    let name = conn.map_or_else(
        || message(cx, "hv-history-conn-gone", vec![]),
        |k| SharedString::from(visible(&k.name, NAME_MAX_CHARS)),
    );
    let name_source = if conn.is_some() {
        TextSource::Data
    } else {
        TextSource::Fluent
    };
    let conn_cell = cell(CONN_PX)
        .flex_shrink(1.0)
        .min_w(px(CONN_MIN_PX))
        .overflow_hidden()
        .child(
            Label::new(name, name_source, t.type_ramp.mono_data, c.text.to_hsla())
                .truncate(true)
                .id("history-row-conn"),
        );
    let glyph = cell(GLYPH_PX).child(outcome_glyph(theme, r.outcome));
    let sql = div()
        .flex()
        .flex_row()
        .items_center()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .child(
            Label::new(
                first_line(&r.statement_masked),
                TextSource::Data,
                t.type_ramp.mono_data,
                sql_ink,
            )
            .truncate(true)
            .id("history-row-sql"),
        );
    let duration = cell(w.duration).justify_end().child(
        Label::new(
            duration_text(cx, u64::from(r.duration_ms)),
            TextSource::Fluent,
            mono11(theme),
            ink2,
        )
        .id("history-row-duration"),
    );
    let rows = cell(w.rows)
        .justify_end()
        .children(rows_text(cx, r).map(|text| {
            Label::new(text, TextSource::Fluent, mono11(theme), ink2).id("history-row-rows")
        }));
    let pinned: SharedString = message(cx, "hv-history-pinned", vec![]);
    let pin = cell(GLYPH_PX).id(("history-pin", index)).when_some(
        hit.pinned.then_some(pinned),
        |el, tip| {
            el.tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                .child(Icon::new(IconName::Pin).size(IconSize::Chrome).color(ink2))
        },
    );
    let bg = if selected {
        c.blue_tint_strong.to_hsla()
    } else {
        hsla(0.0, 0.0, 0.0, 0.0)
    };
    // ## m3b-t29-shots (named exception; the T27 review's m6): each row is a list option whose
    // accessible name is its first SQL line (data) and which says whether it is selected, as
    // the palette's rows do; the move itself is announced by the dialog's live node.
    let option = A11yDecl {
        position: Some((index + 1, total)),
        ..A11yDecl::new(Role::ListBoxOption, first_line(&r.statement_masked).into())
            .selected(Some(selected))
    };
    div()
        .id(("history-row", index))
        .debug_selector(move || format!("history-row-{index}"))
        .declare_full(option)
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(t.spacing.md))
        .h(px(ROW_PX))
        .px(px(t.spacing.lg))
        .bg(bg)
        .child(time)
        .child(mark)
        .child(conn_cell)
        .child(glyph)
        .child(sql)
        .child(duration)
        .child(rows)
        .child(pin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_utc_spells_the_instant() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        // 2026-11-16 11:32:08 UTC.
        assert_eq!(iso_utc(1_794_828_728_000), "2026-11-16T11:32:08Z");
        assert_eq!(iso_utc(951_782_400_000), "2000-02-29T00:00:00Z");
        assert_eq!(iso_utc(-1_000), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn first_line_skips_blank_lines_and_shows_controls() {
        assert_eq!(first_line("\n  \n  SELECT 1;\nFROM t"), "SELECT 1;");
        assert_eq!(first_line("SELECT '\u{202E}x'"), "SELECT '\\u{202E}x'");
        assert_eq!(first_line(""), "");
    }
}
