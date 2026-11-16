//! The M3a shots (Task 35): NOTES surfaces 1–14 of `design/m3a-query/`, each state at 1280x800
//! and the 960x600 minimum, in every locale and theme.
//!
//! A shot is a sequence of steps over a freshly started shell: each step acts, then the harness
//! waits for its condition (bounded and polled, never a fixed sleep). The states are reached
//! the way the product reaches them wherever that is deterministic:
//!
//! - **A live console** (surfaces 1, 3, 4, 6, 8, 13 and the read-only denial of 5): a
//!   `FakeDriver` that answers the SQL-context probe and dials itself (no socket, no DNS); the
//!   profile is saved and connected through `ConnectionsModel`, so the connect binds the
//!   focused `Console 1` (R-M3-37), whose host opens its session and attaches the run UI. Text
//!   and caret go through the editor; Run, the chooser, the context menu, the transaction and
//!   cursor states and the close and quit prompts are the product's own commands, session
//!   requests and guards.
//! - **Fixtures** where a state is transient or carries a clock: Task 32's
//!   `console::view::fixtures` (toolbar states, the large file), Task 30's
//!   `results::view::open_results_shot` (surfaces 9–12 from `ResultStore::from_rows`, no core
//!   runtime, R-M3-15), this task's `exec_ui::fixtures::show_marks` (surface 5, routed from
//!   Task 33, ledger I7) and the copy and clipboard toasts (`results::copy::fixtures`, the
//!   editor's clipboard notices): a shot never writes the machine's clipboard.
//!
//! Each state has a marker (a label of its own source file; data text where the file draws
//! more than this state) and a state check ([`M3a::check`]) read from the app's models after
//! the frame: the band's style and units, the chooser, the prompt, the marks. A grid state's
//! marker is a header the grid drew (data, the same in every locale).
#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hacivat_app::connections::{ConnectionsBoot, ConnectionsModel};
use hacivat_app::console::toolbar::RunUx;
use hacivat_app::console::view::fixtures as console_fixtures;
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::exec_ui;
use hacivat_app::results::ActiveResult;
use hacivat_app::results::view::{ResultsShot, hosts, open_results_shot};
use hacivat_app::shell::Workspace;
use hacivat_driver::fake::{FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows};
use hacivat_driver::{Driver as _, DriverRegistry, ExecOpts, ExecSummary, Protocol, TxStatus};
use hacivat_exec::{ErrorView, ResultTab, TabKey, UnitStatus, UnitSummary};
use hacivat_proto::{AccessLabel, DbmsKind, EnvTag, ProfileId};
use hacivat_results::{
    CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, MemoryBudget, ResultId,
    ResultStore, ResultWriter, StoreSource,
};
use hacivat_session::LiveState;
use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint};
use hacivat_session::session::{
    ErrorPolicy, ExecOrigin, ObserverRef, Request, RowsOutcome, RunEvent, RunId, RunObserver,
    RunSpec, RunUnit, SessionHandle, UnitId,
};
use hacivat_settings::LocalPaths;
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::text::RenderedText;
use hacivat_ui::toast::ToastQueue;
use hacivat_ui_core::gpui_surface::{
    App, Entity, EntityInputHandler as _, Focusable as _, Keystroke, Window,
};
use hacivat_ui_editor::preview::{self, BandStyle};
use hacivat_ui_editor::{EditorView, ExecMarkState, Tier};
use hacivat_values::{ColumnMeta, LogicalType, NativeType, ResultSchema, Value};

// ---- the states -----------------------------------------------------------------------------

/// One M3a state (a shot surface), named after its NOTES surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum M3a {
    // 1 · Console editor area
    ConsoleEditor,
    ConsoleSelection,
    ConsoleUnfocused,
    ConsoleLongLine,
    ConsoleManyLines,
    ConsoleEmpty,
    ConsoleProduction,
    ConsoleMysql,
    // 2 · Editor toolbar
    ToolbarRunning,
    ToolbarCancelling,
    ToolbarReadOnly,
    ToolbarWaiting,
    ToolbarProductionWaiting,
    // 3 · Run preview band
    PreviewStatement,
    PreviewTwoOnLine,
    PreviewSelection,
    PreviewAsk,
    PreviewNothing,
    PreviewUncertain,
    PreviewOffline,
    // 4 · Statement chooser
    ChooserCandidates,
    ChooserHover,
    ChooserUncertain,
    // 5 · Gutter execution status
    MarksRun,
    MarksStopped,
    MarksCancelling,
    MarksCancelled,
    MarksStale,
    MarksTwoOnRow,
    MarksLongLine,
    MarksDenied,
    // 6 · Status bar segments
    StatusTxOpen,
    StatusTxFailed,
    StatusCursorOpen,
    StatusEverything,
    StatusProductionResult,
    // 7 · Large-file and clipboard notices
    NoticeReducedMode,
    NoticePasteRefused,
    NoticeNul,
    NoticeClipboard,
    // 8 · Editor context menu
    ContextMenu,
    ContextDescribe,
    // 9–12 · Results tile, grid, cursor strip, output log (Task 30's fixtures)
    Results(ResultsShot),
    GridRange,
    // 13 · Close with an open transaction
    PromptClose,
    PromptQuit,
    PromptFailed,
    PromptFailedQuit,
    // 14 · Copy feedback
    CopyCopied,
    CopyCopying,
    CopyTooMuch,
    // ## m3b-t22-tx-ui (named exception): design/m3b-safe surfaces 3, 4, 5 and 12, rendered
    // by the M3a machinery until Task 29 moves them to `shots/m3b.rs` ([`M3B_T22`]).
    TxChipMenu,
    TxIsolationMenu,
    TxBadgeOpen,
    TxBadgeFailed,
    TxSegmentMenu,
    TxIdleToast,
    TxShared,
    // ## m3b-t25-results (named exception): design/m3b-safe surface 10's tab menu (the other
    // states are `Results(..)` fixtures, [`M3B_T25`]).
    ResultsTabMenu,
}

// ## m3b-t25-results (named exception)
/// The Task 25 states (after Task 22's).
pub(super) const M3B_T25: [M3a; 10] = [
    M3a::Results(ResultsShot::TitledTabs),
    M3a::ResultsTabMenu,
    M3a::Results(ResultsShot::Untyped),
    M3a::Results(ResultsShot::SetLimit),
    M3a::Results(ResultsShot::Counting),
    M3a::Results(ResultsShot::CountExact),
    M3a::Results(ResultsShot::Evicted),
    M3a::Results(ResultsShot::FetchAll),
    M3a::Results(ResultsShot::Notices),
    M3a::Results(ResultsShot::MysqlWarnings),
];

// ## m3b-t22-tx-ui (named exception)
/// The Task 22 states (after every M3a state).
pub(super) const M3B_T22: [M3a; 7] = [
    M3a::TxChipMenu,
    M3a::TxIsolationMenu,
    M3a::TxBadgeOpen,
    M3a::TxBadgeFailed,
    M3a::TxSegmentMenu,
    M3a::TxIdleToast,
    M3a::TxShared,
];

/// The single-session connection (surface 12).
pub(super) const SHARED: &str = "orders-shared";

pub(super) const M3A: [M3a; 68] = [
    M3a::ConsoleEditor,
    M3a::ConsoleSelection,
    M3a::ConsoleUnfocused,
    M3a::ConsoleLongLine,
    M3a::ConsoleManyLines,
    M3a::ConsoleEmpty,
    M3a::ConsoleProduction,
    M3a::ConsoleMysql,
    M3a::ToolbarRunning,
    M3a::ToolbarCancelling,
    M3a::ToolbarReadOnly,
    M3a::ToolbarWaiting,
    M3a::ToolbarProductionWaiting,
    M3a::PreviewStatement,
    M3a::PreviewTwoOnLine,
    M3a::PreviewSelection,
    M3a::PreviewAsk,
    M3a::PreviewNothing,
    M3a::PreviewUncertain,
    M3a::PreviewOffline,
    M3a::ChooserCandidates,
    M3a::ChooserHover,
    M3a::ChooserUncertain,
    M3a::MarksRun,
    M3a::MarksStopped,
    M3a::MarksCancelling,
    M3a::MarksCancelled,
    M3a::MarksStale,
    M3a::MarksTwoOnRow,
    M3a::MarksLongLine,
    M3a::MarksDenied,
    M3a::StatusTxOpen,
    M3a::StatusTxFailed,
    M3a::StatusCursorOpen,
    M3a::StatusEverything,
    M3a::StatusProductionResult,
    M3a::NoticeReducedMode,
    M3a::NoticePasteRefused,
    M3a::NoticeNul,
    M3a::NoticeClipboard,
    M3a::ContextMenu,
    M3a::ContextDescribe,
    M3a::Results(ResultsShot::FirstRows),
    M3a::Results(ResultsShot::Rows),
    M3a::Results(ResultsShot::ZeroRows),
    M3a::Results(ResultsShot::UpdateCount),
    M3a::Results(ResultsShot::Error),
    M3a::Results(ResultsShot::TwoResults),
    M3a::Results(ResultsShot::WideColumns),
    M3a::Results(ResultsShot::Streaming),
    M3a::Results(ResultsShot::Suspended),
    M3a::Results(ResultsShot::Closed(CloseReason::SessionNeeded)),
    M3a::Results(ResultsShot::Closed(CloseReason::IdleTimeout)),
    M3a::Results(ResultsShot::Closed(CloseReason::TxEnded)),
    M3a::Results(ResultsShot::Closed(CloseReason::ConnectionLost)),
    M3a::Results(ResultsShot::Cancelled),
    M3a::Results(ResultsShot::Memory),
    M3a::Results(ResultsShot::MoreResultSets),
    M3a::Results(ResultsShot::RunLog),
    M3a::Results(ResultsShot::EventsDropped),
    M3a::GridRange,
    M3a::PromptClose,
    M3a::PromptQuit,
    M3a::PromptFailed,
    M3a::PromptFailedQuit,
    M3a::CopyCopied,
    M3a::CopyCopying,
    M3a::CopyTooMuch,
];

