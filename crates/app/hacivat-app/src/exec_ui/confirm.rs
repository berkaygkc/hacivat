//! The confirmation dialog (R-M3-55, R-M3-84; M3b Task 21, `design/m3b-safe` surface 1) and the
//! app's `ConfirmPort`.
//!
//! ## m3b-t21-confirm
//!
//! **The dialog** ([`open`], [`open_in`]): a 480 px alertdialog, asked before anything is sent.
//! It names the connection (environment mark, connection in mono, engine, "statement k of n"),
//! shows the masked statement in a scroll inset of at most 6 lines, lists the gate's reasons
//! and what the statement does, and asks.
//! - **The safe answer is the default.** Cancel is Primary and has the initial focus; Execute,
//!   Execute all and "Execute and don't ask again" are Danger, except for a statement that only
//!   locks rows (`Warn`, RC-2: Default, never focused). Execute all is Danger unless this unit
//!   and every unit it answers are `Warn` (RC-1).
//! - **Enter** acts on key down, never held or with a modifier, and only on the focused button
//!   (Cancel at first); Enter in the type field does nothing. **Escape, the close button, ⌘W
//!   (Ctrl+W), closing the window and quitting answer Cancel**: a dialog dropped without an
//!   answer answers Cancel ([`ConfirmDialog`]'s `Drop`), and the quit guard answers every open
//!   one ([`answer_open_prompts_safely`]). Tab is trapped in the card in visual order: the type
//!   field, its Copy button, the run group, the stop group. No access keys (labels may sit next
//!   to SQL).
//! - **Type to confirm** (`ConfirmQuestion::type_to_confirm`, from `GateItem` unchanged): Execute
//!   and Execute all stay disabled until the field holds exactly those bytes (no case folding,
//!   no Turkish İ/i folding, no trimming, no normalisation). The name is shown made visible
//!   (bidi and invisible characters as `\u{…}`); its Copy button copies the exact bytes.
//! - **"Don't ask again"** is offered only when the question says so **and** never with a typed
//!   name, never for a statement Hacivat could not analyse or one that commits an open MySQL
//!   transaction, and on production never for `DdlDestructive`, `Admin`, `Unknown` or `Routine`
//!   (nor when the class is not known) ([`offers_suppress`]). A multi-statement unit with a write
//!   is never suppressible at the gate (`GateItem` carries that), and [`question_for`] hands it
//!   on unchanged. On production its label names the scope.
//!
//! **A run of several units** ([`ask_plan`], R-M3-84): every `Confirm`/`Warn` unit is asked in
//! order before the run is sent; [`answer_plan`] applies each answer with
//! `hacivat_exec::apply_answer`, which changes only the item's decision: the item's verdict
//! (`ExplainAnalyzeRollback`, `ImplicitCommit`, …), effects, class and fingerprint reach the run
//! unchanged. Execute all (RC-1) answers this unit and only the later units that would ask
//! **without** a typed name; a unit that needs its name typed still asks, and a
//! `RunInTxThenConfirm` unit is never answered here (its prompt comes after it runs). Skip all
//! skips every later unit that would ask. Cancel sends nothing. "Don't ask again" returns the
//! item's `fingerprint` (the class-mixed key) for the caller's `SuppressSet`.
//!
//! **The port** ([`AppConfirmPort`]): the run service asks it from the core runtime; it sends the
//! question over an `mpsc` to a foreground task (the `AppUiPort` pattern), which opens the dialog
//! in the workspace window and sends the answer back on the `oneshot`. A port made for a console
//! ([`AppConfirmPort::for_console`]) fills in the real connection name and production flag from
//! the console's profile at the moment it asks (Task 16's `CommitQuestion` carries
//! `production: true` and an empty name, T16 m13); a console that is gone answers the safe
//! value. A port that is not connected to the UI ([`AppConfirmPort::default`]) answers the safe
//! value at once. [`AppConfirmPort::follow`] gives it the run's `RunTicket::choices`: an open
//! confirm-in-transaction prompt closes once the session decided its question
//! (`RunEvent::CommitDecided` clears it, T1c M7). Nothing here runs a statement or unlocks
//! production (R-M3-52).

use std::sync::{Arc, Mutex};

use hacivat_exec::{
    CommitAnswer, CommitQuestion, ConfirmAnswer, ConfirmPort, ConfirmQuestion, GateClass,
    GateDecision, GateItem, GatePlan, GateReason, apply_answer,
};
use hacivat_l10n::ArgValue;
use hacivat_proto::EnvTag;
use hacivat_session::LiveState;
use hacivat_session::session::{RunEvent, RunId};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::env_swatch;
use hacivat_ui::dialog::DialogSize;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::text::{Label, TextSource, paragraph, scroll_viewport};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Div, Entity, EntityId, FocusHandle,
    Focusable, Global, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, Task, WeakEntity, Window,
    div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use tokio::sync::{mpsc, oneshot, watch};

use super::{answered, in_workspace_window, workspace_window};
use crate::connections::ConnectionsModel;
use crate::connections::capsule::engine_label;
use crate::connections::error_view::env_kind;
use crate::connections::prompt_kit::{
    acting_enter, alert_line, card, cycle_focus, header, plain_input, push_modal, register_part,
    remove_modal, trapped_tab, visible,
};
use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::commit_prompt;
use crate::exec_ui::marks::message;
use crate::exec_ui::orchestrator::profile_of;

/// The confirmation dialog's layer ID.
pub const CONFIRM_LAYER_ID: &str = "confirm-dialog";

/// The SQL inset shows at most this many lines, then scrolls.
const SQL_LINES: f32 = 6.0;
/// The SQL inset draws at most this many characters and lines of a statement (a statement of
/// megabytes still opens at once); the rest is cut with an ellipsis.
const SQL_MAX_CHARS: usize = 20_000;
const SQL_MAX_LINES: usize = 400;
/// At most this many characters of an identifier are drawn (made visible).
const NAME_MAX_CHARS: usize = 200;
/// The stop group's Cancel is at least this wide (NOTES surface 1).
const CANCEL_MIN_PX: f32 = 72.0;

/// The gate's reason IDs the dialog reads (`hacivat-core.ftl`, block `m3b-t15-gate`).
const NOT_ANALYSED: &str = "exec-gate-not-analysed";
const IMPLICIT_COMMIT: &str = "exec-gate-implicit-commit";

// ---- what the dialog shows beyond its question --------------------------------------------------

/// What the dialog shows beyond its [`ConfirmQuestion`]: the statement's class (the Effect row,
/// and the production "don't ask again" rule), the connection's environment and engine, the
/// console's title (the scope line), and, in a run of several, what Execute all and Skip all
/// would answer ([`ask_plan`] fills these from the plan).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfirmContext {
    pub class: Option<GateClass>,
    /// The environment mark; `None` with `production` set in the question draws production.
    pub env: Option<EnvTag>,
    /// The engine and its major version (`PostgreSQL 16`): connection data.
    pub engine: Option<SharedString>,
    /// The console's title for the "don't ask again" scope; the connection's name when `None`.
    pub console: Option<Arc<str>>,
    /// The later units Execute all answers (they would ask without a typed name, RC-1).
    pub covered: u32,
    /// The later units that would ask at all (Skip all skips them).
    pub later_asks: u32,
    /// This unit and every unit Execute all answers only lock rows (Execute all is Default).
    pub all_warn: bool,
}

impl ConfirmContext {
    /// The context a lone question carries (no class known, no later units).
    pub fn for_question(q: &ConfirmQuestion) -> ConfirmContext {
        ConfirmContext {
            env: q.production.then_some(EnvTag::Production),
            ..ConfirmContext::default()
        }
    }

    fn env_of(&self, production: bool) -> Option<EnvTag> {
        self.env
            .or_else(|| production.then_some(EnvTag::Production))
    }
}

/// Whether the dialog offers "Execute and don't ask again" for `q` of class `class` (module
/// docs): never with a typed name, never for a statement Hacivat could not analyse or one that
/// commits an open MySQL transaction, and on production never for `DdlDestructive`, `Admin`,
/// `Unknown` or `Routine`, nor when the class is not known. The question's own flag (the gate's,
/// which already refuses a multi-statement unit with a write) must allow it too.
pub fn offers_suppress(q: &ConfirmQuestion, class: Option<GateClass>) -> bool {
    let unanalysed = q
        .reasons
        .iter()
        .any(|r| r.fluent_id == NOT_ANALYSED || r.fluent_id == IMPLICIT_COMMIT);
    let production_forbids = q.production
        && !matches!(
            class,
            Some(
                GateClass::Read
                    | GateClass::Session
                    | GateClass::Tx
                    | GateClass::Lock
                    | GateClass::Dml
                    | GateClass::DmlUnsafe
                    | GateClass::DdlOther
            )
        );
    q.suppressible && q.type_to_confirm.is_none() && !unanalysed && !production_forbids
}

