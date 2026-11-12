//! Completion items (the harman popup list plus 200 synthetic columns) and the popup.

use std::sync::OnceLock;

use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, BoxShadow, IntoElement, ParentElement, SharedString, Styled, canvas, div, point, px,
};
use hacivat_ui_core::theme::ThemeTokens;

use crate::element::BenchProbe;

/// The harman popup shows at most 41 candidates ("6 / 41").
pub const MAX_ITEMS: usize = 41;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Column,
    Function,
    Keyword,
}

impl CompletionKind {
    pub fn glyph(self) -> &'static str {
        match self {
            CompletionKind::Column => "c",
            CompletionKind::Function => "f",
            CompletionKind::Keyword => "k",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: String,
}

/// The rows of the harman completion popup, in order.
const HARMAN: &[(&str, CompletionKind, &str)] = &[
    ("customer_id", CompletionKind::Column, "bigint · orders"),
    ("city", CompletionKind::Column, "not in scope"),
    (
        "cancelled_at",
        CompletionKind::Column,
        "timestamptz · orders",
    ),
    ("count", CompletionKind::Function, "function"),
    (
        "current_date",
        CompletionKind::Keyword,
        "keyword · function",
    ),
    ("case", CompletionKind::Keyword, "keyword"),
];

const STEMS: &[&str] = &[
    "account", "address", "amount", "balance", "billing", "branch", "campaign", "carrier",
    "channel", "client", "coupon", "currency", "delivery", "discount", "invoice", "order",
    "payment", "product", "region", "shipment",
];
const SUFFIXES: &[(&str, &str)] = &[
    ("id", "bigint"),
    ("name", "text"),
    ("code", "text"),
    ("status", "text"),
    ("type", "text"),
    ("total", "numeric"),
    ("count", "integer"),
    ("created_at", "timestamptz"),
    ("updated_at", "timestamptz"),
    ("note", "text"),
];

fn all_items() -> &'static [CompletionItem] {
    static ITEMS: OnceLock<Vec<CompletionItem>> = OnceLock::new();
    ITEMS.get_or_init(|| {
        let harman = HARMAN.iter().map(|&(label, kind, detail)| CompletionItem {
            label: label.into(),
            kind,
            detail: detail.into(),
        });
        let synthetic = STEMS.iter().flat_map(|stem| {
            SUFFIXES.iter().map(move |(suffix, ty)| CompletionItem {
                label: format!("{stem}_{suffix}"),
                kind: CompletionKind::Column,
                detail: format!("{ty} · {stem}s"),
            })
        });
        harman.chain(synthetic).collect()
    })
}

fn starts_with_ignore_ascii_case(label: &str, prefix: &str) -> bool {
    label.len() >= prefix.len()
        && label.is_char_boundary(prefix.len())
        && label[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Up to `MAX_ITEMS` items whose label starts with `prefix`, ignoring ASCII case.
pub fn complete(prefix: &str) -> Vec<CompletionItem> {
    all_items()
        .iter()
        .filter(|item| starts_with_ignore_ascii_case(&item.label, prefix))
        .take(MAX_ITEMS)
        .cloned()
        .collect()
}

/// Rows shown at once; the footer gives the position in the full list.
const VISIBLE_ROWS: usize = 10;

/// The harman popup: `raised` surface, `radii.md` corners and the `popover_shadow` layers.
/// When `probe` is set, the frame that paints the popup is recorded for the completion bench.
pub fn popup(
    items: &[CompletionItem],
    selected: usize,
    probe: Option<BenchProbe>,
    tokens: &ThemeTokens,
) -> AnyElement {
    let colors = tokens.colors;
    let mono = tokens.type_ramp.code;
    let caption = tokens.type_ramp.caption;
    let shadow = tokens
        .popover_shadow
        .iter()
        .map(|layer| BoxShadow {
            color: layer.color.to_hsla(),
            offset: point(px(layer.x), px(layer.y)),
            blur_radius: px(layer.blur),
            spread_radius: px(layer.spread),
            inset: false,
        })
        .collect();
    let rows = items
        .iter()
        .take(VISIBLE_ROWS)
        .enumerate()
        .map(|(i, item)| {
            let on = i == selected;
            let row = div()
                .h(px(tokens.spacing.tree_row_height))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(tokens.spacing.sm))
                .pl(px(5.))
                .pr(px(7.))
                .rounded(px(tokens.radii.xs))
                .child(
                    div()
                        .w(px(18.))
                        .flex()
                        .justify_center()
                        .font(font_for(&caption))
                        .text_size(px(caption.size_px))
                        .text_color(if on { colors.blue } else { colors.text_2 }.to_hsla())
                        .child(item.kind.glyph()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font(font_for(&mono))
                        .text_size(px(mono.size_px))
                        .text_color(colors.text.to_hsla())
                        .child(SharedString::from(item.label.clone())),
                )
                .child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .font(font_for(&caption))
                        .text_size(px(caption.size_px))
                        .text_color(colors.text_2.to_hsla())
                        .child(SharedString::from(item.detail.clone())),
                );
            if on {
                row.bg(colors.blue_tint_strong.to_hsla())
            } else {
                row
            }
        });
    let footer = div()
        .h(px(tokens.spacing.kv_row_height))
        .px(px(tokens.spacing.md))
        .flex()
        .items_center()
        .justify_end()
        .font(font_for(&caption))
        .text_size(px(caption.size_px))
        .text_color(colors.text_2.to_hsla())
        .child(SharedString::from(format!(
            "{} / {}",
            selected + 1,
            items.len()
        )));
    // Painted last inside the popup, so its paint callback runs once the popup is in the scene.
    let painted_hook = canvas(
        |_, _, _| (),
        move |_, _, _, cx| {
            if let Some(probe) = &probe {
                probe.record_after_present(cx);
            }
        },
    )
    .w(px(0.))
    .h(px(0.));
    div()
        .w(px(tokens.spacing.completion_list_width))
        .flex()
        .flex_col()
        .p(px(tokens.spacing.xs))
        .bg(colors.raised.to_hsla())
        .border_1()
        .border_color(colors.seam_strong.to_hsla())
        .rounded(px(tokens.radii.md))
        .shadow(shadow)
        .children(rows)
        .child(footer)
        .child(painted_hook)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_filters_by_prefix_case_insensitively() {
        assert!(complete("CUS").iter().any(|i| i.label == "customer_id"));
    }

    #[test]
    fn completion_caps_at_41() {
        assert!(complete("").len() <= 41);
    }

    #[test]
    fn list_is_harman_plus_200_unique_synthetic_columns() {
        let items = all_items();
        assert_eq!(items.len(), HARMAN.len() + 200);
        let mut labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), items.len());
        assert_eq!(complete("")[0].label, "customer_id");
    }

    #[test]
    fn non_ascii_prefix_never_panics_and_matches_nothing() {
        assert!(complete("İ").is_empty());
        assert!(complete("customer_idx").is_empty());
    }
}
