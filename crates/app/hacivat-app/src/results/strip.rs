//! The cursor strip (Task 30): fetching, suspended, complete, closed, cancelled, the memory
//! banner, the result sets not shown and the selection dimensions.
//!
//! The strip (26 px, `raised`, caption at `text-2`; NOTES surface 11) says how complete the
//! active result is and offers the next step. [`StripModel::of`] decides what it says from one
//! snapshot, the tab's memory banner and the console's log (the log's `CursorClosed` line knows
//! whether Hacivat's transaction was rolled back, and its `MoreResultSets` line how many sets
//! were drained, R-M3-25); [`render`] lays it out on the Overflow Rule [`Toolbar`], whose compact
//! forms give the strip its narrow shape: first the word "Selection" goes, then the cursor
//! sentence becomes "Cursor open" (its whole text stays in the tooltip). Every item is pinned:
//! buttons and counts never truncate or move into a menu.
//!
//! - **Close cursor** goes through `ResultSource::close_cursor`, whose reason follows the state:
//!   `MemoryLimit` while paused at a memory limit, else `UserClosed` (ledger ruling on T7).
//! - **Fetch more anyway** goes only through `ConsoleResults::raise_cap` (ledger T26 note).
//! - **Re-run** emits `ResultsEvent::Rerun` for the console's run UI.
//!
//! Counts are formatted in the format locale; the selection's size likewise.

use std::rc::Rc;

use hacivat_exec::{LogLine, LogMsg};
use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_results::{CloseReason, Completeness, StoreSnapshot, TruncReason};
use hacivat_session::session::UnitId;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui::toolbar::{ItemFn, Toolbar, ToolbarItem, Wrap};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div, px,
};
use hacivat_ui_core::theme::{Theme, TypeStyle};

use crate::l10n_global::tr;
use crate::results::adapter::Texts;
use crate::results::output_view::core_text;
use crate::results::tabs::MemoryBanner;
use crate::results::toolbar::Action;

/// What the strip leads with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StripKind {
    /// Rows are arriving.
    Fetching {
        rows: u64,
    },
    /// A cursor holds more rows.
    Suspended {
        rows: u64,
    },
    /// Suspended at a memory limit (R-M3-33): the banner.
    Memory {
        rows: u64,
        bytes: u64,
        can_raise: bool,
    },
    Complete {
        rows: u64,
    },
    /// Rows past the per-result cap were counted, not kept.
    Truncated {
        rows: u64,
    },
    Closed {
        rows: u64,
        reason: CloseReason,
        /// Whether Hacivat's transaction was rolled back and the idle bound, from the log's
        /// close line; `None` when no such line was seen.
        known: Option<(bool, u64)>,
    },
    Cancelled {
        rows: u64,
    },
}

// ## m3a-t30 fix round 1 (I2): the facts the strip takes from the log, kept per tab by the tile
// (`view.rs`), so the log's Clear and its ring's eviction cannot change what the strip says.

/// What the log said about one unit's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct StripFacts {
    /// The cursor's close: its reason, whether Hacivat's transaction was rolled back, and the
    /// idle bound.
    pub closed: Option<(CloseReason, bool, u64)>,
    /// Result sets read and not shown (R-M3-25).
    pub more_sets: Option<u32>,
}

impl StripFacts {
    /// What `log` says about `unit` (its newest lines win).
    pub fn from_log(log: &[LogLine], unit: UnitId) -> StripFacts {
        let mut facts = StripFacts::default();
        for line in log.iter().filter(|l| l.unit == Some(unit)) {
            match line.msg {
                LogMsg::CursorClosed {
                    reason,
                    rolled_back,
                    idle_ms,
                    ..
                } => facts.closed = Some((reason, rolled_back, idle_ms)),
                LogMsg::MoreResultSets { skipped } if skipped > 0 => {
                    facts.more_sets = Some(skipped);
                }
                _ => {}
            }
        }
        facts
    }

    /// Takes what `newer` knows; what it does not know is kept.
    pub fn merge(&mut self, newer: StripFacts) {
        if newer.closed.is_some() {
            self.closed = newer.closed;
        }
        if newer.more_sets.is_some() {
            self.more_sets = newer.more_sets;
        }
    }
}

/// What the strip shows for one result tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StripModel {
    pub kind: StripKind,
    /// Result sets of the unit read and not shown (R-M3-25).
    pub more_sets: Option<u32>,
}

