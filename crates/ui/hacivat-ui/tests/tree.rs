//! The tree view in a window (plan-m4 Task 8's list): virtualization, expansion, anchoring,
//! keyboard, pointer, group headers (R-M4-49), type-ahead folds (R-M4-50), accessibility and
//! geometry. `--features scene-check` adds the paint checks.

mod tree_fixture;

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    Entity, Focusable as _, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px,
    size,
};
use hacivat_ui::capsule::EnvKind;
use hacivat_ui::icon::IconName;
use hacivat_ui::input::FocusedInput;
use hacivat_ui::popover::{a11y_record, a11y_take};
use hacivat_ui::text::TextRegistry;
use hacivat_ui::tree::{
    ChildState, Fold, MetaMark, RowAction, RowFlag, RowPill, RowStatus, StatusKind,
    TREE_OVERSCAN_ROWS, TreeEvent, TreeKey, TreeView,
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
    // `public`'s first child is the "Tables" group header: → stops on it (R-M4-49).
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("Tables"));
    press(vcx, "right right");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("orders"));
    // ← on a collapsed row goes to its parent, here the header.
    press(vcx, "left");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("Tables"));
    // ← on an expanded row (a header too) collapses it.
    press(vcx, "left");
    assert!(!tree.read_with(vcx, |t, _| t.is_expanded(t.model().key("Tables"))));
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("Tables"));
    press(vcx, "left");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("public"));
    press(vcx, "left");
    assert!(!tree.read_with(vcx, |t, _| t.is_expanded(t.model().key("public"))));
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("public"));
    press(vcx, "left");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("db"));
    let db = key(&tree, vcx, "db");
    let public = key(&tree, vcx, "public");
    let tables = key(&tree, vcx, "Tables");
    let ev = events.borrow().clone();
    assert!(ev.contains(&TreeEvent::Expand(db)));
    assert!(ev.contains(&TreeEvent::Expand(public)));
    assert!(ev.contains(&TreeEvent::Expand(tables)));
    assert!(ev.contains(&TreeEvent::Collapse(tables)));
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
                        running: true,
                        count: None,
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
fn expand_unloaded_reannounces_after_drop_to_not_loaded(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Fixture::parse(OUTLINE), 600.0);
    expand(&tree, vcx, &["db"]);
    let audit = key(&tree, vcx, "audit");
    tree.update(vcx, |t, cx| t.select(audit, cx));
    press(vcx, "right");
    let loading = || ChildState::Loading {
        label: "Loading…".into(),
        running: true,
        count: None,
    };
    tree.update(vcx, |t, cx| {
        t.update_model(|m| m.set_children(audit, loading()), cx)
    });
    draw(vcx);
    events.borrow_mut().clear();
    // The host cancels the load and drops the children back to `NotLoaded` while the node is
    // still expanded: the node is asked for again, once, so it is never stranded.
    tree.update(vcx, |t, cx| {
        t.update_model(|m| m.set_children(audit, ChildState::NotLoaded), cx)
    });
    tree.update(vcx, |t, cx| t.update_model(|m| m.generation += 1, cx));
    draw(vcx);
    assert_eq!(*events.borrow(), [TreeEvent::Expand(audit)]);
    // A node revealed by its ancestor's expansion, with unloaded children, is asked for too.
    let db = key(&tree, vcx, "db");
    tree.update(vcx, |t, cx| t.set_expanded(db, false, cx));
    draw(vcx);
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(db));
    assert!(tree.read_with(vcx, |t, _| t.is_expanded(audit)));
    events.borrow_mut().clear();
    press(vcx, "right");
    assert_eq!(
        *events.borrow(),
        [TreeEvent::Expand(audit), TreeEvent::Expand(db)]
    );
}

