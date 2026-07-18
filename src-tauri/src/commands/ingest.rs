use std::collections::HashSet;
use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::app_state::{AppState, RegisteredArtifact};
use crate::commands::CommandError;
use crate::domain::PublicSettings;
use crate::ingest::{
    ImportBatchResponse, ImportItemStatus, ImportRequest, ImportSelectionMode, OfflineAudioImporter,
};

/// 表示前端文件复核列表所需的非敏感导入结果。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidate {
    pub id: String,
    pub artifact_id: Option<String>,
    pub display_name: String,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub duration_ms: Option<u64>,
    pub validation_status: &'static str,
    pub safe_message: Option<String>,
}

/// 按用户选择的单文件或批量模式打开系统对话框，并导入受管目录。
#[tauri::command]
pub async fn select_audio_files(
    app: AppHandle,
    state: State<'_, AppState>,
    selection_mode: ImportSelectionMode,
) -> Result<Vec<ImportCandidate>, CommandError> {
    let settings = crate::commands::settings::load_evaluated_settings(state.inner())?;
    ensure_audio_selection_ready(&settings)?;
    let paths = match selection_mode {
        ImportSelectionMode::Single => app
            .dialog()
            .file()
            .add_filter("音频和视频文件", &["wav", "mp3", "m4a", "mp4", "mov"])
            .blocking_pick_file()
            .and_then(|path| path.into_path().ok())
            .into_iter()
            .collect::<Vec<_>>(),
        ImportSelectionMode::Batch => app
            .dialog()
            .file()
            .add_filter("音频和视频文件", &["wav", "mp3", "m4a", "mp4", "mov"])
            .blocking_pick_files()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|path| path.into_path().ok())
            .collect::<Vec<_>>(),
    };
    let mut candidates = import_paths(
        state.importer.clone(),
        state.artifacts.clone(),
        state.import_gate.clone(),
        paths,
        selection_mode,
    )
    .await?;
    let active_artifacts = state
        .repository
        .list_tasks()?
        .into_iter()
        .filter(|task| task.retains_audio_artifact())
        .map(|task| task.artifact_id)
        .collect::<HashSet<_>>();
    mark_active_artifacts_unavailable(&mut candidates, &active_artifacts);
    Ok(candidates)
}

/// 仅在转写和纪要服务均完整配置时允许打开文件选择器。
fn ensure_audio_selection_ready(settings: &PublicSettings) -> Result<(), CommandError> {
    if settings.transcription.ready && settings.minutes.ready {
        Ok(())
    } else {
        Err(CommandError::new(
            "provider_configuration_required",
            "请先完成语音转写和会议纪要服务配置，再选择音频",
            false,
        ))
    }
}

/// 删除尚未处理的受管暂存副本；不会删除用户原始文件。
#[tauri::command]
pub fn release_audio_artifact(
    state: State<'_, AppState>,
    artifact_id: String,
) -> Result<bool, CommandError> {
    let _task_gate = state
        .task_gate
        .lock()
        .map_err(|_| CommandError::new("task_state_unavailable", "任务状态不可用", true))?;
    let is_active = state
        .repository
        .list_tasks()?
        .into_iter()
        .any(|task| task.artifact_id == artifact_id && task.retains_audio_artifact());
    if is_active {
        return Ok(false);
    }
    let removed = state
        .importer
        .remove_artifact(&artifact_id)
        .map_err(|_| CommandError::new("artifact_cleanup_failed", "无法清理音频暂存副本", true))?;
    state
        .artifacts
        .lock()
        .map_err(|_| {
            CommandError::new("artifact_registry_unavailable", "音频导入状态不可用", true)
        })?
        .remove(&artifact_id);
    Ok(removed)
}

/// 将已被活动任务占用的重复导入标记为不可提交，避免共享暂存文件生命周期。
fn mark_active_artifacts_unavailable(
    candidates: &mut [ImportCandidate],
    active_artifacts: &HashSet<String>,
) {
    for candidate in candidates {
        let is_active = candidate
            .artifact_id
            .as_ref()
            .is_some_and(|artifact_id| active_artifacts.contains(artifact_id));
        if is_active {
            candidate.artifact_id = None;
            candidate.validation_status = "invalid";
            candidate.safe_message = Some("该音频已有进行中的任务".to_string());
        }
    }
}

