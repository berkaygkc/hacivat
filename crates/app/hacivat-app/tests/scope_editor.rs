//! M4 Task 20: the connection dialog's Schemas section (design/m4-explorer surface 7), "Encrypt
//! cached metadata" (R-M4-15) and the introspection level pills (surface 6).
//!
//! Profile data files and cache files are read and written on the core runtime, so tests that
//! change them wait for the result with `wait_until` (a condition, never a fixed time).

mod support;

use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::dialog::schemas::{self, EncryptAsk, SaveFault, SchemasSection, Tri};
use hacivat_app::connections::dialog::{
    ConnectionDialog, DialogMode, open_connection_dialog, open_dialog,
};
use hacivat_app::connections::explorer::{self, ShownRow};
use hacivat_app::connections::{ConnectionsModel, import};
use hacivat_app::explorer::{adapter, level_pill, userdata};
use hacivat_driver::fake::FakeDriver;
use hacivat_meta::userdata::ProfileUserData;
use hacivat_meta::{CatalogSnapshot, Level, LoadState, NsKey, ObjectKind};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::profile::{Profile, ProfileDraft};
use hacivat_settings::profile_data::ProfileDataDoc;
use hacivat_ui::tree::{TreeKey, TreeRowData};
use support::catalogs::*;
use support::*;

// ---- fixtures ---------------------------------------------------------------------------------

/// A connections shell in `lang` (the fake drivers answer the server probe).
fn shell(cx: &mut TestAppContext, lang: &str) -> &'static mut VisualTestContext {
    shell_with_fake(cx, lang, answers_probe(FakeDriver::new(DbmsKind::Postgres)))
}

/// A PostgreSQL profile `name` on database `app`, tagged `env`.
fn pg(name: &str, env: EnvTag) -> Profile {
    let mut p = test_profile(name, Some(env), "127.0.0.1");
    p.database = Some("app".to_string());
    p
}

/// `app` with `analytics`, `public` (two tables) and `Sales` (a view), the built-in
/// `pg_catalog`, and the database `billing` listed out of scope.
fn shop() -> Arc<CatalogSnapshot> {
    catalog_gen(
        DbmsKind::Postgres,
        1,
        vec![
            schema(DbmsKind::Postgres, &["app", "analytics"], false, Vec::new()),
            schema(
                DbmsKind::Postgres,
                &["app", "public"],
                false,
                vec![
                    table("orders", Some(16_400), &[("id", "bigint")]),
                    table("users", Some(16_410), &[("id", "integer")]),
                ],
            ),
            schema(
                DbmsKind::Postgres,
                &["app", "Sales"],
                false,
                vec![leaf(ObjectKind::View, "v_sales")],
            ),
            schema(DbmsKind::Postgres, &["app", "pg_catalog"], true, Vec::new()),
        ],
        vec![(NsKey::database("billing"), LoadState::OutOfScope)],
    )
}

fn show(vcx: &VisualTestContext, profile: ProfileId, cat: Arc<CatalogSnapshot>) {
    vcx.cx
        .update(|cx| adapter::set_catalog(profile, Some(cat), None, cx));
    vcx.run_until_parked();
}

/// The profile's data, once its file was read.
fn data(vcx: &VisualTestContext, id: ProfileId) -> Option<Arc<ProfileUserData>> {
    vcx.cx.read(|cx| userdata::get(id, cx))
}

fn wait_data(vcx: &VisualTestContext, id: ProfileId) {
    wait_until(vcx, "the profile's data file to be read", |vcx| {
        data(vcx, id).is_some()
    });
}

/// A shell with the Dev profile `shop` saved and its catalog shown.
fn shop_shell(cx: &mut TestAppContext, lang: &str) -> (&'static mut VisualTestContext, ProfileId) {
    let vcx = shell(cx, lang);
    let id = add_profile(vcx, pg("shop", EnvTag::Dev));
    show(vcx, id, shop());
    wait_data(vcx, id);
    (vcx, id)
}

// ---- the dialog -------------------------------------------------------------------------------

fn dialog(vcx: &VisualTestContext) -> Option<Entity<ConnectionDialog>> {
    in_window(vcx, |window, cx| open_dialog(window, cx))
}

fn section(vcx: &VisualTestContext) -> Entity<SchemasSection> {
    in_window(vcx, |window, cx| schemas::section_state(window, cx)).expect("the Schemas section")
}

/// Reads the section.
fn read_section<R>(vcx: &VisualTestContext, f: impl FnOnce(&SchemasSection, &gpui::App) -> R) -> R {
    let s = section(vcx);
    vcx.cx.read(|cx| f(s.read(cx), cx))
}

/// `connections.toml` once it holds profile `name` (the model queues its writes).
fn connections_file(vcx: &VisualTestContext, name: &str) -> Vec<u8> {
    let file = paths(vcx).connections_toml();
    let needle = format!("\"{name}\"");
    wait_until(vcx, "connections.toml to hold the profile", |_| {
        std::fs::read_to_string(&file).is_ok_and(|t| t.contains(&needle))
    });
    std::fs::read(&file).expect("connections.toml")
}

/// Calls `f` on the section, as a click or a key in it would.
fn with_section<R>(
    vcx: &VisualTestContext,
    f: impl FnOnce(&mut SchemasSection, &mut gpui::Window, &mut gpui::Context<SchemasSection>) -> R,
) -> R {
    let out = in_window(vcx, |window, cx| {
        let s = schemas::section_state(window, cx).expect("the Schemas section");
        s.update(cx, |s, cx| f(s, window, cx))
    });
    vcx.run_until_parked();
    out
}

