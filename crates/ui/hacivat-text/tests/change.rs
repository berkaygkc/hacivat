//! UI-EDIT-03 (M3a Task 5): change sets (canonical sections, apply, invert, compose,
//! transform), the four mapping modes, content and container ranges, anchors, and the
//! 256-entry change log.

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_text::{
    AnchorSet, Annotations, Bias, ChangeSet, EditorModel, MapMode, ModelConfig, Origin, RangeKind,
    Rebased, Section, SelectionSet, TextBuffer, Transaction, UserEvent,
};

use Bias::{After, Before};
use MapMode::{Simple, TrackAfter, TrackBefore, TrackDel};
use RangeKind::{Container, Content};

fn cs(len: usize, edits: &[(Range<usize>, &str)]) -> ChangeSet {
    ChangeSet::from_edits(len, edits.iter().map(|(r, s)| (r.clone(), Arc::from(*s)))).unwrap()
}

fn apply(t: &str, c: &ChangeSet) -> String {
    let mut b = TextBuffer::from_str(t);
    c.apply(&mut b);
    b.to_string()
}

fn tr(changes: ChangeSet) -> Transaction {
    Transaction {
        changes,
        selection: None,
        ann: Annotations {
            user_event: UserEvent::InputType,
            add_to_history: true,
            time: Instant::now(),
            origin: Origin::User,
        },
    }
}

/// Replaces `r` with `text` in `m`.
fn edit(m: &mut EditorModel, r: Range<usize>, text: &str) {
    let len = m.buffer().len_bytes();
    m.dispatch(tr(cs(len, &[(r, text)]))).unwrap();
}

#[test]
fn ui_edit_03_cases_1_to_6() {
    // 1. `SELECT a FROM t`, insert(7, "x").
    let d = cs(15, &[(7..7, "x")]).desc();
    assert_eq!(d.map_pos(7, Before, Simple), Some(7));
    assert_eq!(d.map_pos(7, After, Simple), Some(8));
    for bias in [Before, After] {
        assert_eq!(d.map_pos(9, bias, Simple), Some(10));
        assert_eq!(d.map_pos(3, bias, Simple), Some(3));
    }

    // 2. Delete [5, 10): an anchor inside collapses (Simple) or goes (TrackDel); touching is
    //    not spanning.
    let d = cs(15, &[(5..10, "")]).desc();
    assert_eq!(d.map_pos(7, Before, Simple), Some(5));
    assert_eq!(d.map_pos(7, After, Simple), Some(5));
    assert_eq!(d.map_pos(7, Before, TrackDel), None);
    assert_eq!(d.map_pos(5, After, TrackDel), Some(5));
    assert_eq!(d.map_pos(10, Before, TrackDel), Some(5));
    assert_eq!(d.map_pos(12, After, TrackDel), Some(7));
    // TrackBefore: the character before the position was deleted; TrackAfter: the one after.
    assert_eq!(d.map_pos(10, After, TrackBefore), None);
    assert_eq!(d.map_pos(5, After, TrackBefore), Some(5));
    assert_eq!(d.map_pos(5, After, TrackAfter), None);
    assert_eq!(d.map_pos(10, After, TrackAfter), Some(5));
    assert_eq!(d.map_pos(7, After, TrackBefore), None);
    assert_eq!(d.map_pos(7, After, TrackAfter), None);

    // 3. A content range does not grow with typing at its end; a container does.
    let d = cs(15, &[(8..8, "y")]).desc();
    assert_eq!(d.map_range(7..8, Content), Some(7..8));
    let d = cs(15, &[(15..15, "y")]).desc();
    assert_eq!(d.map_range(0..15, Container), Some(0..16));
    // ... and at its start: the content shifts, the container grows.
    let d = cs(15, &[(7..7, "zz")]).desc();
    assert_eq!(d.map_range(7..8, Content), Some(9..10));
    assert_eq!(d.map_range(7..8, Container), Some(7..10));

    // 4. compose(insert(0, "a"), insert(1, "b")) = insert(0, "ab").
    assert_eq!(
        cs(3, &[(0..0, "a")]).compose(&cs(4, &[(1..1, "b")])),
        cs(3, &[(0..0, "ab")])
    );

    // 5. The inverse of replace [0, 6) → "select" on `SELECT 1` restores `SELECT`.
    let old = TextBuffer::from_str("SELECT 1");
    let c = cs(8, &[(0..6, "select")]);
    assert_eq!(c.invert(&old), cs(8, &[(0..6, "SELECT")]));
    assert_eq!(apply(&apply("SELECT 1", &c), &c.invert(&old)), "SELECT 1");

    // 6. A completion range computed at v10, rebased at v13: an edit far after it leaves it
    //    alone; an edit inside it drops it.
    let mut m = EditorModel::new("SELECT na FROM t", ModelConfig::default());
    for _ in 0..10 {
        let len = m.buffer().len_bytes();
        edit(&mut m, len..len, " ");
    }
    assert_eq!(m.version(), 10);
    let replace = 7..9;
    for _ in 0..3 {
        let len = m.buffer().len_bytes();
        edit(&mut m, len..len, "--");
    }
    assert_eq!(m.version(), 13);
    assert_eq!(
        m.changelog().rebase(10, replace.clone(), Content),
        Rebased::Mapped(7..9)
    );
    edit(&mut m, 8..8, "m");
    assert_eq!(
        m.changelog().rebase(10, replace.clone(), Content),
        Rebased::Deleted
    );
    // The same range as a container follows the edit instead.
    assert_eq!(
        m.changelog().rebase(10, replace, Container),
        Rebased::Mapped(7..10)
    );
}

