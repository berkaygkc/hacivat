//! The console toolbar on the Overflow Rule `Toolbar` (Task 32): Run / Stop, the connection
//! label, the read-only lock chip, the waiting state.
//!
//! NOTES surface 2, left to right: Run (primary, its chord) or Stop (outline) in one slot; the
//! run state (the amber run marker, "Running" and a mono clock repainted once a second, or the
//! hollow marker and "Cancelling…"); the connection label (a seam, the environment mark, the
//! connection's name in mono); the waiting state (a hollow marker, "Waiting for a free
//! connection to orders-dev (4 of 4 in use)", Cancel). On the right: the lock chip of a
//! read-only connection.
//!
//! Overflow (ruling 14): Run/Stop, the run state, the connection name, Cancel and the lock are
//! pinned. The waiting sentence takes its short form first. Nothing else moves.
//!
//! ## m3b-t26-prod-unlock (R-M3-6 REVISED ends, R-M3-81): the production read-only notice, its
//! short lock chip and its explanation are gone: a production console writes behind the gate's
//! confirmations. Its trust signals stay where they were: the environment swatch in the
//! connection label, the hairline and the capsule (`console::decor`), and the lock chip when
//! the profile itself is read-only.
//!
//! ## m3a-f2-visual (named exception, fix round 1): when even that does not fit (960 px in the
//! long pseudo-locales), the short sentence wraps between its marker and Cancel and the bar
//! grows; in a narrow pane the whole row wraps onto lines (`Toolbar::wrap_row`). So the lock
//! chip and the environment mark (trust signals) are never pushed off the bar, and the
//! sentence never leaves it.
//!
//! ## m3c-t41-chrome (R-M3-75; design/m3b-safe surface 24): below the width where every item
//! fits whole on a line (a split pane down to its 120 px floor), the bar takes its narrow
//! stage (`ToolbarItem::narrow`) and each item its narrow form, the named exceptions N1–N4:
//! Run and Stop icon-only, their word and keys in the tooltip and the accessible name (N1);
//! the connection's name ellipsizes, data, whole in its tooltip, while the environment mark
//! never does (N2); the badge drops its age and keeps Commit and Roll back as 20 px icon
//! buttons (N3); the mode chip drops its commit glyph and keeps its word ("Auto", or
//! "Auto-commit" on a read-only pane) (N4). Nothing else relaxes: the read-only label stays
//! whole, wrapping inside its chip, as does the pending read-only chip; the waiting sentence
//! keeps all its words on two lines, then Cancel; the run state keeps its marker and clock.

use std::rc::Rc;

use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::{EnvKind, env_swatch_on_raised};
use hacivat_ui::icon::{Icon, IconName, IconSize};
// ## m3c-t41-chrome
use hacivat_ui::popover::{Declare as _, Tip};
use hacivat_ui::step_trail::run_marker;
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui::toolbar::{ItemFn, Toolbar, ToolbarItem, Wrap};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, px,
};
// ## m3c-t41-chrome
use hacivat_ui_core::gpui_surface::{Role, StatefulInteractiveElement as _};
use hacivat_ui_core::theme::{Theme, TypeStyle};

use crate::l10n_global::{text_fn, tr};
use crate::shell::keymap_state::shortcut_label;

/// The run state the toolbar shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunUx {
    Idle,
    /// Running for this many whole seconds.
    Running(u64),
    Cancelling,
}

/// What the toolbar shows, built by the console host for each frame.
#[derive(Clone)]
pub struct ToolbarModel {
    pub run: RunUx,
    /// Run can be pressed (not while waiting for a session).
    pub run_enabled: bool,
    /// The connection's name (data) and its environment mark.
    pub name: SharedString,
    pub env: Option<EnvKind>,
    /// Waiting for a session: the connection's name and (in use, max).
    pub waiting: Option<(SharedString, u8, u8)>,
    /// The access label of a read-only connection (the lock chip).
    pub read_only: Option<SharedString>,
    /// The connection must be read-only but this console's session is still read-write: its
    /// name. The pending chip says so in place of the notice (fix round 1, C1).
    pub pending_read_only: Option<SharedString>,
}

