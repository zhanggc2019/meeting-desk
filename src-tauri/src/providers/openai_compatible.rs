use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use reqwest::header::HeaderName;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::http::classify_http_status;
use super::retry::{is_replay_safe, sleep_cancellable, RateGate};
use super::{
    AudioArtifactRef, CapabilityEvidence, HttpExecutor, HttpMethod, MinutesCandidate,
    MinutesCapabilities, MinutesGenerationRequest, MinutesProvider, MultipartBody, MultipartFile,
    ProviderCallContext, ProviderCredential, ProviderError, ProviderHttpBody, ProviderHttpConfig,
    ProviderHttpRequest, ProviderMetadata, RawHttpResponse, ReqwestHttpExecutor, Transcript,
    TranscriptSegment, TranscriptionCapabilities, TranscriptionOptions, TranscriptionProvider,
    TranscriptionRequest,
};

/// One explicit component of a provider JSON path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum JsonPathSegment {
    Key(String),
    Index(usize),
}

/// Explicit response JSON path used instead of hard-coded provider field names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonPath(pub Vec<JsonPathSegment>);

impl JsonPath {
    /// Creates a key-only path for tests and statically reviewed adapter profiles.
    pub fn keys(keys: &[&str]) -> Self {
        Self(
            keys.iter()
                .map(|key| JsonPathSegment::Key((*key).to_owned()))
                .collect(),
        )
    }

    /// Validates that the path is non-empty and contains no empty keys.
    pub fn validate(&self) -> Result<(), ProviderError> {
        if self.0.is_empty()
            || self
                .0
                .iter()
                .any(|segment| matches!(segment, JsonPathSegment::Key(key) if key.is_empty()))
        {
            return Err(ProviderError::configuration(
                "invalid_response_mapping",
                "Provider 响应字段映射无效",
            ));
        }
        Ok(())
    }

    /// Selects a value without exposing the configured path in errors or logs.
    fn select<'a>(&self, root: &'a Value) -> Option<&'a Value> {
        let mut current = root;
        for segment in &self.0 {
            current = match segment {
                JsonPathSegment::Key(key) => current.get(key)?,
                JsonPathSegment::Index(index) => current.get(*index)?,
            };
        }
        Some(current)
    }
}

/// Explicit provider text values for one optional boolean transcription feature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleFieldMapping {
    pub field_name: String,
    pub enabled_value: String,
    pub disabled_value: Option<String>,
}

/// Unit used by configured segment timestamp response paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimestampUnit {
    Milliseconds,
    Seconds,
}

/// Explicit JSON paths used to normalize one provider segment object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentResponseMapping {
    pub segments_path: JsonPath,
    pub id_path: Option<JsonPath>,
    pub text_path: JsonPath,
    pub start_path: Option<JsonPath>,
    pub end_path: Option<JsonPath>,
    pub speaker_path: Option<JsonPath>,
    pub confidence_path: Option<JsonPath>,
    pub timestamp_unit: TimestampUnit,
}

/// Explicit JSON paths used to normalize one transcription response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionResponseMapping {
    pub text_path: JsonPath,
    pub language_path: Option<JsonPath>,
    pub segments: Option<SegmentResponseMapping>,
    pub remote_request_id_header: Option<String>,
}

/// Fully explicit multipart and response mapping for a compatible transcription endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionHttpMapping {
    pub method: HttpMethod,
    pub audio_field: String,
    pub upload_file_name: String,
    pub model_field: Option<String>,
    pub language_field: Option<String>,
    pub timestamps_field: Option<ToggleFieldMapping>,
    pub speaker_labels_field: Option<ToggleFieldMapping>,
    pub confidence_field: Option<ToggleFieldMapping>,
    pub static_text_fields: BTreeMap<String, String>,
    pub response: TranscriptionResponseMapping,
    pub capabilities: TranscriptionCapabilities,
}

impl TranscriptionHttpMapping {
    /// Validates every provider-specific field mapping without assuming defaults.
    pub fn validate(&self) -> Result<(), ProviderError> {
        validate_multipart_field(&self.audio_field)?;
        if self.upload_file_name.is_empty()
            || self.upload_file_name.contains(['/', '\\', '\r', '\n'])
        {
            return Err(ProviderError::configuration(
                "invalid_upload_file_name",
                "上传文件名配置无效",
            ));
        }

        for field in [&self.model_field, &self.language_field]
            .into_iter()
            .flatten()
        {
            validate_multipart_field(field)?;
        }
        for toggle in [
            &self.timestamps_field,
            &self.speaker_labels_field,
            &self.confidence_field,
        ]
        .into_iter()
        .flatten()
        {
            validate_toggle_field(toggle)?;
        }
        for field in self.static_text_fields.keys() {
            validate_multipart_field(field)?;
            if looks_secret_field(field) {
                return Err(ProviderError::configuration(
                    "secret_in_request_mapping",
                    "密钥只能通过受控认证配置注入",
                ));
            }
        }

        self.response.text_path.validate()?;
        if let Some(path) = &self.response.language_path {
            path.validate()?;
        }
        if let Some(segments) = &self.response.segments {
            validate_segment_mapping(segments)?;
        }
        if let Some(header) = &self.response.remote_request_id_header {
            validate_response_header(header)?;
        }
        Ok(())
    }
}

/// Determines how a mapped minutes response value becomes a JSON candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonContentMode {
    JsonValue,
    JsonEncodedString,
}

/// Explicit request template and response mapping for a compatible minutes endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinutesHttpMapping {
    pub method: HttpMethod,
    pub body_template: Value,
    pub model_placeholder: String,
    pub prompt_placeholder: String,
    pub schema_placeholder: Option<String>,
    pub response_content_path: JsonPath,
    pub response_content_mode: JsonContentMode,
    pub remote_request_id_header: Option<String>,
    pub capabilities: MinutesCapabilities,
}

