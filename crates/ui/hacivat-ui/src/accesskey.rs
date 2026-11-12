//! Access keys (I18N-23 rule 5): [`AccessKeyAssigner`] picks one key per menu or dialog label.
//!
//! Letters compare with `char::eq_ignore_ascii_case` when both are ASCII and by exact match
//! otherwise (F32): Turkish `I`/`ı` and `Ç`/`ç` are different keys, and no locale-blind casing
//! function is involved.

/// The key one label got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKey {
    /// The letter at this **byte offset** of the label (underline it).
    InLabel(usize),
    /// No usable letter in the label: the key is shown after it, as `Save (S)`.
    Suffix(char),
    /// Nothing left to offer.
    None,
}

/// Whether two keys are the same key.
pub fn same_key(a: char, b: char) -> bool {
    if a.is_ascii() && b.is_ascii() {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

/// Whether a user can type `c` as an access key: ASCII letters and digits, and cased letters of
/// other scripts (Turkish, Cyrillic, Greek). Uncased scripts (CJK) cannot, so they use a suffix.
fn is_key_letter(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric()
    } else {
        c.is_alphabetic() && (c.is_lowercase() || c.is_uppercase())
    }
}

pub struct AccessKeyAssigner;

impl AccessKeyAssigner {
    /// One key per `(label, explicit, english)` entry, in order.
    ///
    /// - `explicit` is the catalog's key for this label. It wins over automatic picks and points
    ///   at its first occurrence in the label, or becomes a suffix when the label lacks it.
    /// - Otherwise the first unclaimed letter of the label.
    /// - Otherwise `english` (the English message's key) as a suffix.
    /// - Otherwise [`AccessKey::None`].
    pub fn assign(labels: &[(&str, Option<char>, Option<char>)]) -> Vec<AccessKey> {
        let mut out = vec![AccessKey::None; labels.len()];
        let mut claimed: Vec<char> = Vec::new();
        let taken = |claimed: &[char], c: char| claimed.iter().any(|&k| same_key(k, c));

        // Explicit keys first, so an automatic pick never steals one.
        for (i, (label, explicit, _)) in labels.iter().enumerate() {
            let Some(key) = *explicit else { continue };
            if taken(&claimed, key) {
                continue;
            }
            claimed.push(key);
            out[i] = match label.char_indices().find(|&(_, c)| same_key(c, key)) {
                Some((at, _)) => AccessKey::InLabel(at),
                None => AccessKey::Suffix(key),
            };
        }

        for (i, (label, _, english)) in labels.iter().enumerate() {
            if out[i] != AccessKey::None {
                continue;
            }
            let letter = label
                .char_indices()
                .find(|&(_, c)| is_key_letter(c) && !taken(&claimed, c));
            if let Some((at, c)) = letter {
                claimed.push(c);
                out[i] = AccessKey::InLabel(at);
            } else if let Some(e) = *english
                && !taken(&claimed, e)
            {
                claimed.push(e);
                out[i] = AccessKey::Suffix(e);
            }
        }
        out
    }
}