/// Whether `typed` is the name to type: exactly its bytes (no case or Turkish İ/i folding, no
/// trimming, no normalisation).
pub fn name_matches(typed: &str, name: &str) -> bool {
    typed.as_bytes() == name.as_bytes()
}

/// The Fluent ID of the Effect row for `class`; `None` for the classes that never ask.
fn class_id(class: GateClass) -> Option<&'static str> {
    Some(match class {
        GateClass::Lock => "hv-confirm-class-lock",
        GateClass::Dml => "hv-confirm-class-dml",
        GateClass::DmlUnsafe => "hv-confirm-class-dml-unsafe",
        GateClass::DdlDestructive => "hv-confirm-class-ddl-destructive",
        GateClass::DdlOther => "hv-confirm-class-ddl-other",
        GateClass::Routine => "hv-confirm-class-routine",
        GateClass::Admin => "hv-confirm-class-admin",
        GateClass::Tx => "hv-confirm-class-tx",
        GateClass::Unknown => "hv-confirm-class-unknown",
        GateClass::Read | GateClass::Session => return None,
    })
}

// ---- the plan: R-M3-84 ------------------------------------------------------------------------

/// The question for plan item `item` (unit `pos` of a run of `total`), with its decision's facts
/// handed on unchanged: the typed name (exact bytes), `irreversible`, `suppressible` (a `Warn` is
/// always suppressible, T15). `None` for an item that does not ask (`Run`, `Deny`,
/// `RunInTxThenConfirm`, `ExplainAnalyzeRollback`).
pub fn question_for(
    run: RunId,
    item: &GateItem,
    total: usize,
    sql_masked: Arc<str>,
    production: bool,
    connection: Arc<str>,
) -> Option<ConfirmQuestion> {
    let (reasons, type_to_confirm, irreversible, suppressible, warn_only) = match &item.decision {
        GateDecision::Confirm {
            reasons,
            type_to_confirm,
            irreversible,
            suppressible,
        } => (
            reasons.clone(),
            type_to_confirm.clone(),
            *irreversible,
            *suppressible,
            false,
        ),
        GateDecision::Warn(reasons) => (reasons.clone(), None, false, true, true),
        _ => return None,
    };
    let position = item.unit.saturating_add(1);
    Some(ConfirmQuestion {
        run,
        unit: item.unit,
        index: position,
        total: u32::try_from(total).unwrap_or(u32::MAX),
        sql_masked,
        reasons,
        type_to_confirm,
        irreversible,
        suppressible,
        warn_only,
        production,
        connection,
    })
}

fn asks(item: &GateItem) -> bool {
    matches!(
        item.decision,
        GateDecision::Confirm { .. } | GateDecision::Warn(_)
    )
}

fn needs_name(item: &GateItem) -> bool {
    matches!(
        &item.decision,
        GateDecision::Confirm {
            type_to_confirm: Some(_),
            ..
        }
    )
}

/// The later units of `plan` after the item of unit `unit` that Execute all answers (they ask,
/// without a typed name; RC-1), by unit.
pub fn execute_all_covers(plan: &GatePlan, unit: u32) -> Vec<u32> {
    plan.items
        .iter()
        .filter(|i| i.unit > unit && asks(i) && !needs_name(i))
        .map(|i| i.unit)
        .collect()
}

/// What the dialog for the item of unit `unit` shows about the rest of the run.
fn plan_context(plan: &GatePlan, unit: u32, base: &ConfirmContext) -> ConfirmContext {
    let covered = execute_all_covers(plan, unit);
    let later_asks = plan
        .items
        .iter()
        .filter(|i| i.unit > unit && asks(i))
        .count();
    let this_warn = plan
        .items
        .iter()
        .find(|i| i.unit == unit)
        .is_some_and(|i| matches!(i.decision, GateDecision::Warn(_)));
    let all_warn = this_warn
        && plan
            .items
            .iter()
            .filter(|i| covered.contains(&i.unit))
            .all(|i| matches!(i.decision, GateDecision::Warn(_)));
    ConfirmContext {
        class: plan.items.iter().find(|i| i.unit == unit).map(|i| i.class),
        covered: u32::try_from(covered.len()).unwrap_or(u32::MAX),
        later_asks: u32::try_from(later_asks).unwrap_or(u32::MAX),
        all_warn,
        ..base.clone()
    }
}

/// How a run's confirmations went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanFlow {
    /// Go on; `Some(key)`: the user chose "don't ask again" for the item whose key it is.
    Go(Option<u64>),
    /// Cancel: nothing is sent.
    Cancelled,
}

/// Applies `answer` to the item of unit `unit` in `plan` (R-M3-84) through
/// `hacivat_exec::apply_answer`, which changes only decisions (verdicts, effects, classes and
/// fingerprints stay): Execute all also answers [`execute_all_covers`]; Skip all also skips every
/// later unit that would ask; "don't ask again" returns the item's `fingerprint` only when the
/// dialog offered it (`offered`), else it is a plain Execute.
pub fn answer_plan(
    plan: &mut GatePlan,
    unit: u32,
    answer: ConfirmAnswer,
    offered: bool,
) -> PlanFlow {
    let fingerprint = plan
        .items
        .iter()
        .find(|i| i.unit == unit)
        .map(|i| i.fingerprint);
    match answer {
        ConfirmAnswer::Cancel => return PlanFlow::Cancelled,
        ConfirmAnswer::Execute | ConfirmAnswer::Skip => {
            apply_answer(plan, unit, answer);
        }
        ConfirmAnswer::ExecuteAndSuppress => {
            apply_answer(plan, unit, ConfirmAnswer::Execute);
            return PlanFlow::Go(fingerprint.filter(|_| offered));
        }
        ConfirmAnswer::ExecuteAll => {
            let covered = execute_all_covers(plan, unit);
            apply_answer(plan, unit, ConfirmAnswer::Execute);
            for later in covered {
                apply_answer(plan, later, ConfirmAnswer::ExecuteAll);
            }
        }
        ConfirmAnswer::SkipAll => {
            let later: Vec<u32> = plan
                .items
                .iter()
                .filter(|i| i.unit > unit && asks(i))
                .map(|i| i.unit)
                .collect();
            apply_answer(plan, unit, ConfirmAnswer::Skip);
            for u in later {
                apply_answer(plan, u, ConfirmAnswer::SkipAll);
            }
        }
    }
    PlanFlow::Go(None)
}

/// A run's answered plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanAnswers {
    /// The plan with every answer applied (decisions only; verdicts unchanged).
    pub plan: GatePlan,
    /// The keys (`GateItem::fingerprint`) the user chose "don't ask again" for.
    pub suppress: Vec<u64>,
}

/// Asks every `Confirm`/`Warn` item of `plan` in order (R-M3-84), one dialog at a time, and
/// answers with the plan to send, or `None` when the user cancelled (nothing is sent). `sql`
/// holds each unit's masked text, by item; `base` the connection's facts.
pub fn ask_plan(
    run: RunId,
    plan: GatePlan,
    sql: Vec<Arc<str>>,
    production: bool,
    connection: Arc<str>,
    base: ConfirmContext,
    cx: &mut App,
) -> Task<Option<PlanAnswers>> {
    cx.spawn(async move |cx| {
        let mut plan = plan;
        let mut suppress = Vec::new();
        let total = plan.items.len();
        let mut at = 0;
        while let Some(pos) = (at..plan.items.len()).find(|&p| asks(&plan.items[p])) {
            at = pos + 1;
            let item = plan.items[pos].clone();
            let text = sql.get(pos).cloned().unwrap_or_else(|| Arc::from(""));
            let Some(q) = question_for(run, &item, total, text, production, connection.clone())
            else {
                continue;
            };
            let ctx = plan_context(&plan, item.unit, &base);
            let offered = offers_suppress(&q, ctx.class);
            let rx = cx.update(|cx| open_in(q, ctx, cx));
            let answer = rx.await.unwrap_or(ConfirmAnswer::Cancel);
            match answer_plan(&mut plan, item.unit, answer, offered) {
                PlanFlow::Cancelled => return None,
                PlanFlow::Go(key) => suppress.extend(key),
            }
        }
        Some(PlanAnswers { plan, suppress })
    })
}

