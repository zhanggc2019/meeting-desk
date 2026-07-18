use std::path::Path;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::app_state::AppState;
use crate::commands::CommandError;
use crate::minutes::{render_minutes_markdown, MeetingMinutes};

/// 表示前端会议列表使用的摘要。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub id: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub meeting_start_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub updated_at: String,
    pub template_name: String,
}

/// 表示前端会议详情使用的稳定结构。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetailResponse {
    pub id: String,
    pub template_name: String,
    pub duration_ms: Option<u64>,
    pub created_at: String,
    pub minutes: Value,
    pub transcript: Value,
}

/// 表示内置纪要模板的公开元数据。
#[derive(Debug, Serialize)]
pub struct MinutesTemplateResponse {
    pub id: &'static str,
    pub version: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

/// 表示 Markdown 导出是否完成或由用户取消。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub status: &'static str,
    pub display_name: Option<String>,
}

/// 在本地历史中搜索标题、摘要和完整逐字稿。
#[tauri::command]
pub fn list_meetings(
    state: State<'_, AppState>,
    query: Option<String>,
) -> Result<Vec<MeetingSummary>, CommandError> {
    let normalized = query.unwrap_or_default().trim().to_lowercase();
    let items = state.repository.search_meetings("")?;
    let mut summaries = Vec::new();
    for item in items {
        let Some(detail) = state.repository.get_meeting(&item.id)? else {
            continue;
        };
        let title = detail.minutes["title"].as_str().map(str::to_string);
        let summary = detail.minutes["summary"].as_str().map(str::to_string);
        let haystack = format!(
            "{} {} {}",
            title.as_deref().unwrap_or_default(),
            summary.as_deref().unwrap_or_default(),
            detail.transcript
        )
        .to_lowercase();
        if !normalized.is_empty() && !haystack.contains(&normalized) {
            continue;
        }
        summaries.push(MeetingSummary {
            id: detail.id,
            title,
            summary,
            meeting_start_at: detail.minutes["meetingTime"]["startAt"]
                .as_str()
                .map(str::to_string),
            duration_ms: detail.transcript_segments["durationMs"].as_u64(),
            updated_at: detail.updated_at,
            template_name: template_name(&detail.template_id).to_string(),
        });
    }
    Ok(summaries)
}

