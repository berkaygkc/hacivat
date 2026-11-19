//! The tree's pure parts (plan-m4 Task 8): flattening, type-ahead folding, accessibility items.

mod tree_fixture;

use hacivat_ui::tree::a11y::{ItemsIn, visible_items};
use hacivat_ui::tree::flat::{Flat, KeySet, RowKind};
use hacivat_ui::tree::typeahead::{Fold, TYPEAHEAD_RESET, TypeAhead, find, prefix_matches};
use hacivat_ui::tree::{ChildState, RowStatus, StatusKind, TreeKey};
use std::time::{Duration, Instant};
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

fn labels(f: &Fixture, flat: &Flat) -> Vec<String> {
    flat.rows()
        .iter()
        .map(|r| {
            let name = f.nodes[&r.key].row.label.to_string();
            let pad = "  ".repeat(r.depth as usize);
            match r.kind {
                RowKind::Node => format!("{pad}{name}"),
                RowKind::Loading => format!("{pad}<loading>"),
                RowKind::Failed => format!("{pad}<failed>"),
            }
        })
        .collect()
}

fn set(f: &Fixture, names: &[&str]) -> KeySet {
    names.iter().map(|n| f.key(n)).collect()
}

#[test]
fn collapsed_tree_is_its_roots() {
    let f = Fixture::parse(OUTLINE);
    let flat = Flat::build(&f, &KeySet::default(), &mut Vec::new());
    assert_eq!(labels(&f, &flat), ["db", "other"]);
}

#[test]
fn expanded_nodes_show_their_children_depth_first() {
    let f = Fixture::parse(OUTLINE);
    let exp = set(&f, &["db", "public", "Tables", "orders"]);
    let flat = Flat::build(&f, &exp, &mut Vec::new());
    assert_eq!(
        labels(&f, &flat),
        [
            "db",
            "  public",
            "    Tables",
            "      orders",
            "        id",
            "        total",
            "      users",
            "    Views",
            "  audit",
            "other"
        ]
    );
    // Position and set size among siblings.
    let orders = &flat.rows()[3];
    assert_eq!((orders.pos, orders.set_size), (1, 2));
    let views = &flat.rows()[7];
    assert_eq!((views.pos, views.set_size), (2, 2));
}

#[test]
fn expanded_not_loaded_is_reported_and_loading_failed_rows_appear() {
    let mut f = Fixture::parse(OUTLINE);
    let exp = set(&f, &["db", "audit"]);
    let mut nl = Vec::new();
    let flat = Flat::build(&f, &exp, &mut nl);
    assert_eq!(nl, [f.key("audit")]);
    assert_eq!(labels(&f, &flat), ["db", "  public", "  audit", "other"]);

    let audit = f.key("audit");
    f.set_children(
        audit,
        ChildState::Loading {
            label: "Loading…".into(),
            running: true,
            count: None,
        },
    );
    let mut nl = Vec::new();
    let flat = Flat::build(&f, &exp, &mut nl);
    assert!(nl.is_empty());
    assert_eq!(
        labels(&f, &flat),
        ["db", "  public", "  audit", "    <loading>", "other"]
    );
    f.set_children(
        audit,
        ChildState::Failed {
            label: "Could not read".into(),
            retry: None,
        },
    );
    let flat = Flat::build(&f, &exp, &mut Vec::new());
    assert_eq!(flat.rows()[3].kind, RowKind::Failed);
    assert_eq!(flat.rows()[3].key, audit);
}

#[test]
fn expand_and_collapse_splice_the_same_rows_as_a_rebuild() {
    let f = Fixture::parse(OUTLINE);
    let mut exp = set(&f, &["db", "Tables", "orders"]);
    let mut flat = Flat::build(&f, &exp, &mut Vec::new());
    assert_eq!(labels(&f, &flat), ["db", "  public", "  audit", "other"]);
    // Expanding `public` brings its already-expanded descendants with it.
    exp.insert(f.key("public"));
    let ix = flat.index_of(f.key("public")).unwrap();
    let n = flat.expand_at(ix, &f, &exp, &mut Vec::new());
    assert_eq!(n, 6);
    let rebuilt = Flat::build(&f, &exp, &mut Vec::new());
    assert_eq!(flat.rows(), rebuilt.rows());
    // A second expand is a no-op.
    assert_eq!(flat.expand_at(ix, &f, &exp, &mut Vec::new()), 0);
    // Collapse removes the whole visible subtree.
    exp.remove(&f.key("public"));
    assert_eq!(flat.collapse_at(ix), 6);
    assert_eq!(labels(&f, &flat), ["db", "  public", "  audit", "other"]);
}