/// A toolbar button's action.
pub type ActionFn = Rc<dyn Fn(&mut Window, &mut App)>;

/// What the toolbar's buttons do (the console host's methods).
#[derive(Clone)]
pub struct ToolbarActions {
    pub run: ActionFn,
    pub stop: ActionFn,
    pub cancel_wait: ActionFn,
}

fn item(f: impl Fn(&mut Window, &mut App) -> AnyElement + 'static) -> ItemFn {
    Rc::new(f)
}

fn same(text: SharedString) -> TextFn {
    Rc::new(move |_| text.clone())
}

/// The console's toolbar (32 px, `raised`).
pub fn build(model: &ToolbarModel, actions: &ToolbarActions, cx: &App) -> Toolbar {
    build_for(model, actions, None, cx)
}

// ## m3b-t22-tx-ui (named exception): the transaction items of console `console` (NOTES
// surfaces 3, 4 and 12; ruling 9): the mode chip and the "Shared session" chip right after the
// connection label, the transaction badge on the right before the lock. The chip and the badge
// are trust signals, pinned. The Overflow Rule's order (NOTES surface 4): the badge takes its
// compact form first (no age, icon-only buttons; priority 12), then the shared chip its icon
// (13, ruling 26), then the pending read-only sentence (15) and the waiting sentence (20).
/// The console's toolbar with console `console`'s transaction items.
pub fn build_for(
    model: &ToolbarModel,
    actions: &ToolbarActions,
    console: Option<crate::console::ConsoleId>,
    cx: &App,
) -> Toolbar {
    let theme = *Theme::get(cx);
    let height = theme.tokens.spacing.editor_toolbar_height;
    // ## m3a-f2-visual (named exception, fix round 1, I2): in a narrow pane the row wraps, so
    // no trust signal (the read-only chip, the environment mark) is cut.
    let mut bar = Toolbar::new("console-toolbar").height(height).wrap_row();

    // Run / Stop, one slot.
    let run_model = model.clone();
    let a = actions.clone();
    let run_label: TextFn = match model.run {
        RunUx::Idle => text_fn(ids::hv::CONSOLE_RUN),
        _ => text_fn(ids::hv::CONSOLE_STOP),
    };
    // ## m3c-t41-chrome (N1): icon-only in a narrow pane.
    let (narrow_model, na) = (model.clone(), actions.clone());
    bar = bar.item(
        ToolbarItem::pinned(
            item(move |_, cx| run_button(&run_model, &a, false, cx)),
            run_label,
        )
        .narrow(item(move |_, cx| run_button(&narrow_model, &na, true, cx))),
    );

    // The run state.
    if model.run != RunUx::Idle {
        let run = model.run;
        let label: TextFn = match run {
            RunUx::Cancelling => text_fn(ids::hv::CONSOLE_CANCELLING),
            _ => text_fn(ids::hv::CONSOLE_RUNNING),
        };
        let mut state = ToolbarItem::pinned(item(move |_, cx| run_state(run, cx)), label)
            // ## m3c-t41-chrome: the marker and the clock; "Cancelling…" wraps.
            .narrow(item(move |_, cx| run_state_narrow(run, cx)));
        if run == RunUx::Cancelling {
            state = state.narrow_wraps(
                tr(cx, ids::hv::CONSOLE_CANCELLING, NoArgs),
                narrow_style(cx),
            );
        }
        bar = bar.item(state);
    }

    // The connection label.
    let (name, env) = (model.name.clone(), model.env);
    let narrow_name = model.name.clone();
    bar = bar.item(
        ToolbarItem::pinned(
            item(move |_, cx| connection_label(name.clone(), env, cx)),
            same(model.name.clone()),
        )
        // ## m3c-t41-chrome (N2): the name ellipsizes (data); the mark never does.
        .narrow(item(move |_, cx| {
            connection_label_narrow(narrow_name.clone(), env, cx)
        })),
    );

    // ## m3b-t22-tx-ui (named exception): the mode chip, then the shared-session chip.
    if let Some(console) = console {
        if crate::console::tx_chip::render(console, cx).is_some() {
            let label: TextFn = Rc::new(move |cx| crate::console::tx_chip::chip_label(console, cx));
            bar = bar.item(
                ToolbarItem::pinned(
                    item(move |_, cx| {
                        crate::console::tx_chip::render(console, cx)
                            .unwrap_or_else(|| div().into_any_element())
                    }),
                    label,
                )
                // ## m3c-t41-chrome (N4): the word stays, the commit glyph goes.
                .narrow(item(move |_, cx| {
                    crate::console::tx_chip::render_narrow(console, cx)
                        .unwrap_or_else(|| div().into_any_element())
                }))
                .narrow_wraps(
                    crate::console::tx_chip::narrow_word(console, cx).unwrap_or_default(),
                    narrow_style(cx),
                ),
            );
        }
        if crate::console::tx_chip::render_shared(console, false, cx).is_some() {
            // Fix round 1 (I2): pinned, so it never leaves the row; it may still take its
            // icon-only form (NOTES ruling 26).
            bar = bar.item(
                ToolbarItem::pinned(
                    item(move |_, cx| {
                        crate::console::tx_chip::render_shared(console, false, cx)
                            .unwrap_or_else(|| div().into_any_element())
                    }),
                    text_fn(ids::hv::CONSOLE_SHARED),
                )
                .priority(13)
                .compact(item(move |_, cx| {
                    crate::console::tx_chip::render_shared(console, true, cx)
                        .unwrap_or_else(|| div().into_any_element())
                })),
            );
        }
    }

    // The waiting state: the sentence, then its short form; Cancel stays.
    // ## m3a-f2-visual (named exception, fix round 1): a trust signal never gives way to the
    // sentence, and the sentence never leaves the bar: after its short form it wraps (the
    // toolbar's wrapping stage), and in a narrow pane the whole row wraps (`wrap_row`).
    if let Some((name, in_use, max)) = model.waiting.clone() {
        let a = actions.clone();
        let long_name = name.clone();
        let full: TextFn = Rc::new(move |cx| waiting_text(cx, &long_name, in_use, max, false));
        let a2 = actions.clone();
        let short_name = name.clone();
        let (a3, wrapped_name) = (actions.clone(), name.clone());
        let a4 = actions.clone();
        let two_lines = SharedString::from(format!(
            "{}\n{}",
            tr(cx, ids::hv::CONSOLE_WAITING_L1, NoArgs),
            waiting_count(cx, in_use, max)
        ));
        let wrap = Wrap {
            text: waiting_text(cx, &name, in_use, max, true),
            style: wrap_style(cx),
            element: item(move |_, cx| waiting_wrapped(&wrapped_name, in_use, max, &a3, cx)),
        };
        bar = bar.item(
            ToolbarItem::pinned(
                item(move |_, cx| waiting(&name, in_use, max, false, &a, cx)),
                full,
            )
            .priority(20)
            .compact(item(move |_, cx| {
                waiting(&short_name, in_use, max, true, &a2, cx)
            }))
            .wraps(wrap)
            // ## m3c-t41-chrome: two lines, every word kept, then Cancel.
            .narrow_sized(Rc::new(move |width, window, cx| {
                waiting_narrow(in_use, max, width, &a4, window, cx)
            }))
            .narrow_wraps(two_lines, narrow_style(cx)),
        );
    }

    // ## m3b-t22-tx-ui (named exception): the user's transaction, right-aligned before what is
    // true about writing (ruling 9).
    if let Some(console) = console
        && crate::console::tx_badge::render(console, cx).is_some()
    {
        let label: TextFn = Rc::new(move |cx| {
            crate::console::tx_badge::badge_text(console, true, cx).unwrap_or_default()
        });
        bar = bar.trailing(
            ToolbarItem::pinned(
                item(move |_, cx| {
                    crate::console::tx_badge::render(console, cx)
                        .unwrap_or_else(|| div().into_any_element())
                }),
                label,
            )
            .priority(12)
            .compact(item(move |_, cx| {
                crate::console::tx_badge::render_compact(console, cx)
                    .unwrap_or_else(|| div().into_any_element())
            }))
            // ## m3c-t41-chrome (N3): the stamp and the age-less text wrapping, then the
            // two 20 px icon buttons.
            .narrow(item(move |_, cx| {
                crate::console::tx_badge::render_narrow(console, cx)
                    .unwrap_or_else(|| div().into_any_element())
            }))
            .narrow_wraps(
                crate::console::tx_badge::narrow_text(console, cx).unwrap_or_default(),
                narrow_style(cx),
            ),
        );
    }

    // Right: what is true about writing. A pending read-only reconnect first (C1), then the
    // lock chip (## m3b-t26-prod-unlock: the production notice is gone).
    if let Some(name) = model.pending_read_only.clone() {
        let (long, short) = (name.clone(), name);
        let label: TextFn = Rc::new(move |cx| pending_text(cx, &long));
        bar = bar.trailing(
            ToolbarItem::pinned(item(move |_, cx| pending_chip(&short, false, cx)), label)
                .priority(15)
                .compact(item(move |_, cx| pending_chip("", true, cx)))
                // ## m3c-t41-chrome: the short sentence wraps inside the chip.
                .narrow(item(move |_, cx| pending_chip_narrow(cx)))
                .narrow_wraps(
                    tr(cx, ids::hv::CONSOLE_RO_PENDING_SHORT, NoArgs),
                    narrow_style(cx),
                ),
        );
    } else if let Some(access) = model.read_only.clone() {
        let narrow_access = access.clone();
        bar = bar.trailing(
            ToolbarItem::pinned(
                item(move |_, cx| lock_chip(access.clone(), cx)),
                same(model.read_only.clone().unwrap_or_default()),
            )
            // ## m3c-t41-chrome: the whole label ("read-only (guarded)"), wrapping inside the
            // chip.
            .narrow(item(move |_, cx| {
                lock_chip_narrow(narrow_access.clone(), cx)
            }))
            .narrow_wraps(
                model.read_only.clone().unwrap_or_default(),
                narrow_style(cx),
            ),
        );
    }
    bar
}

