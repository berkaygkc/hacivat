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
//! **Freshness** (surface 11). Every text the view puts on screen is checked before it can
//! read as current: the first one, and every one that replaces it (an option, the server
//! toggle, a facet that arrived, Refresh, Reload, F5); freshness never carries over to a new
//! text (review I-2). Until the verdict lands a quiet "Checking whether … changed" caption
//! shows. The verdict is Task 22's (`IntroHandle::check_state_verdicts`):
//! - `Current`, or `Changed` with a successful re-read: current only when the catalog still
//!   gives the shown text (a publish that landed while the check ran leaves the old text on
//!   screen: "changed on the server", review I-1) and vouches for it ([`DdlView::vouched`]:
//!   the namespace is not stale, and server text from the `Sources` facet has that facet
//!   loaded and not stale, T19 review m6); otherwise "couldn't check";
//! - gone from a loaded namespace: "no longer exists" (Close tab);
//! - `Unknown`, a `Dropped` the catalog does not confirm, a check that failed or could not
//!   run: "couldn't check" (Reload). An `Unknown` never shows as current.
//!
//! While the tab is open, a newer catalog whose script differs marks it changed (never an
//! automatic reload, surface 11 "Out of scope"), and one that no longer vouches for the same
//! text (a stale namespace, a stale `Sources` facet) turns current into "couldn't check"
//! (review I-3). Comparisons regenerate with the schema the shown text was made with, so a
//! console switch never reads as a server change (review m-3).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_ddl::produce::{GRANTS_PARTIAL, WARN_EXECUTABLE_COMMENT};
use hacivat_ddl::script::{Notice, Script};
use hacivat_introspect::introspector::Staleness;
use hacivat_l10n::{ArgValue, NoArgs, Text, ids};
use hacivat_meta::kind::{DbLevel, database_level};
use hacivat_meta::props::FacetKind;
use hacivat_meta::snapshot::LoadState;
use hacivat_meta::userdata::{DdlViewOptions, Placement, Qualify};
use hacivat_meta::{CatalogSnapshot, Level, Name, ObjectKey};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_sql_editor::SqlLanguageService;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::env_swatch_on_raised;
use hacivat_ui::checkbox::Checkbox;
use hacivat_ui::focus_home::{HomeSlot, focus_home};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::Tip;
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented};
use hacivat_ui::select::{Select, SelectEvent, SelectOption};
use hacivat_ui::step_trail::run_marker;
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui::toolbar::{ItemFn, Toolbar, ToolbarItem};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use hacivat_ui_editor::{DocId, EDITOR_CONTEXT, EditorConfig, EditorView, LanguageService};
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
    /// The tooltip of the server toggle when it is fixed (SQLite, ruling R-W4-12).
    pub server_tip: Option<String>,
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
    /// The text is the `Sources` facet's (a table's or an index's server text), not a
    /// `SourceRef` the object's own props name.
    facet_text: bool,
    /// The current schema the text was generated with (`Qualify::Auto`): comparisons with a
    /// newer catalog regenerate with it, so a console switch never reads as a server change
    /// (review m-3).
    schema: Option<Name>,
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
    /// The latest state check (review fix round): an older answer that lands later is ignored.
    check_seq: u64,
    /// The snapshot the view last looked at (`Arc` identity), so an unrelated global change
    /// rebuilds nothing.
    seen: Option<Arc<CatalogSnapshot>>,
    // ## m4-t33b-carries (T25 review m-5): the notices' Reload button.
    reload_focus: FocusHandle,
    // ## m4-t33d (T33 flaky2 review I1, NOTES surface 11 "Keyboard"): the "no longer exists"
    // notice's Close tab, a Tab stop the tests can name.
    close_focus: FocusHandle,
    // ## m4-t33-flaky2 (NOTES surface 10 "Keyboard": the toolbar and the strip are Tab stops
    // in visual order; surface 11 "Keyboard": the notice's button is the first Tab stop after
    // the toolbar)
    /// The toolbar's buttons: Refresh, Copy, Open in console, and each provided action's (by
    /// command). The view owns them so focus survives a re-render and the tests can name them
    /// (a Button's own handle is a Tab stop too since T33d, R-T33-1).
    toolbar_focus: ToolbarFocus,
    // ## m4-t33d-fr1 (ruling R-T33d-5): the view is a focus home (`focus_target`).
    home: HomeSlot,
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
        // The text takes the editor's commands (selection, the standard copy) and the view's
        // F5 (`DdlViewCode`) while it has the focus.
        let focus = editor.focus_handle(cx);
        crate::shell::register_focus_context(cx, focus.clone(), EDITOR_CONTEXT);
        crate::shell::hooks::add_focus_context(cx, focus.clone(), super::DDL_VIEW_CODE_CONTEXT);
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
            cx.on_release(move |_, cx| {
                crate::shell::unregister_focus_context(cx, &focus);
                super::forget(object, cx);
            }),
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
            check_seq: 0,
            seen: None,
            reload_focus: tab_stop(cx),
            close_focus: tab_stop(cx),
            toolbar_focus: ToolbarFocus {
                refresh: tab_stop(cx),
                copy: tab_stop(cx),
                console: tab_stop(cx),
                provided: RefCell::default(),
            },
            home: HomeSlot::new(),
            _subscriptions: subscriptions,
        };
        view.load(Why::Show, cx);
        view
    }

    /// The read-only editor that holds the text.
    pub fn editor(&self) -> &Entity<EditorView> {
        &self.editor
    }

    // ## m4-t33b-carries (T25 review m-5)
    /// The notices' Reload button (tests: its place in the Tab order).
    pub fn reload_focus(&self) -> &FocusHandle {
        &self.reload_focus
    }

    // ## m4-t33-flaky2
    /// The toolbar's Open in console button (tests: its place in the Tab order).
    pub fn console_focus(&self) -> &FocusHandle {
        &self.toolbar_focus.console
    }

    // ## m4-t33d (T33 flaky2 review m1)
    /// The toolbar's Refresh button (tests: its place in the Tab order).
    pub fn refresh_focus(&self) -> &FocusHandle {
        &self.toolbar_focus.refresh
    }

    // ## m4-t33d (T33 flaky2 review m1)
    /// The toolbar's Copy button (tests: its place in the Tab order).
    pub fn copy_focus(&self) -> &FocusHandle {
        &self.toolbar_focus.copy
    }

    // ## m4-t33d (T33 flaky2 review I1)
    /// The "no longer exists" notice's Close tab button (tests: its place in the Tab order).
    pub fn close_focus(&self) -> &FocusHandle {
        &self.close_focus
    }

    // ## m4-t33d (T33 flaky2 review I2, ruling R-T33d-4); m4-t33d-fr1 (ruling R-T33d-5): the
    // view's focus home primary. A focused control this frame no longer draws (Reload once its
    // notice is gone, Copy or Open in console once disabled, a provided action once disabled,
    // a toolbar item that overflowed into ⋯) hands the focus here (`hacivat_ui::focus_home`):
    // the code area when the text is shown (it is drawn whenever the text is, F5 and Tab work
    // there), else Refresh.
    fn focus_target(&self, cx: &App) -> FocusHandle {
        if self.body == Body::Script {
            self.editor.focus_handle(cx)
        } else {
            self.toolbar_focus.refresh.clone()
        }
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

    fn generate(&self, snap: &Arc<CatalogSnapshot>, schema: Option<Name>, cx: &App) -> Generated {
        let Some(catalog) = &self.catalog else {
            return Generated::NotLoaded;
        };
        let sources = catalog.sources(cx);
        model::generate(self.dbms, snap, &*sources, &self.key, self.opts, schema)
    }

    /// Whether `snap` still gives the shown text (regenerated with the schema it was made
    /// with); `None` when nothing is shown or the object no longer generates.
    fn still_shown(&self, snap: &Arc<CatalogSnapshot>, cx: &App) -> Option<bool> {
        let shown = self.shown.as_ref()?;
        match self.generate(snap, shown.schema.clone(), cx) {
            Generated::Script(s) => Some(model::text_of(&s) == shown.text),
            _ => None,
        }
    }

    /// Whether the catalog vouches for the shown text when a check says it is current (the one
    /// rule every path applies, review I-3): its namespace is not stale (cached, refresh after
    /// DDL with Ask or Never, a held deletion), and server text from the `Sources` facet has
    /// that facet loaded and not stale (T19 review m6: `DdlSources` has no load state).
    fn vouched(&self, snap: &CatalogSnapshot) -> bool {
        !model::is_stale(snap, &self.key) && self.facet_vouched(snap)
    }

    // ## m4-t33b-carries (T25 review m-9)
    /// The facet half of [`DdlView::vouched`]: server text from the `Sources` facet has that
    /// facet loaded and not stale. A check that decided the object (its own verdict) replaces
    /// the namespace's stale mark, never this.
    fn facet_vouched(&self, snap: &CatalogSnapshot) -> bool {
        self.shown.as_ref().is_none_or(|s| {
            !s.facet_text || model::facet_vouches(snap, &self.key, FacetKind::Sources)
        })
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
        // A catalog comparison regenerates with the schema the shown text was made with.
        let schema = match (why, &self.shown) {
            (Why::Catalog, Some(shown)) if self.body == Body::Script => shown.schema.clone(),
            _ => self.current_schema(cx),
        };
        match self.generate(&snap, schema.clone(), cx) {
            Generated::Script(s) => self.got_script(s, schema, why, &snap, cx),
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
        schema: Option<Name>,
        why: Why,
        snap: &Arc<CatalogSnapshot>,
        cx: &mut Context<Self>,
    ) {
        let next = Shown {
            text: model::text_of(&s),
            notices: s.notices,
            server_text: model::is_server_text(self.dbms, self.key.kind, self.opts),
            facet_text: model::is_server_text(self.dbms, self.key.kind, self.opts)
                && !model::needs_sources(self.key.kind),
            schema,
        };
        match (why, &self.shown) {
            (Why::Catalog, Some(shown)) if self.body == Body::Script => {
                if shown.text != next.text {
                    self.freshness = Freshness::Changed;
                } else if self.freshness == Freshness::Current && !self.vouched(snap) {
                    // The same text, but the catalog no longer vouches for it (review I-3).
                    self.freshness = Freshness::Unknown;
                }
            }
            _ => {
                // Freshness never carries over to a new text (review I-2): a first text, or
                // one that replaced the shown one (an option, the server toggle, a facet that
                // arrived, Reload), is checked again before it can read as current.
                let replaced = self.shown.as_ref().is_none_or(|s| {
                    s.text != next.text
                        || s.server_text != next.server_text
                        || s.facet_text != next.facet_text
                });
                self.set_editor_text(&next.text, cx);
                self.shown = Some(next);
                self.body = Body::Script;
                if replaced {
                    self.check(cx);
                }
            }
        }
        self.ask_grants(cx);
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

    /// Checks the object against the server (surface 11): Task 22's verdict for its key.
    fn check(&mut self, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog.clone() else {
            self.freshness = Freshness::Unknown;
            return;
        };
        self.freshness = Freshness::Checking;
        self.check_seq += 1;
        let seq = self.check_seq;
        let check = catalog.check_state(self.key.clone(), cx);
        let key = self.key.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let (verdict, read) = check.outcome(&key).await;
            let _ = this.update(cx, |v, cx| {
                if v.check_seq == seq {
                    v.checked(verdict, read, cx);
                }
            });
        })
        .detach();
    }

    /// The freshness a check's verdict gives (surface 11). A `Current` verdict, or a `Changed`
    /// one whose re-read succeeded, reads current only when the catalog still gives the shown
    /// text (review I-1: a newer publish may have landed while the check ran; the text on
    /// screen is then the old one, "changed") and vouches for it ([`DdlView::vouched`]).
    /// `Unknown`, a `Dropped` the catalog does not confirm and a failed check never read
    /// current ("couldn't check").
    fn checked(&mut self, verdict: Staleness, read: bool, cx: &mut Context<Self>) {
        cx.notify();
        let Some(snap) = self.snapshot(cx) else {
            self.freshness = Freshness::Unknown;
            return;
        };
        if model::is_gone(&snap, &self.key) {
            self.freshness = Freshness::Gone;
            return;
        }
        let decided = match verdict {
            Staleness::Current => true,
            Staleness::Changed => read,
            Staleness::Dropped | Staleness::Unknown => false,
        };
        // ## m4-t27-refresh-ui (named exception, mock 11): the explorer row says it too.
        crate::explorer::stale::record(self.profile, &self.key, verdict, read, &snap, cx);
        // ## m4-t33b-carries (T25 review m-9): the object's own decided verdict replaces the
        // namespace's stale mark (Reload clears "couldn't check" on a stale namespace); a stale
        // mark arriving later demotes it again on the catalog path (`got_script`).
        self.freshness = match (decided, self.still_shown(&snap, cx)) {
            (true, Some(false)) => Freshness::Changed,
            (true, Some(true)) if self.facet_vouched(&snap) => Freshness::Current,
            _ => Freshness::Unknown,
        };
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
            let _ = ticket.await;
            let _ = this.update(cx, |v, cx| {
                // The shown text is now what the catalog holds; a state check says whether
                // it is the server's (a re-read alone never vouches: it may have failed). A
                // text that changed was checked by `load` already.
                let seq = v.check_seq;
                v.load(Why::Show, cx);
                if v.check_seq == seq && v.freshness != Freshness::Gone && v.body == Body::Script {
                    v.check(cx);
                }
            });
        })
        .detach();
    }

    /// Whether there is a text to copy or open.
    pub fn can_copy(&self) -> bool {
        self.body == Body::Script && self.shown.is_some()
    }

    /// The object's name as chrome shows it (control characters visible, review m-1).
    fn name(&self) -> String {
        super::visible_name(self.key.name())
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
            Freshness::Checking if self.body == Body::Script => {
                out.push("hv-ddl-view-checking".to_string())
            }
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
            server_tip: model::shown(self.dbms)
                .server_fixed
                .then(|| tr(cx, ids::hv::DDL_VIEW_SERVER_FIXED_TIP, NoArgs).to_string()),
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
            .map(|q| super::visible_name(&q))
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
            let focus = self.toolbar_focus.provided(id, cx);
            bar = bar.item(
                ToolbarItem::new(
                    35,
                    item(move |_, _| {
                        Button::new(id, l2.clone())
                            .kind(ButtonKind::Ghost)
                            .compact(true)
                            .on_raised(true)
                            .focus_handle(focus.clone())
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
        let focus = self.toolbar_focus.refresh.clone();
        bar = bar.trailing(
            ToolbarItem::new(
                30,
                item(move |_, cx| {
                    Button::new("ddl-refresh", tr(cx, ids::hv::DDL_VIEW_REFRESH, NoArgs))
                        .kind(ButtonKind::Ghost)
                        .icon(IconName::RefreshCw)
                        .compact(true)
                        .on_raised(true)
                        .focus_handle(focus.clone())
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
        let focus = self.toolbar_focus.copy.clone();
        bar = bar.trailing(
            ToolbarItem::new(
                60,
                item(move |_, cx| {
                    Button::new("ddl-copy", tr(cx, ids::hv::DDL_VIEW_COPY, NoArgs))
                        .icon(IconName::Copy)
                        .compact(true)
                        .on_raised(true)
                        .focus_handle(focus.clone())
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
        let focus = self.toolbar_focus.console.clone();
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
                    .focus_handle(focus.clone())
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
                        let check = Checkbox::new(element, tr(cx, msg, NoArgs))
                            .checked(checked)
                            .disabled(fixed)
                            .on_toggle(move |on, _, cx| {
                                let _ = w.update(cx, |v, cx| v.toggle(which, on, cx));
                            })
                            .into_any_element();
                        if !fixed {
                            return check;
                        }
                        // ## m4-t25-ddl-view (ruling R-W4-12): a fixed toggle says why.
                        let tip = tr(cx, ids::hv::DDL_VIEW_SERVER_FIXED_TIP, NoArgs);
                        div()
                            .id("ddl-server-fixed")
                            .debug_selector(|| "ddl-server-fixed".to_string())
                            .child(check)
                            .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                            .into_any_element()
                    }),
                    text_fn_of(msg),
                )
                // ## m4-t33b-carries (T25 carry): its ⋯ menu row keeps the check.
                .checked(checked)
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
                Some(reload_button(weak, &self.reload_focus, cx)),
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
                Some(reload_button(weak, &self.reload_focus, cx)),
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
                            // ## m4-t33d (T33 flaky2 review I1, NOTES surface 11 "Keyboard")
                            .focus_handle(self.close_focus.clone())
                            .on_click(on_view(&w, |v, window, cx| v.close_tab(window, cx))),
                    ),
                    "ddl-gone",
                ))
            }
            // Review m-2: until the verdict lands the text says it is being checked (a quiet
            // caption, no button), never nothing (which would read as current).
            Freshness::Checking if self.body == Body::Script => out.push(row(
                IconName::History,
                tr(
                    cx,
                    ids::hv::DDL_VIEW_CHECKING,
                    ids::hv::HvDdlViewChecking { name: arg() },
                ),
                None,
                None,
                "ddl-checking",
            )),
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let c = &theme.tokens.colors;
        let weak = cx.entity().downgrade();
        let toolbar = self.toolbar(&weak, cx);
        let strip = self.strip(&weak, cx);
        let notices = self.notices(&weak, cx);
        let root = div()
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
            // ## m4-t33b-carries (T25 review m-5, NOTES surface 11 "Keyboard": the notice's
            // button is the first Tab stop after the toolbar; surface 10 "Keyboard": the
            // toolbar and the strip are Tab stops in visual order): the notices' buttons come
            // right after the toolbar, though they are drawn below the options strip. Tab
            // follows paint order, so the notices come first in the tree and a reversed column
            // draws the strip above them.
            .child(
                div()
                    .flex()
                    .flex_col_reverse()
                    .child(div().flex().flex_col().children(notices))
                    .child(
                        div()
                            .border_b_1()
                            .border_color(c.seam.to_hsla())
                            .child(strip),
                    ),
            )
            .child(self.code(cx));
        // ## m4-t33d-fr1 (ruling R-T33d-5)
        focus_home(&self.home, self.focus_target(cx), root)
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

// ## m4-t33-flaky2
/// The toolbar's Tab stops (the view's `toolbar_focus`).
struct ToolbarFocus {
    refresh: FocusHandle,
    copy: FocusHandle,
    console: FocusHandle,
    /// By command, made the first time the action is drawn.
    provided: RefCell<HashMap<&'static str, FocusHandle>>,
}

impl ToolbarFocus {
    fn provided(&self, id: &'static str, cx: &App) -> FocusHandle {
        self.provided
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| tab_stop(cx))
            .clone()
    }
}

/// A focus handle that is a Tab stop.
fn tab_stop(cx: &App) -> FocusHandle {
    cx.focus_handle().tab_stop(true)
}

/// The Reload button of the stale and couldn't-check notices.
fn reload_button(weak: &WeakEntity<DdlView>, focus: &FocusHandle, cx: &App) -> Button {
    Button::new("ddl-reload", tr(cx, ids::hv::DDL_VIEW_RELOAD, NoArgs))
        .icon(IconName::RotateCw)
        .compact(true)
        .focus_handle(focus.clone())
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
        // ## m4-t33b-carries (found by the new DDL view shot): the label keeps its width; a
        // full-width control (the Constraints Select) sizes to its content here instead of
        // squeezing the label it follows ("Constrai" was clipped at every width).
        .child(div().flex_none().child(Label::new(
            tr(cx, msg, NoArgs),
            TextSource::Fluent,
            t.type_ramp.body_sm,
            c.text_2.to_hsla(),
        )))
        .child(div().flex_none().child(control))
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
