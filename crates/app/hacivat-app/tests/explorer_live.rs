//! M4 Task 24: the live explorer (design/m4-explorer surfaces 1–3, 8): cached trees at start,
//! cached-then-validated on connect, refresh, the mass-deletion guard, disconnect, the cache key
//! timing (R-M4-15), progress, coalesced notify, reveal, database rows and the expanded state in
//! `workspace.json`.

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::ThreadId;

use gpui::{Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::explorer::{self, ShownRow};
use hacivat_app::explorer::adapter::{self, NodeInfo};
use hacivat_app::explorer::live::{self as hlive, CatalogSource, DatabaseRow, RevealOutcome};
use hacivat_app::explorer::mass_drop;
use hacivat_app::explorer::rows::DisplayGroup;
use hacivat_driver::fake::FakeDriver;
use hacivat_introspect::ddl_effects::Target;
use hacivat_introspect::engine::IntroHandle;
use hacivat_introspect::hub::{CacheMode, MemoryOnlyReason};
use hacivat_introspect::testing::fake::{FakeCall, FakeIntrospector};
use hacivat_meta::merge::Trigger;
use hacivat_meta::persist::Encryption;
use hacivat_meta::snapshot::{CatalogChange, ContainerKey};
use hacivat_meta::{Catalog, LoadState, NsKey, ObjectKey, ObjectKind, ServerFacts};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_secrets::{MemoryBackend, SecretBackend, SecretError, SecretKey};
use hacivat_session::profile::{Profile, ProfileStore};
use hacivat_settings::LocalPaths;
use hacivat_ui::tree::TreeKey;
use secrecy::SecretString;
use support::catalogs::*;
use support::live::*;
use support::*;

// ---- helpers ----------------------------------------------------------------------------------

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

fn rows(vcx: &VisualTestContext) -> Vec<ShownRow> {
    vcx.run_until_parked();
    vcx.cx.read(explorer::shown_rows)
}

fn labels(vcx: &VisualTestContext) -> Vec<String> {
    rows(vcx)
        .into_iter()
        .map(|r| match &r.status {
            Some(s) => format!("[{}]", strip(s)),
            None => strip(&r.data.label),
        })
        .collect()
}

fn shows(vcx: &VisualTestContext, label: &str) -> bool {
    labels(vcx).iter().any(|l| l == label)
}

fn key_of(vcx: &VisualTestContext, label: &str) -> TreeKey {
    rows(vcx)
        .into_iter()
        .find(|r| strip(&r.data.label) == label && r.status.is_none())
        .unwrap_or_else(|| panic!("`{label}` is shown in {:?}", labels(vcx)))
        .key
}

fn set_expanded(vcx: &VisualTestContext, key: TreeKey, on: bool) {
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.set_expanded(key, on, cx));
    });
    vcx.run_until_parked();
}

fn open(vcx: &VisualTestContext, labels: &[&str]) {
    for l in labels {
        let k = key_of(vcx, l);
        set_expanded(vcx, k, true);
    }
}

fn put_cursor(vcx: &VisualTestContext, key: TreeKey) {
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.select(key, cx));
    });
    vcx.run_until_parked();
}

fn pg_profile(name: &str, env: Option<EnvTag>) -> Profile {
    let mut p = test_profile(name, env, "127.0.0.1");
    p.database = Some("app".to_string());
    p
}

fn save_profiles(paths: &LocalPaths, profiles: &[Profile]) {
    let (store, _) = ProfileStore::open(paths.connections_toml());
    store.save(profiles).expect("profiles saved");
}

fn cache_file(paths: &LocalPaths, id: ProfileId) -> std::path::PathBuf {
    paths.catalog_db(
        &id.0.to_string(),
        u32::from(hacivat_meta::codec::FORMAT_MAJOR),
    )
}

/// A `workspace.json` with one console and the explorer's expanded rows of `id`.
fn workspace_json(id: ProfileId, keys: &[TreeKey]) -> String {
    let hex: Vec<String> = keys.iter().map(|k| format!("\"{:032x}\"", k.0)).collect();
    format!(
        r#"{{"version": 1,
            "tiles": {{"explorer_w": 260.0, "conn_panel_h": 180.0, "editor_h": 420.0,
                      "inspector_w": 300.0, "explorer_visible": true}},
            "panes": {{"root": {{"Leaf": 0}}, "panes": {{"0": {{"tabs": [0], "active": 0}}}},
                      "focused": 0}},
            "tabs": [{{"id": 0, "kind": "console", "n": 1}}],
            "explorer": {{"expanded": {{"{}": [{}]}}}}}}"#,
        id.0,
        hex.join(", ")
    )
}

/// Waits (bounded) until a call reached `gate`.
fn wait_gate(vcx: &VisualTestContext, gate: &hacivat_introspect::testing::fake::FakeGate) {
    let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = entered.clone();
    let g = gate.clone();
    runtime(vcx).spawn(async move {
        g.entered().await;
        flag.store(true, Ordering::SeqCst);
    });
    wait_until(vcx, "the gated call", |_| entered.load(Ordering::SeqCst));
}

fn runtime(vcx: &VisualTestContext) -> tokio::runtime::Handle {
    vcx.cx.read(|cx| cx.global::<ConnectionsModel>().runtime())
}

/// A shell over `fake` whose explorer uses a [`FakeSource`] over `intro`.
fn fake_shell(
    cx: &mut TestAppContext,
    intro: FakeIntrospector,
    prepare: impl FnOnce(&LocalPaths),
) -> (&'static mut VisualTestContext, Arc<FakeSource>) {
    let vcx = shell_with_fake_prepared(
        cx,
        "en",
        answers_probe(FakeDriver::new(DbmsKind::Postgres)),
        prepare,
    );
    let dir = temp_dir(vcx).join("fake-cache");
    let src = Arc::new(FakeSource::new(runtime(vcx), intro, dir));
    let as_source: Arc<dyn CatalogSource> = src.clone();
    vcx.cx.update(|cx| hlive::set_source(as_source, cx));
    (vcx, src)
}

fn handle(vcx: &VisualTestContext, id: ProfileId) -> IntroHandle {
    vcx.cx
        .read(|cx| hlive::handle(id, cx))
        .expect("the catalog is open")
}

