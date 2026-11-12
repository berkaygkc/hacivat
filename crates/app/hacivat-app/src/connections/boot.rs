//! Booting the connections: the core runtime, the profile store, the secret store and the
//! drivers, prepared before GPUI starts (Task 12).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hacivat_driver::{DriverRegistry, EngineCatalog};
use hacivat_secrets::{CredentialFiles, KeyringBackend, MemoryBackend, SecretBackend, SecretStore};
use hacivat_session::pipeline::{ApprovedRoots, ConnectDeps};
use hacivat_session::profile::{LoadedProfiles, ProfileStore};
use hacivat_session::{ConnectionManager, CoreRuntime, ManagerConfig, default_drivers};
use hacivat_settings::LocalPaths;
use hacivat_ui_core::gpui_surface::App;

use crate::connections::model::{ConnectionsModel, push_store_toasts};

/// The forwarder socket directory's name under the app's local data (R18).
const PRIVATE_DIR_NAME: &str = "fwd";

/// The longest private directory path that leaves room for a forwarder socket inside it
/// (`/hv-fwd-<pid>-<n>/fwd.sock`) within the Unix socket path limit (104 bytes on macOS).
const MAX_PRIVATE_DIR_LEN: usize = 70;

/// Everything the connections need, built before the window opens (M2a, P17).
pub struct ConnectionsBoot {
    pub runtime: Arc<CoreRuntime>,
    pub store: ProfileStore,
    pub loaded: LoadedProfiles,
    pub secrets: Arc<SecretStore>,
    pub drivers: Arc<DriverRegistry>,
    /// Where forwarder sockets are created: a private `0700` directory under the app's local
    /// data (R18), see [`private_dir_for`].
    pub private_dir: PathBuf,
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

/// The private directory for forwarder sockets (R18): `<data>/fwd`, or `<cache>/fwd` when the
/// data path is too long for a socket path. It is created `0700` (an existing one is set to
/// `0700`). A failure is logged; the forwarder creates the directory `0700` itself when it
/// binds.
pub fn private_dir_for(paths: &LocalPaths) -> PathBuf {
    let preferred = paths.data.join(PRIVATE_DIR_NAME);
    let dir = if preferred.as_os_str().len() <= MAX_PRIVATE_DIR_LEN {
        preferred
    } else {
        paths.cache.join(PRIVATE_DIR_NAME)
    };
    if let Err(err) = make_private(&dir) {
        tracing::warn!(dir = %dir.display(), %err, "connections: preparing the socket directory failed");
    }
    dir
}

fn make_private(dir: &Path) -> std::io::Result<()> {
    hacivat_settings::fsio::create_dir_all_private(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
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
        private_dir: private_dir_for(paths),
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
        private_dir,
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
        private_dir,
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
