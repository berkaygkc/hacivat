// ## m4b-t16-attach
//! Settings › Script libraries (NOTES M4b surface 4; R-T9-1, T10 m-h): every library in one
//! place, with its binding and its watcher state; removing one never touches the folder.
//!
//! - **One source of truth:** the library service's snapshot (`projection::service`), redrawn
//!   on each of its event batches (the explorer's `Contributors` global, which the projection
//!   touches once per drained batch). Nothing here reads a library folder.
//! - **Rows:** the folder's name (mono), "binds to …" and its aliases (wrapping), the path,
//!   the state (watching, checking every 2 seconds and why, not found, stopped at the file
//!   limit, moved or replaced, …), the script count and the repository line; **Watch:
//!   Automatic / Every 2 seconds** (R-T9-1, `LibraryService::set_watch`, saved in
//!   `libraries.toml`); **Use this folder…** for a library refused as moved or replaced
//!   (confirmed, `LibraryService::adopt_root`); **Reveal** (disabled while the folder is
//!   missing) and **Remove…** (an alertdialog on Cancel: "The folder and its files stay as they
//!   are"; Remove is Default, not Danger, controller ruling e).
//! - **Local mappings** below: env mark, connection › database, "shows", the library, Remove.
//! - **Search** filters libraries by name, path and database (Turkish-tolerant, the page's
//!   own search options).
//! - **Focus:** Search → Attach a folder… → per row (Watch, Use this folder…, Reveal,
//!   Remove…) → per mapping Remove. The section is a first-stop focus home, so a row that
//!   goes away with its focused button hands the focus to "Attach a folder…", never to the
//!   window root (R-M4b-43).
//! - Every name and path from a library is shown through `escape_untrusted` / `display_name`.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_l10n::{ArgValue, L10n, build_key, find};
use hacivat_proto::ProfileId;
use hacivat_scripts::names::escape_untrusted;
use hacivat_scripts::service::{LibraryDiag, LibraryService, UnavailableReason};
use hacivat_scripts::{
    DatabaseName, GitSummary, LibraryId, LibrarySnapshot, LibraryState, LocalMapping, PollReason,
    RegistryError, WatchMode, WorkspaceSnapshot,
};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::env_swatch;
use hacivat_ui::dialog::{DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::focus_home::{HomeSlot, first_stop_home};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented};
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Context, Div, Global, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, SharedString, Styled as _, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::Theme;

use super::Section;
use crate::connections::ConnectionsModel;
use crate::l10n_global::{L10nGlobal, text_fn_id, tr_id};
use crate::scripts::{attach_dialog, file_ops, projection};
use crate::shell::Workspace;

fn text(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    crate::explorer::live::text(cx, id, args)
}

fn arg(s: &str) -> ArgValue {
    ArgValue::Text(s.to_string())
}

fn num(n: usize) -> ArgValue {
    ArgValue::Count(u64::try_from(n).unwrap_or(u64::MAX))
}

/// Opens the settings tab on Script libraries ("Script library settings…").
pub fn open_page(window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    ws.update(cx, |ws, cx| {
        ws.open_registered_tab(super::SETTINGS_KIND, window, cx)
    });
    if let Some(page) = super::open_page(cx) {
        page.update(cx, |p, cx| p.show_section(Section::Scripts, cx));
    }
}

/// The section (a child view of the settings page, which hands it its search query).
pub struct ScriptsSection {
    home: HomeSlot,
    query: String,
    // ## m4b-t16-attach fix round 1 (I6): what the last action on a row could not do.
    errors: HashMap<ErrorKey, &'static str>,
}

// ## m4b-t16-attach fix round 1 (I6)
/// The row an action's error belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ErrorKey {
    Library(LibraryId),
    Mapping(ProfileId, DatabaseName),
}

/// The section while the settings page shows it (actions report their errors to it).
struct OpenSection(WeakEntity<ScriptsSection>);

impl Global for OpenSection {}