impl MinutesHttpMapping {
    /// Validates placeholders and response mappings without imposing provider JSON fields.
    pub fn validate(&self) -> Result<(), ProviderError> {
        if !self.body_template.is_object() {
            return Err(ProviderError::configuration(
                "invalid_minutes_request_template",
                "纪要请求模板必须是 JSON 对象",
            ));
        }
        if self.model_placeholder.is_empty()
            || self.prompt_placeholder.is_empty()
            || self.model_placeholder == self.prompt_placeholder
        {
            return Err(ProviderError::configuration(
                "invalid_minutes_request_template",
                "纪要请求占位符无效",
            ));
        }
        if count_placeholder(&self.body_template, &self.model_placeholder) != 1
            || count_placeholder(&self.body_template, &self.prompt_placeholder) != 1
        {
            return Err(ProviderError::configuration(
                "invalid_minutes_request_template",
                "模型和 Prompt 占位符必须各出现一次",
            ));
        }
        if let Some(schema_placeholder) = &self.schema_placeholder {
            if schema_placeholder.is_empty()
                || schema_placeholder == &self.model_placeholder
                || schema_placeholder == &self.prompt_placeholder
                || count_placeholder(&self.body_template, schema_placeholder) != 1
            {
                return Err(ProviderError::configuration(
                    "invalid_minutes_request_template",
                    "Schema 占位符必须唯一且出现一次",
                ));
            }
        } else if self.capabilities.supports_json_schema {
            return Err(ProviderError::configuration(
                "missing_schema_mapping",
                "声明支持 JSON Schema 时必须显式配置 Schema 占位符",
            ));
        }
        if contains_secret_key(&self.body_template) {
            return Err(ProviderError::configuration(
                "secret_in_request_mapping",
                "密钥只能通过受控认证配置注入",
            ));
        }
        self.response_content_path.validate()?;
        if let Some(header) = &self.remote_request_id_header {
            validate_response_header(header)?;
        }
        Ok(())
    }
}

/// Configurable transcription provider whose field mapping is never implicit.
pub struct OpenAiCompatibleTranscriptionProvider {
    config: ProviderHttpConfig,
    mapping: TranscriptionHttpMapping,
    executor: Arc<dyn HttpExecutor>,
    semaphore: Arc<Semaphore>,
    rate_gate: Arc<RateGate>,
}

impl OpenAiCompatibleTranscriptionProvider {
    /// Creates a provider with an injected executor for deterministic HTTP behavior tests.
    pub fn with_executor(
        config: ProviderHttpConfig,
        mapping: TranscriptionHttpMapping,
        executor: Arc<dyn HttpExecutor>,
    ) -> Result<Self, ProviderError> {
        config.validate()?;
        mapping.validate()?;
        validate_http_capability_evidence(
            mapping.capabilities.evidence,
            mapping.capabilities.supports_async_jobs,
            mapping.capabilities.supports_remote_cancel,
        )?;
        if config.replay_safety != mapping.capabilities.replay_safety {
            return Err(ProviderError::configuration(
                "inconsistent_replay_safety",
                "Provider 能力与 HTTP 重放策略不一致",
            ));
        }
        let max_concurrent = config.max_concurrent;
        let min_interval = Duration::from_millis(config.min_request_interval_ms);
        Ok(Self {
            config,
            mapping,
            executor,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            rate_gate: Arc::new(RateGate::new(min_interval)),
        })
    }

    /// Creates a production provider backed by a redirect-disabled reqwest client.
    pub fn with_reqwest(
        config: ProviderHttpConfig,
        mapping: TranscriptionHttpMapping,
    ) -> Result<Self, ProviderError> {
        let executor = Arc::new(ReqwestHttpExecutor::new(Duration::from_millis(
            config.connect_timeout_ms,
        ))?);
        Self::with_executor(config, mapping, executor)
    }

    /// Performs local capability checks before any credential or HTTP operation is used.
    fn preflight(&self, request: &TranscriptionRequest) -> Result<(), ProviderError> {
        let metadata = &request.artifact.reference.staging_metadata;
        if metadata.byte_length == 0 {
            return Err(ProviderError::input("empty_audio", "音频文件为空"));
        }
        if !self.mapping.capabilities.accepted_media_types.is_empty()
            && !self
                .mapping
                .capabilities
                .accepted_media_types
                .iter()
                .any(|media_type| media_type.eq_ignore_ascii_case(&metadata.mime_type))
        {
            return Err(ProviderError::input(
                "unsupported_audio",
                "当前 Provider 不支持该音频媒体类型",
            ));
        }
        if self
            .mapping
            .capabilities
            .max_audio_bytes
            .is_some_and(|limit| metadata.byte_length > limit)
        {
            return Err(ProviderError::input(
                "file_too_large",
                "音频文件超过当前 Provider 的已知限制",
            ));
        }
        if let (Some(duration), Some(limit)) = (
            metadata.duration_ms,
            self.mapping.capabilities.max_duration_ms,
        ) {
            if duration > limit {
                return Err(ProviderError::input(
                    "duration_limit_exceeded",
                    "音频时长超过当前 Provider 的已知限制",
                ));
            }
        }
        validate_requested_features(&request.options, &self.mapping)?;
        Ok(())
    }

    /// Builds an explicit multipart request from reviewed mapping configuration.
    fn build_request(
        &self,
        context: &ProviderCallContext,
        request: &TranscriptionRequest,
        endpoint: reqwest::Url,
        timeout: Duration,
    ) -> Result<ProviderHttpRequest, ProviderError> {
        let mut text_fields = self.mapping.static_text_fields.clone();
        if let Some(field) = &self.mapping.model_field {
            text_fields.insert(field.clone(), self.config.model.clone());
        }
        if let (Some(field), Some(language)) =
            (&self.mapping.language_field, &request.options.language_hint)
        {
            text_fields.insert(field.clone(), language.clone());
        }
        insert_toggle(
            &mut text_fields,
            self.mapping.timestamps_field.as_ref(),
            request.options.enable_timestamps,
        );
        insert_toggle(
            &mut text_fields,
            self.mapping.speaker_labels_field.as_ref(),
            request.options.enable_speaker_labels,
        );
        insert_toggle(
            &mut text_fields,
            self.mapping.confidence_field.as_ref(),
            request.options.enable_confidence,
        );

        let metadata = &request.artifact.reference.staging_metadata;
        let body = MultipartBody {
            file: MultipartFile {
                field_name: self.mapping.audio_field.clone(),
                reader: request.artifact.reader.clone(),
                upload_file_name: self.mapping.upload_file_name.clone(),
                media_type: metadata.mime_type.clone(),
                byte_length: metadata.byte_length,
            },
            text_fields,
        };

        Ok(ProviderHttpRequest {
            method: self.mapping.method,
            endpoint,
            body: ProviderHttpBody::Multipart(body),
            timeout,
            max_response_bytes: self.config.max_response_bytes,
            response_header_allowlist: self
                .mapping
                .response
                .remote_request_id_header
                .iter()
                .cloned()
                .collect(),
            idempotency: idempotency_header(&self.config, context),
        })
    }

