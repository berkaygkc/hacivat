//! The connection dialog's "Schemas" section (Task 20; design/m4-explorer surface 7): the scope
//! as a checkbox tree, the pattern field with its error at a character position, the
//! all-databases question (PostgreSQL) and "Encrypt cached metadata".
//!
//! ## m4-t20-scope
//!
//! **State.** The section keeps its own state in a [`SchemasSection`] entity per dialog, made
//! the first time the section draws (or by [`edit_scope`]) and dropped with the dialog, as the
//! SSH section does. The dialog's draft stays the one source of the profile: "Encrypt cached
//! metadata" is `draft.profile.catalog_sensitive_set`. The scope pattern is not a profile field: it
//! is `ProfileUserData.scope` in the profile's data file, written through
//! [`crate::explorer::userdata`] when the dialog saves.
//!
//! **The pattern and the tree are one scope** (HV-INTRO-02, mock 7). The pattern is parsed with
//! `hacivat_introspect::scope::parse_scope` on every change; the tree shows what the parsed
//! pattern selects among the databases and schemas the profile's last catalog lists
//! (`ScopePattern::select_explicit`, so the server's matching rules, never a second
//! implementation). While the list shows the whole pattern, a click in the tree writes it
//! again from the checked rows (`@` for the current database, `*` for a whole database, names
//! quoted where a bare name would not select exactly that name). Otherwise a click adds or
//! removes one clause for its row and keeps every term the list cannot show; a click that
//! cannot be made that way asks first. What the user types is never rewritten: the tree
//! follows it. With no catalog (not connected, never read) the tree's place says so and the
//! pattern alone decides.
//!
//! **Errors.** A pattern that does not parse keeps its text; the field's border turns `text-2`,
//! an ink line names the problem and the position in characters (`ScopeError.column`, Unicode
//! scalar values, never bytes), and Save / Save and connect are disabled ([`blocks_save`]): a
//! pattern that does not parse is never saved. Never red (Hue Is Meaning).
//!
//! **All databases (PostgreSQL).** Reading another database opens one connection per database
//! (alg/02 HV-INTRO-02 edge 10): the first change that reaches a database other than the
//! current one (the All databases box, a box in the tree, a typed pattern) asks in place first,
//! focused on "Keep the current database". The question is asked once per dialog: after "Read
//! all", further changes do not ask again. Keep restores the pattern as it was. While the
//! question is open the dialog does not save.
//!
//! **Encrypt cached metadata** (R-M4-15, ruling R-W3-1). The box shows the profile's effective
//! sensitivity (`Profile::catalog_sensitive`): as the user set it, else derived from the
//! environment (on for production). A click writes an explicit `Some(true)` / `Some(false)`; a
//! new profile nobody clicked for stays derived. On a saved profile a change of the effective
//! value deletes the cache, so it asks first in place (the safe answer is focused): after a
//! click on the box, and after an environment change that flips a derived value (Keep pins the
//! old value explicitly; Save waits for the answer, [`environment_changed`]).
//!
//! **Save** ([`save_profile`]) reads the stored profile then. Unless the user chose in this
//! dialog, the stored choice stands (a profile changed meanwhile is not overwritten). When the
//! stored and the new effective values differ: 1. the profile's cache files are deleted, 2. the
//! scope is written to the data file, 3. the profile is saved through `ConnectionsModel`. A
//! step that fails stops the save there with its own message ([`ScopeSaveError`]), so the flag
//! never says "encrypted" while plaintext files are on disk, nor "plaintext" while an encrypted
//! cache is (Task 11 refuses that downgrade).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use hacivat_introspect::filter::FilterWarningKind;
use hacivat_introspect::scope::{
    CurrentCtx, NsInfo, ScopeError, ScopeErrorKind, ScopePattern, ScopeWarningKind, parse_scope,
};
use hacivat_l10n::{ArgValue, NoArgs, ids};
use hacivat_meta::name::DialectId;
use hacivat_meta::persist::CacheStore;
use hacivat_meta::snapshot::ContainerKey;
use hacivat_meta::{Name, NsKey, NsKind, ObjFlags};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_session::profile::{Profile, ProfileDraft};
use hacivat_ui::button::Button;
use hacivat_ui::checkbox::Checkbox;
use hacivat_ui::disclosure::Disclosure;
use hacivat_ui::form::{form_row, hint_row};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::notice::{InlineNotice, NoticeIcon};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::text::{Label, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Div, Entity, EntityId, FocusHandle, Focusable as _,
    Global, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement, Role, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Task, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use tokio::sync::oneshot;

use super::{ConnectionDialog, DialogFocus, open_dialog, open_edit};
use crate::connections::ConnectionsModel;
use crate::connections::model::SaveError;
use crate::explorer::adapter;
use crate::explorer::rows::NameOrder;
use crate::explorer::userdata;
use crate::l10n_global::{L10nGlobal, tr, ui_language};
use crate::shell::AppState;

/// The tree's rows (the mock's `.scr`: 22 px).
const ROW_PX: f32 = 22.0;
/// The tree's well grows to this height, then scrolls (the mock's 178 px).
const TREE_MAX_PX: f32 = 178.0;
/// Indents of the tree's rows: a database (or a one-level schema), a schema below it.
const DB_INDENT_PX: f32 = 6.0;
const SCHEMA_INDENT_PX: f32 = 44.0;
/// A schema name no server lists (NUL is refused in every engine's identifiers): it stands for
/// "every schema, also those not listed yet", so `db:*` and `db:{…}` are told from a list of
/// the names shown.
const SENTINEL: &str = "\u{0}";

// ---- the state ----------------------------------------------------------------------------------

/// A checkbox's state in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tri {
    Unchecked,
    Mixed,
    Checked,
}

/// What the profile's last catalog lists: databases with their schemas (PostgreSQL), or the
/// schemas of a one-level engine (a MySQL/MariaDB database is a schema; SQLite's `main`,
/// `temp` and attached files). System schemas are never listed (they come from the bundled
/// catalog).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeList {
    /// Databases hold schemas (PostgreSQL).
    pub two_level: bool,
    /// Two-level: the databases, the current one first. One-level: one entry with an empty name
    /// holding the schemas.
    pub databases: Vec<ScopeDb>,
}

/// One database of the list and its schemas, in display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeDb {
    pub name: Name,
    /// The profile's database (`@`).
    pub current: bool,
    pub schemas: Vec<Name>,
}

impl ScopeList {
    fn schema_count(&self) -> usize {
        self.databases.iter().map(|d| d.schemas.len()).sum()
    }
}

/// What a pattern selects among the listed names.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Selection {
    dbs: Vec<DbSel>,
    /// Every database (also unlisted ones): `*:*`.
    all_databases: bool,
}

/// What a pattern selects in one listed database.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DbSel {
    /// The database is read at all (PostgreSQL: one of its schemas, or a database-only clause
    /// such as `billing`, which means `billing:@`).
    db: bool,
    /// Every schema of it, also those not listed yet (`db:*`).
    all: bool,
    /// Each listed schema.
    schemas: Vec<bool>,
}

/// A cached selection: (pattern text, list version, database) and what it selects.
type SelCache = (String, u64, Option<String>, Selection);

/// A row of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Db(usize),
    Schema(usize, usize),
}

/// The pending all-databases question: the pattern "Read all" applies (`None`: the typed text
/// stays) and the one "Keep the current database" restores.
struct CostAsk {
    apply: Option<String>,
    keep: String,
}

/// The section's focus targets that are not text fields.
struct ScopeFocus {
    /// The whole section (not a tab stop).
    root: FocusHandle,
    all: FocusHandle,
    tree: FocusHandle,
    cost_keep: FocusHandle,
    cost_yes: FocusHandle,
    encrypt: FocusHandle,
    encrypt_keep: FocusHandle,
    encrypt_apply: FocusHandle,
    rewrite_keep: FocusHandle,
    rewrite_apply: FocusHandle,
}

/// What a new section starts from: the dialog's draft as it was opened.
struct Seed {
    profile: ProfileId,
    engine: DbmsKind,
    is_new: bool,
}

/// The Schemas section of one connection dialog.
pub struct SchemasSection {
    dialog: WeakEntity<ConnectionDialog>,
    profile: ProfileId,
    is_new: bool,
    engine: DbmsKind,
    /// The profile's database (`@` on PostgreSQL), as the draft has it.
    database: Option<String>,
    open: bool,
    pattern: Entity<TextInput>,
    parsed: Result<ScopePattern, ScopeError>,
    /// The last pattern that parsed (the tree shows it while the field holds an error).
    good: ScopePattern,
    /// The scope in the profile's data file (`None`: the default), once read.
    stored: Option<Option<String>>,
    /// The pattern was changed in this dialog (typed, or through the tree).
    touched: bool,
    list: Option<ScopeList>,
    list_generation: Option<u64>,
    expanded: HashSet<Name>,
    cursor: usize,
    /// "Read all" was answered in this dialog.
    cost_confirmed: bool,
    cost: Option<CostAsk>,
    /// The pattern text no question is pending for.
    confirmed_text: String,
    /// The encryption question, with the value it would set.
    encrypt_ask: Option<EncryptAsk>,
    // ## fix round 1
    /// The rewrite question (I-3), with the pattern "Write it again" applies.
    rewrite: Option<String>,
    /// `connections.toml` cannot be written: the section is read-only (M-5).
    read_only: bool,
    /// The saved scope could not be read (M-4).
    stored_unreadable: bool,
    /// Moves when `list` changes (the selection cache's key, M-6).
    list_version: u64,
    /// What the last pattern that parsed selects in the list, for (pattern, list, database).
    sel_cache: RefCell<Option<SelCache>>,
    focus: ScopeFocus,
    _subscriptions: Vec<Subscription>,
}