// ---- the texts ------------------------------------------------------------------------------

/// The dev connection, the production one and the MySQL one.
pub(super) const DEV: &str = "orders-dev";
pub(super) const PROD: &str = "orders-prod";
pub(super) const SHOP: &str = "shop-dev";

/// A three-statement script with Turkish identifiers and strings (INV-3: lexed as written).
const SCRIPT: &str = "-- Siparişler, İzmir şubesi\nSELECT o.id, c.\"müşteri adı\", o.total\n  FROM orders o\n  JOIN customers c ON c.id = o.customer_id\n WHERE c.city = 'İzmir'\n ORDER BY o.created_at DESC;\n\nUPDATE orders SET status = 'shipped' WHERE id = 1042;\n\nSELECT count(*) FROM order_lines WHERE note = 'Şanlıurfa';\n";

/// Two statements on one line.
const TWO_ON_LINE: &str = "SELECT 1; SELECT o.id FROM orders o WHERE o.total > 100;\n";

/// Two statements, one per line: at the start of the first, Run asks (D-011).
const TWO: &str =
    "SELECT o.id, o.total FROM orders o;\nSELECT c.\"müşteri adı\" FROM customers c;\n";

/// A `;` inside a MySQL executable comment: the boundary is uncertain (R-M3-24).
const UNCERTAIN: &str = "SELECT 1 /*! ; */ ;\nSELECT 2;\n";

/// A MySQL executable comment (code on the inlay well, delimiters in comment ink).
const MYSQL: &str = "/*!40101 SET NAMES utf8mb4 */;\nSELECT p.sku, p.`ürün adı` FROM products p WHERE p.city = 'İzmir';\n";

/// Four statements, one per line, for the marks.
const MARKS: &str = "SELECT o.id, o.total FROM orders o;\nUPDATE orders SET status = 'shipped' WHERE id = 1042;\nSELECT count(*) FROM order_lines;\nSELECT c.\"müşteri adı\" FROM customers c;\n";

/// Two statements on one row, for "two marks on one row".
const MARKS_ROW: &str = "SELECT 1; SELECT * FROM order_lines_archive;\n";

/// A NUL inside a string: drawn as the `NUL` chip.
const NUL: &str = "INSERT INTO notes (body) VALUES ('a\0b');\n";

/// One long line (a long IN list), then a short statement.
fn long_line() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        let ids: Vec<String> = (1_000..1_060).map(|n| n.to_string()).collect();
        format!(
            "SELECT o.id, o.total, o.status FROM orders o WHERE o.id IN ({}) ORDER BY o.id;\nUPDATE orders SET status = 'cancelled' WHERE customer_id IN (SELECT c.id FROM customers c WHERE c.city = 'Şanlıurfa' AND c.created_at < '2024-01-01');\n",
            ids.join(", ")
        )
    })
}

/// Twelve thousand statements: five-digit line numbers.
fn many_lines() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        (1..=12_000)
            .map(|n| format!("SELECT {n} AS satır;\n"))
            .collect()
    })
}

/// 5 MB of SQL: past T0's 4 MB, below 20 MB (the T1 banner).
fn large() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        let line = "SELECT 'İzmir', 1234567;\n";
        line.repeat((5 << 20) / line.len() + 1)
    })
}

/// The byte offset just after the first `needle` in `text`.
fn after(text: &str, needle: &str) -> usize {
    text.find(needle).expect("the needle is in the text") + needle.len()
}

/// The byte offset of the first `needle` in `text`.
fn at(text: &str, needle: &str) -> usize {
    text.find(needle).expect("the needle is in the text")
}

/// The byte range of statement `stmt` (its text without the `;`) in `text`.
fn range_of(text: &str, stmt: &str) -> std::ops::Range<usize> {
    let start = at(text, stmt);
    start..start + stmt.len()
}

// ---- markers and checks ---------------------------------------------------------------------

const TOOLBAR: &str = "hacivat-app/src/console/toolbar.rs:";
const MARKS_RS: &str = "hacivat-ui-editor/src/marks.rs:";
const MENU: &str = "hacivat-ui/src/menu.rs:";
const DIALOG: &str = "hacivat-ui/src/dialog.rs:";
const TOAST: &str = "hacivat-ui/src/toast.rs:";
const STRIP: &str = "hacivat-app/src/results/strip.rs:";
const OUTPUT: &str = "hacivat-app/src/results/output_view.rs:";
/// A grid header's record path (data, the same in every locale).
pub(super) const GRID_HEADER: &str = "hacivat-ui-grid/src/element/layers.rs#header";

