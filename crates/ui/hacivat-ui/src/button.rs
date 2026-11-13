//! Buttons (DESIGN.md Components, Buttons): 24 px tall, 4 px radius, 9 px padding (6 px for an
//! icon button), 6 px icon gap, the `control` type style, a trailing mono key hint.
//!
//! The visible string is a parameter (`label`), so the caller resolves it from Fluent. An icon
//! button draws no text: its `label` is its tooltip and its accessible name.

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ClickEvent, ElementId, Entity, FocusHandle, Hsla, InteractiveElement,
    IntoElement, ParentElement, Role, SharedString, StatefulInteractiveElement, Styled, Window,
    div, px,
};
use hacivat_ui_core::theme::{Theme, TypeStyle, focus_outline};

use crate::accesskey::AccessKey;
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::{A11yDecl, A11yNode, Declare, Tip, lazy};
use crate::text::{Label, TextSource};

/// A click handler shared between the widgets that take one.
pub type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonKind {
    /// Transparent with a `seam_strong` outline; hover fills.
    #[default]
    Default,
    /// Blue fill, one per toolbar.
    Primary,
    /// No outline, `text_2` ink.
    Ghost,
    /// Red outline and ink; fills red on hover or keyboard focus.
    Danger,
    /// An icon alone, 6 px padding.
    Icon,
}

pub struct Button {
    id: ElementId,
    label: SharedString,
    kind: ButtonKind,
    icon: Option<IconName>,
    key_hint: Option<SharedString>,
    disabled: bool,
    on_click: Option<ClickHandler>,
    focus: Option<FocusHandle>,
    access_key: Option<AccessKey>,
    fixed_width: Option<f32>,
    inert: bool,
    // ## m2a-t17-dialog
    on_raised: bool,
    // ## m2b-t24-shots
    expanded: Option<bool>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: SharedString) -> Self {
        Self {
            id: id.into(),
            label,
            kind: ButtonKind::Default,
            icon: None,
            key_hint: None,
            disabled: false,
            on_click: None,
            focus: None,
            access_key: None,
            fixed_width: None,
            inert: false,
            on_raised: false,
            expanded: None,
        }
    }

    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A shortcut label after the text (`⌘↵`), generated from the keymap.
    pub fn key_hint(mut self, hint: SharedString) -> Self {
        self.key_hint = Some(hint);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Ignores the pointer and the keyboard and is announced as disabled, but keeps its look:
    /// a control a modal layer has made unavailable without dimming it (the title bar, M2a).
    pub fn inert(mut self, inert: bool) -> Self {
        self.inert = inert;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Like [`Button::on_click`] for a handler that is already shared.
    pub fn on_click_rc(mut self, handler: ClickHandler) -> Self {
        self.on_click = Some(handler);
        self
    }

    /// Uses `handle` as this button's focus handle (a dialog focuses its buttons itself).
    pub fn focus_handle(mut self, handle: FocusHandle) -> Self {
        self.focus = Some(handle);
        self
    }

    /// Fixes the outer width (the title bar's 24 px overflow button).
    pub fn fixed_width(mut self, width: f32) -> Self {
        self.fixed_width = Some(width);
        self
    }

    /// Shows the access key in the label (Windows and Linux dialogs).
    pub fn access_key(mut self, key: AccessKey) -> Self {
        self.access_key = Some(key);
        self
    }

    // ## m2a-t17-dialog
    /// The button sits on a raised surface (a dialog): its key hint is `text-2`, never text-3
    /// (DESIGN.md Raised Ink Rule; the mock's `.dlg` maps text-3 to text-2).
    pub fn on_raised(mut self, on_raised: bool) -> Self {
        self.on_raised = on_raised;
        self
    }

    // ## m2b-t24-shots
    /// The button opens a popup (a menu, a popover): it declares whether that popup is open
    /// (`aria-expanded`).
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }
}