fn tables_of(vcx: &VisualTestContext, id: ProfileId, ns: &NsKey) -> Vec<String> {
    vcx.cx.read(|cx| {
        adapter::catalog_of(id, cx)
            .and_then(|c| c.namespaces.get(ns).cloned())
            .map(|n| {
                n.children(0)
                    .filter(|(_, o)| o.kind() == ObjectKind::Table)
                    .map(|(_, o)| o.name().to_string())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn state_of(vcx: &VisualTestContext, id: ProfileId, ns: &NsKey) -> Option<LoadState> {
    vcx.cx.read(|cx| {
        adapter::catalog_of(id, cx)
            .and_then(|c| c.states.get(&ContainerKey::Namespace(ns.clone())).cloned())
    })
}

fn public() -> NsKey {
    NsKey::schema(["app", "public"])
}

/// A connected PostgreSQL profile `app-dev` whose `app.public` the fake server holds with
/// `tables`, loaded in the explorer.
fn connected_loaded(
    cx: &mut TestAppContext,
    tables: &[&str],
) -> (&'static mut VisualTestContext, Arc<FakeSource>, ProfileId) {
    let intro = FakeIntrospector::new(DbmsKind::Postgres);
    intro.put_ns(public(), fake_tables(tables));
    let (vcx, src) = fake_shell(cx, intro, |_| {});
    let id = add_profile(vcx, pg_profile("app-dev", Some(EnvTag::Dev)));
    set_expanded(vcx, adapter::profile_key(id), true);
    connect(vcx, "app-dev");
    let want = tables.len();
    wait_until(vcx, "the current schema loaded", |vcx| {
        tables_of(vcx, id, &public()).len() == want
            && matches!(
                state_of(vcx, id, &public()),
                Some(LoadState::Loaded { stale: false, .. })
            )
    });
    (vcx, src, id)
}

// ---- app start --------------------------------------------------------------------------------

#[gpui::test]
fn app_start_shows_cached_tree_before_any_catalog_query(cx: &mut TestAppContext) {
    let profile = pg_profile("app-dev", Some(EnvTag::Dev));
    let id = profile.id;
    let vcx = shell_with_fake_prepared(
        cx,
        "en",
        answers_probe(FakeDriver::new(DbmsKind::Postgres)),
        |paths| {
            save_profiles(paths, std::slice::from_ref(&profile));
            write_cache(
                &cache_file(paths, id),
                None,
                &[cached_schema(
                    DbmsKind::Postgres,
                    &["app", "public"],
                    &["users", "orders"],
                )],
            );
        },
    );
    // The hub opened the cache (on the core runtime) with no connection: no query ran.
    wait_until(vcx, "the cached tree", |vcx| {
        tables_of(vcx, id, &public()).len() == 2
    });
    assert_eq!(fake(vcx).connect_calls(), 0, "nothing connected");
    assert!(matches!(
        state_of(vcx, id, &public()),
        Some(LoadState::Loaded { stale: true, .. })
    ));
    open(vcx, &["app-dev", "app", "public", "TABLES"]);
    assert!(
        shows(vcx, "users") && shows(vcx, "orders"),
        "{:?}",
        labels(vcx)
    );
}

#[gpui::test]
fn offline_profile_shows_cached_with_its_date(cx: &mut TestAppContext) {
    let profile = pg_profile("app-dev", Some(EnvTag::Dev));
    let id = profile.id;
    let vcx = shell_with_fake_prepared(
        cx,
        "en",
        answers_probe(FakeDriver::new(DbmsKind::Postgres)),
        |paths| {
            save_profiles(paths, std::slice::from_ref(&profile));
            write_cache(
                &cache_file(paths, id),
                None,
                &[cached_schema(
                    DbmsKind::Postgres,
                    &["app", "public"],
                    &["users"],
                )],
            );
        },
    );
    wait_until(vcx, "the cached tree", |vcx| {
        !tables_of(vcx, id, &public()).is_empty()
    });
    vcx.cx.update(|cx| ConnectionsModel::select(cx, Some(id)));
    vcx.run_until_parked();
    let note = vcx
        .cx
        .read(hlive::offline_note)
        .map(|s| strip(&s))
        .expect("the offline strip");
    // Preformatted by ICU4X in the UI locale ("Nov 17, 2025, …").
    assert!(
        note.starts_with("Showing cached metadata from Nov 17, 2025"),
        "{note}"
    );
    assert_eq!(
        text_of(vcx, "explorer-offline-note").as_deref(),
        Some(note.as_str())
    );
    // The root says "cached", with the date in its tooltip.
    let root = rows(vcx)
        .into_iter()
        .find(|r| r.key == adapter::profile_key(id))
        .expect("the root");
    assert_eq!(
        root.data.meta.as_deref().map(strip).as_deref(),
        Some("cached")
    );
    let tip = root.data.tooltip.map(|t| strip(&t)).unwrap_or_default();
    assert!(
        tip.starts_with("Not connected. The tree is the copy saved on Nov 17, 2025"),
        "{tip}"
    );
}

// ---- connect ----------------------------------------------------------------------------------

#[gpui::test]
fn connect_publishes_cached_stale_then_validates(cx: &mut TestAppContext) {
    let intro = FakeIntrospector::new(DbmsKind::Postgres).with_incremental(true);
    intro.put_ns(public(), fake_tables(&["users", "orders", "invoices"]));
    let gate = intro.gate(|c| matches!(c, FakeCall::Introspect { .. }));
    let (vcx, src) = fake_shell(cx, intro, |_| {});
    let id = add_profile(vcx, pg_profile("app-dev", Some(EnvTag::Dev)));
    set_expanded(vcx, adapter::profile_key(id), true);
    write_cache(
        &src.cache_path(id),
        None,
        &[cached_schema(
            DbmsKind::Postgres,
            &["app", "public"],
            &["users", "orders"],
        )],
    );
    connect(vcx, "app-dev");
    // The cached namespace is shown stale before its pass ends.
    wait_gate(vcx, &gate);
    wait_until(vcx, "the cached tree, stale", |vcx| {
        tables_of(vcx, id, &public()) == ["users", "orders"]
    });
    assert!(matches!(
        state_of(vcx, id, &public()),
        Some(LoadState::Loaded { stale: true, .. })
    ));
    gate.release();
    wait_until(vcx, "the validated namespace", |vcx| {
        tables_of(vcx, id, &public()).len() == 3
            && matches!(
                state_of(vcx, id, &public()),
                Some(LoadState::Loaded { stale: false, .. })
            )
    });
    let calls = src.intro.calls();
    assert_eq!(calls.first(), Some(&FakeCall::ListNamespaces), "{calls:?}");
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, FakeCall::Introspect { ns, .. } if *ns == public())),
        "{calls:?}"
    );
}

#[gpui::test]
fn refresh_object_runs_a_fragment_only(cx: &mut TestAppContext) {
    let (vcx, src, id) = connected_loaded(cx, &["users", "orders"]);
    open(vcx, &["app-dev", "app", "public", "TABLES"]);
    let before = src.intro.calls().len();
    put_cursor(vcx, key_of(vcx, "users"));
    assert!(run(vcx, "explorer.refresh"), "Refresh runs on an object");
    wait_until(vcx, "the fragment", |_| {
        src.intro
            .calls()
            .iter()
            .any(|c| matches!(c, FakeCall::Fragment { .. }))
    });
    let after: Vec<FakeCall> = src.intro.calls()[before..].to_vec();
    assert_eq!(after.len(), 1, "one fragment and nothing else: {after:?}");
    let FakeCall::Fragment { ns, keys, .. } = &after[0] else {
        panic!("{after:?}");
    };
    assert_eq!(*ns, public());
    assert_eq!(keys.len(), 1);
    assert_eq!(&*keys[0].path[2], "users");
    let _ = id;
}

// ---- the mass-deletion guard (D-039, R-M4-22) -------------------------------------------------

fn thirty() -> Vec<String> {
    (0..30).map(|i| format!("t{i:02}")).collect()
}

#[gpui::test]
fn user_refresh_mass_drop_asks_and_cancel_keeps_cache(cx: &mut TestAppContext) {
    let names = thirty();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (vcx, src, id) = connected_loaded(cx, &refs);
    // 25 of 30 objects vanish on the server.
    src.intro.edit_ns(&public(), |ns| ns.objects.truncate(5));
    open(vcx, &["app-dev", "app"]);
    put_cursor(vcx, key_of(vcx, "public"));
    assert!(run(vcx, "explorer.refresh"));
    wait_until(vcx, "the confirmation", |vcx| {
        vcx.cx
            .read(|cx| mass_drop::open_dialog(id, &public(), cx))
            .is_some()
    });
    assert_eq!(
        strip(&dialog_title(vcx)),
        "25 objects in public are no longer visible"
    );
    assert!(layer_ids(vcx).contains(&"dialog"));
    // Escape answers Keep cached: nothing is removed.
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!layer_ids(vcx).contains(&"dialog"));
    wait_until(vcx, "the answer applied", |vcx| {
        matches!(
            state_of(vcx, id, &public()),
            Some(LoadState::Loaded { stale: false, .. })
        )
    });
    assert_eq!(
        tables_of(vcx, id, &public()).len(),
        30,
        "Keep cached keeps them"
    );
}

#[gpui::test]
fn background_mass_drop_shows_stale_badge_and_no_dialog(cx: &mut TestAppContext) {
    let names = thirty();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (vcx, src, id) = connected_loaded(cx, &refs);
    src.intro.edit_ns(&public(), |ns| ns.objects.truncate(5));
    let h = handle(vcx, id);
    drop(h.refresh(Target::Namespace(public()), Trigger::Background));
    wait_until(vcx, "the held deletion", |vcx| {
        vcx.cx
            .read(|cx| hlive::pending(id, cx))
            .get(&public())
            .copied()
            == Some(25)
    });
    assert!(
        !layer_ids(vcx).contains(&"dialog"),
        "a background pass never asks"
    );
    assert_eq!(tables_of(vcx, id, &public()).len(), 30, "nothing removed");
    open(vcx, &["app-dev", "app"]);
    let row = rows(vcx)
        .into_iter()
        .find(|r| strip(&r.data.label) == "public")
        .expect("the schema row")
        .data;
    assert_eq!(row.badge.as_deref(), Some("out of date"));
    assert_eq!(
        row.tooltip.map(|t| strip(&t)).as_deref(),
        Some(
            "25 objects disappeared in the last background check. Refresh public to review \
             them; nothing was removed."
        )
    );
}

// ---- disconnect -------------------------------------------------------------------------------

#[gpui::test]
fn disconnect_stops_the_engine_and_keeps_the_cache(cx: &mut TestAppContext) {
    let (vcx, src, id) = connected_loaded(cx, &["users"]);
    let h = handle(vcx, id);
    // The pass was saved (the debounced save runs on the blocking pool).
    let path = src.cache_path(id);
    vcx.cx.update(|cx| ConnectionsModel::disconnect(cx, id));
    wait_until(vcx, "the engine stopped", |vcx| {
        !vcx.cx.read(|cx| hlive::is_open(id, cx))
    });
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stopped.clone();
    let closed = h.closed();
    runtime(vcx).spawn(async move {
        closed.await;
        flag.store(true, Ordering::SeqCst);
    });
    poll_until("the engine's end", || stopped.load(Ordering::SeqCst));
    assert_eq!(tables_of(vcx, id, &public()), ["users"], "the tree stays");
    assert!(path.exists(), "the cache file stays");
    // The cache holds the namespace.
    let mut store = hacivat_meta::persist::CacheStore::open(
        &path,
        Encryption::None,
        hacivat_meta::persist::WriterId(9),
    )
    .expect("the cache")
    .store;
    let loaded = store.load_namespace(&public()).expect("readable");
    assert!(loaded.is_some(), "the namespace was saved");
}

// ---- the cache key (R-M4-15) ------------------------------------------------------------------

/// A memory backend that counts its calls and can hold `get` until released.
struct Counting {
    inner: MemoryBackend,
    calls: Arc<std::sync::atomic::AtomicU64>,
    hold: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    fail_put: bool,
}

impl Counting {
    fn new(held: bool, fail_put: bool) -> Self {
        Counting {
            inner: MemoryBackend::new(),
            calls: Arc::default(),
            hold: Arc::new((std::sync::Mutex::new(held), std::sync::Condvar::new())),
            fail_put,
        }
    }
}

impl SecretBackend for Counting {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (lock, cv) = &*self.hold;
        let mut held = lock.lock().unwrap();
        while *held {
            held = cv.wait(held).unwrap();
        }
        drop(held);
        if self.fail_put {
            return Ok(None);
        }
        self.inner.get(key)
    }
    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_put {
            return Err(SecretError::Backend("no keyring".to_string()));
        }
        self.inner.put(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.inner.delete(key)
    }
    fn name(&self) -> &'static str {
        "counting"
    }
}

