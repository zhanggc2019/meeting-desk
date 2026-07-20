use std::fmt;
use std::fs::File;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{CancellationToken, ProviderCredential, ProviderError};

pub use crate::ingest::{AudioArtifactRef, StagingMetadata};

/// Evidence level attached to a provider capability declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEvidence {
    /// Deterministic behavior implemented by the local mock.
    Mock,
    /// A real provider capability not yet verified by this project.
    Unverified,
    /// A capability verified against a documented or observed real contract.
    Verified,
}

/// Describes when retrying a logical provider operation is safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplaySafety {
    /// Replay is safe because the adapter contract has verified that property.
    VerifiedAlwaysSafe,
    /// Replay is safe only when a configured, verified idempotency key is sent.
    SafeWithVerifiedIdempotencyKey,
    /// Replay is safe only when the request body was not sent.
    BeforeRequestBodySentOnly,
    /// Automatic replay is not allowed.
    NeverAutomaticallyReplay,
}

/// Describes the best-known outcome of a failed remote attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationOutcome {
    /// The operation completed successfully.
    Succeeded,
    /// The request was not sent to the provider.
    NotSent,
    /// The provider explicitly rejected the request.
    Rejected,
    /// The provider accepted enough of the request to return a known failure.
    Failed,
    /// The client cannot determine whether the provider accepted the request.
    Unknown,
}

/// Opens a fresh read-only handle for one ingest-managed staged artifact.
pub trait AudioArtifactReader: Send + Sync {
    /// Opens the artifact from the trusted ingest registry without exposing its path.
    fn open_readonly(&self) -> std::io::Result<File>;
}

impl<F> AudioArtifactReader for F
where
    F: Fn() -> std::io::Result<File> + Send + Sync,
{
    /// Delegates opening to a trusted closure supplied by the integration layer.
    fn open_readonly(&self) -> std::io::Result<File> {
        self()
    }
}

/// Trusted provider input that adds an opaque read-only opener to the safe artifact reference.
#[derive(Clone)]
pub struct ManagedAudioArtifact {
    pub reference: AudioArtifactRef,
    pub reader: Arc<dyn AudioArtifactReader>,
}

impl ManagedAudioArtifact {
    /// Creates a trusted artifact from an ingest-generated reference and opaque reader.
    pub fn new(reference: AudioArtifactRef, reader: Arc<dyn AudioArtifactReader>) -> Self {
        Self { reference, reader }
    }
}

impl fmt::Debug for ManagedAudioArtifact {
    /// Formats only safe metadata and intentionally omits the staged path.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedAudioArtifact")
            .field("artifact_id", &self.reference.id)
            .field("import_batch_id", &self.reference.import_batch_id)
            .field("mime_type", &self.reference.staging_metadata.mime_type)
            .field("byte_length", &self.reference.staging_metadata.byte_length)
            .field("duration_ms", &self.reference.staging_metadata.duration_ms)
            .field("reader", &"[REDACTED]")
            .finish()
    }
}

/// Optional transcription features requested by the task orchestrator.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionOptions {
    pub language_hint: Option<String>,
    pub enable_timestamps: bool,
    pub enable_speaker_labels: bool,
    pub enable_confidence: bool,
}

/// Provider-neutral transcription input.
#[derive(Clone)]
pub struct TranscriptionRequest {
    pub artifact: ManagedAudioArtifact,
    pub options: TranscriptionOptions,
}

/// Provider-supported format identifiers for one HTTPS recording-file URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAudioFormat {
    Wav,
    Mp3,
    Ogg,
    Opus,
    M4a,
}

impl RemoteAudioFormat {
    /// Returns the stable provider-facing format value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
            Self::Ogg => "ogg",
            Self::Opus => "opus",
            Self::M4a => "m4a",
        }
    }
}

/// A validated remote recording reference whose potentially signed URL is always redacted.
#[derive(Clone)]
pub struct RemoteAudioFile {
    pub id: String,
    url: reqwest::Url,
    pub format: RemoteAudioFormat,
    pub byte_length: Option<u64>,
    pub duration_ms: Option<u64>,
}

impl RemoteAudioFile {
    /// Validates one HTTPS file URL without fetching it or exposing query parameters.
    pub fn new(
        id: impl Into<String>,
        url: &str,
        format: RemoteAudioFormat,
        byte_length: Option<u64>,
        duration_ms: Option<u64>,
    ) -> Result<Self, ProviderError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(ProviderError::input(
                "invalid_remote_audio_url",
                "远程录音标识不能为空",
            ));
        }
        let url = reqwest::Url::parse(url).map_err(|_| {
            ProviderError::input("invalid_remote_audio_url", "录音文件 URL 格式无效")
        })?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.host_str().is_none()
        {
            return Err(ProviderError::input(
                "invalid_remote_audio_url",
                "录音文件 URL 必须使用 HTTPS，且不得包含用户凭据或片段",
            ));
        }
        if url.as_str().len() > 8_192 {
            return Err(ProviderError::input(
                "invalid_remote_audio_url",
                "录音文件 URL 过长",
            ));
        }
        Ok(Self {
            id,
            url,
            format,
            byte_length,
            duration_ms,
        })
    }

    /// Exposes the validated URL only inside the trusted provider request builder.
    pub(crate) fn url(&self) -> &reqwest::Url {
        &self.url
    }
}

