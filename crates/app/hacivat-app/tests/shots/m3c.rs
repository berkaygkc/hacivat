// ## m3c-t43-shots
//! The M3c shots (Task 43, part A): `design/m3b-safe` surfaces 14–22, each state at 1280x800 and
//! the 960x600 minimum, in every locale and theme, after every M3b shot.
//!
//! Part A covers the surfaces already merged into `m3c`: multi-cursor (Task 31), folding (32),
//! the editor find bar (33), the gutter and line commands (34), the grid model and its local
//! sort (35, 36), grid find and go to row (37), the views and the value viewer (38). Task 40's
//! grid AccessKit subtree draws nothing (its checks live in `cross_surface_m3c.rs`). **Part B**
//! adds surfaces 23 (Task 39's grid context menu), 24 (Task 41's narrow panes) and 25 (Task 42's
//! settings rows) at the marked extension points (`## m3c-t43-part-b`).
//!
//! The states are reached as the M3a and M3b ones are: steps over a live console of the fake
//! connections (`m3a::live`) for the editor surfaces, and steps over a result tile of fixture
//! stores (`m3a::results` then [`tile`]: no core runtime, R-M3-15) for the grid surfaces. Each
//! step waits on a condition. What the shots keep deterministic:
//!
//! - **The find pasteboard is seeded** ([`seed_find_pasteboard`]): every state installs an
//!   in-memory one first (empty, or with a fixed query), so no shot reads or writes the
//!   machine's macOS find pasteboard (Task 33, M10).
//! - **Sort jobs are held** (`results::sort::hold_publishing`): the "Sorting N rows…" note is
//!   shot with its job held, after the simulated clock passed `SLOW_AFTER`.
//! - **Clocks are the headless app's simulated clock** (the harness advances it only while a step
//!   waits); marks carry fixed durations (`exec_ui::fixtures::show_marks`).
//! - **No fetch ever runs:** the fixture stores' `FetchControl` ignores every request, so the
//!   go-to "Fetching to row" state stays at its first line.
//!
//! States whose picture depends on how far a background job got (the editor find's and the grid
//! find's "counting" lines, the text view's "Building" line) have no hold seam yet; they are left
//! out and listed in the Task 43 report.
#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hacivat_app::console::Consoles;
use hacivat_app::console::find_bar;
use hacivat_app::exec_ui;
use hacivat_app::results::adapter::ResultDisplay;
use hacivat_app::results::view::{ResultsShot, hosts};
use hacivat_app::results::views::{self, TextState, ViewMode};
use hacivat_app::results::{ActiveResult, find, goto, sort, value_viewer};
use hacivat_exec::{ErrorView, ResultTab, TabKey, UnitStatus};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_results::{
    CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, MemoryBudget, ResultId,
    ResultSource, ResultStore, ResultWriter, StoreSource,
};
use hacivat_session::session::{RunId, UnitId};
use hacivat_text::{SelectionRange, SelectionSet};
use hacivat_ui::popover::A11yDecl;
use hacivat_ui::text::RenderedText;
use hacivat_ui::toast::ToastQueue;
use hacivat_ui_core::gpui_surface::{
    App, Entity, EntityInputHandler as _, Focusable as _, Window, point, px,
};
use hacivat_ui_editor::preview::{self, BandStyle};
use hacivat_ui_editor::{EditorView, ExecMarkState};
use hacivat_ui_grid::model::{SortKey, SortSpec};
use hacivat_ui_grid::{ColId, GridView, ModelCell, ModelRow};
use hacivat_values::valfmt::DisplayZone;
use hacivat_values::{
    Chunk, ColumnMeta, LenUnit, LogicalType, NativeType, ResultSchema, TruncatedVal, Value,
};

use super::m3a::{
    self, DEV, Expect, MARKS_RS, Step, active_console, after, at, drawn, editor_in, instant, live,
    menu_open, message, rows, shows_label, step, succeeded,
};

// ---- the states -----------------------------------------------------------------------------

/// One M3c state (a shot surface), named after its `design/m3b-safe` surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum M3c {
    // 14 · Multi-cursor (Task 31)
    /// ⌃G twice from a selected word: three selections.
    MultiNext,
    /// A column selection: one range per line in the same display columns.
    MultiColumn,
    /// An input method's marked text mirrored at every caret (all in plain text).
    MultiImeMirrored,
    /// A caret inside a string: the composition goes to the main caret only, said once.
    MultiImeMain,
    /// Carets in three statements: the band shows only the main caret's statement.
    MultiRunMain,
    // 15 · Folding (Task 32)
    /// The pointer over the gutter: every open range's chevron.
    FoldHover,
    /// Two statements folded: their inlays, the line numbers skipping the hidden lines.
    FoldFolded,
    /// Folds over statements with execution marks: each fold's header carries the one that
    /// most needs the user.
    FoldMarks,
    /// A failure inside a folded statement: "failed inside the fold · line n".
    FoldFailed,
    /// The caret inside a folded statement: the band covers the header row.
    FoldRunCaret,
    /// A cancel not answered inside a fold: the amber hourglass on the header.
    FoldEscalated,
    /// "Tab moves focus": a chevron with the keyboard focus (its ring).
    FoldFocused,
    // 16 · The editor find bar (Task 33)
    /// "3 of 41".
    FindCount,
    /// Turkish-tolerant on: dotted and dotless i match.
    FindTolerant,
    /// Regular expression on.
    FindRegex,
    /// An invalid pattern: ink glyph, "Invalid pattern", the steps disabled.
    FindInvalid,
    /// "No matches".
    FindNone,
    /// Select all occurrences from the bar: every match a selection.
    FindSelectAll,
    /// Select all past 10,000: "Selected the first 10,000 of n matches".
    FindCapped,
    // 17 · The gutter and line commands (Task 34)
    /// ▶ on the hovered statement's first row.
    GutterHover,
    /// The ▶ tooltip: "Last runs", up to three.
    GutterLastRuns,
    /// "Tab moves focus": ▶ with the keyboard focus, every ▶ of the view shown.
    GutterFocused,
    /// A cancel not answered: no ▶ over the hourglass while the pointer is on its row.
    GutterNoRun,
    /// "Mixed line endings" in the status bar.
    EolMixed,
    /// Its menu: the counts, "Normalize to LF" (most lines), "Normalize to CRLF".
    EolMenu,
    /// ⌘U after two ⌃G: back to two selections, no text changed.
    UndoSelection,
    /// A statement recalled from the history: "From history: statement 2 of 48 · …".
    HistoryRecall,
    /// A confirmed statement waiting for Commit or Roll back: the amber stamp.
    MarkAwaiting,
    /// A confirmed statement rolled back: the undo glyph, "rolled back".
    MarkRolledBack,
    // 18 · The grid model and its local sort (Tasks 35, 36)
    /// Sorted by one column, ascending: the blue arrow.
    SortOne,
    /// Two sort levels: arrows and level digits.
    SortTwo,
    /// A partial result sorted: "Sorted locally…" + Clear sort.
    SortPartial,
    /// A sort job running past 200 ms (held): "Sorting n rows…" + Cancel.
    SortRunning,
    // ## m3c-t43-shots (fix round 1, m3)
    /// The sorted text column's header tooltip: code-point order (no mapped collation).
    SortCodePoint,
    /// A partial result with rows after the sort: both facts, Re-sort and Clear sort.
    SortPartialTail,
    /// A complete result with rows after the sort: "n new rows at the end (not sorted)".
    SortTail,
    /// Two frozen columns, the rest scrolled sideways under them.
    GridFrozen,
    /// Two ranges: "Selection 2 ranges · 10 cells".
    GridMultiRect,
    /// The column resize handle and its width tag.
    GridResize,
    // 19 · Grid find (Task 37)
    /// "k of N" in the result toolbar.
    GfindCount,
    /// Filter rows: only the matching rows, the note row with Show all rows.
    GfindFilter,
    /// A truncated cell scanned: "· may be incomplete".
    GfindIncomplete,
    /// An invalid pattern.
    GfindInvalid,
    /// Select all occurrences: every match a range.
    GfindSelectAll,
    /// Select all past 10,000: the toast says both numbers.
    GfindCapped,
    // 20 · Go to row (Task 37)
    /// The popover: field, hint, key hints.
    GotoOpen,
    /// `1.000` in a Turkish format locale: row 1000.
    GotoTurkish,
    /// Past the last row: "There's no row …".
    GotoOutOfRange,
    /// A column that does not exist.
    GotoUnknownColumn,
    /// A hidden column: its error and "Show it and go".
    GotoHidden,
    /// More than 1,000,000 rows to fetch: the question in place.
    GotoAsk,
    /// Fetching toward the row: the amber marker, Stop fetching.
    GotoFetching,
    // 21 · Views (Task 38)
    /// Above 100,000 rows: Transposed disabled in the switch.
    ViewTransposedOff,
    /// Transposed: one column per row.
    ViewTransposed,
    /// The record inspector beside the table.
    ViewRecord,
    /// Duplicate column names suffixed "(2)".
    ViewRecordDuplicates,
    /// A value clamped to three lines with "Show more".
    ViewRecordMore,
    /// A one-row result: the Record toggle with its word.
    ViewOneRow,
    /// The text view: a Pretty ASCII table.
    ViewText,
    /// Above 100,000 rows the text view asks first.
    ViewTextAsk,
    // 22 · The value viewer (Task 38)
    /// Wrapped text.
    ViewerText,
    /// JSON, Pretty.
    ViewerJsonPretty,
    /// JSON, Raw.
    ViewerJsonRaw,
    /// Bytes as hex.
    ViewerHex,
    /// XML, Pretty.
    ViewerXml,
    /// Not valid JSON: Raw kept, Pretty disabled, the line beside the switch.
    ViewerJsonInvalid,
    /// A value cut at the value limit: "Showing the first 1 MB of 5 MB" and the re-run.
    ViewerTruncated,
    // ## m3c-t43-part-b: surface 23 (Task 39's grid menu), 24 (Task 41's narrow panes) and
    // 25 (Task 42's settings rows) join here, after their tasks merge.
}

/// Task 31's states.
pub(super) const T31: [M3c; 5] = [
    M3c::MultiNext,
    M3c::MultiColumn,
    M3c::MultiImeMirrored,
    M3c::MultiImeMain,
    M3c::MultiRunMain,
];

/// Task 32's states.
pub(super) const T32: [M3c; 7] = [
    M3c::FoldHover,
    M3c::FoldFolded,
    M3c::FoldMarks,
    M3c::FoldFailed,
    M3c::FoldRunCaret,
    M3c::FoldEscalated,
    M3c::FoldFocused,
];

/// Task 33's states.
pub(super) const T33: [M3c; 7] = [
    M3c::FindCount,
    M3c::FindTolerant,
    M3c::FindRegex,
    M3c::FindInvalid,
    M3c::FindNone,
    M3c::FindSelectAll,
    M3c::FindCapped,
];

/// Task 34's states.
pub(super) const T34: [M3c; 10] = [
    M3c::GutterHover,
    M3c::GutterLastRuns,
    M3c::GutterFocused,
    M3c::GutterNoRun,
    M3c::EolMixed,
    M3c::EolMenu,
    M3c::UndoSelection,
    M3c::HistoryRecall,
    M3c::MarkAwaiting,
    M3c::MarkRolledBack,
];

/// Tasks 35 and 36's states.
pub(super) const T35_36: [M3c; 10] = [
    M3c::SortOne,
    M3c::SortTwo,
    M3c::SortPartial,
    M3c::SortRunning,
    M3c::SortCodePoint,
    M3c::SortPartialTail,
    M3c::SortTail,
    M3c::GridFrozen,
    M3c::GridMultiRect,
    M3c::GridResize,
];

/// Task 37's states.
pub(super) const T37: [M3c; 13] = [
    M3c::GfindCount,
    M3c::GfindFilter,
    M3c::GfindIncomplete,
    M3c::GfindInvalid,
    M3c::GfindSelectAll,
    M3c::GfindCapped,
    M3c::GotoOpen,
    M3c::GotoTurkish,
    M3c::GotoOutOfRange,
    M3c::GotoUnknownColumn,
    M3c::GotoHidden,
    M3c::GotoAsk,
    M3c::GotoFetching,
];

/// Task 38's states.
pub(super) const T38: [M3c; 15] = [
    M3c::ViewTransposedOff,
    M3c::ViewTransposed,
    M3c::ViewRecord,
    M3c::ViewRecordDuplicates,
    M3c::ViewRecordMore,
    M3c::ViewOneRow,
    M3c::ViewText,
    M3c::ViewTextAsk,
    M3c::ViewerText,
    M3c::ViewerJsonPretty,
    M3c::ViewerJsonRaw,
    M3c::ViewerHex,
    M3c::ViewerXml,
    M3c::ViewerJsonInvalid,
    M3c::ViewerTruncated,
];