/// One dialog's section, and whether its "Encrypt cached metadata" was chosen in this dialog
/// (read while the dialog itself is being updated, so it is not inside the entity).
struct SectionRef {
    state: Entity<SchemasSection>,
    sensitive_chosen: Rc<Cell<bool>>,
}

/// The encryption question in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptAsk {
    /// The box was clicked: Apply sets this value.
    Toggle(bool),
    /// The environment changed a derived value to this one: Keep pins the old one.
    Environment(bool),
}

impl EncryptAsk {
    /// The value the change leads to.
    pub fn to(self) -> bool {
        match self {
            EncryptAsk::Toggle(v) | EncryptAsk::Environment(v) => v,
        }
    }
}

/// Why the section's part of a Save stopped it (nothing was saved).
#[derive(Debug)]
pub enum ScopeSaveError {
    /// The profile's cache files could not be deleted.
    CacheNotDeleted,
    /// The profile's data file cannot be read, so the scope cannot be written.
    DataUnreadable,
    /// The scope could not be written.
    DataNotWritten,
    /// `ConnectionsModel::save` failed.
    Profile(SaveError),
}

// ## m4-t20-scope (fix round 1, I-4): a deterministic fault per Save step, for the tests.
/// A Save step made to fail (feature `test-support`).
#[cfg(feature = "test-support")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFault {
    /// Before any file is deleted.
    CacheDelete,
    /// After the cache files were deleted.
    ScopeWrite,
    /// After the scope was written.
    ProfileSave,
}

#[cfg(feature = "test-support")]
#[derive(Default)]
struct Faults(Option<SaveFault>);

#[cfg(feature = "test-support")]
impl Global for Faults {}

/// Makes the next Saves fail at `fault` (`None`: none).
#[cfg(feature = "test-support")]
pub fn inject_save_fault(cx: &mut App, fault: Option<SaveFault>) {
    cx.default_global::<Faults>().0 = fault;
}

#[cfg(feature = "test-support")]
fn faults(cx: &App) -> [bool; 3] {
    let f = cx.try_global::<Faults>().and_then(|f| f.0);
    [
        f == Some(SaveFault::CacheDelete),
        f == Some(SaveFault::ScopeWrite),
        f == Some(SaveFault::ProfileSave),
    ]
}

#[cfg(not(feature = "test-support"))]
fn faults(_: &App) -> [bool; 3] {
    [false; 3]
}

/// The Schemas section of each open dialog, by the dialog's entity ID.
#[derive(Default)]
struct Sections(HashMap<EntityId, SectionRef>);

impl Global for Sections {}

// ---- entry points ---------------------------------------------------------------------------------

/// The Schemas section of the form for `draft` (the dialog's hook calls it while drawing).
pub(crate) fn section(draft: &ProfileDraft, window: &mut Window, cx: &mut App) -> AnyElement {
    let Some(dialog) = open_dialog(window, cx) else {
        return div().into_any_element();
    };
    let seed = Seed {
        profile: draft.profile.id,
        engine: draft.profile.engine,
        is_new: draft.is_new,
    };
    let state = state_for(&dialog, seed, window, cx);
    state.update(cx, |s, cx| s.sync(&draft.profile, cx));
    render(&state, draft, window, cx)
}

/// The Schemas section of the dialog open in `window` (tests and the dialog's own reads).
pub fn section_state(window: &Window, cx: &App) -> Option<Entity<SchemasSection>> {
    let dialog = open_dialog(window, cx)?;
    cx.try_global::<Sections>()?
        .0
        .get(&dialog.entity_id())
        .map(|r| r.state.clone())
}

/// Whether the focus is inside the Schemas section of the dialog open in `window` (the dialog
/// names it `scope`).
pub(crate) fn contains_focus(window: &Window, cx: &App) -> bool {
    section_state(window, cx).is_some_and(|s| s.read(cx).focus.root.contains_focused(window, cx))
}

/// Whether the dialog `dialog` must not save now: its pattern does not parse, the
/// all-databases question waits for an answer, or so does an environment change that would
/// delete the cache.
pub(crate) fn blocks_save(dialog: EntityId, cx: &App) -> bool {
    cx.try_global::<Sections>()
        .and_then(|s| s.0.get(&dialog))
        .is_some_and(|r| {
            let s = r.state.read(cx);
            s.parsed.is_err()
                || s.cost.is_some()
                || matches!(s.encrypt_ask, Some(EncryptAsk::Environment(_)))
        })
}

fn sensitive_chosen(dialog: EntityId, cx: &App) -> bool {
    cx.try_global::<Sections>()
        .and_then(|s| s.0.get(&dialog))
        .is_some_and(|r| r.sensitive_chosen.get())
}

/// After an edit of a saved profile: an environment change that flips its derived
/// sensitivity (R-W3-1) would delete the cache at Save, so it asks first (and Save waits);
/// changed back, the question goes. A value set explicitly, here or before, does not follow
/// the environment.
pub(crate) fn environment_changed(
    dialog: &ConnectionDialog,
    id: EntityId,
    window: &mut Window,
    cx: &mut App,
) {
    if dialog.is_new || dialog.draft.profile.catalog_sensitive_set.is_some() {
        return;
    }
    let Some(stored) = cx
        .try_global::<ConnectionsModel>()
        .and_then(|m| m.get(dialog.draft.profile.id))
    else {
        return;
    };
    let Some(state) = cx
        .try_global::<Sections>()
        .and_then(|s| s.0.get(&id))
        .map(|r| r.state.clone())
    else {
        return;
    };
    let before = stored.catalog_sensitive();
    let now = dialog.draft.profile.catalog_sensitive();
    state.update(cx, |s, cx| {
        if now != before {
            s.encrypt_ask = Some(EncryptAsk::Environment(now));
            window.focus(&s.focus.encrypt_keep, cx);
        } else if matches!(s.encrypt_ask, Some(EncryptAsk::Environment(_))) {
            s.encrypt_ask = None;
        }
        cx.notify();
    });
}

/// Where the profile caches live (`LocalPaths::catalog_db`'s folder).
fn cache_dir(cx: &App, profile: ProfileId) -> Option<PathBuf> {
    let paths = &cx.try_global::<AppState>()?.paths;
    paths
        .catalog_db(&profile.0.to_string(), 1)
        .parent()
        .map(PathBuf::from)
}

/// Saves `draft` for the dialog through `ConnectionsModel` (the one writer of profiles), with
/// the section's parts (see the module docs, "Save"). A step that fails stops the save there.
pub(crate) fn save_profile(
    mut draft: ProfileDraft,
    cx: &mut Context<ConnectionDialog>,
) -> Task<Result<ProfileId, ScopeSaveError>> {
    let me = cx.entity_id();
    let id = draft.profile.id;
    let Some(model) = cx.try_global::<ConnectionsModel>() else {
        return Task::ready(Err(ScopeSaveError::Profile(SaveError::Io(
            "no connections".to_string(),
        ))));
    };
    let runtime = model.runtime();
    // Read at Save (fix round 1, M-8): unless the user chose in this dialog, the stored choice
    // stands, even when the profile changed while the dialog was open.
    let stored = model.get(id);
    if !sensitive_chosen(me, cx) {
        draft.profile.catalog_sensitive_set = stored.as_ref().and_then(|s| s.catalog_sensitive_set);
    }
    let drop_cache = stored
        .as_ref()
        .is_some_and(|s| s.catalog_sensitive() != draft.profile.catalog_sensitive());
    let dir = cache_dir(cx, id);
    let scope = cx
        .try_global::<Sections>()
        .and_then(|s| s.0.get(&me))
        .and_then(|r| r.state.read(cx).scope_to_save());
    let [fail_delete, fail_scope, fail_save] = faults(cx);
    cx.spawn(async move |_, cx| {
        if drop_cache {
            let dir = dir.filter(|_| !fail_delete);
            let Some(dir) = dir else {
                return Err(ScopeSaveError::CacheNotDeleted);
            };
            // ## m4-t24-live (named lines; fix round 1, I-1): the profile's open cache store
            // closes first (its engine stops; T19 carry), and nothing reopens it until the
            // `reopen` below, which every path from here reaches.
            let release = cx.update(|cx| crate::explorer::live::release_cache(id, cx));
            release.await;
            let (tx, rx) = oneshot::channel();
            runtime.spawn_blocking(move || {
                let _ = tx.send(CacheStore::delete_profile_files(&dir, &id));
            });
            if !matches!(rx.await, Ok(Ok(()))) {
                // Nothing changed: the cache opens again as it was.
                cx.update(|cx| crate::explorer::live::reopen(id, false, cx));
                return Err(ScopeSaveError::CacheNotDeleted);
            }
        }
        let saved = async {
            if fail_scope {
                return Err(ScopeSaveError::DataNotWritten);
            }
            if let Some(scope) = scope {
                let write = cx.update(|cx| userdata::set_scope(id, scope, cx));
                match write.await {
                    Ok(_) => {}
                    Err(userdata::UserDataError::Unreadable) => {
                        return Err(ScopeSaveError::DataUnreadable);
                    }
                    Err(_) => return Err(ScopeSaveError::DataNotWritten),
                }
            }
            if fail_save {
                return Err(ScopeSaveError::Profile(SaveError::Io(
                    "injected".to_string(),
                )));
            }
            let save = cx.update(|cx| ConnectionsModel::save(cx, draft));
            save.await.map_err(ScopeSaveError::Profile)
        }
        .await;
        // ## m4-t24-live (named lines): the catalog opens again (when connected) with the
        // profile as stored now, so an "encrypt on" save never recreates a plaintext cache.
        if drop_cache {
            cx.update(|cx| crate::explorer::live::reopen(id, true, cx));
        }
        saved
    })
}

