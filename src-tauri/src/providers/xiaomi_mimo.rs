use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Semaphore;

use super::base64_audio::encode_managed_audio;
use super::openai_compatible::{
    acquire_permit, effective_deadline, execute_with_retry, provider_metadata, request_timeout,
    require_credential,
};
use super::retry::RateGate;
use super::{
    CapabilityEvidence, HttpExecutor, HttpMethod, ProviderCallContext, ProviderCredential,
    ProviderCredentialPlacement, ProviderError, ProviderHttpBody, ProviderHttpConfig,
    ProviderHttpRequest, RawHttpResponse, ReplaySafety, ReqwestHttpExecutor, Transcript,
    TranscriptionCapabilities, TranscriptionProvider, TranscriptionRequest,
};

const MIMO_MODEL: &str = "mimo-v2.5-asr";
const MAX_ENCODED_AUDIO_BYTES: usize = 10_000_000;
const MAX_RAW_AUDIO_BYTES: u64 = 7_499_982;

/// Xiaomi MiMo V2.5 ASR adapter for its documented OpenAI-compatible chat endpoint.
pub struct XiaomiMimoTranscriptionProvider {
    config: ProviderHttpConfig,
    executor: Arc<dyn HttpExecutor>,
    semaphore: Arc<Semaphore>,
    rate_gate: Arc<RateGate>,
}

impl XiaomiMimoTranscriptionProvider {
    /// Creates a MiMo provider with an injected executor for deterministic contract tests.
    pub fn with_executor(
        config: ProviderHttpConfig,
        executor: Arc<dyn HttpExecutor>,
    ) -> Result<Self, ProviderError> {
        config.validate()?;
        validate_config(&config)?;
        let max_concurrent = config.max_concurrent;
        let min_interval = Duration::from_millis(config.min_request_interval_ms);
        Ok(Self {
            config,
            executor,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            rate_gate: Arc::new(RateGate::new(min_interval)),
        })
    }

    /// Creates a production MiMo provider backed by the shared secure reqwest executor.
    pub fn with_reqwest(config: ProviderHttpConfig) -> Result<Self, ProviderError> {
        let executor = Arc::new(ReqwestHttpExecutor::new(Duration::from_millis(
            config.connect_timeout_ms,
        ))?);
        Self::with_executor(config, executor)
    }

    /// Rejects unsupported media, sizes, features, and language values before reading audio.
    fn preflight(&self, request: &TranscriptionRequest) -> Result<&'static str, ProviderError> {
        let metadata = &request.artifact.reference.staging_metadata;
        if metadata.byte_length == 0 {
            return Err(ProviderError::input("empty_audio", "音频文件为空"));
        }
        if metadata.byte_length > MAX_RAW_AUDIO_BYTES {
            return Err(ProviderError::input(
                "file_too_large",
                "音频文件编码后将超过 Xiaomi MiMo 的 10 MB 限制",
            ));
        }
        let mime_type = canonical_mime_type(&metadata.mime_type)?;
        if request.options.enable_timestamps
            || request.options.enable_speaker_labels
            || request.options.enable_confidence
        {
            return Err(ProviderError::input(
                "unsupported_option",
                "Xiaomi MiMo 当前不返回时间戳、说话人或置信度字段",
            ));
        }
        validate_language(request.options.language_hint.as_deref())?;
        Ok(mime_type)
    }

    /// Reads the managed audio cancellably and returns the documented data URL representation.
    async fn encode_audio_data_url(
        &self,
        request: &TranscriptionRequest,
        mime_type: &str,
        context: &ProviderCallContext,
    ) -> Result<String, ProviderError> {
        let data_url = format!(
            "data:{mime_type};base64,{}",
            encode_managed_audio(request, context).await?
        );
        if data_url.len() > MAX_ENCODED_AUDIO_BYTES {
            return Err(ProviderError::input(
                "file_too_large",
                "音频文件编码后超过 Xiaomi MiMo 的 10 MB 限制",
            ));
        }
        Ok(data_url)
    }

    /// Builds the exact documented non-streaming MiMo ASR request body.
    async fn build_request(
        &self,
        context: &ProviderCallContext,
        request: &TranscriptionRequest,
        endpoint: reqwest::Url,
        timeout: Duration,
        mime_type: &str,
    ) -> Result<ProviderHttpRequest, ProviderError> {
        let audio_data = self
            .encode_audio_data_url(request, mime_type, context)
            .await?;
        let language = request.options.language_hint.as_deref().unwrap_or("auto");
        Ok(ProviderHttpRequest {
            method: HttpMethod::Post,
            endpoint,
            body: ProviderHttpBody::Json(json!({
                "model": self.config.model,
                "messages": [{
                    "role": "user",
                    "content": [{
                        "type": "input_audio",
                        "input_audio": {"data": audio_data}
                    }]
                }],
                "asr_options": {"language": language},
                "stream": false
            })),
            headers: std::collections::BTreeMap::new(),
            timeout,
            max_response_bytes: self.config.max_response_bytes,
            response_header_allowlist: Vec::new(),
            idempotency: None,
        })
    }

    /// Normalizes the documented chat completion response into the provider-neutral transcript.
    fn parse_response(
        &self,
        response: &RawHttpResponse,
        request: &TranscriptionRequest,
        started_at: chrono::DateTime<Utc>,
    ) -> Result<Transcript, ProviderError> {
        let body: Value = serde_json::from_slice(&response.body).map_err(|_| {
            ProviderError::protocol(
                "invalid_provider_response",
                "Xiaomi MiMo 返回的转写响应不是有效 JSON",
            )
        })?;
        let text = body
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ProviderError::protocol("empty_transcript", "Xiaomi MiMo 返回了空转写"))?
            .to_owned();
        let remote_request_id = body.get("id").and_then(Value::as_str).map(str::to_owned);
        Ok(Transcript {
            schema_version: "1".to_owned(),
            text,
            language: None,
            duration_ms: request.artifact.reference.staging_metadata.duration_ms,
            segments: Vec::new(),
            provider_metadata: provider_metadata(&self.config, remote_request_id, started_at),
        })
    }
}

