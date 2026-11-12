use hacivat_ui::panes::{Axis, FocusDir, Pane, PaneId, PaneNode, PaneTree, TabId};

/// One pane (id 0) holding `tabs`, with `active` as the active index.
fn tree_with_tabs(tabs: &[u32], active: usize) -> PaneTree {
    let mut t = PaneTree::new(TabId(tabs[0]));
    let pane = t.panes.get_mut(&PaneId(0)).unwrap();
    pane.tabs = tabs.iter().map(|&n| TabId(n)).collect();
    pane.active = active;
    t
}

/// pane 0 | (pane 1 over pane 2).
fn sample_tree() -> PaneTree {
    let mut t = PaneTree::new(TabId(1));
    let right = t.split(PaneId(0), Axis::Horizontal, TabId(2));
    t.split(right, Axis::Vertical, TabId(3));
    t
}

fn ratio(t: &PaneTree) -> f32 {
    match &t.root {
        PaneNode::Split { ratio, .. } => *ratio,
        PaneNode::Leaf(_) => panic!("root is a leaf"),
    }
}

#[test]
fn split_then_close_collapses() {
    let mut t = PaneTree::new(TabId(1));
    let p2 = t.split(PaneId(0), Axis::Horizontal, TabId(2));
    assert_eq!(t.close_tab(p2, TabId(2)), None);
    assert_eq!(t.root, PaneNode::Leaf(PaneId(0)));
}

#[test]
fn closing_active_tab_activates_left_neighbour() {
    let mut t = tree_with_tabs(&[1, 2, 3], 2);
    assert_eq!(t.close_tab(PaneId(0), TabId(3)), Some(TabId(2)));
}

#[test]
fn pane_tree_round_trips_json() {
    let t = sample_tree();
    assert_eq!(
        serde_json::from_value::<PaneTree>(serde_json::to_value(&t).unwrap()).unwrap(),
        t
    );
}

#[test]
fn ratio_is_clamped() {
    let mut t = sample_tree();
    t.set_ratio(&[], 0.01);
    assert_eq!(ratio(&t), 0.1);
    t.set_ratio(&[], 0.99);
    assert_eq!(ratio(&t), 0.9);
}

#[test]
fn closing_the_first_tab_activates_its_right_neighbour() {
    let mut t = tree_with_tabs(&[1, 2, 3], 0);
    assert_eq!(t.close_tab(PaneId(0), TabId(1)), Some(TabId(2)));
    assert_eq!(t.panes[&PaneId(0)].tabs, vec![TabId(2), TabId(3)]);
}

#[test]
fn closing_an_inactive_tab_keeps_the_active_tab() {
    let mut t = tree_with_tabs(&[1, 2, 3], 2);
    assert_eq!(t.close_tab(PaneId(0), TabId(1)), Some(TabId(3)));
    assert_eq!(t.panes[&PaneId(0)].active, 1);
}

#[test]
fn closing_the_last_tab_of_the_only_pane_keeps_an_empty_pane() {
    let mut t = PaneTree::new(TabId(1));
    assert_eq!(t.close_tab(PaneId(0), TabId(1)), None);
    assert_eq!(t.root, PaneNode::Leaf(PaneId(0)));
    assert_eq!(
        t.panes[&PaneId(0)],
        Pane {
            tabs: vec![],
            active: 0
        }
    );
}

#[test]
fn split_focuses_the_new_pane_and_closing_it_returns_focus() {
    let mut t = PaneTree::new(TabId(1));
    let p2 = t.split(PaneId(0), Axis::Horizontal, TabId(2));
    assert_eq!(p2, PaneId(1));
    assert_eq!(t.focused, p2);
    t.close_tab(p2, TabId(2));
    assert_eq!(t.focused, PaneId(0));
}

#[test]
fn move_focus_follows_the_layout() {
    // pane 0 | (pane 1 over pane 2)
    let mut t = sample_tree();
    t.activate(PaneId(0), 0);
    t.move_focus(FocusDir::Right);
    assert_eq!(
        t.focused,
        PaneId(1),
        "right from the left pane: the top-right pane"
    );
    t.move_focus(FocusDir::Down);
    assert_eq!(t.focused, PaneId(2));
    t.move_focus(FocusDir::Down);
    assert_eq!(t.focused, PaneId(2), "nothing below: focus stays");
    t.move_focus(FocusDir::Left);
    assert_eq!(t.focused, PaneId(0));
    t.move_focus(FocusDir::Up);
    assert_eq!(t.focused, PaneId(0));
}

#[test]
fn next_and_prev_tab_wrap_in_the_focused_pane() {
    let mut t = tree_with_tabs(&[1, 2, 3], 2);
    t.next_tab();
    assert_eq!(t.panes[&PaneId(0)].active, 0);
    t.prev_tab();
    assert_eq!(t.panes[&PaneId(0)].active, 2);
}

#[test]
fn activate_ignores_an_out_of_range_index() {
    let mut t = tree_with_tabs(&[1, 2], 0);
    t.activate(PaneId(0), 5);
    assert_eq!(t.panes[&PaneId(0)].active, 0);
    t.activate(PaneId(0), 1);
    assert_eq!(t.panes[&PaneId(0)].active, 1);
}

#[test]
fn set_ratio_follows_a_path_and_ignores_a_path_that_misses() {
    let mut t = sample_tree();
    // root = Split(a: pane 0, b: Split(pane 1, pane 2)).
    t.set_ratio(&[true], 0.3);
    let PaneNode::Split { b, .. } = &t.root else {
        panic!()
    };
    let PaneNode::Split { ratio, .. } = &**b else {
        panic!()
    };
    assert_eq!(*ratio, 0.3);
    let before = t.clone();
    t.set_ratio(&[false, true], 0.7);
    assert_eq!(t, before);
}