impl M3a {
    pub(super) fn name(self) -> &'static str {
        match self {
            M3a::ConsoleEditor => "console-editor",
            M3a::ConsoleSelection => "console-selection",
            M3a::ConsoleUnfocused => "console-unfocused",
            M3a::ConsoleLongLine => "console-long-line",
            M3a::ConsoleManyLines => "console-many-lines",
            M3a::ConsoleEmpty => "console-empty",
            M3a::ConsoleProduction => "console-production",
            M3a::ConsoleMysql => "console-mysql-comment",
            M3a::ToolbarRunning => "toolbar-running",
            M3a::ToolbarCancelling => "toolbar-cancelling",
            M3a::ToolbarReadOnly => "toolbar-read-only",
            M3a::ToolbarWaiting => "toolbar-waiting",
            M3a::ToolbarProductionWaiting => "toolbar-production-waiting",
            M3a::PreviewStatement => "preview-statement",
            M3a::PreviewTwoOnLine => "preview-two-on-line",
            M3a::PreviewSelection => "preview-selection",
            M3a::PreviewAsk => "preview-ask",
            M3a::PreviewNothing => "preview-nothing",
            M3a::PreviewUncertain => "preview-uncertain",
            M3a::PreviewOffline => "preview-offline",
            M3a::ChooserCandidates => "chooser-candidates",
            M3a::ChooserHover => "chooser-hover",
            M3a::ChooserUncertain => "chooser-uncertain",
            M3a::MarksRun => "marks-run",
            M3a::MarksStopped => "marks-stopped",
            M3a::MarksCancelling => "marks-cancelling",
            M3a::MarksCancelled => "marks-cancelled",
            M3a::MarksStale => "marks-stale",
            M3a::MarksTwoOnRow => "marks-two-on-row",
            M3a::MarksLongLine => "marks-long-line",
            M3a::MarksDenied => "marks-denied",
            M3a::StatusTxOpen => "status-tx-open",
            M3a::StatusTxFailed => "status-tx-failed",
            M3a::StatusCursorOpen => "status-cursor-open",
            M3a::StatusEverything => "status-everything",
            M3a::StatusProductionResult => "status-production-result",
            M3a::NoticeReducedMode => "notice-reduced-mode",
            M3a::NoticePasteRefused => "notice-paste-refused",
            M3a::NoticeNul => "notice-nul",
            M3a::NoticeClipboard => "notice-clipboard",
            M3a::ContextMenu => "context-menu",
            M3a::ContextDescribe => "context-describe",
            M3a::Results(r) => results_name(r),
            M3a::GridRange => "grid-range",
            M3a::PromptClose => "prompt-close",
            M3a::PromptQuit => "prompt-quit",
            M3a::PromptFailed => "prompt-failed",
            M3a::PromptFailedQuit => "prompt-failed-quit",
            M3a::CopyCopied => "copy-copied",
            M3a::CopyCopying => "copy-copying",
            M3a::CopyTooMuch => "copy-too-much",
            // ## m3b-t22-tx-ui
            M3a::TxChipMenu => "tx-chip-menu",
            M3a::TxIsolationMenu => "tx-isolation-menu",
            M3a::TxBadgeOpen => "tx-badge-open",
            M3a::TxBadgeFailed => "tx-badge-failed",
            M3a::TxSegmentMenu => "tx-segment-menu",
            M3a::TxIdleToast => "tx-idle-toast",
            M3a::TxShared => "tx-shared",
            // ## m3b-t25-results (named exception)
            M3a::ResultsTabMenu => "results-tab-menu",
        }
    }

    /// The label that proves the state is on screen: its creation site and, where that site
    /// draws more than this state, its (data) text. Fix round 1 (I1): a label only this state
    /// draws, wherever one exists; the console states whose label is the toolbar's connection
    /// name are told apart by their state check ([`M3a::check`]).
    pub(super) fn marker(self) -> (&'static str, Option<&'static str>) {
        match self {
            M3a::ConsoleEditor
            | M3a::ConsoleSelection
            | M3a::ConsoleUnfocused
            | M3a::ConsoleLongLine
            | M3a::ConsoleManyLines
            | M3a::ConsoleEmpty
            | M3a::PreviewStatement
            | M3a::NoticeNul => (TOOLBAR, Some(DEV)),
            M3a::ConsoleProduction => (TOOLBAR, Some(PROD)),
            M3a::ConsoleMysql => (TOOLBAR, Some(SHOP)),
            // The toolbar's own state labels (fix round 1: ids in `console/toolbar.rs`).
            M3a::ToolbarRunning => ("#console-running", None),
            M3a::ToolbarCancelling => ("#console-cancelling", None),
            M3a::ToolbarReadOnly => ("#console-lock-label", None),
            M3a::ToolbarWaiting | M3a::ToolbarProductionWaiting => ("#console-waiting-text", None),
            // The status bar's transient sentence: "Nothing to run here", Describe.
            M3a::PreviewNothing | M3a::ContextDescribe => ("#status-message", None),
            // The band's hint at the end of the unit's line ("1 of 3", "2 choices", "boundary
            // uncertain", "not connected") and every end-of-line status: `marks.rs`.
            M3a::PreviewTwoOnLine
            | M3a::PreviewSelection
            | M3a::PreviewAsk
            | M3a::PreviewUncertain
            | M3a::PreviewOffline
            | M3a::MarksRun
            | M3a::MarksStopped
            | M3a::MarksCancelling
            | M3a::MarksCancelled
            | M3a::MarksStale
            | M3a::MarksTwoOnRow
            | M3a::MarksLongLine
            | M3a::MarksDenied => (MARKS_RS, None),
            M3a::ChooserCandidates
            | M3a::ChooserHover
            | M3a::ChooserUncertain
            | M3a::ContextMenu => (MENU, None),
            M3a::StatusTxOpen | M3a::StatusEverything => ("#status-tx-open", None),
            M3a::StatusTxFailed => ("#status-tx-failed", None),
            // The active result's rows in the status bar (Status Follows The Tab).
            M3a::StatusProductionResult => ("#status-rows", None),
            M3a::StatusCursorOpen => ("#button-label-status-cursor", None),
            M3a::NoticeReducedMode => ("#editor-t1-title", None),
            M3a::NoticePasteRefused | M3a::NoticeClipboard => (TOAST, None),
            M3a::Results(r) => results_marker(r),
            M3a::GridRange => (GRID_HEADER, Some("customer")),
            M3a::PromptClose | M3a::PromptQuit | M3a::PromptFailed | M3a::PromptFailedQuit => {
                (DIALOG, None)
            }
            M3a::CopyCopied | M3a::CopyCopying | M3a::CopyTooMuch => (TOAST, None),
            // ## m3b-t22-tx-ui: the menu's title row, the badge's own labels, the toast.
            M3a::TxChipMenu | M3a::TxIsolationMenu | M3a::TxSegmentMenu => {
                ("#popup-menu-title", None)
            }
            M3a::TxBadgeOpen => ("#tx-badge-text", None),
            M3a::TxBadgeFailed => ("#tx-badge-failed", None),
            M3a::TxIdleToast => (TOAST, None),
            M3a::TxShared => ("#tx-chip-mode", None),
            // ## m3b-t25-results (named exception)
            // The menu's rows are button labels; `check` asks for the menu layer.
            M3a::ResultsTabMenu => ("hacivat-ui/src/button.rs:", None),
        }
    }

    // ## m3a-t35-shots (fix round 1, I2)
    /// The trust signals the state must show (NOTES cross-cutting #7: environment, read-only,
    /// transaction and cursor never collapse): each group lists the label ids (`#id`) of which
    /// one must be visible and painted.
    pub(super) fn required(self) -> &'static [&'static [&'static str]] {
        const PROD_NOTICE: &[&str] = &[
            "#button-label-console-prod-notice",
            "#button-label-console-prod-short",
        ];
        const ENV: &[&str] = &["#status-env-label"];
        const TX: &[&str] = &["#status-tx-open"];
        const TX_FAILED: &[&str] = &["#status-tx-failed"];
        const CURSOR: &[&str] = &["#button-label-status-cursor"];
        const CLOCK: &[&str] = &["#console-clock"];
        // ## m3a-f2-visual (named exception): rows · timing is not a trust signal (NOTES #7);
        // it is the status bar's last cell to give way, whole (NOTES surface 6), so where the bar
        // has no room (960 px in the pseudo-locales) the result tile's toolbar, which says the
        // same, must still show it: the surface never loses the count.
        const ROWS: &[&str] = &["#status-rows", "#results-rows-timing"];
        // ## m3b-t22-tx-ui (named exception): the transaction badge and the mode chip are
        // trust signals too (NOTES M3b cross-cutting #7).
        const BADGE: &[&str] = &["#tx-badge-text"];
        const BADGE_FAILED: &[&str] = &["#tx-badge-failed"];
        const CHIP: &[&str] = &["#tx-chip-mode"];
        match self {
            M3a::TxChipMenu | M3a::TxIsolationMenu => &[CHIP],
            M3a::TxBadgeOpen | M3a::TxShared => &[TX, BADGE, CHIP],
            M3a::TxBadgeFailed => &[TX_FAILED, BADGE_FAILED, CHIP],
            M3a::TxSegmentMenu | M3a::TxIdleToast => &[TX, BADGE],
            M3a::StatusTxOpen => &[TX, BADGE],
            M3a::StatusTxFailed => &[TX_FAILED, BADGE_FAILED],
            M3a::ConsoleProduction | M3a::ToolbarProductionWaiting | M3a::MarksDenied => {
                &[PROD_NOTICE, ENV]
            }
            M3a::StatusProductionResult => &[ENV],
            M3a::ToolbarRunning => &[CLOCK],
            // ## m3b-t22-tx-ui (named exception): `StatusTxOpen` / `StatusTxFailed` moved up,
            // with the badge.
            M3a::PromptClose | M3a::PromptQuit => &[TX],
            M3a::PromptFailed | M3a::PromptFailedQuit => &[TX_FAILED],
            M3a::StatusCursorOpen => &[CURSOR],
            M3a::StatusEverything => &[TX, CURSOR, ROWS],
            _ => &[],
        }
    }

    /// Whether the title-bar capsule shows production (the one filled badge).
    pub(super) fn shows_production(self) -> bool {
        matches!(
            self,
            M3a::ConsoleProduction | M3a::ToolbarProductionWaiting | M3a::MarksDenied
        )
    }

    /// Whether the state is a grid (its marker must be a header the grid drew).
    pub(super) fn is_grid(self) -> bool {
        matches!(
            self,
            M3a::GridRange
                | M3a::CopyCopied
                | M3a::Results(
                    ResultsShot::Rows
                        | ResultsShot::FirstRows
                        | ResultsShot::ZeroRows
                        | ResultsShot::TwoResults
                        | ResultsShot::Streaming
                        | ResultsShot::Suspended
                        | ResultsShot::Closed(_)
                        | ResultsShot::Cancelled
                        | ResultsShot::Memory
                        | ResultsShot::MoreResultSets
                )
        )
    }

    /// The steps after the shell opened: each acts, then the harness waits for its condition.
    pub(super) fn steps(self) -> Vec<Step> {
        use M3a::*;
        let caret = |text: &'static str, needle: &str| {
            let c = after(text, needle);
            c..c
        };
        match self {
            ConsoleEditor => live(
                DEV,
                SCRIPT,
                caret(SCRIPT, "o.total"),
                Expect::Band(BandStyle::Run),
            ),
            ConsoleSelection => live(
                DEV,
                SCRIPT,
                at(SCRIPT, "  FROM")..after(SCRIPT, "'İzmir'"),
                Expect::Band(BandStyle::Run),
            ),
            ConsoleUnfocused => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    at(SCRIPT, "  FROM")..after(SCRIPT, "'İzmir'"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(instant(|window, cx| window.blur(cx)));
                s
            }
            ConsoleLongLine => {
                let t = long_line();
                live(
                    DEV,
                    t,
                    after(t, "1059")..after(t, "1059"),
                    Expect::Band(BandStyle::Run),
                )
            }
            ConsoleManyLines => {
                let t = many_lines();
                let c = t.len() - 1;
                live(DEV, t, c - 4..c - 4, Expect::Band(BandStyle::Run))
            }
            ConsoleEmpty => live(DEV, "", 0..0, Expect::Any),
            ConsoleProduction => live(
                PROD,
                SCRIPT,
                caret(SCRIPT, "o.total"),
                Expect::Band(BandStyle::Run),
            ),
            ConsoleMysql => live(
                SHOP,
                MYSQL,
                caret(MYSQL, "p.sku"),
                Expect::Band(BandStyle::Run),
            ),
            ToolbarRunning => toolbar(DEV, |tab, cx| {
                console_fixtures::show_run(tab, RunUx::Running(3), cx)
            }),
            ToolbarCancelling => toolbar(DEV, |tab, cx| {
                console_fixtures::show_run(tab, RunUx::Cancelling, cx)
            }),
            ToolbarReadOnly => toolbar(DEV, |tab, cx| {
                console_fixtures::show_lock(tab, AccessLabel::ReadOnlyGuarded, cx)
            }),
            ToolbarWaiting => toolbar(DEV, |tab, cx| console_fixtures::show_waiting(tab, 4, 4, cx)),
            ToolbarProductionWaiting => toolbar(PROD, |tab, cx| {
                console_fixtures::show_waiting(tab, 4, 4, cx)
            }),
            PreviewStatement => live(
                DEV,
                SCRIPT,
                caret(SCRIPT, "SET status"),
                Expect::Band(BandStyle::Run),
            ),
            PreviewTwoOnLine => live(
                DEV,
                TWO_ON_LINE,
                caret(TWO_ON_LINE, "o.id FROM"),
                Expect::Band(BandStyle::Run),
            ),
            PreviewSelection => live(
                DEV,
                SCRIPT,
                at(SCRIPT, "SELECT o.id")..SCRIPT.len() - 1,
                Expect::Units(3),
            ),
            PreviewAsk => live(DEV, TWO, 0..0, Expect::Choices),
            PreviewNothing => {
                let blank = after(SCRIPT, "DESC;\n");
                live(DEV, SCRIPT, blank..blank, Expect::Nothing)
            }
            PreviewUncertain => live(SHOP, UNCERTAIN, 0..0, Expect::Band(BandStyle::Uncertain)),
            PreviewOffline => offline(SCRIPT, after(SCRIPT, "o.total")),
            ChooserCandidates => {
                let mut s = live(DEV, TWO, 0..0, Expect::Choices);
                s.push(run_to_chooser());
                s
            }
            ChooserHover => {
                let mut s = live(DEV, TWO, 0..0, Expect::Choices);
                s.push(run_to_chooser());
                s.push(key("down", |window, cx| {
                    editor_in(window, cx).is_some_and(|e| {
                        preview::current(e.read(cx))
                            .is_some_and(|b| b.from_override && b.units.len() == 2)
                    })
                }));
                s
            }
            ChooserUncertain => {
                let mut s = live(SHOP, UNCERTAIN, 0..0, Expect::Band(BandStyle::Uncertain));
                s.push(run_to_chooser());
                s
            }
            MarksRun => marks(
                MARKS,
                vec![
                    (
                        "SELECT o.id, o.total FROM orders o",
                        succeeded(42, Some(rows(100))),
                    ),
                    (
                        "UPDATE orders SET status = 'shipped' WHERE id = 1042",
                        succeeded(38, Some(RowsOutcome::Affected(12))),
                    ),
                    ("SELECT count(*) FROM order_lines", running(3_000)),
                    (
                        "SELECT c.\"müşteri adı\" FROM customers c",
                        (UnitStatus::Queued { position: 1 }, Duration::ZERO),
                    ),
                ],
            ),
            MarksStopped => marks(
                MARKS,
                vec![
                    (
                        "SELECT o.id, o.total FROM orders o",
                        succeeded(42, Some(rows(100))),
                    ),
                    (
                        "UPDATE orders SET status = 'shipped' WHERE id = 1042",
                        failed(
                            "23514",
                            "new row for relation \"orders\" violates check constraint \"orders_status_check\"",
                        ),
                    ),
                    (
                        "SELECT count(*) FROM order_lines",
                        (UnitStatus::Skipped, Duration::ZERO),
                    ),
                    (
                        "SELECT c.\"müşteri adı\" FROM customers c",
                        (UnitStatus::Skipped, Duration::ZERO),
                    ),
                ],
            ),
            MarksCancelling => marks(
                MARKS,
                vec![
                    (
                        "SELECT o.id, o.total FROM orders o",
                        succeeded(42, Some(rows(100))),
                    ),
                    (
                        "UPDATE orders SET status = 'shipped' WHERE id = 1042",
                        (UnitStatus::Cancelling, Duration::from_secs(4)),
                    ),
                    (
                        "SELECT count(*) FROM order_lines",
                        (UnitStatus::Queued { position: 1 }, Duration::ZERO),
                    ),
                ],
            ),
            MarksCancelled => marks(
                MARKS,
                vec![
                    (
                        "SELECT o.id, o.total FROM orders o",
                        succeeded(42, Some(rows(100))),
                    ),
                    (
                        "UPDATE orders SET status = 'shipped' WHERE id = 1042",
                        (UnitStatus::Cancelled, Duration::from_millis(12_408)),
                    ),
                    (
                        "SELECT count(*) FROM order_lines",
                        (UnitStatus::Skipped, Duration::ZERO),
                    ),
                ],
            ),
            MarksStale => {
                let mut s = marks(
                    MARKS,
                    vec![
                        (
                            "SELECT o.id, o.total FROM orders o",
                            succeeded(42, Some(rows(100))),
                        ),
                        (
                            "UPDATE orders SET status = 'shipped' WHERE id = 1042",
                            succeeded(38, Some(RowsOutcome::Affected(12))),
                        ),
                    ],
                );
                // An edit inside the first statement: its mark goes stale.
                s.push(type_at(
                    after(MARKS, "o.total"),
                    ", o.status",
                    |window, cx| {
                        // Fix round 1 (M9): the edit landed and made the first mark stale.
                        stale_first(editor_in(window, cx).as_ref(), cx)
                    },
                ));
                s
            }
            MarksTwoOnRow => marks(
                MARKS_ROW,
                vec![
                    ("SELECT 1", succeeded(3, Some(rows(1)))),
                    (
                        "SELECT * FROM order_lines_archive",
                        failed("42P01", "relation \"order_lines_archive\" does not exist"),
                    ),
                ],
            ),
            MarksLongLine => {
                let t = long_line();
                let first = &t[..t.find(';').expect("a statement")];
                let second_start = t.find('\n').expect("two lines") + 1;
                let second =
                    &t[second_start..t[second_start..].find(';').expect("a second") + second_start];
                let mut s = live(DEV, t, 0..0, Expect::Band(BandStyle::Run));
                s.push(show_marks(vec![
                    (range_of(t, first), succeeded(1_204, Some(rows(60)))),
                    (range_of(t, second), failed("40P01", "deadlock detected")),
                ]));
                s
            }
            MarksDenied => {
                // A read-only console never starts a transaction: not sent (DenyLayer::
                // Transaction). (An UPDATE is sent and refused by the server, 25006: a failure
                // with its chip, as surface 5's Failed row shows.)
                let text = "BEGIN;\nUPDATE orders SET status = 'shipped' WHERE id = 1042;\n";
                let mut s = live(PROD, text, 2..2, Expect::Band(BandStyle::Run));
                // Run asks (the statement, or the whole script); Enter runs the statement.
                s.push(run_to_chooser());
                s.push(run_denied());
                s
            }
            StatusTxOpen => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(user_tx(&["BEGIN", UPD1, UPD2, UPD3], Some(3), false));
                s
            }
            StatusTxFailed => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(user_tx(&["BEGIN", UPD1, UPD2, FAILS], None, true));
                s
            }
            StatusCursorOpen => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(open_cursor());
                s
            }
            StatusEverything => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    at(SCRIPT, "SELECT o.id")..after(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(user_tx(&["BEGIN", UPD1], Some(1), false));
                s.push(open_cursor());
                s.push(suspended_tab());
                s
            }
            StatusProductionResult => results(ResultsShot::Rows, Some(EnvTag::Production)),
            NoticeReducedMode => vec![
                save_only(DEV, EnvTag::Dev),
                step(
                    |window, cx| {
                        let id = profile_named(cx, DEV).expect("saved");
                        console_fixtures::open(id, large(), window, cx);
                    },
                    |window, cx| {
                        editor_in(window, cx)
                            .is_some_and(|e| e.read(cx).status().tier == Tier::Reduced)
                    },
                ),
            ],
            NoticePasteRefused => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(toast(|cx| {
                    ToastQueue::push(
                        cx,
                        hacivat_ui_editor::clipboard::refused_toast(32_925_696, 20 << 20),
                    )
                }));
                s
            }
            NoticeNul => {
                let c = after(NUL, "'a\0b'");
                let mut s = live(DEV, NUL, c..c, Expect::Band(BandStyle::Run));
                s.push(toast(|cx| {
                    ToastQueue::push(cx, hacivat_ui_editor::clipboard::nul_toast(1))
                }));
                s
            }
            NoticeClipboard => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(toast(|cx| {
                    ToastQueue::push(
                        cx,
                        hacivat_ui_editor::clipboard::unavailable_toast(std::rc::Rc::new(
                            |_, _| {},
                        )),
                    )
                }));
                s
            }
            ContextMenu => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    at(SCRIPT, "SELECT o.id")..after(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(editor_command("editor.context_menu"));
                s
            }
            ContextDescribe => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(editor_command("editor.describe_exec_range"));
                s
            }
            Results(r) => results(r, Some(EnvTag::Dev)),
            GridRange => {
                let mut s = results(ResultsShot::Rows, Some(EnvTag::Dev));
                s.push(grid_range());
                s
            }
            PromptClose => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(user_tx(&["BEGIN", UPD1, UPD2, UPD3], Some(3), false));
                s.push(prompt(false));
                s
            }
            PromptQuit => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(user_tx(&["BEGIN", UPD1, UPD2, UPD3], Some(3), false));
                s.push(prompt(true));
                s
            }
            PromptFailed => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(user_tx(&["BEGIN", UPD1, UPD2, FAILS], None, true));
                s.push(prompt(false));
                s
            }
            PromptFailedQuit => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(user_tx(&["BEGIN", UPD1, UPD2, FAILS], None, true));
                s.push(prompt(true));
                s
            }
            CopyCopied => {
                let mut s = results(ResultsShot::Rows, Some(EnvTag::Dev));
                s.push(grid_range());
                s.push(toast(|cx| {
                    hacivat_app::results::copy::fixtures::show_copied(3, 2, cx)
                }));
                s
            }
            CopyCopying => {
                let mut s = results(ResultsShot::Rows, Some(EnvTag::Dev));
                s.push(toast(|cx| {
                    hacivat_app::results::copy::fixtures::show_copying(48_000, cx)
                }));
                s
            }
            CopyTooMuch => {
                let mut s = results(ResultsShot::Rows, Some(EnvTag::Dev));
                s.push(toast(|cx| {
                    hacivat_app::results::copy::copy_refused(
                        hacivat_ui_grid::model::CopyRefusal::TooMany { cells: 8_000_000 },
                        cx,
                    )
                }));
                s
            }
            // ## m3b-t22-tx-ui
            TxChipMenu | TxIsolationMenu => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(tx_menu(if self == TxChipMenu {
                    TxMenu::Chip
                } else {
                    TxMenu::Isolation
                }));
                s
            }
            TxBadgeOpen | TxIdleToast | TxSegmentMenu => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(user_tx(&["BEGIN", UPD1, UPD2, UPD3], Some(3), false));
                if self == TxSegmentMenu {
                    s.push(tx_menu(TxMenu::Segment));
                }
                if self == TxIdleToast {
                    s.push(step(
                        |window, cx| {
                            let session = session_in(window, cx).expect("the console's session");
                            hacivat_app::console::idle_toast::show(session.id(), 10, cx);
                        },
                        |_, cx| !ToastQueue::visible_ids(cx).is_empty(),
                    ));
                }
                s
            }
            TxBadgeFailed => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(user_tx(&["BEGIN", UPD1, UPD2, FAILS], None, true));
                s
            }
            TxShared => {
                let mut s = live(
                    SHARED,
                    SCRIPT,
                    caret(SCRIPT, "o.total"),
                    Expect::Band(BandStyle::Run),
                );
                s.push(user_tx(&["BEGIN", UPD1], Some(1), false));
                s
            }
            // ## m3b-t25-results (named exception): the titled tabs, then the first tab's menu
            // under it.
            ResultsTabMenu => {
                let mut s = results(ResultsShot::TitledTabs, Some(EnvTag::Dev));
                s.push(step(
                    |window, cx| {
                        let console = active_console(window, cx).expect("a console");
                        hacivat_app::results::view::open_tab_menu_for(console, 3, window, cx);
                    },
                    |window, cx| menu_open(window, cx),
                ));
                s
            }
        }
    }

    /// What the app's models (and, for the toasts and menus, the frame) must say once the frame
    /// is drawn (a violation otherwise): the state check that keeps a marker from passing on an
    /// unrelated label. Fix round 1 (I1): every state has one.
    pub(super) fn check(
        self,
        window: &mut Window,
        cx: &App,
        frame: &[RenderedText],
    ) -> Option<String> {
        use M3a::*;
        let editor = editor_in(window, cx);
        let band = || editor.as_ref().and_then(|e| preview::current(e.read(cx)));
        let console = active_console(window, cx);
        let marks = || editor.as_ref().map_or(0, |e| e.read(cx).marks().len());
        let text = || {
            editor
                .as_ref()
                .map(|e| e.read(cx).snapshot().buffer.to_string())
                .unwrap_or_default()
        };
        let focused = editor
            .as_ref()
            .is_some_and(|e| e.read(cx).focus_handle(cx).is_focused(window));
        let selected = editor
            .as_ref()
            .is_some_and(|e| e.read(cx).status().selected_chars > 0);
        let viewport = window.viewport_size();
        let caret_on_screen = editor.as_ref().is_some_and(|e| {
            e.read(cx)
                .caret_bounds()
                .is_some_and(|b| b.right() <= viewport.width && b.bottom() <= viewport.height)
        });
        let toolbar = || {
            console
                .and_then(|c| Consoles::get(cx, c))
                .and_then(|s| console_fixtures::shown(s.tab, cx))
        };
        let tx = || console.and_then(|c| exec_ui::tx::user_tx(c, cx));
        let session = session_in(window, cx);
        let cursor = || {
            session
                .as_ref()
                .is_some_and(|s| s.state().borrow().cursor.is_some())
        };
        let toast = |id: &str| shows_toast(frame, &message(cx, id, Vec::new()));
        let ok = match self {
            ConsoleEditor | PreviewStatement | PreviewTwoOnLine | ContextDescribe => {
                focused && band().is_some_and(|b| b.style == BandStyle::Run && !b.from_override)
            }
            ConsoleSelection => focused && selected,
            ConsoleUnfocused => !focused && selected,
            ConsoleLongLine | ConsoleManyLines => focused && caret_on_screen,
            ConsoleEmpty => text().is_empty(),
            ConsoleProduction => {
                session
                    .as_ref()
                    .is_some_and(|s| s.state().borrow().read_only)
                    && band().is_some_and(|b| b.style == BandStyle::Run)
            }
            ConsoleMysql => text().contains("/*!40101"),
            ToolbarRunning => toolbar().is_some_and(|t| matches!(t.run, Some(RunUx::Running(3)))),
            ToolbarCancelling => {
                toolbar().is_some_and(|t| matches!(t.run, Some(RunUx::Cancelling)))
            }
            ToolbarReadOnly => toolbar().is_some_and(|t| t.read_only.is_some()),
            ToolbarWaiting | ToolbarProductionWaiting => {
                toolbar().is_some_and(|t| t.waiting == Some((4, 4)))
            }
            PreviewSelection => {
                band().is_some_and(|b| b.style == BandStyle::Run && b.units.len() == 3)
            }
            PreviewAsk => band().is_some_and(|b| b.choices == 2),
            PreviewNothing => band().is_none(),
            PreviewUncertain => band().is_some_and(|b| b.style == BandStyle::Uncertain),
            PreviewOffline => band().is_some_and(|b| b.style == BandStyle::Offline),
            ChooserCandidates | ChooserUncertain => {
                console.is_some_and(|c| exec_ui::chooser::is_open(c, cx))
            }
            ChooserHover => {
                console.is_some_and(|c| exec_ui::chooser::is_open(c, cx))
                    && band().is_some_and(|b| b.from_override && b.units.len() == 2)
            }
            MarksRun | MarksStopped => marks() == 4,
            MarksCancelling | MarksCancelled => marks() == 3,
            MarksStale => stale_first(editor.as_ref(), cx),
            MarksTwoOnRow | MarksLongLine => marks() == 2,
            MarksDenied => console.is_some_and(|c| {
                exec_ui::last_run_marks(c, cx)
                    .iter()
                    .any(|m| matches!(m, Some(ExecMarkState::Failed { .. })))
            }),
            StatusTxOpen => tx().is_some_and(|t| !t.failed && t.statements == 3),
            StatusTxFailed => tx().is_some_and(|t| t.failed),
            StatusCursorOpen => cursor(),
            StatusEverything => {
                tx().is_some_and(|t| !t.failed && t.statements >= 1)
                    && cursor()
                    && console
                        .and_then(|c| hacivat_app::results::context::result_context_of(c, cx))
                        .is_some_and(|rc| {
                            matches!(
                                rc.completeness,
                                hacivat_app::results::CompletenessView::Suspended
                            )
                        })
                    // ## m3a-f2-visual (fix round 1, M2): at 1280 the bar has room for its own
                    // rows · timing cell; only at 960 (in the pseudo-locales) may the tile's
                    // count stand in for it (`required`).
                    && (f32::from(viewport.width) < 1280.0
                        || frame.iter().any(|r| {
                            r.visible && r.painted_px > 0.0 && r.path.ends_with("#status-rows")
                        }))
            }
            StatusProductionResult | Results(_) | GridRange | NoticeReducedMode => true,
            // ## m3b-t25-results (named exception)
            ResultsTabMenu => menu_open(window, cx),
            NoticePasteRefused => toast("hui-editor-paste-refused-title"),
            NoticeNul => text().contains('\0') && toast("hui-editor-nul-title"),
            NoticeClipboard => toast("hui-editor-clipboard-title"),
            // The menu's own row "Describe execution range" (its rows are drawn as button
            // labels, in runs around an access key on Linux and Windows: P15's finder).
            ContextMenu => {
                let drawn: Vec<RenderedText> = frame
                    .iter()
                    .filter(|r| r.visible && r.painted_px > 0.0)
                    .cloned()
                    .collect();
                let row = message(cx, "hv-cmd-editor-describe-exec-range", Vec::new());
                super::support::find_label_runs(&drawn, &row).is_some()
            }
            PromptClose | PromptQuit | PromptFailed | PromptFailedQuit => {
                exec_ui::close_prompt::open_prompt_dialog(cx).is_some()
            }
            CopyCopied => toast("hv-copy-done-line"),
            CopyCopying => shows_toast(
                frame,
                &message(
                    cx,
                    "hv-copy-working",
                    vec![("cells", hacivat_l10n::ArgValue::Count(48_000))],
                ),
            ),
            CopyTooMuch => toast("hv-copy-refused-title"),
            // ## m3b-t22-tx-ui
            TxChipMenu | TxIsolationMenu => {
                menu_open(window, cx)
                    && tx().is_none()
                    && console
                        .is_some_and(|c| hacivat_app::console::tx_badge::view(c, cx).is_some())
            }
            TxBadgeOpen => console.is_some_and(|c| {
                hacivat_app::console::tx_badge::user_tx(c, cx)
                    .is_some_and(|u| !u.failed && u.statements == 3)
            }),
            TxShared => console.is_some_and(|c| {
                hacivat_app::console::tx_badge::user_tx(c, cx).is_some_and(|u| !u.failed)
                    && hacivat_app::console::tx_chip::render_shared(c, false, cx).is_some()
                    // Fix round 1 (I2): the chip (its words or its icon) is painted inside the
                    // window.
                    && hacivat_app::console::tx_badge::bounds_of(
                        c,
                        hacivat_app::console::tx_badge::Focus::Shared,
                        cx,
                    )
                    .is_some_and(|b| {
                        b.size.width > hacivat_ui_core::gpui_surface::px(0.0)
                            && b.right() <= viewport.width
                            && b.bottom() <= viewport.height
                    })
            }),
            TxBadgeFailed => console.is_some_and(|c| {
                hacivat_app::console::tx_badge::user_tx(c, cx).is_some_and(|u| u.failed)
            }),
            TxSegmentMenu => menu_open(window, cx) && tx().is_some_and(|t| t.statements == 3),
            TxIdleToast => toast("hv-tx-idle-body"),
        };
        (!ok).then(|| format!("{}: the state check failed", self.name()))
    }
}

