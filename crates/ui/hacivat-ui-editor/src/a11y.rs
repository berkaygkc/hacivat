//! The editor's accessibility baseline (UI-EDIT-24 subset, D-073; Task 29): role
//! `MultilineTextInput` with its label, a ±100-line window of `TextRun` children split at 255
//! characters with stable node ids, the text selection, and polite announcements, built only
//! while assistive technology is active.
//!
//! **Which node.** The editor's accessibility node is its focusable column (the view's root
//! `div`, which tracks the focus), declared by [`declare`] from the view's `render`. GPUI moves
//! the platform focus to the node of the element that tracks the focus (`div.rs`'s
//! `set_focusable`/`set_focus`); a node on the inner text element (Task 1b's element hooks)
//! would never be the focused node, so assistive technology would read the whole window
//! instead of the text (Task 29, named exception in `view.rs` and `element.rs`).
//!
//! **Nothing without assistive technology.** [`declare`] returns the column untouched unless
//! `window.is_a11y_active()`: no label is evaluated, no window is built, no line is anchored,
//! and an announcement made meanwhile (or the last live region) is dropped at the next frame
//! rather than read out later.
//!
//! **The window** ([`window`], pure): the lines within `a11y_window_lines` (D-073: 100) of the
//! main caret's line, or the whole document up to [`OPTIMIZED_MAX_BYTES`] when the
//! screen-reader-optimised setting is on and the document is in T0 (T1 never grows it). Each
//! line is one or more runs of at most [`RUN_MAX_CHARS`] characters (AccessKit stores their
//! lengths as `u8`); a character is a grapheme cluster (CRLF is one), cut at char boundaries
//! if it is longer than 255 bytes; a line's break ends its last run; a line longer than
//! [`LINE_MAX_BYTES`] exposes only that many bytes, around the caret when the caret is on it
//! (the end of the text is on its last line); the default window exposes at most
//! [`WINDOW_MAX_BYTES`] in all, the lines nearest the caret first.
//!
//! **Stable ids.** Each window line gets a key that follows the line: the start of every line
//! of the last window is anchored (`Bias::After`, `MapMode::TrackDel`) and rebased through the
//! change log; a line takes the key of the first anchor inside it, so a line keeps its key when
//! lines are inserted above it, when text is typed at its start, or when the line below is
//! joined to it. Keys are never reused. The runs' node ids are `synthetic_node_id((key, run))`.
//! The window and its AccessKit run nodes are rebuilt only when the text version or the
//! window's lines change.
//!
//! **Announcements** ([`announce`], `EditorView::announce`): polite, in a `Status` leaf whose
//! node is new for each batch (so the same sentence twice is read twice), carries the text as
//! its name and its value (Windows and Linux read the name, macOS the value), and leaves the
//! tree after [`LIVE_HOLD`] so the subtree re-entering the tree later (a tab shown again) does
//! not read it again. [`exec_announcement`] words an execution status.
//!
//! **Cached embedding.** GPUI's `reuse_prepaint` carries no accessibility state, so a cached
//! editor reused on a frame would drop out of the tree; [`embed`] caches the editor only while
//! no assistive technology is attached (Task 32 embeds the console's editor with it).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ops::Range;
use std::rc::Rc;
// ## m3c-t32-fold (named exception)
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_text::units::snap_to_grapheme;
use hacivat_text::{AnchorSet, Bias, ChangeLog, MapMode, Snapshot, TextBuffer};
use hacivat_ui::l10n::UiArg;
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{
    A11ySubtreeBuilder, AnyElement, AnyView, App, Div, Entity, IntoElement, Role, SharedString,
    Stateful, StatefulInteractiveElement, StyleRefinement, Window, accesskit,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::commands::message;
use crate::config::{EditorConfig, Tier};
use crate::large_file;
use crate::marks::{ExecMarkState, RowsOutcome};
use crate::view::EditorView;

/// How long a live region stays in the tree once declared: long enough for every screen
/// reader to read it, short enough that the editor's subtree re-entering the tree later (a tab
/// shown again) does not read it again.
pub const LIVE_HOLD: Duration = Duration::from_secs(3);
/// The most announcements kept for the next frame; older ones are dropped.
pub const MAX_PENDING_ANNOUNCEMENTS: usize = 4;
/// The most characters in one text run (AccessKit's lengths are `u8`; UI-EDIT-24 step 2).
pub const RUN_MAX_CHARS: usize = 255;
/// The most bytes in one character (`character_lengths` holds `u8`s).
const CHAR_MAX_BYTES: usize = u8::MAX as usize;
/// The largest document the screen-reader-optimised mode exposes whole (D-073: 2 MB).
pub const OPTIMIZED_MAX_BYTES: usize = 2 << 20;
/// The most bytes of one line in the window: a longer line (a minified dump, a 15 MB single
/// line in T1) exposes this many, around the caret when the caret is on it.
pub const LINE_MAX_BYTES: usize = 64 << 10;
/// The most bytes the default window exposes in all (±100 lines of long lines would be
/// megabytes per frame); the lines nearest the caret come first.
pub const WINDOW_MAX_BYTES: usize = 256 << 10;

/// The module-owned state (P11): a shared handle, so the closure GPUI calls after the column's
/// prepaint can reach it.
#[derive(Debug, Clone, Default)]
pub struct A11yState {
    inner: Rc<RefCell<A11yNodes>>,
}

/// What the accessibility declaration keeps between frames.
#[derive(Debug, Default)]
pub struct A11yNodes {
    /// Polite announcements waiting for the next frame's live region, oldest first.
    pub pending: VecDeque<SharedString>,
    /// The start of every line of the last window, anchored to the line's key.
    anchors: AnchorSet<u64>,
    /// The text version `anchors` are at.
    anchored_at: u64,
    /// The last line key handed out (keys start at 1 and are never reused).
    last_key: u64,
    /// The last window (with its run nodes): its text version, its lines, the exposed start of
    /// a long caret line.
    window: Option<(WindowKey, Built)>,
    /// The live region, until it has been held for [`LIVE_HOLD`].
    live: Option<LiveRegion>,
    last_live: u64,
    windows_built: u64,
    nodes_built: u64,
}

/// The live region: a new node per batch of announcements.
#[derive(Debug, Clone)]
struct LiveRegion {
    seq: u64,
    text: SharedString,
    /// When it was first declared (the executor clock).
    since: Instant,
}

/// What a built window is reused for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowKey {
    version: u64,
    lines: Range<usize>,
    long_from: Option<usize>,
    // ## m3c-t32-fold (named exception)
    hidden: Arc<[Range<u32>]>,
}

