//! Go to script (HV-SCRIPT-09, R-M4b-9; NOTES surface 7): its own layer over M4's
//! [`Palette`](hacivat_ui::palette::Palette) (there is no palette mode), listing scripts by name
//! across the libraries, with the connection chooser after a pick.
//!
//! ## m4b-t01b-seams: the stub opens an empty Go-to-script layer: its placeholder, no row, and
//! Escape closes it. Task 15 fills the rows from the "Go to script" index (Turkish-tolerant,
//! `commands::palette::search_opts`) and the chooser ([`super::chooser`]). The `scripts.goto`
//! command stays disabled until then, so nothing on a user path opens this layer.
//!
//! ## m4b-t15-goto
//! - **The layer** is the shipped palette's anatomy (the query field over 23 px rows on the
//!   raised popover surface, 12 rows at once, the 24 px key-hint footer, the same width and
//!   place) in a view of its own: the palette widget draws one title per row and always has an
//!   active row, while these rows carry a folder with its environment mark, a library, and the
//!   chooser's rows (mark, connection, database, "current", engine) and may have **no** active
//!   row (production elsewhere). Layer `scripts-goto`, context `Palette`, non-modal.
//! - **Results.** Each query change runs `ScriptIndex::query_scoped` with
//!   `commands::palette::search_opts` (I18N-09, D-074: `musteri` finds `müşteri_segment.sql` for
//!   a Turkish user); each library is queried under the environments of the connections it is
//!   shown under, so a `@production` file of a library only staging shows never appears
//!   (R-M4b-6), and a library no connection shows offers only its unscoped files. The index is
//!   built from names only, off the UI thread, once per snapshot generation; a query runs off
//!   the UI thread past 2,000 scripts (`ScriptIndex::is_large`), and the latest query wins
//!   (older answers are dropped by their sequence number). The index cuts a query to 256
//!   characters. Every name is the index's escaped display text (R-M4b-23).
//! - **A pick** computes the candidates now (`chooser::ordered`) and follows R-M4b-9: one that is
//!   not production opens at once (or focuses its open tab, R-M4b-4); production or two or more
//!   show the chooser in the same layer; none offers "Open without a connection" when an
//!   unbound tab can show the file (otherwise the hit is stale and the results are refreshed).
//! - **The chooser** has no query field: its own focus handle holds the focus (↑ ↓ ↵ Escape).
//!   Its rows are recomputed when a profile or the library changes, and on every Enter before
//!   anything opens; an active row the user moved to stays active only while it is the same
//!   place in the same environment, otherwise the preselection rule applies again (production
//!   is never preselected unless it is the active tab's connection). Escape goes back to the
//!   results when it came from them, else it closes; closing gives the focus back to where it
//!   was (the layer's restore handle), and the opened tab's editor takes it after an open.
//! - The view is a focus home whose primary is the query field or the chooser (R-M4b-43).

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_l10n::ArgValue;
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_scripts::index::{Hit, ScriptIndex};
use hacivat_scripts::service::LibraryService;
use hacivat_ui::actions::ActionFn;
use hacivat_ui::capsule::env_swatch_on_raised;
use hacivat_ui::focus_home::{HomeSlot, focus_home};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::l10n::ui_text;
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Entity, FluentBuilder as _, FocusHandle,
    Focusable as _, Global, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    WeakEntity, Window, div, px,
};
use hacivat_ui_core::keymap::{KeymapPlatform, display_chord, parse_chord};
use hacivat_ui_core::theme::{Theme, popover_surface};

use super::chooser::{self, Candidate, Decision};
use super::{DatabaseName, LibraryId, RelPath};
use crate::commands::palette::{PALETTE_CONTEXT, search_opts};
use crate::connections::ConnectionsModel;
use crate::l10n_global::text_fn_id;
use crate::shell::{Layer, Workspace};

/// The Go-to-script layer's ID.
pub const GOTO_LAYER: &str = "scripts-goto";

/// Rows shown at once (the shipped palette's).
const VISIBLE_ROWS: usize = hacivat_ui::palette::VISIBLE_ROWS;
/// Row height (DESIGN.md: completion items are 23 px).
const ROW_PX: f32 = hacivat_ui::palette::ROW_HEIGHT_PX;
/// Footer height (the palette's key-hint footer).
const FOOTER_PX: f32 = hacivat_ui::palette::FOOTER_HEIGHT_PX;
/// The most hits a query lists.
const MAX_HITS: usize = 200;

/// The chooser's rows as (connection, database), and the active row (tests).
pub type ChooserRows = (Vec<(String, String)>, Option<usize>);

/// A filled command's run and enabled functions.
type Filled = (ActionFn, fn(&App) -> bool);

/// The connection the chooser leaves out: the one a tab or tree row is under, every database
/// of it (## m4b-t15-goto fix round 1, R-T15-1).
type Exclude = Option<ProfileId>;

