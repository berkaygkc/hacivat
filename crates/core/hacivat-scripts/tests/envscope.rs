//! M4b Task 4: `@<environment>/` scoping fails closed (HV-SCRIPT-04; R-M4b-6, Q3, Q8).
//!
//! Only the five tags scope, ASCII case-insensitive and never through locale casing; an
//! unknown `@name`, a contradictory chain or an untagged profile hides the file.

use std::time::SystemTime;

use hacivat_proto::EnvTag::{self, Dev, Local, Production, Staging, Test};
use hacivat_scripts::envscope::{self, EnvFolder, classify, folder_scope, scope_of};
use hacivat_scripts::{
    EntryKind, EnvScope, EnvSet, LinkState, RelPath, ScopeProblem, ScriptEntry, visible,
};

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

/// An entry whose stored scope says "anywhere": `visible` must recompute the scope from the
/// path, so a caller that builds an entry by hand cannot widen it.
fn entry(p: &str) -> ScriptEntry {
    entry_kind(p, EntryKind::Sql)
}

fn entry_kind(p: &str, kind: EntryKind) -> ScriptEntry {
    ScriptEntry {
        rel: rel(p),
        kind,
        scope: EnvScope::Any,
        link: LinkState::Plain,
        dataless: false,
        len: 0,
        mtime: SystemTime::UNIX_EPOCH,
        target: None,
    }
}

#[test]
fn envscope_table() {
    // (path, env of the connection, visible?) — 18 rows, fixed by the plan:
    let rows = [
        ("a.sql", Some(Production), true),
        ("a.sql", None, true),
        ("@production/a.sql", Some(Production), true),
        ("@production/a.sql", Some(Staging), false),
        ("@production/a.sql", None, false),
        ("@PRODUCTION/a.sql", Some(Production), true),
        ("@Production/x/y/a.sql", Some(Production), true),
        ("@prod/a.sql", Some(Production), false),
        ("@productİon/a.sql", Some(Production), false),
        ("@production /a.sql", Some(Production), false),
        ("@production/@staging/a.sql", Some(Production), false),
        ("@production/@staging/a.sql", Some(Staging), false),
        ("@staging/@staging/a.sql", Some(Staging), true),
        ("x/@dev/a.sql", Some(Dev), true),
        ("x/@dev/a.sql", Some(Test), false),
        ("@local/a.sql", Some(Local), true),
        ("@test/a.sql", Some(Test), true),
        ("email@host/a.sql", Some(Dev), true), // '@' only at a component start scopes
    ];
    for (p, env, want) in rows {
        assert_eq!(visible(&entry(p), env), want, "{p} {env:?}");
    }
}

/// alg/28 HV-SCRIPT-04 test cases, beyond the plan's table.
#[test]
fn spec_rows_unknown_and_contradiction_hide_everywhere() {
    for p in [
        "@prod/a.sql",
        "@productİon/a.sql",
        "@production /a.sql",
        "@production/@staging/a.sql",
        "@/a.sql",
        "@dev/a.sql",
    ] {
        assert!(!visible(&entry(p), None), "{p} with no env");
    }
    for p in [
        "@prod/a.sql",
        "@productİon/a.sql",
        "@production/@staging/a.sql",
        "@/a.sql",
    ] {
        for tag in EnvTag::ALL {
            assert!(!visible(&entry(p), Some(tag)), "{p} {tag:?}");
        }
    }
    assert!(visible(&entry("a.sql"), None));
}

#[test]
fn untagged_profile_sees_only_unscoped() {
    assert!(visible(&entry("a.sql"), None));
    assert!(visible(&entry("x/y/a.sql"), None));
    assert!(visible(&entry("email@host/a.sql"), None));
    for tag in EnvTag::ALL {
        let p = format!("@{}/a.sql", tag.id());
        assert!(!visible(&entry(&p), None), "{p}");
        assert!(visible(&entry(&p), Some(tag)), "{p}");
    }
}

#[test]
fn file_named_at_production_is_not_a_scope() {
    // Folder components only: a file named `@production.sql` (or `@prod.sql`) scopes nothing.
    assert_eq!(scope_of(&rel("@production.sql")), EnvScope::Any);
    assert_eq!(scope_of(&rel("@prod.sql")), EnvScope::Any);
    assert_eq!(scope_of(&rel("x/@production")), EnvScope::Any);
    for tag in EnvTag::ALL {
        assert!(visible(&entry("@production.sql"), Some(tag)));
    }
    assert!(visible(&entry("@production.sql"), None));
}

