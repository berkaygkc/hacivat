//! The 40 px title bar (DESIGN.md): padding `0 10px 0 14px`, the traffic-light inset on macOS,
//! the brand, the connection [`Capsule`], a centred palette field, trailing controls and, on
//! Windows and Linux, the app menu and the window controls.
//!
//! The bar is draggable (`start_window_move`) and a double click zooms; only its inert regions
//! (inset, brand, spacers) start a drag so buttons keep their clicks. The Overflow Rule is
//! [`titlebar_fit`]: trailing segments collapse into an overflow button before the palette
//! label truncates.

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ClickEvent, Div, ElementId, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Role, SharedString, StatefulInteractiveElement, Styled, Window,
    WindowControlArea, div, px,
};
use hacivat_ui_core::theme::{Caps, Theme, TypeStyle};
use hacivat_ui_core::window::{ChromeKind, TRAFFIC_LIGHTS_INSET_PX};

use crate::button::{Button, ButtonKind, ClickHandler, hover_listener, interaction_state};
use crate::capsule::Capsule;
use crate::icon::{Icon, IconName, IconSize};
use crate::l10n::ui_text;
use crate::popover::Declare;
use crate::popover::lazy;
use crate::text::{Label, TextSource};

/// Width of the overflow button that takes the trailing segments.
pub const OVERFLOW_BUTTON_PX: f32 = 24.0;
/// The palette field's preferred and minimum widths.
pub const PALETTE_PREF_PX: f32 = 360.0;
pub const PALETTE_MIN_PX: f32 = 220.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TitleBarFit {
    /// The trailing segments live behind the overflow button.
    pub segments_in_overflow: bool,
    pub palette_width: f32,
    /// The palette label ellipsizes (its width fell below the minimum).
    pub palette_truncates: bool,
}

/// The Overflow Rule (F13). With `available = width - fixed`: when the preferred palette and
/// all segments fit, nothing changes. Otherwise the segments move behind a 24 px overflow
/// button and the palette takes what is left, up to its preferred width; its label truncates
/// only when that falls below `palette_min`.
pub fn titlebar_fit(
    width: f32,
    fixed: f32,
    palette_pref: f32,
    palette_min: f32,
    segments: f32,
) -> TitleBarFit {
    let available = width - fixed;
    if available >= palette_pref + segments {
        return TitleBarFit {
            segments_in_overflow: false,
            palette_width: palette_pref,
            palette_truncates: false,
        };
    }
    let palette_width = palette_pref.min(available - OVERFLOW_BUTTON_PX).max(0.0);
    TitleBarFit {
        segments_in_overflow: true,
        palette_width,
        palette_truncates: palette_width < palette_min,
    }
}

struct PaletteField {
    placeholder: SharedString,
    keys: SharedString,
    on_click: ClickHandler,
}

/// What the Overflow Rule needs to know (the bar cannot measure its own children).
struct FitInputs {
    /// Width of everything that never moves: brand, capsule, paddings, window controls.
    fixed: f32,
    /// Combined width of the trailing segments.
    segments: f32,
    on_overflow: ClickHandler,
}

pub struct TitleBar {
    chrome: ChromeKind,
    brand: Option<SharedString>,
    capsule: Option<Capsule>,
    palette: Option<PaletteField>,
    right: Vec<AnyElement>,
    app_menu: Option<ClickHandler>,
    fit: Option<FitInputs>,
}

impl TitleBar {
    pub fn new(chrome: ChromeKind) -> Self {
        Self {
            chrome,
            brand: None,
            capsule: None,
            palette: None,
            right: Vec::new(),
            app_menu: None,
            fit: None,
        }
    }

    /// The product word, pre-cased for small caps (`cm-brand-hacivat-caps`).
    pub fn brand(mut self, word_caps: SharedString) -> Self {
        self.brand = Some(word_caps);
        self
    }

    pub fn capsule(mut self, capsule: Capsule) -> Self {
        self.capsule = Some(capsule);
        self
    }

    pub fn palette_field(
        mut self,
        placeholder: SharedString,
        keys: SharedString,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.palette = Some(PaletteField {
            placeholder,
            keys,
            on_click: Rc::new(on_click),
        });
        self
    }

