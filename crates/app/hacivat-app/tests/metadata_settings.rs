//! M4 Task 27: Settings › Metadata (design/m4-explorer surface 13), "Clear cached metadata…",
//! the persisted "Introspect live" choice (T26 carry) and the M3c `show_section` carry.

mod support;

use std::sync::Arc;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::explorer;
use hacivat_app::explorer::adapter;
use hacivat_app::explorer::live::{self as hlive, CatalogSource};
use hacivat_app::explorer::userdata;
use hacivat_app::settings_ui::rows::{self, Section};
use hacivat_app::settings_ui::{metadata, settings_rows};
use hacivat_app::shell::flush_pending_writes;
use hacivat_driver::fake::FakeDriver;
use hacivat_introspect::testing::fake::FakeIntrospector;
use hacivat_meta::snapshot::ContainerKey;
use hacivat_meta::{LoadState, NsKey};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_settings::{LocalPaths, RefreshAfterDdl, ValidateInterval, ValidateMinutes};
use support::live::*;
use support::*;

// ---- helpers ----------------------------------------------------------------------------------

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

/// A shell over a fresh root in `lang` with the settings tab open.
fn settings_tab(
    cx: &mut TestAppContext,
    lang: &str,
) -> (&'static mut VisualTestContext, LocalPaths) {
    // The temp root lives until the test ends (the temp-dir hygiene rule).
    let root = temp_root();
    let paths = LocalPaths::under(&root);
    write(
        paths.settings_toml(),
        "version = 1\n\n[ui]\ntime_zone = \"UTC\"\n",
    );
    let vcx = shell_at(cx, &root, lang);
    with_ws(vcx, |w, window, cx| {
        w.open_registered_tab("settings", window, cx)
    });
    vcx.run_until_parked();
    (vcx, paths)
}

fn selector(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn bounds(vcx: &mut VisualTestContext, name: &str) -> gpui::Bounds<gpui::Pixels> {
    vcx.debug_bounds(selector(name.to_string()))
        .unwrap_or_else(|| panic!("`{name}` is not on screen"))
}

fn click_at(vcx: &mut VisualTestContext, name: &str) {
    let at = bounds(vcx, name).center();
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
}

fn show_metadata(vcx: &mut VisualTestContext) {
    let index = Section::ALL
        .iter()
        .position(|s| *s == Section::Metadata)
        .expect("the Metadata section");
    click_at(vcx, &format!("settings-nav-{index}"));
}

fn label_with_id(vcx: &VisualTestContext, id: &str) -> Option<String> {
    let needle = format!("#{id}");
    registry_frame(vcx)
        .into_iter()
        .find(|r| r.path.ends_with(&needle))
        .map(|r| r.text.to_string())
}

fn settings(vcx: &VisualTestContext) -> hacivat_settings::Settings {
    vcx.cx
        .read(|cx| cx.global::<hacivat_app::AppState>().settings.clone())
}

fn file(vcx: &VisualTestContext, paths: &LocalPaths) -> String {
    vcx.cx.update(flush_pending_writes);
    vcx.run_until_parked();
    std::fs::read_to_string(paths.settings_toml()).expect("settings.toml")
}

/// Replaces the text of the field `name` with `text` and presses Enter.
fn type_in(vcx: &mut VisualTestContext, name: &str, text: &str) {
    click_at(vcx, name);
    // Select-all is cmd-a on macOS and ctrl-a elsewhere; `cmd-a` alone left "512" → "51" on
    // Linux and Windows (CI 44769983427).
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    vcx.simulate_keystrokes("backspace");
    vcx.simulate_input(text);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
}

fn public() -> NsKey {
    NsKey::schema(["app", "public"])
}

fn runtime(vcx: &VisualTestContext) -> tokio::runtime::Handle {
    vcx.cx.read(|cx| cx.global::<ConnectionsModel>().runtime())
}

// ---- the section ------------------------------------------------------------------------------

#[gpui::test]
fn the_metadata_section_lists_its_rows(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx, "en");
    show_metadata(vcx);
    assert_eq!(
        label_with_id(vcx, "settings-section-title").as_deref(),
        Some("METADATA")
    );
    let keys: Vec<&str> = metadata::metadata_rows().iter().map(|r| r.key).collect();
    assert_eq!(
        keys,
        [
            "metadata.refresh_after_ddl",
            "metadata.validate_interval_min",
            "metadata.cache_mb_per_profile",
            "metadata.cache_mb_total",
            "metadata.clear_cache",
        ]
    );
    for key in &keys {
        assert!(
            vcx.debug_bounds(selector(format!("settings-row-{key}")))
                .is_some(),
            "{key}"
        );
    }
    assert_eq!(
        label_with_id(vcx, "settings-row-note-metadata.refresh_after_ddl").as_deref(),
        Some(
            "After a console runs a statement that changes the schema. Inside a transaction it \
             waits for COMMIT."
        )
    );
    // Search names the rows (Turkish-tolerant, English layer).
    let found: Vec<&'static str> = vcx.cx.read(|cx| {
        let l10n = &cx.global::<hacivat_app::l10n_global::L10nGlobal>().l10n;
        let rows: Vec<_> = settings_rows()
            .iter()
            .map(|r| hacivat_app::settings_ui::SearchRow::of(r, l10n))
            .collect();
        hacivat_app::settings_ui::filter_rows(
            "cache",
            &rows,
            &hacivat_l10n::SearchOpts {
                turkish_tolerant: true,
                mode: hacivat_l10n::MatchMode::Contains,
            },
        )
    });
    assert_eq!(
        found,
        [
            "metadata.cache_mb_per_profile",
            "metadata.cache_mb_total",
            "metadata.clear_cache"
        ]
    );
}

