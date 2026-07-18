use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    CapabilityEvidence, MinutesCandidate, MinutesCapabilities, MinutesGenerationRequest,
    MinutesProvider, OperationOutcome, ProviderCallContext, ProviderCredential, ProviderError,
    ProviderErrorCategory, ProviderMetadata, ReplaySafety, Transcript, TranscriptionCapabilities,
    TranscriptionProvider, TranscriptionRequest,
};

/// Deterministic mock scenarios available without a network or API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MockScenario {
    Success,
    Delay,
    TimeoutConnect,
    TimeoutAfterSend,
    Http401,
    Http403,
    Http429,
    Http500,
    NetworkUnavailable,
    MalformedResponse,
    EmptyTranscript,
    InvalidMinutesSchema,
    CancelUpload,
    CancelPoll,
    CancelGenerate,
}

/// Non-sensitive deterministic mock configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MockConfig {
    pub scenario: MockScenario,
    pub delay_ms: u64,
}

impl MockConfig {
    /// Validates that mock delays remain finite and suitable for automated tests.
    pub fn validate(&self) -> Result<(), ProviderError> {
        if self.delay_ms > 60_000 {
            return Err(ProviderError::configuration(
                "invalid_mock_delay",
                "Mock 延迟不能超过 60 秒",
            ));
        }
        Ok(())
    }
}

/// Safe mock call metadata that excludes paths, bodies, prompts, and generated text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MockCallRecord {
    pub operation_id: String,
    pub artifact_id: Option<String>,
    pub operation_kind: String,
    pub scenario: MockScenario,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub outcome: OperationOutcome,
}

/// One provider implementation shared by deterministic ASR and minutes mock paths.
pub struct MockProvider {
    config: MockConfig,
    transcript_text: String,
    minutes_candidate: Value,
    schema_version: String,
    calls: Arc<Mutex<Vec<MockCallRecord>>>,
}