// ---- open prompts -------------------------------------------------------------------------------

/// The open confirmation and commit prompts (the quit guard answers them; the port follows
/// them).
#[derive(Default)]
pub(crate) struct OpenPrompts {
    pub(crate) confirms: Vec<WeakEntity<ConfirmDialog>>,
    pub(crate) commits: Vec<(Option<ConsoleId>, WeakEntity<commit_prompt::CommitPrompt>)>,
}

impl Global for OpenPrompts {}

/// Answers every open confirmation with Cancel and every open commit prompt with Roll back
/// (Undo), and closes them: a quit or a window close never runs or keeps anything (D-014). The
/// quit guard calls it first (named exception in `close_prompt.rs`).
pub fn answer_open_prompts_safely(cx: &mut App) {
    let Some(open) = cx.try_global::<OpenPrompts>() else {
        return;
    };
    let confirms: Vec<_> = open
        .confirms
        .iter()
        .filter_map(WeakEntity::upgrade)
        .collect();
    let commits: Vec<_> = open
        .commits
        .iter()
        .filter_map(|(_, p)| p.upgrade())
        .collect();
    let mut views = Vec::new();
    for d in confirms {
        views.push(d.entity_id());
        d.update(cx, |d, _| d.finish(ConfirmAnswer::Cancel));
    }
    for p in commits {
        views.push(p.entity_id());
        p.update(cx, |p, _| p.finish(CommitAnswer::RollBack));
    }
    if views.is_empty() {
        return;
    }
    in_workspace_window(cx, move |ws, window, cx| {
        for view in views {
            ws.update(cx, |ws, cx| ws.remove_layer(view, window, cx));
        }
    });
}

/// Whether a confirmation dialog is open (tests).
pub fn confirm_open(cx: &App) -> bool {
    cx.try_global::<OpenPrompts>()
        .is_some_and(|o| o.confirms.iter().any(|d| d.upgrade().is_some()))
}

fn track_confirm(view: WeakEntity<ConfirmDialog>, cx: &mut App) {
    let open = cx.default_global::<OpenPrompts>();
    open.confirms.retain(|d| d.upgrade().is_some());
    open.confirms.push(view);
}

// ---- opening ------------------------------------------------------------------------------------

/// Asks `question` (its lone context, [`ConfirmContext::for_question`]); any close (Escape,
/// window close, ⌘W, quit) answers `Cancel`. Without a workspace window it answers `Cancel`.
pub fn open(question: ConfirmQuestion, cx: &mut App) -> oneshot::Receiver<ConfirmAnswer> {
    let ctx = ConfirmContext::for_question(&question);
    open_in(question, ctx, cx)
}

/// Asks `question` with `ctx` (module docs). The dialog opens in the workspace window after this
/// update; without one the answer is `Cancel` at once.
pub fn open_in(
    question: ConfirmQuestion,
    ctx: ConfirmContext,
    cx: &mut App,
) -> oneshot::Receiver<ConfirmAnswer> {
    if workspace_window(cx).is_none() {
        return answered(ConfirmAnswer::Cancel);
    }
    let (reply, rx) = oneshot::channel();
    cx.defer(move |cx| {
        let Some(handle) = workspace_window(cx) else {
            // Dropping the reply answers Cancel (`unwrap_or`), but say it.
            let _ = reply.send(ConfirmAnswer::Cancel);
            return;
        };
        let mut reply = Some(reply);
        let opened = handle.update(cx, |_, window, cx| {
            if let Some(reply) = reply.take() {
                show(question, ctx, reply, window, cx);
            }
        });
        if opened.is_err()
            && let Some(reply) = reply.take()
        {
            let _ = reply.send(ConfirmAnswer::Cancel);
        }
    });
    rx
}

/// Pushes the dialog over `window`'s workspace.
fn show(
    question: ConfirmQuestion,
    ctx: ConfirmContext,
    reply: oneshot::Sender<ConfirmAnswer>,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| ConfirmDialog::new(question, ctx, reply, window, cx));
    track_confirm(view.downgrade(), cx);
    let cancel = view.read(cx).cancel.clone();
    if !push_modal(CONFIRM_LAYER_ID, view.clone().into(), &cancel, window, cx) {
        view.update(cx, |d, _| d.finish(ConfirmAnswer::Cancel));
    }
    let parts = view.read(cx).parts();
    for (name, handle) in parts {
        register_part(cx, name, &handle, None);
    }
    if let Some(input) = view.read(cx).typed.clone() {
        let handle = input.read(cx).focus_handle(cx);
        register_part(cx, "confirm-type", &handle, Some(&input));
    }
}

/// One footer button as the dialog draws it.
#[derive(Debug, Clone)]
struct ButtonSpec {
    id: &'static str,
    label_id: &'static str,
    kind: ButtonKind,
    enabled: bool,
    handle: FocusHandle,
    answer: ConfirmAnswer,
}

/// One button of the open confirmation dialog, as drawn now (tests): its element ID, its kind,
/// whether it is enabled, and its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawnButton {
    pub id: &'static str,
    pub kind: ButtonKind,
    pub enabled: bool,
    pub answer: ConfirmAnswer,
}

/// The buttons of the topmost open confirmation dialog, in visual order (tests).
pub fn open_dialog_buttons(cx: &App) -> Vec<DrawnButton> {
    let Some(d) = cx
        .try_global::<OpenPrompts>()
        .and_then(|o| o.confirms.iter().rev().find_map(WeakEntity::upgrade))
    else {
        return Vec::new();
    };
    d.read(cx)
        .buttons(cx)
        .into_iter()
        .map(|b| DrawnButton {
            id: b.id,
            kind: b.kind,
            enabled: b.enabled,
            answer: b.answer,
        })
        .collect()
}

// ---- the dialog ---------------------------------------------------------------------------------

