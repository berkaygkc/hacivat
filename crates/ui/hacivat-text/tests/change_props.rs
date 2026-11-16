//! UI-EDIT-03 properties (M3a Task 5): invert undoes apply; compose applies like one change
//! after the other and is associative; transform converges; mapping is monotonic; anchors map
//! like single positions; the change log's rebase equals mapping through every change since.
//!
//! Texts mix one-, two- and four-byte characters with CR, LF and `;`, and every edit falls on a
//! character boundary (as the model guarantees).

use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use hacivat_text::{
    AnchorSet, Annotations, Bias, ChangeSet, EditorModel, MapMode, ModelConfig, Origin, RangeKind,
    Rebased, SelectionSet, TextBuffer, Transaction, UserEvent,
};
use proptest::prelude::*;
use proptest::strategy::BoxedStrategy;

fn piece() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec!["a", "b", "ç", "İ", "ı", "😀", " ", "\r", "\n", ";"])
}

fn text_s() -> impl Strategy<Value = String> {
    prop::collection::vec(piece(), 0..24).prop_map(|p| p.concat())
}

fn ins_s() -> impl Strategy<Value = String> {
    prop::collection::vec(piece(), 0..4).prop_map(|p| p.concat())
}

fn boundaries(t: &str) -> Vec<usize> {
    t.char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(t.len()))
        .collect()
}

/// Byte-range edits of a text.
type Edits = Vec<(Range<usize>, Arc<str>)>;

/// Non-overlapping edits of `t` on character boundaries.
fn edits_on(t: String) -> impl Strategy<Value = (String, Edits)> {
    let n = boundaries(&t).len();
    prop::collection::vec((0..n, 0..4usize, ins_s()), 0..5).prop_map(move |mut raw| {
        let b = boundaries(&t);
        raw.sort_by_key(|(s, ..)| *s);
        let mut edits = Vec::new();
        let mut taken = 0;
        for (s, l, ins) in raw {
            if s < taken {
                continue;
            }
            let e = (s + l).min(b.len() - 1);
            edits.push((b[s]..b[e], Arc::from(ins.as_str())));
            taken = e;
        }
        (t.clone(), edits)
    })
}

fn apply(t: &str, c: &ChangeSet) -> String {
    let mut b = TextBuffer::from_str(t);
    c.apply(&mut b);
    b.to_string()
}

/// A change of `t` and the text it makes.
fn step(t: String) -> impl Strategy<Value = (ChangeSet, String)> {
    edits_on(t).prop_map(|(t, edits)| {
        let c = ChangeSet::from_edits(t.len(), edits).unwrap();
        let next = apply(&t, &c);
        (c, next)
    })
}

/// `n` changes, each of the text the one before made.
fn chain(t: String, n: usize) -> BoxedStrategy<Vec<ChangeSet>> {
    if n == 0 {
        return Just(Vec::new()).boxed();
    }
    step(t)
        .prop_flat_map(move |(c, next)| {
            chain(next, n - 1).prop_map(move |rest| {
                let mut all = vec![c.clone()];
                all.extend(rest);
                all
            })
        })
        .boxed()
}

fn chain3() -> impl Strategy<Value = (String, ChangeSet, ChangeSet, ChangeSet)> {
    text_s().prop_flat_map(|t| {
        chain(t.clone(), 3).prop_map(move |c| (t.clone(), c[0].clone(), c[1].clone(), c[2].clone()))
    })
}

/// A text and up to six changes made one after the other.
fn logs_s() -> impl Strategy<Value = (String, Vec<ChangeSet>)> {
    (text_s(), 0..7usize)
        .prop_flat_map(|(t, n)| chain(t.clone(), n).prop_map(move |c| (t.clone(), c)))
}

fn bias_s() -> impl Strategy<Value = Bias> {
    prop_oneof![Just(Bias::Before), Just(Bias::After)]
}

fn mode_s() -> impl Strategy<Value = MapMode> {
    prop_oneof![
        Just(MapMode::Simple),
        Just(MapMode::TrackDel),
        Just(MapMode::TrackBefore),
        Just(MapMode::TrackAfter)
    ]
}

fn kind_s() -> impl Strategy<Value = RangeKind> {
    prop_oneof![Just(RangeKind::Content), Just(RangeKind::Container)]
}

fn tr(changes: ChangeSet, selection: Option<SelectionSet>) -> Transaction {
    Transaction {
        changes,
        selection,
        ann: Annotations {
            user_event: UserEvent::Command("test"),
            add_to_history: true,
            time: Instant::now(),
            origin: Origin::Program,
        },
    }
}

/// `r` mapped through each change in turn, the way a reader holding the change sets would.
fn direct_map(changes: &[ChangeSet], r: Range<usize>, kind: RangeKind) -> Rebased {
    let mut r = r;
    for c in changes {
        match c.desc().map_range(r, kind) {
            Some(next) => r = next,
            None => return Rebased::Deleted,
        }
    }
    Rebased::Mapped(r)
}