#[test]
fn parent_and_subtree_end() {
    let f = Fixture::parse(OUTLINE);
    let exp = set(&f, &["db", "public", "Tables", "orders"]);
    let flat = Flat::build(&f, &exp, &mut Vec::new());
    let ix = |n: &str| flat.index_of(f.key(n)).unwrap();
    assert_eq!(flat.parent_of(ix("total")), Some(ix("orders")));
    assert_eq!(flat.parent_of(ix("users")), Some(ix("Tables")));
    assert_eq!(flat.parent_of(ix("db")), None);
    assert_eq!(flat.subtree_end(ix("public")), ix("audit"));
    assert!(flat.is_open(ix("orders")));
    assert!(!flat.is_open(ix("users")));
}

#[test]
fn deep_trees_flatten_without_recursion() {
    // 50,000 levels: an iterative walk, no stack overflow.
    let mut f = Fixture::flat_roots(0);
    let mut parent = None;
    let mut exp = KeySet::default();
    for i in 0..50_000 {
        let k = f.insert(parent, 0, &format!("n{i}"));
        exp.insert(k);
        parent = Some(k);
    }
    let flat = Flat::build(&f, &exp, &mut Vec::new());
    assert_eq!(flat.len(), 50_000);
    assert_eq!(flat.rows()[49_999].depth, 49_999);
}

// ---- type-ahead -------------------------------------------------------------------------------

#[test]
fn tolerant_fold_matches_every_i_and_turkish_pairs() {
    let t = Fold::Tolerant;
    for (text, typed) in [
        ("İSTANBUL", "i"),
        ("İSTANBUL", "ist"),
        ("ılık", "I"),
        ("ılık", "i"),
        ("Index", "ı"),
        ("Çarşı", "çarş"),
        ("ŞEHİR", "şehir"),
        ("I\u{307}zmir", "izm"),
        ("Orders", "ord"),
    ] {
        assert!(prefix_matches(text, typed, t), "{text} / {typed}");
    }
    assert!(!prefix_matches("users", "ord", t));
    assert!(!prefix_matches("ab", "abc", t));
    assert!(!prefix_matches("cari", "ç", t), "ç is not c");
}

#[test]
fn the_widget_default_fold_is_the_locale_fold() {
    // R-M4-50 (D-074): the host turns the tolerant fold on for Turkish users.
    assert_eq!(Fold::default(), Fold::Locale { turkic: false });
    assert!(!prefix_matches("ılık", "i", Fold::default()));
    assert!(prefix_matches("Index", "i", Fold::default()));
}

#[test]
fn locale_fold_is_the_access_key_fold() {
    let tr = Fold::Locale { turkic: true };
    assert!(prefix_matches("İSTANBUL", "i", tr));
    assert!(prefix_matches("ILIK", "ı", tr));
    assert!(!prefix_matches("ILIK", "i", tr));
    let en = Fold::Locale { turkic: false };
    assert!(prefix_matches("Index", "i", en));
    assert!(!prefix_matches("İSTANBUL", "i", en));
}

#[test]
fn typeahead_buffer_resets_after_a_pause() {
    let t0 = Instant::now();
    let mut ta = TypeAhead::default();
    assert_eq!(ta.push("o", t0), "o");
    assert_eq!(ta.push("r", t0 + Duration::from_millis(300)), "or");
    assert!(ta.is_active(t0 + Duration::from_millis(900)));
    let later = t0 + Duration::from_millis(300) + TYPEAHEAD_RESET;
    assert!(!ta.is_active(later));
    assert_eq!(ta.push("u", later), "u");
}

#[test]
fn find_from_cursor_wraps_and_cycles_on_a_repeated_letter() {
    let names = ["sales", "orders", "staff", "users", "stock"];
    let text = |ix: usize| Some(names[ix]);
    let t = Fold::Tolerant;
    // A longer prefix includes the cursor row itself.
    assert_eq!(find(5, Some(0), "sa", t, text), Some(0));
    assert_eq!(find(5, Some(0), "st", t, text), Some(2));
    // One letter (or the same letter repeated) cycles after the cursor, wrapping.
    assert_eq!(find(5, Some(0), "s", t, text), Some(2));
    assert_eq!(find(5, Some(2), "ss", t, text), Some(4));
    assert_eq!(find(5, Some(4), "sss", t, text), Some(0));
    assert_eq!(find(5, None, "u", t, text), Some(3));
    assert_eq!(find(5, Some(1), "x", t, text), None);
    // Rows that cannot be selected are skipped.
    let only_odd = |ix: usize| (ix % 2 == 1).then_some(names[ix]);
    assert_eq!(find(5, None, "s", t, only_odd), None);
}

// ---- accessibility items ----------------------------------------------------------------------