const KEY: [u8; 32] = [7u8; 32];

fn key_hex() -> String {
    KEY.iter().map(|b| format!("{b:02x}")).collect()
}

fn prod_with_encrypted_cache(
    cx: &mut TestAppContext,
    backend: Arc<Counting>,
    expanded: bool,
) -> (&'static mut VisualTestContext, ProfileId) {
    let profile = pg_profile("orders-prod", Some(EnvTag::Production));
    let id = profile.id;
    backend
        .inner
        .put(
            &SecretKey {
                profile: id,
                slot: hacivat_secrets::SecretSlot::CatalogKey,
            },
            &SecretString::from(key_hex()),
        )
        .expect("the key is stored");
    let as_backend: Arc<dyn SecretBackend> = backend.clone();
    let vcx = shell_with_fake_backend(
        cx,
        "en",
        answers_probe(FakeDriver::new(DbmsKind::Postgres)),
        as_backend,
        move |paths| {
            save_profiles(paths, std::slice::from_ref(&profile));
            write_cache(
                &cache_file(paths, id),
                Some(KEY),
                &[cached_schema(
                    DbmsKind::Postgres,
                    &["app", "public"],
                    &["users"],
                )],
            );
            if expanded {
                write(
                    paths.workspace_json(),
                    &workspace_json(id, &[adapter::profile_key(id)]),
                );
            }
        },
    );
    (vcx, id)
}

