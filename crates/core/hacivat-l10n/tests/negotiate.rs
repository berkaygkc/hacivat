//! Catalog discovery and loading (I18N-02) and locale negotiation (I18N-03): the normative rows.

use std::ops::Deref;
use std::path::Path;

use hacivat_l10n::{
    CatalogLocale, Catalogs, FormatInputs, L10n, LanguageIdentifier, LoadDiag, NegDiag, Negotiated,
    NegotiationCtx, PrefSource, Status, build_runtime, negotiate,
};

const EN_TOML: &str = "tag = \"en\"\nnative_name = \"English\"\nenglish_name = \"English\"\nshort_name = \"EN\"\ndirection = \"ltr\"\nstatus = \"stable\"\n";
const TR_TOML: &str = "tag = \"tr\"\nnative_name = \"Türkçe\"\nenglish_name = \"Turkish\"\nshort_name = \"TR\"\ndirection = \"ltr\"\nstatus = \"beta\"\n";

/// An overlay directory that lives as long as the value (it derefs to its path).
struct Overlay(tempfile::TempDir);

impl Deref for Overlay {
    type Target = Path;
    fn deref(&self) -> &Path {
        self.0.path()
    }
}

fn tmp_overlay(files: &[(&str, &str)]) -> Overlay {
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in files {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    Overlay(dir)
}

fn tags(c: &Catalogs) -> Vec<&'static str> {
    c.locales()
        .iter()
        .map(|l| &*Box::leak(l.tag.to_string().into_boxed_str()))
        .collect()
}

fn locale<'a>(c: &'a Catalogs, tag: &str) -> &'a CatalogLocale {
    c.locales()
        .iter()
        .find(|l| l.tag == tag)
        .unwrap_or_else(|| panic!("no locale {tag} in {:?}", tags(c)))
}

fn status(c: &Catalogs, tag: &str) -> Status {
    locale(c, tag).status
}

fn english_name(c: &Catalogs, tag: &str) -> String {
    locale(c, tag).english_name.clone()
}

fn l10n_from(c: &Catalogs, chain: &[&str]) -> L10n {
    let n = Negotiated {
        chain: chain
            .iter()
            .map(|t| t.parse::<LanguageIdentifier>().unwrap())
            .collect(),
        format_locale: "en-US".into(),
        diags: Vec::new(),
    };
    let f = FormatInputs::new(&n, "UTC");
    L10n::new(build_runtime(c, &n, &f, 1))
}

fn set(tag: &str) -> PrefSource {
    PrefSource::Setting(tag.into())
}

fn env(var: &'static str, value: &str) -> PrefSource {
    PrefSource::Env(var, value.into())
}

fn os(tags: &[&str]) -> PrefSource {
    PrefSource::OsList(tags.iter().map(|t| t.to_string()).collect())
}

fn ctx(sources: &[PrefSource], os_format: Option<&str>, allow_pseudo: bool) -> NegotiationCtx {
    NegotiationCtx {
        sources: sources.to_vec(),
        format_pref: None,
        os_format: os_format.map(str::to_string),
        allow_experimental: false,
        allow_pseudo,
    }
}

/// Negotiates over the embedded catalogs (`en`, `tr`).
fn neg(sources: &[PrefSource], os_format: Option<&str>) -> Negotiated {
    negotiate(
        &ctx(sources, os_format, false),
        Catalogs::embedded().locales(),
    )
}

fn neg_pseudo(sources: &[PrefSource]) -> Negotiated {
    negotiate(&ctx(sources, None, true), Catalogs::embedded().locales())
}

fn chain(n: &Negotiated) -> Vec<String> {
    n.chain.iter().map(|t| t.to_string()).collect()
}

