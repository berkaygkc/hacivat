//! The command palette glue (§8.13, spec C18): ranks the registered actions for the query
//! (`rank_commands`) and hosts Task 11's `Palette` as the workspace layer `palette`.

use std::sync::Arc;

use hacivat_l10n::{MatchMode, SearchOpts, build_key, find, turkish_tolerant_for};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::palette::{Palette, PaletteEvent, PaletteItem};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Context, Entity, Focusable, Global, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, WeakEntity, Window, div,
    px,
};
use hacivat_ui_core::keymap::Keymap;
use hacivat_ui_core::theme::Theme;

use crate::l10n_global::{L10nGlobal, text_fn};
use crate::shell::keymap_state::shortcut_label;
use crate::shell::{ActiveKeymap, Layer, Workspace};

/// The palette's layer ID.
pub const PALETTE_LAYER: &str = "palette";
/// The palette's key context (pushed on the context stack while it is open).
pub const PALETTE_CONTEXT: &str = "Palette";
/// How many recently run commands are remembered for ranking.
const RECENT_MAX: usize = 20;

/// One action as the palette searches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandEntry {
    pub id: &'static str,
    /// The title in the UI language (`l10n.t_dynamic(title_id)`).
    pub title: SharedString,
    /// The English title (`l10n.t_english(title_id)`), the second search layer.
    pub english_title: SharedString,
    /// The bound chord as the platform shows it.
    pub keys: Option<SharedString>,
}

/// Commands run from the palette, most recent first.
#[derive(Debug, Clone, Default)]
pub struct RecentCommands(pub Vec<String>);

impl Global for RecentCommands {}

/// Ranks `entries` for `query` (F14, C18):
/// 1. matches on the localized title, by kind (ExactWord > Prefix > Substring > Fuzzy, the
///    search key's score bands) and score;
/// 2. then matches on the English title only;
/// 3. then matches on the action ID only.
///
/// Ties go to the more recent command (`recent` is most recent first), then to registry order.
/// An empty query lists every entry. Only localized-title matches are highlighted.
pub fn rank_commands(
    query: &str,
    entries: &[CommandEntry],
    recent: &[String],
    opts: &SearchOpts,
) -> Vec<PaletteItem> {
    let mut scored: Vec<(u8, u32, usize, usize, PaletteItem)> = entries
        .iter()
        .enumerate()
        .filter_map(|(order, e)| {
            let (layer, score, highlight) =
                if let Some(m) = find(&build_key(&e.title, opts), query, opts) {
                    (0, m.score, m.ranges)
                } else if let Some(m) = find(&build_key(&e.english_title, opts), query, opts) {
                    (1, m.score, Vec::new())
                } else {
                    let m = find(&build_key(e.id, opts), query, opts)?;
                    (2, m.score, Vec::new())
                };
            let recency = recent.iter().position(|r| r == e.id).unwrap_or(usize::MAX);
            let item = PaletteItem {
                id: SharedString::new_static(e.id),
                title: e.title.clone(),
                keys: e.keys.clone(),
                highlight,
            };
            Some((layer, score, recency, order, item))
        })
        .collect();
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(b.1.cmp(&a.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });
    scored.into_iter().map(|s| s.4).collect()
}

/// The enabled registered actions, titled in the UI language with their bound keys.
pub fn command_entries(cx: &App) -> Vec<CommandEntry> {
    let (Some(reg), Some(global)) = (
        cx.try_global::<ActionRegistry>(),
        cx.try_global::<L10nGlobal>(),
    ) else {
        return Vec::new();
    };
    reg.all()
        .iter()
        .filter(|d| (d.enabled)(cx))
        .map(|d| CommandEntry {
            id: d.id,
            title: global.l10n.t_dynamic(d.title_id, Vec::new()).into(),
            english_title: global.l10n.t_english(d.title_id).into(),
            keys: shortcut_label(cx, d.id).map(Into::into),
        })
        .collect()
}

/// The search options for the UI language: fuzzy, Turkish-tolerant for Turkish users (D-074).
pub fn search_opts(cx: &App) -> SearchOpts {
    let turkish_tolerant = cx.try_global::<L10nGlobal>().is_some_and(|g| {
        let rt = g.l10n.snapshot();
        let ui = rt
            .ui_chain
            .first()
            .map(ToString::to_string)
            .unwrap_or_default();
        turkish_tolerant_for(&ui, &rt.format.format_locale)
    });
    SearchOpts {
        turkish_tolerant,
        mode: MatchMode::Fuzzy,
    }
}

fn ranked(query: &str, cx: &App) -> Vec<PaletteItem> {
    let recent = cx
        .try_global::<RecentCommands>()
        .map(|r| r.0.clone())
        .unwrap_or_default();
    rank_commands(query, &command_entries(cx), &recent, &search_opts(cx))
}