#[test]
fn a_folder_entry_is_scoped_by_its_own_name() {
    // The `@production` folder row itself is hidden under staging, like its files.
    let f = entry_kind("@production", EntryKind::Folder);
    assert!(visible(&f, Some(Production)));
    assert!(!visible(&f, Some(Staging)));
    assert!(!visible(&f, None));
    assert_eq!(
        folder_scope(&rel("@production")),
        EnvScope::Only(EnvSet::of(Production))
    );
    assert!(!visible(
        &entry_kind("@prod", EntryKind::Folder),
        Some(Production)
    ));
}

#[test]
fn tags_are_ascii_case_insensitive_only() {
    for name in ["@production", "@PRODUCTION", "@Production", "@pRoDuCtIoN"] {
        assert_eq!(classify(name), EnvFolder::Tag(Production), "{name}");
    }
    // Turkish dotted capital I and dotless small i are not ASCII `I`/`i`, whatever the locale
    // (never `to_lowercase`; R-M4b-6, alg/26 INV).
    for name in [
        "@PRODUCTİON",
        "@productİon",
        "@productıon",
        "@PRODUCTıON",
        "@DEV\u{307}",
    ] {
        assert!(
            matches!(classify(name), EnvFolder::Unknown { .. }),
            "{name:?} must not scope"
        );
    }
    assert_eq!(classify("@LOCAL"), EnvFolder::Tag(Local));
    assert_eq!(classify("@Test"), EnvFolder::Tag(Test));
    assert_eq!(classify("@sTaGiNg"), EnvFolder::Tag(Staging));
    assert_eq!(classify("@DEV"), EnvFolder::Tag(Dev));
    assert_eq!(classify("prod"), EnvFolder::NotEnv);
    assert_eq!(classify("email@host"), EnvFolder::NotEnv);
}

#[test]
fn did_you_mean_suggestions() {
    let sug = |n: &str| match classify(n) {
        EnvFolder::Unknown { suggestion } => suggestion,
        other => panic!("{n:?} classified as {other:?}"),
    };
    assert_eq!(sug("@prod"), Some(Production));
    assert_eq!(sug("@PROD"), Some(Production));
    assert_eq!(sug("@productİon"), Some(Production));
    assert_eq!(sug("@production "), Some(Production));
    assert_eq!(sug("@prodution"), Some(Production));
    assert_eq!(sug("@stage"), Some(Staging));
    assert_eq!(sug("@staing"), Some(Staging));
    assert_eq!(sug("@lcoal"), Some(Local));
    assert_eq!(sug("@tset"), Some(Test));
    assert_eq!(sug("@develop"), Some(Dev));
    assert_eq!(sug("@"), None);
    assert_eq!(sug("@x"), None);
    assert_eq!(sug("@orders"), None);
}

#[test]
fn at_lookalikes_and_invisible_characters_fail_closed() {
    // A full-width or small commercial at, or an invisible character after the `@`, makes a
    // folder that reads as `@production` to a person: it must not be an ordinary folder
    // (visible on staging), so it is an unknown environment folder, hidden everywhere. (An
    // invisible character *before* the `@` is skipped: fix round 1, I1, below.)
    for name in [
        "\u{FF20}production",
        "\u{FE6B}production",
        " \u{FF20}production",
        "@production\u{200B}",
        "@pro\u{200D}duction",
        "@\u{202E}noitcudorp",
    ] {
        assert!(
            matches!(classify(name), EnvFolder::Unknown { .. }),
            "{name:?}: {:?}",
            classify(name)
        );
        if !name.contains('\u{202E}') {
            assert_eq!(
                classify(name),
                EnvFolder::Unknown {
                    suggestion: Some(Production)
                },
                "{name:?}"
            );
        }
        let p = format!("{name}/purge.sql");
        for tag in EnvTag::ALL {
            assert!(!visible(&entry(&p), Some(tag)), "{p:?} {tag:?}");
        }
        assert!(!visible(&entry(&p), None), "{p:?}");
    }
}

