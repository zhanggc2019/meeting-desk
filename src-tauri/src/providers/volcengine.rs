use std::collections::BTreeMap;
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
    CapabilityEvidence, HttpExecutor, HttpMethod, OperationOutcome, ProviderCallContext,
    ProviderCredential, ProviderCredentialPlacement, ProviderError, ProviderErrorCategory,
    ProviderHttpBody, ProviderHttpConfig, ProviderHttpRequest, RawHttpResponse, ReplaySafety,
    ReqwestHttpExecutor, Transcript, TranscriptSegment, TranscriptionCapabilities,
    TranscriptionProvider, TranscriptionRequest, UrlTranscriptionRequest,
};

const VOLCENGINE_MODEL: &str = "bigmodel";
const VOLCENGINE_RESOURCE_ID: &str = "volc.bigasr.auc_turbo";
const MAX_AUDIO_BYTES: u64 = 100_000_000;
const MAX_DURATION_MS: u64 = 2 * 60 * 60 * 1_000;
const STATUS_HEADER: &str = "x-api-status-code";
const LOG_ID_HEADER: &str = "x-tt-logid";

/// Volcengine recording-file flash ASR adapter for local Base64 audio submissions.
pub struct VolcengineFlashTranscriptionProvider {
    config: ProviderHttpConfig,
    executor: Arc<dyn HttpExecutor>,
    semaphore: Arc<Semaphore>,
    rate_gate: Arc<RateGate>,
}

impl VolcengineFlashTranscriptionProvider {
    /// Creates a Volcengine flash provider with an injected executor for contract tests.
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

    /// Creates a production provider backed by the shared redirect-disabled HTTP executor.
    pub fn with_reqwest(config: ProviderHttpConfig) -> Result<Self, ProviderError> {
        let executor = Arc::new(ReqwestHttpExecutor::new(Duration::from_millis(
            config.connect_timeout_ms,
        ))?);
        Self::with_executor(config, executor)
    }

