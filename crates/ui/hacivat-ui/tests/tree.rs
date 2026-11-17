//! The tree view in a window: virtualization, expansion, anchoring, keyboard, pointer,
//! accessibility and geometry (plan-m4 Task 8's list).

mod tree_fixture;

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    Entity, Focusable as _, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px,
    size,
};
use hacivat_ui::input::FocusedInput;
use hacivat_ui::text::TextRegistry;
use hacivat_ui::tree::{
    ChildState, RowStatus, StatusKind, TREE_OVERSCAN_ROWS, TreeEvent, TreeKey, TreeView,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use tree_fixture::Fixture;

const OUTLINE: &str = "
db
  public
    #Tables 2
      orders
        id
        total
      users
    #Views 1
      v_orders
  audit?
other
";

type Tree = Entity<TreeView<Fixture>>;
type Events = Rc<RefCell<Vec<TreeEvent>>>;

fn open(
    cx: &mut TestAppContext,
    f: Fixture,
    height: f32,
) -> (Tree, &mut VisualTestContext, Events) {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
    });
    let (tree, vcx) = cx.add_window_view(|_, cx| TreeView::new(f, cx));
    vcx.simulate_resize(size(px(320.0), px(height)));
    let events: Events = Rc::default();
    let sink = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&tree, move |_, e: &TreeEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach()
    });
    let focus = tree.read_with(vcx, |t, cx| t.focus_handle(cx));
    vcx.update(|w, cx| w.focus(&focus, cx));
    draw(vcx);
    (tree, vcx, events)
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

fn key(tree: &Tree, vcx: &mut VisualTestContext, label: &str) -> TreeKey {
    tree.read_with(vcx, |t, _| t.model().key(label))
}

fn cursor_label(tree: &Tree, vcx: &mut VisualTestContext) -> Option<String> {
    tree.read_with(vcx, |t, _| {
        t.cursor()
            .map(|k| t.model().nodes[&k].row.label.to_string())
    })
}

fn shown(tree: &Tree, vcx: &mut VisualTestContext) -> Vec<String> {
    tree.read_with(vcx, |t, _| {
        t.flat()
            .rows()
            .iter()
            .map(|r| t.model().nodes[&r.key].row.label.to_string())
            .collect()
    })
}

fn expand(tree: &Tree, vcx: &mut VisualTestContext, labels: &[&str]) {
    for l in labels {
        let k = key(tree, vcx, l);
        tree.update(vcx, |t, cx| t.set_expanded(k, true, cx));
    }
    draw(vcx);
}

fn press(vcx: &mut VisualTestContext, keys: &str) {
    vcx.simulate_keystrokes(keys);
    draw(vcx);
}

fn texts(vcx: &mut VisualTestContext) -> Vec<hacivat_ui::text::RenderedText> {
    vcx.read(|cx| cx.global::<TextRegistry>().frame.clone())
}

#[gpui::test]
fn only_visible_rows_are_built(cx: &mut TestAppContext) {
    let (tree, vcx, _) = open(cx, Fixture::flat_roots(100_000), 900.0);
    let built = tree.read_with(vcx, |t, _| t.built_rows());
    // 900 / 23 → 40 rows intersect the viewport, plus the overscan below.
    assert!(built.len() <= 40 + 2 * TREE_OVERSCAN_ROWS, "{built:?}");
    assert!(built.len() >= 39, "{built:?}");
    tree.update(vcx, |t, cx| t.scroll_by(23.0 * 50_000.0 + 5.0, cx));
    draw(vcx);
    let built = tree.read_with(vcx, |t, _| t.built_rows());
    assert_eq!(built.start, 50_000 - TREE_OVERSCAN_ROWS);
    assert!(built.len() <= 41 + 2 * TREE_OVERSCAN_ROWS, "{built:?}");
    // Only the built rows drew their labels.
    let drawn: Vec<_> = texts(vcx).into_iter().map(|t| t.text.to_string()).collect();
    assert!(drawn.contains(&"r50000".to_string()), "{drawn:?}");
    assert!(drawn.len() <= built.len(), "{}", drawn.len());
}

