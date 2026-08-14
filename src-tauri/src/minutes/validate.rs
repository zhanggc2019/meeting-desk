use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDate};

use crate::providers::{Transcript, TranscriptSegment};

use super::{
    ActionItem, MeetingContext, MeetingMinutes, MinutesError, RiskOrIssue, SupportedStatement,
    TitleSource, Topic, ValidationOptions, MEETING_MINUTES_SCHEMA_VERSION,
};

const MAX_TITLE_CHARS: usize = 200;
const MAX_SHORT_TEXT_CHARS: usize = 500;
const MAX_BODY_TEXT_CHARS: usize = 2_000;
const MAX_SUMMARY_CHARS: usize = 5_000;
const MAX_COLLECTION_ITEMS: usize = 100;
const MAX_EVIDENCE_ITEMS: usize = 100;

#[derive(Clone, Copy)]
enum ValidationMode {
    ModelCandidate,
    VerifiedValue,
}

/// 校验 Provider-neutral transcript 的空文本、segment、时间戳和 confidence 不变量。
pub fn validate_transcript(
    transcript: &Transcript,
    options: ValidationOptions,
) -> Result<(), MinutesError> {
    validate_confidence_threshold(options.low_confidence_threshold)?;
    if transcript.schema_version != "1" {
        return Err(MinutesError::InvalidTranscript {
            code: "invalid_transcript_schema_version",
        });
    }
    if transcript.text.trim().is_empty() {
        return Err(MinutesError::EmptyTranscript);
    }
    let mut ids = HashSet::new();
    for segment in &transcript.segments {
        if segment.id.trim().is_empty() || segment.id.chars().count() > 200 {
            return Err(MinutesError::InvalidTranscript {
                code: "invalid_segment_id",
            });
        }
        if !ids.insert(segment.id.as_str()) {
            return Err(MinutesError::InvalidTranscript {
                code: "duplicate_segment_id",
            });
        }
        if segment.text.trim().is_empty() {
            return Err(MinutesError::InvalidTranscript {
                code: "empty_segment_text",
            });
        }
        if matches!((segment.start_ms, segment.end_ms), (Some(start), Some(end)) if start > end) {
            return Err(MinutesError::InvalidTranscript {
                code: "invalid_segment_time_range",
            });
        }
        if segment
            .speaker_label
            .as_deref()
            .is_some_and(|label| label.trim().is_empty() || label.chars().count() > 200)
        {
            return Err(MinutesError::InvalidTranscript {
                code: "invalid_speaker_label",
            });
        }
        if segment
            .confidence
            .is_some_and(|confidence| !confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
        {
            return Err(MinutesError::InvalidTranscript {
                code: "invalid_segment_confidence",
            });
        }
    }
    Ok(())
}

/// 校验并规范化可信会议上下文，不从 transcript 补齐任何字段。
pub fn normalize_meeting_context(context: &MeetingContext) -> Result<MeetingContext, MinutesError> {
    let mut normalized = context.clone();
    normalize_optional_text(
        &mut normalized.known_title,
        MAX_TITLE_CHARS,
        "/meetingContext/knownTitle",
    )?;
    normalize_optional_text(
        &mut normalized.known_start_at,
        MAX_SHORT_TEXT_CHARS,
        "/meetingContext/knownStartAt",
    )?;
    normalize_optional_text(
        &mut normalized.known_end_at,
        MAX_SHORT_TEXT_CHARS,
        "/meetingContext/knownEndAt",
    )?;
    for participant in &mut normalized.known_participants {
        normalize_required_text(
            participant,
            MAX_TITLE_CHARS,
            "/meetingContext/knownParticipants",
        )?;
    }
    stable_dedup(&mut normalized.known_participants);
    if normalized.known_participants.len() > MAX_COLLECTION_ITEMS {
        return Err(schema_violation(
            "collection_too_large",
            "/meetingContext/knownParticipants",
        ));
    }
    validate_meeting_time(
        normalized.known_start_at.as_deref(),
        normalized.known_end_at.as_deref(),
        "/meetingContext",
    )?;
    Ok(normalized)
}

/// 校验模型候选值，并由可信代码计算可无歧义规范化的 dueDate。
pub fn validate_model_minutes(
    minutes: MeetingMinutes,
    expected_schema_version: &str,
    transcript: &Transcript,
    context: &MeetingContext,
    options: ValidationOptions,
) -> Result<MeetingMinutes, MinutesError> {
    validate_minutes(
        minutes,
        expected_schema_version,
        transcript,
        context,
        options,
        ValidationMode::ModelCandidate,
    )
}

/// 重新校验已持久化或 fixture 中的最终纪要值。
pub fn validate_verified_minutes(
    minutes: MeetingMinutes,
    expected_schema_version: &str,
    transcript: &Transcript,
    context: &MeetingContext,
    options: ValidationOptions,
) -> Result<MeetingMinutes, MinutesError> {
    validate_minutes(
        minutes,
        expected_schema_version,
        transcript,
        context,
        options,
        ValidationMode::VerifiedValue,
    )
}

/// 仅在期限文本自身是完整公历日期时返回 YYYY-MM-DD。
pub fn normalize_explicit_due_date(value: &str) -> Option<String> {
    let value = value.trim();
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(date.format("%Y-%m-%d").to_string());
    }
    parse_chinese_date(value).map(|date| date.format("%Y-%m-%d").to_string())
}

