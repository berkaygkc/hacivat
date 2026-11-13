//! The vault (NOTES surface 4, B13, Q23, Q38, Task 21): create it, unlock it, reset it, the
//! damaged-file notice, and the label the "Remember" select shows for `Forever` when the vault
//! holds the secrets.
//!
//! **Two ways in.**
//! - The connect pipeline asks (`Question::CreateVault`, `Question::UnlockVault { attempt }`):
//!   the dialog answers `Master(..)` and the pipeline creates or unlocks on its blocking pool,
//!   counting the unlock attempts per connect (B13: 3 per gesture).
//! - Hacivat itself needs the vault: [`ensure_vault_before_forever`] before a `Forever` save
//!   (Task 1's line in `ConnectionsModel::save`), [`unlock_vault`] when a delete or duplicate
//!   met a locked vault (T6 carry). The dialog then creates or unlocks on the core runtime's
//!   blocking pool itself (Argon2, Q38) and counts its own attempts, from 1 for each gesture: a
//!   Cancel after a wrong master password never carries an attempt into the next gesture.
//!
//! Nothing here touches the vault on the UI thread: `exists`, `is_unlocked` and `resettable`
//! may wait for a whole Argon2 run or do file I/O (T6 review N1), so they run on the blocking
//! pool and come back through a `oneshot`.
//!
//! **Surfaces.**
//! - Create (480, layer `vault-dialog`): two masked fields (Master password, Repeat), the live
//!   rules "At least 8 characters" and "Both entries match" / "The two entries don't match",
//!   the "can't be recovered" notice and "Not now keeps this password until Hacivat restarts.";
//!   Not now / Create vault (primary, disabled until both rules hold). Enter in Master password
//!   moves to Repeat; Enter in Repeat creates only when enabled.
//! - Unlock (480, layer `vault-dialog`): one masked field, after a miss "That isn't the master
//!   password." and "Attempt n of 3" under it; Reset vault… (ghost, far left) / Cancel / Unlock
//!   (primary, disabled while empty). One reset entry only (Task 2 ruling: no "Forgot it?").
//! - Reset (420 alertdialog, layer `vault-reset`, stacked over Unlock or Damaged): type the
//!   word `hv-conn-vault-reset-word` shows (`reset` in EN and TR) into a mono field; the Danger
//!   button stays disabled until the field equals it exactly and is never the default: Enter
//!   does nothing here (B13). Escape returns to the dialog under it.
//! - Damaged (480 alertdialog, layer `vault-damaged`), after an unlock found the file corrupt:
//!   Reset vault… / Close (primary, with the focus).
//!
//! **Reset is offered only for a regular file** (T6 review N3): a symlink or any other non-file
//! at `vault.bin` is reported as `Denied` and never reset; the button stays hidden until the
//! blocking pool says the path may be reset.

// ## m2b-t21-prompts

use std::rc::Rc;
use std::sync::Arc;

use hacivat_l10n::{MsgRef, NoArgs, Text, ids};
use hacivat_proto::EnvTag;
use hacivat_secrets::{BackendChoice, SecretError, SecretStore};
use hacivat_session::LiveState;
use hacivat_session::pipeline::{Answer, Question};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::DialogSize;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::notice::{InlineNotice, NoticeIcon};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Context, Div, Entity, FocusHandle, Focusable, Global, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, Role, SharedString, Styled, Subscription,
    Task, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use tokio::sync::oneshot;

use crate::connections::ConnectionsModel;
use crate::connections::prompt_kit::{
    self, EnterGate, Part, alert_line, card, clear_all, cycle_focus, footer, header, inset,
    is_empty, meta_item, meta_line, plain_input, register_part, remove_modal, row, secret_input,
    static_value, subtitle, take_secret, trapped_tab, under_field,
};
use crate::l10n_global::tr;
use crate::shell::{AppState, Workspace};

/// The create and unlock dialogs' layer ID.
pub const VAULT_LAYER_ID: &str = "vault-dialog";
/// The reset confirmation's layer ID.
pub const RESET_LAYER_ID: &str = "vault-reset";
/// The damaged-file notice's layer ID.
pub const DAMAGED_LAYER_ID: &str = "vault-damaged";

/// The shortest master password, in characters (B13; the vault counts the same way).
const MIN_MASTER_CHARS: usize = 8;
/// Unlock attempts per gesture (B13).
const UNLOCK_ATTEMPTS: u8 = 3;
/// The width of the masked fields.
const FIELD_PX: f32 = 240.0;

/// Whether a `Forever` put may go ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum VaultReady {
    /// The keyring holds `Forever` secrets.
    NotNeeded,
    /// The vault exists and is unlocked.
    Ready,
    /// The user declined to create or unlock the vault.
    Declined,
}

// ---- the Remember label ---------------------------------------------------------------------

/// Whether the vault holds `Forever` secrets (the probe chose it). Never blocks: the choice is
/// read from the probe's cache, `None` (not the vault) until the probe ran.
fn uses_vault(cx: &App) -> bool {
    cx.try_global::<ConnectionsModel>()
        .is_some_and(|m| m.secrets().backend_choice() == Some(BackendChoice::Vault))
}

/// The label of the `Forever` retention: "Forever (encrypted file)" when the vault holds the
/// secrets, else the keyring's (M2a).
pub(crate) fn forever_label(cx: &App) -> MsgRef<NoArgs> {
    if uses_vault(cx) {
        ids::hv::CONN_RET_FOREVER_VAULT
    } else {
        ids::hv::CONN_RET_FOREVER
    }
}

/// The hint under the `Forever` retention, as [`forever_label`].
pub(crate) fn forever_hint(cx: &App) -> MsgRef<NoArgs> {
    if uses_vault(cx) {
        ids::hv::CONN_RET_FOREVER_VAULT_H
    } else {
        ids::hv::CONN_RET_FOREVER_H
    }
}

