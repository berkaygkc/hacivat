//! The parameter prompt (SQL-PARAM-01, HV-EXEC-03; M3b Task 24, `design/m3b-safe` surface 8):
//! the values a run's placeholders take, asked once per name before anything is classified or
//! sent (P-M3b-2: the gate classifies the text rewritten with them).
//!
//! ## m3b-t24-exec-ui
//!
//! - **One row per parameter** ([`ParamPrompt`]): its name in Fragment Mono (data), a mono
//!   value field, the type hint ("unknown": a prompt carries no type), a **NULL** toggle. A
//!   named parameter is one row for every unit that names it; an ordinal or anonymous one (`?`,
//!   `$n`) is a row of its own unit (T15 I4), and its answer is keyed so (`(unit, name)`).
//! - **Remembered values pre-fill, never skip** (D8 ruling, R-M3-79): a value remembered for the
//!   profile in this app session fills its field and shows "remembered"; the prompt still asks.
//!   The caller remembers only answers whose prompt is [`ParamPrompt::rememberable`].
//! - **Values are data** (R-M3-79, `alg/20` :245): the text is sent as typed, never formatted or
//!   re-cast with the UI locale. Machine format first: a canonical integer is bound with an
//!   integer hint, `\x` and hex pairs as bytes, anything else as text (a decimal stays exact as
//!   text). A Turkish-grouped number (`1.234`, `12.345.678`) where the format locale groups
//!   with a dot is **ambiguous**: the field is flagged and Run stays disabled until the value
//!   is edited or "Use 1.234" confirms the machine reading (NOTES ruling 17, fail-closed).
//!   Bytes written `\x…` that are not hex pairs are refused on every engine (the T13 M-6 carry:
//!   SQLite would store the text silently): flagged, Run disabled.
//! - **Notices**: a template variable (`${var}`) refuses the statement (Run disabled; the
//!   orchestrator logs the refusal too); PostgreSQL's `?` is an operator, said once.
//! - **Keys**: the first empty value has the focus (else the first value); Tab moves value →
//!   NULL → next row, then Cancel and Run (trapped); Enter in a value runs when Run is enabled;
//!   Escape, ⌘W, the close button and Cancel send nothing. A prompt dropped without an answer
//!   (its window closed, the app quit) answers `None`: nothing is sent.

use std::sync::Arc;

use tokio::sync::oneshot;

use hacivat_l10n::ArgValue;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::DialogSize;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::popover::{A11yNode, Declare as _, Tip};
use hacivat_ui::text::{Label, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::FluentBuilder as _;
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, ClickEvent, Context, Entity, EntityId, FocusHandle, Focusable, Global,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use hacivat_values::{BindValue, LogicalType};

use super::{answered, workspace_window};
use crate::connections::prompt_kit::{
    acting_enter, alert_line, card, cycle_focus, header, plain_input, push_modal, remove_modal,
    trapped_tab,
};
use crate::exec_ui::confirm::{caption, closes_dialog, grouped_footer, scroll_body, sql_inset};
use hacivat_exec::ParamPrompt;

/// The prompt's layer ID.
pub const PARAMS_LAYER_ID: &str = "params-prompt";

/// The answers, each with the prompt it answers (keyed by name and, for an ordinal or anonymous
/// parameter, its unit); `None` when the prompt was cancelled: nothing is sent.
pub type ParamValues = Option<Vec<(ParamPrompt, BindValue)>>;

/// Why the statement cannot run as written, or what the user should know (NOTES surface 8).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PromptNotice {
    /// `${var}` is a template variable, not a parameter: the statement can't run (Run disabled).
    Template(Arc<str>),
    /// On PostgreSQL `?` is an operator here (it is not a parameter unless the setting says so).
    PgQuestionMark,
}

