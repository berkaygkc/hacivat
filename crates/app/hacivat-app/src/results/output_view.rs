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
//!
//! ## m3b-t25-results (design/m3b-safe surface 10; ledger 336/342/362)
//! - **Only new lines are read.** The tile asks the log for the lines after the last one it
//!   took (`SessionLog::lines_since`) and [`LogRows::append`]s them: the work per change is the
//!   new lines', never the whole ring's. The rows keep at most `SessionLog::CAPACITY` lines,
//!   as the ring does; lines the ring pushed out before the tile read them become one
//!   "n events not shown" line ([`LogRows::taken`] counts what the last change took).
//! - **Notices and warnings** (R-M3-77; NOTES ruling 23): one row each: an info glyph (`text-2`)
//!   for a notice, the triangle in ink and a 600 label for a warning; the label names the
//!   server's severity (`exec-notice-*`, so no separate severity chip); the code as a chip; then
//!   the server's text verbatim in mono `text-2` on the same row (ellipsized, whole in the
//!   tooltip). Messages only: the drivers keep no DETAIL or HINT.
//! - **The code chip of a MySQL or MariaDB error** shows the vendor code beside the SQLSTATE,
//!   `1792 · 25006` (ledger 340), as the server's own client does.
//! - **Line selection** (element-level keys while the log has the focus): ↑/↓ move the selected
//!   line, Enter goes to its statement (`ResultsEvent::GoToSource`); a click selects a line.

use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::Arc;

use hacivat_exec::{LogArg, LogLine, LogMsg, SessionLog, Severity, duration_message, span_message};
use hacivat_l10n::ArgValue;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::text::{Label, TextSource};
// ## m3a-t30 fix round 1 (I3): controls in server text (a line break, a bidi override) are
// shown as their chips, as in a cell, never obeyed or passed to the shaper.
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ElementId, Entity, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use hacivat_ui_grid::VirtualList;
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
        // ## m3b-t25-results: a warning's triangle in ink, a notice's info glyph (NOTES
        // ruling 23; never amber or red).
        LogMsg::Notice { .. } if is_warning(msg) => (IconName::TriangleAlert, true),
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
        // ## m3b-t23-escalation (named exception): the escalation's and the error bar's lines
        // (design/m3b-safe surface 6: octagon-x, unplug).
        LogMsg::Terminated { .. } => (IconName::OctagonX, false),
        LogMsg::TerminateFailed => (IconName::OctagonX, true),
        LogMsg::Disconnected { .. } => (IconName::Unplug, false),
        LogMsg::RunStopped { .. } | LogMsg::RolledBackContinue { .. } => (IconName::Info, false),
        // ## m3b-t24-exec-ui (named exception): the run UI's lines before a run is sent: a
        // refusal in ink (as `NotStarted`), an answer as a note.
        LogMsg::RunUi { warn: true, .. } => (IconName::TriangleAlert, true),
        LogMsg::RunUi { warn: false, .. } => (IconName::Info, false),
    }
}

// ## m3b-t25-results

/// Whether `msg` is a server warning (a PostgreSQL `WARNING`, a MySQL warning).
fn is_warning(msg: &LogMsg) -> bool {
    matches!(msg, LogMsg::Notice { .. }) && msg.severity() == Severity::Warning
}

/// The code chip of a line: an error's SQLSTATE (with the vendor code before it on MySQL and
/// MariaDB, `1792 · 25006`), a notice's code. Codes are data, never translated.
pub fn code_chip(msg: &LogMsg) -> Option<String> {
    match msg {
        LogMsg::Error { view, .. } | LogMsg::NotStarted(view) => {
            let code = view.code.as_deref()?;
            Some(match view.vendor_code.as_deref() {
                Some(vendor) => format!("{vendor} \u{b7} {code}"),
                None => code.to_string(),
            })
        }
        LogMsg::Notice { code, .. } => code.as_deref().map(str::to_string),
        _ => None,
    }
}

/// One row of the list: a line, its server text (or failure kind), its hint. Lines are
/// numbered from the first the rows ever took, so dropping old ones moves no index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Line(u64),
    Detail(u64),
    Hint(u64),
}

impl Item {
    fn line(self) -> u64 {
        match self {
            Item::Line(l) | Item::Detail(l) | Item::Hint(l) => l,
        }
    }
}

