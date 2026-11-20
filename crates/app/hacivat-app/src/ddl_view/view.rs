//! The DDL view (design/m4-explorer surfaces 10 and 11): the toolbar, the options strip, the
//! notices and a read-only editor holding exactly the object's script.
//!
//! ## m4-t25-ddl-view
//!
//! **The text** is Task 19's `create_script_with` for the object, the options and the
//! profile's current schema ([`super::model::generate`]): never localized, never edited. The
//! editor is read-only (`EditorConfig::read_only`: every edit command, typing, paste and the
//! input method refuse); its text changes only when the view puts a new script in it.
//!
//! **Loading.** An object of a kind whose definition is the server's text (R-M4-24) is read at
//! `Sources` first when its namespace holds less (one fragment, `elevate`); a definition still
//! missing is asked for once as the `Sources` facet; the grants facet is asked for once while
//! Grants is on. Each wait shows "Reading the definition of …"; what stays missing says so
//! (`DdlError::NotLoaded`: never a text that is not the object's, F31).
//!
//! **Freshness** (surface 11). When a script is first shown the view asks `check_state`. Its
//! ticket answers only a generation (Task 22 keeps the per-key verdict inside the engine), so
//! the view decides from the catalog: the object is gone from a loaded namespace → "no longer
//! exists" with Close tab; its namespace was published again since the check began and the
//! script differs from the one shown → "changed on the server" with Reload; the check failed,
//! or the namespace is stale (cached, refresh after DDL with Ask/Never, a held deletion) →
//! "couldn't check" with Reload; else current. While the tab is open, a newer catalog whose
//! script differs marks it changed the same way (never an automatic reload, surface 11 "Out of
//! scope"). Server text is held to the same rule (T19 review carry m6: `DdlSources` has no
//! load state of its own). Gap (task report): a check answered `Unknown` on a namespace that is
//! not stale reads as current until the ticket carries the verdict.

use std::rc::Rc;
use std::sync::Arc;

use hacivat_ddl::produce::{GRANTS_PARTIAL, WARN_EXECUTABLE_COMMENT};
use hacivat_ddl::script::{Notice, Script};
use hacivat_l10n::{ArgValue, NoArgs, Text, ids};
use hacivat_meta::kind::{DbLevel, database_level};
use hacivat_meta::props::FacetKind;
use hacivat_meta::snapshot::LoadState;
use hacivat_meta::userdata::{DdlViewOptions, Placement, Qualify};
use hacivat_meta::{CatalogSnapshot, Level, Name, NamespaceSnapshot, ObjectKey};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_sql_editor::SqlLanguageService;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::env_swatch_on_raised;
use hacivat_ui::checkbox::Checkbox;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented};
use hacivat_ui::select::{Select, SelectEvent, SelectOption};
use hacivat_ui::step_trail::run_marker;
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui::toolbar::{ItemFn, Toolbar, ToolbarItem};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, WeakEntity,
    Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use hacivat_ui_editor::{DocId, EditorConfig, EditorView, LanguageService};
use sql_dialect::{ClientFlavor, DialectCtx};

use super::model::{self, Generated};
use super::source::{self, DdlCatalog};
use crate::connections::ConnectionsModel;
use crate::console::Consoles;
use crate::explorer::actions::{ActionSurface, NodeRef, provided_actions};
use crate::explorer::{adapter, live, userdata};
use crate::l10n_global::tr;
use crate::shell::{DdlObjectId, TabKind, Workspace};

/// What the code area holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Body {
    /// Reading the definition (elevation, a facet).
    Loading,
    /// The script.
    Script,
    /// No DDL for this kind (`hv-ddl-view-unsupported`).
    Unsupported,
    /// The definition could not be read (`hv-ddl-view-not-loaded`).
    NotLoaded,
}

/// What the view knows of the object's freshness (surface 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Not checked yet, or the check runs.
    Checking,
    Current,
    /// Changed on the server since the shown text was loaded.
    Changed,
    /// No longer exists on the server.
    Gone,
    /// The check could not decide ("couldn't check").
    Unknown,
}

/// The grants facet as the view shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantsUx {
    /// Grants are off, or the engine has none (SQLite).
    Off,
    Loading,
    Loaded,
    /// Not loaded (the script says so: `ddl-grants-not-loaded`).
    NotLoaded,
    /// The login could not read every grant.
    Partial,
}

/// What a DDL view shows (tests, shots).
#[derive(Debug, Clone, PartialEq)]
pub struct DdlViewState {
    pub body: Body,
    /// The shown text, exactly as Copy and Open in console use it.
    pub text: Option<String>,
    /// The Fluent ids of every notice shown, in order.
    pub notices: Vec<String>,
    pub freshness: Freshness,
    pub options: DdlViewOptions,
    /// The strip's controls shown: `qualification`, `constraints`, `server`, `grants`, `owner`.
    pub controls: Vec<&'static str>,
    /// The shown text is the server's (the source caption).
    pub server_text: bool,
    pub grants: GrantsUx,
    /// The provided toolbar actions (`ActionSurface::DdlViewToolbar`), by command id.
    pub toolbar_actions: Vec<&'static str>,
}

/// Why the view rebuilds its script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Why {
    /// Open, an option, a facet or an elevation answered, Reload: the new script is shown.
    Show,
    /// The catalog moved on its own: a different script is announced, not shown.
    Catalog,
}

/// The shown script.
#[derive(Clone)]
struct Shown {
    text: String,
    notices: Vec<Notice>,
    server_text: bool,
}