#[gpui::test]
fn encrypted_profile_not_expanded_makes_no_keyring_call_at_start(cx: &mut TestAppContext) {
    let backend = Arc::new(Counting::new(false, false));
    let (vcx, id) = prod_with_encrypted_cache(cx, backend.clone(), false);
    wait_until(vcx, "the start scan", |vcx| {
        vcx.cx
            .read(|cx| hlive::row(id, cx))
            .is_some_and(|r| r.locked)
    });
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0, "no keyring call");
    assert_eq!(vcx.cx.read(hlive::ensures), 0);
    assert!(vcx.cx.read(|cx| adapter::catalog_of(id, cx)).is_none());
    // The root stays collapsed with the key glyph and says why.
    let root = rows(vcx)
        .into_iter()
        .find(|r| r.key == adapter::profile_key(id))
        .expect("the root");
    assert_eq!(
        root.data.meta_mark,
        Some(hacivat_ui::tree::MetaMark::Glyph(
            hacivat_ui::icon::IconName::KeyRound
        ))
    );
    assert!(
        root.data
            .tooltip
            .is_some_and(|t| t.contains("Opening this connection reads its key"))
    );
    // The user expanding the root is what reads the key.
    put_cursor(vcx, adapter::profile_key(id));
    let tree = vcx.cx.read(|cx| explorer::tree(cx).expect("the tree"));
    in_window(vcx, |window, cx| {
        let focus = tree.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    vcx.simulate_keystrokes("right");
    wait_until(vcx, "the tree after the key", |vcx| {
        !tables_of(vcx, id, &public()).is_empty()
    });
    assert!(backend.calls.load(Ordering::SeqCst) > 0);
}

#[gpui::test]
fn encrypted_expanded_profile_paints_after_key_without_blocking_ui(cx: &mut TestAppContext) {
    let backend = Arc::new(Counting::new(true, false));
    let (vcx, id) = prod_with_encrypted_cache(cx, backend.clone(), true);
    // The key fetch waits on the blocking pool; the UI keeps running meanwhile.
    wait_until(vcx, "the key fetch", |_| {
        backend.calls.load(Ordering::SeqCst) > 0
    });
    vcx.run_until_parked();
    assert!(vcx.cx.read(|cx| adapter::catalog_of(id, cx)).is_none());
    push_toast(
        vcx,
        "hv-explorer-level-not-saved",
        hacivat_ui::toast::ToastKind::Info,
    );
    assert!(!toast_texts(vcx).is_empty(), "the UI thread is free");
    {
        let (lock, cv) = &*backend.hold;
        *lock.lock().unwrap() = false;
        cv.notify_all();
    }
    wait_until(vcx, "the tree after the key", |vcx| {
        tables_of(vcx, id, &public()) == ["users"]
    });
    assert!(matches!(
        vcx.cx.read(|cx| hlive::cache_mode(id, cx)),
        Some(CacheMode::Persistent {
            encrypted: true,
            ..
        })
    ));
    // The restored expansion of the root shows its database.
    assert!(shows(vcx, "app"), "{:?}", labels(vcx));
}

#[gpui::test]
fn encrypted_profile_without_keyring_runs_memory_only_with_a_notice(cx: &mut TestAppContext) {
    let backend = Arc::new(Counting::new(false, true));
    let as_backend: Arc<dyn SecretBackend> = backend.clone();
    let vcx = shell_with_fake_backend(
        cx,
        "en",
        answers_probe(FakeDriver::new(DbmsKind::Postgres)),
        as_backend,
        |_| {},
    );
    let id = add_profile(vcx, pg_profile("orders-prod", Some(EnvTag::Production)));
    vcx.cx.update(|cx| hlive::open(id, cx));
    wait_until(vcx, "the catalog", |vcx| {
        vcx.cx.read(|cx| hlive::cache_mode(id, cx)).is_some()
    });
    assert_eq!(
        vcx.cx.read(|cx| hlive::cache_mode(id, cx)),
        Some(CacheMode::MemoryOnly(MemoryOnlyReason::KeyringUnavailable))
    );
    assert!(
        toast_texts(vcx).contains(
            &"The Keychain isn't available, so orders-prod's metadata is kept in memory until you quit."
                .to_string()
        ),
        "{:?}",
        toast_texts(vcx)
    );
    let paths = paths(vcx);
    assert!(
        !cache_file(&paths, id).exists(),
        "nothing is written to disk"
    );
}

// ---- progress and coalescing ------------------------------------------------------------------

#[gpui::test]
fn progress_row_shows_phase_and_count_and_repaints_only_on_change(cx: &mut TestAppContext) {
    let intro = FakeIntrospector::new(DbmsKind::Postgres);
    intro.put_ns(public(), fake_tables(&["users"]));
    let gate = intro.gate(|c| matches!(c, FakeCall::Introspect { .. }));
    let (vcx, _src) = fake_shell(cx, intro, |_| {});
    let id = add_profile(vcx, pg_profile("app-dev", Some(EnvTag::Dev)));
    set_expanded(vcx, adapter::profile_key(id), true);
    connect(vcx, "app-dev");
    wait_gate(vcx, &gate);
    wait_until(vcx, "the namespace listed", |vcx| shows(vcx, "app-dev"));
    wait_until(vcx, "the progress", |vcx| {
        vcx.cx
            .read(|cx| hlive::row(id, cx))
            .is_some_and(|r| !r.progress.tasks.is_empty())
    });
    wait_until(vcx, "the database row", |vcx| shows(vcx, "app"));
    open(vcx, &["app"]);
    wait_until(vcx, "the schema row", |vcx| shows(vcx, "public"));
    open(vcx, &["public"]);
    // One status row: the phase as its label, the count as its own text.
    let status: Vec<ShownRow> = rows(vcx)
        .into_iter()
        .filter(|r| r.status.is_some())
        .collect();
    assert_eq!(status.len(), 1, "{:?}", labels(vcx));
    assert_eq!(strip(status[0].status.as_ref().unwrap()), "Reading names…");
    // Fix round 1 (m-6): the count is the row's own text, `{done}/{total}` from the engine.
    let state = vcx.cx.read(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        let model = tree.read(cx).model().clone();
        hacivat_ui::tree::TreeModel::children(&model, status[0].key)
    });
    let hacivat_ui::tree::ChildState::Loading { running, count, .. } = state else {
        panic!("a loading row: {state:?}");
    };
    assert!(running);
    let task = vcx
        .cx
        .read(|cx| hlive::row(id, cx))
        .and_then(|r| r.progress.tasks.first().cloned())
        .expect("the task");
    let want = format!("{}/{}", task.done, task.total.expect("a total"));
    assert_eq!(count.map(|c| strip(&c)).as_deref(), Some(want.as_str()));
    // Nothing changes while the pass waits: no repaint.
    let generation = vcx.cx.read(hlive::generation);
    for _ in 0..5 {
        vcx.run_until_parked();
    }
    assert_eq!(vcx.cx.read(hlive::generation), generation);
    gate.release();
    wait_until(vcx, "the pass", |vcx| {
        !tables_of(vcx, id, &public()).is_empty()
    });
}

#[gpui::test]
fn notify_is_coalesced_to_one_per_frame(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let id = add_profile(vcx, pg_profile("app-dev", Some(EnvTag::Dev)));
    // A catalog whose publisher the test keeps (a detached engine handle).
    let catalog = Arc::new(Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        hacivat_meta::name::IdentCtx::new(hacivat_meta::name::DialectId::Postgres),
    ))));
    let publisher = catalog.publisher().expect("the publisher");
    struct Detached(Arc<Catalog>);
    impl CatalogSource for Detached {
        fn ensure(
            &self,
            _: Arc<Profile>,
        ) -> futures::future::BoxFuture<
            '_,
            Result<hacivat_introspect::hub::ProfileCatalog, hacivat_introspect::hub::HubError>,
        > {
            let pc = hacivat_introspect::hub::ProfileCatalog {
                catalog: self.0.clone(),
                handle: IntroHandle::detached(self.0.clone()),
                cache: CacheMode::MemoryOnly(MemoryOnlyReason::CacheUnavailable),
            };
            Box::pin(async move { Ok(pc) })
        }
        fn shutdown(&self, _: ProfileId) -> futures::future::BoxFuture<'_, ()> {
            Box::pin(async {})
        }
        fn has_cache(&self, _: ProfileId) -> bool {
            false
        }
    }
    let source: Arc<dyn CatalogSource> = Arc::new(Detached(catalog.clone()));
    vcx.cx.update(|cx| hlive::set_source(source, cx));
    vcx.cx.update(|cx| hlive::open(id, cx));
    wait_until(vcx, "the catalog open", |vcx| {
        vcx.cx.read(|cx| hlive::is_open(id, cx))
    });
    let applied = vcx.cx.read(|cx| hlive::applied(id, cx));
    // 100 publishes while the UI thread does not run.
    for i in 0..100 {
        let ns = cached_schema(DbmsKind::Postgres, &["app", "public"], &[&format!("t{i}")]);
        publisher
            .publish(vec![CatalogChange::Put(Arc::new(ns))])
            .expect("published");
    }
    let last = catalog.current().generation;
    wait_until(vcx, "the last generation shown", |vcx| {
        vcx.cx
            .read(|cx| adapter::catalog_of(id, cx))
            .is_some_and(|c| c.generation == last)
    });
    // The last publish reached the inbox (the processed signal); any later wake finds nothing
    // new.
    wait_until(vcx, "the forwarder took the last event", |vcx| {
        vcx.cx.read(|cx| hlive::forwarded_generation(id, cx)) >= last
    });
    vcx.run_until_parked();
    assert_eq!(
        vcx.cx.read(|cx| hlive::applied(id, cx)),
        applied + 1,
        "one explorer update for the whole batch"
    );
}

// ---- threading --------------------------------------------------------------------------------

#[gpui::test]
fn ui_thread_never_awaits_catalog_io(cx: &mut TestAppContext) {
    let main: ThreadId = std::thread::current().id();
    let (vcx, src, id) = connected_loaded(cx, &["users", "orders"]);
    open(vcx, &["app-dev", "app", "public", "TABLES"]);
    put_cursor(vcx, key_of(vcx, "users"));
    assert!(run(vcx, "explorer.refresh"));
    wait_until(vcx, "the fragment", |_| {
        src.intro
            .calls()
            .iter()
            .any(|c| matches!(c, FakeCall::Fragment { .. }))
    });
    vcx.cx.update(|cx| ConnectionsModel::disconnect(cx, id));
    wait_until(vcx, "the engine stopped", |vcx| {
        !vcx.cx.read(|cx| hlive::is_open(id, cx))
    });
    wait_until(vcx, "the source stopped the engine", |_| {
        src.made(id).is_none()
    });
    let threads = src.threads.lock().unwrap().clone();
    assert!(threads.len() >= 3, "the probe saw the calls: {threads:?}");
    assert!(
        threads.iter().all(|t| *t != main),
        "a cache open, a key fetch or a metadata connection ran on the UI thread"
    );
}