/// The message `id` with `args`, as the app renders it in the shot's locale.
fn message(cx: &App, id: &str, args: Vec<(&'static str, hacivat_l10n::ArgValue)>) -> String {
    let text = cx
        .global::<hacivat_app::l10n_global::L10nGlobal>()
        .l10n
        .t_dynamic(id, args);
    super::strip(&text)
}

/// Whether the frame shows (visible and painted) a label of `file` reading exactly `text`.
fn shows_label(frame: &[RenderedText], file: &str, text: &str) -> bool {
    frame.iter().any(|r| {
        r.visible && r.painted_px > 0.0 && r.path.contains(file) && super::strip(&r.text) == text
    })
}

/// Whether a toast shows `text` (its title or body).
fn shows_toast(frame: &[RenderedText], text: &str) -> bool {
    shows_label(frame, TOAST, text)
}

// ## m3b-t22-tx-ui (named exception)
/// Which transaction menu a state opens.
#[derive(Debug, Clone, Copy)]
enum TxMenu {
    Chip,
    Isolation,
    Segment,
}

/// Opens the active console's transaction menu (from its chip or its status segment) or its
/// isolation menu, and waits for the menu layer.
fn tx_menu(which: TxMenu) -> Step {
    step(
        move |window, cx| {
            let console = active_console(window, cx).expect("a console");
            match which {
                TxMenu::Chip => hacivat_app::console::tx_chip::open_chip_menu(console, window, cx),
                TxMenu::Isolation => {
                    hacivat_app::console::tx_chip::open_isolation_menu(console, window, cx);
                }
                TxMenu::Segment => {
                    hacivat_app::console::tx_chip::open_segment_menu(console, window, cx);
                }
            }
        },
        |window, cx| menu_open(window, cx),
    )
}

/// Whether a popup menu is the window's top layer.
fn menu_open(window: &mut Window, cx: &App) -> bool {
    window
        .root::<Workspace>()
        .flatten()
        .is_some_and(|ws| ws.read(cx).layer_ids().last() == Some(&"menu"))
}

/// Whether the first fixture mark is stale and the second is not (`marks-stale`, M9).
fn stale_first(editor: Option<&Entity<EditorView>>, cx: &App) -> bool {
    editor.is_some_and(|e| {
        let marks = e.read(cx).marks();
        marks.len() == 2
            && marks.is_stale(exec_ui::fixtures::fixture_key(0))
            && !marks.is_stale(exec_ui::fixtures::fixture_key(1))
    })
}

fn results_name(r: ResultsShot) -> &'static str {
    match r {
        ResultsShot::FirstRows => "results-first-rows",
        ResultsShot::Rows => "results-rows",
        ResultsShot::ZeroRows => "results-zero-rows",
        ResultsShot::UpdateCount => "results-update-count",
        ResultsShot::Error => "results-error",
        ResultsShot::TwoResults => "results-two",
        ResultsShot::WideColumns => "results-wide",
        ResultsShot::Streaming => "strip-streaming",
        ResultsShot::Suspended => "strip-suspended",
        ResultsShot::Closed(CloseReason::SessionNeeded) => "strip-closed-session-needed",
        ResultsShot::Closed(CloseReason::IdleTimeout) => "strip-closed-idle",
        ResultsShot::Closed(CloseReason::TxEnded) => "strip-closed-tx-ended",
        ResultsShot::Closed(CloseReason::ConnectionLost) => "strip-closed-connection-lost",
        ResultsShot::Closed(_) => "strip-closed-other",
        ResultsShot::Cancelled => "strip-cancelled",
        ResultsShot::Memory => "strip-memory",
        ResultsShot::MoreResultSets => "strip-more-result-sets",
        ResultsShot::RunLog => "output-run-log",
        ResultsShot::EventsDropped => "output-events-dropped",
        // ## m3b-t25-results (named exception)
        ResultsShot::TitledTabs => "results-titled-tabs",
        ResultsShot::Untyped => "results-untyped",
        ResultsShot::SetLimit => "results-set-limit",
        ResultsShot::Counting => "results-counting",
        ResultsShot::CountExact => "results-count-exact",
        ResultsShot::Evicted => "strip-evicted",
        ResultsShot::FetchAll => "results-fetch-all",
        ResultsShot::Notices => "output-notices",
        ResultsShot::MysqlWarnings => "output-mysql-warnings",
    }
}

