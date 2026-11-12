//! The two shared surface treatments (F4, F26). Every floating surface uses
//! [`popover_surface`] and every focusable element uses [`focus_outline`]; no widget
//! re-implements either.

use gpui::{BoxShadow, Div, Styled, div, point, px};

use super::{ShadowLayer, Theme};

/// Width of the focus outline and of the gap between it and the element (DESIGN.md: "1px blue
/// outline at 1px offset").
pub const FOCUS_OUTLINE_PX: f32 = 1.0;

fn box_shadows(layers: &[ShadowLayer]) -> Vec<BoxShadow> {
    layers
        .iter()
        .map(|l| BoxShadow {
            color: l.color.to_hsla(),
            offset: point(px(l.x), px(l.y)),
            blur_radius: px(l.blur),
            spread_radius: px(l.spread),
            inset: false,
        })
        .collect()
}

/// A floating surface (popover, menu, palette, toast, dialog): `raised` fill, radius 6, a
/// `seam_strong` hairline border and the theme's `popover_shadow`. Only floating surfaces cast
/// that shadow (named rule).
pub fn popover_surface(theme: &Theme) -> Div {
    let t = theme.tokens;
    div()
        .bg(t.colors.raised.to_hsla())
        .border_1()
        .border_color(t.colors.seam_strong.to_hsla())
        .rounded(px(t.radii.md))
        .shadow(box_shadows(t.popover_shadow))
}

/// Draws the focus outline around `el` when `focused`: a 1 px `blue` ring at a 1 px offset.
///
/// GPUI has no outline primitive, so the ring is two hard drop shadows appended after the
/// element's own shadows: `blue` spread 2 px, then a 1 px gap in `ground` on top of it. The
/// element's background covers the inside; a transparent element shows `ground` there.
pub fn focus_outline<E: Styled>(mut el: E, theme: &Theme, focused: bool) -> E {
    if !focused {
        return el;
    }
    let c = &theme.tokens.colors;
    let ring = |color, spread| BoxShadow {
        color,
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(0.0),
        spread_radius: px(spread),
        inset: false,
    };
    let style = el.style();
    let mut shadows = style.box_shadow.take().unwrap_or_default();
    shadows.push(ring(c.blue.to_hsla(), 2.0 * FOCUS_OUTLINE_PX));
    shadows.push(ring(c.ground.to_hsla(), FOCUS_OUTLINE_PX));
    style.box_shadow = Some(shadows);
    el
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{DARK, LIGHT, ThemeMode};
    use gpui::{AbsoluteLength, Fill};

    #[test]
    fn popover_surface_is_raised_radius_6_seam_strong_with_popover_shadow() {
        for (mode, tokens) in [(ThemeMode::Dark, &DARK), (ThemeMode::Light, &LIGHT)] {
            let mut d = popover_surface(&Theme::new(mode));
            let s = d.style();
            assert_eq!(
                s.background,
                Some(Fill::from(tokens.colors.raised.to_hsla()))
            );
            assert_eq!(s.border_color, Some(tokens.colors.seam_strong.to_hsla()));
            assert_eq!(s.border_widths.top, Some(AbsoluteLength::from(px(1.0))));
            assert_eq!(s.corner_radii.top_left, Some(AbsoluteLength::from(px(6.0))));
            let shadows = s.box_shadow.clone().unwrap();
            assert_eq!(shadows.len(), tokens.popover_shadow.len());
            assert_eq!(shadows[0].blur_radius, px(tokens.popover_shadow[0].blur));
            assert_eq!(shadows[0].color, tokens.popover_shadow[0].color.to_hsla());
        }
    }

    #[test]
    fn focus_outline_adds_blue_ring_with_one_px_gap_only_when_focused() {
        let theme = Theme::new(ThemeMode::Dark);
        let mut plain = focus_outline(div(), &theme, false);
        assert_eq!(plain.style().box_shadow, None);

        let mut focused = focus_outline(div(), &theme, true);
        let shadows = focused.style().box_shadow.clone().unwrap();
        assert_eq!(shadows.len(), 2);
        assert_eq!(shadows[0].color, DARK.colors.blue.to_hsla());
        assert_eq!(shadows[0].spread_radius, px(2.0));
        assert_eq!(shadows[1].color, DARK.colors.ground.to_hsla());
        assert_eq!(shadows[1].spread_radius, px(1.0));
        assert!(shadows.iter().all(|s| s.blur_radius == px(0.0) && !s.inset));
    }

    #[test]
    fn focus_outline_keeps_existing_shadows_underneath() {
        let theme = Theme::new(ThemeMode::Dark);
        let mut el = focus_outline(popover_surface(&theme), &theme, true);
        let shadows = el.style().box_shadow.clone().unwrap();
        assert_eq!(shadows.len(), DARK.popover_shadow.len() + 2);
        assert_eq!(shadows.last().unwrap().color, DARK.colors.ground.to_hsla());
    }
}