impl StripModel {
    /// The strip of a result in `snapshot`'s state, with the tab's memory `banner` and what the
    /// log said about its unit (`facts`).
    pub fn of(
        snapshot: &StoreSnapshot,
        banner: Option<MemoryBanner>,
        facts: StripFacts,
    ) -> StripModel {
        let rows = snapshot.row_count();
        let closed = |reason: CloseReason| StripKind::Closed {
            rows,
            reason,
            known: facts
                .closed
                .filter(|(r, ..)| *r == reason)
                .map(|(_, rolled_back, idle_ms)| (rolled_back, idle_ms)),
        };
        let kind = match snapshot.completeness() {
            Completeness::Streaming => StripKind::Fetching { rows },
            Completeness::Suspended(_) => match banner {
                Some(b) => StripKind::Memory {
                    rows,
                    bytes: snapshot.bytes(),
                    can_raise: b.can_raise,
                },
                None => StripKind::Suspended { rows },
            },
            Completeness::Complete | Completeness::Truncated(TruncReason::FollowingResultSets) => {
                StripKind::Complete { rows }
            }
            Completeness::Truncated(TruncReason::ResultCap) => StripKind::Truncated { rows },
            Completeness::Closed { reason } => closed(*reason),
            Completeness::Cancelled => StripKind::Cancelled { rows },
            // The rows were given back with their tab: nothing on screen shows them.
            Completeness::Released => closed(CloseReason::TabClosed),
        };
        StripModel {
            kind,
            more_sets: facts.more_sets,
        }
    }

    /// An idle close the log never told about: its bound is unknown, so the sentence names no
    /// duration (`hv-strip-closed-idle`), never "0 ms".
    fn idle_unknown(&self) -> Option<u64> {
        match self.kind {
            StripKind::Closed {
                rows,
                reason: CloseReason::IdleTimeout,
                known: None,
            } => Some(rows),
            _ => None,
        }
    }

    /// The Fluent ID of the strip's leading sentence.
    pub fn sentence_id(&self) -> &'static str {
        match self.kind {
            StripKind::Fetching { .. } => ids::hv::STRIP_FETCHING.id,
            StripKind::Suspended { .. } => ids::hv::STRIP_ROWS_MORE.id,
            StripKind::Memory { .. } => ids::hv::STRIP_MEMORY.id,
            StripKind::Complete { .. } => ids::hv::STRIP_COMPLETE.id,
            StripKind::Truncated { .. } => ids::hv::STRIP_TRUNCATED.id,
            StripKind::Cancelled { .. } => ids::hv::STRIP_CANCELLED.id,
            StripKind::Closed { .. } if self.idle_unknown().is_some() => {
                ids::hv::STRIP_CLOSED_IDLE.id
            }
            StripKind::Closed { .. } => self.closed_msg().map_or("", |m| m.fluent_id()),
        }
    }

    /// The log message a closed cursor's sentence is (`cursor-closed-*`, Task 23).
    fn closed_msg(&self) -> Option<LogMsg> {
        match self.kind {
            StripKind::Closed {
                rows,
                reason,
                known,
            } => {
                let (rolled_back, idle_ms) = known.unwrap_or((false, 0));
                Some(LogMsg::CursorClosed {
                    rows,
                    reason,
                    rolled_back,
                    idle_ms,
                })
            }
            _ => None,
        }
    }

    /// The leading sentence in the UI language.
    pub fn sentence(&self, cx: &App) -> SharedString {
        match self.kind {
            StripKind::Fetching { rows } => tr(
                cx,
                ids::hv::STRIP_FETCHING,
                ids::hv::HvStripFetching { rows: Count(rows) },
            ),
            StripKind::Suspended { rows } => tr(
                cx,
                ids::hv::STRIP_ROWS_MORE,
                ids::hv::HvStripRowsMore { rows: Count(rows) },
            ),
            StripKind::Memory { rows, bytes, .. } => tr(
                cx,
                ids::hv::STRIP_MEMORY,
                ids::hv::HvStripMemory {
                    rows: Count(rows),
                    size: Text(Texts::of(cx).size(bytes)),
                },
            ),
            StripKind::Complete { rows } => tr(
                cx,
                ids::hv::STRIP_COMPLETE,
                ids::hv::HvStripComplete { rows: Count(rows) },
            ),
            StripKind::Truncated { rows } => tr(
                cx,
                ids::hv::STRIP_TRUNCATED,
                ids::hv::HvStripTruncated { rows: Count(rows) },
            ),
            StripKind::Cancelled { rows } => tr(
                cx,
                ids::hv::STRIP_CANCELLED,
                ids::hv::HvStripCancelled { rows: Count(rows) },
            ),
            StripKind::Closed { .. } => match self.idle_unknown() {
                Some(rows) => tr(
                    cx,
                    ids::hv::STRIP_CLOSED_IDLE,
                    ids::hv::HvStripClosedIdle { rows: Count(rows) },
                ),
                None => self
                    .closed_msg()
                    .map(|m| core_text(&m, cx))
                    .unwrap_or_default(),
            },
        }
    }

    // ## m3a-f2-visual (named exception)
    /// Whether the leading sentence may wrap: a reason (closed, cancelled, the memory limit,
    /// the cap), never a count.
    pub fn wraps(&self) -> bool {
        matches!(
            self.kind,
            StripKind::Closed { .. }
                | StripKind::Cancelled { .. }
                | StripKind::Memory { .. }
                | StripKind::Truncated { .. }
        )
    }

    /// Whether the strip offers Re-run (a result that ended without all its rows).
    pub fn offers_rerun(&self) -> bool {
        matches!(
            self.kind,
            StripKind::Closed { .. } | StripKind::Cancelled { .. }
        )
    }
}

