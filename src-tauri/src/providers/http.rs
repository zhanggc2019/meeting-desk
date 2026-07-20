use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderName, HeaderValue, RETRY_AFTER};
use reqwest::{multipart, Client, Url};
use serde::{Deserialize, Serialize};

use super::{
    CancellationToken, OperationOutcome, ProviderCredential, ProviderError, ProviderErrorCategory,
    ReplaySafety, RetryPolicy,
};

const MAX_RESPONSE_LIMIT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1_000;

/// Explicitly configured credential placement for an HTTP adapter.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderCredentialPlacement {
    None,
    Bearer,
    Header {
        header_name: String,
        prefix: Option<String>,
    },
}

/// Backwards-friendly name for the configured credential placement.
pub type AuthStrategy = ProviderCredentialPlacement;

impl fmt::Debug for ProviderCredentialPlacement {
    /// Formats only the strategy kind and redacts custom header details.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::Bearer => formatter.write_str("Bearer([REDACTED])"),
            Self::Header { .. } => formatter.write_str("Header([REDACTED])"),
        }
    }
}

/// Supported HTTP methods for configurable provider requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpMethod {
    Get,
    Post,
}

/// Non-secret HTTP settings shared by one provider adapter instance.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHttpConfig {
    pub provider_id: String,
    pub adapter_id: String,
    pub adapter_version: String,
    pub endpoint: String,
    pub model: String,
    pub auth: ProviderCredentialPlacement,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub overall_timeout_ms: u64,
    pub max_response_bytes: u64,
    pub max_concurrent: usize,
    pub min_request_interval_ms: u64,
    pub retry: RetryPolicy,
    pub replay_safety: ReplaySafety,
    pub idempotency_header: Option<String>,
    pub allow_insecure_loopback: bool,
}

impl ProviderHttpConfig {
    /// Validates the non-secret provider profile without making a network request.
    pub fn validate(&self) -> Result<Url, ProviderError> {
        if self.provider_id.trim().is_empty()
            || self.adapter_id.trim().is_empty()
            || self.adapter_version.trim().is_empty()
            || self.model.trim().is_empty()
        {
            return Err(ProviderError::configuration(
                "invalid_provider_config",
                "Provider 标识、适配器版本和模型不能为空",
            ));
        }

        validate_timeout(self.connect_timeout_ms)?;
        validate_timeout(self.request_timeout_ms)?;
        validate_timeout(self.overall_timeout_ms)?;
        self.retry.validate()?;

        if self.max_response_bytes == 0 || self.max_response_bytes > MAX_RESPONSE_LIMIT_BYTES {
            return Err(ProviderError::configuration(
                "invalid_response_limit",
                "响应大小上限无效",
            ));
        }
        if self.max_concurrent == 0 || self.max_concurrent > 16 {
            return Err(ProviderError::configuration(
                "invalid_concurrency_limit",
                "Provider 并发数必须在 1 到 16 之间",
            ));
        }

        validate_auth_strategy(&self.auth)?;
        if let Some(header) = &self.idempotency_header {
            validate_header_name(header, "invalid_idempotency_header")?;
        }
        if self.replay_safety == ReplaySafety::SafeWithVerifiedIdempotencyKey
            && self.idempotency_header.is_none()
        {
            return Err(ProviderError::configuration(
                "missing_idempotency_header",
                "幂等重放策略需要显式配置已验证的幂等 Header",
            ));
        }

        let url = Url::parse(&self.endpoint).map_err(|_| {
            ProviderError::configuration("invalid_provider_endpoint", "Provider 地址无效")
        })?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ProviderError::configuration(
                "invalid_provider_endpoint",
                "Provider 地址不得包含凭据、查询参数或片段",
            ));
        }

        let is_https = url.scheme() == "https";
        let is_allowed_loopback = self.allow_insecure_loopback
            && url.scheme() == "http"
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
        if !is_https && !is_allowed_loopback {
            return Err(ProviderError::configuration(
                "insecure_provider_endpoint",
                "生产 Provider 必须使用 HTTPS",
            ));
        }
        Ok(url)
    }

    /// Returns whether verified idempotency is configured for retry decisions.
    pub(crate) fn has_idempotency_key(&self) -> bool {
        self.replay_safety == ReplaySafety::SafeWithVerifiedIdempotencyKey
            && self.idempotency_header.is_some()
    }
}

