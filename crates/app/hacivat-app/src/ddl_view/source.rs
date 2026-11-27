//! What a DDL view reads and asks of a profile's catalog ([`DdlCatalog`]).
//!
//! ## m4-t25-ddl-view
//!
//! The app's catalog is the profile's engine (R-M4-31: `CatalogHub` through the live host's
//! `IntroHandle`); [`EngineCatalog`] is it. Every request answers a `RefreshTicket` that the
//! view awaits on a GPUI foreground task (a `tokio::sync` channel; the I/O runs on the
//! `CoreRuntime`), and the snapshot is read synchronously, as the explorer reads it (spec §5.4
//! rule 2). A profile whose catalog is not open answers every request at once with
//! `IntroError::NotImplemented`, as a stub does: the view then says it could not check or read.
//!
//! **A seam for Tasks 15b and 22** (plan Task 25: "the tests use a fake `IntroHandle`"). At this
//! base `IntroHandle::load_facet` answers `NotImplemented` (Task 15b), and the model keeps no
//! source text and no facet value anywhere (`SourceRef` is a hash; Task 15b adds the facet side
//! table), so [`EngineCatalog::sources`] answers "not loaded" for every text and facet: a view,
//! a routine, a trigger or server text is `DdlError::NotLoaded` and grants carry
//! `ddl-grants-not-loaded` (F31: never a script that silently lacks part of the object). Tests,
//! and the tasks that fill these, install their own catalog with [`set_catalog_provider`].

use std::rc::Rc;
use std::sync::Arc;

use hacivat_ddl::produce::{DdlSources, NoSources};
use hacivat_introspect::IntroHandle;
use hacivat_introspect::ddl_effects::Target;
use hacivat_introspect::introspector::Staleness;
use hacivat_introspect::ticket::RefreshTicket;
use hacivat_meta::merge::Trigger;
use hacivat_meta::props::FacetKind;
use hacivat_meta::{CatalogSnapshot, Level, ObjectKey};
use hacivat_proto::ProfileId;
use hacivat_ui_core::gpui_surface::{App, Global};
use tokio::sync::oneshot;

use crate::explorer::{adapter, live};

/// One profile's catalog as a DDL view uses it. Requests are made on the UI thread and never
/// block it: each answers a ticket at once.
pub trait DdlCatalog {
    /// The newest snapshot (`None`: the profile has no catalog yet).
    fn snapshot(&self, cx: &App) -> Option<Arc<CatalogSnapshot>>;
    /// Reads `key` at `level` (Task 15's `elevate`: one fragment of that object).
    fn elevate(&self, key: ObjectKey, level: Level, cx: &App) -> RefreshTicket;
    /// Checks `key` against the server and re-reads it when it changed or was dropped
    /// (HV-INTRO-15, Task 22): its verdict, then the ticket of the re-read.
    fn check_state(&self, key: ObjectKey, cx: &App) -> StateCheck;
    /// Re-reads `key` from the server (Refresh and Reload; `Trigger::User`).
    fn refresh(&self, key: ObjectKey, cx: &App) -> RefreshTicket;
    /// Loads facet `kind` of `key` (R-M4-30; Task 15b).
    fn load_facet(&self, key: ObjectKey, kind: FacetKind, cx: &App) -> RefreshTicket;
    /// The source texts and facets loaded so far.
    fn sources(&self, cx: &App) -> Rc<dyn DdlSources>;
}

/// A state check of one object (Task 22, `IntroHandle::check_state_verdicts`): the verdict
/// as soon as the stamps were compared, and the ticket that resolves once what changed or was
/// dropped was read again. A verdict whose sender was dropped (the check failed, was
/// cancelled, or there is no engine) decided nothing: `Unknown`.
pub struct StateCheck {
    pub verdict: oneshot::Receiver<Vec<(ObjectKey, Staleness)>>,
    pub ticket: RefreshTicket,
}

impl StateCheck {
    /// A check that cannot run: no verdict, the ticket a stub's.
    pub fn undecided() -> Self {
        let (_, verdict) = oneshot::channel();
        StateCheck {
            verdict,
            ticket: RefreshTicket::not_implemented(),
        }
    }

    /// Awaits the verdict of `key` (`Unknown` when the check decided nothing about it), then
    /// the re-read; the re-read's failure is reported beside it.
    pub async fn outcome(self, key: &ObjectKey) -> (Staleness, bool) {
        let verdict = match self.verdict.await {
            Ok(v) => v
                .into_iter()
                .find(|(k, _)| k.same_object(key))
                .map_or(Staleness::Unknown, |(_, s)| s),
            Err(_) => Staleness::Unknown,
        };
        let read = self.ticket.await.is_ok();
        (verdict, read)
    }
}

