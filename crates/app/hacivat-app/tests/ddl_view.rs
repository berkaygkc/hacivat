//! M4 Task 25: the DDL view tab (design/m4-explorer surfaces 10 and 11; R-M4-24, R-M4-23,
//! R-W0-1, R-W0-5, R-W0-6): the text is exactly Task 19's script, the options strip re-renders
//! and is remembered per profile, Copy and Open in console, elevation with a loading state,
//! the stale / gone / couldn't-check notices, unsupported kinds, MySQL executable comments, the
//! read-only buffer, the server toggle per engine, grants loaded on demand and the provided
//! toolbar actions.
//!
//! Tasks 15b (facets) and 22 (freshness) are not merged at this base, so most tests use a fake
//! catalog ([`FakeCatalog`], the plan's "fake `IntroHandle`"); the engine's own path runs on
//! Task 15's fakes where it can (elevation, the real stubs).

mod support;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use gpui::{TestAppContext, VisualTestContext};
use hacivat_app::console::Consoles;
use hacivat_app::ddl_view::source::StateCheck;
use hacivat_app::ddl_view::{
    self, Body, DdlCatalog, DdlViewState, Freshness, GrantsUx, set_catalog_provider,
};
use hacivat_app::explorer::adapter;
use hacivat_app::explorer::live::{self as hlive, CatalogSource};
use hacivat_app::explorer::userdata;
use hacivat_app::explorer::{
    ActionSlot, ActionSurface, ExplorerActionProvider, NodeRef, ProvidedAction,
    register_action_provider,
};
use hacivat_ddl::produce::{DdlSources, create_script_with};
use hacivat_ddl::script::DelimStyle;
use hacivat_ddl::{DdlTarget, GenOptions};
use hacivat_driver::fake::FakeDriver;
use hacivat_introspect::RefreshTicket;
use hacivat_introspect::introspector::Staleness;
use hacivat_introspect::testing::fake::{FakeCall, FakeIntrospector, FakeObject};
use hacivat_introspect::ticket::TicketSender;
use hacivat_meta::namespace::{Level, Obj, Subtree, Watermark};
use hacivat_meta::props::{Facet, FacetKind, Grant, GrantsFacet, Props, ViewProps};
use hacivat_meta::snapshot::LoadState;
use hacivat_meta::userdata::{DdlViewOptions, Placement, Qualify};
use hacivat_meta::view::SnapshotView;
use hacivat_meta::{CatalogSnapshot, Name, NsKey, ObjectKey, ObjectKind, SourceRef};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::profile::Profile;
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui_core::gpui_surface::App;
use sql_dialect::{DialectId, ServerVersion};
use support::catalogs::*;
use support::*;

// ---- the fake catalog -------------------------------------------------------------------------

/// What a request does when it is made.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Answer {
    /// At once, with the snapshot's generation.
    Now,
    /// At once, `IntroError::NotImplemented` (a stub).
    Stub,
    /// Held until the test answers it.
    Hold,
}

#[derive(Default)]
struct Shared {
    /// `SourceRef` texts and `Sources` facet texts (by the object's path).
    texts: RefCell<HashMap<SourceRef, Name>>,
    server: RefCell<HashMap<String, Name>>,
    grants: RefCell<HashMap<String, Facet<GrantsFacet>>>,
    /// Every request, in order: `elevate users`, `check users`, `refresh users`,
    /// `facet grants users`.
    calls: RefCell<Vec<String>>,
    held: RefCell<Vec<(String, TicketSender)>>,
    check: Cell<Option<Answer>>,
    facet: Cell<Option<Answer>>,
    // ## T25 round 1: Task 22's per-key verdict (default `Current`), sent before the ticket.
    verdict: Cell<Option<Staleness>>,
    held_verdicts: RefCell<Vec<(String, VerdictTx)>>,
}

type VerdictTx = tokio::sync::oneshot::Sender<Vec<(ObjectKey, Staleness)>>;

/// The plan's fake `IntroHandle`: the explorer's catalog of the profile as the snapshot, texts
/// and facets the test sets, and tickets it answers.
#[derive(Clone)]
struct FakeCatalog {
    profile: ProfileId,
    s: Rc<Shared>,
}

