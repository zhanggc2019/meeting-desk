use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::app_state::AppState;
use crate::commands::CommandError;
use crate::domain::{MeetingDetail, TaskRecord, TaskStatus};
use crate::meeting_export::{export_meeting_document, ExportContent, ExportFormat};
use crate::minutes::{render_minutes_markdown, MeetingMinutes};
use crate::storage::MeetingRepository;

/// 表示前端会议列表使用的摘要。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub id: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub meeting_start_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub processing_duration_ms: Option<u64>,
    pub updated_at: String,
    pub template_name: String,
}

/// 表示会议历史分页查询参数，页码从 1 开始。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingPageQuery {
    #[serde(default)]
    pub query: String,
    pub page: u64,
    pub page_size: u64,
}

/// 表示会议历史分页结果及搜索后的总数。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingPageResponse {
    pub items: Vec<MeetingSummary>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
    pub total_pages: u64,
}

/// 表示前端会议详情使用的稳定结构。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetailResponse {
    pub id: String,
    pub source_name: String,
    pub template_name: String,
    pub duration_ms: Option<u64>,
    pub processing_duration_ms: Option<u64>,
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

/// 表示文档导出是否完成或由用户取消。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub status: &'static str,
    pub display_name: Option<String>,
}

/// 表示前端提交的导出格式与内容组合。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMeetingRequest {
    pub format: ExportFormat,
    pub contents: Vec<ExportContent>,
}

/// 表示一次本地媒体试听请求的结果。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackResult {
    pub status: &'static str,
    pub rebound_source: bool,
    pub source_path: Option<String>,
}

/// 在本地历史中搜索标题、摘要和完整逐字稿。
#[tauri::command]
pub fn list_meetings(
    state: State<'_, AppState>,
    query: Option<String>,
) -> Result<Vec<MeetingSummary>, CommandError> {
    build_meeting_summaries(&state.repository, query.as_deref())
}

/// 返回搜索后的会议历史分页，保留旧版完整列表命令的兼容性。
#[tauri::command]
pub fn list_meetings_page(
    state: State<'_, AppState>,
    query: MeetingPageQuery,
) -> Result<MeetingPageResponse, CommandError> {
    let operation_started = Instant::now();
    let has_query = !query.query.trim().is_empty();
    let offset = pagination_offset(query.page, query.page_size)?;
    let (mut items, total) =
        state
            .repository
            .search_meetings_page(&query.query, offset, query.page_size)?;
    let actual_page = clamped_page(total, query.page, query.page_size);
    if actual_page != query.page {
        let actual_offset = pagination_offset(actual_page, query.page_size)?;
        items = state
            .repository
            .search_meetings_page(&query.query, actual_offset, query.page_size)?
            .0;
    }
    let mut summaries = Vec::with_capacity(items.len());
    for item in items {
        let Some(detail) = state.repository.get_meeting(&item.id)? else {
            continue;
        };
        let processing_duration_ms = state
            .repository
            .get_completed_task_for_meeting(&detail.id)?
            .as_ref()
            .and_then(task_processing_duration_ms);
        summaries.push(meeting_summary_from_detail(detail, processing_duration_ms));
    }
    let response = meeting_page_response(summaries, total, actual_page, query.page_size);
    log::info!(
        target: "app.meetings",
        "meeting_page_loaded has_query={} page={} page_size={} result_count={} total={} elapsed_ms={}",
        has_query,
        response.page,
        response.page_size,
        response.items.len(),
        response.total,
        operation_started.elapsed().as_millis(),
    );
    Ok(response)
}

