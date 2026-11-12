//! Searching the settings page (I18N-09): a row matches when the query is found in its title in
//! the UI language or in its English title, so a Turkish user finds "Theme" by typing `theme` and
//! an English user finds it in a Turkish UI (ALG-L10N-07).

use hacivat_l10n::{L10n, MatchMode, SearchOpts, build_key, find, turkish_tolerant_for};

use super::rows::SettingRow;

/// A row's searchable texts: its title in the UI language and in English.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRow {
    pub key: &'static str,
    pub local: String,
    pub english: String,
}

impl SearchRow {
    /// The texts of `row` under `l10n` (the English layer is `L10n::t_english`).
    pub fn of(row: &SettingRow, l10n: &L10n) -> SearchRow {
        SearchRow {
            key: row.key,
            local: l10n.t_machine(row.title, hacivat_l10n::NoArgs),
            english: l10n.t_english(row.title.id),
        }
    }
}

/// The search options for the installed localization: `Contains`, with Turkish-tolerant folding
/// when the UI language or the format locale is Turkish.
pub fn search_opts(l10n: &L10n) -> SearchOpts {
    let rt = l10n.snapshot();
    let ui = rt
        .ui_chain
        .first()
        .map(ToString::to_string)
        .unwrap_or_default();
    SearchOpts {
        turkish_tolerant: turkish_tolerant_for(&ui, &rt.format.format_locale),
        mode: MatchMode::Contains,
    }
}

/// The keys of the rows matching `query`, in row order. An empty query (or only spaces) matches
/// every row.
pub fn filter_rows(query: &str, rows: &[SearchRow], opts: &SearchOpts) -> Vec<&'static str> {
    let query = query.trim();
    rows.iter()
        .filter(|row| {
            query.is_empty()
                || [&row.local, &row.english]
                    .into_iter()
                    .any(|text| find(&build_key(text, opts), query, opts).is_some())
        })
        .map(|row| row.key)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &'static str, local: &str, english: &str) -> SearchRow {
        SearchRow {
            key,
            local: local.to_string(),
            english: english.to_string(),
        }
    }

    fn tolerant() -> SearchOpts {
        SearchOpts {
            turkish_tolerant: true,
            mode: MatchMode::Contains,
        }
    }

    #[test]
    fn an_empty_query_keeps_every_row_and_a_match_in_either_layer_keeps_one() {
        let rows = [
            row("a", "Dil", "Language"),
            row("b", "Tema", "Theme"),
            row("c", "Isıtıcı", "Heater"),
        ];
        assert_eq!(filter_rows("  ", &rows, &tolerant()), ["a", "b", "c"]);
        assert_eq!(filter_rows("lang", &rows, &tolerant()), ["a"]);
        assert_eq!(filter_rows("tema", &rows, &tolerant()), ["b"]);
        // Turkish-tolerant folding: dotless and dotted i are the same letter.
        assert_eq!(filter_rows("isitici", &rows, &tolerant()), ["c"]);
        assert_eq!(filter_rows("zzz", &rows, &tolerant()), Vec::<&str>::new());
    }
}
