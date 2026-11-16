//! M3b Task 4: every reason, class and destructive-pattern message exists in English and
//! Turkish, in Task 4's block of `hacivat-core.ftl`. A test-local reader: `hacivat-l10n` may
//! not be a dependency of a `crates/sql` crate (`SqlNotRuntimeFree`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sql_safety::{CoarseClass, DestructiveKind, ReasonCode};

const BLOCK: &str = "## m3b-t04-safety";

fn locale_file(locale: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../locales")
        .join(locale)
        .join("hacivat-core.ftl")
}

/// One message of Task 4's block: its value and the comment lines above it.
struct Msg {
    value: String,
    comments: Vec<String>,
}

/// The messages of Task 4's block in `locale` (from its `##` header to the next `##` header
/// or the end of the file), by id. Panics on a duplicate id.
fn block(locale: &str) -> BTreeMap<String, Msg> {
    let text = std::fs::read_to_string(locale_file(locale)).unwrap();
    let text = text.replace("\r\n", "\n");
    let start = text
        .lines()
        .position(|l| l.trim_end() == BLOCK)
        .unwrap_or_else(|| panic!("{locale}: no `{BLOCK}` block"));
    let mut out = BTreeMap::new();
    let mut comments = Vec::new();
    for line in text.lines().skip(start + 1) {
        if line.starts_with("## ") {
            break;
        }
        if let Some(c) = line.strip_prefix('#') {
            comments.push(c.trim().to_string());
            continue;
        }
        if line.trim().is_empty() {
            comments.clear();
            continue;
        }
        if line.starts_with(' ') {
            continue; // continuation line
        }
        let (id, value) = line
            .split_once(" = ")
            .unwrap_or_else(|| panic!("{locale}: unexpected line `{line}`"));
        let msg = Msg {
            value: value.to_string(),
            comments: std::mem::take(&mut comments),
        };
        assert!(
            out.insert(id.to_string(), msg).is_none(),
            "{locale}: duplicate {id}"
        );
    }
    out
}

fn all_ids() -> Vec<&'static str> {
    let mut ids: Vec<&'static str> = ReasonCode::ALL.iter().map(|c| c.fluent_id()).collect();
    ids.extend(CoarseClass::ALL.iter().map(|c| c.fluent_id()));
    ids.extend(DestructiveKind::ALL.iter().map(|k| k.fluent_id()));
    ids
}

#[test]
fn every_reason_class_and_destructive_id_exists_in_en_and_tr() {
    let ids = all_ids();
    let unique: BTreeSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "two codes share an id");
    for locale in ["en", "tr"] {
        let msgs = block(locale);
        for id in &ids {
            let m = msgs
                .get(*id)
                .unwrap_or_else(|| panic!("{locale}: `{id}` missing"));
            assert!(!m.value.trim().is_empty(), "{locale}: `{id}` is empty");
        }
        // The block holds Task 4's ids only.
        for id in msgs.keys() {
            assert!(unique.contains(&id.as_str()), "{locale}: stray id `{id}`");
        }
    }
}

#[test]
fn ids_follow_the_family_grammar() {
    for c in ReasonCode::ALL {
        assert!(c.fluent_id().starts_with("safety-reason-"), "{c:?}");
    }
    for c in CoarseClass::ALL {
        assert!(c.fluent_id().starts_with("safety-class-"), "{c:?}");
    }
    for k in DestructiveKind::ALL {
        assert!(k.fluent_id().starts_with("safety-destructive-"), "{k:?}");
    }
    for id in all_ids() {
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "{id}"
        );
    }
}

#[test]
fn turkish_messages_await_po_review() {
    for (id, m) in block("tr") {
        assert!(
            m.comments.iter().any(|c| c == "Review: PO"),
            "tr: `{id}` lacks `# Review: PO`"
        );
    }
}

#[test]
fn destructive_messages_name_the_target_and_variables_match() {
    let en = block("en");
    let tr = block("tr");
    for k in DestructiveKind::ALL {
        let id = k.fluent_id();
        assert!(en[id].value.contains("{ $table }"), "en: {id}");
        assert!(tr[id].value.contains("{ $table }"), "tr: {id}");
    }
    // Every variable an English message uses is documented and used in Turkish too.
    for (id, m) in &en {
        for var in vars(&m.value) {
            assert!(
                m.comments
                    .iter()
                    .any(|c| c.starts_with(&format!("${var} (String)"))),
                "en: `{id}` does not document ${var}"
            );
            assert!(
                vars(&tr[id].value).contains(&var),
                "tr: `{id}` lacks ${var}"
            );
        }
        assert_eq!(vars(&m.value), vars(&tr[id].value), "{id}");
    }
}

fn vars(value: &str) -> BTreeSet<String> {
    value
        .split("{ $")
        .skip(1)
        .map(|rest| rest.split(' ').next().unwrap_or_default().to_string())
        .collect()
}