#[test]
fn a_position_at_a_replacement_keeps_to_its_side() {
    // Replace [4, 6) ("ab") with "XYZ" in "0123ab67".
    let d = cs(8, &[(4..6, "XYZ")]).desc();
    // At the start: before the new text, whatever the bias.
    assert_eq!(d.map_pos(4, Before, Simple), Some(4));
    assert_eq!(d.map_pos(4, After, Simple), Some(4));
    // Strictly inside: the bias picks the side.
    assert_eq!(d.map_pos(5, Before, Simple), Some(4));
    assert_eq!(d.map_pos(5, After, Simple), Some(7));
    // At the end: after the new text, whatever the bias.
    assert_eq!(d.map_pos(6, Before, Simple), Some(7));
    assert_eq!(d.map_pos(6, After, Simple), Some(7));
    assert_eq!(d.map_pos(8, Before, Simple), Some(9));
    // Past the end clamps to the end.
    assert_eq!(d.map_pos(99, Before, Simple), Some(9));
}

#[test]
fn from_edits_is_canonical() {
    // Adjacent edits merge into one replacement.
    assert_eq!(
        cs(5, &[(1..2, "x"), (2..3, "y")]).sections(),
        [
            Section::Retain(1),
            Section::Replace {
                del: 2,
                ins: Arc::from("xy")
            },
            Section::Retain(2)
        ]
    );
    // An empty edit is no edit.
    let empty = cs(5, &[(2..2, "")]);
    assert!(empty.is_identity());
    assert_eq!(empty, ChangeSet::identity(5));
    assert_eq!(ChangeSet::identity(5).sections(), [Section::Retain(5)]);
    assert_eq!(ChangeSet::identity(0).sections(), []);
    // Insertions at one point keep their input order; edits come in any order.
    assert_eq!(apply("xyz", &cs(3, &[(1..1, "a"), (1..1, "b")])), "xabyz");
    assert_eq!(apply("xyz", &cs(3, &[(2..3, "!"), (0..1, "")])), "y!");
    // Overlaps and edits past the end are refused.
    assert!(ChangeSet::from_edits(5, [(1..3, Arc::from("")), (2..4, Arc::from(""))]).is_err());
    assert!(ChangeSet::from_edits(5, [(4..6, Arc::from(""))]).is_err());
    let c = cs(9, &[(0..2, "çç"), (5..9, "")]);
    assert_eq!((c.len_old(), c.len_new()), (9, 7));
}

#[test]
fn invert_and_compose_handle_multibyte_text() {
    let t = "SEÇ ılık İ😀;";
    let old = TextBuffer::from_str(t);
    let c = cs(t.len(), &[(0..4, "seç"), (5..7, ""), (14..18, "x😀y")]);
    let after = apply(t, &c);
    assert_eq!(after, "seç lık İx😀y;");
    let inv = c.invert(&old);
    assert_eq!(apply(&after, &inv), t);
    // A change then its inverse changes nothing.
    assert_eq!(apply(t, &c.compose(&inv)), t);
    // Typing then deleting the same character composes to the identity.
    let typed = cs(3, &[(1..1, "ş")]);
    let deleted = cs(5, &[(1..3, "")]);
    assert!(typed.compose(&deleted).is_identity());
}