/// Every M3c state in render order: by task (31–38).
// ## m3c-t43-part-b: then Task 39's, Task 41's and Task 42's.
pub(super) fn all() -> Vec<M3c> {
    T31.into_iter()
        .chain(T32)
        .chain(T33)
        .chain(T34)
        .chain(T35_36)
        .chain(T37)
        .chain(T38)
        .collect()
}

/// The states whose marker signature is shared, told apart only by their model check (the
/// selection's shape, a fold's or a mark's state, an icon the frame drew, the find's options,
/// the grid's sort, selection or view, the popover's line). Their checks read the models (and,
/// where a Fluent sentence is the difference, the sentence the frame drew).
pub(super) const TOLD_APART_BY_CHECK: [&[&str]; 12] = [
    &[
        "multi-next-occurrence",
        "multi-column",
        "multi-ime-mirrored",
        "multi-run-main-caret",
        "undo-selection",
    ],
    &["multi-ime-main-caret", "find-capped", "history-recall"],
    &[
        "fold-chevrons-hover",
        "fold-chevron-focused",
        "gutter-run-hover",
        "gutter-run-focused",
        "gutter-no-run",
    ],
    &["fold-folded", "fold-run-caret"],
    &[
        "fold-marks",
        "fold-failed",
        "fold-cancel-unanswered",
        "marks-awaiting",
        "marks-rolled-back",
    ],
    &[
        "find-count",
        "find-tolerant",
        "find-regex",
        "find-invalid",
        "find-none",
        "find-select-all",
    ],
    &[
        "sort-one",
        "sort-two-levels",
        "grid-frozen",
        "grid-multi-rect",
    ],
    &[
        "sort-partial",
        "sort-running",
        "sort-partial-tail",
        "sort-tail",
    ],
    &["gfind-count", "gfind-invalid", "gfind-select-all"],
    &[
        "goto-popover",
        "goto-turkish",
        "goto-out-of-range",
        "goto-unknown-column",
        "goto-hidden-column",
    ],
    &["view-transposed", "view-text"],
    &["view-record", "view-record-duplicates"],
];

/// The states whose marker a bare live console or result tile already shows (a gutter state's
/// connection name, a grid state's header): what they are about draws no label of its own (an
/// icon, a hover, a sort arrow, a selection box), so their model check and the icons the frame
/// drew hold them.
pub(super) const BY_CHECK_ALONE: [&str; 9] = [
    "fold-chevrons-hover",
    "fold-chevron-focused",
    "gutter-run-hover",
    "gutter-run-focused",
    "gutter-no-run",
    "sort-one",
    "sort-two-levels",
    "grid-frozen",
    "grid-multi-rect",
];

// ---- known findings -------------------------------------------------------------------------

/// Where a known finding shows: the pseudo-locales (`en-XA`, `tr-XT`), only at the 960x600
/// minimum, either of them, or everywhere.
// ## m3c-shotfix: kept for the next finding while the list is empty.
#[allow(
    dead_code,
    reason = "the known-findings list is empty after the M3c shot fixes"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Where {
    Pseudo,
    PseudoMin,
    PseudoOrMin,
    All,
}

impl Where {
    fn holds(self, locale: &str, size: (f32, f32)) -> bool {
        let pseudo = matches!(locale, "en-XA" | "tr-XT");
        let min = size.0 < 1000.0;
        match self {
            Where::Pseudo => pseudo,
            Where::PseudoMin => pseudo && min,
            Where::PseudoOrMin => pseudo || min,
            Where::All => true,
        }
    }
}

/// A product defect this task's shots found in a merged M3c surface, carried to its owner
/// (Task 43 may change only shots, tests and DESIGN.md). The harness prints each as a "known
/// finding" line instead of failing; every other violation still fails. The list is the
/// controller's to burn down: an entry goes when its owner's fix lands, and nothing outside
/// it is tolerated.
#[derive(Debug, Clone, Copy)]
pub(super) struct Finding {
    pub(super) state: &'static str,
    pub(super) at: Where,
    /// A fragment of the violation line: the label's `#id` or its creation site.
    pub(super) label: &'static str,
    /// The task that owns the fix.
    pub(super) owner: &'static str,
}

#[allow(
    dead_code,
    reason = "the known-findings list is empty after the M3c shot fixes"
)]
const fn finding(
    state: &'static str,
    at: Where,
    label: &'static str,
    owner: &'static str,
) -> Finding {
    Finding {
        state,
        at,
        label,
        owner,
    }
}

/// The known findings (see the Task 43 part A report, "Product findings").
// ## m3c-shotfix: F1–F8 are fixed at their source (the shot-fix report); the list is empty.
pub(super) const KNOWN_FINDINGS: [Finding; 0] = [];

// ## m3c-t43-shots (fix round 1, I1, I2, m1)

/// The one violation kind a finding tolerates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// `(b) clipped: "…" (… px painted) at <path>#<id>`: the label is cut.
    Clipped,
    /// `(g) a grid header is data, never pseudo-localised: "…" at …#header`: allowed only for
    /// the header text the shot passes (the Transposed name column's word, F7).
    GridHeaderWord,
}

/// The marker of each kind in a violation line.
const CLIPPED: &str = ": (b) clipped: ";
const GRID_HEADER_WORD: &str = ": (g) a grid header is data, never pseudo-localised: ";

/// What no finding may ever match: a trust line, a missing surface, a failed state check, and
/// the other rules' kinds.
const NEVER: [&str; 8] = [
    ": (t) ",
    "the surface is missing",
    "the state check failed",
    ": (a) ",
    ": (c) ",
    ": (d) ",
    ": (e) ",
    ": (m) ",
];

/// Every label id of `state` that is a trust signal: its trust groups and the labels the harness
/// never lets be cut (`Surface::uncut_trust`).
pub(super) fn trust_ids(state: M3c) -> Vec<&'static str> {
    let mut ids: Vec<&'static str> = state
        .required()
        .iter()
        .flat_map(|g| g.iter().copied())
        .collect();
    ids.extend(["#capsule-text-name", "#confirm-type-name"]);
    ids
}

impl Finding {
    /// The kind this finding tolerates (the label names it for the header-word finding).
    pub(super) fn kind(&self) -> Kind {
        if self.label == "a grid header is data" {
            Kind::GridHeaderWord
        } else {
            Kind::Clipped
        }
    }

    /// Whether `violation`, a line of `state`'s shot in `locale` at `size`, is this finding:
    /// the finding's state and scope, its one kind, and its label at the end of the line's
    /// `at <path>` part (an `#id` suffix, an id fragment, or a creation site without an id).
    /// `header_word` is the Transposed name column's word in the shot's locale. A trust label,
    /// a missing surface or any other rule's line is never a finding.
    pub(super) fn matches(
        &self,
        state: M3c,
        locale: &str,
        size: (f32, f32),
        violation: &str,
        header_word: Option<&str>,
    ) -> bool {
        if self.state != state.name() || !self.at.holds(locale, size) {
            return false;
        }
        if NEVER.iter().any(|n| violation.contains(n)) {
            return false;
        }
        let Some(at) = violation.rsplit_once(" at ").map(|(_, at)| at) else {
            return false;
        };
        let (site, id) = match at.rsplit_once('#') {
            Some((site, id)) => (site, Some(format!("#{id}"))),
            None => (at, None),
        };
        // A trust label is never tolerated, whatever the entry says.
        if id
            .as_deref()
            .is_some_and(|id| trust_ids(state).contains(&id))
        {
            return false;
        }
        match self.kind() {
            Kind::Clipped => {
                violation.contains(CLIPPED)
                    && match (&id, self.label.strip_prefix('#')) {
                        // An `#id` label: the line's id is exactly it.
                        (Some(id), Some(_)) => id == self.label,
                        // A creation site: no id, the site holds it.
                        (None, None) if self.label.contains(".rs:") => site.contains(self.label),
                        // An id fragment (`value-viewer-`): inside the line's id.
                        (Some(id), None) if !self.label.contains(".rs:") => id.contains(self.label),
                        _ => false,
                    }
            }
            Kind::GridHeaderWord => {
                let Some((_, rest)) = violation.split_once(GRID_HEADER_WORD) else {
                    return false;
                };
                let text = rest.rsplit_once(" at ").map_or(rest, |(t, _)| t);
                id.as_deref() == Some("#header")
                    && header_word
                        .is_some_and(|w| super::strip(text.trim_matches('"')) == super::strip(w))
            }
        }
    }
}

thread_local! {
    /// The findings that matched a line in this process (`KNOWN_FINDINGS` indices).
    static MATCHED: RefCell<std::collections::BTreeSet<usize>> = RefCell::default();
}

/// The known finding a violation of `state`'s shot in `locale` at `size` is, if any (recorded
/// for [`stale_findings`]).
pub(super) fn known_finding(
    state: M3c,
    locale: &str,
    size: (f32, f32),
    violation: &str,
    header_word: Option<&str>,
) -> Option<&'static Finding> {
    let (i, f) = KNOWN_FINDINGS
        .iter()
        .enumerate()
        .find(|(_, f)| f.matches(state, locale, size, violation, header_word))?;
    MATCHED.with(|m| m.borrow_mut().insert(i));
    Some(f)
}

/// The Transposed name column's word in the shot's locale (F7's tolerated header text).
pub(super) fn header_word(cx: &App) -> String {
    message(cx, "hv-transposed-col", Vec::new())
}

/// The findings of a state whose 16 shots all rendered in this process that matched nothing:
/// stale entries (the owner fixed the defect), which must go before they hide a regression.
pub(super) fn stale_findings(
    rendered: &std::collections::BTreeMap<&'static str, usize>,
) -> Vec<String> {
    MATCHED.with(|m| {
        let m = m.borrow();
        KNOWN_FINDINGS
            .iter()
            .enumerate()
            .filter(|(i, f)| rendered.get(f.state) == Some(&16) && !m.contains(i))
            .map(|(_, f)| {
                format!(
                    "stale known finding ({}): {f:?} matched no line; remove it",
                    f.owner
                )
            })
            .collect()
    })
}

// ---- texts and data -------------------------------------------------------------------------

/// Three statements naming `musteri_id` (surface 14).
const MULTI: &str = "SELECT musteri_id, sehir FROM siparisler WHERE sehir = 'İzmir';\nSELECT musteri_id, toplam FROM faturalar;\nUPDATE siparisler SET durum = 'kargoda' WHERE musteri_id = 1042;\n";
/// Aligned rows for a column selection (surface 14).
const COLUMN: &str = "INSERT INTO sehirler (kod, ad) VALUES\n  (34, 'İstanbul'),\n  (35, 'İzmir'),\n  (06, 'Ankara'),\n  (16, 'Bursa');\n";
/// Three multi-line statements in a region (surfaces 15 and 17).
const FOLDS: &str = "-- region Raporlar\nSELECT o.id,\n       o.total\n  FROM orders o\n WHERE o.total > 100;\n\nUPDATE orders\n   SET status = 'shipped'\n WHERE id = 1042;\n\nSELECT c.\"müşteri adı\",\n       count(*)\n  FROM customers c\n GROUP BY 1;\n-- endregion\n";
/// The first statement of [`FOLDS`] (line 1).
const FOLD_1: &str = "SELECT o.id,\n       o.total\n  FROM orders o\n WHERE o.total > 100;";
/// The second (line 6).
const FOLD_2: &str = "UPDATE orders\n   SET status = 'shipped'\n WHERE id = 1042;";
/// The third (line 10).
const FOLD_3: &str = "SELECT c.\"müşteri adı\",\n       count(*)\n  FROM customers c\n GROUP BY 1;";
/// Line endings of both kinds (surface 17).
const EOL: &str = "SELECT 1;\r\nSELECT 2;\r\nSELECT o.id\n  FROM orders o\n WHERE o.total > 100;\n";

/// A script with 41 occurrences of `sehir` (surface 16): 40 rows of a VALUES list and the
/// insert's column list.
fn find_script() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        let cities = ["İzmir", "Istanbul", "ıgdır", "Ankara", "Bursa"];
        let mut s = String::from("INSERT INTO adresler (id, sehir, ilce) VALUES\n");
        for i in 0..40 {
            let end = if i == 39 { ";" } else { "," };
            s.push_str(&format!(
                "  ({}, 'sehir: {}', 'merkez'){end}\n",
                i + 1,
                cities[i % cities.len()]
            ));
        }
        s.push_str("SELECT sehir, count(*) FROM adresler GROUP BY sehir;\n");
        s
    })
}

/// A script with more than 10,000 occurrences of `x` (the Select all cap).
fn many_matches() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        let mut s = String::from("-- x\n");
        for i in 0..1_050 {
            s.push_str(&format!("SELECT x, x, x, x, x, x, x, x, x, x FROM t{i};\n"));
        }
        s
    })
}

// ---- names, markers, trust ------------------------------------------------------------------