/// The DDL view of one object.
pub struct DdlView {
    pub profile: ProfileId,
    pub object: DdlObjectId,
    key: ObjectKey,
    dbms: DbmsKind,
    catalog: Option<Rc<dyn DdlCatalog>>,
    opts: DdlViewOptions,
    editor: Entity<EditorView>,
    constraints: Entity<Select>,
    body: Body,
    shown: Option<Shown>,
    freshness: Freshness,
    /// Requests made once per view (a Refresh asks again).
    elevated: bool,
    sources_asked: bool,
    grants_asked: bool,
    grants_pending: bool,
    /// The snapshot the view last looked at (`Arc` identity), so an unrelated global change
    /// rebuilds nothing.
    seen: Option<Arc<CatalogSnapshot>>,
    _subscriptions: Vec<Subscription>,
}

/// The `DocId` space of DDL views (consoles number theirs from 1 by tab).
const DOC_BASE: u64 = 1 << 48;

impl DdlView {
    pub fn new(
        profile: ProfileId,
        object: DdlObjectId,
        key: ObjectKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let dbms = cx
            .try_global::<ConnectionsModel>()
            .and_then(|m| m.get(profile))
            .map_or(DbmsKind::Postgres, |p| p.engine);
        let doc = DocId(DOC_BASE + object.0);
        let service = SqlLanguageService::new();
        service.set_sql_context(
            doc,
            DialectCtx::new(live::dialect_of(dbms), None),
            ClientFlavor::None,
            cx,
        );
        let service: Rc<dyn LanguageService> = service;
        let title: TextFn = Rc::new(move |cx: &App| super::tab_title(profile, object, cx));
        let cfg = EditorConfig {
            read_only: true,
            copy_line_without_selection: false,
            ..EditorConfig::default()
        };
        let editor = cx.new(|cx| EditorView::new(doc, "", cfg, service, title, window, cx));
        userdata::ensure(profile, cx);
        let opts = userdata::get(profile, cx).map_or_else(DdlViewOptions::default, |d| d.ddl_view);
        let constraints =
            cx.new(|cx| Select::new(placement_options(), placement_index(opts.placement), cx));
        constraints.update(cx, |s, _| {
            s.set_label(text_fn_of(ids::hv::DDL_VIEW_CONSTRAINTS));
        });
        let subscriptions = vec![
            cx.observe_global::<adapter::ExplorerCatalogs>(|v, cx| v.on_catalog(cx)),
            cx.observe_global::<userdata::UserData>(|v, cx| v.on_user_data(cx)),
            cx.subscribe(&constraints, |v, _, e: &SelectEvent, cx| {
                let SelectEvent::Changed(i) = *e;
                let placement = [
                    Placement::InsideColumn,
                    Placement::InsideTable,
                    Placement::AfterTable,
                ][i.min(2)];
                v.set_options(
                    DdlViewOptions {
                        placement,
                        ..v.opts
                    },
                    cx,
                );
            }),
            cx.on_release(move |_, cx| super::forget(object, cx)),
        ];
        let mut view = DdlView {
            profile,
            object,
            key,
            dbms,
            catalog: source::catalog_for(profile, cx),
            opts,
            editor,
            constraints,
            body: Body::Loading,
            shown: None,
            freshness: Freshness::Checking,
            elevated: false,
            sources_asked: false,
            grants_asked: false,
            grants_pending: false,
            seen: None,
            _subscriptions: subscriptions,
        };
        view.load(Why::Show, cx);
        view
    }

    /// The read-only editor that holds the text.
    pub fn editor(&self) -> &Entity<EditorView> {
        &self.editor
    }

    /// The object shown (its key follows a rename).
    pub fn key(&self) -> &ObjectKey {
        &self.key
    }

    fn snapshot(&self, cx: &App) -> Option<Arc<CatalogSnapshot>> {
        self.catalog.as_ref()?.snapshot(cx)
    }

    /// The user's current schema of the profile (`Qualify::Auto`): the schema (PostgreSQL) or
    /// database (MySQL/MariaDB) of its focused console's session, else none (every name is
    /// qualified).
    fn current_schema(&self, cx: &App) -> Option<Name> {
        let session = Consoles::focused(cx)
            .and_then(|c| Consoles::get(cx, c))
            .filter(|s| s.profile == Some(self.profile))
            .and_then(|s| s.session.clone())
            .or_else(|| {
                Consoles::ids(cx)
                    .into_iter()
                    .filter_map(|c| Consoles::get(cx, c))
                    .filter(|s| s.profile == Some(self.profile))
                    .find_map(|s| s.session.clone())
            })?;
        let state = session.state();
        let schema = state.borrow().schema.clone()?;
        let name = match self.dbms {
            DbmsKind::MySql | DbmsKind::MariaDb => schema.database,
            _ => schema.schema,
        };
        name.filter(|n| !n.is_empty())
            .map(|n| Name::from(n.as_str()))
    }

    fn generate(&self, snap: &Arc<CatalogSnapshot>, cx: &App) -> Generated {
        let Some(catalog) = &self.catalog else {
            return Generated::NotLoaded;
        };
        let sources = catalog.sources(cx);
        model::generate(
            self.dbms,
            snap,
            &*sources,
            &self.key,
            self.opts,
            self.current_schema(cx),
        )
    }

    /// An object with a server id renamed on the server keeps its handle; the key (and the tab's
    /// title) follow the new name.
    fn follow_rename(&mut self, snap: &CatalogSnapshot, cx: &mut Context<Self>) {
        if self.key.server_id.is_none() {
            return;
        }
        let Some(now) = snap.find(&self.key).and_then(|h| h.key()) else {
            return;
        };
        if now.path != self.key.path {
            self.key = now.clone();
            super::rekey(self.object, now, cx);
            cx.refresh_windows();
        }
    }