fn results_marker(r: ResultsShot) -> (&'static str, Option<&'static str>) {
    match r {
        // Surface 10's grid: a header it drew (data).
        ResultsShot::Rows | ResultsShot::TwoResults => (GRID_HEADER, Some("customer")),
        ResultsShot::FirstRows | ResultsShot::ZeroRows => (GRID_HEADER, Some("token")),
        ResultsShot::WideColumns => (GRID_HEADER, Some("c0")),
        // Surface 11's strip.
        ResultsShot::Streaming
        | ResultsShot::Suspended
        | ResultsShot::Closed(_)
        | ResultsShot::Cancelled
        | ResultsShot::Memory
        | ResultsShot::MoreResultSets => (STRIP, None),
        // Surface 12's log.
        ResultsShot::UpdateCount
        | ResultsShot::Error
        | ResultsShot::RunLog
        | ResultsShot::EventsDropped => (OUTPUT, None),
        // ## m3b-t25-results (named exception)
        ResultsShot::TitledTabs => ("#results-tab-0", None),
        ResultsShot::Untyped => ("#results-tab-0-untyped", None),
        ResultsShot::SetLimit => ("#results-set-limit", None),
        ResultsShot::Counting | ResultsShot::CountExact => ("#results-count-state", None),
        ResultsShot::Evicted => (STRIP, None),
        ResultsShot::FetchAll => (DIALOG, None),
        ResultsShot::Notices => ("#log-notice-0", None),
        ResultsShot::MysqlWarnings => ("#log-code-2", None),
    }
}

