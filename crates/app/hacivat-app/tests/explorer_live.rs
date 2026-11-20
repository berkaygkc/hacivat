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
    paths.catalog_db(&id.0.to_string(), u32::from(hacivat_meta::codec::FORMAT_MAJOR))
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
                &[cached_schema(DbmsKind::Postgres, &["app", "public"], &["users", "orders"])],
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
    assert!(shows(vcx, "users") && shows(vcx, "orders"), "{:?}", labels(vcx));
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
                &[cached_schema(DbmsKind::Postgres, &["app", "public"], &["users"])],
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
    assert_eq!(text_of(vcx, "explorer-offline-note").as_deref(), Some(note.as_str()));
    // The root says "cached", with the date in its tooltip.
    let root = rows(vcx)
        .into_iter()
        .find(|r| r.key == adapter::profile_key(id))
        .expect("the root");
    assert_eq!(root.data.meta.as_deref().map(strip).as_deref(), Some("cached"));
    let tip = root.data.tooltip.map(|t| strip(&t)).unwrap_or_default();
    assert!(tip.starts_with("Not connected. The tree is the copy saved on Nov 17, 2025"), "{tip}");
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
        &[cached_schema(DbmsKind::Postgres, &["app", "public"], &["users", "orders"])],
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
        calls.iter().any(|c| matches!(c, FakeCall::Introspect { ns, .. } if *ns == public())),
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
    src.intro
        .edit_ns(&public(), |ns| ns.objects.truncate(5));
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
    assert_eq!(tables_of(vcx, id, &public()).len(), 30, "Keep cached keeps them");
}

#[gpui::test]
fn background_mass_drop_shows_stale_badge_and_no_dialog(cx: &mut TestAppContext) {
    let names = thirty();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (vcx, src, id) = connected_loaded(cx, &refs);
    src.intro
        .edit_ns(&public(), |ns| ns.objects.truncate(5));
    let h = handle(vcx, id);
    drop(h.refresh(Target::Namespace(public()), Trigger::Background));
    wait_until(vcx, "the held deletion", |vcx| {
        vcx.cx
            .read(|cx| hlive::pending(id, cx))
            .get(&public())
            .copied()
            == Some(25)
    });
    assert!(!layer_ids(vcx).contains(&"dialog"), "a background pass never asks");
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
    vcx.cx
        .update(|cx| ConnectionsModel::disconnect(cx, id));
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
                &[cached_schema(DbmsKind::Postgres, &["app", "public"], &["users"])],
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
        Some(hacivat_ui::tree::MetaMark::Glyph(hacivat_ui::icon::IconName::KeyRound))
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
    wait_until(vcx, "the key fetch", |_| backend.calls.load(Ordering::SeqCst) > 0);
    vcx.run_until_parked();
    assert!(vcx.cx.read(|cx| adapter::catalog_of(id, cx)).is_none());
    push_toast(vcx, "hv-explorer-level-not-saved", hacivat_ui::toast::ToastKind::Info);
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
    assert!(!cache_file(&paths, id).exists(), "nothing is written to disk");
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
    // Settle any late wake: it finds nothing new.
    settle(vcx, std::time::Duration::from_millis(20));
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
    vcx.cx
        .update(|cx| ConnectionsModel::disconnect(cx, id));
    wait_until(vcx, "the engine stopped", |vcx| {
        !vcx.cx.read(|cx| hlive::is_open(id, cx))
    });
    settle(vcx, std::time::Duration::from_millis(20));
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
        vec![schema(DbmsKind::Postgres, &["app", "public"], false, vec![])],
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
                vec![schema(DbmsKind::Postgres, &["app", "public"], false, vec![])],
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
    assert!(keys.contains(&hex(adapter::database_key(id, "app"))), "{keys:?}");
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
                &[cached_schema(DbmsKind::Postgres, &["app", "public"], &["users"])],
            );
            write(paths.workspace_json(), &workspace_json(id, &[root, db, sch, group]));
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
        &[cached_schema(DbmsKind::Postgres, &["app", "public"], &["users"])],
    );
    vcx.cx.update(hlive::start);
    wait_until(vcx, "the cached rows", |vcx| shows(vcx, "public"));
    settle(vcx, std::time::Duration::from_millis(20));
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
    settle(vcx, std::time::Duration::from_millis(20));
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
        vcx.cx.read(|cx| hlive::is_open(id, cx))
    });
    settle(vcx, std::time::Duration::from_millis(20));
    assert!(src.intro.calls().is_empty(), "{:?}", src.intro.calls());
    assert_eq!(src.meta.attempts.load(Ordering::SeqCst), 0);
    set_expanded(vcx, adapter::profile_key(id), true);
    wait_until(vcx, "the current schema loaded", |vcx| {
        tables_of(vcx, id, &public()) == ["users"]
    });
    assert_eq!(src.intro.calls().first(), Some(&FakeCall::ListNamespaces));
}
