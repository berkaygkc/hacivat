//! The connection-event relay (M2b, Q4 ruling, Q42): `ConnectionManager::events()` is a
//! `tokio::sync::broadcast` on the core runtime. A relay task there forwards each event into an
//! `mpsc` channel that a GPUI foreground task awaits (the `AppUiPort` pattern; no polling), and
//! each event goes to its handler:
//!
//! - `SessionLost`, `IdleDisconnected`, `IdleTransaction`, `EnforcementGap` → the session
//!   toasts (Task 22);
//! - `ProfileChanges` → `ConnectionsModel::apply_changes` (Task 19).
//!
//! When the relay falls behind (`RecvError::Lagged`) some events are gone, so the live states
//! are read again from the manager instead (Q42).

use hacivat_session::ConnectionEvent;
use hacivat_ui_core::gpui_surface::App;
#[cfg(feature = "test-support")]
use hacivat_ui_core::gpui_surface::Global;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use crate::connections::model::ConnectionsModel;
use crate::connections::{live, session_toasts};

/// What the relay forwards.
enum Relayed {
    Event(ConnectionEvent),
    /// Events were dropped: re-read the states.
    Lagged,
}

/// How many events reached a handler (test seam, feature `test-support`).
#[cfg(feature = "test-support")]
#[derive(Default)]
struct HandledEvents(usize);

#[cfg(feature = "test-support")]
impl Global for HandledEvents {}

/// Starts the relay and the foreground task (from `connections::boot`, after the model is set).
/// Without a model it does nothing.
pub fn install(cx: &mut App) {
    let Some(model) = cx.try_global::<ConnectionsModel>() else {
        return;
    };
    // Subscribed here, before any connect can send: no event is missed at boot.
    let mut events = model.manager().events();
    let (relay, mut incoming) = mpsc::unbounded_channel::<Relayed>();
    model.runtime().spawn(async move {
        loop {
            let item = match events.recv().await {
                Ok(event) => Relayed::Event(event),
                Err(RecvError::Lagged(_)) => Relayed::Lagged,
                Err(RecvError::Closed) => break,
            };
            if relay.send(item).is_err() {
                break; // the app is gone
            }
        }
    });
    #[cfg(feature = "test-support")]
    cx.set_global(HandledEvents::default());
    cx.spawn(async move |cx| {
        while let Some(item) = incoming.recv().await {
            cx.update(|cx| handle(item, cx));
        }
    })
    .detach();
}

fn handle(item: Relayed, cx: &mut App) {
    let event = match item {
        Relayed::Event(event) => event,
        Relayed::Lagged => {
            live::mirror(cx);
            return;
        }
    };
    match event {
        ConnectionEvent::SessionLost { profile, lost } => session_toasts::lost(profile, lost, cx),
        ConnectionEvent::IdleDisconnected { profile } => session_toasts::idle(profile, cx),
        ConnectionEvent::IdleTransaction { profile, minutes } => {
            session_toasts::idle_tx(profile, minutes, cx);
        }
        ConnectionEvent::EnforcementGap { profile, reason } => {
            session_toasts::gap(profile, reason, cx);
        }
        ConnectionEvent::ProfileChanges { profile, changes } => {
            let saved = ConnectionsModel::apply_changes(cx, profile, changes);
            cx.spawn(async move |_| {
                if let Err(e) = saved.await {
                    tracing::debug!(error = ?e, "connections: profile changes not saved");
                }
            })
            .detach();
        }
    }
    #[cfg(feature = "test-support")]
    {
        cx.default_global::<HandledEvents>().0 += 1;
    }
}

/// How many connection events reached their handler since boot (test seam, fix round 1 M5).
#[cfg(feature = "test-support")]
pub fn handled_events(cx: &App) -> usize {
    cx.try_global::<HandledEvents>().map_or(0, |h| h.0)
}