/// The `Forever` option's label and hint as texts re-read on every render, so an open select
/// follows the probe's choice (and a language switch).
pub(crate) fn forever_texts() -> (TextFn, TextFn) {
    (
        Rc::new(|cx: &App| tr(cx, forever_label(cx), NoArgs)),
        Rc::new(|cx: &App| tr(cx, forever_hint(cx), NoArgs)),
    )
}

// ---- the vault's state (blocking pool) --------------------------------------------------------

/// The vault as the blocking pool found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum VaultState {
    /// The keyring (or no backend) holds `Forever` secrets.
    NotVault,
    Absent,
    Locked,
    Unlocked,
}

impl VaultState {
    /// Whether a `Forever` put must ask the user first (create or unlock).
    pub(crate) fn needs_dialog(self) -> bool {
        matches!(self, VaultState::Absent | VaultState::Locked)
    }
}

/// Probes the backend (once; the store caches it) and reads the vault's state. Blocking: it may
/// wait for a whole Argon2 run and does file I/O (T6 review N1), so only the core runtime's
/// blocking pool calls it.
pub(crate) fn vault_state(secrets: &SecretStore) -> VaultState {
    // The first call probes the keyring and settles the backend's choice.
    let _ = secrets.keyring_available();
    match secrets.vault() {
        None => VaultState::NotVault,
        Some(v) if !v.exists() => VaultState::Absent,
        Some(v) if !v.is_unlocked() => VaultState::Locked,
        Some(_) => VaultState::Unlocked,
    }
}

/// [`vault_state`] on the core runtime's blocking pool.
fn probe(cx: &App) -> oneshot::Receiver<VaultState> {
    let (tx, rx) = oneshot::channel();
    let Some(model) = cx.try_global::<ConnectionsModel>() else {
        let _ = tx.send(VaultState::NotVault);
        return rx;
    };
    let secrets = model.secrets().clone();
    model.runtime().spawn_blocking(move || {
        let _ = tx.send(vault_state(&secrets));
    });
    rx
}

/// Runs `f` on the vault, on the core runtime's blocking pool.
fn on_vault<T: Send + 'static>(
    cx: &App,
    f: impl FnOnce(&dyn hacivat_secrets::VaultBackend) -> Result<T, SecretError> + Send + 'static,
) -> oneshot::Receiver<Result<T, SecretError>> {
    let (tx, rx) = oneshot::channel();
    match cx.try_global::<ConnectionsModel>() {
        Some(model) => {
            let secrets: Arc<SecretStore> = model.secrets().clone();
            model.runtime().spawn_blocking(move || {
                let out = match secrets.vault() {
                    Some(vault) => f(vault),
                    None => Err(SecretError::Unavailable),
                };
                let _ = tx.send(out);
            });
        }
        None => {
            let _ = tx.send(Err(SecretError::Unavailable));
        }
    }
    rx
}

/// Whether the vault path may be reset (a regular file, or nothing), on the blocking pool.
fn resettable(cx: &App) -> oneshot::Receiver<Result<bool, SecretError>> {
    on_vault(cx, |vault| Ok(vault.resettable()))
}

// ---- who asks -------------------------------------------------------------------------------

/// The connection a vault dialog names: its name and tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Asker {
    name: String,
    env: Option<EnvTag>,
}

/// The connection behind a pipeline's vault question: the one profile that is connecting, if
/// exactly one is (the question names none).
fn connecting(cx: &App) -> Option<Asker> {
    let m = cx.try_global::<ConnectionsModel>()?;
    let mut connecting = m
        .profiles()
        .iter()
        .filter(|p| matches!(m.manager().state(p.id), LiveState::Connecting(_)));
    let p = connecting.next()?;
    if connecting.next().is_some() {
        return None;
    }
    Some(Asker {
        name: p.name.clone(),
        env: p.environment,
    })
}

/// The subtitle naming `asker`.
fn asker_subtitle(theme: &Theme, asker: Option<&Asker>) -> Option<Div> {
    let asker = asker?;
    Some(subtitle(
        theme,
        asker.env,
        vec![Part::Mono(asker.name.clone().into())],
    ))
}

// ---- how a dialog answers ----------------------------------------------------------------------

/// Where a create or unlock dialog's outcome goes.
enum Mode {
    /// The pipeline asked: the master password goes back on the reply; the pipeline acts.
    Asked(Option<oneshot::Sender<Answer>>),
    /// Hacivat needs the vault: the dialog acts here and reports whether the vault is ready.
    Driven(Option<oneshot::Sender<bool>>),
}

impl Mode {
    fn finish(&mut self, ready: bool) {
        match self {
            Mode::Asked(reply) => {
                if let Some(reply) = reply.take() {
                    // Only a cancel finishes an asked dialog this way.
                    let _ = reply.send(Answer::Cancelled);
                }
            }
            Mode::Driven(done) => {
                if let Some(done) = done.take() {
                    let _ = done.send(ready);
                }
            }
        }
    }

    fn open(&self) -> bool {
        match self {
            Mode::Asked(reply) => reply.is_some(),
            Mode::Driven(done) => done.is_some(),
        }
    }
}

/// Closes the layer of the view `cx` renders.
fn close_self<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    let view = cx.entity_id();
    window.defer(cx, move |window, cx| remove_modal(view, window, cx));
}

/// A toast with `msg`.
fn toast(cx: &mut App, msg: MsgRef<NoArgs>) {
    let text = Rc::new(move |cx: &App| tr(cx, msg, NoArgs));
    ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
}

/// A field in a 240 px cell with a debug selector.
fn field_cell(input: &Entity<TextInput>, selector: &'static str) -> Div {
    div()
        .debug_selector(move || selector.to_string())
        .w(px(FIELD_PX))
        .max_w_full()
        .child(input.clone())
}