/// The confirmation dialog (module docs).
pub struct ConfirmDialog {
    q: ConfirmQuestion,
    ctx: ConfirmContext,
    reply: Option<oneshot::Sender<ConfirmAnswer>>,
    /// The type-to-confirm field, when the question asks for a typed name.
    typed: Option<Entity<TextInput>>,
    /// Enter was pressed in the field over a name that does not match: the hint shows.
    missed: bool,
    root: FocusHandle,
    copy: FocusHandle,
    execute: FocusHandle,
    execute_all: FocusHandle,
    suppress: FocusHandle,
    skip: FocusHandle,
    skip_all: FocusHandle,
    cancel: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for ConfirmDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl Drop for ConfirmDialog {
    /// A dialog that goes without an answer (its window closed, the app quit) answers Cancel.
    fn drop(&mut self) {
        self.finish(ConfirmAnswer::Cancel);
    }
}

impl ConfirmDialog {
    fn new(
        q: ConfirmQuestion,
        ctx: ConfirmContext,
        reply: oneshot::Sender<ConfirmAnswer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let typed = q.type_to_confirm.as_ref().map(|_| plain_input(cx, true));
        if let Some(input) = &typed {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    // Enter in the field never executes (M2b vault-reset precedent).
                    InputEvent::Submitted => {
                        this.missed = !this.name_ok(cx);
                        cx.notify();
                    }
                    InputEvent::Cancelled => this.answer(ConfirmAnswer::Cancel, window, cx),
                    InputEvent::Changed => {
                        this.missed = false;
                        cx.notify();
                    }
                    InputEvent::CompositionChanged(_) => {}
                },
            ));
        }
        ConfirmDialog {
            q,
            ctx,
            reply: Some(reply),
            typed,
            missed: false,
            root: cx.focus_handle(),
            copy: cx.focus_handle().tab_stop(true),
            execute: cx.focus_handle().tab_stop(true),
            execute_all: cx.focus_handle().tab_stop(true),
            suppress: cx.focus_handle().tab_stop(true),
            skip: cx.focus_handle().tab_stop(true),
            skip_all: cx.focus_handle().tab_stop(true),
            cancel: cx.focus_handle().tab_stop(true),
            _subscriptions: subscriptions,
        }
    }

    /// The named focus handles (tests).
    fn parts(&self) -> Vec<(&'static str, FocusHandle)> {
        vec![
            ("confirm-copy", self.copy.clone()),
            ("confirm-execute", self.execute.clone()),
            ("confirm-execute-all", self.execute_all.clone()),
            ("confirm-suppress", self.suppress.clone()),
            ("confirm-skip", self.skip.clone()),
            ("confirm-skip-all", self.skip_all.clone()),
            ("confirm-cancel", self.cancel.clone()),
        ]
    }

    /// Sends `answer` once; later answers (and the drop) send nothing.
    pub(crate) fn finish(&mut self, answer: ConfirmAnswer) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(answer);
        }
    }

    /// The typed field's text.
    fn typed_text<'a>(&self, cx: &'a App) -> &'a str {
        self.typed.as_ref().map_or("", |i| i.read(cx).value())
    }

    /// Whether the name is typed (or none is asked for).
    fn name_ok(&self, cx: &App) -> bool {
        match &self.q.type_to_confirm {
            Some(name) => name_matches(self.typed_text(cx), name),
            None => true,
        }
    }

    /// The hint shows once the field cannot become the name by typing on (or Enter missed).
    fn mismatch(&self, cx: &App) -> bool {
        let Some(name) = &self.q.type_to_confirm else {
            return false;
        };
        let typed = self.typed_text(cx);
        self.missed || (!typed.is_empty() && !name.as_bytes().starts_with(typed.as_bytes()))
    }

    fn multi(&self) -> bool {
        self.q.total > 1
    }

    fn shows_execute_all(&self) -> bool {
        self.ctx.covered > 0
    }

    fn shows_skip_all(&self) -> bool {
        self.ctx.later_asks > 0
    }

    fn shows_suppress(&self) -> bool {
        offers_suppress(&self.q, self.ctx.class)
    }

    /// Whether `answer` may be given now (a disabled or absent button never answers).
    fn allowed(&self, answer: ConfirmAnswer, cx: &App) -> bool {
        match answer {
            ConfirmAnswer::Cancel => true,
            ConfirmAnswer::Execute => self.name_ok(cx),
            ConfirmAnswer::ExecuteAll => self.shows_execute_all() && self.name_ok(cx),
            ConfirmAnswer::ExecuteAndSuppress => self.shows_suppress() && self.name_ok(cx),
            ConfirmAnswer::Skip => self.multi(),
            ConfirmAnswer::SkipAll => self.shows_skip_all(),
        }
    }

    /// Answers `answer` (when allowed) and closes.
    fn answer(&mut self, answer: ConfirmAnswer, window: &mut Window, cx: &mut Context<Self>) {
        if self.reply.is_none() || !self.allowed(answer, cx) {
            return;
        }
        self.finish(answer);
        if let Some(input) = &self.typed {
            input.update(cx, |i, cx| {
                i.set_value("");
                cx.notify();
            });
        }
        let view = cx.entity_id();
        window.defer(cx, move |window, cx| remove_modal(view, window, cx));
    }

    /// The buttons in visual order, as drawn now (module docs: Danger for every answer that
    /// sends, except `Warn`'s; Cancel Primary).
    fn buttons(&self, cx: &App) -> Vec<ButtonSpec> {
        let ok = self.name_ok(cx);
        let run_kind = if self.q.warn_only {
            ButtonKind::Default
        } else {
            ButtonKind::Danger
        };
        let spec = |id, label_id, kind, enabled, handle: &FocusHandle, answer| ButtonSpec {
            id,
            label_id,
            kind,
            enabled,
            handle: handle.clone(),
            answer,
        };
        let mut out = vec![spec(
            "confirm-execute",
            "hv-confirm-execute",
            run_kind,
            ok,
            &self.execute,
            ConfirmAnswer::Execute,
        )];
        if self.shows_execute_all() {
            let kind = if self.ctx.all_warn {
                ButtonKind::Default
            } else {
                ButtonKind::Danger
            };
            out.push(spec(
                "confirm-execute-all",
                "hv-confirm-execute-all",
                kind,
                ok,
                &self.execute_all,
                ConfirmAnswer::ExecuteAll,
            ));
        }
        if self.shows_suppress() {
            let label = if self.q.production {
                "hv-confirm-execute-suppress-prod"
            } else {
                "hv-confirm-execute-suppress"
            };
            out.push(spec(
                "confirm-suppress",
                label,
                run_kind,
                ok,
                &self.suppress,
                ConfirmAnswer::ExecuteAndSuppress,
            ));
        }
        if self.multi() {
            out.push(spec(
                "confirm-skip",
                "hv-confirm-skip",
                ButtonKind::Default,
                true,
                &self.skip,
                ConfirmAnswer::Skip,
            ));
        }
        if self.shows_skip_all() {
            out.push(spec(
                "confirm-skip-all",
                "hv-confirm-skip-all",
                ButtonKind::Default,
                true,
                &self.skip_all,
                ConfirmAnswer::SkipAll,
            ));
        }
        out.push(spec(
            "confirm-cancel",
            "hv-confirm-cancel",
            ButtonKind::Primary,
            true,
            &self.cancel,
            ConfirmAnswer::Cancel,
        ));
        out
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        match stroke.key.as_str() {
            "escape" => self.answer(ConfirmAnswer::Cancel, window, cx),
            "enter" => {
                if !acting_enter(event) {
                    return cx.stop_propagation();
                }
                // Enter acts on the focused button only; in the field it does nothing.
                let focused = self
                    .buttons(cx)
                    .into_iter()
                    .find(|b| b.enabled && b.handle.is_focused(window));
                if let Some(b) = focused {
                    self.answer(b.answer, window, cx);
                }
            }
            // ⌘W (Ctrl+W elsewhere) closes the dialog: Cancel.
            "w" if closes_dialog(stroke, cx) => self.answer(ConfirmAnswer::Cancel, window, cx),
            _ if trapped_tab(event) => {
                let mut order = Vec::new();
                if let Some(input) = &self.typed {
                    order.push(input.read(cx).focus_handle(cx));
                    order.push(self.copy.clone());
                }
                order.extend(
                    self.buttons(cx)
                        .into_iter()
                        .filter(|b| b.enabled)
                        .map(|b| b.handle),
                );
                cycle_focus(&order, stroke.modifiers.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn copy_name(&mut self, cx: &mut Context<Self>) {
        if let Some(name) = &self.q.type_to_confirm {
            cx.write_to_clipboard(ClipboardItem::new_string(name.to_string()));
        }
    }
}

/// Whether `stroke` is the close-window chord (⌘W on macOS, Ctrl+W elsewhere), unshifted.
pub(crate) fn closes_dialog(stroke: &hacivat_ui_core::gpui_surface::Keystroke, cx: &App) -> bool {
    let m = &stroke.modifiers;
    stroke.key == "w" && m.secondary(cx) && !m.shift && !m.alt
}

impl Render for ConfirmDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let q = &self.q;
        let warn = q.warn_only;
        let name: Arc<str> = Arc::from(visible(&q.connection, NAME_MAX_CHARS));
        let title_id = if warn {
            "hv-confirm-title-warn"
        } else {
            "hv-confirm-title"
        };
        let title = message(cx, title_id, vec![("name", text_arg(&name))]);
        let unit = self.multi().then(|| {
            message(
                cx,
                "hv-confirm-sub-unit",
                vec![
                    ("i", ArgValue::Count(u64::from(q.index))),
                    ("n", ArgValue::Count(u64::from(q.total))),
                ],
            )
        });
        let sub = subtitle(
            &theme,
            self.ctx.env_of(q.production),
            &name,
            self.ctx.engine.clone(),
            unit,
        );

        // The body: the statement, why it asks, what it does, the typed name, the scope lines.
        let mut body = div().w_full().flex().flex_col().gap(px(t.spacing.lg));
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap(px(t.spacing.xs))
                .child(caption(
                    &theme,
                    message(cx, "hv-confirm-sql", vec![]),
                    "confirm-sql-caption",
                ))
                .child(sql_inset(&theme, &q.sql_masked, "confirm-sql")),
        );
        let reasons: Vec<SharedString> = q.reasons.iter().map(|r| reason_text(cx, r)).collect();
        let description: SharedString = reasons
            .iter()
            .map(SharedString::as_ref)
            .collect::<Vec<_>>()
            .join(" ")
            .into();
        if !reasons.is_empty() {
            body = body.child(reasons_list(&theme, reasons));
        }
        if let Some(id) = self.ctx.class.and_then(class_id) {
            body = body.child(effect_row(
                &theme,
                message(cx, "hv-confirm-effect", vec![]),
                message(cx, id, vec![]),
            ));
        }
        if q.irreversible {
            body = body.child(info_line(
                &theme,
                message(cx, "hv-confirm-mysql-ddl", vec![]),
            ));
        }
        if let (Some(raw), Some(input)) = (&q.type_to_confirm, &self.typed) {
            let shown = visible(raw, NAME_MAX_CHARS);
            let label = message(
                cx,
                "hv-confirm-type-label",
                vec![("name", ArgValue::Text(shown))],
            );
            let copy = Button::new("confirm-copy", message(cx, "hv-confirm-copy-name", vec![]))
                .kind(ButtonKind::Icon)
                .icon(IconName::Copy)
                .compact(true)
                .focus_handle(self.copy.clone())
                .on_click(cx.listener(|this, _, _, cx| this.copy_name(cx)));
            let mut block = div()
                .flex()
                .flex_col()
                .gap(px(t.spacing.xs))
                .child(type_line(&theme, label, copy))
                .child(
                    div()
                        .debug_selector(|| "confirm-type".to_string())
                        .w_full()
                        .child(input.clone()),
                );
            if self.mismatch(cx) {
                block = block.child(alert_line(
                    &theme,
                    message(cx, "hv-confirm-type-hint", vec![]),
                    "confirm-type-hint",
                ));
            }
            body = body.child(block);
        }
        if self.shows_execute_all() {
            body = body.child(note(
                &theme,
                message(
                    cx,
                    "hv-confirm-execute-all-scope",
                    vec![("n", ArgValue::Count(u64::from(self.ctx.covered)))],
                ),
                "confirm-execute-all-scope",
            ));
        }
        let suppress = self.shows_suppress();
        if suppress {
            let console = self
                .ctx
                .console
                .as_deref()
                .map_or_else(|| name.to_string(), |c| visible(c, NAME_MAX_CHARS));
            body = body.child(note(
                &theme,
                message(
                    cx,
                    "hv-confirm-suppress-scope",
                    vec![("console", ArgValue::Text(console))],
                ),
                "confirm-suppress-scope",
            ));
        }
        let calm = !q.production
            && !q
                .reasons
                .iter()
                .any(|r| r.fluent_id == NOT_ANALYSED || r.fluent_id == IMPLICIT_COMMIT);
        let last = message(
            cx,
            if calm {
                "hv-confirm-calm-line"
            } else {
                "hv-confirm-prod-line"
            },
            vec![],
        );
        body = body.child(
            paragraph(
                last,
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
            )
            .id("confirm-last-line"),
        );

        // The footer: the run group, 28 px, the stop group (RC-3; it wraps under, right-aligned).
        let mut run_group: Vec<AnyElement> = Vec::new();
        let mut stop_group: Vec<AnyElement> = Vec::new();
        for spec in self.buttons(cx) {
            let answer = spec.answer;
            let button = Button::new(spec.id, message(cx, spec.label_id, vec![]))
                .kind(spec.kind)
                .disabled(!spec.enabled)
                .on_raised(true)
                .focus_handle(spec.handle.clone())
                .on_click(cx.listener(move |this, _, window, cx| this.answer(answer, window, cx)));
            match answer {
                ConfirmAnswer::Execute
                | ConfirmAnswer::ExecuteAll
                | ConfirmAnswer::ExecuteAndSuppress => run_group.push(button.into_any_element()),
                ConfirmAnswer::Skip | ConfirmAnswer::SkipAll => {
                    stop_group.push(button.into_any_element());
                }
                ConfirmAnswer::Cancel => stop_group.push(
                    div()
                        .min_w(px(CANCEL_MIN_PX))
                        .flex()
                        .flex_row()
                        .justify_end()
                        .child(button)
                        .into_any_element(),
                ),
            }
        }

        let card = card(&theme, DialogSize::Wide, CONFIRM_LAYER_ID)
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .child(header(
                &theme,
                title.clone(),
                "confirm-title",
                Some(sub),
                cx.listener(|this, _, window, cx| this.answer(ConfirmAnswer::Cancel, window, cx)),
                cx,
            ))
            .child(scroll_body(body))
            .child(grouped_footer(&theme, run_group, stop_group));
        A11yNode::new("confirm-node", Role::AlertDialog, title, true, card).description(description)
    }
}

