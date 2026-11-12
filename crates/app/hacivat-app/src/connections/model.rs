//! `ConnectionsModel`: the profiles, their live states and the one ordered writer of
//! `connections.toml` (Task 12). Profile writes go only through it.

use std::collections::HashMap;
use std::sync::Arc;

use hacivat_proto::ProfileId;
use hacivat_secrets::{SecretError, SecretStore};
use hacivat_session::profile::{Profile, ProfileDraft};
use hacivat_session::{ConnectionManager, LiveState, TestHandle};
use hacivat_ui_core::gpui_surface::{App, Global, Task};
use secrecy::SecretString;
use tokio::runtime::Handle;

use crate::connections::interaction::interaction;

/// Why a profile write failed (M2a).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SaveError {
    StoreReadOnly,
    Io(String),
    Secret(SecretError),
}

/// The app's connections (a Global, M2a).
pub struct ConnectionsModel {
    profiles: Vec<Arc<Profile>>,
    live: HashMap<ProfileId, LiveState>,
    selected: Option<ProfileId>,
    current: Option<ProfileId>,
    manager: Arc<ConnectionManager>,
    secrets: Arc<SecretStore>,
    runtime: Handle,
    read_only: bool,
}

impl Global for ConnectionsModel {}

impl ConnectionsModel {
    pub fn profiles(&self) -> &[Arc<Profile>] {
        &self.profiles
    }

    pub fn get(&self, id: ProfileId) -> Option<Arc<Profile>> {
        self.profiles.iter().find(|p| p.id == id).cloned()
    }

    /// The mirrored live state of `id`.
    pub fn live(&self, id: ProfileId) -> LiveState {
        self.live.get(&id).cloned().unwrap_or_default()
    }

    pub fn set_live(&mut self, id: ProfileId, state: LiveState) {
        self.live.insert(id, state);
    }

    /// The profile selected in the explorer.
    pub fn selected(&self) -> Option<ProfileId> {
        self.selected
    }

    /// Selects `id` in the explorer (Task 12).
    pub fn select(_cx: &mut App, _id: Option<ProfileId>) {}

    /// The profile the capsule shows: the last one connected or attempted.
    pub fn current(&self) -> Option<ProfileId> {
        self.current
    }

    /// Makes `id` current (a connect was attempted).
    pub fn note_attempt(&mut self, id: ProfileId) {
        self.current = Some(id);
    }

    /// Saves `draft` (and its password, by its retention) (Task 12).
    pub fn save(_cx: &mut App, _draft: ProfileDraft) -> Task<Result<ProfileId, SaveError>> {
        Task::ready(Err(SaveError::StoreReadOnly))
    }

    /// Deletes `id` and its secrets (Task 12).
    pub fn delete(_cx: &mut App, _id: ProfileId) -> Task<Result<(), SaveError>> {
        Task::ready(Err(SaveError::StoreReadOnly))
    }

    /// Saves a copy of `id` under a unique name (Task 12).
    pub fn duplicate(_cx: &mut App, _id: ProfileId) -> Task<Result<ProfileId, SaveError>> {
        Task::ready(Err(SaveError::StoreReadOnly))
    }

    /// Connects `id` through the manager (Task 12).
    pub fn connect(_cx: &mut App, _id: ProfileId, _draft_password: Option<SecretString>) {}

    pub fn disconnect(_cx: &mut App, _id: ProfileId) {}

    pub fn cancel_connect(_cx: &mut App, _id: ProfileId) {}

    /// Tests `draft` through the manager. Needs the model (call it only after `boot`).
    pub fn test(cx: &mut App, draft: ProfileDraft) -> TestHandle {
        let ix = interaction(cx);
        cx.global::<ConnectionsModel>().manager.test(draft, ix)
    }

    pub fn manager(&self) -> &Arc<ConnectionManager> {
        &self.manager
    }

    pub fn secrets(&self) -> &Arc<SecretStore> {
        &self.secrets
    }

    /// The core runtime's handle.
    pub fn runtime(&self) -> Handle {
        self.runtime.clone()
    }

    /// Whether `connections.toml` cannot be written (P29).
    pub fn store_read_only(&self) -> bool {
        self.read_only
    }
}