    /// The trailing segments (view switcher, language and theme toggles).
    pub fn right(mut self, segments: Vec<AnyElement>) -> Self {
        self.right = segments;
        self
    }

    /// The hamburger that opens the application menu (Windows and Linux).
    pub fn app_menu(
        mut self,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.app_menu = Some(Rc::new(on_click));
        self
    }

    /// Enables the Overflow Rule: with `fixed` px of unmovable content and `segments` px of
    /// trailing segments, a narrow window replaces the segments by an overflow button that
    /// calls `on_overflow` (the host opens a popup menu of the segments' actions).
    pub fn overflow(
        mut self,
        fixed: f32,
        segments: f32,
        on_overflow: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.fit = Some(FitInputs {
            fixed,
            segments,
            on_overflow: Rc::new(on_overflow),
        });
        self
    }
}

/// Makes `el` start a window drag on press and zoom on a double click.
fn drag_region(el: Div) -> Div {
    el.window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count >= 2 {
                if cfg!(target_os = "macos") {
                    window.titlebar_double_click();
                } else {
                    window.zoom_window();
                }
            } else {
                window.start_window_move();
            }
        })
}

impl IntoElement for TitleBar {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |window, cx| self.build(window, cx)).into_any_element()
    }
}

impl TitleBar {
    fn build(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let chrome = self.chrome;
        let width = f32::from(window.viewport_size().width);
        let fit = self
            .fit
            .as_ref()
            .map(|f| titlebar_fit(width, f.fixed, PALETTE_PREF_PX, PALETTE_MIN_PX, f.segments));

        let mut bar = div()
            .debug_selector(|| "title-bar".to_string())
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(t.spacing.titlebar_height))
            .flex_none()
            .pl(px(t.spacing.s_2xl))
            .pr(px(t.spacing.lg))
            .border_b_1()
            .border_color(c.seam.to_hsla())
            .bg(c.ground.to_hsla());