    /// Rejects unsupported local artifacts and unsupported normalized output options.
    fn preflight(&self, request: &TranscriptionRequest) -> Result<&'static str, ProviderError> {
        let metadata = &request.artifact.reference.staging_metadata;
        if metadata.byte_length == 0 {
            return Err(ProviderError::input("empty_audio", "音频文件为空"));
        }
        if metadata.byte_length > MAX_AUDIO_BYTES {
            return Err(ProviderError::input(
                "file_too_large",
                "音频文件超过火山引擎录音文件极速版的 100 MB 限制",
            ));
        }
        if metadata
            .duration_ms
            .is_some_and(|duration| duration > MAX_DURATION_MS)
        {
            return Err(ProviderError::input(
                "duration_limit_exceeded",
                "音频时长超过火山引擎录音文件极速版的 2 小时限制",
            ));
        }
        validate_options(&request.options)?;
        audio_format(&metadata.mime_type)
    }

    /// Validates known URL metadata before allowing the provider to fetch the recording.
    fn preflight_url(request: &UrlTranscriptionRequest) -> Result<&'static str, ProviderError> {
        if request
            .audio
            .byte_length
            .is_some_and(|length| length > MAX_AUDIO_BYTES)
        {
            return Err(ProviderError::input(
                "file_too_large",
                "录音文件超过火山引擎录音文件极速版的 100 MB 限制",
            ));
        }
        if request
            .audio
            .duration_ms
            .is_some_and(|duration| duration > MAX_DURATION_MS)
        {
            return Err(ProviderError::input(
                "duration_limit_exceeded",
                "录音时长超过火山引擎录音文件极速版的 2 小时限制",
            ));
        }
        validate_options(&request.options)?;
        remote_audio_format(request.audio.format)
    }

    /// Builds the documented one-shot Base64 recording-file request and required headers.
    async fn build_request(
        &self,
        context: &ProviderCallContext,
        request: &TranscriptionRequest,
        endpoint: reqwest::Url,
        timeout: Duration,
        format: &str,
    ) -> Result<ProviderHttpRequest, ProviderError> {
        let mut audio = json!({
            "data": encode_managed_audio(request, context).await?,
            "format": format
        });
        if let Some(language) = request.options.language_hint.as_deref() {
            audio["language"] = Value::String(language.to_owned());
        }
        Ok(self.assemble_request(context, endpoint, timeout, audio))
    }

    /// Builds the documented URL recording-file request without fetching the URL locally.
    fn build_url_request(
        &self,
        context: &ProviderCallContext,
        request: &UrlTranscriptionRequest,
        endpoint: reqwest::Url,
        timeout: Duration,
        format: &str,
    ) -> ProviderHttpRequest {
        let mut audio = json!({
            "url": request.audio.url().as_str(),
            "format": format
        });
        if let Some(language) = request.options.language_hint.as_deref() {
            audio["language"] = Value::String(language.to_owned());
        }
        self.assemble_request(context, endpoint, timeout, audio)
    }

    /// Assembles shared headers and recognition options for data and URL inputs.
    fn assemble_request(
        &self,
        context: &ProviderCallContext,
        endpoint: reqwest::Url,
        timeout: Duration,
        audio: Value,
    ) -> ProviderHttpRequest {
        let headers = BTreeMap::from([
            (
                "X-Api-Resource-Id".to_owned(),
                VOLCENGINE_RESOURCE_ID.to_owned(),
            ),
            ("X-Api-Request-Id".to_owned(), context.operation_id.clone()),
            ("X-Api-Sequence".to_owned(), "-1".to_owned()),
        ]);
        ProviderHttpRequest {
            method: HttpMethod::Post,
            endpoint,
            body: ProviderHttpBody::Json(json!({
                "user": {"uid": context.task_id},
                "audio": audio,
                "request": {
                    "model_name": self.config.model,
                    "enable_itn": true,
                    "enable_punc": true,
                    "enable_ddc": true,
                    "show_utterances": true,
                    "enable_speaker_info": false
                }
            })),
            headers,
            timeout,
            max_response_bytes: self.config.max_response_bytes,
            response_header_allowlist: vec![STATUS_HEADER.to_owned(), LOG_ID_HEADER.to_owned()],
            idempotency: None,
        }
    }

    /// Classifies Volcengine's application status header without retaining remote messages.
    fn validate_provider_status(response: &RawHttpResponse) -> Result<(), ProviderError> {
        let status = response.header(STATUS_HEADER).ok_or_else(|| {
            ProviderError::protocol("invalid_provider_response", "火山引擎响应缺少业务状态码")
        })?;
        if status == "20000000" {
            return Ok(());
        }
        let remote_request_id = response.header(LOG_ID_HEADER).map(str::to_owned);
        let error = match status {
            "20000003" => ProviderError::input("empty_transcript", "火山引擎未检测到人声"),
            "45000002" => ProviderError::input("empty_audio", "火山引擎判定音频为空"),
            "45000151" => ProviderError::input("unsupported_audio", "火山引擎无法识别该音频格式"),
            "45000001" => ProviderError::new(
                "invalid_provider_request",
                ProviderErrorCategory::Provider,
                false,
                false,
                "火山引擎拒绝了请求参数",
                None,
                None,
                OperationOutcome::Rejected,
            ),
            value if value.starts_with("550") => ProviderError::new(
                "provider_unavailable",
                ProviderErrorCategory::Provider,
                true,
                false,
                "火山引擎服务暂时不可用",
                None,
                None,
                OperationOutcome::Failed,
            ),
            _ => ProviderError::new(
                "provider_rejected_request",
                ProviderErrorCategory::Provider,
                false,
                false,
                "火山引擎返回了未识别的业务错误",
                None,
                None,
                OperationOutcome::Rejected,
            ),
        };
        Err(error.with_remote_request_id(remote_request_id))
    }

    /// Normalizes full text and documented utterance timestamps from a successful response.
    fn parse_response(
        &self,
        response: &RawHttpResponse,
        fallback_duration_ms: Option<u64>,
        started_at: chrono::DateTime<Utc>,
    ) -> Result<Transcript, ProviderError> {
        Self::validate_provider_status(response)?;
        let body: Value = serde_json::from_slice(&response.body).map_err(|_| {
            ProviderError::protocol(
                "invalid_provider_response",
                "火山引擎返回的转写响应不是有效 JSON",
            )
        })?;
        let text = body
            .pointer("/result/text")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ProviderError::protocol("empty_transcript", "火山引擎返回了空转写"))?
            .to_owned();
        let segments = parse_segments(&body)?;
        let duration_ms = body
            .pointer("/audio_info/duration")
            .and_then(Value::as_u64)
            .or(fallback_duration_ms);
        Ok(Transcript {
            schema_version: "1".to_owned(),
            text,
            language: None,
            duration_ms,
            segments,
            provider_metadata: provider_metadata(
                &self.config,
                response.header(LOG_ID_HEADER).map(str::to_owned),
                started_at,
            ),
        })
    }
}