impl ScriptsSection {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Every batch of the library service's events (and every profile change) redraws.
        cx.observe_global::<crate::explorer::contrib::Contributors>(|_, cx| cx.notify())
            .detach();
        cx.observe_global::<ConnectionsModel>(|_, cx| cx.notify())
            .detach();
        let this = cx.entity().downgrade();
        cx.set_global(OpenSection(this));
        ScriptsSection {
            home: HomeSlot::new(),
            query: String::new(),
            errors: HashMap::new(),
        }
    }

    /// The page's search query (the section filters its libraries by it).
    pub fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        if self.query != query {
            self.query = query.to_string();
            cx.notify();
        }
    }
}

/// The page's search options (Turkish-tolerant where the language is).
fn matches(query: &str, texts: &[&str], cx: &App) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    let Some(global) = cx.try_global::<L10nGlobal>() else {
        return true;
    };
    let l10n: &L10n = &global.l10n;
    let opts = super::search::search_opts(l10n);
    texts
        .iter()
        .any(|t| find(&build_key(t, &opts), query, &opts).is_some())
}

/// A library's folder name, for display.
fn library_name(lib: &LibrarySnapshot) -> String {
    lib.root
        .path
        .file_name()
        .map(|n| hacivat_scripts::pathsafe::display_name(&n.to_string_lossy()))
        .unwrap_or_default()
}

/// The libraries the search shows (all of them without a query).
fn shown_libraries(snap: &WorkspaceSnapshot, query: &str, cx: &App) -> Vec<Arc<LibrarySnapshot>> {
    snap.libraries
        .iter()
        .filter(|l| {
            let name = library_name(l);
            let path = l.root.path.display().to_string();
            let mut texts: Vec<&str> = vec![&name, &path, l.db.as_str()];
            texts.extend(l.aliases.iter().map(|a| a.as_str()));
            matches(query, &texts, cx)
        })
        .cloned()
        .collect()
}

/// Whether the search finds any library (the page then shows the section under its results).
pub(crate) fn any_match(query: &str, cx: &App) -> bool {
    projection::service(cx).is_some_and(|s| !shown_libraries(&s.snapshot(), query, cx).is_empty())
}

// ---- actions ---------------------------------------------------------------------------------

fn confirm(
    title: TextFn,
    subtitle: String,
    body: TextFn,
    yes: &'static str,
    window: &mut Window,
    cx: &mut App,
    on_yes: impl FnOnce(&mut App) + 'static,
) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    let button = |id: &'static str, kind| DialogButton {
        label: text_fn_id(id),
        kind,
        accesskey: None,
        english_accesskey: None,
    };
    // The safe answer is the default (NOTES cross-cutting 7): Cancel is Primary and focused.
    // The folder heads the body, whole and wrapping (data, escaped): what the question is
    // about (NOTES surface 4 shows it under the title; the dialog's subtitle is one line).
    let body: TextFn = Rc::new(move |cx: &App| {
        let path = text(cx, "hv-scripts-remove-path", vec![("path", arg(&subtitle))]);
        format!("{path}\n{}", body(cx)).into()
    });
    let spec = DialogSpec {
        title,
        body,
        buttons: vec![
            button(yes, ButtonKind::Default),
            button("hv-tx-cancel", ButtonKind::Primary),
        ],
        cancel: 1,
        primary: 1,
        size: DialogSize::Standard,
    };
    let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec, window, cx));
    dialog.update(cx, |d, _| d.set_role(Role::AlertDialog));
    let mut keep = Some(dialog.clone());
    let mut on_yes = Some(on_yes);
    cx.subscribe(&dialog, move |_, event: &DialogEvent, cx| {
        keep.take();
        if matches!(event, DialogEvent::Pressed(0))
            && let Some(f) = on_yes.take()
        {
            f(cx);
        }
    })
    .detach();
}

// ## m4b-t16-attach fix round 1 (I6): every action says what it could not do, on its row.
/// Shows (or, with `None`, clears) the error of `key`'s row.
fn report(key: ErrorKey, reason: Option<&'static str>, cx: &mut App) {
    let Some(section) = cx.try_global::<OpenSection>().and_then(|o| o.0.upgrade()) else {
        return;
    };
    section.update(cx, |s, cx| {
        match reason {
            Some(r) => {
                s.errors.insert(key, r);
            }
            None => {
                s.errors.remove(&key);
            }
        }
        cx.notify();
    });
}