/// What the layer shows.
enum Mode {
    /// The query field and the scripts that match.
    Results,
    /// "Open <file> under": the connections that show the file.
    Choose(Box<Choose>),
}

/// The chooser's state.
struct Choose {
    library: LibraryId,
    rel: RelPath,
    exclude: Exclude,
    /// Opened from Go to script's results (Escape goes back to them, with this query).
    back_query: Option<String>,
    rows: Vec<Candidate>,
    /// The focused place the rows were ordered for.
    focused: Option<(ProfileId, Option<DatabaseName>)>,
    active: Option<usize>,
    first: usize,
    /// The row the user moved onto, as it was then (place and environment).
    picked: Option<(ProfileId, DatabaseName, Option<EnvTag>)>,
}

// ## m4b-t15-goto fix round 1 (I1, R-T15-2)
/// What the chooser showed as active when the user pressed Enter or clicked.
#[derive(Debug, Clone, PartialEq)]
enum Seen {
    /// A row: its connection, database and environment.
    Row(ProfileId, DatabaseName, Option<EnvTag>),
    /// "Open without a connection" (no candidate, and an unbound tab can show the file).
    Unbound,
    /// No active row.
    Nothing,
}

impl Seen {
    fn of(ch: &Choose, cx: &App) -> Seen {
        if ch.rows.is_empty() {
            return if chooser::opens_unbound(ch.library, &ch.rel, cx) {
                Seen::Unbound
            } else {
                Seen::Nothing
            };
        }
        match ch.active.and_then(|a| ch.rows.get(a)) {
            Some(c) => Seen::Row(c.profile, c.database.clone(), c.env),
            None => Seen::Nothing,
        }
    }
}

