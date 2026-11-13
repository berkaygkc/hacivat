//! The parts the SSH and vault prompts share (Task 21; NOTES surfaces 1–4, M2a surface 6): the
//! modal layer a prompt opens in, the forwarding of its answer to the asker, the dialog header
//! (title, environment mark, connection, hop), form rows with the 480 px dialogs' 120 px label
//! column, the fingerprint inset, masked fields, the route row and the "made visible" rendering
//! of server text (NOTES cross-cutting #3).
//!
//! **Keys (Q34).** Every prompt handles Enter on key down only, never held or with a modifier,
//! and suppresses the key-up Enter in the capture phase so a focused button's keyboard click
//! never presses it a second time ([`card`]). A field's `Submitted` reaches the prompt only for
//! such an Enter ([`EnterGate`]).
//!
//! **Secrets.** A masked field's value becomes a `SecretString` the moment it is read, and the
//! field is cleared then ([`take_secret`]); a prompt clears every field when it closes.

// ## m2b-t21-prompts

use std::fmt::Write as _;
use std::rc::Rc;

use hacivat_driver::EngineCatalog;
use hacivat_net::ssh::HopSpec;
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_secrets::Retention;
use hacivat_session::pipeline::Answer;
use hacivat_session::profile::{ProfileEndpoint, TransportSpec};
use hacivat_ui::capsule::env_swatch;
use hacivat_ui::dialog::{DialogSize, dialog_frame};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::TextInput;
use hacivat_ui::l10n::ui_text;
use hacivat_ui::popover::Declare;
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, AnyWindowHandle, App, AppContext as _, ClickEvent, Context, Div, Entity,
    EntityId, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Role, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, div,
    px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use secrecy::SecretString;
use tokio::sync::oneshot;

use crate::connections::ConnectionsModel;
use crate::connections::error_view::env_kind;
use crate::shell::Workspace;
use crate::shell::layers::{DIALOG_CONTEXT, Layer};

/// The label column of the 420 and 480 px dialogs (NOTES cross-cutting #5).
pub(crate) const LABEL_PX: f32 = 120.0;

/// The separator between the subtitle's parts, and the arrow between route stops (symbols).
const SEPARATOR: &str = "·";
const ROUTE_ARROW: &str = "→";
/// What a cut-off server text ends with.
const CUT: char = '…';

// ---- layers and answers ---------------------------------------------------------------------

/// Opens `view` as the modal layer `id` over `window`'s workspace and focuses `focus`. False
/// without a workspace: the caller's reply is then dropped, which answers `Cancelled`.
pub(crate) fn push_modal(
    id: &'static str,
    view: AnyView,
    focus: &FocusHandle,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return false;
    };
    ws.update(cx, |ws, cx| {
        ws.push_layer(
            Layer {
                id,
                context: DIALOG_CONTEXT,
                view,
                restore_focus: None,
                modal: true,
            },
            window,
            cx,
        );
    });
    window.focus(focus, cx);
    true
}

/// Removes the layer showing `view`, wherever it is in the stack (a reset stacked over the
/// unlock closes both).
pub(crate) fn remove_modal(view: EntityId, window: &mut Window, cx: &mut App) {
    if let Some(ws) = window.root::<Workspace>().flatten() {
        ws.update(cx, |ws, cx| ws.remove_layer(view, window, cx));
    }
}

/// Forwards the prompt's own answer (`answer`) to the asker (`reply`). When the asker goes away
/// first (the connect was cancelled, the profile forgotten), the orphaned prompt `view` closes.
/// A prompt that closes without answering drops its sender, and dropping `reply` answers
/// `Cancelled`.
pub(crate) fn forward(
    reply: oneshot::Sender<Answer>,
    answer: oneshot::Receiver<Answer>,
    window: AnyWindowHandle,
    view: EntityId,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let mut reply = reply;
        let answered = {
            let closed = reply.closed();
            futures::pin_mut!(closed);
            match futures::future::select(closed, answer).await {
                futures::future::Either::Left(_) => None,
                futures::future::Either::Right((answer, _)) => Some(answer),
            }
        };
        match answered {
            Some(Ok(answer)) => {
                let _ = reply.send(answer);
            }
            Some(Err(_)) => {}
            None => {
                cx.update(|cx| {
                    let _ = window.update(cx, |_, window, cx| remove_modal(view, window, cx));
                });
            }
        }
    })
    .detach();
}