/// "Edit scope…" (the explorer's command): the edit dialog of the cursor row's profile, with
/// the Schemas section open and the pattern focused.
pub fn edit_scope(window: &mut Window, cx: &mut App) {
    let Some(id) = cursor_profile(cx) else {
        return;
    };
    open_edit(id, DialogFocus::Default, window, cx);
    let Some(dialog) = open_dialog(window, cx) else {
        return;
    };
    // Another dialog was already open (the seam then only focuses it): leave it alone.
    if dialog.read(cx).draft.profile.id != id {
        return;
    }
    let draft = &dialog.read(cx).draft;
    let seed = Seed {
        profile: draft.profile.id,
        engine: draft.profile.engine,
        is_new: draft.is_new,
    };
    let state = state_for(&dialog, seed, window, cx);
    state.update(cx, |s, cx| {
        s.open = true;
        let handle = s.pattern.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    });
}

/// Whether "Edit scope…" can run: the tree's cursor row belongs to a profile.
pub fn can_edit_scope(cx: &App) -> bool {
    cursor_profile(cx).is_some()
}

fn cursor_profile(cx: &App) -> Option<ProfileId> {
    let tree = crate::connections::explorer::tree(cx)?;
    let tree = tree.read(cx);
    let cursor = tree.cursor()?;
    tree.model().profile_of(cursor)
}

/// The Fluent message of a filter parse warning (the view filter, Task 24).
pub fn filter_warning_message(kind: FilterWarningKind) -> &'static str {
    match kind {
        FilterWarningKind::EmptyItem => "intro-filter-warn-empty-item",
    }
}

/// The section of `dialog`, made from `seed` the first time.
fn state_for(
    dialog: &Entity<ConnectionDialog>,
    seed: Seed,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SchemasSection> {
    let id = dialog.entity_id();
    if let Some(r) = cx.try_global::<Sections>().and_then(|s| s.0.get(&id)) {
        return r.state.clone();
    }
    let state = cx.new(|cx| SchemasSection::new(dialog, seed, window, cx));
    cx.default_global::<Sections>().0.insert(
        id,
        SectionRef {
            state: state.clone(),
            sensitive_chosen: Rc::default(),
        },
    );
    cx.observe_release(dialog, move |_, cx| {
        cx.default_global::<Sections>().0.remove(&id);
    })
    .detach();
    state
}

// ---- patterns -----------------------------------------------------------------------------------

/// The dialect an engine's patterns parse in.
fn dialect_of(engine: DbmsKind) -> DialectId {
    match engine {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        DbmsKind::Sqlite => DialectId::Sqlite,
        _ => DialectId::Postgres,
    }
}

fn two_level(engine: DbmsKind) -> bool {
    dialect_of(engine) == DialectId::Postgres
}

/// The scope of a profile that has none saved (R-M4-14): PostgreSQL every schema of the
/// connected database, MySQL/MariaDB every database, SQLite every schema.
pub fn default_pattern(engine: DbmsKind) -> &'static str {
    if two_level(engine) { "@:*" } else { "*" }
}

fn parse(text: &str, engine: DbmsKind) -> Result<ScopePattern, ScopeError> {
    parse_scope(text, dialect_of(engine))
}

/// The default pattern, parsed (it always parses).
fn default_parsed(engine: DbmsKind) -> ScopePattern {
    parse(default_pattern(engine), engine)
        .or_else(|_| parse("", engine))
        .unwrap_or_else(|_| unreachable!("the empty pattern parses"))
}

/// `@` for the session's database and schema: the profile's database (MySQL/MariaDB: also the
/// schema; SQLite: `main`).
fn current_ctx(engine: DbmsKind, database: Option<&str>) -> CurrentCtx {
    let db = database.filter(|d| !d.is_empty()).map(Name::from);
    match engine {
        DbmsKind::Sqlite => CurrentCtx {
            database: None,
            schema: Some(Name::from("main")),
            ident: None,
        },
        e if two_level(e) => CurrentCtx {
            database: db,
            schema: None,
            ident: None,
        },
        _ => CurrentCtx {
            database: db.clone(),
            schema: db,
            ident: None,
        },
    }
}