impl fmt::Debug for ProviderHttpConfig {
    /// Formats safe profile metadata while hiding endpoint, model, and auth details.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderHttpConfig")
            .field("provider_id", &self.provider_id)
            .field("adapter_id", &self.adapter_id)
            .field("adapter_version", &self.adapter_version)
            .field("endpoint", &"[REDACTED]")
            .field("model", &"[CONFIGURED]")
            .field("auth", &self.auth)
            .field("connect_timeout_ms", &self.connect_timeout_ms)
            .field("request_timeout_ms", &self.request_timeout_ms)
            .field("overall_timeout_ms", &self.overall_timeout_ms)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("max_concurrent", &self.max_concurrent)
            .field("min_request_interval_ms", &self.min_request_interval_ms)
            .field("retry", &self.retry)
            .field("replay_safety", &self.replay_safety)
            .field("idempotency_header", &self.idempotency_header.is_some())
            .field("allow_insecure_loopback", &self.allow_insecure_loopback)
            .finish()
    }
}

/// A streamed multipart file part sourced from an ingest-managed staged copy.
#[derive(Clone)]
pub struct MultipartFile {
    pub field_name: String,
    pub reader: std::sync::Arc<dyn super::AudioArtifactReader>,
    pub upload_file_name: String,
    pub media_type: String,
    pub byte_length: u64,
}

impl fmt::Debug for MultipartFile {
    /// Formats only safe size and media metadata.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MultipartFile")
            .field("field_name", &"[CONFIGURED]")
            .field("reader", &"[REDACTED]")
            .field("upload_file_name", &"[CONFIGURED]")
            .field("media_type", &self.media_type)
            .field("byte_length", &self.byte_length)
            .finish()
    }
}

/// Explicit multipart request body produced by a transcription codec.
#[derive(Clone)]
pub struct MultipartBody {
    pub file: MultipartFile,
    pub text_fields: BTreeMap<String, String>,
}

impl fmt::Debug for MultipartBody {
    /// Redacts multipart field values and file paths.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MultipartBody")
            .field("file", &self.file)
            .field("text_field_count", &self.text_fields.len())
            .finish()
    }
}

/// Sensitive request body that must never be serialized or logged.
#[derive(Clone)]
pub enum ProviderHttpBody {
    Empty,
    Json(serde_json::Value),
    Multipart(MultipartBody),
}

impl fmt::Debug for ProviderHttpBody {
    /// Formats only the body kind and never body content.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Empty"),
            Self::Json(_) => formatter.write_str("Json([REDACTED])"),
            Self::Multipart(body) => formatter.debug_tuple("Multipart").field(body).finish(),
        }
    }
}

/// An internal HTTP request produced by an explicit provider codec.
#[derive(Clone)]
pub struct ProviderHttpRequest {
    pub method: HttpMethod,
    pub endpoint: Url,
    pub body: ProviderHttpBody,
    pub headers: BTreeMap<String, String>,
    pub timeout: Duration,
    pub max_response_bytes: u64,
    pub response_header_allowlist: Vec<String>,
    pub idempotency: Option<(String, String)>,
}

impl fmt::Debug for ProviderHttpRequest {
    /// Formats safe request metadata without URL, headers, or body values.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderHttpRequest")
            .field("method", &self.method)
            .field("endpoint", &"[REDACTED]")
            .field("body", &self.body)
            .field("header_count", &self.headers.len())
            .field("timeout", &self.timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .field(
                "response_header_allowlist_count",
                &self.response_header_allowlist.len(),
            )
            .field("idempotency", &self.idempotency.is_some())
            .finish()
    }
}

/// Size-limited raw HTTP response kept inside the provider adapter boundary.
#[derive(Clone)]
pub struct RawHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl RawHttpResponse {
    /// Creates a raw response for a production executor or deterministic test double.
    pub fn new(status: u16, headers: BTreeMap<String, String>, body: Vec<u8>) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }

    /// Returns a captured allowlisted header using a case-insensitive lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

impl fmt::Debug for RawHttpResponse {
    /// Formats only status, sizes, and header names.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RawHttpResponse")
            .field("status", &self.status)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("body_length", &self.body.len())
            .finish()
    }
}

/// Transport failure kinds that are independent of reqwest internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportErrorKind {
    Connect,
    Timeout,
    Network,
    RequestBuild,
    BodyRead,
    LocalFile,
    ResponseTooLarge,
    Cancelled,
}

