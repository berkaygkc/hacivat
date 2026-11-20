//! Settings › Metadata (Task 27; design/m4-explorer surface 13): refresh after DDL (Always, Ask,
//! Never), the background check interval, the cache limits and "Clear cached metadata for this
//! connection…".
//!
//! ## m4-t27-refresh-ui
//!
//! - **Refresh the explorer after DDL**: a Segmented row (`rows.rs`); a choice reaches every
//!   open refresh coordinator at once (`explorer::refresh_prompt::preference_changed`).
//! - **Check for changes in the background**: a 150 px Select (Automatic, Off, Every 5, 10,
//!   30 min; NOTES ruling 14).
//! - **Cache per connection / for all connections**: a number field and an MB/GB select
//!   (R-M4-25), saved in MiB on Enter or when the focus leaves the row; outside 1 MiB–1 TiB it
//!   is set to the nearest bound; what is not a whole number puts the saved size back.
//! - **Cached metadata**: "Clear cached metadata…" for the active console's connection, with
//!   its size on this computer (read on the core runtime's blocking pool). It asks first (Cancel
//!   focused, NOTES cross-cutting 8); Clear closes the cache store, deletes the profile's
//!   cache files, shows an empty catalog and, when connected, reads it again
//!   (`explorer::live::release_cache` / `reopen`). The explorer's `metadata.clear_cache` asks
//!   the same for the cursor row's connection.
//! - The background check and the cache limits are read when a connection's catalog opens
//!   (the hub makes its engine then): a change applies from the next connect.
//!
//! Every write goes through `shell::update_settings`, the one settings writer.

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use hacivat_l10n::{ArgValue, ExactDecimal, NoArgs, ids};
use hacivat_meta::persist::CacheStore;
use hacivat_proto::ProfileId;
use hacivat_settings::{CacheMb, Settings, ValidateInterval, ValidateMinutes};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::{DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::icon::IconName;
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::select::{Select, SelectEvent, SelectOption};
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    Subscription, Task, Window, div, px,
};
use tokio::sync::oneshot;

use super::rows::{self, SettingRow};
use super::{Section, SettingsPage};
use crate::console::Consoles;
use crate::explorer::live;
use crate::l10n_global::{text_fn, tr};
use crate::shell::{AppState, update_settings};

/// The background-check select (NOTES surface 13).
const CHECK_SELECT_PX: f32 = 150.0;
/// A cache size's number field and its unit select.
const CACHE_FIELD_PX: f32 = 88.0;
const CACHE_UNIT_PX: f32 = 76.0;

/// The Metadata section's rows.
pub fn metadata_rows() -> Vec<SettingRow> {
    rows::settings_rows()
        .into_iter()
        .filter(|r| rows::row_section(r.key) == Some(Section::Metadata))
        .collect()
}

// ---- the background check ------------------------------------------------------------------

/// The interval choices, in the select's order, as stored (`metadata.validate_interval_min`).
const INTERVALS: [&str; 5] = ["auto", "off", "5", "10", "30"];

/// The stored value of `v` (one of [`INTERVALS`]).
pub fn interval_value(v: ValidateInterval) -> String {
    match v {
        ValidateInterval::Auto => "auto".to_string(),
        ValidateInterval::Off => "off".to_string(),
        ValidateInterval::Minutes(m) => m.get().to_string(),
    }
}

fn interval_of(value: &str) -> ValidateInterval {
    match value {
        "off" => ValidateInterval::Off,
        "5" => ValidateInterval::Minutes(ValidateMinutes::Five),
        "10" => ValidateInterval::Minutes(ValidateMinutes::Ten),
        "30" => ValidateInterval::Minutes(ValidateMinutes::Thirty),
        _ => ValidateInterval::Auto,
    }
}

/// Saves the background check `value` (one of [`INTERVALS`]).
pub fn apply_interval(value: &str, cx: &mut App) {
    let v = interval_of(value);
    let saved = cx
        .try_global::<AppState>()
        .map(|s| s.settings.metadata.validate_interval_min);
    if saved != Some(v) {
        update_settings(cx, |s| s.metadata.validate_interval_min = v);
    }
}

