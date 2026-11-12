//! Locale-aware `NUMBER`, `DATETIME` and `LIST` (I18N-05). Bodies in Task 9.

use std::borrow::Borrow;

use fluent_bundle::FluentResource;
use fluent_bundle::bundle::FluentBundle;
use fluent_bundle::memoizer::MemoizerKind;

/// The format locale, time zone and generation the formatters read while a message renders.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct FormatCtx {
    pub format_locale: String,
    pub time_zone: String,
    pub generation: u64,
}

/// Runs `f` with `ctx` as the current format context (re-entrant). Stub: just calls `f`.
pub fn with_format_ctx<R>(ctx: &FormatCtx, f: impl FnOnce() -> R) -> R {
    let _ = ctx;
    f()
}

/// Adds `NUMBER`, `DATETIME`, `LIST` and the `hv_format` formatter to `bundle`. Stub: no-op.
pub fn install_functions<R, M>(bundle: &mut FluentBundle<R, M>)
where
    R: Borrow<FluentResource>,
    M: MemoizerKind,
{
    let _ = bundle;
}