/// Sanitized transport error with explicit remote outcome semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError {
    pub kind: TransportErrorKind,
    pub outcome: OperationOutcome,
}

impl TransportError {
    /// Creates a sanitized transport error without retaining the underlying request.
    pub fn new(kind: TransportErrorKind, outcome: OperationOutcome) -> Self {
        Self { kind, outcome }
    }

    /// Converts the transport failure into the stable ProviderError contract.
    pub fn into_provider_error(self) -> ProviderError {
        match self.kind {
            TransportErrorKind::Connect => ProviderError::new(
                "network_unavailable",
                ProviderErrorCategory::Network,
                true,
                false,
                "无法连接云端服务",
                None,
                None,
                self.outcome,
            ),
            TransportErrorKind::Timeout => ProviderError::new(
                "request_timeout",
                ProviderErrorCategory::Timeout,
                true,
                false,
                "云端请求超时",
                None,
                None,
                self.outcome,
            ),
            TransportErrorKind::Network | TransportErrorKind::BodyRead => ProviderError::new(
                "network_unavailable",
                ProviderErrorCategory::Network,
                true,
                false,
                "云端连接中断",
                None,
                None,
                self.outcome,
            ),
            TransportErrorKind::RequestBuild => ProviderError::configuration(
                "provider_request_build_failed",
                "Provider 请求配置无效",
            ),
            TransportErrorKind::LocalFile => ProviderError::new(
                "staged_file_unavailable",
                ProviderErrorCategory::LocalResource,
                false,
                false,
                "受管音频文件不可读取",
                None,
                None,
                OperationOutcome::NotSent,
            ),
            TransportErrorKind::ResponseTooLarge => ProviderError::new(
                "response_too_large",
                ProviderErrorCategory::Protocol,
                false,
                false,
                "Provider 响应超过安全上限",
                None,
                None,
                OperationOutcome::Failed,
            ),
            TransportErrorKind::Cancelled => ProviderError::cancelled(),
        }
    }
}

/// Testable boundary around actual HTTP execution.
#[async_trait]
pub trait HttpExecutor: Send + Sync {
    /// Executes one size-limited request while observing cancellation.
    async fn execute(
        &self,
        request: &ProviderHttpRequest,
        credential: Option<&ProviderCredential>,
        auth: &ProviderCredentialPlacement,
        cancellation_token: &CancellationToken,
    ) -> Result<RawHttpResponse, TransportError>;
}

/// Reqwest-backed production HTTP executor with redirects disabled.
pub struct ReqwestHttpExecutor {
    client: Client,
}

impl ReqwestHttpExecutor {
    /// Builds an executor with a finite connection timeout and no credential-forwarding redirects.
    pub fn new(connect_timeout: Duration) -> Result<Self, ProviderError> {
        let client = Client::builder()
            .connect_timeout(connect_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| {
                ProviderError::configuration(
                    "http_client_build_failed",
                    "无法初始化安全 HTTP 客户端",
                )
            })?;
        Ok(Self { client })
    }

    /// Builds a reqwest multipart form while streaming the managed staged file.
    async fn build_multipart(
        body: &MultipartBody,
        cancellation_token: &CancellationToken,
    ) -> Result<multipart::Form, TransportError> {
        if cancellation_token.is_cancelled() {
            return Err(TransportError::new(
                TransportErrorKind::Cancelled,
                OperationOutcome::NotSent,
            ));
        }
        let file = body.file.reader.open_readonly().map_err(|_| {
            TransportError::new(TransportErrorKind::LocalFile, OperationOutcome::NotSent)
        })?;
        let file = tokio::fs::File::from_std(file);
        let file_part = multipart::Part::stream_with_length(file, body.file.byte_length)
            .file_name(body.file.upload_file_name.clone())
            .mime_str(&body.file.media_type)
            .map_err(|_| {
                TransportError::new(TransportErrorKind::RequestBuild, OperationOutcome::NotSent)
            })?;

        let mut form = multipart::Form::new().part(body.file.field_name.clone(), file_part);
        for (name, value) in &body.text_fields {
            form = form.text(name.clone(), value.clone());
        }
        Ok(form)
    }