// ---- reveal, database rows --------------------------------------------------------------------

#[gpui::test]
fn reveal_expands_ancestors_and_selects(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let id = add_profile(vcx, pg_profile("app-dev", Some(EnvTag::Dev)));
    let cat = catalog(
        DbmsKind::Postgres,
        vec![schema(
            DbmsKind::Postgres,
            &["app", "public"],
            false,
            vec![table("users", Some(16_410), &[("id", "integer")])],
        )],
    );
    vcx.cx
        .update(|cx| adapter::set_catalog(id, Some(cat.clone()), None, cx));
    vcx.run_until_parked();
    let key = cat
        .namespaces
        .get(&public())
        .and_then(|n| {
            n.children(0)
                .find(|(_, o)| &**o.name() == "users")
                .and_then(|(i, _)| n.object_key(i))
        })
        .expect("the table's key");
    let outcome = in_window(vcx, |window, cx| hlive::reveal(id, &key, window, cx));
    assert_eq!(outcome, RevealOutcome::Revealed);
    vcx.run_until_parked();
    let cursor = vcx
        .cx
        .read(|cx| explorer::tree(cx).and_then(|t| t.read(cx).cursor()));
    assert_eq!(cursor, Some(adapter::object_tree_key(id, &key)));
    assert!(shows(vcx, "users"));
    // A table no loaded namespace holds is missing; one in an unloaded schema is pending.
    let mut gone = key.clone();
    gone.path[2] = "nope".into();
    gone.server_id = None;
    let outcome = in_window(vcx, |window, cx| hlive::reveal(id, &gone, window, cx));
    assert_eq!(outcome, RevealOutcome::Missing);
}

#[gpui::test]
fn database_rows_marks_greyed_pg_databases_out_of_scope(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let id = add_profile(vcx, pg_profile("app-dev", Some(EnvTag::Dev)));
    let cat = catalog_gen(
        DbmsKind::Postgres,
        1,
        vec![schema(
            DbmsKind::Postgres,
            &["app", "public"],
            false,
            vec![],
        )],
        vec![
            (NsKey::database("reports"), LoadState::OutOfScope),
            (NsKey::schema(["reports", "public"]), LoadState::OutOfScope),
        ],
    );
    vcx.cx
        .update(|cx| adapter::set_catalog(id, Some(cat), None, cx));
    let rows = vcx.cx.read(|cx| hlive::database_rows(id, cx));
    assert_eq!(
        rows,
        [
            DatabaseRow {
                name: "app".to_string(),
                in_scope: true,
                connected: false,
            },
            DatabaseRow {
                name: "reports".to_string(),
                in_scope: false,
                connected: false,
            },
        ]
    );
}

// ---- the expanded state (PO-Q15) --------------------------------------------------------------

#[gpui::test]
fn expanded_state_is_saved_in_workspace_json(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let id = add_profile(vcx, pg_profile("app-dev", Some(EnvTag::Dev)));
    vcx.cx.update(|cx| {
        adapter::set_catalog(
            id,
            Some(catalog(
                DbmsKind::Postgres,
                vec![schema(
                    DbmsKind::Postgres,
                    &["app", "public"],
                    false,
                    vec![],
                )],
            )),
            None,
            cx,
        )
    });
    open(vcx, &["app-dev", "app"]);
    with_ws(vcx, |ws, _, _| ws.flush_layout());
    let text = std::fs::read_to_string(paths(vcx).workspace_json()).expect("the file");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    let keys: Vec<String> = value["explorer"]["expanded"][id.0.to_string()]
        .as_array()
        .expect("the profile's keys")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let hex = |k: TreeKey| format!("{:032x}", k.0);
    assert!(keys.contains(&hex(adapter::profile_key(id))), "{keys:?}");
    assert!(
        keys.contains(&hex(adapter::database_key(id, "app"))),
        "{keys:?}"
    );
}

#[gpui::test]
fn expanded_state_restored_after_restart_from_cache(cx: &mut TestAppContext) {
    let profile = pg_profile("app-dev", Some(EnvTag::Dev));
    let id = profile.id;
    let root = adapter::profile_key(id);
    let db = adapter::database_key(id, "app");
    let sch = adapter::namespace_key(id, &public());
    let group = adapter::group_key(sch, DisplayGroup::Tables);
    let vcx = shell_with_fake_prepared(
        cx,
        "en",
        answers_probe(FakeDriver::new(DbmsKind::Postgres)),
        |paths| {
            save_profiles(paths, std::slice::from_ref(&profile));
            write_cache(
                &cache_file(paths, id),
                None,
                &[cached_schema(
                    DbmsKind::Postgres,
                    &["app", "public"],
                    &["users"],
                )],
            );
            write(
                paths.workspace_json(),
                &workspace_json(id, &[root, db, sch, group]),
            );
        },
    );
    wait_until(vcx, "the restored rows", |vcx| shows(vcx, "users"));
    let expanded = vcx.cx.read(|cx| {
        explorer::tree(cx)
            .expect("the tree")
            .read(cx)
            .expanded()
            .clone()
    });
    for k in [root, db, sch, group] {
        assert!(expanded.contains(&k));
    }
    assert_eq!(fake(vcx).connect_calls(), 0, "restoring connects nothing");
}

#[gpui::test]
fn restore_never_triggers_introspection_of_out_of_scope_nodes(cx: &mut TestAppContext) {
    let intro = FakeIntrospector::new(DbmsKind::Postgres);
    intro.put_ns(public(), fake_tables(&["users"]));
    intro.put_ns(NsKey::schema(["app", "audit"]), fake_tables(&["log"]));
    let profile = pg_profile("app-dev", Some(EnvTag::Dev));
    let id = profile.id;
    let audit = NsKey::schema(["app", "audit"]);
    let restored = [
        adapter::profile_key(id),
        adapter::database_key(id, "app"),
        adapter::namespace_key(id, &public()),
        // Not in the cached snapshot: never expanded, never read.
        adapter::namespace_key(id, &audit),
    ];
    let p2 = profile.clone();
    let (vcx, src) = fake_shell(cx, intro, move |paths| {
        save_profiles(paths, std::slice::from_ref(&p2));
        write(paths.workspace_json(), &workspace_json(id, &restored));
    });
    write_cache(
        &src.cache_path(id),
        None,
        &[cached_schema(
            DbmsKind::Postgres,
            &["app", "public"],
            &["users"],
        )],
    );
    vcx.cx.update(hlive::start);
    wait_until(vcx, "the cached rows", |vcx| shows(vcx, "public"));
    // Processed: the catalog is open, its lifecycle settled, and its engine never told to
    // connect (only `on_connect` reads the server).
    wait_until(vcx, "the catalog open and settled", |vcx| {
        vcx.cx
            .read(|cx| hlive::is_open(id, cx) && hlive::settled(id, cx))
    });
    assert!(!vcx.cx.read(|cx| hlive::engine_connected(id, cx)));
    assert!(src.intro.calls().is_empty(), "{:?}", src.intro.calls());
    assert_eq!(src.meta.attempts.load(Ordering::SeqCst), 0);
    // On connect only what on-connect schedules runs: the listing and the current namespace.
    connect(vcx, "app-dev");
    wait_until(vcx, "the validated namespace", |vcx| {
        matches!(
            state_of(vcx, id, &public()),
            Some(LoadState::Loaded { stale: false, .. })
        )
    });
    // Processed: the engine has nothing running or queued.
    wait_until(vcx, "the engine idle", |vcx| {
        vcx.cx
            .read(|cx| hlive::row(id, cx))
            .is_some_and(|r| r.progress.tasks.is_empty())
    });
    let calls = src.intro.calls();
    assert!(
        !calls
            .iter()
            .any(|c| matches!(c, FakeCall::Introspect { ns, .. } if *ns == audit)),
        "{calls:?}"
    );
    let model_info = vcx.cx.read(|cx| {
        explorer::tree(cx)
            .expect("the tree")
            .read(cx)
            .model()
            .info(adapter::namespace_key(id, &audit))
    });
    assert!(
        !matches!(model_info, Some(NodeInfo::Schema { .. }))
            || !vcx.cx.read(|cx| explorer::tree(cx)
                .expect("the tree")
                .read(cx)
                .is_expanded(adapter::namespace_key(id, &audit)))
    );
    let _ = ObjectKey::clone;
}