/// 执行结构边界、可信上下文、证据和低置信度语义校验。
fn validate_minutes(
    mut minutes: MeetingMinutes,
    expected_schema_version: &str,
    transcript: &Transcript,
    context: &MeetingContext,
    options: ValidationOptions,
    mode: ValidationMode,
) -> Result<MeetingMinutes, MinutesError> {
    validate_transcript(transcript, options)?;
    let context = normalize_meeting_context(context)?;
    if expected_schema_version != MEETING_MINUTES_SCHEMA_VERSION
        || minutes.schema_version != expected_schema_version
    {
        return Err(MinutesError::SchemaVersionMismatch);
    }

    normalize_and_validate_structure(&mut minutes)?;
    validate_protected_context(&mut minutes, &context, mode)?;
    let segments = transcript
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| (segment.id.as_str(), (index, segment)))
        .collect::<HashMap<_, _>>();
    normalize_all_evidence(
        &mut minutes,
        &segments,
        !transcript.segments.is_empty(),
        mode,
    )?;
    validate_item_semantics(&mut minutes, transcript, &segments, options, mode)?;
    Ok(minutes)
}

/// 规范化所有用户可见文本，并执行 JSON Schema 对应的长度和集合边界。
fn normalize_and_validate_structure(minutes: &mut MeetingMinutes) -> Result<(), MinutesError> {
    normalize_optional_text(&mut minutes.title, MAX_TITLE_CHARS, "/title")?;
    normalize_optional_text(&mut minutes.summary, MAX_SUMMARY_CHARS, "/summary")?;
    validate_collection_size(minutes.participants.len(), "/participants")?;
    let mut participants = HashSet::new();
    for participant in &mut minutes.participants {
        normalize_required_text(participant, MAX_TITLE_CHARS, "/participants")?;
        if !participants.insert(participant.clone()) {
            return Err(schema_violation("duplicate_item", "/participants"));
        }
    }
    validate_collection_size(minutes.topics.len(), "/topics")?;
    for topic in &mut minutes.topics {
        normalize_topic(topic)?;
    }
    validate_collection_size(minutes.conclusions.len(), "/conclusions")?;
    for statement in &mut minutes.conclusions {
        normalize_statement(statement, "/conclusions")?;
    }
    validate_collection_size(minutes.decisions.len(), "/decisions")?;
    for statement in &mut minutes.decisions {
        normalize_statement(statement, "/decisions")?;
    }
    validate_collection_size(minutes.action_items.len(), "/actionItems")?;
    for item in &mut minutes.action_items {
        normalize_action_item(item)?;
    }
    validate_collection_size(minutes.risks_and_issues.len(), "/risksAndIssues")?;
    for item in &mut minutes.risks_and_issues {
        normalize_risk_or_issue(item)?;
    }
    stable_dedup(&mut minutes.topics);
    stable_dedup(&mut minutes.conclusions);
    stable_dedup(&mut minutes.decisions);
    stable_dedup(&mut minutes.action_items);
    stable_dedup(&mut minutes.risks_and_issues);
    validate_meeting_time(
        minutes.meeting_time.start_at.as_deref(),
        minutes.meeting_time.end_at.as_deref(),
        "/meetingTime",
    )?;
    Ok(())
}