/// A name as a pattern term: bare when that selects exactly it (no separator, no space, no
/// ASCII capital a server would fold), else quoted with `""`.
fn term(name: &str) -> String {
    let bare = !name.is_empty()
        && name.chars().all(|c| {
            !c.is_whitespace() && !matches!(c, ':' | ',' | '|' | '{' | '}' | '!' | '"' | '@' | '*')
        })
        && !name.bytes().any(|b| b.is_ascii_uppercase());
    if bare {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

/// The `|`-separated clauses of a pattern's canonical text, as written: a `|` inside a quoted
/// name or a regex does not separate (`\` escapes the next character in a regex).
fn clause_texts(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut depth, mut escaped) = (false, 0usize, false);
    for c in text.chars() {
        if escaped {
            escaped = false;
        } else if quoted {
            quoted = c != '"';
        } else if depth > 0 {
            match c {
                '\\' => escaped = true,
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        } else {
            match c {
                '"' => quoted = true,
                '{' => depth = 1,
                '|' => {
                    out.push(std::mem::take(&mut cur));
                    continue;
                }
                _ => {}
            }
        }
        cur.push(c);
    }
    if !cur.is_empty() || !out.is_empty() {
        out.push(cur);
    }
    out
}

/// What `pattern` selects among `list` (the server's matching rules, through
/// `select_explicit`).
fn evaluate(pattern: &ScopePattern, list: &ScopeList, ctx: &CurrentCtx) -> Selection {
    let schema_key = |db: &ScopeDb, s: &str| {
        if list.two_level {
            NsKey::schema([db.name.as_ref(), s])
        } else {
            NsKey::schema([s])
        }
    };
    let mut infos = Vec::new();
    for db in &list.databases {
        if list.two_level {
            infos.push(NsInfo {
                key: NsKey::database(db.name.clone()),
                system: false,
            });
        }
        infos.push(NsInfo {
            key: schema_key(db, SENTINEL),
            system: false,
        });
        for s in &db.schemas {
            infos.push(NsInfo {
                key: schema_key(db, s),
                system: false,
            });
        }
    }
    let every = NsKey::schema([SENTINEL, SENTINEL]);
    if list.two_level {
        infos.push(NsInfo {
            key: every.clone(),
            system: false,
        });
    }
    let chosen: BTreeSet<NsKey> = pattern.select_explicit(&infos, ctx);
    let dbs = list
        .databases
        .iter()
        .map(|db| {
            let schemas: Vec<bool> = db
                .schemas
                .iter()
                .map(|s| chosen.contains(&schema_key(db, s)))
                .collect();
            let all = chosen.contains(&schema_key(db, SENTINEL));
            DbSel {
                db: all
                    || schemas.iter().any(|b| *b)
                    || (list.two_level && chosen.contains(&NsKey::database(db.name.clone()))),
                all,
                schemas,
            }
        })
        .collect();
    Selection {
        dbs,
        all_databases: list.two_level && chosen.contains(&every),
    }
}

/// A database's box: its listed schemas (or, with none listed, the database as a whole); a
/// database read without any listed schema of it (`billing`, that is `billing:@`) is mixed.
fn db_state(sel: &DbSel) -> Tri {
    let on = sel.schemas.iter().filter(|b| **b).count();
    if (sel.schemas.is_empty() && sel.all) || (on > 0 && on == sel.schemas.len()) {
        Tri::Checked
    } else if sel.db {
        Tri::Mixed
    } else {
        Tri::Unchecked
    }
}

/// The pattern for the checked rows: `*:*` for every database; per database `@:*` / `db:*`
/// when it is checked whole, `@:a,b` when some of its schemas are; one-level `*` or `a,b`.
fn generate(sel: &Selection, list: &ScopeList) -> String {
    if !list.two_level {
        let Some(DbSel { schemas, .. }) = sel.dbs.first() else {
            return String::new();
        };
        let db = &list.databases[0];
        if !schemas.is_empty() && schemas.iter().all(|b| *b) {
            return "*".to_string();
        }
        return db
            .schemas
            .iter()
            .zip(schemas)
            .filter(|(_, on)| **on)
            .map(|(s, _)| term(s))
            .collect::<Vec<_>>()
            .join(",");
    }
    if sel.all_databases {
        return "*:*".to_string();
    }
    let mut clauses = Vec::new();
    for (db, s) in list.databases.iter().zip(&sel.dbs) {
        let name = if db.current {
            "@".to_string()
        } else {
            term(&db.name)
        };
        match db_state(s) {
            Tri::Checked => clauses.push(format!("{name}:*")),
            Tri::Mixed => {
                let names: Vec<String> = db
                    .schemas
                    .iter()
                    .zip(&s.schemas)
                    .filter(|(_, on)| **on)
                    .map(|(n, _)| term(n))
                    .collect();
                if names.is_empty() {
                    // Read without a listed schema: the database-only clause (`db:@`).
                    clauses.push(name);
                } else {
                    clauses.push(format!("{name}:{}", names.join(",")));
                }
            }
            Tri::Unchecked => {}
        }
    }
    clauses.join("|")
}

/// Whether `sel` reads a database other than the current one (PostgreSQL).
fn reaches_other_databases(sel: &Selection, list: &ScopeList) -> bool {
    list.two_level
        && (sel.all_databases
            || list
                .databases
                .iter()
                .zip(&sel.dbs)
                .any(|(db, s)| !db.current && s.db))
}

/// The databases and schemas `profile`'s last catalog lists (`None`: none read).
fn list_of(
    profile: ProfileId,
    engine: DbmsKind,
    database: Option<&str>,
    cx: &App,
) -> Option<ScopeList> {
    let cat = adapter::catalog_of(profile, cx).filter(|c| c.generation > 0)?;
    let mut keys: BTreeSet<NsKey> = cat
        .namespaces
        .keys()
        .filter(|k| k.kind != NsKind::Server)
        .cloned()
        .collect();
    keys.extend(cat.states.keys().filter_map(|k| match k {
        ContainerKey::Namespace(k) if k.kind != NsKind::Server => Some(k.clone()),
        _ => None,
    }));
    let system = |k: &NsKey| {
        cat.namespaces.get(k).is_some_and(|ns| {
            ns.root()
                .flags()
                .intersects(ObjFlags::SYSTEM | ObjFlags::PRE_INTROSPECTED)
        })
    };
    let lang = ui_language(cx).map(|l| l.to_string()).unwrap_or_default();
    let order = NameOrder::new(&lang);
    let two = two_level(engine);
    let mut dbs: Vec<ScopeDb> = Vec::new();
    let mut add = |db: &Name, schema: Option<&Name>| {
        let i = match dbs.iter().position(|d| d.name == *db) {
            Some(i) => i,
            None => {
                dbs.push(ScopeDb {
                    name: db.clone(),
                    current: two && database == Some(db.as_ref()),
                    schemas: Vec::new(),
                });
                dbs.len() - 1
            }
        };
        if let Some(s) = schema
            && !dbs[i].schemas.contains(s)
        {
            dbs[i].schemas.push(s.clone());
        }
    };
    if two && let Some(db) = database.filter(|d| !d.is_empty()) {
        add(&Name::from(db), None);
    }
    for k in &keys {
        if system(k) {
            continue;
        }
        match (two, k.kind, k.path.as_slice()) {
            (true, NsKind::Database, [db]) => add(db, None),
            (true, NsKind::Schema, [db, s]) => add(db, Some(s)),
            (false, NsKind::Schema, [.., s]) => add(&Name::from(""), Some(s)),
            _ => {}
        }
    }
    for db in &mut dbs {
        db.schemas.sort_by(|a, b| order.cmp(a, b));
    }
    dbs.sort_by(|a, b| {
        b.current
            .cmp(&a.current)
            .then_with(|| order.cmp(&a.name, &b.name))
    });
    let list = ScopeList {
        two_level: two,
        databases: dbs,
    };
    (list.schema_count() > 0 || list.databases.len() > 1).then_some(list)
}

// ---- the section ----------------------------------------------------------------------------------

impl SchemasSection {
    fn new(
        dialog: &Entity<ConnectionDialog>,
        seed: Seed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pattern = cx.new(|cx| {
            let mut input = TextInput::new(cx, Rc::new(|_: &App| SharedString::default()));
            input.set_placeholder_source(TextSource::Data);
            input.set_mono(true);
            input
        });
        let _ = pattern.read(cx).focus_handle(cx).tab_stop(true);
        let weak = dialog.downgrade();
        let mut subscriptions = vec![cx.subscribe_in(
            &pattern,
            window,
            move |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Changed => this.typed(window, cx),
                // Escape in the field cancels the dialog without saving (P16).
                InputEvent::Cancelled => {
                    if let Some(dialog) = weak.upgrade() {
                        dialog.update(cx, |d, cx| d.escape(window, cx));
                    }
                }
                _ => {}
            },
        )];
        // An edit shows the saved scope once the profile's data file has been read.
        subscriptions.push(cx.observe_global::<userdata::UserData>(|this, cx| {
            this.stored_arrived(cx);
        }));
        let stop = |cx: &mut Context<Self>| cx.focus_handle().tab_stop(true);
        let focus = ScopeFocus {
            root: cx.focus_handle(),
            all: stop(cx),
            tree: stop(cx),
            cost_keep: stop(cx),
            cost_yes: stop(cx),
            encrypt: stop(cx),
            encrypt_keep: stop(cx),
            encrypt_apply: stop(cx),
            rewrite_keep: stop(cx),
            rewrite_apply: stop(cx),
        };
        let good = default_parsed(seed.engine);
        let mut s = SchemasSection {
            dialog: dialog.downgrade(),
            profile: seed.profile,
            is_new: seed.is_new,
            engine: seed.engine,
            database: None,
            open: false,
            pattern,
            parsed: Ok(good.clone()),
            good,
            stored: seed.is_new.then_some(None),
            touched: false,
            list: None,
            list_generation: None,
            expanded: HashSet::new(),
            cursor: 0,
            cost_confirmed: false,
            cost: None,
            confirmed_text: String::new(),
            encrypt_ask: None,
            rewrite: None,
            read_only: false,
            stored_unreadable: false,
            list_version: 0,
            sel_cache: RefCell::new(None),
            focus,
            _subscriptions: subscriptions,
        };
        if !seed.is_new {
            userdata::ensure(seed.profile, cx);
            if let Some(data) = userdata::get(seed.profile, cx) {
                s.stored = Some(data.scope.clone());
            }
            s.stored_unreadable = userdata::is_unreadable(seed.profile, cx);
        }
        let text = s
            .stored
            .clone()
            .flatten()
            .unwrap_or_else(|| default_pattern(seed.engine).to_string());
        s.set_text(&text, cx);
        s.confirmed_text = text;
        s
    }

    // ---- reads (tests and the dialog) ----

    /// The disclosure is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The pattern field.
    pub fn pattern_input(&self) -> Entity<TextInput> {
        self.pattern.clone()
    }

    /// The pattern as typed.
    pub fn pattern_text(&self, cx: &App) -> String {
        self.pattern.read(cx).value().to_string()
    }

    /// Why the pattern does not parse (`None`: it parses).
    pub fn error(&self) -> Option<&ScopeError> {
        self.parsed.as_ref().err()
    }

    /// The tree's rows: the name (a schema's indented by its database) and its box.
    pub fn rows(&self) -> Vec<(String, Tri)> {
        let Some(list) = &self.list else {
            return Vec::new();
        };
        let sel = self.selection(list);
        self.visible_rows(list)
            .into_iter()
            .map(|r| match r {
                Row::Db(i) => (list.databases[i].name.to_string(), db_state(&sel.dbs[i])),
                Row::Schema(i, j) => (
                    format!(
                        "{}.{}",
                        list.databases[i].name, list.databases[i].schemas[j]
                    ),
                    if sel.dbs[i].schemas[j] {
                        Tri::Checked
                    } else {
                        Tri::Unchecked
                    },
                ),
            })
            .collect()
    }

    /// The listed databases and schemas (`None`: nothing read yet).
    pub fn list(&self) -> Option<&ScopeList> {
        self.list.as_ref()
    }

    /// Whether the all-databases question is open.
    pub fn cost_question_open(&self) -> bool {
        self.cost.is_some()
    }

    /// The encryption question, with the value it would set.
    pub fn encrypt_question(&self) -> Option<EncryptAsk> {
        self.encrypt_ask
    }

    /// Whether "All databases" is checked.
    pub fn all_databases(&self) -> bool {
        self.list
            .as_ref()
            .is_some_and(|l| self.selection(l).all_databases)
    }

    /// What the last pattern that parsed selects in `list` (the shown list), computed once per
    /// pattern, list and database (fix round 1, M-6: the section draws every frame).
    fn selection(&self, list: &ScopeList) -> Selection {
        let text = self.good.to_text();
        if let Some((t, v, d, sel)) = &*self.sel_cache.borrow()
            && t == text
            && *v == self.list_version
            && *d == self.database
        {
            return sel.clone();
        }
        let ctx = current_ctx(self.engine, self.database.as_deref());
        let sel = evaluate(&self.good, list, &ctx);
        *self.sel_cache.borrow_mut() = Some((
            text.to_string(),
            self.list_version,
            self.database.clone(),
            sel.clone(),
        ));
        sel
    }

    /// Whether the saved scope could not be read (the pattern shown is the default).
    pub fn stored_unreadable(&self) -> bool {
        self.stored_unreadable
    }

    /// The tree's rows as shown (a database's schemas while it is expanded).
    fn visible_rows(&self, list: &ScopeList) -> Vec<Row> {
        let mut out = Vec::new();
        for (i, db) in list.databases.iter().enumerate() {
            if list.two_level {
                out.push(Row::Db(i));
                if !self.expanded.contains(&db.name) {
                    continue;
                }
            }
            out.extend((0..db.schemas.len()).map(|j| Row::Schema(i, j)));
        }
        out
    }

    /// The scope the dialog saves: `Some(value)` when it changed (`None` inside: the default).
    fn scope_to_save(&self) -> Option<Option<String>> {
        if !self.touched {
            return None;
        }
        let parsed = self.parsed.as_ref().ok()?;
        let text = parsed.to_text().to_string();
        let value = (text != default_parsed(self.engine).to_text()).then_some(text);
        if self.stored.as_ref() == Some(&value) {
            return None;
        }
        Some(value)
    }

    // ---- changes ----

    /// Follows the dialog's draft: another engine re-parses the pattern in its dialect (the
    /// default again while it was not changed), another database moves `@`.
    fn sync(&mut self, p: &Profile, cx: &mut Context<Self>) {
        let database = p.database.clone();
        // Fix round 1 (M-5): a read-only store makes the section read-only too.
        let read_only = cx
            .try_global::<ConnectionsModel>()
            .is_some_and(|m| m.store_read_only());
        if read_only != self.read_only {
            self.read_only = read_only;
            self.pattern.update(cx, |i, cx| {
                i.set_read_only(read_only);
                cx.notify();
            });
        }
        if p.engine != self.engine {
            self.engine = p.engine;
            self.list_version += 1;
            self.expanded.clear();
            self.cursor = 0;
            self.list_generation = None;
            if !self.touched {
                let text = default_pattern(p.engine).to_string();
                self.set_text(&text, cx);
                self.confirmed_text = text;
            } else {
                let text = self.pattern_text(cx);
                self.reparse(&text);
            }
            cx.notify();
        }
        if database != self.database {
            self.database = database;
            self.list_generation = None;
        }
        let generation = adapter::catalogs_generation(cx);
        if self.list_generation != Some(generation) {
            self.list_generation = Some(generation);
            let list = list_of(self.profile, self.engine, self.database.as_deref(), cx);
            if list != self.list {
                if let Some(l) = &list {
                    // The current database starts open (the mock).
                    for db in l.databases.iter().filter(|d| d.current || !l.two_level) {
                        self.expanded.insert(db.name.clone());
                    }
                }
                self.list = list;
                self.list_version += 1;
                self.cursor = 0;
                cx.notify();
            }
        }
    }

    /// The saved scope arrived (an edit): shown unless the pattern was changed meanwhile.
    fn stored_arrived(&mut self, cx: &mut Context<Self>) {
        if self.stored.is_some() || self.is_new {
            return;
        }
        let unreadable = userdata::is_unreadable(self.profile, cx);
        if unreadable != self.stored_unreadable {
            self.stored_unreadable = unreadable;
            cx.notify();
        }
        let Some(data) = userdata::get(self.profile, cx) else {
            return;
        };
        self.stored = Some(data.scope.clone());
        if !self.touched {
            let text = data
                .scope
                .clone()
                .unwrap_or_else(|| default_pattern(self.engine).to_string());
            self.set_text(&text, cx);
            self.confirmed_text = text;
        }
        cx.notify();
    }

    fn reparse(&mut self, text: &str) {
        self.parsed = parse(text, self.engine);
        if let Ok(p) = &self.parsed {
            self.good = p.clone();
        }
    }

    /// Puts `text` in the field (no `Changed` event) and parses it.
    fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.pattern.update(cx, |i, cx| {
            i.set_value(text.to_string());
            cx.notify();
        });
        self.reparse(text);
    }

    /// A typed change of the pattern.
    fn typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.touched = true;
        let text = self.pattern_text(cx);
        let before = self.list.as_ref().map(|l| self.selection(l));
        self.reparse(&text);
        if self.parsed.is_ok() && self.cost.is_none() {
            if self.needs_cost_question(before.as_ref()) {
                let keep = self.confirmed_text.clone();
                self.ask_cost(None, keep, window, cx);
            } else {
                self.confirmed_text = text;
            }
        }
        self.dialog_changed(cx);
        cx.notify();
    }

    /// Whether the scope now reaches another database while it did not before, unanswered.
    fn needs_cost_question(&self, before: Option<&Selection>) -> bool {
        let Some(list) = &self.list else {
            return false;
        };
        let now = self.selection(list);
        !self.cost_confirmed
            && reaches_other_databases(&now, list)
            && !before.is_some_and(|b| reaches_other_databases(b, list))
    }

    /// Applies a pattern the tree wrote (asking first when it reaches another database).
    fn apply_from_tree(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.touched = true;
        let before = self.list.as_ref().map(|l| self.selection(l));
        let keep = self.pattern_text(cx);
        self.set_text(&text, cx);
        if self.needs_cost_question(before.as_ref()) {
            // Not applied until "Read all": the field shows what it was.
            self.set_text(&keep, cx);
            self.ask_cost(Some(text), keep, window, cx);
        } else {
            self.confirmed_text = text;
        }
        self.dialog_changed(cx);
        cx.notify();
    }

    fn ask_cost(
        &mut self,
        apply: Option<String>,
        keep: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cost = Some(CostAsk { apply, keep });
        window.focus(&self.focus.cost_keep, cx);
    }

    /// The answer to the all-databases question.
    pub fn answer_cost(&mut self, read_all: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self.cost.take() else {
            return;
        };
        if read_all {
            self.cost_confirmed = true;
            if let Some(text) = ask.apply {
                self.set_text(&text, cx);
            }
            self.confirmed_text = self.pattern_text(cx);
        } else {
            self.set_text(&ask.keep, cx);
            self.confirmed_text = ask.keep;
        }
        window.focus(&self.focus.tree, cx);
        self.dialog_changed(cx);
        cx.notify();
    }

    /// Clicks the box of tree row `index` (as shown).
    ///
    /// Fix round 1 (I-3): when the list shows the whole pattern (writing it again from the
    /// checked rows gives the same syntax tree), the pattern is written again; otherwise the
    /// click adds (or removes) one clause for the row, keeping every term the list cannot show
    /// (a regex, an exclusion, a name not listed). A click that cannot be made that way asks
    /// before writing the pattern again.
    pub fn toggle_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(list) = self.list.clone() else {
            return;
        };
        let Some(row) = self.visible_rows(&list).get(index).copied() else {
            return;
        };
        self.cursor = index;
        let before = self.selection(&list);
        let mut sel = before.clone();
        sel.all_databases = false;
        match row {
            Row::Db(i) => {
                let on = db_state(&sel.dbs[i]) != Tri::Checked;
                let d = &mut sel.dbs[i];
                d.db = on;
                d.all = on;
                d.schemas.iter_mut().for_each(|b| *b = on);
            }
            Row::Schema(i, j) => {
                let d = &mut sel.dbs[i];
                d.schemas[j] = !d.schemas[j];
                d.all = false;
                d.db = d.schemas.iter().any(|b| *b);
            }
        }
        let rewrite = generate(&sel, &list);
        if self.list_shows_all(&list, &before) {
            self.apply_from_tree(rewrite, window, cx);
            return;
        }
        let edited = self
            .edit_for_row(&list, row, &before)
            .filter(|t| self.edit_reaches(t, &list, &before, &sel, row));
        match edited {
            Some(text) => self.apply_from_tree(text, window, cx),
            None => self.ask_rewrite(rewrite, window, cx),
        }
    }

    /// Clicks "All databases".
    pub fn toggle_all_databases(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(list) = self.list.clone() else {
            return;
        };
        let before = self.selection(&list);
        let rewrite = if before.all_databases {
            default_pattern(self.engine).to_string()
        } else {
            "*:*".to_string()
        };
        if self.list_shows_all(&list, &before) {
            self.apply_from_tree(rewrite, window, cx);
            return;
        }
        // Keep what the list cannot show: "*:*" is added; reading less asks first.
        let added = (!before.all_databases)
            .then(|| self.with_clauses(&["*:*".to_string()], &[]))
            .flatten()
            .filter(|t| {
                parse(t, self.engine).is_ok_and(|p| {
                    let ctx = current_ctx(self.engine, self.database.as_deref());
                    evaluate(&p, &list, &ctx).all_databases
                })
            });
        match added {
            Some(text) => self.apply_from_tree(text, window, cx),
            None => self.ask_rewrite(rewrite, window, cx),
        }
    }

    /// Whether the list shows the whole pattern: writing it again from `sel` (what it selects
    /// among the listed names) gives the same syntax tree.
    fn list_shows_all(&self, list: &ScopeList, sel: &Selection) -> bool {
        parse(&generate(sel, list), self.engine).is_ok_and(|p| p == self.good)
    }

    /// The pattern with the clauses `add` (texts) and without those equal to `remove`, from the
    /// last pattern that parsed (`None`: a clause text did not parse).
    fn with_clauses(&self, add: &[String], remove: &[String]) -> Option<String> {
        let parsed = |t: &str| {
            parse(t, self.engine)
                .ok()
                .and_then(|p| p.clauses().first().cloned())
        };
        let removed: Vec<_> = remove.iter().map(|t| parsed(t)).collect::<Option<_>>()?;
        let texts = clause_texts(self.good.to_text());
        if texts.len() != self.good.clauses().len() {
            return None;
        }
        let mut out: Vec<String> = texts
            .into_iter()
            .zip(self.good.clauses())
            .filter(|(_, c)| !removed.contains(c))
            .map(|(t, _)| t)
            .collect();
        for a in add {
            let c = parsed(a)?;
            if !self.good.clauses().contains(&c) {
                out.push(a.clone());
            }
        }
        Some(out.join("|"))
    }

    /// The smallest change of the pattern for a click on `row` (see [`Self::toggle_row`]).
    fn edit_for_row(&self, list: &ScopeList, row: Row, before: &Selection) -> Option<String> {
        let db_term = |i: usize| {
            let db = &list.databases[i];
            if db.current {
                "@".to_string()
            } else {
                term(&db.name)
            }
        };
        let (clause, on) = match row {
            Row::Db(i) => (
                format!("{}:*", db_term(i)),
                db_state(&before.dbs[i]) != Tri::Checked,
            ),
            Row::Schema(i, j) => {
                let s = term(&list.databases[i].schemas[j]);
                let clause = if list.two_level {
                    format!("{}:{s}", db_term(i))
                } else {
                    s
                };
                (clause, !before.dbs[i].schemas[j])
            }
        };
        let negative = format!("!{clause}");
        let excluded = parse(&negative, self.engine)
            .ok()
            .and_then(|p| p.clauses().first().cloned())
            .is_some_and(|c| self.good.clauses().contains(&c));
        if on && excluded {
            // Undo this row's own exclusion.
            self.with_clauses(&[], &[negative])
        } else if on {
            self.with_clauses(&[clause], &[])
        } else {
            self.with_clauses(&[negative], &[])
        }
    }

    /// Whether `text` makes the change `want` asks of `row`, and only it.
    fn edit_reaches(
        &self,
        text: &str,
        list: &ScopeList,
        before: &Selection,
        want: &Selection,
        row: Row,
    ) -> bool {
        let Ok(p) = parse(text, self.engine) else {
            return false;
        };
        let ctx = current_ctx(self.engine, self.database.as_deref());
        let after = evaluate(&p, list, &ctx);
        after.dbs.iter().enumerate().all(|(i, a)| {
            let toggled = match row {
                Row::Db(d) | Row::Schema(d, _) => d == i,
            };
            if !toggled {
                return *a == before.dbs[i];
            }
            let all_ok = match row {
                Row::Db(_) => a.all == want.dbs[i].all,
                Row::Schema(..) => true,
            };
            a.schemas == want.dbs[i].schemas && all_ok
        })
    }

    /// Asks before writing the pattern again from the checked names (I-3).
    fn ask_rewrite(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.rewrite = Some(text);
        window.focus(&self.focus.rewrite_keep, cx);
        cx.notify();
    }

    /// The answer to the rewrite question.
    pub fn answer_rewrite(&mut self, apply: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.rewrite.take() else {
            return;
        };
        window.focus(&self.focus.tree, cx);
        if apply {
            self.apply_from_tree(text, window, cx);
        }
        cx.notify();
    }

    /// Whether the rewrite question is open.
    pub fn rewrite_question_open(&self) -> bool {
        self.rewrite.is_some()
    }

    /// Expands (`true`) or collapses a database row.
    fn set_expanded(&mut self, name: &Name, open: bool, cx: &mut Context<Self>) {
        if open {
            self.expanded.insert(name.clone());
        } else {
            self.expanded.remove(name);
        }
        cx.notify();
    }

    /// The keys of the focused tree: ↑/↓ move, Space toggles, → / ← expand or collapse a
    /// database (mock 7 keyboard).
    fn on_tree_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(list) = self.list.clone() else {
            return;
        };
        let rows = self.visible_rows(&list);
        if rows.is_empty() || event.keystroke.modifiers.modified() {
            return;
        }
        let last = rows.len() - 1;
        let cur = self.cursor.min(last);
        match event.keystroke.key.as_str() {
            "down" => self.cursor = (cur + 1).min(last),
            "up" => self.cursor = cur.saturating_sub(1),
            "space" => self.toggle_row(cur, window, cx),
            "right" | "left" => {
                let i = match rows[cur] {
                    Row::Db(i) | Row::Schema(i, _) => i,
                };
                if !list.two_level {
                    return;
                }
                let name = list.databases[i].name.clone();
                let open = event.keystroke.key == "right";
                self.set_expanded(&name, open, cx);
                if !open && let Some(at) = rows.iter().position(|r| *r == Row::Db(i)) {
                    self.cursor = at;
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// A click on "Encrypt cached metadata" (the box shows the effective value; `on` is the new
    /// one): a new profile changes at once (it has no cache), and so does a saved one whose
    /// cache already matches `on`; any other saved profile asks first.
    pub fn toggle_encrypt(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog.upgrade() else {
            return;
        };
        let stored = cx
            .try_global::<ConnectionsModel>()
            .and_then(|m| m.get(self.profile));
        if self.is_new || stored.as_ref().is_none_or(|s| s.catalog_sensitive() == on) {
            // Back to what the saved profile has: its own setting again (a derived value
            // stays derived) when that gives `on` with the draft's environment.
            let mut back = dialog.read(cx).draft.profile.clone();
            back.catalog_sensitive_set = stored.as_ref().and_then(|s| s.catalog_sensitive_set);
            let set = if !self.is_new && back.catalog_sensitive() == on {
                back.catalog_sensitive_set
            } else {
                Some(on)
            };
            self.encrypt_ask = None;
            self.set_sensitive(&dialog, set, cx);
        } else {
            self.encrypt_ask = Some(EncryptAsk::Toggle(on));
            window.focus(&self.focus.encrypt_keep, cx);
        }
        cx.notify();
    }

    /// The answer to the encryption question: after a click, Apply sets the value; after an
    /// environment change, Keep pins the value the cache has, Apply keeps it derived.
    pub fn answer_encrypt(&mut self, apply: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self.encrypt_ask.take() else {
            return;
        };
        if let Some(dialog) = self.dialog.upgrade() {
            match (ask, apply) {
                (EncryptAsk::Toggle(on), true) => self.set_sensitive(&dialog, Some(on), cx),
                (EncryptAsk::Toggle(_), false) => {}
                (EncryptAsk::Environment(_), true) => {
                    let keep = dialog.read(cx).draft.profile.catalog_sensitive_set;
                    self.set_sensitive(&dialog, keep, cx);
                }
                (EncryptAsk::Environment(to), false) => {
                    self.set_sensitive(&dialog, Some(!to), cx);
                }
            }
        }
        window.focus(&self.focus.encrypt, cx);
        cx.notify();
    }

    /// Writes the user's choice into the draft (the dialog's edit hook is not run here: it
    /// would update this section while it is being updated).
    fn set_sensitive(
        &mut self,
        dialog: &Entity<ConnectionDialog>,
        set: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        if let Some(r) = cx
            .try_global::<Sections>()
            .and_then(|s| s.0.get(&dialog.entity_id()))
        {
            r.sensitive_chosen.set(true);
        }
        dialog.update(cx, |d, cx| {
            d.draft.profile.catalog_sensitive_set = set;
            cx.notify();
        });
    }

    /// The dialog redraws (its footer follows [`blocks_save`]).
    fn dialog_changed(&self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.dialog.upgrade() {
            dialog.update(cx, |_, cx| cx.notify());
        }
    }

    /// Opens (`true`) or folds the disclosure.
    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.open = open;
        cx.notify();
    }
}

// ---- rendering ----------------------------------------------------------------------------------

fn t_dynamic(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    match cx.try_global::<L10nGlobal>() {
        Some(g) => g.l10n.t_dynamic(id, args).into(),
        None => SharedString::from(id.to_string()),
    }
}

fn count(n: usize) -> ArgValue {
    ArgValue::Count(u64::try_from(n).unwrap_or(u64::MAX))
}

/// The folded section's summary: what the scope reads (mock 7). Always a Fluent message (the
/// disclosure draws its summary as one): with nothing listed, the pattern is its argument.
fn summary(s: &SchemasSection, cx: &App) -> SharedString {
    let Some(list) = &s.list else {
        return t_dynamic(
            cx,
            "hv-scope-summary-pattern",
            vec![("pattern", ArgValue::Text(s.pattern_text(cx)))],
        );
    };
    let sel = s.selection(list);
    let dbs = list.databases.len();
    if list.two_level && sel.all_databases {
        return t_dynamic(cx, "hv-scope-summary-mysql", vec![("total", count(dbs))]);
    }
    let total = list.schema_count();
    let on: usize = sel
        .dbs
        .iter()
        .map(|d| d.schemas.iter().filter(|b| **b).count())
        .sum();
    if !list.two_level && total > 0 && on == total {
        return t_dynamic(cx, "hv-scope-summary-mysql", vec![("total", count(total))]);
    }
    t_dynamic(
        cx,
        "hv-scope-summary",
        vec![("n", count(on)), ("total", count(total))],
    )
}

/// The error line's sentence for `e` in `text`: the mock's "Expected a schema name after “:”"
/// where that is what failed, else the general sentence of its kind.
fn error_text(e: &ScopeError, text: &str, cx: &App) -> SharedString {
    let column = count(e.column as usize);
    match e.kind {
        ScopeErrorKind::UnclosedBrace => t_dynamic(
            cx,
            "intro-scope-err-unclosed-brace",
            vec![("column", column)],
        ),
        ScopeErrorKind::BadRegex => {
            t_dynamic(cx, "intro-scope-err-bad-regex", vec![("column", column)])
        }
        ScopeErrorKind::Syntax => {
            let before = text
                .chars()
                .take((e.column as usize).saturating_sub(1))
                .filter(|c| !c.is_whitespace())
                .last();
            if before == Some(':') {
                t_dynamic(cx, "hv-scope-pattern-error", vec![("pos", column)])
            } else {
                t_dynamic(cx, "intro-scope-syntax-error", vec![("column", column)])
            }
        }
    }
}

fn warning_text(kind: ScopeWarningKind, column: u32, cx: &App) -> SharedString {
    match kind {
        ScopeWarningKind::DbTermIgnored => t_dynamic(
            cx,
            "intro-scope-warn-db-term-ignored",
            vec![("column", count(column as usize))],
        ),
    }
}

fn well(theme: &Theme) -> hacivat_ui_core::gpui_surface::Hsla {
    let c = &theme.tokens.colors;
    match theme.mode {
        ThemeMode::Dark => c.panel,
        ThemeMode::Light => c.ground,
    }
    .to_hsla()
}

/// A tree row's box: the M2a checkbox, with the mixed state (blue fill, minus glyph).
fn tri_box(state: Tri, theme: &Theme) -> Div {
    let t = theme.tokens;
    let c = &t.colors;
    let mark = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(14.0))
        .rounded(px(t.radii.xs))
        .border_1();
    match state {
        Tri::Unchecked => mark.bg(well(theme)).border_color(c.seam_strong.to_hsla()),
        Tri::Checked | Tri::Mixed => mark
            .bg(c.blue.to_hsla())
            .border_color(c.blue.to_hsla())
            .child(
                Icon::new(if state == Tri::Checked {
                    IconName::Check
                } else {
                    IconName::Minus
                })
                .size(IconSize::Chrome)
                .color(c.on_accent.to_hsla()),
            ),
    }
}