#[gpui::test]
fn loading_and_failed_child_rows_render_their_labels(cx: &mut TestAppContext) {
    let mut f = Fixture::parse(OUTLINE);
    let (audit, other) = (f.key("audit"), f.key("other"));
    f.set_children(
        audit,
        ChildState::Loading {
            label: "Reading columns… 3,400 of 15,000".into(),
            running: true,
            count: None,
        },
    );
    f.set_children(
        other,
        ChildState::Failed {
            label: "Some objects could not be read".into(),
            retry: None,
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
fn group_header_rows_take_focus_but_never_selection(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Fixture::parse(OUTLINE), 600.0);
    expand(&tree, vcx, &["db", "public", "Tables", "orders", "Views"]);
    let public = key(&tree, vcx, "public");
    let tables = key(&tree, vcx, "Tables");
    let views = key(&tree, vcx, "Views");
    let selection = |tree: &Tree, vcx: &mut VisualTestContext| {
        tree.read_with(vcx, |t, _| {
            t.selection().iter().copied().collect::<Vec<_>>()
        })
    };
    tree.update(vcx, |t, cx| t.select(public, cx));
    // ↓ stops on "Tables": the cursor moves, the selection stays on `public`.
    press(vcx, "down");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("Tables"));
    assert_eq!(selection(&tree, vcx), [public]);
    press(vcx, "down");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("orders"));
    press(vcx, "up");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("Tables"));
    // ← collapses it; Enter expands and collapses it (never an activation); → expands it.
    press(vcx, "left");
    assert!(!tree.read_with(vcx, |t, _| t.is_expanded(tables)));
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("Tables"));
    press(vcx, "enter");
    assert!(tree.read_with(vcx, |t, _| t.is_expanded(tables)));
    press(vcx, "enter");
    assert!(!tree.read_with(vcx, |t, _| t.is_expanded(tables)));
    press(vcx, "right");
    assert!(tree.read_with(vcx, |t, _| t.is_expanded(tables)));
    // The menu key asks for the kind-group menu without selecting it.
    events.borrow_mut().clear();
    press(vcx, "shift-f10");
    let b = tree.read_with(vcx, |t, _| t.row_bounds(tables)).unwrap();
    assert_eq!(
        *events.borrow(),
        [TreeEvent::ContextMenu(tables, b.bottom_left())]
    );
    // Type-ahead skips headers: `t` finds `total`, `v` finds `v_orders`.
    press(vcx, "t");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("total"));
    vcx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    press(vcx, "v");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("v_orders"));
    press(vcx, "end");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("other"));
    press(vcx, "home");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("db"));
    // No event ever named a header as the selection or an activation.
    for e in events.borrow().iter() {
        for h in [tables, views] {
            assert_ne!(*e, TreeEvent::SelectionChanged(Some(h)));
            assert_ne!(*e, TreeEvent::Activate(h));
        }
    }
    assert!(
        selection(&tree, vcx)
            .iter()
            .all(|k| *k != tables && *k != views)
    );
}

#[gpui::test]
fn a_header_without_children_is_passed_over(cx: &mut TestAppContext) {
    // Environment headers (NOTES M4 surface 1): labels, skipped by the keys and the pointer.
    let f = Fixture::parse("#Production 1\nprod_db\n#Local 1\nlocal_db\n");
    let (tree, vcx, events) = open(cx, f, 600.0);
    press(vcx, "down");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("prod_db"));
    press(vcx, "down");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("local_db"));
    press(vcx, "up home");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("prod_db"));
    let header = key(&tree, vcx, "Local");
    let b = tree.read_with(vcx, |t, _| t.row_bounds(header)).unwrap();
    events.borrow_mut().clear();
    vcx.simulate_click(b.center(), Modifiers::none());
    draw(vcx);
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("prod_db"));
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
}