/// The rows of `items` again, in the same order, with titles and keys in the current language
/// and keymap (a language switch keeps the query and the rows, F16).
fn relabelled(items: &[PaletteItem], query: &str, cx: &App) -> Vec<PaletteItem> {
    let entries = command_entries(cx);
    let opts = search_opts(cx);
    items
        .iter()
        .filter_map(|item| {
            let e = entries.iter().find(|e| item.id == e.id)?;
            let highlight = find(&build_key(&e.title, &opts), query, &opts)
                .map(|m| m.ranges)
                .unwrap_or_default();
            Some(PaletteItem {
                id: item.id.clone(),
                title: e.title.clone(),
                keys: e.keys.clone(),
                highlight,
            })
        })
        .collect()
}

fn remember(cx: &mut App, id: &str) {
    let recent = &mut cx.default_global::<RecentCommands>().0;
    recent.retain(|r| r != id);
    recent.insert(0, id.to_string());
    recent.truncate(RECENT_MAX);
}

/// The open palette host (one per app in M1).
#[derive(Default)]
struct OpenPalette(Option<WeakEntity<PaletteHost>>);

impl Global for OpenPalette {}

/// The layer view around the `Palette`: places it under the title bar and keeps its rows in
/// the current language and keymap.
pub struct PaletteHost {
    palette: Entity<Palette>,
    /// The localization generation and keymap the rows were built with.
    built_with: (u64, Option<Arc<Keymap>>),
    _subscription: Subscription,
}

fn signature(cx: &App) -> (u64, Option<Arc<Keymap>>) {
    (
        cx.try_global::<L10nGlobal>()
            .map_or(0, |g| g.l10n.generation()),
        cx.try_global::<ActiveKeymap>().map(|a| a.keymap.clone()),
    )
}

impl PaletteHost {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let palette =
            cx.new(|cx| Palette::new(cx, text_fn(hacivat_l10n::ids::hv::TITLEBAR_PALETTE)));
        let items = ranked("", cx);
        palette.update(cx, |p, _| p.set_items(items));
        let subscription = cx.subscribe_in(
            &palette,
            window,
            |_, palette, event: &PaletteEvent, window, cx| match event {
                PaletteEvent::QueryChanged(query) => {
                    let items = ranked(query, cx);
                    palette.update(cx, |p, cx| {
                        p.set_items(items);
                        cx.notify();
                    });
                }
                PaletteEvent::Confirmed(id) => {
                    close_palette(window, cx);
                    remember(cx, id);
                    let id = id.clone();
                    window.defer(cx, move |window, cx| {
                        ActionRegistry::run(&id, window, cx);
                    });
                }
                PaletteEvent::Dismissed => close_palette(window, cx),
            },
        );
        Self {
            palette,
            built_with: signature(cx),
            _subscription: subscription,
        }
    }
}

impl Render for PaletteHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = signature(cx);
        if now != self.built_with {
            self.built_with = now;
            let p = self.palette.read(cx);
            let items = relabelled(p.items(), &p.query(), cx);
            self.palette.update(cx, |p, _| p.set_items(items));
        }
        let t = Theme::get(cx).tokens;
        // The completion popup's list plus detail width (DESIGN.md), inside the window.
        let width = (t.spacing.completion_list_width + t.spacing.completion_detail_width)
            .min(f32::from(window.viewport_size().width) - 2.0 * t.spacing.lg);
        div()
            .absolute()
            .top(px(t.spacing.titlebar_height + t.spacing.sm))
            .left_0()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("palette-layer")
                    .w(px(width.max(0.0)))
                    .on_mouse_down_out(cx.listener(|_, _, window, cx| close_palette(window, cx)))
                    .child(self.palette.clone()),
            )
    }
}

/// The open palette view, if any.
pub fn open_palette_view(cx: &App) -> Option<Entity<Palette>> {
    let host = cx.try_global::<OpenPalette>()?.0.as_ref()?.upgrade()?;
    Some(host.read(cx).palette.clone())
}

/// `workspace.command_palette`: pushes the palette as layer `palette` (context `Palette`) with
/// every enabled action listed, and focuses its query. An open palette just takes the focus.
pub fn open_palette(window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    if ws.read(cx).layer_ids().contains(&PALETTE_LAYER) {
        if let Some(p) = open_palette_view(cx) {
            let focus = p.focus_handle(cx);
            window.focus(&focus, cx);
        }
        return;
    }
    let host = cx.new(|cx| PaletteHost::new(window, cx));
    let focus = host.read(cx).palette.focus_handle(cx);
    ws.update(cx, |ws, cx| {
        ws.push_layer(
            Layer {
                id: PALETTE_LAYER,
                context: PALETTE_CONTEXT,
                view: host.clone().into(),
                restore_focus: None,
            },
            window,
            cx,
        );
    });
    window.focus(&focus, cx);
    cx.set_global(OpenPalette(Some(host.downgrade())));
}

/// Pops the palette when it is the topmost layer (focus goes back to where it was).
fn close_palette(window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    if ws.read(cx).layer_ids().last() == Some(&PALETTE_LAYER) {
        ws.update(cx, |ws, cx| ws.pop_layer(window, cx));
    }
}
