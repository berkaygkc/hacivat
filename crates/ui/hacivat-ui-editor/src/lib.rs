//! The SQL console's editor element (UI-EDIT-01 view, -07 paint, -08 scroll, -09 layout cache,
//! -11 decorations, -12 IME, -14 clipboard, -19 marks, -21 preview, -23 tiers, -24 a11y).
//! Database-agnostic: SQL arrives through a [`LanguageService`] (the app's
//! `hacivat-sql-editor`). Reaches GPUI only through `hacivat_ui_core::gpui_surface`.
//!
//! Types and seams complete since M3a Task 1b (P11). `EditorView` holds one state struct per
//! module (`marks::MarkStore`, `preview::BandState`, `a11y::A11yState`,
//! `large_file::TierState`, `clipboard::PasteState`); its public methods delegate to the
//! module functions in one line. Task 19 owns the view, the element and every call site (all
//! wired since Task 1b's fix round 1); Tasks 25, 27, 28 and 29 own only their modules' bodies. Task 1b stubs paint nothing; the
//! input handler inserts plain text through the model; [`FailClosedService`] never previews a
//! range.

pub mod a11y;
pub mod clipboard;
pub mod commands;
mod config;
mod decor;
pub mod element;
pub mod gutter;
mod ime;
pub mod large_file;
mod layout_cache;
pub mod marks;
mod mouse;
mod paint;
pub mod preview;
mod probe;
mod scroll;
pub mod scrollbar;
mod service;
mod theme;
mod view;

pub use commands::{CommandState, EditorCommand, commands};
pub use config::{CaretBlink, DocContext, DocId, EDITOR_CONTEXT, EditorConfig, Tier, TierLimits};
pub use decor::{HighlightClass, STYLES as HIGHLIGHT_STYLES};
pub use element::{FrameLayout, VisibleRows};
pub use gutter::GutterCtx;
pub use marks::{ExecMarkState, MarkKey, RowsOutcome};
pub use preview::BandState;
pub use probe::FrameProbe;
pub use scrollbar::ScrollbarCtx;
pub use service::{
    Confidence, DecorationSource, ExecAction, ExecAt, ExecPreview, ExecRequest, FailClosedService,
    InvalidateFn, LanguageService, NoTokens, PreviewState, SourceId, StatementSpan,
};
pub use view::{EditorEvent, EditorStatus, EditorView, FocusedEditor};
