//! The editor's configuration and document context (the app maps `[editor]` settings here).

/// The GPUI key context of a focused editor (keymap rows `context = "Editor"`).
pub const EDITOR_CONTEXT: &str = "Editor";

/// One editor document's ID (the app numbers consoles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocId(pub u64);

/// How the caret blinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CaretBlink {
    /// The platform's half-period; static under reduced motion and after 10 s idle.
    #[default]
    System,
    Off,
}

/// The large-file tier limits (UI-EDIT-23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TierLimits {
    /// Full features up to this size (4 MB).
    pub full_max_bytes: u64,
    /// Reduced mode up to this size (20 MB); nothing larger enters the editor in M3a
    /// (R-M3-49).
    pub reduced_max_bytes: u64,
    // ## m4b-t11-editor (named items): the T2 bounds (UI-EDIT-23 step 1, UI-EDIT-14 step 5).
    /// A T2 preview shows at most this much of its file (2.5 MB, DataGrip's
    /// `idea.max.intellisense.filesize`), cut at a line break (`large_file::preview_cut`).
    pub preview_bytes: u64,
    /// A paste that makes the document larger than `reduced_max_bytes` but at most this
    /// (100 MB) asks the host first, when the host takes the question
    /// (`EditorConfig::confirm_large_paste`); anything larger is refused.
    pub paste_max_bytes: u64,
}

impl Default for TierLimits {
    fn default() -> Self {
        TierLimits {
            full_max_bytes: 4 << 20,
            reduced_max_bytes: 20 << 20,
            // ## m4b-t11-editor
            preview_bytes: 2_621_440,
            paste_max_bytes: 100 << 20,
        }
    }
}

/// What an editor does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditorConfig {
    pub tab_size: u8,
    pub insert_spaces: bool,
    /// Copy and cut take the whole line without a selection.
    pub copy_line_without_selection: bool,
    pub caret_blink: CaretBlink,
    pub reduced_motion: bool,
    pub tiers: TierLimits,
    /// Lines around the caret in the screen-reader window (D-073: 100).
    pub a11y_window_lines: u32,
    pub a11y_optimized: bool,
    pub read_only: bool,
    // ## m4b-t11-editor (named item)
    /// The host answers `EditorEvent::PasteNeedsConfirm` (its dialog, then
    /// `EditorView::confirm_paste`): a paste that makes the document 20–100 MB asks instead of
    /// being refused. Off by default, so a host that shows no dialog keeps M3's refusal past
    /// 20 MB (R-M3-49) and no paste waits for an answer that never comes.
    pub confirm_large_paste: bool,
}

impl Default for EditorConfig {
    /// 4 spaces per tab, line-wise copy, the system caret blink, the default tiers, a 100-line
    /// screen-reader window, editable, large pastes refused past 20 MB.
    fn default() -> Self {
        EditorConfig {
            tab_size: 4,
            insert_spaces: true,
            copy_line_without_selection: true,
            caret_blink: CaretBlink::System,
            reduced_motion: false,
            tiers: TierLimits::default(),
            a11y_window_lines: 100,
            a11y_optimized: false,
            read_only: false,
            confirm_large_paste: false,
        }
    }
}

/// The large-file tier of a document (UI-EDIT-23; M3a has T0 and T1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tier {
    #[default]
    Full,
    /// "Reduced mode".
    Reduced,
    /// The M4b T2 preview (`EditorView::set_preview`): read-only, the T2 banner; it never
    /// moves to another tier (the host reopens the file instead).
    Preview,
}

/// What the language service needs to know about a document besides its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DocContext {
    /// A connection is bound and connected (else the preview is `NoConnection`).
    pub connected: bool,
    pub tier: Tier,
}