// ---- steps ----------------------------------------------------------------------------------

/// What a step does in the window.
type Act = Box<dyn Fn(&mut Window, &mut App)>;
/// Whether the step has landed.
type Ready = Box<dyn Fn(&mut Window, &mut App) -> bool>;

/// What a step does, then what it waits for.
pub(super) struct Step {
    pub(super) act: Act,
    pub(super) ready: Ready,
}

fn step(
    act: impl Fn(&mut Window, &mut App) + 'static,
    ready: impl Fn(&mut Window, &mut App) -> bool + 'static,
) -> Step {
    Step {
        act: Box::new(act),
        ready: Box::new(ready),
    }
}

/// A step that is done once the app is parked.
fn instant(act: impl Fn(&mut Window, &mut App) + 'static) -> Step {
    step(act, |_, _| true)
}

/// What the band must show before the shot is taken.
#[derive(Debug, Clone, Copy)]
enum Expect {
    /// Nothing to wait for (an empty console has no band and no sentence).
    Any,
    Band(BandStyle),
    /// `n` units (a selection run as separate statements).
    Units(usize),
    /// Run would open the chooser.
    Choices,
    /// No band: "Nothing to run here".
    Nothing,
}

/// A live console of `name` over `text`, the selection `sel`, the editor focused; the band as
/// `expect` says.
fn live(
    name: &'static str,
    text: &'static str,
    sel: std::ops::Range<usize>,
    expect: Expect,
) -> Vec<Step> {
    vec![connect(name), set_text(text, sel, expect)]
}

/// Saves the profile `name` and connects it with `Console 1` in front: the connect binds it,
/// its host opens the session, and the run UI reads the SQL context (the editor is then
/// connected).
fn connect(name: &'static str) -> Step {
    step(
        move |_, cx| {
            let id = save(cx, profile(name));
            ConnectionsModel::connect(cx, id, None);
        },
        move |window, cx| {
            let Some(id) = profile_named(cx, name) else {
                return false;
            };
            let live = cx.global::<ConnectionsModel>().live(id);
            assert!(
                !matches!(live, LiveState::Failed(_)),
                "{name} failed to connect: {live:?}"
            );
            matches!(live, LiveState::Connected(_))
                && active_console(window, cx).is_some_and(|c| exec_ui::connected(c, cx))
        },
    )
}

/// Saves the profile `name` (never connected).
fn save_only(name: &'static str, _env: EnvTag) -> Step {
    instant(move |_, cx| {
        save(cx, profile(name));
    })
}

/// Sets the active console's text and selection and focuses its editor; waits until the band
/// shows what `expect` says.
fn set_text(text: &'static str, sel: std::ops::Range<usize>, expect: Expect) -> Step {
    step(
        move |window, cx| {
            let editor = editor_in(window, cx).expect("the active tab is a console");
            editor.update(cx, |e, cx| {
                e.set_text(text, cx);
                e.select_range(sel.clone(), cx);
            });
            let handle = editor.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        },
        move |window, cx| {
            let Some(editor) = editor_in(window, cx) else {
                return false;
            };
            let e = editor.read(cx);
            if e.snapshot().buffer.len_bytes() != text.len() {
                return false;
            }
            let band = preview::current(e);
            match expect {
                Expect::Any => true,
                Expect::Band(style) => band.is_some_and(|b| b.style == style),
                Expect::Units(n) => band.is_some_and(|b| b.units.len() == n),
                Expect::Choices => band.is_some_and(|b| b.choices > 0),
                Expect::Nothing => preview::status_message(e, cx).is_some(),
            }
        },
    )
}