#[gpui::test]
fn expansion_state_survives_model_generation_change(cx: &mut TestAppContext) {
    let (tree, vcx, _) = open(cx, Fixture::parse(OUTLINE), 600.0);
    expand(&tree, vcx, &["db", "public", "Tables"]);
    let tables = key(&tree, vcx, "Tables");
    // A new generation: a column renamed, a table added.
    tree.update(vcx, |t, cx| {
        t.update_model(
            |m| {
                let id = m.key("id");
                m.row_mut(id).label = "id2".into();
                m.insert(Some(tables), 0, "accounts");
            },
            cx,
        )
    });
    draw(vcx);
    assert!(tree.read_with(vcx, |t, _| t.is_expanded(tables)));
    assert_eq!(
        shown(&tree, vcx),
        [
            "db", "public", "Tables", "accounts", "orders", "users", "Views", "audit", "other"
        ]
    );
    // A key that vanishes and returns comes back expanded.
    let public = key(&tree, vcx, "public");
    let db = key(&tree, vcx, "db");
    let kids = tree.read_with(vcx, |t, _| t.model().nodes[&db].children.clone());
    tree.update(vcx, |t, cx| {
        t.update_model(
            |m| m.set_children(db, ChildState::Loaded(Vec::new().into())),
            cx,
        )
    });
    draw(vcx);
    assert_eq!(shown(&tree, vcx), ["db", "other"]);
    tree.update(vcx, |t, cx| {
        t.update_model(|m| m.set_children(db, kids), cx)
    });
    draw(vcx);
    assert!(tree.read_with(vcx, |t, _| t.is_expanded(public)));
    assert_eq!(shown(&tree, vcx).len(), 9);
}

#[gpui::test]
fn anchor_row_kept_when_rows_inserted_above(cx: &mut TestAppContext) {
    let (tree, vcx, _) = open(cx, Fixture::flat_roots(1_000), 460.0);
    tree.update(vcx, |t, cx| t.scroll_by(23.0 * 500.0 + 7.0, cx));
    draw(vcx);
    let anchor = tree.read_with(vcx, |t, _| t.flat().rows()[500].key);
    let before = tree.read_with(vcx, |t, _| t.row_bounds(anchor)).unwrap();
    tree.update(vcx, |t, cx| {
        t.update_model(
            |m| {
                for i in 0..10 {
                    m.insert(None, 3, &format!("new{i}"));
                }
            },
            cx,
        )
    });
    draw(vcx);
    let after = tree.read_with(vcx, |t, _| t.row_bounds(anchor)).unwrap();
    assert_eq!(before.origin, after.origin);
    // Expanding a node above the viewport keeps it too.
    let first = tree.read_with(vcx, |t, _| t.flat().rows()[0].key);
    tree.update(vcx, |t, cx| {
        t.update_model(
            |m| {
                for i in 0..4 {
                    m.insert(Some(first), i, &format!("kid{i}"));
                }
            },
            cx,
        );
        t.set_expanded(first, true, cx);
    });
    draw(vcx);
    let after = tree.read_with(vcx, |t, _| t.row_bounds(anchor)).unwrap();
    assert_eq!(before.origin, after.origin);
}

#[gpui::test]
fn right_expands_left_collapses_and_goes_to_parent(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Fixture::parse(OUTLINE), 600.0);
    press(vcx, "down");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("db"));
    press(vcx, "right");
    assert!(tree.read_with(vcx, |t, _| t.is_expanded(t.model().key("db"))));
    press(vcx, "right");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("public"));
    press(vcx, "right right");
    // `public`'s first child is a group header: → lands on the first selectable row below it.
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("public"));
    let tables = key(&tree, vcx, "Tables");
    tree.update(vcx, |t, cx| t.set_expanded(tables, true, cx));
    press(vcx, "right");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("orders"));
    // ← on a leaf-like collapsed row goes to the nearest selectable ancestor (past the header).
    press(vcx, "left");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("public"));
    // ← on an expanded row collapses it.
    press(vcx, "left");
    assert!(!tree.read_with(vcx, |t, _| t.is_expanded(t.model().key("public"))));
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("public"));
    press(vcx, "left");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("db"));
    let db = key(&tree, vcx, "db");
    let public = key(&tree, vcx, "public");
    let ev = events.borrow().clone();
    assert!(ev.contains(&TreeEvent::Expand(db)));
    assert!(ev.contains(&TreeEvent::Expand(public)));
    assert!(ev.contains(&TreeEvent::Collapse(public)));
}