#[async_trait]
impl TranscriptionProvider for XiaomiMimoTranscriptionProvider {
    /// Returns the capabilities verified from Xiaomi's published MiMo V2.5 ASR contract.
    fn capabilities(&self) -> TranscriptionCapabilities {
        TranscriptionCapabilities {
            evidence: CapabilityEvidence::Verified,
            accepted_media_types: vec![
                "audio/mpeg".to_owned(),
                "audio/mp3".to_owned(),
                "audio/wav".to_owned(),
            ],
            max_audio_bytes: Some(MAX_RAW_AUDIO_BYTES),
            max_duration_ms: None,
            supports_async_jobs: false,
            supports_timestamps: false,
            supports_speaker_labels: false,
            supports_confidence: false,
            supports_remote_cancel: false,
            supports_remote_urls: false,
            replay_safety: self.config.replay_safety,
        }
    }

    /// Encodes one managed MP3/WAV, sends it with cancellation, and normalizes the response.
    async fn transcribe(
        &self,
        context: &ProviderCallContext,
        request: TranscriptionRequest,
        credential: Option<&ProviderCredential>,
    ) -> Result<Transcript, ProviderError> {
        let mime_type = self.preflight(&request)?;
        require_credential(&self.config, credential)?;
        let endpoint = self.config.validate()?;
        let deadline = effective_deadline(context, &self.config);
        let _permit = acquire_permit(
            self.semaphore.clone(),
            &context.cancellation_token,
            deadline,
        )
        .await?;
        let started_at = Utc::now();
        let timeout = request_timeout(&self.config, deadline)?;
        let http_request = self
            .build_request(context, &request, endpoint, timeout, mime_type)
            .await?;
        let response = execute_with_retry(
            self.executor.as_ref(),
            &self.config,
            &http_request,
            credential,
            &context.cancellation_token,
            &context.operation_id,
            deadline,
            self.rate_gate.as_ref(),
        )
        .await?;
        self.parse_response(&response, &request, started_at)
    }
}

/// Validates MiMo-specific model, authentication, and replay constraints.
fn validate_config(config: &ProviderHttpConfig) -> Result<(), ProviderError> {
    if config.model != MIMO_MODEL {
        return Err(ProviderError::configuration(
            "unsupported_model",
            "Xiaomi MiMo ASR 适配器只支持 mimo-v2.5-asr",
        ));
    }
    let supported_auth = match &config.auth {
        ProviderCredentialPlacement::Bearer => true,
        ProviderCredentialPlacement::Header {
            header_name,
            prefix,
        } => header_name.eq_ignore_ascii_case("api-key") && prefix.is_none(),
        ProviderCredentialPlacement::None => false,
    };
    if !supported_auth {
        return Err(ProviderError::configuration(
            "unsupported_auth_strategy",
            "Xiaomi MiMo ASR 只支持 Bearer 或 api-key 鉴权",
        ));
    }
    if !matches!(
        config.replay_safety,
        ReplaySafety::BeforeRequestBodySentOnly | ReplaySafety::NeverAutomaticallyReplay
    ) {
        return Err(ProviderError::configuration(
            "unsupported_replay_safety",
            "Xiaomi MiMo ASR 未验证请求发送后的自动重放安全性",
        ));
    }
    Ok(())
}