impl A11yState {
    /// The shared nodes.
    pub fn nodes(&self) -> std::cell::RefMut<'_, A11yNodes> {
        self.inner.borrow_mut()
    }
}

/// How much accessibility work an editor has done (tests: none without assistive technology).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct A11yStats {
    /// Windows built (a frame at the same text version and lines reuses the last one).
    pub windows_built: u64,
    /// Lines anchored for their keys.
    pub keyed_lines: usize,
    /// Times the window's AccessKit run nodes were built.
    pub nodes_built: u64,
}

/// The editor's accessibility work so far.
pub fn stats(a: &A11yState) -> A11yStats {
    let n = a.nodes();
    A11yStats {
        windows_built: n.windows_built,
        keyed_lines: n.anchors.len(),
        nodes_built: n.nodes_built,
    }
}

/// The editor's state (for the host's checks and tests).
pub fn state(v: &EditorView) -> &A11yState {
    &v.a11y
}

/// What one frame declares, built by `EditorView::a11y_frame` only while assistive technology
/// is active.
#[derive(Debug, Clone)]
pub struct A11yFrame {
    pub snapshot: Snapshot,
    /// The editor's accessible label, evaluated when the UI language changes, not per frame.
    pub label: SharedString,
    /// `a11y_window_lines` (D-073: 100), `a11y_optimized`, `read_only`.
    pub config: EditorConfig,
    /// T1 ("Reduced mode") disables the optimised window growth.
    pub tier: Tier,
    pub focused: bool,
    // ## m3c-t32-fold (named exception)
    /// The lines folds hide (sorted, merged): they leave the window's runs.
    pub hidden: Arc<[Range<u32>]>,
}