fn interval_label(value: &'static str) -> TextFn {
    match value {
        "auto" => text_fn(ids::hv::METADATA_CHECK_AUTO),
        "off" => text_fn(ids::hv::SETTINGS_OFF),
        n => {
            let n: u64 = n.parse().unwrap_or_default();
            Rc::new(move |cx: &App| {
                live::text(cx, "hv-metadata-every-min", vec![("n", ArgValue::Count(n))])
            })
        }
    }
}

// ---- the cache sizes -------------------------------------------------------------------------

/// A cache size's unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheUnit {
    Mb,
    Gb,
}

impl CacheUnit {
    const ALL: [CacheUnit; 2] = [CacheUnit::Mb, CacheUnit::Gb];

    fn index(self) -> usize {
        match self {
            CacheUnit::Mb => 0,
            CacheUnit::Gb => 1,
        }
    }

    fn mib(self) -> u64 {
        match self {
            CacheUnit::Mb => 1,
            CacheUnit::Gb => 1024,
        }
    }
}

/// How `mb` shows: whole GB when it is a multiple of 1,024, else MB.
pub fn shown_size(mb: u32) -> (u32, CacheUnit) {
    if mb >= 1024 && mb.is_multiple_of(1024) {
        (mb / 1024, CacheUnit::Gb)
    } else {
        (mb, CacheUnit::Mb)
    }
}

/// A typed size in `unit`, as MiB inside `CacheMb`'s range (set to the nearest bound outside
/// it); `None` when `text` is not a whole number (digits, with group separators allowed).
pub fn parse_size(text: &str, unit: CacheUnit) -> Option<CacheMb> {
    let digits: String = text
        .trim()
        .chars()
        .filter(|c| !matches!(c, ',' | '.' | ' ' | '\u{a0}' | '\u{202f}' | '\''))
        .collect();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().unwrap_or(u64::MAX);
    let mib = n.saturating_mul(unit.mib());
    let clamped = mib.clamp(u64::from(CacheMb::MIN), u64::from(CacheMb::MAX));
    CacheMb::new(u32::try_from(clamped).unwrap_or(CacheMb::MAX))
}

/// One cache size row's controls.
struct CacheField {
    key: &'static str,
    input: Entity<TextInput>,
    unit: Entity<Select>,
    /// The saved MiB the field shows, its unit, and the text the field was given.
    shown: Option<(u32, CacheUnit, String)>,
}

impl CacheField {
    fn saved(&self, cx: &App) -> u32 {
        cx.try_global::<AppState>()
            .map_or(0, |s| saved_size(&s.settings, self.key))
    }

    fn typed(&self, cx: &App) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|(_, _, t)| *self.input.read(cx).value() != **t)
    }

    /// The unit the field's number is in.
    fn unit(&self) -> CacheUnit {
        self.shown.as_ref().map_or(CacheUnit::Mb, |(_, u, _)| *u)
    }

    /// Puts `mb` in the field in `unit` (whole numbers only: a size that is not a whole
    /// number of `unit` shows in MB).
    fn show(&mut self, mb: u32, unit: Option<CacheUnit>, cx: &mut App) {
        let (n, unit) = match unit {
            Some(CacheUnit::Gb) if mb.is_multiple_of(1024) => (mb / 1024, CacheUnit::Gb),
            Some(_) => (mb, CacheUnit::Mb),
            None => shown_size(mb),
        };
        let text = n.to_string();
        self.input.update(cx, |input, cx| {
            if input.value() != text {
                input.set_value(text.clone());
                cx.notify();
            }
        });
        self.unit.update(cx, |s, cx| {
            if s.selected() != unit.index() {
                s.set_selected(unit.index(), cx);
            }
        });
        self.shown = Some((mb, unit, text));
    }

    /// Saves what was typed (in the select's unit); what is not a number puts the saved size
    /// back.
    fn commit(&mut self, cx: &mut App) {
        if !self.typed(cx) {
            return;
        }
        let text = self.input.read(cx).value().to_string();
        let unit = self.unit();
        let Some(size) = parse_size(&text, unit) else {
            let saved = self.saved(cx);
            self.show(saved, Some(unit), cx);
            return;
        };
        if size.get() != self.saved(cx) {
            let key = self.key;
            update_settings(cx, move |s| set_size(s, key, size));
        }
        self.show(size.get(), Some(unit), cx);
    }

    /// Before a frame: the field follows the saved size unless it holds typed text.
    fn follow(&mut self, cx: &mut App) {
        if self.typed(cx) {
            return;
        }
        let saved = self.saved(cx);
        if self.shown.as_ref().map(|(mb, _, _)| *mb) != Some(saved) {
            self.show(saved, None, cx);
        }
    }
}