impl fmt::Debug for RemoteAudioFile {
    /// Formats only safe metadata and never the remote URL or its signed query.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteAudioFile")
            .field("id", &self.id)
            .field("url", &"[REDACTED]")
            .field("format", &self.format)
            .field("byte_length", &self.byte_length)
            .field("duration_ms", &self.duration_ms)
            .finish()
    }
}

/// Provider-neutral request for a provider-fetched HTTPS recording file.
#[derive(Clone)]
pub struct UrlTranscriptionRequest {
    pub audio: RemoteAudioFile,
    pub options: TranscriptionOptions,
}

impl fmt::Debug for UrlTranscriptionRequest {
    /// Formats safe metadata while preserving URL redaction.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UrlTranscriptionRequest")
            .field("audio", &self.audio)
            .field("options", &self.options)
            .finish()
    }
}

impl fmt::Debug for TranscriptionRequest {
    /// Formats safe request metadata without file paths or content.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TranscriptionRequest")
            .field("artifact", &self.artifact)
            .field("options", &self.options)
            .finish()
    }
}

/// A normalized provider-neutral transcript segment.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub id: String,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub speaker_label: Option<String>,
    pub text: String,
    pub confidence: Option<f64>,
}

impl fmt::Debug for TranscriptSegment {
    /// Formats segment metadata while redacting transcript text.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TranscriptSegment")
            .field("id", &self.id)
            .field("start_ms", &self.start_ms)
            .field("end_ms", &self.end_ms)
            .field("speaker_label", &self.speaker_label)
            .field("text", &"[REDACTED]")
            .field("confidence", &self.confidence)
            .finish()
    }
}

/// Safe metadata about a provider result; it never contains request or response bodies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMetadata {
    pub provider_id: String,
    pub adapter_id: String,
    pub adapter_version: String,
    pub model: String,
    pub remote_request_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

/// A normalized provider-neutral transcript.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub schema_version: String,
    pub text: String,
    pub language: Option<String>,
    pub duration_ms: Option<u64>,
    pub segments: Vec<TranscriptSegment>,
    pub provider_metadata: ProviderMetadata,
}

impl fmt::Debug for Transcript {
    /// Formats transcript metadata while redacting full text and segment bodies.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Transcript")
            .field("schema_version", &self.schema_version)
            .field("text", &"[REDACTED]")
            .field("language", &self.language)
            .field("duration_ms", &self.duration_ms)
            .field("segment_count", &self.segments.len())
            .field("provider_metadata", &self.provider_metadata)
            .finish()
    }
}

/// Trusted prompt and Schema payload assembled by the minutes module.
#[derive(Clone)]
pub struct MinutesGenerationRequest {
    pub prompt: String,
    pub output_schema: Value,
    pub schema_version: String,
}

impl fmt::Debug for MinutesGenerationRequest {
    /// Redacts the prompt and Schema values from debug formatting.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MinutesGenerationRequest")
            .field("prompt", &"[REDACTED]")
            .field("output_schema", &"[REDACTED]")
            .field("schema_version", &self.schema_version)
            .finish()
    }
}

/// Untrusted structured candidate returned for validation by the minutes module.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinutesCandidate {
    pub schema_version: String,
    pub value: Value,
    pub provider_metadata: ProviderMetadata,
}

impl fmt::Debug for MinutesCandidate {
    /// Formats candidate metadata while redacting the untrusted model value.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MinutesCandidate")
            .field("schema_version", &self.schema_version)
            .field("value", &"[REDACTED]")
            .field("provider_metadata", &self.provider_metadata)
            .finish()
    }
}

/// Verified or explicitly unknown transcription limits and features.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionCapabilities {
    pub evidence: CapabilityEvidence,
    pub accepted_media_types: Vec<String>,
    pub max_audio_bytes: Option<u64>,
    pub max_duration_ms: Option<u64>,
    pub supports_async_jobs: bool,
    pub supports_timestamps: bool,
    pub supports_speaker_labels: bool,
    pub supports_confidence: bool,
    pub supports_remote_cancel: bool,
    #[serde(default)]
    pub supports_remote_urls: bool,
    pub replay_safety: ReplaySafety,
}

