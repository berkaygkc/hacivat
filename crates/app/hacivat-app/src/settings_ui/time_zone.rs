// ## m4-t32-shots (named exception; PO ruling R-W5-14 a)
//! Settings › General › Time zone (`ui.time_zone`): a Select of "System (Zone)" (the zone the
//! operating system reports, `UTC` when it reports none), UTC, and a fixed list of IANA zones.
//! A hand-edited zone that is not in the list shows as one more choice (as a hand-edited tab
//! size does), so the select always names what is saved.
//!
//! A choice is saved through `shell::update_settings` (the one writer) and takes effect at
//! once: the formats are rebuilt with the new zone (`switch_locale`), and the connection
//! manager follows the setting (`connections::boot::follow_time_zone`): connections opened from
//! then on use it, open ones keep theirs.

use std::rc::Rc;

use hacivat_l10n::{ArgValue, ids};
use hacivat_settings::{Settings, TimeZonePref};
use hacivat_ui::select::{Select, SelectEvent, SelectOption};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Styled as _, Subscription, div, px,
};

use super::SettingsPage;
use crate::l10n_global::{HostLocale, switch_locale, text_fn};
use crate::shell::{AppState, update_settings};

/// The row key.
pub const TIME_ZONE: &str = "ui.time_zone";
/// The select's width (the longest zone and "System (America/Los_Angeles)" fit).
const SELECT_PX: f32 = 240.0;
/// The stored value of the system choice and of UTC.
const SYSTEM: &str = "system";
const UTC: &str = "UTC";

/// The zones offered after System and UTC: one or two per inhabited region, west to east.
pub const ZONES: [&str; 21] = [
    "America/Los_Angeles",
    "America/Denver",
    "America/Chicago",
    "America/New_York",
    "America/Sao_Paulo",
    "Europe/London",
    // ## m4-t32-shots (fix round 1, review N-3, controller ruling): Africa, Moscow, Jakarta.
    "Africa/Lagos",
    "Europe/Berlin",
    "Europe/Paris",
    "Africa/Cairo",
    "Africa/Johannesburg",
    "Europe/Istanbul",
    "Europe/Moscow",
    "Asia/Dubai",
    "Asia/Kolkata",
    "Asia/Jakarta",
    "Asia/Singapore",
    "Asia/Shanghai",
    "Asia/Tokyo",
    "Australia/Sydney",
    "Pacific/Auckland",
];

/// The stored value of `pref` (`system`, or the IANA name).
pub fn value(pref: &TimeZonePref) -> String {
    match pref {
        TimeZonePref::System => SYSTEM.to_string(),
        TimeZonePref::Iana(z) => z.clone(),
    }
}

/// The zone the operating system reports (`UTC` when it reports none, as the formats do).
pub fn system_zone(cx: &App) -> String {
    cx.try_global::<HostLocale>()
        .and_then(|h| h.0.time_zone.clone())
        .filter(|z| !z.trim().is_empty())
        .unwrap_or_else(|| UTC.to_string())
}

/// The select's values, in order: System, UTC, [`ZONES`], and the saved zone when it is none
/// of those.
pub fn values(saved: &TimeZonePref) -> Vec<String> {
    let mut out: Vec<String> = [SYSTEM, UTC]
        .iter()
        .chain(ZONES.iter())
        .map(|s| (*s).to_string())
        .collect();
    let saved = value(saved);
    if !out.contains(&saved) {
        out.push(saved);
    }
    out
}

// ## m4-t32-shots (fix round 1, review M-3)
/// The zone sessions and formats use for `pref`: a well-formed name the tz database does not
/// know fails closed to the system zone (never sent to a server).
pub fn session_zone(pref: &TimeZonePref) -> TimeZonePref {
    match pref {
        TimeZonePref::Iana(z) if !hacivat_l10n::effective::is_known_zone(z) => TimeZonePref::System,
        other => other.clone(),
    }
}

/// The saved zone when it is one the tz database does not know (the row says so).
pub fn unknown_zone(settings: &Settings) -> Option<String> {
    match &settings.ui.time_zone {
        TimeZonePref::Iana(z) if !hacivat_l10n::effective::is_known_zone(z) => Some(z.clone()),
        _ => None,
    }
}

/// "System (Europe/Istanbul)".
fn system_label(cx: &App) -> SharedString {
    let zone = system_zone(cx);
    crate::explorer::live::text(
        cx,
        "hv-settings-time-zone-system",
        vec![("zone", ArgValue::Text(zone))],
    )
}

/// The notice under the row while the saved zone is unknown.
pub fn unknown_note(zone: &str, cx: &App) -> SharedString {
    crate::explorer::live::text(
        cx,
        "hv-settings-time-zone-unknown",
        vec![
            ("zone", ArgValue::Text(zone.to_string())),
            ("system", ArgValue::Text(system_label(cx).to_string())),
        ],
    )
}