#[test]
fn items_carry_level_expanded_position_and_selection() {
    let mut f = Fixture::parse(OUTLINE);
    let orders = f.key("orders");
    f.row_mut(orders).status = Some(RowStatus {
        kind: StatusKind::Modified,
        glyph: "M".into(),
        text: "Modified".into(),
    });
    let exp = set(&f, &["db", "public", "Tables", "orders"]);
    let flat = Flat::build(&f, &exp, &mut Vec::new());
    let sel: KeySet = [orders].into_iter().collect();
    let items = visible_items(
        &f,
        ItemsIn {
            flat: &flat,
            visible: 0..flat.len(),
            overscan: 5,
            expanded: &exp,
            selection: &sel,
            cursor: Some(orders),
        },
    );
    assert_eq!(items.len(), flat.len());
    let by = |n: &str| items.iter().find(|i| i.name.as_ref() == n).unwrap();
    let o = by("orders");
    assert_eq!((o.level, o.position, o.set_size), (4, 1, 2));
    assert_eq!(o.expanded, Some(true));
    assert_eq!(o.selected, Some(true));
    assert!(o.focused);
    assert_eq!(o.description.as_deref(), Some("Modified"));
    let users = by("users");
    assert_eq!(users.expanded, None, "a leaf has no expanded state");
    assert_eq!(users.selected, Some(false));
    let header = by("Tables");
    assert_eq!(header.selected, None, "a group header is never selected");
    assert!(
        header.focusable,
        "a header that can expand takes the cursor (R-M4-49)"
    );
    assert!(!header.focused);
    assert_eq!(header.expanded, Some(true));
    assert_eq!(by("audit").expanded, Some(false));
    // Actions only on items that take the cursor.
    let actions = |i: &hacivat_ui::tree::a11y::ItemNode| {
        let node = i.node();
        hacivat_ui::tree::a11y::ITEM_ACTIONS
            .iter()
            .filter(|a| node.supports_action(**a))
            .count()
    };
    assert_eq!(actions(o), 6);
    assert_eq!(actions(header), 6);
}

#[test]
fn a_header_with_the_cursor_is_the_active_item_but_never_selected() {
    let f = Fixture::parse(OUTLINE);
    let exp = set(&f, &["db", "public", "Tables"]);
    let flat = Flat::build(&f, &exp, &mut Vec::new());
    let tables = f.key("Tables");
    let sel: KeySet = [f.key("public")].into_iter().collect();
    let items = visible_items(
        &f,
        ItemsIn {
            flat: &flat,
            visible: 0..flat.len(),
            overscan: 0,
            expanded: &exp,
            selection: &sel,
            cursor: Some(tables),
        },
    );
    let header = items.iter().find(|i| i.key == tables).unwrap();
    assert!(header.focused);
    assert_eq!(header.selected, None);
    let public = items.iter().find(|i| i.name.as_ref() == "public").unwrap();
    assert_eq!(public.selected, Some(true));
    assert!(!public.focused);
}

#[test]
fn a_header_without_children_is_a_label_the_keys_pass_over() {
    // Environment headers (NOTES M4 surface 1): no chevron, not collapsible, no cursor stop.
    let f = Fixture::parse("#Production 1\nprod_db\n#Local 1\nlocal_db\n");
    let flat = Flat::build(&f, &KeySet::default(), &mut Vec::new());
    let items = visible_items(
        &f,
        ItemsIn {
            flat: &flat,
            visible: 0..flat.len(),
            overscan: 0,
            expanded: &KeySet::default(),
            selection: &KeySet::default(),
            cursor: Some(f.key("Production")),
        },
    );
    let header = &items[0];
    assert_eq!(header.name.as_ref(), "Production");
    assert!(!header.focusable && !header.focused);
    assert_eq!((header.expanded, header.selected), (None, None));
    assert!(
        !header
            .node()
            .supports_action(hacivat_ui::tree::a11y::ITEM_ACTIONS[0])
    );
}

#[test]
fn loading_and_failed_items_are_named_by_their_message_and_take_no_action() {
    let mut f = Fixture::parse(OUTLINE);
    let audit = f.key("audit");
    f.set_children(
        audit,
        ChildState::Failed {
            label: "Couldn't read audit".into(),
            retry: None,
        },
    );
    let exp = set(&f, &["db", "audit"]);
    let flat = Flat::build(&f, &exp, &mut Vec::new());
    let items = visible_items(
        &f,
        ItemsIn {
            flat: &flat,
            visible: 0..flat.len(),
            overscan: 0,
            expanded: &exp,
            selection: &KeySet::default(),
            cursor: None,
        },
    );
    let failed = items.iter().find(|i| i.kind == RowKind::Failed).unwrap();
    assert_eq!(failed.name.as_ref(), "Couldn't read audit");
    assert_eq!((failed.level, failed.selected), (3, None));
    assert!(!failed.focusable);
    // Its id differs from its parent's (same key, another kind).
    assert_ne!(failed.id_key(), (audit.0, RowKind::Node as u8));
}