/// Opens `mode`'s dialog with the Schemas section open.
fn open(vcx: &VisualTestContext, mode: DialogMode) {
    in_window(vcx, |window, cx| open_connection_dialog(mode, window, cx));
    wait_until(vcx, "the Schemas section to be drawn", |vcx| {
        in_window(vcx, |window, cx| schemas::section_state(window, cx)).is_some()
    });
    with_section(vcx, |s, _, cx| s.set_open(true, cx));
}

fn rows(vcx: &VisualTestContext) -> Vec<(String, Tri)> {
    read_section(vcx, |s, _| s.rows())
}

fn pattern(vcx: &VisualTestContext) -> String {
    read_section(vcx, |s, cx| s.pattern_text(cx))
}

fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

/// Replaces the pattern by typing `text` into the focused field.
fn type_pattern(vcx: &mut VisualTestContext, text: &str) {
    let input = read_section(vcx, |s, _| s.pattern_input());
    let handle = vcx.cx.read(|cx| input.read(cx).focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
    vcx.simulate_keystrokes(&primary("a"));
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

/// The draft's effective "Encrypt cached metadata" (what the box shows).
fn draft_sensitive(vcx: &VisualTestContext) -> bool {
    let d = dialog(vcx).expect("a dialog");
    vcx.cx.read(|cx| d.read(cx).profile().catalog_sensitive())
}

/// The draft's choice (`None`: derived from the environment).
fn draft_choice(vcx: &VisualTestContext) -> Option<bool> {
    let d = dialog(vcx).expect("a dialog");
    vcx.cx.read(|cx| d.read(cx).profile().catalog_sensitive_set)
}

/// Waits until every queued write has landed: the profile data writer (a barrier through its
/// ordered queue) and the profile list's writer (fix round 1, M-7: no fixed sleep before a
/// "nothing was written" check).
fn quiet(vcx: &VisualTestContext) {
    let barrier = vcx.cx.update(userdata::barrier);
    wait_task(vcx, barrier);
    let drained = vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .drain_writes(Duration::from_secs(10))
    });
    assert!(drained, "the profile writer drains");
    vcx.run_until_parked();
}

/// The save problem the form shows (`None`: none).
fn problem(vcx: &VisualTestContext) -> Option<String> {
    let d = dialog(vcx).expect("a dialog");
    vcx.cx
        .read(|cx| d.read(cx).save_problem_text(cx))
        .map(|t| t.to_string())
}

/// Waits until a Save that started has finished (it failed when the dialog is still open).
fn wait_saved(vcx: &VisualTestContext) {
    wait_until(vcx, "the save to finish", |vcx| {
        dialog(vcx).is_none_or(|d| vcx.cx.read(|cx| !d.read(cx).is_saving()))
    });
}

fn stored(vcx: &VisualTestContext, id: ProfileId) -> Arc<Profile> {
    vcx.cx
        .read(|cx| cx.global::<ConnectionsModel>().get(id))
        .expect("the profile is saved")
}

fn dialog_open(vcx: &VisualTestContext) -> bool {
    layer_ids(vcx).contains(&"connection-dialog")
}

/// Runs Save and waits until it finished (or was refused).
fn save(vcx: &VisualTestContext) {
    run(vcx, "connection_dialog.save");
    wait_saved(vcx);
}