/// Makes the catalog of a profile (`None`: none).
pub type CatalogProvider = Rc<dyn Fn(ProfileId, &App) -> Option<Rc<dyn DdlCatalog>>>;

#[derive(Default)]
struct Provider(Option<CatalogProvider>);

impl Global for Provider {}

/// Replaces how DDL views find a profile's catalog (tests; Task 15b's facet store). Views
/// opened later use it.
pub fn set_catalog_provider(provider: CatalogProvider, cx: &mut App) {
    cx.set_global(Provider(Some(provider)));
}

/// The catalog of `profile`: the installed provider's, else the profile's engine.
pub fn catalog_for(profile: ProfileId, cx: &App) -> Option<Rc<dyn DdlCatalog>> {
    if let Some(p) = cx.try_global::<Provider>().and_then(|p| p.0.clone()) {
        return p(profile, cx);
    }
    Some(Rc::new(EngineCatalog { profile }))
}

/// The profile's engine through the live host (Task 24's `CatalogHost`).
pub struct EngineCatalog {
    pub profile: ProfileId,
}

impl EngineCatalog {
    fn handle(&self, cx: &App) -> Option<IntroHandle> {
        live::handle(self.profile, cx)
    }

    fn ask(&self, cx: &App, f: impl FnOnce(IntroHandle) -> RefreshTicket) -> RefreshTicket {
        match self.handle(cx) {
            Some(h) => f(h),
            // Not open: answered at once, as a stub answers.
            None => RefreshTicket::not_implemented(),
        }
    }
}

impl DdlCatalog for EngineCatalog {
    fn snapshot(&self, cx: &App) -> Option<Arc<CatalogSnapshot>> {
        // The engine's newest generation (the explorer's copy may trail it by a frame), else
        // what the explorer shows (the cached catalog of a profile that is not open).
        self.handle(cx)
            .map(|h| h.catalog().current())
            .filter(|s| s.generation > 0)
            .or_else(|| adapter::catalog_of(self.profile, cx))
    }

    fn elevate(&self, key: ObjectKey, level: Level, cx: &App) -> RefreshTicket {
        self.ask(cx, |h| h.elevate(key, level))
    }

    fn check_state(&self, key: ObjectKey, cx: &App) -> StateCheck {
        match self.handle(cx) {
            Some(h) => {
                let (ticket, verdict) = h.check_state_verdicts(vec![key]);
                StateCheck { verdict, ticket }
            }
            None => StateCheck::undecided(),
        }
    }

    fn refresh(&self, key: ObjectKey, cx: &App) -> RefreshTicket {
        let snap = self.snapshot(cx);
        let ns = snap
            .as_ref()
            .and_then(|s| s.find(&key).map(|o| o.ns.key().clone()));
        let Some(ns) = ns else {
            return RefreshTicket::not_implemented();
        };
        // ## m4-t27-refresh-ui (named exception, T25 review m-9): a fragment never clears a
        // namespace's stale mark (a cached namespace, refresh after DDL with Ask or Never), so
        // Reload there would keep "couldn't check" for ever: a stale namespace is read again
        // whole (a pass publishes it current), any other object alone.
        let stale = snap
            .as_deref()
            .is_some_and(|s| super::model::is_stale(s, &key));
        let target = if stale {
            Target::Namespace(ns)
        } else {
            Target::Objects {
                ns,
                keys: vec![key],
            }
        };
        self.ask(cx, |h| h.refresh(target, Trigger::User))
    }

    fn load_facet(&self, key: ObjectKey, kind: FacetKind, cx: &App) -> RefreshTicket {
        self.ask(cx, |h| h.load_facet(vec![key], kind))
    }

    // ## m4-t27-refresh-ui (named exception; the T15b hook): texts and facet values from the
    // catalog snapshot (Task 15b's store).
    fn sources(&self, cx: &App) -> Rc<dyn DdlSources> {
        match self.snapshot(cx) {
            Some(s) => Rc::new(SnapshotSources(s)),
            None => Rc::new(NoSources),
        }
    }
}

// ## m4-t27-refresh-ui (named exception; the T15b hook)
/// The DDL producer's texts and facet values, read from one catalog snapshot.
struct SnapshotSources(Arc<CatalogSnapshot>);

impl DdlSources for SnapshotSources {
    fn text(&self, r: &hacivat_meta::SourceRef) -> Option<hacivat_meta::Name> {
        self.0.source_text(r)
    }

    fn server_text(&self, k: &ObjectKey) -> Option<hacivat_meta::Name> {
        self.0.server_text(k)
    }

    fn grants(
        &self,
        k: &ObjectKey,
    ) -> hacivat_meta::props::Facet<hacivat_meta::props::GrantsFacet> {
        self.0.grants_facet(k)
    }
}
