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
//! editing; `set_text` measures it again). ## m3a-f4-ime (EDIT-2): it never exceeds the
//! document (an emptied document ends a long-line T1 at once), and while a long line holds T1
//! the whole text is measured again off the UI thread once it has rested
//! [`REMEASURE_AFTER`] ([`after_edit`]), so deleting, shortening or splitting the line ends T1
//! (with the 80 % margin above); a measure of an older version is dropped.

use std::rc::Rc;

use hacivat_text::TextBuffer;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::l10n::UiArg;
use hacivat_ui::text::{TextSource, paragraph, scroll_viewport};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Context, Global, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels,
    SharedString, StatefulInteractiveElement, Styled, TextRun, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};

use crate::commands::message;
use crate::config::{Tier, TierLimits};
use crate::view::EditorView;

/// The longest line T0 takes (UI-EDIT-23 step 1: 64 KB).
pub const LONG_LINE_MAX_BYTES: u64 = 64 << 10;
/// A document returns to T0 only below this share of a limit, in percent.
pub const RETURN_BELOW_PERCENT: u64 = 80;
// ## m3a-f4-ime (EDIT-2)
/// How long the text rests after an edit before a long-line T1 measures its longest line again.
pub const REMEASURE_AFTER: std::time::Duration = std::time::Duration::from_millis(300);

/// The gap between the notice's icon and its text (the inline notice's 9 px, `notice.rs`).
const ICON_GAP_PX: f32 = 9.0;
/// Drops the icon onto the first line's cap height (the inline notice's 1 px).
const ICON_NUDGE_PX: f32 = 1.0;
// ## m3c-narrow (F11; design/m3b-safe surface 24)
/// An editor narrower than this draws the banner's narrow form ([`banner`]). A rounded design
/// line (fix round 1, review M4): at 180 px the wide form's margins (2 × 10), padding (2 × 10),
/// border (2), icon (12) and its gap (9) leave its text 117 px.
pub const NARROW_BELOW_PX: f32 = 180.0;

// ## m3c-narrow
/// Whether an editor `width` wide draws the banner's narrow form.
pub fn narrow(width: Pixels) -> bool {
    f32::from(width) < NARROW_BELOW_PX
}

// ## m3c-narrow (fix round 1, review M5)
/// Whether `s` shows the banner at all (T1): the narrow form, and the text's two-line minimum
/// beside it, apply only then.
pub fn shows_banner(s: &TierState) -> bool {
    s.tier != Tier::Full
}

// ## m3c-narrow (fix round 1, I1)
/// The narrow form's well around its title: padding 6 above and below, the 1 px border.
fn narrow_well_px(t: &hacivat_ui_core::theme::ThemeTokens) -> f32 {
    2.0 * t.spacing.sm + 2.0
}