/// One text run: at most [`RUN_MAX_CHARS`] characters of one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// Its bytes in the text (a line's last run ends with the line's break).
    pub bytes: Range<usize>,
    /// The UTF-8 length of each character (a grapheme cluster; CRLF is one), AccessKit's
    /// `character_lengths`.
    pub char_lens: Vec<u8>,
}

/// One line of the window and its runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunNode {
    /// 0-based.
    pub line: usize,
    /// The line's bytes, with its break.
    pub bytes: Range<usize>,
    /// At least one (an empty last line has one run with no characters).
    pub runs: Vec<Run>,
    /// The line's stable key ([`keyed_window`]); 0 from [`window`].
    pub key: u64,
}

/// The lines within `lines` of `caret_line`, clamped to the text.
fn lines_around(buf: &TextBuffer, caret_line: usize, lines: u32) -> Range<usize> {
    let count = buf.len_lines().max(1);
    let caret = caret_line.min(count - 1);
    let n = usize::try_from(lines).unwrap_or(usize::MAX);
    caret.saturating_sub(n)..caret.saturating_add(n).saturating_add(1).min(count)
}

/// The window of lines within `lines` of `caret_line` (D-073: `lines` = 100), each split into
/// runs (pure; the keys are 0).
pub fn window(buf: &TextBuffer, caret_line: usize, lines: u32) -> Vec<RunNode> {
    let range = capped(
        buf,
        lines_around(buf, caret_line, lines),
        caret_line,
        WINDOW_MAX_BYTES,
    );
    let caret = buf.line_to_byte(caret_line);
    build(buf, range, caret)
}

/// `lines` narrowed to at most `budget` exposed bytes, growing from `caret_line` one line
/// below, then one above, while they fit; the caret's line always stays (`usize::MAX`: no cap,
/// the optimised mode, itself bounded by its 2 MB document).
fn capped(buf: &TextBuffer, lines: Range<usize>, caret_line: usize, budget: usize) -> Range<usize> {
    if budget == usize::MAX {
        return lines;
    }
    let cost = |l: usize| buf.line_range(l, true).len().min(LINE_MAX_BYTES);
    let caret = caret_line.clamp(lines.start, lines.end.saturating_sub(1).max(lines.start));
    let (mut lo, mut hi) = (caret, caret + 1);
    let mut used = cost(caret);
    let (mut down, mut up) = (true, true);
    while down || up {
        if down {
            if hi < lines.end && used + cost(hi) <= budget {
                used += cost(hi);
                hi += 1;
            } else {
                down = false;
            }
        }
        if up {
            if lo > lines.start && used + cost(lo - 1) <= budget {
                used += cost(lo - 1);
                lo -= 1;
            } else {
                up = false;
            }
        }
    }
    lo..hi
}

fn build(buf: &TextBuffer, lines: Range<usize>, caret: usize) -> Vec<RunNode> {
    lines.map(|line| line_node(buf, line, caret)).collect()
}

/// The bytes of a line that the window exposes: all of it, or [`LINE_MAX_BYTES`] of a longer
/// one (around `caret` when it is on the line), on grapheme boundaries.
fn exposed(buf: &TextBuffer, bytes: &Range<usize>, caret: usize) -> Range<usize> {
    if bytes.len() <= LINE_MAX_BYTES {
        return bytes.clone();
    }
    // ## m3a-t29 fix round 1 (I3): the end of the text is a position on its last line.
    let on_line = bytes.contains(&caret) || (caret == bytes.end && bytes.end == buf.len_bytes());
    let start = if on_line {
        caret
            .saturating_sub(LINE_MAX_BYTES / 2)
            .clamp(bytes.start, bytes.end - LINE_MAX_BYTES)
    } else {
        bytes.start
    };
    let from = snap_to_grapheme(buf, start, Bias::Before).max(bytes.start);
    let to = snap_to_grapheme(buf, from + LINE_MAX_BYTES, Bias::Before).min(bytes.end);
    from..to
}