/// Deviation D-1: a connected profile whose root is collapsed has its cache opened but no
/// catalog query until the root opens (then on-connect runs once).
#[gpui::test]
fn a_collapsed_connected_root_reads_nothing_until_it_opens(cx: &mut TestAppContext) {
    let intro = FakeIntrospector::new(DbmsKind::Postgres);
    intro.put_ns(public(), fake_tables(&["users"]));
    let (vcx, src) = fake_shell(cx, intro, |_| {});
    let id = add_profile(vcx, pg_profile("app-dev", Some(EnvTag::Dev)));
    connect(vcx, "app-dev");
    wait_until(vcx, "the catalog open", |vcx| {
        vcx.cx
            .read(|cx| hlive::is_open(id, cx) && hlive::settled(id, cx))
    });
    // Processed: the engine was never told to connect, and only that reads the server.
    assert!(!vcx.cx.read(|cx| hlive::engine_connected(id, cx)));
    assert!(src.intro.calls().is_empty(), "{:?}", src.intro.calls());
    assert_eq!(src.meta.attempts.load(Ordering::SeqCst), 0);
    set_expanded(vcx, adapter::profile_key(id), true);
    wait_until(vcx, "the current schema loaded", |vcx| {
        tables_of(vcx, id, &public()) == ["users"]
    });
    assert_eq!(src.intro.calls().first(), Some(&FakeCall::ListNamespaces));
}

// ---- fix round 1 ------------------------------------------------------------------------------

/// The review's hostile probe 1 (I-1, flipped to assert the fix): a connection that flaps (open
/// → close → open) while the first `ensure` waits on the keyring leaves the explorer on a live
/// engine that the hub still owns.
#[gpui::test]
fn hostile_flapping_open_close_open_keeps_a_live_engine(cx: &mut TestAppContext) {
    use hacivat_app::explorer::live::HubSource;
    use hacivat_introspect::hub::{CatalogHub, HubSettings};
    let backend = Arc::new(Counting::new(true, false));
    let (vcx, id) = prod_with_encrypted_cache(cx, backend.clone(), false);
    // The test's own hub (built as `live::install` builds the app's), so its map can be read.
    let hub = vcx.cx.update(|cx| {
        let state = cx.global::<hacivat_app::shell::AppState>();
        let model = cx.global::<ConnectionsModel>();
        let sessions: Arc<dyn hacivat_introspect::hub::SessionLookup> = model.manager().clone();
        CatalogHub::new_on(
            model.runtime(),
            sessions,
            HubSettings {
                paths: state.paths.clone(),
                metadata: state.settings.metadata,
            },
            model.secrets().clone(),
            Arc::new(hacivat_meta::system::SystemCatalogs::empty()),
        )
    });
    let source: Arc<dyn CatalogSource> = Arc::new(HubSource {
        hub: hub.clone(),
        paths: paths(vcx),
    });
    vcx.cx.update(|cx| hlive::set_source(source, cx));
    wait_until(vcx, "the start scan (locked root)", |vcx| {
        vcx.cx
            .read(|cx| hlive::row(id, cx))
            .is_some_and(|r| r.locked)
    });
    // open #1 waits on the keyring under the hub's profile lock.
    vcx.cx.update(|cx| hlive::open(id, cx));
    wait_until(vcx, "the key fetch", |_| {
        backend.calls.load(Ordering::SeqCst) > 0
    });
    // The connection drops (close) and comes back (open #2) while the key is still pending.
    vcx.cx.update(|cx| drop(hlive::close(id, cx)));
    vcx.cx.update(|cx| hlive::open(id, cx));
    vcx.run_until_parked();
    {
        let (lock, cv) = &*backend.hold;
        *lock.lock().unwrap() = false;
        cv.notify_all();
    }
    wait_until(vcx, "the lifecycle settled, open", |vcx| {
        vcx.cx
            .read(|cx| hlive::is_open(id, cx) && hlive::settled(id, cx))
    });
    wait_until(vcx, "the hub idle", |_| hub.pending_locks() == 0);
    let explorer = vcx
        .cx
        .read(|cx| hlive::handle(id, cx))
        .expect("the explorer's catalog is open");
    let in_hub = hub.get(id).expect("the hub still owns the engine");
    assert!(
        Arc::ptr_eq(in_hub.handle.catalog(), explorer.catalog()),
        "the explorer and the hub hold the same catalog"
    );
}

/// I-1: a close asked while an open is in flight wins: the open's engine is stopped by the same
/// lifecycle, once, and nothing stays open.
#[gpui::test]
fn a_close_during_an_open_leaves_nothing_open(cx: &mut TestAppContext) {
    let backend = Arc::new(Counting::new(true, false));
    let (vcx, id) = prod_with_encrypted_cache(cx, backend.clone(), false);
    vcx.cx.update(|cx| hlive::open(id, cx));
    wait_until(vcx, "the key fetch", |_| {
        backend.calls.load(Ordering::SeqCst) > 0
    });
    let closed = vcx.cx.update(|cx| hlive::close(id, cx));
    {
        let (lock, cv) = &*backend.hold;
        *lock.lock().unwrap() = false;
        cv.notify_all();
    }
    wait_task(vcx, closed);
    assert!(!vcx.cx.read(|cx| hlive::is_open(id, cx)));
    assert!(vcx.cx.read(|cx| hlive::settled(id, cx)));
}

/// I-1 and the T19 carry: while Save deletes the cache files nothing reopens the store (a
/// connect arriving meanwhile waits), and the reopen is made from the profile as saved.
#[gpui::test]
fn a_connect_during_the_cache_delete_waits_for_the_reopen(cx: &mut TestAppContext) {
    let (vcx, src, id) = connected_loaded(cx, &["users"]);
    let released = vcx.cx.update(|cx| hlive::release_cache(id, cx));
    wait_task(vcx, released);
    assert!(src.made(id).is_none(), "the store is closed");
    let ensures = src.ensures.load(Ordering::SeqCst);
    // A connect event while the files are being deleted opens nothing.
    let state = vcx.cx.read(|cx| cx.global::<ConnectionsModel>().live(id));
    vcx.cx.update(|cx| hlive::on_live_state(id, &state, cx));
    vcx.cx.update(|cx| hlive::open(id, cx));
    wait_until(vcx, "the lifecycle settled", |vcx| {
        vcx.cx.read(|cx| hlive::settled(id, cx))
    });
    assert_eq!(
        src.ensures.load(Ordering::SeqCst),
        ensures,
        "no reopen while deleting"
    );
    assert!(!vcx.cx.read(|cx| hlive::is_open(id, cx)));
    // The profile is saved encrypted, then the reopen comes: made from the saved profile.
    let mut p = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().get(id))
        .expect("the profile")
        .as_ref()
        .clone();
    p.catalog_sensitive_set = Some(true);
    let saved = vcx
        .cx
        .update(|cx| ConnectionsModel::save(cx, hacivat_session::profile::ProfileDraft::edit(p)));
    assert!(wait_task(vcx, saved).is_ok());
    vcx.cx.update(|cx| hlive::reopen(id, true, cx));
    wait_until(vcx, "reopened", |vcx| {
        vcx.cx.read(|cx| hlive::is_open(id, cx))
    });
    assert_eq!(
        src.log.lock().unwrap().last().map(String::as_str),
        Some("ensure sensitive=true"),
        "{:?}",
        src.log.lock().unwrap()
    );
}