#[gpui::test]
fn settings_write_through_update_settings_only(cx: &mut TestAppContext) {
    let (vcx, paths) = settings_tab(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    show_metadata(vcx);
    // Refresh after DDL: the Segmented.
    let ask = vcx
        .cx
        .read(|cx| rows::choices(cx, rows::REFRESH_AFTER_DDL))
        .iter()
        .position(|c| c.value == "ask")
        .expect("Ask");
    click_at(vcx, &format!("segmented-{}-{ask}", rows::REFRESH_AFTER_DDL));
    assert_eq!(
        settings(vcx).metadata.refresh_after_ddl,
        RefreshAfterDdl::Ask
    );
    assert!(file(vcx, &paths).contains("refresh_after_ddl = \"ask\""));
    // The background check: the Select (Automatic, Off, 5, 10, 30 min).
    click_at(vcx, "settings-metadata-check");
    click_at(vcx, "menu-item-3");
    assert_eq!(
        settings(vcx).metadata.validate_interval_min,
        ValidateInterval::Minutes(ValidateMinutes::Ten)
    );
    assert!(file(vcx, &paths).contains("validate_interval_min = 10"));
    click_at(vcx, "settings-metadata-check");
    click_at(vcx, "menu-item-1");
    assert_eq!(
        settings(vcx).metadata.validate_interval_min,
        ValidateInterval::Off
    );
    // The cache sizes: a number and a unit; saved in MiB.
    type_in(vcx, "settings-metadata-cache-conn-field", "256");
    assert_eq!(settings(vcx).metadata.cache_mb_per_profile.get(), 256);
    assert!(file(vcx, &paths).contains("cache_mb_per_profile = 256"));
    // 4 GB shows as 4 GB; 8 typed there is 8 GB.
    type_in(vcx, "settings-metadata-cache-total-field", "8");
    assert_eq!(settings(vcx).metadata.cache_mb_total.get(), 8192);
    assert!(file(vcx, &paths).contains("cache_mb_total = 8192"));
    // Not a number: nothing is written, the saved size shows again.
    type_in(vcx, "settings-metadata-cache-conn-field", "lots");
    assert_eq!(settings(vcx).metadata.cache_mb_per_profile.get(), 256);
    // Above 1 TiB: the bound.
    type_in(vcx, "settings-metadata-cache-conn-field", "5000000");
    assert_eq!(settings(vcx).metadata.cache_mb_per_profile.get(), 1 << 20);
    // The shell's settings and the file agree (one writer, in order).
    let on_disk = hacivat_settings::load(&paths.settings_toml())
        .settings
        .metadata;
    assert_eq!(on_disk, settings(vcx).metadata);
}

#[test]
fn sizes_parse_and_show() {
    use metadata::{CacheUnit, parse_size, shown_size};
    assert_eq!(shown_size(512), (512, CacheUnit::Mb));
    assert_eq!(shown_size(4096), (4, CacheUnit::Gb));
    assert_eq!(shown_size(1500), (1500, CacheUnit::Mb));
    assert_eq!(parse_size("2", CacheUnit::Gb).map(|s| s.get()), Some(2048));
    assert_eq!(
        parse_size("1.024", CacheUnit::Mb).map(|s| s.get()),
        Some(1024)
    );
    assert_eq!(parse_size("0", CacheUnit::Mb).map(|s| s.get()), Some(1));
    assert_eq!(parse_size("x1", CacheUnit::Mb), None);
    assert_eq!(parse_size("", CacheUnit::Mb), None);
    assert_eq!(metadata::mb_tenths(40_265_318), "38.4");
    assert_eq!(metadata::mb_tenths(0), "0.0");
}

#[gpui::test]
fn metadata_settings_fit_960x600_in_tr(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx, "tr");
    vcx.simulate_resize(gpui::size(gpui::px(960.0), gpui::px(600.0)));
    vcx.run_until_parked();
    show_metadata(vcx);
    assert_eq!(
        label_with_id(vcx, "settings-section-title").as_deref(),
        Some("ÜST VERİ")
    );
    let window = gpui::px(960.0);
    let mut controls: Vec<String> = (0..3)
        .map(|i| format!("segmented-{}-{i}", rows::REFRESH_AFTER_DDL))
        .collect();
    controls.extend(
        [
            "settings-metadata-check",
            "settings-metadata-cache-conn-field",
            "settings-metadata-cache-conn-unit",
            "settings-metadata-cache-total-field",
            "settings-metadata-cache-total-unit",
            "settings-metadata-clear",
        ]
        .map(String::from),
    );
    for name in &controls {
        let b = bounds(vcx, name);
        assert!(b.right() <= window, "{name} {b:?}");
        assert!(b.size.width > gpui::px(0.0), "{name} {b:?}");
    }
    for key in metadata::metadata_rows().iter().map(|r| r.key) {
        let b = bounds(vcx, &format!("settings-row-{key}"));
        assert!(b.right() <= window, "{key} {b:?}");
    }
    // The Segmented's labels are whole (never ellipsized).
    let drawn: Vec<String> = registry_frame(vcx)
        .into_iter()
        .map(|r| r.text.to_string())
        .collect();
    for word in ["Her zaman", "Sor", "Hiçbir zaman"] {
        assert!(drawn.iter().any(|t| t == word), "{word}: {drawn:?}");
    }
}