/// The layer view.
pub struct GotoHost {
    input: Entity<TextInput>,
    chooser_focus: FocusHandle,
    home: HomeSlot,
    mode: Mode,
    query: String,
    hits: Vec<Hit>,
    /// The query `hits` answer.
    hits_query: String,
    /// Whether the results answer the current query and snapshot (no "No script matches"
    /// before they do).
    loaded: bool,
    active: Option<usize>,
    first: usize,
    /// The index and the snapshot generation it was built from.
    index: Option<(u64, ScriptIndex)>,
    /// The generation being built.
    building: Option<u64>,
    /// The latest query's number (older answers are dropped).
    seq: u64,
    _build: Option<Task<()>>,
    _query: Option<Task<()>>,
    _events: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl GotoHost {
    fn new(mode: Mode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new(cx, text_fn_id("hv-scripts-goto-ph")));
        let chooser_focus = cx.focus_handle().tab_stop(true);
        let on_input = cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, input, event: &InputEvent, window, cx| match event {
                InputEvent::Changed => {
                    let q = input.read(cx).value().to_string();
                    if q != this.query {
                        this.query = q;
                        this.run_query(cx);
                    }
                }
                InputEvent::Submitted => {
                    if let Some(ix) = this.active {
                        this.pick(ix, window, cx);
                    }
                }
                InputEvent::Cancelled => close(window, cx),
                InputEvent::CompositionChanged(_) => {}
            },
        );
        // A profile added, removed, renamed or retagged: the rows are computed again.
        let on_profiles = cx.observe_global_in::<ConnectionsModel>(window, |this, _, cx| {
            this.refresh(cx);
        });
        let mut this = Self {
            input,
            chooser_focus,
            home: HomeSlot::new(),
            mode,
            query: String::new(),
            hits: Vec::new(),
            hits_query: String::new(),
            loaded: false,
            active: None,
            first: 0,
            index: None,
            building: None,
            seq: 0,
            _build: None,
            _query: None,
            _events: None,
            _subscriptions: vec![on_input, on_profiles],
        };
        this.follow_service(cx);
        this.refresh(cx);
        this
    }

    /// The handle that holds the focus in the current mode.
    fn primary(&self, cx: &App) -> FocusHandle {
        match self.mode {
            Mode::Results => self.input.read(cx).focus_handle(cx),
            Mode::Choose(_) => self.chooser_focus.clone(),
        }
    }

    /// Redraws after every change of the library service's snapshot (a file added, renamed,
    /// hidden; a library attached).
    fn follow_service(&mut self, cx: &mut Context<Self>) {
        let Some(service) = super::open::service(cx) else {
            return;
        };
        let mut events = service.subscribe(None);
        self._events = Some(cx.spawn(async move |this, cx| {
            use tokio::sync::broadcast::error::RecvError;
            while let Ok(_) | Err(RecvError::Lagged(_)) = events.recv().await {
                while events.try_recv().is_ok() {}
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// Computes what the current mode shows again (the chooser's rows, the results).
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::Choose(_)) {
            self.recompute_rows(cx);
        }
        self.run_query(cx);
        cx.notify();
    }

    // ---- results ----------------------------------------------------------------------------

    /// Runs the current query over the index of the current snapshot (building it first when
    /// the snapshot moved on).
    fn run_query(&mut self, cx: &mut Context<Self>) {
        let Some(service) = super::open::service(cx) else {
            self.set_hits(Vec::new(), cx);
            return;
        };
        let generation = service.snapshot().generation;
        let index = match &self.index {
            Some((g, index)) if *g == generation => index.clone(),
            _ => {
                self.build(service, generation, cx);
                return;
            }
        };
        self.seq += 1;
        let seq = self.seq;
        let query = self.query.clone();
        let opts = search_opts(cx);
        let scopes = scopes(cx);
        if !index.is_large() {
            let hits = index.query_scoped(&query, opts, MAX_HITS, &scopes);
            self._query = None;
            self.set_hits(hits, cx);
            return;
        }
        let job =
            cx.background_spawn(async move { index.query_scoped(&query, opts, MAX_HITS, &scopes) });
        self._query = Some(cx.spawn(async move |this, cx| {
            let hits = job.await;
            let _ = this.update(cx, |this, cx| {
                // The latest query wins.
                if this.seq == seq {
                    this.set_hits(hits, cx);
                }
            });
        }));
    }

    /// Builds the index of the current snapshot off the UI thread, then runs the query.
    fn build(&mut self, service: Arc<LibraryService>, generation: u64, cx: &mut Context<Self>) {
        if self.building == Some(generation) {
            return;
        }
        self.building = Some(generation);
        let job = cx.background_spawn(async move {
            let snap = service.snapshot();
            (snap.generation, ScriptIndex::build(&snap))
        });
        self._build = Some(cx.spawn(async move |this, cx| {
            let (built, index) = job.await;
            let _ = this.update(cx, |this, cx| {
                this.building = None;
                this.index = Some((built, index));
                this.run_query(cx);
            });
        }));
    }

    fn set_hits(&mut self, hits: Vec<Hit>, cx: &mut Context<Self>) {
        // ## m4b-t15-goto fix round 1 (review minor 6): a refresh of the same query keeps the
        // file that was active (a library event between a frame and Enter never moves it).
        let kept = (self.hits_query == self.query)
            .then(|| self.active.and_then(|a| self.hits.get(a)))
            .flatten()
            .and_then(|was| {
                hits.iter()
                    .position(|h| h.library == was.library && h.rel == was.rel)
            });
        self.hits_query = self.query.clone();
        self.active = kept.or((!hits.is_empty()).then_some(0));
        self.first = scroll_to(
            if kept.is_some() { self.first } else { 0 },
            self.active.unwrap_or(0),
        );
        self.hits = hits;
        self.loaded = true;
        cx.notify();
    }

    /// The user picked hit `ix`: open it, or ask where (R-M4b-9).
    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.hits.get(ix).cloned() else {
            return;
        };
        let (rows, focused) = chooser::ordered(hit.library, &hit.rel, None, cx);
        match chooser::decide(&rows) {
            Decision::Open(c) => {
                close(window, cx);
                super::open_script(cx, hit.library, hit.rel, Some(c.profile), Some(c.database));
            }
            Decision::Unbound if !chooser::opens_unbound(hit.library, &hit.rel, cx) => {
                // ## m4b-t15-goto fix round 1 (review minor 3): listed only where the script may
                // not run (a single-session MySQL profile, R-T14-3): `open_script` says why.
                if let Some((p, db)) = chooser::refused_place(hit.library, &hit.rel, None, cx) {
                    close(window, cx);
                    super::open_script(cx, hit.library, hit.rel, Some(p), Some(db));
                    return;
                }
                // The hit no longer shows anywhere (hidden or gone since it was listed).
                self.run_query(cx);
            }
            Decision::Unbound | Decision::Choose => {
                let active = chooser::preselect(&rows, focused.as_ref());
                self.mode = Mode::Choose(Box::new(Choose {
                    library: hit.library,
                    rel: hit.rel,
                    exclude: None,
                    back_query: Some(self.query.clone()),
                    rows,
                    focused,
                    active,
                    first: 0,
                    picked: None,
                }));
                window.focus(&self.chooser_focus, cx);
                cx.notify();
            }
        }
    }

    // ---- the chooser ------------------------------------------------------------------------

    fn choose_for(library: LibraryId, rel: RelPath, exclude: Exclude, cx: &App) -> Choose {
        let (rows, focused) = chooser::ordered(library, &rel, exclude, cx);
        let active = chooser::preselect(&rows, focused.as_ref());
        Choose {
            library,
            rel,
            exclude,
            back_query: None,
            rows,
            focused,
            active,
            first: 0,
            picked: None,
        }
    }

    /// Computes the chooser's rows again from the live snapshot and profiles. The row the user
    /// moved onto stays active while it is the same place in the same environment; otherwise
    /// the preselection rule applies again.
    fn recompute_rows(&mut self, cx: &mut Context<Self>) {
        let Mode::Choose(ch) = &mut self.mode else {
            return;
        };
        let (rows, focused) = chooser::ordered(ch.library, &ch.rel, ch.exclude, cx);
        let kept = ch.picked.as_ref().and_then(|(p, d, env)| {
            rows.iter()
                .position(|c| c.profile == *p && c.database == *d && c.env == *env)
        });
        if kept.is_none() {
            ch.picked = None;
        }
        ch.active = kept.or_else(|| chooser::preselect(&rows, focused.as_ref()));
        ch.rows = rows;
        ch.focused = focused;
        ch.first = ch.first.min(ch.rows.len().saturating_sub(VISIBLE_ROWS));
        cx.notify();
    }

    fn move_active(&mut self, down: bool) {
        match &mut self.mode {
            Mode::Results => {
                let n = self.hits.len();
                if n == 0 {
                    return;
                }
                let next = match self.active {
                    None => 0,
                    Some(a) if down => (a + 1).min(n - 1),
                    Some(a) => a.saturating_sub(1),
                };
                self.active = Some(next);
                self.first = scroll_to(self.first, next);
            }
            Mode::Choose(ch) => {
                let n = ch.rows.len();
                if n == 0 {
                    return;
                }
                // With no active row (production only), ↓ moves onto the first, ↑ the last.
                let next = match ch.active {
                    None if down => 0,
                    None => n - 1,
                    Some(a) if down => (a + 1).min(n - 1),
                    Some(a) => a.saturating_sub(1),
                };
                ch.active = Some(next);
                let c = &ch.rows[next];
                ch.picked = Some((c.profile, c.database.clone(), c.env));
                ch.first = scroll_to(ch.first, next);
            }
        }
    }

    /// Enter in the chooser. ## m4b-t15-goto fix round 1 (I1, R-T15-2): the rows are checked
    /// against the live state first, and only the row the user saw highlighted opens, only if
    /// it survived the check unchanged (same connection, database and environment; the file is
    /// the chooser's). Anything else redraws and opens nothing: the preselection rule is never
    /// applied at activation time, so a row the user never saw never opens. "Open without a
    /// connection" opens only while there is still no candidate.
    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let seen = match &self.mode {
            Mode::Choose(ch) => Seen::of(ch, cx),
            Mode::Results => return,
        };
        self.recompute_rows(cx);
        let Mode::Choose(ch) = &mut self.mode else {
            return;
        };
        let (library, rel) = (ch.library, ch.rel.clone());
        match seen {
            Seen::Row(profile, database, env) => {
                let Some(ix) = ch
                    .rows
                    .iter()
                    .position(|c| c.profile == profile && c.database == database && c.env == env)
                else {
                    return;
                };
                ch.active = Some(ix);
                close(window, cx);
                super::open_script(cx, library, rel, Some(profile), Some(database));
            }
            Seen::Unbound => {
                if ch.rows.is_empty() && chooser::opens_unbound(library, &rel, cx) {
                    close(window, cx);
                    super::open_script(cx, library, rel, None, None);
                }
            }
            // No row was active. With no candidate at all, a place where the script may not run
            // (a single-session MySQL profile) says why (review minor 3).
            Seen::Nothing => {
                if ch.rows.is_empty()
                    && let Some((p, db)) = chooser::refused_place(library, &rel, ch.exclude, cx)
                {
                    close(window, cx);
                    super::open_script(cx, library, rel, Some(p), Some(db));
                }
            }
        }
    }

    /// A click on chooser row `ix`.
    fn click_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Mode::Choose(ch) = &mut self.mode
            && let Some(c) = ch.rows.get(ix)
        {
            ch.picked = Some((c.profile, c.database.clone(), c.env));
            ch.active = Some(ix);
        }
        self.choose(window, cx);
    }

    /// Escape in the chooser: back to the results it came from, else close.
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let back = match &self.mode {
            Mode::Choose(ch) => ch.back_query.clone(),
            Mode::Results => None,
        };
        let Some(q) = back else {
            close(window, cx);
            return;
        };
        self.mode = Mode::Results;
        self.input.update(cx, |i, cx| {
            i.set_value(q.clone());
            cx.notify();
        });
        self.query = q;
        self.run_query(cx);
        let focus = self.input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.read(cx).is_composing() {
            return;
        }
        let m = &event.keystroke.modifiers;
        if m.platform || m.control || m.alt || m.function || m.shift {
            return;
        }
        let choosing = matches!(self.mode, Mode::Choose(_));
        match event.keystroke.key.as_str() {
            "up" => self.move_active(false),
            "down" => self.move_active(true),
            // In the results these come from the field (`Submitted`, `Cancelled`).
            "enter" if choosing => self.choose(window, cx),
            "escape" if choosing => self.back(window, cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    // ---- tests ------------------------------------------------------------------------------

    /// The results' rows as "folder/name · library" (tests).
    pub fn result_rows(&self) -> Vec<String> {
        self.hits
            .iter()
            .map(|h| {
                let path = if h.display_folder.is_empty() {
                    h.display_name.clone()
                } else {
                    format!("{}/{}", h.display_folder, h.display_name)
                };
                format!("{path} · {}", h.library_name)
            })
            .collect()
    }

    /// Whether the chooser is shown.
    pub fn is_choosing(&self) -> bool {
        matches!(self.mode, Mode::Choose(_))
    }

    /// The chooser's rows as (connection, database) and the active row (tests).
    pub fn chooser_rows(&self) -> Option<ChooserRows> {
        let Mode::Choose(ch) = &self.mode else {
            return None;
        };
        Some((
            ch.rows
                .iter()
                .map(|c| (c.name.clone(), c.database.as_str().to_owned()))
                .collect(),
            ch.active,
        ))
    }

    /// The results' active row (tests).
    pub fn active_result(&self) -> Option<usize> {
        self.active
    }

    /// Picks result row `ix`, as Enter or a click on it does (the shots).
    pub fn confirm_result(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.pick(ix, window, cx);
    }

    /// The query field (tests type into it).
    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    /// The chooser's focus handle (tests).
    pub fn chooser_focus(&self) -> &FocusHandle {
        &self.chooser_focus
    }
}

/// The first visible row once `active` is shown.
fn scroll_to(first: usize, active: usize) -> usize {
    if active < first {
        active
    } else if active >= first + VISIBLE_ROWS {
        active + 1 - VISIBLE_ROWS
    } else {
        first
    }
}

/// The environments each library is shown under: those of the places that bind it.
fn scopes(cx: &App) -> HashMap<LibraryId, Vec<Option<EnvTag>>> {
    let mut out: HashMap<LibraryId, Vec<Option<EnvTag>>> = HashMap::new();
    for place in super::projection::script_places(cx) {
        for lib in &place.libraries {
            let envs = out.entry(*lib).or_default();
            if !envs.contains(&place.profile.environment) {
                envs.push(place.profile.environment);
            }
        }
    }
    out
}

fn msg(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    super::tabs::msg(cx, id, args)
}

fn current_platform() -> KeymapPlatform {
    if cfg!(target_os = "macos") {
        KeymapPlatform::MacOs
    } else if cfg!(windows) {
        KeymapPlatform::Windows
    } else {
        KeymapPlatform::Linux
    }
}

fn key_label(chord: &str) -> SharedString {
    let presses = parse_chord(chord).unwrap_or_default();
    display_chord(&presses, current_platform()).into()
}

/// The environment of the folder `rel` sits in, when an `@<env>` folder scopes it.
fn folder_env(rel: &RelPath) -> Option<EnvTag> {
    match hacivat_scripts::envscope::scope_of(rel) {
        hacivat_scripts::envscope::EnvScope::Only(set) => {
            EnvTag::ALL.into_iter().find(|t| set.contains(*t))
        }
        _ => None,
    }
}

impl Render for GotoHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = t.colors;
        let ty = t.type_ramp;
        let width = (t.spacing.completion_list_width + t.spacing.completion_detail_width)
            .min(f32::from(window.viewport_size().width) - 2.0 * t.spacing.lg);

        let row = |id: (&'static str, usize), active: bool| {
            div()
                .id(id)
                .role(Role::ListBoxOption)
                .aria_selected(active)
                .flex()
                .items_center()
                .gap(px(t.spacing.md))
                .h(px(ROW_PX))
                .px(px(t.spacing.lg))
                .when(active, |el| el.bg(c.blue_tint_strong.to_hsla()))
                .when(!active, |el| el.hover(|s| s.bg(c.hover.to_hsla())))
        };
        let note = |text: SharedString| {
            div()
                .flex()
                .items_center()
                .h(px(ROW_PX))
                .px(px(t.spacing.lg))
                .min_w(px(0.0))
                .child(
                    Label::new(text, TextSource::Fluent, ty.body, c.text_2.to_hsla())
                        .truncate(true),
                )
        };
        let hint = |keys: &str, text: SharedString| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(t.spacing.xs))
                .child(Label::new(
                    key_label(keys),
                    TextSource::Shortcut,
                    ty.mono_kbd,
                    c.text_2.to_hsla(),
                ))
                .child(Label::new(
                    text,
                    TextSource::Fluent,
                    ty.caption,
                    c.text_2.to_hsla(),
                ))
        };

        let (top, rows, hints, foot_note): (AnyElement, Vec<AnyElement>, _, Option<SharedString>) =
            match &self.mode {
                Mode::Results => {
                    let has_libraries = super::open::service(cx)
                        .is_some_and(|s| !s.snapshot().libraries.is_empty());
                    let mut rows: Vec<AnyElement> = self
                        .hits
                        .iter()
                        .enumerate()
                        .skip(self.first)
                        .take(VISIBLE_ROWS)
                        .map(|(ix, h)| {
                            let active = self.active == Some(ix);
                            let icon = if h.rel.name().ends_with(".md") {
                                IconName::FileText
                            } else {
                                IconName::FileCode
                            };
                            row(("scripts-goto-row", ix), active)
                                .aria_label(SharedString::from(h.display_name.clone()))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.pick(ix, window, cx);
                                }))
                                .child(Icon::new(icon).size(IconSize::Chrome))
                                .child(
                                    div().flex_shrink(1.0).min_w(px(0.0)).child(
                                        Label::new(
                                            h.display_name.clone(),
                                            TextSource::Data,
                                            ty.mono_data,
                                            c.text.to_hsla(),
                                        )
                                        .truncate(true)
                                        .highlight(h.ranges.clone(), c.blue.to_hsla()),
                                    ),
                                )
                                .when(!h.display_folder.is_empty(), |el| {
                                    el.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(t.spacing.xs))
                                            .flex_shrink(1000.0)
                                            .min_w(px(0.0))
                                            .when_some(folder_env(&h.rel), |el, env| {
                                                el.child(env_swatch_on_raised(
                                                    crate::connections::capsule::env_kind(Some(
                                                        env,
                                                    )),
                                                    &theme,
                                                ))
                                            })
                                            .child(
                                                Label::new(
                                                    format!("{}/", h.display_folder),
                                                    TextSource::Data,
                                                    ty.mono_micro,
                                                    c.text_2.to_hsla(),
                                                )
                                                .truncate(true),
                                            ),
                                    )
                                })
                                .child(div().flex_1())
                                .child(
                                    div().flex_shrink(10000.0).min_w(px(0.0)).child(
                                        Label::new(
                                            h.library_name.clone(),
                                            TextSource::Data,
                                            ty.caption,
                                            c.text_2.to_hsla(),
                                        )
                                        .truncate(true),
                                    ),
                                )
                                .into_any_element()
                        })
                        .collect();
                    if self.hits.is_empty() && self.loaded {
                        if has_libraries {
                            rows.push(
                                note(msg(cx, "hv-scripts-goto-none", Vec::new()))
                                    .into_any_element(),
                            );
                        } else {
                            rows.push(
                                note(msg(cx, "hv-scripts-goto-empty", Vec::new()))
                                    .into_any_element(),
                            );
                            rows.push(
                                div()
                                    .px(px(t.spacing.lg))
                                    .pb(px(t.spacing.sm))
                                    .child(Label::new(
                                        msg(cx, "hv-scripts-goto-empty-body", Vec::new()),
                                        TextSource::Fluent,
                                        ty.caption,
                                        c.text_2.to_hsla(),
                                    ))
                                    .into_any_element(),
                            );
                        }
                    }
                    let top = div()
                        .p(px(t.spacing.sm))
                        .child(self.input.clone())
                        .into_any_element();
                    let hints = vec![
                        ("up down", ui_text(cx, "hui-palette-hint-navigate")),
                        ("enter", msg(cx, "hv-scripts-goto-hint-open", Vec::new())),
                        ("escape", ui_text(cx, "hui-palette-hint-close")),
                    ];
                    (top, rows, hints, None)
                }
                Mode::Choose(ch) => {
                    let file = super::tabs::display_name(super::file_name(&ch.rel));
                    // ## m4b-t15-goto fix round 1 (review minor 7): the file name in mono, as the
                    // mock draws it: the message is split around its `$file` argument.
                    let (before, file_part, after) = if ch.rows.is_empty() {
                        (msg(cx, "hv-scripts-choose-none", Vec::new()), None, None)
                    } else {
                        header_parts(cx, file)
                    };
                    let sans = |text: SharedString| {
                        Label::new(text, TextSource::Fluent, ty.body_sm, c.text_2.to_hsla())
                            .truncate(true)
                    };
                    let top = div()
                        .flex()
                        .items_center()
                        .gap(px(t.spacing.sm))
                        .h(px(ROW_PX))
                        .px(px(t.spacing.lg))
                        .min_w(px(0.0))
                        .child(Icon::new(IconName::FileCode).size(IconSize::Chrome))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .flex_shrink(1.0)
                                .min_w(px(0.0))
                                .when(!before.trim().is_empty(), |el| {
                                    el.child(div().flex_none().child(sans(before.clone())))
                                })
                                .when_some(file_part, |el, f| {
                                    el.child(
                                        div().flex_shrink(1.0).min_w(px(0.0)).px(px(3.0)).child(
                                            Label::new(
                                                f,
                                                TextSource::Data,
                                                ty.mono_data,
                                                c.text.to_hsla(),
                                            )
                                            .truncate(true),
                                        ),
                                    )
                                })
                                .when_some(after.filter(|a| !a.trim().is_empty()), |el, a| {
                                    el.child(div().flex_none().child(sans(a)))
                                }),
                        )
                        .into_any_element();
                    let focused = ch.focused.as_ref().map(|(p, _)| *p);
                    let mut rows: Vec<AnyElement> = ch
                        .rows
                        .iter()
                        .enumerate()
                        .skip(ch.first)
                        .take(VISIBLE_ROWS)
                        .map(|(ix, cand)| {
                            let active = ch.active == Some(ix);
                            let name = super::tabs::display_name(&cand.name);
                            row(("scripts-choose-row", ix), active)
                                .debug_selector(move || format!("scripts-choose-row-{ix}"))
                                // ## m4b-t15-goto fix round 1 (review minor 2): with the
                                // environment word (the mark is the only other cue).
                                .aria_label(SharedString::from(format!(
                                    "{} {name} {}",
                                    crate::connections::capsule::env_word(cx, cand.env),
                                    super::tabs::display_name(cand.database.as_str())
                                )))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.click_row(ix, window, cx);
                                }))
                                .child(env_swatch_on_raised(
                                    crate::connections::capsule::env_kind(cand.env),
                                    &theme,
                                ))
                                .child(
                                    div().flex_shrink(1.0).min_w(px(0.0)).child(
                                        Label::new(
                                            name,
                                            TextSource::Data,
                                            ty.mono_data,
                                            c.text.to_hsla(),
                                        )
                                        .truncate(true),
                                    ),
                                )
                                .child(
                                    div().flex_shrink(100.0).min_w(px(0.0)).child(
                                        Label::new(
                                            super::tabs::display_name(cand.database.as_str()),
                                            TextSource::Data,
                                            ty.mono_micro,
                                            c.text_2.to_hsla(),
                                        )
                                        .truncate(true),
                                    ),
                                )
                                .when(focused == Some(cand.profile), |el| {
                                    el.child(
                                        div()
                                            .flex()
                                            .flex_none()
                                            .items_center()
                                            .h(px(16.0))
                                            .px(px(t.spacing.xs))
                                            .border_1()
                                            .border_color(c.seam_strong.to_hsla())
                                            .rounded(px(3.0))
                                            .child(Label::new(
                                                msg(cx, "hv-scripts-choose-current", Vec::new()),
                                                TextSource::Fluent,
                                                ty.caption,
                                                c.text_2.to_hsla(),
                                            )),
                                    )
                                })
                                .child(div().flex_1())
                                .child(div().flex_none().child(Label::new(
                                    cand.engine.display_name(),
                                    TextSource::Data,
                                    ty.mono_micro,
                                    c.text_2.to_hsla(),
                                )))
                                .into_any_element()
                        })
                        .collect();
                    if ch.rows.is_empty() {
                        let unbound = chooser::opens_unbound(ch.library, &ch.rel, cx);
                        rows.push(
                            row(("scripts-choose-unbound", 0), unbound)
                                .when(unbound, |el| {
                                    el.on_click(cx.listener(|this, _, window, cx| {
                                        this.choose(window, cx);
                                    }))
                                })
                                .child(Icon::new(IconName::Unplug).size(IconSize::Chrome))
                                .child(Label::new(
                                    msg(cx, "hv-scripts-choose-none-row", Vec::new()),
                                    TextSource::Fluent,
                                    ty.body,
                                    if unbound {
                                        c.text.to_hsla()
                                    } else {
                                        c.text_disabled.to_hsla()
                                    },
                                ))
                                .into_any_element(),
                        );
                        rows.push(
                            div()
                                .px(px(t.spacing.lg))
                                .pb(px(t.spacing.sm))
                                .child(Label::new(
                                    msg(cx, "hv-scripts-choose-none-body", Vec::new()),
                                    TextSource::Fluent,
                                    ty.caption,
                                    c.text_2.to_hsla(),
                                ))
                                .into_any_element(),
                        );
                    }
                    let back = if ch.back_query.is_some() {
                        "hv-scripts-choose-hint-back"
                    } else {
                        "hv-scripts-choose-hint-close"
                    };
                    let hints = vec![
                        ("up down", ui_text(cx, "hui-palette-hint-navigate")),
                        (
                            "enter",
                            msg(cx, "hv-scripts-choose-hint-choose", Vec::new()),
                        ),
                        ("escape", msg(cx, back, Vec::new())),
                    ];
                    let prod_elsewhere = ch
                        .rows
                        .iter()
                        .any(|r| r.is_production() && Some(r.profile) != focused);
                    let foot_note =
                        prod_elsewhere.then(|| msg(cx, "hv-scripts-choose-prod-note", Vec::new()));
                    (top, rows, hints, foot_note)
                }
            };

        let footer = div()
            .flex()
            .items_center()
            .gap(px(t.spacing.xl))
            .h(px(FOOTER_PX))
            .px(px(t.spacing.lg))
            .border_t_1()
            .border_color(c.seam.to_hsla())
            .overflow_hidden()
            .children(hints.into_iter().map(|(k, text)| hint(k, text)))
            .when_some(foot_note, |el, text| {
                el.child(
                    div().flex_shrink(1.0).min_w(px(0.0)).child(
                        Label::new(text, TextSource::Fluent, ty.caption, c.text_2.to_hsla())
                            .truncate(true),
                    ),
                )
            });

        let choosing = matches!(self.mode, Mode::Choose(_));
        let surface = popover_surface(&theme)
            .id("scripts-goto")
            .flex()
            .flex_col()
            .w_full()
            .overflow_hidden()
            .when(choosing, |el| el.track_focus(&self.chooser_focus))
            .on_key_down(cx.listener(Self::on_key_down))
            .child(top)
            .child(
                div().border_t_1().border_color(c.seam.to_hsla()).child(
                    div()
                        .id("scripts-goto-list")
                        .role(Role::ListBox)
                        .flex()
                        .flex_col()
                        .py(px(t.spacing.xs))
                        .children(rows),
                ),
            )
            .child(footer);
        let primary = self.primary(cx);

        div()
            .absolute()
            .top(px(t.spacing.titlebar_height + t.spacing.sm))
            .left_0()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("scripts-goto-layer")
                    .w(px(width.max(0.0)))
                    .on_mouse_down_out(cx.listener(|_, _, window, cx| close(window, cx)))
                    .child(focus_home(&self.home, primary, surface)),
            )
    }
}

