use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 离线音频导入对外暴露的稳定错误码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestErrorCode {
    InvalidPolicy,
    InvalidSelection,
    BatchLimitExceeded,
    SourceNotFound,
    SourceNotFile,
    SourceUnreadable,
    SourceChangedDuringImport,
    EmptyAudio,
    FileTooLarge,
    UnsupportedExtension,
    ExtensionContentMismatch,
    CorruptAudio,
    UnsupportedAudio,
    UnsupportedAudioTracks,
    AudioStorageFailed,
    ArtifactNotFound,
}

/// 离线导入模块的内部错误；消息不包含路径或底层解析内容。
#[derive(Debug, Error)]
pub enum IngestError {
    #[error("invalid ingest policy")]
    InvalidPolicy,
    #[error("invalid file selection")]
    InvalidSelection,
    #[error("batch limit exceeded")]
    BatchLimitExceeded,
    #[error("source file not found")]
    SourceNotFound,
    #[error("source is not a regular file")]
    SourceNotFile,
    #[error("source file cannot be read")]
    SourceUnreadable,
    #[error("source changed during import")]
    SourceChangedDuringImport,
    #[error("audio file is empty")]
    EmptyAudio,
    #[error("audio file is too large")]
    FileTooLarge,
    #[error("audio extension is unsupported")]
    UnsupportedExtension,
    #[error("audio extension does not match its content")]
    ExtensionContentMismatch,
    #[error("audio file is corrupt")]
    CorruptAudio,
    #[error("audio format is unsupported")]
    UnsupportedAudio,
    #[error("multiple audio tracks are unsupported")]
    UnsupportedAudioTracks,
    #[error("managed audio storage failed")]
    AudioStorageFailed,
    #[error("managed audio artifact was not found")]
    ArtifactNotFound,
}

impl IngestError {
    /// 返回可序列化且不包含敏感上下文的稳定错误码。
    pub fn code(&self) -> IngestErrorCode {
        match self {
            Self::InvalidPolicy => IngestErrorCode::InvalidPolicy,
            Self::InvalidSelection => IngestErrorCode::InvalidSelection,
            Self::BatchLimitExceeded => IngestErrorCode::BatchLimitExceeded,
            Self::SourceNotFound => IngestErrorCode::SourceNotFound,
            Self::SourceNotFile => IngestErrorCode::SourceNotFile,
            Self::SourceUnreadable => IngestErrorCode::SourceUnreadable,
            Self::SourceChangedDuringImport => IngestErrorCode::SourceChangedDuringImport,
            Self::EmptyAudio => IngestErrorCode::EmptyAudio,
            Self::FileTooLarge => IngestErrorCode::FileTooLarge,
            Self::UnsupportedExtension => IngestErrorCode::UnsupportedExtension,
            Self::ExtensionContentMismatch => IngestErrorCode::ExtensionContentMismatch,
            Self::CorruptAudio => IngestErrorCode::CorruptAudio,
            Self::UnsupportedAudio => IngestErrorCode::UnsupportedAudio,
            Self::UnsupportedAudioTracks => IngestErrorCode::UnsupportedAudioTracks,
            Self::AudioStorageFailed => IngestErrorCode::AudioStorageFailed,
            Self::ArtifactNotFound => IngestErrorCode::ArtifactNotFound,
        }
    }

    /// 返回供 UI 本地化的安全消息键，不回显文件名、路径或解析器详情。
    pub fn safe_message_key(&self) -> &'static str {
        match self {
            Self::InvalidPolicy => "ingest.error.invalid_policy",
            Self::InvalidSelection => "ingest.error.invalid_selection",
            Self::BatchLimitExceeded => "ingest.error.batch_limit_exceeded",
            Self::SourceNotFound => "ingest.error.source_not_found",
            Self::SourceNotFile => "ingest.error.source_not_file",
            Self::SourceUnreadable => "ingest.error.source_unreadable",
            Self::SourceChangedDuringImport => "ingest.error.source_changed",
            Self::EmptyAudio => "ingest.error.empty_audio",
            Self::FileTooLarge => "ingest.error.file_too_large",
            Self::UnsupportedExtension => "ingest.error.unsupported_extension",
            Self::ExtensionContentMismatch => "ingest.error.extension_content_mismatch",
            Self::CorruptAudio => "ingest.error.corrupt_audio",
            Self::UnsupportedAudio => "ingest.error.unsupported_audio",
            Self::UnsupportedAudioTracks => "ingest.error.unsupported_audio_tracks",
            Self::AudioStorageFailed => "ingest.error.audio_storage_failed",
            Self::ArtifactNotFound => "ingest.error.artifact_not_found",
        }
    }
}
