//! The M3b shots (Task 29): `design/m3b-safe` surfaces 1–13, each state at 1280x800 and the
//! 960x600 minimum, in every locale and theme, after every M3a shot.
//!
//! Tasks 21 to 27 drew these states; the ones they parked in `shots/m3a.rs` (Tasks 22, 23, 25,
//! 26 and 27) and the dialog fixtures `shots.rs` opened (Tasks 21 and 24) live here now, with
//! the states no task drew yet (Task 29's). Two kinds of state:
//!
//! - **Fixtures** (surfaces 1, 2, 8 and 9): the dialog, prompt or menu opens through its task's
//!   `fixtures::open` over a fresh shell, as before.
//! - **Steps** (the rest): the M3a machinery (`m3a::live`, `m3a::results`, …) reaches the state
//!   over a live console of the fake connections, each step followed by a bounded wait on its
//!   condition.
//!
//! Each state has a marker (a label it draws, with its data text where the label's site draws
//! more than this state) plus the labels it must also show ([`M3b::also_shown`]); no two states
//! share that signature, except the few told apart only by their model check (a focus ring, a
//! typed value, a highlighted row: [`TOLD_APART_BY_CHECK`]). Each state also has a model check
//! ([`M3b::check`]) read from the app's models after the frame, and the trust signals it carries
//! ([`M3b::required`]: production, transaction, read-only).
#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

use std::time::Duration;

use hacivat_app::console::Consoles;
use hacivat_app::console::toolbar::RunUx;
use hacivat_app::console::view::fixtures as console_fixtures;
use hacivat_app::exec_ui::{self, commit_prompt, confirm, params_prompt, policy_popover};
use hacivat_app::results::view::ResultsShot;
use hacivat_app::shell::Workspace;
use hacivat_exec::UnitStatus;
use hacivat_proto::EnvTag;
use hacivat_session::session::{TxCommand, TxMode};
use hacivat_ui::popover::A11yDecl;
use hacivat_ui::text::RenderedText;
use hacivat_ui::toast::ToastQueue;
use hacivat_ui_core::gpui_surface::{App, Keystroke, Window};
use hacivat_ui_editor::ExecMarkState;
use hacivat_ui_editor::preview::{self, BandStyle};

use super::m3a::{
    self, DEV, DIALOG, Expect, FAILS, MARKS, MARKS_RS, PROD, SCRIPT, SHARED, Step, TxMenu, UPD1,
    UPD2, UPD3, active_console, after, at, drawn, editor_in, instant, live, marks, menu_open,
    message, results, rows, session_in, shows_toast, step, succeeded, tx_menu, user_tx,
};

use confirm::fixtures::State as Cf;
use params_prompt::fixtures::State as Pf;
use policy_popover::fixtures::State as Ch;

/// The read-only connection (Task 29's chip state).
pub(super) const READ_ONLY: &str = "orders-ro";

// ---- the states -----------------------------------------------------------------------------

/// One M3b state (a shot surface), named after its `design/m3b-safe` surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum M3b {
    // 1, 2 · Gate confirmation and confirm-in-transaction dialogs (Task 21's fixtures)
    Confirm(confirm::fixtures::State),
    Commit(commit_prompt::fixtures::State),
    // 3, 4, 5, 12 · Transaction chip, badge, idle toast, shared session (Task 22)
    TxChipMenu,
    TxIsolationMenu,
    TxBadgeOpen,
    TxBadgeFailed,
    TxSegmentMenu,
    TxIdleToast,
    TxShared,
    // 6, 7 · Cancel escalation and the error bar (Task 23)
    EscSlowToast,
    EscDialog,
    EscDialogFailed,
    EscDialogLocal,
    EscDialogProduction,
    EscKeptWaiting,
    ErrBar,
    ErrBarFailedTx,
    ErrBarFocused,
    // 8, 9 · Parameter prompt and chooser additions (Task 24's fixtures)
    Params(params_prompt::fixtures::State),
    Chooser(policy_popover::fixtures::State),
    // 10 · Results additions (Task 30's results fixtures with Task 25's states)
    Results(ResultsShot),
    ResultsTabMenu,
    // 11 · History dialog (Task 27)
    HistList,
    HistSearch,
    HistAllFailed,
    HistRedacted,
    HistOther,
    HistEmpty,
    HistNoMatch,
    // 13 · Production console after the unlock (Task 26)
    TxChipProduction,
    // Task 29: surface states no task drew yet.
    /// 3 · A read-only session: the chip says Auto-commit, disabled.
    TxChipReadOnly,
    /// 3 · Smart mode in the chip.
    TxChipSmart,
    /// 9 · "Selection: As one": the band covers the selection as one unit.
    PreviewAsOne,
    /// 12 · The connection dialog's Advanced › Sessions row, checked.
    ConnSessions,
}

/// Task 22's states (first of the M3b states).
pub(super) const T22: [M3b; 7] = [
    M3b::TxChipMenu,
    M3b::TxIsolationMenu,
    M3b::TxBadgeOpen,
    M3b::TxBadgeFailed,
    M3b::TxSegmentMenu,
    M3b::TxIdleToast,
    M3b::TxShared,
];

/// Task 25's states.
pub(super) const T25: [M3b; 11] = [
    M3b::Results(ResultsShot::TitledTabs),
    M3b::ResultsTabMenu,
    M3b::Results(ResultsShot::Untyped),
    M3b::Results(ResultsShot::SetLimit),
    M3b::Results(ResultsShot::Counting),
    M3b::Results(ResultsShot::CountExact),
    M3b::Results(ResultsShot::Evicted),
    M3b::Results(ResultsShot::FetchAll),
    M3b::Results(ResultsShot::Notices),
    M3b::Results(ResultsShot::MysqlWarnings),
    M3b::Results(ResultsShot::CrowdedTabs),
];

/// Task 23's states.
pub(super) const T23: [M3b; 9] = [
    M3b::EscSlowToast,
    M3b::EscDialog,
    M3b::EscDialogFailed,
    M3b::EscDialogLocal,
    M3b::EscDialogProduction,
    M3b::EscKeptWaiting,
    M3b::ErrBar,
    M3b::ErrBarFailedTx,
    M3b::ErrBarFocused,
];