/// The open Go-to-script host (one per app, as the command palette's).
#[derive(Default)]
struct OpenGoto(Option<WeakEntity<GotoHost>>);

impl Global for OpenGoto {}

/// Pushes the layer with `mode` (an open one switches to it), focused.
fn show(mode: Mode, window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    if ws.read(cx).layer_ids().contains(&GOTO_LAYER)
        && let Some(host) = open_host(cx)
    {
        host.update(cx, |h, cx| {
            if let Mode::Choose(_) = mode {
                h.mode = mode;
            } else if matches!(h.mode, Mode::Choose(_)) {
                // A second ⇧⌘O over the chooser goes back to the results.
                h.mode = Mode::Results;
                h.run_query(cx);
            }
            cx.notify();
        });
        let focus = host.read(cx).primary(cx);
        window.focus(&focus, cx);
        return;
    }
    let host = cx.new(|cx| GotoHost::new(mode, window, cx));
    let focus = host.read(cx).primary(cx);
    ws.update(cx, |ws, cx| {
        ws.push_layer(
            Layer {
                id: GOTO_LAYER,
                context: PALETTE_CONTEXT,
                view: host.clone().into(),
                restore_focus: None,
                modal: false,
            },
            window,
            cx,
        );
    });
    window.focus(&focus, cx);
    cx.set_global(OpenGoto(Some(host.downgrade())));
}