#[gpui::test]
fn expand_unloaded_emits_expand_once(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Fixture::parse(OUTLINE), 600.0);
    expand(&tree, vcx, &["db"]);
    let audit = key(&tree, vcx, "audit");
    tree.update(vcx, |t, cx| t.select(audit, cx));
    events.borrow_mut().clear();
    press(vcx, "right");
    assert_eq!(*events.borrow(), [TreeEvent::Expand(audit)]);
    // The host starts loading; new generations announce nothing more.
    tree.update(vcx, |t, cx| {
        t.update_model(
            |m| {
                m.set_children(
                    audit,
                    ChildState::Loading {
                        label: "Loading…".into(),
                    },
                )
            },
            cx,
        )
    });
    draw(vcx);
    tree.update(vcx, |t, cx| t.update_model(|m| m.generation += 1, cx));
    draw(vcx);
    assert_eq!(*events.borrow(), [TreeEvent::Expand(audit)]);
    // Collapse, the host drops the children, expand again: one more.
    press(vcx, "left");
    tree.update(vcx, |t, cx| {
        t.update_model(|m| m.set_children(audit, ChildState::NotLoaded), cx)
    });
    press(vcx, "right");
    assert_eq!(
        *events.borrow(),
        [
            TreeEvent::Expand(audit),
            TreeEvent::Collapse(audit),
            TreeEvent::Expand(audit)
        ]
    );
    // Children dropped while still expanded (a cancelled load) are asked for again, once.
    tree.update(vcx, |t, cx| {
        t.update_model(
            |m| {
                m.set_children(
                    audit,
                    ChildState::Loading {
                        label: "Loading…".into(),
                    },
                )
            },
            cx,
        )
    });
    tree.update(vcx, |t, cx| {
        t.update_model(|m| m.set_children(audit, ChildState::NotLoaded), cx)
    });
    tree.update(vcx, |t, cx| t.update_model(|m| m.generation += 1, cx));
    draw(vcx);
    assert_eq!(events.borrow().len(), 4);
    assert_eq!(events.borrow()[3], TreeEvent::Expand(audit));
    // A node restored expanded by the host with unloaded children is announced once too.
    events.borrow_mut().clear();
    let other = key(&tree, vcx, "other");
    tree.update(vcx, |t, cx| {
        t.update_model(|m| m.set_children(other, ChildState::NotLoaded), cx);
        t.set_expanded(other, true, cx);
    });
    draw(vcx);
    tree.update(vcx, |t, cx| t.update_model(|m| m.generation += 1, cx));
    draw(vcx);
    assert_eq!(*events.borrow(), [TreeEvent::Expand(other)]);
}

#[gpui::test]
fn loading_and_failed_child_rows_render_their_labels(cx: &mut TestAppContext) {
    let mut f = Fixture::parse(OUTLINE);
    let (audit, other) = (f.key("audit"), f.key("other"));
    f.set_children(
        audit,
        ChildState::Loading {
            label: "Reading columns… 3,400 of 15,000".into(),
        },
    );
    f.set_children(
        other,
        ChildState::Failed {
            label: "Some objects could not be read".into(),
        },
    );
    let (tree, vcx, _) = open(cx, f, 600.0);
    expand(&tree, vcx, &["db", "audit", "other"]);
    let drawn: Vec<_> = texts(vcx).into_iter().map(|t| t.text.to_string()).collect();
    assert!(
        drawn.contains(&"Reading columns… 3,400 of 15,000".to_string()),
        "{drawn:?}"
    );
    assert!(
        drawn.contains(&"Some objects could not be read".to_string()),
        "{drawn:?}"
    );
    // Neither row takes the cursor.
    tree.update(vcx, |t, cx| t.select(other, cx));
    press(vcx, "up");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("audit"));
}

