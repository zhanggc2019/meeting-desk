use serde_json::Value;

use super::MinutesError;

/// MeetingMinutes v1.1.0 的唯一 JSON Schema 源文本。
pub const MEETING_MINUTES_SCHEMA_JSON: &str =
    include_str!("../../../shared/schemas/meeting-minutes/1.1.0.schema.json");

/// 解析内置 JSON Schema，供 Provider structured output 使用。
pub fn meeting_minutes_schema() -> Result<Value, MinutesError> {
    serde_json::from_str(MEETING_MINUTES_SCHEMA_JSON)
        .map_err(|_| MinutesError::InvalidEmbeddedSchema)
}
