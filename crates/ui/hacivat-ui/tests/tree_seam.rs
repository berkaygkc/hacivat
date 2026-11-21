//! M4 Task 1b: the tree seam, written against Task 1b's flat stub (the first test keeps its
//! name from then).
//!
//! ## m4-t08-tree (named exception, ratified in fix round 1): Task 8 replaced the stub with the
//! widget, and this file now checks the seam's contract against it: a collapsed tree shows
//! only its roots, an unloaded node is asked for when it expands, the status text reaches
//! assistive technology only while it is attached (no role and no item otherwise). The items
//! are read from `TreeView::a11y_items` (the view's copy), because synthetic AccessKit children
//! never reach `popover::a11y_record` (the grid's T40 subtree has the same limit); the root's
//! `Tree` role is a recorded declaration. The widget's full behaviour is `tests/tree.rs`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Entity, Focusable as _, Modifiers, MouseButton, Role, TestAppContext, VisualTestContext, point,
    px, size,
};
use hacivat_ui::popover::{a11y_record, a11y_take};
use hacivat_ui::text::{TextRegistry, TextSource};
use hacivat_ui::tree::{
    ChildState, Fold, RowStatus, StatusKind, TreeEvent, TreeKey, TreeModel, TreeRowData, TreeView,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};

/// Two roots; `orders` has loaded children, `audit` has children nobody loaded yet.
struct Model {
    rows: HashMap<TreeKey, (TreeRowData, ChildState)>,
    roots: Vec<TreeKey>,
    generation: u64,
}

const ORDERS: TreeKey = TreeKey(1);
const AUDIT: TreeKey = TreeKey(2);
const ID: TreeKey = TreeKey(3);

impl Model {
    fn new() -> Model {
        let row = |name: &str| TreeRowData::new(name.to_string(), TextSource::Catalog);
        let mut rows = HashMap::new();
        rows.insert(ORDERS, (row("orders"), ChildState::Loaded(Arc::from([ID]))));
        rows.insert(AUDIT, (row("audit"), ChildState::NotLoaded));
        rows.insert(ID, (row("id_column"), ChildState::Leaf));
        Model {
            rows,
            roots: vec![ORDERS, AUDIT],
            generation: 1,
        }
    }
}

impl TreeModel for Model {
    fn roots(&self) -> Arc<[TreeKey]> {
        self.roots.clone().into()
    }
    fn row(&self, key: TreeKey) -> Option<TreeRowData> {
        self.rows.get(&key).map(|(r, _)| r.clone())
    }
    fn children(&self, key: TreeKey) -> ChildState {
        self.rows
            .get(&key)
            .map_or(ChildState::Leaf, |(_, c)| c.clone())
    }
    fn generation(&self) -> u64 {
        self.generation
    }
}

type Tree = Entity<TreeView<Model>>;
type Events = Rc<RefCell<Vec<TreeEvent>>>;

fn open(cx: &mut TestAppContext, model: Model) -> (Tree, &mut VisualTestContext, Events) {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
    });
    let (tree, vcx) = cx.add_window_view(|_, cx| TreeView::new(model, cx));
    vcx.simulate_resize(size(px(320.0), px(400.0)));
    let events: Events = Rc::default();
    let sink = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&tree, move |_, e: &TreeEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach()
    });
    draw(vcx);
    (tree, vcx, events)
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

fn drawn(vcx: &mut VisualTestContext) -> Vec<String> {
    vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .map(|t| t.text.to_string())
            .collect()
    })
}

/// The centre of root row `ix` (23 px rows from the top of the window).
fn row_point(ix: usize) -> gpui::Point<gpui::Pixels> {
    point(px(40.0), px(23.0 * ix as f32 + 11.0))
}

fn mouse_down(vcx: &mut VisualTestContext, ix: usize, button: MouseButton, clicks: usize) {
    vcx.simulate_event(gpui::MouseDownEvent {
        position: row_point(ix),
        modifiers: Modifiers::none(),
        button,
        click_count: clicks,
        first_mouse: false,
    });
    vcx.simulate_event(gpui::MouseUpEvent {
        position: row_point(ix),
        modifiers: Modifiers::none(),
        button,
        click_count: clicks,
    });
    draw(vcx);
}