#[gpui::test]
fn selection_follows_key_after_reorder(cx: &mut TestAppContext) {
    let (tree, vcx, _) = open(cx, Fixture::flat_roots(50), 600.0);
    let k = tree.read_with(vcx, |t, _| t.flat().rows()[10].key);
    tree.update(vcx, |t, cx| t.select(k, cx));
    tree.update(vcx, |t, cx| {
        t.update_model(
            |m| {
                m.roots.reverse();
                m.generation += 1;
            },
            cx,
        )
    });
    draw(vcx);
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(k));
    press(vcx, "down");
    // Reversed: r10 is now row 39; ↓ goes to row 40, which is r9.
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("r9"));
}

#[gpui::test]
fn group_header_rows_are_skipped_by_arrow_keys(cx: &mut TestAppContext) {
    let (tree, vcx, _) = open(cx, Fixture::parse(OUTLINE), 600.0);
    expand(&tree, vcx, &["db", "public", "Tables", "Views"]);
    let public = key(&tree, vcx, "public");
    tree.update(vcx, |t, cx| t.select(public, cx));
    press(vcx, "down");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("orders"));
    press(vcx, "down down");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("v_orders"));
    press(vcx, "up");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("users"));
    // Home and End skip headers as well (none at the ends here) and Enter activates.
    press(vcx, "end");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("other"));
    press(vcx, "home");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("db"));
}

#[gpui::test]
fn header_click_toggles_and_header_is_never_selected(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Fixture::parse(OUTLINE), 600.0);
    expand(&tree, vcx, &["db", "public"]);
    let tables = key(&tree, vcx, "Tables");
    let b = tree.read_with(vcx, |t, _| t.row_bounds(tables)).unwrap();
    vcx.simulate_click(b.center(), Modifiers::none());
    draw(vcx);
    assert!(tree.read_with(vcx, |t, _| t.is_expanded(tables)));
    assert!(tree.read_with(vcx, |t, _| t.selection().is_empty()));
    assert!(events.borrow().contains(&TreeEvent::Expand(tables)));
}

#[gpui::test]
fn chevron_click_toggles_without_activating(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Fixture::parse(OUTLINE), 600.0);
    let db = key(&tree, vcx, "db");
    let b = tree.read_with(vcx, |t, _| t.row_bounds(db)).unwrap();
    // Left edge (1) + padding (8) + half the 12 px chevron.
    let chevron = point(b.left() + px(15.0), b.center().y);
    vcx.simulate_click(chevron, Modifiers::none());
    draw(vcx);
    assert!(tree.read_with(vcx, |t, _| t.is_expanded(db)));
    assert_eq!(*events.borrow(), [TreeEvent::Expand(db)]);
    // A double click on the label activates (and selects) without toggling.
    let label = point(b.left() + px(60.0), b.center().y);
    vcx.simulate_event(gpui::MouseDownEvent {
        position: label,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    draw(vcx);
    assert!(tree.read_with(vcx, |t, _| t.is_expanded(db)));
    assert!(events.borrow().contains(&TreeEvent::Activate(db)));
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(db));
}

#[gpui::test]
fn context_menu_from_right_click_and_keyboard(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Fixture::parse(OUTLINE), 600.0);
    let other = key(&tree, vcx, "other");
    let b = tree.read_with(vcx, |t, _| t.row_bounds(other)).unwrap();
    vcx.simulate_mouse_down(b.center(), MouseButton::Right, Modifiers::none());
    draw(vcx);
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(other));
    assert!(
        events
            .borrow()
            .contains(&TreeEvent::ContextMenu(other, b.center()))
    );
    events.borrow_mut().clear();
    press(vcx, "shift-f10");
    assert_eq!(
        *events.borrow(),
        [TreeEvent::ContextMenu(other, b.bottom_left())]
    );
    events.borrow_mut().clear();
    press(vcx, "enter");
    assert_eq!(*events.borrow(), [TreeEvent::Activate(other)]);
}