    /// Normalizes one successful JSON response using only configured paths.
    fn parse_response(
        &self,
        response: &RawHttpResponse,
        artifact: &AudioArtifactRef,
        started_at: chrono::DateTime<Utc>,
    ) -> Result<Transcript, ProviderError> {
        let json: Value = serde_json::from_slice(&response.body).map_err(|_| {
            ProviderError::protocol(
                "invalid_provider_response",
                "Provider 返回的转写响应不是有效 JSON",
            )
        })?;
        let text = required_string(&self.mapping.response.text_path, &json)?;
        if text.trim().is_empty() {
            return Err(ProviderError::protocol(
                "empty_transcript",
                "Provider 返回了空转写",
            ));
        }

        let language = optional_string(self.mapping.response.language_path.as_ref(), &json)?;
        let segments = match &self.mapping.response.segments {
            Some(mapping) => parse_segments(mapping, &json)?,
            None => Vec::new(),
        };
        let remote_request_id = self
            .mapping
            .response
            .remote_request_id_header
            .as_deref()
            .and_then(|header| response.header(header))
            .map(str::to_owned);

        Ok(Transcript {
            schema_version: "1".to_owned(),
            text,
            language,
            duration_ms: artifact.staging_metadata.duration_ms,
            segments,
            provider_metadata: provider_metadata(&self.config, remote_request_id, started_at),
        })
    }
}

#[async_trait]
impl TranscriptionProvider for OpenAiCompatibleTranscriptionProvider {
    /// Returns only caller-configured capabilities and never upgrades evidence implicitly.
    fn capabilities(&self) -> TranscriptionCapabilities {
        self.mapping.capabilities.clone()
    }

    /// Streams one managed file, applies bounded retries, and normalizes the response.
    async fn transcribe(
        &self,
        context: &ProviderCallContext,
        request: TranscriptionRequest,
        credential: Option<&ProviderCredential>,
    ) -> Result<Transcript, ProviderError> {
        self.preflight(&request)?;
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
        let http_request = self.build_request(context, &request, endpoint, timeout)?;
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
        self.parse_response(&response, &request.artifact.reference, started_at)
    }
}

/// Configurable minutes provider whose request template and response path are explicit.
pub struct OpenAiCompatibleMinutesProvider {
    config: ProviderHttpConfig,
    mapping: MinutesHttpMapping,
    executor: Arc<dyn HttpExecutor>,
    semaphore: Arc<Semaphore>,
    rate_gate: Arc<RateGate>,
}

impl OpenAiCompatibleMinutesProvider {
    /// Creates a provider with an injected executor for deterministic HTTP behavior tests.
    pub fn with_executor(
        config: ProviderHttpConfig,
        mapping: MinutesHttpMapping,
        executor: Arc<dyn HttpExecutor>,
    ) -> Result<Self, ProviderError> {
        config.validate()?;
        mapping.validate()?;
        validate_http_capability_evidence(
            mapping.capabilities.evidence,
            mapping.capabilities.supports_async_jobs,
            mapping.capabilities.supports_remote_cancel,
        )?;
        if config.replay_safety != mapping.capabilities.replay_safety {
            return Err(ProviderError::configuration(
                "inconsistent_replay_safety",
                "Provider 能力与 HTTP 重放策略不一致",
            ));
        }
        let max_concurrent = config.max_concurrent;
        let min_interval = Duration::from_millis(config.min_request_interval_ms);
        Ok(Self {
            config,
            mapping,
            executor,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            rate_gate: Arc::new(RateGate::new(min_interval)),
        })
    }

    /// Creates a production provider backed by a redirect-disabled reqwest client.
    pub fn with_reqwest(
        config: ProviderHttpConfig,
        mapping: MinutesHttpMapping,
    ) -> Result<Self, ProviderError> {
        let executor = Arc::new(ReqwestHttpExecutor::new(Duration::from_millis(
            config.connect_timeout_ms,
        ))?);
        Self::with_executor(config, mapping, executor)
    }

    /// Renders the reviewed JSON template using exact-value placeholder replacement.
    fn render_body(&self, request: &MinutesGenerationRequest) -> Result<Value, ProviderError> {
        let mut body = self.mapping.body_template.clone();
        replace_placeholder(
            &mut body,
            &self.mapping.model_placeholder,
            Value::String(self.config.model.clone()),
        );
        replace_placeholder(
            &mut body,
            &self.mapping.prompt_placeholder,
            Value::String(request.prompt.clone()),
        );
        if let Some(schema_placeholder) = &self.mapping.schema_placeholder {
            replace_placeholder(&mut body, schema_placeholder, request.output_schema.clone());
        }
        Ok(body)
    }

    /// Builds the configured JSON request without exposing prompt or Schema to logs.
    fn build_request(
        &self,
        context: &ProviderCallContext,
        request: &MinutesGenerationRequest,
        endpoint: reqwest::Url,
        timeout: Duration,
    ) -> Result<ProviderHttpRequest, ProviderError> {
        Ok(ProviderHttpRequest {
            method: self.mapping.method,
            endpoint,
            body: ProviderHttpBody::Json(self.render_body(request)?),
            timeout,
            max_response_bytes: self.config.max_response_bytes,
            response_header_allowlist: self
                .mapping
                .remote_request_id_header
                .iter()
                .cloned()
                .collect(),
            idempotency: idempotency_header(&self.config, context),
        })
    }

    /// Extracts one untrusted JSON candidate for Agent 5's validator.
    fn parse_response(
        &self,
        response: &RawHttpResponse,
        schema_version: String,
        started_at: chrono::DateTime<Utc>,
    ) -> Result<MinutesCandidate, ProviderError> {
        let json: Value = serde_json::from_slice(&response.body).map_err(|_| {
            ProviderError::protocol(
                "invalid_provider_response",
                "Provider 返回的纪要响应不是有效 JSON",
            )
        })?;
        let content = self
            .mapping
            .response_content_path
            .select(&json)
            .ok_or_else(|| {
                ProviderError::protocol(
                    "invalid_provider_response",
                    "Provider 纪要响应缺少已配置字段",
                )
            })?;
        let value = match self.mapping.response_content_mode {
            JsonContentMode::JsonValue => content.clone(),
            JsonContentMode::JsonEncodedString => {
                let encoded = content.as_str().ok_or_else(|| {
                    ProviderError::protocol(
                        "invalid_provider_response",
                        "Provider 纪要字段类型与配置不一致",
                    )
                })?;
                serde_json::from_str(encoded).map_err(|_| {
                    ProviderError::protocol(
                        "invalid_minutes_json",
                        "Provider 纪要候选不是有效 JSON",
                    )
                })?
            }
        };
        let remote_request_id = self
            .mapping
            .remote_request_id_header
            .as_deref()
            .and_then(|header| response.header(header))
            .map(str::to_owned);
        Ok(MinutesCandidate {
            schema_version,
            value,
            provider_metadata: provider_metadata(&self.config, remote_request_id, started_at),
        })
    }
}

