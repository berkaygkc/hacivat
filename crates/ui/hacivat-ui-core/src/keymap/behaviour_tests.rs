//! Keymap behaviour tests (UI-EDIT-13). Fixtures are inline TOML; the shipped data is tested in
//! `hacivat-ui` (`keymaps` unit tests and `tests/keymap_collisions.rs`).

use super::*;
use crate::gpui_surface::{Keystroke, Modifiers};

fn m(ctrl: bool, alt: bool, shift: bool, cmd: bool) -> Mods {
    Mods {
        ctrl,
        alt,
        shift,
        cmd,
    }
}
fn ctrl() -> Mods {
    m(true, false, false, false)
}
fn ctrl_shift() -> Mods {
    m(true, false, true, false)
}
fn ctrl_alt() -> Mods {
    m(true, true, false, false)
}
fn cmd() -> Mods {
    m(false, false, false, true)
}
/// A raw press: the key character is NOT pre-normalized, `resolve` must do it.
fn kp(mods: Mods, c: char) -> KeyPress {
    KeyPress {
        mods,
        key: NormKey::Char(c),
        typed: None,
    }
}
fn kp_typed(mods: Mods, c: char, typed: char) -> KeyPress {
    KeyPress {
        mods,
        key: NormKey::Char(c),
        typed: Some(typed),
    }
}
fn kpn(mods: Mods, k: NamedKey) -> KeyPress {
    KeyPress {
        mods,
        key: NormKey::Named(k),
        typed: None,
    }
}
fn ctx(names: &[&'static str]) -> ContextStack {
    ContextStack(names.to_vec())
}
fn ws() -> ContextStack {
    ctx(&["Workspace"])
}
fn layout(id: &str) -> KeyboardLayoutId {
    KeyboardLayoutId(id.into())
}
fn us() -> KeyboardLayoutId {
    layout("us")
}
fn tr_q() -> KeyboardLayoutId {
    layout("tr-q")
}
fn fixture_on(p: KeymapPlatform, toml: &str) -> Keymap {
    let (km, diags) = Keymap::from_sources(p, toml, "", &[], None);
    assert!(diags.is_empty(), "{diags:?}");
    km
}
fn fixture(toml: &str) -> Keymap {
    fixture_on(KeymapPlatform::Windows, toml)
}
fn entry(chord: &str, command: &str) -> String {
    format!("[[binding]]\nchord=\"{chord}\"\ncommand=\"{command}\"\ncontext=\"Workspace\"\n")
}
fn fixture_chord(chord: &str, command: &str) -> Keymap {
    fixture(&entry(chord, command))
}
fn fixture_with_overlay(chord: &str, layout_id: &str, alt: &str) -> Keymap {
    let id = layout(layout_id);
    let (km, d) = Keymap::from_sources(
        KeymapPlatform::Windows,
        &entry(chord, "t.c"),
        "",
        &[(&id, &entry(alt, "t.c"))],
        None,
    );
    assert!(d.is_empty(), "{d:?}");
    km
}
fn defaults(p: KeymapPlatform) -> Keymap {
    let chord = if p == KeymapPlatform::MacOs {
        "cmd-k"
    } else {
        "ctrl-k"
    };
    fixture_on(p, &entry(chord, "workspace.command_palette"))
}
const DUP: &str = "[[binding]]\nchord=\"ctrl-k\"\ncommand=\"a\"\ncontext=\"Workspace\"\n[[binding]]\nchord=\"ctrl-k\"\ncommand=\"b\"\ncontext=\"Workspace\"\n";

#[test]
fn tr_q_dotless_i_matches_ctrl_i() {
    let km = fixture("[[binding]]\nchord=\"ctrl-i\"\ncommand=\"t.x\"\ncontext=\"Workspace\"");
    assert_eq!(
        km.resolve(&ws(), &[], &kp(ctrl(), 'ı'), &tr_q(), false),
        Resolution::Command("t.x".into())
    );
}

#[test]
fn shift_dotless_i_gives_capital_i_matches_ctrl_shift_i() {
    assert_eq!(normalize_char('I'), 'i');
    assert_eq!(normalize_char('İ'), 'i');
    let km = fixture("[[binding]]\nchord=\"ctrl-shift-i\"\ncommand=\"t.y\"\ncontext=\"Workspace\"");
    assert_eq!(
        km.resolve(&ws(), &[], &kp(ctrl_shift(), 'I'), &tr_q(), false),
        Resolution::Command("t.y".into())
    );
    assert_eq!(
        km.resolve(&ws(), &[], &kp(ctrl_shift(), 'İ'), &tr_q(), false),
        Resolution::Command("t.y".into())
    );
}

#[test]
fn altgr_char_passes_through() {
    let km = fixture_on(
        KeymapPlatform::Windows,
        "[[binding]]\nchord=\"ctrl-alt-q\"\ncommand=\"t.q\"\ncontext=\"Workspace\"",
    );
    assert_eq!(
        km.resolve(&ws(), &[], &kp_typed(ctrl_alt(), 'q', '@'), &tr_q(), false),
        Resolution::PassThrough
    );
    assert_eq!(
        km.resolve(&ws(), &[], &kp(ctrl_alt(), 'q'), &us(), false),
        Resolution::Command("t.q".into())
    );
}

#[test]
fn altgr_rule_does_not_apply_on_macos() {
    let km = fixture_on(
        KeymapPlatform::MacOs,
        "[[binding]]\nchord=\"ctrl-alt-q\"\ncommand=\"t.q\"\ncontext=\"Workspace\"",
    );
    assert_eq!(
        km.resolve(&ws(), &[], &kp_typed(ctrl_alt(), 'q', 'œ'), &us(), false),
        Resolution::Command("t.q".into())
    );
}

#[test]
fn dead_key_never_matches() {
    assert_eq!(
        normalize(&Keystroke {
            modifiers: Modifiers::none(),
            key: "dead_circumflex".into(),
            key_char: None
        }),
        None
    );
}

#[test]
fn normalize_keeps_chords_whose_key_char_is_absent() {
    // GPUI reports `key_char: None` for cmd-s and ctrl-k chords; those are not dead keys.
    let k = Keystroke {
        modifiers: Modifiers {
            platform: true,
            ..Modifiers::none()
        },
        key: "k".into(),
        key_char: None,
    };
    assert_eq!(normalize(&k), Some(kp(cmd(), 'k')));
}

#[test]
fn normalize_maps_named_keys_typed_chars_and_dotless_i() {
    let tab = Keystroke {
        modifiers: Modifiers {
            control: true,
            ..Modifiers::none()
        },
        key: "tab".into(),
        key_char: None,
    };
    assert_eq!(normalize(&tab), Some(kpn(ctrl(), NamedKey::Tab)));
    let f4 = Keystroke {
        modifiers: Modifiers::none(),
        key: "f4".into(),
        key_char: None,
    };
    assert_eq!(normalize(&f4), Some(kpn(Mods::default(), NamedKey::F(4))));
    let altgr_q = Keystroke {
        modifiers: Modifiers {
            control: true,
            alt: true,
            ..Modifiers::none()
        },
        key: "q".into(),
        key_char: Some("@".into()),
    };
    let p = normalize(&altgr_q).unwrap();
    assert_eq!((p.key, p.typed), (NormKey::Char('q'), Some('@')));
    let plain = Keystroke {
        modifiers: Modifiers::none(),
        key: "ı".into(),
        key_char: Some("ı".into()),
    };
    let p = normalize(&plain).unwrap();
    assert_eq!((p.key, p.typed), (NormKey::Char('i'), None));
}

#[test]
fn composing_blocks_bindings() {
    let km = defaults(KeymapPlatform::MacOs);
    assert_eq!(
        km.resolve(&ws(), &[], &kp(cmd(), 'k'), &us(), true),
        Resolution::PassThrough
    );
    assert_eq!(
        km.resolve(&ws(), &[], &kp(cmd(), 'k'), &us(), false),
        Resolution::Command("workspace.command_palette".into())
    );
}

#[test]
fn during_composition_binding_fires_while_composing() {
    let km = fixture(
        "[[binding]]\nchord=\"escape\"\ncommand=\"t.esc\"\ncontext=\"Workspace\"\nduring_composition=true",
    );
    assert_eq!(
        km.resolve(
            &ws(),
            &[],
            &kpn(Mods::default(), NamedKey::Escape),
            &us(),
            true
        ),
        Resolution::Command("t.esc".into())
    );
}

#[test]
fn two_key_chord_is_pending_then_fires() {
    let km = fixture_chord("ctrl-k ctrl-s", "t.save");
    assert_eq!(
        km.resolve(&ws(), &[], &kp(ctrl(), 'k'), &us(), false),
        Resolution::Pending
    );
    assert_eq!(
        km.resolve(&ws(), &[kp(ctrl(), 'k')], &kp(ctrl(), 's'), &us(), false),
        Resolution::Command("t.save".into())
    );
    assert_eq!(
        km.resolve(&ws(), &[kp(ctrl(), 'k')], &kp(ctrl(), 'x'), &us(), false),
        Resolution::PassThrough
    );
}

#[test]
fn innermost_context_wins() {
    let km = fixture(
        "[[binding]]\nchord=\"escape\"\ncommand=\"ws.esc\"\ncontext=\"Workspace\"\n[[binding]]\nchord=\"escape\"\ncommand=\"pal.esc\"\ncontext=\"Palette\"",
    );
    assert_eq!(
        km.resolve(
            &ctx(&["Workspace", "Palette"]),
            &[],
            &kpn(Mods::default(), NamedKey::Escape),
            &us(),
            false
        ),
        Resolution::Command("pal.esc".into())
    );
    assert_eq!(
        km.resolve(
            &ctx(&["Workspace"]),
            &[],
            &kpn(Mods::default(), NamedKey::Escape),
            &us(),
            false
        ),
        Resolution::Command("ws.esc".into())
    );
}

#[test]
fn tr_q_overlay_displays_shift_7_for_slash() {
    let km = fixture_with_overlay("ctrl-/", "tr-q", "ctrl-shift-7");
    assert_eq!(
        display_chord(
            &km.bindings_for("t.c", &tr_q())[0].chord,
            KeymapPlatform::Windows
        ),
        "Ctrl+Shift+7"
    );
    assert_eq!(
        display_chord(
            &km.bindings_for("t.c", &us())[0].chord,
            KeymapPlatform::Windows
        ),
        "Ctrl+/"
    );
}

#[test]
fn overlay_chord_resolves_only_on_its_layout() {
    let km = fixture_with_overlay("ctrl-/", "tr-q", "ctrl-shift-7");
    assert_eq!(
        km.resolve(&ws(), &[], &kp(ctrl_shift(), '7'), &tr_q(), false),
        Resolution::Command("t.c".into())
    );
    assert_eq!(
        km.resolve(&ws(), &[], &kp(ctrl_shift(), '7'), &us(), false),
        Resolution::PassThrough
    );
}

#[test]
fn symbol_chord_ignores_shift_when_layout_needs_it() {
    let tr = tr_q();
    let (km, d) = Keymap::from_sources(
        KeymapPlatform::Windows,
        &entry("ctrl-/", "t.c"),
        "",
        &[(&tr, "shift_level = \"/\"")],
        None,
    );
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(
        km.resolve(&ws(), &[], &kp(ctrl_shift(), '/'), &tr, false),
        Resolution::Command("t.c".into())
    );
    assert_eq!(
        km.resolve(&ws(), &[], &kp(ctrl_shift(), '/'), &us(), false),
        Resolution::PassThrough
    );
}

#[test]
fn mac_display_uses_glyphs() {
    assert_eq!(
        display_chord(&parse_chord("cmd-shift-p").unwrap(), KeymapPlatform::MacOs),
        "⇧⌘P"
    );
    assert_eq!(
        display_chord(
            &parse_chord("ctrl-alt-shift-cmd-k").unwrap(),
            KeymapPlatform::MacOs
        ),
        "⌃⌥⇧⌘K"
    );
}

#[test]
fn windows_display_uses_english_modifier_names() {
    assert_eq!(
        display_chord(
            &parse_chord("ctrl-k ctrl-s").unwrap(),
            KeymapPlatform::Linux
        ),
        "Ctrl+K Ctrl+S"
    );
    assert_eq!(
        display_chord(
            &parse_chord("ctrl-shift-tab").unwrap(),
            KeymapPlatform::Windows
        ),
        "Ctrl+Shift+Tab"
    );
}

#[test]
fn duplicate_chord_in_context_is_diagnosed() {
    let (_, d) = Keymap::from_sources(KeymapPlatform::Windows, DUP, "", &[], None);
    assert_eq!(
        d,
        vec![KeymapDiag::DuplicateChord {
            chord: "ctrl-k".into(),
            context: "Workspace".into()
        }]
    );
}

#[test]
fn bindings_lists_every_loaded_binding() {
    let km = fixture_chord("ctrl-k ctrl-s", "t.save");
    assert_eq!(km.bindings().len(), 1);
    assert_eq!(km.bindings()[0].command, "t.save");
    assert_eq!(km.platform(), KeymapPlatform::Windows);
}

#[test]
fn windows_turkish_klid_maps_to_tr_q() {
    assert_eq!(
        layout_id_from_platform("0000041F"),
        KeyboardLayoutId("tr-q".into())
    );
    assert_eq!(
        layout_id_from_platform("com.apple.keylayout.Turkish"),
        KeyboardLayoutId("tr-q".into())
    );
}

#[test]
fn other_platform_layout_ids_map_to_known_layouts() {
    for (id, want) in [
        ("0001041f", "tr-f"),
        ("00000407", "de"),
        ("0000040C", "fr"),
        ("00000415", "pl"),
        ("00000409", "us"),
        ("com.apple.keylayout.German", "de"),
        ("com.apple.keylayout.French", "fr"),
        ("com.apple.keylayout.Polish", "pl"),
        ("com.apple.keylayout.US", "us"),
        ("tr", "tr-q"),
        ("tr(f)", "tr-f"),
        ("de", "de"),
        ("something-unknown", "us"),
    ] {
        assert_eq!(layout_id_from_platform(id), layout(want), "{id}");
    }
}

#[test]
fn parse_chord_reads_modifiers_keys_and_sequences() {
    let c = parse_chord("ctrl-shift-7").unwrap();
    assert_eq!(c, vec![kp(ctrl_shift(), '7')]);
    let c = parse_chord("cmd-k cmd-s").unwrap();
    assert_eq!(c, vec![kp(cmd(), 'k'), kp(cmd(), 's')]);
    assert_eq!(
        parse_chord("ctrl-tab").unwrap(),
        vec![kpn(ctrl(), NamedKey::Tab)]
    );
    assert_eq!(
        parse_chord("ctrl-f4").unwrap(),
        vec![kpn(ctrl(), NamedKey::F(4))]
    );
    assert_eq!(parse_chord("ctrl--").unwrap(), vec![kp(ctrl(), '-')]);
    assert_eq!(
        parse_chord("alt-,").unwrap(),
        vec![kp(m(false, true, false, false), ',')]
    );
    assert_eq!(parse_chord("ctrl-I").unwrap(), vec![kp(ctrl(), 'i')]);
}

#[test]
fn parse_chord_rejects_garbage() {
    for bad in ["", "ctrl-", "hyper-k", "ctrl-ctrl-k", "ctrl-kk", "ctrl-f99"] {
        assert!(parse_chord(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn chord_string_round_trips() {
    for s in [
        "ctrl-alt-s",
        "ctrl-shift-tab",
        "cmd-,",
        "ctrl-k ctrl-s",
        "ctrl-f4",
    ] {
        assert_eq!(chord_string(&parse_chord(s).unwrap()), s);
    }
}

#[test]
fn preset_replaces_default_by_command() {
    let (km, d) = Keymap::from_sources(
        KeymapPlatform::Windows,
        &(entry("ctrl-k", "t.palette") + &entry("ctrl-w", "t.close")),
        &entry("ctrl-shift-a", "t.palette"),
        &[],
        None,
    );
    assert!(d.is_empty(), "{d:?}");
    let chords: Vec<String> = km
        .bindings()
        .iter()
        .map(|b| format!("{}={}", chord_string(&b.chord), b.command))
        .collect();
    assert_eq!(chords, vec!["ctrl-w=t.close", "ctrl-shift-a=t.palette"]);
}

#[test]
fn user_binding_wins_over_a_default_on_the_same_chord() {
    let (km, d) = Keymap::from_sources(
        KeymapPlatform::Windows,
        &entry("ctrl-k", "t.default"),
        "",
        &[],
        Some(&entry("ctrl-k", "t.user")),
    );
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(
        km.resolve(&ws(), &[], &kp(ctrl(), 'k'), &us(), false),
        Resolution::Command("t.user".into())
    );
    assert_eq!(km.bindings().len(), 1);
}

#[test]
fn platforms_field_filters_bindings() {
    let toml = "[[binding]]\nchord=\"ctrl-q\"\ncommand=\"app.quit\"\ncontext=\"Workspace\"\nplatforms=[\"linux\"]";
    assert_eq!(fixture_on(KeymapPlatform::Linux, toml).bindings().len(), 1);
    assert_eq!(
        fixture_on(KeymapPlatform::Windows, toml).bindings().len(),
        0
    );
}

#[test]
fn bad_chord_is_a_parse_diag_with_line_number() {
    let toml = "[[binding]]\nchord=\"ctrl-k\"\ncommand=\"a\"\ncontext=\"Workspace\"\n\n[[binding]]\nchord=\"hyper-k\"\ncommand=\"b\"\ncontext=\"Workspace\"\n";
    let (km, d) = Keymap::from_sources(KeymapPlatform::Windows, toml, "", &[], None);
    assert_eq!(km.bindings().len(), 1);
    assert_eq!(d.len(), 1);
    match &d[0] {
        KeymapDiag::Parse { source, line, .. } => {
            assert_eq!((*source, *line), (BindingSource::Default, 7));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn altgr_table_parses_and_looks_up() {
    let t = AltGrTable::parse("layout = \"tr-q\"\n[keys]\nq = \"@\"\n\"7\" = \"{\"\n");
    assert_eq!(t.layout(), "tr-q");
    let tables = AltGrTables(vec![t]);
    assert_eq!(tables.get("tr-q", 'q'), Some('@'));
    assert_eq!(tables.get("tr-q", 'Q'), Some('@'));
    assert_eq!(tables.get("tr-q", '7'), Some('{'));
    assert_eq!(tables.get("tr-q", 'w'), None);
    assert_eq!(tables.get("de", 'q'), None);
}

#[test]
fn altgr_table_keeps_dead_keys_and_folded_keys() {
    let t = AltGrTable::parse(
        "layout = \"tr-f\"\n[keys]\n\"ı\" = \"¶\"\ni = \"ß\"\n[dead]\nq = \"¨\"\n[shift_keys]\ns = \"º\"\n",
    );
    let tables = AltGrTables(vec![t]);
    // `ı` and `i` fold onto one name, so both characters are reported.
    assert_eq!(tables.get_all("tr-f", 'i', false), ['¶', 'ß']);
    assert_eq!(tables.get_all("tr-f", 'ı', false), ['¶', 'ß']);
    // A dead key types its accent.
    assert_eq!(tables.get("tr-f", 'q'), Some('¨'));
    // The Shift+AltGr layer is separate.
    assert!(tables.get_all("tr-f", 's', false).is_empty());
    assert_eq!(tables.get_all("tr-f", 's', true), ['º']);
}

#[test]
fn collisions_report_every_character_of_a_folded_key() {
    let tables = AltGrTables(vec![AltGrTable::parse(
        "layout = \"tr-f\"\n[keys]\n\"ı\" = \"¶\"\ni = \"ß\"\n",
    )]);
    let km = fixture(&entry("ctrl-alt-i", "t.i"));
    let got: Vec<char> = altgr_collisions(&km, &tables)
        .into_iter()
        .map(|c| c.produces)
        .collect();
    assert_eq!(got, ['¶', 'ß']);
}

fn tables() -> AltGrTables {
    AltGrTables(vec![
        AltGrTable::parse("layout = \"pl\"\n[keys]\nl = \"ł\"\ns = \"ś\"\n"),
        AltGrTable::parse("layout = \"de\"\n[keys]\nm = \"µ\"\n[shift_keys]\nm = \"º\"\n"),
    ])
}

#[test]
fn collisions_name_chord_layout_and_character() {
    let km = fixture(
        &(entry("ctrl-alt-l", "t.l")
            + &entry("ctrl-alt-shift-m", "t.m")
            + &entry("ctrl-alt-x", "t.x")
            + &entry("alt-s", "t.s")),
    );
    assert_eq!(
        altgr_collisions(&km, &tables()),
        vec![
            Collision {
                chord: "ctrl-alt-shift-m".into(),
                layout: "de".into(),
                produces: 'º'
            },
            Collision {
                chord: "ctrl-alt-l".into(),
                layout: "pl".into(),
                produces: 'ł'
            },
        ]
    );
}

#[test]
fn collisions_are_empty_on_macos_and_ignore_named_keys() {
    let mac = fixture_on(KeymapPlatform::MacOs, &entry("ctrl-alt-l", "t.l"));
    assert!(altgr_collisions(&mac, &tables()).is_empty());
    let named = fixture(&entry("ctrl-alt-up", "t.u"));
    assert!(altgr_collisions(&named, &tables()).is_empty());
}

#[test]
fn option_only_printables_flags_bare_option_on_mac() {
    let km = fixture_on(
        KeymapPlatform::MacOs,
        &(entry("alt-s", "t.a")
            + &entry("alt-shift-s", "t.b")
            + &entry("cmd-alt-s", "t.c")
            + &entry("alt-up", "t.d")
            + &entry("ctrl-alt-s", "t.e")),
    );
    assert_eq!(
        option_only_printables(&km),
        vec!["alt-s".to_string(), "alt-shift-s".to_string()]
    );
    let win = fixture_on(KeymapPlatform::Windows, &entry("alt-s", "t.a"));
    assert!(option_only_printables(&win).is_empty());
}

#[test]
fn unreachable_needs_an_overlay_for_shift_level_characters() {
    let fr = layout("fr");
    let shift = ['1', '2'];
    let none = fixture(
        &(entry("alt-2", "t.two") + &entry("ctrl-k", "t.k") + &entry("alt-shift-2", "t.s")),
    );
    assert_eq!(
        unreachable_chords(&none, &fr, &shift),
        vec!["alt-2".to_string()]
    );
    assert!(unreachable_chords(&none, &us(), &[]).is_empty());
    let (fixed, d) = Keymap::from_sources(
        KeymapPlatform::Windows,
        &entry("alt-2", "t.two"),
        "",
        &[(&fr, &entry("alt-shift-2", "t.two"))],
        None,
    );
    assert!(d.is_empty(), "{d:?}");
    assert!(unreachable_chords(&fixed, &fr, &shift).is_empty());
}