#[gpui::test]
fn no_binding_fires_during_ime_composition(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Fixture::parse(OUTLINE), 600.0);
    vcx.update(|_, cx| {
        cx.set_global(FocusedInput {
            entity: None,
            composing: true,
        })
    });
    press(vcx, "down enter right o");
    assert!(events.borrow().is_empty());
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), None);
    vcx.update(|_, cx| {
        cx.set_global(FocusedInput {
            entity: None,
            composing: false,
        })
    });
    press(vcx, "down");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("db"));
}

#[gpui::test]
fn chords_with_modifiers_are_left_to_the_keymap(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Fixture::parse(OUTLINE), 600.0);
    press(vcx, "cmd-down ctrl-down alt-down cmd-o");
    assert!(events.borrow().is_empty());
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), None);
}

#[gpui::test]
fn type_ahead_folds_turkish_i_and_cycles(cx: &mut TestAppContext) {
    let f = Fixture::parse(
        "
Index
ılık
orders
İSTANBUL
iade
users
",
    );
    let (tree, vcx, _) = open(cx, f, 600.0);
    press(vcx, "i");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("Index"));
    press(vcx, "i");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("ılık"));
    press(vcx, "i");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("İSTANBUL"));
    vcx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    press(vcx, "i s t");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("İSTANBUL"));
    vcx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    press(vcx, "u");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("users"));
    vcx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    // A typed `ı` (Turkish keyboard) finds the dotted and dotless rows alike.
    vcx.simulate_input("ı");
    draw(vcx);
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("Index"));
}

#[gpui::test]
fn multi_select_with_shift_and_secondary_click(cx: &mut TestAppContext) {
    let (tree, vcx, _) = open(cx, Fixture::flat_roots(10), 600.0);
    tree.update(vcx, |t, _| t.set_multi_select(true));
    press(vcx, "down shift-down shift-down");
    assert_eq!(tree.read_with(vcx, |t, _| t.selection().len()), 3);
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("r2"));
    let r5 = tree.read_with(vcx, |t, _| t.flat().rows()[5].key);
    let b = tree.read_with(vcx, |t, _| t.row_bounds(r5)).unwrap();
    let secondary = if cfg!(target_os = "macos") {
        Modifiers::command()
    } else {
        Modifiers::control()
    };
    vcx.simulate_click(b.center(), secondary);
    draw(vcx);
    assert_eq!(tree.read_with(vcx, |t, _| t.selection().len()), 4);
    vcx.simulate_click(b.center(), secondary);
    draw(vcx);
    assert_eq!(tree.read_with(vcx, |t, _| t.selection().len()), 3);
    press(vcx, "space");
    assert_eq!(tree.read_with(vcx, |t, _| t.selection().len()), 4);
    // A plain click selects one.
    vcx.simulate_click(b.center(), Modifiers::none());
    draw(vcx);
    assert_eq!(tree.read_with(vcx, |t, _| t.selection().len()), 1);
}

#[gpui::test]
fn page_keys_move_by_a_viewport(cx: &mut TestAppContext) {
    let (tree, vcx, _) = open(cx, Fixture::flat_roots(500), 230.0);
    press(vcx, "down pagedown");
    // 230 px → 10 rows; a page is 9 steps.
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("r9"));
    press(vcx, "pagedown");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("r18"));
    let (scroll, top) = tree.read_with(vcx, |t, _| (t.scroll_offset(), t.built_rows()));
    assert!(scroll > 0.0, "the cursor was revealed: {scroll} {top:?}");
    press(vcx, "pageup");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("r9"));
}