/// A line under a field: an optional glyph and a wrapping caption.
fn line(
    theme: &Theme,
    icon: Option<IconName>,
    text: SharedString,
    ink: hacivat_ui_core::gpui_surface::Hsla,
    id: &'static str,
) -> Div {
    let t = theme.tokens;
    hint_row(
        theme,
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(t.spacing.sm))
            .w_full()
            .min_w_0()
            .children(icon.map(|i| {
                div()
                    .flex_none()
                    .mt(px(1.0))
                    .child(Icon::new(i).size(IconSize::Chrome).color(ink))
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(paragraph(text, TextSource::Fluent, t.type_ramp.caption, ink).id(id)),
            ),
    )
}

fn render(
    state: &Entity<SchemasSection>,
    draft: &ProfileDraft,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = *Theme::get(cx);
    let s = state.read(cx);
    let summary = summary(s, cx);
    let root = s.focus.root.clone();
    let open = s.open;
    let content = open.then(|| render_content(state, draft, &theme, window, cx));
    let toggle = state.downgrade();
    let mut disclosure = Disclosure::new("conn-scope", tr(cx, ids::hv::SCOPE_SECTION, NoArgs))
        .summary(summary)
        .open(open)
        .on_toggle(move |open, _, cx| {
            let _ = toggle.update(cx, |s, cx| s.set_open(open, cx));
        });
    if let Some(content) = content {
        disclosure = disclosure.content(content);
    }
    div()
        .debug_selector(|| "conn-scope-section".to_string())
        .track_focus(&root)
        .relative()
        .child(hacivat_ui::follow::focus_probe(root.clone()))
        .child(disclosure)
        .into_any_element()
}

fn render_content(
    state: &Entity<SchemasSection>,
    draft: &ProfileDraft,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme.tokens;
    let c = &t.colors;
    let s = state.read(cx);
    let read_only = cx
        .try_global::<ConnectionsModel>()
        .is_some_and(|m| m.store_read_only());
    let mut col = div().flex().flex_col().w_full().gap(px(t.spacing.md));
    let weak = state.downgrade();

    // All databases (PostgreSQL, with a list), or the MySQL note.
    if let Some(list) = &s.list {
        if list.two_level {
            let me = weak.clone();
            col = col.child(hint_row(
                theme,
                Checkbox::new("scope-all-dbs", tr(cx, ids::hv::SCOPE_ALL_DBS, NoArgs))
                    .checked(s.all_databases())
                    .disabled(read_only)
                    .focus_handle(s.focus.all.clone())
                    .on_toggle(move |_, window, cx| {
                        let _ = me.update(cx, |s, cx| s.toggle_all_databases(window, cx));
                    }),
            ));
        } else if matches!(s.engine, DbmsKind::MySql | DbmsKind::MariaDb) {
            col = col.child(line(
                theme,
                None,
                tr(cx, ids::hv::SCOPE_MYSQL_NOTE, NoArgs),
                c.text_2.to_hsla(),
                "scope-mysql-note",
            ));
        }
    }
    col = col.child(hint_row(theme, render_tree(state, s, theme, window, cx)));

    // The all-databases question, in place (never a modal), focused on its safe answer.
    if s.cost.is_some() {
        let n = s.list.as_ref().map_or(0, |l| l.databases.len());
        let (keep, yes) = (weak.clone(), weak.clone());
        // Full width (the mock's `.notice`): its two answers stay on one line in Turkish.
        col = col.child(
            InlineNotice::new(NoticeIcon::Info)
                .line(tr(cx, ids::hv::SCOPE_ALL_DBS_TITLE, NoArgs))
                .secondary(t_dynamic(
                    cx,
                    "hv-scope-all-dbs-body",
                    vec![("n", count(n))],
                ))
                .action(
                    Button::new("scope-cost-keep", tr(cx, ids::hv::SCOPE_ALL_DBS_NO, NoArgs))
                        .focus_handle(s.focus.cost_keep.clone())
                        .on_raised(true)
                        .on_click(move |_, window, cx| {
                            let _ = keep.update(cx, |s, cx| s.answer_cost(false, window, cx));
                        }),
                )
                .action(
                    Button::new(
                        "scope-cost-yes",
                        t_dynamic(cx, "hv-scope-all-dbs-yes", vec![("n", count(n))]),
                    )
                    .focus_handle(s.focus.cost_yes.clone())
                    .on_raised(true)
                    .on_click(move |_, window, cx| {
                        let _ = yes.update(cx, |s, cx| s.answer_cost(true, window, cx));
                    }),
                ),
        );
    }

    // Fix round 1 (I-3): a click the pattern cannot take without losing terms asks first.
    if s.rewrite.is_some() {
        let (keep, apply) = (weak.clone(), weak.clone());
        col = col.child(
            InlineNotice::new(NoticeIcon::Info)
                .line(t_dynamic(cx, "hv-scope-rewrite-title", Vec::new()))
                .secondary(t_dynamic(cx, "hv-scope-rewrite-body", Vec::new()))
                .action(
                    Button::new(
                        "scope-rewrite-keep",
                        t_dynamic(cx, "hv-scope-rewrite-keep", Vec::new()),
                    )
                    .focus_handle(s.focus.rewrite_keep.clone())
                    .on_raised(true)
                    .on_click(move |_, window, cx| {
                        let _ = keep.update(cx, |s, cx| s.answer_rewrite(false, window, cx));
                    }),
                )
                .action(
                    Button::new(
                        "scope-rewrite-apply",
                        t_dynamic(cx, "hv-scope-rewrite-apply", Vec::new()),
                    )
                    .focus_handle(s.focus.rewrite_apply.clone())
                    .on_raised(true)
                    .on_click(move |_, window, cx| {
                        let _ = apply.update(cx, |s, cx| s.answer_rewrite(true, window, cx));
                    }),
                ),
        );
    }

    // Fix round 1 (M-4): the saved scope could not be read.
    if s.stored_unreadable {
        col = col.child(line(
            theme,
            Some(IconName::Info),
            t_dynamic(cx, "hv-scope-stored-unreadable", Vec::new()),
            c.text.to_hsla(),
            "scope-stored-unreadable",
        ));
    }

    // The pattern: the same scope as text.
    let err = s.error().cloned();
    let text = s.pattern_text(cx);
    let border = if err.is_some() {
        c.text_2.to_hsla()
    } else {
        c.seam_strong.to_hsla()
    };
    let mut field = div()
        .debug_selector(|| "scope-pattern".to_string())
        .w_full()
        .min_w_0()
        .rounded(px(t.radii.sm))
        .bg(well(theme));
    if err.is_some() {
        // The error state: the field's border goes text-2 (never red).
        field = field.border_1().border_color(border);
    }
    let field = field.child(s.pattern.clone());
    col = col.child(form_row(
        theme,
        tr(cx, ids::hv::SCOPE_PATTERN, NoArgs),
        field,
    ));
    match &err {
        Some(e) => {
            col = col.child(line(
                theme,
                Some(IconName::CircleAlert),
                error_text(e, &text, cx),
                c.text.to_hsla(),
                "scope-pattern-error",
            ));
        }
        None => {
            if let Ok(p) = &s.parsed {
                for w in p.warnings() {
                    col = col.child(line(
                        theme,
                        Some(IconName::Info),
                        warning_text(w.kind, w.column, cx),
                        c.text.to_hsla(),
                        "scope-pattern-warning",
                    ));
                }
                let ctx = current_ctx(s.engine, s.database.as_deref());
                if p.mentions_current() && ctx.database.is_none() && ctx.schema.is_none() {
                    col = col.child(line(
                        theme,
                        Some(IconName::Info),
                        t_dynamic(cx, "intro-scope-current-unknown", Vec::new()),
                        c.text.to_hsla(),
                        "scope-current-unknown",
                    ));
                }
            }
            col = col.child(line(
                theme,
                None,
                tr(cx, ids::hv::SCOPE_PATTERN_HINT, NoArgs),
                c.text_2.to_hsla(),
                "scope-pattern-hint",
            ));
        }
    }
    col = col.child(line(
        theme,
        None,
        tr(cx, ids::hv::SCOPE_BUILTIN_NOTE, NoArgs),
        c.text_2.to_hsla(),
        "scope-builtin-note",
    ));

    // Encrypt cached metadata (R-M4-15), and its question.
    let me = weak.clone();
    col = col.child(
        div().mt(px(t.spacing.sm)).child(hint_row(
            theme,
            Checkbox::new("scope-encrypt", tr(cx, ids::hv::SCOPE_ENCRYPT, NoArgs))
                .checked(draft.profile.catalog_sensitive())
                .disabled(read_only)
                .focus_handle(s.focus.encrypt.clone())
                .on_toggle(move |on, window, cx| {
                    let _ = me.update(cx, |s, cx| s.toggle_encrypt(on, window, cx));
                }),
        )),
    );
    col = col.child(line(
        theme,
        None,
        tr(cx, ids::hv::SCOPE_ENCRYPT_SUB, NoArgs),
        c.text_2.to_hsla(),
        "scope-encrypt-sub",
    ));
    if let Some(ask) = s.encrypt_ask {
        let on = ask.to();
        let name = draft.profile.name.clone();
        let (keep, apply) = (weak.clone(), weak);
        let (question, verb) = if on {
            ("hv-scope-encrypt-confirm-on", ids::hv::SCOPE_ENCRYPT_APPLY)
        } else {
            (
                "hv-scope-encrypt-confirm-off",
                ids::hv::SCOPE_ENCRYPT_APPLY_OFF,
            )
        };
        // Full width, as the all-databases question.
        col = col.child(
            InlineNotice::new(NoticeIcon::Info)
                .line(t_dynamic(
                    cx,
                    question,
                    vec![("name", ArgValue::Text(name))],
                ))
                .action(
                    Button::new(
                        "scope-encrypt-keep",
                        tr(cx, ids::hv::SCOPE_ENCRYPT_KEEP, NoArgs),
                    )
                    .focus_handle(s.focus.encrypt_keep.clone())
                    .on_raised(true)
                    .on_click(move |_, window, cx| {
                        let _ = keep.update(cx, |s, cx| s.answer_encrypt(false, window, cx));
                    }),
                )
                .action(
                    Button::new("scope-encrypt-apply", tr(cx, verb, NoArgs))
                        .icon(IconName::Eraser)
                        .focus_handle(s.focus.encrypt_apply.clone())
                        .on_raised(true)
                        .on_click(move |_, window, cx| {
                            let _ = apply.update(cx, |s, cx| s.answer_encrypt(true, window, cx));
                        }),
                ),
        );
    }
    col.into_any_element()
}

/// The checkbox tree in its field well (or, with nothing listed, the sentence that says so).
fn render_tree(
    state: &Entity<SchemasSection>,
    s: &SchemasSection,
    theme: &Theme,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let t = theme.tokens;
    let c = &t.colors;
    let frame = div()
        .w_full()
        .min_w_0()
        .rounded(px(t.radii.sm))
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .bg(well(theme));
    let Some(list) = &s.list else {
        // Not connected and nothing cached (mock "Not connected").
        return frame
            .px(px(10.0))
            .py(px(4.0))
            .flex()
            .flex_row()
            .items_start()
            .gap(px(t.spacing.sm))
            .child(
                div().flex_none().mt(px(1.0)).child(
                    Icon::new(IconName::Plug)
                        .size(IconSize::Chrome)
                        .color(c.text_3.to_hsla()),
                ),
            )
            .child(
                div().flex_1().min_w_0().child(
                    paragraph(
                        tr(cx, ids::hv::SCOPE_CONNECT_TO_LIST, NoArgs),
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id("scope-connect-to-list"),
                ),
            )
            .into_any_element();
    };
    let sel = s.selection(list);
    let rows = s.visible_rows(list);
    let focused = s.focus.tree.is_focused(window);
    let cursor = s.cursor.min(rows.len().saturating_sub(1));
    let mut body = div()
        .id("scope-tree")
        .flex()
        .flex_col()
        .w_full()
        .max_h(px(TREE_MAX_PX))
        .overflow_y_scroll()
        .py(px(2.0))
        // The frame checks treat this clip as a scroll viewport (rows scroll under it).
        .child(hacivat_ui::text::scroll_viewport());
    for (ix, row) in rows.iter().enumerate() {
        let (indent, state_of, icon, name, hint, dim, chevron) = match *row {
            Row::Db(i) => {
                let db = &list.databases[i];
                let hint = if db.current {
                    Some(tr(cx, ids::hv::SCOPE_CURRENT_DB, NoArgs))
                } else {
                    Some(tr(cx, ids::hv::SCOPE_OTHER_DB_HINT, NoArgs))
                };
                (
                    DB_INDENT_PX,
                    db_state(&sel.dbs[i]),
                    IconName::Cylinder,
                    db.name.to_string(),
                    hint,
                    !db.current,
                    Some((db.name.clone(), s.expanded.contains(&db.name))),
                )
            }
            Row::Schema(i, j) => (
                if list.two_level {
                    SCHEMA_INDENT_PX
                } else {
                    DB_INDENT_PX + 4.0
                },
                if sel.dbs[i].schemas[j] {
                    Tri::Checked
                } else {
                    Tri::Unchecked
                },
                if list.two_level || s.engine == DbmsKind::Sqlite {
                    IconName::FolderTree
                } else {
                    IconName::Cylinder
                },
                list.databases[i].schemas[j].to_string(),
                None,
                false,
                None,
            ),
        };
        // Names are data: shown verbatim, bidi-isolated; dim rows keep text-3 on the well
        // (field wells restore text-3, NOTES cross-cutting #3).
        let ink = if dim { c.text_3 } else { c.text }.to_hsla();
        let mut r = div()
            .id(("scope-row", ix))
            .debug_selector(move || format!("scope-row-{ix}"))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.sm))
            .h(px(ROW_PX))
            .w_full()
            .min_w_0()
            .pl(px(indent))
            .pr(px(t.spacing.md))
            .cursor_pointer();
        if focused && ix == cursor {
            r = r.bg(c.blue_tint.to_hsla());
        }
        if let Some((db, open)) = chevron {
            let toggle = state.downgrade();
            r = r.child(
                div()
                    .id(("scope-chevron", ix))
                    .debug_selector(move || format!("scope-chevron-{ix}"))
                    .flex_none()
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        let db = db.clone();
                        let _ = toggle.update(cx, |s, cx| s.set_expanded(&db, !open, cx));
                    })
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(IconSize::Chrome)
                        .color(c.text_2.to_hsla()),
                    ),
            );
        }
        let click = state.downgrade();
        r = r
            .on_click(move |_, window, cx| {
                let _ = click.update(cx, |s, cx| {
                    window.focus(&s.focus.tree, cx);
                    s.toggle_row(ix, window, cx);
                });
            })
            .child(tri_box(state_of, theme))
            .child(
                Icon::new(icon)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            )
            .child(
                div().flex_shrink(1.0).min_w_0().child(
                    Label::new(
                        crate::explorer::rows::isolate(&name),
                        TextSource::Catalog,
                        t.type_ramp.mono_data,
                        ink,
                    )
                    .truncate(true),
                ),
            );
        if let Some(hint) = hint {
            r = r.child(div().flex_none().child(Label::new(
                hint,
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text_2.to_hsla(),
            )));
        }
        let mut node = A11yNode::new(
            ("scope-row-node", ix),
            Role::CheckBox,
            SharedString::from(name),
            false,
            r,
        )
        .toggled(state_of == Tri::Checked);
        // Fix round 1 (M-2): the a11y layer has no mixed state; the word says it.
        if state_of == Tri::Mixed {
            node = node.description(t_dynamic(cx, "hv-scope-tree-mixed", Vec::new()));
        }
        body = body.child(node);
    }
    let key = state.downgrade();
    let tree = div()
        .track_focus(&s.focus.tree)
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            let _ = key.update(cx, |s, cx| s.on_tree_key(event, window, cx));
        })
        .child(body);
    A11yNode::new(
        "scope-tree-node",
        Role::Tree,
        tr(cx, ids::hv::SCOPE_SECTION, NoArgs),
        false,
        frame.child(tree),
    )
    .into_any_element()
}