/// What the strip's buttons do.
#[derive(Clone)]
pub struct StripActions {
    pub close_cursor: Action,
    pub fetch_more: Action,
    pub fetch_all: Action,
    pub fetch_anyway: Action,
    pub rerun: Action,
}

/// The strip's ID: its toolbar's debug selector is `toolbar-results-strip`, and the strip is
/// `results-strip`.
pub const STRIP_ID: &str = "results-strip";

// ## m3a-f2-visual: the wrapped reason's line pitch (NOTES surface 11).
const WRAP_LINE_PX: f32 = 16.0;

/// The cursor strip of `model`, with the selection's size (`rows × cols`) on the right.
pub fn render(
    model: StripModel,
    selection: Option<(u64, u64)>,
    actions: StripActions,
    cx: &App,
) -> AnyElement {
    let t = Theme::get(cx).tokens;
    // ## m3a-f2-visual (fix round 1, I2): in a narrow pane the row wraps; nothing is cut.
    let mut bar = Toolbar::new(STRIP_ID)
        .height(t.spacing.strip_height)
        .wrap_row();
    let sentence = model.sentence(cx);
    let lead = lead_glyph(model.kind);
    let count_like = matches!(
        model.kind,
        StripKind::Suspended { .. } | StripKind::Memory { .. } | StripKind::Closed { .. }
    );
    let sentence_text: TextFn = {
        let s = sentence.clone();
        Rc::new(move |_| s.clone())
    };
    // ## m3a-f2-visual (named exception): a reason sentence that still does not fit once the
    // strip's other forms are compact wraps, and the strip grows (NOTES surface 11: padding 4,
    // line height 16). Counts never wrap: Fetching, the suspended row count, Complete.
    let wrap = model.wraps().then(|| {
        let t = Theme::get(cx).tokens;
        let style = TypeStyle {
            line_height: WRAP_LINE_PX,
            ..t.type_ramp.caption
        };
        let s = sentence.clone();
        Wrap {
            text: sentence.clone(),
            style,
            element: Rc::new(move |_, cx| {
                let t = Theme::get(cx).tokens;
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.sm))
                    .flex_1()
                    .min_w(px(0.0))
                    .children(lead.map(|g| g.element(cx)))
                    .child(
                        div().flex_1().min_w(px(0.0)).child(
                            paragraph(
                                s.clone(),
                                TextSource::Fluent,
                                style,
                                if count_like {
                                    t.colors.text.to_hsla()
                                } else {
                                    t.colors.text_2.to_hsla()
                                },
                            )
                            .id("strip-sentence"),
                        ),
                    )
                    .into_any_element()
            }),
        }
    });
    let sentence_item = ToolbarItem::pinned(
        Rc::new(move |_, cx| {
            let t = Theme::get(cx).tokens;
            // Counts and reasons in `text`; the strip's other words in `text-2`.
            let ink = if count_like {
                t.colors.text
            } else {
                t.colors.text_2
            };
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .children(lead.map(|g| g.element(cx)))
                .child(
                    Label::new(
                        sentence.clone(),
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        ink.to_hsla(),
                    )
                    .id("strip-sentence"),
                )
                .into_any_element()
        }),
        sentence_text,
    );
    // ## m3a-f2-visual (named exception)
    bar = bar.item(match wrap {
        Some(wrap) => sentence_item.wraps(wrap),
        None => sentence_item,
    });
    match model.kind {
        StripKind::Suspended { .. } => {
            bar = bar.item(seam());
            let whole = tr(cx, ids::hv::STRIP_CURSOR_OPEN, NoArgs);
            let short = tr(cx, ids::hv::STRIP_CURSOR_OPEN_SHORT, NoArgs);
            let label: TextFn = {
                let whole = whole.clone();
                Rc::new(move |_| whole.clone())
            };
            // ## m3a-f2-visual (named exception): "Cursor open" wraps as the last stage (the
            // count before it never does; the buttons never truncate).
            let t = Theme::get(cx).tokens;
            let style = TypeStyle {
                line_height: WRAP_LINE_PX,
                ..t.type_ramp.caption
            };
            let wrapped = short.clone();
            let wrap = Wrap {
                text: short.clone(),
                style,
                element: Rc::new(move |_, cx| {
                    let t = Theme::get(cx).tokens;
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(
                            paragraph(
                                wrapped.clone(),
                                TextSource::Fluent,
                                style,
                                t.colors.text_2.to_hsla(),
                            )
                            .id("strip-cursor"),
                        )
                        .into_any_element()
                }),
            };
            bar = bar.item(
                ToolbarItem::pinned(caption_item(whole, "strip-cursor"), label)
                    .compact(caption_item(short, "strip-cursor"))
                    .priority(2)
                    .wraps(wrap),
            );
            bar = bar.item(button(
                "strip-close-cursor",
                ids::hv::STRIP_CLOSE_CURSOR,
                actions.close_cursor.clone(),
                cx,
            ));
            bar = bar.item(button(
                "strip-fetch-more",
                ids::hv::STRIP_FETCH_MORE,
                actions.fetch_more.clone(),
                cx,
            ));
            bar = bar.item(button(
                "strip-fetch-all",
                ids::hv::STRIP_FETCH_ALL,
                actions.fetch_all.clone(),
                cx,
            ));
        }
        StripKind::Memory { can_raise, .. } => {
            if can_raise {
                bar = bar.item(button(
                    "strip-fetch-anyway",
                    ids::hv::STRIP_FETCH_ANYWAY,
                    actions.fetch_anyway.clone(),
                    cx,
                ));
            }
            bar = bar.item(button(
                "strip-close-cursor",
                ids::hv::STRIP_CLOSE_CURSOR,
                actions.close_cursor.clone(),
                cx,
            ));
        }
        _ => {}
    }
    if model.offers_rerun() {
        bar = bar.item(button(
            "strip-rerun",
            ids::hv::STRIP_RERUN,
            actions.rerun.clone(),
            cx,
        ));
    }
    if let Some(n) = model.more_sets {
        let text = tr(
            cx,
            ids::hv::STRIP_MORE_SETS,
            ids::hv::HvStripMoreSets {
                n: Count(u64::from(n)),
            },
        );
        bar = bar.item(seam());
        let label: TextFn = {
            let text = text.clone();
            Rc::new(move |_| text.clone())
        };
        bar = bar.item(ToolbarItem::pinned(
            Rc::new(move |_, cx| {
                let t = Theme::get(cx).tokens;
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        Icon::new(IconName::Layers)
                            .size(IconSize::Chrome)
                            .color(t.colors.text_2.to_hsla()),
                    )
                    .child(
                        Label::new(
                            text.clone(),
                            TextSource::Fluent,
                            t.type_ramp.caption,
                            t.colors.text_2.to_hsla(),
                        )
                        .id("strip-more-sets"),
                    )
                    .into_any_element()
            }),
            label,
        ));
    }
    if let Some((rows, cols)) = selection {
        let word = tr(cx, ids::hv::STRIP_SELECTION, NoArgs);
        let dims = tr(
            cx,
            ids::hv::STRIP_DIMS,
            ids::hv::HvStripDims {
                rows: Count(rows),
                cols: Count(cols),
            },
        );
        let label: TextFn = {
            let dims = dims.clone();
            Rc::new(move |_| dims.clone())
        };
        let full = selection_item(Some(word), dims.clone());
        let compact = selection_item(None, dims);
        // The word "Selection" is the first thing the strip gives up (NOTES surface 11).
        bar = bar.trailing(
            ToolbarItem::pinned(full, label)
                .compact(compact)
                .priority(1),
        );
    }
    div()
        .debug_selector(|| STRIP_ID.to_string())
        .w_full()
        .flex_none()
        .child(bar)
        .into_any_element()
}