/// The ink, fill and outline of a button in one state.
#[derive(Clone, Copy)]
struct Look {
    ink: Hsla,
    fill: Option<Hsla>,
    outline: Hsla,
}

fn look(kind: ButtonKind, disabled: bool, hot: bool, theme: &Theme) -> Look {
    let c = &theme.tokens.colors;
    let clear = TRANSPARENT;
    if disabled {
        // text-3 ink; the seam-strong outline stays except on ghost, which has none.
        return Look {
            ink: c.text_3.to_hsla(),
            fill: None,
            outline: match kind {
                ButtonKind::Ghost | ButtonKind::Icon => clear,
                _ => c.seam_strong.to_hsla(),
            },
        };
    }
    match (kind, hot) {
        (ButtonKind::Default, false) => Look {
            ink: c.text.to_hsla(),
            fill: None,
            outline: c.seam_strong.to_hsla(),
        },
        (ButtonKind::Default, true) => Look {
            ink: c.text.to_hsla(),
            fill: Some(c.hover.to_hsla()),
            outline: c.seam_strong.to_hsla(),
        },
        (ButtonKind::Primary, false) => Look {
            ink: c.on_accent.to_hsla(),
            fill: Some(c.blue.to_hsla()),
            outline: c.blue.to_hsla(),
        },
        (ButtonKind::Primary, true) => Look {
            ink: c.on_accent.to_hsla(),
            fill: Some(c.blue_hover.to_hsla()),
            outline: c.blue_hover.to_hsla(),
        },
        (ButtonKind::Ghost | ButtonKind::Icon, false) => Look {
            ink: c.text_2.to_hsla(),
            fill: None,
            outline: clear,
        },
        (ButtonKind::Ghost | ButtonKind::Icon, true) => Look {
            ink: c.text.to_hsla(),
            fill: Some(c.hover.to_hsla()),
            outline: clear,
        },
        (ButtonKind::Danger, false) => Look {
            ink: c.red.to_hsla(),
            fill: None,
            outline: c.red_line.to_hsla(),
        },
        (ButtonKind::Danger, true) => Look {
            ink: c.on_red.to_hsla(),
            fill: Some(c.red.to_hsla()),
            outline: c.red.to_hsla(),
        },
    }
}

/// `text` as a label with its access key shown: underlined in the label, or as a `(K)` suffix.
/// `id`, when given, is every text piece's ID (`#id` in the rendered-text records).
// ## m2b-t24-shots (follow-up): the `id` parameter.
pub(crate) fn key_label(
    text: &SharedString,
    key: Option<AccessKey>,
    style: TypeStyle,
    color: Hsla,
    id: Option<&ElementId>,
) -> AnyElement {
    key_label_from(text, TextSource::Fluent, key, style, color, id)
}

// ## m2b-t24-shots
/// [`key_label`] for a text from `source` (data, such as a host alias in a menu).
pub(crate) fn key_label_from(
    text: &SharedString,
    source: TextSource,
    key: Option<AccessKey>,
    style: TypeStyle,
    color: Hsla,
    id: Option<&ElementId>,
) -> AnyElement {
    let plain = |s: SharedString| {
        let label = Label::new(s, source, style, color);
        match id {
            Some(id) => label.id(id.clone()),
            None => label,
        }
    };
    match key {
        Some(AccessKey::InLabel(at)) if at < text.len() && text.is_char_boundary(at) => {
            let key_len = text[at..].chars().next().map_or(0, char::len_utf8);
            let (before, rest) = text.split_at(at);
            let (letter, after) = rest.split_at(key_len);
            div()
                .flex()
                .flex_row()
                .flex_none()
                .child(plain(before.to_string().into()))
                .child(div().underline().child(plain(letter.to_string().into())))
                .child(plain(after.to_string().into()))
                .into_any_element()
        }
        Some(AccessKey::Suffix(c)) => div()
            .flex()
            .flex_row()
            .flex_none()
            .gap(px(4.0))
            .child(plain(text.clone()))
            .child(Label::new(
                format!("({c})"),
                TextSource::Shortcut,
                style,
                color,
            ))
            .into_any_element(),
        _ => plain(text.clone()).into_any_element(),
    }
}

