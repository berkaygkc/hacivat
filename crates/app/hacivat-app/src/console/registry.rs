//! The console registry (working since Task 1b): every console's state, by its ID and its tab,
//! and the focused one.

use std::collections::HashMap;
use std::sync::Arc;

use hacivat_exec::SessionLog;
use hacivat_proto::ProfileId;
use hacivat_session::session::{ClientId, SessionHandle};
use hacivat_ui::panes::TabId;
use hacivat_ui_core::gpui_surface::{App, Entity, Global};
use hacivat_ui_editor::EditorView;
use sql_dialect::DialectCtx;

use crate::results::ConsoleResults;

/// A console's ID, unique in the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConsoleId(pub u64);

/// One console: its tab and number, its binding and client, its session, editor and results,
/// its SQL context and its own output log.
#[derive(Debug, Clone)]
pub struct ConsoleState {
    pub id: ConsoleId,
    pub tab: TabId,
    pub n: u32,
    pub profile: Option<ProfileId>,
    pub client: ClientId,
    pub session: Option<SessionHandle>,
    pub editor: Option<Entity<EditorView>>,
    pub results: Option<Entity<ConsoleResults>>,
    pub sql: Option<DialectCtx>,
    pub log: Arc<SessionLog>,
}

/// Every console (a global).
#[derive(Debug, Default)]
pub struct Consoles {
    by_tab: HashMap<TabId, ConsoleId>,
    states: HashMap<ConsoleId, ConsoleState>,
    focused: Option<ConsoleId>,
    next: u64,
}

impl Global for Consoles {}

impl Consoles {
    /// Registers `s` under a fresh ID (the registry assigns `s.id`), keyed by its tab; returns
    /// the ID. A console registered for a tab replaces the tab's earlier one.
    pub fn register(cx: &mut App, mut s: ConsoleState) -> ConsoleId {
        let c = cx.default_global::<Consoles>();
        c.next += 1;
        let id = ConsoleId(c.next);
        s.id = id;
        if let Some(old) = c.by_tab.insert(s.tab, id) {
            c.states.remove(&old);
        }
        c.states.insert(id, s);
        id
    }

    pub fn get(cx: &App, id: ConsoleId) -> Option<&ConsoleState> {
        cx.try_global::<Consoles>()?.states.get(&id)
    }

    /// Changes console `id`'s state (nothing when it is gone).
    pub fn update(cx: &mut App, id: ConsoleId, f: impl FnOnce(&mut ConsoleState)) {
        if let Some(s) = cx.default_global::<Consoles>().states.get_mut(&id) {
            f(s);
        }
    }

    /// The console of `tab`.
    pub fn by_tab(cx: &App, tab: TabId) -> Option<ConsoleId> {
        cx.try_global::<Consoles>()?.by_tab.get(&tab).copied()
    }

    /// Forgets console `id` (its tab closed).
    pub fn remove(cx: &mut App, id: ConsoleId) {
        let c = cx.default_global::<Consoles>();
        if let Some(s) = c.states.remove(&id) {
            c.by_tab.remove(&s.tab);
        }
        if c.focused == Some(id) {
            c.focused = None;
        }
    }

    /// The console the user last focused (`focused_console` prefers the focused editor).
    pub fn focused(cx: &App) -> Option<ConsoleId> {
        cx.try_global::<Consoles>()?.focused
    }

    pub fn set_focused(cx: &mut App, id: Option<ConsoleId>) {
        cx.default_global::<Consoles>().focused = id;
    }

    /// Every console's ID, in no particular order.
    pub fn ids(cx: &App) -> Vec<ConsoleId> {
        cx.try_global::<Consoles>()
            .map(|c| c.states.keys().copied().collect())
            .unwrap_or_default()
    }
}