fn path_of(key: &ObjectKey) -> String {
    key.path
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

impl FakeCatalog {
    fn request(&self, what: String, answer: Answer, cx: &App) -> RefreshTicket {
        self.s.calls.borrow_mut().push(what.clone());
        let generation = adapter::catalog_of(self.profile, cx).map_or(0, |c| c.generation);
        match answer {
            Answer::Now => RefreshTicket::ready(Ok(generation)),
            Answer::Stub => RefreshTicket::not_implemented(),
            Answer::Hold => {
                let (ticket, tx) = RefreshTicket::pending();
                self.s.held.borrow_mut().push((what, tx));
                ticket
            }
        }
    }

    fn calls(&self) -> Vec<String> {
        self.s.calls.borrow().clone()
    }

    /// Answers the held request `what` with `generation` (a check: its verdict first).
    fn answer(&self, what: &str, generation: u64) {
        let verdict = {
            let mut v = self.s.held_verdicts.borrow_mut();
            v.iter()
                .position(|(w, _)| w == what)
                .map(|at| v.remove(at).1)
        };
        if let Some(tx) = verdict {
            let st = self.s.verdict.get().unwrap_or(Staleness::Current);
            let _ = tx.send(vec![(users_key(), st)]);
        }
        let mut held = self.s.held.borrow_mut();
        let at = held
            .iter()
            .position(|(w, _)| w == what)
            .unwrap_or_else(|| panic!("`{what}` is held"));
        held.remove(at).1.send(Ok(generation));
    }
}

struct FakeSources(Rc<Shared>);

impl DdlSources for FakeSources {
    fn text(&self, r: &SourceRef) -> Option<Name> {
        self.0.texts.borrow().get(r).cloned()
    }
    fn server_text(&self, object: &ObjectKey) -> Option<Name> {
        self.0.server.borrow().get(&path_of(object)).cloned()
    }
    fn grants(&self, object: &ObjectKey) -> Facet<GrantsFacet> {
        self.0
            .grants
            .borrow()
            .get(&path_of(object))
            .cloned()
            .unwrap_or_else(Facet::not_loaded)
    }
}

impl DdlCatalog for FakeCatalog {
    fn snapshot(&self, cx: &App) -> Option<Arc<CatalogSnapshot>> {
        adapter::catalog_of(self.profile, cx)
    }
    fn elevate(&self, key: ObjectKey, _: Level, cx: &App) -> RefreshTicket {
        self.request(format!("elevate {}", key.name()), Answer::Now, cx)
    }
    fn check_state(&self, key: ObjectKey, cx: &App) -> StateCheck {
        let a = self.s.check.get().unwrap_or(Answer::Now);
        let what = format!("check {}", key.name());
        let (tx, verdict) = tokio::sync::oneshot::channel();
        match a {
            Answer::Now => {
                let st = self.s.verdict.get().unwrap_or(Staleness::Current);
                let _ = tx.send(vec![(key, st)]);
            }
            Answer::Hold => self.s.held_verdicts.borrow_mut().push((what.clone(), tx)),
            // A stub decides nothing (the sender is dropped).
            Answer::Stub => drop(tx),
        }
        StateCheck {
            verdict,
            ticket: self.request(what, a, cx),
        }
    }
    fn refresh(&self, key: ObjectKey, cx: &App) -> RefreshTicket {
        self.request(format!("refresh {}", key.name()), Answer::Now, cx)
    }
    fn load_facet(&self, key: ObjectKey, kind: FacetKind, cx: &App) -> RefreshTicket {
        let a = self.s.facet.get().unwrap_or(Answer::Stub);
        let k = match kind {
            FacetKind::Grants => "grants",
            FacetKind::Sources => "sources",
            _ => "other",
        };
        self.request(format!("facet {k} {}", key.name()), a, cx)
    }
    fn sources(&self, _: &App) -> Rc<dyn DdlSources> {
        Rc::new(FakeSources(self.s.clone()))
    }
}

/// Installs a fake catalog for every profile (one shared state).
fn install_fake(vcx: &VisualTestContext) -> Rc<Shared> {
    let s: Rc<Shared> = Rc::default();
    let shared = s.clone();
    vcx.cx.update(|cx| {
        set_catalog_provider(
            Rc::new(move |profile, _| {
                Some(Rc::new(FakeCatalog {
                    profile,
                    s: shared.clone(),
                }) as Rc<dyn DdlCatalog>)
            }),
            cx,
        )
    });
    s
}

fn fake_of(s: &Rc<Shared>, profile: ProfileId) -> FakeCatalog {
    FakeCatalog {
        profile,
        s: s.clone(),
    }
}

// ---- catalogs ---------------------------------------------------------------------------------

fn loaded(stale: bool) -> LoadState {
    LoadState::Loaded {
        level: Level::Sources,
        at: SystemTime::UNIX_EPOCH,
        watermark: Watermark::None,
        stale,
    }
}

fn public() -> NsKey {
    NsKey::schema(["app", "public"])
}

fn view_obj(name: &str, text: &str) -> Subtree {
    Subtree::new(
        Obj::new(ObjectKind::View, name).with_props(Props::View(Arc::new(ViewProps {
            source: Some(SourceRef::of(text)),
            ..ViewProps::default()
        }))),
    )
}

/// `app.public` of PostgreSQL at generation `generation` holding `objects`, loaded.
fn pg_cat(generation: u64, objects: Vec<Subtree>, stale: bool) -> Arc<CatalogSnapshot> {
    catalog_gen(
        DbmsKind::Postgres,
        generation,
        vec![schema_at(
            DbmsKind::Postgres,
            &["app", "public"],
            false,
            Level::Sources,
            objects,
        )],
        vec![(public(), loaded(stale))],
    )
}

fn users(columns: &[(&str, &str)]) -> Subtree {
    table("users", Some(16_400), columns)
}

fn users_key() -> ObjectKey {
    ObjectKey {
        kind: ObjectKind::Table,
        path: ["app", "public", "users"]
            .into_iter()
            .map(Name::from)
            .collect(),
        server_id: Some(hacivat_meta::ServerId::Oid(16_400)),
        overload_sig: None,
        raw_kind: None,
    }
}

fn key(kind: ObjectKind, path: &[&str]) -> ObjectKey {
    ObjectKey {
        kind,
        path: path.iter().copied().map(Name::from).collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    }
}

fn pg_profile(name: &str) -> Profile {
    let mut p = test_profile(name, Some(EnvTag::Dev), "127.0.0.1");
    p.database = Some("app".to_string());
    p
}

fn show(vcx: &VisualTestContext, profile: ProfileId, cat: Arc<CatalogSnapshot>) {
    vcx.cx
        .update(|cx| adapter::set_catalog(profile, Some(cat), None, cx));
    vcx.run_until_parked();
}

/// A shell with connections, the fake catalog and a PostgreSQL profile showing `cat`.
fn pg_shell(
    cx: &mut TestAppContext,
    lang: &str,
    cat: Arc<CatalogSnapshot>,
) -> (&'static mut VisualTestContext, Rc<Shared>, ProfileId) {
    let vcx = shell_with_fake(cx, lang, answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    in_window(vcx, |window, _| window.activate_window());
    let s = install_fake(vcx);
    let id = add_profile(vcx, pg_profile("app-dev"));
    show(vcx, id, cat);
    (vcx, s, id)
}

fn open_view(vcx: &VisualTestContext, profile: ProfileId, key: &ObjectKey) {
    let ws = workspace(vcx);
    let k = key.clone();
    let opened = in_window(vcx, |window, cx| {
        ddl_view::open_ddl_view(&ws, profile, k, window, cx)
    });
    assert!(opened.is_ok(), "{opened:?}");
    vcx.run_until_parked();
}

fn state(vcx: &VisualTestContext, profile: ProfileId, key: &ObjectKey) -> DdlViewState {
    vcx.run_until_parked();
    vcx.cx
        .read(|cx| ddl_view::state_of(profile, key, cx))
        .expect("the DDL view is open")
}

fn shown_text(vcx: &VisualTestContext, profile: ProfileId, key: &ObjectKey) -> String {
    state(vcx, profile, key).text.expect("a text is shown")
}

/// The text of the view's read-only editor.
fn editor_text_of(vcx: &VisualTestContext, profile: ProfileId, key: &ObjectKey) -> String {
    vcx.cx.read(|cx| {
        let v = ddl_view::view_of(profile, key, cx).expect("the view");
        v.read(cx).editor().read(cx).snapshot().buffer.to_string()
    })
}

fn clipboard(vcx: &VisualTestContext) -> String {
    vcx.cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default()
}

/// The script Task 19 produces for `key` with no current schema and options `opts`.
fn create_script_text(
    cat: &Arc<CatalogSnapshot>,
    sources: &dyn DdlSources,
    key: &ObjectKey,
    dialect: DialectId,
    opts: GenOptions,
) -> String {
    let view = SnapshotView::new(cat.clone());
    let target = DdlTarget::new(dialect, ServerVersion::new(0, 0, 0));
    create_script_with(&view, sources, std::slice::from_ref(key), &target, &opts)
        .expect("a script")
        .text(DelimStyle::Semicolon)
}

// ---- elevation ----------------------------------------------------------------------------------

/// Elevation runs on the engine with Task 15's fakes: a view in a schema read at `Structure`
/// is read at `Sources` by one fragment of that view and nothing else; the view shows its
/// loading state while the fragment runs.
#[gpui::test]
fn view_ddl_elevates_with_a_fragment_only(cx: &mut TestAppContext) {
    let reports = NsKey::schema(["app", "reports"]);
    let intro = FakeIntrospector::new(DbmsKind::Postgres);
    intro.put_ns(
        NsKey::schema(["app", "public"]),
        vec![FakeObject::table("users", &["id"]).with_oid(20_000)],
    );
    intro.put_ns(
        reports.clone(),
        vec![FakeObject::view("v_sales", &["id"]).with_oid(20_100)],
    );
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    in_window(vcx, |window, _| window.activate_window());
    let dir = temp_dir(vcx).join("fake-cache");
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .runtime()
    });
    let src = Arc::new(support::live::FakeSource::new(rt, intro, dir));
    let as_source: Arc<dyn CatalogSource> = src.clone();
    vcx.cx.update(|cx| hlive::set_source(as_source, cx));
    let id = add_profile(vcx, pg_profile("app-dev"));
    vcx.cx.update(|cx| {
        let tree = hacivat_app::connections::explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| {
            t.set_expanded(adapter::profile_key(id), true, cx)
        });
    });
    connect(vcx, "app-dev");
    let handle = || vcx.cx.read(|cx| hlive::handle(id, cx));
    wait_until(vcx, "the engine", |_| handle().is_some());
    // `reports` is not the current schema: it is read at Structure.
    let ticket = handle().expect("the engine").expand(reports.clone());
    let loaded_at =
        |vcx: &VisualTestContext| {
            vcx.cx.read(|cx| {
                hlive::handle(id, cx).and_then(|h| {
                    match h.catalog().current().state(
                        &hacivat_meta::snapshot::ContainerKey::Namespace(reports.clone()),
                    ) {
                        LoadState::Loaded { level, .. } => Some(*level),
                        _ => None,
                    }
                })
            })
        };
    drop(ticket);
    wait_until(vcx, "reports loaded", |vcx| {
        loaded_at(vcx) == Some(Level::Structure)
    });
    // Nothing else runs (the on-connect passes are done), so every call below is the view's.
    wait_until(vcx, "the engine idle", |vcx| {
        vcx.cx.read(|cx| {
            hlive::handle(id, cx).is_some_and(|h| h.progress().borrow().tasks.is_empty())
        })
    });
    let view_key = vcx.cx.read(|cx| {
        let snap = hlive::handle(id, cx).unwrap().catalog().current();
        let ns = snap.namespaces.get(&reports).unwrap().clone();
        let (idx, _) = ns
            .children(0)
            .find(|(_, o)| &**o.name() == "v_sales")
            .expect("the view");
        ns.object_key(idx).expect("its key")
    });
    let before = src.intro.calls().len();
    // The fragment is held, so the loading state shows.
    let gate = src.intro.gate(|c| matches!(c, FakeCall::Fragment { .. }));
    open_view(vcx, id, &view_key);
    let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = entered.clone();
    let g = gate.clone();
    vcx.cx
        .read(|cx| {
            cx.global::<hacivat_app::connections::ConnectionsModel>()
                .runtime()
        })
        .spawn(async move {
            g.entered().await;
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    wait_until(vcx, "the fragment runs", |_| {
        entered.load(std::sync::atomic::Ordering::SeqCst)
    });
    let st = state(vcx, id, &view_key);
    assert_eq!(st.body, Body::Loading);
    assert!(
        texts_drawn_by(vcx, "ddl_view/view.rs")
            .iter()
            .any(|t| t == "Reading the definition of v_sales…"),
        "{:?}",
        texts_drawn_by(vcx, "ddl_view/view.rs")
    );
    gate.release();
    wait_until(vcx, "the elevation answered", |vcx| {
        state(vcx, id, &view_key).body != Body::Loading
    });
    let after: Vec<FakeCall> = src.intro.calls()[before..].to_vec();
    let reads: Vec<&FakeCall> = after
        .iter()
        .filter(|c| !matches!(c, FakeCall::CheckState { .. }))
        .collect();
    assert_eq!(reads.len(), 1, "one fragment and no pass: {after:?}");
    let FakeCall::Fragment { ns, keys, level } = reads[0] else {
        panic!("{after:?}");
    };
    assert_eq!(*ns, reports);
    assert_eq!(keys.as_slice(), std::slice::from_ref(&view_key));
    assert_eq!(*level, Level::Sources);
    // At this base no source text has a home (Task 15b): the view says the definition is not
    // loaded, never a text that is not the object's.
    let st = state(vcx, id, &view_key);
    assert_eq!(st.body, Body::NotLoaded);
    assert_eq!(st.text, None);
    let _ = src;
}

/// T25 round 1: Task 22's real state check through the engine (T15's fakes, whose stamps are
/// the objects' versions): the verdict reaches the view (`IntroHandle::check_state_verdicts`),
/// so the table reads current; a check that fails decides nothing ("couldn't check").
#[gpui::test]
fn engine_state_check_verdict_reaches_the_view(cx: &mut TestAppContext) {
    let public = NsKey::schema(["app", "public"]);
    let intro = FakeIntrospector::new(DbmsKind::Postgres);
    intro.put_ns(
        public.clone(),
        vec![FakeObject::table("users", &["id"]).with_oid(20_000)],
    );
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    in_window(vcx, |window, _| window.activate_window());
    let dir = temp_dir(vcx).join("fake-cache");
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .runtime()
    });
    let src = Arc::new(support::live::FakeSource::new(rt, intro, dir));
    let as_source: Arc<dyn CatalogSource> = src.clone();
    vcx.cx.update(|cx| hlive::set_source(as_source, cx));
    let id = add_profile(vcx, pg_profile("app-dev"));
    vcx.cx.update(|cx| {
        let tree = hacivat_app::connections::explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| {
            t.set_expanded(adapter::profile_key(id), true, cx)
        });
    });
    connect(vcx, "app-dev");
    let users = || {
        vcx.cx.read(|cx| {
            let snap = hlive::handle(id, cx)?.catalog().current();
            let ns = snap.namespaces.get(&public)?.clone();
            let (idx, _) = ns.children(0).find(|(_, o)| &**o.name() == "users")?;
            ns.object_key(idx)
        })
    };
    wait_until(vcx, "public loaded", |_| users().is_some());
    wait_until(vcx, "the engine idle", |vcx| {
        vcx.cx.read(|cx| {
            hlive::handle(id, cx).is_some_and(|h| h.progress().borrow().tasks.is_empty())
        })
    });
    let k = users().expect("users");
    open_view(vcx, id, &k);
    wait_until(vcx, "the verdict", |vcx| {
        state(vcx, id, &k).freshness != Freshness::Checking
    });
    let st = state(vcx, id, &k);
    assert_eq!(st.freshness, Freshness::Current, "{st:?}");
    assert!(
        src.intro
            .calls()
            .iter()
            .any(|c| matches!(c, FakeCall::CheckState { .. })),
        "Task 22's stamps were read"
    );
    // The next check fails on the server: nothing is decided.
    src.intro.fail(
        |c| matches!(c, FakeCall::CheckState { .. }),
        hacivat_introspect::introspector::IntroError::Protocol(Name::from("test")),
        1,
    );
    click(vcx, "button-ddl-refresh");
    wait_until(vcx, "the failed check", |vcx| {
        state(vcx, id, &k).freshness == Freshness::Unknown
    });
    assert!(
        state(vcx, id, &k)
            .notices
            .contains(&"hv-ddl-view-unknown".to_string())
    );
}

