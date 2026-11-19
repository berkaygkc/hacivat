//! `profile_data`: the only reader and writer of `<config>/profiles/<uuid>.toml` (M4 Task 1c).
//! Each owner keeps its own section (`[metadata]` M4, `[masks]`/`[ai]` later milestones); a
//! save replaces one section and leaves every other byte of the file as it was.

use hacivat_settings::LocalPaths;
use hacivat_settings::profile_data::{ProfileDataDoc, ProfileDataError};
use serde::{Deserialize, Serialize};

const UUID: &str = "6f9619ff-8b86-d011-b42d-00c04fc964ff";

/// M4's section, as a stand-in for `hacivat_meta::ProfileUserData`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Metadata {
    scope: String,
    level: Option<String>,
    virtual_fks: Vec<VirtualFk>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct VirtualFk {
    from: String,
    to: String,
}

/// Another owner's section (M6 masks).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Masks {
    columns: Vec<String>,
}

fn metadata() -> Metadata {
    Metadata {
        scope: "@:*".into(),
        level: Some("structure".into()),
        virtual_fks: vec![VirtualFk {
            from: "public.orders.customer_id".into(),
            to: "public.customers.id".into(),
        }],
    }
}

fn paths() -> (LocalPaths, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    (LocalPaths::under(dir.path()), dir)
}

fn write(paths: &LocalPaths, text: &str) {
    let path = paths.profile_toml(UUID);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn read(paths: &LocalPaths) -> String {
    std::fs::read_to_string(paths.profile_toml(UUID)).expect("read the file")
}

/// A file another build wrote: comments, odd spacing, inline tables, an array of tables.
const OTHERS: &str = "# Per-profile data. Hand-edited.\n\n[ai]\n  model   =   \"local\"   # the user's choice\nallow = [ \"explain\",\"summarize\" ]\n\n[[ai.rules]]\nname='no-prod'\n\n# masks before metadata\n[masks]\ncolumns = [\"email\",   \"phone\"]\nextra = { keep = true }\n";

#[test]
fn unknown_sections_survive_a_metadata_save() {
    let (paths, _dir) = paths();
    write(&paths, OTHERS);
    let mut doc = ProfileDataDoc::load(&paths, UUID).expect("load");
    assert_eq!(doc.section::<Metadata>("metadata").unwrap(), None);
    doc.save_section("metadata", &metadata()).expect("save");
    let text = read(&paths);
    // Every byte of the other sections is where it was; the new section follows them.
    let (before, added) = text.split_at(OTHERS.len());
    assert_eq!(before, OTHERS);
    assert!(added.contains("[metadata]"), "{added}");
    assert!(added.contains("[[metadata.virtual_fks]]"), "{added}");

    // Saved again with another value: the section is replaced in place, the rest unchanged.
    let mut changed = metadata();
    changed.level = None;
    changed.scope = "public".into();
    doc.save_section("metadata", &changed).expect("save");
    let again = read(&paths);
    assert!(again.starts_with(OTHERS), "{again}");
    assert_eq!(again.matches("[metadata]").count(), 1, "{again}");

    // A section in the middle of the file keeps its place and the sections after it keep
    // their bytes.
    let middle =
        format!("{OTHERS}\n[metadata]\nscope = \"x\"\nvirtual_fks = []\n\n# tail\n[zz]\nk = 1\n");
    write(&paths, &middle);
    let mut doc = ProfileDataDoc::load(&paths, UUID).expect("load");
    doc.save_section("metadata", &metadata()).expect("save");
    let text = read(&paths);
    assert!(text.starts_with(OTHERS), "{text}");
    assert!(text.ends_with("\n# tail\n[zz]\nk = 1\n"), "{text}");

    // What was saved reads back, through this doc and a fresh one.
    assert_eq!(
        doc.section::<Metadata>("metadata").unwrap(),
        Some(metadata())
    );
    let fresh = ProfileDataDoc::load(&paths, UUID).expect("load");
    assert_eq!(
        fresh.section::<Metadata>("metadata").unwrap(),
        Some(metadata())
    );
    assert_eq!(
        fresh.section::<Masks>("masks").unwrap(),
        Some(Masks {
            columns: vec!["email".into(), "phone".into()]
        })
    );
}

#[test]
fn two_owners_write_through_one_writer() {
    let (paths, _dir) = paths();
    // Both owners loaded the (missing) file before either saved.
    let mut m4 = ProfileDataDoc::load(&paths, UUID).expect("load");
    let mut m6 = ProfileDataDoc::load(&paths, UUID).expect("load");
    m4.save_section("metadata", &metadata()).expect("save");
    let masks = Masks {
        columns: vec!["ssn".into()],
    };
    m6.save_section("masks", &masks).expect("save");
    // The second save re-read the file: the first owner's section is still there.
    let fresh = ProfileDataDoc::load(&paths, UUID).expect("load");
    assert_eq!(
        fresh.section::<Metadata>("metadata").unwrap(),
        Some(metadata())
    );
    assert_eq!(fresh.section::<Masks>("masks").unwrap(), Some(masks));
    // And the first owner's doc sees the second's section after its own next save.
    m4.save_section("metadata", &metadata()).expect("save");
    assert!(m4.section::<Masks>("masks").unwrap().is_some());
}

#[test]
fn a_bad_uuid_or_a_corrupt_file_is_refused() {
    let (paths, _dir) = paths();
    for bad in [
        "../settings",
        "6f9619ff/../../x",
        "",
        "not-a-uuid",
        "6F9619FF-8B86-D011-B42D-00C04FC964FF/",
    ] {
        assert!(
            matches!(
                ProfileDataDoc::load(&paths, bad),
                Err(ProfileDataError::InvalidProfileId)
            ),
            "{bad:?}"
        );
    }
    // A file that is not TOML is never overwritten: other owners' data may be in it.
    write(&paths, "[ai\nmodel = ");
    assert!(matches!(
        ProfileDataDoc::load(&paths, UUID),
        Err(ProfileDataError::Corrupt)
    ));
    assert_eq!(read(&paths), "[ai\nmodel = ");
    // A section of the wrong shape is an error for that section only.
    write(&paths, "[metadata]\nscope = 3\n[masks]\ncolumns = []\n");
    let doc = ProfileDataDoc::load(&paths, UUID).expect("load");
    assert!(matches!(
        doc.section::<Metadata>("metadata"),
        Err(ProfileDataError::InvalidSection(name)) if name == "metadata"
    ));
    assert_eq!(
        doc.section::<Masks>("masks").unwrap(),
        Some(Masks { columns: vec![] })
    );
}

#[cfg(unix)]
#[test]
fn profile_data_written_atomically_with_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let (paths, _dir) = paths();
    let mut doc = ProfileDataDoc::load(&paths, UUID).expect("load");
    doc.save_section("metadata", &metadata()).expect("save");
    let file = paths.profile_toml(UUID);
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let dir_mode = std::fs::metadata(file.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir_mode, 0o700);
    // Atomic: no temporary sibling is left behind.
    let names: Vec<String> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, [format!("{UUID}.toml")]);
}