/// The rows of a console's output log.
pub struct LogRows {
    console: ConsoleId,
    results: WeakEntity<ConsoleResults>,
    lines: VecDeque<LogLine>,
    /// The number of the first kept line (lines dropped from the front before it).
    first: u64,
    items: VecDeque<Item>,
    built: Cell<u64>,
    // ## m3b-t25-results
    /// The next `seq` to read from the log.
    next_seq: u64,
    /// The lines the last change took.
    taken: u64,
    /// The selected line (its number).
    selected: Option<u64>,
    /// The list these rows are in, and the log's focus (a click selects and focuses).
    list: Option<WeakEntity<VirtualList<LogRows>>>,
    focus: Option<FocusHandle>,
    /// "n events not shown": the lines the ring pushed out before they were read, since the
    /// last clear; the list's first row while there are any.
    missed: Option<LogLine>,
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
            lines: VecDeque::new(),
            first: 0,
            items: VecDeque::new(),
            built: Cell::new(0),
            next_seq: 0,
            taken: 0,
            selected: None,
            list: None,
            focus: None,
            missed: None,
        }
    }

    /// Shows `lines` (the log's current lines, oldest first) in place of what it showed.
    pub fn set_lines(&mut self, lines: Arc<[LogLine]>) {
        self.clear();
        // ## m3b-t25-results: the log's own "n events not shown" heads the list.
        let mut rest = lines.iter();
        if let Some(first) = lines.first()
            && matches!(first.msg, LogMsg::Dropped { .. })
        {
            self.missed = Some(first.clone());
            rest.next();
        }
        self.push_lines(rest.cloned());
        self.taken = lines.len() as u64;
        if let Some(last) = lines.last() {
            self.next_seq = last.seq.saturating_add(1);
        }
    }

    // ## m3b-t25-results

    /// Forgets every line (the log was cleared).
    pub fn clear(&mut self) {
        self.first = self.first.saturating_add(self.lines.len() as u64);
        self.lines.clear();
        self.items.clear();
        self.selected = None;
        self.missed = None;
    }

    /// Takes the log's new lines (`SessionLog::lines_since(next_seq)`): `gap` when the ring
    /// dropped lines before they were read, which become one "n events not shown" line.
    pub fn append(&mut self, new: &[LogLine], gap: bool) {
        self.taken = new.len() as u64;
        let Some(first) = new.first() else {
            return;
        };
        if gap {
            let missed = first.seq.saturating_sub(self.next_seq);
            if missed > 0 {
                let before = match self.missed.as_ref().map(|l| &l.msg) {
                    Some(LogMsg::Dropped { n }) => *n,
                    _ => 0,
                };
                self.missed = Some(LogLine {
                    seq: first.seq,
                    at: first.at,
                    severity: Severity::Warning,
                    msg: LogMsg::Dropped {
                        n: before.saturating_add(missed),
                    },
                    source: None,
                    unit: None,
                    tag: None,
                });
            }
        }
        self.push_lines(new.iter().cloned());
        if let Some(last) = new.last() {
            self.next_seq = last.seq.saturating_add(1);
        }
    }

    /// Reads `log`'s lines after the last one taken, and answers them. A change that brought
    /// none, while the log no longer holds the newest line shown, was a Clear: the rows forget
    /// their lines.
    pub fn take_new(&mut self, log: &SessionLog) -> Arc<[LogLine]> {
        let (new, gap) = log.lines_since(self.next_seq);
        if new.is_empty()
            && let Some(last) = self.lines.back()
            && log.lines_since(last.seq).0.is_empty()
        {
            self.clear();
            self.taken = 0;
            return new;
        }
        self.append(&new, gap);
        new
    }

    /// The `seq` the next read starts at.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// How many lines the last change took (a change costs its new lines only).
    pub fn taken(&self) -> u64 {
        self.taken
    }

    fn push_lines(&mut self, lines: impl Iterator<Item = LogLine>) {
        for line in lines {
            let n = self.first + self.lines.len() as u64;
            self.items.push_back(Item::Line(n));
            match &line.msg {
                LogMsg::Error { view, .. } | LogMsg::NotStarted(view) => {
                    self.items.push_back(Item::Detail(n));
                    if view.hint_id.is_some() {
                        self.items.push_back(Item::Hint(n));
                    }
                }
                _ => {}
            }
            self.lines.push_back(line);
            // As many as the ring keeps.
            if self.lines.len() > SessionLog::CAPACITY {
                self.lines.pop_front();
                self.first += 1;
                while self.items.front().is_some_and(|i| i.line() < self.first) {
                    self.items.pop_front();
                }
                if self.selected.is_some_and(|s| s < self.first) {
                    self.selected = None;
                }
            }
        }
    }

    /// The lines shown (a copy: the clipboard's and the tests' read), "n events not shown"
    /// first when the ring dropped lines unread.
    pub fn lines(&self) -> Vec<LogLine> {
        self.missed
            .iter()
            .chain(self.lines.iter())
            .cloned()
            .collect()
    }

    /// The lines shown, oldest first, without copying them.
    pub fn iter_lines(&self) -> impl Iterator<Item = &LogLine> {
        self.lines.iter()
    }

    /// How many lines are shown.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Whether no line is shown.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.missed.is_none()
    }

    /// The rows before the first line's: "n events not shown".
    fn header(&self) -> u64 {
        u64::from(self.missed.is_some())
    }

    /// The line numbered `n`.
    fn line(&self, n: u64) -> Option<&LogLine> {
        let ix = usize::try_from(n.checked_sub(self.first)?).ok()?;
        self.lines.get(ix)
    }

    /// Rows built since the list was made or [`reset_built`](Self::reset_built).
    pub fn built(&self) -> u64 {
        self.built.get()
    }

    pub fn reset_built(&self) {
        self.built.set(0);
    }

    /// The list these rows are in and the log's focus (set once by the tile).
    pub(crate) fn attach(&mut self, list: WeakEntity<VirtualList<LogRows>>, focus: FocusHandle) {
        self.list = Some(list);
        self.focus = Some(focus);
    }

    /// The selected line, as an index into [`lines`](Self::lines).
    pub fn selected(&self) -> Option<usize> {
        let n = self.selected?;
        usize::try_from(n.checked_sub(self.first)?).ok()
    }

    /// Selects the line at `ix` of [`lines`](Self::lines).
    pub fn select(&mut self, ix: usize) {
        if ix < self.lines.len() {
            self.selected = Some(self.first + ix as u64);
        }
    }

    /// Moves the selection `down` (or up) one line; the first move selects the newest line
    /// (up) or the oldest shown (down). Answers the row to scroll into view.
    pub fn step(&mut self, down: bool) -> Option<u64> {
        if self.lines.is_empty() {
            return None;
        }
        let last = self.first + self.lines.len() as u64 - 1;
        let next = match self.selected {
            None if down => self.first,
            None => last,
            Some(s) if down => (s + 1).min(last),
            Some(s) => s.saturating_sub(1).max(self.first),
        };
        self.selected = Some(next);
        self.row_of(next)
    }

    /// The row index of line `n`.
    fn row_of(&self, n: u64) -> Option<u64> {
        let at = self.items.partition_point(|i| i.line() < n);
        (at < self.items.len()).then_some(at as u64 + self.header())
    }

    /// Go to source of the selected line: the event, when the line names a statement.
    pub fn go_to_selected(&self) -> Option<ResultsEvent> {
        let line = self.line(self.selected?)?;
        let range = line.source.clone()?;
        Some(ResultsEvent::GoToSource {
            console: self.console,
            range,
        })
    }

    /// What a click on line `n` does: select its statement, when it names one and failed.
    fn on_click(&self, n: u64) -> Option<impl Fn(&mut App) + 'static> {
        let line = self.line(n)?;
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

    fn line_row(&self, n: u64, cx: &App) -> Option<AnyElement> {
        let line = self.line(n)?;
        // Data ids are line-relative (the row's index among the lines shown).
        let ix = n.saturating_sub(self.first).to_string();
        Some(line_row_of(line, &ix, cx))
    }
}