// ## m3c-t41-chrome: the narrow forms (design/m3b-safe surface 24). Each fills the slot the
// bar gives it.

/// A narrow chip's wrapping label (15 px lines).
fn narrow_style(cx: &App) -> TypeStyle {
    crate::console::tx_badge::narrow_style(cx)
}

/// Where a `size` px marker or glyph sits beside a narrow label's first line.
fn first_line_top(cx: &App, size: f32) -> f32 {
    ((narrow_style(cx).line_height - size) / 2.0).max(0.0)
}

/// N1: Run's or Stop's word and keys ("Run · ⌘↵"), the icon-only button's tooltip and name.
fn run_button_narrow_label(cx: &App, model: &ToolbarModel) -> SharedString {
    let (id, command) = match model.run {
        RunUx::Idle => (ids::hv::CONSOLE_RUN, "editor.run"),
        _ => (ids::hv::CONSOLE_STOP, "exec.cancel"),
    };
    let word = tr(cx, id, NoArgs);
    match shortcut_label(cx, command) {
        Some(keys) => tr(
            cx,
            ids::hv::CONSOLE_BUTTON_KEYS,
            ids::hv::HvConsoleButtonKeys {
                label: Text(word.to_string()),
                keys: Text(keys),
            },
        ),
        None => word,
    }
}