/// 从本地仓库构建会议摘要，并在数据库查询后执行正文搜索过滤。
fn build_meeting_summaries(
    repository: &MeetingRepository,
    query: Option<&str>,
) -> Result<Vec<MeetingSummary>, CommandError> {
    let normalized = query.unwrap_or_default().trim().to_lowercase();
    let items = repository.search_meetings("")?;
    let tasks = repository.list_tasks()?;
    let mut summaries = Vec::new();
    for item in items {
        let Some(detail) = repository.get_meeting(&item.id)? else {
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
        let processing_duration_ms = processing_duration_for_meeting(&tasks, &detail.id);
        summaries.push(meeting_summary_from_detail(detail, processing_duration_ms));
    }
    Ok(summaries)
}

/// 把完整会议记录转换为不包含逐字稿正文的列表摘要。
fn meeting_summary_from_detail(
    detail: MeetingDetail,
    processing_duration_ms: Option<u64>,
) -> MeetingSummary {
    MeetingSummary {
        id: detail.id,
        title: detail.minutes["title"].as_str().map(str::to_string),
        summary: detail.minutes["summary"].as_str().map(str::to_string),
        meeting_start_at: detail.minutes["meetingTime"]["startAt"]
            .as_str()
            .map(str::to_string),
        duration_ms: detail.transcript_segments["durationMs"].as_u64(),
        processing_duration_ms,
        updated_at: detail.updated_at,
        template_name: template_name(&detail.template_id).to_string(),
    }
}

/// 对搜索完成的会议摘要应用有界分页，并返回过滤后的总数。
#[cfg(test)]
fn paginate_meetings(
    summaries: Vec<MeetingSummary>,
    page: u64,
    page_size: u64,
) -> Result<MeetingPageResponse, CommandError> {
    pagination_offset(page, page_size)?;
    let total = summaries.len() as u64;
    let actual_page = clamped_page(total, page, page_size);
    let offset = pagination_offset(actual_page, page_size)? as usize;
    let items = summaries
        .into_iter()
        .skip(offset)
        .take(page_size as usize)
        .collect();
    Ok(meeting_page_response(items, total, actual_page, page_size))
}

/// 构造稳定的会议分页响应，空结果仍保留第 1 页。
fn meeting_page_response(
    items: Vec<MeetingSummary>,
    total: u64,
    page: u64,
    page_size: u64,
) -> MeetingPageResponse {
    MeetingPageResponse {
        items,
        total,
        page,
        page_size,
        total_pages: total.div_ceil(page_size).max(1),
    }
}

/// 把越过最后一页的请求收敛到当前有效页，便于删除最后一项后继续浏览。
fn clamped_page(total: u64, page: u64, page_size: u64) -> u64 {
    page.min(total.div_ceil(page_size).max(1))
}

/// 校验从 1 开始的分页参数，并安全计算数据库 OFFSET。
fn pagination_offset(page: u64, page_size: u64) -> Result<u64, CommandError> {
    if page == 0 || !(1..=100).contains(&page_size) {
        return Err(CommandError::new(
            "pagination_invalid",
            "分页参数无效：页码须从 1 开始，每页最多 100 条",
            false,
        ));
    }
    let offset = page
        .saturating_sub(1)
        .checked_mul(page_size)
        .ok_or_else(|| CommandError::new("pagination_invalid", "分页参数超出支持范围", false))?;
    if offset > i64::MAX as u64 {
        return Err(CommandError::new(
            "pagination_invalid",
            "分页参数超出支持范围",
            false,
        ));
    }
    Ok(offset)
}

/// 返回指定会议的完整结构化纪要和逐字稿。
#[tauri::command]
pub fn get_meeting_detail(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingDetailResponse, CommandError> {
    let operation_started = Instant::now();
    let detail = state
        .repository
        .get_meeting(&meeting_id)?
        .ok_or_else(|| CommandError::new("meeting_not_found", "未找到该会议记录", false))?;
    let tasks = state.repository.list_tasks()?;
    let processing_duration_ms = processing_duration_for_meeting(&tasks, &detail.id);
    let transcript = normalize_transcript(&detail.transcript, detail.transcript_segments);
    let response = MeetingDetailResponse {
        id: detail.id,
        source_name: detail.source_name,
        template_name: template_name(&detail.template_id).to_string(),
        duration_ms: transcript["durationMs"].as_u64(),
        processing_duration_ms,
        created_at: detail.created_at,
        minutes: detail.minutes,
        transcript,
    };
    log::info!(
        target: "app.meetings",
        "meeting_detail_loaded meeting_id={} duration_ms={:?} elapsed_ms={}",
        response.id,
        response.duration_ms,
        operation_started.elapsed().as_millis(),
    );
    Ok(response)
}

/// 授权应用内播放器读取原始媒体；旧记录缺少路径时引导用户重新关联。
#[tauri::command]
pub fn play_meeting_media(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<PlaybackResult, CommandError> {
    let detail = state
        .repository
        .get_meeting(&meeting_id)?
        .ok_or_else(|| CommandError::new("meeting_not_found", "未找到该录音记录", false))?;
    let saved_path = detail
        .source_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|path| path.is_file());
    let (media_path, rebound_source) = if let Some(path) = saved_path {
        (path, false)
    } else {
        let selected = app
            .dialog()
            .file()
            .set_title("选择此记录对应的原始媒体文件")
            .add_filter("音频和视频文件", &["wav", "mp3", "m4a", "mp4", "mov"])
            .blocking_pick_file()
            .and_then(|value| value.into_path().ok());
        let Some(path) = selected else {
            log::info!(
                target: "app.meetings",
                "meeting_playback_cancelled meeting_id={}",
                meeting_id,
            );
            return Ok(PlaybackResult {
                status: "cancelled",
                rebound_source: false,
                source_path: None,
            });
        };
        validate_playback_media(&path)?;
        state
            .repository
            .set_meeting_source_path(&meeting_id, &path.to_string_lossy())?;
        (path, true)
    };
    validate_playback_media(&media_path)?;
    app.asset_protocol_scope()
        .allow_file(&media_path)
        .map_err(|_| {
            CommandError::new(
                "playback_authorization_failed",
                "无法读取原始媒体文件",
                true,
            )
        })?;
    log::info!(
        target: "app.meetings",
        "meeting_playback_ready meeting_id={} rebound_source={}",
        meeting_id,
        rebound_source,
    );
    Ok(PlaybackResult {
        status: "ready",
        rebound_source,
        source_path: Some(media_path.to_string_lossy().into_owned()),
    })
}

/// 校验重新关联的媒体文件存在且扩展名属于应用支持范围。
fn validate_playback_media(path: &Path) -> Result<(), CommandError> {
    if !path.is_file() {
        return Err(CommandError::new(
            "playback_file_missing",
            "原始媒体文件不存在，请重新选择",
            false,
        ));
    }
    let supported = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "wav" | "mp3" | "m4a" | "mp4" | "mov"
            )
        });
    if !supported {
        return Err(CommandError::new(
            "playback_format_unsupported",
            "请选择 WAV、MP3、M4A、MP4 或 MOV 文件",
            false,
        ));
    }
    Ok(())
}

