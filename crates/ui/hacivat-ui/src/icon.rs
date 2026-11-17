//! Lucide icons at the two design sizes (DESIGN.md Components: 12 px inside chrome and
//! controls, 14 px by default). The files are bundled by `hacivat_ui_core::assets`.

use std::cell::RefCell;

use hacivat_ui_core::gpui_surface::{AnyElement, Hsla, IntoElement, Styled, px, svg};
use hacivat_ui_core::theme::Theme;

use crate::popover::lazy;

/// Declares [`IconName`], its bundled paths and [`IconName::ALL`] from one list, so an icon
/// cannot be added to one of them and missed by another (## m3b-t01b-seams fix round 2, M1).
macro_rules! icon_names {
    ($($(#[$meta:meta])* $name:ident => $path:literal,)*) => {
        /// The icons the chrome widgets draw.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum IconName {
            $($(#[$meta])* $name,)*
        }

        impl IconName {
            /// Every icon, in declaration order (the app's test loads each one's asset).
            pub const ALL: &'static [IconName] = &[$(IconName::$name,)*];

            /// The bundled asset path.
            pub fn path(self) -> &'static str {
                match self {
                    $(IconName::$name => $path,)*
                }
            }
        }
    };
}

icon_names! {
    ArrowLeftRight => "icons/lucide/arrow-left-right.svg",
    Check => "icons/lucide/check.svg",
    ChevronDown => "icons/lucide/chevron-down.svg",
    ChevronRight => "icons/lucide/chevron-right.svg",
    ChevronsUpDown => "icons/lucide/chevrons-up-down.svg",
    CircleCheck => "icons/lucide/circle-check.svg",
    Ellipsis => "icons/lucide/ellipsis.svg",
    FileCode => "icons/lucide/file-code-2.svg",
    GitCommit => "icons/lucide/git-commit-horizontal.svg",
    Info => "icons/lucide/info.svg",
    Languages => "icons/lucide/languages.svg",
    LayoutPanelLeft => "icons/lucide/layout-panel-left.svg",
    Lock => "icons/lucide/lock.svg",
    Menu => "icons/lucide/menu.svg",
    Minus => "icons/lucide/minus.svg",
    Moon => "icons/lucide/moon.svg",
    Plus => "icons/lucide/plus.svg",
    Search => "icons/lucide/search.svg",
    Square => "icons/lucide/square.svg",
    Sun => "icons/lucide/sun.svg",
    TriangleAlert => "icons/lucide/triangle-alert.svg",
    X => "icons/lucide/x.svg",
    // M2a connections (Task 1).
    Database => "icons/lucide/database.svg",
    LockOpen => "icons/lucide/lock-open.svg",
    RotateCw => "icons/lucide/rotate-cw.svg",
    Copy => "icons/lucide/copy.svg",
    ClipboardPaste => "icons/lucide/clipboard-paste.svg",
    Plug => "icons/lucide/plug.svg",
    Unplug => "icons/lucide/unplug.svg",
    Pencil => "icons/lucide/pencil.svg",
    Trash2 => "icons/lucide/trash-2.svg",
    ExternalLink => "icons/lucide/external-link.svg",
    Route => "icons/lucide/route.svg",
    CircleAlert => "icons/lucide/circle-alert.svg",
    CircleX => "icons/lucide/circle-x.svg",
    FolderOpen => "icons/lucide/folder-open.svg",
    Undo2 => "icons/lucide/undo-2.svg",
    FileKey => "icons/lucide/file-key.svg",
    // ## m2b-t24-shots: the M2b connection icons (T19 M6, T20 M1c).
    FileUp => "icons/lucide/file-up.svg",
    KeyRound => "icons/lucide/key-round.svg",
    ArrowRight => "icons/lucide/arrow-right.svg",
    Fingerprint => "icons/lucide/fingerprint.svg",
    // ## m3a-t01b-seams: the M3a query icons (design/m3a-query/NOTES.md "Icons"; Tasks 27, 28,
    // 30 draw them).
    ScrollText => "icons/lucide/scroll-text.svg",
    SkipForward => "icons/lucide/skip-forward.svg",
    ArrowDownToLine => "icons/lucide/arrow-down-to-line.svg",
    Eraser => "icons/lucide/eraser.svg",
    ClipboardX => "icons/lucide/clipboard-x.svg",
    Ban => "icons/lucide/ban.svg",
    CopyCheck => "icons/lucide/copy-check.svg",
    // ## m3a-t30-results-view: a result tab (NOTES surface 9) and "more result sets" (surface 11);
    // both SVGs were already bundled.
    Table2 => "icons/lucide/table-2.svg",
    Layers => "icons/lucide/layers.svg",
    // ## m3b-t01b-seams (P-M3b-37): Run (Task 41), pinned tabs and history rows, Count rows,
    // filter rows, sort and frozen columns. `Search` and `ChevronsUpDown` are above.
    Play => "icons/lucide/play.svg",
    Pin => "icons/lucide/pin.svg",
    Sigma => "icons/lucide/sigma.svg",
    Filter => "icons/lucide/filter.svg",
    ArrowUpDown => "icons/lucide/arrow-up-down.svg",
    Snowflake => "icons/lucide/snowflake.svg",
    // ## m3b-t01b-seams (controller ruling RC-4): the M3b mocks' icons.
    History => "icons/lucide/history.svg",
    Timer => "icons/lucide/timer.svg",
    Hourglass => "icons/lucide/hourglass.svg",
    Users => "icons/lucide/users.svg",
    EyeOff => "icons/lucide/eye-off.svg",
    OctagonX => "icons/lucide/octagon-x.svg",
    Clock => "icons/lucide/clock.svg",
    // ## m3c-t31-multi (controller ruling 8): the M3c icons (Tasks 33 and 36 draw them).
    ArrowUp => "icons/lucide/arrow-up.svg",
    CaseSensitive => "icons/lucide/case-sensitive.svg",
    WholeWord => "icons/lucide/whole-word.svg",
    Regex => "icons/lucide/regex.svg",
    // ## m3c-t36: the descending sort arrow (the SVG ships since M2; the variant is new).
    // ## m3c-t33-find (named exception): also the find bar's next match (already bundled).
    ArrowDown => "icons/lucide/arrow-down.svg",
    // ## m3c-t38 (named exception; NOTES M3c icons "reuses shipped glyphs"): the view switch
    // and the record toggle (the SVGs ship since M2; the variants are new).
    TableProperties => "icons/lucide/table-properties.svg",
    AlignLeft => "icons/lucide/align-left.svg",
    PanelRight => "icons/lucide/panel-right.svg",
    /// The Hacivat mark (`icons/hacivat/hv-mark.svg`).
    HvMark => "icons/hacivat/hv-mark.svg",
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconSize {
    /// 12 px: inside chrome and controls.
    Chrome,
    /// 16 px: the title-bar brand mark.
    Brand,
    /// 14 px.
    #[default]
    Default,
}

impl IconSize {
    pub fn px(self) -> f32 {
        match self {
            IconSize::Chrome => 12.0,
            IconSize::Brand => 16.0,
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

/// One icon a widget drew (recorded between [`icon_record`] and [`icon_take`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawnIcon {
    pub name: IconName,
    pub color: Hsla,
    pub size_px: f32,
}

thread_local! {
    static DRAWN: RefCell<Option<Vec<DrawnIcon>>> = const { RefCell::new(None) };
}

/// Starts (or restarts) recording every icon drawn on this thread. GPUI's test platform does
/// not rasterize SVGs, so tests assert on what the widgets asked for: the glyph and its ink.
pub fn icon_record() {
    DRAWN.with(|d| *d.borrow_mut() = Some(Vec::new()));
}

/// Stops recording and returns the icons drawn since [`icon_record`], in drawing order.
pub fn icon_take() -> Vec<DrawnIcon> {
    DRAWN.with(|d| d.borrow_mut().take().unwrap_or_default())
}

fn icon_note(icon: DrawnIcon) {
    DRAWN.with(|d| {
        if let Some(log) = d.borrow_mut().as_mut() {
            log.push(icon);
        }
    });
}

impl IntoElement for Icon {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |_, cx| {
            let color = self
                .color
                .unwrap_or_else(|| Theme::get(cx).tokens.colors.text_2.to_hsla());
            icon_note(DrawnIcon {
                name: self.name,
                color,
                size_px: self.size.px(),
            });
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

    const ALL: [IconName; 52] = [
        IconName::ArrowLeftRight,
        IconName::Check,
        IconName::ChevronDown,
        IconName::ChevronRight,
        IconName::ChevronsUpDown,
        IconName::CircleCheck,
        IconName::Ellipsis,
        IconName::FileCode,
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
        IconName::Database,
        IconName::LockOpen,
        IconName::RotateCw,
        IconName::Copy,
        IconName::ClipboardPaste,
        IconName::Plug,
        IconName::Unplug,
        IconName::Pencil,
        IconName::Trash2,
        IconName::ExternalLink,
        IconName::Route,
        IconName::CircleAlert,
        IconName::CircleX,
        IconName::FolderOpen,
        IconName::Undo2,
        IconName::FileKey,
        // ## m2b-t24-shots
        IconName::FileUp,
        IconName::KeyRound,
        IconName::ArrowRight,
        IconName::Fingerprint,
        // ## m3a-t01b-seams
        IconName::ScrollText,
        IconName::SkipForward,
        IconName::ArrowDownToLine,
        IconName::Eraser,
        IconName::ClipboardX,
        IconName::Ban,
        IconName::CopyCheck,
        IconName::HvMark,
        // ## m3a-t30-results-view
        IconName::Table2,
        IconName::Layers,
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

    // ## m3b-t01b-seams (controller ruling RC-4)
    /// `IconName::ALL` comes from the same list as the enum (`icon_names!`): every variant once,
    /// in declaration order, so the app's asset test covers every icon.
    #[test]
    fn all_lists_every_icon_in_declaration_order() {
        for (i, n) in IconName::ALL.iter().enumerate() {
            assert_eq!(*n as usize, i, "{n:?}");
        }
        // ## m3c-t31-multi (named exception): four more (controller ruling 8).
        // ## m3c-t36 (named exception): and `ArrowDown`, the descending sort arrow.
        // ## m3c-t33-find (named exception): the find bar uses the same `ArrowDown` (one variant).
        // ## m3c-t38 (named exception): the view switch's two and the record toggle's.
        assert_eq!(IconName::ALL.len(), 73);
    }
}