#[gpui::test]
fn collapsing_hides_no_selection_in_single_select(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Fixture::parse(OUTLINE), 600.0);
    expand(&tree, vcx, &["db", "public", "Tables"]);
    let orders = key(&tree, vcx, "orders");
    let tables = key(&tree, vcx, "Tables");
    tree.update(vcx, |t, cx| t.select(orders, cx));
    events.borrow_mut().clear();
    // The host collapses the group that holds the selection: the cursor moves up to the
    // header, and the hidden row is no longer selected.
    tree.update(vcx, |t, cx| t.set_expanded(tables, false, cx));
    draw(vcx);
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(tables));
    assert!(tree.read_with(vcx, |t, _| t.selection().is_empty()));
    assert_eq!(*events.borrow(), [TreeEvent::SelectionChanged(None)]);
    // In a multi-select the hidden row stays selected (R-M4-50: the bulk-action milestone
    // decides).
    tree.update(vcx, |t, cx| {
        t.set_multi_select(true);
        t.set_expanded(tables, true, cx);
        t.select(orders, cx);
        t.set_expanded(tables, false, cx);
    });
    draw(vcx);
    assert!(tree.read_with(vcx, |t, _| t.selection().contains(&orders)));
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
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(tables));
    assert_eq!(*events.borrow(), [TreeEvent::Expand(tables)]);
    // A right click asks for its menu at the pointer.
    events.borrow_mut().clear();
    vcx.simulate_mouse_down(b.center(), MouseButton::Right, Modifiers::none());
    draw(vcx);
    assert_eq!(
        *events.borrow(),
        [TreeEvent::ContextMenu(tables, b.center())]
    );
    assert!(tree.read_with(vcx, |t, _| t.selection().is_empty()));
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
fn typeahead_fold_is_set_by_the_host(cx: &mut TestAppContext) {
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
    let pause = |vcx: &mut VisualTestContext| {
        vcx.executor()
            .advance_clock(std::time::Duration::from_secs(2))
    };
    // The widget's default (R-M4-50): the locale fold, where `i` is neither `ı` nor `İ`.
    press(vcx, "i");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("Index"));
    press(vcx, "i");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("iade"));
    // The host turns the Turkish-tolerant fold on (D-074): every i is one letter.
    tree.update(vcx, |t, _| t.set_typeahead_fold(Fold::Tolerant));
    pause(vcx);
    press(vcx, "i");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("Index"));
    press(vcx, "i");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("ılık"));
    press(vcx, "i");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("İSTANBUL"));
    pause(vcx);
    press(vcx, "i s t");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("İSTANBUL"));
    pause(vcx);
    press(vcx, "u");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("users"));
    pause(vcx);
    // A typed `ı` (Turkish keyboard) finds the dotted and dotless rows alike.
    vcx.simulate_input("ı");
    draw(vcx);
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("Index"));
    // Within the 1 s buffer of the executor clock the prefix grows.
    pause(vcx);
    press(vcx, "o");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("orders"));
    vcx.executor()
        .advance_clock(std::time::Duration::from_millis(400));
    press(vcx, "r");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("orders"));
}

#[gpui::test]
fn multi_select_range_and_toggle(cx: &mut TestAppContext) {
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
    press(vcx, "down down");
    tree.update(vcx, |t, cx| {
        t.set_a11y_label(std::rc::Rc::new(|_| "Database explorer".into()));
        t.assume_a11y_active(true, cx);
    });
    a11y_record();
    draw(vcx);
    let decls = a11y_take();
    // The root is the tree, named by the host.
    assert!(
        decls
            .iter()
            .any(|d| d.role == Some(gpui::Role::Tree) && d.name.as_ref() == "Database explorer"),
        "{decls:?}"
    );
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
    // The items describe the frame being drawn (no one-frame lag): one key press, one frame,
    // and the header that took the cursor is the active item, still unselected.
    press(vcx, "down");
    let items = tree.read_with(vcx, |t, _| t.a11y_items());
    let focused: Vec<_> = items.iter().filter(|i| i.focused).collect();
    assert_eq!(focused.len(), 1);
    assert_eq!(focused[0].name.as_ref(), "Tables");
    assert_eq!(focused[0].selected, None);

    // Detached again: no more builds.
    tree.update(vcx, |t, cx| t.assume_a11y_active(false, cx));
    draw(vcx);
    let builds = tree.read_with(vcx, |t, _| t.a11y_builds());
    press(vcx, "down");
    assert_eq!(tree.read_with(vcx, |t, _| t.a11y_builds()), builds);
}

