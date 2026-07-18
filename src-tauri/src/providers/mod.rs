//! Provider-agnostic cloud API contracts, mocks, and configurable HTTP adapters.

mod cancellation;
mod contract;
mod error;
mod http;
mod mock;
mod openai_compatible;
mod retry;
mod secret;

pub use cancellation::CancellationToken;
pub use contract::{
    AudioArtifactReader, AudioArtifactRef, CapabilityEvidence, ManagedAudioArtifact,
    MinutesCandidate, MinutesCapabilities, MinutesGenerationRequest, MinutesProvider,
    OperationOutcome, ProviderCallContext, ProviderMetadata, ReplaySafety, StagingMetadata,
    Transcript, TranscriptSegment, TranscriptionCapabilities, TranscriptionOptions,
    TranscriptionProvider, TranscriptionRequest,
};
pub use error::{ProviderError, ProviderErrorCategory};
pub use http::{
    AuthStrategy, HttpExecutor, HttpMethod, MultipartBody, MultipartFile,
    ProviderCredentialPlacement, ProviderHttpBody, ProviderHttpConfig, ProviderHttpRequest,
    RawHttpResponse, ReqwestHttpExecutor, TransportError, TransportErrorKind,
};
pub use mock::{MockCallRecord, MockConfig, MockProvider, MockScenario};
pub use openai_compatible::{
    JsonContentMode, JsonPath, JsonPathSegment, MinutesHttpMapping,
    OpenAiCompatibleMinutesProvider, OpenAiCompatibleTranscriptionProvider, SegmentResponseMapping,
    TimestampUnit, ToggleFieldMapping, TranscriptionHttpMapping, TranscriptionResponseMapping,
};
pub use retry::RetryPolicy;
pub use secret::ProviderCredential;