/// 校验议题字段。
fn normalize_topic(topic: &mut Topic) -> Result<(), MinutesError> {
    normalize_required_text(&mut topic.title, MAX_SHORT_TEXT_CHARS, "/topics/title")?;
    normalize_optional_text(&mut topic.summary, MAX_SUMMARY_CHARS, "/topics/summary")?;
    validate_evidence_shape(&topic.evidence_segment_ids, "/topics/evidenceSegmentIds")
}

/// 校验结论或决策字段。
fn normalize_statement(
    statement: &mut SupportedStatement,
    path: &'static str,
) -> Result<(), MinutesError> {
    normalize_required_text(&mut statement.content, MAX_BODY_TEXT_CHARS, path)?;
    validate_evidence_shape(&statement.evidence_segment_ids, path)
}

/// 校验待办字段，但不允许模型在此阶段推导 dueDate。
fn normalize_action_item(item: &mut ActionItem) -> Result<(), MinutesError> {
    normalize_required_text(
        &mut item.description,
        MAX_BODY_TEXT_CHARS,
        "/actionItems/description",
    )?;
    normalize_optional_text(&mut item.owner, MAX_SHORT_TEXT_CHARS, "/actionItems/owner")?;
    normalize_optional_text(
        &mut item.due_date_text,
        MAX_SHORT_TEXT_CHARS,
        "/actionItems/dueDateText",
    )?;
    normalize_optional_text(
        &mut item.due_date,
        MAX_SHORT_TEXT_CHARS,
        "/actionItems/dueDate",
    )?;
    validate_evidence_shape(
        &item.evidence_segment_ids,
        "/actionItems/evidenceSegmentIds",
    )
}

/// 校验风险或问题字段。
fn normalize_risk_or_issue(item: &mut RiskOrIssue) -> Result<(), MinutesError> {
    normalize_required_text(
        &mut item.description,
        MAX_BODY_TEXT_CHARS,
        "/risksAndIssues/description",
    )?;
    normalize_optional_text(
        &mut item.impact,
        MAX_SUMMARY_CHARS,
        "/risksAndIssues/impact",
    )?;
    normalize_optional_text(
        &mut item.mitigation,
        MAX_SUMMARY_CHARS,
        "/risksAndIssues/mitigation",
    )?;
    validate_evidence_shape(
        &item.evidence_segment_ids,
        "/risksAndIssues/evidenceSegmentIds",
    )
}

