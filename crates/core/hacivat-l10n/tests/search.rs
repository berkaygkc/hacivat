//! The UI search key (I18N-09, ALG-I18N-06) and when Turkish-tolerant folding is on (D-074).

use hacivat_l10n::{
    Catalogs, MatchKind, MatchMode, OsLocale, SearchOpts, build_key, effective, find, negotiate,
    turkish_tolerant_for,
};
use hacivat_settings::{LanguagePref, Settings};

fn hit(text: &str, query: &str, o: &SearchOpts) -> bool {
    find(&build_key(text, o), query, o).is_some()
}

/// The original text of the first highlight range.
fn hl(text: &str, query: &str, o: &SearchOpts) -> String {
    let m = find(&build_key(text, o), query, o).expect("a match");
    text[m.ranges[0].clone()].to_string()
}

/// The original text of every highlight range.
fn hls(text: &str, query: &str, o: &SearchOpts) -> Vec<String> {
    let m = find(&build_key(text, o), query, o).expect("a match");
    m.ranges
        .iter()
        .map(|r| text[r.clone()].to_string())
        .collect()
}

const TOL: SearchOpts = SearchOpts {
    turkish_tolerant: true,
    mode: MatchMode::Contains,
};
const OFF: SearchOpts = SearchOpts {
    turkish_tolerant: false,
    mode: MatchMode::Contains,
};

// --- The brief's rows ---

#[test]
fn search_rows() {
    let tol = SearchOpts {
        turkish_tolerant: true,
        mode: MatchMode::Contains,
    };
    let off = SearchOpts {
        turkish_tolerant: false,
        ..tol
    };
    assert!(hit("user_id", "ID", &tol));
    assert_eq!(hl("İSTANBUL_SUBE", "istanbul", &tol), "İSTANBUL");
    assert!(hit("ILIK", "ılık", &tol));
    assert!(hit("müşteri", "musteri", &tol));
    assert!(!hit("müşteri", "musteri", &off));
    assert_eq!(hl("Straße", "stras", &tol), "Straß");
    assert!(!hit("orders", "ıd", &tol));
    assert!(hit("ad\u{2067}min", "admin", &tol));
}

#[test]
fn tolerant_only_for_turkish() {
    assert!(turkish_tolerant_for("tr", "en-US"));
    assert!(turkish_tolerant_for("en", "tr-TR"));
    assert!(!turkish_tolerant_for("en", "en-US"));
}

// --- The rest of the ALG-I18N-06 table (Contains, tolerant) ---

#[test]
fn table_highlights() {
    assert_eq!(hl("user_id", "ID", &TOL), "id");
    assert_eq!(hl("ILIK", "ılık", &TOL), "ILIK");
    assert_eq!(hl("müşteri", "musteri", &TOL), "müşteri");
    assert_eq!(hl("Straße", "strasse", &TOL), "Straße");
    assert_eq!(hl("\u{FB01}le_list", "file", &TOL), "\u{FB01}le");
    assert_eq!(hl("I\u{307}zmir", "izmir", &TOL), "I\u{307}zmir");
    assert_eq!(hl("ＵＳＥＲ_ＩＤ", "user_id", &TOL), "ＵＳＥＲ_ＩＤ");
    assert_eq!(hl("ad\u{2067}min", "admin", &TOL), "ad\u{2067}min");
    assert!(!hit("", "a", &TOL));
}

#[test]
fn empty_query_matches_without_highlight() {
    let m = find(&build_key("abc", &TOL), "   ", &TOL).expect("matches all");
    assert!(m.ranges.is_empty());
    assert!(find(&build_key("", &TOL), "", &TOL).is_some());
}

