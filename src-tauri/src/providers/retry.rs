use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{CancellationToken, OperationOutcome, ProviderError, ReplaySafety};

/// Bounded retry and backoff settings shared by HTTP providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub max_retry_after_ms: u64,
}

impl RetryPolicy {
    /// Validates retry settings and rejects unbounded or contradictory values.
    pub fn validate(&self) -> Result<(), ProviderError> {
        if self.max_retries > 10 {
            return Err(ProviderError::configuration(
                "invalid_retry_policy",
                "自动重试次数不能超过 10",
            ));
        }
        if self.base_delay_ms == 0 || self.max_delay_ms < self.base_delay_ms {
            return Err(ProviderError::configuration(
                "invalid_retry_policy",
                "重试退避配置无效",
            ));
        }
        if self.max_retry_after_ms == 0 {
            return Err(ProviderError::configuration(
                "invalid_retry_policy",
                "Retry-After 上限必须大于零",
            ));
        }
        Ok(())
    }

    /// Computes capped exponential backoff with deterministic full jitter.
    pub(crate) fn delay_for(
        &self,
        retry_index: u32,
        operation_id: &str,
        error: &ProviderError,
    ) -> Duration {
        if let Some(retry_after_ms) = error.retry_after_ms {
            return Duration::from_millis(retry_after_ms.min(self.max_retry_after_ms));
        }

        let exponent = retry_index.saturating_sub(1).min(31);
        let multiplier = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
        let cap_ms = self
            .base_delay_ms
            .saturating_mul(multiplier)
            .min(self.max_delay_ms);

        let mut hasher = DefaultHasher::new();
        operation_id.hash(&mut hasher);
        retry_index.hash(&mut hasher);
        let jitter_ms = if cap_ms == 0 {
            0
        } else {
            hasher.finish() % cap_ms.saturating_add(1)
        };
        Duration::from_millis(jitter_ms)
    }

    /// Returns whether another retry is available after the current attempt.
    pub(crate) fn has_retry(&self, attempts_completed: u32) -> bool {
        attempts_completed <= self.max_retries
    }
}

/// Returns whether an adapter's evidenced replay policy allows this failure to be retried.
pub(crate) fn is_replay_safe(
    replay_safety: ReplaySafety,
    idempotency_key_configured: bool,
    outcome: OperationOutcome,
) -> bool {
    match replay_safety {
        ReplaySafety::VerifiedAlwaysSafe => true,
        ReplaySafety::SafeWithVerifiedIdempotencyKey => idempotency_key_configured,
        ReplaySafety::BeforeRequestBodySentOnly => outcome == OperationOutcome::NotSent,
        ReplaySafety::NeverAutomaticallyReplay => false,
    }
}

/// Sleeps for a retry or rate-limit delay while observing cancellation and overall deadline.
pub(crate) async fn sleep_cancellable(
    delay: Duration,
    token: &CancellationToken,
    deadline: Instant,
) -> Result<(), ProviderError> {
    if token.is_cancelled() {
        return Err(ProviderError::cancelled());
    }

    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() || delay > remaining {
        return Err(ProviderError::operation_timeout());
    }

    tokio::select! {
        _ = token.cancelled() => Err(ProviderError::cancelled()),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}

/// A provider-local gate for minimum request spacing and shared 429 cooldown.
pub(crate) struct RateGate {
    next_allowed: Mutex<Instant>,
    min_interval: Duration,
}

impl RateGate {
    /// Creates a rate gate with an explicit minimum request interval.
    pub(crate) fn new(min_interval: Duration) -> Self {
        Self {
            next_allowed: Mutex::new(Instant::now()),
            min_interval,
        }
    }

    /// Reserves the next fair request slot and waits cancellably for it.
    pub(crate) async fn wait(
        &self,
        token: &CancellationToken,
        deadline: Instant,
    ) -> Result<(), ProviderError> {
        let delay = {
            let mut next_allowed = self.next_allowed.lock().await;
            let now = Instant::now();
            let slot = (*next_allowed).max(now);
            *next_allowed = slot + self.min_interval;
            slot.saturating_duration_since(now)
        };

        if delay.is_zero() {
            return Ok(());
        }
        sleep_cancellable(delay, token, deadline).await
    }

    /// Extends the shared cooldown after a provider rate-limit response.
    pub(crate) async fn penalize(&self, delay: Duration) {
        let mut next_allowed = self.next_allowed.lock().await;
        let penalized_until = Instant::now() + delay;
        if penalized_until > *next_allowed {
            *next_allowed = penalized_until;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_replay_safe, RetryPolicy};
    use crate::providers::{OperationOutcome, ProviderError, ProviderErrorCategory, ReplaySafety};

    /// Verifies that retries after a sent body require evidenced replay safety.
    #[test]
    fn refuses_unknown_outcome_without_idempotency() {
        assert!(!is_replay_safe(
            ReplaySafety::SafeWithVerifiedIdempotencyKey,
            false,
            OperationOutcome::Unknown,
        ));
        assert!(is_replay_safe(
            ReplaySafety::SafeWithVerifiedIdempotencyKey,
            true,
            OperationOutcome::Unknown,
        ));
    }

    /// Verifies that server delays are capped by the local safety policy.
    #[test]
    fn caps_retry_after_delay() {
        let policy = RetryPolicy {
            max_retries: 2,
            base_delay_ms: 10,
            max_delay_ms: 100,
            max_retry_after_ms: 250,
        };
        let error = ProviderError::new(
            "http_429",
            ProviderErrorCategory::RateLimit,
            true,
            true,
            "请求过于频繁",
            Some(429),
            Some(5_000),
            OperationOutcome::Rejected,
        );
        assert_eq!(
            policy.delay_for(1, "operation", &error),
            std::time::Duration::from_millis(250)
        );
    }
}
