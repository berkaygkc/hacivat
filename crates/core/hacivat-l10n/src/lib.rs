//! Fluent + ICU4X localization: catalogs, negotiation, typed message IDs, rendering with
//! per-message fallback, the atomically swapped runtime, locale-aware formatting,
//! pseudo-locales, the search key and display casing (I18N-01..07, -09, -16).
//!
//! Task 1 commits the signatures with non-panicking stubs (F31). Task 6 owns `catalog`,
//! `negotiate`, `runtime`, `render` and `args`; Task 9 owns `format`, `pseudo`, `search`,
//! `effective` and `casing`.

pub mod args;
pub mod casing;
pub mod catalog;
pub mod effective;
pub mod format;
pub mod negotiate;
pub mod pseudo;
pub mod render;
pub mod runtime;
pub mod search;

pub use args::{ArgValue, Count, ExactDecimal, IntoFluentArgs, MsgRef, NoArgs, Text, WhenValue};
pub use casing::case_for_display;
pub use catalog::{CatalogLocale, CatalogSource, Catalogs, Direction, LoadDiag, Status};
pub use effective::{EffectiveLocale, OsLocale, effective};
pub use format::{FormatCtx, install_functions, with_format_ctx};
pub use negotiate::{NegDiag, Negotiated, NegotiationCtx, PrefSource, negotiate};
pub use pseudo::{PseudoMode, transform_literal, transform_resource, wrap_message};
pub use runtime::{FormatInputs, L10n, Runtime, build_runtime};
pub use search::{
    Match, MatchKind, MatchMode, SearchKey, SearchOpts, build_key, find, turkish_tolerant_for,
};
pub use unic_langid::LanguageIdentifier;