fn focus_field(vcx: &mut VisualTestContext, name: &str) {
    let d = dialog(vcx).expect("a dialog");
    let input = vcx
        .cx
        .read(|cx| d.read(cx).input(name))
        .unwrap_or_else(|| panic!("no field {name}"));
    let handle = vcx.cx.read(|cx| input.read(cx).focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
}

fn profile_file(vcx: &VisualTestContext, id: ProfileId) -> std::path::PathBuf {
    paths(vcx).profile_toml(&id.0.to_string())
}

/// The `[metadata]` table of `id`'s data file, read from disk.
fn file_data(vcx: &VisualTestContext, id: ProfileId) -> Option<ProfileUserData> {
    let doc = ProfileDataDoc::load(&paths(vcx), &id.0.to_string()).expect("the file reads");
    doc.section(userdata::SECTION).expect("the table reads")
}

// ---- tests: pattern and tree ------------------------------------------------------------------

#[gpui::test]
fn pattern_and_tree_round_trip(cx: &mut TestAppContext) {
    let (vcx, id) = shop_shell(cx, "en");
    open(vcx, DialogMode::Edit(id));
    // R-M4-14: a profile with no saved scope reads every schema of its database. The built-in
    // schema is not listed; another database is, unchecked, with its own-connection hint.
    assert_eq!(pattern(vcx), "@:*");
    let all = vec![
        ("app".to_string(), Tri::Checked),
        ("app.analytics".to_string(), Tri::Checked),
        ("app.public".to_string(), Tri::Checked),
        ("app.Sales".to_string(), Tri::Checked),
        ("billing".to_string(), Tri::Unchecked),
    ];
    assert_eq!(rows(vcx), all);
    let s = section(vcx);
    assert_eq!(
        strip(&vcx.cx.read(|cx| schemas::summary_text(&s, cx))),
        "3 of 3 schemas"
    );

    // Tree → pattern: a schema unchecked; the capital name is quoted, so it selects exactly
    // `Sales` (a bare `Sales` would be folded to `sales`).
    with_section(vcx, |s, window, cx| s.toggle_row(1, window, cx));
    assert_eq!(pattern(vcx), "@:public,\"Sales\"");
    assert_eq!(rows(vcx)[0], ("app".to_string(), Tri::Mixed));
    assert_eq!(rows(vcx)[1], ("app.analytics".to_string(), Tri::Unchecked));
    assert_eq!(
        strip(&vcx.cx.read(|cx| schemas::summary_text(&s, cx))),
        "2 of 3 schemas"
    );

    // Pattern → tree: what is typed stays as typed; the tree follows it.
    type_pattern(vcx, "@:analytics");
    assert_eq!(pattern(vcx), "@:analytics");
    assert_eq!(
        rows(vcx),
        vec![
            ("app".to_string(), Tri::Mixed),
            ("app.analytics".to_string(), Tri::Checked),
            ("app.public".to_string(), Tri::Unchecked),
            ("app.Sales".to_string(), Tri::Unchecked),
            ("billing".to_string(), Tri::Unchecked),
        ]
    );
    // A bare `sales` selects the folded name, which `Sales` is not.
    type_pattern(vcx, "@:sales");
    assert_eq!(rows(vcx)[3], ("app.Sales".to_string(), Tri::Unchecked));
    type_pattern(vcx, "@:\"Sales\"");
    assert_eq!(rows(vcx)[3], ("app.Sales".to_string(), Tri::Checked));

    // The database's box checks the whole database again: the pattern the tree writes reads
    // back as the same tree.
    with_section(vcx, |s, window, cx| s.toggle_row(0, window, cx));
    assert_eq!(pattern(vcx), "@:*");
    assert_eq!(rows(vcx), all);
}

/// Fix round 1 (I-3): a click in the list never drops what it cannot show (a regex, an
/// exclusion, a name not listed): it adds or removes one clause for its row, and asks before
/// writing the pattern again when that cannot work.
#[gpui::test]
fn a_tree_click_keeps_terms_the_list_cannot_show(cx: &mut TestAppContext) {
    let (vcx, id) = shop_shell(cx, "en");
    open(vcx, DialogMode::Edit(id));
    let rewrite = |vcx: &VisualTestContext| read_section(vcx, |s, _| s.rewrite_question_open());
    // An exclusion: unchecking `public` adds one, checking it again removes it.
    type_pattern(vcx, "@:*|!@:{tmp_.*}");
    assert_eq!(rows(vcx)[2], ("app.public".to_string(), Tri::Checked));
    with_section(vcx, |s, window, cx| s.toggle_row(2, window, cx));
    assert_eq!(pattern(vcx), "@:*|!@:{tmp_.*}|!@:public");
    assert_eq!(rows(vcx)[2], ("app.public".to_string(), Tri::Unchecked));
    with_section(vcx, |s, window, cx| s.toggle_row(2, window, cx));
    assert_eq!(pattern(vcx), "@:*|!@:{tmp_.*}");
    // A name not listed stays when a listed one is checked.
    type_pattern(vcx, "@:public,reports");
    with_section(vcx, |s, window, cx| s.toggle_row(1, window, cx));
    assert_eq!(pattern(vcx), "@:public,reports|@:analytics");
    assert_eq!(rows(vcx)[1], ("app.analytics".to_string(), Tri::Checked));
    // An exclusion that a clause cannot undo (a regex removes `public`): asked first.
    type_pattern(vcx, "@:*|!@:{p.*}");
    assert_eq!(rows(vcx)[2], ("app.public".to_string(), Tri::Unchecked));
    with_section(vcx, |s, window, cx| s.toggle_row(2, window, cx));
    assert!(rewrite(vcx));
    assert_eq!(pattern(vcx), "@:*|!@:{p.*}", "nothing changed yet");
    with_section(vcx, |s, window, cx| s.answer_rewrite(false, window, cx));
    assert!(!rewrite(vcx));
    assert_eq!(pattern(vcx), "@:*|!@:{p.*}");
    with_section(vcx, |s, window, cx| s.toggle_row(2, window, cx));
    with_section(vcx, |s, window, cx| s.answer_rewrite(true, window, cx));
    assert_eq!(pattern(vcx), "@:*");
    assert_eq!(rows(vcx)[2], ("app.public".to_string(), Tri::Checked));
}

#[gpui::test]
fn syntax_error_points_at_the_column_in_scalars(cx: &mut TestAppContext) {
    let (vcx, id) = shop_shell(cx, "en");
    open(vcx, DialogMode::Edit(id));
    enable_registry(vcx);
    // "müş{": the brace is the 4th character (6th byte).
    type_pattern(vcx, "müş{");
    let err = read_section(vcx, |s, _| s.error().cloned()).expect("the pattern does not parse");
    assert_eq!(err.column, 4);
    let line = text_of(vcx, "scope-pattern-error").expect("the error line is drawn");
    assert_eq!(
        line,
        "The pattern has an error at character 4: a brace isn't closed."
    );
    // The mock's case: a schema name missing after ":" (character 20).
    type_pattern(vcx, "@:public|analytics:");
    let line = text_of(vcx, "scope-pattern-error").expect("the error line is drawn");
    assert_eq!(
        line,
        "Expected a schema name after \u{201c}:\u{201d} (character 20)."
    );
    // A pattern that does not parse is never saved: Save does nothing, the dialog stays.
    save(vcx);
    assert!(dialog_open(vcx));
    assert_eq!(data(vcx, id).expect("read").scope, None);
    // Fixed, it saves.
    type_pattern(vcx, "@:public");
    assert!(text_of(vcx, "scope-pattern-error").is_none());
    save(vcx);
    wait_until(vcx, "the dialog to close", |vcx| !dialog_open(vcx));
    assert_eq!(
        file_data(vcx, id).expect("written").scope.as_deref(),
        Some("@:public")
    );
}

#[gpui::test]
fn pg_all_databases_prompts_once(cx: &mut TestAppContext) {
    let (vcx, id) = shop_shell(cx, "en");
    open(vcx, DialogMode::Edit(id));
    let question = |vcx: &VisualTestContext| read_section(vcx, |s, _| s.cost_question_open());
    // All databases asks first and changes nothing yet; Save waits for the answer.
    with_section(vcx, |s, window, cx| s.toggle_all_databases(window, cx));
    assert!(question(vcx));
    assert_eq!(pattern(vcx), "@:*");
    save(vcx);
    assert!(dialog_open(vcx));
    // Keep the current database: nothing changes.
    with_section(vcx, |s, window, cx| s.answer_cost(false, window, cx));
    assert!(!question(vcx));
    assert_eq!(pattern(vcx), "@:*");
    // Asked again until it is answered with Read all.
    with_section(vcx, |s, window, cx| s.toggle_all_databases(window, cx));
    assert!(question(vcx));
    with_section(vcx, |s, window, cx| s.answer_cost(true, window, cx));
    assert_eq!(pattern(vcx), "*:*");
    assert!(read_section(vcx, |s, _| s.all_databases()));
    // Once answered, never again in this dialog: off and on, or another database's box.
    with_section(vcx, |s, window, cx| s.toggle_all_databases(window, cx));
    assert_eq!(pattern(vcx), "@:*");
    let billing = rows(vcx)
        .iter()
        .position(|(n, _)| n == "billing")
        .expect("billing is listed");
    with_section(vcx, |s, window, cx| s.toggle_row(billing, window, cx));
    assert!(!question(vcx));
    assert_eq!(pattern(vcx), "@:*|billing:*");

    // A typed pattern that reaches another database asks too (a new dialog); Keep restores
    // the pattern as it was.
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!dialog_open(vcx));
    open(vcx, DialogMode::Edit(id));
    type_pattern(vcx, "*:*");
    assert!(question(vcx));
    with_section(vcx, |s, window, cx| s.answer_cost(false, window, cx));
    assert_eq!(pattern(vcx), "@:*");
}