/// The glyph the strip leads with: the amber run marker while fetching, the green check when
/// complete, ink glyphs otherwise (Hue Is Meaning; errors and limits never get a new hue).
#[derive(Debug, Clone, Copy)]
enum Lead {
    RunMarker,
    Icon(IconName, Ink),
}

#[derive(Debug, Clone, Copy)]
enum Ink {
    Text,
    Text2,
    Green,
}

impl Lead {
    fn element(self, cx: &App) -> AnyElement {
        let c = Theme::get(cx).tokens.colors;
        match self {
            Lead::RunMarker => div()
                .size(px(7.0))
                .flex_none()
                .bg(c.amber.to_hsla())
                .into_any_element(),
            Lead::Icon(name, ink) => {
                let color = match ink {
                    Ink::Text => c.text,
                    Ink::Text2 => c.text_2,
                    Ink::Green => c.green,
                };
                Icon::new(name)
                    .size(IconSize::Chrome)
                    .color(color.to_hsla())
                    .into_any_element()
            }
        }
    }
}

fn lead_glyph(kind: StripKind) -> Option<Lead> {
    match kind {
        StripKind::Fetching { .. } => Some(Lead::RunMarker),
        StripKind::Suspended { .. } => None,
        StripKind::Memory { .. } => Some(Lead::Icon(IconName::CircleAlert, Ink::Text)),
        StripKind::Complete { .. } => Some(Lead::Icon(IconName::Check, Ink::Green)),
        StripKind::Truncated { .. } => Some(Lead::Icon(IconName::CircleAlert, Ink::Text)),
        StripKind::Closed { .. } => Some(Lead::Icon(IconName::Info, Ink::Text2)),
        StripKind::Cancelled { .. } => Some(Lead::Icon(IconName::Ban, Ink::Text2)),
    }
}