#[test]
fn key_hasher_spreads_keys_that_differ_in_either_half() {
    use std::hash::{BuildHasher, BuildHasherDefault};
    let b = BuildHasherDefault::<hacivat_ui::tree::flat::KeyHasher>::default();
    let h = |k: u128| b.hash_one(TreeKey(k));
    assert_ne!(h(1), h(2));
    assert_ne!(h(1 << 64), h(2 << 64));
    // Both halves count (swapped or equal halves stay apart), and a key is not its own hash.
    assert_ne!(h(1), h(1 << 64));
    assert_ne!(h((1 << 64) | 1), h(0));
    assert_ne!(h(7), 7);
}

#[test]
fn items_cover_the_visible_rows_plus_overscan() {
    let f = Fixture::flat_roots(1_000);
    let flat = Flat::build(&f, &KeySet::default(), &mut Vec::new());
    let items = visible_items(
        &f,
        ItemsIn {
            flat: &flat,
            visible: 100..140,
            overscan: 5,
            expanded: &KeySet::default(),
            selection: &KeySet::default(),
            cursor: None,
        },
    );
    assert_eq!(items.first().unwrap().row, 95);
    assert_eq!(items.last().unwrap().row, 144);
    assert_eq!(items.len(), 50);
    // Ids derive from the key, not the row.
    assert_eq!(items[0].id_key(), (TreeKey(96).0, RowKind::Node as u8));
}

// ## m4-t08-tree (fix round 1)

#[test]
fn key_hasher_spreads_structured_keys_over_the_bits_the_table_reads() {
    // M9: sequential keys in the low half, the high half, and both. hashbrown indexes buckets
    // by the low bits and tags them with the top 7.
    use std::collections::HashSet;
    use std::hash::{BuildHasher, BuildHasherDefault};
    let b = BuildHasherDefault::<hacivat_ui::tree::flat::KeyHasher>::default();
    type Family = (&'static str, fn(u128) -> u128);
    let families: [Family; 3] = [
        ("low", |i| i),
        ("high", |i| i << 64),
        ("both", |i| (i << 64) | i),
    ];
    for (name, key) in families {
        let hashes: Vec<u64> = (0..65_536u128)
            .map(|i| b.hash_one(TreeKey(key(i))))
            .collect();
        let all: HashSet<u64> = hashes.iter().copied().collect();
        let low: HashSet<u64> = hashes.iter().map(|h| h & 0xffff).collect();
        let top: HashSet<u64> = hashes.iter().map(|h| h >> 57).collect();
        assert_eq!(all.len(), 65_536, "{name}: no two keys share a hash");
        // A uniform hash fills about 63 % of 65,536 buckets.
        assert!(
            low.len() >= 35_000,
            "{name}: low 16 bits take {}",
            low.len()
        );
        assert_eq!(top.len(), 128, "{name}: top 7 bits take {}", top.len());
    }
}

#[test]
fn a_node_inside_its_own_subtree_is_shown_but_not_expanded_again() {
    // M7: a host bug (a cycle) never loops the flattening on the UI thread.
    let mut f = Fixture::parse(OUTLINE);
    let (public, orders) = (f.key("public"), f.key("orders"));
    f.set_children(orders, ChildState::Loaded(vec![public].into()));
    let exp = set(&f, &["db", "public", "Tables", "orders"]);
    let flat = Flat::build(&f, &exp, &mut Vec::new());
    assert_eq!(
        labels(&f, &flat),
        [
            "db",
            "  public",
            "    Tables",
            "      orders",
            "        public",
            "      users",
            "    Views",
            "  audit",
            "other"
        ]
    );
}

#[test]
fn rows_carry_what_they_are_to_the_keys() {
    // M5: the flatten reads headers and selectability once, so a key press scans bits.
    let f = Fixture::parse("#Env 1\nroot\n  #Tables 1\n    t\n");
    let exp = set(&f, &["root"]);
    let flat = Flat::build(&f, &exp, &mut Vec::new());
    let bits: Vec<_> = flat
        .rows()
        .iter()
        .map(|r| (r.is_header(), r.is_selectable(), r.is_focusable()))
        .collect();
    assert_eq!(
        bits,
        [
            // A header without children: a label.
            (true, false, false),
            (false, true, true),
            // A kind group: the cursor, never the selection.
            (true, false, true),
        ]
    );
}
