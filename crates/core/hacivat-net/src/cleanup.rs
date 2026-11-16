//! Cleanup guards: async teardown a connect step leaves behind (a forwarder, a tunnel), run in
//! reverse order when the connection closes (HV-CONN-01). Working since Task 1.

use std::fmt;

use futures::future::BoxFuture;

/// One teardown action, run at most once (HV-CONN-01).
pub struct CleanupGuard(Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>);

impl CleanupGuard {
    pub fn new(f: impl FnOnce() -> BoxFuture<'static, ()> + Send + 'static) -> Self {
        CleanupGuard(Box::new(f))
    }

    /// Runs the teardown.
    pub fn run(self) -> BoxFuture<'static, ()> {
        (self.0)()
    }
}

impl fmt::Debug for CleanupGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CleanupGuard")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn a_guard_runs_its_teardown() {
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let guard = CleanupGuard::new(move || {
            Box::pin(async move {
                flag.store(true, Ordering::SeqCst);
            })
        });
        assert!(!ran.load(Ordering::SeqCst));
        guard.run().await;
        assert!(ran.load(Ordering::SeqCst));
    }
}