// ---- the text -----------------------------------------------------------------------------------

#[gpui::test]
fn shown_text_equals_create_script_bytes(cx: &mut TestAppContext) {
    let cat = pg_cat(
        1,
        vec![
            users(&[("id", "integer"), ("name", "text")]),
            view_obj("v_open", "SELECT id\n  FROM users"),
        ],
        false,
    );
    let (vcx, s, id) = pg_shell(cx, "en", cat.clone());
    open_view(vcx, id, &users_key());
    assert_eq!(tab_title(vcx), "users · DDL");
    let want = create_script_text(
        &cat,
        &FakeSources(s.clone()),
        &users_key(),
        DialectId::Postgres,
        GenOptions::default(),
    );
    assert_eq!(
        want,
        "CREATE TABLE public.users (\n    id integer,\n    name text\n);\n"
    );
    let st = state(vcx, id, &users_key());
    assert_eq!(st.body, Body::Script);
    assert_eq!(st.text.as_deref(), Some(want.as_str()));
    assert_eq!(editor_text_of(vcx, id, &users_key()), want);
    assert!(!st.server_text);
    // Grants were asked for once (the stub leaves them not loaded, and the script says so).
    assert_eq!(
        fake_of(&s, id)
            .calls()
            .iter()
            .filter(|c| c.starts_with("facet grants"))
            .count(),
        1
    );
    assert_eq!(st.grants, GrantsUx::NotLoaded);
    assert!(st.notices.contains(&"ddl-grants-not-loaded".to_string()));
    // A view's text is its server text, verbatim.
    s.texts.borrow_mut().insert(
        SourceRef::of("SELECT id\n  FROM users"),
        Name::from("SELECT id\n  FROM users"),
    );
    let v = key(ObjectKind::View, &["app", "public", "v_open"]);
    open_view(vcx, id, &v);
    let want = create_script_text(
        &cat,
        &FakeSources(s.clone()),
        &v,
        DialectId::Postgres,
        GenOptions::default(),
    );
    let st = state(vcx, id, &v);
    assert_eq!(st.text.as_deref(), Some(want.as_str()));
    assert!(want.contains("SELECT id\n  FROM users"), "{want}");
    assert!(st.server_text);
}

#[gpui::test]
fn options_change_rerenders_and_persists_per_profile(cx: &mut TestAppContext) {
    let cat = pg_cat(
        1,
        vec![
            users(&[("id", "integer")]),
            table("orders", Some(16_401), &[("id", "integer")]),
        ],
        false,
    );
    let (vcx, _s, id) = pg_shell(cx, "en", cat.clone());
    open_view(vcx, id, &users_key());
    let st = state(vcx, id, &users_key());
    assert_eq!(st.options, DdlViewOptions::default());
    assert_eq!(st.options.qualify, Qualify::Auto);
    assert_eq!(st.options.placement, Placement::InsideTable);
    assert!(shown_text(vcx, id, &users_key()).starts_with("CREATE TABLE public.users"));
    // Qualification: Never (the strip's second segment).
    click(vcx, "segmented-ddl-qualify-1");
    let st = state(vcx, id, &users_key());
    assert_eq!(st.options.qualify, Qualify::Never);
    assert!(
        st.text
            .as_deref()
            .unwrap()
            .starts_with("CREATE TABLE users ("),
        "{:?}",
        st.text
    );
    // Remembered per profile: written to the profile's data file…
    let task = vcx.cx.update(userdata::barrier);
    wait_task(vcx, task);
    vcx.cx.update(userdata::forget_all);
    vcx.cx.update(|cx| userdata::ensure(id, cx));
    wait_until(vcx, "the data read again", |vcx| {
        vcx.cx.read(|cx| userdata::get(id, cx)).is_some()
    });
    let saved = vcx
        .cx
        .read(|cx| userdata::get(id, cx))
        .expect("read")
        .ddl_view;
    assert_eq!(saved.qualify, Qualify::Never);
    // …and the next object of the profile opens with it.
    let orders = key(ObjectKind::Table, &["app", "public", "orders"]);
    open_view(vcx, id, &orders);
    assert_eq!(state(vcx, id, &orders).options.qualify, Qualify::Never);
    assert!(shown_text(vcx, id, &orders).starts_with("CREATE TABLE orders ("));
    // Another profile keeps its own (the default).
    let other = add_profile(vcx, pg_profile("app-two"));
    show(vcx, other, cat);
    open_view(vcx, other, &users_key());
    assert_eq!(
        state(vcx, other, &users_key()).options.qualify,
        Qualify::Auto
    );
}

#[gpui::test]
fn copy_puts_raw_sql_on_the_clipboard(cx: &mut TestAppContext) {
    let cat = pg_cat(
        1,
        vec![users(&[("id", "integer"), ("name", "text")])],
        false,
    );
    let (vcx, _s, id) = pg_shell(cx, "en", cat);
    open_view(vcx, id, &users_key());
    let text = shown_text(vcx, id, &users_key());
    click(vcx, "button-ddl-copy");
    assert_eq!(clipboard(vcx), text);
    assert!(clipboard(vcx).contains('\n') && !clipboard(vcx).contains('\r'));
    assert!(
        toast_texts(vcx).contains(&"Copied the DDL of users".to_string()),
        "{:?}",
        toast_texts(vcx)
    );
    // The palette command copies the active DDL view's text too.
    vcx.cx
        .write_to_clipboard(gpui::ClipboardItem::new_string("x".to_string()));
    assert!(run(vcx, "ddl_view.copy"));
    assert_eq!(clipboard(vcx), text);
}

