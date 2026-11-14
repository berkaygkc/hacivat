//! Change sets and their mapping (UI-EDIT-03; Task 5). Offsets are bytes.
//!
//! Working since Task 1b: `from_edits`, `desc`, applying a set (through the model) and
//! `ChangeDesc::map_pos`/`map_range` with plain (`Simple`) mapping, every mode treated alike.
//! `invert` and `compose`, and the deletion-tracking modes, arrive in Task 5.

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

/// An edit of a whole text: sections covering the old text in order.
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
    /// `None` when the position's surroundings were deleted.
    TrackDel,
    TrackBefore,
    TrackAfter,
}

/// How a range maps: content ranges shrink with deletions at their edges (`(After, Before)`),
/// container ranges grow with insertions at their edges (`(Before, After)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeKind {
    Content,
    Container,
}

/// Two edits of `from_edits` overlap, or one lies outside the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverlapError;

impl ChangeSet {
    /// A set from byte-range edits of a text of `len_old` bytes, in any order; overlapping
    /// edits or an edit past the end are an error.
    pub fn from_edits(
        len_old: usize,
        edits: impl IntoIterator<Item = (Range<usize>, Arc<str>)>,
    ) -> Result<Self, OverlapError> {
        let mut edits: Vec<(Range<usize>, Arc<str>)> = edits.into_iter().collect();
        edits.sort_by_key(|(r, _)| (r.start, r.end));
        let mut sections = Vec::with_capacity(edits.len() * 2 + 1);
        let mut at = 0;
        let mut len_new = 0;
        for (r, ins) in edits {
            if r.start < at || r.end < r.start || r.end > len_old {
                return Err(OverlapError);
            }
            if r.start > at {
                sections.push(Section::Retain(r.start - at));
                len_new += r.start - at;
            }
            len_new += ins.len();
            sections.push(Section::Replace {
                del: r.end - r.start,
                ins,
            });
            at = r.end;
        }
        if at < len_old {
            sections.push(Section::Retain(len_old - at));
            len_new += len_old - at;
        }
        Ok(ChangeSet {
            sections,
            len_old,
            len_new,
        })
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
        self.sections.iter().all(|s| match s {
            Section::Retain(_) => true,
            Section::Replace { del, ins } => *del == 0 && ins.is_empty(),
        })
    }

    /// The set that undoes this one on `old`, the text it applies to (Task 5).
    pub fn invert(&self, old: &TextBuffer) -> ChangeSet {
        let _ = old;
        unimplemented!("ChangeSet::invert: Task 5")
    }

    /// This set, then `next` (Task 5).
    pub fn compose(&self, next: &ChangeSet) -> ChangeSet {
        let _ = next;
        unimplemented!("ChangeSet::compose: Task 5")
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

    /// The replaced ranges of the old text and their insertions, last first (so applying them
    /// in this order keeps the earlier offsets valid).
    pub(crate) fn replacements_from_end(&self) -> Vec<(Range<usize>, Arc<str>)> {
        let mut at = 0;
        let mut out = Vec::new();
        for s in &self.sections {
            match s {
                Section::Retain(n) => at += n,
                Section::Replace { del, ins } => {
                    out.push((at..at + del, ins.clone()));
                    at += del;
                }
            }
        }
        out.reverse();
        out
    }
}

impl ChangeDesc {
    pub fn len_old(&self) -> usize {
        self.len_old
    }

    pub fn len_new(&self) -> usize {
        self.len_new
    }

    /// Where `pos` (in the old text) lands in the new one. Task 1b: plain mapping for every
    /// mode (a position inside a replaced range goes to its start, `Before`, or past its
    /// insertion, `After`); Task 5 adds the tracking modes' `None`.
    pub fn map_pos(&self, pos: usize, bias: Bias, mode: MapMode) -> Option<usize> {
        let _ = mode;
        let pos = pos.min(self.len_old);
        let (mut old, mut new) = (0, 0);
        for &(del, ins, replaced) in &self.parts {
            if !replaced {
                if pos < old + del {
                    return Some(new + pos - old);
                }
            } else if pos < old + del || (pos == old && del == 0) {
                if pos == old && bias == Bias::Before {
                    return Some(new);
                }
                if del > 0 {
                    return Some(match bias {
                        Bias::Before => new,
                        Bias::After => new + ins,
                    });
                }
            }
            old += del;
            new += ins;
        }
        Some(new + (pos - old))
    }

    /// Where `r` lands: both ends mapped with `kind`'s biases.
    pub fn map_range(&self, r: Range<usize>, kind: RangeKind) -> Option<Range<usize>> {
        let (start_bias, end_bias) = match kind {
            RangeKind::Content => (Bias::After, Bias::Before),
            RangeKind::Container => (Bias::Before, Bias::After),
        };
        let start = self.map_pos(r.start, start_bias, MapMode::Simple)?;
        let end = self.map_pos(r.end, end_bias, MapMode::Simple)?;
        Some(start.min(end)..end.max(start))
    }
}