    /// Rebuilds the script from the newest snapshot (`why`), asking for what is missing.
    fn load(&mut self, why: Why, cx: &mut Context<Self>) {
        cx.notify();
        let Some(snap) = self.snapshot(cx) else {
            self.body = Body::NotLoaded;
            return;
        };
        self.seen = Some(snap.clone());
        self.follow_rename(&snap, cx);
        if snap.find(&self.key).is_none() {
            if model::is_gone(&snap, &self.key) {
                self.freshness = Freshness::Gone;
                if self.shown.is_none() {
                    self.body = Body::NotLoaded;
                }
            } else if !self.elevated && self.shown.is_none() {
                self.elevate(cx);
            } else if self.shown.is_none() {
                self.body = Body::NotLoaded;
            }
            return;
        }
        // A definition kept as the server's text is read at Sources first (R-M4-24).
        if model::needs_sources(self.key.kind)
            && !self.elevated
            && model::loaded_level(&snap, &self.key).is_none_or(|l| l < Level::Sources)
        {
            self.elevate(cx);
            return;
        }
        match self.generate(&snap, cx) {
            Generated::Script(s) => self.got_script(s, why, &snap, cx),
            Generated::Unsupported => {
                self.body = Body::Unsupported;
                self.shown = None;
                self.set_editor_text("", cx);
            }
            // A part of the definition is missing (a trigger's text in a table read at
            // Structure): read the object at Sources, then ask for the Sources facet.
            Generated::NotLoaded
                if !self.elevated
                    && model::loaded_level(&snap, &self.key).is_none_or(|l| l < Level::Sources) =>
            {
                self.elevate(cx);
            }
            Generated::NotLoaded if !self.sources_asked => {
                self.sources_asked = true;
                self.ask(|c, key, cx| c.load_facet(key, FacetKind::Sources, cx), cx);
            }
            Generated::NotLoaded | Generated::Failed => {
                if why == Why::Show || self.shown.is_none() {
                    self.body = Body::NotLoaded;
                    self.shown = None;
                    self.set_editor_text("", cx);
                }
            }
        }
    }

    fn got_script(
        &mut self,
        s: Script,
        why: Why,
        snap: &Arc<CatalogSnapshot>,
        cx: &mut Context<Self>,
    ) {
        let next = Shown {
            text: model::text_of(&s),
            notices: s.notices,
            server_text: model::is_server_text(self.dbms, self.key.kind, self.opts),
        };
        let first = self.shown.is_none();
        match (why, &self.shown) {
            (Why::Catalog, Some(shown)) if self.body == Body::Script => {
                if shown.text != next.text {
                    self.freshness = Freshness::Changed;
                } else if model::is_stale(snap, &self.key) && self.freshness == Freshness::Current {
                    self.freshness = Freshness::Unknown;
                }
            }
            _ => {
                if self.freshness == Freshness::Changed {
                    // The shown text is now the newest the catalog holds.
                    self.freshness = Freshness::Current;
                }
                self.set_editor_text(&next.text, cx);
                self.shown = Some(next);
                self.body = Body::Script;
            }
        }
        self.ask_grants(cx);
        if first && self.shown.is_some() {
            self.check(cx);
        }
    }