/// A console bound to `name`, unconnected, over `text` (Task 32's fixture): the band is offline.
fn offline(text: &'static str, caret: usize) -> Vec<Step> {
    vec![
        save_only(DEV, EnvTag::Dev),
        step(
            |window, cx| {
                let id = profile_named(cx, DEV).expect("saved");
                console_fixtures::open(id, text, window, cx);
            },
            |window, cx| editor_in(window, cx).is_some(),
        ),
        step(
            move |window, cx| {
                let editor = editor_in(window, cx).expect("a console");
                editor.update(cx, |e, cx| e.select_range(caret..caret, cx));
                let handle = editor.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            },
            |window, cx| {
                editor_in(window, cx).is_some_and(|e| {
                    preview::current(e.read(cx)).is_some_and(|b| b.style == BandStyle::Offline)
                })
            },
        ),
    ]
}

/// A live console of `name` over the script, then `show` on its tab's toolbar.
fn toolbar(name: &'static str, show: fn(hacivat_ui::panes::TabId, &mut App)) -> Vec<Step> {
    let mut s = live(
        name,
        SCRIPT,
        after(SCRIPT, "o.total")..after(SCRIPT, "o.total"),
        Expect::Band(BandStyle::Run),
    );
    s.push(instant(move |window, cx| {
        let tab = window
            .root::<Workspace>()
            .flatten()
            .and_then(|ws| ws.read(cx).active_tab())
            .expect("an active tab");
        show(tab, cx);
    }));
    s
}

/// Runs `editor.run` in the active console and waits for the chooser.
fn run_to_chooser() -> Step {
    step(
        |window, cx| {
            let editor = editor_in(window, cx).expect("a console");
            editor.update(cx, |e, cx| assert!(e.run_command("editor.run", window, cx)));
        },
        |window, cx| active_console(window, cx).is_some_and(|c| exec_ui::chooser::is_open(c, cx)),
    )
}

/// Enter in the open chooser runs the preselected `BEGIN` on a production console: nothing is
/// sent; the mark says why.
fn run_denied() -> Step {
    step(
        |window, cx| {
            window.dispatch_keystroke(Keystroke::parse("enter").expect("a keystroke"), cx);
        },
        |window, cx| {
            active_console(window, cx).is_some_and(|c| {
                exec_ui::last_run_marks(c, cx)
                    .iter()
                    .any(|m| matches!(m, Some(ExecMarkState::Failed { .. })))
            })
        },
    )
}

/// Runs the editor command `id` in the active console.
fn editor_command(id: &'static str) -> Step {
    instant(move |window, cx| {
        let editor = editor_in(window, cx).expect("a console");
        editor.update(cx, |e, cx| assert!(e.run_command(id, window, cx), "{id}"));
    })
}

/// Presses `keys` in the window (the focused element hears them).
fn key(keys: &'static str, ready: impl Fn(&mut Window, &mut App) -> bool + 'static) -> Step {
    step(
        move |window, cx| {
            let k = Keystroke::parse(keys).expect("a keystroke");
            window.dispatch_keystroke(k, cx);
        },
        ready,
    )
}

/// Types `text` at byte `at` of the active editor (the platform's text input path).
fn type_at(
    at: usize,
    text: &'static str,
    ready: impl Fn(&mut Window, &mut App) -> bool + 'static,
) -> Step {
    step(
        move |window, cx| {
            let editor = editor_in(window, cx).expect("a console");
            editor.update(cx, |e, cx| {
                e.select_range(at..at, cx);
                e.replace_text_in_range(None, text, window, cx);
            });
        },
        ready,
    )
}

/// Pushes a toast and waits for it to show.
fn toast(push: fn(&mut App)) -> Step {
    step(move |_, cx| push(cx), |_, _| true)
}

/// A live console over `text` with marks over each `(statement, status)`.
fn marks(text: &'static str, units: Vec<(&'static str, (UnitStatus, Duration))>) -> Vec<Step> {
    let mut s = live(DEV, text, text.len()..text.len(), Expect::Nothing);
    s.push(show_marks(
        units
            .into_iter()
            .map(|(stmt, status)| (range_of(text, stmt), status))
            .collect(),
    ));
    s
}

/// Places the marks (`exec_ui::fixtures::show_marks`) on the active console.
fn show_marks(units: Vec<(std::ops::Range<usize>, (UnitStatus, Duration))>) -> Step {
    let n = units.len();
    let units = std::cell::RefCell::new(Some(units));
    step(
        move |window, cx| {
            let console = active_console(window, cx).expect("a console");
            if let Some(units) = units.borrow_mut().take() {
                let placed = exec_ui::fixtures::show_marks(
                    console,
                    units.into_iter().map(|(r, (s, d))| (r, s, d)).collect(),
                    cx,
                );
                assert_eq!(placed, n, "every mark placed");
            }
        },
        |_, _| true,
    )
}

fn rows(n: u64) -> RowsOutcome {
    RowsOutcome::Fetched {
        rows: n,
        more: false,
    }
}

fn succeeded(ms: u64, rows: Option<RowsOutcome>) -> (UnitStatus, Duration) {
    (
        UnitStatus::Succeeded(UnitSummary {
            rows,
            exec_ms: ms,
            fetch_ms: 0,
        }),
        Duration::from_millis(ms),
    )
}

fn running(ms: u64) -> (UnitStatus, Duration) {
    (
        UnitStatus::Running {
            since: Instant::now(),
        },
        Duration::from_millis(ms),
    )
}

fn failed(code: &str, server: &str) -> (UnitStatus, Duration) {
    (
        UnitStatus::Failed(ErrorView {
            title_id: "exec-error-server",
            code: Some(code.into()),
            server_text: Some(server.into()),
            hint_id: None,
            doc_offset: None,
            // ## m3b-t25-results (named exception)
            vendor_code: None,
        }),
        Duration::from_millis(9),
    )
}

// ---- transactions, cursors and prompts ------------------------------------------------------

const UPD1: &str = "UPDATE orders SET status = 'shipped' WHERE id = 1042";
const UPD2: &str = "UPDATE orders SET status = 'shipped' WHERE id = 1043";
const UPD3: &str = "UPDATE order_lines SET qty = qty - 1 WHERE order_id = 1042";
/// A statement the fake fails, leaving the transaction failed (as PostgreSQL does).
const FAILS: &str = "UPDATE inventory SET qty = qty - 1 WHERE sku = 'TR-1042'";

/// Sends `sqls` on the active console's session as the user's statements, in order; waits for
/// its user transaction (`statements`, or failed).
fn user_tx(sqls: &'static [&'static str], statements: Option<u32>, failed: bool) -> Step {
    step(
        move |window, cx| {
            let session = session_in(window, cx).expect("the console's session");
            for sql in sqls {
                // The ticket's reply is not awaited: the session runs its requests in order.
                let _ticket = session.submit(Request::Exec {
                    sql: sql.to_string(),
                    opts: ExecOpts::default(),
                    origin: ExecOrigin::User,
                });
            }
        },
        move |window, cx| {
            active_console(window, cx)
                .and_then(|c| exec_ui::tx::user_tx(c, cx))
                .is_some_and(|t| t.failed == failed && statements.is_none_or(|n| t.statements == n))
        },
    )
}

/// Opens the close prompt (the tab's close guard) or the quit prompt (the quit guard).
fn prompt(quit: bool) -> Step {
    step(
        move |window, cx| {
            if quit {
                let decision = exec_ui::close_prompt::quit_guard(cx);
                assert!(matches!(decision, hacivat_app::shell::CloseDecision::Defer));
            } else {
                assert!(ActionRegistry::run("workspace.close_tab", window, cx));
            }
        },
        |_, cx| exec_ui::close_prompt::open_prompt_dialog(cx).is_some(),
    )
}

/// Opens a cursor on the active console's session (a suspendable read that keeps rows on the
/// server): the status bar's "Cursor open".
fn open_cursor() -> Step {
    step(
        |window, cx| {
            let session = session_in(window, cx).expect("the console's session");
            let run = RunId(0xC0FFEE);
            let _ticket = session.run(RunSpec {
                run,
                units: vec![RunUnit::plain(
                    UnitId { run, ordinal: 0 },
                    BIG.into(),
                    Protocol::Extended,
                    true,
                )]
                .into(),
                policy: ErrorPolicy::StopOnError,
                fetch: FetchPolicy::default(),
                observer: ObserverRef(Arc::new(Stores::default())),
                // ## m3b-t01c-core-seams
                confirm_deadline: hacivat_session::session::RunSpec::CONFIRM_DEADLINE,
            });
        },
        |window, cx| session_in(window, cx).is_some_and(|s| s.state().borrow().cursor.is_some()),
    )
}

/// The read [`open_cursor`] runs.
const BIG: &str = "SELECT id FROM generate_series(1, 1000000) AS g(id)";

/// Opens a store for each result of a session-level run.
#[derive(Default)]
struct Stores {
    next: AtomicU64,
}