/// The run state's narrow form: the amber marker and the clock ("Running" stays its tooltip
/// and name), or the hollow marker and "Cancelling…", wrapping.
fn run_state_narrow(run: RunUx, cx: &App) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    match run {
        RunUx::Running(seconds) => {
            let word = tr(cx, ids::hv::CONSOLE_RUNNING, NoArgs);
            let tip = word.clone();
            div()
                .id("console-run-state")
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.sm))
                .child(run_marker(&theme))
                .child(
                    Label::new(
                        tr(
                            cx,
                            ids::hv::CONSOLE_CLOCK,
                            ids::hv::HvConsoleClock { s: Count(seconds) },
                        ),
                        TextSource::Fluent,
                        t.type_ramp.mono_data,
                        c.text_2.to_hsla(),
                    )
                    .id("console-clock"),
                )
                .declare(Role::Status, word, None)
                .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                .into_any_element()
        }
        RunUx::Cancelling => div()
            .id("console-run-state")
            .flex()
            .flex_row()
            .items_start()
            .flex_1()
            .min_w(px(0.0))
            .gap(px(t.spacing.sm))
            .child(
                div()
                    .size(px(7.0))
                    .flex_none()
                    .mt(px(first_line_top(cx, 7.0)))
                    .border_1()
                    .border_color(c.amber.to_hsla()),
            )
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    paragraph(
                        tr(cx, ids::hv::CONSOLE_CANCELLING, NoArgs),
                        TextSource::Fluent,
                        narrow_style(cx),
                        c.text.to_hsla(),
                    )
                    .id("console-cancelling"),
                ),
            )
            .into_any_element(),
        RunUx::Idle => div().into_any_element(),
    }
}