// ---- Create ---------------------------------------------------------------------------------

/// The create dialog.
pub struct VaultCreate {
    mode: Mode,
    asker: Option<Asker>,
    master: Entity<TextInput>,
    repeat: Entity<TextInput>,
    root: FocusHandle,
    not_now: FocusHandle,
    create: FocusHandle,
    busy: bool,
    enter: EnterGate,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for VaultCreate {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

/// Whether the create rules hold: at least 8 characters, and both entries equal.
fn create_rules(master: &str, repeat: &str) -> (bool, Option<bool>) {
    let long_enough = master.chars().count() >= MIN_MASTER_CHARS;
    let matches = (!master.is_empty() && !repeat.is_empty()).then(|| master == repeat);
    (long_enough, matches)
}

impl VaultCreate {
    fn new(mode: Mode, asker: Option<Asker>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let master = secret_input(cx);
        let repeat = secret_input(cx);
        let subscriptions = vec![
            cx.subscribe_in(
                &master,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submitted => {
                        if this.enter.take() {
                            let next = this.repeat.read(cx).focus_handle(cx);
                            window.focus(&next, cx);
                        }
                    }
                    InputEvent::Cancelled => this.not_now(window, cx),
                    InputEvent::Changed => cx.notify(),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &repeat,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submitted => {
                        if this.enter.take() {
                            this.create(window, cx);
                        }
                    }
                    InputEvent::Cancelled => this.not_now(window, cx),
                    InputEvent::Changed => cx.notify(),
                    _ => {}
                },
            ),
        ];
        VaultCreate {
            mode,
            asker,
            master,
            repeat,
            root: cx.focus_handle(),
            not_now: cx.focus_handle().tab_stop(true),
            create: cx.focus_handle().tab_stop(true),
            busy: false,
            enter: EnterGate::default(),
            _subscriptions: subscriptions,
        }
    }

    fn ok(&self, cx: &App) -> bool {
        let (long_enough, matches) =
            create_rules(self.master.read(cx).value(), self.repeat.read(cx).value());
        long_enough && matches == Some(true)
    }

    /// Not now (also Escape and the close button): nothing is created; a `Forever` secret is
    /// kept until restart (Q38).
    fn not_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        clear_all(&[self.master.clone(), self.repeat.clone()], cx);
        self.mode.finish(false);
        close_self(window, cx);
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.mode.open() || !self.ok(cx) {
            return;
        }
        let Some(master) = take_secret(&self.master, cx) else {
            return;
        };
        clear_all(std::slice::from_ref(&self.repeat), cx);
        match &mut self.mode {
            Mode::Asked(reply) => {
                if let Some(reply) = reply.take() {
                    let _ = reply.send(Answer::Master(master));
                }
                close_self(window, cx);
            }
            Mode::Driven(_) => {
                self.busy = true;
                cx.notify();
                let created = on_vault(cx, move |vault| vault.create(master));
                cx.spawn_in(window, async move |this, cx| {
                    let created = matches!(created.await, Ok(Ok(())));
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.busy = false;
                        if created {
                            toast(cx, ids::hv::CONN_VAULT_CREATED);
                        } else {
                            tracing::warn!("connections: the vault could not be created");
                        }
                        this.mode.finish(created);
                        close_self(window, cx);
                    });
                })
                .detach();
            }
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.not_now(window, cx),
            "enter" if prompt_kit::acting_enter(event) => {
                if self.not_now.is_focused(window) {
                    self.not_now(window, cx);
                } else if self.create.is_focused(window) {
                    self.create(window, cx);
                }
            }
            "enter" => {}
            _ if trapped_tab(event) => {
                let mut order = vec![
                    self.master.read(cx).focus_handle(cx),
                    self.repeat.read(cx).focus_handle(cx),
                    self.not_now.clone(),
                ];
                if self.ok(cx) {
                    order.push(self.create.clone());
                }
                cycle_focus(&order, event.keystroke.modifiers.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// The live rules (NOTES "Validation lines").
    fn rules(&self, theme: &Theme, cx: &App) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        let (long_enough, matches) =
            create_rules(self.master.read(cx).value(), self.repeat.read(cx).value());
        let hollow = || {
            div()
                .flex_none()
                .size(px(7.0))
                .mx(px(2.5))
                .border_1()
                .border_color(c.seam_strong.to_hsla())
                .into_any_element()
        };
        let glyph = |name, ink: hacivat_ui_core::gpui_surface::Hsla| {
            Icon::new(name)
                .size(IconSize::Chrome)
                .color(ink)
                .into_any_element()
        };
        let line = |mark, text: SharedString, ink, id: &'static str| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.sm))
                .child(mark)
                .child(Label::new(text, TextSource::Fluent, t.type_ramp.caption, ink).id(id))
        };
        let min = line(
            if long_enough {
                glyph(IconName::Check, c.green.to_hsla())
            } else {
                hollow()
            },
            tr(cx, ids::hv::CONN_VAULT_MIN, NoArgs),
            c.text_2.to_hsla(),
            "vault-rule-min",
        );
        let matching = matches.map(|matches| {
            if matches {
                line(
                    glyph(IconName::Check, c.green.to_hsla()),
                    tr(cx, ids::hv::CONN_VAULT_MATCH, NoArgs),
                    c.text_2.to_hsla(),
                    "vault-rule-match",
                )
            } else {
                line(
                    glyph(IconName::X, c.text.to_hsla()),
                    tr(cx, ids::hv::CONN_VAULT_MISMATCH, NoArgs),
                    c.text.to_hsla(),
                    "vault-rule-match",
                )
            }
        });
        div()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(min)
            .children(matching)
    }
}

