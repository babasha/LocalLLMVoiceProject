use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Shared shutdown flag for all pipeline threads (lock-free to check in hot loops).
#[derive(Clone, Default)]
pub struct ShutdownSignal {
    flag: Arc<AtomicBool>,
}

impl ShutdownSignal {
    pub fn new() -> Self {
        Self::default()
    }

    /// Signal shutdown to all threads.
    pub fn shutdown(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Check if shutdown has been requested.
    pub fn is_shutdown(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}
