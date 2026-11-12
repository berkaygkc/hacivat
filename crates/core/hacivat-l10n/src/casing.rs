//! Display casing of human text (INV-3): the sanctioned replacement for the banned
//! `to_uppercase`. It has no M1 caller; M2's error renderers use it (F33).

use icu_casemap::CaseMapper;
use unic_langid::LanguageIdentifier;

/// Upper-cases `text` by the rules of `lang` with ICU4X `CaseMapper::uppercase`: Turkish and
/// Azerbaijani map `i` to `İ`, every language maps `ß` to `SS`.
pub fn case_for_display(text: &str, lang: &LanguageIdentifier) -> String {
    // Only the language subtag matters for case mapping; an unparsable tag maps by root rules.
    let icu_lang = icu_locale::LanguageIdentifier::try_from_str(lang.language.as_str())
        .unwrap_or(icu_locale::LanguageIdentifier::UNKNOWN);
    CaseMapper::new()
        .uppercase_to_string(text, &icu_lang)
        .into_owned()
}