#[test]
fn unknown_beats_a_later_tag_and_names_the_folder() {
    assert!(matches!(
        scope_of(&rel("@prod/@production/a.sql")),
        EnvScope::Hidden(ScopeProblem::Unknown(n)) if n == "@prod"
    ));
    assert!(matches!(
        scope_of(&rel("@production/@prod/a.sql")),
        EnvScope::Hidden(ScopeProblem::Unknown(n)) if n == "@prod"
    ));
    assert_eq!(
        scope_of(&rel("@production/@staging/a.sql")),
        EnvScope::Hidden(ScopeProblem::Contradiction)
    );
    assert_eq!(
        scope_of(&rel("@production/x/@production/a.sql")),
        EnvScope::Only(EnvSet::of(Production))
    );
}

#[test]
fn an_unknown_name_with_a_bidi_control_is_escaped_in_the_problem() {
    let s = scope_of(&rel("@pro\u{202E}d/a.sql"));
    assert!(
        matches!(&s, EnvScope::Hidden(ScopeProblem::Unknown(n)) if n == "@pro<U+202E>d"),
        "{s:?}"
    );
}

#[test]
fn a_stored_narrower_scope_is_kept() {
    // A link at `a.sql` whose target lives under `@staging/` carries the target's scope
    // (the scan stores it); `visible` keeps the stricter of the stored and the path scope.
    let mut e = entry("a.sql");
    e.scope = EnvScope::Only(EnvSet::of(Staging));
    assert!(visible(&e, Some(Staging)));
    assert!(!visible(&e, Some(Production)));
    assert!(!visible(&e, None));
    e.scope = EnvScope::Hidden(ScopeProblem::Contradiction);
    for tag in EnvTag::ALL {
        assert!(!visible(&e, Some(tag)));
    }
}

#[test]
fn combine_intersects() {
    use EnvScope::{Any, Hidden, Only};
    let p = Only(EnvSet::of(Production));
    let s = Only(EnvSet::of(Staging));
    assert_eq!(envscope::combine(&Any, &Any), Any);
    assert_eq!(envscope::combine(&Any, &p), p);
    assert_eq!(envscope::combine(&p, &Any), p);
    assert_eq!(envscope::combine(&p, &p), p);
    assert_eq!(
        envscope::combine(&p, &s),
        Hidden(ScopeProblem::Contradiction)
    );
    let unknown = Hidden(ScopeProblem::Unknown("@prod".into()));
    assert_eq!(envscope::combine(&Any, &unknown), unknown);
    assert_eq!(envscope::combine(&unknown, &p), unknown);
}

#[test]
fn nfd_names_do_not_change_ascii_tags() {
    // macOS keeps NFD names; the tags are ASCII, so NFC/NFD never changes a scope.
    assert_eq!(
        scope_of(&rel("@production/mu\u{0308}s\u{0327}teri.sql")),
        EnvScope::Only(EnvSet::of(Production))
    );
    assert!(visible(&entry("@dev/rapor\u{0308}.sql"), Some(Dev)));
}

// ## m4b-t04-scanner fix round 1 (review I1, m7, I2).

/// Characters a person does not see in front of the `@` are skipped (controller ruling on I1):
/// the reviewer's seven names, the zero-width control, and more of the same classes.
const UNSEEN_LEADS: [&str; 19] = [
    " ",                        // SPACE
    "\u{00A0}",                 // NO-BREAK SPACE (macOS Option+Space)
    "\u{2002}",                 // EN SPACE
    "\u{3000}",                 // IDEOGRAPHIC SPACE
    "\u{2800}",                 // BRAILLE PATTERN BLANK
    "\u{0301}",                 // COMBINING ACUTE ACCENT
    "\u{1D159}",                // MUSICAL SYMBOL NULL NOTEHEAD
    "\u{200B}",                 // ZERO WIDTH SPACE
    "\u{202F}",                 // NARROW NO-BREAK SPACE
    "\u{2003}\u{2009}",         // EM SPACE, THIN SPACE
    "\u{FEFF}",                 // BOM
    "\u{0345}",                 // COMBINING GREEK YPOGEGRAMMENI (an alphabetic mark)
    "\u{0E31}",                 // THAI CHARACTER MAI HAN-AKAT (a mark of class 0)
    "\u{20DD}",                 // COMBINING ENCLOSING CIRCLE (Me)
    "\u{3164}",                 // HANGUL FILLER
    "\u{115F}",                 // HANGUL CHOSEONG FILLER
    "\u{E000}",                 // a private-use character
    "\u{202E}",                 // RIGHT-TO-LEFT OVERRIDE
    "\u{00A0}\u{0301}\u{200B}", // a mixed run
];