    fn set_editor_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let same = self.editor.read(cx).snapshot().buffer.to_string() == text;
        if !same {
            self.editor.update(cx, |e, cx| e.set_text(text, cx));
        }
    }

    /// Makes request `f` and runs `load(Show)` when it answers (whatever it answers).
    fn ask(
        &mut self,
        f: impl FnOnce(&dyn DdlCatalog, ObjectKey, &App) -> hacivat_introspect::RefreshTicket,
        cx: &mut Context<Self>,
    ) {
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        if self.shown.is_none() {
            self.body = Body::Loading;
        }
        let ticket = f(&*catalog, self.key.clone(), cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let _ = ticket.await;
            let _ = this.update(cx, |v, cx| v.load(Why::Show, cx));
        })
        .detach();
    }

    fn elevate(&mut self, cx: &mut Context<Self>) {
        self.elevated = true;
        self.ask(|c, key, cx| c.elevate(key, Level::Sources, cx), cx);
    }

    /// Asks for the grants facet once while Grants is on and the engine has grants.
    fn ask_grants(&mut self, cx: &mut Context<Self>) {
        if !self.grants_wanted() || self.grants_asked {
            return;
        }
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        if catalog.sources(cx).grants(&self.key).state != LoadState::NotLoaded {
            return;
        }
        self.grants_asked = true;
        self.grants_pending = true;
        let ticket = catalog.load_facet(self.key.clone(), FacetKind::Grants, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let _ = ticket.await;
            let _ = this.update(cx, |v, cx| {
                v.grants_pending = false;
                v.load(Why::Show, cx);
            });
        })
        .detach();
    }

    fn grants_wanted(&self) -> bool {
        model::shown(self.dbms).grants && self.opts.grants
    }

    fn server_text_shown(&self) -> bool {
        model::is_server_text(self.dbms, self.key.kind, self.opts)
    }

    /// Checks the object against the server (surface 11).
    fn check(&mut self, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog.clone() else {
            self.freshness = Freshness::Unknown;
            return;
        };
        self.freshness = Freshness::Checking;
        let base = self.arena(cx);
        let ticket = catalog.check_state(self.key.clone(), cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let answer = ticket.await;
            let _ = this.update(cx, |v, cx| v.checked(answer.is_ok(), base, cx));
        })
        .detach();
    }

    /// The arena the object lives in now.
    fn arena(&self, cx: &App) -> Option<Arc<NamespaceSnapshot>> {
        let snap = self.snapshot(cx)?;
        snap.find(&self.key).map(|h| h.ns)
    }

    /// The verdict once a check (or a re-read) answered.
    fn checked(&mut self, ok: bool, base: Option<Arc<NamespaceSnapshot>>, cx: &mut Context<Self>) {
        cx.notify();
        let Some(snap) = self.snapshot(cx) else {
            self.freshness = Freshness::Unknown;
            return;
        };
        if model::is_gone(&snap, &self.key) {
            self.freshness = Freshness::Gone;
            return;
        }
        let Some(now) = snap.find(&self.key).map(|h| h.ns) else {
            self.freshness = Freshness::Unknown;
            return;
        };
        if !ok || model::is_stale(&snap, &self.key) {
            self.freshness = Freshness::Unknown;
            return;
        }
        let republished = base.as_ref().is_none_or(|b| !Arc::ptr_eq(b, &now));
        if republished
            && let Generated::Script(s) = self.generate(&snap, cx)
            && self
                .shown
                .as_ref()
                .is_some_and(|shown| shown.text != model::text_of(&s))
        {
            self.freshness = Freshness::Changed;
            return;
        }
        // Gap (module docs): an `Unknown` answer on a namespace that is not stale lands here.
        self.freshness = Freshness::Current;
    }

    fn on_catalog(&mut self, cx: &mut Context<Self>) {
        let snap = self.snapshot(cx);
        let same = match (&snap, &self.seen) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        if let Some(snap) = &snap
            && self.body == Body::Script
            && model::is_gone(snap, &self.key)
        {
            self.seen = Some(snap.clone());
            self.freshness = Freshness::Gone;
            cx.notify();
            return;
        }
        if self.freshness == Freshness::Gone {
            return;
        }
        self.load(Why::Catalog, cx);
    }

    fn on_user_data(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = userdata::get(self.profile, cx)
            && d.ddl_view != self.opts
        {
            self.opts = d.ddl_view;
            self.constraints.update(cx, |s, cx| {
                s.set_selected(placement_index(self.opts.placement), cx)
            });
            self.load(Why::Show, cx);
        }
    }

    /// Changes the options: re-renders and remembers them for the profile.
    pub fn set_options(&mut self, opts: DdlViewOptions, cx: &mut Context<Self>) {
        if opts == self.opts {
            return;
        }
        self.opts = opts;
        // A failed write keeps the choice for this view (the file is the explorer's to report).
        userdata::update(self.profile, move |d| d.ddl_view = opts, cx).detach();
        self.load(Why::Show, cx);
    }

    /// Re-reads the object from the server and shows what it reads (Refresh, Reload).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        self.freshness = Freshness::Checking;
        self.elevated = false;
        self.sources_asked = false;
        self.grants_asked = false;
        let ticket = catalog.refresh(self.key.clone(), cx);
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let answer = ticket.await;
            let _ = this.update(cx, |v, cx| {
                v.load(Why::Show, cx);
                // The shown text is now what the catalog holds: current unless the re-read
                // failed, the namespace is stale or the object is gone.
                v.checked(answer.is_ok(), None, cx);
            });
        })
        .detach();
    }

    /// Whether there is a text to copy or open.
    pub fn can_copy(&self) -> bool {
        self.body == Body::Script && self.shown.is_some()
    }

    fn name(&self) -> String {
        self.key.name().to_string()
    }

    /// Puts the shown text on the clipboard (raw, `\n`), with a toast.
    pub fn copy(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(shown) = self.shown.as_ref().filter(|_| self.body == Body::Script) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(shown.text.clone()));
        let name = self.name();
        let label: TextFn = Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::DDL_VIEW_COPIED,
                ids::hv::HvDdlViewCopied {
                    name: Text(name.clone()),
                },
            )
        });
        ToastQueue::push(cx, Toast::new(ToastKind::Info, label));
    }

    /// Opens a console on the profile with the shown text inserted, not run.
    pub fn open_in_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self
            .shown
            .as_ref()
            .filter(|_| self.body == Body::Script)
            .map(|s| s.text.clone())
        else {
            return;
        };
        // A database-level engine's console runs on the profile's database: the text goes with
        // the object's (`open_with_text` refuses another, with a notice).
        let database = (database_level(self.dbms) == DbLevel::Database)
            .then(|| self.key.path.first().map(|d| d.to_string()))
            .flatten();
        let profile = self.profile;
        cx.defer_in(window, move |_, window, cx| {
            crate::console::open_with_text(profile, database.as_deref(), &text, false, window, cx);
        });
    }

    /// Closes this view's tab (the "no longer exists" notice).
    fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws) = window.root::<Workspace>().flatten() else {
            return;
        };
        let (profile, object) = (self.profile, self.object);
        cx.defer_in(window, move |_, _, cx| {
            ws.update(cx, |w, cx| {
                let tab = w
                    .layout()
                    .0
                    .panes
                    .values()
                    .flat_map(|p| p.tabs.clone())
                    .find(|t| w.tab_kind(*t) == Some(TabKind::DdlView { profile, object }));
                if let Some(tab) = tab {
                    w.force_close_tab(tab, cx);
                }
            });
        });
    }

    fn controls(&self) -> Vec<&'static str> {
        let s = model::shown(self.dbms);
        let server_text = self.server_text_shown();
        let mut out = Vec::new();
        if !server_text {
            out.push("qualification");
            out.push("constraints");
        }
        if s.server {
            out.push("server");
        }
        if s.grants {
            out.push("grants");
        }
        if s.owner {
            out.push("owner");
        }
        out
    }

    fn grants_ux(&self, cx: &App) -> GrantsUx {
        if !self.grants_wanted() {
            return GrantsUx::Off;
        }
        if self.grants_pending {
            return GrantsUx::Loading;
        }
        let Some(catalog) = &self.catalog else {
            return GrantsUx::NotLoaded;
        };
        match catalog.sources(cx).grants(&self.key).state {
            LoadState::Loaded { .. } => GrantsUx::Loaded,
            LoadState::Partial { .. } => GrantsUx::Partial,
            _ => GrantsUx::NotLoaded,
        }
    }

    fn provided(&self, cx: &App) -> Vec<crate::explorer::ProvidedAction> {
        let node = NodeRef {
            profile: self.profile,
            key: adapter::object_tree_key(self.profile, &self.key),
            object: Some(self.key.clone()),
            kind: Some(self.key.kind),
        };
        provided_actions(&node, ActionSurface::DdlViewToolbar, cx)
    }

    /// The notices shown, as `(Fluent id, its text)`, in order: freshness, then the body's, then
    /// the script's.
    fn notice_ids(&self) -> Vec<String> {
        let mut out = Vec::new();
        match self.freshness {
            Freshness::Changed => out.push("hv-ddl-view-stale".to_string()),
            Freshness::Gone => out.push("hv-ddl-view-gone".to_string()),
            Freshness::Unknown => out.push("hv-ddl-view-unknown".to_string()),
            Freshness::Checking | Freshness::Current => {}
        }
        match self.body {
            Body::Unsupported => out.push("hv-ddl-view-unsupported".to_string()),
            Body::NotLoaded => out.push("hv-ddl-view-not-loaded".to_string()),
            Body::Loading | Body::Script => {}
        }
        if self.body == Body::Script
            && let Some(shown) = &self.shown
        {
            for n in &shown.notices {
                out.push(notice_id(n.id).to_string());
            }
        }
        out
    }

    /// What the view shows (tests, shots).
    pub fn state(&self, cx: &App) -> DdlViewState {
        DdlViewState {
            body: self.body,
            text: self
                .shown
                .as_ref()
                .filter(|_| self.body == Body::Script)
                .map(|s| s.text.clone()),
            notices: self.notice_ids(),
            freshness: self.freshness,
            options: self.opts,
            controls: self.controls(),
            server_text: self.body == Body::Script
                && self.shown.as_ref().is_some_and(|s| s.server_text),
            grants: self.grants_ux(cx),
            toolbar_actions: self.provided(cx).iter().map(|a| a.id).collect(),
        }
    }
}

