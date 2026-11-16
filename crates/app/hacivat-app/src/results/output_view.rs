//! The output log view (Task 30) on `VirtualList`: severity glyphs, the Fluent sentence, the
//! verbatim server text, Clear / Copy / Scroll to end.
//!
//! One console's [`SessionLog`](hacivat_exec::SessionLog) (Task 23) as 21 px rows (NOTES
//! surface 12). A line is one Fluent sentence (`LogMsg::fluent_id` and `args`, rendered here
//! with the format locale's numbers and an `exec-dur-*` duration) after an ink glyph; nothing
//! but the sentence is translated.
//!
//! - **Server text is never inside a sentence** (NOTES cross-cutting 4). An error line is
//!   "Statement 3 failed" at 600 with its SQLSTATE as a mono chip; the server's text follows
//!   verbatim on its own row (mono 11.5 `text-2`, indented 32 px, ellipsized with the whole in
//!   its tooltip), as data. Without server text that row says what kind of failure it was
//!   (`ErrorView::title_id`); Hacivat's hint, when there is one, is a third row.
//! - **A click on an error line** (or its server text) asks the console's run UI to select the
//!   statement (`ResultsEvent::GoToSource`, emitted on the console's `ConsoleResults`).
//! - **Cost.** Only the visible rows are built ([`LogRows::built`] counts them), so 10,000
//!   lines cost what a screenful costs.
//! - **Copy** is `copy_invariant` (Task 23): ISO timestamps, ASCII digits, message IDs.

use std::cell::Cell;
use std::sync::Arc;

use hacivat_exec::{LogArg, LogLine, LogMsg, Severity, duration_message, span_message};
use hacivat_l10n::ArgValue;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::text::{Label, TextSource};
// ## m3a-t30 fix round 1 (I3): controls in server text (a line break, a bidi override) are
// shown as their chips, as in a cell, never obeyed or passed to the shaper.
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use hacivat_ui_grid::element::truncate::display_text;

use crate::console::ConsoleId;
use crate::l10n_global::{L10nGlobal, tr_id};
use crate::results::tabs::ConsoleResults;
use crate::results::view::ResultsEvent;

/// A log row's height (NOTES surface 12).
pub const LOG_ROW_PX: f32 = 21.0;

