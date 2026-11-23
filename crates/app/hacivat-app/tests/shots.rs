//! The screenshot harness (Task 18, feature `shots`): renders every M1 surface headlessly (R8:
//! GPUI's `HeadlessAppContext` with the platform's real text system and its headless renderer)
//! and writes `target/shots/<surface>-<theme>-<locale>-<w>x<h>@<s>x.png`.
//!
//! ```text
//! cargo test -p hacivat-app --features shots --test shots -- --ignored
//! ```
//!
//! The target has `harness = false`: on macOS the platform (and its text system) can only be
//! created on the main thread, and libtest runs every test on a spawned thread. `main` keeps
//! libtest's contract: the self-checks run unless `--ignored`, and `render_all_surfaces` (the
//! main check) runs only with `--ignored` or `--include-ignored`. A positional filter that
//! names no test selects shots by file name (substring, or whole name with `--exact`):
//! `-- --ignored palette-open` renders only those and keeps the other PNGs. `SHOTS_DIR`
//! overrides the output directory.
//!
//! Every surface must show in its frame: a label drawn by its own source file (palette, dialog,
//! toast, menu, settings page; the title bar for the shells, the capsule for the demo), and
//! visible: inside the window and its scroll viewport (Task 24 fix round 1). The SSH section's
//! surfaces name labels only that section draws, and their route and jump rows must show too.
//!
//! The M2a surfaces (Task 21) render the connection flow through `ConnectionsBoot::for_tests`
//! with a `FakeDriver` registry and a memory secret store, from the fixtures Tasks 9, 17, 18 and
//! 20 own (R28, P19): the connection dialog (also at 1024x700), the test strip, the error
//! dialog, the production prompt, the explorer and its menu, the delete and import dialogs,
//! the live capsule states, the explanation popover and the sticky toast; plus the shell at its
//! 960x600 minimum with the explorer populated.
//!
//! Every frame is checked before it is written:
//! - (a) in the pseudo locales (`en-XA`, `tr-XT`) every `TextSource::Fluent` label is bracketed;
//!   (## m3b-ci) a label drawn as access-key runs (Linux, Windows) is checked joined
//!   (`is_key_run`), and so is a surface marker drawn that way;
//! - (b) no label is clipped: wider than its box, or cut at its container's clip (a control
//!   row running past its card, a tile cutting its last row), wrapping paragraphs included;
//!   text partly scrolled out of a marked scroll viewport is not clipped (`text.rs`,
//!   `cut_by_clip`, has the policy and its limits);
//! - (c) every label is at least 11 px nominal (synthesized small caps record their nominal
//!   size; their 0.7x glyph runs are the one exception);
//! - (d) the One Filled Badge rule holds inside the title-bar capsule, expecting production
//!   only where the capsule shows it, and no badge-sized `cp_fill` quad sits anywhere else (the
//!   7 px production marks of dialogs and the explorer are not badges; T21 ruling);
//! - (f) (Task 24 fix round 1) no label runs past a side of the dialog card it is drawn in
//!   (`RenderedText::outside_card`): a card's own clip is the window, so (b) cannot see it;
//! - (e) (Task 22 fix round 2) the title bar's layout, read from `TitleBarProbe`: the trailing
//!   controls (EN/TR, theme, window controls) lie wholly inside the window and end at the
//!   bar's right padding, the palette field is at least `PALETTE_MIN_PX` wide and ends before
//!   them, and the capsule ends before the palette field. A pushed-off icon button draws no
//!   clipped text, so (b) alone cannot see it. ## m4-m3carry (P2-04): or the field is its
//!   `PALETTE_ICON_PX` icon; beside a whole field the capsule gives up no trust word the icon's
//!   room would keep, and a collapsed field keeps at least one (`check_trust_room`, from
//!   `CapsuleProbe`).
//!
//! The M2b surfaces (Task 24) render at 1280x800 and the 960x600 minimum, from the fixtures of
//! Tasks 19 to 22 and Task 24's own: the certificate review, the pin-changed stop and the
//! plaintext question; the TLS section; the SSH section (off, folded, one hop, two jumps, the
//! import menu); the host-key prompts; keyboard-interactive; the SSH secret prompts; the vault
//! dialogs; the live states (T22's, plus reconnecting outside production and a session lost
//! without losses); the new error copy. The follow-up adds the three labels of fix round 1: a
//! pinned connection (status bar, sheet, test report), the "Connect through SSH…" fix and the
//! sheet's vault Password row (the harness boots that shot on a vault backend whose keyring
//! probe already ran); their markers are the labels' own `#id`s, which also say which label
//! they show (`#sheet-enc-pinned`). The sheet's pinned label must show whole in every locale,
//! its vault label in English and Turkish (`Surface::uncut`).
//! The import menu opens once the core runtime has read the config: the harness waits for it
//! (a condition, never a fixed sleep).
//!
//! The M3a surfaces (Task 35, `shots/m3a.rs`): NOTES surfaces 1–14 of `design/m3a-query/`,
//! 68 states at 1280x800 and 960x600, after the 1,012 M1/M2 shots (which stay unchanged). A
//! state is reached by steps, each followed by a bounded wait on a condition: the UI's timers
//! run on the headless app's simulated clock, which moves only while a step waits; core work
//! runs on real threads. Each state also has a state check read from the app's models (the
//! band, the chooser, the marks, the prompt), and:
//! - (g) the grid's own text: a header is data (never pseudo-localised, never Fluent), and a
//!   grid run wider than its cell or header is clipped; a grid state must show a header;
//! - (b) applies to visible labels: a label its container cut away whole draws no cut text.
//!
//! (m) (F5 follow-up) Every frame with an open menu: each row the menu shows is whole inside
//! its rows' column (a menu taller than its room scrolls whole rows), and none lies over the
//! status bar or past the window's bottom (`hacivat_ui::menu::MenuLayout`, recorded as the
//! menu prepaints). (b) cannot see a row drawn over the status bar: nothing clips it there.
//!
//! `--shard k/n` renders the k-th of n interleaved slices (run n processes side by side);
//! each shot prints its time, and the run its total. `SHOTS_DUMP=1` prints every label of
//! every frame.
//!
//! Windows has no headless renderer at the pin: the test prints `shots: no headless renderer
//! (R8)` and passes.
// ## m3a-t01b-seams: the shared app test helpers (`stop_core_runtime`, P20).
#[path = "support/mod.rs"]
mod support;
// ## m3a-t35-shots: the M3a surfaces live in their own file (NOTES surfaces 1–14).
#[path = "shots/m3a.rs"]
mod m3a;
// ## m3b-t29-shots: and the M3b surfaces in theirs (`design/m3b-safe` surfaces 1–13).
#[path = "shots/m3b.rs"]
mod m3b;
// ## m3c-t43-shots: and the M3c surfaces in theirs (`design/m3b-safe` surfaces 14–22; part B
// adds 23–25).
#[path = "shots/m3c.rs"]
mod m3c;
// ## m4-t32-shots: the M4 states (`design/m4-explorer`; the PO's R-W5-14 surfaces).
#[path = "shots/m4.rs"]
mod m4;
// ## m4b-t11-editor: the M4b states (`design/m4b-scripts`), Task 11's first.
#[path = "shots/m4b.rs"]
mod m4b;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hacivat_app::connections::dialog::ssh::{self, SshImportEnv};
use hacivat_app::connections::{
    ConnectionsBoot, capsule, cert_dialog, delete, dialog, error_view, explorer, hostkey_prompt,
    import, kbd_prompt, plaintext_prompt, prod_prompt, secret_prompt, test_strip, vault_dialog,
};
use hacivat_app::l10n_global::text_fn_id;
// ## m3b-t21-confirm, ## m3b-t24-exec-ui; ## m3b-t29-shots: their fixtures open from
// `shots/m3b.rs`.
use hacivat_app::menus;
use hacivat_app::shell::DemoCapsule;
use hacivat_app::shell::Workspace;
use hacivat_app::{AppState, Startup, start};
use hacivat_driver::DriverRegistry;
use hacivat_driver::fake::FakeDriver;
use hacivat_l10n::OsLocale;
use hacivat_proto::DbmsKind;
use hacivat_secrets::{
    BackendChoice, FallbackBackend, FileVault, MemoryBackend, SecretBackend as _, SecretStore,
};
use hacivat_settings::{LanguagePref, LocalPaths, ThemePref};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::capsule::{CapsuleProbe, TrustRooms};
use hacivat_ui::text::{RenderedText, TextRegistry, TextSource};
use hacivat_ui::titlebar::{PALETTE_ICON_PX, PALETTE_MIN_PX, TitleBarProbe};
use hacivat_ui_core::app::{TextRendering, bootstrap};
use hacivat_ui_core::assets::BundledAssets;
use hacivat_ui_core::gpui_surface::{
    AnyWindowHandle, App, Bounds, HeadlessAppContext, Pixels, PlatformTextSystem, Window,
    current_headless_renderer, current_platform, point, px, size,
};
use hacivat_ui_core::named_rules::assert_one_filled_badge_in;
use hacivat_ui_core::platform::apply_text_rendering;
use hacivat_ui_core::theme::{Theme, ThemeMode};

/// The surfaces, in render order: M1's, then M2a's (Task 21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    Shell,
    ShellDemoProduction,
    ShellNarrow,
    PaletteOpen,
    SettingsGeneral,
    DialogAbout,
    ToastInfo,
    MenuApp,
    // ## m2a-t21-shots
    ConnDialogNew,
    ConnDialogPasted,
    ConnDialogEngineSwitch,
    ConnDialogSqlite,
    ConnTestRunning,
    ConnTestOk,
    ConnTestFailed,
    ConnErrorAuth,
    ConnErrorUnknownDb,
    ConnErrorUnsupported,
    ConnErrorTls,
    ConnProdPrompt,
    ConnProdPromptUnset,
    ExplorerEmpty,
    ExplorerList,
    ExplorerMenu,
    ConnDeleteConfirm,
    ConnImportPgService,
    CapsuleConnecting,
    CapsuleConnectedUnencrypted,
    CapsuleProductionRo,
    CapsuleFailed,
    CapsuleUnsupported,
    UnencryptedPopover,
    ToastStickyDemo,
    ShellMinimum,
    // ## m2b-t22-live (fix round 1, M1): every T22 state, at 1280x800 and the 960x600 minimum.
    LiveReconnecting,
    LiveIdle,
    LiveLostTx,
    LiveLostReconnected,
    LiveGap,
    LiveRoEnforced,
    LiveRoGuarded,
    LiveRoPg13,
    LiveSshRo,
    LiveSshRw,
    // ## m2b-t24-shots: every other M2b surface, at 1280x800 and 960x600.
    CertReview,
    CertPinChanged,
    PlaintextQuestion,
    ConnDialogTlsPinned,
    SshSectionOff,
    SshSectionFolded,
    SshSectionHop,
    SshSectionJumps,
    SshImportMenu,
    HostKeyNew,
    HostKeyChanged,
    HostKeyChangedOpenSsh,
    KbdPrompt,
    SshPassword,
    SshPassphrase,
    VaultCreate,
    VaultUnlock,
    VaultReset,
    VaultDamaged,
    LiveReconnectingStaging,
    LiveLostClean,
    ErrInsecureTls,
    ErrInsecurePlain,
    ErrPinChanged,
    ErrSshAuth,
    ErrSshChannel,
    ErrSshAgent,
    ErrHostKeyChanged,
    ErrHostKeyRevoked,
    // ## m2b-t24-shots (fix round 1, M7)
    ErrHostKeyUnknown,
    // ## m2b-t24-shots (follow-up)
    LiveTlsPinned,
    ConnTestOkPinned,
    ErrTlsNotOffered,
    LiveVaultSecret,
    // ## m3a-t35-shots: every M3a state (`m3a::M3A`), at 1280x800 and 960x600.
    M3a(m3a::M3a),
    // ## m3b-t21-confirm, ## m3b-t24-exec-ui; ## m3b-t29-shots: every M3b state (`m3b::all`:
    // Tasks 21–27's and Task 29's), at 1280x800 and 960x600.
    M3b(m3b::M3b),
    // ## m3c-t43-shots: every M3c state (`m3c::all`: Tasks 31–38's), at 1280x800 and 960x600.
    M3c(m3c::M3c),
    // ## m4-t32-shots: every M4 state (`m4::M4_STATES`), at 1280x800 and 960x600.
    M4(m4::M4),
    // ## m4b-t11-editor: every M4b state (`m4b::M4B_STATES`), at 1280x800 and 960x600.
    M4b(m4b::M4b),
}

const SURFACES: [Surface; 78] = [
    Surface::Shell,
    Surface::ShellDemoProduction,
    Surface::ShellNarrow,
    Surface::PaletteOpen,
    Surface::SettingsGeneral,
    Surface::DialogAbout,
    Surface::ToastInfo,
    Surface::MenuApp,
    // ## m2a-t21-shots
    Surface::ConnDialogNew,
    Surface::ConnDialogPasted,
    Surface::ConnDialogEngineSwitch,
    Surface::ConnDialogSqlite,
    Surface::ConnTestRunning,
    Surface::ConnTestOk,
    Surface::ConnTestFailed,
    Surface::ConnErrorAuth,
    Surface::ConnErrorUnknownDb,
    Surface::ConnErrorUnsupported,
    Surface::ConnErrorTls,
    Surface::ConnProdPrompt,
    Surface::ConnProdPromptUnset,
    Surface::ExplorerEmpty,
    Surface::ExplorerList,
    Surface::ExplorerMenu,
    Surface::ConnDeleteConfirm,
    Surface::ConnImportPgService,
    Surface::CapsuleConnecting,
    Surface::CapsuleConnectedUnencrypted,
    Surface::CapsuleProductionRo,
    Surface::CapsuleFailed,
    Surface::CapsuleUnsupported,
    Surface::UnencryptedPopover,
    Surface::ToastStickyDemo,
    Surface::ShellMinimum,
    // ## m2b-t22-live
    Surface::LiveReconnecting,
    Surface::LiveIdle,
    Surface::LiveLostTx,
    Surface::LiveLostReconnected,
    Surface::LiveGap,
    Surface::LiveRoEnforced,
    Surface::LiveRoGuarded,
    Surface::LiveRoPg13,
    Surface::LiveSshRo,
    Surface::LiveSshRw,
    // ## m2b-t24-shots
    Surface::CertReview,
    Surface::CertPinChanged,
    Surface::PlaintextQuestion,
    Surface::ConnDialogTlsPinned,
    Surface::SshSectionOff,
    Surface::SshSectionFolded,
    Surface::SshSectionHop,
    Surface::SshSectionJumps,
    Surface::SshImportMenu,
    Surface::HostKeyNew,
    Surface::HostKeyChanged,
    Surface::HostKeyChangedOpenSsh,
    Surface::KbdPrompt,
    Surface::SshPassword,
    Surface::SshPassphrase,
    Surface::VaultCreate,
    Surface::VaultUnlock,
    Surface::VaultReset,
    Surface::VaultDamaged,
    Surface::LiveReconnectingStaging,
    Surface::LiveLostClean,
    Surface::ErrInsecureTls,
    Surface::ErrInsecurePlain,
    Surface::ErrPinChanged,
    Surface::ErrSshAuth,
    Surface::ErrSshChannel,
    Surface::ErrSshAgent,
    Surface::ErrHostKeyChanged,
    Surface::ErrHostKeyRevoked,
    Surface::ErrHostKeyUnknown,
    Surface::LiveTlsPinned,
    Surface::ConnTestOkPinned,
    Surface::ErrTlsNotOffered,
    Surface::LiveVaultSecret,
];
// ## m2b-t24-shots
/// Where Task 24's surfaces start in [`SURFACES`].
const FIRST_T24: usize = 44;
const LOCALES: [&str; 4] = ["en", "tr", "en-XA", "tr-XT"];
const THEMES: [ThemePref; 2] = [ThemePref::Dark, ThemePref::Light];
const PSEUDO: [&str; 2] = ["en-XA", "tr-XT"];