/// Task 27's states.
pub(super) const T27: [M3b; 7] = [
    M3b::HistList,
    M3b::HistSearch,
    M3b::HistAllFailed,
    M3b::HistRedacted,
    M3b::HistOther,
    M3b::HistEmpty,
    M3b::HistNoMatch,
];

/// Task 26's state.
pub(super) const T26: [M3b; 1] = [M3b::TxChipProduction];

/// Task 29's states (last).
pub(super) const T29: [M3b; 4] = [
    M3b::TxChipReadOnly,
    M3b::TxChipSmart,
    M3b::PreviewAsOne,
    M3b::ConnSessions,
];

/// Task 21's states: every confirmation state, then every commit-prompt state.
pub(super) fn t21() -> Vec<M3b> {
    confirm::fixtures::STATES
        .into_iter()
        .map(M3b::Confirm)
        .chain(commit_prompt::fixtures::STATES.into_iter().map(M3b::Commit))
        .collect()
}

/// Task 24's states: every parameter-prompt state, then every chooser-addition state.
pub(super) fn t24() -> Vec<M3b> {
    params_prompt::fixtures::STATES
        .into_iter()
        .map(M3b::Params)
        .chain(
            policy_popover::fixtures::STATES
                .into_iter()
                .map(M3b::Chooser),
        )
        .collect()
}

/// Every M3b state in render order: the order the tasks added them (22, 25, 21, 23, 24, 27,
/// 26), then Task 29's.
pub(super) fn all() -> Vec<M3b> {
    T22.into_iter()
        .chain(T25)
        .chain(t21())
        .chain(T23)
        .chain(t24())
        .chain(T27)
        .chain(T26)
        .chain(T29)
        .collect()
}

/// The states whose marker signature is shared, told apart only by their model check: a
/// typed value the field does not record as a label, a highlighted row or a disabled row's
/// reason (menu rows draw no ids), a focus ring, which lost-connection sentence the body reads
/// (Fluent, locale-dependent).
pub(super) const TOLD_APART_BY_CHECK: [&[&str]; 5] = [
    &["confirm-type", "confirm-typed"],
    &["commit-lost-own", "commit-lost-savepoint"],
    &["errbar", "errbar-focused"],
    &["params-two-names", "params-null"],
    &[
        "chooser-as-one",
        "chooser-as-one-highlighted",
        "chooser-as-one-read-only",
    ],
];

// ---- names, markers, trust ------------------------------------------------------------------

/// A label's element ID suffix, as markers name them.
const SQL_LINE: &str = "#confirm-sql-line";
/// The SQL every parameter state but two shows (its last line, data).
const PARAMS_SQL: &str = "WHERE musteri_id = :musteri_id AND tarih >= :baslangic";