impl Render for VaultCreate {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let title = tr(cx, ids::hv::CONN_VAULT_CREATE_TITLE, NoArgs);
        let why = tr(cx, ids::hv::CONN_VAULT_CREATE_WHY, NoArgs);
        let file = cx
            .try_global::<AppState>()
            .map(|s| tilde(&s.paths.vault_bin()))
            .unwrap_or_default();
        let ok = self.ok(cx);
        let not_now = Button::new("vault-not-now", tr(cx, ids::hv::CONN_VAULT_NOTNOW, NoArgs))
            .kind(ButtonKind::Ghost)
            .focus_handle(self.not_now.clone())
            .on_click(cx.listener(|this, _, window, cx| this.not_now(window, cx)));
        let create = Button::new(
            "vault-create",
            tr(cx, ids::hv::CONN_VAULT_CREATE_BTN, NoArgs),
        )
        .kind(ButtonKind::Primary)
        .disabled(!ok || self.busy)
        .focus_handle(self.create.clone())
        .on_click(cx.listener(|this, _, window, cx| this.create(window, cx)));
        let card = card(&theme, DialogSize::Wide, VAULT_LAYER_ID)
            .track_focus(&self.root)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, _| {
                this.enter.note(event);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .child(header(
                &theme,
                title.clone(),
                "vault-create-title",
                asker_subtitle(&theme, self.asker.as_ref()),
                cx.listener(|this, _, window, cx| this.not_now(window, cx)),
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.xs))
                    .child(
                        paragraph(
                            why.clone(),
                            TextSource::Fluent,
                            t.type_ramp.body,
                            c.text.to_hsla(),
                        )
                        .id("vault-create-why"),
                    )
                    .child(
                        paragraph(
                            tr(cx, ids::hv::CONN_VAULT_CREATE_WHAT, NoArgs),
                            TextSource::Fluent,
                            t.type_ramp.body,
                            c.text_2.to_hsla(),
                        )
                        .id("vault-create-what"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.md))
                    .child(row(
                        &theme,
                        tr(cx, ids::hv::CONN_KV_FILE, NoArgs),
                        static_value(&theme, file, "vault-file"),
                    ))
                    .child(row(
                        &theme,
                        tr(cx, ids::hv::CONN_VAULT_MASTER, NoArgs),
                        field_cell(&self.master, "vault-new"),
                    ))
                    .child(row(
                        &theme,
                        tr(cx, ids::hv::CONN_VAULT_REPEAT, NoArgs),
                        field_cell(&self.repeat, "vault-repeat"),
                    ))
                    .child(under_field(&theme, self.rules(&theme, cx))),
            )
            .child(InlineNotice::new(NoticeIcon::Info).line(tr(
                cx,
                ids::hv::CONN_VAULT_NORECOVER,
                NoArgs,
            )))
            .child(
                paragraph(
                    tr(cx, ids::hv::CONN_VAULT_NOTNOW_NOTE, NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                )
                .id("vault-notnow-note"),
            )
            .child(footer(
                &theme,
                Vec::new(),
                vec![not_now.into_any_element(), create.into_any_element()],
            ));
        A11yNode::new("vault-create-node", Role::Dialog, title, true, card).description(why)
    }
}

/// `path` with the home directory written as `~`.
fn tilde(path: &std::path::Path) -> String {
    match std::env::home_dir()
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) => format!("~/{}", rest.to_string_lossy()),
        None => path.to_string_lossy().into_owned(),
    }
}

// ---- Unlock ---------------------------------------------------------------------------------

/// The unlock dialog.
pub struct VaultUnlock {
    mode: Mode,
    asker: Option<Asker>,
    field: Entity<TextInput>,
    /// The attempt this dialog asks for, from 1.
    attempt: u8,
    /// Whether the vault path may be reset (known only once the blocking pool said so).
    resettable: bool,
    busy: bool,
    root: FocusHandle,
    reset: FocusHandle,
    cancel: FocusHandle,
    unlock: FocusHandle,
    enter: EnterGate,
    _subscription: Subscription,
}

impl Focusable for VaultUnlock {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl VaultUnlock {
    fn new(
        mode: Mode,
        asker: Option<Asker>,
        attempt: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = secret_input(cx);
        let subscription = cx.subscribe_in(
            &field,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => {
                    if this.enter.take() {
                        this.unlock(window, cx);
                    }
                }
                InputEvent::Cancelled => this.cancel(window, cx),
                InputEvent::Changed => cx.notify(),
                _ => {}
            },
        );
        VaultUnlock {
            mode,
            asker,
            field,
            attempt: attempt.max(1),
            resettable: false,
            busy: false,
            root: cx.focus_handle(),
            reset: cx.focus_handle().tab_stop(true),
            cancel: cx.focus_handle().tab_stop(true),
            unlock: cx.focus_handle().tab_stop(true),
            enter: EnterGate::default(),
            _subscription: subscription,
        }
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        clear_all(std::slice::from_ref(&self.field), cx);
        self.mode.finish(false);
        close_self(window, cx);
    }

    fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.mode.open() {
            return;
        }
        let Some(master) = take_secret(&self.field, cx) else {
            return;
        };
        match &mut self.mode {
            Mode::Asked(reply) => {
                if let Some(reply) = reply.take() {
                    let _ = reply.send(Answer::Master(master));
                }
                close_self(window, cx);
            }
            Mode::Driven(_) => {
                self.busy = true;
                cx.notify();
                let unlocked = on_vault(cx, move |vault| vault.unlock(master));
                cx.spawn_in(window, async move |this, cx| {
                    let unlocked = unlocked.await.unwrap_or(Err(SecretError::Unavailable));
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.busy = false;
                        this.unlocked(unlocked, window, cx);
                    });
                })
                .detach();
            }
        }
    }

    /// What a driven unlock found: unlocked (done), a wrong password (the next attempt, at most
    /// three), a damaged file (the damaged notice) or another error (done, not ready).
    fn unlocked(
        &mut self,
        result: Result<(), SecretError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(()) => {
                self.mode.finish(true);
                close_self(window, cx);
            }
            Err(SecretError::WrongMasterPassword) if self.attempt < UNLOCK_ATTEMPTS => {
                self.attempt += 1;
                let field = self.field.read(cx).focus_handle(cx);
                window.focus(&field, cx);
                cx.notify();
            }
            Err(SecretError::VaultCorrupt) => {
                self.mode.finish(false);
                close_self(window, cx);
                let resettable = self.resettable;
                window.defer(cx, move |window, cx| {
                    open_damaged(resettable, window, cx);
                });
            }
            Err(e) => {
                tracing::debug!(error = ?e, "connections: the vault was not unlocked");
                self.mode.finish(false);
                close_self(window, cx);
            }
        }
    }

    /// Reset vault…: stacks the reset confirmation; a reset answers this dialog (`Cancelled`
    /// to the pipeline, not ready to Hacivat) and closes it.
    fn open_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.resettable || self.busy {
            return;
        }
        let me = cx.entity().downgrade();
        open_reset(
            Box::new(move |window, cx| {
                if let Some(me) = me.upgrade() {
                    me.update(cx, |this, cx| {
                        clear_all(std::slice::from_ref(&this.field), cx);
                        this.mode.finish(false);
                        let view = cx.entity_id();
                        window.defer(cx, move |window, cx| remove_modal(view, window, cx));
                    });
                }
            }),
            window,
            cx,
        );
    }

    fn order(&self, cx: &App) -> Vec<FocusHandle> {
        let mut order = vec![self.field.read(cx).focus_handle(cx)];
        if self.resettable {
            order.push(self.reset.clone());
        }
        order.push(self.cancel.clone());
        if !is_empty(&self.field, cx) {
            order.push(self.unlock.clone());
        }
        order
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.cancel(window, cx),
            "enter" if prompt_kit::acting_enter(event) => {
                if self.reset.is_focused(window) {
                    self.open_reset(window, cx);
                } else if self.cancel.is_focused(window) {
                    self.cancel(window, cx);
                } else if self.unlock.is_focused(window) {
                    self.unlock(window, cx);
                }
            }
            "enter" => {}
            _ if trapped_tab(event) => {
                let order = self.order(cx);
                cycle_focus(&order, event.keystroke.modifiers.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for VaultUnlock {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let title = tr(cx, ids::hv::CONN_VAULT_UNLOCK_TITLE, NoArgs);
        let body = match &self.asker {
            Some(asker) => tr(
                cx,
                ids::hv::CONN_VAULT_UNLOCK_BODY,
                ids::hv::HvConnVaultUnlockBody {
                    name: Text(asker.name.clone()),
                },
            ),
            None => tr(cx, ids::hv::CONN_VAULT_UNLOCK_BODY_ANY, NoArgs),
        };
        let mut rows = div().flex().flex_col().gap(px(t.spacing.md)).child(row(
            &theme,
            tr(cx, ids::hv::CONN_VAULT_MASTER, NoArgs),
            field_cell(&self.field, "vault-unlock"),
        ));
        if self.attempt > 1 {
            rows = rows.child(under_field(
                &theme,
                meta_line(&theme)
                    .child(alert_line(
                        &theme,
                        tr(cx, ids::hv::CONN_VAULT_UNLOCK_WRONG, NoArgs),
                        "vault-unlock-wrong",
                    ))
                    .child(meta_item(
                        &theme,
                        tr(
                            cx,
                            ids::hv::CONN_VAULT_ATTEMPT,
                            ids::hv::HvConnVaultAttempt {
                                n: prompt_kit::count(usize::from(self.attempt)),
                            },
                        ),
                        "vault-attempt",
                    )),
            ));
        }
        let reset = self.resettable.then(|| {
            Button::new(
                "vault-reset-open",
                tr(cx, ids::hv::CONN_VAULT_RESET_LINK, NoArgs),
            )
            .kind(ButtonKind::Ghost)
            .disabled(self.busy)
            .focus_handle(self.reset.clone())
            .on_click(cx.listener(|this, _, window, cx| this.open_reset(window, cx)))
            .into_any_element()
        });
        let cancel = Button::new("vault-cancel", tr(cx, ids::hv::CONN_PW_CANCEL, NoArgs))
            .kind(ButtonKind::Ghost)
            .focus_handle(self.cancel.clone())
            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)));
        let unlock = Button::new(
            "vault-unlock-btn",
            tr(cx, ids::hv::CONN_VAULT_UNLOCK_BTN, NoArgs),
        )
        .kind(ButtonKind::Primary)
        .disabled(self.busy || is_empty(&self.field, cx))
        .focus_handle(self.unlock.clone())
        .on_click(cx.listener(|this, _, window, cx| this.unlock(window, cx)));
        let card = card(&theme, DialogSize::Wide, VAULT_LAYER_ID)
            .track_focus(&self.root)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, _| {
                this.enter.note(event);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .child(header(
                &theme,
                title.clone(),
                "vault-unlock-title",
                asker_subtitle(&theme, self.asker.as_ref()),
                cx.listener(|this, _, window, cx| this.cancel(window, cx)),
                cx,
            ))
            .child(
                paragraph(
                    body.clone(),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )
                .id("vault-unlock-body"),
            )
            .child(rows)
            .child(footer(
                &theme,
                reset.into_iter().collect(),
                vec![cancel.into_any_element(), unlock.into_any_element()],
            ));
        A11yNode::new("vault-unlock-node", Role::Dialog, title, true, card).description(body)
    }
}

// ---- Reset ----------------------------------------------------------------------------------

/// What runs once the vault was reset (it answers and closes the dialog under the reset).
type OnReset = Box<dyn FnOnce(&mut Window, &mut App)>;

