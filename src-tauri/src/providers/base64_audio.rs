use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use tokio::io::AsyncReadExt;

use super::{
    OperationOutcome, ProviderCallContext, ProviderError, ProviderErrorCategory,
    TranscriptionRequest,
};

/// Reads one managed audio artifact cancellably and returns raw Base64 without exposing its path.
pub(crate) async fn encode_managed_audio(
    request: &TranscriptionRequest,
    context: &ProviderCallContext,
) -> Result<String, ProviderError> {
    if context.cancellation_token.is_cancelled() {
        return Err(ProviderError::cancelled());
    }
    let file = request
        .artifact
        .reader
        .open_readonly()
        .map_err(|_| local_file_error())?;
    let expected = request.artifact.reference.staging_metadata.byte_length;
    let mut file = tokio::fs::File::from_std(file);
    let mut bytes = Vec::with_capacity(expected as usize);
    let read_result = tokio::select! {
        _ = context.cancellation_token.cancelled() => return Err(ProviderError::cancelled()),
        result = file.read_to_end(&mut bytes) => result,
    };
    read_result.map_err(|_| local_file_error())?;
    if bytes.len() as u64 != expected {
        return Err(ProviderError::input(
            "staged_file_changed",
            "受管音频文件大小发生变化",
        ));
    }
    Ok(STANDARD.encode(bytes))
}

/// Creates a sanitized local artifact read error before any provider request is sent.
fn local_file_error() -> ProviderError {
    ProviderError::new(
        "staged_file_unavailable",
        ProviderErrorCategory::LocalResource,
        false,
        false,
        "受管音频文件不可读取",
        None,
        None,
        OperationOutcome::NotSent,
    )
}
