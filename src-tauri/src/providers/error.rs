use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::OperationOutcome;

/// Stable provider error categories exposed to orchestration and IPC layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorCategory {
    Configuration,
    Input,
    Authentication,
    Permission,
    RateLimit,
    Network,
    Timeout,
    Provider,
    Protocol,
    Cancellation,
    LocalResource,
}

/// A sanitized provider failure that contains no headers, body, prompt, transcript, or secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "camelCase")]
#[error("{code}: {safe_message}")]
pub struct ProviderError {
    pub code: String,
    pub category: ProviderErrorCategory,
    pub retryable: bool,
    pub replay_safe: bool,
    pub safe_message: String,
    pub http_status: Option<u16>,
    pub retry_after_ms: Option<u64>,
    pub outcome: OperationOutcome,
    pub remote_request_id: Option<String>,
}

impl ProviderError {
    /// Creates a fully specified sanitized provider error.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        code: impl Into<String>,
        category: ProviderErrorCategory,
        retryable: bool,
        replay_safe: bool,
        safe_message: impl Into<String>,
        http_status: Option<u16>,
        retry_after_ms: Option<u64>,
        outcome: OperationOutcome,
    ) -> Self {
        Self {
            code: code.into(),
            category,
            retryable,
            replay_safe,
            safe_message: safe_message.into(),
            http_status,
            retry_after_ms,
            outcome,
            remote_request_id: None,
        }
    }

    /// Creates a non-retryable configuration failure before any request is sent.
    pub fn configuration(code: impl Into<String>, safe_message: impl Into<String>) -> Self {
        Self::new(
            code,
            ProviderErrorCategory::Configuration,
            false,
            false,
            safe_message,
            None,
            None,
            OperationOutcome::NotSent,
        )
    }

    /// Creates a non-retryable input failure before any request is sent.
    pub fn input(code: impl Into<String>, safe_message: impl Into<String>) -> Self {
        Self::new(
            code,
            ProviderErrorCategory::Input,
            false,
            false,
            safe_message,
            None,
            None,
            OperationOutcome::NotSent,
        )
    }

    /// Creates a non-retryable protocol failure with no raw response details.
    pub fn protocol(code: impl Into<String>, safe_message: impl Into<String>) -> Self {
        Self::new(
            code,
            ProviderErrorCategory::Protocol,
            false,
            false,
            safe_message,
            None,
            None,
            OperationOutcome::Failed,
        )
    }

    /// Creates a terminal cancellation result.
    pub fn cancelled() -> Self {
        Self::new(
            "cancelled",
            ProviderErrorCategory::Cancellation,
            false,
            false,
            "任务已取消",
            None,
            None,
            OperationOutcome::Unknown,
        )
    }

    /// Creates an overall operation timeout that must not be retried internally.
    pub fn operation_timeout() -> Self {
        Self::new(
            "operation_timeout",
            ProviderErrorCategory::Timeout,
            false,
            false,
            "云端处理超过总超时时间",
            None,
            None,
            OperationOutcome::Unknown,
        )
    }

    /// Attaches a reviewed remote request identifier without exposing response bodies.
    pub fn with_remote_request_id(mut self, request_id: Option<String>) -> Self {
        self.remote_request_id = request_id;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderError;

    /// Verifies that serialized errors contain only the stable sanitized contract.
    #[test]
    fn serializes_sanitized_error_contract() {
        let error = ProviderError::configuration("invalid_endpoint", "服务地址无效");
        let serialized = serde_json::to_string(&error).expect("error should serialize");
        assert!(serialized.contains("invalid_endpoint"));
        assert!(!serialized.contains("Authorization"));
    }
}