/// The reset confirmation.
pub struct VaultReset {
    word: Entity<TextInput>,
    on_reset: Option<OnReset>,
    busy: bool,
    root: FocusHandle,
    cancel: FocusHandle,
    confirm: FocusHandle,
    _subscription: Subscription,
}

impl Focusable for VaultReset {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl VaultReset {
    /// Whether the field equals the word exactly (no casing, no trimming: B13).
    fn confirmed(&self, cx: &App) -> bool {
        self.word.read(cx).value() == tr(cx, ids::hv::CONN_VAULT_RESET_WORD, NoArgs).as_ref()
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        close_self(window, cx);
    }

    /// Deletes the vault on the core runtime, then answers the dialog under it.
    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.confirmed(cx) {
            return;
        }
        self.busy = true;
        cx.notify();
        let reset = on_vault(cx, |vault| vault.reset());
        cx.spawn_in(window, async move |this, cx| {
            let reset = matches!(reset.await, Ok(Ok(())));
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                close_self(window, cx);
                if reset {
                    toast(cx, ids::hv::CONN_VAULT_RESET_DONE);
                    if let Some(on_reset) = this.on_reset.take() {
                        window.defer(cx, on_reset);
                    }
                } else {
                    tracing::warn!("connections: the vault could not be reset");
                }
            });
        })
        .detach();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.cancel(window, cx),
            // Enter never resets (B13): it presses Cancel when Cancel has the focus, and
            // nothing anywhere else.
            "enter" => {
                if prompt_kit::acting_enter(event) && self.cancel.is_focused(window) {
                    self.cancel(window, cx);
                }
            }
            _ if trapped_tab(event) => {
                let mut order = vec![self.word.read(cx).focus_handle(cx), self.cancel.clone()];
                if self.confirmed(cx) {
                    order.push(self.confirm.clone());
                }
                cycle_focus(&order, event.keystroke.modifiers.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for VaultReset {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let title = tr(cx, ids::hv::CONN_VAULT_RESET_TITLE, NoArgs);
        let body = tr(cx, ids::hv::CONN_VAULT_RESET_BODY, NoArgs);
        let word = tr(cx, ids::hv::CONN_VAULT_RESET_WORD, NoArgs);
        let confirmed = self.confirmed(cx);
        let cancel = Button::new(
            "vault-reset-cancel",
            tr(cx, ids::hv::CONN_PW_CANCEL, NoArgs),
        )
        .kind(ButtonKind::Ghost)
        .focus_handle(self.cancel.clone())
        .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)));
        let confirm = Button::new(
            "vault-reset-confirm",
            tr(cx, ids::hv::CONN_VAULT_RESET_BTN, NoArgs),
        )
        .kind(ButtonKind::Danger)
        .icon(IconName::Trash2)
        .disabled(!confirmed || self.busy)
        .focus_handle(self.confirm.clone())
        .on_click(cx.listener(|this, _, window, cx| this.reset(window, cx)));
        let card = card(&theme, DialogSize::Standard, RESET_LAYER_ID)
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .child(header(
                &theme,
                title.clone(),
                "vault-reset-title",
                None,
                cx.listener(|this, _, window, cx| this.cancel(window, cx)),
                cx,
            ))
            .child(
                paragraph(
                    body.clone(),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )
                .id("vault-reset-body"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.xs))
                    .child(
                        paragraph(
                            tr(
                                cx,
                                ids::hv::CONN_VAULT_RESET_TYPE,
                                ids::hv::HvConnVaultResetType {
                                    word: Text(word.to_string()),
                                },
                            ),
                            TextSource::Fluent,
                            t.type_ramp.body,
                            c.text_2.to_hsla(),
                        )
                        .id("vault-reset-type"),
                    )
                    .child(
                        div()
                            .debug_selector(|| "vault-reset-word".to_string())
                            .w_full()
                            .child(self.word.clone()),
                    ),
            )
            .child(footer(
                &theme,
                Vec::new(),
                vec![cancel.into_any_element(), confirm.into_any_element()],
            ));
        A11yNode::new("vault-reset-node", Role::AlertDialog, title, true, card).description(body)
    }
}

/// Stacks the reset confirmation over the top layer and focuses its field; `on_reset` runs
/// once the vault was reset.
fn open_reset(on_reset: OnReset, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| {
        let word = plain_input(cx, true);
        let subscription = cx.subscribe_in(
            &word,
            window,
            |this: &mut VaultReset, _, event: &InputEvent, window, cx| match event {
                InputEvent::Changed => cx.notify(),
                // A focused field swallows Escape (P16): back to the dialog under it.
                InputEvent::Cancelled => this.cancel(window, cx),
                // Enter in the field does nothing (B13).
                _ => {}
            },
        );
        VaultReset {
            word,
            on_reset: Some(on_reset),
            busy: false,
            root: cx.focus_handle(),
            cancel: cx.focus_handle().tab_stop(true),
            confirm: cx.focus_handle().tab_stop(true),
            _subscription: subscription,
        }
    });
    let (field, input, cancel, confirm) = {
        let v = view.read(cx);
        (
            v.word.read(cx).focus_handle(cx),
            v.word.clone(),
            v.cancel.clone(),
            v.confirm.clone(),
        )
    };
    register_part(cx, "vault-reset-word", &field, Some(&input));
    register_part(cx, "vault-reset-cancel", &cancel, None);
    register_part(cx, "vault-reset-confirm", &confirm, None);
    let _ = prompt_kit::push_modal(RESET_LAYER_ID, view.into(), &field, window, cx);
}

// ---- Damaged --------------------------------------------------------------------------------

/// The damaged-file notice.
pub struct VaultDamaged {
    resettable: bool,
    root: FocusHandle,
    reset: FocusHandle,
    close: FocusHandle,
}

impl Focusable for VaultDamaged {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl VaultDamaged {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        close_self(window, cx);
    }