#[test]
fn transform_converges_on_disjoint_changes_and_refuses_overlaps() {
    let t = "SELECT 1; SELECT 2;";
    // A header inserted at 0, and an edit of the second statement.
    let a = cs(t.len(), &[(0..0, "-- h\n")]);
    let b = cs(t.len(), &[(17..18, "42")]);
    let (a2, b2) = a.transform(&b).expect("disjoint");
    assert_eq!(apply(&apply(t, &b), &a2), apply(&apply(t, &a), &b2));
    assert_eq!(apply(&apply(t, &b), &a2), "-- h\nSELECT 1; SELECT 42;");
    // Insertions at one point: the first set's text comes first.
    let x = cs(3, &[(1..1, "X")]);
    let y = cs(3, &[(1..1, "Y")]);
    let (x2, y2) = x.transform(&y).unwrap();
    assert_eq!(apply(&apply("abc", &y), &x2), "aXYbc");
    assert_eq!(apply(&apply("abc", &x), &y2), "aXYbc");
    // A deletion next to another deletion, and an insertion at a deletion's edge, converge.
    let d1 = cs(6, &[(1..3, "")]);
    let d2 = cs(6, &[(3..5, "")]);
    let (p, q) = d1.transform(&d2).unwrap();
    assert_eq!(apply(&apply("012345", &d2), &p), "05");
    assert_eq!(apply(&apply("012345", &d1), &q), "05");
    let edge = cs(6, &[(3..3, "!")]);
    let (p, q) = edge.transform(&d1).unwrap();
    assert_eq!(apply(&apply("012345", &d1), &p), "0!345");
    assert_eq!(apply(&apply("012345", &edge), &q), "0!345");
    // Overlapping deletions, and an insertion strictly inside a deletion, are refused.
    assert!(
        cs(6, &[(1..4, "")])
            .transform(&cs(6, &[(3..5, "")]))
            .is_none()
    );
    assert!(
        cs(6, &[(2..2, "?")])
            .transform(&cs(6, &[(1..4, "")]))
            .is_none()
    );
    assert!(
        cs(6, &[(1..4, "")])
            .transform(&cs(6, &[(2..2, "?")]))
            .is_none()
    );
}

#[test]
fn content_and_container_ranges_follow_their_rules() {
    // "0123456789": content [3, 6).
    let delete_inside = cs(10, &[(4..5, "")]).desc();
    assert_eq!(delete_inside.map_range(3..6, Content), None);
    assert_eq!(delete_inside.map_range(3..6, Container), Some(3..5));
    let overlapping = cs(10, &[(5..8, "")]).desc();
    assert_eq!(overlapping.map_range(3..6, Content), None);
    assert_eq!(overlapping.map_range(3..6, Container), Some(3..5));
    // Edits at the edges leave a content range's text alone.
    let around = cs(10, &[(1..3, "ab c"), (6..9, "")]).desc();
    assert_eq!(around.map_range(3..6, Content), Some(5..8));
    assert_eq!(around.map_range(3..6, Container), Some(5..8));
    // A container goes only when all of it was deleted.
    let swallow = cs(10, &[(2..7, "xyz")]).desc();
    assert_eq!(swallow.map_range(3..6, Container), None);
    assert_eq!(swallow.map_range(2..7, Container), None);
    assert_eq!(swallow.map_range(1..7, Container), Some(1..5));
    // An empty range is a point: content keeps to the text before it, a container grows.
    let typed = cs(10, &[(4..4, "ab")]).desc();
    assert_eq!(typed.map_range(4..4, Content), Some(4..4));
    assert_eq!(typed.map_range(4..4, Container), Some(4..6));
    assert_eq!(cs(10, &[(2..6, "")]).desc().map_range(4..4, Content), None);
    // Out-of-range input clamps.
    assert_eq!(typed.map_range(8..99, Content), Some(10..12));
}

#[test]
fn changed_range_covers_every_edit_in_the_new_text() {
    let d = cs(10, &[(1..2, "xyz"), (6..9, "")]).desc();
    assert_eq!(d.changed_range(), Some(1..8));
    assert_eq!(ChangeSet::identity(10).desc().changed_range(), None);
    assert_eq!(cs(3, &[(3..3, "!")]).desc().changed_range(), Some(3..4));
}