/// 校验 title、time、participants 只来自可信上下文。
fn validate_protected_context(
    minutes: &mut MeetingMinutes,
    context: &MeetingContext,
    mode: ValidationMode,
) -> Result<(), MinutesError> {
    if matches!(mode, ValidationMode::ModelCandidate) {
        if let Some(title) = context.known_title.as_ref() {
            minutes.title = Some(title.clone());
            minutes.title_source = TitleSource::Context;
        } else {
            minutes.title_source = if minutes.title.is_some() {
                TitleSource::Generated
            } else {
                TitleSource::Unknown
            };
        }
        minutes.meeting_time.start_at = context.known_start_at.clone();
        minutes.meeting_time.end_at = context.known_end_at.clone();
        minutes.participants = context.known_participants.clone();
        return Ok(());
    }
    match context.known_title.as_deref() {
        Some(title) => {
            if minutes.title.as_deref() != Some(title)
                || minutes.title_source != TitleSource::Context
            {
                return Err(semantic_violation("context_field_mismatch", "/title"));
            }
        }
        None => match (&minutes.title, minutes.title_source) {
            (None, TitleSource::Unknown) | (Some(_), TitleSource::Generated) => {}
            _ => return Err(semantic_violation("invalid_title_source", "/titleSource")),
        },
    }
    if minutes.meeting_time.start_at != context.known_start_at
        || minutes.meeting_time.end_at != context.known_end_at
    {
        return Err(semantic_violation("context_field_mismatch", "/meetingTime"));
    }
    if minutes.participants != context.known_participants {
        return Err(semantic_violation(
            "inferred_identity_rejected",
            "/participants",
        ));
    }
    Ok(())
}

/// 规范化全部 evidence ID，拒绝悬空引用并按 transcript 顺序排序。
fn normalize_all_evidence(
    minutes: &mut MeetingMinutes,
    segments: &HashMap<&str, (usize, &TranscriptSegment)>,
    require_evidence: bool,
    mode: ValidationMode,
) -> Result<(), MinutesError> {
    if matches!(mode, ValidationMode::ModelCandidate) {
        minutes.topics.retain_mut(|topic| {
            sanitize_model_evidence(&mut topic.evidence_segment_ids, segments, require_evidence)
        });
        minutes.conclusions.retain_mut(|statement| {
            sanitize_model_evidence(
                &mut statement.evidence_segment_ids,
                segments,
                require_evidence,
            )
        });
        minutes.decisions.retain_mut(|statement| {
            sanitize_model_evidence(
                &mut statement.evidence_segment_ids,
                segments,
                require_evidence,
            )
        });
        minutes.action_items.retain_mut(|item| {
            sanitize_model_evidence(&mut item.evidence_segment_ids, segments, require_evidence)
        });
        minutes.risks_and_issues.retain_mut(|item| {
            sanitize_model_evidence(&mut item.evidence_segment_ids, segments, require_evidence)
        });
        return Ok(());
    }
    for topic in &mut minutes.topics {
        normalize_evidence(
            &mut topic.evidence_segment_ids,
            segments,
            require_evidence,
            "/topics/evidenceSegmentIds",
        )?;
    }
    for statement in &mut minutes.conclusions {
        normalize_evidence(
            &mut statement.evidence_segment_ids,
            segments,
            require_evidence,
            "/conclusions/evidenceSegmentIds",
        )?;
    }
    for statement in &mut minutes.decisions {
        normalize_evidence(
            &mut statement.evidence_segment_ids,
            segments,
            require_evidence,
            "/decisions/evidenceSegmentIds",
        )?;
    }
    for item in &mut minutes.action_items {
        normalize_evidence(
            &mut item.evidence_segment_ids,
            segments,
            require_evidence,
            "/actionItems/evidenceSegmentIds",
        )?;
    }
    for item in &mut minutes.risks_and_issues {
        normalize_evidence(
            &mut item.evidence_segment_ids,
            segments,
            require_evidence,
            "/risksAndIssues/evidenceSegmentIds",
        )?;
    }
    Ok(())
}

/// 清理模型候选中的悬空 evidence，并判断对应可选条目是否仍有足够证据保留。
fn sanitize_model_evidence(
    ids: &mut Vec<String>,
    segments: &HashMap<&str, (usize, &TranscriptSegment)>,
    require_evidence: bool,
) -> bool {
    if segments.is_empty() {
        ids.clear();
        return !require_evidence;
    }
    ids.retain(|id| segments.contains_key(id.as_str()));
    ids.sort_by_key(|id| segments.get(id.as_str()).map(|(index, _)| *index));
    ids.dedup();
    !require_evidence || !ids.is_empty()
}