impl M3b {
    pub(super) fn name(self) -> &'static str {
        match self {
            M3b::Confirm(s) => s.name(),
            M3b::Commit(s) => s.name(),
            M3b::TxChipMenu => "tx-chip-menu",
            M3b::TxIsolationMenu => "tx-isolation-menu",
            M3b::TxBadgeOpen => "tx-badge-open",
            M3b::TxBadgeFailed => "tx-badge-failed",
            M3b::TxSegmentMenu => "tx-segment-menu",
            M3b::TxIdleToast => "tx-idle-toast",
            M3b::TxShared => "tx-shared",
            M3b::EscSlowToast => "esc-slow-toast",
            M3b::EscDialog => "esc-dialog",
            M3b::EscDialogFailed => "esc-dialog-failed",
            M3b::EscDialogLocal => "esc-dialog-local",
            M3b::EscDialogProduction => "esc-dialog-production",
            M3b::EscKeptWaiting => "esc-kept-waiting",
            M3b::ErrBar => "errbar",
            M3b::ErrBarFailedTx => "errbar-failed-tx",
            M3b::ErrBarFocused => "errbar-focused",
            M3b::Params(s) => s.name(),
            M3b::Chooser(s) => s.name(),
            M3b::Results(r) => results_name(r),
            M3b::ResultsTabMenu => "results-tab-menu",
            M3b::HistList => "history-list",
            M3b::HistSearch => "history-search",
            M3b::HistAllFailed => "history-all-failed",
            M3b::HistRedacted => "history-redacted",
            M3b::HistOther => "history-other",
            M3b::HistEmpty => "history-empty",
            M3b::HistNoMatch => "history-nomatch",
            M3b::TxChipProduction => "tx-chip-production",
            M3b::TxChipReadOnly => "tx-chip-read-only",
            M3b::TxChipSmart => "tx-chip-smart",
            M3b::PreviewAsOne => "preview-as-one",
            M3b::ConnSessions => "conn-dialog-sessions",
        }
    }

    /// Whether the state opens through its task's fixture (no live console).
    pub(super) fn is_fixture(self) -> bool {
        matches!(
            self,
            M3b::Confirm(_) | M3b::Commit(_) | M3b::Params(_) | M3b::Chooser(_)
        )
    }

    /// Opens a fixture state over the fresh shell (the others are reached by [`M3b::steps`]).
    pub(super) fn open(self, window: &mut Window, cx: &mut App) {
        match self {
            M3b::Confirm(s) => confirm::fixtures::open(s, window, cx),
            M3b::Commit(s) => commit_prompt::fixtures::open(s, window, cx),
            M3b::Params(s) => params_prompt::fixtures::open(s, window, cx),
            M3b::Chooser(s) => policy_popover::fixtures::open(s, window, cx),
            _ => {}
        }
    }

    /// The label that proves the state is on screen: its creation site (a `#id` or a source
    /// file) and, where that site draws more than this state, its data text (the same in every
    /// locale).
    pub(super) fn marker(self) -> (&'static str, Option<&'static str>) {
        match self {
            // 1 · the statement's SQL (data), or a label only this state draws.
            M3b::Confirm(s) => match s {
                Cf::Unknown => (SQL_LINE, Some("UPDATE orders")),
                Cf::Multi => (SQL_LINE, Some("DROP TABLE order_archive_2019;")),
                Cf::MultiType | Cf::Type | Cf::Typed => (SQL_LINE, Some("DELETE FROM müşteriler;")),
                Cf::Mismatch => ("#confirm-type-hint", None),
                Cf::DropSchema => (SQL_LINE, Some("DROP SCHEMA reporting CASCADE;")),
                Cf::MysqlDdl => (SQL_LINE, Some("ALTER TABLE orders DROP COLUMN legacy_ref;")),
                Cf::ImplicitCommit => (
                    SQL_LINE,
                    Some("CREATE INDEX idx_orders_created ON orders (created_at);"),
                ),
                Cf::Routine => (SQL_LINE, Some("CALL close_month('2026-10');")),
                Cf::Calm => ("#confirm-suppress-scope", None),
                Cf::Warn => (
                    SQL_LINE,
                    Some("SELECT * FROM orders WHERE id = 4211 FOR UPDATE;"),
                ),
                Cf::Explain => (
                    SQL_LINE,
                    Some("EXPLAIN ANALYZE UPDATE orders SET total = recalc_total(id);"),
                ),
                Cf::SuppressProd => (
                    SQL_LINE,
                    Some("CREATE INDEX CONCURRENTLY idx_orders_customer ON orders (customer_id);"),
                ),
                Cf::NotAnalysed => (
                    SQL_LINE,
                    Some(
                        "/*!50001 CREATE ALGORITHM=UNDEFINED VIEW v_orders AS SELECT * FROM orders */;",
                    ),
                ),
                Cf::MultiStaging => ("#confirm-execute-all-scope", None),
            },
            // 2 · each form's own buttons; the lost forms their body.
            M3b::Commit(s) => match s {
                commit_prompt::fixtures::State::Savepoint => ("#button-label-commit-undo", None),
                commit_prompt::fixtures::State::Own => ("#button-label-commit-rollback", None),
                commit_prompt::fixtures::State::LostOwn
                | commit_prompt::fixtures::State::LostSavepoint => ("#commit-dropped-body", None),
            },
            // 3, 4, 5, 12
            M3b::TxChipMenu => ("#popup-menu-title", None),
            // The isolation levels are SQL (data, untranslated).
            M3b::TxIsolationMenu => ("#menu-item-label", Some("SERIALIZABLE")),
            M3b::TxBadgeOpen => ("#tx-badge-text", None),
            M3b::TxBadgeFailed => ("#tx-badge-failed", None),
            M3b::TxSegmentMenu => ("#popup-menu-title", None),
            // Keep open, the idle toast's third action.
            M3b::TxIdleToast => ("#button-label-toast-action-1-2", None),
            // The shared connection's name (data): at 960 in the pseudo-locales the chip is
            // icon-only (its words stay in the tooltip), so the check asks for it painted.
            M3b::TxShared => ("#console-connection-name", Some(SHARED)),
            // 6, 7
            M3b::EscSlowToast => ("#button-label-toast-action-1-0", None),
            M3b::EscDialog => ("#escalation-other-tx", None),
            M3b::EscDialogFailed => ("#escalation-server-text", None),
            M3b::EscDialogLocal => ("#escalation-title", None),
            M3b::EscDialogProduction => ("#console-connection-name", Some(PROD)),
            M3b::EscKeptWaiting => (MARKS_RS, None),
            M3b::ErrBar | M3b::ErrBarFocused => ("#button-label-errbar-retry", None),
            M3b::ErrBarFailedTx => ("#button-label-errbar-rollback-continue", None),
            // 8 · the statement's SQL (data), or a label only this state draws.
            M3b::Params(s) => match s {
                Pf::TwoNames | Pf::Null => (SQL_LINE, Some(PARAMS_SQL)),
                Pf::Remembered => ("#params-remembered-label-0", None),
                Pf::Turkish => ("#params-ambiguous", None),
                Pf::Template => (SQL_LINE, Some("SELECT * FROM ${tablo} WHERE id = :id")),
                Pf::PgQuestionMark => (
                    SQL_LINE,
                    Some("SELECT * FROM belgeler WHERE veri ? 'etiket' AND id = :id"),
                ),
                Pf::BadHex => ("#params-bad-hex", None),
            },
            // 9 · fix round 1 (I4): what only a menu draws: the uncertain chooser's wrapping
            // sentences (`#menu-item-text`), the candidates' SQL hints (`#menu-item-hint`), the
            // popover's title. No button of the shell under the fixture is one of them.
            M3b::Chooser(s) => match s {
                Ch::Uncertain | Ch::Highlighted | Ch::ReadOnly => ("#menu-item-text", None),
                Ch::Customize => ("#menu-item-hint", None),
                Ch::Popover => ("#popup-menu-title", None),
            },
            // 10
            M3b::Results(r) => results_marker(r),
            // The menu's rows (`#menu-item-label`, fix round 1, I4); `check` asks for the layer.
            M3b::ResultsTabMenu => ("#menu-item-label", None),
            // 11 · the selected entry's ISO time (data), the empty states their own sentence.
            M3b::HistList => ("#history-meta-time", Some("2026-11-16T11:21:00Z")),
            M3b::HistSearch => ("#history-meta-time", Some("2026-11-15T06:30:00Z")),
            M3b::HistAllFailed => ("#history-meta-time", Some("2026-11-16T08:04:00Z")),
            M3b::HistRedacted => ("#history-meta-time", Some("2026-11-16T07:47:00Z")),
            M3b::HistOther => ("#history-meta-time", Some("2026-11-16T11:32:00Z")),
            M3b::HistEmpty => ("#history-empty", None),
            M3b::HistNoMatch => ("#history-nomatch", None),
            // 13 · the production console's own toolbar under its menu.
            M3b::TxChipProduction => ("#console-connection-name", Some(PROD)),
            // Task 29
            M3b::TxChipReadOnly => ("#console-connection-name", Some(READ_ONLY)),
            M3b::TxChipSmart => ("#tx-chip-mode", None),
            M3b::PreviewAsOne => ("#console-connection-name", Some(DEV)),
            M3b::ConnSessions => ("#checkbox-label-conn-single-session", None),
        }
    }

    /// Labels (`#id`) the state must show besides its marker: what tells it from a state with
    /// the same marker.
    pub(super) fn also_shown(self) -> &'static [&'static str] {
        match self {
            M3b::Confirm(Cf::MultiType) => &["#confirm-unit", "#confirm-type-name"],
            M3b::Confirm(Cf::Multi) => &["#confirm-unit", "#button-label-confirm-skip-all"],
            M3b::Confirm(Cf::MultiStaging) => &["#button-label-confirm-execute-all"],
            M3b::Confirm(Cf::Type | Cf::Typed) => &["#confirm-type-name"],
            M3b::Confirm(Cf::MysqlDdl | Cf::ImplicitCommit) => &["#confirm-irreversible"],
            M3b::Commit(commit_prompt::fixtures::State::Savepoint) => &["#commit-line-2"],
            M3b::Commit(commit_prompt::fixtures::State::Own) => &["#commit-countdown-time"],
            M3b::TxChipMenu => &["#tx-chip-mode"],
            M3b::TxIsolationMenu => &["#popup-menu-title"],
            M3b::TxSegmentMenu => &["#status-tx-open"],
            M3b::EscDialogLocal => &["#button-label-escalation-keep"],
            M3b::EscDialogProduction => &["#escalation-title"],
            M3b::ErrBar | M3b::ErrBarFocused => &["#errbar-title"],
            M3b::Params(Pf::Template | Pf::PgQuestionMark) => &["#params-notice"],
            M3b::Chooser(Ch::Customize) => &["#popup-menu-title", "#menu-item-label"],
            M3b::Results(ResultsShot::CountExact) => &["#strip-cursor"],
            M3b::Results(ResultsShot::FetchAll) => &["#results-count-state"],
            M3b::ResultsTabMenu => &["#results-tab-1-repeat"],
            M3b::TxChipProduction => &["#popup-menu-title"],
            M3b::TxChipReadOnly => &["#tx-chip-mode"],
            M3b::ConnSessions => &["#conn-adv-budget"],
            _ => &[],
        }
    }

    /// The trust signals the state must show (NOTES M3b cross-cutting #7): each group lists the
    /// label ids of which one must be visible and painted.
    pub(super) fn required(self) -> &'static [&'static [&'static str]] {
        const ENV: &[&str] = &["#status-env-label"];
        const TX: &[&str] = &["#status-tx-open"];
        const TX_FAILED: &[&str] = &["#status-tx-failed"];
        const BADGE: &[&str] = &["#tx-badge-text"];
        const BADGE_FAILED: &[&str] = &["#tx-badge-failed"];
        const CHIP: &[&str] = &["#tx-chip-mode"];
        const LOCK: &[&str] = &["#console-lock-label"];
        // Every dialog's subtitle names the connection beside its environment mark (NOTES
        // cross-cutting #6): on production it is a trust signal.
        const DIALOG_CONN: &[&str] = &["#confirm-connection"];
        match self {
            M3b::TxChipMenu | M3b::TxIsolationMenu | M3b::TxChipSmart => &[CHIP],
            M3b::TxChipProduction => &[CHIP, ENV],
            M3b::TxBadgeOpen | M3b::TxShared => &[TX, BADGE, CHIP],
            M3b::TxBadgeFailed => &[TX_FAILED, BADGE_FAILED, CHIP],
            M3b::TxSegmentMenu | M3b::TxIdleToast => &[TX, BADGE],
            M3b::TxChipReadOnly => &[CHIP, LOCK],
            M3b::EscDialogProduction => &[ENV, DIALOG_CONN],
            M3b::Results(ResultsShot::CrowdedTabs) => &[ENV],
            // Fix round 1 (m3): every confirmation, staging ones too (the check asks which
            // environment the dialog's mark draws).
            M3b::Confirm(_) => &[DIALOG_CONN],
            M3b::Commit(_) => &[DIALOG_CONN],
            _ => &[],
        }
    }

    /// Whether the title-bar capsule shows production (the one filled badge).
    pub(super) fn shows_production(self) -> bool {
        matches!(self, M3b::EscDialogProduction | M3b::TxChipProduction)
    }

    /// The steps after the shell opened (none for a fixture state).
    pub(super) fn steps(self) -> Vec<Step> {
        use M3b::*;
        let caret = |text: &'static str, needle: &str| {
            let c = after(text, needle);
            c..c
        };
        let script = |name: &'static str| {
            live(
                name,
                SCRIPT,
                caret(SCRIPT, "o.total"),
                Expect::Band(BandStyle::Run),
            )
        };
        match self {
            Confirm(_) | Commit(_) | Params(_) | Chooser(_) => Vec::new(),
            TxChipProduction => {
                let mut s = script(PROD);
                s.push(tx_menu(TxMenu::Chip));
                s
            }
            TxChipMenu | TxIsolationMenu => {
                let mut s = script(DEV);
                s.push(tx_menu(if self == TxChipMenu {
                    TxMenu::Chip
                } else {
                    TxMenu::Isolation
                }));
                s
            }
            TxBadgeOpen | TxIdleToast | TxSegmentMenu => {
                let mut s = script(DEV);
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
                let mut s = script(DEV);
                s.push(user_tx(&["BEGIN", UPD1, UPD2, FAILS], None, true));
                s
            }
            TxShared => {
                let mut s = script(SHARED);
                s.push(user_tx(&["BEGIN", UPD1], Some(1), false));
                s
            }
            // The titled tabs, then the first tab's menu under it.
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
            // The crowded strip on production.
            Results(ResultsShot::CrowdedTabs) => {
                results(ResultsShot::CrowdedTabs, Some(EnvTag::Production))
            }
            Results(r) => results(r, Some(EnvTag::Dev)),
            // The fixtures with fixed clocks (no server that ignores a cancel on cue).
            EscSlowToast => {
                let mut s = script(DEV);
                s.push(step(
                    |window, cx| {
                        let c = active_console(window, cx).expect("a console");
                        exec_ui::escalation::fixtures::show_toast(c, 11, cx);
                    },
                    |_, cx| !ToastQueue::visible_ids(cx).is_empty(),
                ));
                s
            }
            EscDialog | EscDialogFailed | EscDialogLocal | EscDialogProduction => {
                use exec_ui::escalation::{OtherTx, OwnTx, Reach, fixtures};
                let mut s = script(if self == EscDialogProduction {
                    PROD
                } else {
                    DEV
                });
                s.push(step(
                    move |window, cx| {
                        let c = active_console(window, cx).expect("a console");
                        let (own, reach, others) = match self {
                            EscDialog => (
                                OwnTx::Open(3),
                                Reach::Profile,
                                vec![(format!("{DEV} (2)"), OtherTx::Open(3))],
                            ),
                            EscDialogFailed | EscDialogProduction => {
                                (OwnTx::Open(3), Reach::Profile, Vec::new())
                            }
                            _ => (OwnTx::None, Reach::File("app.db".into()), Vec::new()),
                        };
                        let mut facts =
                            fixtures::facts(c, own, reach, others, cx).expect("the facts");
                        if self == EscDialogLocal {
                            // SQLite: nothing can be terminated "on the server".
                            facts.terminate = None;
                        }
                        let refused = (self == EscDialogFailed).then_some((
                            "42501",
                            "must be a superuser to terminate superuser process",
                        ));
                        fixtures::open(c, facts, 14, refused, window, cx);
                    },
                    |window, cx| {
                        window.root::<Workspace>().flatten().is_some_and(|ws| {
                            ws.read(cx)
                                .layer_ids()
                                .contains(&exec_ui::escalation::ESCALATION_LAYER_ID)
                        })
                    },
                ));
                s
            }
            EscKeptWaiting => {
                let mut s = marks(
                    MARKS,
                    vec![
                        (
                            "SELECT o.id, o.total FROM orders o",
                            succeeded(42, Some(rows(100))),
                        ),
                        (
                            "UPDATE orders SET status = 'shipped' WHERE id = 1042",
                            (UnitStatus::Cancelling, Duration::from_secs(14)),
                        ),
                        (
                            "SELECT count(*) FROM order_lines",
                            (UnitStatus::Queued { position: 1 }, Duration::ZERO),
                        ),
                    ],
                );
                s.push(instant(|window, cx| {
                    let c = active_console(window, cx).expect("a console");
                    exec_ui::escalation::fixtures::escalate_mark(
                        c,
                        exec_ui::fixtures::fixture_key(1),
                        14,
                        cx,
                    );
                }));
                s
            }
            ErrBar | ErrBarFailedTx | ErrBarFocused => {
                let mut s = script(DEV);
                s.push(instant(move |window, cx| {
                    let c = active_console(window, cx).expect("a console");
                    // The run waits on the bar, so the toolbar shows it running (Stop).
                    if let Some(tab) = Consoles::get(cx, c).map(|s| s.tab) {
                        console_fixtures::show_run(tab, RunUx::Running(3), cx);
                    }
                    match self {
                        ErrBarFailedTx => exec_ui::error_bar::fixtures::show(
                            c,
                            (3, 6),
                            "25P02",
                            Some((2, false)),
                            cx,
                        ),
                        _ => exec_ui::error_bar::fixtures::show(c, (2, 5), "42P01", None, cx),
                    }
                }));
                if self == ErrBarFocused {
                    s.push(step(
                        |window, cx| {
                            exec_ui::error_bar::fixtures::focus(window, cx);
                        },
                        |window, cx| {
                            active_console(window, cx).is_some_and(|c| {
                                exec_ui::error_bar::button_focus(c, cx)
                                    .first()
                                    .is_some_and(|(_, h)| h.is_focused(window))
                            })
                        },
                    ));
                }
                s
            }
            // The dialog over a live console, from its fixture (the mock's entries at a fixed
            // "now"; no store).
            HistList | HistSearch | HistAllFailed | HistRedacted | HistOther | HistEmpty
            | HistNoMatch => {
                use hacivat_app::history::dialog::fixtures::{self, Scenario};
                let scenario = match self {
                    HistList => Scenario::List,
                    HistSearch => Scenario::Search,
                    HistAllFailed => Scenario::AllFailed,
                    HistRedacted => Scenario::Redacted,
                    HistOther => Scenario::Other,
                    HistEmpty => Scenario::Empty,
                    _ => Scenario::NoMatch,
                };
                let mut s = live(DEV, "", 0..0, Expect::Any);
                s.push(step(
                    move |window, cx| {
                        let c = active_console(window, cx).expect("a console");
                        let f = fixtures::scenario(scenario, c, cx);
                        fixtures::open(c, f, window, cx);
                    },
                    |window, cx| {
                        window.root::<Workspace>().flatten().is_some_and(|ws| {
                            ws.read(cx)
                                .layer_ids()
                                .contains(&hacivat_app::history::dialog::HISTORY_LAYER_ID)
                        })
                    },
                ));
                s
            }
            // ## Task 29: a read-only connection's console (Auto-commit, the chip disabled).
            TxChipReadOnly => script(READ_ONLY),
            // Smart mode, set through the chip's own command.
            TxChipSmart => {
                let mut s = script(DEV);
                s.push(step(
                    |window, cx| {
                        let c = active_console(window, cx).expect("a console");
                        hacivat_app::console::tx_chip::send(
                            c,
                            TxCommand::SetMode(TxMode::Smart),
                            cx,
                        );
                    },
                    |window, cx| {
                        session_in(window, cx)
                            .is_some_and(|s| s.state().borrow().tx.mode == TxMode::Smart)
                    },
                ));
                s
            }
            // The script's three statements selected, then the console's selection policy set
            // to As one (CHOOSE-06): the band covers the selection as one unit.
            PreviewAsOne => {
                let mut s = live(
                    DEV,
                    SCRIPT,
                    at(SCRIPT, "SELECT o.id")..SCRIPT.len() - 1,
                    Expect::Units(3),
                );
                s.push(step(
                    |window, cx| {
                        let c = active_console(window, cx).expect("a console");
                        let policy = exec_ui::orchestrator::policy_of(c, cx);
                        exec_ui::orchestrator::set_policy(
                            c,
                            hacivat_sql_editor::RunPolicy {
                                selection_as_one: true,
                                ..policy
                            },
                            cx,
                        );
                    },
                    |window, cx| {
                        editor_in(window, cx).is_some_and(|e| {
                            preview::current(e.read(cx)).is_some_and(|b| b.units.len() == 1)
                        })
                    },
                ));
                s
            }
            // The new connection dialog, its Advanced section opened from the keyboard and the
            // Sessions checkbox checked: the body follows the focus down to it.
            ConnSessions => vec![
                step(
                    hacivat_app::connections::dialog::new_fixture,
                    |window, cx| {
                        hacivat_app::connections::dialog::open_dialog(window, cx).is_some()
                    },
                ),
                step(
                    |window, cx| {
                        tab_to("advanced", window, cx);
                        press("space", window, cx);
                    },
                    |_, _| true,
                ),
                step(
                    |window, cx| {
                        tab_to("single-session", window, cx);
                        press("space", window, cx);
                    },
                    |window, cx| dialog_focus(window, cx) == Some("single-session"),
                ),
            ],
        }
    }

    /// What the app's models (and, for toasts and menus, the frame) must say once the frame is
    /// drawn (a violation otherwise): the state check that keeps a marker from passing on an
    /// unrelated label.
    pub(super) fn check(
        self,
        window: &mut Window,
        cx: &App,
        frame: &[RenderedText],
        decls: &[A11yDecl],
    ) -> Option<String> {
        use M3b::*;
        let editor = editor_in(window, cx);
        let band = || editor.as_ref().and_then(|e| preview::current(e.read(cx)));
        let console = active_console(window, cx);
        let toolbar = || {
            console
                .and_then(|c| Consoles::get(cx, c))
                .and_then(|s| console_fixtures::shown(s.tab, cx))
        };
        let tx = || console.and_then(|c| exec_ui::tx::user_tx(c, cx));
        let session = session_in(window, cx);
        let mode = || session.as_ref().map(|s| s.state().borrow().tx.mode);
        let toast = |id: &str| shows_toast(frame, &message(cx, id, Vec::new()));
        let viewport = window.viewport_size();
        let ok = match self {
            Confirm(s) => check_confirm(s, cx, frame),
            Commit(s) => check_commit(s, window, cx, frame),
            Params(s) => check_params(s, cx, frame),
            Chooser(s) => check_chooser(s, window, cx, frame, decls),
            TxChipMenu | TxIsolationMenu => {
                menu_open(window, cx)
                    && tx().is_none()
                    && console
                        .is_some_and(|c| hacivat_app::console::tx_badge::view(c, cx).is_some())
            }
            TxChipProduction => {
                menu_open(window, cx) && tx().is_none() && mode() == Some(TxMode::Manual)
            }
            TxBadgeOpen => console.is_some_and(|c| {
                hacivat_app::console::tx_badge::user_tx(c, cx)
                    .is_some_and(|u| !u.failed && u.statements == 3)
            }),
            TxShared => console.is_some_and(|c| {
                hacivat_app::console::tx_badge::user_tx(c, cx).is_some_and(|u| !u.failed)
                    && hacivat_app::console::tx_chip::render_shared(c, false, cx).is_some()
                    // The chip (its words or its icon) is painted inside the window.
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
            EscSlowToast => toast("hv-cancel-slow-title"),
            EscDialog => drawn(frame, "escalation-title") && drawn(frame, "escalation-other-tx"),
            EscDialogFailed => {
                drawn(frame, "escalation-terminate-failed")
                    && drawn(frame, "escalation-server-text")
            }
            EscDialogLocal => {
                drawn(frame, "escalation-title")
                    && !drawn(frame, "button-label-escalation-terminate")
            }
            // The subtitle's connection, on production.
            EscDialogProduction => {
                drawn(frame, "escalation-title") && drawn(frame, "confirm-connection")
            }
            EscKeptWaiting => editor.as_ref().is_some_and(|e| {
                matches!(
                    e.read(cx).marks().state(exec_ui::fixtures::fixture_key(1)),
                    Some(ExecMarkState::Escalated { .. })
                )
            }),
            ErrBar | ErrBarFailedTx | ErrBarFocused => console.is_some_and(|c| {
                let offered =
                    exec_ui::error_bar::open_question(c, cx).map_or(0, |(_, _, o)| o.len());
                let focus = exec_ui::error_bar::button_focus(c, cx);
                let in_bar = focus.first().is_some_and(|(_, h)| h.is_focused(window));
                drawn(frame, "errbar-title")
                    // The run that waits shows Stop.
                    && toolbar().is_some_and(|t| matches!(t.run, Some(RunUx::Running(3))))
                    && match self {
                        ErrBarFailedTx => offered == 3 && !in_bar,
                        ErrBarFocused => offered == 4 && in_bar,
                        _ => offered == 4 && !in_bar,
                    }
            }),
            // Fix round 1 (m5): what the tile holds.
            Results(r) => {
                console.is_some_and(|c| check_results(r, c, cx))
                    // The fetch-all question is the window's top layer.
                    && (!matches!(r, ResultsShot::FetchAll)
                        || window.root::<Workspace>().flatten().is_some_and(|ws| {
                            ws.read(cx).layer_ids().last() == Some(&"dialog")
                        }))
            }
            ResultsTabMenu => menu_open(window, cx),
            // What the dialog holds, and the part only this state draws.
            HistList | HistSearch | HistAllFailed | HistRedacted | HistOther | HistEmpty
            | HistNoMatch => hacivat_app::history::dialog::shown(cx).is_some_and(|d| {
                let n = d.statements.len();
                match self {
                    HistList => n == 5 && !d.all && drawn(frame, "history-row-sql"),
                    HistSearch => n == 2 && d.query == "musteri" && d.selected == Some(1),
                    HistAllFailed => n == 1 && drawn(frame, "history-server-text"),
                    // A masked entry is never offered to run.
                    HistRedacted => {
                        d.selected == Some(2)
                            && drawn(frame, "history-redacted-line")
                            && drawn(frame, "history-masked-line")
                            && !d.can_run
                    }
                    HistOther => n == 8 && drawn(frame, "history-target"),
                    HistEmpty => n == 0 && d.query.is_empty(),
                    _ => n == 0 && d.query == "faturalar",
                }
            }),
            // Task 29
            // Fix round 1 (m6): the chip as drawn: its words, and the view it draws from.
            TxChipReadOnly => {
                session
                    .as_ref()
                    .is_some_and(|s| s.state().borrow().read_only)
                    && console.is_some_and(|c| {
                        hacivat_app::console::tx_badge::view(c, cx)
                            .is_some_and(|v| v.read_only && v.mode == TxMode::Auto)
                    })
                    && m3a::shows_label(
                        frame,
                        "#tx-chip-mode",
                        &super::strip(&hacivat_app::console::tx_chip::mode_text(cx, TxMode::Auto)),
                    )
            }
            TxChipSmart => {
                mode() == Some(TxMode::Smart)
                    && tx().is_none()
                    && m3a::shows_label(
                        frame,
                        "#tx-chip-mode",
                        &super::strip(&hacivat_app::console::tx_chip::mode_text(cx, TxMode::Smart)),
                    )
            }
            PreviewAsOne => {
                band().is_some_and(|b| b.style == BandStyle::Run && b.units.len() == 1)
                    && console
                        .is_some_and(|c| exec_ui::orchestrator::policy_of(c, cx).selection_as_one)
            }
            ConnSessions => {
                dialog_focus(window, cx) == Some("single-session")
                    && m3a::shows_label(
                        frame,
                        "#conn-adv-budget",
                        &message(cx, "hv-conn-budget-single", Vec::new()),
                    )
            }
        };
        (!ok).then(|| format!("{}: the state check failed", self.name()))
    }
}

// ---- the fixture states' checks -------------------------------------------------------------

/// Surface 1: the open dialog's buttons as the rules say (T26: production never offers "don't
/// ask again" and never Execute all; R-M3-55: Cancel always; a name to type keeps Execute off;
/// Warn's Execute is Default).
fn check_confirm(s: Cf, cx: &App, frame: &[RenderedText]) -> bool {
    use hacivat_ui::button::ButtonKind;
    if !confirm::confirm_open(cx) {
        return false;
    }
    let buttons = confirm::open_dialog_buttons(cx);
    let has = |id: &str| buttons.iter().any(|b| b.id == id);
    let execute = buttons.iter().find(|b| b.id == "confirm-execute");
    let production = !matches!(s, Cf::Calm | Cf::NotAnalysed | Cf::MultiStaging);
    // Fix round 1 (m3): the mark the subtitle draws is production's, or staging's.
    let env = if production {
        EnvTag::Production
    } else {
        EnvTag::Staging
    };
    let rules = has("confirm-cancel")
        && execute.is_some()
        && confirm::open_dialog_env(cx) == Some(Some(env))
        && (!production || !has("confirm-suppress") && !has("confirm-execute-all"));
    rules
        && match s {
            Cf::Multi => has("confirm-skip-all") && !has("confirm-execute-all"),
            Cf::MultiStaging => {
                has("confirm-execute-all") && has("confirm-skip-all") && has("confirm-suppress")
            }
            Cf::MultiType => execute.is_some_and(|b| !b.enabled) && has("confirm-skip-all"),
            // Fix round 1 (I4): a lone statement: no "statement k of n" (a multi-unit frame
            // fails these).
            Cf::Type | Cf::DropSchema => {
                execute.is_some_and(|b| !b.enabled) && !drawn(frame, "confirm-unit")
            }
            Cf::Typed => execute.is_some_and(|b| b.enabled) && !drawn(frame, "confirm-unit"),
            Cf::Mismatch => {
                execute.is_some_and(|b| !b.enabled) && drawn(frame, "confirm-type-hint")
            }
            // Fix round 1 (I4): one statement (the staging multi-unit frame fails it).
            Cf::Calm => {
                has("confirm-suppress") && !has("confirm-skip-all") && !has("confirm-execute-all")
            }
            Cf::NotAnalysed => !has("confirm-suppress"),
            Cf::Warn => execute.is_some_and(|b| b.kind == ButtonKind::Default),
            Cf::MysqlDdl | Cf::ImplicitCommit => drawn(frame, "confirm-irreversible"),
            // A production DDL marked suppressible by the fixture: the dialog still offers no
            // "don't ask again" (T26 fix round 2, N1).
            Cf::SuppressProd => !has("confirm-suppress"),
            Cf::Unknown | Cf::Routine | Cf::Explain => {
                execute.is_some_and(|b| b.kind == ButtonKind::Danger)
            }
        }
}

/// Surface 2: the prompt is the window's top layer and says what its form says.
fn check_commit(
    s: commit_prompt::fixtures::State,
    window: &mut Window,
    cx: &App,
    frame: &[RenderedText],
) -> bool {
    use commit_prompt::fixtures::State as Cm;
    let top = window
        .root::<Workspace>()
        .flatten()
        .is_some_and(|ws| ws.read(cx).layer_ids().last() == Some(&commit_prompt::COMMIT_LAYER_ID));
    let own_lost = m3a::shows_label(
        frame,
        "#commit-dropped-body",
        &message(cx, "hv-commit-dropped-body", Vec::new()),
    );
    top && match s {
        Cm::Savepoint => {
            drawn(frame, "button-label-commit-keep") && drawn(frame, "commit-countdown-time")
        }
        Cm::Own => {
            drawn(frame, "button-label-commit-commit") && commit_prompt::countdown_of(cx).is_some()
        }
        Cm::LostOwn => own_lost && !drawn(frame, "commit-countdown-time"),
        Cm::LostSavepoint => {
            drawn(frame, "commit-dropped-body")
                && !own_lost
                && !drawn(frame, "commit-countdown-time")
        }
    }
}

/// Surface 8: the open prompt's rows and whether Run may answer (fail-closed: a flagged value,
/// a template variable or bad bytes keep it off).
fn check_params(s: Pf, cx: &App, frame: &[RenderedText]) -> bool {
    let Some(prompt) = params_prompt::open_prompt(cx) else {
        return false;
    };
    let p = prompt.read(cx);
    let n = p.prompts().len();
    let nulls = (0..n).filter(|i| p.is_null(*i)).count();
    match s {
        Pf::TwoNames => n == 2 && nulls == 0 && p.prompts().iter().all(|r| r.remembered.is_none()),
        Pf::Null => n == 2 && nulls == 1,
        Pf::Remembered => p.prompts().first().is_some_and(|r| r.remembered.is_some()),
        Pf::Turkish => !p.run_enabled(cx) && drawn(frame, "params-ambiguous"),
        Pf::Template => !p.run_enabled(cx) && drawn(frame, "params-notice"),
        Pf::PgQuestionMark => n == 1 && drawn(frame, "params-notice"),
        Pf::BadHex => !p.run_enabled(cx) && drawn(frame, "params-bad-hex"),
    }
}

/// Surface 9: the fixture's menu is the window's top layer and shows the row this state is
/// about (its rows are button labels, in runs around an access key on Linux and
/// Windows).
fn check_chooser(
    s: Ch,
    window: &mut Window,
    cx: &App,
    frame: &[RenderedText],
    decls: &[A11yDecl],
) -> bool {
    let shows = |id: &str| {
        let drawn: Vec<RenderedText> = frame
            .iter()
            .filter(|r| r.visible && r.painted_px > 0.0)
            .cloned()
            .collect();
        super::support::find_label_runs(&drawn, &message(cx, id, Vec::new())).is_some()
    };
    let top = window
        .root::<Workspace>()
        .flatten()
        .and_then(|ws| ws.read(cx).layer_ids().last().copied());
    // Task 24's fixture shows the chooser's rows, and the popover, on the popover's layer.
    top == Some(policy_popover::POLICY_LAYER)
        && match s {
            // Fix round 1 (I4): the highlighted row is the menu's active descendant (what a
            // screen reader hears): "Run as one" in the highlighted state, none before it.
            Ch::Uncertain => {
                shows("hv-chooser-run-as-one-hint")
                    && !shows("hv-chooser-run-as-one-ro")
                    && !decls.iter().any(|d| d.active_descendant)
            }
            Ch::Highlighted => {
                let row = super::strip(&message(
                    cx,
                    "hv-chooser-row",
                    vec![
                        (
                            "kind",
                            hacivat_l10n::ArgValue::Text(
                                message(cx, "hv-chooser-run-as-one", Vec::new()).to_string(),
                            ),
                        ),
                        ("n", hacivat_l10n::ArgValue::Count(2)),
                    ],
                ));
                shows("hv-chooser-run-as-one-hint")
                    && decls
                        .iter()
                        .any(|d| d.active_descendant && super::strip(&d.name) == row)
            }
            Ch::ReadOnly => {
                shows("hv-chooser-run-as-one-ro") && !decls.iter().any(|d| d.active_descendant)
            }
            Ch::Customize => shows("hv-chooser-customize"),
            Ch::Popover => m3a::shows_label(
                frame,
                "#popup-menu-title",
                &message(cx, "hv-policy-title", Vec::new()),
            ),
        }
}

// ---- results --------------------------------------------------------------------------------

// Fix round 1 (T29 review m5)
/// What console `c`'s results hold in a Task 25 state.
fn check_results(r: ResultsShot, c: hacivat_app::console::ConsoleId, cx: &App) -> bool {
    use hacivat_app::results::ActiveResult;
    use hacivat_app::results::tabs::CountState;
    use hacivat_exec::LogMsg;
    let Some(results) = Consoles::get(cx, c).and_then(|s| s.results.clone()) else {
        return false;
    };
    let results = results.read(cx);
    let active = results.active_tab();
    let logged = |f: &dyn Fn(&LogMsg) -> bool| results.output().lines().iter().any(|l| f(&l.msg));
    match r {
        ResultsShot::TitledTabs => {
            let titles = results.titles(cx);
            titles.len() == 4
                && titles.iter().filter(|t| t.repeat == Some(2)).count() == 1
                && results.tabs()[0].comment().is_some()
        }
        ResultsShot::Untyped => active.is_some_and(|t| t.untyped()),
        ResultsShot::SetLimit => logged(&|m| matches!(m, LogMsg::ResultSetLimit { .. })),
        ResultsShot::Counting => {
            active.is_some_and(|t| matches!(t.count(), CountState::Counting { .. }))
        }
        ResultsShot::CountExact => {
            active.is_some_and(|t| t.count() == CountState::Exact(1_048_213))
        }
        ResultsShot::Evicted => active.is_some_and(|t| t.evicted()),
        ResultsShot::FetchAll => active.is_some_and(|t| t.count() == CountState::Exact(40_000_000)),
        ResultsShot::Notices => {
            results.active() == ActiveResult::Output
                && logged(&|m| matches!(m, LogMsg::Notice { .. }))
        }
        ResultsShot::MysqlWarnings => {
            results.active() == ActiveResult::Output
                && logged(&|m| matches!(m, LogMsg::Notice { code: Some(_), .. }))
        }
        ResultsShot::CrowdedTabs => {
            results.tabs().len() == 30 && results.active() == ActiveResult::Tab(20)
        }
        _ => unreachable!("{r:?} is an M3a state"),
    }
}

/// Task 25's results states' names (the M3a mapping delegates them here).
pub(super) fn results_name(r: ResultsShot) -> &'static str {
    match r {
        ResultsShot::TitledTabs => "results-titled-tabs",
        ResultsShot::Untyped => "results-untyped",
        ResultsShot::SetLimit => "results-set-limit",
        ResultsShot::Counting => "results-counting",
        ResultsShot::CountExact => "results-count-exact",
        ResultsShot::Evicted => "strip-evicted",
        ResultsShot::FetchAll => "results-fetch-all",
        ResultsShot::Notices => "output-notices",
        ResultsShot::MysqlWarnings => "output-mysql-warnings",
        ResultsShot::CrowdedTabs => "results-crowded-tabs",
        _ => unreachable!("{r:?} is an M3a state"),
    }
}

