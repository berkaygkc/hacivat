//! UI-EDIT-13 CI gate: AltGr collisions, bare-Option chords on macOS, and per-layout
//! reachability, over every shipped preset (F12, F27).

use hacivat_ui::keymaps::{PRESETS, accepted_altgr_shadowing, layouts_altgr, load, shift_level};
use hacivat_ui_core::keymap::{
    Collision, KeyboardLayoutId, Keymap, KeymapPlatform, altgr_collisions, option_only_printables,
    unreachable_chords,
};

const ALL_PLATFORMS: [KeymapPlatform; 3] = [
    KeymapPlatform::MacOs,
    KeymapPlatform::Windows,
    KeymapPlatform::Linux,
];

/// The "hacivat" preset with one extra user chord bound to a made-up command.
fn with_extra(platform: KeymapPlatform, chord: &str) -> Keymap {
    let user = format!(
        "[[binding]]\nchord = \"{chord}\"\ncommand = \"test.extra\"\ncontext = \"Workspace\"\n"
    );
    let (km, diags) = load(platform, "hacivat", Some(&user));
    assert!(diags.is_empty(), "{diags:?}");
    km
}

#[test]
fn every_preset_has_exactly_its_accepted_altgr_collisions() {
    for preset in PRESETS {
        for p in [KeymapPlatform::Windows, KeymapPlatform::Linux] {
            assert_eq!(
                altgr_collisions(&load(p, preset, None).0, &layouts_altgr()),
                accepted_altgr_shadowing(preset),
                "{preset} {p:?}"
            );
        }
    }
}

#[test]
fn hacivat_preset_accepts_no_shadowing_and_datagrip_names_polish_s() {
    assert!(accepted_altgr_shadowing("hacivat").is_empty());
    assert!(accepted_altgr_shadowing("datagrip").contains(&Collision {
        chord: "ctrl-alt-s".into(),
        layout: "pl".into(),
        produces: 'ś'
    }));
}

#[test]
fn adding_ctrl_alt_l_fails_naming_polish_l_stroke() {
    let km = with_extra(KeymapPlatform::Windows, "ctrl-alt-l");
    let c = altgr_collisions(&km, &layouts_altgr());
    assert!(c.iter().any(|c| c.layout == "pl" && c.produces == 'ł'));
}

#[test]
fn mac_defaults_never_use_bare_option_on_printable() {
    for preset in PRESETS {
        assert!(
            option_only_printables(&load(KeymapPlatform::MacOs, preset, None).0).is_empty(),
            "{preset}"
        );
    }
}

#[test]
fn known_altgr_facts() {
    let t = layouts_altgr();
    assert_eq!(t.get("tr-q", 'q'), Some('@'));
    assert_eq!(t.get("de", 'm'), Some('µ'));
    assert_eq!(t.get("pl", 'l'), Some('ł'));
    assert_eq!(t.get("pl", 's'), Some('ś'));
}

/// Asserts the complete AltGr layer of `layout`: every listed key types exactly the listed
/// characters (sorted), and no other key in ASCII or the Turkish/Latin key caps types any.
fn assert_altgr_set(layout: &str, expected: &[(char, &str)]) {
    let t = layouts_altgr();
    let mut domain: Vec<char> = (0x20u8..0x7f)
        .map(char::from)
        .filter(|c| !c.is_ascii_uppercase())
        .collect();
    domain.extend("çşğüöıéèàê°²§".chars());
    for key in domain {
        let mut want: Vec<char> = expected
            .iter()
            .filter(|(k, _)| {
                hacivat_ui_core::keymap::normalize_char(*k)
                    == hacivat_ui_core::keymap::normalize_char(key)
            })
            .flat_map(|(_, v)| v.chars())
            .collect();
        want.sort_unstable();
        assert_eq!(
            t.get_all(layout, key, false),
            want.as_slice(),
            "{layout} key {key:?}"
        );
    }
}