impl MockProvider {
    /// Creates a mock with caller-supplied non-sensitive fixtures.
    pub fn new(
        config: MockConfig,
        transcript_text: String,
        minutes_candidate: Value,
        schema_version: String,
    ) -> Result<Self, ProviderError> {
        config.validate()?;
        if transcript_text.trim().is_empty() && config.scenario != MockScenario::EmptyTranscript {
            return Err(ProviderError::configuration(
                "invalid_mock_fixture",
                "Mock 转写 fixture 不能为空",
            ));
        }
        if schema_version.trim().is_empty() {
            return Err(ProviderError::configuration(
                "invalid_mock_fixture",
                "Mock Schema 版本不能为空",
            ));
        }
        Ok(Self {
            config,
            transcript_text,
            minutes_candidate,
            schema_version,
            calls: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Returns a snapshot of safe mock call metadata for assertions.
    pub fn call_records(&self) -> Vec<MockCallRecord> {
        self.calls
            .lock()
            .expect("mock call record mutex should not be poisoned")
            .clone()
    }

    /// Applies deterministic delay or waits for cancellation in cancellation scenarios.
    async fn apply_timing(&self, context: &ProviderCallContext) -> Result<(), ProviderError> {
        if matches!(
            self.config.scenario,
            MockScenario::CancelUpload | MockScenario::CancelPoll | MockScenario::CancelGenerate
        ) {
            let remaining = context.remaining();
            if remaining.is_zero() {
                return Err(ProviderError::operation_timeout());
            }
            return tokio::select! {
                _ = context.cancellation_token.cancelled() => Err(ProviderError::cancelled()),
                _ = tokio::time::sleep(remaining) => Err(ProviderError::operation_timeout()),
            };
        }

        if self.config.delay_ms > 0
            && matches!(
                self.config.scenario,
                MockScenario::Success | MockScenario::Delay
            )
        {
            return tokio::select! {
                _ = context.cancellation_token.cancelled() => Err(ProviderError::cancelled()),
                _ = tokio::time::sleep(Duration::from_millis(self.config.delay_ms)) => Ok(()),
            };
        }
        if context.cancellation_token.is_cancelled() {
            return Err(ProviderError::cancelled());
        }
        Ok(())
    }

    /// Returns the scenario's deterministic failure, if any.
    fn scenario_error(&self) -> Option<ProviderError> {
        match self.config.scenario {
            MockScenario::TimeoutConnect => Some(ProviderError::new(
                "connect_timeout",
                ProviderErrorCategory::Timeout,
                true,
                true,
                "Mock 连接超时",
                None,
                None,
                OperationOutcome::NotSent,
            )),
            MockScenario::TimeoutAfterSend => Some(ProviderError::new(
                "request_timeout",
                ProviderErrorCategory::Timeout,
                true,
                false,
                "Mock 请求超时",
                None,
                None,
                OperationOutcome::Unknown,
            )),
            MockScenario::Http401 => Some(mock_http_error(
                "http_401",
                ProviderErrorCategory::Authentication,
                401,
                false,
            )),
            MockScenario::Http403 => Some(mock_http_error(
                "http_403",
                ProviderErrorCategory::Permission,
                403,
                false,
            )),
            MockScenario::Http429 => Some(ProviderError::new(
                "http_429",
                ProviderErrorCategory::RateLimit,
                true,
                true,
                "Mock 限流",
                Some(429),
                Some(1),
                OperationOutcome::Rejected,
            )),
            MockScenario::Http500 => Some(mock_http_error(
                "http_5xx",
                ProviderErrorCategory::Provider,
                500,
                true,
            )),
            MockScenario::NetworkUnavailable => Some(ProviderError::new(
                "network_unavailable",
                ProviderErrorCategory::Network,
                true,
                true,
                "Mock 网络不可用",
                None,
                None,
                OperationOutcome::NotSent,
            )),
            MockScenario::MalformedResponse => Some(ProviderError::protocol(
                "invalid_provider_response",
                "Mock 响应格式错误",
            )),
            _ => None,
        }
    }

    /// Records one safe mock completion without fixture or prompt content.
    fn record_call(
        &self,
        context: &ProviderCallContext,
        artifact_id: Option<String>,
        operation_kind: &str,
        started_at: DateTime<Utc>,
        outcome: OperationOutcome,
    ) {
        self.calls
            .lock()
            .expect("mock call record mutex should not be poisoned")
            .push(MockCallRecord {
                operation_id: context.operation_id.clone(),
                artifact_id,
                operation_kind: operation_kind.to_owned(),
                scenario: self.config.scenario,
                started_at,
                completed_at: Utc::now(),
                outcome,
            });
    }

    /// Creates safe result metadata for a local mock result.
    fn metadata(&self, started_at: DateTime<Utc>) -> ProviderMetadata {
        ProviderMetadata {
            provider_id: "mock".to_owned(),
            adapter_id: "mock".to_owned(),
            adapter_version: "1".to_owned(),
            model: "mock".to_owned(),
            remote_request_id: None,
            started_at,
            completed_at: Utc::now(),
        }
    }
}

impl fmt::Debug for MockProvider {
    /// Redacts fixture values while keeping deterministic scenario metadata visible.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MockProvider")
            .field("config", &self.config)
            .field("transcript_text", &"[REDACTED]")
            .field("minutes_candidate", &"[REDACTED]")
            .field("schema_version", &self.schema_version)
            .finish()
    }
}

#[async_trait]
impl TranscriptionProvider for MockProvider {
    /// Returns deterministic mock capabilities without claiming real-provider support.
    fn capabilities(&self) -> TranscriptionCapabilities {
        TranscriptionCapabilities {
            evidence: CapabilityEvidence::Mock,
            accepted_media_types: Vec::new(),
            max_audio_bytes: None,
            max_duration_ms: None,
            supports_async_jobs: false,
            supports_timestamps: false,
            supports_speaker_labels: false,
            supports_confidence: false,
            supports_remote_cancel: true,
            replay_safety: ReplaySafety::VerifiedAlwaysSafe,
        }
    }

    /// Returns a deterministic transcript or configured safe failure without using a key.
    async fn transcribe(
        &self,
        context: &ProviderCallContext,
        request: TranscriptionRequest,
        _credential: Option<&ProviderCredential>,
    ) -> Result<Transcript, ProviderError> {
        let started_at = Utc::now();
        let artifact_id = Some(request.artifact.reference.id.clone());
        let result = async {
            if request.artifact.reference.staging_metadata.byte_length == 0 {
                return Err(ProviderError::input("empty_audio", "音频文件为空"));
            }
            self.apply_timing(context).await?;
            if self.config.scenario == MockScenario::EmptyTranscript {
                return Err(ProviderError::protocol(
                    "empty_transcript",
                    "Mock 返回了空转写",
                ));
            }
            if let Some(error) = self.scenario_error() {
                return Err(error);
            }
            Ok(Transcript {
                schema_version: "1".to_owned(),
                text: self.transcript_text.clone(),
                language: None,
                duration_ms: request.artifact.reference.staging_metadata.duration_ms,
                segments: Vec::new(),
                provider_metadata: self.metadata(started_at),
            })
        }
        .await;
        self.record_call(
            context,
            artifact_id,
            "transcription",
            started_at,
            result_outcome(&result),
        );
        result
    }
}

#[async_trait]
impl MinutesProvider for MockProvider {
    /// Returns deterministic mock capabilities without claiming real-provider support.
    fn capabilities(&self) -> MinutesCapabilities {
        MinutesCapabilities {
            evidence: CapabilityEvidence::Mock,
            supports_json_schema: true,
            supported_schema_versions: vec![self.schema_version.clone()],
            max_input_characters: None,
            supports_async_jobs: false,
            supports_remote_cancel: true,
            replay_safety: ReplaySafety::VerifiedAlwaysSafe,
        }
    }

    /// Returns an untrusted deterministic candidate for the real minutes validator.
    async fn generate_candidate(
        &self,
        context: &ProviderCallContext,
        request: MinutesGenerationRequest,
        _credential: Option<&ProviderCredential>,
    ) -> Result<MinutesCandidate, ProviderError> {
        let started_at = Utc::now();
        let result = async {
            if request.prompt.trim().is_empty() {
                return Err(ProviderError::input(
                    "empty_minutes_prompt",
                    "纪要 Prompt 不能为空",
                ));
            }
            self.apply_timing(context).await?;
            if let Some(error) = self.scenario_error() {
                return Err(error);
            }
            let value = if self.config.scenario == MockScenario::InvalidMinutesSchema {
                serde_json::json!({"fixtureInvalid": true})
            } else {
                self.minutes_candidate.clone()
            };
            Ok(MinutesCandidate {
                schema_version: request.schema_version,
                value,
                provider_metadata: self.metadata(started_at),
            })
        }
        .await;
        self.record_call(
            context,
            None,
            "minutes",
            started_at,
            result_outcome(&result),
        );
        result
    }
}

/// Creates one deterministic mock HTTP failure.
fn mock_http_error(
    code: &str,
    category: ProviderErrorCategory,
    status: u16,
    retryable: bool,
) -> ProviderError {
    ProviderError::new(
        code,
        category,
        retryable,
        retryable,
        "Mock Provider 返回错误",
        Some(status),
        None,
        OperationOutcome::Rejected,
    )
}

/// Derives a safe operation outcome from a mock result.
fn result_outcome<T>(result: &Result<T, ProviderError>) -> OperationOutcome {
    match result {
        Ok(_) => OperationOutcome::Succeeded,
        Err(error) => error.outcome,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::Utc;
    use serde_json::json;

    use super::{MockConfig, MockProvider, MockScenario};
    use crate::ingest::AudioSourceKind;
    use crate::providers::{
        AudioArtifactRef, CancellationToken, ManagedAudioArtifact, MinutesGenerationRequest,
        MinutesProvider, ProviderCallContext, StagingMetadata, TranscriptionOptions,
        TranscriptionProvider, TranscriptionRequest,
    };

    /// Creates a deterministic mock provider with short non-sensitive fixtures.
    fn provider(scenario: MockScenario, delay_ms: u64) -> MockProvider {
        MockProvider::new(
            MockConfig { scenario, delay_ms },
            "short fixture transcript".to_owned(),
            json!({"schemaVersion": "1.0.0"}),
            "1.0.0".to_owned(),
        )
        .expect("mock provider should build")
    }

    /// Creates a safe in-memory artifact reference that the mock never opens.
    fn request() -> TranscriptionRequest {
        TranscriptionRequest {
            artifact: ManagedAudioArtifact::new(
                AudioArtifactRef {
                    id: "artifact-test".to_owned(),
                    import_batch_id: Some("batch-test".to_owned()),
                    source_kind: AudioSourceKind::UserSelectedFile,
                    staging_metadata: StagingMetadata {
                        mime_type: "audio/test".to_owned(),
                        byte_length: 4,
                        duration_ms: Some(100),
                        sha256: None,
                        validated_at: Utc::now(),
                    },
                },
                Arc::new(|| {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "mock provider never opens the artifact",
                    ))
                }),
            ),
            options: TranscriptionOptions::default(),
        }
    }

    /// Creates one short operation context.
    fn context(token: CancellationToken) -> ProviderCallContext {
        ProviderCallContext::with_timeout(
            "task-test",
            "operation-test",
            token,
            Duration::from_secs(1),
        )
    }

    /// Verifies that the same provider trait supports a complete offline mock flow.
    #[tokio::test]
    async fn mock_success_returns_transcript_and_minutes_candidate() {
        let provider = provider(MockScenario::Success, 0);
        let transcript = provider
            .transcribe(&context(CancellationToken::new()), request(), None)
            .await
            .expect("mock transcript should succeed");
        assert_eq!(transcript.text, "short fixture transcript");

        let candidate = provider
            .generate_candidate(
                &context(CancellationToken::new()),
                MinutesGenerationRequest {
                    prompt: "short fixture prompt".to_owned(),
                    output_schema: json!({"type": "object"}),
                    schema_version: "1.0.0".to_owned(),
                },
                None,
            )
            .await
            .expect("mock candidate should succeed");
        assert_eq!(candidate.value["schemaVersion"], "1.0.0");
        assert_eq!(provider.call_records().len(), 2);
    }

    /// Verifies that cancellation interrupts a delayed mock and records no fixture content.
    #[tokio::test]
    async fn mock_delay_is_cancellable_and_call_record_is_safe() {
        let provider = provider(MockScenario::Delay, 500);
        let token = CancellationToken::new();
        let cancel_token = token.clone();
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cancel_token.cancel();
        });
        let error = provider
            .transcribe(&context(token), request(), None)
            .await
            .expect_err("cancelled mock should fail");
        cancel_task.await.expect("cancel task should finish");
        assert_eq!(error.code, "cancelled");
        let records = serde_json::to_string(&provider.call_records())
            .expect("safe call records should serialize");
        assert!(!records.contains("short fixture transcript"));
    }

    /// Verifies that an empty transcript never becomes a successful DTO.
    #[tokio::test]
    async fn mock_empty_transcript_is_rejected() {
        let provider = provider(MockScenario::EmptyTranscript, 0);
        let error = provider
            .transcribe(&context(CancellationToken::new()), request(), None)
            .await
            .expect_err("empty transcript should fail");
        assert_eq!(error.code, "empty_transcript");
    }
}