/// The grid's header labels (data: the same in every locale).
const HEADER: &str = m3a::GRID_HEADER;
/// The fold gutter's file (the inlay).
const FOLD_GUTTER: &str = "hacivat-ui-editor/src/fold_gutter.rs:";
/// The menu file.
const MENU: &str = "hacivat-ui/src/menu.rs:";
/// The views' file (the switch, the inspector, the text view).
const VIEWS: &str = "hacivat-app/src/results/views.rs:";
/// The go-to popover's file.
const GOTO: &str = "hacivat-app/src/results/goto.rs:";
/// The grid view's file (its header tooltip).
const GRID_VIEW: &str = "hacivat-ui-grid/src/view.rs:";

impl M3c {
    pub(super) fn name(self) -> &'static str {
        use M3c::*;
        match self {
            MultiNext => "multi-next-occurrence",
            MultiColumn => "multi-column",
            MultiImeMirrored => "multi-ime-mirrored",
            MultiImeMain => "multi-ime-main-caret",
            MultiRunMain => "multi-run-main-caret",
            FoldHover => "fold-chevrons-hover",
            FoldFolded => "fold-folded",
            FoldMarks => "fold-marks",
            FoldFailed => "fold-failed",
            FoldRunCaret => "fold-run-caret",
            FoldEscalated => "fold-cancel-unanswered",
            FoldFocused => "fold-chevron-focused",
            FindCount => "find-count",
            FindTolerant => "find-tolerant",
            FindRegex => "find-regex",
            FindInvalid => "find-invalid",
            FindNone => "find-none",
            FindSelectAll => "find-select-all",
            FindCapped => "find-capped",
            GutterHover => "gutter-run-hover",
            GutterLastRuns => "gutter-last-runs",
            GutterFocused => "gutter-run-focused",
            GutterNoRun => "gutter-no-run",
            EolMixed => "eol-mixed",
            EolMenu => "eol-menu",
            UndoSelection => "undo-selection",
            HistoryRecall => "history-recall",
            MarkAwaiting => "marks-awaiting",
            MarkRolledBack => "marks-rolled-back",
            SortOne => "sort-one",
            SortTwo => "sort-two-levels",
            SortPartial => "sort-partial",
            SortRunning => "sort-running",
            SortCodePoint => "sort-code-point-tip",
            SortPartialTail => "sort-partial-tail",
            SortTail => "sort-tail",
            GridFrozen => "grid-frozen",
            GridMultiRect => "grid-multi-rect",
            GridResize => "grid-resize",
            GfindCount => "gfind-count",
            GfindFilter => "gfind-filter",
            GfindIncomplete => "gfind-incomplete",
            GfindInvalid => "gfind-invalid",
            GfindSelectAll => "gfind-select-all",
            GfindCapped => "gfind-capped",
            GotoOpen => "goto-popover",
            GotoTurkish => "goto-turkish",
            GotoOutOfRange => "goto-out-of-range",
            GotoUnknownColumn => "goto-unknown-column",
            GotoHidden => "goto-hidden-column",
            GotoAsk => "goto-fetch-ask",
            GotoFetching => "goto-fetching",
            ViewTransposedOff => "view-transposed-disabled",
            ViewTransposed => "view-transposed",
            ViewRecord => "view-record",
            ViewRecordDuplicates => "view-record-duplicates",
            ViewRecordMore => "view-record-show-more",
            ViewOneRow => "view-one-row",
            ViewText => "view-text",
            ViewTextAsk => "view-text-ask",
            ViewerText => "viewer-text",
            ViewerJsonPretty => "viewer-json-pretty",
            ViewerJsonRaw => "viewer-json-raw",
            ViewerHex => "viewer-hex",
            ViewerXml => "viewer-xml-pretty",
            ViewerJsonInvalid => "viewer-json-invalid",
            ViewerTruncated => "viewer-truncated",
        }
    }

    /// Whether the state is a grid state (its grid must draw a header).
    pub(super) fn is_grid(self) -> bool {
        T35_36.contains(&self)
            || T37.contains(&self)
            || matches!(
                self,
                M3c::ViewTransposedOff
                    | M3c::ViewRecord
                    | M3c::ViewRecordDuplicates
                    | M3c::ViewRecordMore
                    | M3c::ViewOneRow
            )
    }

    /// The label that proves the state is on screen: its creation site (a `#id` or a source
    /// file) and, where that site draws more than this state, its data text (the same in every
    /// locale).
    pub(super) fn marker(self) -> (&'static str, Option<&'static str>) {
        use M3c::*;
        match self {
            // 14 · the status bar's caret count; the IME states the marked text (data).
            MultiNext | MultiColumn | MultiRunMain | UndoSelection => {
                ("#editor-segment-carets", None)
            }
            MultiImeMirrored => ("#editor-segment-carets", None),
            MultiImeMain => ("#status-message", None),
            // 15 · the inlay of a folded statement, else the gutter's own files.
            FoldHover | FoldFocused => ("#console-connection-name", Some(DEV)),
            FoldFolded | FoldRunCaret => (FOLD_GUTTER, None),
            FoldMarks => (MARKS_RS, None),
            FoldFailed => (MARKS_RS, None),
            FoldEscalated => (MARKS_RS, None),
            // 16 · the bar's count.
            FindCount | FindTolerant | FindRegex | FindSelectAll => ("#find-count-text", None),
            FindInvalid => ("#find-count-text", None),
            FindNone => ("#find-count-text", None),
            FindCapped => ("#status-message", None),
            // 17
            GutterHover | GutterFocused | GutterNoRun => ("#console-connection-name", Some(DEV)),
            GutterLastRuns => ("hacivat-ui-editor/src/gutter.rs:", None),
            EolMixed => ("#editor-segment-eol-label", None),
            EolMenu => (MENU, None),
            HistoryRecall => ("#status-message", None),
            MarkAwaiting | MarkRolledBack => (MARKS_RS, None),
            // 18 · a header the grid drew (data), or the sort note.
            SortOne | SortTwo | GridFrozen | GridMultiRect => (HEADER, Some("şehir")),
            SortPartial | SortRunning | SortPartialTail | SortTail => ("#results-sort-note", None),
            GridResize => (HEADER, Some("müşteri")),
            // The header tooltip (the grid's own view draws it).
            SortCodePoint => (GRID_VIEW, None),
            // 19 · the grid find bar's count, the filter note, the incomplete mark.
            GfindCount | GfindSelectAll => ("#grid-find-count-text", None),
            GfindFilter => ("#results-filter-note", None),
            GfindIncomplete => ("#grid-find-incomplete-text", None),
            GfindInvalid => ("#grid-find-count-text", None),
            GfindCapped => (m3a::TOAST, None),
            // 20 · the popover's line.
            GotoOpen | GotoTurkish | GotoOutOfRange | GotoUnknownColumn | GotoHidden => {
                ("#goto-line", None)
            }
            GotoAsk => (GOTO, None),
            GotoFetching => ("#button-label-goto-stop", None),
            // 21
            ViewTransposedOff => ("#results-rows-timing", None),
            ViewTransposed => (VIEWS, None),
            ViewRecord | ViewRecordDuplicates => ("#record-pos", None),
            ViewRecordMore => ("#record-pos", None),
            ViewOneRow => (VIEWS, None),
            ViewText => (VIEWS, None),
            ViewTextAsk => ("#text-big-title", None),
            // 22 · the viewer's column (data).
            ViewerText => ("#value-viewer-column", Some("not")),
            ViewerJsonPretty => ("#value-viewer-column", Some("belge")),
            ViewerJsonRaw => ("#value-viewer-column", Some("ham_belge")),
            ViewerHex => ("#value-viewer-column", Some("imza")),
            ViewerXml => ("#value-viewer-column", Some("fatura_xml")),
            ViewerJsonInvalid => ("#value-viewer-column", Some("bozuk_belge")),
            ViewerTruncated => ("#value-viewer-column", Some("ek")),
        }
    }

    /// Labels (`#id`) the state must show besides its marker: what tells it from a state with
    /// the same marker.
    pub(super) fn also_shown(self) -> &'static [&'static str] {
        use M3c::*;
        match self {
            // The marks' own file draws M3b's escalation shot too: the editor's status cell
            // tells a live editor apart (M3b's `esc-kept-waiting` asks for no more).
            // Likewise the gutter states, whose connection name M3b's `preview-as-one` shows.
            FoldMarks | FoldFailed | FoldEscalated | MarkAwaiting | MarkRolledBack | FoldHover
            | FoldFocused | GutterHover | GutterFocused | GutterNoRun => &["#editor-segment-lncol"],
            GfindInvalid => &["#header"],
            GfindFilter => &["#header", "#grid-find-count-text"],
            GotoAsk => &["#header", "#goto-line"],
            GotoFetching => &["#header", "#goto-line"],
            ViewRecordMore => &["#header", "#button-label-record-show-more-0"],
            // ## m3c-shotfix F8: the footer's size again (the viewer keeps its header and footer
            // in any room); the model check asks for the truncation notice.
            ViewerTruncated => &["#value-viewer-size"],
            // A grid state shows its grid (a header it drew), whatever its own marker.
            s if s.is_grid() => &["#header"],
            _ => &[],
        }
    }

    /// The trust signals the state must show (NOTES cross-cutting #3: a popover never covers
    /// one): each group lists the label ids of which one must be visible and painted.
    pub(super) fn required(self) -> &'static [&'static [&'static str]] {
        const ENV: &[&str] = &["#status-env-label"];
        match self {
            // The go-to popover and the value viewer over a production result.
            M3c::GotoAsk | M3c::ViewerTruncated => &[ENV],
            _ => &[],
        }
    }

    /// Whether the title-bar capsule shows production (the one filled badge).
    pub(super) fn shows_production(self) -> bool {
        false
    }

    /// The steps after the shell opened.
    pub(super) fn steps(self) -> Vec<Step> {
        let mut s = vec![seed_find_pasteboard(None)];
        s.extend(self.state_steps());
        s
    }

    fn state_steps(self) -> Vec<Step> {
        use M3c::*;
        match self {
            // ---- 14
            MultiNext | UndoSelection => {
                let start = at(MULTI, "musteri_id");
                let mut s = live(DEV, MULTI, start..start + "musteri_id".len(), Expect::Any);
                s.push(command("editor.select_next_occurrence"));
                s.push(command("editor.select_next_occurrence"));
                s.push(carets(3));
                if self == UndoSelection {
                    s.push(command("editor.undo_selection"));
                    s.push(carets(2));
                }
                s
            }
            MultiColumn => {
                let mut s = live(DEV, COLUMN, 0..0, Expect::Any);
                s.push(select(column_ranges()));
                s.push(carets(4));
                s
            }
            MultiImeMirrored | MultiImeMain => {
                let mut s = live(DEV, MULTI, 0..0, Expect::Any);
                // Plain text everywhere, or (main caret only) one caret inside `'İzmir'`.
                let first = if self == MultiImeMirrored {
                    after(MULTI, "SELECT ")
                } else {
                    after(MULTI, "'İz")
                };
                let second = after(MULTI, "toplam");
                let third = after(MULTI, "kargoda' ") - 1;
                s.push(select(vec![
                    (first, first),
                    (second, second),
                    (third, third),
                ]));
                s.push(carets(3));
                s.push(step(
                    |window, cx| {
                        let editor = editor_in(window, cx).expect("a console");
                        editor.update(cx, |e, cx| {
                            e.replace_and_mark_text_in_range(None, "shuju", None, window, cx);
                        });
                    },
                    |window, cx| {
                        editor_in(window, cx).is_some_and(|e| e.read(cx).status().composing)
                    },
                ));
                s
            }
            MultiRunMain => {
                let mut s = live(DEV, MULTI, 0..0, Expect::Any);
                let a = after(MULTI, "SELECT musteri_id, s");
                let b = after(MULTI, "toplam FROM f");
                let c = after(MULTI, "kargoda' WHERE m");
                // The main caret is the last one added (the third statement's).
                s.push(select(vec![(a, a), (b, b), (c, c)]));
                s.push(band(BandStyle::Run));
                s
            }
            // ---- 15
            FoldHover => {
                let mut s = live(DEV, FOLDS, 0..0, Expect::Any);
                s.push(folds_ready());
                s.push(hover_gutter(Gutter::Fold, 1));
                s
            }
            FoldFolded => {
                let mut s = live(DEV, FOLDS, 0..0, Expect::Any);
                s.push(fold(&[1, 10]));
                s
            }
            FoldMarks | FoldFailed | FoldEscalated => {
                let mut s = live(DEV, FOLDS, FOLDS.len()..FOLDS.len(), Expect::Any);
                let marks = match self {
                    FoldMarks => vec![
                        (FOLD_1, succeeded(42, Some(rows(100)))),
                        (FOLD_2, succeeded(12, None)),
                        (FOLD_3, (UnitStatus::Queued { position: 1 }, Duration::ZERO)),
                    ],
                    FoldFailed => vec![
                        (FOLD_1, succeeded(42, Some(rows(100)))),
                        (FOLD_2, failed("23514", "new row violates check constraint")),
                        (FOLD_3, (UnitStatus::Skipped, Duration::ZERO)),
                    ],
                    _ => vec![
                        (FOLD_1, succeeded(42, Some(rows(100)))),
                        (FOLD_2, (UnitStatus::Cancelling, Duration::from_secs(14))),
                        (FOLD_3, (UnitStatus::Queued { position: 1 }, Duration::ZERO)),
                    ],
                };
                s.push(show_marks(FOLDS, marks));
                if self == FoldEscalated {
                    s.push(instant(|window, cx| {
                        let c = active_console(window, cx).expect("a console");
                        exec_ui::escalation::fixtures::escalate_mark(
                            c,
                            exec_ui::fixtures::fixture_key(1),
                            14,
                            cx,
                        );
                    }));
                }
                // A failure inside a folded region: the region's header carries it, naming its line.
                s.push(fold(if self == FoldFailed {
                    &[0]
                } else {
                    &[1, 6, 10]
                }));
                s
            }
            FoldRunCaret => {
                let caret = after(FOLDS, "o.to");
                let mut s = live(DEV, FOLDS, caret..caret, Expect::Band(BandStyle::Run));
                s.push(fold(&[1]));
                s.push(band(BandStyle::Run));
                s
            }
            FoldFocused => {
                let mut s = live(DEV, FOLDS, 0..0, Expect::Any);
                s.push(folds_ready());
                s.push(command("editor.toggle_tab_moves_focus"));
                s.push(gutter_tab_to(GutterStop::Fold));
                s
            }
            // ---- 16
            FindCount | FindTolerant | FindRegex | FindInvalid | FindNone | FindSelectAll => {
                let (query, flips): (&'static str, &'static [usize]) = match self {
                    FindCount => ("sehir", &[]),
                    // Turkish-tolerant: `izmir` finds İzmir; Istanbul and ıgdır by their i.
                    FindTolerant => ("ızmır", &[]),
                    FindRegex => (r"\(\d+, 'sehir", &[2]),
                    FindInvalid => (r"sehir: (İzmir", &[2]),
                    FindNone => ("ilce_kodu", &[]),
                    _ => ("sehir", &[]),
                };
                let tolerant = self == FindTolerant;
                let mut s = live(DEV, find_script(), 0..0, Expect::Any);
                s.push(open_find(query, flips, tolerant));
                if self == FindSelectAll {
                    s.push(step(
                        |window, cx| {
                            let c = active_console(window, cx).expect("a console");
                            find_bar::focus_cycle(window, cx);
                            let _ = c;
                            run_in_find("editor.select_all_occurrences", window, cx);
                        },
                        |window, cx| {
                            editor_in(window, cx).is_some_and(|e| e.read(cx).status().carets > 1)
                        },
                    ));
                }
                s
            }
            FindCapped => {
                let mut s = live(DEV, many_matches(), 0..0, Expect::Any);
                s.push(open_find("x", &[1], false));
                s.push(step(
                    |window, cx| run_in_find("editor.select_all_occurrences", window, cx),
                    |window, cx| {
                        editor_in(window, cx).is_some_and(|e| e.read(cx).status().carets == 10_000)
                    },
                ));
                s
            }
            // ---- 17
            GutterHover | GutterLastRuns => {
                let mut s = live(DEV, FOLDS, 0..0, Expect::Any);
                if self == GutterLastRuns {
                    s.push(show_marks(FOLDS, vec![(FOLD_2, succeeded(12, None))]));
                    // ## fix round 1 (m9): the run's time, through the same message the live one
                    // uses (`hv-history-time`, the format locale's medium time), at a fixed wall
                    // clock (the live one reads the system clock; no zone conversion here).
                    s.push(instant(|window, cx| {
                        let editor = editor_in(window, cx).expect("a console");
                        let when: hacivat_ui::text::TextFn = Rc::new(|cx: &App| {
                            cx.global::<hacivat_app::l10n_global::L10nGlobal>()
                                .l10n
                                .t_dynamic("hv-history-time", vec![("t", run_time())])
                                .into()
                        });
                        editor.update(cx, |e, cx| {
                            e.set_exec_when(exec_ui::fixtures::fixture_key(0), Some(when), cx)
                        });
                    }));
                }
                s.push(hover_gutter(Gutter::Run, 6));
                if self == GutterLastRuns {
                    // Again over the ▶ the first move showed: its tooltip starts on a move over it.
                    s.push(hover_gutter(Gutter::Run, 6));
                }
                if self == GutterLastRuns {
                    s.push(step(|_, _| {}, |_, cx| tooltip_open(cx)));
                }
                s
            }
            GutterFocused => {
                let mut s = live(DEV, FOLDS, 0..0, Expect::Any);
                s.push(command("editor.toggle_tab_moves_focus"));
                s.push(gutter_tab_to(GutterStop::Run));
                s
            }
            GutterNoRun => {
                let mut s = live(DEV, FOLDS, FOLDS.len()..FOLDS.len(), Expect::Any);
                s.push(show_marks(
                    FOLDS,
                    vec![
                        (FOLD_1, succeeded(42, Some(rows(100)))),
                        (FOLD_2, (UnitStatus::Cancelling, Duration::from_secs(14))),
                    ],
                ));
                s.push(instant(|window, cx| {
                    let c = active_console(window, cx).expect("a console");
                    exec_ui::escalation::fixtures::escalate_mark(
                        c,
                        exec_ui::fixtures::fixture_key(1),
                        14,
                        cx,
                    );
                }));
                s.push(hover_gutter(Gutter::Run, 6));
                s
            }
            EolMixed | EolMenu => {
                let mut s = live(DEV, EOL, EOL.len()..EOL.len(), Expect::Any);
                s.push(step(
                    |_, _| {},
                    |window, cx| {
                        editor_in(window, cx)
                            .is_some_and(|e| e.read(cx).status().mixed_line_endings)
                    },
                ));
                if self == EolMenu {
                    s.push(step(
                        |window, cx| {
                            let editor = editor_in(window, cx).expect("a console");
                            let at = point(
                                px(900.0),
                                px(f32::from(window.viewport_size().height) - 30.0),
                            );
                            hacivat_app::console::status::open_eol_menu(
                                editor.downgrade(),
                                at,
                                window,
                                cx,
                            );
                        },
                        |window, cx| menu_open(window, cx),
                    ));
                }
                s
            }
            HistoryRecall => {
                let mut s = live(DEV, "", 0..0, Expect::Any);
                s.push(step(
                    |window, cx| {
                        let editor = editor_in(window, cx).expect("a console");
                        let sentence = message(
                            cx,
                            "hv-history-recall",
                            vec![
                                ("i", hacivat_l10n::ArgValue::Count(2)),
                                ("n", hacivat_l10n::ArgValue::Count(48)),
                                ("older", hacivat_l10n::ArgValue::Text("⌃⌥↑".into())),
                                ("newer", hacivat_l10n::ArgValue::Text("⌃⌥↓".into())),
                            ],
                        );
                        editor.update(cx, |e, cx| {
                            e.show_recalled(
                                "SELECT o.id, o.total FROM orders o WHERE o.total > 100;",
                                sentence.into(),
                                cx,
                            );
                        });
                    },
                    |window, cx| {
                        editor_in(window, cx).is_some_and(|e| {
                            hacivat_ui_editor::commands::status_message(e.read(cx)).is_some()
                        })
                    },
                ));
                s
            }
            MarkAwaiting | MarkRolledBack => {
                let mut s = live(DEV, FOLDS, FOLDS.len()..FOLDS.len(), Expect::Any);
                let second = if self == MarkAwaiting {
                    (
                        UnitStatus::Running {
                            since: Instant::now(),
                        },
                        Duration::from_secs(3),
                    )
                } else {
                    (
                        UnitStatus::Failed(ErrorView {
                            title_id: "exec-error-confirm-undone",
                            code: None,
                            server_text: None,
                            hint_id: None,
                            doc_offset: None,
                            vendor_code: None,
                        }),
                        Duration::from_millis(9),
                    )
                };
                s.push(show_marks(
                    FOLDS,
                    vec![(FOLD_1, succeeded(42, Some(rows(100)))), (FOLD_2, second)],
                ));
                if self == MarkAwaiting {
                    s.push(instant(|window, cx| {
                        let editor = editor_in(window, cx).expect("a console");
                        let key = exec_ui::fixtures::fixture_key(1);
                        editor.update(cx, |e, cx| {
                            e.set_exec_state(key, ExecMarkState::Awaiting, None, None, cx);
                        });
                    }));
                }
                s
            }
            // ---- 18
            SortOne | SortTwo => {
                let mut s = tile(EnvTag::Dev, || vec![complete(orders(40))]);
                let spec = if self == SortOne {
                    SortSpec::by(ColId(1), false)
                } else {
                    let mut spec = SortSpec::by(ColId(1), false);
                    spec.keys.push(SortKey {
                        col: ColId(2),
                        desc: true,
                    });
                    spec
                };
                s.push(sort_by(spec));
                s
            }
            SortPartial | SortPartialTail | SortTail => {
                let partial = self != SortTail;
                let mut s = tile(EnvTag::Dev, move || {
                    vec![live_source(orders(40), Completeness::Suspended(cursor()))]
                });
                s.push(sort_by(SortSpec::by(ColId(1), false)));
                if self != SortPartial {
                    s.push(instant(move |_, _| {
                        WRITER.with(|w| {
                            let mut w = w.borrow_mut();
                            let w = w.as_mut().expect("the live store's writer");
                            let (schema, rows) = orders_from(40, 12);
                            w.append(Chunk::from_values(&schema, rows));
                            if !partial {
                                w.set_state(Completeness::Complete);
                            }
                        });
                    }));
                    s.push(step(
                        |_, _| {},
                        |window, cx| {
                            grid(window, cx).is_some_and(|g| g.read(cx).tail_boundary().is_some())
                        },
                    ));
                }
                s
            }
            // The text column sorted: its collation is not known here, so its header tooltip says
            // text compares by code point (T36 ruling R1). The pointer rests on the header.
            SortCodePoint => {
                let mut s = tile(EnvTag::Dev, || vec![complete(orders(40))]);
                s.push(sort_by(SortSpec::by(ColId(1), false)));
                // Again once the tooltip's hover area is laid out: a tooltip starts on a move.
                s.push(hover_header(1));
                s.push(hover_header(1));
                s.push(step(|_, _| {}, |_, cx| header_tip_open(cx)));
                s
            }
            SortRunning => {
                let mut s = tile(EnvTag::Dev, || vec![complete(orders(40))]);
                s.push(instant(|_, cx| sort::hold_publishing(cx)));
                s.push(step(
                    |window, cx| {
                        let g = grid(window, cx).expect("a grid");
                        g.update(cx, |g, cx| {
                            g.request_sort(SortSpec::by(ColId(1), false), cx)
                        });
                    },
                    |window, cx| {
                        grid(window, cx)
                            .and_then(|g| sort::status(&g, cx))
                            .is_some_and(|s| s.slow)
                    },
                ));
                s
            }
            GridFrozen => {
                let mut s = tile(EnvTag::Dev, || vec![complete(wide(40))]);
                s.push(step(
                    |window, cx| {
                        let g = grid(window, cx).expect("a grid");
                        g.update(cx, |g, cx| g.set_frozen(2, cx));
                    },
                    // Scrolled sideways once the grid has laid its columns out (retried).
                    |window, cx| {
                        let Some(g) = grid(window, cx) else {
                            return false;
                        };
                        if g.read(cx).scroll_state().left_x <= 0.0 {
                            g.update(cx, |g, cx| g.scroll_px(0.0, 420.0, cx));
                        }
                        g.read(cx).frozen() == 2 && g.read(cx).scroll_state().left_x > 0.0
                    },
                ));
                s
            }
            GridMultiRect => {
                let mut s = tile(EnvTag::Dev, || vec![complete(orders(40))]);
                s.push(probe_grid());
                // Rows 1–2 × şehir, müşteri; then ⌘-drag rows 4–5 × müşteri…durum (all in view at 960 × 600).
                s.push(step(
                    |window, cx| drag_cells(window, cx, (0, 1), (1, 2), false),
                    |window, cx| {
                        grid(window, cx).is_some_and(|g| g.read(cx).selection().ranges() == 1)
                    },
                ));
                s.push(step(
                    |window, cx| drag_cells(window, cx, (3, 2), (4, 4), true),
                    |window, cx| {
                        grid(window, cx).is_some_and(|g| g.read(cx).selection().ranges() == 2)
                    },
                ));
                s
            }
            GridResize => {
                let mut s = tile(EnvTag::Dev, || vec![complete(orders(40))]);
                s.push(probe_grid());
                s.push(step(
                    |window, cx| hover_column_edge(window, cx, 2),
                    |window, cx| {
                        grid(window, cx).is_some_and(|g| g.read(cx).resize_tag().is_some())
                    },
                ));
                s
            }
            // ---- 19
            GfindCount | GfindFilter | GfindInvalid | GfindSelectAll => {
                let mut s = tile(EnvTag::Dev, || vec![complete(orders(40))]);
                let (query, regex) = match self {
                    GfindInvalid => ("(İzmir", true),
                    _ => ("Bursa", false),
                };
                s.push(grid_find(query, regex, self == GfindFilter));
                if self == GfindSelectAll {
                    s.push(step(
                        |window, cx| {
                            let c = active_console(window, cx).expect("a console");
                            find::select_all(c, cx);
                        },
                        |window, cx| {
                            grid(window, cx).is_some_and(|g| g.read(cx).selection().ranges() > 1)
                        },
                    ));
                }
                s
            }
            GfindIncomplete => {
                let mut s = tile(EnvTag::Dev, || vec![complete(with_cut_note(12))]);
                s.push(grid_find("teslimat", false, false));
                s
            }
            GfindCapped => {
                let mut s = tile(EnvTag::Dev, || vec![complete(orders(12_000))]);
                s.push(grid_find("a", false, false));
                s.push(step(
                    |window, cx| {
                        let c = active_console(window, cx).expect("a console");
                        find::select_all(c, cx);
                    },
                    |_, cx| !ToastQueue::visible_ids(cx).is_empty(),
                ));
                s
            }
            // ---- 20
            GotoOpen | GotoTurkish | GotoOutOfRange | GotoUnknownColumn | GotoHidden => {
                let n = if self == GotoTurkish { 1_200 } else { 40 };
                let mut s = tile(EnvTag::Dev, move || vec![complete(orders(n))]);
                if self == GotoTurkish {
                    s.push(display("tr-TR"));
                }
                if self == GotoHidden {
                    s.push(instant(|window, cx| {
                        let g = grid(window, cx).expect("a grid");
                        g.update(cx, |g, cx| g.set_column_hidden(ColId(4), true, cx));
                    }));
                }
                let text = match self {
                    GotoOpen => "",
                    GotoTurkish => "1.000:şehir",
                    GotoOutOfRange => "40000",
                    GotoUnknownColumn => "12:ilce",
                    _ => "12:durum",
                };
                s.push(open_goto(text));
                s
            }
            GotoAsk | GotoFetching => {
                let env = if self == GotoAsk {
                    EnvTag::Production
                } else {
                    EnvTag::Dev
                };
                let mut s = tile(env, || {
                    vec![live_source(orders(500), Completeness::Suspended(cursor()))]
                });
                let target = if self == GotoAsk { "1500000" } else { "2500" };
                s.push(open_goto(target));
                s.push(step(
                    |window, cx| {
                        let c = active_console(window, cx).expect("a console");
                        goto::submit(c, cx);
                    },
                    move |window, cx| {
                        let Some(c) = active_console(window, cx) else {
                            return false;
                        };
                        match goto::state(c, cx) {
                            Some(goto::GotoState::Ask { .. }) => self == GotoAsk,
                            Some(goto::GotoState::Fetching { .. }) => self == GotoFetching,
                            _ => false,
                        }
                    },
                ));
                s
            }
            // ---- 21
            ViewTransposedOff => {
                let mut s = tile(EnvTag::Dev, || vec![complete(numbered(100_001))]);
                s.push(step(
                    |_, _| {},
                    |window, cx| {
                        active_console(window, cx)
                            .is_some_and(|c| views::transposed_reason(c, cx).is_some())
                    },
                ));
                s
            }
            ViewTransposed => {
                let mut s = tile(EnvTag::Dev, || vec![complete(orders(6))]);
                s.push(view_mode(ViewMode::Transposed));
                s
            }
            ViewRecord | ViewRecordDuplicates | ViewRecordMore => {
                let mut s = tile(EnvTag::Dev, move || match self {
                    ViewRecordDuplicates => vec![complete(duplicates(12))],
                    ViewRecordMore => vec![complete(long_notes(12))],
                    _ => vec![complete(orders(184))],
                });
                s.push(focus_cell(3, 1));
                s.push(step(views::toggle_record, |window, cx| {
                    views::record_open(window, cx)
                        && active_console(window, cx)
                            .is_some_and(|c| views::record(c, cx).is_some())
                }));
                s
            }
            ViewOneRow => tile(EnvTag::Dev, || vec![complete(orders(1))]),
            ViewText => {
                let mut s = tile(EnvTag::Dev, || vec![complete(orders(24))]);
                s.push(view_mode(ViewMode::Text));
                s.push(step(
                    |_, _| {},
                    |window, cx| {
                        active_console(window, cx)
                            .is_some_and(|c| views::text_state(c, cx) == Some(TextState::Ready))
                    },
                ));
                s
            }
            ViewTextAsk => {
                let mut s = tile(EnvTag::Dev, || vec![complete(numbered(100_001))]);
                s.push(view_mode(ViewMode::Text));
                s.push(step(
                    |_, _| {},
                    |window, cx| {
                        active_console(window, cx).is_some_and(|c| {
                            matches!(views::text_state(c, cx), Some(TextState::Asking { .. }))
                        })
                    },
                ));
                s
            }
            // ---- 22
            ViewerText | ViewerJsonPretty | ViewerJsonRaw | ViewerHex | ViewerXml
            | ViewerJsonInvalid | ViewerTruncated => {
                let env = if self == ViewerTruncated {
                    EnvTag::Production
                } else {
                    EnvTag::Dev
                };
                let mut s = tile(env, move || vec![complete(viewer_values())]);
                let col = match self {
                    ViewerText => 1,
                    ViewerJsonPretty => 2,
                    ViewerJsonRaw => 3,
                    ViewerHex => 4,
                    ViewerXml => 5,
                    ViewerJsonInvalid => 6,
                    _ => 7,
                };
                s.push(open_viewer(1, col));
                if matches!(self, ViewerJsonPretty | ViewerXml) {
                    s.push(step(
                        |window, cx| {
                            let c = active_console(window, cx).expect("a console");
                            value_viewer::set_pretty(c, true, cx);
                        },
                        |window, cx| {
                            active_console(window, cx)
                                .and_then(|c| value_viewer::view(c, cx))
                                .is_some_and(|v| v.pretty && !v.body.is_empty())
                        },
                    ));
                }
                s
            }
        }
    }

    /// What the app's models (and the frame) must say once the frame is drawn (a violation
    /// otherwise): the state check that keeps a marker from passing on an unrelated label.
    pub(super) fn check(
        self,
        window: &mut Window,
        cx: &App,
        frame: &[RenderedText],
        _decls: &[A11yDecl],
    ) -> Option<String> {
        use M3c::*;
        let editor = editor_in(window, cx);
        let status = editor.as_ref().map(|e| e.read(cx).status());
        let carets = status.map_or(0, |s| s.carets);
        let band = || editor.as_ref().and_then(|e| preview::current(e.read(cx)));
        let console = active_console(window, cx);
        let g = grid(window, cx);
        let said = |id: &str, args| shows_label(frame, "#status-message", &message(cx, id, args));
        let find_view = || console.and_then(|c| find::status(c, cx));
        let folded = || {
            editor
                .as_ref()
                .map_or(0, |e| e.read(cx).folds().folded().len())
        };
        let plays = || PLAYS.with(|p| *p.borrow());
        let ok = match self {
            MultiNext => carets == 3 && selected_words(&editor, cx) == 3,
            UndoSelection => carets == 2 && selected_words(&editor, cx) == 2,
            MultiColumn => carets == 4 && selected_words(&editor, cx) == 4,
            MultiImeMirrored => {
                status.is_some_and(|s| s.composing)
                    && carets == 3
                    && editor.as_ref().is_some_and(|e| {
                        e.read(cx)
                            .snapshot()
                            .buffer
                            .to_string()
                            .matches("shuju")
                            .count()
                            == 3
                    })
            }
            MultiImeMain => {
                status.is_some_and(|s| s.composing)
                    && carets == 1
                    && said("hv-editor-ime-primary-only", Vec::new())
            }
            MultiRunMain => {
                carets == 3
                    && band().is_some_and(|b| {
                        b.units.len() == 1 && b.units[0].start >= at(MULTI, "UPDATE siparisler")
                    })
            }
            FoldHover => folded() == 0 && CHEVRONS.with(|c| *c.borrow()) >= 3,
            FoldFocused => {
                folded() == 0
                    && editor.as_ref().is_some_and(|e| {
                        matches!(
                            e.read(cx).gutter_focus(window),
                            Some(hacivat_ui_editor::gutter::GutterButton::Fold(_))
                        )
                    })
            }
            FoldFolded => folded() == 2 && band().is_none_or(|b| b.style != BandStyle::Run),
            FoldRunCaret => {
                folded() == 1
                    && band().is_some_and(|b| b.style == BandStyle::Run && b.units.len() == 1)
            }
            FoldMarks | FoldFailed | FoldEscalated => {
                folded() == if self == FoldFailed { 1 } else { 3 }
                    && editor.as_ref().is_some_and(|e| {
                        let m = e.read(cx).marks();
                        let second = m.state(exec_ui::fixtures::fixture_key(1));
                        match self {
                            FoldFailed => {
                                matches!(second, Some(ExecMarkState::Failed { .. }))
                                    && said_in_marks(
                                        frame,
                                        &message(
                                            cx,
                                            "hv-editor-fold-failed",
                                            vec![("line", hacivat_l10n::ArgValue::Count(7))],
                                        ),
                                    )
                            }
                            FoldEscalated => {
                                matches!(second, Some(ExecMarkState::Escalated { .. }))
                            }
                            _ => m.len() == 3,
                        }
                    })
            }
            FindCount | FindTolerant | FindRegex | FindSelectAll => {
                let st = editor.as_ref().map(|e| e.read(cx).find_status());
                let opts = console.and_then(|c| find_bar::options(c, cx));
                console.is_some_and(|c| find_bar::is_open(c, cx))
                    && st.is_some_and(|s| s.total.is_some_and(|t| t > 1))
                    && match self {
                        FindCount => carets <= 1 && opts.is_some_and(|o| !o.regex && !o.tolerant),
                        FindTolerant => opts.is_some_and(|o| o.tolerant),
                        FindRegex => opts.is_some_and(|o| o.regex),
                        _ => carets > 1,
                    }
            }
            // "Invalid pattern" in the count's place; nothing searched.
            FindInvalid => {
                console.is_some_and(|c| find_bar::error(c, cx).is_some())
                    && shows_label(
                        frame,
                        "#find-count-text",
                        &message(cx, "hv-find-invalid", Vec::new()),
                    )
            }
            FindNone => {
                editor
                    .as_ref()
                    .is_some_and(|e| e.read(cx).find_status().total == Some(0))
                    && shows_label(
                        frame,
                        "#find-count-text",
                        &message(cx, "hv-find-none", Vec::new()),
                    )
            }
            FindCapped => carets == 10_000 && drawn(frame, "status-message"),
            GutterHover => plays() >= 1,
            // Fix round 1 (m9): with the run's fixed time in the tooltip's row.
            GutterLastRuns => {
                plays() >= 1
                    && tooltip_open(cx)
                    && shows_label(
                        frame,
                        "hacivat-ui-editor/src/gutter.rs:",
                        &message(cx, "hv-history-time", vec![("t", run_time())]),
                    )
            }
            GutterFocused => {
                plays() >= 1
                    && editor.as_ref().is_some_and(|e| {
                        matches!(
                            e.read(cx).gutter_focus(window),
                            Some(hacivat_ui_editor::gutter::GutterButton::Run(_))
                        )
                    })
            }
            GutterNoRun => {
                editor.as_ref().is_some_and(|e| {
                    matches!(
                        e.read(cx).marks().state(exec_ui::fixtures::fixture_key(1)),
                        Some(ExecMarkState::Escalated { .. })
                    )
                }) && HOVERED.with(|h| *h.borrow())
                    && plays() == 0
            }
            EolMixed => status.is_some_and(|s| s.mixed_line_endings) && !menu_open(window, cx),
            EolMenu => status.is_some_and(|s| s.mixed_line_endings) && menu_open(window, cx),
            HistoryRecall => {
                said_any(frame)
                    && editor
                        .as_ref()
                        .is_some_and(|e| e.read(cx).snapshot().buffer.len_bytes() > 0)
            }
            MarkAwaiting | MarkRolledBack => editor.as_ref().is_some_and(|e| {
                let want = |s: &ExecMarkState| match self {
                    MarkAwaiting => matches!(s, ExecMarkState::Awaiting),
                    _ => matches!(s, ExecMarkState::RolledBack),
                };
                e.read(cx)
                    .marks()
                    .state(exec_ui::fixtures::fixture_key(1))
                    .is_some_and(want)
            }),
            // One key on the text column, its tooltip drawn, no job running.
            SortCodePoint => {
                header_tip_open(cx)
                    && g.as_ref().is_some_and(|g| {
                        sort::status(g, cx).is_some_and(|s| {
                            s.shown.keys.len() == 1
                                && s.shown.keys[0].col == ColId(1)
                                && s.running.is_none()
                        })
                    })
            }
            SortOne | SortTwo => g.as_ref().is_some_and(|g| {
                let n = if self == SortOne { 1 } else { 2 };
                sort::status(g, cx).is_some_and(|s| s.shown.keys.len() == n && s.running.is_none())
            }),
            SortPartial | SortPartialTail | SortTail | SortRunning => g.as_ref().is_some_and(|g| {
                let tail = g.read(cx).tail_boundary().is_some();
                let st = sort::status(g, cx);
                match self {
                    SortRunning => st.is_some_and(|s| s.slow),
                    SortPartial => !tail && st.is_some_and(|s| s.sorted.is_some()),
                    _ => tail,
                }
            }),
            GridFrozen => g.as_ref().is_some_and(|g| {
                g.read(cx).frozen() == 2 && g.read(cx).scroll_state().left_x > 0.0
            }),
            GridMultiRect => g
                .as_ref()
                .is_some_and(|g| g.read(cx).selection().ranges() == 2),
            GridResize => g
                .as_ref()
                .is_some_and(|g| g.read(cx).resize_tag().is_some()),
            GfindCount | GfindSelectAll | GfindFilter | GfindInvalid | GfindIncomplete
            | GfindCapped => {
                let v = find_view();
                console.is_some_and(|c| find::is_open(c, cx))
                    && match self {
                        GfindCount => v.is_some_and(|v| {
                            v.total.is_some_and(|t| t > 1) && v.filtered.is_none()
                        }),
                        GfindFilter => v.is_some_and(|v| v.filtered.is_some()),
                        GfindInvalid => {
                            v.is_some_and(|v| v.invalid)
                                && shows_label(
                                    frame,
                                    "#grid-find-count-text",
                                    &message(cx, "hv-find-invalid", Vec::new()),
                                )
                        }
                        GfindIncomplete => v.is_some_and(|v| v.incomplete > 0),
                        GfindSelectAll => g
                            .as_ref()
                            .is_some_and(|g| g.read(cx).selection().ranges() > 1),
                        _ => g
                            .as_ref()
                            .is_some_and(|g| g.read(cx).selection().ranges() == 10_000),
                    }
            }
            GotoOpen | GotoTurkish | GotoOutOfRange | GotoUnknownColumn | GotoHidden => console
                .is_some_and(|c| {
                    goto::is_open(c, cx)
                        && matches!(
                            (self, goto::line(c, cx)),
                            (GotoOpen, Some(goto::GotoLine::Hint(_)))
                                | (GotoTurkish, Some(goto::GotoLine::Target(_)))
                                | (
                                    GotoOutOfRange | GotoUnknownColumn,
                                    Some(goto::GotoLine::Error(_))
                                )
                                | (GotoHidden, Some(goto::GotoLine::Hidden(..)))
                        )
                }),
            GotoAsk => console
                .is_some_and(|c| matches!(goto::state(c, cx), Some(goto::GotoState::Ask { .. }))),
            GotoFetching => console.is_some_and(|c| {
                matches!(goto::state(c, cx), Some(goto::GotoState::Fetching { .. }))
            }),
            ViewTransposedOff => console.is_some_and(|c| {
                views::mode(c, cx) == ViewMode::Table && views::transposed_reason(c, cx).is_some()
            }),
            ViewTransposed => console.is_some_and(|c| views::mode(c, cx) == ViewMode::Transposed),
            ViewRecord | ViewRecordDuplicates | ViewRecordMore => {
                views::record_open(window, cx)
                    && console
                        .and_then(|c| views::record(c, cx))
                        .is_some_and(|r| match self {
                            ViewRecordDuplicates => r.fields.iter().any(|f| f.dup.is_some()),
                            ViewRecordMore => r.fields.iter().any(|f| f.cut),
                            _ => r.fields.iter().all(|f| f.dup.is_none()),
                        })
            }
            ViewOneRow => {
                !views::record_open(window, cx)
                    && shows_label(frame, VIEWS, &message(cx, "hv-record", Vec::new()))
            }
            ViewText => console.is_some_and(|c| {
                views::mode(c, cx) == ViewMode::Text
                    && views::text_state(c, cx) == Some(TextState::Ready)
            }),
            ViewTextAsk => console.is_some_and(|c| {
                matches!(views::text_state(c, cx), Some(TextState::Asking { .. }))
            }),
            ViewerText | ViewerJsonPretty | ViewerJsonRaw | ViewerHex | ViewerXml
            | ViewerJsonInvalid | ViewerTruncated => console
                .and_then(|c| value_viewer::view(c, cx))
                .is_some_and(|v| {
                    use value_viewer::ValueForm as F;
                    !v.body.is_empty()
                        && match self {
                            ViewerText => v.form == F::Text,
                            ViewerJsonPretty => v.form == F::Json { valid: true } && v.pretty,
                            ViewerJsonRaw => v.form == F::Json { valid: true } && !v.pretty,
                            ViewerHex => v.form == F::Binary,
                            ViewerXml => v.form == F::Xml && v.pretty,
                            ViewerJsonInvalid => v.form == F::Json { valid: false } && !v.pretty,
                            _ => v.truncated.is_some(),
                        }
                }),
        };
        (!ok).then(|| {
            format!(
                "{}: the state check failed (carets {carets}, ▶ {}, chevrons {}, folded {})",
                self.name(),
                plays(),
                CHEVRONS.with(|c| *c.borrow()),
                folded()
            )
        })
    }
}