/// Negotiates over `en` plus stable catalogs named `extra`.
fn chain_with(extra: &[&str], sources: &[PrefSource]) -> Vec<String> {
    let mut files = vec![("en/locale.toml".to_string(), EN_TOML.to_string())];
    for tag in extra {
        files.push((
            format!("{tag}/locale.toml"),
            format!("tag = \"{tag}\"\nnative_name = \"{tag}\"\nenglish_name = \"{tag}\"\nshort_name = \"{tag}\"\ndirection = \"ltr\"\nstatus = \"stable\"\n"),
        ));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let c = Catalogs::from_memory(&refs);
    chain(&negotiate(&ctx(sources, None, false), c.locales()))
}

// I18N-02

#[test]
fn embedded_has_en_stable_and_tr_beta() {
    let c = Catalogs::embedded();
    assert_eq!(tags(&c), ["en", "tr"]);
    assert_eq!(status(&c, "en"), Status::Stable);
    assert_eq!(status(&c, "tr"), Status::Beta);
}

#[test]
fn overlay_without_manifest_is_experimental() {
    let c = Catalogs::embedded().with_overlay(&tmp_overlay(&[("de/hacivat-ui.ftl", "hv-x = X")]));
    assert_eq!(status(&c, "de"), Status::Experimental);
    assert_eq!(english_name(&c, "de"), "de");
}

#[test]
fn overlay_overrides_per_id() {
    let c = Catalogs::from_memory(&[
        ("en/locale.toml", EN_TOML),
        ("tr/locale.toml", TR_TOML),
        ("en/hacivat-ui.ftl", "hv-a = A\nhv-b = B"),
        ("tr/hacivat-ui.ftl", "hv-a = A-tr\nhv-b = B-tr"),
    ])
    .with_overlay(&tmp_overlay(&[("tr/hacivat-ui.ftl", "hv-a = A-overlay")]));
    let l = l10n_from(&c, &["tr", "en"]);
    assert_eq!(l.t_dynamic("hv-a", vec![]), "A-overlay");
    assert_eq!(l.t_dynamic("hv-b", vec![]), "B-tr");
}

#[test]
fn junk_line_keeps_other_messages() {
    let c = Catalogs::from_memory(&[
        ("en/locale.toml", EN_TOML),
        (
            "en/hacivat-ui.ftl",
            "hv-a = A\n\n\n\n\n\n!!junk\nhv-b = B\n",
        ),
    ]);
    assert!(c.diags().contains(&LoadDiag::Junk {
        file: "en/hacivat-ui.ftl".into(),
        line: 7
    }));
    let l = l10n_from(&c, &["en"]);
    assert_eq!(
        (l.t_dynamic("hv-a", vec![]), l.t_dynamic("hv-b", vec![])),
        ("A".into(), "B".into())
    );
}

#[test]
fn pseudo_folder_on_disk_is_rejected() {
    let c =
        Catalogs::embedded().with_overlay(&tmp_overlay(&[("en-XA/hacivat-ui.ftl", "hv-a = A")]));
    assert!(c.diags().contains(&LoadDiag::ReservedPseudo {
        folder: "en-XA".into()
    }));
    assert!(!tags(&c).contains(&"en-XA"));
}

// I18N-02 step 5: overlay safety limits.

#[test]
fn overlay_skips_unsafe_files() {
    let big = format!("hv-big = {}\n", "x".repeat(1024 * 1024));
    let dir = tmp_overlay(&[
        ("tr/notes.txt", "not a catalog"),
        ("tr/big.ftl", &big),
        ("tr/hacivat-ui.ftl", "hv-a = A-overlay"),
    ]);
    std::fs::write(dir.join("tr/latin1.ftl"), b"hv-l = \xfe\n").unwrap();
    let c = Catalogs::embedded().with_overlay(&dir);
    let skipped: Vec<String> = c
        .diags()
        .iter()
        .filter_map(|d| match d {
            LoadDiag::OverlaySkipped { path, .. } => {
                Some(path.file_name().unwrap().to_string_lossy().into_owned())
            }
            _ => None,
        })
        .collect();
    let mut skipped = skipped;
    skipped.sort();
    assert_eq!(skipped, ["big.ftl", "latin1.ftl", "notes.txt"]);
    let l = l10n_from(&c, &["tr", "en"]);
    assert_eq!(l.t_dynamic("hv-a", vec![]), "A-overlay");
}

#[cfg(unix)]
#[test]
fn overlay_symlink_leaving_the_root_is_skipped() {
    let outside = tmp_overlay(&[("secret.ftl", "hv-a = leaked")]);
    let dir = tmp_overlay(&[("tr/hacivat-ui.ftl", "hv-b = B-overlay")]);
    std::os::unix::fs::symlink(outside.join("secret.ftl"), dir.join("tr/secret.ftl")).unwrap();
    let c = Catalogs::embedded().with_overlay(&dir);
    assert!(c.diags().iter().any(|d| matches!(
        d,
        LoadDiag::OverlaySkipped { path, .. } if path.ends_with("tr/secret.ftl")
    )));
    let l = l10n_from(&c, &["tr", "en"]);
    assert_eq!(l.t_dynamic("hv-b", vec![]), "B-overlay");
    assert_ne!(l.t_dynamic("hv-a", vec![]), "leaked");
}

#[test]
fn non_canonical_folder_is_rejected() {
    let c =
        Catalogs::embedded().with_overlay(&tmp_overlay(&[("tr_TR/hacivat-ui.ftl", "hv-a = A")]));
    assert!(c.diags().contains(&LoadDiag::NonCanonicalTag {
        folder: "tr_TR".into()
    }));
    assert_eq!(tags(&c), ["en", "tr"]);
}

#[test]
fn locale_max_has_likely_subtags() {
    let c = Catalogs::embedded();
    assert_eq!(locale(&c, "tr").max.to_string(), "tr-Latn-TR");
    assert_eq!(locale(&c, "tr").native_name, "Türkçe");
    assert_eq!(locale(&c, "tr").short_name, "TR");
}

// I18N-03

#[test]
fn os_tr_tr_gives_tr_chain() {
    let n = neg(&[os(&["tr-TR", "en-US"])], None);
    assert_eq!(chain(&n), ["tr", "en"]);
    assert_eq!(n.format_locale, "tr-TR");
}

#[test]
fn english_setting_keeps_turkish_region() {
    let n = neg(&[set("en"), os(&["tr-TR"])], Some("tr-TR"));
    assert_eq!(chain(&n), ["en"]);
    assert_eq!(n.format_locale, "tr-TR");
}

#[test]
fn lang_c_is_english_with_diag() {
    let n = neg(&[env("LANG", "C")], None);
    assert_eq!(chain(&n), ["en"]);
    assert_eq!(n.diags, vec![NegDiag::CLocale { var: "LANG".into() }]);
}

#[test]
fn codeset_and_modifier_are_stripped() {
    assert_eq!(
        chain(&neg(&[env("LANG", "tr_TR.ISO-8859-9")], None)),
        ["tr", "en"]
    );
    assert_eq!(
        chain(&neg(&[env("LANG", "tr_TR@euro")], None)),
        ["tr", "en"]
    );
}

#[test]
fn never_across_languages() {
    assert_eq!(chain(&neg(&[set("az")], None)), ["en"]);
}

#[test]
fn numbering_extension_kept_in_format_only() {
    let n = neg(&[os(&["tr-TR-u-nu-arab"])], None);
    assert_eq!(chain(&n), ["tr", "en"]);
    assert_eq!(n.format_locale, "tr-TR-u-nu-arab");
}

#[test]
fn zh_hk_picks_hant() {
    assert_eq!(
        chain_with(&["zh-Hans", "zh-Hant"], &[os(&["zh-HK"])]),
        ["zh-Hant", "en"]
    );
}

#[test]
fn pseudo_needs_permission() {
    assert_eq!(chain(&neg(&[set("en-XA")], None)), ["en"]);
    assert_eq!(chain(&neg_pseudo(&[set("en-XA")])), ["en-XA", "en"]);
}

// I18N-03: further rows of the ALG-I18N-01 table.

#[test]
fn az_has_no_match_and_formats_as_en_us() {
    let n = neg(&[set("az")], None);
    assert_eq!(n.format_locale, "en-US");
    assert_eq!(
        n.diags,
        vec![NegDiag::NoMatch {
            requested: "az".into()
        }]
    );
}

#[test]
fn language_list_is_split_and_deduped() {
    let n = neg(&[env("LANGUAGE", "tr:en_US:tr_TR")], None);
    assert_eq!(chain(&n), ["tr", "en"]);
}

#[test]
fn regional_sibling_matches_the_generic_catalog() {
    assert_eq!(chain(&neg(&[os(&["en-GB"])], None)), ["en"]);
    assert_eq!(chain(&neg(&[os(&["tr-CY"])], None)), ["tr", "en"]);
}

#[test]
fn unparsable_tag_is_reported() {
    let n = neg(&[set("not a tag")], None);
    assert_eq!(chain(&n), ["en"]);
    assert_eq!(
        n.diags,
        vec![NegDiag::Unparsable {
            source: "setting".into(),
            value: "not a tag".into()
        }]
    );
}

#[test]
fn experimental_needs_an_explicit_choice() {
    let c = Catalogs::embedded().with_overlay(&tmp_overlay(&[("de/hacivat-ui.ftl", "hv-x = X")]));
    let from_os = negotiate(&ctx(&[os(&["de-DE"])], None, false), c.locales());
    assert_eq!(chain(&from_os), ["en"]);
    assert_eq!(from_os.format_locale, "de-DE");
    let chosen = negotiate(&ctx(&[set("de")], None, false), c.locales());
    assert_eq!(chain(&chosen), ["de", "en"]);
}

#[test]
fn regional_catalog_gets_its_language_parent() {
    assert_eq!(
        chain_with(&["pt", "pt-BR"], &[os(&["pt-BR"])]),
        ["pt-BR", "pt", "en"]
    );
}

#[test]
fn explicit_regional_format_wins() {
    let mut c = ctx(&[os(&["en-US"])], Some("en-US"), false);
    c.format_pref = Some("tr_TR".into());
    assert_eq!(
        negotiate(&c, Catalogs::embedded().locales()).format_locale,
        "tr-TR"
    );
}
