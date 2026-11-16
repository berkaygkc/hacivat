//! The core runtime: the Tokio runtime every driver, network, file and keyring call runs on,
//! never the UI thread (§5.4). Working since Task 1.

use std::io;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::runtime::{Builder, Handle, Runtime};

/// The most worker threads the core runtime starts.
const MAX_WORKERS: usize = 4;

/// The Tokio runtime of the core layer, threads named `hv-core-<n>` (§5.4, P15). It shuts
/// down with a budget, never by a plain drop, so a blocked keychain call cannot hang the exit.
pub struct CoreRuntime {
    rt: Mutex<Option<Runtime>>,
    handle: Handle,
}

impl CoreRuntime {
    /// A multi-thread runtime with `min(4, available_parallelism)` workers. Installs the
    /// rustls ring provider first (P5).
    pub fn new() -> io::Result<CoreRuntime> {
        hacivat_net::install_crypto_provider();
        let workers = std::thread::available_parallelism()
            .map_or(1, NonZeroUsize::get)
            .min(MAX_WORKERS);
        let next = AtomicUsize::new(0);
        let rt = Builder::new_multi_thread()
            .worker_threads(workers)
            .thread_name_fn(move || format!("hv-core-{}", next.fetch_add(1, Ordering::SeqCst)))
            .enable_all()
            .build()?;
        let handle = rt.handle().clone();
        Ok(CoreRuntime {
            rt: Mutex::new(Some(rt)),
            handle,
        })
    }

    /// The handle to spawn on. After `shutdown`, spawned tasks never run.
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }

    /// Stops the runtime, waiting at most `budget` for its tasks (P15). Later calls do nothing.
    pub fn shutdown(&self, budget: Duration) {
        let rt = self
            .rt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(rt) = rt {
            rt.shutdown_timeout(budget);
        }
    }
}

impl Drop for CoreRuntime {
    /// A runtime that was never shut down is let go in the background, never waited for.
    fn drop(&mut self) {
        let rt = self
            .rt
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(rt) = rt {
            rt.shutdown_background();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn workers_are_named_and_shutdown_is_bounded() {
        let core = CoreRuntime::new().unwrap();
        let name = core
            .handle()
            .block_on(async {
                tokio::spawn(async { std::thread::current().name().map(str::to_string) })
                    .await
                    .unwrap()
            })
            .unwrap();
        assert!(name.starts_with("hv-core-"), "{name}");

        // A task stuck in a blocking call does not hold the shutdown past its budget.
        core.handle()
            .spawn_blocking(|| std::thread::sleep(Duration::from_secs(5)));
        let started = Instant::now();
        core.shutdown(Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_secs(2));
        core.shutdown(Duration::from_millis(100)); // a second call is a no-op
    }
}