#[async_trait]
impl MinutesProvider for OpenAiCompatibleMinutesProvider {
    /// Returns only caller-configured capabilities and never upgrades evidence implicitly.
    fn capabilities(&self) -> MinutesCapabilities {
        self.mapping.capabilities.clone()
    }

    /// Generates an untrusted candidate with bounded retry, timeout, and cancellation.
    async fn generate_candidate(
        &self,
        context: &ProviderCallContext,
        request: MinutesGenerationRequest,
        credential: Option<&ProviderCredential>,
    ) -> Result<MinutesCandidate, ProviderError> {
        if request.prompt.trim().is_empty() {
            return Err(ProviderError::input(
                "empty_minutes_prompt",
                "纪要 Prompt 不能为空",
            ));
        }
        if !self
            .mapping
            .capabilities
            .supported_schema_versions
            .is_empty()
            && !self
                .mapping
                .capabilities
                .supported_schema_versions
                .contains(&request.schema_version)
        {
            return Err(ProviderError::input(
                "unsupported_schema_version",
                "当前 Provider 配置不支持目标纪要 Schema 版本",
            ));
        }
        if self
            .mapping
            .capabilities
            .max_input_characters
            .is_some_and(|limit| request.prompt.chars().count() as u64 > limit)
        {
            return Err(ProviderError::input(
                "minutes_input_too_large",
                "纪要输入超过当前 Provider 的已知限制",
            ));
        }

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
        let http_request = self.build_request(context, &request, endpoint, timeout)?;
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
        self.parse_response(&response, request.schema_version, started_at)
    }
}

/// Executes an HTTP request with finite retries and evidenced replay safety.
#[allow(clippy::too_many_arguments)]
async fn execute_with_retry(
    executor: &dyn HttpExecutor,
    config: &ProviderHttpConfig,
    request: &ProviderHttpRequest,
    credential: Option<&ProviderCredential>,
    cancellation_token: &super::CancellationToken,
    operation_id: &str,
    deadline: Instant,
    rate_gate: &RateGate,
) -> Result<RawHttpResponse, ProviderError> {
    let mut attempts_completed = 0_u32;
    loop {
        if cancellation_token.is_cancelled() {
            return Err(ProviderError::cancelled());
        }
        if Instant::now() >= deadline {
            return Err(ProviderError::operation_timeout());
        }

        rate_gate.wait(cancellation_token, deadline).await?;
        attempts_completed = attempts_completed.saturating_add(1);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ProviderError::operation_timeout());
        }
        let mut attempt_request = request.clone();
        attempt_request.timeout = attempt_request.timeout.min(remaining);
        let transport_result = tokio::select! {
            _ = cancellation_token.cancelled() => return Err(ProviderError::cancelled()),
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                return Err(ProviderError::operation_timeout());
            }
            result = executor.execute(
                &attempt_request,
                credential,
                &config.auth,
                cancellation_token,
            ) => result,
        };
        let result = transport_result
            .map_err(|error| error.into_provider_error())
            .and_then(|response| match classify_http_status(&response) {
                Some(error) => Err(error),
                None => Ok(response),
            });

        match result {
            Ok(response) => return Ok(response),
            Err(mut error) => {
                let replay_safe = is_replay_safe(
                    config.replay_safety,
                    config.has_idempotency_key(),
                    error.outcome,
                );
                error.replay_safe = replay_safe;
                if !error.retryable || !replay_safe || !config.retry.has_retry(attempts_completed) {
                    return Err(error);
                }

                let retry_index = attempts_completed;
                let delay = config.retry.delay_for(retry_index, operation_id, &error);
                if error.http_status == Some(429) {
                    rate_gate.penalize(delay).await;
                }
                sleep_cancellable(delay, cancellation_token, deadline).await?;
            }
        }
    }
}

/// Acquires a fair provider concurrency permit with cancellation and deadline support.
async fn acquire_permit(
    semaphore: Arc<Semaphore>,
    cancellation_token: &super::CancellationToken,
    deadline: Instant,
) -> Result<OwnedSemaphorePermit, ProviderError> {
    if Instant::now() >= deadline {
        return Err(ProviderError::operation_timeout());
    }
    tokio::select! {
        _ = cancellation_token.cancelled() => Err(ProviderError::cancelled()),
        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
            Err(ProviderError::operation_timeout())
        }
        permit = semaphore.acquire_owned() => permit.map_err(|_| ProviderError::configuration(
            "provider_queue_closed",
            "Provider 并发队列已关闭",
        )),
    }
}

/// Computes the earlier of caller and provider-profile overall deadlines.
fn effective_deadline(context: &ProviderCallContext, config: &ProviderHttpConfig) -> Instant {
    context
        .deadline
        .min(Instant::now() + Duration::from_millis(config.overall_timeout_ms))
}

/// Computes one finite attempt timeout that cannot exceed the operation deadline.
fn request_timeout(
    config: &ProviderHttpConfig,
    deadline: Instant,
) -> Result<Duration, ProviderError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(ProviderError::operation_timeout());
    }
    Ok(remaining.min(Duration::from_millis(config.request_timeout_ms)))
}

/// Requires a non-empty caller-owned credential whenever auth is configured.
fn require_credential(
    config: &ProviderHttpConfig,
    credential: Option<&ProviderCredential>,
) -> Result<(), ProviderError> {
    if !matches!(config.auth, super::ProviderCredentialPlacement::None)
        && credential.is_none_or(ProviderCredential::is_empty)
    {
        return Err(ProviderError::configuration(
            "provider_not_configured",
            "Provider 密钥尚未配置",
        ));
    }
    Ok(())
}

/// Builds a deterministic opaque idempotency value when the adapter is explicitly configured.
fn idempotency_header(
    config: &ProviderHttpConfig,
    context: &ProviderCallContext,
) -> Option<(String, String)> {
    config.idempotency_header.as_ref().map(|header| {
        let value = hex::encode(Sha256::digest(context.operation_id.as_bytes()));
        (header.clone(), value)
    })
}

