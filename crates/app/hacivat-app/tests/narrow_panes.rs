//! ## m3c-narrow: a console pane at the 120 px split floor (design/m3b-safe surface 24, R-M3-75)
//! draws its own surfaces whole in every locale: the empty console's placeholder wraps where it
//! fits (F10), the reduced-mode banner keeps its title whole and lets the rest scroll (F11), and
//! a pane's tabs give way instead of being cut: the inactive tabs first, then the active tab's
//! title ellipsizes (F12). The scene is the shots' `narrow-*` layout: the centre column split
//! side by side, the second pane focused and 120 px wide, written to `workspace.json` before
//! the app starts. The window is the 960 x 600 minimum, where the console tile is the shortest.

mod support;

use gpui::{TestAppContext, VisualTestContext, px, size};
use hacivat_app::console::view::fixtures;
use hacivat_app::shell::{TileLayout, Workspace};
use hacivat_driver::fake::FakeDriver;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_settings::LocalPaths;
use hacivat_ui::panes::{Axis, PaneId, PaneNode, PaneTree, TabId};
use hacivat_ui::text::RenderedText;
use support::{
    add_profile, in_window, profile_id, registry_frame, shell_with_fake_prepared,
    stop_core_runtime_test, switch_to, test_profile, wait_until,
};

/// The split floor: each side of a split keeps 120 px (`hacivat_ui::panes`).
const NARROW_PX: f32 = 120.0;
/// The minimum window, where the console tile is the shortest.
const WINDOW: (f32, f32) = (960.0, 600.0);
const DEV: &str = "orders-dev";

/// Every locale: English and Turkish, and with `pseudo` the two pseudo-locales.
fn locales() -> Vec<&'static str> {
    let mut v = vec!["en", "tr"];
    if cfg!(feature = "pseudo") {
        v.extend(["en-XA", "tr-XT"]);
    }
    v
}

/// The split ratio that leaves the second pane `want` px of a window `width` px wide (the
/// shots' `narrow_ratio`): the centre column is the window less the explorer and its 1 px seam,
/// and the panes share it less their own 1 px seam.
fn ratio(width: f32, want: f32) -> f32 {
    let centre = width - TileLayout::default().explorer_w - 1.0;
    1.0 - (want + 1.0) / centre
}

/// Writes the narrow layout (the shots' `before_start`): two consoles side by side, the second
/// focused and [`NARROW_PX`] wide in a [`WINDOW`]-wide window.
fn narrow_layout(paths: &LocalPaths) {
    let tiles = TileLayout::default();
    let mut panes = PaneTree::new(TabId(0));
    panes.split(PaneId(0), Axis::Horizontal, TabId(1));
    panes.set_ratio(&[], ratio(WINDOW.0, NARROW_PX));
    let value = serde_json::json!({
        "tiles": serde_json::to_value(tiles).expect("the tiles"),
        "panes": serde_json::to_value(&panes).expect("the panes"),
        "tabs": [
            {"id": 0, "kind": "console", "n": 1},
            {"id": 1, "kind": "console", "n": 2},
        ],
    });
    hacivat_settings::workspace_state::save(
        &paths.workspace_json(),
        hacivat_app::shell::workspace::WORKSPACE_STATE_VERSION,
        &value,
    )
    .expect("workspace.json");
}

/// A shell in the narrow layout at [`WINDOW`], with a console over `text` opened in the focused
/// narrow pane (beside its "Console 2" tab) and focused.
fn narrow_shell(cx: &mut TestAppContext, text: &str) -> &'static mut VisualTestContext {
    let vcx = shell_with_fake_prepared(cx, "en", FakeDriver::new(DbmsKind::Postgres), |p| {
        narrow_layout(p)
    });
    vcx.simulate_resize(size(px(WINDOW.0), px(WINDOW.1)));
    vcx.run_until_parked();
    add_profile(vcx, test_profile(DEV, Some(EnvTag::Dev), "10.0.4.11"));
    let id = profile_id(vcx, DEV);
    let tab =
        in_window(vcx, |window, cx| fixtures::open(id, text, window, cx)).expect("a console tab");
    wait_until(vcx, "the fixture console", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::console::Consoles::by_tab(cx, tab))
            .is_some()
    });
    settle(vcx);
    assert!(narrow(vcx), "the focused pane is the narrow one");
    vcx
}

/// The focused pane is the second, at the narrow ratio.
fn narrow(vcx: &VisualTestContext) -> bool {
    let (panes, _) = in_window(vcx, |window, cx| {
        window
            .root::<Workspace>()
            .flatten()
            .map(|ws| ws.read(cx).layout())
    })
    .expect("the workspace");
    let want = ratio(WINDOW.0, NARROW_PX);
    panes.focused == PaneId(1)
        && matches!(panes.root, PaneNode::Split { ratio, .. } if (ratio - want).abs() < 1e-4)
}