// ---- tests: Encrypt cached metadata -------------------------------------------------------------

/// Opens the New dialog and waits for its Schemas section.
fn open_new(vcx: &VisualTestContext) {
    in_window(vcx, |window, cx| {
        open_connection_dialog(DialogMode::New, window, cx)
    });
    wait_until(vcx, "the Schemas section to be drawn", |vcx| {
        in_window(vcx, |window, cx| schemas::section_state(window, cx)).is_some()
    });
}

/// Fills a new profile's name and host.
fn fill_new(vcx: &mut VisualTestContext, name: &str, host: &str) {
    focus_field(vcx, "name");
    vcx.simulate_input(name);
    focus_field(vcx, "host");
    vcx.simulate_input(host);
    vcx.run_until_parked();
}

#[gpui::test]
fn sensitive_defaults_on_for_production_profiles(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    // A new profile derives it from its environment: on for production only (PO-Q2, R-W3-1).
    open_new(vcx);
    assert!(!draft_sensitive(vcx));
    click(vcx, "segmented-conn-env-4");
    assert!(draft_sensitive(vcx), "production turns it on");
    click(vcx, "segmented-conn-env-1");
    assert!(!draft_sensitive(vcx), "dev turns it off again");
    click(vcx, "segmented-conn-env-4");
    // Saved with production and the box untouched: derived, so encrypted.
    fill_new(vcx, "orders-prod", "10.0.4.7");
    save(vcx);
    assert!(!dialog_open(vcx));
    let prod = stored(vcx, profile_id(vcx, "orders-prod"));
    assert_eq!(prod.catalog_sensitive_set, None);
    assert!(prod.catalog_sensitive());

    // Once the box is clicked, the environment no longer moves it.
    open_new(vcx);
    click(vcx, "segmented-conn-env-4");
    with_section(vcx, |s, window, cx| s.toggle_encrypt(false, window, cx));
    // A new profile has no cache: no question.
    assert_eq!(read_section(vcx, |s, _| s.encrypt_question()), None);
    assert_eq!(draft_choice(vcx), Some(false));
    click(vcx, "segmented-conn-env-1");
    click(vcx, "segmented-conn-env-4");
    assert!(!draft_sensitive(vcx), "the user's choice stands");
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!dialog_open(vcx));

    // A saved, derived profile that becomes production in the dialog would delete its cache:
    // that asks first, and Save waits. Keep pins the value its cache has.
    let dev = add_profile(vcx, pg("shop", EnvTag::Dev));
    open(vcx, DialogMode::Edit(dev));
    click(vcx, "segmented-conn-env-4");
    assert_eq!(
        read_section(vcx, |s, _| s.encrypt_question()),
        Some(EncryptAsk::Environment(true))
    );
    save(vcx);
    assert!(dialog_open(vcx), "Save waits for the answer");
    // Back to dev: the question goes.
    click(vcx, "segmented-conn-env-1");
    assert_eq!(read_section(vcx, |s, _| s.encrypt_question()), None);
    click(vcx, "segmented-conn-env-4");
    with_section(vcx, |s, window, cx| s.answer_encrypt(false, window, cx));
    assert_eq!(draft_choice(vcx), Some(false));
    assert!(!draft_sensitive(vcx));
    save(vcx);
    assert!(!dialog_open(vcx));
    let shop = stored(vcx, dev);
    assert_eq!(shop.environment, Some(EnvTag::Production));
    assert_eq!(shop.catalog_sensitive_set, Some(false));
}

/// Ruling R-W3-1: an imported production profile writes no choice, so it encrypts.
#[gpui::test]
fn an_imported_production_profile_is_sensitive(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let path = paths(vcx).data.join("pg_service.conf");
    write(&path, "[orders-prod]\nhost=db-prod\ndbname=orders\n");
    vcx.cx
        .update(|cx| cx.set_global(import::ImportSources(vec![path])));
    run(vcx, "connection.import_pg_service");
    wait_until(vcx, "the import dialog to open", |vcx| {
        vcx.cx.read(import::open_view).is_some()
    });
    let view = vcx.cx.read(import::open_view).expect("the import dialog");
    assert_eq!(
        vcx.cx.read(|cx| view.read(cx).env(0)),
        Some(EnvTag::Production)
    );
    vcx.cx.update(|cx| view.update(cx, |v, cx| v.confirm(cx)));
    wait_until(vcx, "the import to save", |vcx| {
        vcx.cx
            .read(|cx| cx.global::<ConnectionsModel>().profiles().len())
            == 1
    });
    let p = stored(vcx, profile_id(vcx, "orders-prod"));
    assert_eq!(p.catalog_sensitive_set, None);
    assert!(p.catalog_sensitive());
    quiet(vcx);
    let text = std::fs::read_to_string(paths(vcx).connections_toml()).expect("connections.toml");
    assert!(!text.contains("catalog_sensitive"), "{text}");
}