/// Fully transparent (an outline that keeps its space but shows nothing).
pub(crate) const TRANSPARENT: Hsla = Hsla {
    h: 0.0,
    s: 0.0,
    l: 0.0,
    a: 0.0,
};

/// Whether `handle` shows its focus ring: focused by keyboard navigation, not by a click.
pub(crate) fn focus_visible(handle: &FocusHandle, window: &Window) -> bool {
    handle.is_focused(window) && window.last_input_was_keyboard()
}

/// The hover flag and focus handle a widget keeps across frames, keyed by its element id.
pub(crate) fn interaction_state(
    id: &ElementId,
    window: &mut Window,
    cx: &mut App,
) -> (Entity<bool>, FocusHandle) {
    let hover = window.use_keyed_state(ElementId::from((id.clone(), "hover")), cx, |_, _| false);
    let focus = window.use_keyed_state(ElementId::from((id.clone(), "focus")), cx, |_, cx| {
        cx.focus_handle()
    });
    let handle = focus.read(cx).clone();
    (hover, handle)
}

/// An `on_hover` listener that records the pointer state in `hover` and repaints on change.
pub(crate) fn hover_listener(
    hover: Entity<bool>,
) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
    move |hovered, _, cx| {
        hover.update(cx, |state, cx| {
            if *state != *hovered {
                *state = *hovered;
                cx.notify();
            }
        });
    }
}

impl IntoElement for Button {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |window, cx| self.build(window, cx)).into_any_element()
    }
}