/// N2: the seam, the environment mark (never cut) and the name, which ellipsizes (data; its
/// tooltip is the whole name).
fn connection_label_narrow(name: SharedString, env: Option<EnvKind>, cx: &App) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let tip = name.clone();
    let mut row = div()
        .id("console-connection")
        .flex()
        .flex_row()
        .items_center()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(t.spacing.sm))
        .child(
            div()
                .w(px(1.0))
                .h(px(16.0))
                .flex_none()
                .bg(c.seam.to_hsla()),
        );
    if let Some(env) = env {
        row = row.child(
            env_swatch_on_raised(env, &theme)
                .flex_none()
                .debug_selector(|| "console-env-swatch".to_string()),
        );
    }
    row.child(
        Label::new(
            name,
            TextSource::Data,
            t.type_ramp.mono_data,
            c.text.to_hsla(),
        )
        .truncate(true)
        .id("console-connection-name"),
    )
    .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
    .into_any_element()
}

/// "4 of 4 in use".
fn waiting_count(cx: &App, in_use: u8, max: u8) -> SharedString {
    tr(
        cx,
        ids::hv::CONSOLE_WAITING_L2,
        ids::hv::HvConsoleWaitingL2 {
            inUse: Count(u64::from(in_use)),
            max: Count(u64::from(max)),
        },
    )
}