/// Renders the message `id` (a `hacivat-core.ftl` message) with `args`; the ID when no
/// localization is installed.
fn dynamic(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> String {
    match cx.try_global::<L10nGlobal>() {
        Some(g) => g.l10n.t_dynamic(id, args),
        None => id.to_string(),
    }
}

/// `ms` as one `exec-dur-*` message (`340 ms`, `2.34 s`), its numbers in the format locale.
pub fn duration_text(ms: u64, cx: &App) -> String {
    unit_msg_text(duration_message(ms), cx)
}

// ## m3b-t16-run-service (named exception)
/// An `exec-dur-*` message in the UI language: a seconds value with two decimals (the one
/// duration rule, ledger 535), every other number a count in the format locale.
fn unit_msg_text(m: hacivat_exec::UnitMsg, cx: &App) -> String {
    let args = m
        .args
        .iter()
        .map(|(name, n)| match m.decimal(name) {
            Some(d) => (*name, ArgValue::Decimal(hacivat_l10n::ExactDecimal(d))),
            None => (*name, ArgValue::Count(*n)),
        })
        .collect();
    dynamic(cx, m.id, args)
}

/// The sentence of a log message in the UI language: its counts in the format locale, its
/// durations as `exec-dur-*` messages, a statement number raw.
pub fn core_text(msg: &LogMsg, cx: &App) -> SharedString {
    let args = msg
        .args()
        .into_iter()
        .map(|(name, arg)| {
            let value = match arg {
                LogArg::U64(n) => ArgValue::Count(n),
                LogArg::Str(s) => ArgValue::Text(s.to_string()),
                LogArg::DurationMs(ms) => ArgValue::Text(duration_text(ms, cx)),
                // ## m3b-t16-run-service (named exception): a configured span (idle bound).
                LogArg::SpanMs(ms) => ArgValue::Text(unit_msg_text(span_message(ms), cx)),
            };
            (name, value)
        })
        .collect();
    dynamic(cx, msg.fluent_id(), args).into()
}

/// The glyph of a line (ink only: an error is `text`, everything else `text-2`).
fn glyph(msg: &LogMsg) -> (IconName, bool) {
    match msg {
        LogMsg::RowsRetrieved { .. } | LogMsg::RowsAffected { .. } | LogMsg::Completed { .. } => {
            (IconName::Check, false)
        }
        LogMsg::TxCommitted | LogMsg::TxRolledBack => (IconName::GitCommit, false),
        LogMsg::CursorClosed { .. } | LogMsg::Summary(_) => (IconName::Info, false),
        LogMsg::MoreResultSets { .. } => (IconName::Layers, false),
        LogMsg::Cancelled { .. } => (IconName::Ban, false),
        LogMsg::ReadOnlyDenied(_) => (IconName::Lock, false),
        LogMsg::Dropped { .. } => (IconName::Ellipsis, false),
        LogMsg::Error { .. } | LogMsg::CopyUnsupported | LogMsg::NotStarted(_) => {
            (IconName::TriangleAlert, true)
        }
        // ## m3b-t01c-core-seams (named exception): the M3b lines, with existing glyphs; no
        // M3a run writes them (Tasks 16 and 25 own their look).
        LogMsg::Notice { .. }
        | LogMsg::Retried { .. }
        | LogMsg::Evicted { .. }
        | LogMsg::SkippedByUser { .. } => (IconName::Info, false),
        LogMsg::ResultSetLimit { .. } => (IconName::Layers, false),
        LogMsg::ImplicitCommit
        | LogMsg::ConfirmCommitted { .. }
        | LogMsg::ConfirmRolledBack { .. } => (IconName::GitCommit, false),
        LogMsg::SafetyDenied { .. } => (IconName::Lock, false),
        LogMsg::ConfirmStatementFailed => (IconName::TriangleAlert, true),
        // ## m3b-t16-run-service (named exception): Task 25 owns their look.
        LogMsg::NoticesDropped { .. } => (IconName::Ellipsis, false),
        LogMsg::ClosedEarly { .. } => (IconName::Ban, false),
        LogMsg::RunAsOneRolledBack | LogMsg::RunAsOneTxFailed => (IconName::Info, false),
        // ## m3b-t19-confirm-tx (merge reconciliation): the transaction ended with a failed
        // statement — warning ink, as the user must check what happened to earlier work.
        LogMsg::TxRolledBackByServer | LogMsg::TxEndedByStatement => {
            (IconName::TriangleAlert, true)
        }
        // ## m3b-t22-tx-ui (named exception): the read-only skip of a startup script.
        LogMsg::StartupScriptSkipped => (IconName::Lock, false),
    }
}

/// One row of the list: a line, its server text (or failure kind), its hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Line(usize),
    Detail(usize),
    Hint(usize),
}

/// The rows of a console's output log.
pub struct LogRows {
    console: ConsoleId,
    results: WeakEntity<ConsoleResults>,
    lines: Arc<[LogLine]>,
    items: Vec<Item>,
    built: Cell<u64>,
}

impl std::fmt::Debug for LogRows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogRows")
            .field("console", &self.console)
            .field("lines", &self.lines.len())
            .field("rows", &self.items.len())
            .finish_non_exhaustive()
    }
}

impl LogRows {
    /// The rows of `console`'s log; clicks on error lines emit on `results`.
    pub fn new(console: ConsoleId, results: WeakEntity<ConsoleResults>) -> Self {
        LogRows {
            console,
            results,
            lines: Arc::from(Vec::new()),
            items: Vec::new(),
            built: Cell::new(0),
        }
    }

    /// Shows `lines` (the log's current lines, oldest first).
    pub fn set_lines(&mut self, lines: Arc<[LogLine]>) {
        self.items.clear();
        for (ix, line) in lines.iter().enumerate() {
            self.items.push(Item::Line(ix));
            if let LogMsg::Error { view, .. } | LogMsg::NotStarted(view) = &line.msg {
                self.items.push(Item::Detail(ix));
                if view.hint_id.is_some() {
                    self.items.push(Item::Hint(ix));
                }
            }
        }
        self.lines = lines;
    }

    /// The lines shown.
    pub fn lines(&self) -> &Arc<[LogLine]> {
        &self.lines
    }

    /// Rows built since the list was made or [`reset_built`](Self::reset_built).
    pub fn built(&self) -> u64 {
        self.built.get()
    }

    pub fn reset_built(&self) {
        self.built.set(0);
    }

