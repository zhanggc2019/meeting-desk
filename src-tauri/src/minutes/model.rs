use serde::{Deserialize, Serialize};

/// MeetingMinutes 当前唯一受支持的 Schema 版本。
pub const MEETING_MINUTES_SCHEMA_VERSION: &str = "1.1.0";

/// 表示已经通过结构与语义校验的会议纪要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingMinutes {
    pub schema_version: String,
    #[serde(default)]
    pub content_type: ContentType,
    pub title: Option<String>,
    pub title_source: TitleSource,
    pub meeting_time: MeetingTime,
    pub participants: Vec<String>,
    pub summary: Option<String>,
    pub topics: Vec<Topic>,
    pub conclusions: Vec<SupportedStatement>,
    pub decisions: Vec<SupportedStatement>,
    pub action_items: Vec<ActionItem>,
    pub risks_and_issues: Vec<RiskOrIssue>,
}

/// 标识录音正文的主要内容形态，驱动模板约束、展示文案和导出结构。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Meeting,
    Speech,
    Lecture,
    Course,
    Interview,
    Report,
    ArticleMaterial,
    #[default]
    Other,
}

/// 标识会议标题来自可信上下文、模型概括或保持未知。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleSource {
    Context,
    Generated,
    Unknown,
}

/// 表示只允许从可信上下文复制的会议起止时间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingTime {
    pub start_at: Option<String>,
    pub end_at: Option<String>,
}

/// 表示一个有可选概括和证据定位的主要议题。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Topic {
    pub title: String,
    pub summary: Option<String>,
    pub evidence_segment_ids: Vec<String>,
}

/// 表示一个有明确转写证据的结论或决策。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupportedStatement {
    pub content: String,
    pub evidence_segment_ids: Vec<String>,
}

/// 表示一个明确的待办事项及其未经推断的负责人和期限。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionItem {
    pub description: String,
    pub owner: Option<String>,
    pub due_date_text: Option<String>,
    pub due_date: Option<String>,
    pub evidence_segment_ids: Vec<String>,
}

/// 表示风险或尚未解决的问题。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RiskOrIssue {
    pub kind: RiskKind,
    pub description: String,
    pub impact: Option<String>,
    pub mitigation: Option<String>,
    pub evidence_segment_ids: Vec<String>,
}

/// 区分风险和当前问题。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskKind {
    Risk,
    Issue,
}

/// 表示由用户或可信业务层显式提供的会议上下文。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingContext {
    pub known_title: Option<String>,
    pub known_start_at: Option<String>,
    pub known_end_at: Option<String>,
    pub known_participants: Vec<String>,
}

/// 控制低置信度语义校验，不为缺失的 ASR confidence 编造判断。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ValidationOptions {
    pub low_confidence_threshold: Option<f64>,
}