// ---- shared parts (the commit prompt draws them too) -------------------------------------------

/// A message argument naming data (already made visible).
pub(crate) fn text_arg(s: &str) -> ArgValue {
    ArgValue::Text(s.to_string())
}

/// The subtitle (NOTES surface 1): the 7 px environment mark, the connection in Fragment Mono,
/// the engine (data), and in a run of several "statement k of n"; text-2 throughout (Raised
/// Ink), wrapping between parts.
pub(crate) fn subtitle(
    theme: &Theme,
    env: Option<EnvTag>,
    connection: &str,
    engine: Option<SharedString>,
    unit: Option<SharedString>,
) -> Div {
    let t = theme.tokens;
    let ink = t.colors.text_2.to_hsla();
    let mut row = div()
        .debug_selector(|| "confirm-subtitle".to_string())
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(t.spacing.sm))
        .children(env.map(|e| env_swatch(env_kind(e), theme)));
    if !connection.is_empty() {
        row = row.child(
            Label::new(
                connection.to_string(),
                TextSource::Data,
                t.type_ramp.mono_data,
                t.colors.text.to_hsla(),
            )
            .id("confirm-connection"),
        );
    }
    if let Some(engine) = engine {
        row = row.child(
            Label::new(engine, TextSource::Data, t.type_ramp.body_sm, ink).id("confirm-engine"),
        );
    }
    if let Some(unit) = unit {
        row = row.child(
            Label::new(unit, TextSource::Fluent, t.type_ramp.body_sm, ink).id("confirm-unit"),
        );
    }
    row
}

/// A 12 px text-2 caption, recorded as `id`.
pub(crate) fn caption(theme: &Theme, text: SharedString, id: &'static str) -> Label {
    let t = theme.tokens;
    Label::new(
        text,
        TextSource::Fluent,
        t.type_ramp.caption,
        t.colors.text_2.to_hsla(),
    )
    .id(id)
}

/// A 12.5 px text-2 note that wraps, recorded as `id`.
fn note(theme: &Theme, text: SharedString, id: &'static str) -> impl IntoElement {
    let t = theme.tokens;
    // A full-width box, so the paragraph is measured at the width it wraps to. 12.5 px, not the
    // 12 px caption: the caption's 16.2 px line is laid out at 16 px, so a note of three lines
    // or more would be cut by 0.6 px (the frame checks see it).
    div().w_full().min_w_0().child(
        paragraph(
            text,
            TextSource::Fluent,
            t.type_ramp.body_sm,
            t.colors.text_2.to_hsla(),
        )
        .id(id),
    )
}

/// `sql` as the inset draws it: bidi and invisible characters and controls made visible (a
/// backslash stays single: this is SQL, not server text), tabs as four spaces, at most
/// [`SQL_MAX_LINES`] lines and [`SQL_MAX_CHARS`] characters.
pub(crate) fn shown_sql(sql: &str) -> Vec<String> {
    let flat = sql.replace("\r\n", "\n").replace('\t', "    ");
    let mut chars = 0usize;
    let mut out = Vec::new();
    for line in flat.split('\n') {
        if out.len() >= SQL_MAX_LINES || chars >= SQL_MAX_CHARS {
            out.push("…".to_string());
            break;
        }
        let room = SQL_MAX_CHARS - chars;
        let shown = line
            .split('\\')
            .map(|piece| visible(piece, SQL_MAX_CHARS))
            .collect::<Vec<_>>()
            .join("\\");
        let cut: String = shown.chars().take(room).collect();
        chars += cut.chars().count().max(1);
        let cut_short = cut.len() < shown.len();
        out.push(if cut_short { format!("{cut}…") } else { cut });
    }
    out
}