/// Creates safe provider result metadata.
fn provider_metadata(
    config: &ProviderHttpConfig,
    remote_request_id: Option<String>,
    started_at: chrono::DateTime<Utc>,
) -> ProviderMetadata {
    ProviderMetadata {
        provider_id: config.provider_id.clone(),
        adapter_id: config.adapter_id.clone(),
        adapter_version: config.adapter_version.clone(),
        model: config.model.clone(),
        remote_request_id,
        started_at,
        completed_at: Utc::now(),
    }
}

/// Validates requested features against both capabilities and explicit request mappings.
fn validate_requested_features(
    options: &TranscriptionOptions,
    mapping: &TranscriptionHttpMapping,
) -> Result<(), ProviderError> {
    if options.language_hint.is_some() && mapping.language_field.is_none() {
        return Err(ProviderError::input(
            "unsupported_option",
            "当前 Provider 未配置语言提示字段",
        ));
    }
    let checks = [
        (
            options.enable_timestamps,
            mapping.capabilities.supports_timestamps,
            mapping.timestamps_field.is_some(),
        ),
        (
            options.enable_speaker_labels,
            mapping.capabilities.supports_speaker_labels,
            mapping.speaker_labels_field.is_some(),
        ),
        (
            options.enable_confidence,
            mapping.capabilities.supports_confidence,
            mapping.confidence_field.is_some(),
        ),
    ];
    if checks
        .into_iter()
        .any(|(requested, supported, mapped)| requested && (!supported || !mapped))
    {
        return Err(ProviderError::input(
            "unsupported_option",
            "当前 Provider 未配置所请求的转写能力",
        ));
    }
    Ok(())
}

/// Rejects capability claims that the current synchronous HTTP adapter cannot implement.
fn validate_http_capability_evidence(
    evidence: CapabilityEvidence,
    supports_async_jobs: bool,
    supports_remote_cancel: bool,
) -> Result<(), ProviderError> {
    if evidence == CapabilityEvidence::Mock {
        return Err(ProviderError::configuration(
            "invalid_capability_evidence",
            "真实 HTTP Provider 不能声明为 Mock 能力",
        ));
    }
    if supports_async_jobs || supports_remote_cancel {
        return Err(ProviderError::configuration(
            "unsupported_capability_config",
            "当前 HTTP adapter 尚未实现异步作业或远端撤销",
        ));
    }
    Ok(())
}

/// Inserts an enabled or explicitly configured disabled multipart field.
fn insert_toggle(
    fields: &mut BTreeMap<String, String>,
    mapping: Option<&ToggleFieldMapping>,
    enabled: bool,
) {
    if let Some(mapping) = mapping {
        if enabled {
            fields.insert(mapping.field_name.clone(), mapping.enabled_value.clone());
        } else if let Some(value) = &mapping.disabled_value {
            fields.insert(mapping.field_name.clone(), value.clone());
        }
    }
}

/// Extracts a required provider string without returning raw values in an error.
fn required_string(path: &JsonPath, root: &Value) -> Result<String, ProviderError> {
    path.select(root)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            ProviderError::protocol(
                "invalid_provider_response",
                "Provider 响应缺少已配置的文本字段",
            )
        })
}

/// Extracts an optional provider string while rejecting a configured wrong type.
fn optional_string(path: Option<&JsonPath>, root: &Value) -> Result<Option<String>, ProviderError> {
    let Some(path) = path else {
        return Ok(None);
    };
    match path.select(root) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(ProviderError::protocol(
            "invalid_provider_response",
            "Provider 响应字段类型与配置不一致",
        )),
    }
}

/// Normalizes configured provider segments and validates time/confidence invariants.
fn parse_segments(
    mapping: &SegmentResponseMapping,
    root: &Value,
) -> Result<Vec<TranscriptSegment>, ProviderError> {
    let values = mapping
        .segments_path
        .select(root)
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ProviderError::protocol(
                "invalid_provider_response",
                "Provider segments 字段类型与配置不一致",
            )
        })?;

    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let id = match &mapping.id_path {
                Some(path) => required_scalar_string(path, value)?,
                None => format!("segment-{}", index + 1),
            };
            let text = required_string(&mapping.text_path, value)?;
            let start_ms =
                optional_timestamp(mapping.start_path.as_ref(), value, mapping.timestamp_unit)?;
            let end_ms =
                optional_timestamp(mapping.end_path.as_ref(), value, mapping.timestamp_unit)?;
            if matches!((start_ms, end_ms), (Some(start), Some(end)) if start > end) {
                return Err(ProviderError::protocol(
                    "invalid_provider_response",
                    "Provider segment 时间范围无效",
                ));
            }
            let confidence = optional_number(mapping.confidence_path.as_ref(), value)?;
            if confidence.is_some_and(|number| !(0.0..=1.0).contains(&number)) {
                return Err(ProviderError::protocol(
                    "invalid_provider_response",
                    "Provider confidence 超出有效范围",
                ));
            }
            Ok(TranscriptSegment {
                id,
                start_ms,
                end_ms,
                speaker_label: optional_scalar_string(mapping.speaker_path.as_ref(), value)?,
                text,
                confidence,
            })
        })
        .collect()
}

/// Converts one required JSON scalar to a stable string identifier.
fn required_scalar_string(path: &JsonPath, root: &Value) -> Result<String, ProviderError> {
    match path.select(root) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(Value::Number(value)) => Ok(value.to_string()),
        _ => Err(ProviderError::protocol(
            "invalid_provider_response",
            "Provider segment 标识字段无效",
        )),
    }
}

/// Converts one optional JSON scalar to a string without inventing missing values.
fn optional_scalar_string(
    path: Option<&JsonPath>,
    root: &Value,
) -> Result<Option<String>, ProviderError> {
    let Some(path) = path else {
        return Ok(None);
    };
    match path.select(root) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(Value::Number(value)) => Ok(Some(value.to_string())),
        _ => Err(ProviderError::protocol(
            "invalid_provider_response",
            "Provider segment 字段类型无效",
        )),
    }
}

/// Extracts one optional numeric value with finite-number validation.
fn optional_number(path: Option<&JsonPath>, root: &Value) -> Result<Option<f64>, ProviderError> {
    let Some(path) = path else {
        return Ok(None);
    };
    match path.select(root) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_f64()
            .filter(|number| number.is_finite())
            .map(Some)
            .ok_or_else(|| {
                ProviderError::protocol("invalid_provider_response", "Provider 数值字段无效")
            }),
        _ => Err(ProviderError::protocol(
            "invalid_provider_response",
            "Provider 数值字段类型无效",
        )),
    }
}