fn line_node(buf: &TextBuffer, line: usize, caret: usize) -> RunNode {
    let bytes = buf.line_range(line, true);
    let shown = exposed(buf, &bytes, caret);
    let text = buf.slice(shown.clone());
    let mut runs = Vec::with_capacity(shown.len() / RUN_MAX_CHARS + 1);
    let mut run = Run {
        bytes: shown.start..shown.start,
        char_lens: Vec::new(),
    };
    for grapheme in text.graphemes(true) {
        let mut rest = grapheme;
        while !rest.is_empty() {
            let mut cut = rest.len().min(CHAR_MAX_BYTES);
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            if run.char_lens.len() == RUN_MAX_CHARS {
                let end = run.bytes.end;
                runs.push(std::mem::replace(
                    &mut run,
                    Run {
                        bytes: end..end,
                        char_lens: Vec::new(),
                    },
                ));
            }
            run.char_lens.push(u8::try_from(cut).unwrap_or(u8::MAX));
            run.bytes.end += cut;
            rest = &rest[cut..];
        }
    }
    runs.push(run);
    RunNode {
        line,
        bytes,
        runs,
        key: 0,
    }
}

impl A11yNodes {
    /// Moves the anchors to `version` through the change log; they start over when the log no
    /// longer reaches back to them.
    fn rebase(&mut self, log: &ChangeLog, version: u64) {
        if self.anchored_at == version {
            return;
        }
        match log.descs_since(self.anchored_at) {
            Some(descs) if log.version() == version => {
                for d in descs {
                    self.anchors.map(d);
                }
            }
            _ => self.anchors = AnchorSet::default(),
        }
        self.anchored_at = version;
    }

    /// Gives each line the key of the first anchor inside it (a new key when there is none)
    /// and anchors the window's lines for the next rebuild.
    fn assign_keys(&mut self, nodes: &mut [RunNode], len: usize) {
        let old = std::mem::take(&mut self.anchors);
        let mut old = old.iter().peekable();
        let mut fresh = AnchorSet::default();
        for n in nodes.iter_mut() {
            while old.peek().is_some_and(|(at, _)| *at < n.bytes.start) {
                old.next();
            }
            // The text's last line also holds the position at its very end.
            let inside = |at: usize| at < n.bytes.end || (n.bytes.end == len && at == len);
            n.key = match old.peek() {
                Some(&(at, &key)) if inside(at) => {
                    old.next();
                    key
                }
                _ => {
                    self.last_key += 1;
                    self.last_key
                }
            };
            fresh.insert(n.bytes.start, Bias::After, MapMode::TrackDel, n.key);
        }
        self.anchors = fresh;
    }
}

/// The window within `lines` of `caret_line` with each line's stable key, rebased from the
/// last window through `log` (`snap.version` must be the log's version). A frame at the same
/// text version and lines reuses the last window.
pub fn keyed_window(
    a: &A11yState,
    snap: &Snapshot,
    log: &ChangeLog,
    caret_line: usize,
    lines: u32,
) -> Rc<[RunNode]> {
    let range = capped(
        &snap.buffer,
        lines_around(&snap.buffer, caret_line, lines),
        caret_line,
        WINDOW_MAX_BYTES,
    );
    keyed(
        a,
        snap,
        log,
        range,
        snap.buffer.line_to_byte(caret_line),
        &Arc::from([]),
    )
    .lines
}

/// A built window and its AccessKit run nodes (built together, reused together).
#[derive(Debug, Clone)]
struct Built {
    lines: Rc<[RunNode]>,
    nodes: Rc<[(ChildKey, accesskit::Node)]>,
}

