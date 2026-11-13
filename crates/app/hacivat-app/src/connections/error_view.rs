//! The connect-error block and dialog (NOTES surface 3, HV-CONN-03, Task 20).
//!
//! An error is a sentence about the user's situation, a trail that shows where the connection
//! broke, and one-click fixes; the server's raw text is one disclosure away, never the heading.
//! The block draws:
//! - the heading (`error-conn-<class>-title` with the error's arguments, never the server's
//!   text) and the explanation (`-body`);
//! - the step trail with ✕ at the failed step (R25; `config.invalid` has none);
//! - the fixes from `fixes_for` ([`fix_actions`]); `UseDatabase` is one button per suggestion;
//! - "Technical details": the stage, the class ID, the code chip, the server's message verbatim
//!   (data), the elapsed time, and "Copy details (English)" (R26).
//!
//! Inline (the connection dialog's test strip) every fix is emitted as
//! [`ErrorViewEvent::Fix`] for the dialog to run (P40). In the error dialog
//! ([`open_error_dialog`]: a 480 px modal layer `connection-error`) the fixes run here:
//! - `ReenterPassword` opens a masked field with "Attempt n of 3" and Connect (P8). The view
//!   keeps the gesture's count of server rejections, starting at the error's `attempts`; each
//!   re-entry passes it to the pipeline as `prior_rejections`, and the pipeline's answer carries
//!   the gesture's running count (`ConnectError::attempts`). `n` is the count + 1; at 3 the
//!   field and the fix are gone, so a gesture never causes more than 3 rejections.
//! - `UseDatabase` and `ConnectAnyway` (persists `allow_unsupported_server`) save the profile
//!   through `ConnectionsModel` and connect. A read-only connections file turns them off (and the
//!   certificate fixes, which save a pin or a CA file), with a line that says why; a failed save
//!   is shown.
//! - M2b (Task 19): "Review certificate…" and "Import CA…" close the dialog and run in
//!   `cert_dialog` (it saves the change and connects); M2a's interim "Encrypt without
//!   verifying" is gone (B6).
//! - `TryAgain` connects; `EditConnection` opens the connection dialog; `SupportedVersions`
//!   opens R11's URL.
//!
//! A connect started here is watched: success closes the dialog, a failure shows the new
//! error. The dialog traps Tab (like `Dialog`); Escape closes it (the workspace pops the layer,
//! and the re-entry field's `Cancelled` does the same, P16).

use std::collections::HashMap;
use std::panic::Location;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_proto::{EnvTag, ErrClass, ProfileId};
use hacivat_session::LiveState;
use hacivat_session::pipeline::ConnectError;
use hacivat_session::profile::{Profile, ProfileDraft};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::{EnvKind, env_swatch};
use hacivat_ui::dialog::{DialogSize, dialog_frame};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::popover::{A11yDecl, A11yNode, Declare};
use hacivat_ui::step_trail::StepTrail;
use hacivat_ui::text::{Label, RenderedText, TextRegistry, TextSource};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Div, Entity, EventEmitter,
    FocusHandle, Focusable, Hsla, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Render, Role, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Task, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, TypeStyle, focus_outline};
use secrecy::SecretString;

use crate::connections::dialog::{DialogMode, open_connection_dialog};
use crate::connections::fixes::{
    FixAction, FixButton, MAX_REJECTIONS, SUPPORTED_VERSIONS_URL, fix_actions, fix_buttons,
};
use crate::connections::interaction::interaction;
use crate::connections::model::ConnectionsModel;
use crate::connections::report::{
    Fmt, error_body, error_details_text, error_title, tech_rows, trail_a11y, trail_items,
};
use crate::l10n_global::tr;
use crate::shell::Workspace;
use crate::shell::layers::{DIALOG_CONTEXT, Layer};

/// The error dialog's layer ID.
pub const ERROR_LAYER_ID: &str = "connection-error";

/// The width of the re-entered password field (the mock's `pwrow` input).
const REENTER_FIELD_PX: f32 = 240.0;
/// The technical details' label column (mock `.kvx` 110 px).
const TECH_LABEL_PX: f32 = 110.0;

/// What the error block emits inline.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ErrorViewEvent {
    /// A fix the host runs (the connection dialog, P40).
    Fix(FixAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Inside the test strip: fixes are emitted.
    Inline,
    /// The error dialog of a saved profile: fixes run here.
    Dialog { profile: ProfileId },
}

/// What `connect` does after reading the state the manager left (`ErrorView::after_start`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterStart {
    /// The connect runs: watch it.
    Watch,
    /// The profile is connected: close the dialog.
    Close,
    /// Nothing runs (any new failure is already shown).
    Done,
}

/// A line shown under the fixes after a save failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notice {
    SaveFailed,
}

/// The error block, inline or as the error dialog (NOTES surface 3).
pub struct ErrorView {
    error: Arc<ConnectError>,
    mode: Mode,
    /// Server rejections of a password in this gesture (P8).
    rejections: u8,
    tech_open: bool,
    reenter: Option<Entity<TextInput>>,
    /// A connect started from here is running.
    pending: bool,
    notice: Option<Notice>,
    root: FocusHandle,
    /// The fix buttons' focus handles, keyed by button ID (the list changes as fixes come and
    /// go, so a position would hand a handle to another button).
    fix_focus: HashMap<SharedString, FocusHandle>,
    connect_focus: FocusHandle,
    tech_focus: FocusHandle,
    copy_focus: FocusHandle,
    close_focus: FocusHandle,
    watch: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl EventEmitter<ErrorViewEvent> for ErrorView {}

impl Focusable for ErrorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

/// A focus handle that is a tab stop.
fn stop(cx: &mut App) -> FocusHandle {
    cx.focus_handle().tab_stop(true)
}

impl ErrorView {
    /// The block inside the test strip: every fix is emitted.
    pub fn inline(error: Arc<ConnectError>, cx: &mut Context<Self>) -> Self {
        Self::with_mode(error, Mode::Inline, cx)
    }