    /// Injects a caller-owned secret into the configured authentication location.
    fn apply_authentication(
        mut request: reqwest::RequestBuilder,
        credential: Option<&ProviderCredential>,
        auth: &ProviderCredentialPlacement,
    ) -> Result<reqwest::RequestBuilder, TransportError> {
        match auth {
            ProviderCredentialPlacement::None => Ok(request),
            ProviderCredentialPlacement::Bearer => {
                let secret = required_secret(credential)?;
                request = request.bearer_auth(secret);
                Ok(request)
            }
            ProviderCredentialPlacement::Header {
                header_name,
                prefix,
            } => {
                let secret = required_secret(credential)?;
                let header_name = HeaderName::from_bytes(header_name.as_bytes()).map_err(|_| {
                    TransportError::new(TransportErrorKind::RequestBuild, OperationOutcome::NotSent)
                })?;
                let value = format!("{}{}", prefix.as_deref().unwrap_or_default(), secret);
                let header_value = HeaderValue::from_str(&value).map_err(|_| {
                    TransportError::new(TransportErrorKind::RequestBuild, OperationOutcome::NotSent)
                })?;
                request = request.header(header_name, header_value);
                Ok(request)
            }
        }
    }

    /// Converts a reqwest error into a sanitized transport classification.
    fn classify_reqwest_error(error: &reqwest::Error) -> TransportError {
        if error.is_connect() {
            return TransportError::new(TransportErrorKind::Connect, OperationOutcome::NotSent);
        }
        if error.is_timeout() {
            return TransportError::new(TransportErrorKind::Timeout, OperationOutcome::Unknown);
        }
        if error.is_builder() || error.is_request() {
            return TransportError::new(
                TransportErrorKind::RequestBuild,
                OperationOutcome::NotSent,
            );
        }
        TransportError::new(TransportErrorKind::Network, OperationOutcome::Unknown)
    }
}

#[async_trait]
impl HttpExecutor for ReqwestHttpExecutor {
    /// Executes one request without logging URL, headers, or body content.
    async fn execute(
        &self,
        request: &ProviderHttpRequest,
        credential: Option<&ProviderCredential>,
        auth: &ProviderCredentialPlacement,
        cancellation_token: &CancellationToken,
    ) -> Result<RawHttpResponse, TransportError> {
        if cancellation_token.is_cancelled() {
            return Err(TransportError::new(
                TransportErrorKind::Cancelled,
                OperationOutcome::NotSent,
            ));
        }

        let mut builder = match request.method {
            HttpMethod::Get => self.client.get(request.endpoint.clone()),
            HttpMethod::Post => self.client.post(request.endpoint.clone()),
        }
        .timeout(request.timeout);

        builder = Self::apply_authentication(builder, credential, auth)?;
        for (name, value) in &request.headers {
            if is_sensitive_managed_header(name) {
                return Err(TransportError::new(
                    TransportErrorKind::RequestBuild,
                    OperationOutcome::NotSent,
                ));
            }
            let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                TransportError::new(TransportErrorKind::RequestBuild, OperationOutcome::NotSent)
            })?;
            let header_value = HeaderValue::from_str(value).map_err(|_| {
                TransportError::new(TransportErrorKind::RequestBuild, OperationOutcome::NotSent)
            })?;
            builder = builder.header(header_name, header_value);
        }
        if let Some((name, value)) = &request.idempotency {
            let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                TransportError::new(TransportErrorKind::RequestBuild, OperationOutcome::NotSent)
            })?;
            let header_value = HeaderValue::from_str(value).map_err(|_| {
                TransportError::new(TransportErrorKind::RequestBuild, OperationOutcome::NotSent)
            })?;
            builder = builder.header(header_name, header_value);
        }

        builder = match &request.body {
            ProviderHttpBody::Empty => builder,
            ProviderHttpBody::Json(body) => builder.json(body),
            ProviderHttpBody::Multipart(body) => {
                let form = Self::build_multipart(body, cancellation_token).await?;
                builder.multipart(form)
            }
        };

        let mut response = tokio::select! {
            _ = cancellation_token.cancelled() => {
                return Err(TransportError::new(
                    TransportErrorKind::Cancelled,
                    OperationOutcome::Unknown,
                ));
            }
            result = builder.send() => result.map_err(|error| Self::classify_reqwest_error(&error))?,
        };

        let status = response.status().as_u16();
        let mut headers = BTreeMap::new();
        let mut allowed_headers = request.response_header_allowlist.clone();
        allowed_headers.push(RETRY_AFTER.as_str().to_owned());
        for allowed in allowed_headers {
            if let Ok(name) = HeaderName::from_bytes(allowed.as_bytes()) {
                if let Some(value) = response.headers().get(&name) {
                    if let Ok(value) = value.to_str() {
                        headers.insert(name.as_str().to_owned(), value.to_owned());
                    }
                }
            }
        }

        let mut body = Vec::new();
        loop {
            let chunk = tokio::select! {
                _ = cancellation_token.cancelled() => {
                    return Err(TransportError::new(
                        TransportErrorKind::Cancelled,
                        OperationOutcome::Unknown,
                    ));
                }
                result = response.chunk() => result.map_err(|_| TransportError::new(
                    TransportErrorKind::BodyRead,
                    OperationOutcome::Failed,
                ))?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            let next_length = body.len().saturating_add(chunk.len());
            if next_length as u64 > request.max_response_bytes {
                return Err(TransportError::new(
                    TransportErrorKind::ResponseTooLarge,
                    OperationOutcome::Failed,
                ));
            }
            body.extend_from_slice(&chunk);
        }

        Ok(RawHttpResponse::new(status, headers, body))
    }
}