/// The height the narrow form's title wraps to in an editor `width` wide (its text gets the
/// width less 4 px margins, 6 px padding and the border on each side), measured as the
/// paragraph lays it out (the line height snapped to the device pixel).
fn narrow_title_px(
    title: &SharedString,
    width: Pixels,
    style: &hacivat_ui_core::theme::TypeStyle,
    t: &hacivat_ui_core::theme::ThemeTokens,
    window: &Window,
) -> f32 {
    let room = f32::from(width) - 2.0 * t.spacing.xs - narrow_well_px(t);
    let pitch = window.pixel_snap(px(style.line_height));
    let run = TextRun {
        len: title.len(),
        font: hacivat_ui_core::fonts::font_for(style),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_text(
            title.clone(),
            px(style.size_px),
            &[run],
            Some(px(room.max(1.0))),
            None,
        )
        .map(|lines| lines.iter().map(|l| f32::from(l.size(pitch).height)).sum())
        .unwrap_or(style.line_height)
        .ceil()
}

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

// ## m3a-f4-ime (PO finding EDIT-2)
/// The longest line of `buf`, in bytes (lines end at `\n` or `\r`, as `set_text` measures):
/// O(document), so only off the UI thread.
fn longest_of(buf: &TextBuffer) -> u64 {
    let (mut longest, mut run) = (0u64, 0u64);
    for chunk in buf.chunks_in(0..buf.len_bytes()) {
        for b in chunk.bytes() {
            if b == b'\n' || b == b'\r' {
                longest = longest.max(run);
                run = 0;
            } else {
                run += 1;
            }
        }
    }
    longest.max(run)
}

/// After an edit: while a long line holds T1, the whole text is measured again once it has
/// rested [`REMEASURE_AFTER`] (a newer edit restarts the wait), on the background executor;
/// the answer applies only to the version it measured. Otherwise a waiting measure is dropped.
pub(crate) fn after_edit(v: &mut EditorView, cx: &mut Context<EditorView>) {
    if v.tier.tier != Tier::Reduced || !v.tier.by_line {
        v.remeasure = None;
        return;
    }
    let version = v.model.version();
    let buf = v.model.buffer().clone();
    v.remeasure = Some(cx.spawn(async move |this, cx| {
        cx.background_executor().timer(REMEASURE_AFTER).await;
        let longest = cx
            .background_executor()
            .spawn(async move { longest_of(&buf) })
            .await;
        let _ = this.update(cx, |v, cx| {
            if v.model.version() == version {
                v.remeasure = None;
                v.remeasured(longest, cx);
            }
        });
    }));
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

// ## m3a-f4-ime (EDIT-2)
/// The line length T1 ends below, as the copy says it: whole kilobytes under 80 % of 64 KB
/// (51 KB, a little under the exact 52,428.8 bytes, so the promise holds).
fn return_line(cx: &App) -> SharedString {
    let kb = LONG_LINE_MAX_BYTES * RETURN_BELOW_PERCENT / 100 / 1024;
    message(cx, "hui-editor-t1-line-size", &[("n", UiArg::Count(kb))])
}

/// The banner's two lines: what happened, and what it means.
fn lines(s: &TierState, cx: &App) -> Option<(SharedString, SharedString)> {
    if s.tier == Tier::Full {
        return None;
    }
    // ## m3a-t29 fix round 1 (M1): the copy follows the cause. The size title when the size
    // caused T1. ## m3a-f4-ime (EDIT-2): the body says when T1 ends for each cause: below
    // 3.2 MB, every line below 51 KB, or both.
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
    let size = || UiArg::Text(size_text(return_size(s), cx));
    let line = || UiArg::Text(return_line(cx));
    let body = match (by_size, s.by_line) {
        (true, true) => message(
            cx,
            "hui-editor-t1-both-body",
            &[("size", size()), ("line", line())],
        ),
        (false, true) => message(cx, "hui-editor-t1-long-body", &[("size", line())]),
        _ => message(cx, "hui-editor-t1-body", &[("size", size())]),
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
///
/// ## m3c-narrow (F11; design/m3b-safe surface 24): `narrow` (the width of an editor narrower
/// than [`NARROW_BELOW_PX`]) draws the narrow form: 4 px beside the well, padding 6, no icon,
/// the title and the body below it in a scroll viewport that gives up its height first. Fix
/// round 1 (I1): the well never gets shorter than its title and padding (measured at that
/// width, [`narrow_title_px`]), so the title is whole wherever the editor's column holds it;
/// the text's two-line minimum (`element::NARROW_TEXT_MIN_LINES`) gives way before it. The well
/// clips, so a column too short for the title itself records the title as cut.
pub fn banner(
    s: &TierState,
    narrow: Option<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let (title, body) = lines(s, cx)?;
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let well = match theme.mode {
        ThemeMode::Dark => c.panel,
        ThemeMode::Light => c.ground,
    };
    let style = t.type_ramp.body_sm;
    // ## m3c-narrow
    if let Some(width) = narrow {
        // ## m3c-narrow (fix round 1, I1): the well keeps its title.
        let keep = narrow_title_px(&title, width, &style, t, window) + narrow_well_px(t);
        let notice = div()
            .debug_selector(|| "editor-t1-notice".to_string())
            .flex()
            .flex_col()
            .flex_shrink(1.0)
            .min_h(px(keep))
            .overflow_hidden()
            .px(px(t.spacing.sm))
            .py(px(t.spacing.sm))
            .rounded(px(t.radii.sm))
            .border_1()
            .border_color(c.seam_strong.to_hsla())
            .bg(well.to_hsla())
            .child(
                div()
                    // ## m3c-narrow (fix round 1, I1): the title's box, for the scene checks.
                    .debug_selector(|| "editor-t1-title-box".to_string())
                    .flex_none()
                    .child(
                        paragraph(title, TextSource::Fluent, style, c.text.to_hsla())
                            .id("editor-t1-title"),
                    ),
            )
            .child(
                div()
                    .id("editor-t1-body-scroll")
                    .flex_shrink(1.0)
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(scroll_viewport())
                    .child(
                        paragraph(body, TextSource::Fluent, style, c.text_2.to_hsla())
                            .id("editor-t1-body"),
                    ),
            );
        return Some(
            div()
                .debug_selector(|| "editor-t1-banner".to_string())
                .flex()
                .flex_col()
                .flex_shrink(1.0)
                // ## m3c-narrow (fix round 1, I1): never shorter than the well's own minimum.
                .min_h(px(keep))
                .mt(px(t.spacing.md))
                .mx(px(t.spacing.xs))
                .mb(px(t.spacing.s_2xs))
                .child(notice)
                .into_any_element(),
        );
    }
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