fn saved_size(s: &Settings, key: &str) -> u32 {
    if key == rows::CACHE_TOTAL {
        s.metadata.cache_mb_total.get()
    } else {
        s.metadata.cache_mb_per_profile.get()
    }
}

fn set_size(s: &mut Settings, key: &str, size: CacheMb) {
    if key == rows::CACHE_TOTAL {
        s.metadata.cache_mb_total = size;
    } else {
        s.metadata.cache_mb_per_profile = size;
    }
}

// ---- the page's controls --------------------------------------------------------------------

/// The Metadata section's own controls, owned by the settings page.
pub struct MetadataControls {
    check: Entity<Select>,
    caches: [CacheField; 2],
    _subscriptions: Vec<Subscription>,
}

impl MetadataControls {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let options = INTERVALS
            .iter()
            .map(|v| SelectOption {
                label: interval_label(v),
                hint: None,
                disabled: false,
            })
            .collect();
        let selected = cx
            .try_global::<AppState>()
            .map(|s| interval_value(s.settings.metadata.validate_interval_min))
            .and_then(|v| INTERVALS.iter().position(|i| *i == v))
            .unwrap_or(0);
        let check = cx.new(|cx| {
            let mut s = Select::new(options, selected, cx);
            s.set_label(text_fn(ids::hv::METADATA_SET_CHECK));
            s
        });
        let mut subs = vec![cx.subscribe(&check, |_, _, e: &SelectEvent, cx| {
            let SelectEvent::Changed(i) = *e;
            if let Some(v) = INTERVALS.get(i) {
                apply_interval(v, cx);
                cx.notify();
            }
        })];
        let make = |key: &'static str,
                    title,
                    window: &mut Window,
                    cx: &mut Context<SettingsPage>,
                    subs: &mut Vec<Subscription>,
                    i: usize| {
            let input = cx.new(|cx| {
                let mut input = TextInput::new(cx, text_fn(title));
                input.set_mono(true);
                input
            });
            let units = CacheUnit::ALL
                .iter()
                .map(|u| SelectOption {
                    label: text_fn(match u {
                        CacheUnit::Mb => ids::hv::METADATA_UNIT_MB,
                        CacheUnit::Gb => ids::hv::METADATA_UNIT_GB,
                    }),
                    hint: None,
                    disabled: false,
                })
                .collect();
            let unit = cx.new(|cx| {
                let mut s = Select::new(units, 0, cx);
                s.set_label(text_fn(title));
                s
            });
            subs.push(
                cx.subscribe(&input, move |page, _, e: &InputEvent, cx| match e {
                    InputEvent::Submitted => {
                        page.metadata.caches[i].commit(cx);
                        cx.notify();
                    }
                    // Escape (`Cancelled`) puts the saved size back.
                    InputEvent::Cancelled => {
                        let saved = page.metadata.caches[i].saved(cx);
                        let unit = page.metadata.caches[i].unit();
                        page.metadata.caches[i].show(saved, Some(unit), cx);
                        cx.notify();
                    }
                    InputEvent::Changed | InputEvent::CompositionChanged(_) => {}
                }),
            );
            subs.push(cx.subscribe(&unit, move |page, _, e: &SelectEvent, cx| {
                let SelectEvent::Changed(u) = *e;
                let Some(unit) = CacheUnit::ALL.get(u).copied() else {
                    return;
                };
                // A typed number is saved first, in the unit it was typed in; the saved size then
                // shows in the chosen unit (in MB when it is not a whole number of GB).
                let field = &mut page.metadata.caches[i];
                field.commit(cx);
                let saved = field.saved(cx);
                field.show(saved, Some(unit), cx);
                cx.notify();
            }));
            let focus = input.focus_handle(cx);
            subs.push(cx.on_focus_out(&focus, window, move |page, _, _, cx| {
                page.metadata.caches[i].commit(cx);
                cx.notify();
            }));
            CacheField {
                key,
                input,
                unit,
                shown: None,
            }
        };
        let per = make(
            rows::CACHE_PER_PROFILE,
            ids::hv::METADATA_SET_CACHE_CONN,
            window,
            cx,
            &mut subs,
            0,
        );
        let total = make(
            rows::CACHE_TOTAL,
            ids::hv::METADATA_SET_CACHE_TOTAL,
            window,
            cx,
            &mut subs,
            1,
        );
        MetadataControls {
            check,
            caches: [per, total],
            _subscriptions: subs,
        }
    }

    /// Before a frame: the controls follow the saved settings.
    pub fn follow(&mut self, settings: &Settings, cx: &mut App) {
        let want = INTERVALS
            .iter()
            .position(|i| *i == interval_value(settings.metadata.validate_interval_min))
            .unwrap_or(0);
        self.check.update(cx, |s, cx| {
            if s.selected() != want {
                s.set_selected(want, cx);
            }
        });
        for f in &mut self.caches {
            f.follow(cx);
        }
    }

    /// The control of metadata row `key` (`None`: a Segmented row).
    pub fn control(&self, key: &str, cx: &mut App) -> Option<Vec<AnyElement>> {
        Some(match key {
            rows::VALIDATE_INTERVAL => vec![
                div()
                    .debug_selector(|| "settings-metadata-check".to_string())
                    .flex_none()
                    .w(px(CHECK_SELECT_PX))
                    .child(self.check.clone())
                    .into_any_element(),
            ],
            rows::CACHE_PER_PROFILE | rows::CACHE_TOTAL => {
                let f = self.caches.iter().find(|f| f.key == key)?;
                let id = if key == rows::CACHE_TOTAL {
                    "total"
                } else {
                    "conn"
                };
                vec![
                    div()
                        .debug_selector(move || format!("settings-metadata-cache-{id}-field"))
                        .flex_none()
                        .w(px(CACHE_FIELD_PX))
                        .child(f.input.clone())
                        .into_any_element(),
                    div()
                        .debug_selector(move || format!("settings-metadata-cache-{id}-unit"))
                        .flex_none()
                        .w(px(CACHE_UNIT_PX))
                        .child(f.unit.clone())
                        .into_any_element(),
                ]
            }
            rows::CLEAR_CACHE => {
                let profile = active_profile(cx);
                let label = tr(cx, ids::hv::METADATA_CLEAR, NoArgs);
                let button = Button::new("settings-metadata-clear", label)
                    .kind(ButtonKind::Default)
                    .icon(IconName::Eraser)
                    .disabled(profile.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(p) = profile {
                            ask_clear(p, cx);
                        }
                    });
                vec![
                    div()
                        .debug_selector(|| "settings-metadata-clear".to_string())
                        .child(button)
                        .into_any_element(),
                ]
            }
            _ => return None,
        })
    }

    /// The clear row's note: the active console's connection and its cache size.
    pub fn clear_note(&self, cx: &mut App) -> Option<SharedString> {
        let profile = active_profile(cx)?;
        let name = profile_name(profile, cx)?;
        let Some(size) = cache_size(profile, cx) else {
            request_size(profile, cx);
            return None;
        };
        let size = live::text(
            cx,
            "hv-metadata-size-mb",
            vec![("n", ArgValue::Decimal(ExactDecimal(mb_tenths(size))))],
        );
        Some(live::text(
            cx,
            "hv-metadata-clear-note",
            vec![
                ("name", ArgValue::Text(name)),
                ("size", ArgValue::Text(size.to_string())),
            ],
        ))
    }
}

