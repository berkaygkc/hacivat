//! The SQL language service of the console editor (UI-EDIT-11 L1, -21 source, -06 token oracle;
//! R-M3-8, R-M3-22; Task 24): each committed snapshot is analysed off the UI thread into a
//! `DocSnapshot`; highlighting is rebased through the editor's change log; the Run preview uses
//! only a snapshot at exactly the editor's version and shows a range as runnable only when Run
//! sends exactly that range.
//!
//! - [`SqlLanguageService`]: the editor's `LanguageService` (`service`), its per-document
//!   worker (`worker`), the highlight source (`highlight`), statement spans (`statements`),
//!   the preview (`preview`, [`preview_of`]) and the token oracle (`words`).
//! - [`RopeChunks`]: a snapshot's rope as the lexer's chunked input.
//! - [`effective_flavor`]: the client flavor a console's text is split with (never
//!   `ClientFlavor::None` on PostgreSQL).

mod chunks;
mod highlight;
mod preview;
mod service;
mod statements;
mod words;
mod worker;

pub use chunks::RopeChunks;
pub use preview::{SYNC_SELECTION_LIMIT, points as exec_points, preview_of};
pub use service::{SqlLanguageService, effective_flavor};
/// The statement spans of an analysis (the property tests read them without the executor).
#[doc(hidden)]
pub use statements::spans as statement_spans;