// ---- keys -----------------------------------------------------------------------------------

/// Whether `event` is an Enter that may act: on key down, not held, without a modifier.
pub(crate) fn acting_enter(event: &KeyDownEvent) -> bool {
    event.keystroke.key == "enter" && !event.is_held && !event.keystroke.modifiers.modified()
}

/// Whether `event` is a Tab the prompt traps (Shift-Tab included; never a chord).
pub(crate) fn trapped_tab(event: &KeyDownEvent) -> bool {
    let m = &event.keystroke.modifiers;
    event.keystroke.key == "tab" && !m.control && !m.platform && !m.alt
}

/// Moves the focus to the next (or, `back`, the previous) handle of `order`, wrapping.
pub(crate) fn cycle_focus(order: &[FocusHandle], back: bool, window: &mut Window, cx: &mut App) {
    if order.is_empty() {
        return;
    }
    let n = order.len();
    let at = order.iter().position(|h| h.is_focused(window));
    let next = match (at, back) {
        (Some(i), false) => (i + 1) % n,
        (Some(i), true) => (i + n - 1) % n,
        (None, false) => 0,
        (None, true) => n - 1,
    };
    window.focus(&order[next], cx);
}

/// What the last key down in a prompt was, read when a field reports `Submitted`: the field
/// emits it for any Enter, and only an acting Enter ([`acting_enter`]) may submit (Q34).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct EnterGate {
    acting: bool,
}

impl EnterGate {
    /// Records the key down (call it from the card's capture phase).
    pub(crate) fn note(&mut self, event: &KeyDownEvent) {
        self.acting = acting_enter(event);
    }

    /// Whether the Enter just seen may act; reading it spends it.
    pub(crate) fn take(&mut self) -> bool {
        std::mem::take(&mut self.acting)
    }
}

// ---- layout ---------------------------------------------------------------------------------

/// A prompt's card (`size`): the dialog frame with element ID and debug selector `id`; a press
/// inside never reaches the scrim; the key-up Enter is suppressed (Q34: Enter acts on key down
/// only, so the focused button's keyboard click never presses it again).
pub(crate) fn card(theme: &Theme, size: DialogSize, id: &'static str) -> Stateful<Div> {
    dialog_frame(theme, size)
        .id(id)
        .debug_selector(move || id.to_string())
        .capture_key_up(|event, window, _| {
            let stroke = &event.keystroke;
            if stroke.key == "enter" && !stroke.modifiers.modified() {
                window.prevent_default();
            }
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// The header: the title (wrapping, `body_strong`, recorded as `title_id`), the subtitle under
/// it, and the close button (pointer and assistive technology only; Escape closes from the
/// keyboard), which runs `on_close`.
pub(crate) fn header(
    theme: &Theme,
    title: SharedString,
    title_id: &'static str,
    subtitle: Option<Div>,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    let t = theme.tokens;
    let c = &t.colors;
    let close = div()
        .id("prompt-close")
        .debug_selector(|| "prompt-close".to_string())
        .declare(Role::Button, ui_text(cx, "hui-dialog-close"), None)
        .flex()
        .items_center()
        .flex_none()
        .p(px(t.spacing.xs))
        .rounded(px(t.radii.sm))
        .cursor_pointer()
        .on_click(on_close)
        .child(
            // Raised Ink Rule: text-2 on the raised dialog, never text-3.
            Icon::new(IconName::X)
                .size(IconSize::Chrome)
                .color(c.text_2.to_hsla()),
        );
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(t.spacing.xl))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(t.spacing.s_2xs))
                .child(
                    paragraph(
                        title,
                        TextSource::Fluent,
                        t.type_ramp.body_strong,
                        c.text.to_hsla(),
                    )
                    .id(title_id),
                )
                .children(subtitle),
        )
        .child(close)
}

/// One part of a subtitle.
pub(crate) enum Part {
    /// Hacivat's own words (12.5 px).
    Fluent(SharedString),
    /// A literal: a connection name, a user, a host (Fragment Mono 11.5).
    Mono(SharedString),
}

