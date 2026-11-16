//! Change sets and their mapping (UI-EDIT-03). Offsets are bytes.
//!
//! A [`ChangeSet`] is a run of sections over the old text, `Retain(n)` and
//! `Replace { del, ins }`, always in canonical form: no empty section, no two neighbours of the
//! same kind. So two sets are equal exactly when they make the same edit, and
//! [`ChangeSet::compose`] is associative as a value, not just in effect.
//!
//! **Mapping** ([`ChangeDesc::map_pos`]) follows the CodeMirror model, re-implemented. A
//! replacement deletes its old bytes and inserts its text at their start:
//! - a position before or after every change shifts by what changed before it;
//! - a position at the start of a replacement that deletes text stays before the new text, and
//!   one at its end goes after it, whatever the bias;
//! - a position strictly inside a deletion, or exactly at a pure insertion, goes before the new
//!   text with [`Bias::Before`] and after it with [`Bias::After`];
//! - the tracking modes answer `None` instead when the deletion took the position's
//!   surroundings: [`MapMode::TrackDel`] when it spans the position (not just touches it),
//!   [`MapMode::TrackBefore`] when the character before it went, [`MapMode::TrackAfter`] when
//!   the one after it went.
//!
//! **Ranges** ([`ChangeDesc::map_range`]): a [`RangeKind::Content`] range is text that matters
//! as it is (a highlighted token, a find hit, a completion's replace range): any edit inside it
//! drops it (`None`), and edits at its edges leave it as it was, never growing it. A
//! [`RangeKind::Container`] range holds text (a statement's execution mark, the Run preview, a
//! fold): it maps its ends `(Before, After)`, so typing at either edge grows it, and it goes only
//! when an edit deleted all of it.
//!
//! [`ChangeSet::transform`] rebases one of two changes of the same text over the other, for
//! history kept across a non-history edit and for queued observer transactions; it refuses
//! (`None`) when the two touch the same text.

use std::ops::Range;
use std::sync::Arc;

use crate::buffer::{Bias, TextBuffer};

/// One run of a change set over the old text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Section {
    /// Keep this many bytes.
    Retain(usize),
    /// Delete `del` bytes and insert `ins`.
    Replace { del: usize, ins: Arc<str> },
}

/// An edit of a whole text: sections covering the old text in order, in canonical form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChangeSet {
    sections: Vec<Section>,
    len_old: usize,
    len_new: usize,
}

/// A change set's lengths only: what mapping needs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ChangeDesc {
    /// `(old bytes, new bytes, replaced)` per section.
    parts: Vec<(usize, usize, bool)>,
    len_old: usize,
    len_new: usize,
}

/// How a position maps through a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapMode {
    Simple,
    /// `None` when a deletion spans the position (touching it is not spanning it).
    TrackDel,
    /// `None` when the character before the position was deleted.
    TrackBefore,
    /// `None` when the character after the position was deleted.
    TrackAfter,
}

/// How a range maps (module docs): content ranges are dropped by any edit inside them and never
/// grow; container ranges grow with insertions at their edges and go only when all of them was
/// deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeKind {
    Content,
    Container,
}

/// Two edits of `from_edits` overlap, or one lies outside the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverlapError;

/// What a change set is built from, a piece at a time; it keeps the canonical form.
#[derive(Default)]
struct Builder {
    sections: Vec<Section>,
    /// The replacement being built: bytes deleted and text inserted so far.
    del: usize,
    ins: Ins,
    pending: bool,
    len_old: usize,
    len_new: usize,
}

/// A replacement's text while it is built: one shared piece is kept as it is (no copy); a
/// second piece makes a copy.
#[derive(Default)]
enum Ins {
    #[default]
    Empty,
    One(Arc<str>),
    Many(String),
}

impl Builder {
    fn retain(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        self.flush();
        self.len_old += n;
        self.len_new += n;
        if let Some(Section::Retain(m)) = self.sections.last_mut() {
            *m += n;
        } else {
            self.sections.push(Section::Retain(n));
        }
    }

    fn delete(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        self.pending = true;
        self.del += n;
        self.len_old += n;
    }

    fn insert(&mut self, text: &Arc<str>) {
        if text.is_empty() {
            return;
        }
        self.pending = true;
        self.len_new += text.len();
        self.ins = match std::mem::take(&mut self.ins) {
            Ins::Empty => Ins::One(text.clone()),
            Ins::One(a) => Ins::Many(format!("{a}{text}")),
            Ins::Many(mut s) => {
                s.push_str(text);
                Ins::Many(s)
            }
        };
    }

    fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.pending = true;
        self.len_new += text.len();
        self.ins = match std::mem::take(&mut self.ins) {
            Ins::Empty => Ins::Many(text.to_owned()),
            Ins::One(a) => Ins::Many(format!("{a}{text}")),
            Ins::Many(mut s) => {
                s.push_str(text);
                Ins::Many(s)
            }
        };
    }

    fn flush(&mut self) {
        if !self.pending {
            return;
        }
        let ins = match std::mem::take(&mut self.ins) {
            Ins::Empty => Arc::from(""),
            Ins::One(a) => a,
            Ins::Many(s) => Arc::from(s),
        };
        self.sections.push(Section::Replace { del: self.del, ins });
        self.del = 0;
        self.pending = false;
    }

    fn finish(mut self) -> ChangeSet {
        self.flush();
        ChangeSet {
            sections: self.sections,
            len_old: self.len_old,
            len_new: self.len_new,
        }
    }
}

/// A section as consumed piece by piece: a replacement is its insertion (at its start), then
/// its deletion.
#[derive(Debug, Clone)]
enum Op {
    Retain(usize),
    Delete(usize),
    Insert(Arc<str>),
}

fn ops(sections: &[Section]) -> Vec<Op> {
    let mut out = Vec::with_capacity(sections.len() + 1);
    for s in sections {
        match s {
            Section::Retain(n) => out.push(Op::Retain(*n)),
            Section::Replace { del, ins } => {
                if !ins.is_empty() {
                    out.push(Op::Insert(ins.clone()));
                }
                if *del > 0 {
                    out.push(Op::Delete(*del));
                }
            }
        }
    }
    out
}

/// A cursor over ops: the current op and how much of it is consumed.
struct Cursor {
    ops: Vec<Op>,
    i: usize,
    off: usize,
}

impl Cursor {
    fn new(sections: &[Section]) -> Self {
        Cursor {
            ops: ops(sections),
            i: 0,
            off: 0,
        }
    }

    fn peek(&self) -> Option<&Op> {
        self.ops.get(self.i)
    }

    /// What is left of the current op.
    fn rem(&self) -> usize {
        match self.peek() {
            Some(Op::Retain(n) | Op::Delete(n)) => n - self.off,
            Some(Op::Insert(s)) => s.len() - self.off,
            None => 0,
        }
    }

    /// The current op is a deletion that has been partly consumed.
    fn inside_delete(&self) -> bool {
        matches!(self.peek(), Some(Op::Delete(_))) && self.off > 0
    }

    /// Consumes `k` bytes of the current op (at most what is left of it).
    fn advance(&mut self, k: usize) {
        let len = match self.peek() {
            Some(Op::Retain(n) | Op::Delete(n)) => *n,
            Some(Op::Insert(s)) => s.len(),
            None => return,
        };
        self.off += k;
        if self.off >= len {
            self.i += 1;
            self.off = 0;
        }
    }

    /// `k` bytes of the current insertion, shared when they are all of it.
    fn take_insert(&mut self, k: usize) -> Piece {
        let Some(Op::Insert(s)) = self.peek() else {
            unreachable!("take_insert on a non-insertion");
        };
        let piece = if self.off == 0 && k == s.len() {
            Piece::Shared(s.clone())
        } else {
            let part = s
                .get(self.off..self.off + k)
                .expect("change sets cut inserted text on character boundaries");
            Piece::Owned(part.to_owned())
        };
        self.advance(k);
        piece
    }
}

enum Piece {
    Shared(Arc<str>),
    Owned(String),
}

impl Builder {
    fn insert_piece(&mut self, p: Piece) {
        match p {
            Piece::Shared(a) => self.insert(&a),
            Piece::Owned(s) => self.insert_str(&s),
        }
    }
}