impl Surface {
    fn name(self) -> &'static str {
        match self {
            Surface::Shell => "shell",
            Surface::ShellDemoProduction => "shell-demo-production",
            Surface::ShellNarrow => "shell-narrow",
            Surface::PaletteOpen => "palette-open",
            Surface::SettingsGeneral => "settings-general",
            Surface::DialogAbout => "dialog-about",
            Surface::ToastInfo => "toast-info",
            Surface::MenuApp => "menu-app",
            Surface::ConnDialogNew => "conn-dialog-new",
            Surface::ConnDialogPasted => "conn-dialog-pasted",
            Surface::ConnDialogEngineSwitch => "conn-dialog-engine-switch",
            Surface::ConnDialogSqlite => "conn-dialog-sqlite",
            Surface::ConnTestRunning => "conn-test-running",
            Surface::ConnTestOk => "conn-test-ok",
            Surface::ConnTestFailed => "conn-test-failed",
            Surface::ConnErrorAuth => "conn-error-auth",
            Surface::ConnErrorUnknownDb => "conn-error-unknown-db",
            Surface::ConnErrorUnsupported => "conn-error-unsupported",
            Surface::ConnErrorTls => "conn-error-tls",
            Surface::ConnProdPrompt => "conn-prod-prompt",
            Surface::ConnProdPromptUnset => "conn-prod-prompt-unset",
            Surface::ExplorerEmpty => "explorer-empty",
            Surface::ExplorerList => "explorer-list",
            Surface::ExplorerMenu => "explorer-menu",
            Surface::ConnDeleteConfirm => "conn-delete-confirm",
            Surface::ConnImportPgService => "conn-import-pg-service",
            Surface::CapsuleConnecting => "capsule-connecting",
            Surface::CapsuleConnectedUnencrypted => "capsule-connected-unencrypted",
            Surface::CapsuleProductionRo => "capsule-production-ro",
            Surface::CapsuleFailed => "capsule-failed",
            Surface::CapsuleUnsupported => "capsule-unsupported",
            Surface::UnencryptedPopover => "unencrypted-popover",
            Surface::ToastStickyDemo => "toast-sticky-demo",
            Surface::ShellMinimum => "shell-minimum",
            // ## m2b-t22-live
            Surface::LiveReconnecting => "live-reconnecting",
            Surface::LiveIdle => "live-idle",
            Surface::LiveLostTx => "live-lost-tx",
            Surface::LiveLostReconnected => "live-lost-reconnected",
            Surface::LiveGap => "live-gap",
            Surface::LiveRoEnforced => "live-ro-enforced",
            Surface::LiveRoGuarded => "live-ro-guarded",
            Surface::LiveRoPg13 => "live-ro-pg13",
            Surface::LiveSshRo => "live-ssh-ro",
            Surface::LiveSshRw => "live-ssh-rw",
            // ## m2b-t24-shots
            Surface::CertReview => "cert-review",
            Surface::CertPinChanged => "cert-pin-changed",
            Surface::PlaintextQuestion => "plaintext-question",
            Surface::ConnDialogTlsPinned => "conn-dialog-tls-pinned",
            Surface::SshSectionOff => "ssh-section-off",
            Surface::SshSectionFolded => "ssh-section-folded",
            Surface::SshSectionHop => "ssh-section-hop",
            Surface::SshSectionJumps => "ssh-section-jumps",
            Surface::SshImportMenu => "ssh-import-menu",
            Surface::HostKeyNew => "hostkey-new",
            Surface::HostKeyChanged => "hostkey-changed",
            Surface::HostKeyChangedOpenSsh => "hostkey-changed-openssh",
            Surface::KbdPrompt => "kbd-prompt",
            Surface::SshPassword => "ssh-password",
            Surface::SshPassphrase => "ssh-passphrase",
            Surface::VaultCreate => "vault-create",
            Surface::VaultUnlock => "vault-unlock",
            Surface::VaultReset => "vault-reset",
            Surface::VaultDamaged => "vault-damaged",
            Surface::LiveReconnectingStaging => "live-reconnecting-staging",
            Surface::LiveLostClean => "live-lost-clean",
            Surface::ErrInsecureTls => "conn-error-insecure-tls",
            Surface::ErrInsecurePlain => "conn-error-insecure-plain",
            Surface::ErrPinChanged => "conn-error-pin-changed",
            Surface::ErrSshAuth => "conn-error-ssh-auth",
            Surface::ErrSshChannel => "conn-error-ssh-channel",
            Surface::ErrSshAgent => "conn-error-ssh-agent",
            Surface::ErrHostKeyChanged => "conn-error-host-key-changed",
            Surface::ErrHostKeyRevoked => "conn-error-host-key-revoked",
            Surface::ErrHostKeyUnknown => "conn-error-host-key-unknown",
            Surface::LiveTlsPinned => "live-tls-pinned",
            Surface::ConnTestOkPinned => "conn-test-ok-pinned",
            Surface::ErrTlsNotOffered => "conn-error-tls-not-offered",
            Surface::LiveVaultSecret => "live-vault-secret",
            // ## m3a-t35-shots
            Surface::M3a(s) => s.name(),
            // ## m3b-t29-shots
            Surface::M3b(s) => s.name(),
            // ## m3c-t43-shots
            Surface::M3c(s) => s.name(),
            // ## m4-t32-shots
            Surface::M4(s) => s.name(),
            // ## m4b-t11-editor
            Surface::M4b(s) => s.name(),
        }
    }

    // ## m2b-t22-live
    /// Whether the surface is one of Task 22's live states.
    fn is_live(self) -> bool {
        matches!(
            self,
            Surface::LiveReconnecting
                | Surface::LiveIdle
                | Surface::LiveLostTx
                | Surface::LiveLostReconnected
                | Surface::LiveGap
                | Surface::LiveRoEnforced
                | Surface::LiveRoGuarded
                | Surface::LiveRoPg13
                | Surface::LiveSshRo
                | Surface::LiveSshRw
        )
    }

    /// The logical window sizes of the matrix shots: 1280x800, plus 1024x700 for the
    /// connection dialog (the Overflow Rule); `shell-narrow` only at 1024x700 and
    /// `shell-minimum` only at the 960x600 minimum window (DESIGN.md).
    fn sizes(self) -> &'static [(f32, f32)] {
        // ## m2b-t24-shots: every M2b surface, at the default window and the minimum one.
        // ## m3a-t35-shots: and every M3a state.
        // ## m3b-t29-shots: and every M3b state.
        // ## m3c-t43-shots: and every M3c state.
        if self.is_live()
            || self.is_m2b()
            || matches!(self, Surface::M3a(_) | Surface::M3b(_) | Surface::M3c(_))
            // ## m4-t32-shots: and every M4 state.
            || matches!(self, Surface::M4(_))
            // ## m4b-t11-editor: and every M4b state.
            || matches!(self, Surface::M4b(_))
        {
            // ## m2b-t22-live: the default window and the minimum one (ruling I1).
            return &[(1280.0, 800.0), (960.0, 600.0)];
        }
        match self {
            Surface::ShellNarrow => &[(1024.0, 700.0)],
            Surface::ShellMinimum => &[(960.0, 600.0)],
            // ## m3a-f5-demo (review m-5): the in-window app menu is taller than the minimum
            // window's room, so check (m) sees a menu that shows a window of its rows.
            Surface::MenuApp => &[(1280.0, 800.0), (960.0, 600.0)],
            Surface::ConnDialogNew
            | Surface::ConnDialogPasted
            | Surface::ConnDialogEngineSwitch
            | Surface::ConnDialogSqlite => &[(1280.0, 800.0), (1024.0, 700.0)],
            _ => &[(1280.0, 800.0)],
        }
    }

    fn is_demo(self) -> bool {
        self == Surface::ShellDemoProduction
    }

    // ## m3b-t29-shots
    /// The M3b state, if the surface is one.
    fn m3b(self) -> Option<m3b::M3b> {
        match self {
            Surface::M3b(s) => Some(s),
            _ => None,
        }
    }

    // ## m3c-t43-shots
    /// The M3c state, if the surface is one.
    fn m3c(self) -> Option<m3c::M3c> {
        match self {
            Surface::M3c(s) => Some(s),
            _ => None,
        }
    }

    // ## m2b-t24-shots
    /// Whether the surface is one Task 24 added (every M2b surface outside T22's live set).
    fn is_m2b(self) -> bool {
        SURFACES
            .iter()
            .position(|s| *s == self)
            .is_some_and(|at| at >= FIRST_T24)
    }

    /// Whether the title-bar capsule shows production (the one filled badge): the demo, and
    /// the capsule fixtures of a production connection (connected, or failed with PRODUCTION
    /// first, NOTES §7).
    fn shows_production(self) -> bool {
        // ## m3a-t35-shots
        if let Surface::M3a(s) = self {
            return s.shows_production();
        }
        // ## m3b-t29-shots
        if let Surface::M3b(s) = self {
            return s.shows_production();
        }
        // ## m3c-t43-shots
        if let Surface::M3c(s) = self {
            return s.shows_production();
        }
        // ## m4b-t14-tabs (named line): the connected production script tab.
        if let Surface::M4b(s) = self {
            return s.connects();
        }
        matches!(
            self,
            Surface::ShellDemoProduction
                | Surface::CapsuleProductionRo
                | Surface::CapsuleFailed
                // ## m2b-t22-live
                | Surface::LiveReconnecting
                | Surface::LiveLostTx
                | Surface::LiveLostReconnected
                | Surface::LiveGap
                | Surface::LiveRoEnforced
                | Surface::LiveSshRo
        )
    }

    /// Whether the shell boots the M2a connections (a `FakeDriver` registry, a memory secret
    /// store): every M2a surface. M1's surfaces render the shell without them, as in M1.
    fn needs_connections(self) -> bool {
        !matches!(
            self,
            Surface::Shell
                | Surface::ShellDemoProduction
                | Surface::ShellNarrow
                | Surface::PaletteOpen
                | Surface::SettingsGeneral
                | Surface::DialogAbout
                | Surface::ToastInfo
                | Surface::MenuApp
        )
    }

    /// Proof that the surface is on screen: a label created in this source file (and, for the
    /// demo, with this text). `RenderedText::path` is the label's creation site.
    fn marker(self) -> (&'static str, Option<&'static str>) {
        match self {
            Surface::Shell | Surface::ShellNarrow => ("hacivat-ui/src/titlebar.rs:", None),
            Surface::ShellDemoProduction => ("hacivat-ui/src/capsule.rs:", Some("orders-prod")),
            Surface::PaletteOpen => ("hacivat-ui/src/palette.rs:", None),
            Surface::SettingsGeneral => ("hacivat-app/src/settings_ui/", None),
            Surface::DialogAbout => ("hacivat-ui/src/dialog.rs:", None),
            Surface::ToastInfo => ("hacivat-ui/src/toast.rs:", None),
            Surface::MenuApp => ("hacivat-ui/src/menu.rs:", None),
            Surface::ConnDialogNew | Surface::ConnDialogSqlite => {
                ("hacivat-app/src/connections/dialog/", None)
            }
            Surface::ConnDialogPasted | Surface::ConnDialogEngineSwitch => {
                ("hacivat-ui/src/notice.rs:", None)
            }
            Surface::ConnTestRunning | Surface::ConnTestOk => {
                ("hacivat-app/src/connections/test_strip.rs:", None)
            }
            Surface::ConnTestFailed
            | Surface::ConnErrorAuth
            | Surface::ConnErrorUnknownDb
            | Surface::ConnErrorUnsupported
            | Surface::ConnErrorTls => ("hacivat-app/src/connections/error_view.rs:", None),
            Surface::ConnProdPrompt | Surface::ConnProdPromptUnset => {
                ("hacivat-app/src/connections/prod_prompt.rs:", None)
            }
            Surface::ExplorerEmpty => ("hacivat-app/src/connections/explorer.rs:", None),
            // ## m4-t14-explorer (named exception): the profile rows are the explorer tree's
            // roots (R-M4-18), drawn by the tree widget.
            Surface::ExplorerList | Surface::ShellMinimum => {
                ("hacivat-ui/src/tree/view/row.rs:", Some("orders-prod"))
            }
            Surface::ExplorerMenu => ("hacivat-ui/src/menu.rs:", None),
            Surface::ConnDeleteConfirm => ("hacivat-ui/src/dialog.rs:", None),
            Surface::ConnImportPgService => ("hacivat-app/src/connections/import.rs:", None),
            Surface::CapsuleConnecting => ("hacivat-ui/src/capsule.rs:", Some("orders-staging")),
            Surface::CapsuleConnectedUnencrypted => {
                ("hacivat-ui/src/capsule.rs:", Some("orders-dev"))
            }
            Surface::CapsuleProductionRo | Surface::CapsuleFailed => {
                ("hacivat-ui/src/capsule.rs:", Some("orders-prod"))
            }
            Surface::CapsuleUnsupported => ("hacivat-ui/src/capsule.rs:", Some("legacy")),
            Surface::UnencryptedPopover => ("hacivat-ui/src/explain.rs:", None),
            Surface::ToastStickyDemo => ("hacivat-ui/src/toast.rs:", None),
            // ## m2b-t22-live
            Surface::LiveReconnecting | Surface::LiveSshRo => {
                ("hacivat-ui/src/capsule.rs:", Some("orders-prod"))
            }
            Surface::LiveSshRw => ("hacivat-ui/src/capsule.rs:", Some("orders-staging")),
            Surface::LiveIdle
            | Surface::LiveLostTx
            | Surface::LiveLostReconnected
            | Surface::LiveGap => ("hacivat-ui/src/toast.rs:", None),
            Surface::LiveRoEnforced | Surface::LiveRoGuarded | Surface::LiveRoPg13 => {
                ("hacivat-app/src/connections/access_popover.rs:", None)
            }
            // ## m2b-t24-shots
            Surface::CertReview | Surface::CertPinChanged => {
                ("hacivat-app/src/connections/cert_dialog.rs:", None)
            }
            Surface::PlaintextQuestion => {
                ("hacivat-app/src/connections/plaintext_prompt.rs:", None)
            }
            Surface::ConnDialogTlsPinned => ("hacivat-app/src/connections/dialog/", None),
            // Fix round 1 (I2): labels only the SSH section draws (the tunnel's checkbox, the
            // folded summary, the route's stops), which must also be visible.
            Surface::SshSectionOff => ("#checkbox-label-ssh-use", None),
            Surface::SshSectionFolded => ("#disclosure-summary-conn-ssh", None),
            Surface::SshSectionHop => ("#ssh-route-1", None),
            Surface::SshSectionJumps => ("#ssh-route-3", None),
            // An alias of the fixture config, drawn by the open menu's row label.
            Surface::SshImportMenu => ("hacivat-ui/src/button.rs:", Some("bastion")),
            Surface::HostKeyNew | Surface::HostKeyChanged | Surface::HostKeyChangedOpenSsh => {
                ("hacivat-app/src/connections/hostkey_prompt.rs:", None)
            }
            Surface::KbdPrompt => ("hacivat-app/src/connections/kbd_prompt.rs:", None),
            Surface::SshPassword | Surface::SshPassphrase => {
                ("hacivat-app/src/connections/secret_prompt.rs:", None)
            }
            Surface::VaultCreate
            | Surface::VaultUnlock
            | Surface::VaultReset
            | Surface::VaultDamaged => ("hacivat-app/src/connections/vault_dialog.rs:", None),
            Surface::LiveReconnectingStaging => {
                ("hacivat-ui/src/capsule.rs:", Some("orders-staging"))
            }
            Surface::LiveLostClean => ("hacivat-ui/src/toast.rs:", None),
            Surface::ErrInsecureTls
            | Surface::ErrInsecurePlain
            | Surface::ErrPinChanged
            | Surface::ErrSshAuth
            | Surface::ErrSshChannel
            | Surface::ErrSshAgent
            | Surface::ErrHostKeyChanged
            | Surface::ErrHostKeyRevoked
            | Surface::ErrHostKeyUnknown => ("hacivat-app/src/connections/error_view.rs:", None),
            // ## m2b-t24-shots (follow-up): the labels themselves, by IDs that name them.
            Surface::LiveTlsPinned => ("#sheet-enc-pinned", None),
            Surface::ConnTestOkPinned => ("#diag-tls-value", None),
            Surface::ErrTlsNotOffered => ("#button-label-fix-ssh", None),
            Surface::LiveVaultSecret => ("#sheet-secret-vault", None),
            // ## m3a-t35-shots
            Surface::M3a(s) => s.marker(),
            // ## m3b-t29-shots: each M3b state's own label.
            Surface::M3b(s) => s.marker(),
            // ## m3c-t43-shots: each M3c state's own label.
            Surface::M3c(s) => s.marker(),
            // ## m4-t32-shots
            Surface::M4(s) => s.marker(),
            // ## m4b-t11-editor
            Surface::M4b(s) => s.marker(),
        }
    }

    // ## m2b-t24-shots (fix round 1, I2)
    /// Labels (`#id`) that must be visible too: the rows a surface is about.
    fn also_shown(self) -> &'static [&'static str] {
        match self {
            // The whole route: this computer, the bastion, the database.
            Surface::SshSectionHop => &["#ssh-route-0", "#ssh-route-2"],
            // Both jump rows and the route's end.
            Surface::SshSectionJumps => &["#ssh-jump-index-0", "#ssh-jump-index-1", "#ssh-route-4"],
            // The menu's title row.
            Surface::SshImportMenu => &["#popup-menu-title"],
            // ## m2b-t24-shots (follow-up): the status bar says it too; the fix's neighbour.
            Surface::LiveTlsPinned => &["#status-enc-pinned"],
            Surface::ErrTlsNotOffered => &["#button-label-fix-edit"],
            // ## m3a-t35-shots: a grid state shows its grid (a header it drew), whatever its
            // own marker (the strip, the copy toast).
            Surface::M3a(s) if s.is_grid() => &["#header"],
            // ## m3b-t29-shots: what tells an M3b state from one with the same marker.
            Surface::M3b(s) => s.also_shown(),
            // ## m3c-t43-shots: a grid state's header among them (`m3c::M3c::also_shown`).
            Surface::M3c(s) => s.also_shown(),
            // ## m4-t32-shots
            Surface::M4(s) => s.also_shown(),
            // ## m4b-t11-editor
            Surface::M4b(s) => s.also_shown(),
            _ => &[],
        }
    }

    // ## m2b-t24-shots (follow-up)
    /// Labels (`#id`) that must show whole in the sheet's value column (about 150 px at either
    /// window size), and whether in the pseudo-locales too. "TLS 1.3 · pinned" ends in the word
    /// that tells it from "verified", so it must fit everywhere. "vault · forever" leads with
    /// its word; a pseudo-locale's padding may ellipsize "forever" by the sheet's one-line value
    /// rule (DESIGN.md), as it does "Keychain · forever".
    fn uncut(self) -> &'static [(&'static str, bool)] {
        match self {
            Surface::LiveTlsPinned => &[("#sheet-enc-pinned", true)],
            Surface::LiveVaultSecret => &[("#sheet-secret-vault", false)],
            _ => &[],
        }
    }

    /// A violation when the frame shows no visible label of this surface (fix round 1: a label
    /// scrolled out of its viewport, or laid out below the window, does not count), or does not
    /// show one of [`Surface::also_shown`].
    fn missing_from(self, name: &str, frame: &[RenderedText]) -> Option<String> {
        let (file, text) = self.marker();
        // ## m3a-t35-shots (fix round 1, I2): a label counts only when it drew something.
        let shown = frame
            .iter()
            .any(|r| drawn(r) && r.path.contains(file) && text.is_none_or(|t| strip(&r.text) == t))
            // ## m3b-ci (controller fix): or drawn as access-key runs (`b` + `astion` on Linux).
            || text.is_some_and(|t| {
                (0..frame.len()).any(|i| {
                    is_key_run(frame, i)
                        && frame[i].path.contains(file)
                        && frame[i..i + 3].iter().all(drawn)
                        && key_run_text(frame, i) == t
                })
            });
        if !shown {
            return Some(format!(
                "{name}: the surface is missing: no visible label from {file} {text:?}"
            ));
        }
        let hidden: Vec<&str> = self
            .also_shown()
            .iter()
            .filter(|id| !frame.iter().any(|r| drawn(r) && r.path.ends_with(**id)))
            .copied()
            .collect();
        (!hidden.is_empty())
            .then(|| format!("{name}: the surface is missing: {hidden:?} not visible"))
    }

    // ## m3a-t35-shots (fix round 1, I2)
    /// A violation for each trust-signal group ([`m3a::M3a::required`]) of which no label is
    /// visible and painted: production, read-only, the transaction and the cursor never
    /// collapse (NOTES cross-cutting #7).
    fn trust_missing(self, name: &str, frame: &[RenderedText]) -> Vec<String> {
        // ## m3b-t29-shots: an M3b state's trust signals too.
        let required = match self {
            Surface::M3a(state) => state.required(),
            Surface::M3b(state) => state.required(),
            // ## m3c-t43-shots
            Surface::M3c(state) => state.required(),
            // ## m4b-t21-shots (named line): a connected production script tab's too.
            Surface::M4b(state) => state.required(),
            _ => return Vec::new(),
        };
        // ## m3b-t29-shots (T29 review I2): a trust signal counts only drawn whole: never
        // ellipsized nor clipped (NOTES cross-cutting #7, "never hidden or truncated").
        // ## m4b-t22-pre (T21 r1): nor cut or hidden at a scroll area's fold (`folded`), which
        // check (b) lets through as scrolling.
        let whole = |r: &RenderedText| drawn(r) && !r.truncated && !r.clipped && !r.folded;
        let mut out: Vec<String> = required
            .iter()
            .filter(|group| {
                !frame
                    .iter()
                    .any(|r| whole(r) && group.iter().any(|id| r.path.ends_with(*id)))
            })
            .map(|group| format!("{name}: (t) a trust signal is not shown whole: {group:?}"))
            .collect();
        out.extend(
            self.uncut_trust()
                .into_iter()
                .filter(|id| {
                    frame.iter().any(|r| {
                        r.visible && r.path.ends_with(id) && (r.truncated || r.clipped || r.folded)
                    })
                })
                .map(|id| format!("{name}: (t) a trust label is cut short: {id}")),
        );
        out
    }

    // ## m3b-t29-shots (T29 review I2)
    /// The labels of an M3a or M3b state that must never be ellipsized or clipped, in any locale:
    /// every id of its trust groups, the title bar's connection name where the capsule shows
    /// production, and a typed-name prompt's name.
    fn uncut_trust(self) -> Vec<&'static str> {
        let (required, production) = match self {
            Surface::M3a(state) => (state.required(), state.shows_production()),
            Surface::M3b(state) => (state.required(), state.shows_production()),
            // ## m3c-t43-shots
            Surface::M3c(state) => (state.required(), state.shows_production()),
            // ## m4b-t21-shots (named line; fix round 1, I2)
            Surface::M4b(state) => (state.required(), state.production_capsule()),
            _ => return Vec::new(),
        };
        let mut ids: Vec<&'static str> = required.iter().flat_map(|g| g.iter().copied()).collect();
        if production {
            ids.push("#capsule-text-name");
        }
        ids.push("#confirm-type-name");
        ids
    }

    // ## m2b-t24-shots (follow-up)
    /// A violation for each [`Surface::uncut`] label the frame ellipsized or clipped in
    /// `locale` (a pseudo-locale only where the label must fit there too).
    fn cut_short(self, name: &str, locale: &str, frame: &[RenderedText]) -> Vec<String> {
        self.uncut()
            .iter()
            .filter(|(_, in_pseudo)| *in_pseudo || !PSEUDO.contains(&locale))
            .filter(|(id, _)| {
                frame
                    .iter()
                    .any(|r| r.path.ends_with(*id) && (r.truncated || r.clipped))
            })
            .map(|(id, _)| format!("{name}: {id} is cut short"))
            .collect()
    }

    /// Brings the surface on screen in a freshly started shell.
    fn open(self, window: &mut Window, cx: &mut App) {
        // ## m3a-t35-shots: an M3a state is reached by its steps (`m3a::M3a::steps`).
        if let Surface::M3a(_) = self {
            return;
        }
        // ## m3b-t21-confirm, ## m3b-t24-exec-ui; ## m3b-t29-shots: an M3b dialog opens
        // through its fixture, any other M3b state by its steps.
        if let Surface::M3b(state) = self {
            return state.open(window, cx);
        }
        // ## m3c-t43-shots: every M3c state is reached by its steps.
        if let Surface::M3c(_) = self {
            return;
        }
        // ## m4-t32-shots: an M4 state saves its fixtures, then its steps expand the tree.
        if let Surface::M4(state) = self {
            return state.open(window, cx);
        }
        // ## m4b-t11-editor: an M4b state saves its profile, then its steps open the console.
        if let Surface::M4b(state) = self {
            return state.open(window, cx);
        }
        if self.needs_connections() {
            open_m2a(self, window, cx);
            return;
        }
        let action = match self {
            Surface::Shell | Surface::ShellDemoProduction | Surface::ShellNarrow => return,
            Surface::PaletteOpen => "workspace.command_palette",
            Surface::SettingsGeneral => "workspace.open_settings",
            Surface::DialogAbout => "app.about",
            Surface::ToastInfo => "workspace.reload_translations",
            Surface::MenuApp => {
                open_app_menu(window, cx);
                return;
            }
            _ => unreachable!("an M2a surface opens through its fixture"),
        };
        assert!(
            ActionRegistry::run(action, window, cx),
            "{action} did not run"
        );
    }
}

// ## m2a-t21-shots