/// KBDTUQ (kbdtuq.dll 10.0.25393.1), AltGr layer incl. dead keys. `ı` folds onto `i`.
#[test]
fn kbdtuq_altgr_set_is_complete() {
    assert_altgr_set(
        "tr-q",
        &[
            ('"', "<"),
            ('*', "\\"),
            ('-', "|"),
            ('0', "}"),
            ('1', ">"),
            ('2', "£"),
            ('3', "#"),
            ('4', "$"),
            ('5', "½"),
            ('7', "{"),
            ('8', "["),
            ('9', "]"),
            ('<', "|"),
            ('a', "æ"),
            ('e', "€"),
            ('q', "@"),
            ('s', "ß"),
            ('t', "₺"),
            ('ı', "i"),
            (',', "`"),
            ('ü', "~"),
            ('ğ', "¨"),
            ('ş', "´"),
        ],
    );
    // No AltGr character on 6 (the old table had 3/4 fractions there).
    assert_eq!(layouts_altgr().get("tr-q", '6'), None);
}

/// KBDTUF (kbdtuf.dll 10.0.25393.1), AltGr layer incl. dead keys. `ı` and `i` fold together.
#[test]
fn kbdtuf_altgr_set_is_complete() {
    assert_altgr_set(
        "tr-f",
        &[
            ('+', "¬"),
            (',', "\u{ad}"),
            ('-', "|"),
            ('.', "÷"),
            ('/', "\\"),
            ('0', "}"),
            ('1', "¹"),
            ('2', "²"),
            ('3', "#"),
            ('4', "¼"),
            ('5', "½"),
            ('6', "¾"),
            ('7', "{"),
            ('8', "["),
            ('9', "]"),
            ('<', "|"),
            ('b', "×"),
            ('d', "¥"),
            ('e', "€"),
            ('f', "@"),
            ('h', "ø"),
            ('i', "ß"),
            ('ı', "¶"),
            ('j', "«"),
            ('p', "£"),
            ('s', "µ"),
            ('t', "₺"),
            ('u', "æ"),
            ('v', "¢"),
            ('ö', "»"),
            ('q', "¨"),
            ('w', "~"),
            ('x', "`"),
            ('y', "´"),
        ],
    );
}

#[test]
fn datagrip_ctrl_alt_s_is_reported_on_turkish_f_too() {
    let km = load(KeymapPlatform::Windows, "datagrip", None).0;
    let c = altgr_collisions(&km, &layouts_altgr());
    assert!(c.contains(&Collision {
        chord: "ctrl-alt-s".into(),
        layout: "tr-f".into(),
        produces: 'µ'
    }));
    assert_eq!(c.len(), 3, "{c:?}");
}

#[test]
fn shift_levels_come_from_the_klc_data() {
    let level = |l: &str| shift_level(&KeyboardLayoutId(l.into()));
    assert!(level("tr-f").contains(&'>') && level("tr-f").contains(&'*'));
    assert!(level("tr-q").contains(&'>') && level("tr-q").contains(&'/'));
    assert!(level("fr").contains(&'1') && !level("fr").contains(&','));
}

#[test]
fn default_chords_are_reachable_on_every_layout() {
    for preset in PRESETS {
        for p in ALL_PLATFORMS {
            let km = load(p, preset, None).0;
            for l in ["us", "tr-q", "tr-f", "de", "fr", "pl"] {
                let l = KeyboardLayoutId(l.into());
                assert!(
                    unreachable_chords(&km, &l, &shift_level(&l)).is_empty(),
                    "{preset} {p:?} {l:?}"
                );
            }
        }
    }
}

#[test]
fn azerty_digit_without_overlay_is_unreachable() {
    let km = with_extra(KeymapPlatform::Windows, "alt-2");
    let fr = KeyboardLayoutId("fr".into());
    assert_eq!(
        unreachable_chords(&km, &fr, &shift_level(&fr)),
        vec!["alt-2".to_string()]
    );
}