#[gpui::test]
fn open_in_console_inserts_without_running(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer")])], false);
    let (vcx, _s, id) = pg_shell(cx, "en", cat);
    connect(vcx, "app-dev");
    wait_until(vcx, "connected", |vcx| {
        matches!(
            live(vcx, "app-dev"),
            hacivat_session::LiveState::Connected(_)
        )
    });
    open_view(vcx, id, &users_key());
    let text = shown_text(vcx, id, &users_key());
    let tabs = tab_count(vcx);
    click(vcx, "button-ddl-console");
    assert_eq!(tab_count(vcx), tabs + 1, "a new console");
    let console = vcx.cx.read(|cx| {
        Consoles::ids(cx)
            .into_iter()
            .filter(|c| {
                Consoles::get(cx, *c).is_some_and(|s| s.profile == Some(id) && s.editor.is_some())
            })
            .max_by_key(|c| Consoles::get(cx, *c).map(|s| s.n))
    });
    wait_until(vcx, "the console's editor", |vcx| {
        vcx.cx.read(|cx| {
            Consoles::ids(cx).into_iter().any(|c| {
                Consoles::get(cx, c).is_some_and(|s| s.profile == Some(id) && s.editor.is_some())
            })
        })
    });
    let console = console.or_else(|| {
        vcx.cx.read(|cx| {
            Consoles::ids(cx)
                .into_iter()
                .filter(|c| {
                    Consoles::get(cx, *c)
                        .is_some_and(|s| s.profile == Some(id) && s.editor.is_some())
                })
                .max_by_key(|c| Consoles::get(cx, *c).map(|s| s.n))
        })
    });
    let console = console.expect("the new console");
    wait_until(vcx, "the inserted text", |vcx| {
        editor_text(vcx, console) == text
    });
    // Not run (review I-4: a condition, not a sleep): wait until the console could run (its
    // session idle and its SQL context read, the moment a queued run fires, as Open data's
    // does), let every task run, and the session saw no statement of the text.
    wait_until(vcx, "the console ready to run", |vcx| {
        vcx.cx.read(|cx| {
            let idle = Consoles::get(cx, console)
                .and_then(|s| s.session.as_ref())
                .is_some_and(|s| {
                    s.state().borrow().state == hacivat_session::session::SessionState::Idle
                });
            idle && hacivat_app::exec_ui::sql_context(console, cx)
                != hacivat_app::exec_ui::context::SqlContext::Pending
        })
    });
    vcx.run_until_parked();
    let f = fake(vcx);
    let ran = f
        .typed_log()
        .into_iter()
        .any(|c| matches!(c, hacivat_driver::fake::FakeTypedCall::Query(q) if q.contains("CREATE TABLE")));
    assert!(!ran, "the DDL was not run");
}

// ---- freshness (surface 11) -------------------------------------------------------------------

#[gpui::test]
fn stale_object_notice_and_reload(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer")])], false);
    let (vcx, s, id) = pg_shell(cx, "en", cat);
    // The check is held: the view says nothing yet.
    s.check.set(Some(Answer::Hold));
    open_view(vcx, id, &users_key());
    let old = shown_text(vcx, id, &users_key());
    assert_eq!(state(vcx, id, &users_key()).freshness, Freshness::Checking);
    // The check found a change and re-read it: a column was added on the server.
    let changed = pg_cat(
        2,
        vec![users(&[("id", "integer"), ("email", "text")])],
        false,
    );
    show(vcx, id, changed);
    s.verdict.set(Some(Staleness::Changed));
    fake_of(&s, id).answer("check users", 2);
    let st = state(vcx, id, &users_key());
    assert_eq!(st.freshness, Freshness::Changed);
    assert!(st.notices.contains(&"hv-ddl-view-stale".to_string()));
    // The shown text is the one loaded, until Reload.
    assert_eq!(st.text.as_deref(), Some(old.as_str()));
    assert!(
        texts_drawn_by(vcx, "ddl_view/view.rs")
            .iter()
            .any(|t| t == "users changed on the server since it was loaded"),
        "{:?}",
        texts_drawn_by(vcx, "ddl_view/view.rs")
    );
    // Reload re-reads, then checks again: `Changed` with a re-read whose script is the one now
    // shown is current.
    s.check.set(Some(Answer::Now));
    click(vcx, "button-ddl-reload");
    let st = state(vcx, id, &users_key());
    assert!(
        st.text.as_deref().unwrap().contains("email text"),
        "{:?}",
        st.text
    );
    assert_eq!(st.freshness, Freshness::Current);
    assert!(!st.notices.contains(&"hv-ddl-view-stale".to_string()));
    assert!(
        fake_of(&s, id)
            .calls()
            .contains(&"refresh users".to_string())
    );
    // A namespace marked stale (refresh after DDL with Ask) shows the stale form at once; a
    // check that answers `Unknown` keeps it ("couldn't check").
    s.check.set(Some(Answer::Now));
    s.verdict.set(Some(Staleness::Unknown));
    show(
        vcx,
        id,
        pg_cat(
            3,
            vec![users(&[("id", "integer"), ("email", "text")])],
            true,
        ),
    );
    click(vcx, "button-ddl-refresh");
    let st = state(vcx, id, &users_key());
    assert_eq!(st.freshness, Freshness::Unknown);
    assert!(st.notices.contains(&"hv-ddl-view-unknown".to_string()));
}

/// T25 round 1 (coordinator item 2): only Task 22's `Current` verdict shows the text as
/// current. `Unknown` and a `Dropped` the catalog does not confirm read "couldn't check", even
/// on a namespace that is not stale (before, the stale flag alone decided).
#[gpui::test]
fn unknown_verdict_never_shows_as_current(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer")])], false);
    let (vcx, s, id) = pg_shell(cx, "en", cat);
    s.verdict.set(Some(Staleness::Unknown));
    open_view(vcx, id, &users_key());
    let st = state(vcx, id, &users_key());
    assert_eq!(st.freshness, Freshness::Unknown);
    assert!(st.notices.contains(&"hv-ddl-view-unknown".to_string()));
    assert!(
        texts_drawn_by(vcx, "ddl_view/view.rs").iter().any(|t| t
            == "Hacivat couldn't check whether users changed on the server since it was loaded"),
        "{:?}",
        texts_drawn_by(vcx, "ddl_view/view.rs")
    );
    // A drop the catalog does not show (the re-read failed) is not a fact either.
    s.verdict.set(Some(Staleness::Dropped));
    click(vcx, "button-ddl-reload");
    assert_eq!(state(vcx, id, &users_key()).freshness, Freshness::Unknown);
    // `Current` is current: no notice.
    s.verdict.set(Some(Staleness::Current));
    click(vcx, "button-ddl-reload");
    let st = state(vcx, id, &users_key());
    assert_eq!(st.freshness, Freshness::Current);
    assert!(!st.notices.iter().any(|n| n.starts_with("hv-ddl-view-")));
    // F5 in the text reloads (a check again: `Unknown` now).
    s.verdict.set(Some(Staleness::Unknown));
    let editor = vcx.cx.read(|cx| {
        ddl_view::view_of(id, &users_key(), cx)
            .unwrap()
            .read(cx)
            .editor()
            .clone()
    });
    in_window(vcx, |window, cx| {
        let focus = gpui::Focusable::focus_handle(editor.read(cx), cx);
        window.focus(&focus, cx);
    });
    vcx.run_until_parked();
    assert!(
        context_stack(vcx)
            .0
            .contains(&ddl_view::DDL_VIEW_CODE_CONTEXT)
    );
    let refreshes = |s: &Rc<Shared>| {
        s.calls
            .borrow()
            .iter()
            .filter(|c| *c == "refresh users")
            .count()
    };
    let before = refreshes(&s);
    vcx.simulate_keystrokes("f5");
    vcx.run_until_parked();
    assert_eq!(refreshes(&s), before + 1, "F5 reloads the view");
    assert_eq!(state(vcx, id, &users_key()).freshness, Freshness::Unknown);
}

/// The T19 carry (m6): server text from the `Sources` facet is current only while that facet
/// is loaded and not stale; `DdlSources::server_text` has no load state of its own.
#[gpui::test]
fn server_text_is_current_only_with_its_facet_loaded(cx: &mut TestAppContext) {
    let (vcx, s, id) = mysql_shell(cx, mysql_cat(vec![table("t", None, &[("id", "int")])]));
    s.server.borrow_mut().insert(
        "api.t".to_string(),
        Name::from("CREATE TABLE `t` (\n  `id` int DEFAULT NULL\n) ENGINE=InnoDB"),
    );
    let k = key(ObjectKind::Table, &["api", "t"]);
    open_view(vcx, id, &k);
    assert_eq!(state(vcx, id, &k).freshness, Freshness::Current);
    click(vcx, "checkbox-ddl-server");
    // Server text, a `Current` verdict, but no facet state vouches for the text.
    let st = state(vcx, id, &k);
    assert!(st.server_text);
    click(vcx, "button-ddl-refresh");
    assert_eq!(state(vcx, id, &k).freshness, Freshness::Unknown);
    // The facet loaded and not stale: current.
    let mut cat = (*mysql_cat(vec![table("t", None, &[("id", "int")])])).clone();
    cat.generation = 2;
    let mut states = (*cat.states).clone();
    states.insert(
        hacivat_meta::snapshot::ContainerKey::Facet {
            ns: NsKey::schema(["api"]),
            object: k.clone(),
            facet: FacetKind::Sources,
        },
        loaded(false),
    );
    cat.states = Arc::new(states);
    show(vcx, id, Arc::new(cat));
    click(vcx, "button-ddl-refresh");
    assert_eq!(state(vcx, id, &k).freshness, Freshness::Current);
}

