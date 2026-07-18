//! 会议纪要的唯一 Schema、Prompt、解析、语义校验和 Markdown 渲染边界。

mod error;
mod markdown;
mod model;
mod parse;
mod prompt;
mod schema;
mod templates;
mod validate;

pub use error::MinutesError;
pub use markdown::render_minutes_markdown;
pub use model::{
    ActionItem, MeetingContext, MeetingMinutes, MeetingTime, RiskKind, RiskOrIssue,
    SupportedStatement, TitleSource, Topic, ValidationOptions, MEETING_MINUTES_SCHEMA_VERSION,
};
pub use parse::{
    extract_model_json, parse_and_validate_model_output, validate_provider_candidate,
    MAX_MODEL_OUTPUT_BYTES,
};
pub use prompt::{build_prompt, BuiltMinutesPrompt, PromptBuildRequest};
pub use schema::{meeting_minutes_schema, MEETING_MINUTES_SCHEMA_JSON};
pub use templates::{
    get_template, list_templates, MinutesTemplate, ACADEMIC_LECTURE_TEMPLATE_ID,
    ADAPTIVE_TEMPLATE_ID, ARTICLE_OUTLINE_TEMPLATE_ID, BUILTIN_TEMPLATE_VERSION,
    BUSINESS_PLAN_TEMPLATE_ID, COURSE_SUMMARY_TEMPLATE_ID, CUSTOMER_COMMUNICATION_TEMPLATE_ID,
    IN_DEPTH_INTERVIEW_TEMPLATE_ID, PROFILE_INTERVIEW_TEMPLATE_ID, PROJECT_WEEKLY_TEMPLATE_ID,
    RESEARCH_PROJECT_TEMPLATE_ID, STANDARD_MEETING_TEMPLATE_ID,
};
pub use validate::{
    normalize_explicit_due_date, normalize_meeting_context, validate_model_minutes,
    validate_transcript, validate_verified_minutes,
};

#[cfg(test)]
mod tests;
