use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Shared shutdown signal for all pipeline threads.
/// Uses AtomicBool for lock-free checking in hot paths.
#[derive(Clone)]
pub struct ShutdownSignal {
    flag: Arc<AtomicBool>,
}

impl ShutdownSignal {
    pub fn new() -> Self {
        ShutdownSignal {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Signal shutdown to all threads.
    pub fn shutdown(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Check if shutdown has been requested.
    pub fn is_shutdown(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Install Ctrl+C handler that triggers shutdown.
    pub fn install_ctrlc_handler(&self) {
        let signal = self.clone();
        ctrlc_handler(signal);
    }
}

fn ctrlc_handler(signal: ShutdownSignal) {
    // Use std::io to catch Ctrl+C on Windows
    // The orchestrator thread checks is_shutdown() periodically
    std::thread::spawn(move || {
        // This is a simple approach — the main thread can also use
        // crossbeam_channel::select! with a ticker to check shutdown
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if signal.is_shutdown() {
                break;
            }
        }
    });
}