/// The row of `line`, its label ids suffixed `ix`.
fn line_row_of(line: &LogLine, ix: &str, cx: &App) -> AnyElement {
    let t = Theme::get(cx).tokens;
    let c = t.colors;
    let (icon, ink_text) = glyph(&line.msg);
    let mut style = t.type_ramp.body_sm;
    // ## m3b-t25-results: a warning's label is 600, as an error's.
    if matches!(line.msg, LogMsg::Error { .. } | LogMsg::Summary(_)) || is_warning(&line.msg) {
        style.weight = 600;
    }
    let ink = match line.msg {
        LogMsg::Dropped { .. } | LogMsg::NoticesDropped { .. } => c.text_2,
        _ => c.text,
    };
    let code = code_chip(&line.msg);
    let server_text = match &line.msg {
        LogMsg::Notice { text, .. } => Some(text.clone()),
        _ => None,
    };
    let row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .min_w(px(0.0))
        .child(
            Icon::new(icon)
                .size(IconSize::Chrome)
                .color(if ink_text { c.text } else { c.text_2 }.to_hsla()),
        )
        .child(
            div().flex_none().child(
                Label::new(
                    core_text(&line.msg, cx),
                    TextSource::Fluent,
                    style,
                    ink.to_hsla(),
                )
                .id(ElementId::Name(format!("log-sentence-{ix}").into())),
            ),
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
        }))
        // ## m3b-t25-results: a notice's text, verbatim, on its own row's end.
        .children(server_text.map(|text| {
            div().flex_1().min_w(px(0.0)).child(
                Label::new(
                    SharedString::from(display_text(&text)),
                    TextSource::Data,
                    t.type_ramp.mono_data,
                    c.text_2.to_hsla(),
                )
                .truncate(true)
                .id(ElementId::Name(format!("log-notice-{ix}").into())),
            )
        }));
    row.into_any_element()
}