impl Button {
    fn build(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let Button {
            id,
            label,
            kind,
            icon,
            key_hint,
            disabled,
            on_click,
            focus,
            access_key,
            fixed_width,
            inert,
            on_raised,
            expanded,
        } = self;

        // Hover and focus state live with the element across frames.
        let (hover, keyed_focus) = interaction_state(&id, window, cx);
        let handle = focus.unwrap_or(keyed_focus);
        let focused = !disabled && focus_visible(&handle, window);
        let hot =
            !disabled && !inert && (*hover.read(cx) || (kind == ButtonKind::Danger && focused));
        let look = look(kind, disabled, hot, &theme);

        let mut style = t.type_ramp.control;
        if kind == ButtonKind::Primary {
            style.weight = 600;
        }
        let icon_only = kind == ButtonKind::Icon;
        let selector = format!("button-{id}");
        let node_id = ElementId::from((id.clone(), "node"));
        // ## m2b-t24-shots (follow-up): the label's ID (`button-label-<id>`), so a test or a
        // shot can name one button's text.
        let label_id = ElementId::Name(format!("button-label-{id}").into());

        let mut body = div()
            .id(id)
            .debug_selector(move || selector)
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .gap(px(t.spacing.sm))
            .h(px(t.spacing.control_height))
            .px(px(if fixed_width.is_some() {
                0.0
            } else if icon_only {
                6.0
            } else {
                9.0
            }))
            .rounded(px(t.radii.sm))
            .border_1()
            .border_color(look.outline);
        if let Some(width) = fixed_width {
            body = body.w(px(width)).justify_center();
        }
        if let Some(fill) = look.fill {
            body = body.bg(fill);
        }
        if let Some(name) = icon {
            body = body.child(Icon::new(name).size(IconSize::Chrome).color(look.ink));
        }
        if !icon_only {
            body = body.child(key_label(
                &label,
                access_key,
                style,
                look.ink,
                Some(&label_id),
            ));
        }
        if let Some(hint) = key_hint {
            // A primary's hint takes its own ink (on-accent) on every surface: text-3 on its
            // blue fill is unreadable (m2a-t21-shots). On a raised surface the others use
            // text-2 (never text-3).
            let dim = match (on_raised, kind) {
                (_, ButtonKind::Primary) => look.ink,
                (true, _) => t.colors.text_2.to_hsla(),
                (false, _) => t.colors.text_3.to_hsla(),
            };
            body = body.child(Label::new(
                hint,
                TextSource::Shortcut,
                t.type_ramp.mono_kbd,
                dim,
            ));
        }

        if disabled || inert {
            return A11yNode::new(node_id, Role::Button, label, false, body)
                .disabled(true)
                .into_any_element();
        }
        // ## m2b-t24-shots: a popup trigger says whether its popup is open.
        let mut decl = A11yDecl::new(Role::Button, label.clone());
        decl.expanded = expanded;
        // ## m2b-t24-shots (fix round 1, I2): a following scroll container keeps it in view.
        let body = body
            .relative()
            .child(crate::follow::focus_probe(handle.clone()));
        let mut body = focus_outline(body.track_focus(&handle).tab_stop(true), &theme, focused)
            .declare_full(decl)
            .cursor_pointer()
            .on_hover(hover_listener(hover));
        if let Some(handler) = on_click {
            body = body.on_click(move |event, window, cx| handler(event, window, cx));
        }
        if icon_only {
            let tip = label;
            body = body.tooltip(move |_, cx| Tip::view(tip.clone(), cx));
        }
        body.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_ui_core::theme::{DARK, ThemeMode};

    fn theme() -> Theme {
        Theme::new(ThemeMode::Dark)
    }

    #[test]
    fn default_button_is_outlined_and_fills_on_hover() {
        let rest = look(ButtonKind::Default, false, false, &theme());
        assert_eq!(rest.fill, None);
        assert_eq!(rest.outline, DARK.colors.seam_strong.to_hsla());
        let hot = look(ButtonKind::Default, false, true, &theme());
        assert_eq!(hot.fill, Some(DARK.colors.hover.to_hsla()));
    }

    #[test]
    fn primary_swaps_fill_and_border_to_blue_hover() {
        let hot = look(ButtonKind::Primary, false, true, &theme());
        assert_eq!(hot.fill, Some(DARK.colors.blue_hover.to_hsla()));
        assert_eq!(hot.outline, DARK.colors.blue_hover.to_hsla());
        assert_eq!(hot.ink, DARK.colors.on_accent.to_hsla());
    }

    #[test]
    fn danger_is_red_outline_at_rest_and_red_fill_when_hot() {
        let rest = look(ButtonKind::Danger, false, false, &theme());
        assert_eq!(
            (rest.ink, rest.fill, rest.outline),
            (
                DARK.colors.red.to_hsla(),
                None,
                DARK.colors.red_line.to_hsla()
            )
        );
        let hot = look(ButtonKind::Danger, false, true, &theme());
        assert_eq!(hot.ink, DARK.colors.on_red.to_hsla());
        assert_eq!(hot.fill, Some(DARK.colors.red.to_hsla()));
    }

    #[test]
    fn disabled_buttons_have_text_3_ink_no_fill_and_no_hover_response() {
        for kind in [
            ButtonKind::Default,
            ButtonKind::Primary,
            ButtonKind::Danger,
            ButtonKind::Ghost,
            ButtonKind::Icon,
        ] {
            let rest = look(kind, true, false, &theme());
            assert_eq!(rest.ink, DARK.colors.text_3.to_hsla(), "{kind:?}");
            assert_eq!(rest.fill, None, "{kind:?}");
        }
        assert_eq!(
            look(ButtonKind::Danger, true, false, &theme()).outline,
            DARK.colors.seam_strong.to_hsla(),
            "a disabled danger button keeps a plain outline"
        );
        assert_eq!(
            look(ButtonKind::Ghost, true, false, &theme()).outline.a,
            0.0,
            "a disabled ghost button has no outline"
        );
    }
}