/// Fake cache files of `id` (every name `CacheStore::delete_profile_files` owns), and one of
/// another profile that must stay.
fn plant_cache(
    vcx: &VisualTestContext,
    id: ProfileId,
    other: ProfileId,
) -> Vec<std::path::PathBuf> {
    let p = paths(vcx);
    let mine = vec![
        p.catalog_db(&id.0.to_string(), 1),
        p.catalog_db(&id.0.to_string(), 2),
    ];
    let theirs = p.catalog_db(&other.0.to_string(), 1);
    for f in mine.iter().chain([&theirs]) {
        std::fs::create_dir_all(f.parent().expect("a folder")).expect("cache folder");
        std::fs::write(f, b"cache").expect("cache file");
    }
    mine
}

#[gpui::test]
fn turning_encryption_on_or_off_confirms_and_drops_the_cache(cx: &mut TestAppContext) {
    let (vcx, id) = shop_shell(cx, "en");
    let other = add_profile(vcx, pg("other", EnvTag::Dev));
    let mine = plant_cache(vcx, id, other);
    let theirs = paths(vcx).catalog_db(&other.0.to_string(), 1);
    open(vcx, DialogMode::Edit(id));
    let ask = |vcx: &VisualTestContext| read_section(vcx, |s, _| s.encrypt_question());

    // On: asks first, focused on the answer that changes nothing.
    with_section(vcx, |s, window, cx| s.toggle_encrypt(true, window, cx));
    assert_eq!(ask(vcx), Some(EncryptAsk::Toggle(true)));
    assert!(!draft_sensitive(vcx));
    with_section(vcx, |s, window, cx| s.answer_encrypt(false, window, cx));
    assert_eq!(ask(vcx), None);
    assert!(!draft_sensitive(vcx));
    // Apply: the draft changes; the files go only when the dialog saves.
    with_section(vcx, |s, window, cx| s.toggle_encrypt(true, window, cx));
    with_section(vcx, |s, window, cx| s.answer_encrypt(true, window, cx));
    assert!(draft_sensitive(vcx));
    assert!(
        mine.iter().all(|f| f.exists()),
        "nothing deleted before Save"
    );
    save(vcx);
    assert!(!dialog_open(vcx));
    assert_eq!(stored(vcx, id).catalog_sensitive_set, Some(true));
    assert!(mine.iter().all(|f| !f.exists()), "the cache is gone");
    assert!(theirs.exists(), "another profile's cache stays");

    // Off: asks with the other sentence, and the encrypted cache goes before the flag does.
    let mine = plant_cache(vcx, id, other);
    open(vcx, DialogMode::Edit(id));
    with_section(vcx, |s, window, cx| s.toggle_encrypt(false, window, cx));
    assert_eq!(ask(vcx), Some(EncryptAsk::Toggle(false)));
    with_section(vcx, |s, window, cx| s.answer_encrypt(true, window, cx));
    save(vcx);
    assert!(!dialog_open(vcx));
    assert_eq!(stored(vcx, id).catalog_sensitive_set, Some(false));
    assert!(mine.iter().all(|f| !f.exists()));
    assert!(theirs.exists());
}

/// Fix round 1 (I-4): every Save step made to fail, in both directions, leaves the stored
/// flag and the cache files consistent: never "encrypted" with files written in the clear
/// still on disk, never "plaintext" with the encrypted cache still on disk.
#[gpui::test]
fn save_fails_closed_at_each_step(cx: &mut TestAppContext) {
    let (vcx, id) = shop_shell(cx, "en");
    let other = add_profile(vcx, pg("other", EnvTag::Dev));
    let steps = [
        (SaveFault::CacheDelete, "hv-scope-save-cache-failed", true),
        (SaveFault::ScopeWrite, "hv-scope-save-data-failed", false),
        (SaveFault::ProfileSave, "hv-conn-dlg-save-failed", false),
    ];
    for to in [true, false] {
        for (fault, message, files_stay) in steps {
            let before = stored(vcx, id).catalog_sensitive();
            assert_eq!(before, !to);
            let mine = plant_cache(vcx, id, other);
            open(vcx, DialogMode::Edit(id));
            // A scope of this step's own, so a write of it is told apart from earlier steps'.
            let scope = format!("@:\"{fault:?}-{to}\"");
            type_pattern(vcx, &scope);
            with_section(vcx, |s, window, cx| s.toggle_encrypt(to, window, cx));
            with_section(vcx, |s, window, cx| s.answer_encrypt(true, window, cx));
            vcx.cx
                .update(|cx| schemas::inject_save_fault(cx, Some(fault)));
            save(vcx);
            // Nothing was saved: the dialog stays, saying what failed.
            assert!(dialog_open(vcx), "{fault:?}");
            let expected = vcx.cx.read(|cx| {
                cx.global::<hacivat_app::l10n_global::L10nGlobal>()
                    .l10n
                    .t_dynamic(message, Vec::new())
            });
            assert_eq!(
                problem(vcx).as_deref(),
                Some(expected.as_str()),
                "{fault:?}"
            );
            quiet(vcx);
            let now = stored(vcx, id);
            assert_eq!(now.catalog_sensitive(), before, "{fault:?}: the flag stays");
            // The files either stay with the flag they were written under, or are gone.
            assert_eq!(
                mine.iter().all(|f| f.exists()),
                files_stay,
                "{fault:?} to {to}"
            );
            if !files_stay {
                assert!(mine.iter().all(|f| !f.exists()), "{fault:?} to {to}");
            }
            if fault != SaveFault::ProfileSave {
                assert_ne!(
                    file_data(vcx, id).and_then(|d| d.scope),
                    Some(scope.clone()),
                    "{fault:?}: the scope is not written either"
                );
            }
            vcx.cx.update(|cx| schemas::inject_save_fault(cx, None));
            vcx.simulate_keystrokes("escape");
            vcx.run_until_parked();
            assert!(!dialog_open(vcx));
        }
        // Then it saves.
        let mine = plant_cache(vcx, id, other);
        open(vcx, DialogMode::Edit(id));
        with_section(vcx, |s, window, cx| s.toggle_encrypt(to, window, cx));
        with_section(vcx, |s, window, cx| s.answer_encrypt(true, window, cx));
        save(vcx);
        assert!(!dialog_open(vcx));
        assert_eq!(stored(vcx, id).catalog_sensitive(), to);
        assert!(mine.iter().all(|f| !f.exists()));
    }
}