/// `hv-scripts-set-failed`'s variant for a registry refusal.
fn reason_of(e: &RegistryError) -> &'static str {
    match e {
        RegistryError::ReadOnly => "readonly",
        RegistryError::UnknownLibrary => "state",
        _ => "other",
    }
}

/// Runs `f` with the service off the UI thread; its answer is `key`'s row error (`None`: done).
fn act(
    key: ErrorKey,
    cx: &mut App,
    f: impl AsyncFnOnce(Arc<LibraryService>) -> Option<&'static str> + 'static,
) {
    let Some(svc) = projection::service(cx) else {
        return;
    };
    report(key.clone(), None, cx);
    cx.spawn(async move |cx| {
        let reason = f(svc).await;
        cx.update(|cx| report(key, reason, cx));
    })
    .detach();
}

fn remove(lib: &LibrarySnapshot, mappings: usize, window: &mut Window, cx: &mut App) {
    let name = library_name(lib);
    let path = escape_untrusted(&lib.root.path.display().to_string(), 512);
    let id = lib.id;
    // ## m4b-t16-attach fix round 1 (m7): unsaved script tabs of this library lose their
    // recovery copies with it (R-T5-4); the question says so, with how many.
    let dirty = crate::scripts::tabs::all_docs(cx)
        .iter()
        .filter(|d| {
            let d = d.read(cx);
            d.spec(cx).is_some_and(|s| s.library == id) && d.is_dirty(cx)
        })
        .count();
    let title: TextFn =
        Rc::new(move |cx: &App| text(cx, "hv-scripts-remove-title", vec![("library", arg(&name))]));
    let body: TextFn = Rc::new(move |cx: &App| {
        let body = if mappings == 0 {
            tr_id(cx, "hv-scripts-remove-body-none")
        } else {
            text(cx, "hv-scripts-remove-body", vec![("n", num(mappings))])
        };
        if dirty == 0 {
            body
        } else {
            let tabs = text(cx, "hv-scripts-remove-dirty", vec![("n", num(dirty))]);
            format!("{body}\n{tabs}").into()
        }
    });
    confirm(
        title,
        path,
        body,
        "hv-scripts-remove-btn",
        window,
        cx,
        move |cx| {
            // The folder is never touched (detach): only `libraries.toml` and recovery copies.
            act(ErrorKey::Library(id), cx, async move |svc| {
                svc.detach(id).await.err().as_ref().map(reason_of)
            });
        },
    );
}

fn adopt(lib: &LibrarySnapshot, window: &mut Window, cx: &mut App) {
    let name = library_name(lib);
    let path = escape_untrusted(&lib.root.path.display().to_string(), 512);
    let id = lib.id;
    // ## m4b-t20-adversarial (named line, T16 re-review m4): the folder this confirm is for;
    // another one swapped in before the press is refused.
    let seen = lib.refused_identity;
    let title: TextFn =
        Rc::new(move |cx: &App| text(cx, "hv-scripts-adopt-title", vec![("library", arg(&name))]));
    let shown = path.clone();
    let body: TextFn =
        Rc::new(move |cx: &App| text(cx, "hv-scripts-adopt-body", vec![("path", arg(&shown))]));
    confirm(
        title,
        path,
        body,
        "hv-scripts-adopt-btn",
        window,
        cx,
        move |cx| {
            act(ErrorKey::Library(id), cx, async move |svc| {
                // Answered once the folder was validated: `false` means it can't be used.
                match svc.adopt_root(id, seen).await {
                    Ok(true) => None,
                    Ok(false) => Some("refused"),
                    Err(e) => Some(reason_of(&e)),
                }
            });
        },
    );
}

fn set_watch(id: LibraryId, mode: WatchMode, cx: &mut App) {
    act(ErrorKey::Library(id), cx, async move |svc| {
        svc.set_watch(id, mode).await.err().as_ref().map(reason_of)
    });
}

fn unmap(profile: ProfileId, database: DatabaseName, cx: &mut App) {
    let key = ErrorKey::Mapping(profile, database.clone());
    act(key, cx, async move |svc| {
        match svc.unmap(profile, database).await {
            Ok(true) => None,
            Ok(false) => Some("state"),
            Err(e) => Some(reason_of(&e)),
        }
    });
}