// ---- checks' helpers ------------------------------------------------------------------------

thread_local! {
    /// The ▶ icons the last frame drew (recorded by [`record_icons`]).
    static PLAYS: RefCell<usize> = const { RefCell::new(0) };
    /// The fold chevrons the last frame drew.
    static CHEVRONS: RefCell<usize> = const { RefCell::new(0) };
    /// The pointer was moved over the gutter.
    static HOVERED: RefCell<bool> = const { RefCell::new(false) };
    /// The writer of the last live fixture store (rows that arrive after a sort).
    static WRITER: RefCell<Option<ResultWriter>> = const { RefCell::new(None) };
    /// The shown grid's body origin (window x, y), found by [`probe_grid`].
    static BODY: RefCell<Option<(f32, f32)>> = const { RefCell::new(None) };
    /// The probe's subscription, until it answered.
    static PROBE: RefCell<Option<hacivat_ui_core::gpui_surface::Subscription>> =
        const { RefCell::new(None) };
}

/// Counts the ▶ and chevron icons the harness's frame drew (called by `shots.rs` around the
/// frame: [`record_icons`] before, [`take_icons`] after). The active editor is notified first:
/// a cached view that did not change replays its last paint without building its elements
/// again, and would record no icon.
pub(super) fn record_icons(window: &mut Window, cx: &mut App) {
    PLAYS.with(|p| *p.borrow_mut() = 0);
    CHEVRONS.with(|p| *p.borrow_mut() = 0);
    if let Some(editor) = editor_in(window, cx) {
        editor.update(cx, |_, cx| cx.notify());
    }
    hacivat_ui::icon::icon_record();
}