/// Text callbacks for tests: the section's labels as the dialog draws them.
pub fn summary_text(state: &Entity<SchemasSection>, cx: &App) -> String {
    summary(state.read(cx), cx).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg_list() -> ScopeList {
        ScopeList {
            two_level: true,
            databases: vec![
                ScopeDb {
                    name: Name::from("app"),
                    current: true,
                    schemas: vec![
                        Name::from("analytics"),
                        Name::from("public"),
                        Name::from("Sales"),
                    ],
                },
                ScopeDb {
                    name: Name::from("billing"),
                    current: false,
                    schemas: Vec::new(),
                },
            ],
        }
    }

    fn sel_of(text: &str, list: &ScopeList) -> Selection {
        let p = parse(text, DbmsKind::Postgres).expect("parses");
        evaluate(&p, list, &current_ctx(DbmsKind::Postgres, Some("app")))
    }

    #[test]
    fn tree_and_pattern_round_trip_on_postgres() {
        let list = pg_list();
        for text in ["@:*", "@:public,\"Sales\"", "@:*|billing:*", "*:*", ""] {
            let sel = sel_of(text, &list);
            let again = generate(&sel, &list);
            assert_eq!(sel_of(&again, &list), sel, "{text:?} -> {again:?}");
        }
        assert_eq!(generate(&sel_of("@:*", &list), &list), "@:*");
        assert_eq!(
            generate(&sel_of("@:public,\"Sales\"", &list), &list),
            "@:public,\"Sales\""
        );
        assert_eq!(generate(&sel_of("*:*", &list), &list), "*:*");
    }

    #[test]
    fn clauses_split_outside_quotes_and_regexes() {
        assert_eq!(
            clause_texts(r#"@:*|!@:{a|b\}}|"x|y":z|@:"q""|""#),
            vec![r"@:*", r"!@:{a|b\}}", r#""x|y":z"#, r#"@:"q""|""#]
        );
        assert!(clause_texts("").is_empty());
        assert_eq!(clause_texts("@:*"), vec!["@:*"]);
    }

    #[test]
    fn a_bare_capital_folds_on_postgres_so_it_is_quoted() {
        assert_eq!(term("Sales"), "\"Sales\"");
        assert_eq!(term("müşteri"), "müşteri");
        assert_eq!(term("a b"), "\"a b\"");
        assert_eq!(term("x\"y"), "\"x\"\"y\"");
        assert_eq!(term("a:b"), "\"a:b\"");
    }

    #[test]
    fn other_databases_are_told_from_the_current_one() {
        let list = pg_list();
        assert!(!reaches_other_databases(&sel_of("@:*", &list), &list));
        assert!(reaches_other_databases(
            &sel_of("@:*|billing:*", &list),
            &list
        ));
        assert!(reaches_other_databases(&sel_of("*:*", &list), &list));
    }

    #[test]
    fn mysql_patterns_list_databases_by_name() {
        let list = ScopeList {
            two_level: false,
            databases: vec![ScopeDb {
                name: Name::from(""),
                current: false,
                schemas: vec![Name::from("api"), Name::from("api_log")],
            }],
        };
        let ctx = current_ctx(DbmsKind::MySql, Some("api"));
        let sel = |t: &str| evaluate(&parse(t, DbmsKind::MySql).expect("parses"), &list, &ctx);
        assert_eq!(generate(&sel("*"), &list), "*");
        assert_eq!(generate(&sel("api"), &list), "api");
        assert_eq!(sel("api").dbs[0].schemas, vec![true, false]);
    }
}
