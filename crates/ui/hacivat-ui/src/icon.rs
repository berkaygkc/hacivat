//! Lucide icons at the two design sizes (DESIGN.md Components: 12 px inside chrome and
//! controls, 14 px by default). The files are bundled by `hacivat_ui_core::assets`.

use hacivat_ui_core::gpui_surface::{AnyElement, Hsla, IntoElement, Styled, px, svg};
use hacivat_ui_core::theme::Theme;

use crate::popover::lazy;

/// The icons the chrome widgets draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconName {
    ArrowLeftRight,
    Check,
    ChevronDown,
    ChevronRight,
    ChevronsUpDown,
    CircleCheck,
    Ellipsis,
    GitCommit,
    Info,
    Languages,
    LayoutPanelLeft,
    Lock,
    Menu,
    Minus,
    Moon,
    Plus,
    Search,
    Square,
    Sun,
    TriangleAlert,
    X,
    /// The Hacivat mark (`icons/hacivat/hv-mark.svg`).
    HvMark,
}

impl IconName {
    /// The bundled asset path.
    pub fn path(self) -> &'static str {
        match self {
            IconName::ArrowLeftRight => "icons/lucide/arrow-left-right.svg",
            IconName::Check => "icons/lucide/check.svg",
            IconName::ChevronDown => "icons/lucide/chevron-down.svg",
            IconName::ChevronRight => "icons/lucide/chevron-right.svg",
            IconName::ChevronsUpDown => "icons/lucide/chevrons-up-down.svg",
            IconName::CircleCheck => "icons/lucide/circle-check.svg",
            IconName::Ellipsis => "icons/lucide/ellipsis.svg",
            IconName::GitCommit => "icons/lucide/git-commit-horizontal.svg",
            IconName::Info => "icons/lucide/info.svg",
            IconName::Languages => "icons/lucide/languages.svg",
            IconName::LayoutPanelLeft => "icons/lucide/layout-panel-left.svg",
            IconName::Lock => "icons/lucide/lock.svg",
            IconName::Menu => "icons/lucide/menu.svg",
            IconName::Minus => "icons/lucide/minus.svg",
            IconName::Moon => "icons/lucide/moon.svg",
            IconName::Plus => "icons/lucide/plus.svg",
            IconName::Search => "icons/lucide/search.svg",
            IconName::Square => "icons/lucide/square.svg",
            IconName::Sun => "icons/lucide/sun.svg",
            IconName::TriangleAlert => "icons/lucide/triangle-alert.svg",
            IconName::X => "icons/lucide/x.svg",
            IconName::HvMark => "icons/hacivat/hv-mark.svg",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconSize {
    /// 12 px: inside chrome and controls.
    Chrome,
    /// 14 px.
    #[default]
    Default,
}

impl IconSize {
    pub fn px(self) -> f32 {
        match self {
            IconSize::Chrome => 12.0,
            IconSize::Default => 14.0,
        }
    }
}

/// A Lucide icon. Its colour is `text_2` unless set (icons carry meaning only when coloured).
#[derive(Debug, Clone, Copy)]
pub struct Icon {
    name: IconName,
    size: IconSize,
    color: Option<Hsla>,
}

impl Icon {
    pub fn new(name: IconName) -> Self {
        Self {
            name,
            size: IconSize::Default,
            color: None,
        }
    }

    pub fn size(mut self, size: IconSize) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl IntoElement for Icon {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |_, cx| {
            let color = self
                .color
                .unwrap_or_else(|| Theme::get(cx).tokens.colors.text_2.to_hsla());
            svg()
                .path(self.name.path())
                .size(px(self.size.px()))
                .flex_none()
                .text_color(color)
                .into_any_element()
        })
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_ui_core::assets::BundledAssets;
    use hacivat_ui_core::gpui_surface::AssetSource;

    const ALL: [IconName; 22] = [
        IconName::ArrowLeftRight,
        IconName::Check,
        IconName::ChevronDown,
        IconName::ChevronRight,
        IconName::ChevronsUpDown,
        IconName::CircleCheck,
        IconName::Ellipsis,
        IconName::GitCommit,
        IconName::Info,
        IconName::Languages,
        IconName::LayoutPanelLeft,
        IconName::Lock,
        IconName::Menu,
        IconName::Minus,
        IconName::Moon,
        IconName::Plus,
        IconName::Search,
        IconName::Square,
        IconName::Sun,
        IconName::TriangleAlert,
        IconName::X,
        IconName::HvMark,
    ];

    #[test]
    fn every_icon_is_a_bundled_asset() {
        for name in ALL {
            assert!(
                BundledAssets.load(name.path()).unwrap().is_some(),
                "{name:?} -> {}",
                name.path()
            );
        }
    }

    #[test]
    fn sizes_are_the_design_sizes() {
        assert_eq!(IconSize::Chrome.px(), 12.0);
        assert_eq!(IconSize::Default.px(), 14.0);
    }
}