/// Takes the icons [`record_icons`] recorded.
pub(super) fn take_icons() {
    use hacivat_ui::icon::IconName;
    let icons = hacivat_ui::icon::icon_take();
    PLAYS.with(|p| *p.borrow_mut() = icons.iter().filter(|i| i.name == IconName::Play).count());
    CHEVRONS.with(|p| {
        *p.borrow_mut() = icons
            .iter()
            .filter(|i| matches!(i.name, IconName::ChevronDown | IconName::ChevronRight))
            .count()
    });
}

/// Resets the per-shot state (every shot starts clean).
pub(super) fn reset() {
    HOVERED.with(|h| *h.borrow_mut() = false);
    WRITER.with(|w| *w.borrow_mut() = None);
    BODY.with(|b| *b.borrow_mut() = None);
    PROBE.with(|p| *p.borrow_mut() = None);
    PLAYS.with(|p| *p.borrow_mut() = 0);
    CHEVRONS.with(|p| *p.borrow_mut() = 0);
}

/// How many selection ranges of the active editor are not empty.
fn selected_words(editor: &Option<Entity<EditorView>>, cx: &App) -> usize {
    editor.as_ref().map_or(0, |e| {
        e.read(cx)
            .snapshot()
            .selection
            .ranges()
            .iter()
            .filter(|r| !r.is_empty())
            .count()
    })
}