#[async_trait]
impl TranscriptionProvider for VolcengineFlashTranscriptionProvider {
    /// Returns limits and output fields verified from the recording-file flash documentation.
    fn capabilities(&self) -> TranscriptionCapabilities {
        TranscriptionCapabilities {
            evidence: CapabilityEvidence::Verified,
            accepted_media_types: vec![
                "audio/wav".to_owned(),
                "audio/mpeg".to_owned(),
                "audio/mp3".to_owned(),
                "audio/ogg".to_owned(),
                "audio/opus".to_owned(),
            ],
            max_audio_bytes: Some(MAX_AUDIO_BYTES),
            max_duration_ms: Some(MAX_DURATION_MS),
            supports_async_jobs: false,
            supports_timestamps: true,
            supports_speaker_labels: false,
            supports_confidence: false,
            supports_remote_cancel: false,
            supports_remote_urls: true,
            replay_safety: self.config.replay_safety,
        }
    }

    /// Sends one managed recording file and normalizes the one-shot flash response.
    async fn transcribe(
        &self,
        context: &ProviderCallContext,
        request: TranscriptionRequest,
        credential: Option<&ProviderCredential>,
    ) -> Result<Transcript, ProviderError> {
        let format = self.preflight(&request)?;
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
            .build_request(context, &request, endpoint, timeout, format)
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
        self.parse_response(
            &response,
            request.artifact.reference.staging_metadata.duration_ms,
            started_at,
        )
    }

    /// Sends one validated HTTPS recording URL for provider-side retrieval and recognition.
    async fn transcribe_url(
        &self,
        context: &ProviderCallContext,
        request: UrlTranscriptionRequest,
        credential: Option<&ProviderCredential>,
    ) -> Result<Transcript, ProviderError> {
        let format = Self::preflight_url(&request)?;
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
        let http_request = self.build_url_request(context, &request, endpoint, timeout, format);
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
        self.parse_response(&response, request.audio.duration_ms, started_at)
    }
}

/// Validates the fixed flash model, new-console API key header, and safe replay policy.
fn validate_config(config: &ProviderHttpConfig) -> Result<(), ProviderError> {
    if config.model != VOLCENGINE_MODEL {
        return Err(ProviderError::configuration(
            "unsupported_model",
            "火山引擎录音文件极速版模型必须为 bigmodel",
        ));
    }
    let valid_auth = matches!(
        &config.auth,
        ProviderCredentialPlacement::Header { header_name, prefix }
            if header_name.eq_ignore_ascii_case("x-api-key") && prefix.is_none()
    );
    if !valid_auth {
        return Err(ProviderError::configuration(
            "unsupported_auth_strategy",
            "火山引擎新控制台预设必须使用 X-Api-Key 鉴权",
        ));
    }
    if !matches!(
        config.replay_safety,
        ReplaySafety::BeforeRequestBodySentOnly | ReplaySafety::NeverAutomaticallyReplay
    ) {
        return Err(ProviderError::configuration(
            "unsupported_replay_safety",
            "火山引擎录音识别未验证请求发送后的自动重放安全性",
        ));
    }
    Ok(())
}

/// Maps supported MIME types to Volcengine's documented audio format values.
fn audio_format(value: &str) -> Result<&'static str, ProviderError> {
    match value.to_ascii_lowercase().as_str() {
        "audio/wav" | "audio/x-wav" | "audio/wave" => Ok("wav"),
        "audio/mpeg" | "audio/mp3" => Ok("mp3"),
        "audio/ogg" => Ok("ogg"),
        "audio/opus" => Ok("opus"),
        _ => Err(ProviderError::input(
            "unsupported_audio",
            "火山引擎录音文件极速版不支持该媒体类型",
        )),
    }
}