// ---- Clear cached metadata ----------------------------------------------------------------------

fn pg_profile(name: &str) -> hacivat_session::profile::Profile {
    let mut p = test_profile(name, Some(EnvTag::Dev), "127.0.0.1");
    p.database = Some("app".to_string());
    p
}

fn tables_of(vcx: &VisualTestContext, id: ProfileId) -> usize {
    vcx.cx.read(|cx| {
        adapter::catalog_of(id, cx)
            .and_then(|c| c.namespaces.get(&public()).cloned())
            .map_or(0, |n| n.children(0).count())
    })
}

fn loaded(vcx: &VisualTestContext, id: ProfileId) -> bool {
    vcx.cx.read(|cx| {
        adapter::catalog_of(id, cx).is_some_and(|c| {
            matches!(
                c.states.get(&ContainerKey::Namespace(public())),
                Some(LoadState::Loaded { stale: false, .. })
            )
        })
    })
}

#[gpui::test]
fn clear_cache_confirms_deletes_and_revalidates(cx: &mut TestAppContext) {
    let intro = FakeIntrospector::new(DbmsKind::Postgres);
    intro.put_ns(public(), fake_tables(&["users", "orders"]));
    let vcx = shell_with_fake_prepared(
        cx,
        "en",
        answers_probe(FakeDriver::new(DbmsKind::Postgres)),
        |_| {},
    );
    in_window(vcx, |window, _| window.activate_window());
    let dir = temp_dir(vcx).join("fake-cache");
    let src = Arc::new(FakeSource::new(runtime(vcx), intro, dir));
    let as_source: Arc<dyn CatalogSource> = src.clone();
    vcx.cx.update(|cx| hlive::set_source(as_source, cx));
    let id = add_profile(vcx, pg_profile("app-dev"));
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| {
            t.set_expanded(adapter::profile_key(id), true, cx)
        });
    });
    connect(vcx, "app-dev");
    wait_until(vcx, "loaded", |vcx| {
        tables_of(vcx, id) == 2 && loaded(vcx, id)
    });
    // The profile's cache file where the app keeps it.
    let cache = paths(vcx).catalog_db(
        &id.0.to_string(),
        u32::from(hacivat_meta::codec::FORMAT_MAJOR),
    );
    std::fs::create_dir_all(cache.parent().expect("a folder")).expect("folder");
    std::fs::write(&cache, b"cached rows").expect("the cache file");
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.select(adapter::profile_key(id), cx));
    });
    vcx.run_until_parked();
    // It asks first, on Cancel; Escape cancels and nothing is deleted.
    assert!(run(vcx, "metadata.clear_cache"));
    vcx.run_until_parked();
    assert_eq!(
        strip(&dialog_title(vcx)),
        "Clear cached metadata for app-dev?"
    );
    assert_eq!(vcx.cx.read(metadata::clear_question), Some(id));
    // ## m4-t33b-carries (T27 carry): the size in the subtitle, once it is measured.
    wait_until(vcx, "the size in the subtitle", |vcx| {
        text_of(vcx, "dialog-subtitle")
            .map(|t| strip(&t))
            .as_deref()
            == Some("0 MB on this computer")
    });
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!layer_ids(vcx).contains(&"dialog"));
    assert!(cache.exists(), "Cancel deletes nothing");
    let ensures = src.ensures.load(std::sync::atomic::Ordering::SeqCst);
    // Review m-6: the source records whether the file still existed when the store closed.
    *src.watch.lock().unwrap() = Some(cache.clone());
    // Clear (shift-tab from Cancel, then Enter): the store closes, the file goes, the catalog
    // is read again because the profile is connected.
    assert!(run(vcx, "metadata.clear_cache"));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("shift-tab enter");
    wait_until(vcx, "the file deleted", |_| !cache.exists());
    wait_until(vcx, "the catalog opened again", |_| {
        src.ensures.load(std::sync::atomic::Ordering::SeqCst) > ensures
    });
    let log = src.log.lock().unwrap().clone();
    assert!(
        log.iter().any(|l| l == "shutdown watched=true"),
        "the store closed while the file still existed (close before delete): {log:?}"
    );
    wait_until(vcx, "revalidated", |vcx| {
        tables_of(vcx, id) == 2 && loaded(vcx, id)
    });
    assert!(
        toast_texts(vcx)
            .iter()
            .any(|t| strip(t) == "Cleared the cached metadata of app-dev"),
        "{:?}",
        toast_texts(vcx)
    );
}