proptest! {
    #[test]
    fn invert_undoes_apply((t, e) in text_s().prop_flat_map(edits_on)) {
        let c = ChangeSet::from_edits(t.len(), e).unwrap();
        let inv = c.invert(&TextBuffer::from_str(&t));
        prop_assert_eq!(inv.len_old(), c.len_new());
        prop_assert_eq!(inv.len_new(), c.len_old());
        prop_assert_eq!(apply(&apply(&t, &c), &inv), t);
    }

    #[test]
    fn compose_applies_like_one_change_after_the_other((t, a, b, _c) in chain3()) {
        let ab = a.compose(&b);
        prop_assert_eq!(ab.len_old(), a.len_old());
        prop_assert_eq!(ab.len_new(), b.len_new());
        prop_assert_eq!(apply(&t, &ab), apply(&apply(&t, &a), &b));
    }

    #[test]
    fn compose_is_associative((_t, a, b, c) in chain3()) {
        prop_assert_eq!(a.compose(&b).compose(&c), a.compose(&b.compose(&c)));
    }

    #[test]
    fn transform_converges(
        (t, a, b) in text_s().prop_flat_map(|t| {
            (step(t.clone()), step(t.clone())).prop_map(move |((a, _), (b, _))| (t.clone(), a, b))
        })
    ) {
        if let Some((a2, b2)) = a.transform(&b) {
            prop_assert_eq!(apply(&apply(&t, &b), &a2), apply(&apply(&t, &a), &b2));
        } else {
            // Refused only when the two touch the same text.
            let (a_ranges, b_ranges) = (replaced(&a), replaced(&b));
            prop_assert!(a_ranges.iter().any(|x| b_ranges.iter().any(|y| touches(x, y))));
        }
    }

    #[test]
    fn mapping_is_monotonic(
        (t, e) in text_s().prop_flat_map(edits_on),
        p in 0..64usize,
        q in 0..64usize,
        bias in bias_s(),
    ) {
        let c = ChangeSet::from_edits(t.len(), e).unwrap();
        let d = c.desc();
        let (lo, hi) = (p.min(q).min(t.len()), p.max(q).min(t.len()));
        let (mlo, mhi) = (
            d.map_pos(lo, bias, MapMode::Simple).unwrap(),
            d.map_pos(hi, bias, MapMode::Simple).unwrap(),
        );
        prop_assert!(mlo <= mhi);
        prop_assert!(mhi <= c.len_new());
    }

    #[test]
    fn anchors_map_like_single_positions(
        (t, e) in text_s().prop_flat_map(edits_on),
        raw in prop::collection::vec((0..64usize, bias_s(), mode_s()), 0..16),
    ) {
        let d = ChangeSet::from_edits(t.len(), e).unwrap().desc();
        let mut set = AnchorSet::default();
        let mut expected = Vec::new();
        for (i, (at, bias, mode)) in raw.into_iter().enumerate() {
            let at = at.min(t.len());
            set.insert(at, bias, mode, i);
            if let Some(m) = d.map_pos(at, bias, mode) {
                expected.push((m, i));
            }
        }
        set.map(&d);
        let got: Vec<(usize, usize)> = set.iter().map(|(o, i)| (o, *i)).collect();
        prop_assert!(got.windows(2).all(|w| w[0].0 <= w[1].0), "sorted: {:?}", got);
        let mut got_sorted = got;
        got_sorted.sort_unstable();
        expected.sort_unstable();
        prop_assert_eq!(got_sorted, expected);
    }

    #[test]
    fn rebase_equals_the_composed_mapping(
        (t, changes) in logs_s(),
        a in 0..64usize,
        b in 0..64usize,
        kind in kind_s(),
    ) {
        let r = a.min(b).min(t.len())..a.max(b).min(t.len());
        let mut m = EditorModel::new(&t, ModelConfig::default());
        for c in &changes {
            m.dispatch(tr(c.clone(), None)).unwrap();
            // Selection-only commits in between take no log entry.
            let len = m.buffer().len_bytes();
            m.dispatch(tr(ChangeSet::identity(len), Some(SelectionSet::cursor(len)))).unwrap();
        }
        // Only changes of the text make versions (and log entries).
        let changed = changes.iter().filter(|c| !c.is_identity()).count();
        prop_assert_eq!(m.version(), changed as u64);
        prop_assert_eq!(m.changelog().rebase(0, r.clone(), kind), direct_map(&changes, r, kind));
    }
}

/// The replaced ranges of `c` in its old text (insertions as empty ranges).
fn replaced(c: &ChangeSet) -> Vec<Range<usize>> {
    let mut at = 0;
    let mut out = Vec::new();
    for s in c.sections() {
        match s {
            hacivat_text::Section::Retain(n) => at += n,
            hacivat_text::Section::Replace { del, .. } => {
                out.push(at..at + del);
                at += del;
            }
        }
    }
    out
}

/// Two replaced ranges touch the same text: deletions that overlap, or an insertion strictly
/// inside a deletion.
fn touches(x: &Range<usize>, y: &Range<usize>) -> bool {
    let overlap = x.start < y.end && y.start < x.end;
    let inside = |p: usize, r: &Range<usize>| r.start < p && p < r.end;
    overlap || inside(x.start, y) || inside(y.start, x)
}