/// `bytes` in MB with one decimal (`38.4`), computed exactly.
pub fn mb_tenths(bytes: u64) -> String {
    let tenths = (u128::from(bytes) * 10 + 524_288) / 1_048_576;
    format!("{}.{}", tenths / 10, tenths % 10)
}

// ---- clearing the cache ---------------------------------------------------------------------

/// The profile of the active console.
pub fn active_profile(cx: &App) -> Option<ProfileId> {
    Consoles::focused(cx)
        .and_then(|c| Consoles::get(cx, c))
        .and_then(|s| s.profile)
        .filter(|p| profile_name(*p, cx).is_some())
}

fn profile_name(profile: ProfileId, cx: &App) -> Option<String> {
    cx.try_global::<crate::connections::ConnectionsModel>()?
        .get(profile)
        .map(|p| p.name.clone())
}

/// The cache sizes read so far, per profile (bytes).
#[derive(Default)]
struct CacheSizes {
    by_profile: HashMap<ProfileId, u64>,
    asked: std::collections::HashSet<ProfileId>,
}

impl Global for CacheSizes {}

fn cache_size(profile: ProfileId, cx: &App) -> Option<u64> {
    cx.try_global::<CacheSizes>()?
        .by_profile
        .get(&profile)
        .copied()
}