/// 从恢复任务所需的现有生命周期快照即时计算指定会议的总处理耗时。
fn processing_duration_for_meeting(tasks: &[TaskRecord], meeting_id: &str) -> Option<u64> {
    tasks
        .iter()
        .find(|task| {
            task.status == TaskStatus::Completed && task.meeting_id.as_deref() == Some(meeting_id)
        })
        .and_then(task_processing_duration_ms)
}

/// 优先使用累计处理时间；旧任务再回退到创建到完成的墙钟差值。
fn task_processing_duration_ms(task: &TaskRecord) -> Option<u64> {
    if let Some(duration_ms) = task.processing_duration_ms {
        return Some(duration_ms);
    }
    let created_at = chrono::DateTime::parse_from_rfc3339(&task.created_at).ok()?;
    let updated_at = chrono::DateTime::parse_from_rfc3339(&task.updated_at).ok()?;
    updated_at
        .signed_duration_since(created_at)
        .num_milliseconds()
        .try_into()
        .ok()
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
    let markdown = render_export_markdown(&minutes, &detail.transcript);
    log::info!(
        target: "app.meetings",
        "meeting_preview_rendered meeting_id={} output_bytes={}",
        meeting_id,
        markdown.len(),
    );
    Ok(markdown)
}

/// 删除本地会议记录；永远不会删除用户选择的原始音频。
#[tauri::command]
pub fn delete_meeting(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<bool, CommandError> {
    let _task_gate = state
        .task_gate
        .lock()
        .map_err(|_| CommandError::new("task_state_unavailable", "任务状态不可用", true))?;
    crate::commands::tasks::ensure_related_tasks_deletable(&state.repository, &meeting_id)?;
    let outcome = state
        .repository
        .delete_meeting_with_related_tasks(&meeting_id)?;
    let artifact_count = outcome.artifact_ids.len();
    for artifact_id in outcome.artifact_ids {
        crate::commands::tasks::cleanup_unused_artifact(&state, &artifact_id);
    }
    log::info!(
        target: "app.meetings",
        "meeting_delete_completed meeting_id={} deleted={} artifact_cleanup_count={}",
        meeting_id,
        outcome.deleted,
        artifact_count,
    );
    Ok(outcome.deleted)
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
    let operation_started = Instant::now();
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
        log::info!(
            target: "app.meetings",
            "meeting_export_cancelled meeting_id={} elapsed_ms={}",
            meeting_id,
            operation_started.elapsed().as_millis(),
        );
        return Ok(ExportResult {
            status: "cancelled",
            display_name: None,
        });
    };
    ensure_markdown_extension(&path)?;
    let markdown = render_export_markdown(&minutes, &detail.transcript);
    let output_bytes = markdown.len();
    tauri::async_runtime::spawn_blocking(move || write_markdown_atomically(&path, &markdown))
        .await
        .map_err(|_| CommandError::new("export_failed", "Markdown 导出未完成", true))?
        .map_err(|_| CommandError::new("export_failed", "无法写入所选文件", true))?;
    log::info!(
        target: "app.meetings",
        "meeting_export_completed meeting_id={} output_bytes={} elapsed_ms={}",
        meeting_id,
        output_bytes,
        operation_started.elapsed().as_millis(),
    );
    Ok(ExportResult {
        status: "exported",
        display_name: Some(display_name),
    })
}

