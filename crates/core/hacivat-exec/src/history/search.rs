// ## m3b-t01c-core-seams
//! History search and recall (Task 17): full text through FTS5 with the fixed Turkish pre-fold
//! (`İ`, `I`, `ı` → `i`, …; never locale casing), filters, pins.
// ## m3b-t17-history
//!
//! The search key is the stored (redacted) statement after [`fold`]; FTS5's `unicode61
//! remove_diacritics 2` tokenizer then folds case and the remaining accents. The same fold
//! runs on the user's text, which never reaches FTS5 as syntax: [`match_expr`] keeps only its
//! words (letters and digits), each a quoted prefix phrase (`"istanbul"*`), and the expression
//! is a bound parameter. Operators (`OR`, `NEAR`, `-`, `^`, `col:`) are therefore plain words.

/// The Turkish-tolerant search key (ALG-I18N-06, D-074 behaviour): a fixed table, the same for
/// every UI language and process locale. `İ`, `I` and `ı` become `i`; `Ş/ş` `s`, `Ğ/ğ` `g`,
/// `Ç/ç` `c`, `Ö/ö` `o`, `Ü/ü` `u`; other ASCII capitals their lowercase; combining marks
/// U+0300–U+036F are dropped (so a decomposed `I` + U+0307 or `s` + U+0327 folds like the
/// precomposed letter).
pub(crate) fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'İ' | 'I' | 'ı' => out.push('i'),
            'Ş' | 'ş' => out.push('s'),
            'Ğ' | 'ğ' => out.push('g'),
            'Ç' | 'ç' => out.push('c'),
            'Ö' | 'ö' => out.push('o'),
            'Ü' | 'ü' => out.push('u'),
            '\u{300}'..='\u{36F}' => {}
            c => out.push(c.to_ascii_lowercase()),
        }
    }
    out
}

/// The FTS5 expression for the user's `text`: `None` when the text has no word to look for.
/// Each word is a quoted prefix phrase; the words are ANDed.
pub(crate) fn match_expr(text: &str) -> Option<String> {
    let folded = fold(text);
    let words: Vec<&str> = folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return None;
    }
    let mut expr = String::new();
    for w in words {
        if !expr.is_empty() {
            expr.push(' ');
        }
        expr.push('"');
        expr.push_str(&w.replace('"', "\"\""));
        expr.push_str("\"*");
    }
    Some(expr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fold_table() {
        assert_eq!(fold("İSTANBUL ılık IĞDIR"), "istanbul ilik igdir");
        assert_eq!(
            fold("ŞEHİR çiğdem ÖĞRENCİ müşteri"),
            "sehir cigdem ogrenci musteri"
        );
        assert_eq!(fold("I\u{307}s\u{327}"), "is");
        // Other letters are left to FTS5 (it folds case and accents itself).
        assert_eq!(fold("ÉCOLE Ä"), "École Ä");
    }

    #[test]
    fn user_text_is_words_only() {
        assert_eq!(
            match_expr("müşteri NEAR(x"),
            Some("\"musteri\"* \"near\"* \"x\"*".into())
        );
        assert_eq!(match_expr("\"*^-()"), None);
        assert_eq!(match_expr(""), None);
        assert_eq!(match_expr("col:term"), Some("\"col\"* \"term\"*".into()));
    }
}