/// Opens the Go-to-script layer on its results (an open one just takes the focus).
pub fn open(window: &mut Window, cx: &mut App) {
    show(Mode::Results, window, cx);
}

/// Opens the chooser for file `rel` of `library` ("Open under another connection…"), without
/// the connection `exclude` (the tab's or the tree row's, every database of it, R-T15-1).
/// Escape closes it.
pub fn open_chooser(
    library: LibraryId,
    rel: RelPath,
    exclude: Option<ProfileId>,
    window: &mut Window,
    cx: &mut App,
) {
    let choose = GotoHost::choose_for(library, rel, exclude, cx);
    show(Mode::Choose(Box::new(choose)), window, cx);
}

/// The open Go-to-script layer's view, if any.
pub fn open_host(cx: &App) -> Option<Entity<GotoHost>> {
    cx.try_global::<OpenGoto>()?.0.as_ref()?.upgrade()
}

/// Removes the layer (the focus goes back to where it was when it is the top one).
// ## m4b-t15-goto fix round 1 (review minor 5): by its view, wherever it is in the stack, so a
// script never opens while Go to script stays open under another layer.
fn close(window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    let Some(host) = open_host(cx) else {
        return;
    };
    let view = host.entity_id();
    ws.update(cx, |ws, cx| ws.remove_layer(view, window, cx));
}

