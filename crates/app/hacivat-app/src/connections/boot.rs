//! Booting the connections: the core runtime, the profile store, the secret store and the
//! drivers, prepared before GPUI starts (Task 12).

use std::sync::Arc;

use hacivat_driver::{DriverRegistry, EngineCatalog};
use hacivat_secrets::{CredentialFiles, KeyringBackend, MemoryBackend, SecretBackend, SecretStore};
use hacivat_session::pipeline::{ApprovedRoots, ConnectDeps};
use hacivat_session::profile::{LoadedProfiles, ProfileStore};
use hacivat_session::{ConnectionManager, CoreRuntime, ManagerConfig, default_drivers};
use hacivat_settings::LocalPaths;
use hacivat_ui_core::gpui_surface::App;

use crate::connections::model::{ConnectionsModel, push_store_toasts};

/// Everything the connections need, built before the window opens (M2a, P17).
pub struct ConnectionsBoot {
    pub runtime: Arc<CoreRuntime>,
    pub store: ProfileStore,
    pub loaded: LoadedProfiles,
    pub secrets: Arc<SecretStore>,
    pub drivers: Arc<DriverRegistry>,
    /// The credential files connects read (`.pgpass`, `pg_service.conf`); none in tests.
    pub files: CredentialFiles,
}

impl ConnectionsBoot {
    /// The app's boot: `connections.toml` under `paths`, the OS keyring (`MemoryBackend` when
    /// `HACIVAT_SECRETS=memory`), the default drivers and the credential files the environment
    /// names. The keyring is not touched here.
    pub fn load(paths: &LocalPaths) -> ConnectionsBoot {
        let choice = std::env::var("HACIVAT_SECRETS").ok();
        boot_with(
            paths,
            default_drivers(),
            secret_backend_from(choice.as_deref()),
            CredentialFiles::from_env(),
        )
    }

    /// A boot for tests: a memory secret backend, `drivers` and no credential files.
    pub fn for_tests(paths: &LocalPaths, drivers: DriverRegistry) -> ConnectionsBoot {
        boot_with(
            paths,
            drivers,
            Arc::new(MemoryBackend::new()),
            CredentialFiles {
                pgpass: None,
                pg_service: None,
            },
        )
    }
}

/// The secret backend for `HACIVAT_SECRETS`: `memory` gives a `MemoryBackend` (dev and test
/// runs that must not touch the keychain); anything else the OS keyring.
pub fn secret_backend_from(choice: Option<&str>) -> Arc<dyn SecretBackend> {
    match choice {
        Some(c) if c.eq_ignore_ascii_case("memory") => Arc::new(MemoryBackend::new()),
        _ => Arc::new(KeyringBackend::new(None)),
    }
}

fn boot_with(
    paths: &LocalPaths,
    drivers: DriverRegistry,
    backend: Arc<dyn SecretBackend>,
    files: CredentialFiles,
) -> ConnectionsBoot {
    let runtime = CoreRuntime::new().expect("the core runtime starts");
    let (store, loaded) = ProfileStore::open(paths.connections_toml());
    ConnectionsBoot {
        runtime: Arc::new(runtime),
        store,
        loaded,
        secrets: Arc::new(SecretStore::new(Some(backend))),
        drivers: Arc::new(drivers),
        files,
    }
}

/// Sets the `ConnectionsModel` Global from `boot` (with its `ConnectionManager`) and shows the
/// store diagnostics as toasts, one per kind (P29): the quarantined file; a file that cannot
/// be read or set aside; the skipped entries (invalid, duplicate ID, unknown engine) as one
/// count; the ignored values as one count; an ignored password (never echoed); a newer file
/// (read-only). It never touches the keyring. A pending profile write is flushed
/// at quit.
pub fn install(cx: &mut App, boot: ConnectionsBoot) {
    let ConnectionsBoot {
        runtime,
        store,
        loaded,
        secrets,
        drivers,
        files,
    } = boot;
    // ## m2a-t17-dialog: the dialog's file pickers approve into the roots connects check.
    let approved_roots = ApprovedRoots::for_user();
    cx.set_global(crate::connections::dialog::DialogEnv {
        approved_roots: approved_roots.clone(),
        pgpass: files.pgpass.clone(),
    });
    let deps = ConnectDeps {
        drivers,
        catalog: EngineCatalog::embedded(),
        secrets: secrets.clone(),
        files,
        transport_wrap: None,
        extra_steps: Vec::new(),
        // Home plus paths the user picks in the connection dialog (T17 adds them).
        approved_roots,
        timeout_override: None,
    };
    let manager =
        ConnectionManager::new(runtime.handle(), Arc::new(deps), ManagerConfig::default());
    let LoadedProfiles {
        profiles,
        diags,
        quarantined,
    } = loaded;
    cx.set_global(ConnectionsModel::new(
        runtime, store, profiles, secrets, manager,
    ));
    cx.on_app_quit(|cx| {
        if let Some(m) = cx.try_global::<ConnectionsModel>() {
            m.flush_writes();
        }
        async {}
    })
    .detach();

    push_store_toasts(cx, &diags, quarantined.as_deref());
}