/// What the prompt shows beyond its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptContext {
    /// The connection's name (data) and environment.
    pub connection: Arc<str>,
    pub env: Option<EnvTag>,
    pub engine: DbmsKind,
    /// The statements asked for, masked (the SQL inset).
    pub sql: Arc<str>,
    pub notices: Vec<PromptNotice>,
    /// The format locale groups thousands with a dot (`tr`): `1.234` is ambiguous there.
    pub grouping_dot: bool,
}

/// How a value field reads (machine format first; module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueRead {
    Value(BindValue),
    /// A number grouped with dots where the locale groups so: `value` reads as a decimal,
    /// `plain` is what the user may have meant.
    Ambiguous {
        value: String,
        plain: String,
    },
    /// Starts with `\x` (or `\X`) but is not `\x` and pairs of hex digits.
    BadHex,
}

/// Whether `s` is `\d{1,3}(\.\d{3})+`, optionally signed.
fn dot_grouped(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    let mut groups = body.split('.');
    let Some(first) = groups.next() else {
        return false;
    };
    let digits = |g: &str| !g.is_empty() && g.bytes().all(|b| b.is_ascii_digit());
    let rest: Vec<&str> = groups.collect();
    digits(first)
        && first.len() <= 3
        && !rest.is_empty()
        && rest.iter().all(|g| g.len() == 3 && digits(g))
}

/// Whether `s` is the canonical decimal spelling of an `i64` (no `+`, no leading zero).
fn canonical_int(s: &str) -> bool {
    s.parse::<i64>().is_ok_and(|n| n.to_string() == s)
}