/// The T19 carry through T20's Save ("Encrypt cached metadata" turned on): the store closes
/// before the files are deleted, and the catalog opens again encrypted.
#[gpui::test]
fn save_closes_the_store_before_it_deletes_the_cache(cx: &mut TestAppContext) {
    use hacivat_app::connections::dialog::schemas;
    use hacivat_app::connections::dialog::{DialogMode, open_connection_dialog, open_dialog};
    let (vcx, src, id) = connected_loaded(cx, &["users"]);
    // A cache file Save deletes (`delete_profile_files` owns it); the source notes whether it
    // still existed when the store was closed.
    let planted = paths(vcx).catalog_db(&id.0.to_string(), 1);
    std::fs::create_dir_all(planted.parent().unwrap()).unwrap();
    std::fs::write(&planted, b"cache").unwrap();
    *src.watch.lock().unwrap() = Some(planted.clone());
    wait_until(vcx, "the profile's data", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::explorer::userdata::get(id, cx))
            .is_some()
    });
    in_window(vcx, |window, cx| {
        open_connection_dialog(DialogMode::Edit(id), window, cx)
    });
    wait_until(vcx, "the Schemas section", |vcx| {
        in_window(vcx, |window, cx| schemas::section_state(window, cx)).is_some()
    });
    let with_section = |vcx: &VisualTestContext, on: bool, apply: bool| {
        in_window(vcx, |window, cx| {
            let s = schemas::section_state(window, cx).expect("the section");
            s.update(cx, |s, cx| {
                s.toggle_encrypt(on, window, cx);
                s.answer_encrypt(apply, window, cx);
            });
        });
        vcx.run_until_parked();
    };
    with_section(vcx, true, true);
    run(vcx, "connection_dialog.save");
    wait_until(vcx, "the save", |vcx| {
        in_window(vcx, |window, cx| open_dialog(window, cx)).is_none()
    });
    assert!(!planted.exists(), "the cache files are deleted");
    wait_until(vcx, "reopened", |vcx| {
        vcx.cx.read(|cx| hlive::is_open(id, cx))
    });
    let log = src.log.lock().unwrap().clone();
    let closed_at = log
        .iter()
        .position(|l| l.starts_with("shutdown"))
        .expect("the store closed");
    assert_eq!(
        log[closed_at], "shutdown watched=true",
        "closed before the delete: {log:?}"
    );
    assert_eq!(
        log.last().map(String::as_str),
        Some("ensure sensitive=true"),
        "{log:?}"
    );
}

// ---- I-4: the carries' wiring ---------------------------------------------------------------

/// T15 carry: the current schema comes from the user's session (`set_current`), never the
/// metadata connection; a connected engine then reads its new current namespace.
#[gpui::test]
fn the_users_session_schema_becomes_the_engines_current(cx: &mut TestAppContext) {
    use hacivat_driver::ExecSummary;
    use hacivat_driver::fake::FakeExec;
    let sales = NsKey::schema(["app", "sales"]);
    let intro = FakeIntrospector::new(DbmsKind::Postgres);
    intro.put_ns(public(), fake_tables(&["users"]));
    intro.put_ns(sales.clone(), fake_tables(&["orders"]));
    let (vcx, src) = fake_shell(cx, intro, |_| {});
    let id = add_profile(vcx, pg_profile("app-dev", Some(EnvTag::Dev)));
    set_expanded(vcx, adapter::profile_key(id), true);
    connect(vcx, "app-dev");
    wait_until(vcx, "the current schema loaded, the engine idle", |vcx| {
        matches!(
            state_of(vcx, id, &public()),
            Some(LoadState::Loaded { stale: false, .. })
        ) && vcx
            .cx
            .read(|cx| hlive::row(id, cx))
            .is_some_and(|r| r.progress.tasks.is_empty())
    });
    assert!(
        !src.intro
            .calls()
            .iter()
            .any(|c| matches!(c, FakeCall::Introspect { ns, .. } if *ns == sales)),
        "not read before the session says so"
    );
    let profile = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().get(id))
        .expect("the profile")
        .as_ref()
        .clone();
    let probe = FakeExec {
        prefix: "SELECT current_database()",
        result: Ok(ExecSummary {
            rows: vec![vec![
                Some("app".to_string()),
                Some("sales".to_string()),
                Some("sales,public".to_string()),
                Some("deploy".to_string()),
                Some("sales, public".to_string()),
            ]],
            row_count: 1,
            ..ExecSummary::default()
        }),
        delay: std::time::Duration::ZERO,
        tx_after: None,
        panics: false,
    };
    let (session, _) = futures::executor::block_on(hacivat_session::testing::fake_session(
        runtime(vcx),
        FakeDriver::new(DbmsKind::Postgres).exec(vec![probe]),
        profile,
    ));
    let switched = session.submit(hacivat_session::session::Request::SwitchSchema(
        "sales".to_string(),
    ));
    let _ = futures::executor::block_on(switched.reply);
    assert_eq!(
        session
            .state()
            .borrow()
            .schema
            .as_ref()
            .and_then(|s| s.schema.clone())
            .as_deref(),
        Some("sales")
    );
    let console = register_test_console(vcx, id, session);
    vcx.cx
        .update(|cx| hacivat_app::console::Consoles::set_focused(cx, Some(console)));
    wait_until(vcx, "the new current namespace read", |_| {
        src.intro
            .calls()
            .iter()
            .any(|c| matches!(c, FakeCall::Introspect { ns, .. } if *ns == sales))
    });
    assert_eq!(
        vcx.cx.read(|cx| hlive::session_database(id, cx)).as_deref(),
        Some("app")
    );
}

/// T19 carry: a level pill set in the profile's data reaches the engine.
#[gpui::test]
fn a_level_pill_reaches_the_engine(cx: &mut TestAppContext) {
    use hacivat_app::explorer::{level_pill, userdata};
    use hacivat_introspect::engine::LevelSource;
    let (vcx, _src, id) = connected_loaded(cx, &["users"]);
    wait_until(vcx, "the profile's data", |vcx| {
        vcx.cx.read(|cx| userdata::get(id, cx)).is_some()
    });
    let h = handle(vcx, id);
    assert_eq!(h.level_of(&public()).source, LevelSource::Auto);
    let key = level_pill::namespace_object_key(&public());
    let write = vcx
        .cx
        .update(|cx| userdata::set_level_override(id, key, Some(hacivat_meta::Level::Names), cx));
    assert!(wait_task(vcx, write).is_ok());
    wait_until(vcx, "the override forwarded", |_| {
        h.level_of(&public()).source == LevelSource::Override
    });
    assert_eq!(h.level_of(&public()).level, hacivat_meta::Level::Names);
}

/// R-W3-2 carry: a saved scope filter reaches the engine (`set_filters`); the objects it no
/// longer keeps leave the tree as a removal (`out_of_scope`), with no deletion notice, and lose
/// their expansion.
#[gpui::test]
fn a_scope_filter_reaches_the_engine_and_out_of_scope_is_a_quiet_removal(cx: &mut TestAppContext) {
    use hacivat_app::explorer::{level_pill, userdata};
    let (vcx, _src, id) = connected_loaded(cx, &["users", "t1"]);
    wait_until(vcx, "the profile's data", |vcx| {
        vcx.cx.read(|cx| userdata::get(id, cx)).is_some()
    });
    open(vcx, &["app", "public", "TABLES", "t1"]);
    let t1 = key_of(vcx, "t1");
    let toasts = toast_texts(vcx);
    let slot = level_pill::namespace_object_key(&public());
    let write = vcx.cx.update(|cx| {
        userdata::update(
            id,
            move |d| {
                d.scope_filters
                    .insert(slot.clone(), "table:-t1".to_string());
            },
            cx,
        )
    });
    assert!(wait_task(vcx, write).is_ok());
    wait_until(vcx, "t1 removed", |vcx| {
        tables_of(vcx, id, &public()) == ["users"]
    });
    assert_eq!(
        toast_texts(vcx),
        toasts,
        "a scope change shows no deletion notice"
    );
    let expanded = vcx.cx.read(|cx| {
        explorer::tree(cx)
            .expect("the tree")
            .read(cx)
            .is_expanded(t1)
    });
    assert!(!expanded, "the removed row's expansion is forgotten");
}