/// Converts an optional provider timestamp to non-negative milliseconds.
fn optional_timestamp(
    path: Option<&JsonPath>,
    root: &Value,
    unit: TimestampUnit,
) -> Result<Option<u64>, ProviderError> {
    let Some(value) = optional_number(path, root)? else {
        return Ok(None);
    };
    if value < 0.0 {
        return Err(ProviderError::protocol(
            "invalid_provider_response",
            "Provider 时间戳不能为负数",
        ));
    }
    let milliseconds = match unit {
        TimestampUnit::Milliseconds => value,
        TimestampUnit::Seconds => value * 1_000.0,
    };
    if milliseconds > u64::MAX as f64 {
        return Err(ProviderError::protocol(
            "invalid_provider_response",
            "Provider 时间戳超出有效范围",
        ));
    }
    Ok(Some(milliseconds.round() as u64))
}

/// Validates all configured paths for one segment mapping.
fn validate_segment_mapping(mapping: &SegmentResponseMapping) -> Result<(), ProviderError> {
    mapping.segments_path.validate()?;
    mapping.text_path.validate()?;
    for path in [
        &mapping.id_path,
        &mapping.start_path,
        &mapping.end_path,
        &mapping.speaker_path,
        &mapping.confidence_path,
    ]
    .into_iter()
    .flatten()
    {
        path.validate()?;
    }
    Ok(())
}

/// Validates one multipart toggle field.
fn validate_toggle_field(mapping: &ToggleFieldMapping) -> Result<(), ProviderError> {
    validate_multipart_field(&mapping.field_name)?;
    if mapping.enabled_value.contains(['\r', '\n'])
        || mapping
            .disabled_value
            .as_ref()
            .is_some_and(|value| value.contains(['\r', '\n']))
    {
        return Err(ProviderError::configuration(
            "invalid_request_mapping",
            "Provider 选项值包含非法字符",
        ));
    }
    Ok(())
}

/// Validates a multipart field name as a small printable token.
fn validate_multipart_field(field: &str) -> Result<(), ProviderError> {
    if field.is_empty()
        || field.len() > 128
        || !field
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-.[].".contains(&byte))
    {
        return Err(ProviderError::configuration(
            "invalid_request_mapping",
            "Provider multipart 字段名无效",
        ));
    }
    Ok(())
}

/// Validates one explicitly allowlisted response header name.
fn validate_response_header(header: &str) -> Result<(), ProviderError> {
    HeaderName::from_bytes(header.as_bytes()).map_err(|_| {
        ProviderError::configuration("invalid_response_mapping", "Provider 响应 Header 映射无效")
    })?;
    Ok(())
}