/// Whether the status bar shows a transient sentence.
fn said_any(frame: &[RenderedText]) -> bool {
    drawn(frame, "status-message")
}

/// Whether a mark's end-of-line status reads `sentence` ("failed inside the fold · line 7").
fn said_in_marks(frame: &[RenderedText], sentence: &str) -> bool {
    shows_label(frame, MARKS_RS, sentence)
}

/// The fixed wall-clock time `gutter-last-runs` shows for its run (14:05:09).
fn run_time() -> hacivat_l10n::ArgValue {
    hacivat_l10n::ArgValue::When(hacivat_l10n::WhenValue::Local {
        year: 2026,
        month: 11,
        day: 17,
        hour: 14,
        minute: 5,
        second: 9,
    })
}

/// Whether the grid's header tooltip is drawn (a label of the grid view's own file).
fn header_tip_open(cx: &App) -> bool {
    cx.global::<hacivat_ui::text::TextRegistry>()
        .frame
        .iter()
        .any(|r| r.visible && r.painted_px > 0.0 && r.path.contains(GRID_VIEW))
}

/// Moves the pointer onto the middle of the shown grid's header for column `col` (its tooltip
/// area: a sorted, frozen or ellipsized header has one).
fn hover_header(col: u32) -> Step {
    use gpui::InputEvent as _;
    step(
        move |window, cx| {
            let Some(at) = grid(window, cx).and_then(|g| {
                g.read(cx)
                    .header_tips()
                    .iter()
                    .find(|t| t.col == ColId(col))
                    .map(|t| t.bounds.center())
            }) else {
                return;
            };
            window.dispatch_event(
                gpui::MouseMoveEvent {
                    position: at,
                    pressed_button: None,
                    modifiers: gpui::Modifiers::none(),
                }
                .to_platform_input(),
                cx,
            );
        },
        move |window, cx| {
            grid(window, cx)
                .is_some_and(|g| g.read(cx).header_tips().iter().any(|t| t.col == ColId(col)))
        },
    )
}

/// Whether the ▶ tooltip's "Last runs" caption is drawn.
fn tooltip_open(cx: &App) -> bool {
    let caption = message(cx, "hv-gutter-last-runs", Vec::new());
    cx.global::<hacivat_ui::text::TextRegistry>()
        .frame
        .iter()
        .any(|r| r.visible && super::strip(&r.text) == caption)
}

// ---- steps ----------------------------------------------------------------------------------

/// Installs an in-memory find pasteboard holding `seed` (Task 33, M10): the shots never read or
/// write the machine's.
fn seed_find_pasteboard(seed: Option<&'static str>) -> Step {
    instant(move |_, cx| {
        reset();
        find_bar::install_find_pasteboard(
            cx,
            Rc::new(move |_: &App| seed.map(str::to_string)),
            Rc::new(|_: &str, _: &App| {}),
        );
    })
}

/// Runs the editor command `id` in the active console.
fn command(id: &'static str) -> Step {
    instant(move |window, cx| {
        let editor = editor_in(window, cx).expect("a console");
        editor.update(cx, |e, cx| assert!(e.run_command(id, window, cx), "{id}"));
    })
}

/// Waits for `n` selection ranges.
fn carets(n: u32) -> Step {
    step(
        |_, _| {},
        move |window, cx| editor_in(window, cx).is_some_and(|e| e.read(cx).status().carets == n),
    )
}

/// Waits for the band to show `style`.
fn band(style: BandStyle) -> Step {
    step(
        |_, _| {},
        move |window, cx| {
            editor_in(window, cx)
                .is_some_and(|e| preview::current(e.read(cx)).is_some_and(|b| b.style == style))
        },
    )
}

/// Makes `ranges` (anchor, head) the selection; the last one is the main caret.
fn select(ranges: Vec<(usize, usize)>) -> Step {
    instant(move |window, cx| {
        let editor = editor_in(window, cx).expect("a console");
        let main = ranges.len() - 1;
        let set = SelectionSet::normalized(
            ranges.iter().map(|(a, h)| SelectionRange::new(*a, *h)),
            main,
        );
        editor.update(cx, |e, cx| e.select_ranges(set, cx));
        let handle = editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    })
}

/// The codes of [`COLUMN`]'s four value rows, as a column selection.
fn column_ranges() -> Vec<(usize, usize)> {
    ["(34", "(35", "(06", "(16"]
        .iter()
        .map(|n| {
            let a = at(COLUMN, n) + 1;
            (a, a + 2)
        })
        .collect()
}

/// Waits until the folding ranges landed (the worker computes them once per version).
fn folds_ready() -> Step {
    step(
        |_, _| {},
        |window, cx| {
            // A fold and its undo: possible only once the ranges are known.
            let Some(editor) = editor_in(window, cx) else {
                return false;
            };
            let folded = editor.update(cx, |e, cx| e.toggle_fold_at(1, cx));
            if folded {
                editor.update(cx, |e, cx| e.toggle_fold_at(1, cx));
            }
            folded
        },
    )
}