/// 在阻塞线程池执行文件复制、容器检查和流式哈希。
pub(crate) async fn import_paths(
    importer: std::sync::Arc<OfflineAudioImporter>,
    registry: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<String, RegisteredArtifact>>,
    >,
    import_gate: std::sync::Arc<std::sync::Mutex<()>>,
    paths: Vec<PathBuf>,
    selection_mode: ImportSelectionMode,
) -> Result<Vec<ImportCandidate>, CommandError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let display_names = paths
        .iter()
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("未命名音频")
                .to_string()
        })
        .collect::<Vec<_>>();
    tauri::async_runtime::spawn_blocking(move || {
        let _import_guard = import_gate
            .lock()
            .map_err(|_| CommandError::new("ingest_failed", "音频导入状态不可用", true))?;
        {
            let artifacts = registry
                .lock()
                .map_err(|_| CommandError::new("ingest_failed", "音频导入状态不可用", true))?;
            validate_staging_capacity(&artifacts, &paths, importer.policy())?;
        }
        let response = importer.import_selected_files(ImportRequest { selection_mode }, paths);
        let candidates = map_candidates(response, display_names);
        let mut artifacts = registry
            .lock()
            .map_err(|_| CommandError::new("ingest_failed", "音频导入状态不可用", true))?;
        for candidate in &candidates {
            if let (Some(id), Some(mime_type), Some(byte_length)) = (
                candidate.artifact_id.as_ref(),
                candidate.mime_type.as_ref(),
                candidate.size_bytes,
            ) {
                artifacts.insert(
                    id.clone(),
                    RegisteredArtifact {
                        id: id.clone(),
                        display_name: candidate.display_name.clone(),
                        mime_type: mime_type.clone(),
                        byte_length,
                        duration_ms: candidate.duration_ms,
                    },
                );
            }
        }
        Ok(candidates)
    })
    .await
    .map_err(|_| CommandError::new("ingest_failed", "音频导入任务未完成", true))?
}

/// 在复制前复核当前暂存量与新选择的累计资源上限。
fn validate_staging_capacity(
    registry: &std::collections::HashMap<String, RegisteredArtifact>,
    paths: &[PathBuf],
    policy: crate::ingest::IngestPolicy,
) -> Result<(), CommandError> {
    let item_count = registry
        .len()
        .checked_add(paths.len())
        .ok_or_else(|| CommandError::new("batch_limit_exceeded", "暂存音频数量超过限制", false))?;
    if item_count > policy.max_batch_items {
        return Err(CommandError::new(
            "batch_limit_exceeded",
            "暂存音频数量超过限制，请先提交或清空当前列表",
            false,
        ));
    }
    let mut total_bytes = registry.values().try_fold(0u64, |total, artifact| {
        total.checked_add(artifact.byte_length).ok_or_else(|| {
            CommandError::new("batch_limit_exceeded", "暂存音频总大小超过限制", false)
        })
    })?;
    for path in paths {
        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        total_bytes = total_bytes.checked_add(metadata.len()).ok_or_else(|| {
            CommandError::new("batch_limit_exceeded", "暂存音频总大小超过限制", false)
        })?;
        if total_bytes > policy.max_batch_total_bytes {
            return Err(CommandError::new(
                "batch_limit_exceeded",
                "暂存音频总大小超过限制，请先提交或清空当前列表",
                false,
            ));
        }
    }
    Ok(())
}

/// 把后端导入结果映射为不含路径和哈希的前端候选项。
fn map_candidates(
    response: ImportBatchResponse,
    display_names: Vec<String>,
) -> Vec<ImportCandidate> {
    response
        .items
        .into_iter()
        .map(|item| {
            let artifact = item.artifact;
            let is_duplicate = item.status == ImportItemStatus::Duplicate;
            let id = if is_duplicate {
                format!("{}-{}", response.batch_id, item.selection_index)
            } else {
                artifact
                    .as_ref()
                    .map(|value| value.id.clone())
                    .unwrap_or_else(|| format!("{}-{}", response.batch_id, item.selection_index))
            };
            let status = match item.status {
                ImportItemStatus::Ready => "ready",
                ImportItemStatus::Duplicate | ImportItemStatus::Failed => "invalid",
            };
            ImportCandidate {
                id,
                artifact_id: if is_duplicate {
                    None
                } else {
                    artifact.as_ref().map(|value| value.id.clone())
                },
                display_name: display_names
                    .get(item.selection_index)
                    .cloned()
                    .unwrap_or_else(|| "未命名音频".to_string()),
                mime_type: artifact
                    .as_ref()
                    .map(|value| value.staging_metadata.mime_type.clone()),
                size_bytes: artifact
                    .as_ref()
                    .map(|value| value.staging_metadata.byte_length),
                duration_ms: artifact
                    .as_ref()
                    .and_then(|value| value.staging_metadata.duration_ms),
                validation_status: status,
                safe_message: if is_duplicate {
                    Some("该音频已在待处理列表或任务中".to_string())
                } else {
                    item.error
                        .map(|error| localize_ingest_error(&error.safe_message_key))
                },
            }
        })
        .collect()
}