/// A script notice's Fluent id in the view (the partial-grants one is the view's sentence).
fn notice_id(id: &'static str) -> &'static str {
    if id == GRANTS_PARTIAL {
        "hv-ddl-view-grants-partial"
    } else {
        id
    }
}

fn text_fn_of(msg: hacivat_l10n::MsgRef<NoArgs>) -> TextFn {
    crate::l10n_global::text_fn(msg)
}

fn placement_index(p: Placement) -> usize {
    match p {
        Placement::InsideColumn => 0,
        Placement::InsideTable => 1,
        Placement::AfterTable => 2,
    }
}

fn placement_options() -> Vec<SelectOption> {
    [
        ids::hv::DDL_VIEW_CONSTRAINTS_INSIDE_COLUMN,
        ids::hv::DDL_VIEW_CONSTRAINTS_INSIDE_TABLE,
        ids::hv::DDL_VIEW_CONSTRAINTS_AFTER_TABLE,
    ]
    .into_iter()
    .map(|m| SelectOption {
        label: text_fn_of(m),
        hint: None,
        disabled: false,
    })
    .collect()
}

// ---- rendering --------------------------------------------------------------------------------

fn item(f: impl Fn(&mut Window, &mut App) -> AnyElement + 'static) -> ItemFn {
    Rc::new(f)
}

/// Runs `f` on the view behind `weak`.
fn on_view(
    weak: &WeakEntity<DdlView>,
    f: impl Fn(&mut DdlView, &mut Window, &mut Context<DdlView>) + 'static,
) -> impl Fn(&hacivat_ui_core::gpui_surface::ClickEvent, &mut Window, &mut App) + 'static {
    let weak = weak.clone();
    move |_, window, cx| {
        let _ = weak.update(cx, |v, cx| f(v, window, cx));
    }
}