#[test]
fn fuzzy_row() {
    let fz = SearchOpts {
        mode: MatchMode::Fuzzy,
        ..TOL
    };
    let m = find(&build_key("events_2026_10", &fz), "e10", &fz).expect("fuzzy hit");
    assert_eq!(m.kind, MatchKind::Fuzzy);
    let picked: Vec<&str> = m
        .ranges
        .iter()
        .map(|r| &"events_2026_10"[r.clone()])
        .collect();
    assert_eq!(picked, vec!["e", "10"]);
    // The table asks whether `events_2026_01` scores lower: it has no `1` before a `0`, so it
    // does not match at all.
    assert!(!hit("events_2026_01", "e10", &fz));
    let weaker = find(&build_key("e_x_y_1_z_0", &fz), "e10", &fz).expect("fuzzy hit");
    assert!(m.score > weaker.score, "{} vs {}", m.score, weaker.score);
    assert!(!hit("events", "e10", &fz));
}

// --- ALG-I18N-06 edge cases ---

#[test]
fn every_i_matches_every_i_when_tolerant() {
    for text in ["id", "user_id", "İD", "ıd", "Id"] {
        assert!(hit(text, "ID", &TOL), "{text}");
    }
    for text in ["İSTANBUL", "Istanbul", "ıstanbul"] {
        assert!(hit(text, "istanbul", &TOL), "{text}");
    }
    for text in ["i", "ı", "I", "İ"] {
        assert!(hit(text, "ı", &TOL), "{text}");
    }
    assert!(hit("İsim", "ISIM", &TOL) && hit("isim", "ISIM", &TOL));
}

#[test]
fn sharp_s_highlights_the_whole_letter() {
    assert_eq!(hl("Straße", "s", &TOL), "S");
    let ranges = hls("ß", "s", &TOL);
    assert_eq!(ranges, vec!["ß".to_string()]);
}

#[test]
fn accents_fold_both_ways_when_tolerant() {
    assert!(hit("musteri", "müş", &TOL));
    assert!(hit("müşteri", "müş", &TOL));
    assert!(hit("Ølberg", "olb", &TOL));
    assert!(hit("Łódź", "lodz", &TOL));
    assert!(hit("Æon", "aeon", &TOL));
    assert!(!hit("Łódź", "lodz", &OFF));
}

#[test]
fn final_sigma_and_width_fold() {
    assert!(hit("ΟΔΟΣ", "οδος", &OFF));
    assert!(hit("ＩＤ", "id", &OFF));
}

#[test]
fn not_tolerant_uses_default_folding() {
    assert!(hit("user_id", "ID", &OFF));
    // Default folding turns `İ` into `i̇`, so the Turkish-only classes stay apart.
    assert!(!hit("orders", "ıd", &OFF));
    assert!(!hit("ılık", "ilik", &OFF));
}

#[test]
fn query_whitespace_is_trimmed_and_collapsed() {
    assert_eq!(hl("page  size limit", "  page   size ", &TOL), "page  size");
    assert!(!hit("abc", "abcd", &TOL));
}

#[test]
fn emoji_and_combining_marks_stay_whole() {
    let text = "a👍🏽b";
    assert_eq!(hl(text, "👍", &TOL), "👍🏽");
    assert_eq!(hl("e\u{301}x", "ex", &TOL), "e\u{301}x");
}

#[test]
fn every_occurrence_is_highlighted_up_to_a_cap() {
    assert_eq!(hls("id_id_ID", "id", &TOL), vec!["id", "id", "ID"]);
    let long = "ab ".repeat(500);
    let m = find(&build_key(&long, &TOL), "ab", &TOL).expect("hits");
    assert_eq!(m.ranges.len(), 100);
}

#[test]
fn match_kinds_rank_exact_word_over_prefix_over_substring() {
    let k = |text: &str, q: &str| find(&build_key(text, &TOL), q, &TOL).expect("hit");
    let exact = k("user_id", "id");
    let prefix = k("identity", "id");
    let sub = k("valid", "id");
    assert_eq!(exact.kind, MatchKind::ExactWord);
    assert_eq!(prefix.kind, MatchKind::Prefix);
    assert_eq!(sub.kind, MatchKind::Substring);
    assert!(exact.score > prefix.score && prefix.score > sub.score);
    // camelCase boundaries come from the original text.
    assert_eq!(k("userId", "id").kind, MatchKind::ExactWord);
    assert_eq!(k("table2026", "2026").kind, MatchKind::ExactWord);
}