/// The subtitle (NOTES: "env mark, connection, …"): the environment mark, then `parts` joined
/// by a separator dot, all in text-2 (Raised Ink); it wraps between parts.
pub(crate) fn subtitle(theme: &Theme, env: Option<EnvTag>, parts: Vec<Part>) -> Div {
    let t = theme.tokens;
    let c = &t.colors;
    let ink = c.text_2.to_hsla();
    let mut row = div()
        .debug_selector(|| "prompt-subtitle".to_string())
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(t.spacing.sm))
        .children(env.map(|e| env_swatch(env_kind(e), theme)));
    for (i, part) in parts.into_iter().enumerate() {
        if i > 0 {
            row = row.child(Label::new(
                SEPARATOR,
                TextSource::Data,
                t.type_ramp.body_sm,
                ink,
            ));
        }
        row = row.child(match part {
            Part::Fluent(text) => Label::new(text, TextSource::Fluent, t.type_ramp.body_sm, ink),
            Part::Mono(text) => Label::new(text, TextSource::Data, t.type_ramp.mono_data, ink),
        });
    }
    row
}

/// A form row of the 420/480 px dialogs: the label (13 px text-2) in the 120 px column, then
/// the control cell.
pub(crate) fn row(theme: &Theme, label: SharedString, control: impl IntoElement) -> Div {
    let t = theme.tokens;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.xl))
        .w_full()
        .min_h(px(t.spacing.field_height))
        .child(
            div().flex_none().w(px(LABEL_PX)).child(
                Label::new(
                    label,
                    TextSource::Fluent,
                    t.type_ramp.body,
                    t.colors.text_2.to_hsla(),
                )
                .truncate(true),
            ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.md))
                .flex_1()
                .min_w_0()
                .child(control),
        )
}

/// Content under a field, aligned with the control column.
pub(crate) fn under_field(theme: &Theme, content: impl IntoElement) -> Div {
    let t = theme.tokens;
    div()
        .flex()
        .flex_row()
        .gap(px(t.spacing.xl))
        .w_full()
        .child(div().flex_none().w(px(LABEL_PX)))
        .child(div().flex_1().min_w_0().child(content))
}

/// A static value in a row: Fragment Mono 12 px text (NOTES `.static`), recorded as `id`.
pub(crate) fn static_value(theme: &Theme, text: impl Into<SharedString>, id: &'static str) -> Div {
    let t = theme.tokens;
    div().min_w_0().child(
        paragraph(
            text.into(),
            TextSource::Data,
            t.type_ramp.mono_data,
            t.colors.text.to_hsla(),
        )
        .id(id),
    )
}

/// The inset (NOTES "Key comparison"): `panel` (dark) or `ground` (light), a 1 px seam-strong
/// border, radius 4, padding 8/10.
pub(crate) fn inset(theme: &Theme) -> Div {
    let t = theme.tokens;
    let c = &t.colors;
    let fill = match theme.mode {
        ThemeMode::Dark => c.panel,
        ThemeMode::Light => c.ground,
    };
    div()
        .flex()
        .flex_col()
        .py(px(t.spacing.md))
        .px(px(t.spacing.lg))
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .rounded(px(t.radii.sm))
        .bg(fill.to_hsla())
}

/// The ink alert line (NOTES: a rejected answer is an ink glyph and plain words, never a hue):
/// the circle-alert glyph and `text` (12 px, text), recorded as `id`.
pub(crate) fn alert_line(theme: &Theme, text: SharedString, id: &'static str) -> Div {
    let t = theme.tokens;
    let c = &t.colors;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.sm))
        .child(
            Icon::new(IconName::CircleAlert)
                .size(IconSize::Chrome)
                .color(c.text.to_hsla()),
        )
        .child(
            Label::new(
                text,
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text.to_hsla(),
            )
            .id(id),
        )
}

/// A meta line (NOTES `.metaline`): 12 px text-2 items that wrap, 12 px apart.
pub(crate) fn meta_line(theme: &Theme) -> Div {
    let t = theme.tokens;
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_x(px(t.spacing.xl))
        .gap_y(px(t.spacing.xs))
}

/// A 12 px text-2 item of a meta line, recorded as `id`.
pub(crate) fn meta_item(theme: &Theme, text: SharedString, id: &'static str) -> Label {
    let t = theme.tokens;
    Label::new(
        text,
        TextSource::Fluent,
        t.type_ramp.caption,
        t.colors.text_2.to_hsla(),
    )
    .id(id)
}