/// The SQL inset (NOTES "New primitives"): panel (dark) / ground (light), 1 px seam-strong,
/// radius 4, padding 7/10, Fragment Mono; at most 6 lines, then it scrolls. Lines wrap.
pub(crate) fn sql_inset(theme: &Theme, sql: &str, id: &'static str) -> impl IntoElement {
    let t = theme.tokens;
    let c = &t.colors;
    let fill = match theme.mode {
        ThemeMode::Dark => c.panel,
        ThemeMode::Light => c.ground,
    };
    let style = t.type_ramp.mono_data;
    let pad_y = 7.0;
    let max_h = SQL_LINES * style.line_height + 2.0 * pad_y + 2.0;
    let lines = shown_sql(sql).into_iter().map(move |line| {
        let line = if line.is_empty() {
            " ".to_string()
        } else {
            line
        };
        paragraph(line, TextSource::Data, style, c.text.to_hsla()).id("confirm-sql-line")
    });
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .flex()
        .flex_col()
        .max_h(px(max_h))
        .overflow_y_scroll()
        .py(px(pad_y))
        .px(px(t.spacing.lg))
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .rounded(px(t.radii.sm))
        .bg(fill.to_hsla())
        // A line partly scrolled out of the inset is not clipped text (the frame checks).
        .child(scroll_viewport())
        .children(lines)
}

/// The text of a gate reason (a `hacivat-core.ftl` message), its identifiers made visible
/// (Fluent isolates them with FSI/PDI).
fn reason_text(cx: &App, r: &GateReason) -> SharedString {
    let args = r
        .args
        .iter()
        .map(|(k, v)| (*k, ArgValue::Text(visible(v, NAME_MAX_CHARS))))
        .collect();
    message(cx, r.fluent_id, args)
}

/// The reasons list: 13 px text, a 3 px square text-2 bullet 7 px before each.
fn reasons_list(theme: &Theme, reasons: Vec<SharedString>) -> impl IntoElement {
    let t = theme.tokens;
    let c = &t.colors;
    let style = t.type_ramp.body;
    let bullet_top = (style.line_height - 3.0) / 2.0;
    div()
        .debug_selector(|| "confirm-reasons".to_string())
        .flex()
        .flex_col()
        .gap(px(t.spacing.xs))
        .children(reasons.into_iter().map(move |text| {
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(7.0))
                .child(
                    div()
                        .flex_none()
                        .mt(px(bullet_top))
                        .size(px(3.0))
                        .bg(c.text_2.to_hsla()),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        paragraph(text, TextSource::Fluent, style, c.text.to_hsla())
                            .id("confirm-reason"),
                    ),
                )
        }))
}

/// The Effect row: "Effect" in text-2, then the class in words.
fn effect_row(theme: &Theme, label: SharedString, class: SharedString) -> impl IntoElement {
    let t = theme.tokens;
    let c = &t.colors;
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(t.spacing.md))
        .child(
            div().flex_none().child(
                Label::new(
                    label,
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )
                .id("confirm-effect-label"),
            ),
        )
        .child(
            div().flex_1().min_w_0().child(
                paragraph(
                    class,
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text.to_hsla(),
                )
                .id("confirm-effect"),
            ),
        )
}

/// An inline notice: the info glyph in text-2, then the text.
fn info_line(theme: &Theme, text: SharedString) -> impl IntoElement {
    let t = theme.tokens;
    let c = &t.colors;
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(t.spacing.sm))
        .child(
            div().flex_none().pt(px(2.0)).child(
                Icon::new(IconName::Info)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            ),
        )
        .child(
            div().flex_1().min_w_0().child(
                paragraph(
                    text,
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text.to_hsla(),
                )
                .id("confirm-irreversible"),
            ),
        )
}

/// "Type `name` to run it" with the name in Fragment Mono and text ink, split out of the
/// message on its isolation marks (the parts around it stay text-2 and wrap), then the Copy
/// button. A message without the marks is drawn whole.
fn type_line(theme: &Theme, message: SharedString, copy: Button) -> impl IntoElement {
    let t = theme.tokens;
    let c = &t.colors;
    let line = div()
        .debug_selector(|| "confirm-type-line".to_string())
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center();
    let Some((before, name, after)) = split_isolated(&message) else {
        return line
            .child(
                paragraph(
                    message,
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text.to_hsla(),
                )
                .id("confirm-type-label"),
            )
            .child(copy);
    };
    let plain = |text: String| {
        Label::new(
            text,
            TextSource::Fluent,
            t.type_ramp.body_sm,
            c.text.to_hsla(),
        )
    };
    line.children((!before.is_empty()).then(|| plain(before)))
        .child(
            Label::new(
                name,
                TextSource::Data,
                t.type_ramp.mono_data,
                c.text.to_hsla(),
            )
            .id("confirm-type-name"),
        )
        .children((!after.is_empty()).then(|| plain(after)))
        .child(div().flex_none().ml(px(t.spacing.sm)).child(copy))
}

/// Splits a rendered message around its first isolated placeable (U+2068 … U+2069).
fn split_isolated(s: &str) -> Option<(String, String, String)> {
    let open = s.find('\u{2068}')?;
    let rest = &s[open + '\u{2068}'.len_utf8()..];
    let close = rest.find('\u{2069}')?;
    Some((
        s[..open].to_string(),
        rest[..close].to_string(),
        rest[close + '\u{2069}'.len_utf8()..].to_string(),
    ))
}

/// The body between the header and the footer: it scrolls when the window is short.
pub(crate) fn scroll_body(body: Div) -> impl IntoElement {
    div()
        .id("confirm-body")
        .debug_selector(|| "confirm-body".to_string())
        .flex()
        .flex_col()
        .flex_shrink(1.0)
        .min_h_0()
        .overflow_y_scroll()
        .child(scroll_viewport())
        .child(body)
}

/// The grouped footer (NOTES "New primitives"): the run group on the left, the stop group on
/// the right, 8 px inside a group and 28 px between them (`danger_isolation`, RC-3); when they
/// do not fit one line the stop group wraps under the run group, still right-aligned. Buttons
/// never truncate.
pub(crate) fn grouped_footer(
    theme: &Theme,
    run: Vec<AnyElement>,
    stop: Vec<AnyElement>,
) -> impl IntoElement {
    let t = theme.tokens;
    let group = || {
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(t.spacing.md))
    };
    div()
        .debug_selector(|| "confirm-footer".to_string())
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_x(px(t.spacing.danger_isolation))
        .gap_y(px(t.spacing.md))
        .pt(px(t.spacing.xs))
        .child(group().children(run))
        .child(group().ml_auto().justify_end().children(stop))
}

// ---- the port -----------------------------------------------------------------------------------

/// What the port knows of the run it asks for ([`AppConfirmPort::follow`]).
#[derive(Debug, Clone)]
pub(crate) struct Follow {
    /// The run's `RunTicket::choices`.
    pub(crate) choices: watch::Receiver<Option<(u64, RunEvent)>>,
    /// Each unit's masked text, by ordinal (the commit prompt shows it).
    pub(crate) sql: Arc<[Arc<str>]>,
}

/// One question on its way to the UI thread.
enum PortRequest {
    Confirm {
        console: Option<ConsoleId>,
        question: ConfirmQuestion,
        reply: oneshot::Sender<ConfirmAnswer>,
    },
    Commit {
        console: Option<ConsoleId>,
        question: CommitQuestion,
        reply: oneshot::Sender<CommitAnswer>,
        follow: Option<Follow>,
    },
    Lost {
        console: Option<ConsoleId>,
        statements_before: Option<u32>,
    },
}

/// The foreground end of every port (installed once, on the first port made for a console).
struct PortLoop(mpsc::UnboundedSender<PortRequest>);

impl Global for PortLoop {}

/// The app's `ConfirmPort` (module docs).
#[derive(Debug, Clone, Default)]
pub struct AppConfirmPort {
    console: Option<ConsoleId>,
    requests: Option<mpsc::UnboundedSender<PortRequest>>,
    follow: Arc<Mutex<Option<Follow>>>,
}

impl std::fmt::Debug for PortRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PortRequest::Confirm { .. } => "Confirm",
            PortRequest::Commit { .. } => "Commit",
            PortRequest::Lost { .. } => "Lost",
        })
    }
}

impl AppConfirmPort {
    /// The port of console `console`: its questions open in the workspace window with the
    /// console's connection name, environment and engine read when it asks.
    pub fn for_console(console: ConsoleId, cx: &mut App) -> Arc<AppConfirmPort> {
        let requests = install(cx);
        Arc::new(AppConfirmPort {
            console: Some(console),
            requests: Some(requests),
            follow: Arc::default(),
        })
    }

    /// A port with no console: its questions open as they are asked (the connection name the
    /// question carries).
    pub fn unbound(cx: &mut App) -> Arc<AppConfirmPort> {
        let requests = install(cx);
        Arc::new(AppConfirmPort {
            console: None,
            requests: Some(requests),
            follow: Arc::default(),
        })
    }