/// Verified or explicitly unknown minutes-generation limits and features.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinutesCapabilities {
    pub evidence: CapabilityEvidence,
    pub supports_json_schema: bool,
    pub supported_schema_versions: Vec<String>,
    pub max_input_characters: Option<u64>,
    pub supports_async_jobs: bool,
    pub supports_remote_cancel: bool,
    pub replay_safety: ReplaySafety,
}

/// Per-operation state supplied by the trusted task orchestrator.
#[derive(Clone, Debug)]
pub struct ProviderCallContext {
    pub task_id: String,
    pub operation_id: String,
    pub cancellation_token: CancellationToken,
    pub deadline: Instant,
}

impl ProviderCallContext {
    /// Creates a provider context with an explicit overall timeout.
    pub fn with_timeout(
        task_id: impl Into<String>,
        operation_id: impl Into<String>,
        cancellation_token: CancellationToken,
        overall_timeout: Duration,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            operation_id: operation_id.into(),
            cancellation_token,
            deadline: Instant::now() + overall_timeout,
        }
    }

    /// Returns the remaining operation time, or zero after the deadline.
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

/// Provider-neutral transcription behavior.
#[async_trait]
pub trait TranscriptionProvider: Send + Sync {
    /// Returns only explicitly configured and evidenced capabilities.
    fn capabilities(&self) -> TranscriptionCapabilities;

    /// Transcribes one managed offline audio artifact.
    async fn transcribe(
        &self,
        context: &ProviderCallContext,
        request: TranscriptionRequest,
        credential: Option<&ProviderCredential>,
    ) -> Result<Transcript, ProviderError>;

    /// Transcribes one provider-fetched HTTPS recording URL when explicitly supported.
    async fn transcribe_url(
        &self,
        _context: &ProviderCallContext,
        _request: UrlTranscriptionRequest,
        _credential: Option<&ProviderCredential>,
    ) -> Result<Transcript, ProviderError> {
        Err(ProviderError::input(
            "remote_url_unsupported",
            "当前 Provider 不支持录音文件 URL",
        ))
    }
}

/// Provider-neutral meeting-minutes generation behavior.
#[async_trait]
pub trait MinutesProvider: Send + Sync {
    /// Returns only explicitly configured and evidenced capabilities.
    fn capabilities(&self) -> MinutesCapabilities;

    /// Generates an untrusted JSON candidate for the minutes module to validate.
    async fn generate_candidate(
        &self,
        context: &ProviderCallContext,
        request: MinutesGenerationRequest,
        credential: Option<&ProviderCredential>,
    ) -> Result<MinutesCandidate, ProviderError>;
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::{
        MinutesCandidate, ProviderMetadata, RemoteAudioFile, RemoteAudioFormat, Transcript,
        TranscriptSegment,
    };

    /// Creates safe provider metadata for Debug redaction tests.
    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            provider_id: "mock".to_owned(),
            adapter_id: "mock".to_owned(),
            adapter_version: "1".to_owned(),
            model: "mock".to_owned(),
            remote_request_id: None,
            started_at: Utc::now(),
            completed_at: Utc::now(),
        }
    }

    /// Verifies that transcript and candidate Debug output never contains body sentinels.
    #[test]
    fn sensitive_result_debug_output_is_redacted() {
        let transcript = Transcript {
            schema_version: "1".to_owned(),
            text: "sentinel-transcript-body".to_owned(),
            language: None,
            duration_ms: None,
            segments: vec![TranscriptSegment {
                id: "segment-1".to_owned(),
                start_ms: None,
                end_ms: None,
                speaker_label: None,
                text: "sentinel-segment-body".to_owned(),
                confidence: None,
            }],
            provider_metadata: metadata(),
        };
        let candidate = MinutesCandidate {
            schema_version: "1.0.0".to_owned(),
            value: json!({"summary": "sentinel-minutes-body"}),
            provider_metadata: metadata(),
        };
        let rendered = format!("{transcript:?} {candidate:?}");
        assert!(!rendered.contains("sentinel-transcript-body"));
        assert!(!rendered.contains("sentinel-segment-body"));
        assert!(!rendered.contains("sentinel-minutes-body"));
    }

    /// Verifies signed query values never appear in remote audio Debug output.
    #[test]
    fn remote_audio_url_is_validated_and_redacted() {
        let audio = RemoteAudioFile::new(
            "remote-1",
            "https://media.example.test/recording.mp3?signature=test-only-sentinel",
            RemoteAudioFormat::Mp3,
            None,
            None,
        )
        .expect("valid HTTPS recording URL");
        let rendered = format!("{audio:?}");
        assert!(!rendered.contains("test-only-sentinel"));
        assert!(!rendered.contains("media.example.test"));
        assert!(RemoteAudioFile::new(
            "remote-2",
            "http://media.example.test/recording.mp3",
            RemoteAudioFormat::Mp3,
            None,
            None,
        )
        .is_err());
    }
}
