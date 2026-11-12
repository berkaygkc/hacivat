//! Pseudo-locales `en-XA` and `tr-XT` (I18N-16), built from the `en` source. Bodies in Task 9.

use std::borrow::Cow;

use fluent_syntax::ast;
use unic_langid::LanguageIdentifier;

/// Which pseudo transform applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PseudoMode {
    /// `en-XA`: accented, lengthened English.
    EnXa,
    /// `tr-XT`: Turkish letters injected, lengthened.
    TrXt,
}

impl PseudoMode {
    /// The pseudo mode for `tag` (`en-XA`, `tr-XT`); the only switch. Stub: `None`.
    pub fn for_tag(tag: &LanguageIdentifier) -> Option<PseudoMode> {
        let _ = tag;
        None
    }
}

/// Transforms one literal, skipping access keys, markup and URLs. Stub: the input unchanged.
pub fn transform_literal(s: &str, m: PseudoMode) -> Cow<'_, str> {
    let _ = m;
    Cow::Borrowed(s)
}

/// Transforms every text element of every message in `res`, leaving `-brand-*` terms untouched
/// (F6). Stub: no-op.
pub fn transform_resource(res: &mut ast::Resource<String>, m: PseudoMode) {
    let _ = (res, m);
}

/// Brackets a rendered message (`[` … `]`). Stub: the input unchanged.
pub fn wrap_message(s: String) -> String {
    s
}