fn label(value: String) -> TextFn {
    match value.as_str() {
        SYSTEM => Rc::new(system_label),
        UTC => text_fn(ids::hv::SETTINGS_TIME_ZONE_UTC),
        // Fix round 1 (review M-3): a saved zone the tz database does not know says so.
        z if !hacivat_l10n::effective::is_known_zone(z) => Rc::new(move |cx: &App| {
            crate::explorer::live::text(
                cx,
                "hv-settings-time-zone-unknown-choice",
                vec![("zone", ArgValue::Text(value.clone()))],
            )
        }),
        _ => {
            let shown = SharedString::from(value);
            Rc::new(move |_: &App| shown.clone())
        }
    }
}

fn options(values: &[String]) -> Vec<SelectOption> {
    values
        .iter()
        .map(|v| SelectOption {
            label: label(v.clone()),
            hint: None,
            disabled: false,
        })
        .collect()
}

/// Saves zone `value` (`system` or an IANA name) and applies it now.
pub fn apply(value: &str, cx: &mut App) {
    let pref = if value == SYSTEM {
        TimeZonePref::System
    } else {
        TimeZonePref::Iana(value.to_string())
    };
    let saved = cx
        .try_global::<AppState>()
        .map(|s| s.settings.ui.time_zone.clone());
    if saved.as_ref() == Some(&pref) {
        return;
    }
    let settings = update_settings(cx, |s| s.ui.time_zone = pref);
    switch_locale(cx, &settings).detach();
}

/// The time zone row's select, owned by the settings page.
pub struct TimeZoneControl {
    select: Entity<Select>,
    /// The values the select offers now (they follow a hand-edited zone).
    values: Vec<String>,
    // ## m4-t33b-carries (T32 re-review RR-5): the saved zone and whether the tz database
    // knows it, looked up once per settings change ([`TimeZoneControl::follow`]), never per
    // frame (the first lookup of a name reads the zoneinfo file).
    checked: Option<(TimeZonePref, Option<String>)>,
    lookups: u64,
    _subscription: Subscription,
}

impl TimeZoneControl {
    pub fn new(cx: &mut Context<SettingsPage>) -> Self {
        let saved = cx
            .try_global::<AppState>()
            .map(|s| s.settings.ui.time_zone.clone())
            .unwrap_or_default();
        let values = values(&saved);
        let selected = values.iter().position(|v| *v == value(&saved)).unwrap_or(0);
        let opts = options(&values);
        let select = cx.new(|cx| {
            let mut s = Select::new(opts, selected, cx);
            s.set_label(text_fn(ids::hv::SETTINGS_TIME_ZONE));
            s
        });
        let subscription = cx.subscribe(&select, |page, _, e: &SelectEvent, cx| {
            let SelectEvent::Changed(i) = *e;
            if let Some(v) = page.time_zone.values.get(i).cloned() {
                apply(&v, cx);
                cx.notify();
            }
        });
        TimeZoneControl {
            select,
            values,
            checked: None,
            lookups: 0,
            _subscription: subscription,
        }
    }

    /// Before a frame: the select follows the saved zone (a hand edit, another window).
    pub fn follow(&mut self, settings: &Settings, cx: &mut App) {
        // ## m4-t33b-carries (RR-5): the unknown-zone check, once per change of the setting.
        if self.checked.as_ref().map(|(p, _)| p) != Some(&settings.ui.time_zone) {
            self.lookups += 1;
            self.checked = Some((settings.ui.time_zone.clone(), unknown_zone(settings)));
        }
        let want = values(&settings.ui.time_zone);
        if want != self.values {
            let opts = options(&want);
            self.select.update(cx, |s, cx| s.set_options(opts, cx));
            self.values = want;
        }
        let index = self
            .values
            .iter()
            .position(|v| *v == value(&settings.ui.time_zone))
            .unwrap_or(0);
        self.select.update(cx, |s, cx| {
            if s.selected() != index {
                s.set_selected(index, cx);
            }
        });
    }

    /// The row's control.
    pub fn control(&self) -> AnyElement {
        div()
            .debug_selector(|| "settings-time-zone".to_string())
            .flex_none()
            .w(px(SELECT_PX))
            .child(self.select.clone())
            .into_any_element()
    }

    /// The select (tests, shots).
    pub fn select(&self) -> &Entity<Select> {
        &self.select
    }

    // ## m4-t33b-carries (RR-5)
    /// The saved zone when the tz database does not know it, as of the last
    /// [`TimeZoneControl::follow`].
    pub fn unknown(&self) -> Option<&str> {
        self.checked.as_ref().and_then(|(_, z)| z.as_deref())
    }

    /// How many times the control looked the saved zone up (tests).
    pub fn lookups(&self) -> u64 {
        self.lookups
    }
}