/// A 1 px seam between the strip's groups.
fn seam() -> ToolbarItem {
    let empty: TextFn = Rc::new(|_| SharedString::default());
    ToolbarItem::pinned(
        Rc::new(|_, cx| {
            let c = Theme::get(cx).tokens.colors;
            div()
                .w(px(1.0))
                .h(px(14.0))
                .flex_none()
                .bg(c.seam_strong.to_hsla())
                .into_any_element()
        }),
        empty,
    )
}

/// A caption in `text-2` with label ID `id`.
fn caption_item(text: SharedString, id: &'static str) -> ItemFn {
    Rc::new(move |_, cx| {
        let t = Theme::get(cx).tokens;
        Label::new(
            text.clone(),
            TextSource::Fluent,
            t.type_ramp.caption,
            t.colors.text_2.to_hsla(),
        )
        .id(id)
        .into_any_element()
    })
}

/// "Selection 3 × 2", or the size alone.
fn selection_item(word: Option<SharedString>, dims: SharedString) -> ItemFn {
    Rc::new(move |_, cx| {
        let t = Theme::get(cx).tokens;
        let ink = t.colors.text_2.to_hsla();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .children(word.clone().map(|w| {
                Label::new(w, TextSource::Fluent, t.type_ramp.caption, ink).id("strip-selection")
            }))
            .child(
                Label::new(dims.clone(), TextSource::Fluent, t.type_ramp.mono_kbd, ink)
                    .id("strip-dims"),
            )
            .into_any_element()
    })
}

/// A strip button (`button-<id>`): pinned, so it never truncates or moves.
fn button(
    id: &'static str,
    msg: hacivat_l10n::MsgRef<NoArgs>,
    on_click: Action,
    cx: &App,
) -> ToolbarItem {
    let text: SharedString = tr(cx, msg, NoArgs);
    let label: TextFn = {
        let text = text.clone();
        Rc::new(move |_| text.clone())
    };
    let run = on_click.clone();
    ToolbarItem::pinned(
        Rc::new(move |_, _| {
            let on_click = on_click.clone();
            Button::new(id, text.clone())
                .kind(ButtonKind::Default)
                .on_raised(true)
                // ## m3a-f2-visual (named exception): the strip's 20 px compact buttons.
                .compact(true)
                .on_click(move |_, window, cx| on_click(window, cx))
                .into_any_element()
        }),
        label,
    )
    .on_choose(move |window, cx| run(window, cx))
}