#[test]
fn word_prefix_mode() {
    let wp = SearchOpts {
        mode: MatchMode::WordPrefix,
        ..TOL
    };
    assert_eq!(hl("user_identity", "ide", &wp), "ide");
    assert!(!hit("valid", "id", &wp));
    // An acronym across word starts.
    assert_eq!(hls("user_id", "ui", &wp), vec!["u", "i"]);
    assert_eq!(hls("customerOrderLines", "col", &wp), vec!["c", "O", "L"]);
}

#[test]
fn turkish_tolerance_follows_language_and_region() {
    assert!(turkish_tolerant_for("tr", "tr-TR"));
    assert!(turkish_tolerant_for("tr-XT", "en-US"));
    assert!(turkish_tolerant_for("en", "tr_TR.UTF-8"));
    assert!(!turkish_tolerant_for("az", "en-US"));
    assert!(!turkish_tolerant_for("", ""));
}

#[test]
fn english_ui_with_a_turkish_region_is_tolerant() {
    // macOS reports "English, region Turkey" as `en-TR`; the format locale resolves to `tr-TR`.
    let mut s = Settings::default();
    s.ui.language = LanguagePref::Tag("en".into());
    let os = OsLocale {
        languages: vec!["en-TR".into()],
        ..OsLocale::default()
    };
    let neg = negotiate(
        &effective(&s, &os, false).ctx,
        Catalogs::embedded().locales(),
    );
    assert_eq!(neg.format_locale, "tr-TR");
    assert!(turkish_tolerant_for("en", &neg.format_locale));
    let os = OsLocale {
        languages: vec!["en-GB".into()],
        ..OsLocale::default()
    };
    let neg = negotiate(
        &effective(&s, &os, false).ctx,
        Catalogs::embedded().locales(),
    );
    assert!(!turkish_tolerant_for("en", &neg.format_locale));
}

#[test]
fn kinds_outrank_each_other_across_items() {
    let fz = SearchOpts {
        mode: MatchMode::Fuzzy,
        ..TOL
    };
    let q = "abcdefghijklmnopqrst";
    // A substring 300 bytes in, versus the best possible 20-character fuzzy hit (every query
    // character at a word start).
    let far = format!("{}{q}", "x".repeat(300));
    let sub = find(&build_key(&far, &fz), q, &fz).expect("substring");
    let scattered = q.chars().map(String::from).collect::<Vec<_>>().join("_");
    let fuzzy = find(&build_key(&scattered, &fz), q, &fz).expect("fuzzy");
    assert_eq!(
        (sub.kind, fuzzy.kind),
        (MatchKind::Substring, MatchKind::Fuzzy)
    );
    assert!(sub.score > fuzzy.score, "{} vs {}", sub.score, fuzzy.score);
    // The worst substring still beats the best acronym, and bands never overlap.
    let worst_sub = find(
        &build_key(&format!("{}id", "x".repeat(900)), &TOL),
        "id",
        &TOL,
    )
    .expect("substring");
    let wp = SearchOpts {
        mode: MatchMode::WordPrefix,
        ..TOL
    };
    let acronym = find(&build_key("user_id", &wp), "ui", &wp).expect("acronym");
    assert_eq!(acronym.kind, MatchKind::Fuzzy);
    assert!(worst_sub.score > acronym.score && acronym.score > fuzzy.score);
    let worst_prefix = find(
        &build_key(&format!("{} identity", "x".repeat(900)), &TOL),
        "id",
        &TOL,
    )
    .expect("prefix");
    assert_eq!(worst_prefix.kind, MatchKind::Prefix);
    let best_sub = find(&build_key("xid", &TOL), "id", &TOL).expect("substring");
    assert!(worst_prefix.score > best_sub.score);
    let worst_exact = find(
        &build_key(&format!("{} id", "x".repeat(900)), &TOL),
        "id",
        &TOL,
    )
    .expect("exact");
    let best_prefix = find(&build_key("identity", &TOL), "id", &TOL).expect("prefix");
    assert!(worst_exact.score > best_prefix.score);
}