    fn with_mode(error: Arc<ConnectError>, mode: Mode, cx: &mut Context<Self>) -> Self {
        ErrorView {
            rejections: error.attempts,
            error,
            mode,
            tech_open: false,
            reenter: None,
            pending: false,
            notice: None,
            root: cx.focus_handle(),
            fix_focus: HashMap::new(),
            connect_focus: stop(cx),
            tech_focus: stop(cx),
            copy_focus: stop(cx),
            close_focus: stop(cx),
            watch: None,
            subscriptions: Vec::new(),
        }
    }

    /// The error shown.
    pub fn error(&self) -> &Arc<ConnectError> {
        &self.error
    }

    /// The gesture's server rejections so far (P8).
    pub fn rejections(&self) -> u8 {
        self.rejections
    }

    /// Whether the re-entered password field is open.
    pub fn reentering(&self) -> bool {
        self.reenter.is_some()
    }

    /// The ID of the focused control of the dialog (`fix-…`, `fix-connect`, `tech-details`,
    /// `copy-details`, `error-close`), if one is focused.
    pub fn focused_control(&self, window: &Window, cx: &App) -> Option<SharedString> {
        if let Some((id, _)) = self.fix_focus.iter().find(|(_, h)| h.is_focused(window)) {
            return Some(id.clone());
        }
        let reenter = self
            .reenter
            .as_ref()
            .map(|i| i.read(cx).focus_handle(cx))
            .filter(|h| h.is_focused(window));
        let named = [
            (reenter.as_ref(), "reenter-field"),
            (Some(&self.connect_focus), "fix-connect"),
            (Some(&self.tech_focus), "tech-details"),
            (Some(&self.copy_focus), "copy-details"),
            (Some(&self.close_focus), "error-close"),
        ];
        named
            .into_iter()
            .find(|(h, _)| h.is_some_and(|h| h.is_focused(window)))
            .map(|(_, id)| SharedString::from(id))
    }

    /// The focus handle of the fix button `id`, made on first use.
    fn fix_handle(&mut self, id: &SharedString, cx: &mut App) -> FocusHandle {
        self.fix_focus
            .entry(id.clone())
            .or_insert_with(|| stop(cx))
            .clone()
    }

    /// Where the dialog's focus starts: the primary fix, else Edit connection, else Close.
    /// A fix that weakens safety (Connect anyway) never has it, so one reflexive Enter cannot
    /// run it.
    fn initial_focus(&mut self, cx: &mut App) -> FocusHandle {
        let buttons = self.fix_buttons(cx);
        let usable = |b: &&FixButton| !b.disabled && !b.action.weakens_safety();
        let chosen = buttons
            .iter()
            .filter(usable)
            .find(|b| b.kind == ButtonKind::Primary)
            .or_else(|| {
                buttons
                    .iter()
                    .filter(usable)
                    .find(|b| b.action == FixAction::EditConnection)
            })
            .map(|b| b.id.clone());
        match chosen {
            Some(id) => self.fix_handle(&id, cx),
            None => self.close_focus.clone(),
        }
    }

    fn in_dialog(&self) -> bool {
        matches!(self.mode, Mode::Dialog { .. })
    }

    /// The fix buttons as drawn: IDs, labels, kinds and whether each is disabled. In the
    /// dialog, fixes that edit the profile are off while the connections file is read-only,
    /// and every fix is off while a connect started here runs.
    pub fn fix_buttons(&self, cx: &App) -> Vec<FixButton> {
        let mut actions = fix_actions(&self.error, self.rejections, self.in_dialog());
        if self.reenter.is_some() {
            actions.retain(|a| *a != FixAction::ReenterPassword);
        }
        let read_only = self.in_dialog()
            && cx
                .try_global::<ConnectionsModel>()
                .is_some_and(ConnectionsModel::store_read_only);
        let mut buttons = fix_buttons(&actions, cx);
        for b in &mut buttons {
            b.disabled = self.pending || (read_only && b.action.edits_profile());
        }
        buttons
    }

    // ---- fixes -------------------------------------------------------------------------------

    fn run_fix(&mut self, action: FixAction, window: &mut Window, cx: &mut Context<Self>) {
        let Mode::Dialog { profile } = self.mode else {
            cx.emit(ErrorViewEvent::Fix(action));
            return;
        };
        if action.starts_new_gesture() {
            // A new gesture gets a fresh budget of rejections; only the re-entry continues it.
            self.rejections = 0;
            self.reenter = None;
        }
        match action {
            FixAction::EditConnection => {
                window.defer(cx, move |window, cx| {
                    close_layer(window, cx);
                    open_connection_dialog(DialogMode::Edit(profile), window, cx);
                });
            }
            FixAction::TryAgain => self.connect(None, window, cx),
            FixAction::ReenterPassword => self.open_reenter(window, cx),
            FixAction::UseDatabase(db) => {
                self.edit_and_connect(move |p| p.database = Some(db), window, cx);
            }
            FixAction::SupportedVersions => cx.open_url(SUPPORTED_VERSIONS_URL),
            FixAction::ConnectAnyway => {
                self.edit_and_connect(|p| p.allow_unsupported_server = true, window, cx);
            }
            // ## m2b-t01-interfaces: the M2b flows, after the error dialog closes (Q9).
            m2b @ (FixAction::ReviewCertificate
            | FixAction::ImportCa
            | FixAction::ConnectThroughSsh
            | FixAction::ReviewHostKey) => {
                let error = self.error.clone();
                window.defer(cx, move |window, cx| {
                    close_layer(window, cx);
                    crate::connections::fixes::run_m2b_fix(&m2b, &error, profile, window, cx);
                });
            }
        }
        cx.notify();
    }