/// Opens an M2a surface through the fixture its task owns (R28, P19).
fn open_m2a(surface: Surface, window: &mut Window, cx: &mut App) {
    match surface {
        Surface::ConnDialogNew => dialog::new_fixture(window, cx),
        Surface::ConnDialogPasted => dialog::pasted_fixture(window, cx),
        Surface::ConnDialogEngineSwitch => dialog::engine_switch_fixture(window, cx),
        Surface::ConnDialogSqlite => dialog::sqlite_fixture(window, cx),
        Surface::ConnTestRunning => test_strip::running_fixture(window, cx),
        Surface::ConnTestOk => test_strip::ok_fixture(window, cx),
        Surface::ConnTestFailed => test_strip::failed_fixture(window, cx),
        Surface::ConnErrorAuth => error_view::auth_fixture(window, cx),
        Surface::ConnErrorUnknownDb => error_view::unknown_db_fixture(window, cx),
        Surface::ConnErrorUnsupported => error_view::unsupported_fixture(window, cx),
        Surface::ConnErrorTls => error_view::tls_fixture(window, cx),
        Surface::ConnProdPrompt => prod_prompt::preselected_fixture(window, cx),
        Surface::ConnProdPromptUnset => prod_prompt::unset_fixture(window, cx),
        Surface::ExplorerEmpty => explorer::empty_fixture(window, cx),
        // The menu opens in `after_paint`, under the selected row as Shift-F10 does: the
        // row's bounds exist only once the list has painted.
        Surface::ExplorerList | Surface::ShellMinimum | Surface::ExplorerMenu => {
            explorer::list_fixture(window, cx)
        }
        Surface::ConnDeleteConfirm => delete::confirm_fixture(window, cx),
        Surface::ConnImportPgService => import::pg_service_fixture(window, cx),
        Surface::CapsuleConnecting => capsule::connecting_fixture(window, cx),
        Surface::CapsuleConnectedUnencrypted => capsule::connected_unencrypted_fixture(window, cx),
        Surface::CapsuleProductionRo => capsule::production_ro_fixture(window, cx),
        Surface::CapsuleFailed => capsule::failed_fixture(window, cx),
        Surface::CapsuleUnsupported => capsule::unsupported_fixture(window, cx),
        Surface::UnencryptedPopover => capsule::unencrypted_popover_fixture(window, cx),
        // ## m2b-t22-live
        Surface::LiveReconnecting => capsule::reconnecting_fixture(window, cx),
        Surface::LiveIdle => capsule::idle_fixture(window, cx),
        Surface::LiveLostTx => capsule::lost_tx_fixture(window, cx),
        Surface::LiveLostReconnected => capsule::lost_reconnected_fixture(window, cx),
        Surface::LiveGap => capsule::gap_fixture(window, cx),
        Surface::LiveRoEnforced => capsule::ro_enforced_fixture(window, cx),
        Surface::LiveRoGuarded => capsule::ro_guarded_fixture(window, cx),
        Surface::LiveRoPg13 => capsule::ro_pg13_fixture(window, cx),
        Surface::LiveSshRo => capsule::ssh_ro_fixture(window, cx),
        Surface::LiveSshRw => capsule::ssh_rw_fixture(window, cx),
        // ## m2b-t24-shots
        Surface::CertReview => cert_dialog::untrusted_fixture(window, cx),
        Surface::CertPinChanged => cert_dialog::pin_changed_fixture(window, cx),
        Surface::PlaintextQuestion => plaintext_prompt::fixture(window, cx),
        Surface::ConnDialogTlsPinned => dialog::tls_pinned_fixture(window, cx),
        Surface::SshSectionOff => ssh::off_fixture(window, cx),
        Surface::SshSectionFolded => ssh::folded_fixture(window, cx),
        Surface::SshSectionHop => ssh::hop_fixture(window, cx),
        Surface::SshSectionJumps => ssh::jumps_fixture(window, cx),
        Surface::SshImportMenu => {
            point_import_at_fixture_config(cx);
            ssh::import_fixture(window, cx);
        }
        Surface::HostKeyNew => hostkey_prompt::new_fixture(window, cx),
        Surface::HostKeyChanged => hostkey_prompt::changed_fixture(window, cx),
        Surface::HostKeyChangedOpenSsh => hostkey_prompt::changed_openssh_fixture(window, cx),
        Surface::KbdPrompt => kbd_prompt::fixture(window, cx),
        Surface::SshPassword => secret_prompt::password_fixture(window, cx),
        Surface::SshPassphrase => secret_prompt::passphrase_fixture(window, cx),
        Surface::VaultCreate => vault_dialog::create_fixture(window, cx),
        Surface::VaultUnlock => vault_dialog::unlock_fixture(window, cx),
        Surface::VaultReset => vault_dialog::reset_fixture(window, cx),
        Surface::VaultDamaged => vault_dialog::damaged_fixture(window, cx),
        Surface::LiveReconnectingStaging => capsule::reconnecting_staging_fixture(window, cx),
        Surface::LiveLostClean => capsule::lost_clean_fixture(window, cx),
        Surface::ErrInsecureTls => error_view::insecure_tls_fixture(window, cx),
        Surface::ErrInsecurePlain => error_view::insecure_plain_fixture(window, cx),
        Surface::ErrPinChanged => error_view::pin_changed_fixture(window, cx),
        Surface::ErrSshAuth => error_view::ssh_auth_fixture(window, cx),
        Surface::ErrSshChannel => error_view::ssh_channel_fixture(window, cx),
        Surface::ErrSshAgent => error_view::ssh_agent_fixture(window, cx),
        Surface::ErrHostKeyChanged => error_view::host_key_changed_fixture(window, cx),
        Surface::ErrHostKeyRevoked => error_view::host_key_revoked_fixture(window, cx),
        Surface::ErrHostKeyUnknown => error_view::host_key_unknown_fixture(window, cx),
        // ## m2b-t24-shots (follow-up)
        Surface::LiveTlsPinned => capsule::tls_pinned_fixture(window, cx),
        Surface::ConnTestOkPinned => test_strip::ok_pinned_fixture(window, cx),
        Surface::ErrTlsNotOffered => error_view::tls_not_offered_fixture(window, cx),
        Surface::LiveVaultSecret => capsule::vault_secret_fixture(window, cx),
        // The widget owns no copy: the app's strings (the Copy as URL toast, the keyring
        // fallback as a long bullet that wraps, the toast's own Dismiss).
        Surface::ToastStickyDemo => hacivat_ui::toast::sticky_demo_fixture(
            text_fn_id("hv-conn-toast-copied"),
            vec![
                text_fn_id("hv-conn-toast-nopw"),
                text_fn_id("hv-conn-keyring-unavailable"),
            ],
            text_fn_id("hui-toast-dismiss"),
            window,
            cx,
        ),
        _ => unreachable!("an M1 surface opens through its action"),
    }
}

// ## m2b-t24-shots

/// The `~/.ssh/config` the import menu shot reads: a bastion behind a jump host, a plain host
/// and a `ProxyCommand` entry (listed, disabled with its reason). No key files: a path under the
/// fixture's temp home would draw a different folder on every run.
const SSH_CONFIG: &str = "Host bastion\n HostName bastion.acme.io\n User deploy\n ProxyJump jump-eu\n\
Host jump-eu\n HostName jump-eu.acme.io\n User deploy\n\
Host reports\n HostName 10.0.4.7\n User ops\n\
Host legacy\n HostName old.acme.io\n ProxyCommand ssh -W %h:%p gw.acme.io\n";

/// Writes [`SSH_CONFIG`] under the shot app's temp root and points the import at it.
fn point_import_at_fixture_config(cx: &mut App) {
    let home = cx.global::<AppState>().paths.config.join("ssh-home");
    let config = home.join(".ssh").join("config");
    std::fs::create_dir_all(config.parent().expect("a folder")).expect("the fixture's ~/.ssh");
    std::fs::write(&config, SSH_CONFIG).expect("the fixture config");
    cx.set_global(SshImportEnv {
        config,
        home,
        local_user: "deploy".to_string(),
    });
}

/// Whether the surface's asynchronous step has landed (the import menu opens once the core
/// runtime has read the config); every other surface is ready at once.
fn ready(surface: Surface, window: &mut Window, cx: &mut App) -> bool {
    match surface {
        Surface::SshImportMenu => {
            ssh::section_state(window, cx).is_some_and(|s| s.read(cx).import_open())
        }
        _ => true,
    }
}

/// The step of a surface that needs one painted frame first.
fn after_paint(surface: Surface, window: &mut Window, cx: &mut App) {
    if surface == Surface::ExplorerMenu {
        explorer::open_context_menu(None, window, cx);
    }
}

/// The connections an M2a shot boots: `ConnectionsBoot::for_tests` (a memory secret store, no
/// credential files) over a `FakeDriver` per engine, so no shot touches the keyring, a socket
/// or DNS.
fn fake_connections(paths: &LocalPaths) -> ConnectionsBoot {
    let mut drivers = DriverRegistry::new();
    for kind in [
        DbmsKind::Postgres,
        DbmsKind::MySql,
        DbmsKind::MariaDb,
        DbmsKind::Sqlite,
    ] {
        drivers.register(Arc::new(FakeDriver::new(kind)));
    }
    ConnectionsBoot::for_tests(paths, drivers)
}

// ## m2b-t24-shots (follow-up)
/// A secret store whose keyring probe has chosen Hacivat's vault, as on Linux without a keyring
/// (B13). The vault file under the shot's temp root is never created.
fn vault_store(paths: &LocalPaths) -> Arc<SecretStore> {
    let backend = FallbackBackend::new(
        Arc::new(MemoryBackend::new()),
        Arc::new(FileVault::open(paths.data.join("vault.bin"))),
        Box::new(|| false),
    );
    assert!(backend.probe_available(), "the probe ran");
    assert_eq!(backend.choice(), Some(BackendChoice::Vault));
    Arc::new(SecretStore::new(Some(Arc::new(backend))))
}

/// `app.show_menu`'s popup. The action exists only where the title bar draws the app-menu
/// button (Windows, Linux); on macOS the harness opens the same popup directly.
fn open_app_menu(window: &mut Window, cx: &mut App) {
    if cfg!(not(target_os = "macos")) {
        assert!(ActionRegistry::run("app.show_menu", window, cx));
        return;
    }
    let ws = window
        .root::<Workspace>()
        .flatten()
        .expect("the root view is the workspace");
    let items = menus::popup_items(&menus::menu_model(cx.global::<ActionRegistry>()), cx);
    let anchor = menus::in_window_menu_anchor(&items, window, cx);
    let menu = ws.update(cx, |ws, cx| ws.open_popup_menu(anchor, items, window, cx));
    menu.update(cx, |m, _| m.label(text_fn_id("hui-menu-app")));
}

/// One frame to render.
#[derive(Debug, Clone, Copy)]
struct Shot {
    surface: Surface,
    theme: ThemePref,
    locale: &'static str,
    size: (f32, f32),
}

impl Shot {
    fn theme_name(&self) -> &'static str {
        match self.theme {
            ThemePref::Light => "light",
            _ => "dark",
        }
    }

    fn file_name(&self, scale: f32) -> String {
        format!(
            "{}-{}-{}-{}x{}@{}x.png",
            self.surface.name(),
            self.theme_name(),
            self.locale,
            self.size.0,
            self.size.1,
            scale
        )
    }
}

/// Every shot: the surface matrix at each surface's sizes x locales x themes, then
/// `shell-demo-production` at 1440x900 x en/tr x dark/light for VF.
fn shots() -> Vec<Shot> {
    let mut out = Vec::new();
    for surface in SURFACES {
        for &size in surface.sizes() {
            for theme in THEMES {
                for locale in LOCALES {
                    out.push(Shot {
                        surface,
                        theme,
                        locale,
                        size,
                    });
                }
            }
        }
    }
    for theme in THEMES {
        for locale in ["en", "tr"] {
            out.push(Shot {
                surface: Surface::ShellDemoProduction,
                theme,
                locale,
                size: (1440.0, 900.0),
            });
        }
    }
    // ## m3a-t35-shots: the M3a states, after every M1/M2 shot.
    for state in m3a::M3A {
        push_matrix(&mut out, Surface::M3a(state));
    }
    // ## m3b-t29-shots: then every M3b state (`m3b::all`: Tasks 22, 25, 21, 23, 24, 27, 26 in
    // the order they were added, then Task 29's).
    for state in m3b::all() {
        push_matrix(&mut out, Surface::M3b(state));
    }
    // ## m3c-t43-shots: then every M3c state (`m3c::all`: Tasks 31 to 38 in order).
    for state in m3c::all() {
        push_matrix(&mut out, Surface::M3c(state));
    }
    // ## m4-t32-shots: then every M4 state.
    for state in m4::M4_STATES {
        push_matrix(&mut out, Surface::M4(state));
    }
    // ## m4b-t11-editor: then every M4b state.
    for state in m4b::M4B_STATES {
        push_matrix(&mut out, Surface::M4b(state));
    }
    out
}

// ## m3b-t29-shots
/// Pushes `surface` at each of its sizes, in every theme and locale.
fn push_matrix(out: &mut Vec<Shot>, surface: Surface) {
    for &size in surface.sizes() {
        for theme in THEMES {
            for locale in LOCALES {
                out.push(Shot {
                    surface,
                    theme,
                    locale,
                    size,
                });
            }
        }
    }
}

fn out_dir() -> PathBuf {
    std::env::var_os("SHOTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../target/shots")
                .canonicalize()
                .unwrap_or_else(|_| {
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/shots")
                })
        })
}

// ## m3a-t35-shots (fix round 1, I2)
/// A label that is visible and drew something: what a marker or a trust check may count. A
/// truncating label that painted nothing drew nothing (it is also recorded not visible, M3);
/// a label recorded before layout (the notice, `hacivat-ui/src/notice.rs`) carries no painted
/// width, so painted 0 alone does not mean nothing was drawn.
fn drawn(r: &RenderedText) -> bool {
    r.visible && (r.painted_px > 0.0 || !r.truncated)
}

/// Without isolation marks (U+2068/U+2069), which wrap placeables.
fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

/// Assertions (a) to (c) on what the frame drew; one message per violation.
fn check_labels(name: &str, locale: &str, frame: &[RenderedText]) -> Vec<String> {
    let mut out = Vec::new();
    // ## m2b-t24-shots
    let split = split_pieces(frame);
    // ## m3b-ci (controller fix): a label drawn as access-key runs is checked whole.
    let key_runs = key_run_pieces(frame);
    for (i, r) in frame.iter().enumerate() {
        let text = strip(&r.text);
        if PSEUDO.contains(&locale)
            && r.source == TextSource::Fluent
            && !bracketed(&text)
            && !split.contains(&i)
            && !key_runs.contains(&i)
        {
            out.push(format!(
                "{name}: (a) Fluent label not bracketed: {:?} at {}",
                text, r.path
            ));
        }
        // ## m3a-t35-shots: a label cut away whole (not visible) shows no cut text.
        if r.clipped && r.visible {
            out.push(format!(
                "{name}: (b) clipped: {:?} ({:.1} px painted) at {}",
                text, r.painted_px, r.path
            ));
        }
        if r.size_px < 11.0 {
            out.push(format!(
                "{name}: (c) {} px < 11: {:?} at {}",
                r.size_px, text, r.path
            ));
        }
        // ## m2b-t24-shots (fix round 1, I1)
        if r.outside_card {
            out.push(format!(
                "{name}: (f) runs past its dialog card: {:?} at {}",
                text, r.path
            ));
        }
    }
    out
}

// ## m3a-t35-shots
/// (g) The grid's own text (the grid records its runs, `hacivat-ui-grid` `layers::record_text`):
/// a header is data (a column name and its type, `TextSource::Catalog`), so in the pseudo
/// locales a bracketed header, or a header recorded as Fluent anywhere, is a pseudo-localised
/// data label; a grid run that is clipped (wider than its cell's or header's room, cut by the
/// column instead of ellipsized) fails.
fn check_grid(name: &str, locale: &str, frame: &[RenderedText]) -> Vec<String> {
    let mut out = Vec::new();
    for r in frame
        .iter()
        .filter(|r| r.path.contains("hacivat-ui-grid/src/"))
    {
        let text = strip(&r.text);
        let header = r.path.ends_with("#header");
        if header
            && (r.source == TextSource::Fluent || PSEUDO.contains(&locale) && bracketed(&text))
        {
            out.push(format!(
                "{name}: (g) a grid header is data, never pseudo-localised: {text:?} at {}",
                r.path
            ));
        }
        if r.clipped {
            out.push(format!(
                "{name}: (g) clipped grid text: {text:?} ({:.1} px painted) at {}",
                r.painted_px, r.path
            ));
        }
    }
    out
}

// ## m2b-t24-shots
fn bracketed(text: &str) -> bool {
    text.starts_with('[') && text.ends_with(']')
}

// ## m3b-ci (controller fix): access-key runs are one label (CI run 44051024805, Linux: 876 of
// its 960 violations were (a) on these runs, and the 16 `ssh-import-menu` "missing" its
// `bastion` marker).
/// Whether `frame[i..i + 3]` is one label drawn as access-key runs. Where access keys show
/// (Linux and Windows; macOS hides them, so a macOS render never draws these) a button's or a
/// menu row's label is drawn as three runs around its key letter (`hacivat-ui/src/button.rs`,
/// `key_label_from`): `[` `Ƈ` `ŀǿǿşḗ]` in a pseudo locale, `""` `b` `astion` in English. The
/// shape is the one that split draws, and the one `tests/support/mod.rs` `button_labels`
/// joins: three consecutive records from `button.rs` with one `#id` (the label's own, so the
/// labels of two different controls never join), one source, and the middle run one character
/// (the key letter). This changes no other check: each run is still checked on its own by (b),
/// (c) and (f) (a clipped key letter is a clipped label), and the trust checks are unchanged.
fn is_key_run(frame: &[RenderedText], i: usize) -> bool {
    let Some([before, key, after]) = frame.get(i..i + 3) else {
        return false;
    };
    let one_label = |r: &RenderedText| r.path == before.path && r.source == before.source;
    before.path.contains("hacivat-ui/src/button.rs:")
        && before.path.contains('#')
        && one_label(key)
        && one_label(after)
        && strip(&key.text).chars().count() == 1
}

/// The joined text of the access-key runs at `frame[i..i + 3]` ([`is_key_run`]).
fn key_run_text(frame: &[RenderedText], i: usize) -> String {
    frame[i..i + 3].iter().map(|r| strip(&r.text)).collect()
}