#[gpui::test]
fn a11y_inactive_builds_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::parse(OUTLINE);
    let other = f.key("other");
    f.row_mut(other).status = Some(RowStatus {
        kind: StatusKind::New,
        glyph: "A".into(),
        text: "Added".into(),
    });
    a11y_record();
    let (tree, vcx, _) = open(cx, f, 600.0);
    expand(&tree, vcx, &["db", "public", "Tables"]);
    press(vcx, "down down down");
    let decls = a11y_take();
    // No role, no item, no string while no assistive technology is attached.
    assert_eq!(tree.read_with(vcx, |t, _| t.a11y_builds()), 0);
    assert!(tree.read_with(vcx, |t, _| t.a11y_items().is_empty()));
    assert!(
        !decls
            .iter()
            .any(|d| matches!(d.role, Some(gpui::Role::Tree | gpui::Role::TreeItem))),
        "{decls:?}"
    );
}

#[gpui::test]
fn status_slot_draws_glyph_and_exposes_its_text(cx: &mut TestAppContext) {
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

// ## m4-t08-tree (fix round 1): I1, I2, M2, M3, M6, M7 and the R-W1-1 seam fields.

/// What a crowded row shows at one width.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Crowd {
    width: f32,
    meta_cut: bool,
    pill_gone: bool,
    badge_text_gone: bool,
    type_cut: bool,
    name_cut: bool,
}

#[gpui::test]
fn a_crowded_row_gives_way_in_the_notes_order(cx: &mut TestAppContext) {
    // NOTES M4 cross-cutting #5: meta → pill → badge to its glyph → type → name (to about seven
    // characters); the chevron, the icon and the flags never give way.
    let name = "customer_order_line_items_archive_2024";
    let mut f = Fixture::parse(&format!("{name}\nb\n"));
    let k = f.key(name);
    {
        let row = f.row_mut(k);
        row.icon = Some(IconName::Table2);
        row.type_text = Some("bigint".into());
        row.badge = Some("out of date".into());
        row.badge_glyph = Some(IconName::History);
        row.flags = vec![
            RowFlag::Env(EnvKind::Production),
            RowFlag::Lock { production: true },
        ];
        row.pill = Some(RowPill {
            text: "Structure".into(),
            explicit: true,
            tooltip: "Introspection level: Structure".into(),
        });
        row.meta = Some("1.204".into());
    }
    let (_tree, vcx, _) = open(cx, f, 200.0);
    let cut = |frame: &[hacivat_ui::text::RenderedText], s: &str| {
        frame
            .iter()
            .find(|t| t.text.as_ref() == s)
            .is_none_or(|t| t.truncated || t.clipped || !t.visible)
    };
    let pill = format!("hv-tree-pill-{}", k.0).leak();
    let mut states = Vec::new();
    let mut width = 760.0;
    while width >= 180.0 {
        vcx.simulate_resize(size(px(width), px(200.0)));
        draw(vcx);
        let frame = texts(vcx);
        let s = Crowd {
            width,
            meta_cut: cut(&frame, "1.204"),
            pill_gone: vcx.debug_bounds(pill).is_none(),
            badge_text_gone: !frame.iter().any(|t| t.text.as_ref() == "out of date"),
            type_cut: cut(&frame, "bigint"),
            name_cut: cut(&frame, name),
        };
        // The trust flags and the kind icon stay whole inside the row at every width.
        for sel in [
            format!("hv-tree-icon-{}", k.0),
            format!("hv-tree-flag-{}-0", k.0),
            format!("hv-tree-flag-{}-1", k.0),
        ] {
            let sel: &'static str = sel.leak();
            let b = vcx
                .debug_bounds(sel)
                .unwrap_or_else(|| panic!("{sel} drawn at {width}"));
            assert!(b.size.width >= px(7.0), "{sel} {b:?} at {width}");
            assert!(b.right() <= px(width), "{sel} {b:?} at {width}");
        }
        states.push(s);
        width -= 4.0;
    }
    assert!(
        !states[0].meta_cut && !states[0].name_cut,
        "{:?}",
        states[0]
    );
    // The widest width at which each part gave way.
    let first = |f: fn(&Crowd) -> bool| {
        states
            .iter()
            .find(|s| f(s))
            .map(|s| s.width)
            .unwrap_or_else(|| panic!("never gave way: {states:?}"))
    };
    let meta = first(|s| s.meta_cut);
    let pill_w = first(|s| s.pill_gone);
    let badge = first(|s| s.badge_text_gone);
    let ty = first(|s| s.type_cut);
    let name_w = first(|s| s.name_cut);
    assert!(
        meta > pill_w && pill_w > badge && badge > ty && ty > name_w,
        "meta {meta}, pill {pill_w}, badge {badge}, type {ty}, name {name_w}"
    );
    // Once a part gave way, every narrower width keeps it given way.
    for s in states.iter().filter(|s| s.width <= name_w) {
        assert!(s.pill_gone && s.badge_text_gone && s.name_cut, "{s:?}");
    }
    // At the narrowest, the name keeps about seven characters and the badge its glyph.
    let frame = texts(vcx);
    let n = frame.iter().find(|t| t.text.as_ref() == name).unwrap();
    assert!(n.truncated && !n.clipped && n.painted_px >= 35.0, "{n:?}");
    assert!(
        vcx.debug_bounds(format!("hv-tree-badge-{}", k.0).leak())
            .is_some()
    );
}