/// The waiting item's narrow form at `width` px: the hollow marker, "Waiting for a connection"
/// and, in `text-2`, "4 of 4 in use" (both wrap), then Cancel under them. Cancel keeps its
/// word wherever the word fits the pane; only where it does not (a long locale at the split
/// floor) it is its glyph, the word its tooltip and accessible name, as N1 draws Run and Stop.
fn waiting_narrow(
    in_use: u8,
    max: u8,
    width: f32,
    actions: &ToolbarActions,
    window: &mut Window,
    cx: &App,
) -> AnyElement {
    let t = Theme::get(cx).tokens;
    let c = &t.colors;
    let cancel = actions.cancel_wait.clone();
    let style = narrow_style(cx);
    let word: SharedString = tr(cx, ids::hv::CONSOLE_WAITING_CANCEL, NoArgs);
    let control = t.type_ramp.control;
    let word_px = f32::from(
        window
            .text_system()
            .shape_line(
                word.clone(),
                px(control.size_px),
                &[hacivat_ui_core::gpui_surface::TextRun {
                    len: word.len(),
                    font: hacivat_ui_core::fonts::font_for(&control),
                    color: c.text.to_hsla(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
            .width,
    );
    // The button's padding (9 px a side) and its 1 px border.
    let glyph_only = word_px + 2.0 * 9.0 + 2.0 > width;
    let mut cancel_button = Button::new("console-waiting-cancel", word)
        .kind(ButtonKind::Ghost)
        .icon_only(glyph_only)
        .on_raised(true)
        .on_click(move |_, window, cx| cancel(window, cx));
    if glyph_only {
        cancel_button = cancel_button.icon(IconName::X);
    }
    div()
        .id("console-waiting")
        .flex()
        .flex_col()
        .items_start()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(t.spacing.s_2xs))
        .child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .w_full()
                .gap(px(t.spacing.sm))
                .child(
                    div()
                        .size(px(7.0))
                        .flex_none()
                        .mt(px(first_line_top(cx, 7.0)))
                        .border_1()
                        .border_color(c.seam_strong.to_hsla()),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(
                            paragraph(
                                tr(cx, ids::hv::CONSOLE_WAITING_L1, NoArgs),
                                TextSource::Fluent,
                                style,
                                c.text.to_hsla(),
                            )
                            .id("console-waiting-text"),
                        )
                        .child(
                            paragraph(
                                waiting_count(cx, in_use, max),
                                TextSource::Fluent,
                                style,
                                c.text_2.to_hsla(),
                            )
                            .id("console-waiting-count"),
                        ),
                ),
        )
        .child(cancel_button)
        .into_any_element()
}

/// A narrow chip around `glyph` and its wrapping `label` (padding 2/7, at least 22 px tall).
fn narrow_chip(
    id: &'static str,
    glyph: AnyElement,
    label: AnyElement,
    border: bool,
    cx: &App,
) -> AnyElement {
    use hacivat_ui::metrics::{NARROW_CHIP_MIN_PX, NARROW_CHIP_PAD_X_PX, NARROW_CHIP_PAD_Y_PX};
    let t = Theme::get(cx).tokens;
    let chip = div()
        .id(id)
        .debug_selector(move || id.to_string())
        .flex()
        .flex_row()
        .items_start()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(NARROW_CHIP_MIN_PX))
        .gap(px(t.spacing.xs))
        .px(px(NARROW_CHIP_PAD_X_PX))
        .py(px(NARROW_CHIP_PAD_Y_PX))
        .rounded(px(t.radii.sm))
        .child(glyph)
        .child(div().flex_1().min_w(px(0.0)).child(label));
    if border {
        chip.border_1()
            .border_color(t.colors.seam_strong.to_hsla())
            .into_any_element()
    } else {
        chip.into_any_element()
    }
}

/// The pending read-only chip's narrow form: the hollow amber marker and the short sentence,
/// wrapping inside the chip.
fn pending_chip_narrow(cx: &App) -> AnyElement {
    let t = Theme::get(cx).tokens;
    let c = &t.colors;
    narrow_chip(
        "console-ro-pending",
        div()
            .size(px(7.0))
            .flex_none()
            .mt(px(first_line_top(cx, 7.0)))
            .border_1()
            .border_color(c.amber.to_hsla())
            .into_any_element(),
        paragraph(
            tr(cx, ids::hv::CONSOLE_RO_PENDING_SHORT, NoArgs),
            TextSource::Fluent,
            narrow_style(cx),
            c.text.to_hsla(),
        )
        .id("console-ro-pending-text")
        .into_any_element(),
        false,
        cx,
    )
}

/// The lock chip's narrow form: the lock and the whole access label, wrapping inside the chip.
fn lock_chip_narrow(access: SharedString, cx: &App) -> AnyElement {
    let t = Theme::get(cx).tokens;
    let c = &t.colors;
    let glyph = IconSize::Chrome.px();
    narrow_chip(
        "console-lock",
        div()
            .flex_none()
            .mt(px(first_line_top(cx, glyph)))
            .child(
                Icon::new(IconName::Lock)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            )
            .into_any_element(),
        paragraph(
            access,
            TextSource::Fluent,
            narrow_style(cx),
            c.text_2.to_hsla(),
        )
        .id("console-lock-label")
        .into_any_element(),
        true,
        cx,
    )
}

fn run_button(
    model: &ToolbarModel,
    actions: &ToolbarActions,
    narrow: bool,
    cx: &App,
) -> AnyElement {
    // ## m3c-t41-chrome: Run's play glyph (`IconName::Play`, Task 1b), as the mock draws it.
    let (label, kind, icon, command) = match model.run {
        RunUx::Idle => (
            tr(cx, ids::hv::CONSOLE_RUN, NoArgs),
            ButtonKind::Primary,
            Some(IconName::Play),
            "editor.run",
        ),
        _ => (
            tr(cx, ids::hv::CONSOLE_STOP, NoArgs),
            ButtonKind::Default,
            Some(IconName::Square),
            "exec.cancel",
        ),
    };
    let disabled = match model.run {
        RunUx::Idle => !model.run_enabled,
        RunUx::Running(_) => false,
        // ## m3b-t23-escalation (named exception; design/m3b-safe ruling 29): Stop stays
        // enabled while cancelling: its second press opens the escalation dialog.
        RunUx::Cancelling => false,
    };
    let on_click = match model.run {
        RunUx::Idle => actions.run.clone(),
        _ => actions.stop.clone(),
    };
    // ## m3b-t23-escalation (named exception, fix round 1, m7)
    let stops = model.run != RunUx::Idle;
    // ## m3c-t41-chrome (N1): the narrow form is the icon alone; its word and keys are the
    // tooltip and the accessible name.
    let label = if narrow {
        run_button_narrow_label(cx, model)
    } else {
        label
    };
    let mut button = Button::new("console-run", label)
        .icon_only(narrow)
        .kind(kind)
        .on_raised(true)
        .disabled(disabled)
        // ## m3b-t23-escalation (named exception, fix round 1, m7): the second click of a
        // double-click on Stop is never the second press that opens the escalation dialog.
        .on_click(move |event, window, cx| {
            if stops && event.click_count() > 1 {
                return;
            }
            on_click(window, cx)
        });
    if let Some(icon) = icon {
        button = button.icon(icon);
    }
    if let Some(keys) = shortcut_label(cx, command) {
        button = button.key_hint(keys.into());
    }
    button.into_any_element()
}

fn run_state(run: RunUx, cx: &App) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let row = div()
        .id("console-run-state")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.sm));
    match run {
        RunUx::Cancelling => row
            .child(
                div()
                    .size(px(7.0))
                    .flex_none()
                    .border_1()
                    .border_color(c.amber.to_hsla()),
            )
            .child(
                Label::new(
                    tr(cx, ids::hv::CONSOLE_CANCELLING, NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text.to_hsla(),
                )
                // ## m3a-t35-shots (fix round 1, I1; named exception): ids the shots' markers name.
                .id("console-cancelling"),
            )
            .into_any_element(),
        RunUx::Running(seconds) => row
            .child(run_marker(&theme))
            .child(
                Label::new(
                    tr(cx, ids::hv::CONSOLE_RUNNING, NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text.to_hsla(),
                )
                // ## m3a-t35-shots (fix round 1, I1; named exception)
                .id("console-running"),
            )
            .child(
                Label::new(
                    tr(
                        cx,
                        ids::hv::CONSOLE_CLOCK,
                        ids::hv::HvConsoleClock { s: Count(seconds) },
                    ),
                    TextSource::Fluent,
                    t.type_ramp.mono_data,
                    c.text_2.to_hsla(),
                )
                // ## m3a-t35-shots (fix round 1, I1; named exception)
                .id("console-clock"),
            )
            .into_any_element(),
        RunUx::Idle => row.into_any_element(),
    }
}

fn connection_label(name: SharedString, env: Option<EnvKind>, cx: &App) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let mut row = div()
        .id("console-connection")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.sm))
        // The seam before the label.
        .child(div().w(px(1.0)).h(px(16.0)).bg(c.seam.to_hsla()));
    if let Some(env) = env {
        // ## m3a-f2-visual (fix round 1, I2): the swatch's selector, for the narrow-pane test.
        row = row.child(
            env_swatch_on_raised(env, &theme).debug_selector(|| "console-env-swatch".to_string()),
        );
    }
    row.child(
        Label::new(
            name,
            TextSource::Data,
            t.type_ramp.mono_data,
            c.text.to_hsla(),
        )
        .id("console-connection-name"),
    )
    .into_any_element()
}