/// Task 25's results states' markers.
pub(super) fn results_marker(r: ResultsShot) -> (&'static str, Option<&'static str>) {
    match r {
        // The repeated title's `(2)`, a label of its own.
        ResultsShot::TitledTabs => ("#results-tab-1-repeat", None),
        ResultsShot::Untyped => ("#results-tab-0-untyped", None),
        ResultsShot::SetLimit => ("#results-set-limit", None),
        ResultsShot::Counting => ("#button-label-results-count-cancel", None),
        ResultsShot::CountExact => ("#results-count-state", None),
        ResultsShot::Evicted => ("#button-label-strip-rerun", None),
        ResultsShot::FetchAll => (DIALOG, None),
        ResultsShot::Notices => ("#log-notice-2", None),
        ResultsShot::MysqlWarnings => ("#log-code-2", None),
        ResultsShot::CrowdedTabs => ("#results-tabs-more", None),
        _ => unreachable!("{r:?} is an M3a state"),
    }
}

// ---- the connection dialog ------------------------------------------------------------------

/// The open connection dialog's focused part (`advanced`, `single-session`, …).
fn dialog_focus(window: &mut Window, cx: &App) -> Option<&'static str> {
    let dialog = hacivat_app::connections::dialog::open_dialog(window, cx)?;
    dialog.read(cx).focused_field(window, cx)
}

/// Presses `keys` in the window.
fn press(keys: &'static str, window: &mut Window, cx: &mut App) {
    window.dispatch_keystroke(Keystroke::parse(keys).expect("a keystroke"), cx);
}

/// Presses Tab until the dialog's focus is on `part` (bounded: the dialog has fewer stops).
fn tab_to(part: &str, window: &mut Window, cx: &mut App) {
    for _ in 0..80 {
        if dialog_focus(window, cx) == Some(part) {
            return;
        }
        press("tab", window, cx);
    }
    panic!("Tab never reached {part} in the connection dialog");
}