/// Whether `s` is `\x` followed by pairs of hex digits (`\x` alone: no bytes).
fn hex_ok(s: &str) -> Option<bool> {
    if s.starts_with("\\X") {
        return Some(false);
    }
    let hex = s.strip_prefix("\\x")?;
    Some(hex.len() % 2 == 0 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Reads one field (module docs): `null` sends NULL; `confirmed` is the text whose machine
/// reading the user accepted with "Use …".
pub fn read_value(
    text: &str,
    null: bool,
    grouping_dot: bool,
    confirmed: Option<&str>,
) -> ValueRead {
    if null {
        return ValueRead::Value(BindValue::Null);
    }
    let typed = |hint: LogicalType| {
        ValueRead::Value(BindValue::Typed {
            hint,
            text: Arc::from(text),
        })
    };
    match hex_ok(text) {
        Some(true) => return typed(LogicalType::Bytes { max: None }),
        Some(false) => return ValueRead::BadHex,
        None => {}
    }
    if canonical_int(text) {
        return typed(LogicalType::Int {
            bits: 64,
            signed: true,
        });
    }
    if grouping_dot && dot_grouped(text) && confirmed != Some(text) {
        return ValueRead::Ambiguous {
            value: text.to_string(),
            plain: text.replace('.', ""),
        };
    }
    typed(LogicalType::Text {
        max_chars: None,
        fixed: false,
    })
}

/// Whether the format locale `locale` (BCP 47, the OS region) groups thousands with a dot, as
/// the results grid formats numbers there (`hacivat_values::valfmt`).
pub fn groups_with_dot(locale: &str) -> bool {
    use hacivat_values::valfmt::{
        BinaryDisplay, ColumnFacts, DisplaySettings, Purpose, format_cell,
    };
    let settings = DisplaySettings::new(locale, None);
    let logical = LogicalType::Int {
        bits: 64,
        signed: true,
    };
    let col = ColumnFacts {
        logical: &logical,
        native_name: "bigint",
        id_like: false,
        binary: BinaryDisplay::default(),
        engine: DbmsKind::Postgres,
    };
    let shown = format_cell(
        hacivat_values::ValueRef::Int(1_234_567),
        &col,
        &settings,
        Purpose::Cell,
    );
    shown.text.contains('.')
}

/// The open prompts (tests read the topmost; at most one per run is open).
#[derive(Default)]
struct OpenParams(Vec<WeakEntity<ParamsPrompt>>);

impl Global for OpenParams {}

/// The topmost open parameter prompt (tests drive it).
pub fn open_prompt(cx: &App) -> Option<Entity<ParamsPrompt>> {
    cx.try_global::<OpenParams>()
        .and_then(|o| o.0.iter().rev().find_map(WeakEntity::upgrade))
}

/// Asks for `prompts` in the workspace window (module docs); `None` when the user cancels or
/// the prompt goes without an answer. Without a workspace window: `None` at once.
pub fn open(
    prompts: Vec<ParamPrompt>,
    ctx: PromptContext,
    cx: &mut App,
) -> oneshot::Receiver<ParamValues> {
    if workspace_window(cx).is_none() {
        return answered(None);
    }
    let (reply, rx) = oneshot::channel();
    cx.defer(move |cx| {
        let Some(handle) = workspace_window(cx) else {
            let _ = reply.send(None);
            return;
        };
        let mut reply = Some(reply);
        let opened = handle.update(cx, |_, window, cx| {
            if let Some(reply) = reply.take() {
                show(prompts, ctx, reply, window, cx);
            }
        });
        if opened.is_err()
            && let Some(reply) = reply.take()
        {
            let _ = reply.send(None);
        }
    });
    rx
}

/// Pushes the prompt over `window`'s workspace.
fn show(
    prompts: Vec<ParamPrompt>,
    ctx: PromptContext,
    reply: oneshot::Sender<ParamValues>,
    window: &mut Window,
    cx: &mut App,
) -> EntityId {
    let view = cx.new(|cx| ParamsPrompt::new(prompts, ctx, reply, window, cx));
    {
        let open = cx.default_global::<OpenParams>();
        open.0.retain(|p| p.upgrade().is_some());
        open.0.push(view.downgrade());
    }
    let first = view.read(cx).initial_focus(cx);
    if !push_modal(PARAMS_LAYER_ID, view.clone().into(), &first, window, cx) {
        view.update(cx, |p, _| p.finish(None));
    }
    view.entity_id()
}

/// One row: the field, its NULL toggle, the value the user confirmed with "Use …".
struct Row {
    input: Entity<TextInput>,
    null: bool,
    null_handle: FocusHandle,
    use_handle: FocusHandle,
    confirmed: Option<String>,
}

/// The parameter prompt (module docs).
pub struct ParamsPrompt {
    prompts: Vec<ParamPrompt>,
    ctx: PromptContext,
    rows: Vec<Row>,
    reply: Option<oneshot::Sender<ParamValues>>,
    root: FocusHandle,
    cancel: FocusHandle,
    run: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for ParamsPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl Drop for ParamsPrompt {
    /// A prompt that goes without an answer answers `None`: nothing is sent.
    fn drop(&mut self) {
        self.finish(None);
    }
}

/// The text a remembered value pre-fills, and whether it is NULL.
fn prefill(v: Option<&BindValue>) -> (String, bool) {
    match v {
        Some(BindValue::Typed { text, .. }) => (text.to_string(), false),
        Some(BindValue::Null) => (String::new(), true),
        None => (String::new(), false),
    }
}

impl ParamsPrompt {
    fn new(
        prompts: Vec<ParamPrompt>,
        ctx: PromptContext,
        reply: oneshot::Sender<ParamValues>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let mut rows = Vec::with_capacity(prompts.len());
        for p in &prompts {
            let input = plain_input(cx, true);
            let (text, null) = prefill(p.remembered.as_ref());
            input.update(cx, |i, _| i.set_value(text));
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submitted => this.submit(window, cx),
                    InputEvent::Cancelled => this.cancel(window, cx),
                    InputEvent::Changed => cx.notify(),
                    InputEvent::CompositionChanged(_) => {}
                },
            ));
            rows.push(Row {
                input,
                null,
                null_handle: cx.focus_handle().tab_stop(true),
                use_handle: cx.focus_handle().tab_stop(true),
                confirmed: None,
            });
        }
        ParamsPrompt {
            prompts,
            ctx,
            rows,
            reply: Some(reply),
            root: cx.focus_handle(),
            cancel: cx.focus_handle().tab_stop(true),
            run: cx.focus_handle().tab_stop(true),
            _subscriptions: subscriptions,
        }
    }

    /// The first empty value field, else the first field, else Cancel.
    fn initial_focus(&self, cx: &App) -> FocusHandle {
        let fields: Vec<(&Row, FocusHandle)> = self
            .rows
            .iter()
            .map(|r| (r, r.input.read(cx).focus_handle(cx)))
            .collect();
        fields
            .iter()
            .find(|(r, _)| !r.null && r.input.read(cx).value().is_empty())
            .or_else(|| fields.first())
            .map_or_else(|| self.cancel.clone(), |(_, h)| h.clone())
    }

    /// Sends `values` once.
    fn finish(&mut self, values: ParamValues) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(values);
        }
    }

    /// How row `i` reads now.
    pub fn read(&self, i: usize, cx: &App) -> Option<ValueRead> {
        let r = self.rows.get(i)?;
        Some(read_value(
            r.input.read(cx).value(),
            r.null,
            self.ctx.grouping_dot,
            r.confirmed.as_deref(),
        ))
    }

    /// Whether a template variable refuses the statement.
    fn refused(&self) -> bool {
        self.ctx
            .notices
            .iter()
            .any(|n| matches!(n, PromptNotice::Template(_)))
    }

    /// Whether Run may answer: no refusal, every row a value.
    pub fn run_enabled(&self, cx: &App) -> bool {
        !self.refused()
            && (0..self.rows.len()).all(|i| matches!(self.read(i, cx), Some(ValueRead::Value(_))))
    }

    /// The values to send, when Run may answer.
    fn values(&self, cx: &App) -> ParamValues {
        if !self.run_enabled(cx) {
            return None;
        }
        self.prompts
            .iter()
            .enumerate()
            .map(|(i, p)| match self.read(i, cx) {
                Some(ValueRead::Value(v)) => Some((p.clone(), v)),
                _ => None,
            })
            .collect()
    }

    /// Run: answers the values and closes, when Run is enabled.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(values) = self.values(cx) else {
            return;
        };
        self.finish(Some(values));
        let view = cx.entity_id();
        window.defer(cx, move |window, cx| remove_modal(view, window, cx));
    }

    /// Cancel: sends nothing and closes.
    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish(None);
        let view = cx.entity_id();
        window.defer(cx, move |window, cx| remove_modal(view, window, cx));
    }

    /// Sets row `i`'s value (tests type through this).
    pub fn set_text(&mut self, i: usize, text: &str, cx: &mut Context<Self>) {
        if let Some(r) = self.rows.get(i) {
            r.input.update(cx, |input, cx| {
                input.set_value(text.to_string());
                cx.notify();
            });
            cx.notify();
        }
    }

    /// Row `i`'s value as typed.
    pub fn text(&self, i: usize, cx: &App) -> Option<String> {
        self.rows
            .get(i)
            .map(|r| r.input.read(cx).value().to_string())
    }

    /// Whether row `i` sends NULL.
    pub fn is_null(&self, i: usize) -> bool {
        self.rows.get(i).is_some_and(|r| r.null)
    }

    /// Toggles row `i`'s NULL.
    pub fn toggle_null(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(r) = self.rows.get_mut(i) {
            r.null = !r.null;
            let null = r.null;
            r.input.update(cx, |input, cx| {
                input.set_read_only(null);
                cx.notify();
            });
            cx.notify();
        }
    }

    /// "Use 1.234": row `i`'s ambiguous value is read as the machine reading (a decimal).
    pub fn use_value(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(r) = self.rows.get_mut(i) {
            r.confirmed = Some(r.input.read(cx).value().to_string());
            cx.notify();
        }
    }

    /// The rows' prompts, in order.
    pub fn prompts(&self) -> &[ParamPrompt] {
        &self.prompts
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        match stroke.key.as_str() {
            "escape" => self.cancel(window, cx),
            "w" if closes_dialog(stroke, cx) => self.cancel(window, cx),
            "enter" => {
                if !acting_enter(event) {
                    return cx.stop_propagation();
                }
                if self.cancel.is_focused(window) {
                    self.cancel(window, cx);
                } else if self.run.is_focused(window) {
                    self.submit(window, cx);
                } else if let Some(i) = self
                    .rows
                    .iter()
                    .position(|r| r.null_handle.is_focused(window))
                {
                    self.toggle_null(i, cx);
                } else if let Some(i) = self
                    .rows
                    .iter()
                    .position(|r| r.use_handle.is_focused(window))
                {
                    self.use_value(i, cx);
                } else {
                    return;
                }
            }
            " " | "space" => {
                if let Some(i) = self
                    .rows
                    .iter()
                    .position(|r| r.null_handle.is_focused(window))
                {
                    self.toggle_null(i, cx);
                } else {
                    return;
                }
            }
            _ if trapped_tab(event) => {
                let order = self.tab_order(cx);
                cycle_focus(&order, stroke.modifiers.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Value → NULL (→ Use, when flagged) per row, then Cancel and Run (when enabled).
    fn tab_order(&self, cx: &App) -> Vec<FocusHandle> {
        let mut order = Vec::new();
        for (i, r) in self.rows.iter().enumerate() {
            order.push(r.input.read(cx).focus_handle(cx));
            order.push(r.null_handle.clone());
            if matches!(self.read(i, cx), Some(ValueRead::Ambiguous { .. })) {
                order.push(r.use_handle.clone());
            }
        }
        order.push(self.cancel.clone());
        if self.run_enabled(cx) {
            order.push(self.run.clone());
        }
        order
    }
}

/// A message in the UI language.
fn text(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    crate::exec_ui::marks::message(cx, id, args)
}

impl Render for ParamsPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let title = text(cx, "hv-params-title", vec![]);
        let count = text(
            cx,
            "hv-params-sub",
            vec![("n", ArgValue::Count(self.prompts.len() as u64))],
        );
        let sub = crate::exec_ui::confirm::subtitle(
            &theme,
            self.ctx.env,
            &crate::connections::prompt_kit::visible(&self.ctx.connection, 200),
            None,
            Some(count),
        );
        let mut body = div().w_full().flex().flex_col().gap(px(t.spacing.lg));
        body = body.child(sql_inset(&theme, &self.ctx.sql, "params-sql"));

        // The rows: Name 104 · Value 1fr · Type 80 · NULL 48 (NOTES "Parameter row").
        let grid = |name: AnyEl, value: AnyEl, kind: AnyEl, null: AnyEl| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(div().w(px(104.0)).flex_none().min_w_0().child(name))
                .child(div().flex_1().min_w_0().child(value))
                .child(div().w(px(80.0)).flex_none().min_w_0().child(kind))
                .child(div().w(px(48.0)).flex_none().child(null))
        };
        let head = |id: &'static str, s: &'static str| -> AnyEl {
            caption(&theme, text(cx, id, vec![]), s).into_any_element()
        };
        let mut rows = div().flex().flex_col().gap(px(t.spacing.md));
        if !self.prompts.is_empty() {
            rows = rows.child(grid(
                head("hv-params-col-name", "params-col-name"),
                head("hv-params-col-value", "params-col-value"),
                head("hv-params-col-type", "params-col-type"),
                div().into_any_element(),
            ));
        }
        for (i, (p, r)) in self.prompts.iter().zip(&self.rows).enumerate() {
            let read = self.read(i, cx);
            let name_text =
                SharedString::from(crate::connections::prompt_kit::visible(&p.name, 200));
            let name_tip = name_text.clone();
            let name: AnyEl = div()
                .id(("params-name", i))
                .tooltip(move |_, cx| Tip::view(name_tip.clone(), cx))
                .child(
                    Label::new(
                        name_text,
                        TextSource::Data,
                        t.type_ramp.mono_data,
                        c.text.to_hsla(),
                    )
                    .truncate(true),
                )
                .into_any_element();
            let flagged = matches!(read, Some(ValueRead::Ambiguous { .. } | ValueRead::BadHex));
            let mut field = div()
                .w_full()
                .debug_selector(move || format!("params-value-{i}"))
                .rounded(px(t.radii.sm))
                .child(r.input.clone());
            if flagged {
                // The flagged field's border turns text-2 (no hue: Hue Is Meaning).
                field = field.border_1().border_color(c.text_2.to_hsla());
            }
            let mut value = div().flex().flex_col().gap(px(t.spacing.xs)).child(field);
            if p.remembered.is_some() {
                let tip = text(cx, "hv-params-remembered-tip", vec![]);
                value = value.child(
                    div()
                        .id(("params-remembered", i))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(t.spacing.xs))
                        .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                        .child(
                            Icon::new(IconName::History)
                                .size(IconSize::Chrome)
                                .color(c.text_2.to_hsla()),
                        )
                        .child(
                            Label::new(
                                text(cx, "hv-params-remembered", vec![]),
                                TextSource::Fluent,
                                t.type_ramp.caption,
                                c.text_2.to_hsla(),
                            )
                            .id(("params-remembered-label", i)),
                        ),
                );
            }
            // A flag spans the row's width under it (NOTES "Flag line"), so it wraps there.
            let mut below: Vec<AnyEl> = Vec::new();
            match &read {
                Some(ValueRead::Ambiguous { value: v, plain }) => {
                    let line = text(
                        cx,
                        "hv-params-ambiguous",
                        vec![
                            ("value", ArgValue::Text(v.clone())),
                            ("plain", ArgValue::Text(plain.clone())),
                        ],
                    );
                    let use_label = text(
                        cx,
                        "hv-params-ambiguous-use",
                        vec![("value", ArgValue::Text(v.clone()))],
                    );
                    below.push(flag_line(&theme, line, "params-ambiguous").into_any_element());
                    below.push(
                        div()
                            .pl(px(20.0))
                            .flex()
                            .flex_row()
                            .child(
                                Button::new(("params-use", i), use_label)
                                    .kind(ButtonKind::Default)
                                    .compact(true)
                                    .on_raised(true)
                                    .focus_handle(r.use_handle.clone())
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.use_value(i, cx);
                                    })),
                            )
                            .into_any_element(),
                    );
                }
                Some(ValueRead::BadHex) => {
                    below.push(
                        flag_line(
                            &theme,
                            text(cx, "hv-params-bad-hex", vec![]),
                            "params-bad-hex",
                        )
                        .into_any_element(),
                    );
                }
                _ => {}
            }
            // The type hint wraps in its 80 px column (never ellipsized).
            let kind: AnyEl = div()
                .w_full()
                .min_w_0()
                .child(
                    paragraph(
                        text(cx, "hv-params-type-unknown", vec![]),
                        TextSource::Fluent,
                        t.type_ramp.mono_micro,
                        c.text_2.to_hsla(),
                    )
                    .id("params-type"),
                )
                .into_any_element();
            let null_tip = text(cx, "hv-params-null-tip", vec![]);
            let pressed = r.null;
            let null: AnyEl = div()
                .id(("params-null", i))
                .debug_selector(move || format!("params-null-{i}"))
                .track_focus(&r.null_handle)
                .declare(Role::Button, null_tip.clone(), None)
                .h(px(22.0))
                .px(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(t.radii.sm))
                .border_1()
                .border_color(if pressed { c.blue } else { c.seam_strong }.to_hsla())
                .when(pressed, |d| d.bg(c.blue_tint_strong.to_hsla()))
                .cursor_pointer()
                .tooltip(move |_, cx| Tip::view(null_tip.clone(), cx))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_null(i, cx)))
                .child(Label::new(
                    "NULL",
                    TextSource::Data,
                    t.type_ramp.mono_micro,
                    c.text.to_hsla(),
                ))
                .into_any_element();
            rows = rows.child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.xs))
                    .child(grid(name, value.into_any_element(), kind, null))
                    .children(below),
            );
        }
        body = body.child(rows);
        for n in &self.ctx.notices {
            let line = match n {
                PromptNotice::Template(var) => text(
                    cx,
                    "hv-params-template",
                    vec![(
                        "var",
                        ArgValue::Text(crate::connections::prompt_kit::visible(var, 200)),
                    )],
                ),
                PromptNotice::PgQuestionMark => text(cx, "hv-params-pg-q", vec![]),
            };
            body = body.child(flag_line(&theme, line, "params-notice"));
        }
        body = body.child(
            paragraph(
                text(cx, "hv-params-note", vec![]),
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text_2.to_hsla(),
            )
            .id("params-note"),
        );

        let enabled = self.run_enabled(cx);
        let resolve = text(cx, "hv-params-resolve-first", vec![]);
        let run = div()
            .id("params-run-host")
            .when(!enabled, |d| {
                d.tooltip(move |_, cx| Tip::view(resolve.clone(), cx))
            })
            .child(
                Button::new("params-run", text(cx, "hv-params-run", vec![]))
                    .kind(ButtonKind::Primary)
                    .icon(IconName::Play)
                    .disabled(!enabled)
                    .on_raised(true)
                    .focus_handle(self.run.clone())
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                        if event.click_count() > 1 {
                            return;
                        }
                        this.submit(window, cx);
                    })),
            );
        let cancel = Button::new("params-cancel", text(cx, "hv-params-cancel", vec![]))
            .kind(ButtonKind::Default)
            .on_raised(true)
            .focus_handle(self.cancel.clone())
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.cancel(window, cx)));
        let card = card(&theme, DialogSize::Wide, PARAMS_LAYER_ID)
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .child(header(
                &theme,
                title.clone(),
                "params-title",
                Some(sub),
                cx.listener(|this, _, window, cx| this.cancel(window, cx)),
                cx,
            ))
            .child(scroll_body(body))
            .child(grouped_footer(
                &theme,
                Vec::new(),
                vec![cancel.into_any_element(), run.into_any_element()],
            ));
        A11yNode::new("params-node", Role::Dialog, title, true, card)
    }
}