/// 校验 owner、due date、决策确认词和低置信度唯一证据。
fn validate_item_semantics(
    minutes: &mut MeetingMinutes,
    transcript: &Transcript,
    segments: &HashMap<&str, (usize, &TranscriptSegment)>,
    options: ValidationOptions,
    mode: ValidationMode,
) -> Result<(), MinutesError> {
    if matches!(mode, ValidationMode::ModelCandidate) {
        sanitize_model_item_semantics(minutes, transcript, segments, options);
        return Ok(());
    }
    for statement in &minutes.conclusions {
        reject_low_confidence_only(
            &statement.evidence_segment_ids,
            segments,
            options.low_confidence_threshold,
            "/conclusions/evidenceSegmentIds",
        )?;
    }
    for statement in &minutes.decisions {
        reject_low_confidence_only(
            &statement.evidence_segment_ids,
            segments,
            options.low_confidence_threshold,
            "/decisions/evidenceSegmentIds",
        )?;
        if !evidence_has_confirmation(&statement.evidence_segment_ids, transcript, segments) {
            return Err(semantic_violation("decision_not_explicit", "/decisions"));
        }
    }
    for item in &mut minutes.action_items {
        reject_low_confidence_only(
            &item.evidence_segment_ids,
            segments,
            options.low_confidence_threshold,
            "/actionItems/evidenceSegmentIds",
        )?;
        if let Some(owner) = item.owner.as_deref() {
            if is_forbidden_owner(owner)
                || !evidence_contains(&item.evidence_segment_ids, owner, transcript, segments)
            {
                return Err(semantic_violation(
                    "inferred_owner_rejected",
                    "/actionItems/owner",
                ));
            }
        }
        if let Some(due_text) = item.due_date_text.as_deref() {
            if !evidence_contains(&item.evidence_segment_ids, due_text, transcript, segments) {
                return Err(semantic_violation(
                    "inferred_due_date_rejected",
                    "/actionItems/dueDateText",
                ));
            }
        }
        let normalized_due_date = item
            .due_date_text
            .as_deref()
            .and_then(normalize_explicit_due_date);
        if item.due_date != normalized_due_date {
            return Err(semantic_violation(
                "ambiguous_due_date",
                "/actionItems/dueDate",
            ));
        }
    }
    Ok(())
}

/// 清洗模型候选中的可选高影响事实，保留其余已经可用的纪要内容。
fn sanitize_model_item_semantics(
    minutes: &mut MeetingMinutes,
    transcript: &Transcript,
    segments: &HashMap<&str, (usize, &TranscriptSegment)>,
    options: ValidationOptions,
) {
    minutes.conclusions.retain(|statement| {
        reject_low_confidence_only(
            &statement.evidence_segment_ids,
            segments,
            options.low_confidence_threshold,
            "/conclusions/evidenceSegmentIds",
        )
        .is_ok()
    });
    minutes.decisions.retain(|statement| {
        reject_low_confidence_only(
            &statement.evidence_segment_ids,
            segments,
            options.low_confidence_threshold,
            "/decisions/evidenceSegmentIds",
        )
        .is_ok()
            && evidence_has_confirmation(&statement.evidence_segment_ids, transcript, segments)
    });
    minutes.action_items.retain_mut(|item| {
        if reject_low_confidence_only(
            &item.evidence_segment_ids,
            segments,
            options.low_confidence_threshold,
            "/actionItems/evidenceSegmentIds",
        )
        .is_err()
        {
            return false;
        }
        let owner_is_supported = item.owner.as_deref().is_none_or(|owner| {
            !is_forbidden_owner(owner)
                && evidence_contains(&item.evidence_segment_ids, owner, transcript, segments)
        });
        if !owner_is_supported {
            item.owner = None;
        }
        let due_date_is_supported = item.due_date_text.as_deref().is_none_or(|due_text| {
            evidence_contains(&item.evidence_segment_ids, due_text, transcript, segments)
        });
        if !due_date_is_supported {
            item.due_date_text = None;
        }
        item.due_date = item
            .due_date_text
            .as_deref()
            .and_then(normalize_explicit_due_date);
        true
    });
}