    /// Opens the masked re-entry field and focuses it.
    fn open_reenter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rejections >= MAX_REJECTIONS {
            return;
        }
        let input = match &self.reenter {
            Some(input) => input.clone(),
            None => {
                let user = self.error.args.user.clone().unwrap_or_default();
                let placeholder = Rc::new(move |cx: &App| {
                    tr(
                        cx,
                        ids::hv::CONN_ERR_NEW_PW,
                        ids::hv::HvConnErrNewPw {
                            user: Text(user.clone()),
                        },
                    )
                });
                let input = cx.new(|cx| {
                    let mut i = TextInput::new(cx, placeholder);
                    i.set_masked(true);
                    i.set_well(true);
                    i
                });
                let sub =
                    cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                        match event {
                            InputEvent::Submitted => this.submit_reenter(window, cx),
                            // Connect is enabled only while the field holds something.
                            InputEvent::Changed => cx.notify(),
                            // A focused field swallows Escape (P16): it closes the dialog.
                            InputEvent::Cancelled => {
                                window.defer(cx, close_layer);
                            }
                            _ => {}
                        }
                    });
                self.subscriptions.push(sub);
                self.reenter = Some(input.clone());
                input
            }
        };
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Connects with the re-entered password, continuing the gesture's count (P8). The field's
    /// text becomes a `SecretString` at once and the field is cleared.
    fn submit_reenter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || self.rejections >= MAX_REJECTIONS {
            return;
        }
        let Some(input) = self.reenter.clone() else {
            return;
        };
        // An empty field sends nothing: it would spend one of the gesture's 3 attempts.
        if input.read(cx).value().is_empty() {
            return;
        }
        let password = input.update(cx, |i, _| {
            let typed = SecretString::from(i.value().to_string());
            i.set_value("");
            typed
        });
        self.connect(Some(password), window, cx);
    }

    /// Saves the profile with `edit` applied (through `ConnectionsModel`, the one writer), then
    /// connects. A failed save is shown; the connect uses the edited profile either way.
    fn edit_and_connect(
        &mut self,
        edit: impl FnOnce(&mut Profile),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Mode::Dialog { profile: id } = self.mode else {
            return;
        };
        let Some(stored) = cx.try_global::<ConnectionsModel>().and_then(|m| m.get(id)) else {
            return;
        };
        if cx.global::<ConnectionsModel>().store_read_only() {
            return;
        }
        let mut profile = (*stored).clone();
        edit(&mut profile);
        self.notice = None;
        let saved = ConnectionsModel::save(cx, ProfileDraft::edit(profile));
        cx.spawn(async move |this, cx| {
            if saved.await.is_err() {
                let _ = this.update(cx, |this, cx| {
                    this.notice = Some(Notice::SaveFailed);
                    cx.notify();
                });
            }
        })
        .detach();
        self.connect(None, window, cx);
    }

    /// Connects the dialog's profile (with a re-entered password, continuing the count) and
    /// watches the outcome: connected closes the dialog, a failure shows the new error.
    fn connect(
        &mut self,
        password: Option<SecretString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Mode::Dialog { profile: id } = self.mode else {
            return;
        };
        let Some(model) = cx.try_global::<ConnectionsModel>() else {
            return;
        };
        let manager = model.manager().clone();
        let Some(profile) = model.get(id) else {
            return;
        };
        let mut changes = manager.changes();
        match password {
            Some(password) => {
                let ix = interaction(cx);
                cx.global_mut::<ConnectionsModel>().note_attempt(id);
                manager.connect_reentry(profile, password, self.rejections, ix);
            }
            None => ConnectionsModel::connect(cx, id, None),
        }
        match self.after_start(manager.state(id), cx) {
            AfterStart::Watch => {}
            AfterStart::Close => {
                window.defer(cx, close_layer);
                return;
            }
            AfterStart::Done => return,
        }
        self.pending = true;
        let window_handle = window.window_handle();
        self.watch = Some(cx.spawn(async move |this, cx| {
            while changes.changed().await.is_ok() {
                match manager.state(id) {
                    // ## m2b-t01-interfaces: a reconnect is still a connect in progress.
                    LiveState::Connecting(_) | LiveState::Reconnecting { .. } => continue,
                    LiveState::Connected(_) => {
                        cx.update(|cx| {
                            let _ =
                                window_handle.update(cx, |_, window, cx| close_layer(window, cx));
                        });
                        break;
                    }
                    LiveState::Failed(error) => {
                        let _ = this.update(cx, |this, cx| this.show_error(error, cx));
                        break;
                    }
                    LiveState::Disconnected | LiveState::Lost { .. } => break,
                }
            }
            // However the watch ended (a state, or the manager's channel closed), nothing of
            // this connect is pending any more.
            let _ = this.update(cx, |this, cx| {
                if this.pending {
                    this.pending = false;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    /// What to do with the profile's state read right after asking the manager to connect.
    /// The connect may already be over (a fast failure on the core runtime landed first), or
    /// the manager may not have started one (already connected, shutting down). A failure
    /// other than the one shown is shown now: the new error, its count and its trail.
    fn after_start(&mut self, state: LiveState, cx: &mut Context<Self>) -> AfterStart {
        match state {
            // ## m2b-t01-interfaces: a reconnect is still a connect in progress.
            LiveState::Connecting(_) | LiveState::Reconnecting { .. } => AfterStart::Watch,
            LiveState::Connected(_) => {
                self.pending = false;
                AfterStart::Close
            }
            LiveState::Failed(error) if !Arc::ptr_eq(&error, &self.error) => {
                self.show_error(error, cx);
                AfterStart::Done
            }
            LiveState::Failed(_) | LiveState::Disconnected | LiveState::Lost { .. } => {
                self.pending = false;
                cx.notify();
                AfterStart::Done
            }
        }
    }

    /// Shows the error a connect started here ended with. The gesture's count only grows; the
    /// re-entry field closes at 3 rejections or when the new error is not a rejection.
    fn show_error(&mut self, error: Arc<ConnectError>, cx: &mut Context<Self>) {
        self.rejections = self.rejections.max(error.attempts);
        let rejected = error.class == ErrClass::AuthInvalid;
        self.error = error;
        self.pending = false;
        if !rejected || self.rejections >= MAX_REJECTIONS {
            self.reenter = None;
        }
        cx.notify();
    }

    fn copy_details(&mut self, cx: &mut Context<Self>) {
        let text = error_details_text(cx, &self.error);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    // ---- keys --------------------------------------------------------------------------------

    /// The dialog's tab order: the enabled fixes, the field and Connect, the details toggle,
    /// Copy details (open), Close.
    fn tab_order(&self, cx: &App) -> Vec<FocusHandle> {
        let mut order: Vec<FocusHandle> = self
            .fix_buttons(cx)
            .iter()
            .filter(|b| !b.disabled)
            .filter_map(|b| self.fix_focus.get(&b.id).cloned())
            .collect();
        if let Some(input) = &self.reenter {
            order.push(input.read(cx).focus_handle(cx));
            if !self.pending && !input.read(cx).value().is_empty() {
                order.push(self.connect_focus.clone());
            }
        }
        order.push(self.tech_focus.clone());
        if self.tech_open {
            order.push(self.copy_focus.clone());
        }
        order.push(self.close_focus.clone());
        order
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        if stroke.key != "tab" || stroke.modifiers.control || stroke.modifiers.platform {
            return;
        }
        let order = self.tab_order(cx);
        let n = order.len();
        if n > 0 {
            let at = order.iter().position(|h| h.is_focused(window));
            let next = match (at, stroke.modifiers.shift) {
                (Some(i), false) => (i + 1) % n,
                (Some(i), true) => (i + n - 1) % n,
                (None, false) => 0,
                (None, true) => n - 1,
            };
            window.focus(&order[next], cx);
        }
        cx.stop_propagation();
    }

    // ---- drawing -----------------------------------------------------------------------------

    fn block(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let f = Fmt::ui(cx);
        let error = self.error.clone();
        let inline = !self.in_dialog();

        // The heading and the explanation: inline with the ink alert glyph (no hue, NOTES #3).
        let heading_style = TypeStyle {
            weight: if inline { 600 } else { 400 },
            ..t.type_ramp.body
        };
        let text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(t.spacing.s_2xs))
            .child(paragraph(
                error_title(&f, &error).into(),
                TextSource::Fluent,
                heading_style,
                c.text.to_hsla(),
                Some("error-title"),
                cx,
            ))
            .child(paragraph(
                error_body(&f, &error).into(),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
                Some("error-body"),
                cx,
            ));
        let head = div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(t.spacing.md))
            .children(inline.then(|| {
                div().pt(px(1.0)).child(
                    Icon::new(IconName::TriangleAlert)
                        .size(IconSize::Chrome)
                        .color(c.text.to_hsla()),
                )
            }))
            .child(text);

        let mut block = div()
            .debug_selector(|| "error-block".to_string())
            .flex()
            .flex_col()
            .gap(px(t.spacing.lg))
            .child(head);
        if !error.trail.is_empty() {
            block = block.child(StepTrail::new(trail_items(&f, &error.trail)).a11y(trail_a11y(&f)));
        }

        // The fixes.
        let buttons = self.fix_buttons(cx);
        let edits_off = buttons
            .iter()
            .any(|b| b.disabled && b.action.edits_profile() && !self.pending);
        let mut row = div()
            .debug_selector(|| "error-fixes".to_string())
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(t.spacing.sm));
        for b in buttons {
            let handle = self.fix_handle(&b.id, cx);
            if b.kind == ButtonKind::Ghost {
                // "Connect anyway" sits apart, at the end of the row (the mock's `grow`).
                row = row.child(div().flex_1());
            }
            let action = b.action.clone();
            let icon = match &b.action {
                FixAction::TryAgain => Some(IconName::RotateCw),
                FixAction::UseDatabase(_) => Some(IconName::Database),
                FixAction::SupportedVersions => Some(IconName::ExternalLink),
                // ## m2b-t19-tls: the CA is a file.
                FixAction::ImportCa => Some(IconName::FileKey),
                _ => None,
            };
            let mut button = Button::new(b.id.clone(), b.label.clone())
                .kind(b.kind)
                .disabled(b.disabled)
                .focus_handle(handle)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.run_fix(action.clone(), window, cx);
                }));
            if let Some(icon) = icon {
                button = button.icon(icon);
            }
            row = row.child(button);
        }
        block = block.child(row);

        // The re-entered password (P8).
        if let Some(input) = self.reenter.clone() {
            let reenter_empty = input.read(cx).value().is_empty();
            let n = u64::from(self.rejections) + 1;
            let attempt = tr(
                cx,
                ids::hv::CONN_ERR_ATTEMPT,
                ids::hv::HvConnErrAttempt { n: Count(n) },
            );
            block = block.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.sm))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(t.spacing.md))
                            .child(
                                div()
                                    .id("reenter-field")
                                    .debug_selector(|| "reenter-field".to_string())
                                    .w(px(REENTER_FIELD_PX))
                                    .child(input),
                            )
                            .child(
                                Button::new(
                                    "fix-connect",
                                    tr(cx, ids::hv::CONN_FIX_CONNECT, NoArgs),
                                )
                                .kind(ButtonKind::Primary)
                                .disabled(self.pending || reenter_empty)
                                .focus_handle(self.connect_focus.clone())
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.submit_reenter(window, cx);
                                    },
                                )),
                            ),
                    )
                    .child(
                        Label::new(
                            attempt,
                            TextSource::Fluent,
                            t.type_ramp.caption,
                            c.text_2.to_hsla(),
                        )
                        .id("reenter-attempt"),
                    ),
            );
        }

        // ## m2a-dialog-retries: the gesture's 3 rejections are spent, so no re-entry is offered.
        if error.class == ErrClass::AuthInvalid && self.rejections >= MAX_REJECTIONS {
            block = block.child(paragraph(
                tr(cx, ids::hv::CONN_ERR_ATTEMPTS_USED, NoArgs),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
                Some("attempts-used"),
                cx,
            ));
        }

        // Why fixes are off.
        if edits_off {
            block = block.child(paragraph(
                tr(cx, ids::hv::CONN_FIX_READ_ONLY, NoArgs),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
                Some("fix-read-only"),
                cx,
            ));
        }
        if self.notice == Some(Notice::SaveFailed) {
            block = block.child(paragraph(
                tr(cx, ids::hv::CONN_FIX_SAVE_FAILED, NoArgs),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
                Some("fix-save-failed"),
                cx,
            ));
        }

        block.child(self.tech(&f, window, cx))
    }

    /// "Technical details": a toggle (chevron + label in `text-2`, the mock's `.tech .dsc`) and,
    /// open, an inset box of rows with "Copy details (English)".
    fn tech(&self, f: &Fmt, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let label = f.s(ids::hv::CONN_ERR_TECH, NoArgs);
        let open = self.tech_open;
        let focused = self.tech_focus.is_focused(window) && window.last_input_was_keyboard();
        let toggle = focus_outline(
            div()
                .id("tech-details")
                .debug_selector(|| "tech-details".to_string())
                .track_focus(&self.tech_focus)
                .declare_full(A11yDecl::new(Role::Button, label.clone()).expanded(open))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.sm))
                .rounded(px(t.radii.xs))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.tech_open = !this.tech_open;
                    cx.notify();
                }))
                .child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
                )
                .child(Label::new(
                    label,
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                )),
            &theme,
            focused,
        );
        let mut tech = div().flex().flex_col().gap(px(t.spacing.md)).child(toggle);
        if !open {
            return tech;
        }
        // The inset restores text-3 on its own fill (NOTES cross-cutting #4), but labels use
        // text-2 here, as on every raised surface.
        let well = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };
        let mut rows = div().flex().flex_col().gap(px(t.spacing.xs));
        for (key, label, value) in tech_rows(f, &self.error) {
            let value_el: AnyElement = match key {
                "code" => div()
                    .flex_none()
                    .border_1()
                    .border_color(c.seam_strong.to_hsla())
                    .rounded(px(t.radii.xs))
                    .px(px(t.spacing.xs))
                    .child(
                        Label::new(
                            value,
                            TextSource::Data,
                            t.type_ramp.mono_micro,
                            c.text.to_hsla(),
                        )
                        .id(SharedString::from(format!("tech-{key}-value"))),
                    )
                    .into_any_element(),
                "server" => paragraph(
                    value.into(),
                    TextSource::Data,
                    t.type_ramp.mono_data,
                    c.text.to_hsla(),
                    Some("tech-server-value"),
                    cx,
                ),
                "class" | "elapsed" => Label::new(
                    value,
                    if key == "class" {
                        TextSource::Data
                    } else {
                        TextSource::Fluent
                    },
                    t.type_ramp.mono_data,
                    c.text.to_hsla(),
                )
                .id(SharedString::from(format!("tech-{key}-value")))
                .into_any_element(),
                _ => Label::new(
                    value,
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text.to_hsla(),
                )
                .id(SharedString::from(format!("tech-{key}-value")))
                .into_any_element(),
            };
            rows = rows.child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(t.spacing.lg))
                    .child(
                        div().w(px(TECH_LABEL_PX)).flex_none().child(
                            Label::new(
                                label,
                                TextSource::Fluent,
                                t.type_ramp.body_sm,
                                c.text_2.to_hsla(),
                            )
                            .id(SharedString::from(format!("tech-{key}-label"))),
                        ),
                    )
                    .child(div().flex_1().min_w_0().child(value_el)),
            );
        }
        let copy = Button::new("copy-details", tr(cx, ids::hv::CONN_ERR_COPY, NoArgs))
            .kind(ButtonKind::Ghost)
            .icon(IconName::Copy)
            .focus_handle(self.copy_focus.clone())
            .on_click(cx.listener(|this, _, _, cx| this.copy_details(cx)));
        tech = tech.child(
            div()
                .debug_selector(|| "tech-box".to_string())
                .flex()
                .flex_col()
                .gap(px(t.spacing.md))
                .py(px(t.spacing.md))
                .px(px(t.spacing.lg))
                .border_1()
                .border_color(c.seam_strong.to_hsla())
                .rounded(px(t.radii.sm))
                .bg(well.to_hsla())
                .child(rows)
                .child(div().flex().flex_row().child(copy)),
        );
        tech
    }

    /// The dialog's header: the title and, under it, the environment mark and the address.
    fn header(&self, cx: &mut Context<Self>) -> Div {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let a = &self.error.args;
        let title = tr(
            cx,
            ids::hv::CONN_ERR_TITLE,
            ids::hv::HvConnErrTitle {
                name: Text(a.name.clone()),
            },
        );
        let env = match self.mode {
            Mode::Dialog { profile } => cx
                .try_global::<ConnectionsModel>()
                .and_then(|m| m.get(profile))
                .and_then(|p| p.environment),
            Mode::Inline => None,
        };
        let address = address_of(&self.error);
        div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.s_2xs))
            .child(
                Label::new(
                    title,
                    TextSource::Fluent,
                    t.type_ramp.body_strong,
                    c.text.to_hsla(),
                )
                .id("error-dialog-title"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.sm))
                    .children(env.map(|e| env_swatch(env_kind(e), &theme)))
                    .children(address.map(|a| {
                        Label::new(
                            a,
                            TextSource::Data,
                            t.type_ramp.mono_data,
                            c.text_2.to_hsla(),
                        )
                    })),
            )
    }
}