fn keyed(
    a: &A11yState,
    snap: &Snapshot,
    log: &ChangeLog,
    lines: Range<usize>,
    caret: usize,
    hidden: &Arc<[Range<u32>]>,
) -> Built {
    let buf = &snap.buffer;
    let caret_line = buf.line_range(buf.byte_to_line(caret), true);
    let long_from =
        (caret_line.len() > LINE_MAX_BYTES).then(|| exposed(buf, &caret_line, caret).start);
    let key = WindowKey {
        version: snap.version,
        lines,
        long_from,
        hidden: hidden.clone(),
    };
    let mut n = a.nodes();
    if let Some((at, w)) = &n.window
        && *at == key
    {
        return w.clone();
    }
    n.rebase(log, snap.version);
    let mut lines = build(buf, key.lines.clone(), caret);
    // ## m3c-t32-fold (named exception): folded lines leave the runs (a screen reader reads
    // the header, as the view shows it).
    if !hidden.is_empty() {
        lines.retain(|n| {
            let l = u32::try_from(n.line).unwrap_or(u32::MAX);
            let i = hidden.partition_point(|r| r.end <= l);
            !hidden.get(i).is_some_and(|r| r.start <= l)
        });
    }
    n.assign_keys(&mut lines, buf.len_bytes());
    // ## m3a-t29 fix round 1 (M2): the run nodes are built with the window, not per frame.
    let mut nodes = Vec::with_capacity(lines.iter().map(|l| l.runs.len()).sum());
    for l in &lines {
        for (i, run) in l.runs.iter().enumerate() {
            let key = ChildKey::Run {
                line: l.key,
                run: u32::try_from(i).unwrap_or(u32::MAX),
            };
            nodes.push((key, run_node(buf, run)));
        }
    }
    let built = Built {
        lines: lines.into(),
        nodes: nodes.into(),
    };
    n.window = Some((key, built.clone()));
    n.windows_built += 1;
    n.nodes_built += 1;
    built
}

/// A child node's key, hashed into its id by `A11ySubtreeBuilder::synthetic_node_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChildKey {
    /// Run `run` of the line keyed `line`.
    Run { line: u64, run: u32 },
    /// The live region of announcement batch `n`.
    Live(u64),
}

/// A text position: a run's key and a character index in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pos {
    pub key: ChildKey,
    pub index: usize,
}

/// What the editor's node declares this frame, with children keyed rather than identified (the
/// ids come from the subtree builder).
#[derive(Debug, Clone)]
pub struct Declaration {
    pub role: Role,
    pub label: SharedString,
    /// The T1 banner's text (NOTES §7: the banner is not focusable; its text is here).
    pub description: Option<SharedString>,
    pub read_only: bool,
    /// The window's runs in order, then the live region.
    pub children: Vec<(ChildKey, accesskit::Node)>,
    /// The main selection's anchor and focus, clamped to the window.
    pub selection: Option<(Pos, Pos)>,
}

impl Declaration {
    /// The editor node's own properties: label, description, read-only.
    pub fn write_parent(&self, node: &mut accesskit::Node) {
        node.set_label(self.label.to_string());
        if let Some(d) = &self.description {
            node.set_description(d.to_string());
        }
        if self.read_only {
            node.set_read_only();
        }
    }

    /// Writes the declaration under the editor's node (`a11y_synthetic_children`).
    pub fn write(self, b: &mut A11ySubtreeBuilder<'_>) {
        let selection = self.selection.map(|(anchor, focus)| {
            let at = |p: Pos| accesskit::TextPosition {
                node: b.synthetic_node_id(p.key),
                character_index: p.index,
            };
            accesskit::TextSelection {
                anchor: at(anchor),
                focus: at(focus),
            }
        });
        let parent = b.parent_node();
        self.write_parent(parent);
        if let Some(s) = selection {
            parent.set_text_selection(s);
        }
        for (key, node) in self.children {
            let id = b.synthetic_node_id(key);
            b.push_child(id, node);
        }
    }
}

/// How many lines around the caret the window holds: all of them in the optimised mode, for a
/// T0 document up to 2 MB (D-073); T1 never grows it.
fn reach(frame: &A11yFrame) -> u32 {
    let grow = frame.config.a11y_optimized
        && frame.tier == Tier::Full
        && frame.snapshot.buffer.len_bytes() <= OPTIMIZED_MAX_BYTES;
    if grow {
        u32::MAX
    } else {
        frame.config.a11y_window_lines
    }
}

