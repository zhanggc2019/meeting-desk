use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 表示任务当前所处的稳定状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Preparing,
    Uploading,
    Transcribing,
    ValidatingTranscript,
    Summarizing,
    ValidatingMinutes,
    Saving,
    RetryWait,
    CancelRequested,
    Interrupted,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    /// 返回任务是否已经进入不可继续处理的终态。
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// 表示前端可以对任务执行的受控动作。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskAction {
    Cancel,
    Retry,
    Delete,
    OpenMeeting,
    ReselectFile,
}

/// 表示可以安全发送到前端的任务错误。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SafeTaskError {
    pub code: String,
    pub retryable: bool,
    pub safe_message: String,
    pub http_status: Option<u16>,
    pub retry_after_ms: Option<u64>,
}

/// 表示任务列表和状态事件使用的公开记录。
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub artifact_id: String,
    pub batch_id: Option<String>,
    pub meeting_id: Option<String>,
    pub display_name: String,
    pub template_id: String,
    pub status: TaskStatus,
    pub progress: Option<f64>,
    pub attempt: u32,
    pub max_attempts: u32,
    pub error: Option<SafeTaskError>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub processing_started_at: Option<String>,
    #[serde(default)]
    pub processing_duration_ms: Option<u64>,
    pub available_actions: Vec<TaskAction>,
}

impl TaskRecord {
    /// 返回任务是否仍依赖受管音频，包括可重试失败和待清理任务。
    pub fn retains_audio_artifact(&self) -> bool {
        !self.status.is_terminal() || self.available_actions.contains(&TaskAction::Retry)
    }
}

/// 表示历史列表所需的最小会议信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingListItem {
    pub id: String,
    pub title: String,
    pub source_name: String,
    pub template_id: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 表示会议详情，包括敏感逐字稿和已验证纪要。
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetail {
    pub id: String,
    pub source_name: String,
    pub template_id: String,
    pub transcript: String,
    pub transcript_segments: Value,
    pub minutes: Value,
    pub created_at: String,
    pub updated_at: String,
}

/// 表示待写入 SQLite 的完整会议结果。
#[derive(Clone)]
pub struct PersistedMeetingInput {
    pub id: String,
    pub source_name: String,
    pub title: String,
    pub template_id: String,
    pub transcript: String,
    pub transcript_segments: Value,
    pub minutes: Value,
    pub schema_version: String,
}

/// 表示不泄露秘密的 Provider 配置就绪状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum ProviderReadiness {
    /// 配置不完整，不能调用真实 Provider。
    #[default]
    Incomplete,
    /// 兼容旧版持久化数据；新版正式界面不再提供 Mock 模式。
    MockExperience,
    /// 真实 Provider 的公开字段和秘密均已配置。
    Ready,
}

/// 表示单个不包含秘密值的 Provider 配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublicProviderConfig {
    #[serde(default)]
    pub preset_id: String,
    pub kind: String,
    pub endpoint: String,
    pub model: String,
    #[serde(default)]
    pub local_model_path: String,
    #[serde(default)]
    pub credential_preset_id: Option<String>,
    pub secret_configured: bool,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub max_retries: u32,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub readiness: ProviderReadiness,
    #[serde(default)]
    pub validation_message: String,
}

impl Default for PublicProviderConfig {
    /// 创建安全的未配置默认值，不包含 Provider 地址或秘密。
    fn default() -> Self {
        Self {
            preset_id: String::new(),
            kind: String::new(),
            endpoint: String::new(),
            model: String::new(),
            local_model_path: String::new(),
            credential_preset_id: None,
            secret_configured: false,
            connect_timeout_ms: 10_000,
            request_timeout_ms: 60_000,
            max_retries: 2,
            ready: false,
            readiness: ProviderReadiness::Incomplete,
            validation_message: "尚未配置服务".to_string(),
        }
    }
}

/// 表示前端设置页可读取的两类 Provider 配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PublicSettings {
    pub transcription: PublicProviderConfig,
    pub minutes: PublicProviderConfig,
}