    /// What a click on line `ix` does: select its statement, when it names one and failed.
    fn on_click(&self, ix: usize) -> Option<impl Fn(&mut App) + 'static> {
        let line = self.lines.get(ix)?;
        if line.severity != Severity::Error {
            return None;
        }
        let range = line.source.clone()?;
        let console = self.console;
        let results = self.results.clone();
        Some(move |cx: &mut App| {
            let range = range.clone();
            let _ = results.update(cx, |_, cx| {
                cx.emit(ResultsEvent::GoToSource { console, range })
            });
        })
    }

    fn line_row(&self, ix: usize, cx: &App) -> Option<AnyElement> {
        let line = self.lines.get(ix)?;
        let t = Theme::get(cx).tokens;
        let c = t.colors;
        let (icon, ink_text) = glyph(&line.msg);
        let mut style = t.type_ramp.body_sm;
        if matches!(line.msg, LogMsg::Error { .. } | LogMsg::Summary(_)) {
            style.weight = 600;
        }
        let ink = match line.msg {
            LogMsg::Dropped { .. } => c.text_2,
            _ => c.text,
        };
        let code = match &line.msg {
            LogMsg::Error { view, .. } | LogMsg::NotStarted(view) => view.code.clone(),
            _ => None,
        };
        let row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(
                Icon::new(icon)
                    .size(IconSize::Chrome)
                    .color(if ink_text { c.text } else { c.text_2 }.to_hsla()),
            )
            .child(
                Label::new(
                    core_text(&line.msg, cx),
                    TextSource::Fluent,
                    style,
                    ink.to_hsla(),
                )
                .id(ElementId::Name(format!("log-sentence-{ix}").into())),
            )
            .children(code.map(|code| {
                div()
                    .flex_none()
                    .px(px(4.0))
                    .rounded(px(3.0))
                    .bg(c.seam_soft.to_hsla())
                    .child(
                        Label::new(
                            SharedString::from(display_text(&code)),
                            TextSource::Data,
                            t.type_ramp.mono_micro,
                            c.text_2.to_hsla(),
                        )
                        .id(ElementId::Name(format!("log-code-{ix}").into())),
                    )
            }));
        Some(row.into_any_element())
    }

    fn detail_row(&self, ix: usize, cx: &App) -> Option<AnyElement> {
        let line = self.lines.get(ix)?;
        let (LogMsg::Error { view, .. } | LogMsg::NotStarted(view)) = &line.msg else {
            return None;
        };
        let t = Theme::get(cx).tokens;
        let ink = t.colors.text_2.to_hsla();
        let id = ElementId::Name(format!("log-detail-{ix}").into());
        let label = match &view.server_text {
            // The server's words, verbatim and whole (the tooltip keeps what the row cuts).
            Some(text) => Label::new(
                SharedString::from(display_text(text)),
                TextSource::Data,
                t.type_ramp.mono_data,
                ink,
            ),
            None => Label::new(
                tr_id(cx, view.title_id),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                ink,
            ),
        };
        Some(
            div()
                .min_w(px(0.0))
                .child(label.truncate(true).id(id))
                .into_any_element(),
        )
    }

    fn hint_row(&self, ix: usize, cx: &App) -> Option<AnyElement> {
        let line = self.lines.get(ix)?;
        let (LogMsg::Error { view, .. } | LogMsg::NotStarted(view)) = &line.msg else {
            return None;
        };
        let hint = view.hint_id?;
        let t = Theme::get(cx).tokens;
        Some(
            Label::new(
                tr_id(cx, hint),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                t.colors.text_2.to_hsla(),
            )
            .id(ElementId::Name(format!("log-hint-{ix}").into()))
            .into_any_element(),
        )
    }
}

impl hacivat_ui_grid::RowRenderer for LogRows {
    fn len(&self) -> u64 {
        self.items.len() as u64
    }

    fn row(&self, ix: u64, _window: &mut Window, cx: &mut App) -> AnyElement {
        self.built.set(self.built.get() + 1);
        let Some(item) = usize::try_from(ix).ok().and_then(|i| self.items.get(i)) else {
            return div().into_any_element();
        };
        let (line, selector, indent, content) = match *item {
            Item::Line(l) => (l, format!("log-row-{l}"), 12.0, self.line_row(l, cx)),
            Item::Detail(l) => (
                l,
                format!("log-row-{l}-detail"),
                32.0,
                self.detail_row(l, cx),
            ),
            Item::Hint(l) => (l, format!("log-row-{l}-hint"), 32.0, self.hint_row(l, cx)),
        };
        let mut row = div()
            .id(ElementId::Name(selector.clone().into()))
            .debug_selector(move || selector)
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(LOG_ROW_PX))
            .pl(px(indent))
            .pr(px(12.0))
            .overflow_hidden()
            .children(content);
        if let Some(go) = self.on_click(line) {
            row = row.cursor_pointer().on_click(move |_, _, cx| go(cx));
        }
        row.into_any_element()
    }
}