impl RunObserver for Stores {
    fn open_result(&self, _: UnitId, _: u32, schema: Arc<ResultSchema>) -> ResultWriter {
        let id = ResultId(1 + self.next.fetch_add(1, Ordering::SeqCst));
        let budget = MemoryBudget::new(1 << 30, 256 << 20);
        let (_store, writer) = ResultStore::new(id, schema, budget, 0);
        writer
    }

    fn event(&self, _: &RunEvent) {}
}

/// Nobody fetches the fixture result's cursor.
struct Inert;

impl FetchControl for Inert {
    fn fetch_more(&self, _: ResultId, _: u64) {}
    fn close(&self, _: ResultId, _: CloseReason) {}
    fn fetch_all(&self, _: ResultId) {}
}

/// Gives the active console a suspended 10,000-row result, timed 12 ms + 119 ms, active (the
/// status bar's rows and timing).
fn suspended_tab() -> Step {
    step(
        |window, cx| {
            let console = active_console(window, cx).expect("a console");
            let schema = Arc::new(ResultSchema {
                columns: vec![ColumnMeta {
                    name: "id".into(),
                    logical: LogicalType::Int {
                        bits: 64,
                        signed: true,
                    },
                    native: NativeType::Pg {
                        oid: 20,
                        typmod: -1,
                        name: "int8".into(),
                    },
                    origin: None,
                }]
                .into(),
            });
            let values = (0..10_000).map(|r| vec![Value::Int(r)]).collect();
            let now = Instant::now();
            let store = ResultStore::from_rows(
                schema,
                values,
                Completeness::Suspended(CursorInfo {
                    opened_at: now,
                    last_fetch: now,
                    holds_tx: true,
                    idle_bound: Duration::from_secs(600),
                }),
            );
            let key = TabKey {
                console: console.0,
                unit_ordinal: 0,
                result_ordinal: 0,
            };
            let tab = ResultTab {
                key,
                title: "".into(),
                unit: UnitId {
                    run: RunId(1),
                    ordinal: 0,
                },
                source: StoreSource::new(store, Arc::new(Inert), FetchPolicy::default()),
            };
            let results = Consoles::get(cx, console)
                .and_then(|s| s.results.clone())
                .expect("the console has results");
            results.update(cx, |r, cx| {
                r.apply(&[tab], cx);
                r.set_timing(key, Some(12), Some(119), cx);
                r.set_active(ActiveResult::Tab(0), cx);
            });
        },
        |_, _| true,
    )
}

// ---- results --------------------------------------------------------------------------------

/// Task 30's results fixture `shot`, on a console of a saved profile in `env`.
fn results(shot: ResultsShot, env: Option<EnvTag>) -> Vec<Step> {
    let name = match env {
        Some(EnvTag::Production) => PROD,
        _ => DEV,
    };
    vec![
        save_only(name, EnvTag::Dev),
        step(
            move |window, cx| {
                let id = profile_named(cx, name);
                open_results_shot(shot, id, window, cx).expect("the results fixture opens");
            },
            |_, _| true,
        ),
    ]
}

/// Selects a 3 × 2 range in the shown grid through the grid's commands (from row 2, column 2).
fn grid_range() -> Step {
    step(
        |window, cx| {
            let grid = hosts(cx)
                .into_iter()
                .find_map(|h| {
                    let shown = h.read(cx).shown()?;
                    h.read(cx).grid_of(shown, 0, cx)
                })
                .expect("the tile shows a grid");
            let handle = grid.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
            for id in [
                "grid.move_down",
                "grid.move_right",
                "grid.select_down",
                "grid.select_down",
                "grid.select_right",
            ] {
                assert!(ActionRegistry::run(id, window, cx), "{id}");
            }
        },
        |_, _| true,
    )
}

// ---- the connections ------------------------------------------------------------------------

/// `FakeDriver` presented as a driver that dials its own direct target, so the pipeline opens
/// no stream and the fake never dials (as `support`'s test shells do).
struct DialsItself {
    fake: Arc<FakeDriver>,
    descriptor: hacivat_driver::DriverDescriptor,
}

impl hacivat_driver::Driver for DialsItself {
    fn descriptor(&self) -> &hacivat_driver::DriverDescriptor {
        &self.descriptor
    }
    fn connect(
        &self,
        req: hacivat_driver::DriverConnect,
    ) -> futures::future::BoxFuture<
        'static,
        Result<Box<dyn hacivat_driver::RawConn>, hacivat_driver::RawError>,
    > {
        hacivat_driver::Driver::connect(self.fake.as_ref(), req)
    }
}

fn dials_itself(fake: FakeDriver) -> Arc<DialsItself> {
    let fake = Arc::new(fake);
    let mut descriptor = fake.descriptor().clone();
    descriptor.caps.stream_injection = false;
    Arc::new(DialsItself { fake, descriptor })
}

/// An `execute` answer: one value, or an error that leaves the transaction failed.
fn exec_answer(prefix: &'static str, value: Option<&str>) -> FakeExec {
    FakeExec {
        prefix,
        result: Ok(ExecSummary {
            rows: vec![vec![value.map(str::to_string)]],
            row_count: 1,
            ..ExecSummary::default()
        }),
        delay: Duration::ZERO,
        tx_after: None,
        panics: false,
    }
}

/// The M3a shots' connections: PostgreSQL and MySQL answer the console's SQL-context probe
/// like servers (a PostgreSQL read-only session's through the guard's typed query) and dial
/// themselves; PostgreSQL also fails [`FAILS`] inside a transaction and answers [`BIG`] with a
/// million rows (a cursor).
pub(super) fn connections(paths: &LocalPaths) -> ConnectionsBoot {
    let big = FakeQuery {
        prefix: "SELECT id FROM generate_series",
        sets: vec![FakeResultSet {
            columns: vec![(
                "id",
                LogicalType::Int {
                    bits: 64,
                    signed: true,
                },
            )],
            rows: FakeRows::Generated {
                count: 1_000_000,
                cell: |row, _| Value::Int(row as i64),
            },
        }],
        ..Default::default()
    };
    let pg = super::support::answers_probe_with(
        FakeDriver::new(DbmsKind::Postgres).tracks_transactions(),
        vec![big],
    );
    let mut exec: Vec<FakeExec> = (0..32)
        .flat_map(|_| {
            [
                exec_answer("SHOW standard_conforming_strings", Some("on")),
                exec_answer("SELECT pg_catalog.txid_current_if_assigned()", None),
            ]
        })
        .collect();
    exec.push(FakeExec {
        prefix: "UPDATE inventory",
        result: Err(hacivat_driver::RawError {
            kind: hacivat_driver::RawErrorKind::Db,
            server: None,
        }),
        delay: Duration::ZERO,
        tx_after: Some(TxStatus::Failed),
        panics: false,
    });
    let pg = pg.exec(exec);
    let mysql = FakeDriver::new(DbmsKind::MySql).tracks_transactions().exec(
        (0..32)
            .map(|_| exec_answer("SELECT @@SESSION.sql_mode", Some("STRICT_TRANS_TABLES")))
            .collect(),
    );
    let mut drivers = DriverRegistry::new();
    drivers.register(dials_itself(pg));
    drivers.register(dials_itself(mysql));
    for kind in [DbmsKind::MariaDb, DbmsKind::Sqlite] {
        drivers.register(Arc::new(FakeDriver::new(kind)));
    }
    ConnectionsBoot::for_tests(paths, drivers)
}

/// The profile `name`: `orders-dev` (dev), `orders-prod` (production) on PostgreSQL,
/// `shop-dev` (dev) on MySQL; IP literals, so nothing resolves.
fn profile(name: &str) -> Profile {
    let (env, engine) = match name {
        PROD => (EnvTag::Production, DbmsKind::Postgres),
        SHOP => (EnvTag::Dev, DbmsKind::MySql),
        _ => (EnvTag::Dev, DbmsKind::Postgres),
    };
    Profile {
        name: name.to_string(),
        engine,
        environment: Some(env),
        // ## m3b-t22-tx-ui (named exception): the single-session connection.
        single_session: name == SHARED,
        endpoint: ProfileEndpoint::Server {
            host: "10.0.4.12".to_string(),
            port: None,
        },
        database: Some(
            if engine == DbmsKind::MySql {
                "shop"
            } else {
                "orders"
            }
            .to_string(),
        ),
        user: Some("deploy".to_string()),
        auth: hacivat_secrets::AuthProviderId::None,
        ..Profile::default()
    }
}

fn save(cx: &mut App, p: Profile) -> ProfileId {
    let id = p.id;
    ConnectionsModel::save(
        cx,
        ProfileDraft {
            is_new: true,
            ..ProfileDraft::edit(p)
        },
    )
    .detach();
    id
}

fn profile_named(cx: &App, name: &str) -> Option<ProfileId> {
    cx.global::<ConnectionsModel>()
        .profiles()
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.id)
}

/// The console of the focused pane's active tab.
fn active_console(window: &mut Window, cx: &App) -> Option<ConsoleId> {
    let ws = window.root::<Workspace>().flatten()?;
    let tab = ws.read(cx).active_tab()?;
    Consoles::by_tab(cx, tab)
}

/// The active console's editor.
fn editor_in(window: &mut Window, cx: &App) -> Option<Entity<EditorView>> {
    Consoles::get(cx, active_console(window, cx)?)?
        .editor
        .clone()
}

/// The active console's session.
fn session_in(window: &mut Window, cx: &App) -> Option<SessionHandle> {
    Consoles::get(cx, active_console(window, cx)?)?
        .session
        .clone()
}