/// Maps a validated remote format to values supported by the flash recording endpoint.
fn remote_audio_format(
    value: crate::providers::RemoteAudioFormat,
) -> Result<&'static str, ProviderError> {
    use crate::providers::RemoteAudioFormat;
    match value {
        RemoteAudioFormat::Wav => Ok("wav"),
        RemoteAudioFormat::Mp3 => Ok("mp3"),
        RemoteAudioFormat::Ogg => Ok("ogg"),
        RemoteAudioFormat::Opus => Ok("opus"),
        RemoteAudioFormat::M4a => Err(ProviderError::input(
            "unsupported_audio",
            "火山引擎录音文件极速版不支持该 URL 文件格式",
        )),
    }
}

/// Validates normalized feature requests shared by local and URL recordings.
fn validate_options(options: &crate::providers::TranscriptionOptions) -> Result<(), ProviderError> {
    if options.enable_confidence || options.enable_speaker_labels {
        return Err(ProviderError::input(
            "unsupported_option",
            "当前火山引擎适配器尚未归一化置信度或说话人字段",
        ));
    }
    validate_language(options.language_hint.as_deref())
}

/// Validates optional language hints against Volcengine's published recording-file list.
fn validate_language(value: Option<&str>) -> Result<(), ProviderError> {
    const LANGUAGES: &[&str] = &[
        "zh-CN", "en-US", "ja-JP", "id-ID", "es-MX", "pt-BR", "de-DE", "fr-FR", "ko-KR", "fil-PH",
        "ms-MY", "th-TH", "ar-SA", "it-IT", "bn-BD", "el-GR", "nl-NL", "ru-RU", "tr-TR", "vi-VN",
        "pl-PL", "ro-RO", "ne-NP", "uk-UA", "yue-CN",
    ];
    if value.is_none_or(|language| LANGUAGES.contains(&language)) {
        Ok(())
    } else {
        Err(ProviderError::input(
            "unsupported_language",
            "火山引擎不支持所选录音语言参数",
        ))
    }
}