#[gpui::test]
fn a_click_after_a_model_change_acts_on_the_row_under_the_pointer(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Fixture::flat_roots(20), 600.0);
    let r5 = tree.read_with(vcx, |t, _| t.flat().rows()[5].key);
    let at = tree
        .read_with(vcx, |t, _| t.row_bounds(r5))
        .unwrap()
        .center();
    let down = |button, clicks| {
        gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
            position: at,
            modifiers: Modifiers::none(),
            button,
            click_count: clicks,
            first_mouse: false,
        })
    };
    // The host publishes three rows above and the click lands before the next frame (one
    // update, so no frame is drawn in between): it hits the listeners of the frame the user
    // saw, whose row was `r5`, now three rows further down.
    vcx.update(|window, cx| {
        tree.update(cx, |t, cx| {
            t.update_model(
                |m| {
                    for i in 0..3 {
                        m.insert(None, 0, &format!("new{i}"));
                    }
                },
                cx,
            )
        });
        assert_eq!(
            tree.read(cx).flat().rows()[5].key,
            TreeKey(3),
            "rebuilt: r2 is row 5"
        );
        window.dispatch_event(down(MouseButton::Left, 2), cx);
    });
    draw(vcx);
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(r5));
    assert!(
        events.borrow().contains(&TreeEvent::Activate(r5)),
        "{:?}",
        events.borrow()
    );
    // A row that is gone by the time the click lands does nothing.
    let r7 = tree.read_with(vcx, |t, _| t.model().key("r7"));
    let at7 = tree
        .read_with(vcx, |t, _| t.row_bounds(r7))
        .unwrap()
        .center();
    events.borrow_mut().clear();
    vcx.update(|window, cx| {
        tree.update(cx, |t, cx| {
            t.update_model(
                |m| {
                    m.roots.retain(|k| *k != r7);
                    m.generation += 1;
                },
                cx,
            )
        });
        for button in [MouseButton::Left, MouseButton::Right] {
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                    position: at7,
                    modifiers: Modifiers::none(),
                    button,
                    click_count: 2,
                    first_mouse: false,
                }),
                cx,
            );
        }
    });
    draw(vcx);
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(r5));
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
}