fn waiting_text(cx: &App, name: &str, in_use: u8, max: u8, short: bool) -> SharedString {
    if short {
        tr(
            cx,
            ids::hv::CONSOLE_WAITING_SHORT,
            ids::hv::HvConsoleWaitingShort {
                inUse: Count(u64::from(in_use)),
                max: Count(u64::from(max)),
            },
        )
    } else {
        tr(
            cx,
            ids::hv::CONSOLE_WAITING,
            ids::hv::HvConsoleWaiting {
                name: Text(name.to_string()),
                inUse: Count(u64::from(in_use)),
                max: Count(u64::from(max)),
            },
        )
    }
}

fn waiting(
    name: &str,
    in_use: u8,
    max: u8,
    short: bool,
    actions: &ToolbarActions,
    cx: &App,
) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    waiting_row(
        Label::new(
            waiting_text(cx, name, in_use, max, short),
            TextSource::Fluent,
            t.type_ramp.caption,
            c.text.to_hsla(),
        )
        .id("console-waiting-text")
        .into_any_element(),
        false,
        actions,
        cx,
    )
}

// ## m3a-f2-visual (named exception, fix round 1): the wrapped form (the toolbar's wrapping
// stage): the short sentence in a paragraph that fills the room the marker and Cancel leave.
fn waiting_wrapped(
    name: &str,
    in_use: u8,
    max: u8,
    actions: &ToolbarActions,
    cx: &App,
) -> AnyElement {
    let t = Theme::get(cx).tokens;
    waiting_row(
        div()
            .flex_1()
            .min_w(px(0.0))
            .child(
                paragraph(
                    waiting_text(cx, name, in_use, max, true),
                    TextSource::Fluent,
                    wrap_style(cx),
                    t.colors.text.to_hsla(),
                )
                .id("console-waiting-text"),
            )
            .into_any_element(),
        true,
        actions,
        cx,
    )
}