/// Parses documented utterance text and millisecond timestamps without guessing extra fields.
fn parse_segments(body: &Value) -> Result<Vec<TranscriptSegment>, ProviderError> {
    let Some(utterances) = body.pointer("/result/utterances") else {
        return Ok(Vec::new());
    };
    let utterances = utterances.as_array().ok_or_else(|| {
        ProviderError::protocol("invalid_provider_response", "火山引擎分句字段格式无效")
    })?;
    utterances
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let text = item.get("text").and_then(Value::as_str).ok_or_else(|| {
                ProviderError::protocol("invalid_provider_response", "火山引擎分句缺少文本字段")
            })?;
            Ok(TranscriptSegment {
                id: format!("segment-{}", index + 1),
                start_ms: item.get("start_time").and_then(Value::as_u64),
                end_ms: item.get("end_time").and_then(Value::as_u64),
                speaker_label: None,
                text: text.to_owned(),
                confidence: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::Utc;
    use tempfile::TempDir;

    use super::{VolcengineFlashTranscriptionProvider, MAX_AUDIO_BYTES, MAX_DURATION_MS};
    use crate::ingest::{AudioArtifactRef, AudioSourceKind, StagingMetadata};
    use crate::providers::{
        CancellationToken, HttpExecutor, ManagedAudioArtifact, ProviderCallContext,
        ProviderCredential, ProviderCredentialPlacement, ProviderHttpBody, ProviderHttpConfig,
        ProviderHttpRequest, RawHttpResponse, RemoteAudioFile, RemoteAudioFormat, ReplaySafety,
        RetryPolicy, Transcript, TranscriptionOptions, TranscriptionProvider, TranscriptionRequest,
        TransportError, UrlTranscriptionRequest,
    };

    /// Captures a provider request and returns one deterministic response.
    struct CapturingExecutor {
        response: RawHttpResponse,
        request: Mutex<Option<ProviderHttpRequest>>,
    }

    #[async_trait]
    impl HttpExecutor for CapturingExecutor {
        /// Stores the request without sending network traffic.
        async fn execute(
            &self,
            request: &ProviderHttpRequest,
            _credential: Option<&ProviderCredential>,
            _auth: &ProviderCredentialPlacement,
            _cancellation_token: &CancellationToken,
        ) -> Result<RawHttpResponse, TransportError> {
            *self.request.lock().expect("request lock") = Some(request.clone());
            Ok(self.response.clone())
        }
    }

    /// Creates a valid new-console flash provider configuration.
    fn config() -> ProviderHttpConfig {
        ProviderHttpConfig {
            provider_id: "volcengine".to_owned(),
            adapter_id: "volcengine_recording_flash".to_owned(),
            adapter_version: "1".to_owned(),
            endpoint: "https://openspeech.bytedance.com/api/v3/auc/bigmodel/recognize/flash"
                .to_owned(),
            model: "bigmodel".to_owned(),
            auth: ProviderCredentialPlacement::Header {
                header_name: "X-Api-Key".to_owned(),
                prefix: None,
            },
            connect_timeout_ms: 1_000,
            request_timeout_ms: 5_000,
            overall_timeout_ms: 10_000,
            max_response_bytes: 256 * 1024,
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

    /// Creates a path-hidden managed audio fixture with controlled metadata.
    fn artifact(
        directory: &TempDir,
        bytes: &[u8],
        mime_type: &str,
        reported_size: u64,
        duration_ms: Option<u64>,
    ) -> ManagedAudioArtifact {
        let path = directory.path().join("fixture.wav");
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
                    duration_ms,
                    sha256: None,
                    validated_at: Utc::now(),
                },
            },
            Arc::new(move || File::open(&reader_path)),
        )
    }

    /// Calls the provider with one safe fake credential.
    async fn transcribe(
        provider: &VolcengineFlashTranscriptionProvider,
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

    /// Calls URL transcription with one validated remote recording fixture.
    async fn transcribe_url(
        provider: &VolcengineFlashTranscriptionProvider,
        request: UrlTranscriptionRequest,
    ) -> Result<Transcript, crate::providers::ProviderError> {
        provider
            .transcribe_url(
                &ProviderCallContext::with_timeout(
                    "task-url",
                    "operation-url",
                    CancellationToken::new(),
                    Duration::from_secs(2),
                ),
                request,
                Some(&ProviderCredential::new("test-only-key".to_owned())),
            )
            .await
    }

    /// Verifies required headers, Base64 request fields, full text, and timestamp normalization.
    #[tokio::test]
    async fn builds_flash_request_and_parses_utterances() {
        let response_headers = BTreeMap::from([
            ("x-api-status-code".to_owned(), "20000000".to_owned()),
            ("x-tt-logid".to_owned(), "log-1".to_owned()),
        ]);
        let executor = Arc::new(CapturingExecutor {
            response: RawHttpResponse::new(
                200,
                response_headers,
                br#"{"audio_info":{"duration":2499},"result":{"text":"contract transcript","utterances":[{"start_time":450,"end_time":1530,"text":"contract transcript"}]}}"#.to_vec(),
            ),
            request: Mutex::new(None),
        });
        let provider =
            VolcengineFlashTranscriptionProvider::with_executor(config(), executor.clone())
                .expect("provider config");
        let directory = TempDir::new().expect("temp dir");
        let transcript = transcribe(
            &provider,
            artifact(&directory, b"audio", "audio/wav", 5, Some(2_499)),
            TranscriptionOptions {
                language_hint: Some("zh-CN".to_owned()),
                enable_timestamps: true,
                ..TranscriptionOptions::default()
            },
        )
        .await
        .expect("transcription response");

        assert_eq!(transcript.text, "contract transcript");
        assert_eq!(transcript.segments.len(), 1);
        assert_eq!(transcript.segments[0].start_ms, Some(450));
        assert_eq!(
            transcript.provider_metadata.remote_request_id.as_deref(),
            Some("log-1")
        );
        let captured = executor
            .request
            .lock()
            .expect("request lock")
            .clone()
            .expect("captured request");
        assert_eq!(
            captured
                .headers
                .get("X-Api-Resource-Id")
                .map(String::as_str),
            Some("volc.bigasr.auc_turbo")
        );
        assert_eq!(
            captured.headers.get("X-Api-Sequence").map(String::as_str),
            Some("-1")
        );
        let ProviderHttpBody::Json(body) = captured.body else {
            panic!("flash request must use JSON");
        };
        assert_eq!(body["audio"]["format"], "wav");
        assert_eq!(body["audio"]["language"], "zh-CN");
        assert_eq!(body["request"]["model_name"], "bigmodel");
        assert_eq!(body["request"]["show_utterances"], true);
    }

    /// Verifies HTTPS file URLs are passed only in the redacted JSON body with no local fetch.
    #[tokio::test]
    async fn builds_flash_url_request_and_redacts_signed_url() {
        let response_headers = BTreeMap::from([
            ("x-api-status-code".to_owned(), "20000000".to_owned()),
            ("x-tt-logid".to_owned(), "log-url".to_owned()),
        ]);
        let executor = Arc::new(CapturingExecutor {
            response: RawHttpResponse::new(
                200,
                response_headers,
                br#"{"audio_info":{"duration":1500},"result":{"text":"url transcript"}}"#.to_vec(),
            ),
            request: Mutex::new(None),
        });
        let provider =
            VolcengineFlashTranscriptionProvider::with_executor(config(), executor.clone())
                .expect("provider config");
        let signed_url = "https://media.example.test/meeting.mp3?signature=test-only-url-sentinel";
        let transcript = transcribe_url(
            &provider,
            UrlTranscriptionRequest {
                audio: RemoteAudioFile::new(
                    "remote-1",
                    signed_url,
                    RemoteAudioFormat::Mp3,
                    Some(10_000),
                    Some(1_500),
                )
                .expect("remote audio"),
                options: TranscriptionOptions::default(),
            },
        )
        .await
        .expect("URL transcription response");

        assert_eq!(transcript.text, "url transcript");
        let captured = executor
            .request
            .lock()
            .expect("request lock")
            .clone()
            .expect("captured request");
        assert!(!format!("{captured:?}").contains("test-only-url-sentinel"));
        let ProviderHttpBody::Json(body) = captured.body else {
            panic!("URL request must use JSON");
        };
        assert_eq!(body["audio"]["url"], signed_url);
        assert_eq!(body["audio"]["format"], "mp3");
        assert!(provider.capabilities().supports_remote_urls);
    }

    /// Verifies file size and duration hard limits fail before HTTP execution.
    #[tokio::test]
    async fn rejects_oversized_or_overlong_recordings() {
        let executor = Arc::new(CapturingExecutor {
            response: RawHttpResponse::new(200, BTreeMap::new(), Vec::new()),
            request: Mutex::new(None),
        });
        let provider =
            VolcengineFlashTranscriptionProvider::with_executor(config(), executor.clone())
                .expect("provider config");
        let directory = TempDir::new().expect("temp dir");
        let size_error = transcribe(
            &provider,
            artifact(
                &directory,
                b"fixture",
                "audio/mpeg",
                MAX_AUDIO_BYTES + 1,
                None,
            ),
            TranscriptionOptions::default(),
        )
        .await
        .expect_err("size limit");
        assert_eq!(size_error.code, "file_too_large");

        let duration_error = transcribe(
            &provider,
            artifact(
                &directory,
                b"fixture",
                "audio/mpeg",
                7,
                Some(MAX_DURATION_MS + 1),
            ),
            TranscriptionOptions::default(),
        )
        .await
        .expect_err("duration limit");
        assert_eq!(duration_error.code, "duration_limit_exceeded");
        assert!(executor.request.lock().expect("request lock").is_none());
    }

    /// Verifies a successful HTTP response still requires Volcengine's business status header.
    #[tokio::test]
    async fn classifies_volcengine_business_errors() {
        let executor = Arc::new(CapturingExecutor {
            response: RawHttpResponse::new(
                200,
                BTreeMap::from([
                    ("x-api-status-code".to_owned(), "55000031".to_owned()),
                    ("x-tt-logid".to_owned(), "log-error".to_owned()),
                ]),
                Vec::new(),
            ),
            request: Mutex::new(None),
        });
        let provider = VolcengineFlashTranscriptionProvider::with_executor(config(), executor)
            .expect("provider config");
        let directory = TempDir::new().expect("temp dir");
        let error = transcribe(
            &provider,
            artifact(&directory, b"fixture", "audio/mp3", 7, None),
            TranscriptionOptions::default(),
        )
        .await
        .expect_err("business error");
        assert_eq!(error.code, "provider_unavailable");
        assert!(error.retryable);
        assert_eq!(error.remote_request_id.as_deref(), Some("log-error"));
    }
}