#[gpui::test]
fn a11y_roles_tree_and_treeitem_with_level_and_expanded(cx: &mut TestAppContext) {
    let (tree, vcx, _) = open(cx, Fixture::parse(OUTLINE), 600.0);
    expand(&tree, vcx, &["db", "public", "Tables"]);
    // Nothing is built while no assistive technology is attached.
    press(vcx, "down down");
    assert_eq!(tree.read_with(vcx, |t, _| t.a11y_builds()), 0);
    assert!(tree.read_with(vcx, |t, _| t.a11y_items().is_empty()));

    tree.update(vcx, |t, cx| t.assume_a11y_active(true, cx));
    draw(vcx);
    let items = tree.read_with(vcx, |t, _| t.a11y_items());
    let names: Vec<_> = items.iter().map(|i| i.name.to_string()).collect();
    assert_eq!(
        names,
        [
            "db", "public", "Tables", "orders", "users", "Views", "audit", "other"
        ]
    );
    let by = |n: &str| items.iter().find(|i| i.name.as_ref() == n).unwrap().clone();
    assert_eq!(by("db").level, 1);
    assert_eq!(by("db").expanded, Some(true));
    assert_eq!(by("orders").level, 4);
    assert_eq!(by("orders").expanded, Some(false));
    assert_eq!((by("users").position, by("users").set_size), (2, 2));
    assert_eq!(by("users").expanded, None);
    assert!(by("public").focused);
    assert_eq!(by("public").selected, Some(true));
    assert_eq!(by("Tables").selected, None);
    let node = by("orders").node();
    assert_eq!(node.role(), gpui::Role::TreeItem);
    assert_eq!(node.level(), Some(4));

    // Detached again: no more builds.
    tree.update(vcx, |t, cx| t.assume_a11y_active(false, cx));
    draw(vcx);
    let builds = tree.read_with(vcx, |t, _| t.a11y_builds());
    press(vcx, "down");
    assert_eq!(tree.read_with(vcx, |t, _| t.a11y_builds()), builds);
}

#[gpui::test]
fn status_slot_draws_its_glyph_and_names_its_text(cx: &mut TestAppContext) {
    let mut f = Fixture::parse(OUTLINE);
    let other = f.key("other");
    f.row_mut(other).status = Some(RowStatus {
        kind: StatusKind::Modified,
        glyph: "M".into(),
        text: "Modified".into(),
    });
    f.row_mut(other).badge = Some("Out of date".into());
    f.row_mut(other).dim = true;
    let (tree, vcx, _) = open(cx, f, 600.0);
    let drawn: Vec<_> = texts(vcx).into_iter().map(|t| t.text.to_string()).collect();
    assert!(drawn.contains(&"M".to_string()), "{drawn:?}");
    assert!(drawn.contains(&"Out of date".to_string()), "{drawn:?}");
    tree.update(vcx, |t, cx| t.assume_a11y_active(true, cx));
    draw(vcx);
    let items = tree.read_with(vcx, |t, _| t.a11y_items());
    let item = items.iter().find(|i| i.key == other).unwrap();
    assert_eq!(item.description.as_deref(), Some("Modified"));
}

#[gpui::test]
fn row_geometry_23px_and_11px_floor(cx: &mut TestAppContext) {
    let mut f = Fixture::parse(OUTLINE);
    let orders = f.key("orders");
    f.row_mut(orders).meta = Some("1.204".into());
    let (tree, vcx, _) = open(cx, f, 600.0);
    expand(&tree, vcx, &["db", "public", "Tables", "orders"]);
    // Selected by a click: no keyboard focus ring, only the selection's left edge.
    let b = tree.read_with(vcx, |t, _| t.row_bounds(orders)).unwrap();
    vcx.simulate_click(b.center(), Modifiers::none());
    draw(vcx);
    assert_eq!(b.size.height, px(23.0));
    let rows: Vec<_> = tree.read_with(vcx, |t, _| t.flat().rows().to_vec());
    for (i, r) in rows.iter().enumerate() {
        let rb = tree.read_with(vcx, |t, _| t.row_bounds(r.key)).unwrap();
        assert_eq!(rb.top(), px(23.0 * i as f32), "row {i}");
    }
    for t in texts(vcx) {
        assert!(t.size_px >= 11.0, "{t:?}");
        assert!(!t.clipped, "{t:?}");
    }
    #[cfg(feature = "scene-check")]
    {
        use hacivat_ui_core::theme::DARK;
        let tint = gpui::Rgba::from(DARK.colors.blue_tint.to_hsla());
        let blue = gpui::Rgba::from(DARK.colors.blue.to_hsla());
        let same = |a: gpui::Rgba, b: gpui::Rgba| {
            (a.r - b.r).abs() < 0.01
                && (a.g - b.g).abs() < 0.01
                && (a.b - b.b).abs() < 0.01
                && (a.a - b.a).abs() < 0.01
        };
        // (fill, left, top, width, height) of every solid quad, logical px.
        let quads: Vec<_> = vcx.update(|window, _| {
            let s = window.scale_factor();
            window
                .rendered_scene()
                .quads
                .iter()
                .filter_map(|q| {
                    Some((
                        gpui::Rgba::from(q.background.as_solid()?),
                        q.bounds.origin.x.0 / s,
                        q.bounds.origin.y.0 / s,
                        q.bounds.size.width.0 / s,
                        q.bounds.size.height.0 / s,
                    ))
                })
                .collect()
        });
        let top = f32::from(b.top());
        let fills: Vec<_> = quads.iter().filter(|q| same(q.0, tint)).collect();
        assert_eq!(fills.len(), 1, "one selected row: {fills:?}");
        assert_eq!((fills[0].2, fills[0].4), (top, 23.0));
        let edges: Vec<_> = quads.iter().filter(|q| same(q.0, blue)).collect();
        assert_eq!(edges.len(), 1, "one blue edge: {edges:?}");
        assert_eq!(
            (edges[0].1, edges[0].2, edges[0].3, edges[0].4),
            (0.0, top, 1.0, 23.0),
            "a 1 px left edge"
        );
    }
}

