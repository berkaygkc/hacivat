//! The SQL language service of the console editor (UI-EDIT-11 L1, -21 source, -06 token oracle;
//! R-M3-8, R-M3-22; Task 24): each committed snapshot is analysed off the UI thread into a
//! `DocSnapshot`; highlighting is rebased through the editor's change log; the Run preview uses
//! only a snapshot at exactly the editor's version.
//!
//! Task 1b stub: [`SqlLanguageService`] behaves like `hacivat_ui_editor::FailClosedService` (no
//! statements, no decorations, an `Uncertain` preview with no range, no `DocSnapshot`).

mod chunks;
mod highlight;
mod preview;
mod service;
mod statements;
mod words;
mod worker;

pub use service::SqlLanguageService;