/// Prevents provider codecs from bypassing the credential injection boundary.
fn is_sensitive_managed_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "cookie" | "x-api-key" | "x-api-access-key"
    )
}

/// Maps a non-success HTTP response into the stable sanitized error contract.
pub(crate) fn classify_http_status(response: &RawHttpResponse) -> Option<ProviderError> {
    if (200..300).contains(&response.status) {
        return None;
    }

    let retry_after_ms = response
        .header(RETRY_AFTER.as_str())
        .and_then(parse_retry_after_ms);
    let (code, category, retryable, safe_message, outcome) = match response.status {
        401 => (
            "http_401",
            ProviderErrorCategory::Authentication,
            false,
            "Provider 凭据无效或缺失",
            OperationOutcome::Rejected,
        ),
        403 => (
            "http_403",
            ProviderErrorCategory::Permission,
            false,
            "Provider 拒绝当前凭据访问",
            OperationOutcome::Rejected,
        ),
        408 => (
            "http_408",
            ProviderErrorCategory::Timeout,
            true,
            "Provider 请求超时",
            OperationOutcome::Rejected,
        ),
        413 => (
            "http_413",
            ProviderErrorCategory::Input,
            false,
            "音频文件超过 Provider 限制",
            OperationOutcome::Rejected,
        ),
        429 => (
            "http_429",
            ProviderErrorCategory::RateLimit,
            true,
            "Provider 请求过于频繁",
            OperationOutcome::Rejected,
        ),
        500..=599 => (
            "http_5xx",
            ProviderErrorCategory::Provider,
            true,
            "Provider 暂时不可用",
            OperationOutcome::Failed,
        ),
        _ => (
            "provider_request_rejected",
            ProviderErrorCategory::Protocol,
            false,
            "Provider 拒绝请求或返回非预期状态",
            OperationOutcome::Rejected,
        ),
    };
    Some(ProviderError::new(
        code,
        category,
        retryable,
        false,
        safe_message,
        Some(response.status),
        retry_after_ms,
        outcome,
    ))
}

/// Parses Retry-After delta-seconds or an HTTP-date into a bounded millisecond hint.
fn parse_retry_after_ms(value: &str) -> Option<u64> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(seconds.saturating_mul(1_000));
    }

    let retry_at = DateTime::parse_from_rfc2822(value.trim())
        .ok()?
        .with_timezone(&Utc);
    let milliseconds = retry_at
        .signed_duration_since(Utc::now())
        .num_milliseconds();
    (milliseconds > 0).then_some(milliseconds as u64)
}

/// Validates a finite timeout value.
fn validate_timeout(timeout_ms: u64) -> Result<(), ProviderError> {
    if timeout_ms == 0 || timeout_ms > MAX_TIMEOUT_MS {
        return Err(ProviderError::configuration(
            "invalid_timeout",
            "Provider 超时时间无效",
        ));
    }
    Ok(())
}