/// 校验并排序一组 evidence ID。
fn normalize_evidence(
    ids: &mut Vec<String>,
    segments: &HashMap<&str, (usize, &TranscriptSegment)>,
    require_evidence: bool,
    path: &'static str,
) -> Result<(), MinutesError> {
    if ids.len() > MAX_EVIDENCE_ITEMS {
        return Err(schema_violation("collection_too_large", path));
    }
    if segments.is_empty() {
        if ids.is_empty() {
            return Ok(());
        }
        return Err(semantic_violation("invalid_evidence_reference", path));
    }
    if require_evidence && ids.is_empty() {
        return Err(semantic_violation("missing_evidence", path));
    }
    if ids.iter().any(|id| !segments.contains_key(id.as_str())) {
        return Err(semantic_violation("invalid_evidence_reference", path));
    }
    ids.sort_by_key(|id| segments.get(id.as_str()).map(|(index, _)| *index));
    ids.dedup();
    Ok(())
}

/// 校验 evidence ID 本身满足 Schema 的非空与长度约束。
fn validate_evidence_shape(ids: &[String], path: &'static str) -> Result<(), MinutesError> {
    if ids.len() > MAX_EVIDENCE_ITEMS {
        return Err(schema_violation("collection_too_large", path));
    }
    if ids
        .iter()
        .any(|id| id.trim().is_empty() || id.chars().count() > 200 || id.trim() != id)
    {
        return Err(schema_violation("invalid_evidence_id", path));
    }
    Ok(())
}

/// 拒绝只由一个或多个低置信度 segment 支持的高影响事实。
fn reject_low_confidence_only(
    ids: &[String],
    segments: &HashMap<&str, (usize, &TranscriptSegment)>,
    threshold: Option<f64>,
    path: &'static str,
) -> Result<(), MinutesError> {
    let Some(threshold) = threshold else {
        return Ok(());
    };
    if ids.is_empty() {
        return Ok(());
    }
    let all_low = ids.iter().all(|id| {
        segments
            .get(id.as_str())
            .and_then(|(_, segment)| segment.confidence)
            .is_some_and(|confidence| confidence < threshold)
    });
    if all_low {
        return Err(semantic_violation("low_confidence_only_evidence", path));
    }
    Ok(())
}

/// 检查 evidence 或无 segment 时的全文是否逐字包含一个敏感事实值。
fn evidence_contains(
    ids: &[String],
    needle: &str,
    transcript: &Transcript,
    segments: &HashMap<&str, (usize, &TranscriptSegment)>,
) -> bool {
    if ids.is_empty() {
        return transcript.text.contains(needle);
    }
    ids.iter().any(|id| {
        segments
            .get(id.as_str())
            .is_some_and(|(_, segment)| segment.text.contains(needle))
    })
}

/// 检查决策 evidence 是否包含有限的明确确认语义标记。
fn evidence_has_confirmation(
    ids: &[String],
    transcript: &Transcript,
    segments: &HashMap<&str, (usize, &TranscriptSegment)>,
) -> bool {
    const MARKERS: [&str; 9] = [
        "决定",
        "确认",
        "通过",
        "同意",
        "拍板",
        "agreed",
        "decided",
        "approved",
        "confirmed",
    ];
    let contains_marker = |text: &str| {
        let lower = text.to_lowercase();
        MARKERS.iter().any(|marker| lower.contains(marker))
    };
    if ids.is_empty() {
        return contains_marker(&transcript.text);
    }
    ids.iter().any(|id| {
        segments
            .get(id.as_str())
            .is_some_and(|(_, segment)| contains_marker(&segment.text))
    })
}

/// 判断 owner 是否为代词或匿名 speaker label。
fn is_forbidden_owner(owner: &str) -> bool {
    let normalized = owner.trim().to_lowercase();
    const PRONOUNS: [&str; 14] = [
        "我", "我们", "他", "她", "他们", "她们", "这边", "那边", "i", "we", "he", "she", "they",
        "us",
    ];
    PRONOUNS.contains(&normalized.as_str())
        || normalized.starts_with("speaker ")
        || normalized.starts_with("speaker_")
        || normalized.starts_with("说话人")
}