/// Where `at` falls in the window, clamped to it.
fn position(w: &[RunNode], at: usize) -> Pos {
    let i = w.partition_point(|n| n.bytes.start <= at).saturating_sub(1);
    let n = &w[i];
    let r = n
        .runs
        .iter()
        .position(|r| at < r.bytes.end)
        .unwrap_or(n.runs.len() - 1);
    let run = &n.runs[r];
    let mut index = 0;
    let mut end = run.bytes.start;
    for &len in &run.char_lens {
        end += usize::from(len);
        if end > at {
            break;
        }
        index += 1;
    }
    Pos {
        key: ChildKey::Run {
            line: n.key,
            run: u32::try_from(r).unwrap_or(u32::MAX),
        },
        index,
    }
}

fn run_node(buf: &TextBuffer, run: &Run) -> accesskit::Node {
    let mut node = accesskit::Node::new(Role::TextRun);
    node.set_value(buf.slice(run.bytes.clone()).into_owned());
    node.set_character_lengths(run.char_lens.clone());
    node
}

/// What the editor declares for `frame` (pure apart from `a`'s keys, window and live region):
/// role `MultilineTextInput`, the label, `description`, read-only, the window's runs, the
/// main selection and the live region (pending announcements become its next batch).
pub fn declaration(
    a: &A11yState,
    frame: &A11yFrame,
    log: &ChangeLog,
    description: Option<SharedString>,
    now: Instant,
) -> Declaration {
    let snap = &frame.snapshot;
    let buf = &snap.buffer;
    let main = snap.selection.main();
    let caret_line = buf.byte_to_line(main.head);
    let (around, budget) = match reach(frame) {
        u32::MAX => (u32::MAX, usize::MAX),
        lines => (lines, WINDOW_MAX_BYTES),
    };
    let lines = capped(
        buf,
        lines_around(buf, caret_line, around),
        caret_line,
        budget,
    );
    let built = keyed(a, snap, log, lines, main.head, &frame.hidden);
    let w = &built.lines;
    let mut children: Vec<(ChildKey, accesskit::Node)> = Vec::with_capacity(built.nodes.len() + 1);
    children.extend(built.nodes.iter().cloned());
    let selection = (!w.is_empty()).then(|| (position(w, main.anchor), position(w, main.head)));
    let mut nodes = a.nodes();
    // ## m3a-t29 fix round 1 (I2): a live region leaves the tree once held for `LIVE_HOLD`,
    // so the subtree re-entering the tree (a tab shown again) does not read it again.
    if nodes
        .live
        .as_ref()
        .is_some_and(|l| now >= l.since + LIVE_HOLD)
    {
        nodes.live = None;
    }
    if !nodes.pending.is_empty() {
        let text: Vec<&str> = nodes.pending.iter().map(SharedString::as_ref).collect();
        let text = SharedString::from(text.join("\n"));
        nodes.pending.clear();
        nodes.last_live += 1;
        nodes.live = Some(LiveRegion {
            seq: nodes.last_live,
            text,
            since: now,
        });
    }
    if let Some(l) = &nodes.live {
        let mut node = accesskit::Node::new(Role::Status);
        node.set_live(accesskit::Live::Polite);
        // ## m3a-t29 fix round 1 (I1): the Windows and Linux adapters announce a live region
        // by its name, macOS by its value.
        node.set_label(l.text.to_string());
        node.set_value(l.text.to_string());
        children.push((ChildKey::Live(l.seq), node));
    }
    Declaration {
        role: Role::MultilineTextInput,
        label: frame.label.clone(),
        description,
        read_only: frame.config.read_only,
        children,
        selection,
    }
}

/// The editor's declaration this frame (`focused`: the column has the focus).
pub fn declaration_of(v: &mut EditorView, focused: bool, cx: &App) -> Declaration {
    let frame = v.a11y_frame(focused, cx);
    let description = large_file::description(&v.tier, cx);
    let now = cx.background_executor().now();
    declaration(&v.a11y, &frame, v.model.changelog(), description, now)
}

