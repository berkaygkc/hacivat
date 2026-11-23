//! A console tab's decoration (Task 32): the connection name as its title (data, ` (n)` from a
//! profile's second console), the environment swatch, the production hairline.
//!
//! An unbound console keeps the kind's title ("Console 1"), no mark and no hairline, so the M2
//! shots do not move (R-M3-37). Read on every render: two map lookups and one profile lookup.

use hacivat_l10n::{Count, Text, ids};
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_session::profile::Profile;
use hacivat_ui::panes::TabId;
use hacivat_ui::text::TextSource;
use hacivat_ui_core::gpui_surface::{App, SharedString};
use std::sync::Arc;

use crate::connections::capsule::env_kind;
use crate::connections::model::ConnectionsModel;
use crate::console::registry::Consoles;
use crate::console::view::host_binding;
use crate::l10n_global::tr;
use crate::shell::TabDecor;

/// The saved profile `id`, while the connections model knows it.
pub(crate) fn profile_of(cx: &App, id: ProfileId) -> Option<Arc<Profile>> {
    cx.try_global::<ConnectionsModel>()?.get(id)
}

/// The title of a console bound to `profile` that is its `slot`-th console (from 1): the
/// connection's name, with ` (n)` from the second on. Data: never pseudo-localised.
pub fn console_title(cx: &App, profile: &Profile, slot: u32) -> SharedString {
    if slot <= 1 {
        return SharedString::from(profile.name.clone());
    }
    tr(
        cx,
        ids::hv::CONSOLE_TAB_TITLE,
        ids::hv::HvConsoleTabTitle {
            name: Text(profile.name.clone()),
            n: Count(u64::from(slot)),
        },
    )
}

/// The `tab_decor` hook.
pub fn tab_decor(tab: TabId, cx: &App) -> TabDecor {
    // ## m4b-t14-tabs (named exception): a script tab's title and its connection's mark, from
    // the profile as it is now (never a console's title).
    if let Some(doc) = crate::scripts::tabs::doc_of_tab(tab, cx) {
        return crate::scripts::tabs::tab_decor(tab, doc.read(cx).id, cx);
    }
    // The host's binding; a console registered for the tab without a host of ours (a test
    // console) is its profile's first.
    let binding = host_binding(cx, tab).or_else(|| {
        let state = Consoles::get(cx, Consoles::by_tab(cx, tab)?)?;
        Some((state.profile?, 1))
    });
    let Some((id, slot)) = binding else {
        return TabDecor::default();
    };
    let Some(profile) = profile_of(cx, id) else {
        return TabDecor::default();
    };
    TabDecor {
        title: Some(console_title(cx, &profile, slot)),
        title_source: TextSource::Data,
        // No swatch for a connection without an environment.
        env: profile.environment.map(|tag| env_kind(Some(tag))),
        hairline: profile.environment == Some(EnvTag::Production),
        // ## m4b-t21-shots (named line)
        title_parts: None,
    }
}