/// Returns whether a field name looks like a forbidden embedded credential.
fn looks_secret_field(field: &str) -> bool {
    let normalized = field.to_ascii_lowercase();
    [
        "api_key",
        "apikey",
        "token",
        "secret",
        "password",
        "authorization",
        "cookie",
        "credential",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

/// Recursively checks a JSON request template for credential-like object keys.
fn contains_secret_key(value: &Value) -> bool {
    match value {
        Value::Object(map) => map
            .iter()
            .any(|(key, value)| looks_secret_field(key) || contains_secret_key(value)),
        Value::Array(values) => values.iter().any(contains_secret_key),
        _ => false,
    }
}

/// Counts exact string placeholder occurrences in a JSON template.
fn count_placeholder(value: &Value, placeholder: &str) -> usize {
    match value {
        Value::String(value) => usize::from(value == placeholder),
        Value::Array(values) => values
            .iter()
            .map(|value| count_placeholder(value, placeholder))
            .sum(),
        Value::Object(map) => map
            .values()
            .map(|value| count_placeholder(value, placeholder))
            .sum(),
        _ => 0,
    }
}

/// Replaces every exact string placeholder with one trusted JSON value.
fn replace_placeholder(value: &mut Value, placeholder: &str, replacement: Value) {
    match value {
        Value::String(current) if current == placeholder => *value = replacement,
        Value::Array(values) => {
            for value in values {
                replace_placeholder(value, placeholder, replacement.clone());
            }
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                replace_placeholder(value, placeholder, replacement.clone());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::json;

    use super::{
        JsonContentMode, JsonPath, MinutesHttpMapping, OpenAiCompatibleMinutesProvider,
        OpenAiCompatibleTranscriptionProvider, TranscriptionHttpMapping,
        TranscriptionResponseMapping,
    };
    use crate::ingest::AudioSourceKind;
    use crate::providers::{
        AudioArtifactRef, AuthStrategy, CancellationToken, CapabilityEvidence, HttpExecutor,
        HttpMethod, ManagedAudioArtifact, MinutesCapabilities, MinutesGenerationRequest,
        MinutesProvider, OperationOutcome, ProviderCallContext, ProviderCredential,
        ProviderCredentialPlacement, ProviderHttpConfig, ProviderHttpRequest, RawHttpResponse,
        ReplaySafety, RetryPolicy, StagingMetadata, Transcript, TranscriptionCapabilities,
        TranscriptionOptions, TranscriptionProvider, TranscriptionRequest, TransportError,
        TransportErrorKind,
    };

    /// Scripted executor that exposes only call count and predetermined safe responses.
    struct ScriptedExecutor {
        responses: Mutex<VecDeque<Result<RawHttpResponse, TransportError>>>,
        calls: AtomicUsize,
    }

    impl ScriptedExecutor {
        /// Creates a deterministic executor response queue.
        fn new(responses: Vec<Result<RawHttpResponse, TransportError>>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                calls: AtomicUsize::new(0),
            }
        }

        /// Returns how many HTTP attempts were made.
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::Acquire)
        }
    }

    #[async_trait]
    impl HttpExecutor for ScriptedExecutor {
        /// Returns the next predetermined response without inspecting sensitive bodies.
        async fn execute(
            &self,
            _request: &ProviderHttpRequest,
            _credential: Option<&ProviderCredential>,
            _auth: &ProviderCredentialPlacement,
            _cancellation_token: &CancellationToken,
        ) -> Result<RawHttpResponse, TransportError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            self.responses
                .lock()
                .expect("script mutex should not be poisoned")
                .pop_front()
                .expect("script should contain enough responses")
        }
    }

    /// Executor that intentionally ignores the token so the adapter deadline/cancel guard is tested.
    struct SlowExecutor;

    #[async_trait]
    impl HttpExecutor for SlowExecutor {
        /// Sleeps much longer than the test cancellation and should be dropped by the adapter.
        async fn execute(
            &self,
            _request: &ProviderHttpRequest,
            _credential: Option<&ProviderCredential>,
            _auth: &ProviderCredentialPlacement,
            _cancellation_token: &CancellationToken,
        ) -> Result<RawHttpResponse, TransportError> {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(RawHttpResponse::new(200, BTreeMap::new(), Vec::new()))
        }
    }

    /// Creates an unverified loopback profile for deterministic tests.
    fn test_http_config(replay_safety: ReplaySafety) -> ProviderHttpConfig {
        ProviderHttpConfig {
            provider_id: "test-provider".to_owned(),
            adapter_id: "explicit-test-codec".to_owned(),
            adapter_version: "1".to_owned(),
            endpoint: "http://127.0.0.1/mock".to_owned(),
            model: "test-model".to_owned(),
            auth: AuthStrategy::Bearer,
            connect_timeout_ms: 500,
            request_timeout_ms: 500,
            overall_timeout_ms: 2_000,
            max_response_bytes: 64 * 1024,
            max_concurrent: 1,
            min_request_interval_ms: 0,
            retry: RetryPolicy {
                max_retries: 2,
                base_delay_ms: 1,
                max_delay_ms: 4,
                max_retry_after_ms: 10,
            },
            replay_safety,
            idempotency_header: None,
            allow_insecure_loopback: true,
        }
    }

    /// Creates safe artifact metadata without reading the repository test audio.
    fn test_artifact() -> ManagedAudioArtifact {
        ManagedAudioArtifact::new(
            AudioArtifactRef {
                id: "artifact-test".to_owned(),
                import_batch_id: None,
                source_kind: AudioSourceKind::UserSelectedFile,
                staging_metadata: StagingMetadata {
                    mime_type: "audio/test".to_owned(),
                    byte_length: 8,
                    duration_ms: Some(1_000),
                    sha256: None,
                    validated_at: Utc::now(),
                },
            },
            Arc::new(|| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "scripted executor never opens the artifact",
                ))
            }),
        )
    }

    /// Creates explicit transcription mappings with no real-provider field claims.
    fn test_transcription_mapping(replay_safety: ReplaySafety) -> TranscriptionHttpMapping {
        TranscriptionHttpMapping {
            method: HttpMethod::Post,
            audio_field: "fixture_audio".to_owned(),
            upload_file_name: "fixture.audio".to_owned(),
            model_field: Some("fixture_model".to_owned()),
            language_field: None,
            timestamps_field: None,
            speaker_labels_field: None,
            confidence_field: None,
            static_text_fields: BTreeMap::new(),
            response: TranscriptionResponseMapping {
                text_path: JsonPath::keys(&["fixture", "text"]),
                language_path: None,
                segments: None,
                remote_request_id_header: None,
            },
            capabilities: TranscriptionCapabilities {
                evidence: CapabilityEvidence::Unverified,
                accepted_media_types: vec!["audio/test".to_owned()],
                max_audio_bytes: None,
                max_duration_ms: None,
                supports_async_jobs: false,
                supports_timestamps: false,
                supports_speaker_labels: false,
                supports_confidence: false,
                supports_remote_cancel: false,
                replay_safety,
            },
        }
    }

    /// Creates one short provider context for tests.
    fn test_context() -> ProviderCallContext {
        ProviderCallContext::with_timeout(
            "task-test",
            "operation-test",
            CancellationToken::new(),
            Duration::from_secs(2),
        )
    }

    /// Creates a runtime-only dummy credential that is not a stored test key.
    fn test_credential() -> ProviderCredential {
        ProviderCredential::new("x".repeat(32))
    }

    /// Verifies explicit response mapping and bounded retry after 429.
    #[tokio::test]
    async fn transcription_retries_429_then_normalizes_success() {
        let executor = Arc::new(ScriptedExecutor::new(vec![
            Ok(RawHttpResponse::new(
                429,
                BTreeMap::from([("retry-after".to_owned(), "0".to_owned())]),
                Vec::new(),
            )),
            Ok(RawHttpResponse::new(
                200,
                BTreeMap::new(),
                serde_json::to_vec(&json!({"fixture": {"text": "short fixture"}}))
                    .expect("fixture should serialize"),
            )),
        ]));
        let provider = OpenAiCompatibleTranscriptionProvider::with_executor(
            test_http_config(ReplaySafety::VerifiedAlwaysSafe),
            test_transcription_mapping(ReplaySafety::VerifiedAlwaysSafe),
            executor.clone(),
        )
        .expect("provider should build");
        let credential = test_credential();
        let transcript = provider
            .transcribe(
                &test_context(),
                TranscriptionRequest {
                    artifact: test_artifact(),
                    options: TranscriptionOptions::default(),
                },
                Some(&credential),
            )
            .await
            .expect("second response should succeed");
        assert_eq!(transcript.text, "short fixture");
        assert_eq!(executor.call_count(), 2);
    }

    /// Verifies that 401 is not retried and the error contains no credential.
    #[tokio::test]
    async fn authentication_failure_is_terminal_and_redacted() {
        let executor = Arc::new(ScriptedExecutor::new(vec![Ok(RawHttpResponse::new(
            401,
            BTreeMap::new(),
            b"sentinel-response-body".to_vec(),
        ))]));
        let provider = OpenAiCompatibleTranscriptionProvider::with_executor(
            test_http_config(ReplaySafety::VerifiedAlwaysSafe),
            test_transcription_mapping(ReplaySafety::VerifiedAlwaysSafe),
            executor.clone(),
        )
        .expect("provider should build");
        let credential = test_credential();
        let error = provider
            .transcribe(
                &test_context(),
                TranscriptionRequest {
                    artifact: test_artifact(),
                    options: TranscriptionOptions::default(),
                },
                Some(&credential),
            )
            .await
            .expect_err("401 should fail");
        let rendered = format!("{error:?}");
        assert_eq!(error.code, "http_401");
        assert_eq!(executor.call_count(), 1);
        assert!(!rendered.contains(&"x".repeat(32)));
        assert!(!rendered.contains("sentinel-response-body"));
    }

    /// Verifies that replay-safe 5xx retries stop at the configured total attempt count.
    #[tokio::test]
    async fn provider_5xx_retries_are_bounded() {
        let responses = (0..3)
            .map(|_| Ok(RawHttpResponse::new(500, BTreeMap::new(), Vec::new())))
            .collect();
        let executor = Arc::new(ScriptedExecutor::new(responses));
        let provider = OpenAiCompatibleTranscriptionProvider::with_executor(
            test_http_config(ReplaySafety::VerifiedAlwaysSafe),
            test_transcription_mapping(ReplaySafety::VerifiedAlwaysSafe),
            executor.clone(),
        )
        .expect("provider should build");
        let credential = test_credential();
        let error = provider
            .transcribe(
                &test_context(),
                TranscriptionRequest {
                    artifact: test_artifact(),
                    options: TranscriptionOptions::default(),
                },
                Some(&credential),
            )
            .await
            .expect_err("all scripted attempts should fail");
        assert_eq!(error.code, "http_5xx");
        assert_eq!(executor.call_count(), 3);
    }

    /// Verifies that a pre-send network failure is retried under the conservative replay policy.
    #[tokio::test]
    async fn pre_send_network_failure_can_retry() {
        let executor = Arc::new(ScriptedExecutor::new(vec![
            Err(TransportError::new(
                TransportErrorKind::Network,
                OperationOutcome::NotSent,
            )),
            Ok(RawHttpResponse::new(
                200,
                BTreeMap::new(),
                serde_json::to_vec(&json!({"fixture": {"text": "short fixture"}}))
                    .expect("fixture should serialize"),
            )),
        ]));
        let provider = OpenAiCompatibleTranscriptionProvider::with_executor(
            test_http_config(ReplaySafety::BeforeRequestBodySentOnly),
            test_transcription_mapping(ReplaySafety::BeforeRequestBodySentOnly),
            executor.clone(),
        )
        .expect("provider should build");
        let credential = test_credential();
        let transcript = provider
            .transcribe(
                &test_context(),
                TranscriptionRequest {
                    artifact: test_artifact(),
                    options: TranscriptionOptions::default(),
                },
                Some(&credential),
            )
            .await
            .expect("second attempt should succeed");
        assert_eq!(transcript.text, "short fixture");
        assert_eq!(executor.call_count(), 2);
    }

    /// Verifies that cancellation drops an executor that does not cooperate with the token.
    #[tokio::test]
    async fn adapter_cancellation_interrupts_http_executor() {
        let provider = OpenAiCompatibleTranscriptionProvider::with_executor(
            test_http_config(ReplaySafety::NeverAutomaticallyReplay),
            test_transcription_mapping(ReplaySafety::NeverAutomaticallyReplay),
            Arc::new(SlowExecutor),
        )
        .expect("provider should build");
        let token = CancellationToken::new();
        let cancel_token = token.clone();
        let context = ProviderCallContext::with_timeout(
            "task-test",
            "operation-test",
            token,
            Duration::from_secs(2),
        );
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cancel_token.cancel();
        });
        let credential = test_credential();
        let error = provider
            .transcribe(
                &context,
                TranscriptionRequest {
                    artifact: test_artifact(),
                    options: TranscriptionOptions::default(),
                },
                Some(&credential),
            )
            .await
            .expect_err("cancel should win");
        cancel_task.await.expect("cancel task should finish");
        assert_eq!(error.code, "cancelled");
    }

    /// Verifies that malformed successful bodies become protocol errors without body leakage.
    #[tokio::test]
    async fn malformed_success_body_is_protocol_error() {
        let executor = Arc::new(ScriptedExecutor::new(vec![Ok(RawHttpResponse::new(
            200,
            BTreeMap::new(),
            b"sentinel-malformed-body".to_vec(),
        ))]));
        let provider = OpenAiCompatibleTranscriptionProvider::with_executor(
            test_http_config(ReplaySafety::NeverAutomaticallyReplay),
            test_transcription_mapping(ReplaySafety::NeverAutomaticallyReplay),
            executor,
        )
        .expect("provider should build");
        let credential = test_credential();
        let error = provider
            .transcribe(
                &test_context(),
                TranscriptionRequest {
                    artifact: test_artifact(),
                    options: TranscriptionOptions::default(),
                },
                Some(&credential),
            )
            .await
            .expect_err("malformed JSON must fail");
        assert_eq!(error.code, "invalid_provider_response");
        assert!(!format!("{error:?}").contains("sentinel-malformed-body"));
    }

    /// Verifies that a JSON-encoded minutes candidate remains untrusted output for validation.
    #[tokio::test]
    async fn minutes_provider_uses_explicit_template_and_response_path() {
        let executor = Arc::new(ScriptedExecutor::new(vec![Ok(RawHttpResponse::new(
            200,
            BTreeMap::new(),
            serde_json::to_vec(&json!({
                "fixture": {"candidate": "{\"schemaVersion\":\"1.0.0\"}"}
            }))
            .expect("fixture should serialize"),
        ))]));
        let mapping = MinutesHttpMapping {
            method: HttpMethod::Post,
            body_template: json!({
                "fixtureModel": "__MODEL__",
                "fixturePrompt": "__PROMPT__",
                "fixtureSchema": "__SCHEMA__"
            }),
            model_placeholder: "__MODEL__".to_owned(),
            prompt_placeholder: "__PROMPT__".to_owned(),
            schema_placeholder: Some("__SCHEMA__".to_owned()),
            response_content_path: JsonPath::keys(&["fixture", "candidate"]),
            response_content_mode: JsonContentMode::JsonEncodedString,
            remote_request_id_header: None,
            capabilities: MinutesCapabilities {
                evidence: CapabilityEvidence::Unverified,
                supports_json_schema: true,
                supported_schema_versions: vec!["1.0.0".to_owned()],
                max_input_characters: None,
                supports_async_jobs: false,
                supports_remote_cancel: false,
                replay_safety: ReplaySafety::NeverAutomaticallyReplay,
            },
        };
        let provider = OpenAiCompatibleMinutesProvider::with_executor(
            test_http_config(ReplaySafety::NeverAutomaticallyReplay),
            mapping,
            executor,
        )
        .expect("provider should build");
        let credential = test_credential();
        let candidate = provider
            .generate_candidate(
                &test_context(),
                MinutesGenerationRequest {
                    prompt: "short non-sensitive fixture".to_owned(),
                    output_schema: json!({"type": "object"}),
                    schema_version: "1.0.0".to_owned(),
                },
                Some(&credential),
            )
            .await
            .expect("candidate should parse");
        assert_eq!(candidate.value["schemaVersion"], "1.0.0");
    }

    /// Keeps imported transcript values out of unrelated debug assertions.
    #[allow(dead_code)]
    fn assert_transcript_type_is_provider_neutral(_transcript: Transcript) {}

    /// Keeps the test module explicit about unknown remote outcomes.
    #[allow(dead_code)]
    fn assert_unknown_outcome_is_representable() {
        let _ = OperationOutcome::Unknown;
    }
}