/// The wrapped sentence's style: the caption at 16 px lines (as the cursor strip's).
fn wrap_style(cx: &App) -> TypeStyle {
    TypeStyle {
        line_height: 16.0,
        ..Theme::get(cx).tokens.type_ramp.caption
    }
}

/// The waiting row around its `sentence`: the hollow `seam-strong` marker before it, Cancel
/// after it (one item, so the toolbar keeps them together; NOTES surface 2).
fn waiting_row(sentence: AnyElement, fill: bool, actions: &ToolbarActions, cx: &App) -> AnyElement {
    let t = Theme::get(cx).tokens;
    let cancel = actions.cancel_wait.clone();
    let row = div().id("console-waiting").flex().flex_row().items_center();
    // The wrapped form fills the slot the toolbar sized for it.
    let row = if fill {
        row.flex_1().min_w(px(0.0))
    } else {
        row
    };
    row.gap(px(t.spacing.sm))
        .child(
            div()
                .size(px(7.0))
                .flex_none()
                .border_1()
                .border_color(t.colors.seam_strong.to_hsla()),
        )
        .child(sentence)
        .child(
            Button::new(
                "console-waiting-cancel",
                tr(cx, ids::hv::CONSOLE_WAITING_CANCEL, NoArgs),
            )
            .kind(ButtonKind::Ghost)
            .on_raised(true)
            .on_click(move |_, window, cx| cancel(window, cx)),
        )
        .into_any_element()
}

fn pending_text(cx: &App, name: &str) -> SharedString {
    tr(
        cx,
        ids::hv::CONSOLE_RO_PENDING,
        ids::hv::HvConsoleRoPending {
            name: Text(name.to_string()),
        },
    )
}

/// The pending read-only chip (C1): the hollow amber marker (pending) and the sentence, or its
/// short form.
fn pending_chip(name: &str, short: bool, cx: &App) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let text = if short {
        tr(cx, ids::hv::CONSOLE_RO_PENDING_SHORT, NoArgs)
    } else {
        pending_text(cx, name)
    };
    div()
        .id("console-ro-pending")
        // ## m3c-t41-chrome: its selector, for the narrow-pane sweep.
        .debug_selector(|| "console-ro-pending".to_string())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.sm))
        .child(
            div()
                .size(px(7.0))
                .flex_none()
                .border_1()
                .border_color(c.amber.to_hsla()),
        )
        .child(
            Label::new(
                text,
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text.to_hsla(),
            )
            .id("console-ro-pending-text"),
        )
        .into_any_element()
}

fn lock_chip(access: SharedString, cx: &App) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    div()
        .id("console-lock")
        // ## m3a-f2-visual (fix round 1, I2)
        .debug_selector(|| "console-lock".to_string())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.xs))
        .h(px(22.0))
        .px(px(t.spacing.sm))
        .rounded(px(t.radii.sm))
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .child(
            Icon::new(IconName::Lock)
                .size(IconSize::Chrome)
                .color(c.text_2.to_hsla()),
        )
        .child(
            Label::new(
                access,
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text_2.to_hsla(),
            )
            // ## m3a-t35-shots (fix round 1, I1; named exception)
            .id("console-lock-label"),
        )
        .into_any_element()
}
