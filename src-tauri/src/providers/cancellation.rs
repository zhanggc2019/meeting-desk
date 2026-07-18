use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Notify;

/// A small cloneable cancellation token that does not require another runtime dependency.
#[derive(Clone, Default)]
pub struct CancellationToken {
    inner: Arc<CancellationState>,
}

#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

impl CancellationToken {
    /// Creates a token in the active state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the token as cancelled and wakes every current waiter.
    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::AcqRel) {
            self.inner.notify.notify_waiters();
        }
    }

    /// Returns whether cancellation has already been requested.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// Waits until cancellation is requested without losing a concurrent notification.
    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }

            let notified = self.inner.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

impl fmt::Debug for CancellationToken {
    /// Formats only the cancellation state and never runtime internals.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::CancellationToken;

    /// Verifies that clones observe the same cancellation state.
    #[tokio::test]
    async fn cancellation_wakes_clones() {
        let token = CancellationToken::new();
        let waiter = token.clone();
        token.cancel();
        waiter.cancelled().await;
        assert!(waiter.is_cancelled());
    }
}