/// The text of Fluent message `id` (a script notice) with its preformatted arguments.
fn notice_text(n: &Notice, cx: &App) -> SharedString {
    let args: Vec<(&'static str, ArgValue)> = n
        .args
        .iter()
        .map(|(k, v)| (*k, ArgValue::Text(v.clone())))
        .collect();
    live::text(cx, notice_id(n.id), args)
}

impl DdlView {
    fn toolbar(&self, weak: &WeakEntity<Self>, cx: &App) -> Toolbar {
        let theme = *Theme::get(cx);
        let mut bar =
            Toolbar::new("ddl-toolbar").height(theme.tokens.spacing.editor_toolbar_height);
        // The connection and the object (pinned: the environment mark is a trust signal).
        let profile = cx
            .try_global::<ConnectionsModel>()
            .and_then(|m| m.get(self.profile));
        let conn: SharedString = profile
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_default()
            .into();
        let env = profile
            .as_ref()
            .map(|p| crate::connections::capsule::env_kind(p.environment));
        let qualified: SharedString = crate::explorer::qualified_name(self.dbms, &self.key)
            .unwrap_or_else(|| self.name())
            .into();
        let kind = self.key.kind;
        let (c2, q2) = (conn.clone(), qualified.clone());
        bar = bar.item(ToolbarItem::pinned(
            item(move |_, cx| crumb(c2.clone(), env, kind, q2.clone(), cx)),
            Rc::new(move |_| qualified.clone()),
        ));
        bar = bar.item(ToolbarItem::pinned(
            item(|_, cx| read_only_chip(cx)),
            text_fn_of(ids::hv::DDL_VIEW_READ_ONLY),
        ));
        // Provided actions (M7's Edit source), in slot order.
        for a in self.provided(cx) {
            let label = a.label.clone();
            let id = a.id;
            let enabled = a.enabled;
            let l2 = label.clone();
            bar = bar.item(
                ToolbarItem::new(
                    35,
                    item(move |_, _| {
                        Button::new(id, l2.clone())
                            .kind(ButtonKind::Ghost)
                            .compact(true)
                            .on_raised(true)
                            .disabled(!enabled)
                            .on_click(move |_, window, cx| {
                                hacivat_ui::actions::ActionRegistry::run(id, window, cx);
                            })
                            .into_any_element()
                    }),
                    Rc::new(move |_| label.clone()),
                )
                .on_choose(move |window, cx| {
                    hacivat_ui::actions::ActionRegistry::run(id, window, cx);
                }),
            );
        }
        let can = self.can_copy();
        let w = weak.clone();
        bar = bar.trailing(
            ToolbarItem::new(
                30,
                item(move |_, cx| {
                    Button::new("ddl-refresh", tr(cx, ids::hv::DDL_VIEW_REFRESH, NoArgs))
                        .kind(ButtonKind::Ghost)
                        .icon(IconName::RefreshCw)
                        .compact(true)
                        .on_raised(true)
                        .on_click(on_view(&w, |v, _, cx| v.refresh(cx)))
                        .into_any_element()
                }),
                text_fn_of(ids::hv::DDL_VIEW_REFRESH),
            )
            .on_choose({
                let w = weak.clone();
                move |_, cx| {
                    let _ = w.update(cx, |v, cx| v.refresh(cx));
                }
            }),
        );
        let w = weak.clone();
        bar = bar.trailing(
            ToolbarItem::new(
                60,
                item(move |_, cx| {
                    Button::new("ddl-copy", tr(cx, ids::hv::DDL_VIEW_COPY, NoArgs))
                        .icon(IconName::Copy)
                        .compact(true)
                        .on_raised(true)
                        .disabled(!can)
                        .on_click(on_view(&w, |v, window, cx| v.copy(window, cx)))
                        .into_any_element()
                }),
                text_fn_of(ids::hv::DDL_VIEW_COPY),
            )
            .on_choose({
                let w = weak.clone();
                move |window, cx| {
                    let _ = w.update(cx, |v, cx| v.copy(window, cx));
                }
            }),
        );
        let w = weak.clone();
        bar.trailing(
            ToolbarItem::new(
                40,
                item(move |_, cx| {
                    Button::new(
                        "ddl-console",
                        tr(cx, ids::hv::DDL_VIEW_OPEN_CONSOLE, NoArgs),
                    )
                    .icon(IconName::Terminal)
                    .compact(true)
                    .on_raised(true)
                    .disabled(!can)
                    .on_click(on_view(&w, |v, window, cx| v.open_in_console(window, cx)))
                    .into_any_element()
                }),
                text_fn_of(ids::hv::DDL_VIEW_OPEN_CONSOLE),
            )
            .on_choose({
                let w = weak.clone();
                move |window, cx| {
                    let _ = w.update(cx, |v, cx| v.open_in_console(window, cx));
                }
            }),
        )
    }

    fn strip(&self, weak: &WeakEntity<Self>, cx: &App) -> Toolbar {
        let theme = *Theme::get(cx);
        let mut bar =
            Toolbar::new("ddl-options").height(theme.tokens.spacing.result_toolbar_height);
        let controls = self.controls();
        let opts = self.opts;
        let shown = model::shown(self.dbms);
        let eff = model::effective(self.dbms, opts);
        if controls.contains(&"qualification") {
            let w = weak.clone();
            bar = bar.item(ToolbarItem::new(
                90,
                item(move |_, cx| qualification(&w, opts.qualify, cx)),
                text_fn_of(ids::hv::DDL_VIEW_QUALIFICATION),
            ));
        }
        if controls.contains(&"constraints") {
            let select = self.constraints.clone();
            bar = bar.item(ToolbarItem::new(
                80,
                item(move |_, cx| {
                    labelled(
                        ids::hv::DDL_VIEW_CONSTRAINTS,
                        select.clone().into_any_element(),
                        cx,
                    )
                }),
                text_fn_of(ids::hv::DDL_VIEW_CONSTRAINTS),
            ));
        }
        let toggles = [
            Toggle {
                which: "server",
                element: "ddl-server",
                show: shown.server,
                checked: eff.server_definitions,
                fixed: shown.server_fixed,
                msg: ids::hv::DDL_VIEW_SERVER,
                priority: 50,
            },
            Toggle {
                which: "grants",
                element: "ddl-grants",
                show: shown.grants,
                checked: opts.grants,
                fixed: false,
                msg: ids::hv::DDL_VIEW_GRANTS,
                priority: 60,
            },
            Toggle {
                which: "owner",
                element: "ddl-owner",
                show: shown.owner,
                checked: opts.owner,
                fixed: false,
                msg: ids::hv::DDL_VIEW_OWNER,
                priority: 55,
            },
        ];
        for t in toggles.into_iter().filter(|t| t.show) {
            let Toggle {
                which,
                element,
                checked,
                fixed,
                msg,
                priority,
                ..
            } = t;
            let w = weak.clone();
            let w2 = weak.clone();
            bar = bar.item(
                ToolbarItem::new(
                    priority,
                    item(move |_, cx| {
                        let w = w.clone();
                        Checkbox::new(element, tr(cx, msg, NoArgs))
                            .checked(checked)
                            .disabled(fixed)
                            .on_toggle(move |on, _, cx| {
                                let _ = w.update(cx, |v, cx| v.toggle(which, on, cx));
                            })
                            .into_any_element()
                    }),
                    text_fn_of(msg),
                )
                .on_choose(move |_, cx| {
                    if !fixed {
                        let _ = w2.update(cx, |v, cx| v.toggle(which, !checked, cx));
                    }
                }),
            );
        }
        let server =
            self.body == Body::Script && self.shown.as_ref().is_some_and(|s| s.server_text);
        let caption = if server {
            ids::hv::DDL_VIEW_SOURCE_SERVER
        } else {
            ids::hv::DDL_VIEW_SOURCE_GENERATED
        };
        bar.trailing(ToolbarItem::new(
            10,
            item(move |_, cx| {
                let t = Theme::get(cx).tokens;
                let c = &t.colors;
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.xs))
                    .child(
                        Icon::new(if server {
                            IconName::Server
                        } else {
                            IconName::TableProperties
                        })
                        .size(IconSize::Chrome)
                        .color(c.text_3.to_hsla()),
                    )
                    .child(
                        Label::new(
                            tr(cx, caption, NoArgs),
                            TextSource::Fluent,
                            t.type_ramp.body_sm,
                            c.text_3.to_hsla(),
                        )
                        .id("ddl-source"),
                    )
                    .into_any_element()
            }),
            text_fn_of(caption),
        ))
    }

    fn toggle(&mut self, which: &str, on: bool, cx: &mut Context<Self>) {
        let mut o = self.opts;
        match which {
            "server" => o.server_definitions = on,
            "grants" => {
                o.grants = on;
                // A facet that is still missing is asked for again when Grants comes back.
                if on {
                    self.grants_asked = false;
                }
            }
            _ => o.owner = on,
        }
        self.set_options(o, cx);
    }

    fn set_qualify(&mut self, q: Qualify, cx: &mut Context<Self>) {
        self.set_options(
            DdlViewOptions {
                qualify: q,
                ..self.opts
            },
            cx,
        );
    }

    fn notices(&self, weak: &WeakEntity<Self>, cx: &App) -> Vec<AnyElement> {
        let name = self.name();
        let mut out = Vec::new();
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let row = |icon: IconName,
                   line: SharedString,
                   second: Option<SharedString>,
                   button: Option<Button>,
                   id: &'static str| {
            let mut text = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(t.spacing.s_2xs))
                .child(
                    paragraph(line, TextSource::Fluent, t.type_ramp.body, c.text.to_hsla()).id(id),
                );
            if let Some(s) = second {
                text = text.child(paragraph(
                    s,
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                ));
            }
            let mut r = div()
                .debug_selector(move || format!("ddl-notice-{id}"))
                .flex()
                .flex_row()
                .items_start()
                .gap(px(9.0))
                .px(px(t.spacing.lg))
                .py(px(t.spacing.sm))
                .border_b_1()
                .border_color(c.seam.to_hsla())
                .child(
                    div().mt(px(1.0)).child(
                        Icon::new(icon)
                            .size(IconSize::Chrome)
                            .color(c.text_2.to_hsla()),
                    ),
                )
                .child(text);
            if let Some(b) = button {
                r = r.child(b);
            }
            r.into_any_element()
        };
        let arg = || Text(name.clone());
        match self.freshness {
            Freshness::Changed => out.push(row(
                IconName::History,
                tr(
                    cx,
                    ids::hv::DDL_VIEW_STALE,
                    ids::hv::HvDdlViewStale { name: arg() },
                ),
                None,
                Some(reload_button(weak, cx)),
                "ddl-stale",
            )),
            Freshness::Unknown => out.push(row(
                IconName::Info,
                tr(
                    cx,
                    ids::hv::DDL_VIEW_UNKNOWN,
                    ids::hv::HvDdlViewUnknown { name: arg() },
                ),
                None,
                Some(reload_button(weak, cx)),
                "ddl-unknown",
            )),
            Freshness::Gone => {
                let w = weak.clone();
                out.push(row(
                    IconName::TriangleAlert,
                    tr(
                        cx,
                        ids::hv::DDL_VIEW_GONE,
                        ids::hv::HvDdlViewGone { name: arg() },
                    ),
                    Some(tr(cx, ids::hv::DDL_VIEW_GONE_BODY, NoArgs)),
                    Some(
                        Button::new("ddl-close", tr(cx, ids::hv::DDL_VIEW_CLOSE_TAB, NoArgs))
                            .compact(true)
                            .on_click(on_view(&w, |v, window, cx| v.close_tab(window, cx))),
                    ),
                    "ddl-gone",
                ))
            }
            Freshness::Checking | Freshness::Current => {}
        }
        if self.body == Body::Script
            && let Some(shown) = &self.shown
        {
            for n in &shown.notices {
                let icon = if n.id == WARN_EXECUTABLE_COMMENT || n.id == GRANTS_PARTIAL {
                    IconName::TriangleAlert
                } else {
                    IconName::Info
                };
                out.push(row(
                    icon,
                    notice_text(n, cx),
                    None,
                    None,
                    "ddl-script-notice",
                ));
            }
        }
        out
    }

    fn code(&self, cx: &App) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let name = Text(self.name());
        let centered = |children: Vec<AnyElement>| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(t.spacing.xs))
                .px(px(t.spacing.lg))
                .py(px(t.spacing.lg))
                .children(children)
                .into_any_element()
        };
        match self.body {
            Body::Script => div()
                .id("ddl-code")
                .flex_1()
                .min_h_0()
                .child(self.editor.clone())
                .into_any_element(),
            Body::Loading => centered(vec![
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.sm))
                    .child(run_marker(&theme))
                    .child(
                        Label::new(
                            tr(
                                cx,
                                ids::hv::DDL_VIEW_LOADING,
                                ids::hv::HvDdlViewLoading { name },
                            ),
                            TextSource::Fluent,
                            t.type_ramp.body,
                            c.text_2.to_hsla(),
                        )
                        .id("ddl-loading"),
                    )
                    .into_any_element(),
            ]),
            Body::Unsupported => centered(vec![
                paragraph(
                    tr(
                        cx,
                        ids::hv::DDL_VIEW_UNSUPPORTED,
                        ids::hv::HvDdlViewUnsupported {
                            kind: Text(model::unsupported_selector(self.key.kind).to_string()),
                        },
                    ),
                    TextSource::Fluent,
                    t.type_ramp.body_strong,
                    c.text.to_hsla(),
                )
                .id("ddl-unsupported")
                .into_any_element(),
                paragraph(
                    tr(
                        cx,
                        ids::hv::DDL_VIEW_UNSUPPORTED_BODY,
                        ids::hv::HvDdlViewUnsupportedBody { name },
                    ),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )
                .into_any_element(),
            ]),
            Body::NotLoaded => centered(vec![
                paragraph(
                    tr(
                        cx,
                        ids::hv::DDL_VIEW_NOT_LOADED,
                        ids::hv::HvDdlViewNotLoaded { name },
                    ),
                    TextSource::Fluent,
                    t.type_ramp.body_strong,
                    c.text.to_hsla(),
                )
                .id("ddl-not-loaded")
                .into_any_element(),
                paragraph(
                    tr(cx, ids::hv::DDL_VIEW_NOT_LOADED_BODY, NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )
                .into_any_element(),
            ]),
        }
    }
}