/// The profile's cache files: the database of the current format and its sidecars.
fn cache_files(profile: ProfileId, cx: &App) -> Vec<PathBuf> {
    let Some(state) = cx.try_global::<AppState>() else {
        return Vec::new();
    };
    let db = state.paths.catalog_db(
        &profile.0.to_string(),
        u32::from(hacivat_meta::codec::FORMAT_MAJOR),
    );
    let mut out = vec![db.clone()];
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut p = db.clone().into_os_string();
        p.push(suffix);
        out.push(PathBuf::from(p));
    }
    out
}

/// Reads the profile's cache size on the blocking pool (once per ask).
fn request_size(profile: ProfileId, cx: &mut App) {
    let sizes = cx.default_global::<CacheSizes>();
    if !sizes.asked.insert(profile) {
        return;
    }
    let Some(rt) = cx
        .try_global::<crate::connections::ConnectionsModel>()
        .map(|m| m.runtime())
    else {
        return;
    };
    let files = cache_files(profile, cx);
    let (tx, rx) = oneshot::channel();
    rt.spawn(async move {
        let size = tokio::task::spawn_blocking(move || {
            files
                .iter()
                .filter_map(|f| std::fs::metadata(f).ok())
                .map(|m| m.len())
                .sum::<u64>()
        })
        .await
        .unwrap_or_default();
        let _ = tx.send(size);
    });
    cx.spawn(async move |cx| {
        if let Ok(size) = rx.await {
            cx.update(|cx| {
                cx.default_global::<CacheSizes>()
                    .by_profile
                    .insert(profile, size);
                cx.refresh_windows();
            });
        }
    })
    .detach();
}

/// Forgets `profile`'s size (read again when next shown).
fn forget_size(profile: ProfileId, cx: &mut App) {
    let sizes = cx.default_global::<CacheSizes>();
    sizes.by_profile.remove(&profile);
    sizes.asked.remove(&profile);
}

/// The open clear question (tests).
struct OpenClear(
    Option<(
        ProfileId,
        hacivat_ui_core::gpui_surface::WeakEntity<hacivat_ui::dialog::Dialog>,
    )>,
);

impl Global for OpenClear {}

/// The profile whose clear question is open (tests).
pub fn clear_question(cx: &App) -> Option<ProfileId> {
    let (p, d) = cx.try_global::<OpenClear>()?.0.as_ref()?;
    d.upgrade().map(|_| *p)
}

const CLEAR: usize = 0;
const CANCEL: usize = 1;

fn text(id: &'static str, args: Vec<(&'static str, ArgValue)>) -> TextFn {
    Rc::new(move |cx: &App| live::text(cx, id, args.clone()))
}