#[test]
fn anchors_map_in_one_pass_and_drop_tracked_deletions() {
    let mut a = AnchorSet::default();
    a.insert(0, After, Simple, "start");
    a.insert(4, Before, TrackDel, "deleted");
    a.insert(5, After, Simple, "inside, after");
    a.insert(5, Before, Simple, "inside, before");
    a.insert(8, Before, TrackDel, "touching");
    a.insert(9, After, TrackBefore, "late");
    assert_eq!(a.len(), 6);
    // Delete [3, 8) and insert "ab" there: "inside" collapses to either side of "ab". At one
    // offset, `Before` anchors come first.
    a.map(&cs(10, &[(3..8, "ab")]).desc());
    assert_eq!(
        a.iter().collect::<Vec<_>>(),
        [
            (0, &"start"),
            (3, &"inside, before"),
            (5, &"touching"),
            (5, &"inside, after"),
            (6, &"late"),
        ]
    );
    // Mapped again through an insertion at 5: the `Before` anchor stays, the `After` one moves.
    a.map(&cs(7, &[(5..5, "!")]).desc());
    assert_eq!(
        a.iter().map(|(o, _)| o).collect::<Vec<_>>(),
        [0, 3, 5, 6, 7]
    );
    a.retain(|v| !v.starts_with("inside"));
    assert_eq!(a.len(), 3);
    assert!(!a.is_empty());
    assert!(AnchorSet::<u8>::default().is_empty());
}

#[test]
fn anchors_at_one_position_each_see_the_change() {
    // Both anchors sit at the end of a deletion: the one-pass mapping must not pass the
    // deletion after the first, or the tracking one would survive (proptest found this).
    let mut a = AnchorSet::default();
    a.insert(1, Before, Simple, "kept");
    a.insert(1, Before, TrackBefore, "tracked");
    a.map(&cs(1, &[(0..1, "")]).desc());
    assert_eq!(a.iter().collect::<Vec<_>>(), [(0, &"kept")]);
    // At a pure insertion, a `Before` anchor added after an `After` one still keeps its side.
    let mut a = AnchorSet::default();
    a.insert(2, After, Simple, "after");
    a.insert(2, Before, Simple, "before");
    a.map(&cs(4, &[(2..2, "xy")]).desc());
    assert_eq!(
        a.iter().collect::<Vec<_>>(),
        [(2, &"before"), (4, &"after")]
    );
}

#[test]
fn the_change_log_keeps_the_last_256_changes() {
    let mut m = EditorModel::new("", ModelConfig::default());
    for _ in 0..300 {
        let len = m.buffer().len_bytes();
        edit(&mut m, len..len, "x");
        // A selection-only commit uses no log entry and keeps the version.
        let len = m.buffer().len_bytes();
        let mut sel = tr(ChangeSet::identity(len));
        sel.selection = Some(SelectionSet::cursor(0));
        m.dispatch(sel).unwrap();
    }
    assert_eq!(m.version(), 300);
    let log = m.changelog();
    assert_eq!(log.version(), 300);
    // At v44 the text was 44 bytes; every later change appended at its end.
    assert_eq!(log.rebase(44, 40..44, Container), Rebased::Mapped(40..300));
    assert_eq!(log.rebase(44, 40..44, Content), Rebased::Mapped(40..44));
    assert_eq!(log.rebase(43, 40..43, Container), Rebased::TooOld);
    assert_eq!(log.rebase(300, 2..3, Content), Rebased::Mapped(2..3));
    assert_eq!(log.rebase(301, 2..3, Content), Rebased::TooOld);
    assert_eq!(log.descs_since(44).map(Iterator::count), Some(256));
    assert!(log.descs_since(43).is_none());
    assert_eq!(log.descs_since(300).map(Iterator::count), Some(0));
}

#[test]
#[ignore = "perf"]
fn anchor_map_is_linear_in_anchors_plus_sections() {
    if std::env::var_os("HACIVAT_PERF").is_none_or(|v| v != "1") {
        return;
    }
    // 1M anchors through a change of 1M sections: quadratic mapping would never finish.
    let len = 4_000_000;
    let edits: Vec<(Range<usize>, Arc<str>)> = (0..len / 8)
        .map(|i| (i * 8..i * 8 + 1, Arc::from("xy")))
        .collect();
    let desc = ChangeSet::from_edits(len, edits).unwrap().desc();
    let mut a = AnchorSet::default();
    for i in 0..1_000_000 {
        a.insert(i * 4, After, Simple, ());
    }
    let start = Instant::now();
    a.map(&desc);
    let took = start.elapsed();
    assert_eq!(a.len(), 1_000_000);
    assert!(
        took < Duration::from_millis(250),
        "AnchorSet::map took {took:?}"
    );
}