/// (a)'s second exception: the frame indices of every access-key label ([`is_key_run`]) whose
/// joined text is bracketed, so its runs pass (a) as the one label they are. Runs that join
/// into an unbracketed text stay checked run by run: (a) never passes a label that is not
/// bracketed whole.
fn key_run_pieces(frame: &[RenderedText]) -> std::collections::BTreeSet<usize> {
    let mut pieces = std::collections::BTreeSet::new();
    let mut i = 0;
    while i < frame.len() {
        if is_key_run(frame, i) && bracketed(&key_run_text(frame, i)) {
            pieces.extend(i..i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    pieces
}

/// (a)'s one exception (Task 24; fix round 1, M2): a Fluent message drawn in pieces around a
/// literal ("Type `reset` to confirm", the word in mono). In frame order: a Fluent piece that
/// opens the bracket, then, alternately, one data piece and a Fluent piece from the first
/// piece's site, up to the piece that closes the bracket; no piece is bracketed on its own.
/// Returns the frame indices of those pieces; anything else is checked label by label.
fn split_pieces(frame: &[RenderedText]) -> std::collections::BTreeSet<usize> {
    let fluent = |r: &RenderedText| r.source == TextSource::Fluent;
    let mut pieces = std::collections::BTreeSet::new();
    let mut i = 0;
    while i < frame.len() {
        let first = strip(&frame[i].text);
        if !(fluent(&frame[i]) && first.starts_with('[') && !first.ends_with(']')) {
            i += 1;
            continue;
        }
        let mut run = vec![i];
        let mut at = i;
        let closed = loop {
            let (Some(literal), Some(next)) = (frame.get(at + 1), frame.get(at + 2)) else {
                break false;
            };
            let piece = strip(&next.text);
            if fluent(literal)
                || !fluent(next)
                || next.path != frame[i].path
                || piece.starts_with('[')
            {
                break false;
            }
            run.push(at + 2);
            at += 2;
            if piece.ends_with(']') {
                break true;
            }
        };
        if closed {
            pieces.extend(run);
            i = at + 1;
        } else {
            i += 1;
        }
    }
    pieces
}

// ## m2b-t22-live (fix round 2, R1-I1)
/// (e) The title bar's layout (a geometry check: an icon button pushed off the window draws no
/// clipped text): the trailing controls sit wholly inside the window and end where the bar's
/// right padding puts them, the palette field keeps `PALETTE_MIN_PX` and ends before them,
/// and the capsule ends before the palette field begins.
fn check_title_bar(width: f32, bar: TitleBarProbe, capsule: Option<Bounds<Pixels>>) -> Vec<String> {
    const SLACK: f32 = 0.5;
    let mut out = Vec::new();
    let Some(trailing) = bar.trailing else {
        return vec!["the title bar drew no trailing controls".to_string()];
    };
    let (left, right) = (f32::from(trailing.left()), f32::from(trailing.right()));
    let end = width - bar.right_padding;
    if left < -SLACK || right > width + SLACK {
        out.push(format!(
            "the trailing controls ({left:.1}..{right:.1}) leave the {width} px window"
        ));
    } else if (right - end).abs() > SLACK {
        out.push(format!(
            "the trailing controls end at {right:.1}, not at {end:.1}: something pushed them"
        ));
    }
    let Some(palette) = bar.palette else {
        out.push("the title bar drew no palette field".to_string());
        return out;
    };
    let (p_left, p_right) = (f32::from(palette.left()), f32::from(palette.right()));
    let palette_w = f32::from(palette.size.width);
    // ## m4-m3carry (U-6 c, P2-04): or it is its icon.
    if bar.palette_collapsed {
        if (palette_w - PALETTE_ICON_PX).abs() > SLACK {
            out.push(format!(
                "the collapsed palette field is {palette_w:.1} px, not its {PALETTE_ICON_PX} px \
                 icon"
            ));
        }
    } else if palette_w < PALETTE_MIN_PX - SLACK {
        out.push(format!(
            "the palette field is {palette_w:.1} px, under its {PALETTE_MIN_PX} px minimum"
        ));
    }
    if p_left < -SLACK || p_right > left + SLACK {
        out.push(format!(
            "the palette field ({p_left:.1}..{p_right:.1}) runs into the trailing controls at \
             {left:.1}"
        ));
    }
    if let Some(capsule) = capsule {
        let c_right = f32::from(capsule.right());
        if c_right > p_left + SLACK {
            out.push(format!(
                "the capsule ends at {c_right:.1}, past the palette field's start at {p_left:.1}"
            ));
        }
    }
    out
}

// ## m4-m3carry (fix round 1, m1)
thread_local! {
    /// Shots whose capsule showed a trust word when (e) checked them (non-empty trust rooms
    /// and a room), and of those, shots whose palette field was collapsed. The run prints both,
    /// so a run where (e) checked nothing real is visible.
    static TRUST_COVERAGE: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

fn note_trust_coverage(bar: TitleBarProbe, capsule: CapsuleProbe) {
    let real = capsule.bounds.is_some()
        && capsule.max_width.is_some()
        && bar.palette.is_some()
        && capsule.trust_rooms.0.iter().any(Option::is_some);
    if real {
        TRUST_COVERAGE.with(|c| {
            let (n, collapsed) = c.get();
            c.set((n + 1, collapsed + usize::from(bar.palette_collapsed)));
        });
    }
}

// ## m4-m3carry (U-6 c; M3 PO item P2-04)
/// (e) The capsule gives up no trust word (an access word, the unencrypted word) that the
/// palette field's icon would keep: beside a whole field, the room the icon frees would keep
/// none; a collapsed field keeps at least one. The capsule's own fit agrees with the trust
/// rooms it reported.
fn check_trust_room(bar: TitleBarProbe, capsule: CapsuleProbe) -> Option<String> {
    let max = capsule.max_width.filter(|_| capsule.bounds.is_some())?;
    bar.palette?;
    let rooms = capsule.trust_rooms;
    let gain = PALETTE_MIN_PX - PALETTE_ICON_PX;
    let given_up = usize::from(capsule.trust_given_up);
    if given_up != rooms.given_up_at(max) {
        return Some(format!(
            "the capsule gave up {given_up} trust words at {max:.1} px; its trust rooms say {}",
            rooms.given_up_at(max)
        ));
    }
    if bar.palette_collapsed {
        (rooms.given_up_at(max - gain) <= given_up).then(|| {
            "the palette field collapsed to its icon and kept no trust word (P2-04)".to_string()
        })
    } else {
        (rooms.given_up_at(max + gain) < given_up).then(|| {
            "the capsule gave up a trust word that the palette field's icon would keep (P2-04)"
                .to_string()
        })
    }
}

// ## m3c-threads
/// The platform text system every shot's app shapes with: one per process (the render runs on
/// `main`), built from the one platform made for it. A platform made per shot only to take its
/// text system is never freed on Linux (its event loop's foreground source holds a clone of
/// the loop's own handle, an `Rc` cycle), and with it its dispatcher's worker and timer
/// threads stayed alive: about five threads per shot on a 4-CPU runner, until the full matrix
/// ran out of threads (CI run 44152258028, after 3,314 shots).
fn shot_text_system() -> Arc<dyn PlatformTextSystem> {
    thread_local! {
        static TEXT_SYSTEM: Arc<dyn PlatformTextSystem> = current_platform(true).text_system();
    }
    TEXT_SYSTEM.with(Arc::clone)
}

// ## m4b-t21-shots fix round 1 (I2)
/// What is wrong with the chooser rows' declared names in `decls`: none declared, or a row
/// whose name does not begin with an environment word (the swatch is the only other cue), or
/// the production row without the production word.
fn chooser_rows_named(cx: &App, decls: &[hacivat_ui::popover::A11yDecl]) -> Vec<String> {
    use hacivat_app::connections::capsule::env_word;
    use hacivat_proto::EnvTag;
    let words: Vec<String> = [
        Some(EnvTag::Production),
        Some(EnvTag::Staging),
        Some(EnvTag::Test),
        Some(EnvTag::Dev),
        Some(EnvTag::Local),
        None,
    ]
    .into_iter()
    .map(|t| strip(&env_word(cx, t)))
    .collect();
    let rows: Vec<String> = decls
        .iter()
        .filter(|d| d.role == Some(hacivat_ui_core::gpui_surface::Role::ListBoxOption))
        .map(|d| strip(&d.name))
        .filter(|n| n.contains("orders-"))
        .collect();
    let mut out = Vec::new();
    if rows.is_empty() {
        out.push("no chooser row declared its name".to_string());
    }
    for row in &rows {
        if !words.iter().any(|w| row.starts_with(&format!("{w} "))) {
            out.push(format!("a chooser row names no environment: {row:?}"));
        }
        if row.contains("orders-prod ") && !row.starts_with(&format!("{} ", words[0])) {
            out.push(format!("the production row does not say so: {row:?}"));
        }
    }
    out
}

// ## m4b-t21-shots (named exception, R-M4b-44): the shot's temp root outlives its app.
thread_local! {
    /// Where [`shot_root`] makes a shot's temp root (`None`: `$TMPDIR/hacivat-shots`).
    /// `shots_render_leaves_no_temp_dir` points it at a folder of its own and checks that
    /// every root is gone once its shot is.
    static SHOT_TMP: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

// ## m4b-t21-shots fix round 1 (m10): a shot's root has a stable path (its shot's name hashed),
// so a path the app shows (Settings, the attach dialog) reads the same in every run on a
// machine; shards render different shots, so their roots never meet.
/// The temp root of shot `name`, under [`SHOT_TMP`]'s folder or `$TMPDIR/hacivat-shots`.
fn shot_root_path(name: &str) -> PathBuf {
    use std::hash::{Hash as _, Hasher as _};
    let mut h = std::hash::DefaultHasher::new();
    name.hash(&mut h);
    let base = SHOT_TMP
        .with(|t| t.borrow().clone())
        .unwrap_or_else(|| std::env::temp_dir().join("hacivat-shots"));
    base.join(format!("hv-{:08x}", h.finish() as u32))
}

/// A shot's temp root, held under its lock (`<root>.lock`), removed (then unlocked) when
/// dropped.
struct ShotRoot(PathBuf);

impl ShotRoot {
    fn path(&self) -> &Path {
        &self.0
    }

    fn lock_of(root: &Path) -> PathBuf {
        root.with_extension("lock")
    }
}

impl Drop for ShotRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        let _ = std::fs::remove_dir_all(Self::lock_of(&self.0));
    }
}

// ## m4b-t21-shots fix round 2 (m-e): two runs on one machine (two worktrees, two agents)
// that render the same shot share its stable path; the root's lock (an atomic `mkdir`) makes
// the second wait for the first instead of deleting its live root. A lock older than
// `STALE_LOCK` is a crashed run's and is taken over.
/// How old a shot root's lock is before it counts as a crashed run's.
const STALE_LOCK: std::time::Duration = std::time::Duration::from_secs(600);

/// The temp root of shot `name` ([`shot_root_path`]): locked, then made empty (a crashed
/// run's leftover goes).
fn shot_root(name: &str) -> ShotRoot {
    let path = shot_root_path(name);
    let lock = ShotRoot::lock_of(&path);
    if let Some(parent) = lock.parent() {
        std::fs::create_dir_all(parent).expect("the shots' temp folder");
    }
    take_lock(&lock, name);
    // ## m4b-t22-pre (T21 r4): the guard owns the lock from here, so a panic below (the root's
    // `create_dir_all`) still releases it.
    let root = ShotRoot(path);
    let _ = std::fs::remove_dir_all(root.path());
    std::fs::create_dir_all(root.path()).expect("temp dir");
    CURRENT_ROOT.with(|c| *c.borrow_mut() = Some(root.path().to_path_buf()));
    root
}

// ## m4b-t22-pre (T21 r4): the lock names its owner (`<lock>/pid`): a lock whose owner is no
// longer running (a killed or aborted run) is taken over at once; `STALE_LOCK` stays the rule
// for a lock with no readable owner, or an owner that cannot be checked.
/// The file in a shot root's lock that holds its owner's process id.
const LOCK_OWNER: &str = "pid";

/// Takes the lock `lock` (an atomic `mkdir`) for shot `name`, writing this process's id into
/// it: waits while another live run holds it, takes it over when its owner is dead or it is
/// older than [`STALE_LOCK`].
fn take_lock(lock: &Path, name: &str) {
    let deadline = std::time::Instant::now() + 2 * STALE_LOCK;
    while std::fs::create_dir(lock).is_err() {
        let stale = std::fs::metadata(lock)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE_LOCK);
        if stale || lock_owner_dead(lock) {
            let _ = std::fs::remove_dir_all(lock);
            continue;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{name}: another run holds {}",
            lock.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // A run that sees the lock before this write reads no owner, and waits.
    let _ = std::fs::write(
        lock.join(LOCK_OWNER),
        std::process::id().to_string().as_bytes(),
    );
}

/// Whether the lock `lock` names an owner that is no longer running. `false` when it names
/// none (yet), names this process, or the check cannot run.
fn lock_owner_dead(lock: &Path) -> bool {
    let Some(pid) = std::fs::read_to_string(lock.join(LOCK_OWNER))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
    else {
        return false;
    };
    pid != std::process::id() && process_gone(pid)
}

/// Whether no process `pid` runs (`ps -p`, as `live_threads` reads; `false` when `ps` cannot
/// run, or off Unix).
fn process_gone(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("ps")
            .args(["-p", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.code() == Some(1))
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// T21 r4: a shot root's lock left by a run that is gone is taken over at once; one held by a
/// live run is waited for; the stale rule still takes over an old lock whose owner is unknown.
#[cfg(not(unix))]
fn a_dead_runs_shot_lock_is_taken_over_at_once() {}

/// T21 r4 (Unix: the shots run there).
#[cfg(unix)]
fn a_dead_runs_shot_lock_is_taken_over_at_once() {
    let tmp = tempfile::tempdir().expect("the check's temp folder");
    // A process that has exited: its id names no running process.
    let mut child = std::process::Command::new("true")
        .spawn()
        .expect("spawn `true`");
    let dead = child.id();
    child.wait().expect("`true` ends");
    if !process_gone(dead) {
        // Its id was taken again at once: nothing to check with it.
        println!("shots: the dead child's id {dead} is running again; check skipped");
        return;
    }
    let run = |lock: PathBuf| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            take_lock(&lock, "check");
            let _ = tx.send(std::fs::read_to_string(lock.join(LOCK_OWNER)).ok());
        });
        rx
    };
    let held = |name: &str, owner: Option<u32>| {
        let lock = tmp.path().join(name);
        std::fs::create_dir(&lock).expect("a held lock");
        if let Some(pid) = owner {
            std::fs::write(lock.join(LOCK_OWNER), pid.to_string()).expect("its owner");
        }
        lock
    };
    let me = std::process::id().to_string();
    // A dead owner: taken over at once, and the lock now names this process.
    let rx = run(held("dead.lock", Some(dead)));
    let owner = rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("a dead run's lock is taken over at once, not after STALE_LOCK");
    assert_eq!(
        owner.as_deref(),
        Some(me.as_str()),
        "the lock names its new owner"
    );
    // A live owner (another live process: this one's parent, or `launchd`/`init`): waited for.
    let parent = std::os::unix::process::parent_id();
    let lock = held("live.lock", Some(parent));
    let rx = run(lock.clone());
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(300))
            .is_err(),
        "a live run's lock is not taken"
    );
    // Its owner lets go: the waiter takes it.
    std::fs::remove_dir_all(&lock).expect("let go");
    let owner = rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the lock is taken once free");
    assert_eq!(owner.as_deref(), Some(me.as_str()));
    // No owner named, but older than STALE_LOCK: the stale rule takes it over.
    let lock = held("old.lock", None);
    let old = std::time::SystemTime::now() - STALE_LOCK - std::time::Duration::from_secs(60);
    std::fs::File::open(&lock)
        .and_then(|f| f.set_modified(old))
        .expect("age the lock");
    let rx = run(lock);
    let owner = rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("a stale lock is taken over");
    assert_eq!(owner.as_deref(), Some(me.as_str()));
}

thread_local! {
    /// The root of the shot being rendered ([`shot_folder`]).
    static CURRENT_ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// A fixture folder `name` inside the shot's own root (fix round 1, m10: a stable path the
/// app may show, gone with the shot).
fn shot_folder(name: &str) -> PathBuf {
    CURRENT_ROOT
        .with(|c| c.borrow().clone())
        .expect("a shot is being rendered")
        .join("folders")
        .join(name)
}

/// The entries left in `base` (the temp roots of shots that are over).
fn left_in(base: &Path) -> Vec<String> {
    std::fs::read_dir(base)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Queues the shape of the CI flake: a task whose update flushes what is pending (any task's
/// update does: the full frame `refresh_windows` asked for draws there), then, one poll later,
/// redraws the window without refreshing it (it notifies the root view, so every cached view
/// is reused), as a late status or session update does. Only a settle runs it, so it reaches
/// the read frame only if the harness settles after asking for its full frame.
fn late_redraw(cx: &mut HeadlessAppContext, handle: AnyWindowHandle) {
    use hacivat_ui_core::gpui_surface::AppContext as _;
    cx.update(|cx| {
        cx.spawn(async move |cx| {
            let _ = cx.update_window(handle, |_, _, _| {});
            YieldOnce(false).await;
            let _ = cx.update_window(handle, |root, _, cx| cx.notify(root.entity_id()));
        })
        .detach();
    });
}

/// Pending once (waking itself), then ready: the task's next poll comes after the tasks
/// already queued.
struct YieldOnce(bool);

impl std::future::Future for YieldOnce {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        if self.0 {
            return std::task::Poll::Ready(());
        }
        self.0 = true;
        cx.waker().wake_by_ref();
        std::task::Poll::Pending
    }
}
// ## m4b-t21-shots

/// Renders `shot` in a fresh app, checks the frame and writes the PNG. Returns the violations.
fn render(shot: Shot, dir: &Path) -> Vec<String> {
    // ## m4b-t21-shots (named exception, R-M4b-44): the root is made before the app, so it
    // is dropped after it (locals drop in reverse order, on a panic too): the app's teardown
    // saves the workspace again into `data/`, which used to re-create the root after its
    // removal (the M4 gate's 3,464 `$TMPDIR/.tmp*/data/` folders).
    let root = shot_root(&shot.file_name(2.0));
    let mut cx = HeadlessAppContext::with_platform(
        shot_text_system(),
        Arc::new(BundledAssets),
        current_headless_renderer,
    );
    cx.update(|cx| {
        bootstrap(cx).expect("bundled fonts register and resolve");
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
        cx.set_global(CapsuleProbe::default());
        cx.set_global(TitleBarProbe::default());
        // ## m4-t32-shots (m3carry review m3): a connected fixture's latency is fixed.
        cx.set_global(hacivat_app::connections::capsule::PinnedLatency(12));
    });
    let paths = LocalPaths::under(root.path());
    paths.ensure_dirs().expect("test dirs");
    let mut outcome = hacivat_settings::load(&paths.settings_toml());
    outcome.settings.ui.language = LanguagePref::Tag(shot.locale.to_string());
    outcome.settings.ui.theme = shot.theme;
    // ## m3a-t35-shots: an M3a shot connects for real, to a fake that dials itself.
    let m3a_state = match shot.surface {
        Surface::M3a(s) => Some(s),
        _ => None,
    };
    // ## m3b-t29-shots: an M3b state reached by steps connects as an M3a one; an M3b dialog
    // opens its fixture over the M2a fakes, as it did before it moved.
    let m3b_state = shot.surface.m3b();
    // ## m3c-t43-shots: every M3c state is reached by steps.
    let m3c_state = shot.surface.m3c();
    let stepped =
        m3a_state.is_some() || m3b_state.is_some_and(|s| !s.is_fixture()) || m3c_state.is_some();
    // ## m4b-t14-tabs (named line): a connected script tab connects as an M3a state does.
    let stepped = stepped || matches!(shot.surface, Surface::M4b(s) if s.connects());
    let mut connections = shot.surface.needs_connections().then(|| {
        if stepped {
            m3a::connections(&paths)
        } else {
            fake_connections(&paths)
        }
    });
    // ## m2b-t24-shots (follow-up): the vault shot runs on a machine without a keyring.
    if shot.surface == Surface::LiveVaultSecret
        && let Some(boot) = connections.as_mut()
    {
        boot.secrets = vault_store(&paths);
    }
    // ## m3c-t43b: a narrow-pane state restores its split layout (`workspace.json`).
    if let Some(state) = m3c_state {
        m3c::before_start(state, &paths, shot.size);
    }
    let mut startup = Startup::new(paths, outcome, OsLocale::default());
    startup.connections = connections;
    let demo = shot.surface.is_demo().then(DemoCapsule::harman);
    let handle: AnyWindowHandle = cx
        .update(|cx| start(cx, startup, demo))
        .expect("the workspace opens")
        .into();
    let (w, h) = shot.size;
    cx.update_window(handle, |_, window, cx| {
        window.resize(size(px(w), px(h)));
        window.bounds_changed(cx);
        // A real app window is key; focus rings and carets draw as they would there.
        window.activate_window();
    })
    .expect("the window is open");
    cx.run_until_parked();
    // ## m3b-t29-shots (fix round 1, I4): the accessibility declarations made from here to the
    // frame, which an M3b state's check reads (a menu's highlighted row is its active
    // descendant). A cached view declares when it renders, not on every frame.
    hacivat_ui::popover::a11y_record();
    cx.update_window(handle, |_, window, cx| shot.surface.open(window, cx))
        .expect("the window is open");
    cx.run_until_parked();
    if shot.surface.needs_connections() && !stepped {
        // Profile writes and the keyring probe run on the core runtime (P22); let them land.
        std::thread::sleep(std::time::Duration::from_millis(30));
        cx.run_until_parked();
    }
    // ## m2b-t24-shots: wait for the surface's own core work (a condition, bounded).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !cx
        .update_window(handle, |_, window, cx| ready(shot.surface, window, cx))
        .expect("the window is open")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "{}: timed out waiting for the surface",
            shot.surface.name()
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        cx.run_until_parked();
    }
    // ## m3a-t35-shots: each step, then its condition (bounded, polled; never a fixed sleep).
    // ## m3b-t29-shots: an M3b state's steps too.
    // ## m3c-t43-shots: an M3c state's steps too.
    let steps = match (m3a_state, m3b_state, m3c_state) {
        (Some(s), _, _) => s.steps(),
        (_, Some(s), _) => s.steps(),
        (_, _, Some(s)) => s.steps(),
        // ## m4-t32-shots: an M4 state's steps after its fixtures.
        _ => match shot.surface {
            Surface::M4(s) => s.steps(),
            // ## m4b-t11-editor
            Surface::M4b(s) => s.steps(),
            _ => Vec::new(),
        },
    };
    for (at, step) in steps.into_iter().enumerate() {
        cx.update_window(handle, |_, window, cx| (step.act)(window, cx))
            .expect("the window is open");
        cx.run_until_parked();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.update(|cx| cx.refresh_windows());
            cx.run_until_parked();
            if cx
                .update_window(handle, |_, window, cx| (step.ready)(window, cx))
                .expect("the window is open")
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{}: timed out waiting for step {at}",
                shot.surface.name()
            );
            // The UI's timers run on the simulated clock (the preview's 50 ms, the toolbar's
            // and the marks' 1 Hz clocks): it moves only while a step waits. Core work runs on
            // real threads: the poll yields to it.
            cx.advance_clock(std::time::Duration::from_millis(25));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    // ## m3c-t43b-fix (review I2): no capsule settle any more; the capsule's own fit (ccedf93)
    // no longer depends on the frames before, so the capture is the frame the steps reached.
    cx.update(|cx| cx.refresh_windows());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        after_paint(shot.surface, window, cx)
    })
    .expect("the window is open");
    cx.run_until_parked();
    // ## m3c-t43-shots: the icons of the last frame (an M3c state's check counts the gutter's ▶
    // and chevrons, which draw no label).
    if m3c_state.is_some() {
        cx.update_window(handle, |_, window, cx| m3c::record_icons(window, cx))
            .expect("the window is open");
    }
    // ## m4b-t21-shots (named exception, R-C0-6: the CI flakes `narrow-empty-dark-en-XA` and
    // `marks-awaiting-dark-tr-XT`): the frame the checks read and the PNG shows is the last
    // full frame, with nothing run after it. `cx.update` does not flush, so the old
    // `refresh_windows` + `run_until_parked` drew its full frame inside the first task that
    // ran, and a later task that notified any other view (a late status or session update
    // under load) drew one more frame that reused the cached editor (`a11y::embed`), which
    // records none of its labels: "no visible label from #editor-placeholder". Now the app
    // settles first, and `update_window` draws the full frame as it returns.
    cx.run_until_parked();
    // The guard: a redraw that only a later settle would run (as a late update does), on every
    // shot. Any order that settles between the full frame and the read loses the cached views'
    // labels on every shot that has one, not once in a hundred runs.
    late_redraw(&mut cx, handle);
    cx.update_window(handle, |_, window, _| window.refresh())
        .expect("the window is open");
    if m3c_state.is_some() {
        m3c::take_icons();
    }
    let decls = hacivat_ui::popover::a11y_take();

    let scale = cx
        .update_window(handle, |_, window, _| window.scale_factor())
        .expect("the window is open");
    let name = shot.file_name(scale);
    let frame = cx.update(|cx| cx.global::<TextRegistry>().frame.clone());
    let mut failures = check_labels(&name, shot.locale, &frame);
    if frame.is_empty() {
        failures.push(format!("{name}: the frame drew no label"));
    }
    failures.extend(shot.surface.missing_from(&name, &frame));
    // ## m3a-f5-demo (follow-up): (m) an open menu shows whole rows only, none over the status
    // bar or past the window's bottom.
    failures.extend(
        cx.update(|cx| hacivat_ui::menu::MenuLayout::violations(cx))
            .into_iter()
            .map(|v| format!("{name}: (m) {v}")),
    );
    // ## m3a-t35-shots: the state check (the app's models agree with what the shot claims),
    // and the grid's own checks (headers are data; no grid text clipped).
    if let Some(state) = m3a_state {
        failures.extend(
            cx.update_window(handle, |_, window, cx| state.check(window, cx, &frame))
                .expect("the window is open")
                .map(|v| format!("{name}: {v}")),
        );
    }
    // ## m3b-t29-shots: every M3b state's model check.
    if let Some(state) = m3b_state {
        failures.extend(
            cx.update_window(handle, |_, window, cx| {
                state.check(window, cx, &frame, &decls)
            })
            .expect("the window is open")
            .map(|v| format!("{name}: {v}")),
        );
    }
    // ## m3c-t43-shots: every M3c state's model check.
    if let Some(state) = m3c_state {
        failures.extend(
            cx.update_window(handle, |_, window, cx| {
                state.check(window, cx, &frame, &decls)
            })
            .expect("the window is open")
            .map(|v| format!("{name}: {v}")),
        );
    }
    failures.extend(check_grid(&name, shot.locale, &frame));
    failures.extend(shot.surface.trust_missing(&name, &frame));
    // ## m4b-t22-pre (T21 r1)
    failures.extend(check_notice_title(&name, &frame));
    // ## m4b-t21-shots fix round 1 (I2): the chooser announces each row's environment.
    if let Surface::M4b(state) = shot.surface
        && state.is_chooser()
    {
        failures.extend(
            cx.update(|cx| chooser_rows_named(cx, &decls))
                .into_iter()
                .map(|v| format!("{name}: (t) {v}")),
        );
    }
    if std::env::var_os("SHOTS_DUMP").is_some() {
        for r in &frame {
            println!(
                "  {:?} {:?} {} {:.1}px",
                r.path,
                strip(&r.text),
                r.visible,
                r.size_px
            );
        }
    }
    // ## m2b-t24-shots (follow-up)
    failures.extend(shot.surface.cut_short(&name, shot.locale, &frame));
    let badge = cx
        .update_window(handle, |_, window, cx| {
            let cp_fill = Theme::get(cx).tokens.colors.cp_fill.to_hsla();
            let capsule = cx.global::<CapsuleProbe>().bounds;
            catch_unwind(AssertUnwindSafe(|| {
                assert_one_filled_badge_in(
                    window,
                    cp_fill,
                    shot.surface.shows_production(),
                    capsule,
                )
            }))
        })
        .expect("the window is open");
    if let Err(panic) = badge {
        let msg = panic
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| "One Filled Badge rule".to_string());
        failures.push(format!("{name}: (d) {msg}"));
    }
    // ## m4-m3carry (U-6 c): the whole capsule probe, its trust flag too.
    let (bar, capsule_probe) =
        cx.update(|cx| (*cx.global::<TitleBarProbe>(), *cx.global::<CapsuleProbe>()));
    let capsule = capsule_probe.bounds;
    // ## m4-m3carry (fix round 1, m1): how many shots (e) checked with real trust data.
    note_trust_coverage(bar, capsule_probe);
    failures.extend(
        check_title_bar(w, bar, capsule)
            .into_iter()
            .chain(check_trust_room(bar, capsule_probe))
            .map(|v| format!("{name}: (e) {v}")),
    );

    let image = cx.capture_screenshot(handle).expect("the frame renders");
    let want = ((w * scale).round() as u32, (h * scale).round() as u32);
    if image.dimensions() != want {
        failures.push(format!(
            "{name}: image is {:?}, expected {want:?}",
            image.dimensions()
        ));
    }
    image.save(dir.join(&name)).expect("write the PNG");
    println!("shot {name}: {} labels", frame.len());
    // ## m3c-t43-shots (fix round 1, I1): the Transposed name column's word in this locale.
    let header_word = m3c_state.map(|_| cx.update(|cx| m3c::header_word(cx)));
    support::stop_core_runtime(&mut cx);
    // ## m3c-t43-shots: a product defect this task found in a merged M3c surface and listed for
    // its owner (`m3c::KNOWN_FINDINGS`) is printed, not failed; every other violation fails
    // (fix round 1: only its one kind on its one label; never a trust, missing-surface or
    // state-check line).
    if let Some(state) = m3c_state {
        failures.retain(|v| {
            match m3c::known_finding(state, shot.locale, shot.size, v, header_word.as_deref()) {
                Some(f) => {
                    println!("known finding ({}): {v}", f.owner);
                    false
                }
                None => true,
            }
        });
    }
    failures
}