/// Ruling R-W4-12: on SQLite the server toggle is shown, checked and fixed, with a tooltip
/// saying why (EN and TR).
#[gpui::test]
fn sqlite_server_toggle_is_fixed_with_a_tooltip(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    in_window(vcx, |window, _| window.activate_window());
    let s = install_fake(vcx);
    let mut p = test_profile("local-file", Some(EnvTag::Local), "127.0.0.1");
    p.engine = DbmsKind::Sqlite;
    let id = add_profile(vcx, p);
    show(
        vcx,
        id,
        catalog_gen(
            DbmsKind::Sqlite,
            1,
            vec![schema_at(
                DbmsKind::Sqlite,
                &["main"],
                false,
                Level::Sources,
                vec![table("t", None, &[("id", "integer")])],
            )],
            vec![(NsKey::schema(["main"]), loaded(false))],
        ),
    );
    s.server.borrow_mut().insert(
        "main.t".to_string(),
        Name::from("CREATE TABLE t (id integer CHECK (id > 0))"),
    );
    let k = key(ObjectKind::Table, &["main", "t"]);
    open_view(vcx, id, &k);
    let st = state(vcx, id, &k);
    assert!(st.controls.contains(&"server"));
    assert!(st.options.server_definitions || st.server_text);
    assert_eq!(
        st.server_tip.as_deref(),
        Some(
            "SQLite always shows the server's text: CHECK constraints and generated columns exist only there."
        )
    );
    assert!(vcx.debug_bounds("ddl-server-fixed").is_some());
    // Fixed: a click changes nothing.
    click(vcx, "checkbox-ddl-server");
    assert!(state(vcx, id, &k).server_text);
    switch_to(vcx, "tr");
    wait_until(vcx, "Turkish chrome", |vcx| {
        state(vcx, id, &k).server_tip.as_deref()
            == Some(
                "SQLite her zaman sunucunun metnini gösterir: CHECK kısıtları ve üretilen sütunlar yalnızca orada bulunur.",
            )
    });
}

/// With the real stubs (no fake catalog: the profile's engine is not open), the check cannot
/// run, and the view says it couldn't check (plan Task 25: "shown as may be out of date").
#[gpui::test]
fn stub_check_state_is_shown_as_unknown(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    in_window(vcx, |window, _| window.activate_window());
    let id = add_profile(vcx, pg_profile("app-dev"));
    show(vcx, id, pg_cat(1, vec![users(&[("id", "integer")])], false));
    open_view(vcx, id, &users_key());
    let st = state(vcx, id, &users_key());
    assert_eq!(st.body, Body::Script);
    assert_eq!(st.freshness, Freshness::Unknown);
    assert!(st.notices.contains(&"hv-ddl-view-unknown".to_string()));
    // Grants: `load_facet` is a stub, so they stay not loaded and the script says so.
    assert_eq!(st.grants, GrantsUx::NotLoaded);
    assert!(st.notices.contains(&"ddl-grants-not-loaded".to_string()));
}

#[gpui::test]
fn dropped_object_notice(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer")])], false);
    let (vcx, s, id) = pg_shell(cx, "en", cat);
    s.check.set(Some(Answer::Hold));
    open_view(vcx, id, &users_key());
    let old = shown_text(vcx, id, &users_key());
    show(vcx, id, pg_cat(2, vec![], false));
    fake_of(&s, id).answer("check users", 2);
    let st = state(vcx, id, &users_key());
    assert_eq!(st.freshness, Freshness::Gone);
    assert!(st.notices.contains(&"hv-ddl-view-gone".to_string()));
    // The last text loaded stays.
    assert_eq!(st.text.as_deref(), Some(old.as_str()));
    let drawn = texts_drawn_by(vcx, "ddl_view/view.rs");
    assert!(
        drawn
            .iter()
            .any(|t| t == "users no longer exists on the server"),
        "{drawn:?}"
    );
    assert!(
        drawn
            .iter()
            .any(|t| t == "The text below is the last one loaded.")
    );
    // Close tab closes it.
    let tabs = tab_count(vcx);
    click(vcx, "button-ddl-close");
    assert_eq!(tab_count(vcx), tabs - 1);
}

// ---- kinds and engines --------------------------------------------------------------------------

#[gpui::test]
fn unsupported_kind_notice(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![unknown("weird", "q")], false);
    let (vcx, _s, id) = pg_shell(cx, "en", cat.clone());
    let k = vcx.cx.read(|_| {
        let ns = cat.namespaces.get(&public()).unwrap().clone();
        let (idx, _) = ns.children(0).next().unwrap();
        ns.object_key(idx).unwrap()
    });
    open_view(vcx, id, &k);
    let st = state(vcx, id, &k);
    assert_eq!(st.body, Body::Unsupported);
    assert_eq!(st.text, None);
    assert!(st.notices.contains(&"hv-ddl-view-unsupported".to_string()));
    let drawn = texts_drawn_by(vcx, "ddl_view/view.rs");
    assert!(
        drawn
            .iter()
            .any(|t| t == "Hacivat can't show DDL for this kind of object yet."),
        "{drawn:?}"
    );
    assert!(
        drawn
            .iter()
            .any(|t| t == "weird stays in the explorer, and Copy qualified name works.")
    );
    // Copy and Open in console wait for a text.
    assert!(!run(vcx, "ddl_view.copy"));
    let v = vcx.cx.read(|cx| ddl_view::view_of(id, &k, cx).unwrap());
    assert!(!vcx.cx.read(|cx| v.read(cx).can_copy()));
}

fn mysql_shell(
    cx: &mut TestAppContext,
    cat: Arc<CatalogSnapshot>,
) -> (&'static mut VisualTestContext, Rc<Shared>, ProfileId) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    in_window(vcx, |window, _| window.activate_window());
    let s = install_fake(vcx);
    let mut p = test_profile("api-dev", Some(EnvTag::Dev), "127.0.0.1");
    p.engine = DbmsKind::MySql;
    let id = add_profile(vcx, p);
    show(vcx, id, cat);
    (vcx, s, id)
}

fn mysql_cat(objects: Vec<Subtree>) -> Arc<CatalogSnapshot> {
    catalog_gen(
        DbmsKind::MySql,
        1,
        vec![schema_at(
            DbmsKind::MySql,
            &["api"],
            false,
            Level::Sources,
            objects,
        )],
        vec![(NsKey::schema(["api"]), loaded(false))],
    )
}

#[gpui::test]
fn mysql_executable_comment_warning(cx: &mut TestAppContext) {
    let text = "CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v_x` AS select 1 AS `a` /*!50001 WITH CASCADED CHECK OPTION */";
    let (vcx, s, id) = mysql_shell(cx, mysql_cat(vec![view_obj("v_x", text)]));
    s.texts
        .borrow_mut()
        .insert(SourceRef::of(text), Name::from(text));
    let k = key(ObjectKind::View, &["api", "v_x"]);
    open_view(vcx, id, &k);
    let st = state(vcx, id, &k);
    assert!(
        st.text.as_deref().unwrap().contains("/*!50001"),
        "{:?}",
        st.text
    );
    assert!(
        st.notices
            .contains(&"ddl-warn-executable-comment".to_string())
    );
    let drawn = texts_drawn_by(vcx, "ddl_view/view.rs");
    assert!(
        drawn
            .iter()
            .any(|t| t.contains("MySQL executable comments")),
        "{drawn:?}"
    );
}

#[gpui::test]
fn server_definitions_toggle_hidden_on_postgres(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer")])], false);
    let (vcx, _s, id) = pg_shell(cx, "en", cat);
    open_view(vcx, id, &users_key());
    let st = state(vcx, id, &users_key());
    assert!(!st.controls.contains(&"server"), "{:?}", st.controls);
    assert!(st.controls.contains(&"qualification") && st.controls.contains(&"constraints"));
    assert!(vcx.debug_bounds("checkbox-ddl-server").is_none());
    assert!(vcx.debug_bounds("checkbox-ddl-grants").is_some());
}

/// MySQL shows the toggle; on, a table's text is `SHOW CREATE TABLE`'s and the strip hides
/// Qualification and Constraints (they shape generated text only, NOTES ruling 16).

#[gpui::test]
fn server_definitions_toggle_on_mysql_shows_server_text(cx: &mut TestAppContext) {
    let (vcx, s, id) = mysql_shell(cx, mysql_cat(vec![table("t", None, &[("id", "int")])]));
    let server = "CREATE TABLE `t` (\n  `id` int DEFAULT NULL\n) ENGINE=InnoDB";
    s.server
        .borrow_mut()
        .insert("api.t".to_string(), Name::from(server));
    let k = key(ObjectKind::Table, &["api", "t"]);
    open_view(vcx, id, &k);
    let st = state(vcx, id, &k);
    assert!(st.controls.contains(&"server"), "{:?}", st.controls);
    assert!(!st.server_text);
    assert!(
        st.text
            .as_deref()
            .unwrap()
            .starts_with("CREATE TABLE api.t"),
        "{:?}",
        st.text
    );
    click(vcx, "checkbox-ddl-server");
    let st = state(vcx, id, &k);
    assert!(st.options.server_definitions);
    assert!(st.server_text);
    assert_eq!(st.text.as_deref(), Some(format!("{server};\n").as_str()));
    assert!(!st.controls.contains(&"qualification"));
    assert!(!st.controls.contains(&"constraints"));
    let drawn = texts_drawn_by(vcx, "ddl_view/view.rs");
    assert!(
        drawn.iter().any(|t| t == "Text from the server"),
        "{drawn:?}"
    );
}