#[gpui::test]
fn every_a11y_action_acts_like_the_keyboard(cx: &mut TestAppContext) {
    use gpui::accesskit::Action;
    let (tree, vcx, events) = open(cx, Fixture::parse(OUTLINE), 600.0);
    expand(&tree, vcx, &["db", "public"]);
    let act = |vcx: &mut VisualTestContext, key: TreeKey, action: Action| {
        vcx.update(|window, cx| {
            tree.update(cx, |t, cx| t.run_a11y_action(key, action, window, cx))
        });
        draw(vcx);
    };
    let k = |vcx: &mut VisualTestContext, l: &str| key(&tree, vcx, l);
    let (public, tables, other, audit) = (
        k(vcx, "public"),
        k(vcx, "Tables"),
        k(vcx, "other"),
        k(vcx, "audit"),
    );
    // Focus: the cursor (and, on a row, the selection).
    act(vcx, public, Action::Focus);
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(public));
    // Click: a header toggles (never activates), a row activates.
    events.borrow_mut().clear();
    act(vcx, tables, Action::Click);
    assert!(tree.read_with(vcx, |t, _| t.is_expanded(tables)));
    assert_eq!(*events.borrow(), [TreeEvent::Expand(tables)]);
    act(vcx, other, Action::Click);
    assert!(events.borrow().contains(&TreeEvent::Activate(other)));
    // Expand and Collapse do nothing on a leaf.
    let users = k(vcx, "users");
    events.borrow_mut().clear();
    act(vcx, users, Action::Expand);
    act(vcx, users, Action::Collapse);
    assert!(!tree.read_with(vcx, |t, _| t.is_expanded(users)));
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    // On a node, they expand (asking for unloaded children) and collapse.
    act(vcx, audit, Action::Expand);
    assert_eq!(*events.borrow(), [TreeEvent::Expand(audit)]);
    act(vcx, tables, Action::Collapse);
    assert!(!tree.read_with(vcx, |t, _| t.is_expanded(tables)));
    assert!(events.borrow().contains(&TreeEvent::Collapse(tables)));
    // ShowContextMenu: the cursor moves there and the menu opens at the row's bottom-left.
    events.borrow_mut().clear();
    act(vcx, other, Action::ShowContextMenu);
    let b = tree.read_with(vcx, |t, _| t.row_bounds(other)).unwrap();
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(other));
    assert!(
        events
            .borrow()
            .contains(&TreeEvent::ContextMenu(other, b.bottom_left())),
        "{:?}",
        events.borrow()
    );
    // ScrollIntoView reveals a row below the viewport.
    let (tree, vcx, _) = open(cx, Fixture::flat_roots(200), 230.0);
    let far = tree.read_with(vcx, |t, _| t.flat().rows()[150].key);
    vcx.update(|window, cx| {
        tree.update(cx, |t, cx| {
            t.run_a11y_action(far, Action::ScrollIntoView, window, cx)
        })
    });
    draw(vcx);
    let b = tree.read_with(vcx, |t, _| t.row_bounds(far)).unwrap();
    assert!(b.top() >= px(0.0) && b.bottom() <= px(230.0), "{b:?}");
}

#[gpui::test]
fn a_vanished_cursor_moves_to_the_nearest_surviving_row(cx: &mut TestAppContext) {
    // A root removed by a new generation: the row above takes the cursor, ↓ goes on from there.
    let (tree, vcx, _) = open(cx, Fixture::flat_roots(10), 600.0);
    let r6 = tree.read_with(vcx, |t, _| t.model().key("r6"));
    tree.update(vcx, |t, cx| t.select(r6, cx));
    tree.update(vcx, |t, cx| {
        t.update_model(
            |m| {
                m.roots.retain(|k| *k != r6);
                m.generation += 1;
            },
            cx,
        )
    });
    draw(vcx);
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("r5"));
    press(vcx, "down");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("r7"));

    // A row the host's expansion hides: its nearest shown ancestor takes the cursor.
    let (tree, vcx, _) = open(cx, Fixture::parse(OUTLINE), 600.0);
    expand(&tree, vcx, &["db", "public", "Tables"]);
    let orders = key(&tree, vcx, "orders");
    let db = key(&tree, vcx, "db");
    tree.update(vcx, |t, cx| t.select(orders, cx));
    tree.update(vcx, |t, cx| t.set_expanded_keys([db], cx));
    draw(vcx);
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("public"));

    // A table dropped by a new generation: its neighbour inside the same group takes it.
    let (tree, vcx, _) = open(cx, Fixture::parse(OUTLINE), 600.0);
    expand(&tree, vcx, &["db", "public", "Tables"]);
    let users = key(&tree, vcx, "users");
    let tables = key(&tree, vcx, "Tables");
    tree.update(vcx, |t, cx| t.select(users, cx));
    tree.update(vcx, |t, cx| {
        t.update_model(
            |m| {
                let orders = m.key("orders");
                m.set_children(tables, ChildState::Loaded(vec![orders].into()))
            },
            cx,
        )
    });
    draw(vcx);
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("orders"));
}