/// Fix round 1 (M-8): a profile changed while its dialog is open is not overwritten by the
/// dialog's old "Encrypt cached metadata", and no cache is deleted for a change the user did
/// not make.
#[gpui::test]
fn save_reads_the_stored_flag_then(cx: &mut TestAppContext) {
    let (vcx, id) = shop_shell(cx, "en");
    let other = add_profile(vcx, pg("other", EnvTag::Dev));
    let mine = plant_cache(vcx, id, other);
    open(vcx, DialogMode::Edit(id));
    type_pattern(vcx, "@:public");
    // Meanwhile the profile is saved with "on" (another window, a reloaded file).
    let mut changed = (*stored(vcx, id)).clone();
    changed.catalog_sensitive_set = Some(true);
    let task = vcx
        .cx
        .update(|cx| ConnectionsModel::save(cx, ProfileDraft::edit(changed)));
    wait_task(vcx, task).expect("the profile saves");
    save(vcx);
    assert!(!dialog_open(vcx));
    assert_eq!(stored(vcx, id).catalog_sensitive_set, Some(true));
    assert!(mine.iter().all(|f| f.exists()), "no cache deleted");
}

#[gpui::test]
fn profile_writes_go_through_connections_model_only(cx: &mut TestAppContext) {
    let (vcx, id) = shop_shell(cx, "en");
    let file = paths(vcx).connections_toml();
    let before = connections_file(vcx, "shop");
    // Changes in the dialog write nothing until Save.
    open(vcx, DialogMode::Edit(id));
    type_pattern(vcx, "@:public");
    with_section(vcx, |s, window, cx| s.toggle_encrypt(true, window, cx));
    with_section(vcx, |s, window, cx| s.answer_encrypt(true, window, cx));
    quiet(vcx);
    assert_eq!(std::fs::read(&file).expect("connections.toml"), before);
    assert!(!profile_file(vcx, id).exists());
    with_section(vcx, |s, window, cx| s.toggle_encrypt(false, window, cx));
    assert!(
        !draft_sensitive(vcx),
        "back to the saved value: no question"
    );
    assert_eq!(draft_choice(vcx), None, "and its derived setting again");

    // Save: the scope goes to the profile's data file, the profile through ConnectionsModel
    // (its list and its file agree).
    save(vcx);
    assert!(!dialog_open(vcx));
    assert_eq!(
        file_data(vcx, id).expect("written").scope.as_deref(),
        Some("@:public")
    );
    quiet(vcx);
    assert_eq!(std::fs::read(&file).expect("connections.toml"), before);

    // A level pill writes the data file only, never connections.toml.
    let model_before = stored(vcx, id);
    set_level_on(vcx, "app", Some(Level::Sources));
    wait_until(vcx, "the level to be written", |vcx| {
        file_data(vcx, id).is_some_and(|d| !d.level_overrides.is_empty())
    });
    quiet(vcx);
    assert_eq!(std::fs::read(&file).expect("connections.toml"), before);
    assert!(Arc::ptr_eq(&model_before, &stored(vcx, id)));
    assert_eq!(
        file_data(vcx, id).expect("written").scope.as_deref(),
        Some("@:public"),
        "the scope stays"
    );

    // A new profile with the box on: one ConnectionsModel save carries the choice.
    open_new(vcx);
    click(vcx, "segmented-conn-env-1");
    with_section(vcx, |s, window, cx| s.toggle_encrypt(true, window, cx));
    fill_new(vcx, "audit", "10.0.4.8");
    save(vcx);
    assert!(!dialog_open(vcx));
    let audit = profile_id(vcx, "audit");
    assert_eq!(stored(vcx, audit).catalog_sensitive_set, Some(true));
    wait_until(vcx, "connections.toml to hold the flag", |_| {
        std::fs::read_to_string(&file).is_ok_and(|t| t.contains("catalog_sensitive = true"))
    });
}

#[gpui::test]
fn escape_cancels_without_saving(cx: &mut TestAppContext) {
    let (vcx, id) = shop_shell(cx, "en");
    let file = paths(vcx).connections_toml();
    let before = connections_file(vcx, "shop");
    open(vcx, DialogMode::Edit(id));
    with_section(vcx, |s, window, cx| s.toggle_encrypt(true, window, cx));
    with_section(vcx, |s, window, cx| s.answer_encrypt(true, window, cx));
    type_pattern(vcx, "@:public");
    // Escape in the pattern field cancels the dialog.
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!dialog_open(vcx));
    quiet(vcx);
    assert!(!stored(vcx, id).catalog_sensitive());
    assert_eq!(std::fs::read(&file).expect("connections.toml"), before);
    assert!(!profile_file(vcx, id).exists());
    assert_eq!(data(vcx, id).expect("read").scope, None);
}