/// Validates the configured authentication strategy without reading a secret.
fn validate_auth_strategy(auth: &ProviderCredentialPlacement) -> Result<(), ProviderError> {
    if let ProviderCredentialPlacement::Header {
        header_name,
        prefix,
    } = auth
    {
        validate_header_name(header_name, "invalid_auth_header")?;
        if prefix
            .as_ref()
            .is_some_and(|value| value.contains(['\r', '\n']))
        {
            return Err(ProviderError::configuration(
                "invalid_auth_prefix",
                "认证前缀包含非法字符",
            ));
        }
    }
    Ok(())
}

/// Validates an HTTP header name and blocks transport-controlled headers.
fn validate_header_name(name: &str, code: &str) -> Result<(), ProviderError> {
    HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| ProviderError::configuration(code, "Provider Header 名称无效"))?;
    if matches!(
        name.to_ascii_lowercase().as_str(),
        "host" | "content-length" | "transfer-encoding" | "connection" | "cookie"
    ) {
        return Err(ProviderError::configuration(
            code,
            "Provider Header 名称不允许由适配器设置",
        ));
    }
    Ok(())
}

/// Returns a non-empty caller-owned secret for immediate header injection.
fn required_secret(credential: Option<&ProviderCredential>) -> Result<&str, TransportError> {
    let credential = credential.ok_or_else(|| {
        TransportError::new(TransportErrorKind::RequestBuild, OperationOutcome::NotSent)
    })?;
    if credential.is_empty() {
        return Err(TransportError::new(
            TransportErrorKind::RequestBuild,
            OperationOutcome::NotSent,
        ));
    }
    credential.expose_secret().map_err(|_| {
        TransportError::new(TransportErrorKind::RequestBuild, OperationOutcome::NotSent)
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{classify_http_status, ProviderHttpConfig, RawHttpResponse};
    use crate::providers::{AuthStrategy, ProviderCredential, ReplaySafety, RetryPolicy};

    /// Creates a valid loopback-only test profile with no real endpoint or credential.
    fn test_config() -> ProviderHttpConfig {
        ProviderHttpConfig {
            provider_id: "test-provider".to_owned(),
            adapter_id: "explicit-test-codec".to_owned(),
            adapter_version: "1".to_owned(),
            endpoint: "http://127.0.0.1/mock".to_owned(),
            model: "test-model".to_owned(),
            auth: AuthStrategy::Bearer,
            connect_timeout_ms: 1_000,
            request_timeout_ms: 1_000,
            overall_timeout_ms: 5_000,
            max_response_bytes: 64 * 1024,
            max_concurrent: 1,
            min_request_interval_ms: 0,
            retry: RetryPolicy {
                max_retries: 2,
                base_delay_ms: 1,
                max_delay_ms: 10,
                max_retry_after_ms: 100,
            },
            replay_safety: ReplaySafety::NeverAutomaticallyReplay,
            idempotency_header: None,
            allow_insecure_loopback: true,
        }
    }

    /// Verifies that production profiles reject non-TLS remote endpoints.
    #[test]
    fn rejects_insecure_remote_endpoint() {
        let mut config = test_config();
        config.endpoint = "http://example.invalid/api".to_owned();
        assert_eq!(
            config.validate().expect_err("http endpoint must fail").code,
            "insecure_provider_endpoint"
        );
    }

    /// Verifies that 401 is terminal while 429 carries a parsed retry hint.
    #[test]
    fn classifies_auth_and_rate_limit_statuses() {
        let unauthorized = RawHttpResponse::new(401, BTreeMap::new(), Vec::new());
        let error = classify_http_status(&unauthorized).expect("401 must be an error");
        assert_eq!(error.code, "http_401");
        assert!(!error.retryable);

        let limited = RawHttpResponse::new(
            429,
            BTreeMap::from([("retry-after".to_owned(), "2".to_owned())]),
            Vec::new(),
        );
        let error = classify_http_status(&limited).expect("429 must be an error");
        assert_eq!(error.retry_after_ms, Some(2_000));
    }

    /// Verifies that credential values cannot appear in Debug output.
    #[test]
    fn credential_and_config_debug_are_redacted() {
        let dummy_value = "x".repeat(32);
        let credential = ProviderCredential::new(dummy_value.clone());
        assert!(!format!("{credential:?}").contains(&dummy_value));

        let mut config = test_config();
        config.endpoint = "https://internal.invalid/sensitive-path".to_owned();
        assert!(!format!("{config:?}").contains("sensitive-path"));
    }
}