    fn open_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.resettable {
            return;
        }
        let me = cx.entity().downgrade();
        open_reset(
            Box::new(move |window, cx| {
                if let Some(me) = me.upgrade() {
                    let view = me.entity_id();
                    remove_modal(view, window, cx);
                }
            }),
            window,
            cx,
        );
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.close(window, cx),
            "enter" if prompt_kit::acting_enter(event) => {
                if self.reset.is_focused(window) {
                    self.open_reset(window, cx);
                } else {
                    self.close(window, cx);
                }
            }
            "enter" => {}
            _ if trapped_tab(event) => {
                let mut order = Vec::new();
                if self.resettable {
                    order.push(self.reset.clone());
                }
                order.push(self.close.clone());
                cycle_focus(&order, event.keystroke.modifiers.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for VaultDamaged {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let title = tr(cx, ids::hv::CONN_VAULT_DAMAGED_TITLE, NoArgs);
        let body = tr(cx, ids::hv::CONN_VAULT_DAMAGED_BODY, NoArgs);
        let file = cx
            .try_global::<AppState>()
            .map(|s| tilde(&s.paths.vault_bin()))
            .unwrap_or_default();
        let reset = self.resettable.then(|| {
            Button::new(
                "vault-damaged-reset",
                tr(cx, ids::hv::CONN_VAULT_RESET_LINK, NoArgs),
            )
            .kind(ButtonKind::Default)
            .focus_handle(self.reset.clone())
            .on_click(cx.listener(|this, _, window, cx| this.open_reset(window, cx)))
            .into_any_element()
        });
        let close = Button::new(
            "vault-damaged-close",
            tr(cx, ids::hv::CONN_FIX_CLOSE, NoArgs),
        )
        .kind(ButtonKind::Primary)
        .focus_handle(self.close.clone())
        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx)));
        let card = card(&theme, DialogSize::Wide, DAMAGED_LAYER_ID)
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .child(header(
                &theme,
                title.clone(),
                "vault-damaged-title",
                None,
                cx.listener(|this, _, window, cx| this.close(window, cx)),
                cx,
            ))
            .child(
                paragraph(
                    body.clone(),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )
                .id("vault-damaged-body"),
            )
            .child(inset(&theme).child(static_value(&theme, file, "vault-damaged-file")))
            .child(
                paragraph(
                    tr(cx, ids::hv::CONN_VAULT_DAMAGED_MEANWHILE, NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                )
                .id("vault-damaged-meanwhile"),
            )
            .child(footer(
                &theme,
                reset.into_iter().collect(),
                vec![close.into_any_element()],
            ));
        A11yNode::new("vault-damaged-node", Role::AlertDialog, title, true, card).description(body)
    }
}

/// Opens the damaged-file notice (Close has the focus).
fn open_damaged(resettable: bool, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| VaultDamaged {
        resettable,
        root: cx.focus_handle(),
        reset: cx.focus_handle().tab_stop(true),
        close: cx.focus_handle().tab_stop(true),
    });
    let (reset, close) = {
        let v = view.read(cx);
        (v.reset.clone(), v.close.clone())
    };
    register_part(cx, "vault-damaged-reset", &reset, None);
    register_part(cx, "vault-damaged-close", &close, None);
    let _ = prompt_kit::push_modal(DAMAGED_LAYER_ID, view.into(), &close, window, cx);
}

// ---- opening the dialogs ----------------------------------------------------------------------

