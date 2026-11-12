use hacivat_ui::accesskey::{AccessKey, AccessKeyAssigner};

/// The key letter each label got, read back out of the label (a suffix key reads as itself).
fn keys(labels: &[(&str, Option<char>, Option<char>)]) -> Vec<char> {
    AccessKeyAssigner::assign(labels)
        .into_iter()
        .zip(labels)
        .map(|(key, (label, _, _))| match key {
            AccessKey::InLabel(at) => label[at..].chars().next().unwrap(),
            AccessKey::Suffix(c) => c,
            AccessKey::None => panic!("no key for {label}"),
        })
        .collect()
}

#[test]
fn access_keys_turkish_menu() {
    assert_eq!(
        keys(&[
            ("Kaydet", None, None),
            ("Kapat", None, None),
            ("Kopyala", None, None)
        ]),
        ['K', 'a', 'o']
    );
    assert_eq!(
        keys(&[("Kaydet", None, None), ("Kopyala", Some('p'), None)]),
        ['K', 'p']
    );
}

#[test]
fn access_keys_ascii_ignore_case_non_ascii_exact() {
    assert_eq!(
        keys(&[("Save", None, None), ("save as", None, None)]),
        ['S', 'a']
    );
    assert_eq!(
        keys(&[("Işık", None, None), ("ışık", None, None)]),
        ['I', 'ı']
    );
    assert_eq!(
        keys(&[("Çık", None, None), ("çiz", None, None)]),
        ['Ç', 'ç']
    );
}

#[test]
fn access_key_suffix_when_no_letters() {
    assert_eq!(
        AccessKeyAssigner::assign(&[("保存", None, Some('S'))]),
        vec![AccessKey::Suffix('S')]
    );
}

#[test]
fn access_key_none_when_nothing_is_left() {
    assert_eq!(
        AccessKeyAssigner::assign(&[("aa", None, None), ("a", None, None)]),
        vec![AccessKey::InLabel(0), AccessKey::None]
    );
}

#[test]
fn explicit_key_found_in_the_label_points_at_it() {
    assert_eq!(
        AccessKeyAssigner::assign(&[("Copy", Some('P'), None)]),
        vec![AccessKey::InLabel(2)]
    );
    // An explicit key the label does not contain is shown as a suffix.
    assert_eq!(
        AccessKeyAssigner::assign(&[("保存", Some('S'), None)]),
        vec![AccessKey::Suffix('S')]
    );
}
