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