// ---- carries ----------------------------------------------------------------------------------

/// T26 carry: the "Introspect live" choice for a system namespace is kept in the profile's data
/// file and read back.
#[gpui::test]
fn introspect_live_choice_persists(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let id = add_profile(vcx, pg_profile("app-dev"));
    vcx.cx.update(|cx| userdata::ensure(id, cx));
    wait_until(vcx, "the data read", |vcx| {
        vcx.cx.read(|cx| userdata::get(id, cx)).is_some()
    });
    let pg_catalog = NsKey::schema(["app", "pg_catalog"]);
    let info = NsKey::schema(["app", "information_schema"]);
    for ns in [pg_catalog.clone(), info.clone(), pg_catalog.clone()] {
        let task = vcx
            .cx
            .update(|cx| userdata::set_live_system(id, ns, true, cx));
        assert!(wait_task(vcx, task).is_ok());
    }
    assert_eq!(
        vcx.cx.read(|cx| userdata::live_system(id, cx)),
        vec![info.clone(), pg_catalog.clone()],
        "sorted, no duplicates"
    );
    // Read back from the file by a fresh reader.
    vcx.cx.update(userdata::forget_all);
    vcx.cx.update(|cx| userdata::ensure(id, cx));
    wait_until(vcx, "read again", |vcx| {
        vcx.cx.read(|cx| userdata::get(id, cx)).is_some()
    });
    assert_eq!(
        vcx.cx.read(|cx| userdata::live_system(id, cx)),
        vec![info.clone(), pg_catalog.clone()]
    );
    let task = vcx
        .cx
        .update(|cx| userdata::set_live_system(id, info, false, cx));
    assert!(wait_task(vcx, task).is_ok());
    assert_eq!(
        vcx.cx.read(|cx| userdata::live_system(id, cx)),
        vec![pg_catalog]
    );
}

/// The M3c carry (P-M4-18): the navigation's click and the shots' fixture show a section the
/// same way (one `SettingsPage::show_section`).
#[gpui::test]
fn show_section_is_the_navigations_own(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx, "en");
    vcx.cx.update(|cx| {
        hacivat_app::settings_ui::fixtures::search("theme", cx);
        hacivat_app::settings_ui::fixtures::show_section(Section::Metadata, cx);
    });
    vcx.run_until_parked();
    assert_eq!(
        vcx.cx.read(hacivat_app::settings_ui::fixtures::shown),
        Some((Section::Metadata, String::new()))
    );
    vcx.cx
        .update(|cx| hacivat_app::settings_ui::fixtures::search("cache", cx));
    vcx.run_until_parked();
    show_metadata(vcx);
    assert_eq!(
        vcx.cx.read(hacivat_app::settings_ui::fixtures::shown),
        Some((Section::Metadata, String::new()))
    );
}

// ---- fix round ----------------------------------------------------------------------------