/// "Open {file} under" split around the file: the words before it, the file, the words after
/// it. A message whose argument cannot be found is drawn whole (`None` for the file).
fn header_parts(
    cx: &App,
    file: String,
) -> (SharedString, Option<SharedString>, Option<SharedString>) {
    const MARK: &str = "\u{E000}";
    let text = msg(
        cx,
        "hv-scripts-choose-title",
        vec![("file", ArgValue::Text(MARK.to_string()))],
    );
    match text.split_once(MARK) {
        Some((before, after)) => {
            // Fluent wraps an argument in isolation marks (FSI ... PDI).
            let before = before.trim_end_matches('\u{2068}');
            let after = after.trim_start_matches('\u{2069}');
            (
                SharedString::from(before.to_string()),
                Some(SharedString::from(file)),
                Some(SharedString::from(after.to_string())),
            )
        }
        None => (
            msg(
                cx,
                "hv-scripts-choose-title",
                vec![("file", ArgValue::Text(file))],
            ),
            None,
            None,
        ),
    }
}

/// `scripts.goto`'s run and enabled functions (`super::install`).
pub(crate) fn filled(id: &str) -> Option<Filled> {
    (id == "scripts.goto").then(|| {
        let run: ActionFn = Rc::new(open);
        (run, (|_| true) as fn(&App) -> bool)
    })
}