type AnyEl = hacivat_ui_core::gpui_surface::AnyElement;

/// A flag line (NOTES "Flag line"): a 12 px ink circle-alert glyph and the text, which wraps.
fn flag_line(theme: &Theme, line: SharedString, id: &'static str) -> impl IntoElement {
    let t = theme.tokens;
    let c = &t.colors;
    let _ = alert_line;
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(t.spacing.sm))
        .child(
            div().flex_none().pt(px(2.0)).child(
                Icon::new(IconName::CircleAlert)
                    .size(IconSize::Chrome)
                    .color(c.text.to_hsla()),
            ),
        )
        .child(
            // A full-width box, so the paragraph is measured at the width it wraps to; 12.5 px
            // (the 12 px caption's line would cut a third line, T21's lesson).
            div().flex_1().min_w_0().child(
                div().w_full().min_w_0().child(
                    paragraph(
                        line,
                        TextSource::Fluent,
                        t.type_ramp.body_sm,
                        c.text.to_hsla(),
                    )
                    .id(id),
                ),
            ),
        )
}

// ---- shots fixtures (design/m3b-safe surface 8) -------------------------------------------------

#[cfg(feature = "shots")]
pub mod fixtures {
    //! The prompt's states for the shots harness (Task 24; Task 29 adds the cross-surface ones).

