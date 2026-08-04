//! Provider-agnostic cloud API contracts, mocks, and configurable HTTP adapters.

mod base64_audio;
mod cancellation;
mod contract;
mod error;
mod factory;
mod http;
mod mock;
mod openai_compatible;
mod retry;
mod secret;
mod volcengine;
mod xiaomi_mimo;

pub use cancellation::CancellationToken;
pub use contract::{
    AudioArtifactReader, AudioArtifactRef, CapabilityEvidence, ManagedAudioArtifact,
    MinutesCandidate, MinutesCapabilities, MinutesGenerationRequest, MinutesProvider,
    OperationOutcome, ProviderCallContext, ProviderMetadata, RemoteAudioFile, RemoteAudioFormat,
    ReplaySafety, StagingMetadata, Transcript, TranscriptSegment, TranscriptionCapabilities,
    TranscriptionOptions, TranscriptionProvider, TranscriptionRequest, UrlTranscriptionRequest,
};
pub use error::{ProviderError, ProviderErrorCategory};
pub use factory::{build_minutes_provider, build_transcription_provider};
pub use http::{
    AuthStrategy, HttpExecutor, HttpMethod, MultipartBody, MultipartFile,
    ProviderCredentialPlacement, ProviderHttpBody, ProviderHttpConfig, ProviderHttpRequest,
    RawHttpResponse, ReqwestHttpExecutor, TransportError, TransportErrorKind,
};
pub use mock::{MockCallRecord, MockConfig, MockProvider, MockScenario};
pub use openai_compatible::{
    openai_chat_completions_minutes_mapping, JsonContentMode, JsonPath, JsonPathSegment,
    MinutesHttpMapping, OpenAiCompatibleMinutesProvider, OpenAiCompatibleTranscriptionProvider,
    SegmentResponseMapping, TimestampUnit, ToggleFieldMapping, TranscriptionHttpMapping,
    TranscriptionResponseMapping,
};
pub use retry::RetryPolicy;
pub use secret::ProviderCredential;
pub use volcengine::VolcengineFlashTranscriptionProvider;
pub use xiaomi_mimo::XiaomiMimoTranscriptionProvider;