impl ChangeSet {
    /// A set from byte-range edits of a text of `len_old` bytes, in any order; overlapping
    /// edits or an edit past the end are an error. Insertions at one point keep their input
    /// order; touching edits merge into one replacement (the canonical form).
    pub fn from_edits(
        len_old: usize,
        edits: impl IntoIterator<Item = (Range<usize>, Arc<str>)>,
    ) -> Result<Self, OverlapError> {
        let mut edits: Vec<(Range<usize>, Arc<str>)> = edits.into_iter().collect();
        edits.sort_by_key(|(r, _)| (r.start, r.end));
        let mut b = Builder::default();
        let mut at = 0;
        for (r, ins) in edits {
            if r.start < at || r.end < r.start || r.end > len_old {
                return Err(OverlapError);
            }
            b.retain(r.start - at);
            b.delete(r.end - r.start);
            b.insert(&ins);
            at = r.end;
        }
        b.retain(len_old - at);
        Ok(b.finish())
    }

    /// The set that changes nothing in a text of `len` bytes (a selection-only transaction).
    pub fn identity(len: usize) -> Self {
        let mut b = Builder::default();
        b.retain(len);
        b.finish()
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    pub fn len_old(&self) -> usize {
        self.len_old
    }

    pub fn len_new(&self) -> usize {
        self.len_new
    }

    /// No section replaces anything.
    pub fn is_identity(&self) -> bool {
        self.sections
            .iter()
            .all(|s| matches!(s, Section::Retain(_)))
    }

    /// Applies the set to `b`, from the end of the text backwards so earlier offsets stay
    /// valid: O(sections · log n + bytes changed).
    ///
    /// # Panics
    /// When `b` is not `len_old()` bytes long, or a replaced range is not on character
    /// boundaries (the model snaps every set before applying it).
    pub fn apply(&self, b: &mut TextBuffer) {
        assert_eq!(
            b.len_bytes(),
            self.len_old,
            "a change set applies to a text of its old length"
        );
        let mut at = self.len_old;
        for s in self.sections.iter().rev() {
            match s {
                Section::Retain(n) => at -= n,
                Section::Replace { del, ins } => {
                    at -= del;
                    assert!(
                        b.is_char_boundary(at) && b.is_char_boundary(at + del),
                        "a change set replaces whole characters"
                    );
                    b.replace(at..at + del, ins);
                }
            }
        }
    }

    /// The set that undoes this one: applied to the text this one makes, it gives back `old`,
    /// the text this one applies to. O(sections + bytes deleted).
    ///
    /// # Panics
    /// When `old` is not `len_old()` bytes long.
    pub fn invert(&self, old: &TextBuffer) -> ChangeSet {
        assert_eq!(
            old.len_bytes(),
            self.len_old,
            "a change set inverts against the text it applies to"
        );
        let mut b = Builder::default();
        let mut at = 0;
        for s in &self.sections {
            match s {
                Section::Retain(n) => {
                    b.retain(*n);
                    at += n;
                }
                Section::Replace { del, ins } => {
                    b.delete(ins.len());
                    b.insert_str(&old.slice(at..at + del));
                    at += del;
                }
            }
        }
        b.finish()
    }

    /// This set, then `next` (which applies to the text this one makes): one set doing both.
    /// O(sections of both + bytes split); an inserted text passed through whole is shared, not
    /// copied.
    ///
    /// # Panics
    /// When `next.len_old()` is not this set's `len_new()`, or `next` cuts this set's inserted
    /// text inside a character.
    pub fn compose(&self, next: &ChangeSet) -> ChangeSet {
        assert_eq!(
            self.len_new, next.len_old,
            "`next` applies to the text this set makes"
        );
        let mut a = Cursor::new(&self.sections);
        let mut b = Cursor::new(&next.sections);
        let mut out = Builder::default();
        loop {
            // This set's deletions consume old text and make nothing `next` sees.
            if let Some(Op::Delete(_)) = a.peek() {
                let n = a.rem();
                out.delete(n);
                a.advance(n);
                continue;
            }
            // `next`'s insertions make new text from nothing of this set's output.
            if let Some(Op::Insert(_)) = b.peek() {
                let n = b.rem();
                let piece = b.take_insert(n);
                out.insert_piece(piece);
                continue;
            }
            let k = a.rem().min(b.rem());
            match (a.peek(), b.peek()) {
                (None, None) => break,
                (Some(Op::Retain(_)), Some(Op::Retain(_))) => {
                    out.retain(k);
                    a.advance(k);
                    b.advance(k);
                }
                (Some(Op::Retain(_)), Some(Op::Delete(_))) => {
                    out.delete(k);
                    a.advance(k);
                    b.advance(k);
                }
                (Some(Op::Insert(_)), Some(Op::Retain(_))) => {
                    let piece = a.take_insert(k);
                    out.insert_piece(piece);
                    b.advance(k);
                }
                (Some(Op::Insert(_)), Some(Op::Delete(_))) => {
                    // Inserted, then deleted: nothing.
                    a.advance(k);
                    b.advance(k);
                }
                _ => unreachable!("lengths were checked"),
            }
        }
        out.finish()
    }

    /// Rebases two changes of the same text over each other: `(self', other')`, where `self'`
    /// applies after `other` and `other'` after `self`, and both orders make the same text.
    /// Insertions at one point put this set's text first. `None` when the two touch the same
    /// text: their deletions overlap, or one inserts strictly inside the other's deletion.
    /// O(sections of both).
    ///
    /// # Panics
    /// When the two are not changes of texts of one length.
    pub fn transform(&self, other: &ChangeSet) -> Option<(ChangeSet, ChangeSet)> {
        assert_eq!(
            self.len_old, other.len_old,
            "transform takes two changes of the same text"
        );
        let mut a = Cursor::new(&self.sections);
        let mut b = Cursor::new(&other.sections);
        let mut a2 = Builder::default();
        let mut b2 = Builder::default();
        loop {
            if let Some(Op::Insert(_)) = a.peek() {
                if b.inside_delete() {
                    return None;
                }
                let n = a.rem();
                let piece = a.take_insert(n);
                a2.insert_piece(piece);
                b2.retain(n);
                continue;
            }
            if let Some(Op::Insert(_)) = b.peek() {
                if a.inside_delete() {
                    return None;
                }
                let n = b.rem();
                let piece = b.take_insert(n);
                b2.insert_piece(piece);
                a2.retain(n);
                continue;
            }
            let k = a.rem().min(b.rem());
            match (a.peek(), b.peek()) {
                (None, None) => break,
                (Some(Op::Retain(_)), Some(Op::Retain(_))) => {
                    a2.retain(k);
                    b2.retain(k);
                }
                // Bytes one side deleted are gone for the other side's rebased set.
                (Some(Op::Delete(_)), Some(Op::Retain(_))) => a2.delete(k),
                (Some(Op::Retain(_)), Some(Op::Delete(_))) => b2.delete(k),
                (Some(Op::Delete(_)), Some(Op::Delete(_))) => return None,
                _ => unreachable!("lengths were checked"),
            }
            a.advance(k);
            b.advance(k);
        }
        Some((a2.finish(), b2.finish()))
    }

    /// The set's lengths.
    pub fn desc(&self) -> ChangeDesc {
        ChangeDesc {
            parts: self
                .sections
                .iter()
                .map(|s| match s {
                    Section::Retain(n) => (*n, *n, false),
                    Section::Replace { del, ins } => (*del, ins.len(), true),
                })
                .collect(),
            len_old: self.len_old,
            len_new: self.len_new,
        }
    }

    /// The bytes history keeps for this set: every byte it deletes or inserts (undoing it
    /// restores the deleted ones; redoing it, the inserted ones).
    pub(crate) fn weight(&self) -> u64 {
        self.sections
            .iter()
            .map(|s| match s {
                Section::Retain(_) => 0,
                Section::Replace { del, ins } => (del + ins.len()) as u64,
            })
            .sum()
    }

    /// Each replaced range of the old text with its inserted text, in order.
    pub(crate) fn edits(&self) -> impl Iterator<Item = (Range<usize>, &Arc<str>)> + '_ {
        let mut at = 0;
        self.sections.iter().filter_map(move |s| match s {
            Section::Retain(n) => {
                at += n;
                None
            }
            Section::Replace { del, ins } => {
                let r = at..at + del;
                at += del;
                Some((r, ins))
            }
        })
    }
}