// ## m4b-t16-attach fix round 1 (m5)
/// The number a library's Settings row is keyed by (its id, folded to 64 bits).
pub fn row_key(id: LibraryId) -> u64 {
    let (hi, lo) = id.0.as_u64_pair();
    hi ^ lo
}

/// The number a local mapping's row is keyed by (its node).
fn mapping_key(profile: ProfileId, database: &DatabaseName) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut h = std::hash::DefaultHasher::new();
    profile.hash(&mut h);
    database.hash(&mut h);
    h.finish()
}

/// An action's error on its row.
fn error_line(reason: &'static str, theme: &Theme, cx: &App) -> AnyElement {
    crate::scripts::attach_dialog::ink_line(
        text(cx, "hv-scripts-set-failed", vec![("reason", arg(reason))]),
        "scripts-set-error",
        theme,
    )
}

// ---- drawing ---------------------------------------------------------------------------------

/// What a library's state line says: its icon, its words and, when polled, why.
fn state_line(lib: &LibrarySnapshot, cx: &App) -> (IconName, SharedString, Option<SharedString>) {
    let status = |state: &str| {
        text(
            cx,
            "hv-scripts-set-status",
            vec![("state", arg(state)), ("n", num(0))],
        )
    };
    let other = |state: &str| text(cx, "hv-scripts-set-state", vec![("state", arg(state))]);
    let stopped = lib.state != LibraryState::Scanning
        && projection::service(cx).is_some_and(|s| {
            s.snapshot().view(None).library(lib.id).is_some_and(|l| {
                l.diagnostics()
                    .iter()
                    .any(|d| matches!(d, LibraryDiag::ScanStopped(_)))
            })
        });
    match &lib.state {
        _ if stopped => {
            let n = hacivat_scripts::scan::Limits::default().max_files;
            (
                IconName::TriangleAlert,
                text(
                    cx,
                    "hv-scripts-set-status",
                    vec![("state", arg("limit")), ("n", num(n))],
                ),
                None,
            )
        }
        LibraryState::Watching => (IconName::Eye, status("watching"), None),
        LibraryState::Polling(reason) => {
            let why = match reason {
                PollReason::UserChoice => tr_id(cx, "hv-scripts-set-watch-chosen"),
                PollReason::WatchBudget => text(
                    cx,
                    "hv-scripts-set-status-tip",
                    vec![("reason", arg("watches"))],
                ),
                PollReason::NetworkFs | PollReason::BackendLost => text(
                    cx,
                    "hv-scripts-set-status-tip",
                    vec![("reason", arg("network"))],
                ),
            };
            (IconName::Timer, status("polling"), Some(why))
        }
        LibraryState::Unavailable(UnavailableReason::Missing) => {
            (IconName::CircleAlert, status("unavailable"), None)
        }
        LibraryState::Unavailable(UnavailableReason::RootChanged) => {
            (IconName::CircleAlert, other("moved"), None)
        }
        LibraryState::Unavailable(UnavailableReason::InEnvFolder) => {
            (IconName::CircleAlert, other("envfolder"), None)
        }
        LibraryState::Unavailable(UnavailableReason::NotResponding) => {
            (IconName::CircleAlert, other("notresponding"), None)
        }
        LibraryState::Unavailable(_) => (IconName::CircleAlert, other("unreadable"), None),
        LibraryState::Unwatched => (IconName::CircleAlert, other("unwatched"), None),
        LibraryState::Scanning => (IconName::Timer, other("scanning"), None),
    }
}

fn caption_line(t: SharedString, theme: &Theme) -> AnyElement {
    let tk = theme.tokens;
    paragraph(
        t,
        TextSource::Fluent,
        tk.type_ramp.caption,
        tk.colors.text_2.to_hsla(),
    )
    .into_any_element()
}