#[test]
fn leading_blanks_and_marks_before_at_scope_like_at() {
    for lead in UNSEEN_LEADS {
        let name = format!("{lead}@production");
        assert_eq!(classify(&name), EnvFolder::Tag(Production), "{name:?}");
        let p = format!("{name}/purge.sql");
        assert!(!visible(&entry(&p), Some(Staging)), "{p:?} on staging");
        assert!(!visible(&entry(&p), None), "{p:?} untagged");
        assert!(visible(&entry(&p), Some(Production)), "{p:?} on production");
        let staging = format!("{lead}@STAGING");
        assert_eq!(classify(&staging), EnvFolder::Tag(Staging), "{staging:?}");
    }
}

#[test]
fn leading_blanks_before_an_unknown_at_name_hide_with_a_suggestion() {
    for lead in UNSEEN_LEADS {
        for (rest, want) in [
            ("@prod", Some(Production)),
            ("@stagin", Some(Staging)),
            ("@x", None),
        ] {
            let name = format!("{lead}{rest}");
            assert_eq!(
                classify(&name),
                EnvFolder::Unknown { suggestion: want },
                "{name:?}"
            );
            let p = format!("{name}/a.sql");
            for tag in EnvTag::ALL {
                assert!(!visible(&entry(&p), Some(tag)), "{p:?} {tag:?}");
            }
        }
    }
    // A lookalike `@` after the blanks is unknown, never a tag.
    assert_eq!(
        classify("\u{00A0}\u{FF20}production"),
        EnvFolder::Unknown {
            suggestion: Some(Production)
        }
    );
}

#[test]
fn visible_characters_before_at_are_ordinary_folders() {
    for name in [
        "x@production",
        "1@production",
        "(@production)",
        "é@production",
        "İ@production",
    ] {
        assert_eq!(classify(name), EnvFolder::NotEnv, "{name:?}");
    }
}

#[test]
fn root_env_folder_names_the_first_env_component() {
    use std::path::Path;
    assert_eq!(
        envscope::root_env_folder(Path::new("/x/@production/reports")),
        Some("@production".into())
    );
    assert_eq!(
        envscope::root_env_folder(Path::new("/x/@prod")),
        Some("@prod".into())
    );
    assert_eq!(
        envscope::root_env_folder(Path::new("/x/\u{00A0}@staging/y")),
        Some("\u{00A0}@staging".into())
    );
    assert_eq!(
        envscope::root_env_folder(Path::new("/x/@pro\u{202E}d/y")),
        Some("@pro<U+202E>d".into())
    );
    assert_eq!(
        envscope::root_env_folder(Path::new("/Users/me@corp/lib")),
        None
    );
    assert_eq!(envscope::root_env_folder(Path::new("/x/reports")), None);
}

#[test]
fn visible_at_uses_the_resolved_target() {
    // I2: the caller of `join` re-checks the scope of what was resolved.
    let link = rel("a.sql");
    let prod = rel("@production/secret.sql");
    assert!(envscope::visible_at(
        &link,
        EntryKind::Sql,
        None,
        Some(Staging)
    ));
    assert!(!envscope::visible_at(
        &link,
        EntryKind::Sql,
        Some(&prod),
        Some(Staging)
    ));
    assert!(envscope::visible_at(
        &link,
        EntryKind::Sql,
        Some(&prod),
        Some(Production)
    ));
    assert!(!envscope::visible_at(
        &link,
        EntryKind::Sql,
        Some(&prod),
        None
    ));
    let staged = rel("@staging/p.sql");
    for tag in EnvTag::ALL {
        assert!(!envscope::visible_at(
            &staged,
            EntryKind::Sql,
            Some(&prod),
            Some(tag)
        ));
    }
    assert!(!envscope::visible_at(
        &rel("@production"),
        EntryKind::Folder,
        None,
        Some(Staging)
    ));
}