/// 校验 optional string 并规范化首尾空白。
fn normalize_optional_text(
    value: &mut Option<String>,
    max_chars: usize,
    path: &'static str,
) -> Result<(), MinutesError> {
    if let Some(value) = value {
        normalize_required_text(value, max_chars, path)?;
    }
    Ok(())
}

/// 校验 required string 并规范化首尾空白。
fn normalize_required_text(
    value: &mut String,
    max_chars: usize,
    path: &'static str,
) -> Result<(), MinutesError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(schema_violation("blank_string", path));
    }
    if trimmed.chars().count() > max_chars {
        return Err(schema_violation("string_too_long", path));
    }
    if is_unknown_placeholder(trimmed) {
        return Err(schema_violation("unknown_placeholder", path));
    }
    if trimmed.len() != value.len() {
        *value = trimmed.to_string();
    }
    Ok(())
}

/// 判断是否错误地用展示占位词替代 null 或空数组。
fn is_unknown_placeholder(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "未知" | "无" | "待定" | "n/a" | "na" | "none" | "null"
    )
}

/// 校验集合最大项数。
fn validate_collection_size(len: usize, path: &'static str) -> Result<(), MinutesError> {
    if len > MAX_COLLECTION_ITEMS {
        return Err(schema_violation("collection_too_large", path));
    }
    Ok(())
}

/// 校验低置信度阈值是有限的 [0, 1] 数值。
fn validate_confidence_threshold(threshold: Option<f64>) -> Result<(), MinutesError> {
    if threshold.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
        return Err(MinutesError::InvalidConfidenceThreshold);
    }
    Ok(())
}

/// 校验 RFC 3339 会议时间及先后顺序。
fn validate_meeting_time(
    start_at: Option<&str>,
    end_at: Option<&str>,
    path: &'static str,
) -> Result<(), MinutesError> {
    let start = start_at
        .map(DateTime::parse_from_rfc3339)
        .transpose()
        .map_err(|_| schema_violation("invalid_date_time", path))?;
    let end = end_at
        .map(DateTime::parse_from_rfc3339)
        .transpose()
        .map_err(|_| schema_violation("invalid_date_time", path))?;
    if matches!((start, end), (Some(start), Some(end)) if end < start) {
        return Err(semantic_violation("invalid_time_range", path));
    }
    Ok(())
}

/// 解析完全明确的中文公历年月日，不补齐缺失信息。
fn parse_chinese_date(value: &str) -> Option<NaiveDate> {
    let without_day = value.strip_suffix('日')?;
    let (year, month_and_day) = without_day.split_once('年')?;
    let (month, day) = month_and_day.split_once('月')?;
    if year.len() != 4
        || !year.chars().all(|value| value.is_ascii_digit())
        || !month.chars().all(|value| value.is_ascii_digit())
        || !day.chars().all(|value| value.is_ascii_digit())
    {
        return None;
    }
    NaiveDate::from_ymd_opt(year.parse().ok()?, month.parse().ok()?, day.parse().ok()?)
}

/// 稳定移除完全相同的重复条目，保留首次出现顺序。
fn stable_dedup<T: PartialEq>(values: &mut Vec<T>) {
    let mut result = Vec::with_capacity(values.len());
    for value in values.drain(..) {
        if !result.contains(&value) {
            result.push(value);
        }
    }
    *values = result;
}

/// 构造不含实际字段值的结构错误。
fn schema_violation(code: &'static str, path: &'static str) -> MinutesError {
    MinutesError::SchemaViolation { code, path }
}

/// 构造不含实际字段值的语义错误。
fn semantic_violation(code: &'static str, path: &'static str) -> MinutesError {
    MinutesError::SemanticViolation { code, path }
}
