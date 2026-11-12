//! Booting the connections: the core runtime, the profile store, the secret store and the
//! drivers, prepared before GPUI starts (Task 12).

use std::path::PathBuf;
use std::sync::Arc;

use hacivat_driver::DriverRegistry;
use hacivat_secrets::SecretStore;
use hacivat_session::profile::{LoadedProfiles, ProfileStore};
use hacivat_session::{CoreRuntime, default_drivers};
use hacivat_settings::LocalPaths;
use hacivat_ui_core::gpui_surface::App;

/// Everything the connections need, built before the window opens (M2a, P17).
pub struct ConnectionsBoot {
    pub runtime: Arc<CoreRuntime>,
    pub store: ProfileStore,
    pub loaded: LoadedProfiles,
    pub secrets: Arc<SecretStore>,
    pub drivers: Arc<DriverRegistry>,
    /// Where forwarder sockets are created: a private `0700` directory (R18). The Task 1 stub
    /// puts `std::env::temp_dir()` here as a placeholder only; Task 12 must create and use a
    /// private `0700` directory for the forwarder's Unix sockets.
    pub private_dir: PathBuf,
}

impl ConnectionsBoot {
    /// The app's boot: `connections.toml` under `paths`, the OS keyring (`MemoryBackend` when
    /// `HACIVAT_SECRETS=memory`) and the default drivers (Task 12). The stub has no secret
    /// backend.
    pub fn load(paths: &LocalPaths) -> ConnectionsBoot {
        boot_with(paths, default_drivers())
    }

    /// A boot for tests: a memory secret backend and `drivers` (Task 12).
    pub fn for_tests(paths: &LocalPaths, drivers: DriverRegistry) -> ConnectionsBoot {
        boot_with(paths, drivers)
    }
}

/// The stub boot shared by `load` and `for_tests` (Task 12 replaces it).
fn boot_with(paths: &LocalPaths, drivers: DriverRegistry) -> ConnectionsBoot {
    let runtime = CoreRuntime::new().expect("the core runtime starts");
    let (store, loaded) = ProfileStore::open(paths.connections_toml());
    ConnectionsBoot {
        runtime: Arc::new(runtime),
        store,
        loaded,
        secrets: Arc::new(SecretStore::new(None)),
        drivers: Arc::new(drivers),
        // Placeholder (not private): Task 12 replaces it with a `0700` directory (R18).
        private_dir: std::env::temp_dir(),
    }
}

/// Sets the `ConnectionsModel` Global from `boot` and shows the store diagnostics as toasts
/// (Task 12). The stub does nothing.
pub fn install(_cx: &mut App, _boot: ConnectionsBoot) {}