impl LogRows {
    fn detail_row(&self, n: u64, cx: &App) -> Option<AnyElement> {
        let line = self.line(n)?;
        let (LogMsg::Error { view, .. } | LogMsg::NotStarted(view)) = &line.msg else {
            return None;
        };
        let t = Theme::get(cx).tokens;
        let ink = t.colors.text_2.to_hsla();
        let ix = n.saturating_sub(self.first);
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

    fn hint_row(&self, n: u64, cx: &App) -> Option<AnyElement> {
        let line = self.line(n)?;
        let (LogMsg::Error { view, .. } | LogMsg::NotStarted(view)) = &line.msg else {
            return None;
        };
        let hint = view.hint_id?;
        let t = Theme::get(cx).tokens;
        let ix = n.saturating_sub(self.first);
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
        self.items.len() as u64 + self.header()
    }

    fn row(&self, ix: u64, _window: &mut Window, cx: &mut App) -> AnyElement {
        self.built.set(self.built.get() + 1);
        // ## m3b-t25-results: "n events not shown" heads the list.
        if let Some(missed) = self.missed.as_ref().filter(|_| ix == 0) {
            return div()
                .id("log-row-dropped")
                .debug_selector(|| "log-row-dropped".to_string())
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .h(px(LOG_ROW_PX))
                .pl(px(12.0))
                .pr(px(12.0))
                .overflow_hidden()
                .child(line_row_of(missed, "dropped", cx))
                .into_any_element();
        }
        let ix = ix - self.header();
        let Some(item) = usize::try_from(ix)
            .ok()
            .and_then(|i| self.items.get(i))
            .copied()
        else {
            return div().into_any_element();
        };
        let line = item.line();
        let l = line.saturating_sub(self.first);
        let (selector, indent, content) = match item {
            Item::Line(n) => (format!("log-row-{l}"), 12.0, self.line_row(n, cx)),
            Item::Detail(n) => (format!("log-row-{l}-detail"), 32.0, self.detail_row(n, cx)),
            Item::Hint(n) => (format!("log-row-{l}-hint"), 32.0, self.hint_row(n, cx)),
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
        // ## m3b-t25-results: the selected line reads as a selection (blue tint, Hue Is
        // Meaning: blue is selection).
        if self.selected == Some(line) {
            row = row.bg(Theme::get(cx).tokens.colors.blue_tint.to_hsla());
        }
        let go = self.on_click(line);
        let (list, focus) = (self.list.clone(), self.focus.clone());
        row = row.on_click(move |_, window, cx| {
            if let Some(focus) = &focus {
                window.focus(focus, cx);
            }
            if let Some(list) = list.as_ref().and_then(WeakEntity::upgrade) {
                select_line(&list, line, cx);
            }
            if let Some(go) = &go {
                go(cx);
            }
        });
        if self.on_click(line).is_some() {
            row = row.cursor_pointer();
        }
        row.into_any_element()
    }
}

/// Selects line `n` of `list`'s rows.
fn select_line(list: &Entity<VirtualList<LogRows>>, n: u64, cx: &mut App) {
    list.update(cx, |l, cx| {
        let rows = l.rows_mut(cx);
        if rows.line(n).is_some() {
            rows.selected = Some(n);
        }
    });
}