/// 将稳定错误键转换为简短中文提示，不暴露路径和解析细节。
fn localize_ingest_error(key: &str) -> String {
    match key {
        "ingest.error.empty_audio" => "媒体文件为空".to_string(),
        "ingest.error.file_too_large" => "媒体文件超过大小限制".to_string(),
        "ingest.error.unsupported_extension" => "仅支持 WAV、MP3、M4A、MP4 和 MOV".to_string(),
        "ingest.error.extension_content_mismatch" => "文件扩展名与实际媒体格式不一致".to_string(),
        "ingest.error.corrupt_audio" => "媒体文件已损坏或结构不完整".to_string(),
        "ingest.error.missing_audio_track" => "视频中没有可转写的音轨".to_string(),
        "ingest.error.batch_limit_exceeded" => "批量文件数量或总大小超过限制".to_string(),
        "ingest.error.source_changed" => "导入期间源文件发生变化，请重新选择".to_string(),
        _ => "媒体文件无法导入，请检查文件后重试".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::Utc;
    use tempfile::TempDir;

    use super::*;

    /// 构造后端门禁测试使用的已就绪公开设置。
    fn ready_settings() -> PublicSettings {
        let mut settings = crate::config::provider_settings_from_environment();
        settings.transcription.secret_configured = true;
        settings.transcription.ready = true;
        settings.minutes.secret_configured = true;
        settings.minutes.ready = true;
        settings
    }

    /// 验证任一服务未配置时不会打开系统文件选择器。
    #[test]
    fn blocks_audio_selection_until_both_providers_are_ready() {
        let mut settings = ready_settings();
        settings.minutes.ready = false;
        let error = ensure_audio_selection_ready(&settings).expect_err("block incomplete settings");
        assert_eq!(error.code, "provider_configuration_required");

        settings.minutes.ready = true;
        assert!(ensure_audio_selection_ready(&settings).is_ok());
    }

    /// 验证与活动任务共用 ID 的重复候选不会再暴露可释放 artifact。
    #[test]
    fn active_duplicate_candidate_is_not_submittable_or_releasable() {
        let mut candidates = vec![ImportCandidate {
            id: "artifact-active".to_string(),
            artifact_id: Some("artifact-active".to_string()),
            display_name: "重复音频.wav".to_string(),
            mime_type: Some("audio/wav".to_string()),
            size_bytes: Some(128),
            duration_ms: None,
            validation_status: "ready",
            safe_message: None,
        }];
        let active = HashSet::from(["artifact-active".to_string()]);

        mark_active_artifacts_unavailable(&mut candidates, &active);

        assert_eq!(candidates[0].validation_status, "invalid");
        assert!(candidates[0].artifact_id.is_none());
        assert_eq!(
            candidates[0].safe_message.as_deref(),
            Some("该音频已有进行中的任务")
        );
    }

    /// 验证导入层识别的重复音频保留独立行，但不会复用可释放 artifact ID。
    #[test]
    fn duplicate_import_maps_to_independent_invalid_candidate() {
        let artifact = crate::ingest::AudioArtifactRef {
            id: "artifact-existing".to_string(),
            import_batch_id: Some("batch-original".to_string()),
            source_kind: crate::ingest::AudioSourceKind::UserSelectedFile,
            staging_metadata: crate::ingest::StagingMetadata {
                mime_type: "audio/wav".to_string(),
                byte_length: 128,
                duration_ms: None,
                sha256: None,
                validated_at: Utc::now(),
            },
        };
        let response = ImportBatchResponse {
            batch_id: "batch-new".to_string(),
            items: vec![crate::ingest::ImportItemResult {
                selection_index: 0,
                status: ImportItemStatus::Duplicate,
                artifact: Some(artifact),
                duplicate_of_artifact_id: Some("artifact-existing".to_string()),
                error: None,
            }],
        };

        let candidates = map_candidates(response, vec!["重复音频.wav".to_string()]);

        assert_eq!(candidates[0].id, "batch-new-0");
        assert_eq!(candidates[0].validation_status, "invalid");
        assert!(candidates[0].artifact_id.is_none());
        assert_eq!(
            candidates[0].safe_message.as_deref(),
            Some("该音频已在待处理列表或任务中")
        );
    }

    /// 验证多次追加在复制前按当前 staging 的累计数量与字节数拒绝超限。
    #[test]
    fn rejects_staging_growth_across_multiple_selections() {
        let temp = TempDir::new().expect("create tempdir");
        let next_path = temp.path().join("next.wav");
        fs::write(&next_path, vec![0u8; 60]).expect("write prospective file");
        let artifact = RegisteredArtifact {
            id: "existing".to_string(),
            display_name: "existing.wav".to_string(),
            mime_type: "audio/wav".to_string(),
            byte_length: 60,
            duration_ms: None,
        };
        let registry = std::collections::HashMap::from([("existing".to_string(), artifact)]);
        let count_policy = crate::ingest::IngestPolicy::new(100, 1, 200).expect("count policy");
        let count_error =
            validate_staging_capacity(&registry, std::slice::from_ref(&next_path), count_policy)
                .expect_err("reject cumulative staging count");
        assert!(count_error.safe_message.contains("数量"));

        let policy = crate::ingest::IngestPolicy::new(100, 3, 100).expect("create policy");

        let error = validate_staging_capacity(&registry, &[next_path], policy)
            .expect_err("reject cumulative staging bytes");

        assert_eq!(error.code, "batch_limit_exceeded");
    }
}