/// Makes `el`, the editor's focusable column, its accessibility node while assistive
/// technology is active (the view's `render`); otherwise returns it untouched and drops the
/// announcements nobody could hear.
pub fn declare(el: Stateful<Div>, v: &mut EditorView, window: &Window, cx: &App) -> Stateful<Div> {
    if !window.is_a11y_active() {
        let mut nodes = v.a11y.nodes();
        if !nodes.pending.is_empty() {
            nodes.pending.clear();
        }
        // ## m3a-t29 fix round 1 (I2): nor is the last one read again when assistive
        // technology attaches later.
        nodes.live = None;
        return el;
    }
    let focused = v.focus.is_focused(window);
    let d = declaration_of(v, focused, cx);
    el.role(d.role).a11y_synthetic_children(move |b| d.write(b))
}

/// The editor as a host's child: cached (`AnyView::cached(style)`) while no assistive
/// technology is attached, re-rendered with its parent while one is, so its node is declared
/// on every frame (GPUI's cached-view reuse carries no accessibility state).
pub fn embed(editor: &Entity<EditorView>, style: StyleRefinement, window: &Window) -> AnyElement {
    embed_with(editor, style, window.is_a11y_active())
}

/// [`embed`] with the assistive-technology state given.
pub fn embed_with(
    editor: &Entity<EditorView>,
    style: StyleRefinement,
    a11y_active: bool,
) -> AnyElement {
    if a11y_active {
        editor.clone().into_any_element()
    } else {
        AnyView::from(editor.clone())
            .cached(style)
            .into_any_element()
    }
}

/// The accessible label for an editor of console `name` ("SQL editor, Console 1"), for
/// `EditorView::new`'s `a11y_label`.
pub fn label(name: SharedString) -> TextFn {
    Rc::new(move |cx: &App| {
        message(
            cx,
            "hui-editor-label",
            &[("name", UiArg::Text(name.clone()))],
        )
    })
}

/// What to announce when statement `statement` (1-based, in the run) reaches `state`; `None`
/// for the states that are not announced (queued, cancelling). The host passes it to
/// `EditorView::announce` (UI-EDIT-24 step 7).
pub fn exec_announcement(cx: &App, statement: u32, state: &ExecMarkState) -> Option<SharedString> {
    let n = ("n", UiArg::Count(u64::from(statement)));
    let rows = |rows: u64| ("rows", UiArg::Count(rows));
    let (id, more): (&'static str, Option<(&'static str, UiArg)>) = match state {
        // ## m3b-t01b-seams: Escalated is not announced, as Cancelling.
        ExecMarkState::Queued
        | ExecMarkState::Cancelling { .. }
        | ExecMarkState::Escalated { .. } => return None,
        ExecMarkState::Running { .. } => ("hui-editor-exec-running", None),
        ExecMarkState::Succeeded { rows: None, .. } => ("hui-editor-exec-done", None),
        ExecMarkState::Succeeded {
            rows: Some(RowsOutcome::Fetched { rows: r, more }),
            ..
        } => (
            if *more {
                "hui-editor-exec-rows-more"
            } else {
                "hui-editor-exec-rows"
            },
            Some(rows(*r)),
        ),
        ExecMarkState::Succeeded {
            rows: Some(RowsOutcome::Affected(r)),
            ..
        } => ("hui-editor-exec-affected", Some(rows(*r))),
        ExecMarkState::Failed {
            sqlstate: Some(code),
        } => (
            "hui-editor-exec-failed-code",
            Some(("sqlstate", UiArg::Text(code.clone()))),
        ),
        ExecMarkState::Failed { sqlstate: None } => ("hui-editor-exec-failed", None),
        ExecMarkState::Cancelled => ("hui-editor-exec-cancelled", None),
        ExecMarkState::Skipped => ("hui-editor-exec-skipped", None),
    };
    let mut args = vec![n];
    args.extend(more);
    Some(message(cx, id, &args))
}

/// A polite live announcement (execution status, "Run executes lines 3–7"): queued for the
/// next frame's live region, at most [`MAX_PENDING_ANNOUNCEMENTS`]. `EditorView::announce`
/// also asks for that frame.
pub fn announce(a: &A11yState, text: SharedString) {
    let mut nodes = a.nodes();
    if nodes.pending.len() == MAX_PENDING_ANNOUNCEMENTS {
        nodes.pending.pop_front();
    }
    nodes.pending.push_back(text);
}