impl Render for DdlView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let c = &theme.tokens.colors;
        let weak = cx.entity().downgrade();
        let toolbar = self.toolbar(&weak, cx);
        let strip = self.strip(&weak, cx);
        let notices = self.notices(&weak, cx);
        div()
            .id("ddl-view")
            .size_full()
            .flex()
            .flex_col()
            .bg(c.panel.to_hsla())
            .child(
                div()
                    .bg(c.raised.to_hsla())
                    .border_b_1()
                    .border_color(c.seam.to_hsla())
                    .child(toolbar),
            )
            .child(
                div()
                    .border_b_1()
                    .border_color(c.seam.to_hsla())
                    .child(strip),
            )
            .children(notices)
            .child(self.code(cx))
    }
}

/// One checkbox of the strip.
struct Toggle {
    which: &'static str,
    /// The checkbox's element id (`checkbox-<id>` for tests).
    element: &'static str,
    show: bool,
    checked: bool,
    /// Checked and not changeable (SQLite's server text).
    fixed: bool,
    msg: hacivat_l10n::MsgRef<NoArgs>,
    priority: u8,
}

/// The Reload button of the stale and couldn't-check notices.
fn reload_button(weak: &WeakEntity<DdlView>, cx: &App) -> Button {
    Button::new("ddl-reload", tr(cx, ids::hv::DDL_VIEW_RELOAD, NoArgs))
        .icon(IconName::RotateCw)
        .compact(true)
        .on_click(on_view(weak, |v, _, cx| v.refresh(cx)))
}

