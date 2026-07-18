/// 表示纪要构建、解析和校验过程中可安全展示的错误。
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum MinutesError {
    #[error("转写文本为空，无法生成会议纪要")]
    EmptyTranscript,
    #[error("转写结构无效")]
    InvalidTranscript { code: &'static str },
    #[error("会议纪要模板不存在")]
    UnknownTemplate,
    #[error("会议纪要模板版本不受支持")]
    TemplateVersionMismatch,
    #[error("低置信度阈值无效")]
    InvalidConfidenceThreshold,
    #[error("模型输出超过安全大小限制")]
    ModelOutputTooLarge,
    #[error("模型没有返回可接受的单一 JSON 对象")]
    InvalidModelOutput,
    #[error("会议纪要 Schema 版本不匹配")]
    SchemaVersionMismatch,
    #[error("会议纪要结构校验失败")]
    SchemaViolation {
        code: &'static str,
        path: &'static str,
    },
    #[error("会议纪要来源或语义校验失败")]
    SemanticViolation {
        code: &'static str,
        path: &'static str,
    },
    #[error("无法构造安全的会议纪要 Prompt")]
    PromptSerialization,
    #[error("内置会议纪要 Schema 无效")]
    InvalidEmbeddedSchema,
}

impl MinutesError {
    /// 返回不含转写或模型正文的稳定机器错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyTranscript => "empty_transcript",
            Self::InvalidTranscript { code } => code,
            Self::UnknownTemplate => "unknown_minutes_template",
            Self::TemplateVersionMismatch => "template_version_mismatch",
            Self::InvalidConfidenceThreshold => "invalid_confidence_threshold",
            Self::ModelOutputTooLarge => "model_output_too_large",
            Self::InvalidModelOutput => "invalid_model_output",
            Self::SchemaVersionMismatch => "schema_version_mismatch",
            Self::SchemaViolation { code, .. } | Self::SemanticViolation { code, .. } => code,
            Self::PromptSerialization => "prompt_serialization_failed",
            Self::InvalidEmbeddedSchema => "invalid_embedded_schema",
        }
    }

    /// 返回安全的 JSON Pointer；与具体字段无关时返回根路径。
    pub fn path(&self) -> &'static str {
        match self {
            Self::SchemaViolation { path, .. } | Self::SemanticViolation { path, .. } => path,
            _ => "/",
        }
    }
}