/// 让用户选择目标位置，并按请求组合导出 Word 或 PDF 文档。
#[tauri::command]
pub async fn export_meeting_document_command(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
    request: ExportMeetingRequest,
) -> Result<ExportResult, CommandError> {
    let operation_started = Instant::now();
    if request.contents.is_empty() {
        return Err(CommandError::new(
            "export_contents_empty",
            "请至少选择一项导出内容",
            false,
        ));
    }
    let detail = state
        .repository
        .get_meeting(&meeting_id)?
        .ok_or_else(|| CommandError::new("meeting_not_found", "未找到该会议记录", false))?;
    let minutes: MeetingMinutes = serde_json::from_value(detail.minutes)
        .map_err(|_| CommandError::new("minutes_invalid", "会议纪要格式无效", false))?;
    let extension = request.format.extension();
    let display_name = format!("{}.{}", safe_file_stem(minutes.title.as_deref()), extension);
    let format_label = match request.format {
        ExportFormat::Docx => "Word 文档",
        ExportFormat::Pdf => "PDF 文档",
    };
    let selected = app
        .dialog()
        .file()
        .add_filter(format_label, &[extension])
        .set_file_name(&display_name)
        .blocking_save_file();
    let Some(path) = selected.and_then(|value| value.into_path().ok()) else {
        log::info!(
            target: "app.meetings",
            "meeting_document_export_cancelled meeting_id={} format={} elapsed_ms={}",
            meeting_id,
            extension,
            operation_started.elapsed().as_millis(),
        );
        return Ok(ExportResult {
            status: "cancelled",
            display_name: None,
        });
    };
    ensure_export_extension(&path, request.format)?;
    let content_count = request.contents.len();
    let contents = request.contents;
    let transcript = normalize_transcript(&detail.transcript, detail.transcript_segments);
    tauri::async_runtime::spawn_blocking(move || {
        export_meeting_document(&path, request.format, &contents, &minutes, &transcript)
    })
    .await
    .map_err(|_| CommandError::new("export_failed", "文档导出未完成", true))??;
    log::info!(
        target: "app.meetings",
        "meeting_document_export_completed meeting_id={} format={} content_count={} elapsed_ms={}",
        meeting_id,
        extension,
        content_count,
        operation_started.elapsed().as_millis(),
    );
    Ok(ExportResult {
        status: "exported",
        display_name: Some(display_name),
    })
}

/// Writes and flushes in the destination directory before atomically replacing the target.
fn write_markdown_atomically(path: &Path, markdown: &str) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "missing destination directory",
        )
    })?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(markdown.as_bytes())?;
    staged.as_file().sync_all()?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
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
        .unwrap_or("录音整理")
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
        "录音整理".to_string()
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