/// 返回指定会议的完整结构化纪要和逐字稿。
#[tauri::command]
pub fn get_meeting_detail(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingDetailResponse, CommandError> {
    let detail = state
        .repository
        .get_meeting(&meeting_id)?
        .ok_or_else(|| CommandError::new("meeting_not_found", "未找到该会议记录", false))?;
    let transcript = normalize_transcript(&detail.transcript, detail.transcript_segments);
    Ok(MeetingDetailResponse {
        id: detail.id,
        template_name: template_name(&detail.template_id).to_string(),
        duration_ms: transcript["durationMs"].as_u64(),
        created_at: detail.created_at,
        minutes: detail.minutes,
        transcript,
    })
}

/// 返回与文件导出完全一致的 Markdown 文本，供应用内安全预览使用。
#[tauri::command]
pub fn get_meeting_markdown_preview(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<String, CommandError> {
    let detail = state
        .repository
        .get_meeting(&meeting_id)?
        .ok_or_else(|| CommandError::new("meeting_not_found", "未找到该会议记录", false))?;
    let minutes: MeetingMinutes = serde_json::from_value(detail.minutes)
        .map_err(|_| CommandError::new("minutes_invalid", "会议纪要格式无效", false))?;
    Ok(render_export_markdown(&minutes, &detail.transcript))
}

/// 删除本地会议记录；永远不会删除用户选择的原始音频。
#[tauri::command]
pub fn delete_meeting(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<bool, CommandError> {
    state
        .repository
        .delete_meeting(&meeting_id)
        .map_err(CommandError::from)
}

/// 返回稳定顺序的全部内置纪要模板及其用户可读描述。
#[tauri::command]
pub fn list_minutes_templates() -> Vec<MinutesTemplateResponse> {
    crate::minutes::list_templates()
        .iter()
        .map(|template| MinutesTemplateResponse {
            id: template.id,
            version: template.version,
            name: template.display_name,
            description: template.description,
        })
        .collect()
}

/// 让用户选择目标位置，并导出纪要与完整逐字稿 Markdown。
#[tauri::command]
pub async fn export_meeting_markdown(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<ExportResult, CommandError> {
    let detail = state
        .repository
        .get_meeting(&meeting_id)?
        .ok_or_else(|| CommandError::new("meeting_not_found", "未找到该会议记录", false))?;
    let minutes: MeetingMinutes = serde_json::from_value(detail.minutes)
        .map_err(|_| CommandError::new("minutes_invalid", "会议纪要格式无效", false))?;
    let display_name = format!("{}.md", safe_file_stem(minutes.title.as_deref()));
    let selected = app
        .dialog()
        .file()
        .add_filter("Markdown", &["md"])
        .set_file_name(&display_name)
        .blocking_save_file();
    let Some(path) = selected.and_then(|value| value.into_path().ok()) else {
        return Ok(ExportResult {
            status: "cancelled",
            display_name: None,
        });
    };
    ensure_markdown_extension(&path)?;
    let markdown = render_export_markdown(&minutes, &detail.transcript);
    tauri::async_runtime::spawn_blocking(move || std::fs::write(path, markdown))
        .await
        .map_err(|_| CommandError::new("export_failed", "Markdown 导出未完成", true))?
        .map_err(|_| CommandError::new("export_failed", "无法写入所选文件", true))?;
    Ok(ExportResult {
        status: "exported",
        display_name: Some(display_name),
    })
}

/// 把旧记录中的 segments 数组兼容成完整 Transcript 对象。
fn normalize_transcript(text: &str, stored: Value) -> Value {
    if stored.is_object() && stored.get("text").is_some() {
        stored
    } else {
        serde_json::json!({
            "schemaVersion": "1",
            "text": text,
            "language": null,
            "durationMs": null,
            "segments": stored.as_array().cloned().unwrap_or_default()
        })
    }
}

/// 按模板 ID 返回稳定中文名称。
fn template_name(template_id: &str) -> &'static str {
    crate::minutes::get_template(template_id, crate::minutes::BUILTIN_TEMPLATE_VERSION)
        .map(|template| template.display_name)
        .unwrap_or("未知模板")
}

/// 组合结构化纪要和逐字稿，并对逐字稿使用缩进代码块以保持原文。
fn render_export_markdown(minutes: &MeetingMinutes, transcript: &str) -> String {
    let mut output = render_minutes_markdown(minutes);
    output.push_str("\n\n## 完整逐字稿\n\n");
    if transcript.trim().is_empty() {
        output.push_str("无可用逐字稿");
    } else {
        for line in transcript.lines() {
            output.push_str("    ");
            output.push_str(line);
            output.push('\n');
        }
        output.pop();
    }
    output.push('\n');
    output
}

/// 生成不含 Windows 非法字符的导出文件名主体。
fn safe_file_stem(title: Option<&str>) -> String {
    let sanitized = title
        .unwrap_or("会议纪要")
        .chars()
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim().trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        "会议纪要".to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

/// 确保最终导出目标为 Markdown 文件。
fn ensure_markdown_extension(path: &Path) -> Result<(), CommandError> {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("md"))
    {
        Ok(())
    } else {
        Err(CommandError::new(
            "export_extension_invalid",
            "导出文件必须使用 .md 扩展名",
            false,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::minutes::{MeetingTime, TitleSource};

    use super::*;

    /// 构造最小有效纪要用于导出测试。
    fn fixture_minutes() -> MeetingMinutes {
        MeetingMinutes {
            schema_version: "1.0.0".to_string(),
            title: Some("项目/周会".to_string()),
            title_source: TitleSource::Generated,
            meeting_time: MeetingTime {
                start_at: None,
                end_at: None,
            },
            participants: Vec::new(),
            summary: Some("摘要".to_string()),
            topics: Vec::new(),
            conclusions: Vec::new(),
            decisions: Vec::new(),
            action_items: Vec::new(),
            risks_and_issues: Vec::new(),
        }
    }

    /// 验证 Markdown 同时包含固定纪要章节和完整逐字稿。
    #[test]
    fn export_contains_minutes_and_transcript() {
        let output = render_export_markdown(&fixture_minutes(), "第一句。\n第二句。");
        assert!(output.contains("## 会议摘要"));
        assert!(output.contains("## 完整逐字稿"));
        assert!(output.contains("    第一句。"));
    }

    /// 验证 Windows 非法文件名字符会被替换。
    #[test]
    fn sanitizes_export_file_name() {
        assert_eq!(safe_file_stem(Some("项目/周会")), "项目_周会");
    }
}
