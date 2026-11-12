//! Pseudo-locales `en-XA` and `tr-XT` (I18N-16, ALG-I18N-03). The literal transform is always
//! compiled; rendering through a pseudo chain needs the `pseudo` feature (`PseudoMode::for_tag`
//! is the only switch).

use std::borrow::Cow;

use hacivat_l10n::{PseudoMode, transform_literal, transform_resource, wrap_message};
use unic_langid::langid;
use unicode_segmentation::UnicodeSegmentation;

fn graphemes(s: &str) -> usize {
    s.graphemes(true).count()
}

// --- The brief's rows (ALG-I18N-03 normative table, I18N-16) ---

#[test]
#[expect(
    clippy::needless_character_iteration,
    reason = "the brief's row, copied literally"
)]
fn pseudo_doubles_short_strings() {
    let s = transform_literal("Save", PseudoMode::EnXa);
    assert!(
        graphemes(&s) >= 8 && s.chars().any(|c| !c.is_ascii()),
        "{s}"
    );
}

#[test]
fn pseudo_skips_access_keys_markup_urls() {
    assert_eq!(transform_literal("S", PseudoMode::EnXa), "S");
    assert_eq!(
        transform_literal("<code>SELECT 1</code>", PseudoMode::EnXa),
        "<code>SELECT 1</code>"
    );
    assert_eq!(
        transform_literal("https://hacivat.dev", PseudoMode::EnXa),
        "https://hacivat.dev"
    );
}

#[test]
fn tr_xt_injects_turkish_letters() {
    let s = transform_literal("Find in table", PseudoMode::TrXt);
    assert!(s.chars().any(|c| "ıİşğçöü".contains(c)), "{s}");
    assert!(graphemes(&s) >= 21, "{s}");
}

#[cfg(feature = "pseudo")]
#[test]
fn pseudo_keeps_placeables_and_brackets() {
    let s = support::render_pseudo("hv-n", "{ $count } rows", 5);
    assert!(
        s.starts_with("[\u{2068}5\u{2069}") && s.ends_with(']'),
        "{s}"
    );
    assert!(!s.contains("rows"), "{s}");
}

#[cfg(feature = "pseudo")]
#[test]
fn pseudo_leaves_brand_terms() {
    assert_eq!(
        support::render_pseudo_with_terms(
            "cm-brand-hacivat-caps",
            "-brand-hacivat-caps = HACIVAT\ncm-brand-hacivat-caps = { -brand-hacivat-caps }"
        ),
        "[HACIVAT]"
    );
}

// --- ALG-I18N-03 edge cases and rows beyond the brief ---

#[test]
fn for_tag_is_the_only_switch() {
    let on = cfg!(feature = "pseudo");
    assert_eq!(
        PseudoMode::for_tag(&langid!("en-XA")),
        on.then_some(PseudoMode::EnXa)
    );
    assert_eq!(
        PseudoMode::for_tag(&langid!("tr-XT")),
        on.then_some(PseudoMode::TrXt)
    );
    assert_eq!(PseudoMode::for_tag(&langid!("tr")), None);
    assert_eq!(PseudoMode::for_tag(&langid!("en")), None);
    assert_eq!(PseudoMode::for_tag(&langid!("ar-XB")), None);
}

#[test]
fn empty_whitespace_numbers_and_punctuation_are_unchanged() {
    for s in ["", "   ", " ", "3", "%", ": ", "1.234,5"] {
        assert!(
            matches!(transform_literal(s, PseudoMode::EnXa), Cow::Borrowed(t) if t == s),
            "{s:?}"
        );
    }
}

#[test]
fn every_ascii_letter_is_replaced() {
    let s = transform_literal("Open the connection", PseudoMode::EnXa);
    assert!(!s.chars().any(|c| c.is_ascii_alphabetic()), "{s}");
    assert!(s.contains(' '), "{s}");
}

#[test]
fn length_budget_follows_the_table() {
    // (literal length, required total length): ≤10 → +100 %, 11–20 → +80 %, 21–30 → +60 %,
    // 31–50 → +40 %, 51–70 → +30 %, > 70 → +20 %.
    for (len, factor) in [
        (4, 2.0),
        (15, 1.8),
        (25, 1.6),
        (40, 1.4),
        (60, 1.3),
        (90, 1.2),
    ] {
        let src: String = "Rows of data ".chars().cycle().take(len).collect();
        let s = transform_literal(&src, PseudoMode::EnXa);
        let want = (len as f64 * factor).ceil() as usize;
        assert!(
            graphemes(&s) >= want,
            "{len}: {} < {want}: {s}",
            graphemes(&s)
        );
    }
}

#[test]
fn words_without_vowels_are_padded() {
    let s = transform_literal("Sync", PseudoMode::EnXa);
    assert!(graphemes(&s) >= 8 && s.contains('·'), "{s}");
}

#[test]
fn markup_and_entities_are_kept_byte_for_byte() {
    let s = transform_literal(
        "Read <a data-l10n-name=\"d\">docs</a>&nbsp;now",
        PseudoMode::EnXa,
    );
    assert!(s.contains("<a data-l10n-name=\"d\">"), "{s}");
    assert!(s.contains("</a>&nbsp;"), "{s}");
    assert!(
        !s.contains("Read") && !s.contains("docs") && !s.contains("now"),
        "{s}"
    );
}

#[test]
fn urls_and_code_inside_text_are_kept() {
    let s = transform_literal("Visit https://example.org now", PseudoMode::EnXa);
    assert!(s.contains(" https://example.org "), "{s}");
    assert!(!s.contains("Visit"), "{s}");
    let s = transform_literal("Run `SELECT 1` first", PseudoMode::EnXa);
    assert!(s.contains("`SELECT 1`") && !s.contains("Run"), "{s}");
}