// ## m4b-t22-pre (T21 r1)
/// (s) A script tab's first notice shows its title whole: visible, not clipped, and not cut or
/// hidden at the fold of the scroll area the notices sit in while a Run-as-script strip shows
/// (check (b) lets a fold through as scrolling). The titles record that at prepaint
/// (`InlineNotice::title_id`); later notices may scroll.
fn check_notice_title(name: &str, frame: &[RenderedText]) -> Option<String> {
    let id = format!("#{}", hacivat_app::scripts::disk_bars::NOTICE_TITLE_ID);
    let first = frame.iter().find(|r| r.path.ends_with(id.as_str()))?;
    (!first.visible || first.clipped || first.folded).then(|| {
        format!(
            "{name}: (s) the first notice's title is not shown whole: {:?} (visible {}, \
             clipped {}, folded {})",
            strip(&first.text),
            first.visible,
            first.clipped,
            first.folded
        )
    })
}

// ## m2b-t24-shots
// ## m3a-t01b-seams: `stop_core_runtime` and `TEARDOWN_BUDGET` moved to `support/mod.rs` (P20).

/// The main check: renders the shots named in `names` (file names), then fails with every
/// violation found. Only a full run (`all`) removes the PNGs already in the directory.
fn render_all_surfaces(names: &[String], all: bool) {
    match current_headless_renderer() {
        Ok(Some(_)) => {}
        Ok(None) => {
            println!("shots: no headless renderer (R8)");
            return;
        }
        Err(err) => panic!("shots: the headless renderer failed to start: {err:#}"),
    }
    // GPUI reads the font-smoothing preference once, on the first glyph (R11).
    apply_text_rendering(TextRendering::Crisp);
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("create the shots directory");
    // Stale PNGs never pass for fresh ones; a filtered run keeps the shots it does not render.
    if all {
        for entry in std::fs::read_dir(&dir)
            .expect("read the shots directory")
            .flatten()
        {
            if entry.path().extension().is_some_and(|e| e == "png") {
                std::fs::remove_file(entry.path()).expect("remove a stale shot");
            }
        }
    }
    let mut failures = Vec::new();
    let mut rendered = 0;
    let run_started = std::time::Instant::now();
    let mut slowest = std::time::Duration::ZERO;
    // ## m3c-t43-shots (fix round 1, m1): the M3c shots rendered, per state.
    let mut m3c_rendered: std::collections::BTreeMap<&'static str, usize> = Default::default();
    // ## m3c-threads: the live threads after the first shot and after the last.
    let mut first_threads: Option<Option<usize>> = None;
    let mut threads = None;
    for shot in shots() {
        if !names.contains(&shot.file_name(2.0)) {
            continue;
        }
        // ## m3a-t35-shots: the per-shot and total harness time (the plan's Step 4 record).
        let started = std::time::Instant::now();
        failures.extend(render(shot, &dir));
        // ## m4b-t21-shots (R-M4b-44): the shot's root is gone with its app (fix round 1, m10:
        // its stable path); reported once, then removed.
        // Fix round 2 (m-e): a root another run holds now (its lock) is not ours to judge.
        let root = shot_root_path(&shot.file_name(2.0));
        if root.exists() && !ShotRoot::lock_of(&root).exists() {
            failures.push(format!(
                "{}: (r) left the temp root {} behind",
                shot.file_name(2.0),
                root.display()
            ));
            let _ = std::fs::remove_dir_all(&root);
        }
        let took = started.elapsed();
        slowest = slowest.max(took);
        println!("shot time {}: {} ms", shot.file_name(2.0), took.as_millis());
        rendered += 1;
        // ## m3c-threads: the live threads stay near the first shot's count; a run that keeps
        // growing stops here, before the OS refuses a thread (whose panics name a bystander).
        threads = live_threads();
        let first = *first_threads.get_or_insert(threads);
        if let (Some(first), Some(now)) = (first, threads)
            && let Some(leak) = thread_leak(first, now, rendered)
        {
            panic!("{leak}");
        }
        // ## m3c-t43-shots (fix round 1, m1)
        if let Surface::M3c(state) = shot.surface {
            *m3c_rendered.entry(state.name()).or_insert(0) += 1;
        }
    }
    // ## m3c-t43-shots (fix round 1, m1): a known finding of a state rendered whole (16 shots)
    // that matched no line is stale: it fails the run (it would hide the next regression).
    failures.extend(m3c::stale_findings(&m3c_rendered));
    let total = run_started.elapsed();
    println!(
        "shots: {rendered} rendered into {} in {:.1} s (mean {} ms, slowest {} ms per shot)",
        dir.display(),
        total.as_secs_f64(),
        total.as_millis() / rendered.max(1) as u128,
        slowest.as_millis()
    );
    // ## m4-m3carry (fix round 1, m1)
    let (with_trust, collapsed) = TRUST_COVERAGE.with(|c| c.get());
    println!(
        "shots: (e) trust check on {with_trust} shots with a capsule trust word, {collapsed} with \
         the palette field collapsed"
    );
    // ## m3c-threads
    let shown = |n: Option<usize>| n.map_or_else(|| "?".to_string(), |n| n.to_string());
    println!(
        "shots: {} live threads after the first shot, {} after the last",
        shown(first_threads.flatten()),
        shown(threads),
    );
    assert!(rendered > 0, "no shot selected");
    assert!(
        failures.is_empty(),
        "{} violations:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ## m3c-threads
/// How many more live threads than after the first shot the run may have at any later shot:
/// room for the threads a process starts once, on the first shot that needs them, and for a
/// shot's own threads that are still ending. A shot that leaks threads crosses it within a
/// few dozen shots (five a shot, the Linux platform's leak, by the eighth).
const THREAD_SLACK: usize = 32;

/// The violation when `now` live threads after `rendered` shots exceed the `first` shot's
/// count by more than [`THREAD_SLACK`].
fn thread_leak(first: usize, now: usize, rendered: usize) -> Option<String> {
    (now > first + THREAD_SLACK).then(|| {
        format!(
            "shots: {now} live threads after {rendered} shots, {first} after the first: a shot \
             leaves threads running (more than {THREAD_SLACK} more)"
        )
    })
}

/// This process's live OS threads: `Threads:` in `/proc/self/status` on Linux, `ps -M` on
/// macOS (no `unsafe`); `None` elsewhere or when the OS does not say.
fn live_threads() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        status
            .lines()
            .find_map(|l| l.strip_prefix("Threads:"))?
            .trim()
            .parse()
            .ok()
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("ps")
            .args(["-M", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        out.status.success().then_some(())?;
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .count()
            .checked_sub(1)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

// ## m3c-threads
fn a_run_whose_threads_keep_growing_fails() {
    assert_eq!(thread_leak(40, 40, 2), None);
    assert_eq!(thread_leak(40, 40 + THREAD_SLACK, 500), None);
    // Fewer threads than after the first shot is no leak.
    assert_eq!(thread_leak(40, 12, 9), None);
    let leak = thread_leak(40, 41 + THREAD_SLACK, 8).expect("a leak");
    assert!(
        leak.contains("73 live threads after 8 shots") && leak.contains("40 after the first"),
        "{leak}"
    );
    // The live count, where the OS says it, counts this thread at least.
    assert!(live_threads().is_none_or(|n| n >= 1));
}

// ## m3c-threads
fn every_shot_shares_one_platform_text_system() {
    assert!(Arc::ptr_eq(&shot_text_system(), &shot_text_system()));
}

// ## m4b-t21-shots
/// Renders the shots named in `names` (file names, each `times` times) into a temp folder of
/// its own, with every shot's temp root under `tmp`; `None` without a headless renderer.
/// Returns each shot's violations and the temp roots it left behind.
fn render_for_check(names: &[&str], times: usize, tmp: &Path) -> Option<Vec<String>> {
    match current_headless_renderer() {
        Ok(Some(_)) => {}
        Ok(None) => {
            println!("shots: no headless renderer (R8)");
            return None;
        }
        Err(err) => panic!("shots: the headless renderer failed to start: {err:#}"),
    }
    apply_text_rendering(TextRendering::Crisp);
    let out = tempfile::tempdir().expect("an output folder");
    SHOT_TMP.with(|t| *t.borrow_mut() = Some(tmp.to_path_buf()));
    let mut found = Vec::new();
    for shot in shots() {
        let name = shot.file_name(2.0);
        if !names.contains(&name.as_str()) {
            continue;
        }
        for _ in 0..times {
            found.extend(render(shot, out.path()));
            found.extend(
                left_in(tmp)
                    .into_iter()
                    .map(|left| format!("{name}: (r) left the temp root {left} behind")),
            );
        }
    }
    SHOT_TMP.with(|t| *t.borrow_mut() = None);
    Some(found)
}

// ## m4b-t21-shots (R-M4b-44)
/// A render of the M4b states (each once: dark, English, 1280x800) leaves no temp root behind:
/// a shot's root outlives its app, whose teardown saves the workspace into it once more.
fn shots_render_leaves_no_temp_dir() {
    let names: Vec<String> = m4b::M4B_STATES
        .iter()
        .map(|s| format!("{}-dark-en-1280x800@2x.png", s.name()))
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let tmp = tempfile::tempdir().expect("the check's temp folder");
    let Some(found) = render_for_check(&names, 1, tmp.path()) else {
        return;
    };
    let left: Vec<&String> = found.iter().filter(|v| v.contains("(r)")).collect();
    assert!(left.is_empty(), "temp roots left behind:\n{left:#?}");
}

// ## m4b-t21-shots (R-C0-6)
/// The repeat guard of the CI flakes `narrow-empty-dark-en-XA-1280x800` (the editor's
/// placeholder) and `marks-awaiting-dark-tr-XT-1280x800` (the gutter's marks): both draw
/// their marker inside the cached editor, so a frame read after a redraw that reused it had
/// lost the marker. Each renders ten times, every time with the late redraw `render` queues,
/// and every frame shows its marker.
fn cached_view_states_render_whole_every_time() {
    let tmp = tempfile::tempdir().expect("the check's temp folder");
    let Some(found) = render_for_check(
        &[
            "narrow-empty-dark-en-XA-1280x800@2x.png",
            "marks-awaiting-dark-tr-XT-1280x800@2x.png",
        ],
        10,
        tmp.path(),
    ) else {
        return;
    };
    assert!(
        found.is_empty(),
        "{} violations:\n{}",
        found.len(),
        found.join("\n")
    );
}
// ## m4b-t21-shots

fn the_matrix_covers_every_surface_locale_and_theme() {
    let all = shots();
    // M1: 8 surfaces x 8 + 4 VF; M2a: 26 surfaces x 8 + the 4 dialog surfaces again at 1024x700;
    // M2b T22: 10 surfaces x 8 at 1280x800 and again at 960x600; M2b T24: 34 surfaces likewise.
    assert_eq!(
        all.len(),
        8 * 4 * 2
            + 4
            + 8
            + 26 * 8
            + 4 * 8
            + 10 * 8 * 2
            + 34 * 8 * 2
            + m3a::M3A.len() * 8 * 2
            // ## m3b-t29-shots: every M3b state (Tasks 21–27 and 29).
            + m3b::all().len() * 8 * 2
            // ## m3c-t43-shots: every M3c state (Tasks 31–38).
            + m3c::all().len() * 8 * 2
            // ## m4-t32-shots: every M4 state.
            + m4::M4_STATES.len() * 8 * 2
            // ## m4b-t11-editor: every M4b state.
            + m4b::M4B_STATES.len() * 8 * 2
    );
    // ## m3a-t35-shots: the 1,012 M1/M2 shots first, unchanged; then 68 M3a states x 16.
    // ## m3a-f5-demo (review m-5): plus the app menu at 960x600 (8).
    // ## m3b-t29-shots: then the M3b states x 16: Task 22's 7, Task 25's 11, Task 21's 20
    // (16 confirmations with Task 29's staging Execute all, 4 commit prompts), Task 23's 9,
    // Task 24's 12, Task 27's 7, Task 26's 1, Task 29's 4.
    assert_eq!(m3b::t21().len(), 20);
    assert_eq!(m3b::t24().len(), 12);
    assert_eq!(m3b::all().len(), 7 + 11 + 20 + 9 + 12 + 7 + 1 + 4);
    // ## m3c-t43-shots: then the M3c states x 16: Task 31's 5, Task 32's 7, Task 33's 7, Task
    // 34's 10, Tasks 35/36's 10, Task 37's 13, Task 38's 15 (part B adds Tasks 39, 41 and 42's).
    // ## m3c-t43b: part B adds Task 39's 9, Task 41's 11, Task 42's 7 and the 6 states part A
    // left.
    // ## m3c-t43b-fix (review I1): Task 41's 12 with `narrow-reduced`.
    assert_eq!(
        m3c::all().len(),
        5 + 7 + 7 + 10 + 10 + 13 + 15 + 9 + 12 + 7 + 6
    );
    // ## m4-t32-shots: and the M4 states x 16.
    assert_eq!(
        all.len(),
        1020 + 68 * 16
            + 71 * 16
            + 101 * 16
            + m4::M4_STATES.len() * 16
            // ## m4b-t11-editor
            + m4b::M4B_STATES.len() * 16
    );
    let m3b_from = 1020 + 68 * 16;
    let order = [
        m3b::T22.to_vec(),
        m3b::T25.to_vec(),
        m3b::t21(),
        m3b::T23.to_vec(),
        m3b::t24(),
        m3b::T27.to_vec(),
        m3b::T26.to_vec(),
        m3b::T29.to_vec(),
    ];
    let mut at = m3b_from;
    for group in order {
        for state in group {
            assert!(
                all[at..at + 16]
                    .iter()
                    .all(|s| s.surface == Surface::M3b(state)),
                "{state:?} at {at}"
            );
            at += 16;
        }
    }
    // ## m3c-t43-shots: the M3c states after the M3b ones, in task order.
    for state in m3c::all() {
        assert!(
            all[at..at + 16]
                .iter()
                .all(|s| s.surface == Surface::M3c(state)),
            "{state:?} at {at}"
        );
        at += 16;
    }
    // ## m4-t32-shots: the M4 states last.
    for state in m4::M4_STATES {
        assert!(
            all[at..at + 16]
                .iter()
                .all(|s| s.surface == Surface::M4(state)),
            "{state:?} at {at}"
        );
        at += 16;
    }
    // ## m4b-t11-editor: the M4b states after them.
    for state in m4b::M4B_STATES {
        assert!(
            all[at..at + 16]
                .iter()
                .all(|s| s.surface == Surface::M4b(state)),
            "{state:?} at {at}"
        );
        at += 16;
    }
    assert_eq!(at, all.len());
    assert!(
        all[..1020]
            .iter()
            .all(|s| !matches!(s.surface, Surface::M3a(_) | Surface::M3b(_)))
    );
    for state in m3a::M3A {
        let surface = Surface::M3a(state);
        assert_eq!(
            all.iter().filter(|s| s.surface == surface).count(),
            16,
            "{state:?}"
        );
    }
    let names: Vec<String> = all.iter().map(|s| s.file_name(2.0)).collect();
    // ## m2b-t24-shots: every T24 surface at both sizes, in every locale and theme.
    for surface in &SURFACES[FIRST_T24..] {
        assert!(surface.is_m2b() && !surface.is_live(), "{surface:?}");
        assert_eq!(
            all.iter().filter(|s| s.surface == *surface).count(),
            16,
            "{surface:?}"
        );
    }
    assert!(!Surface::LiveSshRw.is_m2b() && !Surface::ConnDialogNew.is_m2b());
    assert!(names.contains(&"kbd-prompt-light-tr-960x600@2x.png".to_string()));
    assert!(names.contains(&"vault-reset-dark-tr-XT-960x600@2x.png".to_string()));
    assert!(names.contains(&"live-ssh-ro-dark-tr-XT-960x600@2x.png".to_string()));
    assert!(names.contains(&"shell-dark-en-1280x800@2x.png".to_string()));
    assert!(names.contains(&"shell-narrow-light-tr-XT-1024x700@2x.png".to_string()));
    assert!(names.contains(&"shell-demo-production-light-tr-1440x900@2x.png".to_string()));
    assert!(names.contains(&"menu-app-dark-en-XA-1280x800@2x.png".to_string()));
    assert!(names.contains(&"menu-app-light-tr-XT-960x600@2x.png".to_string()));
    assert!(names.contains(&"conn-dialog-new-light-tr-XT-1024x700@2x.png".to_string()));
    assert!(names.contains(&"capsule-production-ro-dark-tr-1280x800@2x.png".to_string()));
    assert!(names.contains(&"shell-minimum-dark-en-960x600@2x.png".to_string()));
    // ## m3a-t35-shots
    assert!(names.contains(&"strip-closed-tx-ended-light-tr-XT-960x600@2x.png".to_string()));
    assert!(names.contains(&"marks-run-dark-en-1280x800@2x.png".to_string()));
    assert!(!names.contains(&"conn-error-auth-dark-en-1024x700@2x.png".to_string()));
    let unique: std::collections::BTreeSet<&String> = names.iter().collect();
    assert_eq!(unique.len(), names.len());
}

fn pseudo_labels_must_be_bracketed_and_unclipped_and_at_least_11_px() {
    let label = |text: &str, source, size_px, clipped| RenderedText {
        text: text.to_string().into(),
        source,
        size_px,
        painted_px: 10.0,
        clipped,
        truncated: false,
        path: "x.rs:1:1".into(),
        visible: true,
        outside_card: false,
        folded: false,
    };
    let frame = [
        label("[Ŝéåŕçĥ]", TextSource::Fluent, 13.0, false),
        label("\u{2068}[Ŝéåŕçĥ]\u{2069}", TextSource::Fluent, 13.0, false),
        label("EN", TextSource::Data, 11.5, false),
        label("Search", TextSource::Fluent, 13.0, false),
        label("[ok]", TextSource::Fluent, 10.5, true),
    ];
    assert!(check_labels("s", "en", &frame[..3]).is_empty());
    let found = check_labels("s", "en-XA", &frame);
    assert_eq!(found.len(), 3, "{found:#?}");
    assert!(found[0].contains("(a)") && found[0].contains("Search"));
    assert!(found[1].contains("(b)"));
    assert!(found[2].contains("(c) 10.5 px"));
    // Outside the pseudo locales an unbracketed Fluent label is fine.
    assert!(check_labels("s", "tr", &frame[3..4]).is_empty());
    // ## m2b-t24-shots (fix round 1, M2): an empty Fluent label is a violation again (a field
    // with no placeholder draws none). A message split around a literal passes as a whole only
    // when its pieces are adjacent, a data piece between each pair, and close the bracket.
    let at = |text: &str, path: &str| RenderedText {
        path: path.to_string().into(),
        ..label(text, TextSource::Fluent, 13.0, false)
    };
    let word = |text: &str| RenderedText {
        path: "v.rs:9:20#word".into(),
        ..label(text, TextSource::Data, 11.5, false)
    };
    assert_eq!(check_labels("s", "en-XA", &[at("", "x.rs:1:1")]).len(), 1);
    let split = [
        at("[Ŧẏƥḗ ", "v.rs:9:9"),
        word("reset"),
        at(" ŧö çöƞƒıřḿ]", "v.rs:9:9"),
    ];
    assert!(check_labels("s", "en-XA", &split).is_empty());
    // Two literals in one message.
    let two = [
        at("[Ŧẏƥḗ ", "v.rs:9:9"),
        word("a"),
        at(" öř ", "v.rs:9:9"),
        word("b"),
        at(" ŧö çöƞƒıřḿ]", "v.rs:9:9"),
    ];
    assert!(check_labels("s", "en-XA", &two).is_empty());
    // Not adjacent around a literal, not closed, or not one site: each piece is checked.
    let no_literal = [at("[Ŧẏƥḗ ", "v.rs:9:9"), at(" ŧö çöƞƒıřḿ]", "v.rs:9:9")];
    assert_eq!(check_labels("s", "en-XA", &no_literal).len(), 2);
    let apart = [
        at("[Ŧẏƥḗ ", "v.rs:9:9"),
        word("reset"),
        label("[Ŝéåŕçĥ]", TextSource::Fluent, 13.0, false),
        at(" ŧö çöƞƒıřḿ]", "v.rs:9:9"),
    ];
    assert_eq!(check_labels("s", "en-XA", &apart).len(), 2);
    let open = [
        at("[Ŧẏƥḗ ", "v.rs:9:9"),
        word("reset"),
        at(" ŧö çöƞƒıřḿ", "v.rs:9:9"),
    ];
    assert_eq!(check_labels("s", "en-XA", &open).len(), 2);
    let elsewhere = [
        at("[Ŧẏƥḗ ", "v.rs:9:9"),
        word("reset"),
        at(" ŧö çöƞƒıřḿ]", "w.rs:1:1"),
    ];
    assert_eq!(check_labels("s", "en-XA", &elsewhere).len(), 2);
    // ## m3b-ci (controller fix): a label drawn as access-key runs (Linux, Windows) passes (a)
    // whole: three runs from `button.rs` with one `#id`, the middle one the key letter, that
    // join into a bracketed text. A row without a key before it does not take its runs.
    let key = |text: &str| {
        at(
            text,
            "crates/ui/hacivat-ui/src/button.rs:272:21#menu-item-label",
        )
    };
    let close = [key("["), key("Ƈ"), key("ŀǿǿşḗ]")];
    assert!(check_labels("s", "en-XA", &close).is_empty());
    let after_a_plain_row = [key("[Ŏƥḗƞ]"), key("["), key("Ƈ"), key("ŀǿǿşḗ]")];
    assert!(check_labels("s", "en-XA", &after_a_plain_row).is_empty());
    // Runs that join into an unbracketed text, runs of two controls (two ids), runs whose
    // middle is not one letter, and runs from elsewhere than `button.rs` stay checked one by one.
    let unclosed = [key("["), key("Ƈ"), key("ŀǿǿşḗ")];
    assert_eq!(check_labels("s", "en-XA", &unclosed).len(), 3);
    let two_ids = [
        key("["),
        at(
            "Ƈ",
            "crates/ui/hacivat-ui/src/button.rs:272:21#button-label-dialog-button-1",
        ),
        key("ŀǿǿşḗ]"),
    ];
    assert_eq!(check_labels("s", "en-XA", &two_ids).len(), 3);
    let wide_middle = [key("["), key("Ƈŀ"), key("ǿǿşḗ]")];
    assert_eq!(check_labels("s", "en-XA", &wide_middle).len(), 3);
    let not_a_key_label = [
        at("[", "w.rs:1:1#x"),
        at("Ƈ", "w.rs:1:1#x"),
        at("ŀǿǿşḗ]", "w.rs:1:1#x"),
    ];
    assert_eq!(check_labels("s", "en-XA", &not_a_key_label).len(), 3);
    // A clipped run is still a clipped label.
    let clipped_key = [
        key("["),
        RenderedText {
            clipped: true,
            ..key("Ƈ")
        },
        key("ŀǿǿşḗ]"),
    ];
    let found = check_labels("s", "en-XA", &clipped_key);
    assert!(found.len() == 1 && found[0].contains("(b)"), "{found:?}");
    // (f) A label that runs past its dialog card, in any locale.
    let past = RenderedText {
        outside_card: true,
        folded: false,
        ..label("Applies to every hop", TextSource::Fluent, 12.0, false)
    };
    let found = check_labels("s", "en", &[past]);
    assert!(found.len() == 1 && found[0].contains("(f)"), "{found:?}");
    // ## m3a-t35-shots: a label its container cut away whole (a status cell that gave way to
    // nothing, the Overflow Rule) draws nothing: not a clipped label. A visible one is.
    let hidden = RenderedText {
        visible: false,
        ..label("PostgreSQL", TextSource::Fluent, 12.0, true)
    };
    assert!(check_labels("s", "en", &[hidden]).is_empty());
    let shown = label("Ln 2, Col 38", TextSource::Fluent, 12.0, true);
    assert_eq!(check_labels("s", "en", &[shown]).len(), 1);
}

/// The ignored test: it renders, so it runs only on request.
const RENDER_TEST: &str = "render_all_surfaces";

/// The command line, read the way libtest reads it.
#[derive(Debug, Default, PartialEq)]
struct Cli {
    /// Positional arguments: test names, or (for the render) shot file names.
    filters: Vec<String>,
    /// `--skip`: tests whose name contains one of these do not run.
    skips: Vec<String>,
    exact: bool,
    list: bool,
    /// Only the ignored tests (`--ignored`).
    ignored_only: bool,
    /// Ignored tests too (`--include-ignored`).
    include_ignored: bool,
    terse: bool,
    // ## m3a-t35-shots
    /// `--shard k/n`: the k-th of n interleaved slices of the selected shots (1-based), so n
    /// processes render the matrix side by side.
    shard: Option<(usize, usize)>,
    /// Fix round 1 (M6): a `--shard` value that is not `k/n` (`1 <= k <= n`): the binary
    /// exits 2 instead of rendering everything.
    bad_shard: Option<String>,
}

impl Cli {
    fn parse(args: impl IntoIterator<Item = String>) -> Cli {
        let mut cli = Cli::default();
        let mut it = args.into_iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--exact" => cli.exact = true,
                // ## m3a-t35-shots
                "--shard" => cli.set_shard(it.next().unwrap_or_default()),
                s if s.starts_with("--shard=") => cli.set_shard(s["--shard=".len()..].to_string()),
                "--list" => cli.list = true,
                "--ignored" => cli.ignored_only = true,
                "--include-ignored" => cli.include_ignored = true,
                "--skip" => cli.skips.extend(it.next()),
                "--format" => cli.terse |= it.next().as_deref() == Some("terse"),
                // Flags with a value that change nothing here.
                "--test-threads" | "--color" | "--logfile" | "-Z" => {
                    it.next();
                }
                s if s.starts_with("--format=") => cli.terse |= s == "--format=terse",
                s if s.starts_with('-') => {}
                _ => cli.filters.push(a),
            }
        }
        cli
    }

    /// Whether `pattern` selects `name` (substring, or equality with `--exact`).
    fn matches(&self, pattern: &str, name: &str) -> bool {
        if self.exact {
            name == pattern || name.strip_suffix(".png") == Some(pattern)
        } else {
            name.contains(pattern)
        }
    }

    fn skipped(&self, name: &str) -> bool {
        self.skips.iter().any(|s| name.contains(s.as_str()))
    }

    /// Whether test `name` is selected by the filters (and not skipped).
    fn selects_test(&self, name: &str) -> bool {
        !self.skipped(name)
            && (self.filters.is_empty() || self.filters.iter().any(|f| self.matches(f, name)))
    }

    /// The shots the render covers: all of them when the test is selected by its own name or
    /// by no filter, else the shots a filter names. `None` when the render is not selected.
    fn shots(&self, all: &[String]) -> Option<Vec<String>> {
        if self.skipped(RENDER_TEST) {
            return None;
        }
        let picked: Vec<String> = if self.selects_test(RENDER_TEST) {
            all.to_vec()
        } else {
            all.iter()
                .filter(|s| self.filters.iter().any(|f| self.matches(f, s)))
                .cloned()
                .collect()
        };
        // ## m3a-t35-shots: one shard of what is selected.
        let picked: Vec<String> = match self.shard {
            Some((k, n)) => picked
                .into_iter()
                .enumerate()
                .filter(|(i, _)| i % n == k - 1)
                .map(|(_, s)| s)
                .collect(),
            None => picked,
        };
        (!picked.is_empty()).then_some(picked)
    }

    // ## m3a-t35-shots (fix round 1, M6)
    fn set_shard(&mut self, value: String) {
        match parse_shard(&value) {
            Some(shard) => self.shard = Some(shard),
            None => self.bad_shard = Some(value),
        }
    }

    fn runs_ignored(&self) -> bool {
        self.ignored_only || self.include_ignored
    }
}

// ## m3a-t35-shots
/// `k/n` with `1 <= k <= n`.
fn parse_shard(v: &str) -> Option<(usize, usize)> {
    let (k, n) = v.split_once('/')?;
    let (k, n) = (k.parse().ok()?, n.parse().ok()?);
    (1..=n).contains(&k).then_some((k, n))
}

/// libtest's contract without its threads. `--list` lists every test (`--ignored`: only the
/// ignored render). The self-checks run unless `--ignored`; the render runs with `--ignored` or
/// `--include-ignored`. A filter naming the render (or, without matching a test, shots) selects
/// what it renders. Asked for the render by its exact name without `--ignored` (how nextest runs
/// one test), the binary fails instead of passing without rendering.
fn main() {
    let cli = Cli::parse(std::env::args().skip(1));
    // ## m3a-t35-shots (fix round 1, M6)
    if let Some(bad) = &cli.bad_shard {
        eprintln!("--shard {bad:?}: expected k/n with 1 <= k <= n");
        std::process::exit(2);
    }
    let checks: [(&str, fn()); 16] = [
        // ## m4b-t22-pre (T21 r4)
        (
            "a_dead_runs_shot_lock_is_taken_over_at_once",
            a_dead_runs_shot_lock_is_taken_over_at_once,
        ),
        // ## m4b-t21-shots (R-M4b-44, R-C0-6)
        (
            "shots_render_leaves_no_temp_dir",
            shots_render_leaves_no_temp_dir,
        ),
        (
            "cached_view_states_render_whole_every_time",
            cached_view_states_render_whole_every_time,
        ),
        // ## m3c-threads
        (
            "a_run_whose_threads_keep_growing_fails",
            a_run_whose_threads_keep_growing_fails,
        ),
        // ## m3c-threads
        (
            "every_shot_shares_one_platform_text_system",
            every_shot_shares_one_platform_text_system,
        ),
        // ## m3c-t43-shots
        (
            "every_m3c_state_has_its_own_marker",
            every_m3c_state_has_its_own_marker,
        ),
        // ## m3c-t43-shots (fix round 1, m1)
        (
            "known_findings_are_empty_at_the_t45_gate",
            known_findings_are_empty_at_the_t45_gate,
        ),
        // ## m3b-t29-shots
        (
            "every_m3b_state_has_its_own_marker",
            every_m3b_state_has_its_own_marker,
        ),
        // ## m3a-t35-shots (fix round 1, I2)
        ("trust_signals_must_be_drawn", trust_signals_must_be_drawn),
        // ## m3a-t35-shots
        (
            "grid_text_is_data_and_never_clipped",
            grid_text_is_data_and_never_clipped,
        ),
        (
            "an_m3a_grid_state_without_its_grid_fails",
            an_m3a_grid_state_without_its_grid_fails,
        ),
        (
            "a_missing_surface_fails_its_frame",
            a_missing_surface_fails_its_frame,
        ),
        (
            "a_pushed_title_bar_fails_its_frame",
            a_pushed_title_bar_fails_its_frame,
        ),
        (
            "the_matrix_covers_every_surface_locale_and_theme",
            the_matrix_covers_every_surface_locale_and_theme,
        ),
        (
            "pseudo_labels_must_be_bracketed_and_unclipped_and_at_least_11_px",
            pseudo_labels_must_be_bracketed_and_unclipped_and_at_least_11_px,
        ),
        (
            "the_command_line_reads_like_libtest",
            the_command_line_reads_like_libtest,
        ),
    ];
    let all: Vec<String> = shots().iter().map(|s| s.file_name(2.0)).collect();
    let render = cli.shots(&all);
    if cli.list {
        let mut n = 0;
        if !cli.ignored_only {
            for (name, _) in checks.iter().filter(|(name, _)| cli.selects_test(name)) {
                println!("{name}: test");
                n += 1;
            }
        }
        if render.is_some() {
            println!("{RENDER_TEST}: test");
            n += 1;
        }
        if !cli.terse {
            println!("\n{n} tests, 0 benchmarks");
        }
        return;
    }
    let (mut passed, mut ignored) = (0, 0);
    if !cli.ignored_only {
        for (name, check) in checks.iter().filter(|(name, _)| cli.selects_test(name)) {
            check();
            println!("test {name} ... ok");
            passed += 1;
        }
    }
    match render {
        Some(names) if cli.runs_ignored() => {
            render_all_surfaces(&names, names.len() == all.len());
            println!("test {RENDER_TEST} ... ok");
            passed += 1;
        }
        Some(_) if cli.exact => {
            eprintln!("{RENDER_TEST} is ignored: run it with --ignored or --include-ignored");
            std::process::exit(101);
        }
        Some(_) => {
            println!("test {RENDER_TEST} ... ignored, renders PNGs (run with --ignored)");
            ignored += 1;
        }
        None => {}
    }
    println!("\ntest result: ok. {passed} passed; 0 failed; {ignored} ignored");
}

// ## m2b-t22-live (fix round 2, R1-I1)
fn a_pushed_title_bar_fails_its_frame() {
    let b = |x: f32, w: f32| Bounds::new(point(px(x), px(0.0)), size(px(w), px(40.0)));
    // A 960 px window, right padding 10: trailing controls 830..950, palette 400..620.
    let good = TitleBarProbe {
        palette: Some(b(400.0, 220.0)),
        trailing: Some(b(830.0, 120.0)),
        right_padding: 10.0,
        // ## m4-m3carry (U-6 c)
        palette_collapsed: false,
    };
    assert!(check_title_bar(960.0, good, Some(b(100.0, 280.0))).is_empty());
    // Pushed right by a wide capsule: the theme button leaves the window.
    let pushed = TitleBarProbe {
        trailing: Some(b(865.0, 120.0)),
        ..good
    };
    let found = check_title_bar(960.0, pushed, None);
    assert!(found.iter().any(|v| v.contains("leave the")), "{found:?}");
    // Pushed by less than its padding: still inside, but no longer at its place.
    let nudged = TitleBarProbe {
        trailing: Some(b(834.0, 120.0)),
        ..good
    };
    let found = check_title_bar(960.0, nudged, None);
    assert!(
        found.iter().any(|v| v.contains("something pushed them")),
        "{found:?}"
    );
    // A squeezed palette field, and a capsule running under it.
    let squeezed = TitleBarProbe {
        palette: Some(b(500.0, 160.0)),
        ..good
    };
    let found = check_title_bar(960.0, squeezed, Some(b(100.0, 420.0)));
    assert!(found.iter().any(|v| v.contains("under its")), "{found:?}");
    assert!(
        found.iter().any(|v| v.contains("past the palette")),
        "{found:?}"
    );
    // No title bar drawn at all.
    assert!(!check_title_bar(960.0, TitleBarProbe::default(), None).is_empty());

    // ## m4-m3carry (U-6 c, P2-04): a field collapsed to its icon passes; a collapsed field of
    // another width fails; a trust word given up beside a whole field fails, beside the icon
    // it passes.
    let icon = TitleBarProbe {
        palette: Some(b(594.0, PALETTE_ICON_PX)),
        palette_collapsed: true,
        ..good
    };
    assert!(check_title_bar(960.0, icon, Some(b(100.0, 480.0))).is_empty());
    let found = check_title_bar(
        960.0,
        TitleBarProbe {
            palette: Some(b(500.0, 120.0)),
            ..icon
        },
        None,
    );
    assert!(found.iter().any(|v| v.contains("not its")), "{found:?}");
    // A read-write word that gives way below 400 px of room: beside a whole field (room 300)
    // the icon's 144 px would keep it; beside the icon (room 444) it is kept.
    let rooms = TrustRooms([Some(400.0), None, None]);
    let whole = |max: f32, given_up: u8| CapsuleProbe {
        bounds: Some(b(100.0, 280.0)),
        trust_given_up: given_up,
        max_width: Some(max),
        trust_rooms: rooms,
    };
    let found = check_trust_room(good, whole(300.0, 1));
    assert!(found.is_some_and(|v| v.contains("would keep")));
    assert!(check_trust_room(icon, whole(444.0, 0)).is_none());
    // Too narrow for the icon to help (room 200 + 144 < 400): the field stays whole.
    assert!(check_trust_room(good, whole(200.0, 1)).is_none());
    let found = check_trust_room(icon, whole(344.0, 1));
    assert!(found.is_some_and(|v| v.contains("kept no trust word")));
    // A fit that disagrees with its own trust rooms.
    let found = check_trust_room(good, whole(500.0, 1));
    assert!(found.is_some_and(|v| v.contains("trust rooms say")));
    // Nothing to give up: passes either way.
    let none = CapsuleProbe {
        trust_rooms: TrustRooms::default(),
        ..whole(300.0, 0)
    };
    assert!(check_trust_room(good, none).is_none());
}

// ## m3a-t35-shots
/// (g) A grid's header is data: bracketed in a pseudo-locale (or recorded as Fluent) it fails;
/// a clipped grid run fails in any locale; a fitting, ellipsized cell passes.
fn grid_text_is_data_and_never_clipped() {
    let grid = |path: &str, text: &str, source, clipped| RenderedText {
        text: text.to_string().into(),
        source,
        size_px: 13.0,
        painted_px: 40.0,
        clipped,
        truncated: false,
        path: path.to_string().into(),
        visible: true,
        outside_card: false,
        folded: false,
    };
    let header = "crates/ui/hacivat-ui-grid/src/element/layers.rs#header";
    let cell = "crates/ui/hacivat-ui-grid/src/element/layers.rs#cell";
    let fine = [
        grid(header, "customer", TextSource::Catalog, false),
        grid(cell, "[İzmir]", TextSource::Data, false),
        RenderedText {
            truncated: true,
            ..grid(cell, "teslimat cuma günü", TextSource::Data, false)
        },
    ];
    for locale in LOCALES {
        assert!(check_grid("s", locale, &fine).is_empty(), "{locale}");
    }
    let pseudo = [grid(header, "[ćüšŧöḿéř]", TextSource::Catalog, false)];
    assert_eq!(check_grid("s", "en-XA", &pseudo).len(), 1);
    assert!(
        check_grid("s", "en", &pseudo).is_empty(),
        "data may be bracketed"
    );
    let fluent = [grid(header, "customer", TextSource::Fluent, false)];
    assert_eq!(check_grid("s", "tr", &fluent).len(), 1);
    let clipped = [grid(cell, "İzmir", TextSource::Data, true)];
    let found = check_grid("s", "en", &clipped);
    assert!(
        found.len() == 1 && found[0].contains("(g) clipped"),
        "{found:?}"
    );
    // Only the grid's own records are judged here.
    let label = [grid(
        "crates/ui/hacivat-ui/src/menu.rs:1:1",
        "[x]",
        TextSource::Catalog,
        true,
    )];
    assert!(check_grid("s", "en-XA", &label).is_empty());
}

// ## m3a-t35-shots
/// Every M3a grid state needs a header its grid drew, besides its own marker: a frame with the
/// strip or the toast but no grid fails; every grid state's marker is specific (the header's
/// data text, or its own file's label).
fn an_m3a_grid_state_without_its_grid_fails() {
    let at = |path: &str, text: &str| RenderedText {
        text: text.to_string().into(),
        source: TextSource::Fluent,
        size_px: 13.0,
        painted_px: 10.0,
        clipped: false,
        truncated: false,
        path: path.to_string().into(),
        visible: true,
        outside_card: false,
        folded: false,
    };
    let strip = at(
        "crates/app/hacivat-app/src/results/strip.rs:329:21#strip-sentence",
        "Cursor open",
    );
    let header = |text: &str| {
        at(
            "crates/ui/hacivat-ui-grid/src/element/layers.rs#header",
            text,
        )
    };
    let toast = at(
        "crates/ui/hacivat-ui/src/toast.rs:300:9",
        "Copied 3 × 2 cells",
    );
    let mut grids = 0;
    for state in m3a::M3A {
        let surface = Surface::M3a(state);
        if !state.is_grid() {
            continue;
        }
        grids += 1;
        let (file, _) = surface.marker();
        assert!(
            file == m3a::GRID_HEADER || file.ends_with(".rs:"),
            "{state:?}: a specific marker"
        );
        let without = [strip.clone(), toast.clone()];
        assert!(surface.missing_from("s", &without).is_some(), "{state:?}");
        let with = [
            strip.clone(),
            toast.clone(),
            header("customer"),
            header("token"),
            header("c0"),
        ];
        assert!(surface.missing_from("s", &with).is_none(), "{state:?}");
    }
    assert!(grids >= 12, "{grids}");
    // A results surface's header marker is the column's data name, not any header.
    let rows = Surface::M3a(m3a::M3a::GridRange);
    assert!(rows.missing_from("s", &[header("id")]).is_some());
    // The toolbar states name their connection: another console's toolbar does not count.
    let toolbar = |name: &str| {
        [at(
            "crates/app/hacivat-app/src/console/toolbar.rs:284:9#console-connection-name",
            name,
        )]
    };
    // Fix round 1 (I1): a toolbar state needs its own state label; the connection name alone
    // (what a frame shows when the fixture did nothing) fails.
    let running = Surface::M3a(m3a::M3a::ToolbarRunning);
    assert!(running.missing_from("s", &toolbar("orders-dev")).is_some());
    let state_label = |id: &str| {
        at(
            &format!("crates/app/hacivat-app/src/console/toolbar.rs:250:17#{id}"),
            "x",
        )
    };
    for (state, id) in [
        (m3a::M3a::ToolbarRunning, "console-running"),
        (m3a::M3a::ToolbarCancelling, "console-cancelling"),
        (m3a::M3a::ToolbarReadOnly, "console-lock-label"),
        (m3a::M3a::ToolbarWaiting, "console-waiting-text"),
        // ## m3a-f2-visual (fix round 1, M1): the sentence never leaves the bar (it wraps).
        (m3a::M3a::ToolbarProductionWaiting, "console-waiting-text"),
    ] {
        let surface = Surface::M3a(state);
        assert!(
            surface.missing_from("s", &toolbar("orders-dev")).is_some(),
            "{state:?}"
        );
        assert!(
            surface.missing_from("s", &[state_label(id)]).is_none(),
            "{state:?}"
        );
    }
    // Describe and "Nothing to run" need the status message, not the editor segment.
    // ## m3c-t41-chrome (named exception): the Ln/Col label moved (one cell per part).
    let segment = [at(
        "crates/app/hacivat-app/src/console/status.rs:290:13#editor-segment-lncol",
        "Ln 2, Col 38",
    )];
    let describe = Surface::M3a(m3a::M3a::ContextDescribe);
    assert!(describe.missing_from("s", &segment).is_some());
}

// ## m3a-t35-shots (fix round 1, I2)
/// A marker or a trust signal counts only when visible and painted; each production,
/// transaction and cursor state fails without its trust label.
fn trust_signals_must_be_drawn() {
    // Painted 0 models an ellipsized label that had no room for its ellipsis.
    let at = |id: &str, painted: f32, visible: bool| RenderedText {
        text: "x".into(),
        source: TextSource::Fluent,
        size_px: 12.0,
        painted_px: painted,
        clipped: false,
        truncated: painted == 0.0,
        path: format!("crates/x.rs:1:1#{id}").into(),
        visible,
        outside_card: false,
        folded: false,
    };
    // A marker that drew nothing, or is hidden, does not count.
    let cursor = Surface::M3a(m3a::M3a::StatusCursorOpen);
    assert!(
        cursor
            .missing_from("s", &[at("button-label-status-cursor", 40.0, true)])
            .is_none()
    );
    assert!(
        cursor
            .missing_from("s", &[at("button-label-status-cursor", 0.0, true)])
            .is_some()
    );
    assert!(
        cursor
            .missing_from("s", &[at("button-label-status-cursor", 40.0, false)])
            .is_some()
    );
    // Production: the status bar's environment and the transaction mode chip (## m3b-t26-
    // prod-unlock: the production read-only notice is gone, R-M3-81; Manual, D-036).
    let prod = Surface::M3a(m3a::M3a::ConsoleProduction);
    let full = [
        at("tx-chip-mode", 90.0, true),
        at("status-env-label", 90.0, true),
    ];
    assert!(prod.trust_missing("s", &full).is_empty());
    let found = prod.trust_missing("s", &[at("status-env-label", 90.0, true)]);
    assert!(found.len() == 1 && found[0].contains("(t)"), "{found:?}");
    let squeezed = [
        at("tx-chip-mode", 0.0, true),
        at("status-env-label", 90.0, false),
    ];
    // ## m3b-t29-shots (fix round 1, I2): the squeezed chip is also a cut trust label.
    assert_eq!(prod.trust_missing("s", &squeezed).len(), 3);
    // A trust label ellipsized (it drew, but cut short) or clipped does not count as shown,
    // and the production capsule's name must show whole.
    let ellipsized = RenderedText {
        truncated: true,
        ..at("status-env-label", 30.0, true)
    };
    let found = prod.trust_missing("s", &[at("tx-chip-mode", 90.0, true), ellipsized]);
    assert!(
        found.iter().any(|v| v.contains("not shown whole"))
            && found.iter().any(|v| v.contains("cut short")),
        "{found:?}"
    );
    let clipped = RenderedText {
        clipped: true,
        ..at("capsule-text-name", 40.0, true)
    };
    let found = prod.trust_missing("s", &[full[0].clone(), full[1].clone(), clipped]);
    assert!(
        found.len() == 1 && found[0].contains("#capsule-text-name"),
        "{found:?}"
    );
    // ## m4b-t22-pre (T21 r1): a trust label cut or hidden at a scroll area's fold does not
    // count as shown either, and is a cut trust label.
    let folded = RenderedText {
        folded: true,
        ..at("status-env-label", 90.0, true)
    };
    let found = prod.trust_missing("s", &[full[0].clone(), folded]);
    assert!(
        found.iter().any(|v| v.contains("not shown whole"))
            && found.iter().any(|v| v.contains("cut short")),
        "{found:?}"
    );
    // (s) The first notice's title: whole passes; folded, clipped or hidden fails; a later
    // notice's title may fold.
    let title = |folded: bool| RenderedText {
        folded,
        ..at("script-notice-title", 90.0, true)
    };
    assert_eq!(check_notice_title("s", &[title(false), title(true)]), None);
    assert!(
        check_notice_title("s", &[title(true), title(false)]).is_some_and(|v| v.contains("(s)"))
    );
    let clipped_title = RenderedText {
        clipped: true,
        ..title(false)
    };
    assert!(check_notice_title("s", &[clipped_title]).is_some());
    let hidden_title = RenderedText {
        visible: false,
        ..title(false)
    };
    assert!(check_notice_title("s", &[hidden_title]).is_some());
    assert_eq!(
        check_notice_title("s", &[]),
        None,
        "no notice, nothing to check"
    );
    // Transaction and cursor.
    let everything = Surface::M3a(m3a::M3a::StatusEverything);
    let all = [
        at("status-tx-open", 90.0, true),
        at("button-label-status-cursor", 60.0, true),
        at("status-rows", 60.0, true),
    ];
    assert!(everything.trust_missing("s", &all).is_empty());
    assert_eq!(everything.trust_missing("s", &all[..1]).len(), 2);
    // ## m3a-f2-visual: the tile's rows · timing stands in for the bar's cell that gave way
    // (at 960 in the pseudo-locales; the state check requires the bar's own cell at 1280);
    // with neither, the count is missing.
    let tile = [
        at("status-tx-open", 90.0, true),
        at("button-label-status-cursor", 60.0, true),
        at("results-rows-timing", 60.0, true),
    ];
    assert!(everything.trust_missing("s", &tile).is_empty());
    assert_eq!(everything.trust_missing("s", &tile[..2]).len(), 1);
    let failed = Surface::M3a(m3a::M3a::PromptFailed);
    assert_eq!(failed.trust_missing("s", &all).len(), 1);
    // Every production state requires the production signals.
    // ## m3b-t29-shots: since Task 26 (R-M3-81) the read-only notice is no longer one of them,
    // so a production state requires the environment (and, with a session, the mode chip): the
    // count of two that Task 26 left here failed for the toolbar and marks states.
    // ## m3b-t29-shots (fix round 1, m2): the exact groups per production state.
    let env: &[&str] = &["#status-env-label"];
    let chip: &[&str] = &["#tx-chip-mode"];
    let production: Vec<m3a::M3a> = m3a::M3A
        .into_iter()
        .filter(|s| s.shows_production())
        .collect();
    assert_eq!(
        production,
        [
            m3a::M3a::ConsoleProduction,
            m3a::M3a::ToolbarProductionWaiting,
            m3a::M3a::MarksDenied,
            m3a::M3a::StatusProductionResult,
        ]
    );
    // ## m3c-t41-chrome (named exception; F2 review N2): the production waiting state also
    // requires the wait's Cancel.
    let cancel: &[&str] = &["#button-label-console-waiting-cancel"];
    for state in production {
        let want: &[&[&str]] = match state {
            m3a::M3a::StatusProductionResult => &[env],
            m3a::M3a::ToolbarProductionWaiting => &[env, chip, cancel],
            _ => &[env, chip],
        };
        assert_eq!(state.required(), want, "{state:?}");
    }
    // An M1/M2 surface has none.
    assert!(Surface::Shell.trust_missing("s", &[]).is_empty());
    // A label recorded before layout (painted 0, not truncating) counts: the M2a notice.
    let notice = RenderedText {
        truncated: false,
        ..at("notice", 0.0, true)
    };
    assert!(drawn(&notice));
}

// ## m3b-t29-shots
/// Every M3b state has a marker signature (its marker and the labels it must also show) no
/// other M3b state has, except the groups `m3b::TOLD_APART_BY_CHECK` names (their model check
/// tells them apart); a shell-only frame satisfies none of them; and every state that shows
/// production requires its trust signals.
fn every_m3b_state_has_its_own_marker() {
    let mut by_signature: std::collections::BTreeMap<String, Vec<&'static str>> =
        std::collections::BTreeMap::new();
    for state in m3b::all() {
        let (file, text) = state.marker();
        let key = format!("{file} {text:?} {:?}", state.also_shown());
        by_signature.entry(key).or_default().push(state.name());
    }
    let mut shared: Vec<Vec<&str>> = by_signature
        .into_values()
        .filter(|names| names.len() > 1)
        .collect();
    shared.sort();
    let mut expected: Vec<Vec<&str>> = m3b::TOLD_APART_BY_CHECK
        .iter()
        .map(|g| g.to_vec())
        .collect();
    expected.sort();
    assert_eq!(shared, expected, "only these share a marker signature");
    // ## fix round 1 (I4): a real fresh shell's frame (the labels the shell under every
    // fixture draws, as `SHOTS_DUMP=1` printed them for `confirm-typed-dark-en-1280x800`: the
    // title bar, the explorer, the home pane's buttons, the status bar), and a live dev
    // console's (the capsule, the toolbar with Run and the mode chip, the status bar; from
    // `errbar-dark-en-1280x800`). No M3b marker is satisfied by the shell; no fixture state's
    // (a dialog, a prompt, a menu) by a live console either.
    let label = |path: &str, text: &str| RenderedText {
        text: text.to_string().into(),
        source: TextSource::Fluent,
        size_px: 13.0,
        painted_px: 10.0,
        clipped: false,
        truncated: false,
        path: path.to_string().into(),
        visible: true,
        outside_card: false,
        folded: false,
    };
    let shell: Vec<RenderedText> = [
        (
            "crates/ui/hacivat-ui/src/titlebar.rs:297:25#palette-placeholder",
            "Search or run a command",
        ),
        ("crates/ui/hacivat-ui/src/segmented.rs:194:25", "EN"),
        (
            "crates/app/hacivat-app/src/shell/tiles.rs:156:5#panel-explorer-title",
            "EXPLORER",
        ),
        (
            "hacivat-ui/src/input/mod.rs#placeholder",
            "Filter connections",
        ),
        (
            "crates/ui/hacivat-ui/src/button.rs:272:21#button-label-explorer-new",
            "New connection",
        ),
        ("crates/ui/hacivat-ui/src/button.rs:456:31", "⌘N"),
        (
            "crates/ui/hacivat-ui/src/button.rs:272:21#button-label-explorer-paste",
            "Paste URL",
        ),
        (
            "crates/ui/hacivat-ui/src/button.rs:272:21#button-label-explorer-import",
            "Import pg_service.conf…",
        ),
        (
            "crates/app/hacivat-app/src/shell/tiles.rs:197:13#sheet-none",
            "Select a connection to see its details.",
        ),
        (
            "crates/app/hacivat-app/src/connections/home.rs:81:21#home-title",
            "No query open",
        ),
        (
            "crates/ui/hacivat-ui/src/button.rs:272:21#button-label-home-new",
            "New connection",
        ),
        (
            "crates/ui/hacivat-ui/src/button.rs:272:21#button-label-home-paste",
            "New connection from a copied URL",
        ),
        (
            "crates/ui/hacivat-ui/src/button.rs:272:21#button-label-home-connect",
            "Connect to the selected connection",
        ),
        (
            "crates/ui/hacivat-ui/src/button.rs:272:21#button-label-home-palette",
            "Search or run a command",
        ),
        (
            "crates/app/hacivat-app/src/shell/tiles.rs:175:13#results-empty",
            "Run a query to see results here",
        ),
        (
            "crates/ui/hacivat-ui/src/statusbar.rs:17:5",
            "No connection",
        ),
        (
            "crates/app/hacivat-app/src/shell/workspace.rs:1618:13#status-language-0",
            "EN",
        ),
    ]
    .into_iter()
    .map(|(p, t)| label(p, t))
    .collect();
    let console: Vec<RenderedText> = [
        (
            "crates/ui/hacivat-ui/src/capsule.rs:829:21#capsule-text-name",
            "orders-dev",
        ),
        (
            "crates/ui/hacivat-ui/src/capsule.rs:1042:25#capsule-text-access",
            "read-write",
        ),
        (
            "crates/ui/hacivat-ui/src/button.rs:272:21#button-label-console-run",
            "Run",
        ),
        (
            "crates/app/hacivat-app/src/console/toolbar.rs:284:9#console-connection-name",
            "orders-dev",
        ),
        (
            "crates/app/hacivat-app/src/console/tx_chip.rs:170:17#tx-chip-mode",
            "Auto-commit",
        ),
        (
            "crates/app/hacivat-app/src/connections/status.rs:126:17#status-env-label",
            "dev",
        ),
        (
            "crates/app/hacivat-app/src/connections/status.rs:126:17#status-path",
            "Direct",
        ),
        (
            "crates/app/hacivat-app/src/console/status.rs:290:13#editor-segment-lncol",
            "Ln 2, Col 38",
        ),
        ("crates/ui/hacivat-ui-editor/src/marks.rs:1:1", "1 of 3"),
    ]
    .into_iter()
    .map(|(p, t)| label(p, t))
    .collect();
    for state in m3b::all() {
        assert!(
            Surface::M3b(state).missing_from("s", &shell).is_some(),
            "{state:?}: a fresh shell satisfies its marker"
        );
        if state.is_fixture() {
            let both: Vec<RenderedText> = shell.iter().chain(&console).cloned().collect();
            assert!(
                Surface::M3b(state).missing_from("s", &both).is_some(),
                "{state:?}: a live console satisfies its marker"
            );
        }
        if state.shows_production() {
            assert!(!state.required().is_empty(), "{state:?}");
        }
    }
    // A marker with data text needs that text: another statement's SQL line does not count.
    let line = |text: &str| RenderedText {
        text: text.to_string().into(),
        path: "crates/app/hacivat-app/src/exec_ui/confirm.rs:1344:9#confirm-sql-line".into(),
        ..label("", "")
    };
    let routine = Surface::M3b(m3b::M3b::Confirm(
        hacivat_app::exec_ui::confirm::fixtures::State::Routine,
    ));
    assert!(
        routine
            .missing_from("s", &[line("CALL close_month('2026-10');")])
            .is_none()
    );
    assert!(
        routine
            .missing_from("s", &[line("DROP SCHEMA reporting CASCADE;")])
            .is_some()
    );
}

// ## m3c-t43-shots
/// Every M3c state has a marker signature (its marker and the labels it must also show) no
/// other M3c or M3b state has, except the groups `m3c::TOLD_APART_BY_CHECK` names (their model
/// check tells them apart); no M3c marker is satisfied by a live console's own frame (the
/// capsule, the toolbar, the status bar with the line and column) or a result tile's (its grid's
/// header and strip) alone, unless its check holds the state; and every M3c name is new.
fn every_m3c_state_has_its_own_marker() {
    let mut by_signature: std::collections::BTreeMap<String, Vec<&'static str>> =
        std::collections::BTreeMap::new();
    for state in m3c::all() {
        let (file, text) = state.marker();
        let key = format!("{file} {text:?} {:?}", state.also_shown());
        by_signature.entry(key).or_default().push(state.name());
    }
    let mut shared: Vec<Vec<&str>> = by_signature
        .into_values()
        .filter(|names| names.len() > 1)
        .collect();
    shared.sort();
    let mut expected: Vec<Vec<&str>> = m3c::TOLD_APART_BY_CHECK
        .iter()
        .map(|g| g.to_vec())
        .collect();
    expected.sort();
    assert_eq!(shared, expected, "only these share a marker signature");
    // No M3c state shares a marker signature with an M3b state.
    for b in m3b::all() {
        let sig = format!("{:?} {:?}", b.marker(), b.also_shown());
        for c in m3c::all() {
            assert_ne!(
                sig,
                format!("{:?} {:?}", c.marker(), c.also_shown()),
                "{c:?} and {b:?}"
            );
        }
    }
    // The names are new: none is an M1/M2, M3a or M3b shot's.
    let others: std::collections::BTreeSet<&str> = SURFACES
        .iter()
        .map(|s| s.name())
        .chain(m3a::M3A.iter().map(|s| s.name()))
        .chain(m3b::all().iter().map(|s| s.name()))
        .collect();
    for state in m3c::all() {
        assert!(!others.contains(state.name()), "{state:?}");
        // The grid states ask for the header their grid drew.
        if state.is_grid() {
            assert!(state.also_shown().contains(&"#header"), "{state:?}");
        }
    }
    // A live dev console's frame (from `errbar-dark-en-1280x800`), and a result tile's (a grid's
    // header and the strip): no M3c marker but the ones its check tells apart.
    let label = |path: &str, text: &str| RenderedText {
        text: text.to_string().into(),
        source: TextSource::Fluent,
        size_px: 13.0,
        painted_px: 10.0,
        clipped: false,
        truncated: false,
        path: path.to_string().into(),
        visible: true,
        outside_card: false,
        folded: false,
    };
    let console: Vec<RenderedText> = [
        (
            "crates/ui/hacivat-ui/src/capsule.rs:829:21#capsule-text-name",
            "orders-dev",
        ),
        (
            "crates/ui/hacivat-ui/src/button.rs:272:21#button-label-console-run",
            "Run",
        ),
        (
            "crates/app/hacivat-app/src/console/tx_chip.rs:170:17#tx-chip-mode",
            "Auto-commit",
        ),
        (
            "crates/app/hacivat-app/src/connections/status.rs:126:17#status-env-label",
            "dev",
        ),
        (
            "crates/app/hacivat-app/src/console/status.rs:116:13#editor-segment-lncol",
            "Ln 2, Col 38",
        ),
        (
            "crates/app/hacivat-app/src/console/toolbar.rs:284:9#console-connection-name",
            "orders-dev",
        ),
        (
            "crates/ui/hacivat-ui-grid/src/element/layers.rs#header",
            "şehir",
        ),
        (
            "crates/app/hacivat-app/src/results/strip.rs:1:1#strip-rows",
            "40 rows",
        ),
    ]
    .into_iter()
    .map(|(p, t)| label(p, t))
    .collect();
    let mut satisfied: Vec<&str> = m3c::all()
        .into_iter()
        .filter(|s| Surface::M3c(*s).missing_from("s", &console).is_none())
        .map(|s| s.name())
        .collect();
    satisfied.sort();
    let mut by_check: Vec<&str> = m3c::BY_CHECK_ALONE.to_vec();
    by_check.sort();
    assert_eq!(
        satisfied, by_check,
        "markers a bare console or tile satisfies"
    );
    // Every known finding names an M3c state, a label and the merged task that owns its fix;
    // none is about a trust signal (those always fail), and a violation outside the list is
    // never one.
    let names: Vec<&str> = m3c::all().iter().map(|s| s.name()).collect();
    // ## m3c-t43b: part B's list too.
    for f in m3c::all_findings() {
        assert!(names.contains(&f.state), "{f:?}");
        assert!(!f.label.is_empty(), "{f:?}");
        assert!(
            // ## m3c-t43b: and part B's owners.
            [
                "T31", "T32", "T33", "T34", "T35", "T36", "T37", "T38", "T41", "T42"
            ]
            .contains(&f.owner),
            "{f:?}"
        );
        assert!(
            !f.label.contains("(t)") && !f.label.contains("trust"),
            "{f:?}"
        );
    }
    // ## m3c-t43-shots (fix round 1, I2): no finding names a trust label of any M3c state, by
    // id or by a fragment of one.
    for state in m3c::all() {
        for id in m3c::trust_ids(state) {
            for f in m3c::all_findings() {
                assert!(
                    !id.contains(f.label) && !f.label.contains(id),
                    "{f:?} names the trust label {id}"
                );
            }
        }
    }
    known_findings_match_only_their_own_clip();
}

// ## m3c-t43-shots (fix round 1, I1, I2)
/// A finding tolerates only its one kind on its one label (the real lines the harness prints),
/// never a missing surface, a failed state check, another rule's line or a trust line, even
/// when an entry is planted on a trust id.
fn known_findings_match_only_their_own_clip() {
    use m3c::{Finding, M3c, Where};
    let (en, big) = ("en", (1280.0, 800.0));
    let (tr_xt, min) = ("tr-XT", (960.0, 600.0));
    let ask = Finding {
        state: "goto-fetch-ask",
        at: Where::All,
        label: "#goto-line",
        owner: "T37",
    };
    let s = M3c::GotoAsk;
    let n = "goto-fetch-ask-dark-en-1280x800@2x.png";
    let clip = format!(
        "{n}: (b) clipped: \"Row 1,500,000 isn't fetched yet.\" (278.0 px painted) at \
         crates/app/hacivat-app/src/results/goto.rs:808:25#goto-line"
    );
    assert!(ask.matches(s, en, big, &clip, None), "the real clip");
    for never in [
        format!("{n}: the surface is missing: [\"#goto-line\"] not visible"),
        format!("{n}: the surface is missing: no visible label from #goto-line None"),
        format!(
            "{n}: goto-fetch-ask: the state check failed (carets 0, ▶ 0, chevrons 0, folded 0)"
        ),
        format!("{n}: (a) Fluent label not bracketed: \"x\" at goto.rs:808:25#goto-line"),
        format!("{n}: (c) 10 px < 11: \"x\" at goto.rs:808:25#goto-line"),
        format!("{n}: (f) runs past its dialog card: \"x\" at goto.rs:808:25#goto-line"),
        // Another label of the same state, and the same label as a fragment of a longer id.
        format!("{n}: (b) clipped: \"x\" (1.0 px painted) at goto.rs:1:1#goto-line-2"),
        format!("{n}: (b) clipped: \"x\" (1.0 px painted) at goto.rs:1:1#button-label-goto-ok"),
    ] {
        assert!(!ask.matches(s, en, big, &never, None), "{never}");
    }
    // Another state's line on the same label.
    assert!(!ask.matches(M3c::GotoOpen, en, big, &clip, None));
    // An id fragment (`value-viewer-`) matches inside the line's id only, a clip only.
    let viewer = Finding {
        state: "viewer-json-invalid",
        at: Where::PseudoOrMin,
        label: "value-viewer-",
        owner: "T38",
    };
    let v = M3c::ViewerJsonInvalid;
    let vn = "viewer-json-invalid-dark-en-960x600@2x.png";
    let copy = format!(
        "{vn}: (b) clipped: \"Copy raw value\" (80.2 px painted) at \
         crates/ui/hacivat-ui/src/button.rs:272:21#button-label-value-viewer-copy"
    );
    assert!(viewer.matches(v, en, min, &copy, None));
    assert!(!viewer.matches(v, en, big, &copy, None), "out of its scope");
    for never in [
        format!(
            "{vn}: the surface is missing: no visible label from #value-viewer-column Some(\"bozuk_belge\")"
        ),
        format!("{vn}: the surface is missing: [\"#value-viewer-size\"] not visible"),
        format!(
            "{vn}: (a) Fluent label not bracketed: \"x\" at value_viewer.rs:1:1#value-viewer-size"
        ),
    ] {
        assert!(!viewer.matches(v, tr_xt, min, &never, None), "{never}");
    }
    // A creation site without an id.
    let tip = Finding {
        state: "gutter-last-runs",
        at: Where::Pseudo,
        label: "gutter.rs:708",
        owner: "T34",
    };
    let line = "gutter-last-runs-dark-tr-XT-960x600@2x.png: (b) clipped: \"x\" (292.7 px painted) \
                at crates/ui/hacivat-ui-editor/src/gutter.rs:708:19";
    assert!(tip.matches(M3c::GutterLastRuns, tr_xt, min, line, None));
    let with_id = format!("{line}#gutter-run");
    assert!(!tip.matches(M3c::GutterLastRuns, tr_xt, min, &with_id, None));
    // The header word: only the Transposed name column's word, never another header.
    let word = Finding {
        state: "view-transposed",
        at: Where::Pseudo,
        label: "a grid header is data",
        owner: "T38",
    };
    let header = |text: &str| {
        format!(
            "view-transposed-dark-tr-XT-960x600@2x.png: (g) a grid header is data, never \
             pseudo-localised: {text:?} at crates/ui/hacivat-ui-grid/src/element/layers.rs#header"
        )
    };
    let t = M3c::ViewTransposed;
    assert!(word.matches(
        t,
        tr_xt,
        min,
        &header("[Çööööŀüüüüḿƞ]"),
        Some("[Çööööŀüüüüḿƞ]")
    ));
    assert!(!word.matches(t, tr_xt, min, &header("[şéhîr]"), Some("[Çööööŀüüüüḿƞ]")));
    assert!(!word.matches(t, tr_xt, min, &header("[Çööööŀüüüüḿƞ]"), None));
    // I2: an entry planted on a trust id never hides a trust line, nor the trust label's clip.
    for (state, id) in [
        (M3c::GotoAsk, "#status-env-label"),
        (M3c::ViewerTruncated, "#status-env-label"),
        (M3c::GotoAsk, "#capsule-text-name"),
    ] {
        let planted = Finding {
            state: state.name(),
            at: Where::All,
            label: id,
            owner: "T37",
        };
        let n = format!("{}-dark-en-1280x800@2x.png", state.name());
        for line in [
            format!("{n}: (t) a trust signal is not shown whole: [{id:?}]"),
            format!("{n}: (t) a trust label is cut short: {id}"),
            format!("{n}: (b) clipped: \"production\" (30.0 px painted) at status.rs:1:1{id}"),
        ] {
            assert!(!planted.matches(state, en, big, &line, None), "{line}");
        }
    }
    // The listed entries, against the real lines the harness printed for them.
    for (state, locale, size, line, word) in [
        (M3c::GotoAsk, en, big, clip.as_str(), None),
        (M3c::ViewerJsonInvalid, en, min, copy.as_str(), None),
        (M3c::GutterLastRuns, tr_xt, min, line, None),
    ] {
        if m3c::KNOWN_FINDINGS.iter().any(|f| f.state == state.name()) {
            assert!(
                m3c::KNOWN_FINDINGS
                    .iter()
                    .any(|f| f.matches(state, locale, size, line, word)),
                "{line}"
            );
        }
    }
    // ## m3c-t43b-fix (review m2, m3): part B's entries against the real lines the full render
    // printed, each only in its own scope (F11's body in English and Turkish, not pseudo; F12's
    // tab title, a creation site, in the pseudo-locales only). ## m3c-narrow (fix round 1, M9):
    // `KNOWN_FINDINGS_B` is empty since F10–F12 were fixed, so the entries are planted here, as
    // the I2 block above plants its own, and the scoping runs whatever the list holds.
    let reduced = M3c::NarrowReduced;
    let body = "narrow-reduced-dark-en-1280x800@2x.png: (b) clipped: \"Reduced mode keeps typing \
                fast. It ends when the document is smaller than 3.2 MB.\" (57.0 px painted) at \
                crates/ui/hacivat-ui-editor/src/large_file.rs:280:16#editor-t1-body";
    let tab = "narrow-reduced-dark-tr-XT-960x600@2x.png: (b) clipped: \"[Çööööƞşööööŀḗḗḗ 2]\" \
               (117.3 px painted) at crates/ui/hacivat-ui/src/panes.rs:443:24";
    let planted_b = [
        Finding {
            state: reduced.name(),
            at: Where::Pseudo,
            label: "#editor-t1-title",
            owner: "T41",
        },
        Finding {
            state: reduced.name(),
            at: Where::Plain,
            label: "#editor-t1-body",
            owner: "T41",
        },
        Finding {
            state: reduced.name(),
            at: Where::Pseudo,
            label: "hacivat-ui/src/panes.rs:443",
            owner: "T41",
        },
    ];
    let part_b = |line: &str, locale: &str, size| {
        planted_b
            .iter()
            .any(|f| f.matches(reduced, locale, size, line, None))
    };
    assert!(part_b(body, en, big), "{body}");
    assert!(part_b(body, "tr", min), "{body}");
    assert!(!part_b(body, "en-XA", big), "out of its scope: {body}");
    assert!(part_b(tab, tr_xt, min), "{tab}");
    assert!(!part_b(tab, en, min), "out of its scope: {tab}");
    let with_id = format!("{tab}#tab-title");
    assert!(!part_b(&with_id, tr_xt, min), "{with_id}");
    // `#editor-t1-title` is a trust label of `narrow-reduced` now (fix round 1): no entry hides
    // its clip, whatever its scope.
    let title = "narrow-reduced-dark-tr-XT-960x600@2x.png: (b) clipped: \"[Ŧħıııış]\" (57.0 px \
                 painted) at crates/ui/hacivat-ui-editor/src/large_file.rs:279:16#editor-t1-title";
    assert!(!part_b(title, tr_xt, min), "a trust label: {title}");
}

// ## m3c-t43-shots (fix round 1, m1)
/// The T45 gate: with `HACIVAT_T45_GATE=1` no known finding may remain (the controller's ruling:
/// the list is temporary and empty at the M3c gate).
fn known_findings_are_empty_at_the_t45_gate() {
    if std::env::var_os("HACIVAT_T45_GATE").is_some() {
        // ## m3c-t43b: part B's list too.
        let left: Vec<_> = m3c::all_findings().collect();
        assert!(
            left.is_empty(),
            "the T45 gate: every known finding must be fixed first: {left:?}"
        );
    }
}

fn a_missing_surface_fails_its_frame() {
    let at = |path: &str, text: &str| RenderedText {
        text: text.to_string().into(),
        source: TextSource::Fluent,
        size_px: 13.0,
        painted_px: 10.0,
        clipped: false,
        truncated: false,
        path: path.to_string().into(),
        visible: true,
        outside_card: false,
        folded: false,
    };
    let shell = [at(
        "crates/ui/hacivat-ui/src/titlebar.rs:297:25#palette-placeholder",
        "Search",
    )];
    assert!(Surface::Shell.missing_from("s", &shell).is_none());
    for surface in [
        Surface::PaletteOpen,
        Surface::SettingsGeneral,
        Surface::DialogAbout,
        Surface::ToastInfo,
        Surface::MenuApp,
        Surface::ShellDemoProduction,
    ] {
        let found = surface
            .missing_from("s", &shell)
            .expect("a shell-only frame misses it");
        assert!(found.contains("the surface is missing"), "{found}");
    }
    // ## m2a-t21-shots: an M2a surface needs its own file's label too.
    for surface in [
        Surface::ConnDialogNew,
        Surface::ConnErrorAuth,
        Surface::ExplorerList,
        Surface::UnencryptedPopover,
        Surface::ToastStickyDemo,
    ] {
        assert!(surface.missing_from("s", &shell).is_some(), "{surface:?}");
    }
    // ## m2b-t24-shots (fix round 1, I2): a marker that is not visible (scrolled out, below the
    // window) does not count, and the SSH rows a surface is about must show too.
    let route = |k: usize, visible: bool| RenderedText {
        visible,
        ..at(
            &format!("crates/app/hacivat-app/src/connections/dialog/ssh.rs:2760:21#ssh-route-{k}"),
            "x",
        )
    };
    let hop = [route(0, true), route(1, true), route(2, true)];
    assert!(Surface::SshSectionHop.missing_from("s", &hop).is_none());
    let scrolled_out = [route(0, true), route(1, false), route(2, true)];
    let found = Surface::SshSectionHop
        .missing_from("s", &scrolled_out)
        .expect("an off-screen marker does not count");
    assert!(found.contains("no visible label"), "{found}");
    let cut_route = [route(0, true), route(1, true), route(2, false)];
    let found = Surface::SshSectionHop
        .missing_from("s", &cut_route)
        .expect("the whole route must show");
    assert!(found.contains("#ssh-route-2"), "{found}");
    // A Read-only checkbox or the TLS disclosure never stands in for the SSH section.
    let others = [
        at(
            "crates/ui/hacivat-ui/src/checkbox.rs:150:17#checkbox-label-read-only",
            "Read-only",
        ),
        at(
            "crates/ui/hacivat-ui/src/disclosure.rs:160:21#disclosure-summary-conn-tls",
            "Auto",
        ),
    ];
    assert!(Surface::SshSectionOff.missing_from("s", &others).is_some());
    assert!(
        Surface::SshSectionFolded
            .missing_from("s", &others)
            .is_some()
    );
    // ## m2b-t24-shots (follow-up): the new labels' markers name the label shown: a verified
    // connection does not stand in for a pinned one, nor the Keychain for the vault; and the
    // sheet's value must show whole.
    let pinned = |key: &str| {
        [
            at(
                &format!("crates/app/hacivat-app/src/connections/sheet.rs:390:17#sheet-enc-{key}"),
                "x",
            ),
            at(
                &format!(
                    "crates/app/hacivat-app/src/connections/status.rs:126:17#status-enc-{key}"
                ),
                "x",
            ),
        ]
    };
    assert!(
        Surface::LiveTlsPinned
            .missing_from("s", &pinned("pinned"))
            .is_none()
    );
    assert!(
        Surface::LiveTlsPinned
            .missing_from("s", &pinned("verified"))
            .is_some()
    );
    let cut = [
        RenderedText {
            truncated: true,
            ..pinned("pinned")[0].clone()
        },
        pinned("pinned")[1].clone(),
    ];
    // An ellipsized pinned label fails in every locale; a whole one passes.
    for locale in LOCALES {
        let found = Surface::LiveTlsPinned.cut_short("s", locale, &cut);
        assert!(
            found.len() == 1 && found[0].contains("cut short"),
            "{locale}: {found:?}"
        );
        assert!(
            Surface::LiveTlsPinned
                .cut_short("s", locale, &pinned("pinned"))
                .is_empty()
        );
    }
    let secret = |key: &str| {
        [at(
            &format!("crates/app/hacivat-app/src/connections/sheet.rs:410:17#sheet-secret-{key}"),
            "x",
        )]
    };
    assert!(
        Surface::LiveVaultSecret
            .missing_from("s", &secret("vault"))
            .is_none()
    );
    assert!(
        Surface::LiveVaultSecret
            .missing_from("s", &secret("forever"))
            .is_some()
    );
    // The vault label must fit in English and Turkish; a pseudo-locale may ellipsize it.
    let cut_vault = [RenderedText {
        truncated: true,
        ..secret("vault")[0].clone()
    }];
    for (locale, fails) in [
        ("en", true),
        ("tr", true),
        ("en-XA", false),
        ("tr-XT", false),
    ] {
        assert_eq!(
            Surface::LiveVaultSecret
                .cut_short("s", locale, &cut_vault)
                .len(),
            usize::from(fails),
            "{locale}"
        );
    }
    let fixes = |ids: &[&str]| -> Vec<RenderedText> {
        ids.iter()
            .map(|id| {
                at(
                    &format!("crates/ui/hacivat-ui/src/button.rs:256:25#button-label-{id}"),
                    "x",
                )
            })
            .collect()
    };
    assert!(
        Surface::ErrTlsNotOffered
            .missing_from("s", &fixes(&["fix-ssh", "fix-edit"]))
            .is_none()
    );
    assert!(
        Surface::ErrTlsNotOffered
            .missing_from("s", &fixes(&["fix-edit"]))
            .is_some()
    );
    // ## m2b-t24-shots: so does every T24 surface; the import menu needs its alias drawn.
    for surface in &SURFACES[FIRST_T24..] {
        assert!(surface.missing_from("s", &shell).is_some(), "{surface:?}");
    }
    let menu = |text| {
        [
            at("crates/ui/hacivat-ui/src/button.rs:240:21", text),
            at(
                "crates/ui/hacivat-ui/src/menu.rs:560:21#popup-menu-title",
                "Hosts",
            ),
        ]
    };
    assert!(
        Surface::SshImportMenu
            .missing_from("s", &menu("No hosts in ~/.ssh/config"))
            .is_some()
    );
    assert!(
        Surface::SshImportMenu
            .missing_from("s", &menu("bastion"))
            .is_none()
    );
    // ## m3b-ci (controller fix): on Linux and Windows the row is drawn as access-key runs
    // (`""`, `b`, `astion`); the marker counts them joined, and only joined into its text.
    let runs = |texts: [&str; 3]| {
        let mut frame: Vec<RenderedText> = texts
            .map(|t| {
                at(
                    "crates/ui/hacivat-ui/src/button.rs:272:21#menu-item-label",
                    t,
                )
            })
            .into();
        frame.push(at(
            "crates/ui/hacivat-ui/src/menu.rs:560:21#popup-menu-title",
            "Hosts",
        ));
        frame
    };
    assert!(
        Surface::SshImportMenu
            .missing_from("s", &runs(["", "b", "astion"]))
            .is_none()
    );
    assert!(
        Surface::SshImportMenu
            .missing_from("s", &runs(["", "b", "astions"]))
            .is_some()
    );
    // ## m4-t14-explorer (named exception): the tree draws the profile rows.
    let explorer = [at(
        "crates/ui/hacivat-ui/src/tree/view/row.rs:442:21#hv-tree-label",
        "orders-prod",
    )];
    assert!(Surface::ExplorerList.missing_from("s", &explorer).is_none());
    // The demo needs the harman capsule, not just any capsule label.
    let capsule = |text| [at("crates/ui/hacivat-ui/src/capsule.rs:111:21", text)];
    assert!(
        Surface::ShellDemoProduction
            .missing_from("s", &capsule("No connection"))
            .is_some()
    );
    assert!(
        Surface::ShellDemoProduction
            .missing_from("s", &capsule("orders-prod"))
            .is_none()
    );
}

fn the_command_line_reads_like_libtest() {
    let parse = |s: &str| Cli::parse(s.split_whitespace().map(str::to_string));
    let all: Vec<String> = shots().iter().map(|s| s.file_name(2.0)).collect();
    // No filter: everything; the render is ignored unless asked for.
    let cli = parse("--nocapture --test-threads 1");
    assert_eq!(cli.shots(&all).map(|s| s.len()), Some(all.len()));
    assert!(!cli.runs_ignored());
    // nextest: `--list --format terse --ignored` lists only the render.
    let cli = parse("--list --format terse --ignored");
    assert!(cli.list && cli.terse && cli.ignored_only);
    // A filter naming shots renders only those; one naming the test renders all.
    assert_eq!(
        parse("--ignored palette-open-dark")
            .shots(&all)
            .map(|s| s.len()),
        Some(4)
    );
    assert_eq!(
        parse("--ignored render_all").shots(&all).map(|s| s.len()),
        Some(all.len())
    );
    // ## m3a-t35-shots: `--shard k/n` interleaves; the shards cover the selection once.
    let mut covered: Vec<String> = (1..=3)
        .flat_map(|k| {
            parse(&format!("--ignored --shard {k}/3"))
                .shots(&all)
                .expect("a shard")
        })
        .collect();
    covered.sort();
    let mut every = all.clone();
    every.sort();
    assert_eq!(covered, every);
    assert_eq!(
        parse("--ignored --shard=2/2 marks-run")
            .shots(&all)
            .map(|s| s.len()),
        Some(8)
    );
    assert_eq!(parse("--shard 0/2").shard, None);
    assert_eq!(parse("--shard 3/2").shard, None);
    // Fix round 1 (M6): an invalid value is kept, and `main` exits 2 on it.
    assert_eq!(parse("--shard 3/2").bad_shard.as_deref(), Some("3/2"));
    assert_eq!(parse("--shard=x").bad_shard.as_deref(), Some("x"));
    assert_eq!(parse("--shard").bad_shard.as_deref(), Some(""));
    assert_eq!(parse("--shard 2/3").bad_shard, None);
    // A dialog surface renders at both sizes.
    assert_eq!(
        parse("--ignored conn-dialog-new-dark")
            .shots(&all)
            .map(|s| s.len()),
        Some(8)
    );
    // `--exact` matches whole names (a shot with or without `.png`).
    assert_eq!(parse("--exact render_all").shots(&all), None);
    assert_eq!(
        parse("--exact --ignored shell-dark-en-1280x800@2x").shots(&all),
        Some(vec!["shell-dark-en-1280x800@2x.png".to_string()])
    );
    assert!(parse("--exact render_all_surfaces").selects_test(RENDER_TEST));
    assert!(
        !parse("--exact the_matrix")
            .selects_test("the_matrix_covers_every_surface_locale_and_theme")
    );
    // `--skip` drops tests; a filter matching nothing selects nothing.
    assert_eq!(parse("--skip render").shots(&all), None);
    assert_eq!(parse("no-such-shot").shots(&all), None);
}