/// Folds the statements whose headers are on `lines` (retrying until the ranges landed).
fn fold(lines: &'static [u32]) -> Step {
    step(
        |_, _| {},
        move |window, cx| {
            let Some(editor) = editor_in(window, cx) else {
                return false;
            };
            for &line in lines {
                let buf = editor.read(cx).snapshot().buffer;
                if editor.read(cx).folds().fold_on_line(line, &buf).is_none() {
                    editor.update(cx, |e, cx| e.toggle_fold_at(line, cx));
                }
            }
            editor.read(cx).folds().folded().len() == lines.len()
        },
    )
}

/// Places fixture marks over `(statement, status)` of `text` (the active console's).
fn show_marks(text: &'static str, units: Vec<(&'static str, (UnitStatus, Duration))>) -> Step {
    let n = units.len();
    let units = RefCell::new(Some(units));
    instant(move |window, cx| {
        let console = active_console(window, cx).expect("a console");
        if let Some(units) = units.borrow_mut().take() {
            let placed = exec_ui::fixtures::show_marks(
                console,
                units
                    .into_iter()
                    .map(|(stmt, (s, d))| {
                        let start = at(text, stmt);
                        (start..start + stmt.len(), s, d)
                    })
                    .collect(),
                cx,
            );
            assert_eq!(placed, n, "every mark placed");
        }
    })
}

/// A failed unit with a server error.
fn failed(code: &str, server: &str) -> (UnitStatus, Duration) {
    (
        UnitStatus::Failed(ErrorView {
            title_id: "exec-error-server",
            code: Some(code.into()),
            server_text: Some(server.into()),
            hint_id: None,
            doc_offset: None,
            vendor_code: None,
        }),
        Duration::from_millis(9),
    )
}

/// Which gutter column the pointer hovers.
#[derive(Debug, Clone, Copy)]
enum Gutter {
    /// The mark column (▶).
    Run,
    /// The fold column (chevrons).
    Fold,
}

/// Moves the pointer over `line`'s row in the gutter's `column` (the editor's text area is right
/// of a 58 px gutter for three-digit line numbers; the fold column is its last 14 px).
fn hover_gutter(column: Gutter, line: u32) -> Step {
    use gpui::InputEvent as _;
    step(
        move |window, cx| {
            let editor = editor_in(window, cx).expect("a console");
            let Some((x, y)) = editor.read(cx).last_layout().map(|l| {
                let rows = l.rows();
                let left = f32::from(rows.bounds.left());
                let x = match column {
                    Gutter::Run => left - 58.0 + 14.0,
                    Gutter::Fold => left - 7.0,
                };
                let lh = f32::from(rows.line_height);
                let y = f32::from(rows.first_y) + (line - rows.first_line) as f32 * lh + lh / 2.0;
                (x, y)
            }) else {
                return;
            };
            window.dispatch_event(
                gpui::MouseMoveEvent {
                    position: point(px(x), px(y)),
                    pressed_button: None,
                    modifiers: gpui::Modifiers::none(),
                }
                .to_platform_input(),
                cx,
            );
            HOVERED.with(|h| *h.borrow_mut() = true);
        },
        |window, cx| editor_in(window, cx).is_some_and(|e| e.read(cx).last_layout().is_some()),
    )
}

/// Which gutter buttons Tab walks to.
#[derive(Debug, Clone, Copy, PartialEq)]
enum GutterStop {
    Run,
    Fold,
}

/// From the editor, Tab ("Tab moves focus" on) until a gutter button of `stop` has the focus.
fn gutter_tab_to(stop: GutterStop) -> Step {
    use hacivat_ui_editor::gutter::GutterButton;
    step(
        |window, cx| {
            let editor = editor_in(window, cx).expect("a console");
            editor.update(cx, |e, cx| {
                e.run_command("editor.tab", window, cx);
            });
        },
        move |window, cx| {
            let Some(editor) = editor_in(window, cx) else {
                return false;
            };
            match editor.read(cx).gutter_focus(window) {
                Some(GutterButton::Run(_)) if stop == GutterStop::Run => true,
                Some(GutterButton::Fold(_)) if stop == GutterStop::Fold => true,
                _ => {
                    window.focus_next(cx);
                    false
                }
            }
        },
    )
}

/// Opens the editor find bar over the active console with `query`, flipping the toggles at
/// `flips` (0 Match case, 1 Whole words, 2 Regular expression, 3 Turkish-tolerant); with
/// `tolerant`, Turkish-tolerant must end on (it is on by default in Turkish). Waits for the
/// count.
fn open_find(query: &'static str, flips: &'static [usize], tolerant: bool) -> Step {
    step(
        move |window, cx| {
            let editor = editor_in(window, cx).expect("a console");
            editor.update(cx, |e, cx| {
                assert!(e.run_command("editor.find", window, cx))
            });
            let c = active_console(window, cx).expect("a console");
            find_bar::set_query(c, query, window, cx);
            for &i in flips {
                find_bar::flip_toggle(c, i, window, cx);
            }
            let on = find_bar::options(c, cx).is_some_and(|o| o.tolerant);
            if on != tolerant {
                find_bar::flip_toggle(c, 3, window, cx);
            }
        },
        |window, cx| {
            let Some(c) = active_console(window, cx) else {
                return false;
            };
            if find_bar::error(c, cx).is_some() {
                return true;
            }
            editor_in(window, cx).is_some_and(|e| e.read(cx).find_status().total.is_some())
        },
    )
}

/// Runs a find-bar command (Select all occurrences) as the bar's chord would.
fn run_in_find(id: &str, window: &mut Window, cx: &mut App) {
    if !find_bar::run_in_bar(id, window, cx) {
        let editor = editor_in(window, cx).expect("a console");
        editor.update(cx, |e, cx| assert!(e.run_command(id, window, cx), "{id}"));
    }
}

// ---- the result tile ------------------------------------------------------------------------

/// A result tile on a console of a saved profile in `env` (Task 30's fixture console), whose
/// results then show `tabs()` (this task's stores), the first active.
fn tile(env: EnvTag, tabs: impl Fn() -> Vec<Arc<dyn ResultSource>> + 'static) -> Vec<Step> {
    let mut s = m3a::results(ResultsShot::Rows, Some(env));
    s.push(step(
        move |window, cx| {
            let c = active_console(window, cx).expect("a console");
            let results = Consoles::get(cx, c)
                .and_then(|s| s.results.clone())
                .expect("the console's results");
            let tabs: Vec<ResultTab> = tabs()
                .into_iter()
                .enumerate()
                .map(|(i, source)| result_tab(i as u32 + 1, source))
                .collect();
            results.update(cx, |r, cx| {
                r.apply(&tabs, cx);
                r.set_active(ActiveResult::Tab(0), cx);
            });
        },
        |window, cx| grid(window, cx).is_some(),
    ));
    s
}

/// Sets the result display's format locale (go to row reads its separators).
fn display(locale: &'static str) -> Step {
    instant(move |window, cx| {
        let c = active_console(window, cx).expect("a console");
        let results = Consoles::get(cx, c)
            .and_then(|s| s.results.clone())
            .expect("the console's results");
        results.update(cx, |r, cx| {
            r.set_display(
                ResultDisplay::new(locale, DisplayZone::utc(), DbmsKind::Postgres),
                cx,
            )
        });
    })
}

/// The shown grid of the active console's first tab.
fn grid(window: &mut Window, cx: &App) -> Option<Entity<GridView>> {
    let c = active_console(window, cx)?;
    hosts(cx)
        .into_iter()
        .find_map(|h| h.read(cx).grid_of(c, 0, cx))
}

/// Requests `spec` on the shown grid and waits for it to show.
fn sort_by(spec: SortSpec) -> Step {
    let want = spec.keys.len();
    step(
        move |window, cx| {
            let g = grid(window, cx).expect("a grid");
            g.update(cx, |g, cx| g.request_sort(spec.clone(), cx));
        },
        move |window, cx| {
            grid(window, cx)
                .and_then(|g| sort::status(&g, cx))
                .is_some_and(|s| s.shown.keys.len() == want && s.running.is_none())
        },
    )
}

/// Focuses the shown grid's cell at model `(row, col)`.
fn focus_cell(row: u64, col: u32) -> Step {
    step(
        move |window, cx| {
            let g = grid(window, cx).expect("a grid");
            g.update(cx, |g, cx| g.focus_cell(cell(row, col), cx));
            let handle = g.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        },
        move |window, cx| {
            grid(window, cx).is_some_and(|g| g.read(cx).selection().focus() == Some(cell(row, col)))
        },
    )
}

fn cell(row: u64, col: u32) -> ModelCell {
    ModelCell {
        row: ModelRow(row),
        col: ColId(col),
    }
}

/// Switches the active result's view to `mode`.
fn view_mode(mode: ViewMode) -> Step {
    step(
        move |window, cx| {
            let c = active_console(window, cx).expect("a console");
            assert!(views::set_mode(c, mode, cx), "{mode:?}");
        },
        move |window, cx| active_console(window, cx).is_some_and(|c| views::mode(c, cx) == mode),
    )
}

/// Opens the grid find bar with `query` (Regular expression and Filter rows as asked); waits
/// for the scan to finish (or the pattern to be refused).
fn grid_find(query: &'static str, regex: bool, filter: bool) -> Step {
    step(
        move |window, cx| {
            let g = grid(window, cx).expect("a grid");
            let handle = g.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
            let c = active_console(window, cx).expect("a console");
            find::open(c, cx);
            if regex {
                find::flip(c, find::Toggle::Regex, cx);
            }
            if filter {
                find::flip(c, find::Toggle::Filter, cx);
            }
            find::set_query(c, query, cx);
        },
        move |window, cx| {
            let Some(c) = active_console(window, cx) else {
                return false;
            };
            find::status(c, cx).is_some_and(|v| {
                v.invalid || (v.total.is_some() && (!filter || v.filtered.is_some()))
            })
        },
    )
}

/// Opens the go-to popover with `text` in its field.
fn open_goto(text: &'static str) -> Step {
    step(
        move |window, cx| {
            let g = grid(window, cx).expect("a grid");
            let handle = g.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
            let c = active_console(window, cx).expect("a console");
            goto::open(c, cx);
            if !text.is_empty() {
                goto::set_text(c, text, cx);
            }
        },
        |window, cx| active_console(window, cx).is_some_and(|c| goto::is_open(c, cx)),
    )
}

/// Opens the value viewer on model `(row, col)` and waits for its body.
fn open_viewer(row: u64, col: u32) -> Step {
    step(
        move |window, cx| {
            let c = active_console(window, cx).expect("a console");
            value_viewer::open(c, cell(row, col), cx);
        },
        |window, cx| {
            active_console(window, cx)
                .and_then(|c| value_viewer::view(c, cx))
                .is_some_and(|v| !v.body.is_empty())
        },
    )
}

/// Finds where the shown grid's body starts on screen (its first cell's top-left corner, kept
/// for [`cell_rect`]): the grid answers its keyboard context menu (`grid.context_menu`) under
/// the focus cell's bottom-left corner, which places the body without reaching into the grid's
/// private geometry. The answer is an event, delivered once the update ends: the step waits.
// ## m3c-t43-part-b: Task 39's menu answers that event; part B closes the menu it opens.
fn probe_grid() -> Step {
    step(
        |window, cx| {
            let g = grid(window, cx).expect("a grid");
            let sub = cx.subscribe(&g, |g, e: &hacivat_ui_grid::GridEvent, cx| {
                if let hacivat_ui_grid::GridEvent::ContextMenuRequested { at, .. } = e {
                    let h = g.read(cx).scroll_state().row_height();
                    let origin = (f32::from(at.x), f32::from(at.y) - h);
                    BODY.with(|b| *b.borrow_mut() = Some(origin));
                }
            });
            PROBE.with(|p| *p.borrow_mut() = Some(sub));
            g.update(cx, |g, cx| {
                g.focus_cell(cell(0, 0), cx);
                g.dispatch(hacivat_ui_grid::GridCommand::ContextMenu, window, cx);
            });
        },
        |_, _| {
            let done = BODY.with(|b| b.borrow().is_some());
            if done {
                PROBE.with(|p| p.borrow_mut().take());
            }
            done
        },
    )
}

/// The window rectangle `(left, top, width, height)` of the shown grid's cell at model
/// `(row, col)` (identity order, nothing scrolled), after [`probe_grid`].
fn cell_rect(
    window: &mut Window,
    cx: &mut App,
    row: u64,
    col: u32,
) -> Option<(f32, f32, f32, f32)> {
    let (left, top) = BODY.with(|b| *b.borrow())?;
    let g = grid(window, cx)?;
    let grid = g.read(cx);
    let h = grid.scroll_state().row_height();
    let layout = grid.column_layout();
    let x = left + (layout.x_of(col as usize) - layout.x_of(0)) as f32;
    Some((x, top + h * row as f32, layout.width_at(col as usize), h))
}