#[gpui::test]
fn page_keys_count_rows_and_snap_to_a_cursor_stop(cx: &mut TestAppContext) {
    // Rows alternate with labels the keys pass over: a page moves a viewport less one *row*.
    let outline: String = (0..30).map(|i| format!("a{i}\n#H{i} 1\n")).collect();
    let (tree, vcx, _) = open(cx, Fixture::parse(&outline), 230.0);
    press(vcx, "down");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("a0"));
    // 230 px → 10 rows; nine rows down is the label `H4`, so the cursor snaps back to `a4`.
    press(vcx, "pagedown");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("a4"));
    press(vcx, "pagedown");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("a8"));
    press(vcx, "pageup");
    assert_eq!(cursor_label(&tree, vcx).as_deref(), Some("a4"));
}

#[gpui::test]
fn a_chevron_double_click_toggles_once(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Fixture::parse(OUTLINE), 600.0);
    let db = key(&tree, vcx, "db");
    let b = tree.read_with(vcx, |t, _| t.row_bounds(db)).unwrap();
    let chevron = point(b.left() + px(15.0), b.center().y);
    for clicks in [1, 2] {
        vcx.simulate_event(gpui::MouseDownEvent {
            position: chevron,
            modifiers: Modifiers::none(),
            button: MouseButton::Left,
            click_count: clicks,
            first_mouse: false,
        });
        draw(vcx);
    }
    assert!(tree.read_with(vcx, |t, _| t.is_expanded(db)));
    assert_eq!(*events.borrow(), [TreeEvent::Expand(db)]);
}

#[gpui::test]
fn status_rows_show_the_run_marker_the_count_and_the_retry_button(cx: &mut TestAppContext) {
    let mut f = Fixture::parse(OUTLINE);
    let (audit, other) = (f.key("audit"), f.key("other"));
    f.set_children(
        audit,
        ChildState::Loading {
            label: "Reading columns…".into(),
            running: true,
            count: Some("3,400/15,000".into()),
        },
    );
    f.set_children(
        other,
        ChildState::Failed {
            label: "Couldn't read other".into(),
            retry: Some("Retry".into()),
        },
    );
    let (tree, vcx, events) = open(cx, f, 600.0);
    expand(&tree, vcx, &["db", "audit", "other"]);
    let frame = texts(vcx);
    let label = frame
        .iter()
        .find(|t| t.text.as_ref() == "Reading columns…")
        .unwrap();
    assert_eq!(
        label.size_px, 12.0,
        "NOTES surface 2: the phase label is 12 px"
    );
    assert!(frame.iter().any(|t| t.text.as_ref() == "3,400/15,000"));
    assert!(
        vcx.debug_bounds(format!("hv-tree-run-{}", audit.0).leak())
            .is_some()
    );
    // Waiting: no marker.
    tree.update(vcx, |t, cx| {
        t.update_model(
            |m| {
                m.set_children(
                    audit,
                    ChildState::Loading {
                        label: "Waiting for columns".into(),
                        running: false,
                        count: None,
                    },
                )
            },
            cx,
        )
    });
    draw(vcx);
    assert!(
        vcx.debug_bounds(format!("hv-tree-run-{}", audit.0).leak())
            .is_none()
    );
    // Retry asks the host to load the failed node again, and changes nothing else.
    let cursor = tree.read_with(vcx, |t, _| t.cursor());
    let retry = vcx
        .debug_bounds(format!("hv-tree-retry-{}", other.0).leak())
        .expect("the retry button is drawn");
    events.borrow_mut().clear();
    vcx.simulate_click(retry.center(), Modifiers::none());
    draw(vcx);
    assert_eq!(*events.borrow(), [TreeEvent::Retry(other)]);
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), cursor);
    // Accessibility: the count and the retry word describe the status rows.
    tree.update(vcx, |t, cx| t.assume_a11y_active(true, cx));
    draw(vcx);
    let items = tree.read_with(vcx, |t, _| t.a11y_items());
    let failed = items
        .iter()
        .find(|i| i.name.as_ref() == "Couldn't read other")
        .unwrap();
    assert_eq!(failed.description.as_deref(), Some("Retry"));
    assert!(!failed.focusable);
}

