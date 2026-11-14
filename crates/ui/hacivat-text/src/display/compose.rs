//! Line composition (UI-EDIT-07 pure part; Task 11): text, tabs expanded by `unicode-width`,
//! and `Control` fragments for C0/C1, U+FEFF and every bidi control, always visible.

use smallvec::SmallVec;
use unicode_width::UnicodeWidthChar;

use crate::buffer::TextBuffer;
use crate::display::{Decoration, PlaceholderId, RangeSet};

/// One piece of a visual line.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Fragment {
    Text {
        bytes: std::ops::Range<usize>,
    },
    Tab {
        width: u8,
    },
    /// A control character shown as `glyph` (`␀` for NUL; U+FFFD for the ones without a
    /// Control Pictures symbol); `control_name` names it for a label or tooltip.
    Control {
        byte: usize,
        glyph: char,
    },
    Placeholder {
        id: PlaceholderId,
    },
}

/// One painted row: the buffer lines it shows (more than one once folds exist) and its
/// fragments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VisualLine {
    pub first_line: u32,
    pub last_line: u32,
    pub fragments: SmallVec<[Fragment; 8]>,
}

impl VisualLine {
    /// The width in cells of each fragment (wide characters 2, zero-width 0, a tab its
    /// expansion, a control or placeholder 1).
    pub fn cell_widths(&self, buf: &TextBuffer) -> Vec<u32> {
        self.fragments
            .iter()
            .map(|f| match f {
                Fragment::Text { bytes } => buf.slice(bytes.clone()).chars().map(char_cells).sum(),
                Fragment::Tab { width } => u32::from(*width),
                Fragment::Control { .. } | Fragment::Placeholder { .. } => 1,
            })
            .collect()
    }
}

fn char_cells(c: char) -> u32 {
    UnicodeWidthChar::width(c).unwrap_or(0) as u32
}

/// Whether `c` is drawn as a `Control` fragment: C0 (but the tab), DEL, C1, U+FEFF and every
/// bidi control (U+202A-E, U+2066-9, U+200E/F, U+061C), so source can never hide behind them.
pub fn is_visible_control(c: char) -> bool {
    matches!(c,
        '\u{0}'..='\u{8}' | '\u{A}'..='\u{1F}' | '\u{7F}'..='\u{9F}'
        | '\u{FEFF}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        | '\u{200E}' | '\u{200F}' | '\u{061C}')
}

/// The glyph a control is drawn as.
pub fn control_glyph(c: char) -> char {
    match c {
        '\u{0}'..='\u{1F}' => char::from_u32(0x2400 + c as u32).unwrap_or('\u{FFFD}'),
        '\u{7F}' => '\u{2421}',
        _ => '\u{FFFD}',
    }
}

/// A short name for a control (`RLO`, `NUL`, `U+0085`, ...), for the label next to its glyph.
pub fn control_name(c: char) -> String {
    let n = match c {
        '\u{0}' => "NUL",
        '\u{7F}' => "DEL",
        '\u{FEFF}' => "BOM",
        '\u{200E}' => "LRM",
        '\u{200F}' => "RLM",
        '\u{061C}' => "ALM",
        '\u{202A}' => "LRE",
        '\u{202B}' => "RLE",
        '\u{202C}' => "PDF",
        '\u{202D}' => "LRO",
        '\u{202E}' => "RLO",
        '\u{2066}' => "LRI",
        '\u{2067}' => "RLI",
        '\u{2068}' => "FSI",
        '\u{2069}' => "PDI",
        _ => return format!("U+{:04X}", c as u32),
    };
    n.to_string()
}

/// Composes buffer line `line`: text runs, tabs expanded to the next stop (`tab_size` cells,
/// columns by `unicode-width`), visible `Control` fragments, and the `Replace` decorations of
/// `decos` as placeholders. Reads only that line.
pub fn compose_line(
    buf: &TextBuffer,
    line: u32,
    tab_size: u8,
    decos: &[&RangeSet<Decoration>],
) -> VisualLine {
    let tab = u32::from(tab_size.max(1));
    let range = buf.line_range(line as usize, false);
    let mut replaces: Vec<(std::ops::Range<usize>, PlaceholderId)> = Vec::new();
    for set in decos {
        for (r, d) in set.query(range.clone()) {
            if let Decoration::Replace { placeholder } = d {
                let s = r.start.max(range.start);
                replaces.push((s..r.end.min(range.end).max(s), *placeholder));
            }
        }
    }
    replaces.sort_by_key(|(r, _)| r.start);
    let mut next_replace = 0;

    let text = buf.slice(range.clone());
    let mut fragments: SmallVec<[Fragment; 8]> = SmallVec::new();
    let mut run_start = range.start;
    let mut skip_until = range.start;
    let mut col: u32 = 0;
    let flush = |frags: &mut SmallVec<[Fragment; 8]>, start: usize, end: usize| {
        if end > start {
            frags.push(Fragment::Text { bytes: start..end });
        }
    };
    for (i, c) in text.char_indices() {
        let at = range.start + i;
        if at < skip_until {
            run_start = skip_until;
            continue;
        }
        while next_replace < replaces.len()
            && replaces[next_replace].0.end <= at
            && replaces[next_replace].0.start < at
        {
            next_replace += 1;
        }
        if let Some((r, id)) = replaces.get(next_replace)
            && r.start <= at
        {
            flush(&mut fragments, run_start, at);
            fragments.push(Fragment::Placeholder { id: *id });
            col += 1;
            skip_until = r.end.max(at + c.len_utf8());
            run_start = skip_until;
            next_replace += 1;
            continue;
        }
        if c == '\t' {
            flush(&mut fragments, run_start, at);
            let width = tab - col % tab;
            fragments.push(Fragment::Tab { width: width as u8 });
            col += width;
            run_start = at + 1;
        } else if is_visible_control(c) {
            flush(&mut fragments, run_start, at);
            fragments.push(Fragment::Control {
                byte: at,
                glyph: control_glyph(c),
            });
            col += 1;
            run_start = at + c.len_utf8();
        } else {
            col += char_cells(c);
        }
    }
    flush(&mut fragments, run_start, range.end);
    VisualLine {
        first_line: line,
        last_line: line,
        fragments,
    }
}