fn library_row(
    lib: &Arc<LibrarySnapshot>,
    snap: &WorkspaceSnapshot,
    error: Option<&'static str>,
    theme: &Theme,
    cx: &mut Context<ScriptsSection>,
) -> Div {
    // ## m4b-t16-attach fix round 1 (m5): a row's controls are keyed by its library, never by
    // where the (filtered) list puts it.
    let n = row_key(lib.id);
    let key = |part: &'static str| (part, n);
    let t = theme.tokens;
    let c = &t.colors;
    let name = library_name(lib);
    let mut binds = text(
        cx,
        "hv-scripts-map-binds",
        vec![("database", arg(&lib.db.display_escaped()))],
    )
    .to_string();
    if !lib.aliases.is_empty() {
        let aliases: Vec<String> = lib.aliases.iter().map(|a| a.display_escaped()).collect();
        let line = text(
            cx,
            "hv-scripts-set-aliases",
            vec![("aliases", arg(&aliases.join(", ")))],
        );
        binds = format!("{binds} · {line}");
    }
    let (icon, state, why) = state_line(lib, cx);
    let ink = match lib.state {
        LibraryState::Watching | LibraryState::Polling(_) | LibraryState::Scanning => c.text_2,
        _ => c.text,
    };
    let view = snap.view(None);
    let shown = view.library(lib.id);
    let files = shown.as_ref().map_or(0, |l| l.file_count());
    let git = match shown.as_ref().map(|l| l.git()) {
        Some(GitSummary::Ready) => Some(tr_id(cx, "hv-scripts-set-git")),
        Some(GitSummary::Untrusted(_)) | Some(GitSummary::Unavailable(_)) => {
            Some(tr_id(cx, "hv-scripts-set-git-off"))
        }
        _ => None,
    };
    let mut meta = text(cx, "hv-scripts-set-files", vec![("n", num(files))]).to_string();
    if let Some(g) = git {
        meta = format!("{meta} · {g}");
    }
    // Watch: automatic / every 2 seconds (R-T9-1).
    let id = lib.id;
    let watch = Segmented::new(key("scripts-set-watch"))
        .item(SegmentItem {
            content: SegmentContent::Label(tr_id(cx, "hv-scripts-set-watch-auto")),
            pressed: lib.watch == WatchMode::Automatic,
            tooltip: tr_id(cx, "hv-scripts-set-watch-auto"),
            a11y_name: None,
            on_click: Rc::new(move |_, _, cx| set_watch(id, WatchMode::Automatic, cx)),
        })
        .item(SegmentItem {
            content: SegmentContent::Label(tr_id(cx, "hv-scripts-set-watch-poll")),
            pressed: lib.watch == WatchMode::Poll,
            tooltip: tr_id(cx, "hv-scripts-set-watch-poll"),
            a11y_name: None,
            on_click: Rc::new(move |_, _, cx| set_watch(id, WatchMode::Poll, cx)),
        });
    let missing = lib.state == LibraryState::Unavailable(UnavailableReason::Missing);
    let path = lib.root.path.clone();
    let reveal = Button::new(
        key("scripts-set-reveal"),
        text(
            cx,
            "hv-scripts-menu-reveal",
            vec![("os", arg(file_ops::host_os()))],
        ),
    )
    .kind(ButtonKind::Ghost)
    .disabled(missing)
    .on_click(move |_, _, cx| cx.reveal_path(&path));
    let mappings = snap.mappings.iter().filter(|m| m.library == id).count();
    let for_remove = lib.clone();
    let remove_btn = Button::new(
        key("scripts-set-remove"),
        tr_id(cx, "hv-scripts-set-remove"),
    )
    .on_click(move |_, window, cx| remove(&for_remove, mappings, window, cx));
    // ## m4b-t16-attach fix round 1 (I3): only for a folder refused for its identity.
    let adoptable = lib.can_adopt();
    // ## m4b-t16-attach fix round 1 (I1): the manifest's problem, worded as the popover words
    // it (the kept binding while R-T10-7 keeps it).
    let manifest = shown
        .as_ref()
        .and_then(|l| crate::scripts::diagnostics::manifest_problem(l, cx));
    let mut col = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(t.spacing.xs))
        .child(
            Label::new(
                SharedString::from(name),
                TextSource::Data,
                t.type_ramp.mono_data,
                c.text.to_hsla(),
            )
            .truncate(true),
        )
        .child(paragraph(
            binds,
            TextSource::Data,
            t.type_ramp.body_sm,
            c.text_2.to_hsla(),
        ))
        .child(
            Label::new(
                SharedString::from(escape_untrusted(&lib.root.path.display().to_string(), 512)),
                TextSource::Data,
                t.type_ramp.mono_micro,
                c.text_2.to_hsla(),
            )
            .truncate(true),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(t.spacing.sm))
                .child(
                    div()
                        .flex_none()
                        .pt(px(2.0))
                        .child(Icon::new(icon).size(IconSize::Chrome).color(ink.to_hsla())),
                )
                .child(div().flex_1().min_w_0().child(paragraph(
                    state,
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    ink.to_hsla(),
                ))),
        )
        .children(why.map(|w| caption_line(w, theme)))
        .child(caption_line(meta.into(), theme))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(t.spacing.md))
                .child(Label::new(
                    tr_id(cx, "hv-scripts-set-watch"),
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                ))
                .child(watch),
        );
    if let Some(line) = manifest {
        col = col.child(crate::scripts::attach_dialog::ink_line(
            line,
            "scripts-set-manifest",
            theme,
        ));
    }
    if adoptable {
        let for_adopt = lib.clone();
        col = col.child(
            div().child(
                Button::new(key("scripts-set-adopt"), tr_id(cx, "hv-scripts-set-adopt"))
                    .on_click(move |_, window, cx| adopt(&for_adopt, window, cx)),
            ),
        );
    }
    if let Some(reason) = error {
        col = col.child(error_line(reason, theme, cx));
    }
    div()
        .debug_selector({
            let id = lib.id;
            move || format!("scripts-set-library-{id}")
        })
        .flex()
        .flex_row()
        .items_start()
        .gap(px(t.spacing.lg))
        .px(px(t.spacing.lg))
        .py(px(t.spacing.md))
        .border_b_1()
        .border_color(c.seam.to_hsla())
        .child(
            div().flex_none().pt(px(2.0)).child(
                Icon::new(IconName::Folder)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            ),
        )
        .child(col)
        .child(
            div()
                .flex()
                .flex_row()
                .flex_none()
                .gap(px(t.spacing.sm))
                .child(reveal)
                .child(remove_btn),
        )
}