/// The connection, a chevron, the kind icon and the qualified name (mono; the name is data and
/// may ellipsize, the environment mark never does).
fn crumb(
    conn: SharedString,
    env: Option<hacivat_ui::capsule::EnvKind>,
    kind: hacivat_meta::ObjectKind,
    qualified: SharedString,
    cx: &App,
) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .min_w_0()
        .gap(px(t.spacing.sm));
    if let Some(env) = env {
        row = row.child(env_swatch_on_raised(env, &theme));
    }
    row.child(
        Label::new(
            conn,
            TextSource::Data,
            t.type_ramp.mono_data,
            c.text_2.to_hsla(),
        )
        .id("ddl-connection"),
    )
    .child(
        Icon::new(IconName::ChevronRight)
            .size(IconSize::Chrome)
            .color(c.text_2.to_hsla()),
    )
    .child(
        Icon::new(crate::explorer::rows::kind_icon(kind))
            .size(IconSize::Chrome)
            .color(c.text_2.to_hsla()),
    )
    .child(
        Label::new(
            qualified,
            TextSource::Data,
            t.type_ramp.mono_data,
            c.text.to_hsla(),
        )
        .truncate(true)
        .id("ddl-object"),
    )
    .into_any_element()
}

/// The read-only chip (never hides: a trust signal).
fn read_only_chip(cx: &App) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    div()
        .id("ddl-read-only")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.xs))
        .h(px(22.0))
        .px(px(t.spacing.sm))
        .rounded(px(t.radii.sm))
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .child(
            Icon::new(IconName::Lock)
                .size(IconSize::Chrome)
                .color(c.text_2.to_hsla()),
        )
        .child(Label::new(
            tr(cx, ids::hv::DDL_VIEW_READ_ONLY, NoArgs),
            TextSource::Fluent,
            t.type_ramp.body_sm,
            c.text_2.to_hsla(),
        ))
        .into_any_element()
}

/// A strip control after its label.
fn labelled(msg: hacivat_l10n::MsgRef<NoArgs>, control: AnyElement, cx: &App) -> AnyElement {
    let t = Theme::get(cx).tokens;
    let c = &t.colors;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.sm))
        .child(Label::new(
            tr(cx, msg, NoArgs),
            TextSource::Fluent,
            t.type_ramp.body_sm,
            c.text_2.to_hsla(),
        ))
        .child(control)
        .into_any_element()
}

/// Qualification: Auto / Never / Always.
fn qualification(weak: &WeakEntity<DdlView>, current: Qualify, cx: &App) -> AnyElement {
    let mut seg = Segmented::new("ddl-qualify");
    for (q, msg) in [
        (Qualify::Auto, ids::hv::DDL_VIEW_QUAL_AUTO),
        (Qualify::Never, ids::hv::DDL_VIEW_QUAL_NEVER),
        (Qualify::Always, ids::hv::DDL_VIEW_QUAL_ALWAYS),
    ] {
        let label = tr(cx, msg, NoArgs);
        let w = weak.clone();
        seg = seg.item(SegmentItem {
            content: SegmentContent::Label(label.clone()),
            pressed: q == current,
            tooltip: label,
            a11y_name: None,
            on_click: Rc::new(move |_, _, cx| {
                let _ = w.update(cx, |v, cx| v.set_qualify(q, cx));
            }),
        });
    }
    labelled(ids::hv::DDL_VIEW_QUALIFICATION, seg.into_any_element(), cx)
}