/// T11 carry: a cache write that fails for space says so (`model-cache-write-failed`).
#[gpui::test]
fn a_failed_cache_write_shows_its_notice(cx: &mut TestAppContext) {
    let (vcx, _src, id) = connected_loaded(cx, &["users"]);
    let sent = vcx.cx.read(|cx| {
        hlive::inject_event(
            id,
            hacivat_introspect::engine::EngineEvent::CacheWriteFailed,
            cx,
        )
    });
    assert!(sent);
    wait_until(vcx, "the notice", |vcx| {
        toast_ids(vcx)
            .iter()
            .any(|t| t == "model-cache-write-failed")
    });
}

/// T15 m8 / T19 carry: a database row without an explicit level shows the engine's automatic
/// level (grey) for the objects it counted.
#[gpui::test]
fn a_database_row_shows_the_engines_automatic_level(cx: &mut TestAppContext) {
    let (vcx, _src, id) = connected_loaded(cx, &["users"]);
    wait_until(vcx, "the profile's data", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::explorer::userdata::get(id, cx))
            .is_some()
    });
    wait_until(vcx, "the database row's pill", |vcx| {
        rows(vcx)
            .into_iter()
            .find(|r| strip(&r.data.label) == "app")
            .is_some_and(|r| r.data.pill.is_some())
    });
    let pill = rows(vcx)
        .into_iter()
        .find(|r| strip(&r.data.label) == "app")
        .and_then(|r| r.data.pill)
        .expect("the pill");
    assert!(!pill.explicit, "automatic: grey");
    let tip = strip(&pill.tooltip);
    assert!(tip.contains("chosen automatically for 1 object"), "{tip}");
}

// ---- fix round 2 ------------------------------------------------------------------------------

/// R-W3-12: a pass's own phase and counts (the introspector's `report_progress`, through the
/// engine's progress sink) reach the namespace's `Loading { running, count }` row.
#[gpui::test]
fn the_progress_row_shows_the_passes_own_counts(cx: &mut TestAppContext) {
    let intro = FakeIntrospector::new(DbmsKind::Postgres);
    intro.put_ns(public(), fake_tables(&["users"]));
    let gate = intro.gate(|c| matches!(c, FakeCall::Introspect { .. }));
    let (vcx, src) = fake_shell(cx, intro, |_| {});
    *src.report.lock().unwrap() = Some((hacivat_introspect::engine::Phase::Columns, 3, Some(7)));
    let id = add_profile(vcx, pg_profile("app-dev", Some(EnvTag::Dev)));
    set_expanded(vcx, adapter::profile_key(id), true);
    connect(vcx, "app-dev");
    wait_gate(vcx, &gate);
    wait_until(vcx, "the database row", |vcx| shows(vcx, "app"));
    open(vcx, &["app"]);
    wait_until(vcx, "the schema row", |vcx| shows(vcx, "public"));
    open(vcx, &["public"]);
    let schema = key_of(vcx, "public");
    wait_until(vcx, "the pass's own count", |vcx| {
        let state = vcx.cx.read(|cx| {
            let model = explorer::tree(cx)
                .expect("the tree")
                .read(cx)
                .model()
                .clone();
            hacivat_ui::tree::TreeModel::children(&model, schema)
        });
        match state {
            hacivat_ui::tree::ChildState::Loading {
                running: true,
                label,
                count: Some(c),
            } => strip(&label) == "Reading columns…" && strip(&c) == "3/7",
            _ => false,
        }
    });
    gate.release();
    wait_until(vcx, "the pass", |vcx| {
        !tables_of(vcx, id, &public()).is_empty()
    });
}

/// I-6 (T15 fix round 1 carry, T17): the hub's MySQL placeholder knows no
/// `lower_case_table_names`; the engine's scope refresh replaces it with the server's facts
/// (`CatalogChange::SetServer`), and nothing the host does afterwards overrides them.
#[gpui::test]
fn mysql_server_facts_replace_the_placeholder_and_stay(cx: &mut TestAppContext) {
    use hacivat_meta::name::{DialectId, IdentCtx};
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let mut p = test_profile("shop-dev", Some(EnvTag::Dev), "127.0.0.1");
    p.engine = DbmsKind::MySql;
    p.database = Some("shop".to_string());
    let id = add_profile(vcx, p);
    let lctn = |vcx: &VisualTestContext| {
        vcx.cx.read(|cx| {
            hlive::handle(id, cx).map(|h| h.catalog().current().server.ident.lower_case_table_names)
        })
    };
    // The app's own hub: the placeholder is unknown (never a guessed lctn 0).
    vcx.cx.update(|cx| hlive::open(id, cx));
    wait_until(vcx, "the hub's catalog", |vcx| {
        vcx.cx.read(|cx| hlive::is_open(id, cx))
    });
    assert_eq!(lctn(vcx), Some(u8::MAX));
    let closed = vcx.cx.update(|cx| hlive::close(id, cx));
    wait_task(vcx, closed);
    // An engine whose server reports lctn 1.
    let intro = FakeIntrospector::new(DbmsKind::MySql);
    intro.put_ns(NsKey::schema(["shop"]), fake_tables(&["orders"]));
    let mut ident = IdentCtx::new(DialectId::MySql);
    ident.lower_case_table_names = 1;
    intro.set_facts(Some(Ok(ServerFacts::new(DbmsKind::MySql, ident))));
    let src = Arc::new(FakeSource::new(
        runtime(vcx),
        intro,
        temp_dir(vcx).join("fake-cache"),
    ));
    let as_source: Arc<dyn CatalogSource> = src.clone();
    vcx.cx.update(|cx| hlive::set_source(as_source, cx));
    vcx.cx.update(|cx| hlive::open(id, cx));
    wait_until(vcx, "the engine's catalog", |vcx| {
        vcx.cx.read(|cx| hlive::is_open(id, cx))
    });
    assert_eq!(lctn(vcx), Some(u8::MAX), "unknown until the server says");
    drop(handle(vcx, id).on_connect());
    wait_until(vcx, "the server's facts", |vcx| lctn(vcx) == Some(1));
    // The host's later work (a profile refresh, a forwarded level) never overrides them.
    vcx.cx
        .update(|cx| hlive::refresh_key(adapter::profile_key(id), cx));
    wait_until(vcx, "the profile's data", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::explorer::userdata::get(id, cx))
            .is_some()
    });
    let key = hacivat_app::explorer::level_pill::namespace_object_key(&NsKey::schema(["shop"]));
    let write = vcx.cx.update(|cx| {
        hacivat_app::explorer::userdata::set_level_override(
            id,
            key,
            Some(hacivat_meta::Level::Names),
            cx,
        )
    });
    assert!(wait_task(vcx, write).is_ok());
    wait_until(vcx, "the second scope refresh, the engine idle", |vcx| {
        src.intro
            .calls()
            .iter()
            .filter(|c| matches!(c, FakeCall::ListNamespaces))
            .count()
            >= 2
            && vcx
                .cx
                .read(|cx| hlive::row(id, cx))
                .is_some_and(|r| r.progress.tasks.is_empty())
    });
    assert_eq!(lctn(vcx), Some(1));
    let shown = vcx
        .cx
        .read(|cx| adapter::catalog_of(id, cx))
        .map(|c| c.server.ident.lower_case_table_names);
    assert_eq!(shown, Some(1), "the explorer shows the server's facts");
}