fn mapping_row(
    m: &LocalMapping,
    snap: &WorkspaceSnapshot,
    error: Option<&'static str>,
    theme: &Theme,
    cx: &mut Context<ScriptsSection>,
) -> Option<Div> {
    // ## m4b-t16-attach fix round 1 (m5): keyed by the node it maps.
    let j = mapping_key(m.profile, &m.database);
    let t = theme.tokens;
    let c = &t.colors;
    let p = cx.try_global::<ConnectionsModel>()?.get(m.profile);
    let lib = snap.library(m.library)?;
    let (env, conn) = match &p {
        Some(p) => (p.environment, escape_untrusted(&p.name, 512)),
        None => (None, String::new()),
    };
    let place = text(
        cx,
        "hv-scripts-attach-from",
        vec![
            ("conn", arg(&conn)),
            ("database", arg(&escape_untrusted(m.database.as_str(), 512))),
        ],
    );
    let (profile, database) = (m.profile, m.database.clone());
    Some(
        div()
            .debug_selector(move || format!("scripts-set-mapping-{j}"))
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(t.spacing.md))
            .px(px(t.spacing.lg))
            .py(px(t.spacing.sm))
            .border_b_1()
            .border_color(c.seam.to_hsla())
            .child(env_swatch(
                crate::connections::capsule::env_kind(env),
                theme,
            ))
            .child(Label::new(
                place,
                TextSource::Data,
                t.type_ramp.mono_data,
                c.text.to_hsla(),
            ))
            .child(Label::new(
                tr_id(cx, "hv-scripts-set-mapping-shows"),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
            ))
            .child(Label::new(
                SharedString::from(library_name(lib)),
                TextSource::Data,
                t.type_ramp.mono_data,
                c.text.to_hsla(),
            ))
            .child(div().flex_1())
            .child(
                Button::new(
                    ("scripts-set-mapping-remove", j),
                    tr_id(cx, "hv-scripts-set-mapping-remove"),
                )
                .on_click(move |_, _, cx| unmap(profile, database.clone(), cx)),
            )
            // ## m4b-t16-attach fix round 1 (I6): on its own line under the row.
            .children(error.map(|r| div().w_full().child(error_line(r, theme, cx)))),
    )
}