impl Render for ErrorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let block = self.block(window, cx);
        if !self.in_dialog() {
            return block.into_any_element();
        }
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let title = tr(
            cx,
            ids::hv::CONN_ERR_TITLE,
            ids::hv::HvConnErrTitle {
                name: Text(self.error.args.name.clone()),
            },
        );
        let close = Button::new("error-close", tr(cx, ids::hv::CONN_FIX_CLOSE, NoArgs))
            .kind(ButtonKind::Ghost)
            .focus_handle(self.close_focus.clone())
            .on_click(cx.listener(|_, _, window, cx| {
                window.defer(cx, close_layer);
            }));
        let card = dialog_frame(&theme, DialogSize::Wide)
            .id(ERROR_LAYER_ID)
            .debug_selector(|| ERROR_LAYER_ID.to_string())
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.header(cx))
            .child(block)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .pt(px(t.spacing.xs))
                    .child(close),
            );
        A11yNode::new(
            "connection-error-node",
            Role::Dialog,
            title.clone(),
            true,
            card,
        )
        .description(SharedString::from(error_title(&Fmt::ui(cx), &self.error)))
        .into_any_element()
    }
}

/// The address a failed connect used: `host:port`, the socket or the file.
fn address_of(e: &ConnectError) -> Option<SharedString> {
    let a = &e.args;
    match (&a.host, a.port, &a.file) {
        (Some(host), Some(port), _) => Some(format!("{host}:{port}").into()),
        (Some(host), None, _) => Some(host.clone().into()),
        (None, _, Some(file)) => Some(file.clone().into()),
        _ => None,
    }
}