/// Presses and drags over the shown grid from cell `from` (model row, column) to `to` (with
/// ⌘/Ctrl: a new range), through the window's pointer events.
fn drag_cells(window: &mut Window, cx: &mut App, from: (u64, u32), to: (u64, u32), add: bool) {
    use gpui::InputEvent as _;
    let centre = |r: (f32, f32, f32, f32)| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
    let (Some(a), Some(b)) = (
        cell_rect(window, cx, from.0, from.1).map(centre),
        cell_rect(window, cx, to.0, to.1).map(centre),
    ) else {
        return;
    };
    let modifiers = if add {
        gpui::Modifiers::secondary_key(cx)
    } else {
        gpui::Modifiers::none()
    };
    window.dispatch_event(
        gpui::MouseDownEvent {
            button: gpui::MouseButton::Left,
            position: point(px(a.0), px(a.1)),
            modifiers,
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        gpui::MouseMoveEvent {
            position: point(px(b.0), px(b.1)),
            pressed_button: Some(gpui::MouseButton::Left),
            modifiers,
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        gpui::MouseUpEvent {
            button: gpui::MouseButton::Left,
            position: point(px(b.0), px(b.1)),
            modifiers,
            click_count: 1,
        }
        .to_platform_input(),
        cx,
    );
}

/// Moves the pointer onto the right edge of the shown grid's column `col`, in the header (just
/// above the first row).
fn hover_column_edge(window: &mut Window, cx: &mut App, col: u32) {
    use gpui::InputEvent as _;
    let Some((x, y, w, _)) = cell_rect(window, cx, 0, col) else {
        return;
    };
    window.dispatch_event(
        gpui::MouseMoveEvent {
            position: point(px(x + w - 1.0), px(y - 10.0)),
            pressed_button: None,
            modifiers: gpui::Modifiers::none(),
        }
        .to_platform_input(),
        cx,
    );
}

// ---- fixture stores -------------------------------------------------------------------------

/// Nobody listens: the fixtures' cursors are never fetched.
struct NoFetch;

impl FetchControl for NoFetch {
    fn fetch_more(&self, _: ResultId, _: u64) {}
    fn close(&self, _: ResultId, _: CloseReason) {}
    fn fetch_all(&self, _: ResultId) {}
}

fn next_id() -> ResultId {
    static NEXT: AtomicU64 = AtomicU64::new(1 << 43);
    ResultId(NEXT.fetch_add(1, Ordering::Relaxed))
}

/// A store of `data` in state `state`; its writer is kept for rows that arrive later.
fn live_source(
    data: (Arc<ResultSchema>, Vec<Vec<Value>>),
    state: Completeness,
) -> Arc<dyn ResultSource> {
    let (schema, rows) = data;
    let budget = MemoryBudget::new(u64::MAX, u64::MAX);
    let (store, mut writer) = ResultStore::new(next_id(), schema.clone(), budget, 1);
    if !rows.is_empty() {
        writer.append(Chunk::from_values(&schema, rows));
    }
    writer.set_state(state);
    WRITER.with(|w| *w.borrow_mut() = Some(writer));
    StoreSource::new(store, Arc::new(NoFetch), FetchPolicy::default())
}

/// A complete store of `data`.
fn complete(data: (Arc<ResultSchema>, Vec<Vec<Value>>)) -> Arc<dyn ResultSource> {
    live_source(data, Completeness::Complete)
}

fn result_tab(ordinal: u32, source: Arc<dyn ResultSource>) -> ResultTab {
    ResultTab {
        key: TabKey {
            console: 0,
            unit_ordinal: ordinal,
            result_ordinal: 0,
        },
        title: "".into(),
        unit: UnitId {
            run: RunId(1),
            ordinal,
        },
        source,
    }
}

fn cursor() -> CursorInfo {
    let now = Instant::now();
    CursorInfo {
        opened_at: now,
        last_fetch: now,
        holds_tx: true,
        idle_bound: Duration::from_secs(600),
    }
}

fn pg(oid: u32, name: &str) -> NativeType {
    NativeType::Pg {
        oid,
        typmod: -1,
        name: name.into(),
    }
}

fn int8() -> LogicalType {
    LogicalType::Int {
        bits: 64,
        signed: true,
    }
}

fn text() -> LogicalType {
    LogicalType::Text {
        max_chars: None,
        fixed: false,
    }
}

fn column(name: &str, logical: LogicalType, native: NativeType) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        logical,
        native,
        origin: None,
    }
}

const CITIES: [&str; 6] = ["İzmir", "İstanbul", "Ankara", "Bursa", "Şanlıurfa", "Iğdır"];
const NAMES: [&str; 5] = [
    "Çağla Öztürk",
    "İsmail Işık",
    "Ayşe Gül",
    "Ümit Şahin",
    "Deniz Aydın",
];
const STATES: [&str; 3] = ["kargoda", "teslim edildi", "iptal"];

/// `id`, `şehir`, `müşteri`, `tutar`, `durum`: `n` rows from `from`.
fn orders_from(from: u64, n: u64) -> (Arc<ResultSchema>, Vec<Vec<Value>>) {
    let schema = Arc::new(ResultSchema {
        columns: Arc::from([
            column("id", int8(), pg(20, "int8")),
            column("şehir", text(), pg(25, "text")),
            column("müşteri", text(), pg(25, "text")),
            column("tutar", int8(), pg(20, "int8")),
            column("durum", text(), pg(25, "text")),
        ]),
    });
    let rows = (from..from + n)
        .map(|r| {
            vec![
                Value::Int(1_000 + r as i64),
                Value::Text(CITIES[(r % 6) as usize].into()),
                Value::Text(NAMES[(r % 5) as usize].into()),
                Value::Int(((r * 37) % 1_000 + 10) as i64),
                Value::Text(STATES[(r % 3) as usize].into()),
            ]
        })
        .collect();
    (schema, rows)
}

fn orders(n: u64) -> (Arc<ResultSchema>, Vec<Vec<Value>>) {
    orders_from(0, n)
}

/// `n` rows of `(i, "r{i}")`.
fn numbered(n: u64) -> (Arc<ResultSchema>, Vec<Vec<Value>>) {
    let schema = Arc::new(ResultSchema {
        columns: Arc::from([
            column("sıra", int8(), pg(20, "int8")),
            column("şehir", text(), pg(25, "text")),
        ]),
    });
    let rows = (0..n)
        .map(|i| {
            vec![
                Value::Int(i as i64),
                Value::Text(CITIES[(i % 6) as usize].into()),
            ]
        })
        .collect();
    (schema, rows)
}

/// Twelve columns: the frozen state scrolls sideways under the first two.
fn wide(n: u64) -> (Arc<ResultSchema>, Vec<Vec<Value>>) {
    let (base, rows) = orders(n);
    let mut columns: Vec<ColumnMeta> = base.columns.iter().cloned().collect();
    for i in 0..20 {
        columns.push(column(&format!("ölçüm_{}", i + 1), int8(), pg(20, "int8")));
    }
    let rows = rows
        .into_iter()
        .enumerate()
        .map(|(r, mut row)| {
            row.extend((0..20).map(|i| Value::Int((r as i64 + 1) * (i + 3) * 11)));
            row
        })
        .collect();
    (
        Arc::new(ResultSchema {
            columns: columns.into(),
        }),
        rows,
    )
}

/// `id`, `adet`, `ad`, `adet` (a duplicate name).
fn duplicates(n: u64) -> (Arc<ResultSchema>, Vec<Vec<Value>>) {
    let schema = Arc::new(ResultSchema {
        columns: Arc::from([
            column("id", int8(), pg(20, "int8")),
            column("adet", int8(), pg(20, "int8")),
            column("ad", text(), pg(25, "text")),
            column("adet", int8(), pg(20, "int8")),
        ]),
    });
    let rows = (0..n)
        .map(|r| {
            vec![
                Value::Int(r as i64 + 1),
                Value::Int((r as i64 % 7) + 1),
                Value::Text(NAMES[(r % 5) as usize].into()),
                Value::Int((r as i64 % 4) + 2),
            ]
        })
        .collect();
    (schema, rows)
}

/// `not` (a long note, clamped in the inspector), `id`, `müşteri`.
fn long_notes(n: u64) -> (Arc<ResultSchema>, Vec<Vec<Value>>) {
    let schema = Arc::new(ResultSchema {
        columns: Arc::from([
            column("not", text(), pg(25, "text")),
            column("id", int8(), pg(20, "int8")),
            column("müşteri", text(), pg(25, "text")),
        ]),
    });
    let note = "Teslimat cuma günü öğleden sonra yapılacak; kapıda ödeme, fatura şirket \
                adına kesilsin. Müşteri İzmir şubesinden aranmak istiyor, Işıl Hanım'a \
                ulaşılamazsa santral üzerinden bağlanılsın. Paket kırılacak eşya içeriyor.";
    let rows = (0..n)
        .map(|r| {
            vec![
                Value::Text(note.into()),
                Value::Int(r as i64 + 1),
                Value::Text(NAMES[(r % 5) as usize].into()),
            ]
        })
        .collect();
    (schema, rows)
}

/// `id`, `not`: notes, one cut at the value limit (1 MB of 5 MB).
fn with_cut_note(n: u64) -> (Arc<ResultSchema>, Vec<Vec<Value>>) {
    let schema = Arc::new(ResultSchema {
        columns: Arc::from([
            column("id", int8(), pg(20, "int8")),
            column("not", text(), pg(25, "text")),
        ]),
    });
    let rows = (0..n)
        .map(|r| {
            let note = if r == 2 {
                Value::Truncated(Box::new(TruncatedVal {
                    prefix: Value::Text("teslimat notu ".repeat(1 << 16).into()),
                    total_len: Some(5 << 20),
                    unit: LenUnit::Bytes,
                }))
            } else if r % 3 == 0 {
                Value::Text("teslimat cuma günü".into())
            } else {
                Value::Text("kapıda ödeme".into())
            };
            vec![Value::Int(r as i64 + 1), note]
        })
        .collect();
    (schema, rows)
}

/// One column per value-viewer form (surface 22), three rows.
fn viewer_values() -> (Arc<ResultSchema>, Vec<Vec<Value>>) {
    let schema = Arc::new(ResultSchema {
        columns: Arc::from([
            column("id", int8(), pg(20, "int8")),
            column("not", text(), pg(25, "text")),
            column(
                "belge",
                LogicalType::Json { binary: true },
                pg(3802, "jsonb"),
            ),
            column(
                "ham_belge",
                LogicalType::Json { binary: true },
                pg(3802, "jsonb"),
            ),
            column("imza", LogicalType::Bytes { max: None }, pg(17, "bytea")),
            column("fatura_xml", LogicalType::Xml, pg(142, "xml")),
            column(
                "bozuk_belge",
                LogicalType::Json { binary: false },
                pg(114, "json"),
            ),
            column("ek", text(), pg(25, "text")),
        ]),
    });
    let note = "Teslimat cuma günü öğleden sonra yapılacak; kapıda ödeme, fatura şirket \
                adına kesilsin. Müşteri İzmir şubesinden aranmak istiyor, Işıl Hanım'a \
                ulaşılamazsa santral üzerinden bağlanılsın. Paket kırılacak eşya içeriyor; \
                kargo firmasına özellikle belirtilsin.";
    let json = r#"{"siparis":4211,"musteri":{"ad":"Çağla Öztürk","sehir":"İzmir"},"kalemler":[{"sku":"TR-1042","adet":2,"fiyat":149.9},{"sku":"TR-2210","adet":1,"fiyat":89.5}],"hediye":false,"not":null}"#;
    let xml = r#"<fatura no="2026-4211"><musteri sehir="İzmir">Çağla Öztürk</musteri><kalem sku="TR-1042" adet="2">149.90</kalem><kalem sku="TR-2210" adet="1">89.50</kalem><toplam para="TRY">389.30</toplam></fatura>"#;
    let bytes: Arc<[u8]> = Arc::from(
        (0u8..=95)
            .map(|b| b.wrapping_mul(37).wrapping_add(11))
            .collect::<Vec<u8>>(),
    );
    let cut = Value::Truncated(Box::new(TruncatedVal {
        prefix: Value::Text("ek dosya içeriği · ".repeat(1 << 15).into()),
        total_len: Some(5 << 20),
        unit: LenUnit::Bytes,
    }));
    let row = |i: i64, cut: Value| {
        vec![
            Value::Int(i),
            Value::Text(note.into()),
            Value::Json(json.into()),
            Value::Json(json.into()),
            Value::Bytes(bytes.clone()),
            Value::Xml(xml.into()),
            Value::Json(r#"{"siparis": 4211, "kalemler": ["#.into()),
            cut,
        ]
    };
    let rows = vec![
        row(1, Value::Text("ek yok".into())),
        row(2, cut),
        row(3, Value::Null),
    ];
    (schema, rows)
}