impl ScriptsSection {
    fn content(&mut self, query: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let snap = projection::service(cx).map(|s| s.snapshot());
        let attach = Button::new("scripts-set-attach", tr_id(cx, "hv-scripts-set-attach"))
            .on_click(|_, window, cx| attach_dialog::start(None, window, cx));
        let mut out = div()
            .debug_selector(|| "scripts-settings".to_string())
            .flex()
            .flex_col()
            .gap(px(t.spacing.lg))
            .px(px(t.spacing.xl))
            .pb(px(t.spacing.xl))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(t.spacing.lg))
                    .child(div().flex_1().min_w_0().child(paragraph(
                        tr_id(cx, "hv-scripts-set-note"),
                        TextSource::Fluent,
                        t.type_ramp.body,
                        c.text_2.to_hsla(),
                    )))
                    .child(div().flex_none().child(attach)),
            );
        let libraries = snap
            .as_ref()
            .map(|s| shown_libraries(s, query, cx))
            .unwrap_or_default();
        match &snap {
            Some(snap) if !snap.libraries.is_empty() => {
                let mut list = div()
                    .flex()
                    .flex_col()
                    .border_1()
                    .border_color(c.seam_strong.to_hsla())
                    .rounded(px(t.radii.sm));
                for lib in &libraries {
                    let error = self.errors.get(&ErrorKey::Library(lib.id)).copied();
                    list = list.child(library_row(lib, snap, error, &theme, cx));
                }
                out = out.child(list);
                let mappings: Vec<Div> = snap
                    .mappings
                    .iter()
                    .filter(|m| libraries.iter().any(|l| l.id == m.library))
                    .filter_map(|m| {
                        let error = self
                            .errors
                            .get(&ErrorKey::Mapping(m.profile, m.database.clone()))
                            .copied();
                        mapping_row(m, snap, error, &theme, cx)
                    })
                    .collect();
                if !mappings.is_empty() {
                    out = out
                        .child(
                            Label::new(
                                tr_id(cx, "hv-scripts-set-mappings"),
                                TextSource::Fluent,
                                t.type_ramp.panel_title,
                                c.text_2.to_hsla(),
                            )
                            .id("scripts-set-mappings"),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .border_1()
                                .border_color(c.seam_strong.to_hsla())
                                .rounded(px(t.radii.sm))
                                .children(mappings),
                        );
                }
            }
            _ => {
                out = out.child(
                    paragraph(
                        tr_id(cx, "hv-scripts-set-empty"),
                        TextSource::Fluent,
                        t.type_ramp.body,
                        c.text_2.to_hsla(),
                    )
                    .id("scripts-set-empty"),
                );
            }
        }
        first_stop_home(&self.home, out).into_any_element()
    }
}

impl Render for ScriptsSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.query.clone();
        self.content(&query, cx)
    }
}

/// Shot and test fixtures.
#[cfg(feature = "test-support")]
pub mod fixtures {
    use super::*;

    /// Asks to remove the `row`-th library, as its Remove… button does.
    pub fn ask_remove(row: usize, window: &mut Window, cx: &mut App) {
        let Some(svc) = projection::service(cx) else {
            return;
        };
        let snap = svc.snapshot();
        if let Some(lib) = snap.libraries.get(row) {
            let mappings = snap.mappings.iter().filter(|m| m.library == lib.id).count();
            remove(lib, mappings, window, cx);
        }
    }

    /// Chooses `mode` for the `row`-th library shown, as a click on its Watch segment does.
    pub fn choose_watch(row: usize, mode: WatchMode, cx: &mut App) {
        let Some(svc) = projection::service(cx) else {
            return;
        };
        if let Some(lib) = svc.snapshot().libraries.get(row) {
            set_watch(lib.id, mode, cx);
        }
    }
}