/// 校验用户保存路径与所选文档格式一致。
fn ensure_export_extension(path: &Path, format: ExportFormat) -> Result<(), CommandError> {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(format.extension()))
    {
        Ok(())
    } else {
        Err(CommandError::new(
            "export_extension_invalid",
            format!("导出文件必须使用 .{} 扩展名", format.extension()),
            false,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::TaskAction;
    use crate::minutes::{ContentType, MeetingTime, TitleSource, MEETING_MINUTES_SCHEMA_VERSION};

    use super::*;

    /// 构造最小有效纪要用于导出测试。
    fn fixture_minutes() -> MeetingMinutes {
        MeetingMinutes {
            schema_version: MEETING_MINUTES_SCHEMA_VERSION.to_string(),
            content_type: ContentType::Meeting,
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
        assert_eq!(safe_file_stem(None), "录音整理");
    }

    /// 验证试听重新关联仅接受存在且受支持的本地媒体文件。
    #[test]
    fn validates_rebound_playback_media() {
        let directory = tempfile::TempDir::new().expect("create playback directory");
        let supported = directory.path().join("sample.mp3");
        let unsupported = directory.path().join("sample.txt");
        std::fs::write(&supported, b"test audio placeholder").expect("write supported fixture");
        std::fs::write(&unsupported, b"not media").expect("write unsupported fixture");

        assert!(validate_playback_media(&supported).is_ok());
        assert_eq!(
            validate_playback_media(&unsupported)
                .expect_err("reject unsupported media")
                .code,
            "playback_format_unsupported"
        );
        assert_eq!(
            validate_playback_media(&directory.path().join("missing.wav"))
                .expect_err("reject missing media")
                .code,
            "playback_file_missing"
        );
    }

    /// 验证历史耗时只从匹配会议的已完成任务即时派生，不写入会议记录。
    #[test]
    fn derives_processing_duration_from_completed_task() {
        let task = TaskRecord {
            id: "task-1".to_string(),
            artifact_id: "artifact-1".to_string(),
            batch_id: None,
            meeting_id: Some("meeting-1".to_string()),
            display_name: "sample.wav".to_string(),
            template_id: "standard_meeting".to_string(),
            status: TaskStatus::Completed,
            progress: Some(1.0),
            attempt: 1,
            max_attempts: 3,
            error: None,
            created_at: "2026-07-21T01:00:00Z".to_string(),
            updated_at: "2026-07-21T01:02:03Z".to_string(),
            processing_started_at: None,
            processing_duration_ms: Some(42_000),
            source_duration_ms: Some(60_000),
            estimated_processing_ms: Some(42_000),
            available_actions: vec![TaskAction::OpenMeeting],
        };

        assert_eq!(
            processing_duration_for_meeting(std::slice::from_ref(&task), "meeting-1"),
            Some(42_000)
        );
        let mut legacy_task = task.clone();
        legacy_task.processing_duration_ms = None;
        assert_eq!(
            processing_duration_for_meeting(std::slice::from_ref(&legacy_task), "meeting-1"),
            Some(123_000)
        );
        assert_eq!(
            processing_duration_for_meeting(&[task], "other-meeting"),
            None
        );
    }

    /// 验证会议分页返回搜索后的总数，并对页码与每页数量设置安全边界。
    #[test]
    fn paginates_meeting_search_results_with_bounded_parameters() {
        let summaries = (0..5)
            .map(|index| MeetingSummary {
                id: format!("meeting-{index}"),
                title: Some(format!("会议 {index}")),
                summary: None,
                meeting_start_at: None,
                duration_ms: None,
                processing_duration_ms: None,
                updated_at: "2026-08-14T00:00:00Z".to_string(),
                template_name: "标准会议".to_string(),
            })
            .collect::<Vec<_>>();

        let page = paginate_meetings(summaries, 2, 2).expect("paginate meetings");
        assert_eq!(page.total, 5);
        assert_eq!(page.page, 2);
        assert_eq!(page.page_size, 2);
        assert_eq!(page.total_pages, 3);
        assert_eq!(page.items.len(), 2);

        let empty_page = paginate_meetings(Vec::new(), 8, 20).expect("clamp empty meeting page");
        assert_eq!(empty_page.page, 1);
        assert_eq!(empty_page.total_pages, 1);

        let invalid = paginate_meetings(Vec::new(), 1, 0).expect_err("reject invalid page size");
        assert_eq!(invalid.code, "pagination_invalid");
    }

    /// 验证原子导出能够完整替换已有文件，不会留下部分 Markdown。
    #[test]
    fn atomically_replaces_existing_markdown() {
        let directory = tempfile::TempDir::new().expect("create export directory");
        let target = directory.path().join("minutes.md");
        std::fs::write(&target, "old content").expect("write old export");

        write_markdown_atomically(&target, "new complete content").expect("replace export");

        assert_eq!(
            std::fs::read_to_string(target).expect("read export"),
            "new complete content"
        );
    }
}