#[gpui::test]
fn text_identical_in_en_and_tr(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer"), ("ad", "text")])], false);
    let (vcx, _s, id) = pg_shell(cx, "en", cat);
    open_view(vcx, id, &users_key());
    let en = shown_text(vcx, id, &users_key());
    assert!(button_labels(vcx).contains(&"Copy".to_string()));
    switch_to(vcx, "tr");
    wait_until(vcx, "Turkish chrome", |vcx| {
        button_labels(vcx).contains(&"Kopyala".to_string())
    });
    assert_eq!(shown_text(vcx, id, &users_key()), en);
    assert_eq!(editor_text_of(vcx, id, &users_key()), en);
}

#[gpui::test]
fn buffer_refuses_edits(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer")])], false);
    let (vcx, _s, id) = pg_shell(cx, "en", cat);
    open_view(vcx, id, &users_key());
    let text = shown_text(vcx, id, &users_key());
    let editor = vcx.cx.read(|cx| {
        ddl_view::view_of(id, &users_key(), cx)
            .unwrap()
            .read(cx)
            .editor()
            .clone()
    });
    assert!(vcx.cx.read(|cx| editor.read(cx).config().read_only));
    in_window(vcx, |window, cx| {
        let focus = gpui::Focusable::focus_handle(editor.read(cx), cx);
        window.focus(&focus, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_input("DROP ");
    vcx.simulate_keystrokes("backspace enter");
    vcx.cx
        .write_to_clipboard(gpui::ClipboardItem::new_string("x".to_string()));
    let paste = if cfg!(target_os = "macos") {
        "cmd-v"
    } else {
        "ctrl-v"
    };
    vcx.simulate_keystrokes(paste);
    vcx.run_until_parked();
    assert_eq!(editor_text_of(vcx, id, &users_key()), text);
}

#[gpui::test]
fn ddl_view_grants_toggle_loads_facet(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer")])], false);
    let (vcx, s, id) = pg_shell(cx, "en", cat);
    s.facet.set(Some(Answer::Hold));
    open_view(vcx, id, &users_key());
    let st = state(vcx, id, &users_key());
    assert_eq!(st.grants, GrantsUx::Loading);
    let f = fake_of(&s, id);
    assert_eq!(
        f.calls()
            .iter()
            .filter(|c| *c == "facet grants users")
            .count(),
        1
    );
    // The facet arrives (the fake publishes it and answers the ticket).
    s.grants.borrow_mut().insert(
        "app.public.users".to_string(),
        Facet {
            state: loaded(false),
            value: Some(Arc::new(GrantsFacet {
                entries: vec![Grant {
                    grantee: Name::from("reader"),
                    privilege: Name::from("SELECT"),
                    columns: Vec::new(),
                    grantable: false,
                }],
            })),
        },
    );
    f.answer("facet grants users", 1);
    let st = state(vcx, id, &users_key());
    assert_eq!(st.grants, GrantsUx::Loaded);
    assert!(
        st.text
            .as_deref()
            .unwrap()
            .contains("GRANT SELECT ON TABLE public.users TO reader;"),
        "{:?}",
        st.text
    );
    // Grants off: no GRANT and no notice; the facet is not asked again.
    click(vcx, "checkbox-ddl-grants");
    let st = state(vcx, id, &users_key());
    assert!(!st.options.grants);
    assert_eq!(st.grants, GrantsUx::Off);
    assert!(!st.text.as_deref().unwrap().contains("GRANT"));
    // A partial facet says so.
    s.grants.borrow_mut().insert(
        "app.public.users".to_string(),
        Facet {
            state: LoadState::Partial {
                reason: hacivat_meta::snapshot::IntroErrorCode::PermissionDenied,
            },
            value: Some(Arc::new(GrantsFacet::default())),
        },
    );
    click(vcx, "checkbox-ddl-grants");
    let st = state(vcx, id, &users_key());
    assert_eq!(st.grants, GrantsUx::Partial);
    assert!(
        st.notices
            .contains(&"hv-ddl-view-grants-partial".to_string())
    );
    assert_eq!(
        f.calls()
            .iter()
            .filter(|c| *c == "facet grants users")
            .count(),
        1
    );
}

// ---- provided toolbar actions (R-M4-19) -------------------------------------------------------

struct EditSource;

impl ExplorerActionProvider for EditSource {
    fn actions(&self, node: &NodeRef, surface: ActionSurface, _: &App) -> Vec<ProvidedAction> {
        if surface != ActionSurface::DdlViewToolbar || node.kind != Some(ObjectKind::Table) {
            return Vec::new();
        }
        vec![ProvidedAction {
            slot: ActionSlot::Source,
            id: "test.edit_source",
            label: "Edit source".into(),
            enabled: true,
        }]
    }
}

#[gpui::test]
fn toolbar_shows_provided_actions(cx: &mut TestAppContext) {
    let cat = pg_cat(
        1,
        vec![users(&[("id", "integer")]), view_obj("v_open", "SELECT 1")],
        false,
    );
    let (vcx, _s, id) = pg_shell(cx, "en", cat);
    let ran = Rc::new(Cell::new(0));
    let seen = ran.clone();
    vcx.cx.update(|cx| {
        register_action_provider(Rc::new(EditSource), cx);
        let _ = cx.default_global::<ActionRegistry>().register(ActionDef {
            id: "test.edit_source",
            title_id: "hv-ddl-view-copy",
            context: "Workspace",
            run: Rc::new(move |_, _| seen.set(seen.get() + 1)),
            enabled: |_| true,
        });
    });
    open_view(vcx, id, &users_key());
    assert_eq!(
        state(vcx, id, &users_key()).toolbar_actions,
        ["test.edit_source"]
    );
    assert!(button_labels(vcx).contains(&"Edit source".to_string()));
    click(vcx, "button-test.edit_source");
    assert_eq!(ran.get(), 1);
    // A kind the provider does not answer for gets nothing.
    let v = key(ObjectKind::View, &["app", "public", "v_open"]);
    open_view(vcx, id, &v);
    assert!(state(vcx, id, &v).toolbar_actions.is_empty());
}

// ---- the explorer's View DDL and the tab table (T1b carry m3) ----------------------------------

#[gpui::test]
fn view_ddl_command_opens_the_cursor_objects_tab(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer")])], false);
    let (vcx, _s, id) = pg_shell(cx, "en", cat);
    let rows = |vcx: &VisualTestContext| {
        vcx.run_until_parked();
        vcx.cx.read(hacivat_app::connections::explorer::shown_rows)
    };
    let key_of = |vcx: &VisualTestContext, label: &str| {
        rows(vcx)
            .into_iter()
            .rfind(|r| r.data.label.contains(label) && r.status.is_none())
            .unwrap_or_else(|| panic!("{label}"))
            .key
    };
    for l in ["app-dev", "app", "public", "TABLES", "users", "COLUMNS"] {
        let k = key_of(vcx, l);
        vcx.cx.update(|cx| {
            let tree = hacivat_app::connections::explorer::tree(cx).unwrap();
            tree.update(cx, |t, cx| t.set_expanded(k, true, cx));
        });
    }
    // On a column, View DDL opens its table's.
    let col = key_of(vcx, "id");
    vcx.cx.update(|cx| {
        let tree = hacivat_app::connections::explorer::tree(cx).unwrap();
        tree.update(cx, |t, cx| t.select(col, cx));
    });
    let tabs = tab_count(vcx);
    assert!(run(vcx, "explorer.view_ddl"), "View DDL runs on an object");
    assert_eq!(tab_count(vcx), tabs + 1);
    assert_eq!(tab_title(vcx), "users · DDL");
    assert!(state(vcx, id, &users_key()).text.is_some());
}

#[gpui::test]
fn object_table_shrinks_and_title_follows_rename(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer")])], false);
    let (vcx, _s, id) = pg_shell(cx, "en", cat);
    let before = vcx.cx.read(ddl_view::object_count);
    open_view(vcx, id, &users_key());
    assert_eq!(vcx.cx.read(ddl_view::object_count), before + 1);
    // Renamed on the server (same OID): the tab follows the new name.
    let renamed = catalog_gen(
        DbmsKind::Postgres,
        2,
        vec![schema_at(
            DbmsKind::Postgres,
            &["app", "public"],
            false,
            Level::Sources,
            vec![table("customers", Some(16_400), &[("id", "integer")])],
        )],
        vec![(public(), loaded(false))],
    );
    show(vcx, id, renamed);
    assert_eq!(tab_title(vcx), "customers · DDL");
    // Closing the tab drops its entry.
    assert!(run(vcx, "workspace.close_tab"));
    vcx.run_until_parked();
    assert_eq!(vcx.cx.read(ddl_view::object_count), before);
}

// ---- opus review (T25): hostile tests, H1-H4 flipped to assert the fixes (fix round) ----

/// H1. A background read publishes a changed `users` while the view's check is in flight; the
/// check (which compared the server with that newer catalog) then answers `Current`. The view
/// still shows the OLD text, so it must not read as current.
#[gpui::test]
fn review_h1_current_verdict_after_a_publish_never_vouches_the_old_text(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer")])], false);
    let (vcx, s, id) = pg_shell(cx, "en", cat);
    s.check.set(Some(Answer::Hold));
    open_view(vcx, id, &users_key());
    let old = shown_text(vcx, id, &users_key());
    assert_eq!(state(vcx, id, &users_key()).freshness, Freshness::Checking);
    show(
        vcx,
        id,
        pg_cat(
            2,
            vec![users(&[("id", "integer"), ("email", "text")])],
            false,
        ),
    );
    assert_eq!(state(vcx, id, &users_key()).freshness, Freshness::Changed);
    s.verdict.set(Some(Staleness::Current));
    fake_of(&s, id).answer("check users", 2);
    let st = state(vcx, id, &users_key());
    assert_eq!(st.text.as_deref(), Some(old.as_str()), "the old text stays");
    assert_eq!(
        st.freshness,
        Freshness::Changed,
        "the old text (no `email`) is not current: {st:?}"
    );
    assert!(st.notices.contains(&"hv-ddl-view-stale".to_string()));
}

/// H2. The catalog moves to a STALE namespace with a different script ("changed"); the user then
/// flips an option. The view shows the stale namespace's script and must not call it current
/// without a check.
#[gpui::test]
fn review_h2_option_change_after_a_stale_publish_is_not_current(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer")])], false);
    let (vcx, s, id) = pg_shell(cx, "en", cat);
    open_view(vcx, id, &users_key());
    assert_eq!(state(vcx, id, &users_key()).freshness, Freshness::Current);
    show(
        vcx,
        id,
        pg_cat(
            2,
            vec![users(&[("id", "integer"), ("email", "text")])],
            true,
        ),
    );
    assert_eq!(state(vcx, id, &users_key()).freshness, Freshness::Changed);
    let checks = s
        .calls
        .borrow()
        .iter()
        .filter(|c| *c == "check users")
        .count();
    click(vcx, "segmented-ddl-qualify-1");
    let st = state(vcx, id, &users_key());
    assert!(
        st.text.as_deref().unwrap().contains("email"),
        "{:?}",
        st.text
    );
    let checks_after = s
        .calls
        .borrow()
        .iter()
        .filter(|c| *c == "check users")
        .count();
    assert!(
        checks_after > checks,
        "the replaced text was checked ({checks} -> {checks_after})"
    );
    assert_eq!(
        st.freshness,
        Freshness::Unknown,
        "a stale namespace's text is not current: {st:?}"
    );
}

/// H3. MySQL: the generated text was checked Current; the user turns on "Definitions provided
/// by server". The shown text is now the `Sources` facet's, which no facet state vouches for,
/// so it must not read as current (T19 carry m6) until a check says so.
#[gpui::test]
fn review_h3_server_text_toggle_without_a_facet_state_is_not_current(cx: &mut TestAppContext) {
    let (vcx, s, id) = mysql_shell(cx, mysql_cat(vec![table("t", None, &[("id", "int")])]));
    s.server.borrow_mut().insert(
        "api.t".to_string(),
        Name::from("CREATE TABLE `t` (\n  `id` int DEFAULT NULL\n) ENGINE=InnoDB"),
    );
    let k = key(ObjectKind::Table, &["api", "t"]);
    open_view(vcx, id, &k);
    assert_eq!(state(vcx, id, &k).freshness, Freshness::Current);
    click(vcx, "checkbox-ddl-server");
    let st = state(vcx, id, &k);
    assert!(st.server_text);
    assert_eq!(
        st.freshness,
        Freshness::Unknown,
        "unvouched Sources-facet text is not current: {st:?}"
    );
    assert_eq!(
        s.calls.borrow().iter().filter(|c| *c == "check t").count(),
        2,
        "the server text was checked again"
    );
}

/// H4. MySQL server text vouched by a loaded `Sources` facet reads current; then a newer
/// catalog marks that facet stale (same structure, namespace not stale). The text is the
/// stale facet's: it must stop reading as current.
#[gpui::test]
fn review_h4_sources_facet_going_stale_turns_current_off(cx: &mut TestAppContext) {
    let k = key(ObjectKind::Table, &["api", "t"]);
    let with_facet = |generation: u64, stale: bool| {
        let mut cat = (*mysql_cat(vec![table("t", None, &[("id", "int")])])).clone();
        cat.generation = generation;
        let mut states = (*cat.states).clone();
        states.insert(
            hacivat_meta::snapshot::ContainerKey::Facet {
                ns: NsKey::schema(["api"]),
                object: k.clone(),
                facet: FacetKind::Sources,
            },
            loaded(stale),
        );
        cat.states = Arc::new(states);
        Arc::new(cat)
    };
    let (vcx, s, id) = mysql_shell(cx, with_facet(1, false));
    s.server.borrow_mut().insert(
        "api.t".to_string(),
        Name::from("CREATE TABLE `t` (\n  `id` int DEFAULT NULL\n) ENGINE=InnoDB"),
    );
    open_view(vcx, id, &k);
    click(vcx, "checkbox-ddl-server");
    click(vcx, "button-ddl-refresh");
    let st = state(vcx, id, &k);
    assert!(st.server_text);
    assert_eq!(
        st.freshness,
        Freshness::Current,
        "vouched by the loaded facet"
    );
    show(vcx, id, with_facet(2, true));
    let st = state(vcx, id, &k);
    assert_eq!(
        st.freshness,
        Freshness::Unknown,
        "server text of a stale Sources facet is not current: {st:?}"
    );
}

/// H5. The DDL text carries the `Editor` context. Run must stay inert there: nothing executes
/// and no console opens. Fix round (review m-6): Run is disabled in a read-only editor, so the
/// chord no longer even reaches the text (no `ExecuteRequested`), and Run is off the palette.
#[gpui::test]
fn review_h5_run_chord_in_the_ddl_text_executes_nothing(cx: &mut TestAppContext) {
    let cat = pg_cat(1, vec![users(&[("id", "integer")])], false);
    let (vcx, _s, id) = pg_shell(cx, "en", cat);
    connect(vcx, "app-dev");
    wait_until(vcx, "connected", |vcx| {
        matches!(
            live(vcx, "app-dev"),
            hacivat_session::LiveState::Connected(_)
        )
    });
    open_view(vcx, id, &users_key());
    let editor = vcx.cx.read(|cx| {
        ddl_view::view_of(id, &users_key(), cx)
            .unwrap()
            .read(cx)
            .editor()
            .clone()
    });
    let requests = Rc::new(Cell::new(0usize));
    let r2 = requests.clone();
    vcx.cx.update(|cx| {
        cx.subscribe(&editor, move |_, e: &hacivat_ui_editor::EditorEvent, _| {
            if matches!(e, hacivat_ui_editor::EditorEvent::ExecuteRequested(_)) {
                r2.set(r2.get() + 1);
            }
        })
        .detach()
    });
    in_window(vcx, |window, cx| {
        let focus = gpui::Focusable::focus_handle(editor.read(cx), cx);
        window.focus(&focus, cx);
    });
    vcx.run_until_parked();
    let tabs = tab_count(vcx);
    let before = fake(vcx).typed_log().len();
    let chord = if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    };
    vcx.simulate_keystrokes(chord);
    vcx.run_until_parked();
    assert_eq!(
        requests.get(),
        0,
        "Run is disabled in the read-only DDL text"
    );
    assert!(!run(vcx, "editor.run"), "Run is not offered there");
    assert_eq!(tab_count(vcx), tabs, "no console opened");
    let log = fake(vcx).typed_log();
    assert!(
        !log[before..].iter().any(|c| matches!(
            c,
            hacivat_driver::fake::FakeTypedCall::Query(q) if q.contains("CREATE")
        )),
        "the DDL ran: {:?}",
        &log[before..]
    );
}

/// H6. A hostile table name (RLO, a quote, a statement and a line comment, a newline): the
/// text is exactly T19's bytes with the name quoted (the quote doubled, nothing escapes the
/// identifier), the tab title isolates it (FSI…PDI), and the raw editor text matches.
#[gpui::test]
fn review_h6_hostile_identifier_stays_one_quoted_identifier(cx: &mut TestAppContext) {
    let evil = "us\u{202E}ers\"; DROP TABLE x; --";
    let cat = pg_cat(
        1,
        vec![table(evil, Some(16_500), &[("id", "integer")])],
        false,
    );
    let (vcx, s, id) = pg_shell(cx, "en", cat.clone());
    let k = ObjectKey {
        kind: ObjectKind::Table,
        path: ["app", "public", evil]
            .into_iter()
            .map(Name::from)
            .collect(),
        server_id: Some(hacivat_meta::ServerId::Oid(16_500)),
        overload_sig: None,
        raw_kind: None,
    };
    open_view(vcx, id, &k);
    let want = create_script_text(
        &cat,
        &FakeSources(s.clone()),
        &k,
        DialectId::Postgres,
        GenOptions::default(),
    );
    let st = state(vcx, id, &k);
    assert_eq!(st.text.as_deref(), Some(want.as_str()));
    assert!(
        want.contains("public.\"us\u{202E}ers\"\"; DROP TABLE x; --\" ("),
        "{want:?}"
    );
    assert_eq!(editor_text_of(vcx, id, &k), want);
    let raw_title = {
        let ws = workspace(vcx);
        vcx.cx.read(|cx| {
            let w = ws.read(cx);
            w.active_tab().map(|t| w.tab_title(t, cx).to_string())
        })
    }
    .expect("a tab");
    assert!(
        raw_title.starts_with('\u{2068}') && raw_title.contains("\u{2069} · DDL"),
        "{raw_title:?}"
    );
}

/// Review m-1: a name with control characters (a newline, a tab) is drawn on the single-line
/// chrome (the tab title, the toolbar) with the controls visible, never breaking the line (it
/// panicked the tab strip's line shaping in debug builds). The DDL text keeps the name exactly.
#[gpui::test]
fn control_characters_in_a_name_are_visible_in_the_tab_title(cx: &mut TestAppContext) {
    let evil = "a\nb\tc";
    let cat = pg_cat(
        1,
        vec![table(evil, Some(16_600), &[("id", "integer")])],
        false,
    );
    let (vcx, _s, id) = pg_shell(cx, "en", cat);
    let k = ObjectKey {
        kind: ObjectKind::Table,
        path: ["app", "public", evil]
            .into_iter()
            .map(Name::from)
            .collect(),
        server_id: Some(hacivat_meta::ServerId::Oid(16_600)),
        overload_sig: None,
        raw_kind: None,
    };
    open_view(vcx, id, &k);
    // Drawn (the tab strip and the toolbar shaped the title and the crumb).
    vcx.run_until_parked();
    assert_eq!(tab_title(vcx), "a\u{240A}b\u{2409}c · DDL");
    assert_eq!(
        ddl_view::visible_name("x\u{7F}\u{85}y"),
        "x\u{2421}\u{FFFD}y"
    );
    let text = shown_text(vcx, id, &k);
    assert!(text.contains("\"a\nb\tc\""), "{text:?}");
}

// ## m4-t32-shots (fix round 1, review M-5): a name's own bidi controls (a PDI that would close
// the title's isolate early, an RLO that would then reverse " · DDL") are drawn as visible
// marks on single-line chrome: the title keeps exactly one isolate, Fluent's, around the name.
#[gpui::test]
fn bidi_controls_in_a_name_cannot_escape_the_tab_titles_isolate(cx: &mut TestAppContext) {
    let evil = "x\u{2069}\u{202E}y";
    let cat = pg_cat(
        1,
        vec![table(evil, Some(16_610), &[("id", "integer")])],
        false,
    );
    let (vcx, _s, id) = pg_shell(cx, "en", cat);
    let k = ObjectKey {
        kind: ObjectKind::Table,
        path: ["app", "public", evil]
            .into_iter()
            .map(Name::from)
            .collect(),
        server_id: Some(hacivat_meta::ServerId::Oid(16_610)),
        overload_sig: None,
        raw_kind: None,
    };
    open_view(vcx, id, &k);
    vcx.run_until_parked();
    let raw_title = {
        let ws = workspace(vcx);
        vcx.cx.read(|cx| {
            let w = ws.read(cx);
            w.active_tab().map(|t| w.tab_title(t, cx).to_string())
        })
    }
    .expect("a tab");
    assert_eq!(raw_title, "\u{2068}x\u{FFFD}\u{FFFD}y\u{2069} · DDL");
    for c in [
        '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}',
        '\u{2068}', '\u{2069}', '\u{200E}', '\u{200F}', '\u{061C}',
    ] {
        assert_eq!(
            ddl_view::visible_name(&format!("a{c}b")),
            "a\u{FFFD}b",
            "U+{:04X}",
            c as u32
        );
    }
    // The DDL text keeps the name exactly.
    assert!(shown_text(vcx, id, &k).contains(evil));
}

// ## m4-t27-refresh-ui (named exception: the T15b hook in `ddl_view/source.rs`, T25 m-7)

/// `app.public` published through a real `Catalog` with T15b's side tables: `v_text` (a view
/// whose props name its text), `v_facet` (a view whose text is only its `Sources` facet, loaded
/// or `stale_facet`), `users` with a loaded (or `stale_facet`) `Grants` facet.
fn published_with_sources(stale_facet: bool) -> Arc<CatalogSnapshot> {
    let base = pg_cat(1, Vec::new(), false);
    let catalog = hacivat_meta::Catalog::new(base.server.clone());
    let publisher = catalog.publisher().expect("the publisher");
    let text = "SELECT 1 AS one";
    let ns = schema_at(
        DbmsKind::Postgres,
        &["app", "public"],
        false,
        Level::Sources,
        vec![
            view_obj("v_text", text),
            Subtree::new(
                Obj::new(ObjectKind::View, "v_facet")
                    .with_props(Props::View(Arc::new(ViewProps::default()))),
            ),
            users(&[("id", "integer")]),
        ],
    );
    let facet = |object: ObjectKey, facet: FacetKind| hacivat_meta::snapshot::ContainerKey::Facet {
        ns: public(),
        object,
        facet,
    };
    let grants = GrantsFacet {
        entries: vec![Grant {
            grantee: Name::from("reporting"),
            privilege: Name::from("SELECT"),
            columns: Vec::new(),
            grantable: false,
        }],
    };
    publisher
        .publish(vec![
            hacivat_meta::snapshot::CatalogChange::Put(ns),
            hacivat_meta::snapshot::CatalogChange::SetState(
                hacivat_meta::snapshot::ContainerKey::Namespace(public()),
                loaded(false),
            ),
            hacivat_meta::snapshot::CatalogChange::PutSources {
                ns: public(),
                texts: vec![(SourceRef::of(text), Name::from(text))],
            },
            hacivat_meta::snapshot::CatalogChange::SetFacet {
                key: facet(
                    key(ObjectKind::View, &["app", "public", "v_facet"]),
                    FacetKind::Sources,
                ),
                state: loaded(stale_facet),
                value: Some(hacivat_meta::snapshot::FacetValue::Sources(Name::from(
                    "SELECT 2 AS two",
                ))),
            },
            hacivat_meta::snapshot::CatalogChange::SetFacet {
                key: facet(users_key(), FacetKind::Grants),
                state: loaded(stale_facet),
                value: Some(hacivat_meta::snapshot::FacetValue::Grants(Arc::new(grants))),
            },
        ])
        .expect("published");
    catalog.current()
}

/// T25 m-7 with the T15b hook: the app's own `EngineCatalog` (no fake provider) reads texts
/// and facets from the catalog snapshot on every DDL view path: a view's text named by its
/// props, a view's text from its `Sources` facet, a table's grants from its `Grants` facet; a
/// stale facet vouches for nothing (the definition is not loaded, the grants say so).
#[gpui::test]
fn engine_catalog_sources_and_facets_vouch_on_every_path(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    in_window(vcx, |window, _| window.activate_window());
    let id = add_profile(vcx, pg_profile("app-dev"));
    show(vcx, id, published_with_sources(false));
    let v_text = key(ObjectKind::View, &["app", "public", "v_text"]);
    let v_facet = key(ObjectKind::View, &["app", "public", "v_facet"]);
    open_view(vcx, id, &v_text);
    let text = shown_text(vcx, id, &v_text);
    assert!(text.contains("SELECT 1 AS one"), "{text}");
    open_view(vcx, id, &v_facet);
    let text = shown_text(vcx, id, &v_facet);
    assert!(text.contains("SELECT 2 AS two"), "{text}");
    open_view(vcx, id, &users_key());
    let st = state(vcx, id, &users_key());
    assert_eq!(st.grants, GrantsUx::Loaded, "{st:?}");
    assert!(
        st.text.as_deref().is_some_and(|t| t.contains("reporting")),
        "{st:?}"
    );

    // Stale facets vouch for nothing.
    let vcx2 = vcx;
    show(vcx2, id, published_with_sources(true));
    let st = state(vcx2, id, &v_facet);
    assert_eq!(st.body, Body::NotLoaded, "{st:?}");
    let st = state(vcx2, id, &users_key());
    assert_ne!(st.grants, GrantsUx::Loaded, "{st:?}");
    assert!(
        !st.text.as_deref().is_some_and(|t| t.contains("reporting")),
        "{st:?}"
    );
}