#[gpui::test]
fn row_fields_of_the_mocks_render_and_reach_assistive_technology(cx: &mut TestAppContext) {
    let mut f = Fixture::parse(OUTLINE);
    let (db, public, tables) = (f.key("db"), f.key("public"), f.key("Tables"));
    {
        let row = f.row_mut(db);
        row.strong = true;
        row.meta_mark = Some(MetaMark::Ok);
        row.meta = Some("12 ms".into());
        row.flags = vec![
            RowFlag::Env(EnvKind::Production),
            RowFlag::Lock { production: true },
        ];
        row.a11y_description = Some("Production, read-only".into());
        row.tooltip = Some("3 of 4 schemas".into());
    }
    {
        let row = f.row_mut(public);
        row.pill = Some(RowPill {
            text: "Structure".into(),
            explicit: false,
            tooltip: "Introspection level: Structure (inherited)".into(),
        });
        row.row_action = Some(RowAction {
            icon: IconName::Plus,
            label: "Add to scope".into(),
        });
        row.dim = true;
    }
    f.row_mut(tables).filter_glyph = Some("60 objects are hidden".into());
    let orders = f.key("orders");
    f.row_mut(orders).type_text = Some("bigint".into());
    let (tree, vcx, events) = open(cx, f, 600.0);
    expand(&tree, vcx, &["db", "public", "Tables"]);
    let frame = texts(vcx);
    let db_label = frame.iter().find(|t| t.text.as_ref() == "db").unwrap();
    assert_eq!(db_label.size_px, 13.0);
    let ty = frame.iter().find(|t| t.text.as_ref() == "bigint").unwrap();
    assert_eq!(ty.size_px, 11.0);
    assert!(
        vcx.debug_bounds(format!("hv-tree-mark-{}", db.0).leak())
            .is_some()
    );
    assert!(vcx.debug_bounds("hv-tree-filter-glyph").is_some());
    // The row action shows on selection (or hover) only, and asks the host.
    let action = format!("hv-tree-row-action-{}", public.0).leak();
    assert!(vcx.debug_bounds(action).is_none());
    tree.update(vcx, |t, cx| t.select(public, cx));
    draw(vcx);
    let b = vcx.debug_bounds(action).expect("drawn on the selected row");
    events.borrow_mut().clear();
    vcx.simulate_click(b.center(), Modifiers::none());
    draw(vcx);
    assert_eq!(*events.borrow(), [TreeEvent::RowAction(public)]);
    // The pill asks for its list at its bottom-left, without selecting anything.
    tree.update(vcx, |t, cx| t.select(db, cx));
    draw(vcx);
    let pill = vcx
        .debug_bounds(format!("hv-tree-pill-{}", public.0).leak())
        .expect("the pill is drawn");
    events.borrow_mut().clear();
    vcx.simulate_click(pill.center(), Modifiers::none());
    draw(vcx);
    let ev = events.borrow().clone();
    let [TreeEvent::Pill(k, at)] = ev.as_slice() else {
        panic!("{ev:?}");
    };
    assert_eq!(*k, public);
    assert!(
        (f32::from(at.x) - f32::from(pill.left())).abs() <= 1.0
            && (f32::from(at.y) - f32::from(pill.bottom())).abs() <= 1.0,
        "{at:?} vs {pill:?}"
    );
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(db));
    // What assistive technology reads besides the name.
    tree.update(vcx, |t, cx| t.assume_a11y_active(true, cx));
    draw(vcx);
    let items = tree.read_with(vcx, |t, _| t.a11y_items());
    let by = |n: &str| items.iter().find(|i| i.name.as_ref() == n).unwrap().clone();
    assert_eq!(
        by("db").description.as_deref(),
        Some("Production, read-only, 3 of 4 schemas")
    );
    assert_eq!(
        by("public").description.as_deref(),
        Some("Introspection level: Structure (inherited)")
    );
}
