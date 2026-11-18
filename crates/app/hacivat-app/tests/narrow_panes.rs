//! ## m3c-narrow: a console pane at the 120 px split floor (design/m3b-safe surface 24, R-M3-75)
//! draws its own surfaces whole in every locale: the empty console's placeholder wraps where it
//! fits (F10), the reduced-mode banner keeps its title whole and lets the rest scroll (F11), and
//! a pane's tabs give way instead of being cut: the inactive tabs first, then the active tab's
//! title ellipsizes (F12). The scene is the shots' `narrow-*` layout: the centre column split
//! side by side, the second pane focused and 120 px wide, written to `workspace.json` before
//! the app starts. The window is the 960 x 600 minimum, where the console tile is the shortest.

// ## m3c-t45 (controller fix, gate line 2): these are scene tests (`rendered_scene`, debug
// bounds), so the file builds only with `scene-check`, like the other scene-test helpers.
#![cfg(feature = "scene-check")]

mod support;

use gpui::{
    Bounds, Modifiers, MouseButton, Pixels, TestAppContext, VisualTestContext, point, px, size,
};
use hacivat_app::console::view::fixtures;
use hacivat_app::console::{ConsoleId, Consoles};
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
/// focused and [`NARROW_PX`] wide in a [`WINDOW`]-wide window. ## m3c-narrow (fix round 1, I1):
/// `stacked` (a console tile `editor_h` px tall) splits that narrow pane again, top to bottom,
/// [`STACKED_RATIO`] to the upper half, and focuses the lower half.
fn narrow_layout(paths: &LocalPaths, stacked: Option<f32>) {
    let mut tiles = TileLayout::default();
    let mut panes = PaneTree::new(TabId(0));
    panes.split(PaneId(0), Axis::Horizontal, TabId(1));
    panes.set_ratio(&[], ratio(WINDOW.0, NARROW_PX));
    let mut tabs = vec![
        serde_json::json!({"id": 0, "kind": "console", "n": 1}),
        serde_json::json!({"id": 1, "kind": "console", "n": 2}),
    ];
    if let Some(editor_h) = stacked {
        tiles.editor_h = editor_h;
        panes.split(PaneId(1), Axis::Vertical, TabId(2));
        panes.set_ratio(&[true], STACKED_RATIO);
        tabs.push(serde_json::json!({"id": 2, "kind": "console", "n": 3}));
    }
    let value = serde_json::json!({
        "tiles": serde_json::to_value(tiles).expect("the tiles"),
        "panes": serde_json::to_value(&panes).expect("the panes"),
        "tabs": tabs,
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
    let (vcx, _) = narrow_shell_with(cx, text, None);
    assert!(narrow(vcx), "the focused pane is the narrow one");
    vcx
}

/// [`narrow_shell`] over [`narrow_layout`]`(stacked)`, in a window [`STACKED_WINDOW_H`] tall
/// when stacked; also the fixture console's id.
fn narrow_shell_with(
    cx: &mut TestAppContext,
    text: &str,
    stacked: Option<f32>,
) -> (&'static mut VisualTestContext, ConsoleId) {
    let vcx = shell_with_fake_prepared(cx, "en", FakeDriver::new(DbmsKind::Postgres), |p| {
        narrow_layout(p, stacked)
    });
    let height = if stacked.is_some() {
        STACKED_WINDOW_H
    } else {
        WINDOW.1
    };
    vcx.simulate_resize(size(px(WINDOW.0), px(height)));
    vcx.run_until_parked();
    // A test may open several shells in one app: the profile is added once.
    let known = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .profiles()
            .iter()
            .any(|p| p.name == DEV)
    });
    if !known {
        add_profile(vcx, test_profile(DEV, Some(EnvTag::Dev), "10.0.4.11"));
    }
    let id = profile_id(vcx, DEV);
    let tab =
        in_window(vcx, |window, cx| fixtures::open(id, text, window, cx)).expect("a console tab");
    wait_until(vcx, "the fixture console", |vcx| {
        vcx.cx.read(|cx| Consoles::by_tab(cx, tab)).is_some()
    });
    settle(vcx);
    let console = vcx
        .cx
        .read(|cx| Consoles::by_tab(cx, tab))
        .expect("the fixture console");
    (vcx, console)
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

/// The text area console `id`'s editor last painted, and its line height.
fn text_area(vcx: &VisualTestContext, id: ConsoleId) -> (Bounds<Pixels>, Pixels) {
    vcx.cx.read(|cx| {
        let editor = Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("an editor");
        let rows = editor
            .read(cx)
            .last_layout()
            .map(|l| *l.rows())
            .expect("a painted frame");
        (rows.bounds, rows.line_height)
    })
}

/// A document over the T1 size limit: its editor opens in reduced mode.
fn large_text() -> String {
    let line = "SELECT 'İzmir', 1234567;\n";
    line.repeat((5 << 20) / line.len() + 1)
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
/// ## m3c-narrow (fix round 1, M6): in English and Turkish the body shows (its viewport is not
/// squeezed or scrolled away), and wherever the column holds the title and two lines, the text
/// keeps its two lines.
#[gpui::test]
fn the_reduced_mode_banner_is_never_cut_in_a_narrow_pane(cx: &mut TestAppContext) {
    let (vcx, console) = narrow_shell_with(cx, &large_text(), None);
    assert!(narrow(vcx), "the focused pane is the narrow one");
    let mut failures = Vec::new();
    for lang in locales() {
        show_in(vcx, lang);
        failures.extend(whole(vcx, lang, "editor-t1-title"));
        failures.extend(uncut(vcx, lang, "editor-t1-body"));
        if matches!(lang, "en" | "tr") {
            let shown = labels(vcx, "editor-t1-body")
                .iter()
                .any(|r| r.visible && r.painted_px > 0.0);
            if !shown {
                failures.push(format!("{lang}: the body does not show"));
            }
        }
        let (room, need, _, text, line) = measure(vcx, console).expect("the narrow banner");
        if room >= need + 2.0 * line && f32::from(text.size.height) + 0.5 < 2.0 * line {
            failures.push(format!(
                "{lang}: the text keeps under two lines ({text:?}, line {line}) in a column of \
                 {room} px (the title needs {need} px)"
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    stop_core_runtime_test(cx);
}

/// F12: a 120 px pane's tabs give way instead of being cut: the inactive tab ("Console 2", a
/// Fluent title) first, then the active tab's title ellipsizes. Every tab title is drawn under
/// its own id, none is cut, and the active tab's title is shown.
/// ## m3c-narrow (fix round 1, M7): a second console of the same connection is opened, so the
/// narrow pane holds two inactive tabs ("Console 2" and one with an environment mark) beside the
/// active one. The inactive tabs have given way whole (0 px), the active tab ends inside the
/// pane, and the inactive tabs clip their content: the strip shows one environment mark, the
/// active tab's (an inactive tab that did not clip would draw its own over the active tab).
#[gpui::test]
fn a_narrow_panes_tabs_give_way_and_are_never_cut(cx: &mut TestAppContext) {
    let (vcx, console) = narrow_shell_with(cx, "", None);
    assert!(narrow(vcx), "the focused pane is the narrow one");
    let inactive_marked = vcx
        .cx
        .read(|cx| Consoles::get(cx, console).map(|s| s.tab))
        .expect("the fixture's tab");
    let id = profile_id(vcx, DEV);
    let active =
        in_window(vcx, |window, cx| fixtures::open(id, "", window, cx)).expect("a console tab");
    wait_until(vcx, "the second console", |vcx| {
        vcx.cx.read(|cx| Consoles::by_tab(cx, active)).is_some()
    });
    settle(vcx);
    let marked_selector: &'static str =
        Box::leak(format!("tab-{}", inactive_marked.0).into_boxed_str());
    let active_selector: &'static str = Box::leak(format!("tab-{}", active.0).into_boxed_str());
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
        // The narrow pane's active tab ("orders-dev (2)", data) is shown, ellipsized if it must be.
        if !tabs
            .iter()
            .any(|r| r.text.contains(DEV) && r.visible && r.painted_px > 0.0)
        {
            failures.push(format!(
                "{lang}: the active tab's title is not shown: {tabs:#?}"
            ));
        }
        // ## m3c-narrow (fix round 1, M7)
        let inactive = [vcx.debug_bounds("tab-1"), vcx.debug_bounds(marked_selector)];
        let Some(tab) = vcx.debug_bounds(active_selector) else {
            failures.push(format!("{lang}: the active tab is not drawn"));
            continue;
        };
        if inactive
            .iter()
            .any(|b| b.is_none_or(|b| f32::from(b.size.width) > 0.5))
        {
            failures.push(format!(
                "{lang}: the inactive tab did not give way: {inactive:?}"
            ));
        }
        if f32::from(tab.right()) > WINDOW.0 + 0.5 {
            failures.push(format!(
                "{lang}: the active tab runs past the pane: {tab:?}"
            ));
        }
        let marks = marks_in(vcx, px(WINDOW.0 - NARROW_PX), tab.top(), tab.bottom());
        if marks != 1 {
            failures.push(format!(
                "{lang}: {marks} environment marks drawn in the narrow strip (the active tab's only)"
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    stop_core_runtime_test(cx);
}

// ## m3c-narrow (fix round 1, I1)
/// The narrow banner's chrome around its title box: 8 px above and 2 px below the well, its 6 px
/// padding above and below, its 1 px border above and below.
const BANNER_CHROME_PX: f32 = 8.0 + 2.0 + 6.0 * 2.0 + 1.0 * 2.0;

/// The stacked layout's upper share of the narrow pane.
const STACKED_RATIO: f32 = 0.3;
/// The stacked scene's window height: room to drag the console tile taller.
const STACKED_WINDOW_H: f32 = 800.0;

/// The console tile's height (`TileLayout::editor_h`).
fn editor_h(vcx: &VisualTestContext) -> f32 {
    in_window(vcx, |window, cx| {
        window
            .root::<Workspace>()
            .flatten()
            .map(|ws| ws.read(cx).layout().1.editor_h)
    })
    .expect("the workspace")
}

/// Drags the seam between the console tile and the results tile so the console tile is
/// `target` px tall (one shell, no second window), and lets the frame settle.
fn drag_editor_seam(vcx: &mut VisualTestContext, target: f32) {
    let top = 40.0; // the title bar (`spacing.titlebar_height`)
    let x = px(TileLayout::default().explorer_w + 40.0);
    let from = point(x, px(top + editor_h(vcx) + 0.5));
    let to = point(x, px(top + target));
    vcx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    settle(vcx);
}

/// The column's height, the room the title needs with its chrome, the title's box, the text
/// area and its line height.
type Measured = (f32, f32, Bounds<Pixels>, Bounds<Pixels>, f32);

/// What the stacked scene measures: the editor column's height, the room the title needs with
/// its chrome, the text area and its line height.
fn measure(vcx: &mut VisualTestContext, console: ConsoleId) -> Option<Measured> {
    let column = vcx.debug_bounds("editor-column")?;
    let title = vcx.debug_bounds("editor-t1-title-box")?;
    let (text, line) = text_area(vcx, console);
    Some((
        f32::from(column.size.height),
        f32::from(title.size.height) + BANNER_CHROME_PX,
        title,
        text,
        f32::from(line),
    ))
}

/// F11 in a pane that is narrow and short (the 120 px pane split again top to bottom), in
/// tr-XT, in one shell: the console tile is dragged to the height where the editor's column
/// holds the title with its chrome but not also the text's two lines. There the title is whole
/// (not clipped, never under the text area): the two-line minimum gives way first. Then, at a
/// height with room for both, the text keeps its two lines.
#[cfg(feature = "pseudo")]
#[gpui::test]
fn a_narrow_and_short_pane_keeps_the_reduced_mode_title_whole(cx: &mut TestAppContext) {
    let (vcx, console) = narrow_shell_with(cx, &large_text(), Some(TileLayout::default().editor_h));
    show_in(vcx, "tr-XT");
    let (room, need, _, _, line) = measure(vcx, console).expect("the narrow banner");
    // Aim the column at the middle of the narrow-and-short band, [need, need + 2 lines), in at
    // most three drags (the column follows the tile at the pane's share).
    let mut in_band = None;
    let mut last = (editor_h(vcx), room);
    for _ in 0..3 {
        let (tile, column) = last;
        let target = need + line;
        let share = STACKED_RATIO.mul_add(-1.0, 1.0);
        drag_editor_seam(vcx, tile + (target - column) / share);
        let m = measure(vcx, console).expect("the narrow banner");
        last = (editor_h(vcx), m.0);
        if m.0 >= need - 0.5 && m.0 < need + 2.0 * line {
            in_band = Some(m);
            break;
        }
    }
    let (room, need, title, text, line) = in_band
        .unwrap_or_else(|| panic!("no tile height gave the narrow-and-short case: {last:?}"));
    let mut failures = Vec::new();
    if title.bottom() > text.top() + px(0.5) {
        failures.push(format!(
            "the title ({title:?}) runs under the text area ({text:?}) in a column of {room} px \
             (the title needs {need} px)"
        ));
    }
    failures.extend(whole(vcx, "tr-XT, narrow and short", "editor-t1-title"));
    // With room for the title and two lines, the text keeps them.
    let tile = editor_h(vcx);
    drag_editor_seam(
        vcx,
        tile + (3.0 * line + 20.0) / STACKED_RATIO.mul_add(-1.0, 1.0),
    );
    let (room, need, title, text, line) = measure(vcx, console).expect("the narrow banner");
    assert!(
        room >= need + 2.0 * line,
        "the drag left too little room: {room}"
    );
    if f32::from(text.size.height) + 0.5 < 2.0 * line {
        failures.push(format!(
            "the text area keeps under two lines: {text:?} (line {line})"
        ));
    }
    if title.bottom() > text.top() + px(0.5) {
        failures.push(format!(
            "the title ({title:?}) runs under the text area ({text:?})"
        ));
    }
    failures.extend(whole(vcx, "tr-XT, room for both", "editor-t1-title"));
    assert!(failures.is_empty(), "{failures:#?}");
    stop_core_runtime_test(cx);
}

// ## m3c-narrow (fix round 1, M7)
/// How many environment marks (7 px quads) the last frame drew visibly in the band `top` to
/// `bottom` right of `left` (window px): a quad counts where the part its content mask lets
/// through has area.
fn marks_in(vcx: &VisualTestContext, left: Pixels, top: Pixels, bottom: Pixels) -> usize {
    in_window(vcx, |window, _| {
        let scale = window.scale_factor();
        window
            .rendered_scene()
            .quads
            .iter()
            .filter(|q| {
                let w = q.bounds.size.width.0 / scale;
                let h = q.bounds.size.height.0 / scale;
                let b = q.bounds.intersect(&q.content_mask.bounds);
                let (x, y) = (b.origin.x.0 / scale, b.origin.y.0 / scale);
                (w - 7.0).abs() < 0.5
                    && (h - 7.0).abs() < 0.5
                    && b.size.width.0 / scale > 0.5
                    && b.size.height.0 / scale > 0.5
                    && x >= f32::from(left)
                    && y >= f32::from(top)
                    && y < f32::from(bottom)
            })
            .count()
    })
}