        if chrome == ChromeKind::MacTrafficLights {
            bar = bar.child(drag_region(
                div().w(px(TRAFFIC_LIGHTS_INSET_PX)).h_full().flex_none(),
            ));
        }
        if let Some(word) = self.brand {
            let style = TypeStyle {
                size_px: 14.5,
                weight: 600,
                line_height: 14.5 * 1.35,
                letter_spacing_px: 14.5 * 0.09,
                caps: Caps::SynthesizedSmallCaps,
                ..t.type_ramp.body
            };
            bar = bar
                .child(drag_region(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(t.spacing.md))
                        .pr(px(t.spacing.s_2xl))
                        .child(
                            Icon::new(IconName::HvMark)
                                .size(IconSize::Brand)
                                .color(c.text_2.to_hsla()),
                        )
                        .child(Label::new(
                            word,
                            TextSource::Fluent,
                            style,
                            c.text.to_hsla(),
                        )),
                ))
                .child(
                    div()
                        .w(px(1.0))
                        .h(px(18.0))
                        .flex_none()
                        .bg(c.seam.to_hsla()),
                );
        }
        if let Some(capsule) = self.capsule {
            bar = bar.child(div().ml(px(t.spacing.xl)).child(capsule));
        }
        bar = bar.child(drag_region(div().flex_1().h_full()));

        if let Some(palette) = self.palette {
            let truncates = fit.is_some_and(|f| f.palette_truncates);
            let w = fit.map_or(PALETTE_PREF_PX, |f| f.palette_width);
            let mut label_style = t.type_ramp.body;
            label_style.size_px = label_style.size_px.max(11.0);
            let on_click = palette.on_click;
            let field = div()
                .id(ElementId::from("palette-field"))
                .debug_selector(|| "palette-field".to_string())
                .declare(Role::Button, palette.placeholder.clone(), None)
                .flex()
                .flex_row()
                .items_center()
                .flex_none()
                .w(px(w))
                .h(px(t.spacing.field_height))
                .pl(px(9.0))
                .pr(px(t.spacing.sm))
                .gap(px(t.spacing.md))
                .border_1()
                .border_color(c.seam.to_hsla())
                .rounded(px(t.radii.sm))
                .cursor_pointer()
                .on_click(move |event, window, cx| on_click(event, window, cx))
                .child(
                    Icon::new(IconName::Search)
                        .size(IconSize::Chrome)
                        .color(c.text_3.to_hsla()),
                )
                .child(
                    div().flex_1().min_w(px(0.0)).child(
                        Label::new(
                            palette.placeholder,
                            TextSource::Fluent,
                            label_style,
                            c.text_3.to_hsla(),
                        )
                        .truncate(truncates || fit.is_some()),
                    ),
                )
                .child(Label::new(
                    palette.keys,
                    TextSource::Shortcut,
                    t.type_ramp.mono_kbd,
                    c.text_3.to_hsla(),
                ));
            bar = bar.child(field).child(drag_region(div().flex_1().h_full()));
        }

        let overflowed = fit.is_some_and(|f| f.segments_in_overflow);
        if overflowed {
            if let Some(f) = &self.fit {
                let on_overflow = f.on_overflow.clone();
                bar = bar.child(
                    Button::new("titlebar-overflow", ui_text(cx, "hui-menu-more"))
                        .kind(ButtonKind::Icon)
                        .fixed_width(OVERFLOW_BUTTON_PX)
                        .icon(IconName::Ellipsis)
                        .on_click_rc(on_overflow),
                );
            }
        } else if !self.right.is_empty() {
            bar = bar.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.md))
                    .children(self.right),
            );
        }

        let controls = matches!(chrome, ChromeKind::WindowsCaption | ChromeKind::LinuxClient);
        if controls {
            if let Some(on_click) = self.app_menu {
                bar = bar.child(
                    div().ml(px(t.spacing.md)).child(
                        Button::new("titlebar-app-menu", ui_text(cx, "hui-menu-app"))
                            .kind(ButtonKind::Icon)
                            .icon(IconName::Menu)
                            .on_click_rc(on_click),
                    ),
                );
            }
            bar = bar.child(caption_buttons(chrome, window, cx));
        }
        bar.into_any_element()
    }
}

/// Minimize, maximize and close. On Windows the platform handles them through
/// `WindowControlArea`; on Linux (client decorations) the click handlers act.
fn caption_buttons(chrome: ChromeKind, window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let specs = [
        (
            "window-minimize",
            "hui-window-minimize",
            IconName::Minus,
            WindowControlArea::Min,
        ),
        (
            "window-maximize",
            "hui-window-maximize",
            IconName::Square,
            WindowControlArea::Max,
        ),
        (
            "window-close",
            "hui-window-close",
            IconName::X,
            WindowControlArea::Close,
        ),
    ];
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .h_full()
        .ml(px(t.spacing.md));
    for (key, text_id, icon, area) in specs {
        let id = ElementId::from(key);
        let (hover, _) = interaction_state(&id, window, cx);
        let hovered = *hover.read(cx);
        let close = area == WindowControlArea::Close;
        let (fill, ink) = match (hovered, close) {
            (true, true) => (Some(c.red), c.on_red),
            (true, false) => (Some(c.hover), c.text),
            (false, _) => (None, c.text_2),
        };
        let mut button = div()
            .id(id)
            .debug_selector(move || key.to_string())
            .declare(Role::Button, ui_text(cx, text_id), None)
            .flex()
            .items_center()
            .justify_center()
            .w(px(46.0))
            .h_full()
            .flex_none()
            .window_control_area(area)
            .on_hover(hover_listener(hover))
            .child(Icon::new(icon).size(IconSize::Chrome).color(ink.to_hsla()));
        if let Some(fill) = fill {
            button = button.bg(fill.to_hsla());
        }
        if chrome == ChromeKind::LinuxClient {
            button = button.on_click(move |_, window, _| match area {
                WindowControlArea::Min => window.minimize_window(),
                WindowControlArea::Max => window.zoom_window(),
                _ => window.remove_window(),
            });
        }
        row = row.child(button);
    }
    row.into_any_element()
}