#[gpui::test]
fn flat_stub_renders_roots_and_emits_activate(cx: &mut TestAppContext) {
    let (tree, vcx, events) = open(cx, Model::new());
    // Only the roots are drawn, never a child, even when the model has loaded children.
    let texts = drawn(vcx);
    assert!(texts.contains(&"orders".to_string()), "{texts:?}");
    assert!(texts.contains(&"audit".to_string()), "{texts:?}");
    assert!(!texts.contains(&"id_column".to_string()), "{texts:?}");
    // ## m4-t08-tree: nothing was announced for a collapsed tree; expanding shows the loaded
    // child and asks the host for the unloaded node's children, once.
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    tree.update(vcx, |t, cx| t.set_expanded_keys([ORDERS, AUDIT], cx));
    draw(vcx);
    assert!(tree.read_with(vcx, |t, _| t.is_expanded(ORDERS)));
    assert!(drawn(vcx).contains(&"id_column".to_string()));
    assert_eq!(events.borrow().as_slice(), [TreeEvent::Expand(AUDIT)]);
    tree.update(vcx, |t, cx| t.set_expanded_keys([], cx));
    draw(vcx);
    assert!(!drawn(vcx).contains(&"id_column".to_string()));
    events.borrow_mut().clear();

    // A click selects; a double click activates.
    mouse_down(vcx, 1, MouseButton::Left, 1);
    assert_eq!(
        events.borrow().as_slice(),
        [TreeEvent::SelectionChanged(Some(AUDIT))]
    );
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(AUDIT));
    mouse_down(vcx, 1, MouseButton::Left, 2);
    assert_eq!(events.borrow().last(), Some(&TreeEvent::Activate(AUDIT)));

    // Enter activates the cursor row; a right click asks for the menu where it was clicked.
    events.borrow_mut().clear();
    tree.update(vcx, |t, cx| t.select(ORDERS, cx));
    let focus = tree.read_with(vcx, |t, cx| t.focus_handle(cx));
    vcx.update(|w, cx| w.focus(&focus, cx));
    vcx.simulate_keystrokes("enter");
    draw(vcx);
    assert_eq!(
        events.borrow().as_slice(),
        [
            TreeEvent::SelectionChanged(Some(ORDERS)),
            TreeEvent::Activate(ORDERS)
        ]
    );
    events.borrow_mut().clear();
    mouse_down(vcx, 0, MouseButton::Right, 1);
    assert_eq!(
        events.borrow().as_slice(),
        [TreeEvent::ContextMenu(ORDERS, row_point(0))]
    );

    // A key the model does not show is not selected.
    tree.update(vcx, |t, cx| t.select(ID, cx));
    assert_eq!(tree.read_with(vcx, |t, _| t.cursor()), Some(ORDERS));
    // R-M4-50: the widget's own type-ahead default is the locale fold, not Turkish-tolerant.
    assert_eq!(Fold::default(), Fold::Locale { turkic: false });
}

#[gpui::test]
fn status_slot_requires_a_text_alternative(cx: &mut TestAppContext) {
    let mut model = Model::new();
    // The status slot is its own field, and its text alternative is a required field of it.
    let status = RowStatus {
        kind: StatusKind::Modified,
        glyph: "M".into(),
        text: "Modified".into(),
    };
    model.rows.get_mut(&AUDIT).unwrap().0.status = Some(status);
    a11y_record();
    let (tree, vcx, _) = open(cx, model);
    // ## m4-t08-tree: no assistive technology, no role and no item.
    assert!(
        !a11y_take()
            .iter()
            .any(|d| matches!(d.role, Some(Role::Tree | Role::TreeItem)))
    );
    // The glyph is drawn after the label...
    let texts = drawn(vcx);
    assert!(texts.contains(&"M".to_string()), "{texts:?}");
    // ...and, with assistive technology attached, the row's tree item carries the text
    // alternative as its description, while a row without a status carries none.
    a11y_record();
    tree.update(vcx, |t, cx| t.assume_a11y_active(true, cx));
    draw(vcx);
    let decls = a11y_take();
    let items = tree.read_with(vcx, |t, _| t.a11y_items());
    let item = |name: &str| {
        items
            .iter()
            .find(|i| i.name.as_ref() == name)
            .cloned()
            .unwrap_or_else(|| panic!("no tree item {name}: {items:?}"))
    };
    let audit = item("audit");
    assert_eq!(audit.description.as_deref(), Some("Modified"));
    assert_eq!((audit.position, audit.set_size), (2, 2));
    assert_eq!(audit.node().role(), Role::TreeItem);
    assert_eq!(item("orders").description, None);
    // The list itself is a tree.
    assert!(
        decls.iter().any(|d| d.role == Some(Role::Tree)),
        "{decls:?}"
    );
}