/// A source that records the `[metadata]` settings it is given (ruling R-W5-11).
#[derive(Default)]
struct Recording(std::sync::Mutex<Vec<hacivat_settings::MetadataSettings>>);

impl CatalogSource for Recording {
    fn ensure(
        &self,
        _: Arc<hacivat_session::profile::Profile>,
    ) -> futures::future::BoxFuture<
        '_,
        Result<hacivat_introspect::hub::ProfileCatalog, hacivat_introspect::hub::HubError>,
    > {
        Box::pin(async { Err(hacivat_introspect::hub::HubError::Shutdown) })
    }

    fn shutdown(&self, _: ProfileId) -> futures::future::BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn has_cache(&self, _: ProfileId) -> bool {
        false
    }

    fn set_metadata(&self, metadata: hacivat_settings::MetadataSettings) {
        self.0.lock().unwrap().push(metadata);
    }
}

/// Ruling R-W5-11: the background check and the cache caps reach the hub when they change, so
/// the next connect uses them (the hub's own test proves it makes the next catalog with them).
#[gpui::test]
fn metadata_settings_reach_the_hub_for_the_next_connect(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let rec = Arc::new(Recording::default());
    let source: Arc<dyn CatalogSource> = rec.clone();
    vcx.cx.update(|cx| hlive::set_source(source, cx));
    show_metadata(vcx);
    click_at(vcx, "settings-metadata-check");
    click_at(vcx, "menu-item-2");
    type_in(vcx, "settings-metadata-cache-conn-field", "300");
    let seen = rec.0.lock().unwrap().clone();
    assert_eq!(seen.last(), Some(&settings(vcx).metadata), "{seen:?}");
    assert_eq!(
        seen.last().map(|m| m.validate_interval_min),
        Some(ValidateInterval::Minutes(ValidateMinutes::Five))
    );
    assert_eq!(seen.last().map(|m| m.cache_mb_per_profile.get()), Some(300));
}

/// Ruling R-W5-12 (the carries-w1 m10 write-back rule): a key of the profile data file's
/// `[metadata]` table this build does not know survives a write-back unchanged.
#[gpui::test]
fn profile_data_keeps_unknown_keys_on_write_back(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let id = add_profile(vcx, pg_profile("app-dev"));
    let file = paths(vcx).profile_toml(&id.0.to_string());
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("folder");
    std::fs::write(
        &file,
        "[metadata]\nversion = 1\nscope = \"@:*\"\nfuture_flag = true\nfuture_list = [1, 2]\n\n\
         [metadata.future_table]\nkind = \"newer\"\nlimit = 3\n\n\
         [metadata.ddl_view]\nqualify = \"Auto\"\nplacement = \"InsideTable\"\n\
         server_definitions = false\ngrants = true\nowner = true\nfuture_opt = \"kept\"\n",
    )
    .expect("the data file");
    // Whatever was read before the file was written is forgotten.
    vcx.cx.update(userdata::forget_all);
    vcx.cx.update(|cx| userdata::ensure(id, cx));
    wait_until(vcx, "the data read", |vcx| {
        vcx.cx.read(|cx| userdata::get(id, cx)).is_some()
    });
    assert_eq!(
        vcx.cx
            .read(|cx| userdata::get(id, cx))
            .and_then(|d| d.scope.clone()),
        Some("@:*".to_string())
    );
    let pg_catalog = NsKey::schema(["app", "pg_catalog"]);
    let task = vcx
        .cx
        .update(|cx| userdata::set_live_system(id, pg_catalog, true, cx));
    assert!(wait_task(vcx, task).is_ok());
    let text = std::fs::read_to_string(&file).expect("written back");
    for kept in [
        "future_flag = true",
        "future_list = [1, 2]",
        "kind = \"newer\"",
        "limit = 3",
        "scope = \"@:*\"",
        "live_system",
        // Ruling R-W5-13: a nested `ddl_view` key survives too.
        "future_opt = \"kept\"",
    ] {
        assert!(text.contains(kept), "{kept}: {text}");
    }
    // A fresh reader: the unknown keys are still there.
    vcx.cx.update(userdata::forget_all);
    vcx.cx.update(|cx| userdata::ensure(id, cx));
    wait_until(vcx, "read again", |vcx| {
        vcx.cx.read(|cx| userdata::get(id, cx)).is_some()
    });
    let extra = vcx
        .cx
        .read(|cx| userdata::get(id, cx))
        .map(|d| d.extra.clone())
        .unwrap_or_default();
    assert_eq!(
        extra.keys().map(String::as_str).collect::<Vec<_>>(),
        ["future_flag", "future_list", "future_table"]
    );
}