    /// Follows the run whose ticket gave `choices` (`RunTicket::choices`): an open
    /// confirm-in-transaction prompt closes once its question is no longer the one waiting
    /// (the session decided it: `CommitDecided`, the unit ended, the run ended). `sql` holds
    /// each unit's masked text by ordinal, for the prompt.
    pub fn follow(&self, choices: watch::Receiver<Option<(u64, RunEvent)>>, sql: Arc<[Arc<str>]>) {
        if let Ok(mut f) = self.follow.lock() {
            *f = Some(Follow { choices, sql });
        }
    }

    /// The console's connection was lost while a confirm-in-transaction prompt waited: the
    /// prompt says the server rolled the changes back (in the savepoint form, the user's whole
    /// transaction: this statement and `statements_before` before it), with Close only.
    pub fn connection_lost(&self, statements_before: Option<u32>) {
        if let Some(requests) = &self.requests {
            let _ = requests.send(PortRequest::Lost {
                console: self.console,
                statements_before,
            });
        }
    }

    fn current_follow(&self) -> Option<Follow> {
        self.follow.lock().ok().and_then(|f| f.clone())
    }
}

impl ConfirmPort for AppConfirmPort {
    fn confirm(&self, q: ConfirmQuestion) -> oneshot::Receiver<ConfirmAnswer> {
        let Some(requests) = &self.requests else {
            return answered(ConfirmAnswer::Cancel);
        };
        let (reply, rx) = oneshot::channel();
        let sent = requests.send(PortRequest::Confirm {
            console: self.console,
            question: q,
            reply,
        });
        match sent {
            Ok(()) => rx,
            // The UI is gone (quitting): nobody can answer; nothing runs.
            Err(_) => answered(ConfirmAnswer::Cancel),
        }
    }

    fn commit_decision(&self, q: CommitQuestion) -> oneshot::Receiver<CommitAnswer> {
        let Some(requests) = &self.requests else {
            return answered(CommitAnswer::RollBack);
        };
        let (reply, rx) = oneshot::channel();
        let sent = requests.send(PortRequest::Commit {
            console: self.console,
            question: q,
            reply,
            follow: self.current_follow(),
        });
        match sent {
            Ok(()) => rx,
            Err(_) => answered(CommitAnswer::RollBack),
        }
    }
}

/// Installs the port's foreground task once and returns its sender.
fn install(cx: &mut App) -> mpsc::UnboundedSender<PortRequest> {
    if let Some(port) = cx.try_global::<PortLoop>() {
        return port.0.clone();
    }
    let (requests, mut incoming) = mpsc::unbounded_channel::<PortRequest>();
    cx.set_global(PortLoop(requests.clone()));
    cx.spawn(async move |cx| {
        while let Some(request) = incoming.recv().await {
            cx.update(|cx| route(request, cx));
        }
    })
    .detach();
    requests
}

/// What a console's profile says when the port asks: its name, environment and engine.
pub(crate) struct Facts {
    pub(crate) name: Arc<str>,
    pub(crate) env: Option<EnvTag>,
    pub(crate) engine: Option<SharedString>,
}

/// Console `console`'s facts now; `None` when the console (or its profile) is gone.
pub(crate) fn console_facts(cx: &App, console: ConsoleId) -> Option<Facts> {
    Consoles::get(cx, console)?;
    let profile = profile_of(cx, console)?;
    let info = cx
        .try_global::<ConnectionsModel>()
        .map(|m| m.live(profile.id));
    let engine = match &info {
        Some(LiveState::Connected(info)) => engine_label(&profile, Some(info)),
        _ => engine_label(&profile, None),
    };
    Some(Facts {
        name: Arc::from(profile.name.as_str()),
        env: profile.environment,
        engine: Some(engine),
    })
}

/// Opens the prompt of `request` and sends its answer back (the asker going away closes it).
fn route(request: PortRequest, cx: &mut App) {
    match request {
        PortRequest::Confirm {
            console,
            mut question,
            reply,
        } => {
            let mut ctx = ConfirmContext::for_question(&question);
            if let Some(console) = console {
                let Some(facts) = console_facts(cx, console) else {
                    let _ = reply.send(ConfirmAnswer::Cancel);
                    return;
                };
                question.connection = facts.name.clone();
                question.production = facts.env == Some(EnvTag::Production);
                ctx.env = facts.env;
                ctx.engine = facts.engine;
                ctx.console = Some(facts.name);
            }
            let rx = open_in(question, ctx, cx);
            forward(reply, rx, ConfirmAnswer::Cancel, cx);
        }
        PortRequest::Commit {
            console,
            mut question,
            reply,
            follow,
        } => {
            let mut ctx = commit_prompt::CommitContext {
                env: question.production.then_some(EnvTag::Production),
                ..Default::default()
            };
            if let Some(console) = console {
                let Some(facts) = console_facts(cx, console) else {
                    let _ = reply.send(CommitAnswer::RollBack);
                    return;
                };
                question.connection = facts.name;
                question.production = facts.env == Some(EnvTag::Production);
                ctx.env = facts.env;
                ctx.engine = facts.engine;
            }
            ctx.console = console;
            if let Some(f) = &follow {
                ctx.sql = usize::try_from(question.unit.ordinal)
                    .ok()
                    .and_then(|i| f.sql.get(i).cloned());
                ctx.follow = Some(f.choices.clone());
            }
            let rx = commit_prompt::open_in(question, ctx, cx);
            forward(reply, rx, CommitAnswer::RollBack, cx);
        }
        PortRequest::Lost {
            console,
            statements_before,
        } => commit_prompt::connection_lost(console, statements_before, cx),
    }
}

/// Forwards the dialog's answer `rx` to the asker's `reply`; a dialog closed without an answer
/// answers `safe`.
fn forward<T: Send + 'static>(
    reply: oneshot::Sender<T>,
    rx: oneshot::Receiver<T>,
    safe: T,
    cx: &mut App,
) {
    cx.spawn(async move |_| {
        let answer = rx.await.unwrap_or(safe);
        let _ = reply.send(answer);
    })
    .detach();
}

/// The view IDs of the open confirmation dialogs (tests).
pub fn open_confirm_views(cx: &App) -> Vec<EntityId> {
    cx.try_global::<OpenPrompts>()
        .map(|o| {
            o.confirms
                .iter()
                .filter_map(WeakEntity::upgrade)
                .map(|d| d.entity_id())
                .collect()
        })
        .unwrap_or_default()
}

// ---- shots fixtures -----------------------------------------------------------------------------

/// The shots' fixtures of surface 1 (`design/m3b-safe`): each opens one state over the workspace
/// with a question whose asker never answers (the reply is held).
#[cfg(feature = "shots")]
pub mod fixtures {
    use std::sync::Arc;

    use hacivat_exec::{ConfirmAnswer, ConfirmQuestion, GateClass, GateReason};
    use hacivat_proto::EnvTag;
    use hacivat_session::session::RunId;
    use hacivat_ui_core::gpui_surface::{App, Entity, Global, SharedString, Window};
    use tokio::sync::oneshot;

    use super::{ConfirmContext, ConfirmDialog, show};

    /// The askers' ends, kept so each question stays open.
    #[derive(Default)]
    struct Held(Vec<oneshot::Receiver<ConfirmAnswer>>);

    impl Global for Held {}