#[test]
fn multiline_literals_keep_their_newlines() {
    let s = transform_literal("First line\nsecond line", PseudoMode::EnXa);
    assert_eq!(s.matches('\n').count(), 1, "{s}");
    assert!(!s.contains("line"), "{s}");
}

#[test]
fn non_ascii_letters_are_kept() {
    let s = transform_literal("café", PseudoMode::EnXa);
    assert!(s.contains('é'), "{s}");
}

#[test]
fn tr_xt_alternates_dotted_and_dotless_i() {
    let s = transform_literal("Insert into list", PseudoMode::TrXt);
    assert!(s.contains('İ') || s.contains('ı'), "{s}");
    assert!(s.contains('ş'), "{s}");
}

#[test]
fn wrap_message_brackets() {
    assert_eq!(wrap_message("Kaydet".into()), "[Kaydet]");
    assert_eq!(wrap_message(String::new()), "[]");
}

#[test]
fn resource_transform_reaches_variants_and_attributes_but_not_brand_terms() {
    let src = "-brand-x = Hacivat\n-plain = Table\nhv-a = Open { $n ->\n    [one] one row\n   *[other] many rows\n}\n    .title = Close\n    .accesskey = C\n";
    let mut res = match fluent_syntax::parser::parse(src.to_string()) {
        Ok(r) => r,
        Err((r, e)) => panic!("{e:?} in {r:?}"),
    };
    transform_resource(&mut res, PseudoMode::EnXa);
    let out = fluent_syntax::serializer::serialize(&res);
    assert!(out.contains("-brand-x = Hacivat"), "{out}");
    assert!(!out.contains("Table"), "{out}");
    for word in ["Open", "one row", "many rows", "Close"] {
        assert!(!out.contains(word), "{word}: {out}");
    }
    assert!(out.contains(".accesskey = C"), "{out}");
    assert!(
        out.contains("$n ->") && out.contains("[one]") && out.contains("*[other]"),
        "{out}"
    );
}

#[cfg(feature = "pseudo")]
#[test]
fn tr_xt_selects_plurals_with_turkish_rules() {
    // Task 6 note: the pseudo bundle's language is `tr-XT`; Fluent resolves its plural rules by
    // lookup to `tr`. Turkish `one` is n = 1 (also `1,0`); English `one` needs no fraction.
    let p = "{ NUMBER($count, minimumFractionDigits: 1) -> [one] tek *[other] çok }";
    let tek = format!("[{}]", support::transformed("tek", PseudoMode::TrXt));
    let cok_en = format!("[{}]", support::transformed("çok", PseudoMode::EnXa));
    assert_eq!(support::render_in("tr-XT", "hv-p", p, 1), tek);
    assert_eq!(support::render_in("en-XA", "hv-p", p, 1), cok_en);
    let p = "{ $count -> [one] one *[other] other }";
    let other = format!("[{}]", support::transformed("other", PseudoMode::TrXt));
    assert_eq!(support::render_in("tr-XT", "hv-p", p, 2), other);
}

#[cfg(feature = "pseudo")]
#[test]
fn pseudo_chain_formats_numbers_with_the_format_locale() {
    let s = support::render_pseudo("hv-n", "{ $count } rows", 1234);
    assert!(s.starts_with("[\u{2068}1.234\u{2069}"), "{s}");
}

#[cfg(feature = "pseudo")]
mod support {
    use hacivat_l10n::{
        ArgValue, Catalogs, FormatInputs, IntoFluentArgs, L10n, MsgRef, Negotiated, PseudoMode,
        Sink, build_runtime, transform_literal,
    };

    const EN_TOML: &str = "tag = \"en\"\nnative_name = \"English\"\nenglish_name = \"English\"\nshort_name = \"EN\"\ndirection = \"ltr\"\nstatus = \"stable\"\n";

    struct CountArg(u64);

    impl IntoFluentArgs for CountArg {
        fn into_args(self) -> Vec<(&'static str, ArgValue)> {
            vec![("count", ArgValue::Count(self.0))]
        }
    }

    pub fn transformed(s: &str, m: PseudoMode) -> String {
        transform_literal(s, m).into_owned()
    }

    fn l10n(pseudo: &str, en_src: &str) -> L10n {
        let cat = Catalogs::from_memory(&[("en/locale.toml", EN_TOML), ("en/common.ftl", en_src)]);
        let neg = Negotiated {
            chain: vec![pseudo.parse().unwrap(), "en".parse().unwrap()],
            format_locale: "tr-TR".into(),
            diags: Vec::new(),
        };
        L10n::new(build_runtime(
            &cat,
            &neg,
            &FormatInputs::new(&neg, "UTC"),
            1,
        ))
    }

    /// Renders `id = pattern` (an `en` message) through `[pseudo, en]`, UI sink.
    pub fn render_in(pseudo: &str, id: &'static str, pattern: &str, count: u64) -> String {
        // Select variants and the closing brace go on lines of their own in FTL.
        let pattern = if pattern.contains("->") {
            pattern
                .replace(" *[", "\n   *[")
                .replace(" [", "\n    [")
                .replace(" }", "\n}")
        } else {
            pattern.to_string()
        };
        let l = l10n(pseudo, &format!("{id} = {pattern}\n"));
        l.t_rendered(MsgRef::<CountArg>::new(id), CountArg(count), Sink::Ui)
            .text
    }

    pub fn render_pseudo(id: &'static str, pattern: &str, count: u64) -> String {
        render_in("en-XA", id, pattern, count)
    }

    pub fn render_pseudo_with_terms(id: &'static str, src: &str) -> String {
        let l = l10n("en-XA", &format!("{src}\n"));
        l.t_dynamic(id, Vec::new())
    }
}