// ---- tests: the profile data file --------------------------------------------------------------

/// Fix round 1 (I-1): a data file that cannot be read (here a folder at its path) is read
/// once, shown as unreadable, and never read again in a loop.
#[gpui::test]
fn a_failed_read_of_the_data_file_is_final(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let p = pg("shop", EnvTag::Dev);
    let id = p.id;
    std::fs::create_dir_all(paths(vcx).profile_toml(&id.0.to_string())).expect("a folder");
    add_profile(vcx, p);
    show(vcx, id, shop());
    wait_until(vcx, "the read to fail", |vcx| {
        vcx.cx.read(|cx| userdata::is_unreadable(id, cx))
    });
    quiet(vcx);
    let (requests, generation) = vcx
        .cx
        .read(|cx| (userdata::requests(cx), userdata::generation(cx)));
    // More frames and catalog changes: nothing is read again.
    for _ in 0..3 {
        show(vcx, id, shop());
        quiet(vcx);
    }
    assert_eq!(vcx.cx.read(userdata::requests), requests);
    assert_eq!(vcx.cx.read(userdata::generation), generation);
    // No pill is claimed, a level is refused, and the dialog says the scope can't be read.
    expand(vcx, &["shop", "app"]);
    assert!(row_of(vcx, "public").pill.is_none());
    open(vcx, DialogMode::Edit(id));
    assert!(read_section(vcx, |s, _| s.stored_unreadable()));
    type_pattern(vcx, "@:public");
    save(vcx);
    assert!(dialog_open(vcx));
    let expected = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::l10n_global::L10nGlobal>()
            .l10n
            .t_dynamic("hv-scope-save-data-unreadable", Vec::new())
    });
    assert_eq!(problem(vcx).as_deref(), Some(expected.as_str()));
    assert_eq!(
        vcx.cx.read(userdata::requests),
        requests,
        "a refused write sends nothing"
    );
}

#[gpui::test]
fn scope_section_fits_960x600_in_tr(cx: &mut TestAppContext) {
    let (vcx, id) = shop_shell(cx, "tr");
    vcx.simulate_resize(gpui::size(gpui::px(960.0), gpui::px(600.0)));
    vcx.run_until_parked();
    open(vcx, DialogMode::Edit(id));
    enable_registry(vcx);
    let check = |vcx: &mut VisualTestContext, what: &str| {
        in_window(vcx, |window, _| window.refresh());
        vcx.run_until_parked();
        let frame = registry_frame(vcx);
        let mine: Vec<_> = frame
            .iter()
            .filter(|r| r.path.contains("connections/dialog/schemas.rs"))
            .collect();
        assert!(!mine.is_empty(), "{what}: the section draws");
        // The dialog's own text: the section's, and the widgets it draws with (the shell
        // behind the modal layer is the shots' business).
        let in_dialog = |p: &str| {
            [
                "connections/dialog/",
                "hacivat-ui/src/notice.rs",
                "hacivat-ui/src/checkbox.rs",
                "hacivat-ui/src/disclosure.rs",
                "hacivat-ui/src/form.rs",
                "hacivat-ui/src/button.rs",
                "hacivat-ui/src/dialog.rs",
            ]
            .iter()
            .any(|f| p.contains(f))
        };
        for r in frame.iter().filter(|r| r.visible && in_dialog(&r.path)) {
            assert!(!r.clipped, "{what}: clipped {:?} at {}", r.text, r.path);
            assert!(
                !r.outside_card,
                "{what}: past the card {:?} at {}",
                r.text, r.path
            );
            if r.source == hacivat_ui::text::TextSource::Fluent {
                assert!(!r.truncated, "{what}: truncated {:?} at {}", r.text, r.path);
            }
        }
    };
    // Both questions open, the encryption one in view.
    with_section(vcx, |s, window, cx| s.toggle_all_databases(window, cx));
    with_section(vcx, |s, window, cx| s.toggle_encrypt(true, window, cx));
    check(vcx, "questions");
    with_section(vcx, |s, window, cx| s.answer_cost(false, window, cx));
    with_section(vcx, |s, window, cx| s.answer_encrypt(false, window, cx));
    // The error line, in view under the focused field.
    type_pattern(vcx, "@:public|analytics:");
    check(vcx, "error");
    assert!(
        text_of(vcx, "scope-pattern-error").is_some_and(|t| t.contains("20")),
        "the Turkish error names the character"
    );
}

// ---- tests: level pills -----------------------------------------------------------------------

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

fn shown(vcx: &VisualTestContext) -> Vec<ShownRow> {
    vcx.run_until_parked();
    vcx.cx.read(explorer::shown_rows)
}

fn key_of(vcx: &VisualTestContext, label: &str) -> TreeKey {
    shown(vcx)
        .into_iter()
        .find(|r| strip(&r.data.label) == label && r.status.is_none())
        .unwrap_or_else(|| panic!("`{label}` is shown"))
        .key
}

fn row_of(vcx: &VisualTestContext, label: &str) -> TreeRowData {
    shown(vcx)
        .into_iter()
        .find(|r| strip(&r.data.label) == label && r.status.is_none())
        .unwrap_or_else(|| panic!("`{label}` is shown"))
        .data
}

fn expand(vcx: &VisualTestContext, labels: &[&str]) {
    for l in labels {
        let k = key_of(vcx, l);
        vcx.cx.update(|cx| {
            let tree = explorer::tree(cx).expect("the tree");
            tree.update(cx, |t, cx| t.set_expanded(k, true, cx));
        });
        vcx.run_until_parked();
    }
}

