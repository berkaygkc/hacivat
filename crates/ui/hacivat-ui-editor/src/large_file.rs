//! Large-file tiers (UI-EDIT-23 T0/T1, D-071; Task 29).
//!
//! - **T0** (full): up to 4 MB with no line longer than [`LONG_LINE_MAX_BYTES`] (64 KB).
//! - **T1** ("Reduced mode"): anything larger, up to 20 MB. The view shows the inline banner
//!   above the text ([`banner`]: NOTES §7, the inline-notice anatomy at padding 6/10), the
//!   status segment reads `EditorStatus::tier`, the editor's accessible description carries the
//!   banner's text ([`description`]), and the screen-reader window does not grow
//!   (`a11y::declaration`). The host hears `EditorEvent::TierChanged` and passes the tier to
//!   the language service in `DocContext` (viewport-only lexing).
//! - **Back to T0** only below 80 % of each limit that put the document in T1, and within
//!   the other limit (no flapping when typing at the limit). [`TierState`] records the causes
//!   (`by_size`, `by_line`, accumulated while in T1); the banner's copy follows them.
//! - **Nothing above 20 MB** enters the editor in M3a (R-M3-49: the paste guard and the console
//!   restore refuse it); a document that got there anyway stays in T1, as there is no T2 before
//!   M4b.
//!
//! The view keeps the longest line from the lines each edit touched (it only grows during
//! editing; `set_text` measures it again), so a T1 entered by a long line ends when the text is
//! replaced.

use std::rc::Rc;

use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::l10n::UiArg;
use hacivat_ui::text::{TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Global, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};

use crate::commands::message;
use crate::config::{Tier, TierLimits};
use crate::view::EditorView;

/// The longest line T0 takes (UI-EDIT-23 step 1: 64 KB).
pub const LONG_LINE_MAX_BYTES: u64 = 64 << 10;
/// A document returns to T0 only below this share of a limit, in percent.
pub const RETURN_BELOW_PERCENT: u64 = 80;

/// The gap between the notice's icon and its text (the inline notice's 9 px, `notice.rs`).
const ICON_GAP_PX: f32 = 9.0;
/// Drops the icon onto the first line's cap height (the inline notice's 1 px).
const ICON_NUDGE_PX: f32 = 1.0;

/// The module-owned state (P11): the tier, and what the banner says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TierState {
    pub tier: Tier,
    /// The document's size, in bytes.
    pub bytes: u64,
    /// Its longest line, in bytes, as the view measures it.
    pub longest_line: u64,
    pub limits: TierLimits,
    /// In T1 because of the size (accumulated while in T1).
    pub by_size: bool,
    /// In T1 because of a line longer than 64 KB (accumulated while in T1).
    pub by_line: bool,
}

/// Formats a document size for the T1 banner in the format locale (L3; "18,4 MB" in Turkey).
/// The app installs one (Task 32); without it the size is `hui-editor-t1-size` with the
/// megabytes to one decimal, unformatted.
#[derive(Clone)]
pub struct SizeText(pub SizeFn);

/// Bytes to their display text.
pub type SizeFn = Rc<dyn Fn(u64, &App) -> SharedString>;

impl Global for SizeText {}

/// `value` is below `percent` % of `limit` (exactly, in integers).
fn below(value: u64, limit: u64, percent: u64) -> bool {
    u128::from(value) * 100 < u128::from(limit) * u128::from(percent)
}

/// The tier of a document of `bytes` whose longest line has `longest_line` bytes.
pub fn tier_for(bytes: u64, longest_line: u64, limits: &TierLimits) -> Tier {
    if bytes <= limits.full_max_bytes && longest_line <= LONG_LINE_MAX_BYTES {
        Tier::Full
    } else {
        Tier::Reduced
    }
}

/// The state of a document opened at `bytes` with its longest line.
pub fn open(bytes: u64, longest_line: u64, limits: &TierLimits) -> TierState {
    TierState {
        tier: tier_for(bytes, longest_line, limits),
        bytes,
        longest_line,
        limits: *limits,
        by_size: bytes > limits.full_max_bytes,
        by_line: longest_line > LONG_LINE_MAX_BYTES,
    }
}

