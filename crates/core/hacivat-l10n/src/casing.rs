//! Display casing of human text (INV-3): the sanctioned replacement for the banned
//! `to_uppercase`. Body in Task 9 (ICU4X `CaseMapper::uppercase`).

use unic_langid::LanguageIdentifier;

/// Upper-cases `text` by the rules of `lang` (Turkish `i` → `İ`). Stub: the input unchanged.
pub fn case_for_display(text: &str, lang: &LanguageIdentifier) -> String {
    let _ = lang;
    text.to_string()
}