/// The capsule mark of an environment tag.
pub(crate) fn env_kind(tag: EnvTag) -> EnvKind {
    match tag {
        EnvTag::Production => EnvKind::Production,
        EnvTag::Staging => EnvKind::Staging,
        EnvTag::Test => EnvKind::Test,
        EnvTag::Dev => EnvKind::Dev,
        EnvTag::Local => EnvKind::Local,
    }
}

/// Wrapping text (a `Label` is one line), recorded in the [`TextRegistry`] like a label so its
/// source stays checkable under the pseudo-locales; `id` is appended to the recorded path
/// (`#id`) for tests.
#[track_caller]
pub(crate) fn paragraph(
    text: SharedString,
    source: TextSource,
    style: TypeStyle,
    color: Hsla,
    id: Option<&'static str>,
    cx: &mut App,
) -> AnyElement {
    record_text(&text, source, style, id, Location::caller(), cx);
    div()
        .w_full()
        .font(font_for(&style))
        .text_size(px(style.size_px))
        .line_height(px(style.line_height))
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// Records `text` in the [`TextRegistry`] as drawn at `at` (`#id` appended), for text drawn
/// without a `Label` (wrapping paragraphs, a line split into wrapping words).
pub(crate) fn record_text(
    text: &SharedString,
    source: TextSource,
    style: TypeStyle,
    id: Option<&'static str>,
    at: &'static Location<'static>,
    cx: &mut App,
) {
    TextRegistry::record(cx, || {
        let file = at.file().replace('\\', "/");
        let path = match id {
            Some(id) => format!("{file}:{}:{}#{id}", at.line(), at.column()),
            None => format!("{file}:{}:{}", at.line(), at.column()),
        };
        RenderedText {
            text: text.clone(),
            source,
            size_px: style.size_px,
            painted_px: 0.0,
            clipped: false,
            truncated: false,
            path: path.into(),
        }
    });
}

/// Pops the error dialog when it is the top layer (a fix, Close, Escape in the field, or a
/// connect that worked).
fn close_layer(window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    ws.update(cx, |ws, cx| {
        if ws.layer_ids().last() == Some(&ERROR_LAYER_ID) {
            ws.pop_layer(window, cx);
        }
    });
}

/// Opens the error dialog for `id`'s failed connect (NOTES surface 3): a 480 px modal layer
/// `connection-error` over the workspace of `window`.
pub fn open_error_dialog(
    id: ProfileId,
    error: Arc<ConnectError>,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = open_error_dialog_view(id, error, window, cx);
}

/// Like [`open_error_dialog`], returning the dialog's view (tests, the shots harness). `None`
/// when `window` holds no workspace.
pub fn open_error_dialog_view(
    id: ProfileId,
    error: Arc<ConnectError>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<ErrorView>> {
    let ws = window.root::<Workspace>().flatten()?;
    let view = cx.new(|cx| ErrorView::with_mode(error, Mode::Dialog { profile: id }, cx));
    // The primary fix (else Edit connection, else Close) has the initial focus; never a fix
    // that weakens safety.
    let handle = view.update(cx, |v, cx| v.initial_focus(cx));
    ws.update(cx, |ws, cx| {
        ws.push_layer(
            Layer {
                id: ERROR_LAYER_ID,
                context: DIALOG_CONTEXT,
                view: view.clone().into(),
                restore_focus: None,
                modal: true,
            },
            window,
            cx,
        );
    });
    window.focus(&handle, cx);
    Some(view)
}

// ## shots fixtures (R28, P19)

#[cfg(feature = "shots")]
mod shots {
    use std::time::Duration;

    use hacivat_driver::ServerMessage;
    use hacivat_proto::{ErrClass, ProfileId};
    use hacivat_session::pipeline::{
        ConnectError, ErrArgs, Phase, StepOutcome, StepTrace, TrailStep,
    };
    use hacivat_ui_core::gpui_surface::{App, Window};

    use super::*;

    fn trail(failed: TrailStep) -> Vec<StepTrace> {
        let steps = [
            TrailStep::Resolve,
            TrailStep::Connect,
            TrailStep::Tls,
            TrailStep::SignIn,
            TrailStep::CheckServer,
        ];
        let at = steps.iter().position(|s| *s == failed).unwrap_or(0);
        steps
            .iter()
            .enumerate()
            .map(|(i, &step)| StepTrace {
                step,
                outcome: match i.cmp(&at) {
                    std::cmp::Ordering::Less => StepOutcome::Done,
                    std::cmp::Ordering::Equal => StepOutcome::Failed,
                    std::cmp::Ordering::Greater => StepOutcome::Pending,
                },
                elapsed: (i < at).then(|| Duration::from_millis(3 + 7 * i as u64)),
            })
            .collect()
    }

    /// A sample error of `class`, failed at `step`.
    pub(crate) fn sample(
        class: ErrClass,
        step: TrailStep,
        code: Option<&str>,
        server: &str,
    ) -> ConnectError {
        ConnectError {
            class,
            phase: Phase::DriverConnect,
            trail: trail(step),
            args: ErrArgs {
                name: "orders-prod".to_string(),
                host: Some("db-prod.acme.io".to_string()),
                port: Some(5432),
                user: Some("deploy".to_string()),
                database: Some("orders_v2".to_string()),
                file: None,
                engine: "PostgreSQL".to_string(),
                version: Some("9.6.24".to_string()),
                hop: None,
                hop_host: None,
            },
            server: (!server.is_empty()).then(|| ServerMessage {
                sqlstate: code.map(str::to_string),
                vendor_code: None,
                severity: Some("FATAL".to_string()),
                text: server.to_string(),
                detail: None,
                hint: None,
                lang_hint: None,
            }),
            fixes: hacivat_driver::fixes_for(class),
            suggestions: Vec::new(),
            issues: Vec::new(),
            elapsed: Duration::from_millis(214),
            attempts: u8::from(class == ErrClass::AuthInvalid),
            needs_interaction: false,
            certificate: None,
            host_key: None,
            profile_changes: Vec::new(),
        }
    }

    fn open(error: ConnectError, window: &mut Window, cx: &mut App) {
        open_error_dialog(ProfileId::new(), Arc::new(error), window, cx);
    }

    /// `auth.invalid` (28P01), technical details folded.
    pub fn auth_fixture(window: &mut Window, cx: &mut App) {
        let e = sample(
            ErrClass::AuthInvalid,
            TrailStep::SignIn,
            Some("28P01"),
            "password authentication failed for user \"deploy\"",
        );
        open(e, window, cx);
    }

    /// `config.unknown_database` with three databases to use.
    pub fn unknown_db_fixture(window: &mut Window, cx: &mut App) {
        let mut e = sample(
            ErrClass::ConfigUnknownDatabase,
            TrailStep::SignIn,
            Some("3D000"),
            "database \"orders_v2\" does not exist",
        );
        e.suggestions = vec![
            "orders".to_string(),
            "orders_archive".to_string(),
            "postgres".to_string(),
        ];
        open(e, window, cx);
    }

    /// `server.unsupported_version` (PostgreSQL 9.6.24).
    pub fn unsupported_fixture(window: &mut Window, cx: &mut App) {
        let e = sample(
            ErrClass::ServerUnsupportedVersion,
            TrailStep::CheckServer,
            None,
            "",
        );
        open(e, window, cx);
    }

    /// `tls.untrusted` with its certificate: Review certificate…, Import CA…, Edit connection
    /// (m2b-t19-tls: M2a's "Encrypt without verifying" is gone, B6).
    pub fn tls_fixture(window: &mut Window, cx: &mut App) {
        let mut e = sample(ErrClass::TlsUntrusted, TrailStep::Tls, None, "");
        e.certificate = Some(crate::connections::cert_dialog::sample_review(None));
        open(e, window, cx);
    }
}

#[cfg(feature = "shots")]
pub use shots::{auth_fixture, tls_fixture, unknown_db_fixture, unsupported_fixture};

#[cfg(feature = "shots")]
pub(crate) use shots::sample as sample_error;

// ## m2b-t24-shots: the M2b error copy (NOTES M2b surface 7), in M2a's anatomy (R28, P19).
#[cfg(feature = "shots")]
mod shots_m2b {
    use std::time::Duration;

    use hacivat_proto::{ErrClass, FixId, ProfileId};
    use hacivat_session::pipeline::{ConnectError, StepOutcome, StepTrace, TrailStep};
    use hacivat_ui_core::gpui_surface::{App, Window};

    use super::{open_error_dialog, sample_error};
    use crate::connections::cert_dialog::sample_review;

    /// `steps` with every step before `failed` done, `failed` failed and the rest pending.
    fn trail(steps: &[TrailStep], failed: TrailStep) -> Vec<StepTrace> {
        let at = steps.iter().position(|s| *s == failed).unwrap_or(0);
        steps
            .iter()
            .enumerate()
            .map(|(i, &step)| StepTrace {
                step,
                outcome: match i.cmp(&at) {
                    std::cmp::Ordering::Less => StepOutcome::Done,
                    std::cmp::Ordering::Equal => StepOutcome::Failed,
                    std::cmp::Ordering::Greater => StepOutcome::Pending,
                },
                elapsed: (i < at).then(|| Duration::from_millis(3 + 7 * i as u64)),
            })
            .collect()
    }

    /// `class` before the server was checked (no version), failing at `failed` of `steps`.
    fn error(class: ErrClass, steps: &[TrailStep], failed: TrailStep) -> ConnectError {
        let mut e = sample_error(class, failed, None, "");
        e.trail = trail(steps, failed);
        e.args.version = None;
        e
    }

    /// Under SSH the trail starts with the tunnel (NOTES: Resolve drops out, B11).
    const SSH_STEPS: [TrailStep; 5] = [
        TrailStep::Ssh,
        TrailStep::Connect,
        TrailStep::Tls,
        TrailStep::SignIn,
        TrailStep::CheckServer,
    ];
    const TLS_STEPS: [TrailStep; 5] = [
        TrailStep::Resolve,
        TrailStep::Connect,
        TrailStep::Tls,
        TrailStep::SignIn,
        TrailStep::CheckServer,
    ];

    /// An SSH failure at hop `hop` (from 0) of `bastion.acme.io`.
    fn ssh(class: ErrClass, hop: u8) -> ConnectError {
        let mut e = error(class, &SSH_STEPS, TrailStep::Ssh);
        e.args.hop = Some(hop);
        e.args.hop_host = Some("bastion.acme.io".to_string());
        e
    }

    fn open(error: ConnectError, window: &mut Window, cx: &mut App) {
        open_error_dialog(ProfileId::new(), std::sync::Arc::new(error), window, cx);
    }

    /// `auth.insecure_method` over unverified TLS: Review certificate…, Edit connection.
    pub fn insecure_tls_fixture(window: &mut Window, cx: &mut App) {
        let mut e = error(ErrClass::AuthInsecureMethod, &TLS_STEPS, TrailStep::SignIn);
        e.certificate = Some(sample_review(None));
        e.fixes = vec![FixId::ReviewCertificate, FixId::EditConnection];
        open(e, window, cx);
    }

    /// `auth.insecure_method` without TLS: no TLS step, Edit connection only (NOTES ruling 10).
    pub fn insecure_plain_fixture(window: &mut Window, cx: &mut App) {
        let steps = [
            TrailStep::Resolve,
            TrailStep::Connect,
            TrailStep::SignIn,
            TrailStep::CheckServer,
        ];
        let mut e = error(ErrClass::AuthInsecureMethod, &steps, TrailStep::SignIn);
        e.fixes = vec![FixId::EditConnection];
        open(e, window, cx);
    }

    /// `tls.pin_changed`: Review certificate… (NOTES M2b surface 5).
    pub fn pin_changed_fixture(window: &mut Window, cx: &mut App) {
        let mut e = error(ErrClass::TlsPinChanged, &TLS_STEPS, TrailStep::Tls);
        e.certificate = Some(sample_review(Some([0x5C; 32])));
        open(e, window, cx);
    }

    /// `ssh.auth` at hop 2.
    pub fn ssh_auth_fixture(window: &mut Window, cx: &mut App) {
        open(ssh(ErrClass::SshAuth, 1), window, cx);
    }

    /// `ssh.channel_open` at hop 2 (AllowTcpForwarding).
    pub fn ssh_channel_fixture(window: &mut Window, cx: &mut App) {
        open(ssh(ErrClass::SshChannelOpen, 1), window, cx);
    }

    /// `ssh.agent_unavailable` at hop 1.
    pub fn ssh_agent_fixture(window: &mut Window, cx: &mut App) {
        open(ssh(ErrClass::SshAgentUnavailable, 0), window, cx);
    }

    /// `ssh.host_key_changed` at hop 2: Review host key… (NOTES M2b surface 1).
    pub fn host_key_changed_fixture(window: &mut Window, cx: &mut App) {
        let mut e = ssh(ErrClass::SshHostKeyChanged, 1);
        let (old, new) = crate::connections::prompt_kit::shots::host_keys();
        e.host_key = Some(hacivat_session::pipeline::HostKeyMismatch {
            hop: 1,
            host: "bastion.acme.io".to_string(),
            port: 22,
            old,
            new,
            source: hacivat_net::ssh::KnownSource::Hacivat,
        });
        open(e, window, cx);
    }

    /// `ssh.host_key_revoked` at hop 2.
    pub fn host_key_revoked_fixture(window: &mut Window, cx: &mut App) {
        open(ssh(ErrClass::SshHostKeyRevoked, 1), window, cx);
    }
}

#[cfg(feature = "shots")]
pub use shots_m2b::{
    host_key_changed_fixture, host_key_revoked_fixture, insecure_plain_fixture,
    insecure_tls_fixture, pin_changed_fixture, ssh_agent_fixture, ssh_auth_fixture,
    ssh_channel_fixture,
};

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui::TestAppContext;
    use hacivat_driver::errors::Phase;
    use hacivat_session::pipeline::{ErrArgs, StepOutcome, StepTrace, TrailStep};

    use super::*;

    fn error(class: ErrClass, attempts: u8, failed: TrailStep) -> Arc<ConnectError> {
        Arc::new(ConnectError {
            class,
            phase: Phase::DriverConnect,
            trail: vec![StepTrace {
                step: failed,
                outcome: StepOutcome::Failed,
                elapsed: None,
            }],
            args: ErrArgs {
                name: "orders".into(),
                host: Some("127.0.0.1".into()),
                port: Some(5432),
                user: Some("deploy".into()),
                database: None,
                file: None,
                engine: "PostgreSQL".into(),
                version: None,
                hop: None,
                hop_host: None,
            },
            server: None,
            fixes: Vec::new(),
            suggestions: Vec::new(),
            issues: Vec::new(),
            elapsed: Duration::from_millis(5),
            attempts,
            needs_interaction: false,
            certificate: None,
            host_key: None,
            profile_changes: Vec::new(),
        })
    }

    /// A failure that lands before the state is read still replaces the error shown: the
    /// new error, its count and its trail.
    #[gpui::test]
    fn a_failure_read_right_after_the_start_is_shown(cx: &mut TestAppContext) {
        let shown = error(ErrClass::NetworkDns, 2, TrailStep::Resolve);
        let view = cx.new(|cx| {
            ErrorView::with_mode(
                shown.clone(),
                Mode::Dialog {
                    profile: ProfileId::new(),
                },
                cx,
            )
        });
        let fresh = error(ErrClass::AuthInvalid, 3, TrailStep::SignIn);
        let next = view.update(cx, |v, cx| {
            v.pending = true;
            v.after_start(LiveState::Failed(fresh.clone()), cx)
        });
        assert_eq!(next, AfterStart::Done);
        view.read_with(cx, |v, _| {
            assert!(Arc::ptr_eq(v.error(), &fresh));
            assert_eq!(v.rejections(), 3);
            assert_eq!(v.error().trail[0].step, TrailStep::SignIn);
            assert!(!v.pending);
        });
        // The error already shown is not shown again; nothing is pending.
        let again = view.update(cx, |v, cx| {
            v.pending = true;
            v.after_start(LiveState::Failed(fresh.clone()), cx)
        });
        assert_eq!(again, AfterStart::Done);
        assert!(view.read_with(cx, |v, _| !v.pending));
        let connecting = view.update(cx, |v, cx| {
            v.after_start(
                LiveState::Connecting(hacivat_session::pipeline::Progress {
                    trail: Vec::new(),
                    current: None,
                    started: std::time::Instant::now(),
                }),
                cx,
            )
        });
        assert_eq!(connecting, AfterStart::Watch);
        assert_eq!(
            view.update(cx, |v, cx| v.after_start(LiveState::Disconnected, cx)),
            AfterStart::Done
        );
    }
}