/// The tier after a change; `Some` when it moved. Up to T1 as soon as a limit is passed (the
/// causes accumulate while in T1); back to T0 once every limit that caused T1 is below 80 % of
/// itself and every other limit is within itself (## m3a-t29 fix round 1, M1).
pub fn update(
    s: &mut TierState,
    bytes: u64,
    longest_line: u64,
    limits: &TierLimits,
) -> Option<Tier> {
    s.bytes = bytes;
    s.longest_line = longest_line;
    s.limits = *limits;
    let over_size = bytes > limits.full_max_bytes;
    let over_line = longest_line > LONG_LINE_MAX_BYTES;
    let tier = if s.tier == Tier::Full {
        if over_size || over_line {
            s.by_size = over_size;
            s.by_line = over_line;
            Tier::Reduced
        } else {
            Tier::Full
        }
    } else {
        s.by_size |= over_size;
        s.by_line |= over_line;
        let back = !over_size
            && !over_line
            && (!s.by_size || below(bytes, limits.full_max_bytes, RETURN_BELOW_PERCENT))
            && (!s.by_line || below(longest_line, LONG_LINE_MAX_BYTES, RETURN_BELOW_PERCENT));
        if back {
            s.by_size = false;
            s.by_line = false;
            Tier::Full
        } else {
            s.tier
        }
    };
    (tier != s.tier).then(|| {
        s.tier = tier;
        tier
    })
}

/// The editor's tier state.
pub fn state(v: &EditorView) -> &TierState {
    &v.tier
}

/// `bytes` as the banner shows it.
fn size_text(bytes: u64, cx: &App) -> SharedString {
    if let Some(f) = cx.try_global::<SizeText>() {
        return (f.0)(bytes, cx);
    }
    let tenths = (u128::from(bytes) * 10 + (1 << 19)) >> 20;
    let n = format!("{}.{}", tenths / 10, tenths % 10);
    // The only use of the size message: Task 27's `hui-editor-size-mb` replaces it at merge
    // (review M3).
    message(cx, "hui-editor-t1-size", &[("n", UiArg::Text(n.into()))])
}

/// The size T1 ends below: 80 % of the T0 limit.
fn return_size(s: &TierState) -> u64 {
    s.limits.full_max_bytes * RETURN_BELOW_PERCENT / 100
}

/// The banner's two lines: what happened, and what it means.
fn lines(s: &TierState, cx: &App) -> Option<(SharedString, SharedString)> {
    if s.tier == Tier::Full {
        return None;
    }
    // ## m3a-t29 fix round 1 (M1): the copy follows the cause. The size title when the size
    // caused T1; the promise "it ends below 3.2 MB" only when the size alone did (a long line
    // also needs the text replaced, so the line's copy promises nothing).
    let by_size = s.by_size || !s.by_line;
    let title = if by_size {
        message(
            cx,
            "hui-editor-t1-title",
            &[("size", UiArg::Text(size_text(s.bytes, cx)))],
        )
    } else {
        message(cx, "hui-editor-t1-long-title", &[])
    };
    let body = if by_size && !s.by_line {
        message(
            cx,
            "hui-editor-t1-body",
            &[("size", UiArg::Text(size_text(return_size(s), cx)))],
        )
    } else {
        message(cx, "hui-editor-t1-long-body", &[])
    };
    Some((title, body))
}

/// The banner's text as the editor's accessible description (NOTES §7: the banner is not
/// focusable); `None` in T0.
pub fn description(s: &TierState, cx: &App) -> Option<SharedString> {
    lines(s, cx).map(|(title, body)| format!("{title} {body}").into())
}

/// The inline "Reduced mode" banner the view shows above the text in T1 (NOTES §7: the inline
/// notice, padding 6/10, radius 4, 1 px `seam-strong`, a `panel` / `ground` well, the 12 px
/// info glyph in `text-2`; the title in `text`, the body in `text-2`; 8/10/2/10 around it);
/// `None` in T0.
pub fn banner(s: &TierState, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let _ = window;
    let (title, body) = lines(s, cx)?;
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let well = match theme.mode {
        ThemeMode::Dark => c.panel,
        ThemeMode::Light => c.ground,
    };
    let style = t.type_ramp.body_sm;
    let text = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(paragraph(title, TextSource::Fluent, style, c.text.to_hsla()).id("editor-t1-title"))
        .child(paragraph(body, TextSource::Fluent, style, c.text_2.to_hsla()).id("editor-t1-body"));
    let notice = div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(ICON_GAP_PX))
        .px(px(t.spacing.lg))
        .py(px(t.spacing.sm))
        .rounded(px(t.radii.sm))
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .bg(well.to_hsla())
        .child(
            div().flex_none().mt(px(ICON_NUDGE_PX)).child(
                Icon::new(IconName::Info)
                    .color(c.text_2.to_hsla())
                    .size(IconSize::Chrome),
            ),
        )
        .child(text);
    Some(
        div()
            .debug_selector(|| "editor-t1-banner".to_string())
            .flex_none()
            .mt(px(t.spacing.md))
            .mx(px(t.spacing.lg))
            .mb(px(t.spacing.s_2xs))
            .child(notice)
            .into_any_element(),
    )
}