#[gpui::test]
fn keyboard_focus_rings_the_cursor_row_only(cx: &mut TestAppContext) {
    let (tree, vcx, _) = open(cx, Fixture::flat_roots(20), 600.0);
    let r1 = tree.read_with(vcx, |t, _| t.flat().rows()[1].key);
    let b = tree.read_with(vcx, |t, _| t.row_bounds(r1)).unwrap();
    // A click selects without a ring; a key press brings it.
    vcx.simulate_click(b.center(), Modifiers::none());
    draw(vcx);
    #[cfg(feature = "scene-check")]
    assert!(ring_quads(vcx).is_empty());
    press(vcx, "down up");
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(r1));
    #[cfg(feature = "scene-check")]
    {
        // One ring (GPUI splits a border-only quad into its four strips, same bounds).
        let rings = ring_quads(vcx);
        assert!(!rings.is_empty() && rings.len() <= 4, "{rings:?}");
        for r in rings {
            assert_eq!(r, (f32::from(b.top()), 23.0, [1.0; 4]));
        }
    }
}

/// Quads whose border is blue: (top, height, [left, top, right, bottom] widths).
#[cfg(feature = "scene-check")]
fn ring_quads(vcx: &mut VisualTestContext) -> Vec<(f32, f32, [f32; 4])> {
    use hacivat_ui_core::theme::DARK;
    let blue = gpui::Rgba::from(DARK.colors.blue.to_hsla());
    vcx.update(|window, _| {
        let s = window.scale_factor();
        window
            .rendered_scene()
            .quads
            .iter()
            .filter(|q| {
                let c = gpui::Rgba::from(q.border_color);
                (c.r - blue.r).abs() < 0.01 && (c.b - blue.b).abs() < 0.01 && c.a > 0.5
            })
            .map(|q| {
                let w = &q.border_widths;
                (
                    q.bounds.origin.y.0 / s,
                    q.bounds.size.height.0 / s,
                    [w.left.0 / s, w.top.0 / s, w.right.0 / s, w.bottom.0 / s],
                )
            })
            .collect()
    })
}

#[gpui::test]
fn a_long_name_ellipsizes_and_the_meta_stays_whole(cx: &mut TestAppContext) {
    let mut f = Fixture::parse("customer_order_line_items_archive_2024\nb\n");
    let k = f.key("customer_order_line_items_archive_2024");
    f.row_mut(k).meta = Some("1.204".into());
    let (_tree, vcx, _) = open(cx, f, 200.0);
    vcx.simulate_resize(size(px(160.0), px(200.0)));
    draw(vcx);
    let frame = texts(vcx);
    let name = frame
        .iter()
        .find(|t| t.text.as_ref() == "customer_order_line_items_archive_2024")
        .expect("the name is drawn");
    assert!(name.truncated && !name.clipped, "{name:?}");
    let meta = frame.iter().find(|t| t.text.as_ref() == "1.204").unwrap();
    assert!(!meta.truncated && !meta.clipped, "{meta:?}");
}