    /// One state of the confirmation dialog.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum State {
        /// Production, one statement Hacivat can't classify (an `UPDATE` calling a function).
        Unknown,
        /// Statement 2 of 3: `DROP TABLE`, Execute all and Skip all.
        Multi,
        /// Statement 2 of 3 whose name must be typed (Execute and Execute all disabled).
        MultiType,
        /// Production `DELETE` with no `WHERE`: type the name.
        Type,
        /// The name typed exactly (Execute enabled).
        Typed,
        /// The name typed in the wrong letter case (the hint).
        Mismatch,
        /// `DROP SCHEMA … CASCADE`: type the name.
        DropSchema,
        /// MySQL DDL: can't be rolled back.
        MysqlDdl,
        /// MySQL DDL inside an open transaction (implicit commit).
        ImplicitCommit,
        /// `CALL` on production.
        Routine,
        /// Off production, no `WHERE` (calm; "don't ask again" offered).
        Calm,
        /// `SELECT … FOR UPDATE` on production (Warn).
        Warn,
        /// `EXPLAIN ANALYZE` on production: what a rollback can't undo.
        Explain,
        /// Production `CREATE INDEX`: "don't ask again" names its scope.
        SuppressProd,
        /// Off production, a statement Hacivat couldn't analyse (calm, never suppressible).
        NotAnalysed,
    }

    pub const STATES: [State; 15] = [
        State::Unknown,
        State::Multi,
        State::MultiType,
        State::Type,
        State::Typed,
        State::Mismatch,
        State::DropSchema,
        State::MysqlDdl,
        State::ImplicitCommit,
        State::Routine,
        State::Calm,
        State::Warn,
        State::Explain,
        State::SuppressProd,
        State::NotAnalysed,
    ];

    impl State {
        pub fn name(self) -> &'static str {
            match self {
                State::Unknown => "confirm-unknown",
                State::Multi => "confirm-multi",
                State::MultiType => "confirm-multi-type",
                State::Type => "confirm-type",
                State::Typed => "confirm-typed",
                State::Mismatch => "confirm-mismatch",
                State::DropSchema => "confirm-drop-schema",
                State::MysqlDdl => "confirm-mysql-ddl",
                State::ImplicitCommit => "confirm-implicit-commit",
                State::Routine => "confirm-routine",
                State::Calm => "confirm-calm",
                State::Warn => "confirm-warn",
                State::Explain => "confirm-explain",
                State::SuppressProd => "confirm-suppress-prod",
                State::NotAnalysed => "confirm-not-analysed",
            }
        }
    }

    fn reason(id: &'static str, args: &[(&'static str, &str)]) -> GateReason {
        GateReason {
            fluent_id: id,
            args: args.iter().map(|(k, v)| (*k, Arc::from(*v))).collect(),
        }
    }

    fn question(sql: &str, reasons: Vec<GateReason>, production: bool) -> ConfirmQuestion {
        ConfirmQuestion {
            run: RunId(1),
            unit: 0,
            index: 1,
            total: 1,
            sql_masked: Arc::from(sql),
            reasons,
            type_to_confirm: None,
            irreversible: false,
            suppressible: false,
            warn_only: false,
            production,
            connection: Arc::from(if production {
                "orders-prod"
            } else {
                "orders-staging"
            }),
        }
    }

    fn ctx(class: GateClass, env: EnvTag, engine: &str) -> ConfirmContext {
        ConfirmContext {
            class: Some(class),
            env: Some(env),
            engine: Some(SharedString::from(engine.to_string())),
            ..ConfirmContext::default()
        }
    }

    /// Opens `state` over `window`'s workspace.
    pub fn open(state: State, window: &mut Window, cx: &mut App) {
        let pg = "PostgreSQL 16";
        let (mut q, mut c, typed) = match state {
            State::Unknown => (
                question(
                    "UPDATE orders\n   SET total = recalc_total(id)\n WHERE created_at > now() - interval '1 day';",
                    vec![reason(
                        "safety-reason-fn-unknown",
                        &[("name", "recalc_total")],
                    )],
                    true,
                ),
                ctx(GateClass::Unknown, EnvTag::Production, pg),
                None,
            ),
            State::Multi | State::MultiType => {
                let mut q = question(
                    "DROP TABLE order_archive_2019;",
                    vec![reason(
                        "safety-destructive-drop",
                        &[("table", "order_archive_2019")],
                    )],
                    true,
                );
                let mut c = ctx(GateClass::DdlDestructive, EnvTag::Production, pg);
                if state == State::MultiType {
                    q.sql_masked = Arc::from("DELETE FROM müşteriler;");
                    q.reasons = vec![reason(
                        "safety-destructive-no-where",
                        &[("table", "müşteriler")],
                    )];
                    q.type_to_confirm = Some(Arc::from("müşteriler"));
                    c.class = Some(GateClass::DmlUnsafe);
                }
                q.unit = 1;
                q.index = 2;
                q.total = 3;
                c.covered = 1;
                c.later_asks = 1;
                (q, c, None)
            }
            State::Type | State::Typed | State::Mismatch => {
                let mut q = question(
                    "DELETE FROM müşteriler;",
                    vec![reason(
                        "safety-destructive-no-where",
                        &[("table", "müşteriler")],
                    )],
                    true,
                );
                q.type_to_confirm = Some(Arc::from("müşteriler"));
                let typed = match state {
                    State::Typed => Some("müşteriler"),
                    State::Mismatch => Some("Müşteriler"),
                    _ => None,
                };
                (q, ctx(GateClass::DmlUnsafe, EnvTag::Production, pg), typed)
            }
            State::DropSchema => {
                let mut q = question(
                    "DROP SCHEMA reporting CASCADE;",
                    vec![reason(
                        "safety-destructive-drop-cascade",
                        &[("table", "reporting")],
                    )],
                    true,
                );
                q.type_to_confirm = Some(Arc::from("reporting"));
                (
                    q,
                    ctx(GateClass::DdlDestructive, EnvTag::Production, pg),
                    None,
                )
            }
            State::MysqlDdl => {
                let mut q = question(
                    "ALTER TABLE orders DROP COLUMN legacy_ref;",
                    vec![reason(
                        "safety-destructive-drop-column",
                        &[("table", "orders")],
                    )],
                    true,
                );
                q.irreversible = true;
                (
                    q,
                    ctx(GateClass::DdlDestructive, EnvTag::Production, "MySQL 8.4"),
                    None,
                )
            }
            State::ImplicitCommit => {
                let mut q = question(
                    "CREATE INDEX idx_orders_created ON orders (created_at);",
                    vec![reason("exec-gate-implicit-commit", &[])],
                    true,
                );
                q.irreversible = true;
                (
                    q,
                    ctx(GateClass::DdlOther, EnvTag::Production, "MySQL 8.4"),
                    None,
                )
            }
            State::Routine => (
                question(
                    "CALL close_month('2026-10');",
                    vec![reason("safety-reason-routine-call", &[])],
                    true,
                ),
                ctx(GateClass::Routine, EnvTag::Production, pg),
                None,
            ),
            State::Calm => {
                let mut q = question(
                    "UPDATE customers SET newsletter = false;",
                    vec![reason(
                        "safety-destructive-no-where",
                        &[("table", "customers")],
                    )],
                    false,
                );
                q.suppressible = true;
                (q, ctx(GateClass::DmlUnsafe, EnvTag::Staging, pg), None)
            }
            State::Warn => {
                let mut q = question(
                    "SELECT * FROM orders WHERE id = 4211 FOR UPDATE;",
                    vec![reason("safety-reason-locking-clause", &[])],
                    true,
                );
                q.warn_only = true;
                q.suppressible = true;
                (q, ctx(GateClass::Lock, EnvTag::Production, pg), None)
            }
            State::Explain => (
                question(
                    "EXPLAIN ANALYZE UPDATE orders SET total = recalc_total(id);",
                    vec![
                        reason("safety-reason-fn-unknown", &[("name", "recalc_total")]),
                        reason("exec-explain-analyze-not-undone", &[]),
                    ],
                    true,
                ),
                ctx(GateClass::Unknown, EnvTag::Production, pg),
                None,
            ),
            State::SuppressProd => {
                let mut q = question(
                    "CREATE INDEX CONCURRENTLY idx_orders_customer ON orders (customer_id);",
                    vec![reason("safety-reason-schema-change", &[])],
                    true,
                );
                q.suppressible = true;
                (q, ctx(GateClass::DdlOther, EnvTag::Production, pg), None)
            }
            State::NotAnalysed => {
                let q = question(
                    "/*!50001 CREATE ALGORITHM=UNDEFINED VIEW v_orders AS SELECT * FROM orders */;",
                    vec![
                        reason("exec-gate-not-analysed", &[]),
                        reason("safety-reason-executable-comment", &[]),
                    ],
                    false,
                );
                (
                    q,
                    ctx(GateClass::Unknown, EnvTag::Staging, "MySQL 8.4"),
                    None,
                )
            }
        };
        if q.connection.is_empty() {
            q.connection = Arc::from("orders-prod");
        }
        c.console = Some(q.connection.clone());
        let (reply, rx) = oneshot::channel();
        cx.default_global::<Held>().0.push(rx);
        show(q, c, reply, window, cx);
        if let Some(text) = typed {
            let input = cx
                .try_global::<super::OpenPrompts>()
                .and_then(|o| o.confirms.iter().rev().find_map(|d| d.upgrade()))
                .and_then(|d: Entity<ConfirmDialog>| d.read(cx).typed.clone());
            if let Some(input) = input {
                input.update(cx, |i, cx| {
                    i.set_value(text.to_string());
                    cx.notify();
                });
            }
        }
    }
}