/// Runs the level command for `level` on the row labelled `label`.
fn set_level_on(vcx: &VisualTestContext, label: &str, level: Option<Level>) {
    if shown(vcx).iter().all(|r| strip(&r.data.label) != label) {
        expand(vcx, &["shop"]);
    }
    let key = key_of(vcx, label);
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.select(key, cx));
    });
    vcx.run_until_parked();
    let (cmd, _) = level_pill::LEVEL_COMMANDS
        .iter()
        .find(|(_, l)| *l == level)
        .expect("a level command");
    assert!(run(vcx, cmd), "{cmd} runs");
}

#[gpui::test]
fn level_override_shows_explicit_and_inherited_pills(cx: &mut TestAppContext) {
    let (vcx, _id) = shop_shell(cx, "en");
    expand(vcx, &["shop", "app"]);
    // Automatic: grey, the level the schema was read at, for its major objects. A database
    // shows no pill until a level is set on it; nor does a schema not read yet.
    let public = row_of(vcx, "public").pill.expect("an automatic pill");
    assert!(!public.explicit);
    assert_eq!(public.text.as_ref(), "Structure");
    assert_eq!(
        strip(&public.tooltip),
        "Introspection level: Structure, chosen automatically for 2 objects."
    );
    assert!(row_of(vcx, "app").pill.is_none());
    // The level commands are disabled on rows without a level (a profile root).
    let root = key_of(vcx, "shop");
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.select(root, cx));
    });
    assert!(!run(vcx, "explorer.set_level.names"));

    // Set on the database: blue there, grey "inherited" on its schemas.
    set_level_on(vcx, "app", Some(Level::Sources));
    let app = row_of(vcx, "app").pill.expect("the database's pill");
    assert!(app.explicit);
    assert_eq!(app.text.as_ref(), "Sources");
    assert_eq!(
        app.tooltip.as_ref(),
        "Introspection level: Structure and sources, set here. Schemas below inherit it."
    );
    for schema in ["analytics", "public", "Sales"] {
        let pill = row_of(vcx, schema).pill.expect("an inherited pill");
        assert!(!pill.explicit, "{schema}");
        assert_eq!(pill.text.as_ref(), "Sources");
        assert_eq!(
            strip(&pill.tooltip),
            "Introspection level: Structure and sources, inherited from app."
        );
    }
    // Set on a schema: blue there only.
    set_level_on(vcx, "Sales", Some(Level::Names));
    let sales = row_of(vcx, "Sales").pill.expect("the schema's pill");
    assert!(sales.explicit);
    assert_eq!(sales.text.as_ref(), "Names");
    assert!(!row_of(vcx, "public").pill.expect("inherited").explicit);
    // Automatic on the schema: inherited again.
    set_level_on(vcx, "Sales", None);
    let sales = row_of(vcx, "Sales").pill.expect("inherited again");
    assert!(!sales.explicit);
    assert_eq!(sales.text.as_ref(), "Sources");
    // The list the pill opens: four rows, the current setting checked.
    let key = key_of(vcx, "app");
    let target = vcx
        .cx
        .read(|cx| level_pill::target_of(key, cx))
        .expect("a level target");
    let items = level_pill::menu_items(&target);
    let checked: Vec<_> = items.iter().map(|i| i.checked).collect();
    assert_eq!(checked, vec![false, false, false, true]);
    let actions: Vec<_> = items.iter().map(|i| i.action.to_string()).collect();
    assert_eq!(
        actions,
        vec![
            "explorer.set_level.auto",
            "explorer.set_level.names",
            "explorer.set_level.structure",
            "explorer.set_level.sources",
        ]
    );
}

#[gpui::test]
fn edit_scope_opens_the_section_of_the_cursor_rows_profile(cx: &mut TestAppContext) {
    let (vcx, id) = shop_shell(cx, "en");
    expand(vcx, &["shop", "app"]);
    let key = key_of(vcx, "public");
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.select(key, cx));
    });
    assert!(run(vcx, "explorer.edit_scope"));
    let d = dialog(vcx).expect("the edit dialog");
    assert_eq!(vcx.cx.read(|cx| d.read(cx).profile().id), id);
    assert!(read_section(vcx, |s, _| s.is_open()));
    let input = read_section(vcx, |s, _| s.pattern_input());
    let handle = vcx.cx.read(|cx| input.read(cx).focus_handle(cx));
    assert_eq!(focused(vcx), Some(handle), "the pattern has the focus");
}

#[gpui::test]
fn level_override_persists_in_the_user_data_file(cx: &mut TestAppContext) {
    let (vcx, id) = shop_shell(cx, "en");
    expand(vcx, &["shop", "app"]);
    set_level_on(vcx, "public", Some(Level::Names));
    set_level_on(vcx, "app", Some(Level::Structure));
    wait_until(vcx, "both levels to be written", |vcx| {
        file_data(vcx, id).is_some_and(|d| d.level_overrides.len() == 2)
    });
    let on_disk = file_data(vcx, id).expect("written");
    let levels: Vec<(String, Level)> = on_disk
        .level_overrides
        .iter()
        .map(|(k, l)| (k.path.join("."), *l))
        .collect();
    assert_eq!(
        levels,
        vec![
            ("app".to_string(), Level::Structure),
            ("app.public".to_string(), Level::Names),
        ]
    );
    // Two changes in a row land in order (the last one wins).
    set_level_on(vcx, "public", Some(Level::Sources));
    set_level_on(vcx, "public", None);
    wait_until(vcx, "the clear to be written", |vcx| {
        file_data(vcx, id).is_some_and(|d| d.level_overrides.len() == 1)
    });
    // Read again from the file (a restart): the pills come back.
    vcx.cx.update(userdata::forget_all);
    vcx.run_until_parked();
    in_window(vcx, |window, _| window.refresh());
    wait_data(vcx, id);
    let app = row_of(vcx, "app").pill.expect("the database's pill");
    assert!(app.explicit);
    assert_eq!(app.text.as_ref(), "Structure");
    assert!(!row_of(vcx, "public").pill.expect("inherited").explicit);
}