/// A few frames: the editor places its placeholder and picks its banner's form from the last
/// frame it painted.
fn settle(vcx: &VisualTestContext) {
    for _ in 0..3 {
        in_window(vcx, |window, _| window.refresh());
        vcx.run_until_parked();
    }
}

/// Switches to `lang` and lets the frame settle.
fn show_in(vcx: &VisualTestContext, lang: &str) {
    switch_to(vcx, lang);
    vcx.run_until_parked();
    settle(vcx);
}

/// The last frame's labels whose path ends with `#id`.
fn labels(vcx: &VisualTestContext, id: &str) -> Vec<RenderedText> {
    let needle = format!("#{id}");
    registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.ends_with(&needle))
        .collect()
}

/// `#id` was drawn whole: shown, painted, never clipped nor ellipsized.
fn whole(vcx: &VisualTestContext, lang: &str, id: &str) -> Vec<String> {
    let found = labels(vcx, id);
    if found.is_empty() {
        return vec![format!("{lang}: #{id} not drawn")];
    }
    found
        .into_iter()
        .filter(|r| r.clipped || r.truncated || !r.visible || r.painted_px <= 0.0)
        .map(|r| format!("{lang}: #{id} not whole: {r:?}"))
        .collect()
}

/// `#id` is never cut: shown or scrolled away, but never clipped.
fn uncut(vcx: &VisualTestContext, lang: &str, id: &str) -> Vec<String> {
    let found = labels(vcx, id);
    if found.is_empty() {
        return vec![format!("{lang}: #{id} not drawn")];
    }
    found
        .into_iter()
        .filter(|r| r.clipped)
        .map(|r| format!("{lang}: #{id} cut: {r:?}"))
        .collect()
}

/// F10: an empty console's placeholder in a 120 px pane wraps where it fits, never cut.
#[gpui::test]
fn the_empty_consoles_placeholder_is_never_cut_in_a_narrow_pane(cx: &mut TestAppContext) {
    let vcx = narrow_shell(cx, "");
    let mut failures = Vec::new();
    for lang in locales() {
        show_in(vcx, lang);
        failures.extend(whole(vcx, lang, "editor-placeholder"));
    }
    assert!(failures.is_empty(), "{failures:#?}");
    stop_core_runtime_test(cx);
}

/// F11: the reduced-mode banner in a 120 px pane keeps its title whole (a trust signal) and its
/// body is never cut (it wraps, and scrolls when the pane is too short for it).
#[gpui::test]
fn the_reduced_mode_banner_is_never_cut_in_a_narrow_pane(cx: &mut TestAppContext) {
    let line = "SELECT 'İzmir', 1234567;\n";
    let text = line.repeat((5 << 20) / line.len() + 1);
    let vcx = narrow_shell(cx, &text);
    let mut failures = Vec::new();
    for lang in locales() {
        show_in(vcx, lang);
        failures.extend(whole(vcx, lang, "editor-t1-title"));
        failures.extend(uncut(vcx, lang, "editor-t1-body"));
    }
    assert!(failures.is_empty(), "{failures:#?}");
    stop_core_runtime_test(cx);
}

/// F12: a 120 px pane's tabs give way instead of being cut: the inactive tab ("Console 2", a
/// Fluent title) first, then the active tab's title ellipsizes. Every tab title is drawn under
/// its own id, none is cut, and the active tab's title is shown.
#[gpui::test]
fn a_narrow_panes_tabs_give_way_and_are_never_cut(cx: &mut TestAppContext) {
    let vcx = narrow_shell(cx, "");
    let mut failures = Vec::new();
    for lang in locales() {
        show_in(vcx, lang);
        let frame = registry_frame(vcx);
        let tabs: Vec<&RenderedText> = frame
            .iter()
            .filter(|r| r.path.contains("hacivat-ui/src/panes.rs:"))
            .collect();
        for r in &tabs {
            if !r.path.ends_with("#tab-title") {
                failures.push(format!("{lang}: a tab title without its id: {r:?}"));
            }
            if r.clipped {
                failures.push(format!("{lang}: a tab title cut: {r:?}"));
            }
        }
        // The narrow pane's active tab ("orders-dev", data) is shown, ellipsized if it must be.
        if !tabs
            .iter()
            .any(|r| r.text.as_ref() == DEV && r.visible && r.painted_px > 0.0)
        {
            failures.push(format!(
                "{lang}: the active tab's title is not shown: {tabs:#?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    stop_core_runtime_test(cx);
}