/// Opens the create dialog in `window` (focus in Master password).
fn open_create(
    mode: Mode,
    asker: Option<Asker>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<VaultCreate>> {
    let view = cx.new(|cx| VaultCreate::new(mode, asker, window, cx));
    let (master, master_input, repeat, repeat_input, not_now, create) = {
        let v = view.read(cx);
        (
            v.master.read(cx).focus_handle(cx),
            v.master.clone(),
            v.repeat.read(cx).focus_handle(cx),
            v.repeat.clone(),
            v.not_now.clone(),
            v.create.clone(),
        )
    };
    register_part(cx, "vault-new", &master, Some(&master_input));
    register_part(cx, "vault-repeat", &repeat, Some(&repeat_input));
    register_part(cx, "vault-not-now", &not_now, None);
    register_part(cx, "vault-create", &create, None);
    prompt_kit::push_modal(VAULT_LAYER_ID, view.clone().into(), &master, window, cx).then_some(view)
}

/// Opens the unlock dialog in `window` (focus in the field) and asks the blocking pool whether
/// a reset may be offered.
fn open_unlock(
    mode: Mode,
    asker: Option<Asker>,
    attempt: u8,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<VaultUnlock>> {
    let view = cx.new(|cx| VaultUnlock::new(mode, asker, attempt, window, cx));
    let (field, input, reset, cancel, unlock) = {
        let v = view.read(cx);
        (
            v.field.read(cx).focus_handle(cx),
            v.field.clone(),
            v.reset.clone(),
            v.cancel.clone(),
            v.unlock.clone(),
        )
    };
    register_part(cx, "vault-unlock", &field, Some(&input));
    register_part(cx, "vault-reset-open", &reset, None);
    register_part(cx, "vault-cancel", &cancel, None);
    register_part(cx, "vault-unlock-btn", &unlock, None);
    if !prompt_kit::push_modal(VAULT_LAYER_ID, view.clone().into(), &field, window, cx) {
        return None;
    }
    let may_reset = resettable(cx);
    let weak = view.downgrade();
    window
        .spawn(cx, async move |cx| {
            let may_reset = matches!(may_reset.await, Ok(Ok(true)));
            let _ = weak.update(cx, |v, cx| {
                v.resettable = may_reset;
                prompt_kit::note_flag(cx, "vault-resettable", may_reset);
                cx.notify();
            });
        })
        .detach();
    Some(view)
}

/// Asks `q` (`Question::UnlockVault`, `Question::CreateVault`); the answer goes to `reply`.
/// Any other question, or a window without a workspace, answers `Cancelled`.
pub(crate) fn open(q: Question, reply: oneshot::Sender<Answer>, window: &mut Window, cx: &mut App) {
    let asker = connecting(cx);
    let (answer_tx, answer_rx) = oneshot::channel();
    let view = match q {
        Question::CreateVault => {
            open_create(Mode::Asked(Some(answer_tx)), asker, window, cx).map(|v| v.entity_id())
        }
        Question::UnlockVault { attempt } => {
            open_unlock(Mode::Asked(Some(answer_tx)), asker, attempt, window, cx)
                .map(|v| v.entity_id())
        }
        _ => {
            let _ = reply.send(Answer::Cancelled);
            return;
        }
    };
    if let Some(view) = view {
        prompt_kit::forward(reply, answer_rx, window.window_handle(), view, cx);
    }
}

// ---- Hacivat's own gestures --------------------------------------------------------------------

/// The vault flows Hacivat runs itself: one at a time; a gesture that comes while one runs
/// waits for its outcome instead of opening a second dialog.
#[derive(Default)]
struct VaultFlows {
    running: bool,
    waiting: Vec<oneshot::Sender<bool>>,
}

impl Global for VaultFlows {}

/// Which dialog a flow opens.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Flow {
    Create(Option<Asker>),
    Unlock(Option<Asker>),
}

/// The workspace window (the active one, else the first).
fn workspace_window(cx: &App) -> Option<hacivat_ui_core::gpui_surface::AnyWindowHandle> {
    cx.active_window()
        .into_iter()
        .chain(cx.windows())
        .find(|w| w.downcast::<Workspace>().is_some())
}

/// Runs `flow` (or joins the one running); resolves with whether the vault is ready.
fn run_flow(flow: Flow, cx: &mut App) -> oneshot::Receiver<bool> {
    let (tx, rx) = oneshot::channel();
    let flows = cx.default_global::<VaultFlows>();
    flows.waiting.push(tx);
    if flows.running {
        return rx;
    }
    flows.running = true;
    let (done_tx, done_rx) = oneshot::channel();
    if let Some(window) = workspace_window(cx) {
        let _ = window.update(cx, move |_, window, cx| match flow {
            Flow::Create(asker) => {
                let _ = open_create(Mode::Driven(Some(done_tx)), asker, window, cx);
            }
            Flow::Unlock(asker) => {
                let _ = open_unlock(Mode::Driven(Some(done_tx)), asker, 1, window, cx);
            }
        });
    }
    cx.spawn(async move |cx| {
        // A dialog that could not open, or closed unanswered, is not ready.
        let ready = done_rx.await.unwrap_or(false);
        cx.update(|cx| {
            let flows = cx.default_global::<VaultFlows>();
            flows.running = false;
            for waiter in flows.waiting.drain(..) {
                let _ = waiter.send(ready);
            }
        });
    })
    .detach();
    rx
}

/// Makes sure a `Forever` put of the profile `name` (with tag `env`) has somewhere to go, given
/// the vault's `state` as the blocking pool found it ([`vault_state`]): a missing vault is
/// created first (Create, before the first Forever save) and a locked one unlocked (Unlock).
/// `NotNeeded` for the keyring; `Ready` once the vault is there and unlocked; `Declined` when
/// the user said Not now or cancelled, and the put then keeps the secret until restart with the
/// M2a toast (Q38). `ConnectionsModel::save` asks the blocking pool for the state inside its
/// queued job and calls this only when a dialog is due, so a save that needs no dialog never
/// waits for the UI thread (a save right before quit still drains, P15).
pub(crate) fn ensure_vault_before_forever(
    cx: &mut App,
    state: VaultState,
    name: String,
    env: Option<EnvTag>,
) -> Task<VaultReady> {
    let asker = Some(Asker { name, env });
    let flow = match state {
        VaultState::NotVault => return Task::ready(VaultReady::NotNeeded),
        VaultState::Unlocked => return Task::ready(VaultReady::Ready),
        VaultState::Absent => Flow::Create(asker),
        VaultState::Locked => Flow::Unlock(asker),
    };
    let ready = run_flow(flow, cx);
    cx.spawn(async move |_| {
        if ready.await.unwrap_or(false) {
            VaultReady::Ready
        } else {
            VaultReady::Declined
        }
    })
}

/// Unlocks the locked vault for a change to the profile `name` (a delete or a duplicate met
/// `VaultLocked`; T6 carry): the unlock dialog, three attempts. Resolves `true` once unlocked.
pub(crate) fn unlock_vault(cx: &mut App, name: String, env: Option<EnvTag>) -> Task<bool> {
    let found = probe(cx);
    let asker = Some(Asker { name, env });
    cx.spawn(
        async move |cx| match found.await.unwrap_or(VaultState::NotVault) {
            VaultState::Locked => {
                let ready = cx.update(|cx| run_flow(Flow::Unlock(asker), cx));
                ready.await.unwrap_or(false)
            }
            VaultState::Unlocked => true,
            VaultState::NotVault | VaultState::Absent => false,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::create_rules;

    #[test]
    fn the_create_rules_count_characters_and_compare_exactly() {
        assert_eq!(create_rules("", ""), (false, None));
        assert_eq!(create_rules("short", "short"), (false, Some(true)));
        assert_eq!(create_rules("correct horse", ""), (true, None));
        assert_eq!(
            create_rules("correct horse", "correct horsE"),
            (true, Some(false))
        );
        // Eight characters, more than eight bytes: the vault counts characters too.
        assert_eq!(create_rules("ğüşıöçİĞ", "ğüşıöçİĞ"), (true, Some(true)));
        assert!(!create_rules("ğüşıöçİ", "ğüşıöçİ").0);
    }
}