/// Maps accepted aliases to the exact MIME type sent in the MiMo data URL.
fn canonical_mime_type(value: &str) -> Result<&'static str, ProviderError> {
    match value.to_ascii_lowercase().as_str() {
        "audio/mpeg" => Ok("audio/mpeg"),
        "audio/mp3" => Ok("audio/mp3"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Ok("audio/wav"),
        _ => Err(ProviderError::input(
            "unsupported_audio",
            "Xiaomi MiMo 当前只支持 MP3 和 WAV 音频",
        )),
    }
}

/// Validates the optional MiMo language hint against the documented values.
fn validate_language(value: Option<&str>) -> Result<(), ProviderError> {
    if value.is_none_or(|language| matches!(language, "auto" | "zh" | "en")) {
        Ok(())
    } else {
        Err(ProviderError::input(
            "unsupported_language",
            "Xiaomi MiMo 语言参数只支持 auto、zh 或 en",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::Value;
    use tempfile::TempDir;

    use super::{XiaomiMimoTranscriptionProvider, MAX_RAW_AUDIO_BYTES};
    use crate::ingest::{AudioArtifactRef, AudioSourceKind, StagingMetadata};
    use crate::providers::ManagedAudioArtifact;
    use crate::providers::{
        CancellationToken, HttpExecutor, OperationOutcome, ProviderCallContext, ProviderCredential,
        ProviderCredentialPlacement, ProviderHttpBody, ProviderHttpConfig, ProviderHttpRequest,
        RawHttpResponse, ReplaySafety, RetryPolicy, Transcript, TranscriptionOptions,
        TranscriptionProvider, TranscriptionRequest, TransportError,
    };

    /// Captures one redacted-boundary request and returns a scripted response.
    struct CapturingExecutor {
        response: RawHttpResponse,
        request: Mutex<Option<ProviderHttpRequest>>,
    }

    #[async_trait]
    impl HttpExecutor for CapturingExecutor {
        /// Stores the request for contract assertions without calling the network.
        async fn execute(
            &self,
            request: &ProviderHttpRequest,
            _credential: Option<&ProviderCredential>,
            _auth: &ProviderCredentialPlacement,
            _cancellation_token: &CancellationToken,
        ) -> Result<RawHttpResponse, TransportError> {
            *self.request.lock().expect("request capture lock") = Some(request.clone());
            Ok(self.response.clone())
        }
    }

    /// Creates a valid MiMo HTTP profile for isolated adapter tests.
    fn config() -> ProviderHttpConfig {
        ProviderHttpConfig {
            provider_id: "xiaomi_mimo".to_owned(),
            adapter_id: "xiaomi_mimo_asr".to_owned(),
            adapter_version: "1".to_owned(),
            endpoint: "https://api.xiaomimimo.com/v1/chat/completions".to_owned(),
            model: "mimo-v2.5-asr".to_owned(),
            auth: ProviderCredentialPlacement::Bearer,
            connect_timeout_ms: 1_000,
            request_timeout_ms: 5_000,
            overall_timeout_ms: 10_000,
            max_response_bytes: 64 * 1024,
            max_concurrent: 1,
            min_request_interval_ms: 0,
            retry: RetryPolicy {
                max_retries: 1,
                base_delay_ms: 1,
                max_delay_ms: 2,
                max_retry_after_ms: 10,
            },
            replay_safety: ReplaySafety::BeforeRequestBodySentOnly,
            idempotency_header: None,
            allow_insecure_loopback: false,
        }
    }

    /// Creates a managed artifact whose path remains hidden behind the reader contract.
    fn artifact(
        directory: &TempDir,
        bytes: &[u8],
        mime_type: &str,
        reported_size: u64,
    ) -> ManagedAudioArtifact {
        let path = directory.path().join("fixture.mp3");
        std::fs::write(&path, bytes).expect("fixture write");
        let reader_path = path.clone();
        ManagedAudioArtifact::new(
            AudioArtifactRef {
                id: "artifact-1".to_owned(),
                import_batch_id: None,
                source_kind: AudioSourceKind::UserSelectedFile,
                staging_metadata: StagingMetadata {
                    mime_type: mime_type.to_owned(),
                    byte_length: reported_size,
                    duration_ms: Some(1_250),
                    sha256: None,
                    validated_at: Utc::now(),
                },
            },
            Arc::new(move || File::open(&reader_path)),
        )
    }

    /// Calls the provider with one deterministic context and credential.
    async fn transcribe(
        provider: &XiaomiMimoTranscriptionProvider,
        artifact: ManagedAudioArtifact,
        options: TranscriptionOptions,
    ) -> Result<Transcript, crate::providers::ProviderError> {
        provider
            .transcribe(
                &ProviderCallContext::with_timeout(
                    "task-1",
                    "operation-1",
                    CancellationToken::new(),
                    Duration::from_secs(2),
                ),
                TranscriptionRequest { artifact, options },
                Some(&ProviderCredential::new("test-only-key".to_owned())),
            )
            .await
    }

    /// Verifies exact request shape and neutral transcript normalization.
    #[tokio::test]
    async fn builds_documented_request_and_parses_response() {
        let executor = Arc::new(CapturingExecutor {
            response: RawHttpResponse::new(
                200,
                BTreeMap::new(),
                br#"{"id":"request-1","choices":[{"message":{"content":"contract transcript"}}]}"#
                    .to_vec(),
            ),
            request: Mutex::new(None),
        });
        let provider = XiaomiMimoTranscriptionProvider::with_executor(config(), executor.clone())
            .expect("provider config");
        let directory = TempDir::new().expect("temp dir");
        let transcript = transcribe(
            &provider,
            artifact(&directory, b"test-audio", "audio/mpeg", 10),
            TranscriptionOptions {
                language_hint: Some("zh".to_owned()),
                ..TranscriptionOptions::default()
            },
        )
        .await
        .expect("transcription response");

        assert_eq!(transcript.text, "contract transcript");
        assert_eq!(
            transcript.provider_metadata.remote_request_id.as_deref(),
            Some("request-1")
        );
        let captured = executor
            .request
            .lock()
            .expect("request capture lock")
            .clone()
            .expect("captured request");
        let ProviderHttpBody::Json(body) = captured.body else {
            panic!("MiMo request must use JSON");
        };
        assert_eq!(body["model"], Value::String("mimo-v2.5-asr".to_owned()));
        assert_eq!(body["asr_options"]["language"], "zh");
        assert_eq!(body["stream"], false);
        assert!(body["messages"][0]["content"][0]["input_audio"]["data"]
            .as_str()
            .expect("audio data URL")
            .starts_with("data:audio/mpeg;base64,"));
    }

    /// Verifies the documented encoded-size constraint fails before file access or HTTP.
    #[tokio::test]
    async fn rejects_audio_that_would_exceed_encoded_limit() {
        let executor = Arc::new(CapturingExecutor {
            response: RawHttpResponse::new(200, BTreeMap::new(), Vec::new()),
            request: Mutex::new(None),
        });
        let provider = XiaomiMimoTranscriptionProvider::with_executor(config(), executor.clone())
            .expect("provider config");
        let directory = TempDir::new().expect("temp dir");
        let error = transcribe(
            &provider,
            artifact(
                &directory,
                b"small-fixture",
                "audio/mpeg",
                MAX_RAW_AUDIO_BYTES + 1,
            ),
            TranscriptionOptions::default(),
        )
        .await
        .expect_err("oversized audio must fail");
        assert_eq!(error.code, "file_too_large");
        assert!(executor.request.lock().expect("request lock").is_none());
    }

    /// Verifies unsupported media and optional feature requests fail locally.
    #[tokio::test]
    async fn rejects_unsupported_media_and_features() {
        let executor = Arc::new(CapturingExecutor {
            response: RawHttpResponse::new(200, BTreeMap::new(), Vec::new()),
            request: Mutex::new(None),
        });
        let provider = XiaomiMimoTranscriptionProvider::with_executor(config(), executor)
            .expect("provider config");
        let directory = TempDir::new().expect("temp dir");
        let media_error = transcribe(
            &provider,
            artifact(&directory, b"fixture", "audio/mp4", 7),
            TranscriptionOptions::default(),
        )
        .await
        .expect_err("unsupported media must fail");
        assert_eq!(media_error.code, "unsupported_audio");

        let feature_error = transcribe(
            &provider,
            artifact(&directory, b"fixture", "audio/wav", 7),
            TranscriptionOptions {
                enable_speaker_labels: true,
                ..TranscriptionOptions::default()
            },
        )
        .await
        .expect_err("unsupported feature must fail");
        assert_eq!(feature_error.code, "unsupported_option");
    }

    /// Verifies non-empty content is required at the documented response path.
    #[tokio::test]
    async fn rejects_empty_or_malformed_transcript_response() {
        let executor = Arc::new(CapturingExecutor {
            response: RawHttpResponse::new(
                200,
                BTreeMap::new(),
                br#"{"choices":[{"message":{"content":""}}]}"#.to_vec(),
            ),
            request: Mutex::new(None),
        });
        let provider = XiaomiMimoTranscriptionProvider::with_executor(config(), executor)
            .expect("provider config");
        let directory = TempDir::new().expect("temp dir");
        let error = transcribe(
            &provider,
            artifact(&directory, b"fixture", "audio/mp3", 7),
            TranscriptionOptions::default(),
        )
        .await
        .expect_err("empty transcript must fail");
        assert_eq!(error.code, "empty_transcript");
        assert_eq!(error.outcome, OperationOutcome::Failed);
    }
}