/// The footer: the far-left items, a gap, then the right-hand buttons.
pub(crate) fn footer(theme: &Theme, left: Vec<AnyElement>, right: Vec<AnyElement>) -> Div {
    let t = theme.tokens;
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(t.spacing.md))
        .pt(px(t.spacing.xs))
        .children(left)
        .child(div().flex_1())
        .children(right)
}

// ---- fields ---------------------------------------------------------------------------------

/// An empty placeholder.
pub(crate) fn no_placeholder() -> TextFn {
    Rc::new(|_| SharedString::default())
}

/// A masked field on its own well (M2a: bullets painted, nothing copied, an empty accessible
/// value, and `text_for_range` answers bullets: the I2 rule).
pub(crate) fn secret_input<V: 'static>(cx: &mut Context<V>) -> Entity<TextInput> {
    cx.new(|cx| {
        let mut input = TextInput::new(cx, no_placeholder());
        input.set_masked(true);
        input.set_well(true);
        input
    })
}

/// A plain field on its own well (`mono` for typed literals).
pub(crate) fn plain_input<V: 'static>(cx: &mut Context<V>, mono: bool) -> Entity<TextInput> {
    cx.new(|cx| {
        let mut input = TextInput::new(cx, no_placeholder());
        input.set_well(true);
        input.set_mono(mono);
        input
    })
}

/// The field's value as a secret (`None` when empty); the field is cleared at once.
pub(crate) fn take_secret(input: &Entity<TextInput>, cx: &mut App) -> Option<SecretString> {
    input.update(cx, |i, cx| {
        let typed = (!i.value().is_empty()).then(|| SecretString::from(i.value().to_string()));
        i.set_value("");
        cx.notify();
        typed
    })
}

/// Clears `inputs` (a prompt that closes keeps no typed text).
pub(crate) fn clear_all(inputs: &[Entity<TextInput>], cx: &mut App) {
    for input in inputs {
        input.update(cx, |i, cx| {
            i.set_value("");
            cx.notify();
        });
    }
}

/// Whether `input` holds nothing.
pub(crate) fn is_empty(input: &Entity<TextInput>, cx: &App) -> bool {
    input.read(cx).value().is_empty()
}

// ---- the profile behind a question ------------------------------------------------------------

/// What a prompt shows about the profile that asks: its name and tag, its hops and the
/// database end of its route.
#[derive(Debug, Clone)]
pub(crate) struct Subject {
    pub(crate) name: String,
    pub(crate) env: Option<EnvTag>,
    pub(crate) hops: Vec<HopSpec>,
    /// `host:port` of the database (the engine's default port when none is set).
    pub(crate) database: String,
    pub(crate) retention: Retention,
    pub(crate) ssh_retention: Retention,
}

/// The saved profile `id`, as a prompt shows it; `None` when it is not (or no longer) saved.
pub(crate) fn subject(cx: &App, id: ProfileId) -> Option<Subject> {
    let p = cx.try_global::<ConnectionsModel>()?.get(id)?;
    let hops = match &p.transport {
        TransportSpec::SshTunnel(spec) => spec.hops.clone(),
        _ => Vec::new(),
    };
    let database = match &p.endpoint {
        ProfileEndpoint::Server { host, port } => {
            let port = port.or_else(|| {
                EngineCatalog::embedded()
                    .get(p.engine)
                    .and_then(|e| e.default_port)
            });
            match port {
                Some(port) => format!("{}:{port}", host.trim()),
                None => host.trim().to_string(),
            }
        }
        ProfileEndpoint::Socket { path, .. } | ProfileEndpoint::File { path, .. } => {
            path.to_string_lossy().into_owned()
        }
    };
    Some(Subject {
        name: p.name.clone(),
        env: p.environment,
        hops,
        database,
        retention: p.retention,
        ssh_retention: p.ssh_retention,
    })
}

// ---- the route row --------------------------------------------------------------------------