/// Maps non-decreasing positions through one change in a single pass: each query resumes
/// where the last one stopped, so k positions cost O(k + sections).
pub(crate) struct Mapper<'a> {
    parts: &'a [(usize, usize, bool)],
    i: usize,
    old: usize,
    new: usize,
    len_old: usize,
}

impl Mapper<'_> {
    /// Where `pos` lands (the module docs). Positions past the end clamp to it.
    pub(crate) fn map(&mut self, pos: usize, bias: Bias, mode: MapMode) -> Option<usize> {
        let pos = pos.min(self.len_old);
        while let Some(&(del, ins, replaced)) = self.parts.get(self.i) {
            let end = self.old + del;
            if !replaced {
                if end > pos {
                    return Some(self.new + pos - self.old);
                }
            } else {
                if end >= pos {
                    let gone = match mode {
                        MapMode::Simple => false,
                        MapMode::TrackDel => self.old < pos && end > pos,
                        MapMode::TrackBefore => self.old < pos,
                        MapMode::TrackAfter => end > pos,
                    };
                    if gone {
                        return None;
                    }
                }
                if end > pos || (end == pos && del == 0 && bias == Bias::Before) {
                    return Some(if pos == self.old || bias == Bias::Before {
                        self.new
                    } else {
                        self.new + ins
                    });
                }
                if end == pos {
                    // At the end: after the new text. The section is not passed, so another
                    // query at this position still sees it (a tracking mode, or `Before` at a
                    // pure insertion).
                    return Some(self.new + ins);
                }
            }
            self.old = end;
            self.new += ins;
            self.i += 1;
        }
        Some(self.new)
    }
}

