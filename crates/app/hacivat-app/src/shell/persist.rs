//! Ordered background saves: only the latest submitted state is written, writes never overlap,
//! and a pending state can be flushed synchronously (app quit, window close).
//!
//! Every submit replaces the state in one slot. A background write takes whatever the slot holds
//! when it runs, under the slot's lock, so an older state can never land after a newer one; a
//! write that finds the slot empty (a later write or a flush took it) does nothing.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hacivat_ui_core::gpui_surface::{App, AppContext as _, Task};

/// Writes `T` to disk; the error is logged by the caller's write function.
pub type WriteFn<T> = fn(&T);

pub struct LatestWriter<T> {
    slot: Arc<Mutex<Option<T>>>,
    write: WriteFn<T>,
}

impl<T> Clone for LatestWriter<T> {
    fn clone(&self) -> Self {
        Self {
            slot: self.slot.clone(),
            write: self.write,
        }
    }
}

impl<T: Send + 'static> LatestWriter<T> {
    pub fn new(write: WriteFn<T>) -> Self {
        Self {
            slot: Arc::default(),
            write,
        }
    }

    /// Makes `value` the state to write and writes it on the background executor after
    /// `delay`. Dropping the returned task cancels only its own wait; the state stays pending
    /// for a later write or [`flush`](Self::flush).
    pub fn submit(&self, value: T, delay: Option<Duration>, cx: &App) -> Task<()> {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
        let slot = self.slot.clone();
        let write = self.write;
        let executor = cx.background_executor().clone();
        cx.background_spawn(async move {
            if let Some(delay) = delay {
                executor.timer(delay).await;
            }
            write_pending(&slot, write);
        })
    }

    /// Writes the pending state, if any, on this thread.
    pub fn flush(&self) {
        write_pending(&self.slot, self.write);
    }

    /// Whether a state waits to be written.
    pub fn is_pending(&self) -> bool {
        self.slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

fn write_pending<T>(slot: &Mutex<Option<T>>, write: WriteFn<T>) {
    let mut guard = slot.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(value) = guard.take() {
        // The lock is held while writing, so writes are serialized in submit order.
        write(&value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static WRITTEN: StdMutex<Vec<u32>> = StdMutex::new(Vec::new());

    fn record(v: &u32) {
        WRITTEN.lock().unwrap().push(*v);
    }

    #[test]
    fn a_stale_write_finds_nothing_and_flush_writes_the_latest() {
        let w = LatestWriter::new(record);
        *w.slot.lock().unwrap() = Some(1);
        *w.slot.lock().unwrap() = Some(2);
        w.flush();
        // The write that was scheduled for state 1 runs late and must not write anything.
        write_pending(&w.slot, w.write);
        assert_eq!(*WRITTEN.lock().unwrap(), vec![2]);
        assert!(!w.is_pending());
    }
}