/// The route (NOTES "Hop route"): this computer → every hop → the database, each arrow bound to
/// the stop after it so a wrapped line never ends on an arrow. The asking hop (`asking`) is
/// boxed (1 px seam-strong, radius 3, padding 0/4) in text; the other stops are text-2. Stop
/// `k` is recorded as `route-<k>`.
pub(crate) fn route_row(
    theme: &Theme,
    this_computer: SharedString,
    hops: &[HopSpec],
    database: &str,
    asking: Option<usize>,
) -> Div {
    const IDS: [&str; 11] = [
        "route-0", "route-1", "route-2", "route-3", "route-4", "route-5", "route-6", "route-7",
        "route-8", "route-9", "route-10",
    ];
    let t = theme.tokens;
    let c = &t.colors;
    let mut stops: Vec<(SharedString, TextSource, bool)> =
        vec![(this_computer, TextSource::Fluent, false)];
    for (i, hop) in hops.iter().enumerate() {
        stops.push((hop.host.clone().into(), TextSource::Data, asking == Some(i)));
    }
    stops.push((database.to_string().into(), TextSource::Data, false));
    let mut route = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_x(px(t.spacing.sm))
        .gap_y(px(t.spacing.xs))
        .min_w_0();
    for (k, (text, source, boxed)) in stops.into_iter().enumerate() {
        let (style, ink) = match (k, boxed) {
            (0, _) => (t.type_ramp.caption, c.text_2),
            (_, true) => (t.type_ramp.mono_data, c.text),
            (_, false) => (t.type_ramp.mono_data, c.text_2),
        };
        let mut stop = div().min_w_0().child(
            Label::new(text, source, style, ink.to_hsla())
                .truncate(true)
                .id(IDS.get(k).copied().unwrap_or("route-n")),
        );
        if boxed {
            stop = stop
                .debug_selector(|| "route-asking".to_string())
                .border_1()
                .border_color(c.seam_strong.to_hsla())
                .rounded(px(t.radii.xs))
                .px(px(t.spacing.xs));
        }
        let mut item = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.sm))
            .min_w_0()
            .max_w_full();
        if k > 0 {
            item = item.child(div().flex_none().child(Label::new(
                ROUTE_ARROW,
                TextSource::Data,
                t.type_ramp.caption,
                c.text_2.to_hsla(),
            )));
        }
        route = route.child(item.child(stop));
    }
    route
}

/// Whether this computer is a Mac (the route's first stop and the host-key body say so).
pub(crate) fn on_mac() -> bool {
    cfg!(target_os = "macos")
}

// ---- server text ----------------------------------------------------------------------------