impl ChangeDesc {
    pub fn len_old(&self) -> usize {
        self.len_old
    }

    pub fn len_new(&self) -> usize {
        self.len_new
    }

    pub(crate) fn mapper(&self) -> Mapper<'_> {
        Mapper {
            parts: &self.parts,
            i: 0,
            old: 0,
            new: 0,
            len_old: self.len_old,
        }
    }

    /// Where `pos` (in the old text) lands in the new one, or `None` when `mode` tracks a
    /// deletion that took it (the module docs). O(sections).
    pub fn map_pos(&self, pos: usize, bias: Bias, mode: MapMode) -> Option<usize> {
        self.mapper().map(pos, bias, mode)
    }

    /// Where `r` lands, by `kind`'s rules (the module docs); `None` when the change dropped it.
    /// An empty content range keeps to the text before it. Ends past the text clamp to it.
    pub fn map_range(&self, r: Range<usize>, kind: RangeKind) -> Option<Range<usize>> {
        let start = r.start.min(self.len_old);
        let end = r.end.clamp(start, self.len_old);
        match kind {
            RangeKind::Content => {
                if self.touches_inside(start, end) {
                    return None;
                }
                let mut m = self.mapper();
                let (s_bias, e_bias) = if start == end {
                    (Bias::Before, Bias::Before)
                } else {
                    (Bias::After, Bias::Before)
                };
                let s = m.map(start, s_bias, MapMode::Simple)?;
                let e = m.map(end, e_bias, MapMode::Simple)?;
                Some(s..e)
            }
            RangeKind::Container => {
                if start < end && self.deletes_all_of(start, end) {
                    return None;
                }
                let mut m = self.mapper();
                let s = m.map(start, Bias::Before, MapMode::Simple)?;
                let e = m.map(end, Bias::After, MapMode::Simple)?;
                Some(s..e.max(s))
            }
        }
    }

    /// The bytes of the new text the change touched: one range from its first edit to the end
    /// of its last (a pure deletion is an empty range where it was). `None` for the identity.
    pub fn changed_range(&self) -> Option<Range<usize>> {
        let mut new = 0;
        let mut first = None;
        let mut last = 0;
        for &(_, ins, replaced) in &self.parts {
            if replaced {
                first.get_or_insert(new);
                last = new + ins;
            }
            new += ins;
        }
        first.map(|f| f..last)
    }

    /// A deletion overlaps `[start, end)`, or something is inserted strictly inside it; for an
    /// empty range, a deletion spans it.
    fn touches_inside(&self, start: usize, end: usize) -> bool {
        let mut old = 0;
        for &(del, _, replaced) in &self.parts {
            if old > end {
                break;
            }
            if replaced {
                let hit = if del == 0 {
                    start < old && old < end
                } else if start == end {
                    old < start && start < old + del
                } else {
                    old < end && old + del > start
                };
                if hit {
                    return true;
                }
            }
            old += del;
        }
        false
    }

    /// One replacement deleted every byte of `[start, end)` (non-empty).
    fn deletes_all_of(&self, start: usize, end: usize) -> bool {
        let mut old = 0;
        for &(del, _, replaced) in &self.parts {
            if old > start {
                break;
            }
            if replaced && old <= start && end <= old + del {
                return true;
            }
            old += del;
        }
        false
    }
}