    use std::sync::Arc;

    use hacivat_exec::ParamPrompt;
    use hacivat_proto::{DbmsKind, EnvTag};
    use hacivat_ui_core::gpui_surface::{App, Global, Window};
    use hacivat_values::{BindValue, LogicalType};
    use tokio::sync::oneshot;

    use super::{ParamValues, PromptContext, PromptNotice, show};

    #[derive(Default)]
    struct Held(Vec<oneshot::Receiver<ParamValues>>);

    impl Global for Held {}

    /// One state of the prompt (NOTES surface 8).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum State {
        TwoNames,
        Remembered,
        Turkish,
        Null,
        Template,
        PgQuestionMark,
        BadHex,
    }

    pub const STATES: [State; 7] = [
        State::TwoNames,
        State::Remembered,
        State::Turkish,
        State::Null,
        State::Template,
        State::PgQuestionMark,
        State::BadHex,
    ];

    impl State {
        pub fn name(self) -> &'static str {
            match self {
                State::TwoNames => "params-two-names",
                State::Remembered => "params-remembered",
                State::Turkish => "params-turkish",
                State::Null => "params-null",
                State::Template => "params-template",
                State::PgQuestionMark => "params-pg-q",
                State::BadHex => "params-bad-hex",
            }
        }
    }

    fn prompt(name: &str, remembered: Option<BindValue>) -> ParamPrompt {
        ParamPrompt {
            name: Arc::from(name),
            occurrences: 1,
            remembered,
            unit: None,
        }
    }

    /// Opens `state` in `window`.
    pub fn open(state: State, window: &mut Window, cx: &mut App) {
        let sql = "SELECT id, ad, toplam\nFROM musteriler\nWHERE musteri_id = :musteri_id AND tarih >= :baslangic";
        let mut ctx = PromptContext {
            connection: Arc::from("orders-staging"),
            env: Some(EnvTag::Staging),
            engine: DbmsKind::Postgres,
            sql: Arc::from(sql),
            notices: Vec::new(),
            grouping_dot: true,
        };
        let remembered = BindValue::Typed {
            hint: LogicalType::Int {
                bits: 64,
                signed: true,
            },
            text: Arc::from("4127"),
        };
        let prompts = match state {
            State::Remembered => vec![
                prompt("musteri_id", Some(remembered)),
                prompt("baslangic", None),
            ],
            State::Template => {
                ctx.notices = vec![PromptNotice::Template(Arc::from("${tablo}"))];
                ctx.sql = Arc::from("SELECT * FROM ${tablo} WHERE id = :id");
                vec![prompt("id", None)]
            }
            State::PgQuestionMark => {
                ctx.notices = vec![PromptNotice::PgQuestionMark];
                ctx.sql = Arc::from("SELECT * FROM belgeler WHERE veri ? 'etiket' AND id = :id");
                vec![prompt("id", None)]
            }
            _ => vec![prompt("musteri_id", None), prompt("baslangic", None)],
        };
        let (reply, rx) = oneshot::channel();
        cx.default_global::<Held>().0.push(rx);
        let view = show(prompts, ctx, reply, window, cx);
        let Some(p) = super::open_prompt(cx).filter(|p| p.entity_id() == view) else {
            return;
        };
        p.update(cx, |p, cx| match state {
            State::Turkish => {
                p.set_text(0, "1.234", cx);
                p.set_text(1, "2026-11-01", cx);
            }
            State::Null => {
                p.set_text(0, "4127", cx);
                p.toggle_null(1, cx);
            }
            State::BadHex => {
                p.set_text(0, "\\x4g", cx);
                p.set_text(1, "2026-11-01", cx);
            }
            State::TwoNames => p.set_text(0, "4127", cx),
            State::Remembered | State::Template | State::PgQuestionMark => {}
        });
    }
}
