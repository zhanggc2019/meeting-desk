use serde_json::Value;

use crate::providers::{MinutesCandidate, Transcript};

use super::{
    validate_model_minutes, MeetingContext, MeetingMinutes, MinutesError, ValidationOptions,
    MEETING_MINUTES_SCHEMA_VERSION,
};

/// 单次模型输出允许的最大 UTF-8 字节数；外层 transport 仍必须设置响应上限。
pub const MAX_MODEL_OUTPUT_BYTES: usize = 1_048_576;

/// 从模型文本中严格提取一个根 JSON object，不从说明文字中捞取局部 JSON。
pub fn extract_model_json(raw_output: &str) -> Result<Value, MinutesError> {
    if raw_output.len() > MAX_MODEL_OUTPUT_BYTES {
        return Err(MinutesError::ModelOutputTooLarge);
    }
    let trimmed = raw_output.trim();
    if trimmed.is_empty() {
        return Err(MinutesError::InvalidModelOutput);
    }
    let json_text = if trimmed.starts_with("```") {
        extract_single_fenced_json(trimmed)?
    } else {
        trimmed
    };
    if !json_text.starts_with('{') || !json_text.ends_with('}') {
        return Err(MinutesError::InvalidModelOutput);
    }
    let value: Value =
        serde_json::from_str(json_text).map_err(|_| MinutesError::InvalidModelOutput)?;
    if !value.is_object() {
        return Err(MinutesError::InvalidModelOutput);
    }
    Ok(value)
}

/// 解析模型文本并完成结构、上下文、证据和低置信度校验。
pub fn parse_and_validate_model_output(
    raw_output: &str,
    expected_schema_version: &str,
    transcript: &Transcript,
    context: &MeetingContext,
    options: ValidationOptions,
) -> Result<MeetingMinutes, MinutesError> {
    let value = extract_model_json(raw_output)?;
    parse_and_validate_value(value, expected_schema_version, transcript, context, options)
}

/// 校验 Agent 4 Provider 返回的候选，确保候选元数据与 payload 版本一致。
pub fn validate_provider_candidate(
    candidate: MinutesCandidate,
    expected_schema_version: &str,
    transcript: &Transcript,
    context: &MeetingContext,
    options: ValidationOptions,
) -> Result<MeetingMinutes, MinutesError> {
    if candidate.schema_version != expected_schema_version
        || expected_schema_version != MEETING_MINUTES_SCHEMA_VERSION
    {
        return Err(MinutesError::SchemaVersionMismatch);
    }
    parse_and_validate_value(
        candidate.value,
        expected_schema_version,
        transcript,
        context,
        options,
    )
}

/// 将单一 JSON value 严格反序列化为 MeetingMinutes 并执行语义校验。
fn parse_and_validate_value(
    value: Value,
    expected_schema_version: &str,
    transcript: &Transcript,
    context: &MeetingContext,
    options: ValidationOptions,
) -> Result<MeetingMinutes, MinutesError> {
    if value.get("contentType").is_none() {
        return Err(MinutesError::SchemaViolation {
            code: "missing_content_type",
            path: "/contentType",
        });
    }
    let minutes: MeetingMinutes =
        serde_json::from_value(value).map_err(|_| MinutesError::SchemaViolation {
            code: "invalid_minutes_shape",
            path: "/",
        })?;
    validate_model_minutes(
        minutes,
        expected_schema_version,
        transcript,
        context,
        options,
    )
}

/// 接受最多一个且外围只有空白的 JSON 代码围栏。
fn extract_single_fenced_json(value: &str) -> Result<&str, MinutesError> {
    let header_end = value.find('\n').ok_or(MinutesError::InvalidModelOutput)?;
    let language = value[3..header_end].trim();
    if !language.is_empty() && !language.eq_ignore_ascii_case("json") {
        return Err(MinutesError::InvalidModelOutput);
    }
    if !value.ends_with("```") {
        return Err(MinutesError::InvalidModelOutput);
    }
    let inner = value[header_end + 1..value.len() - 3].trim();
    if inner.contains("```") {
        return Err(MinutesError::InvalidModelOutput);
    }
    Ok(inner)
}