/// Asks whether to clear `profile`'s cached metadata (Cancel focused); Clear clears it.
pub fn ask_clear(profile: ProfileId, cx: &mut App) {
    let Some(name) = profile_name(profile, cx) else {
        return;
    };
    let button = |label, kind| DialogButton {
        label,
        kind,
        accesskey: None,
        english_accesskey: None,
    };
    let spec = DialogSpec {
        title: text(
            "hv-metadata-clear-title",
            vec![("name", ArgValue::Text(name))],
        ),
        body: text("hv-metadata-clear-body", Vec::new()),
        buttons: vec![
            button(
                text("hv-metadata-clear-btn", Vec::new()),
                ButtonKind::Default,
            ),
            button(text_fn(ids::hv::CONN_DLG_CANCEL), ButtonKind::Primary),
        ],
        cancel: CANCEL,
        primary: CANCEL,
        size: DialogSize::Standard,
    };
    crate::exec_ui::in_workspace_window(cx, move |ws, window, cx| {
        let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec, window, cx));
        cx.set_global(OpenClear(Some((profile, dialog.downgrade()))));
        // Held until an event comes (the workspace removes the layer on the same event).
        let mut keep = Some(dialog.clone());
        cx.subscribe(&dialog, move |_, event: &DialogEvent, cx| {
            keep.take();
            cx.set_global(OpenClear(None));
            if *event == DialogEvent::Pressed(CLEAR) {
                clear(profile, cx).detach();
            }
        })
        .detach();
    });
}

/// Clears `profile`'s cached metadata: the store closes, its files are deleted (on the blocking
/// pool), the explorer shows an empty catalog and, when connected, reads it again. The task
/// answers whether the files were deleted.
pub fn clear(profile: ProfileId, cx: &mut App) -> Task<bool> {
    let Some(rt) = cx
        .try_global::<crate::connections::ConnectionsModel>()
        .map(|m| m.runtime())
    else {
        return Task::ready(false);
    };
    let dir = cx.try_global::<AppState>().and_then(|s| {
        s.paths
            .catalog_db(&profile.0.to_string(), 1)
            .parent()
            .map(PathBuf::from)
    });
    let Some(dir) = dir else {
        return Task::ready(false);
    };
    let name = profile_name(profile, cx).unwrap_or_default();
    cx.spawn(async move |cx| {
        let release = cx.update(|cx| live::release_cache(profile, cx));
        release.await;
        let (tx, rx) = oneshot::channel();
        rt.spawn_blocking(move || {
            let _ = tx.send(CacheStore::delete_profile_files(&dir, &profile));
        });
        let deleted = matches!(rx.await, Ok(Ok(())));
        cx.update(|cx| {
            if deleted {
                // The explorer shows no cached tree any more; a connected profile reads it again.
                crate::explorer::adapter::set_catalog(profile, None, None, cx);
            }
            live::reopen(profile, deleted, cx);
            forget_size(profile, cx);
            let kind = if deleted {
                ToastKind::Info
            } else {
                ToastKind::Error
            };
            let id = if deleted {
                "hv-metadata-cleared"
            } else {
                "hv-metadata-clear-failed"
            };
            ToastQueue::push(
                cx,
                Toast::new(kind, text(id, vec![("name", ArgValue::Text(name))])),
            );
            cx.refresh_windows();
        });
        deleted
    })
}

// ---- the explorer command ---------------------------------------------------------------

fn cursor_profile(cx: &App) -> Option<ProfileId> {
    let tree = crate::connections::explorer::tree(cx)?;
    let tree = tree.read(cx);
    let cursor = tree.cursor()?;
    tree.model().profile_of(cursor)
}

/// `metadata.clear_cache`: asks to clear the cursor row's connection's cached metadata.
pub fn clear_cache_command(_: &mut Window, cx: &mut App) {
    if let Some(p) = cursor_profile(cx) {
        ask_clear(p, cx);
    }
}

/// Whether `metadata.clear_cache` can run: the cursor is on a connection's row or below it.
pub fn can_clear_cache(cx: &App) -> bool {
    cursor_profile(cx).is_some()
}