/// Whether `c` is a bidi control or an invisible format character that could reorder or hide
/// text: the NOTES set (U+061C, U+200E/F, U+202A–U+202E, U+2066–U+2069) plus the zero-width
/// characters, the line and paragraph separators, the invisible operators and the BOM.
fn is_format_control(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// Server text made visible, as OpenSSH's `strnvis` does (NOTES surface 2): a backslash is
/// doubled first, so the text cannot imitate an escape; C0 and C1 controls (ANSI escapes
/// included; only `\n` is kept) become `\ooo` octal; bidi and invisible format characters
/// become `\u{XXXX}`. At most `max_chars` characters of `text` are shown; the rest is cut and
/// marked with an ellipsis.
pub(crate) fn visible(text: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max_chars * 4));
    for (n, c) in text.chars().enumerate() {
        if n >= max_chars {
            out.push(CUT);
            break;
        }
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push('\n'),
            c if is_format_control(c) => {
                let _ = write!(out, "\\u{{{:04X}}}", u32::from(c));
            }
            c if c.is_control() => {
                let _ = write!(out, "\\{:03o}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// The number `n` as a Fluent count.
pub(crate) fn count(n: usize) -> hacivat_l10n::Count {
    hacivat_l10n::Count(u64::try_from(n).unwrap_or(u64::MAX))
}

// ---- test seams -------------------------------------------------------------------------------

/// The named parts of the open prompts, for the app's own tests (`test-support` only): each
/// prompt registers its fields and buttons when it opens.
#[cfg(feature = "test-support")]
pub mod parts {
    use hacivat_ui::input::TextInput;
    use hacivat_ui_core::gpui_surface::{App, Entity, FocusHandle, Global, WeakEntity, Window};

    #[derive(Default)]
    struct Parts(Vec<(String, FocusHandle, Option<WeakEntity<TextInput>>)>);

    impl Global for Parts {}

    /// Facts a prompt learned off the UI thread (the vault's reset probe).
    #[derive(Default)]
    struct Flags(Vec<(String, bool)>);

    impl Global for Flags {}

    /// Records `name` = `value`.
    pub(crate) fn set_flag(cx: &mut App, name: &str, value: bool) {
        let flags = cx.default_global::<Flags>();
        flags.0.retain(|(n, _)| n != name);
        flags.0.push((name.to_string(), value));
    }

    /// The last value recorded for `name`.
    pub fn flag(cx: &App, name: &str) -> Option<bool> {
        cx.try_global::<Flags>()?
            .0
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| *v)
    }

    /// Registers `name`'s focus handle (and its field) for the tests.
    pub(crate) fn register(
        cx: &mut App,
        name: &str,
        focus: FocusHandle,
        input: Option<&Entity<TextInput>>,
    ) {
        let input = input.map(Entity::downgrade);
        let parts = cx.default_global::<Parts>();
        parts.0.retain(|(n, _, _)| n != name);
        parts.0.push((name.to_string(), focus, input));
    }

    /// The name of the focused registered part.
    pub fn focused(window: &Window, cx: &App) -> Option<String> {
        cx.try_global::<Parts>()?
            .0
            .iter()
            .rev()
            .find(|(_, focus, _)| focus.is_focused(window))
            .map(|(name, _, _)| name.clone())
    }

    /// The field registered as `name`, while its prompt is open.
    pub fn input(cx: &App, name: &str) -> Option<Entity<TextInput>> {
        cx.try_global::<Parts>()?
            .0
            .iter()
            .rev()
            .find(|(n, _, _)| n == name)
            .and_then(|(_, _, input)| input.as_ref()?.upgrade())
    }

    /// The focus handle registered as `name`.
    pub fn focus(cx: &App, name: &str) -> Option<FocusHandle> {
        cx.try_global::<Parts>()?
            .0
            .iter()
            .rev()
            .find(|(n, _, _)| n == name)
            .map(|(_, focus, _)| focus.clone())
    }
}

/// Registers a part for the tests (a no-op without `test-support`).
#[cfg(feature = "test-support")]
pub(crate) fn register_part(
    cx: &mut App,
    name: &str,
    focus: &FocusHandle,
    input: Option<&Entity<TextInput>>,
) {
    parts::register(cx, name, focus.clone(), input);
}

/// Registers a part for the tests (a no-op without `test-support`).
#[cfg(not(feature = "test-support"))]
pub(crate) fn register_part(
    _cx: &mut App,
    _name: &str,
    _focus: &FocusHandle,
    _input: Option<&Entity<TextInput>>,
) {
}

/// Records a fact for the tests (a no-op without `test-support`).
pub(crate) fn note_flag(cx: &mut App, name: &str, value: bool) {
    #[cfg(feature = "test-support")]
    parts::set_flag(cx, name, value);
    #[cfg(not(feature = "test-support"))]
    let _ = (cx, name, value);
}

#[cfg(test)]
mod tests {
    use super::visible;

    #[test]
    fn server_text_is_made_visible_like_strnvis() {
        assert_eq!(visible("Kod girin", 100), "Kod girin");
        assert_eq!(
            visible("a\\033[0m", 100),
            "a\\\\033[0m",
            "a literal escape stays visible"
        );
        assert_eq!(
            visible("\u{1b}[1;32mok\u{1b}[0m", 100),
            "\\033[1;32mok\\033[0m"
        );
        assert_eq!(visible("one\ntwo\r\t", 100), "one\ntwo\\015\\011");
        assert_eq!(visible("\u{9b}", 100), "\\233", "C1 controls too");
        assert_eq!(
            visible("exe.\u{202E}edoc\u{202C}", 100),
            "exe.\\u{202E}edoc\\u{202C}"
        );
        assert_eq!(visible("a\u{200B}b\u{FEFF}", 100), "a\\u{200B}b\\u{FEFF}");
        assert_eq!(
            visible("Doğrulama", 100),
            "Doğrulama",
            "letters stay as they are"
        );
    }

    #[test]
    fn server_text_is_cut_at_its_cap() {
        assert_eq!(visible("abcdef", 3), "abc…");
        assert_eq!(visible("abc", 3), "abc");
    }
}
