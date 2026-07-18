pub mod ingest;
pub mod meetings;
pub mod settings;
pub mod tasks;

use serde::Serialize;

use crate::storage::StorageError;

/// 可安全发送给前端的命令错误，不包含异常正文或会议内容。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: &'static str,
    pub safe_message: String,
    pub retryable: bool,
}

impl CommandError {
    /// 创建不包含底层敏感细节的命令错误。
    pub fn new(code: &'static str, safe_message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            safe_message: safe_message.into(),
            retryable,
        }
    }
}

impl From<StorageError> for CommandError {
    /// 将数据库错误统一映射为安全错误。
    fn from(error: StorageError) -> Self {
        Self::new("local_storage_error", error.to_string(), true)
    }
}
