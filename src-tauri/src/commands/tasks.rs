use std::sync::Arc;
use std::time::Duration;
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use chrono::Utc;
use serde::Deserialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use crate::app_state::{AppState, RegisteredArtifact};
use crate::commands::CommandError;
use crate::domain::{PersistedMeetingInput, SafeTaskError, TaskAction, TaskRecord, TaskStatus};
use crate::ingest::{AudioArtifactRef, AudioSourceKind, IngestPolicy, StagingMetadata};
use crate::minutes::{
    build_prompt, validate_provider_candidate, MeetingContext, PromptBuildRequest,
    ValidationOptions, BUILTIN_TEMPLATE_VERSION, MEETING_MINUTES_SCHEMA_VERSION,
};
use crate::providers::{
    build_minutes_provider, build_transcription_provider, CancellationToken, ManagedAudioArtifact,
    MinutesProvider, ProviderCallContext, ProviderCredential, ProviderError, TranscriptionOptions,
    TranscriptionProvider, TranscriptionRequest,
};
use crate::secrets::{self, SecretKind};
use crate::storage::MeetingRepository;

/// 表示任务列表的前端筛选条件。
#[derive(Debug, Deserialize)]
pub struct TaskQuery {
    pub filter: String,
}

/// Holds the provider-neutral adapters and credentials used by one processing attempt.
#[derive(Clone)]
struct ProcessingProviders {
    transcription: Arc<dyn TranscriptionProvider>,
    minutes: Arc<dyn MinutesProvider>,
    transcription_credential: Option<Arc<ProviderCredential>>,
    minutes_credential: Option<Arc<ProviderCredential>>,
    transcription_timeout_ms: u64,
    minutes_timeout_ms: u64,
    max_attempts: u32,
}

/// Builds a production runtime only from backend-evaluated settings and scoped credentials.
fn load_processing_providers(state: &AppState) -> Result<ProcessingProviders, CommandError> {
    let settings = crate::commands::settings::load_evaluated_settings(state)?;
    if !settings.transcription.ready || !settings.minutes.ready {
        return Err(CommandError::new(
            "provider_not_configured",
            "请先完成语音转写和会议纪要服务配置",
            false,
        ));
    }
    let transcription = build_transcription_provider(&settings.transcription)
        .map_err(|error| provider_configuration_error(error.safe_message))?;
    let minutes = build_minutes_provider(&settings.minutes)
        .map_err(|error| provider_configuration_error(error.safe_message))?;
    let transcription_credential =
        if settings.transcription.preset_id == crate::config::PRESET_LOCAL_FUNASR {
            None
        } else {
            load_provider_credential(
                SecretKind::Transcription,
                settings.transcription.credential_preset_id.as_deref(),
            )?
        };
    let minutes_credential = load_provider_credential(
        SecretKind::Minutes,
        settings.minutes.credential_preset_id.as_deref(),
    )?;
    Ok(ProcessingProviders {
        transcription,
        minutes,
        transcription_credential,
        minutes_credential,
        transcription_timeout_ms: settings.transcription.request_timeout_ms,
        minutes_timeout_ms: settings.minutes.request_timeout_ms,
        max_attempts: settings
            .transcription
            .max_retries
            .max(settings.minutes.max_retries)
            .saturating_add(1),
    })
}

/// Reads one exact provider binding without exposing the credential to IPC or logs.
fn load_provider_credential(
    kind: SecretKind,
    binding_id: Option<&str>,
) -> Result<Option<Arc<ProviderCredential>>, CommandError> {
    let binding_id = binding_id.ok_or_else(|| {
        CommandError::new(
            "provider_not_configured",
            "Provider 凭据绑定无效，请重新保存服务设置",
            false,
        )
    })?;
    let secret = secrets::read_secret_for_binding(kind, binding_id).map_err(|_| {
        CommandError::new(
            "credential_store_error",
            "无法访问 Windows 凭据管理器",
            true,
        )
    })?;
    let secret = secret
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            CommandError::new(
                "provider_not_configured",
                "Provider API Key 不存在，请重新保存服务设置",
                false,
            )
        })?;
    Ok(Some(Arc::new(ProviderCredential::new(secret))))
}

fn provider_configuration_error(safe_message: String) -> CommandError {
    CommandError::new("provider_configuration_invalid", safe_message, false)
}

/// 按导入策略复核整个逻辑批次，并一次性解析全部受管音频引用。
fn resolve_batch_artifacts(
    artifact_ids: &[String],
    registry: &HashMap<String, RegisteredArtifact>,
    policy: IngestPolicy,
) -> Result<Vec<RegisteredArtifact>, CommandError> {
    if artifact_ids.len() > policy.max_batch_items {
        return Err(CommandError::new(
            "batch_limit_exceeded",
            "批量文件数量超过限制，请拆分为多个批次",
            false,
        ));
    }
    let mut total_bytes = 0u64;
    let mut artifacts = Vec::with_capacity(artifact_ids.len());
    for artifact_id in artifact_ids {
        let artifact = registry.get(artifact_id).cloned().ok_or_else(|| {
            CommandError::new(
                "artifact_not_found",
                "音频暂存记录不存在，请重新选择文件",
                false,
            )
        })?;
        total_bytes = total_bytes
            .checked_add(artifact.byte_length)
            .ok_or_else(|| {
                CommandError::new(
                    "batch_limit_exceeded",
                    "批量文件总大小超过限制，请拆分为多个批次",
                    false,
                )
            })?;
        if total_bytes > policy.max_batch_total_bytes {
            return Err(CommandError::new(
                "batch_limit_exceeded",
                "批量文件总大小超过限制，请拆分为多个批次",
                false,
            ));
        }
        artifacts.push(artifact);
    }
    Ok(artifacts)
}

/// 为每个已导入 artifact 创建独立任务，并启动后台 Provider 流程。
#[tauri::command]
pub fn create_processing_tasks(
    state: State<'_, AppState>,
    artifact_ids: Vec<String>,
    template_id: String,
) -> Result<Vec<TaskRecord>, CommandError> {
    let _task_gate = state
        .task_gate
        .lock()
        .map_err(|_| CommandError::new("task_state_unavailable", "任务状态不可用", true))?;
    if artifact_ids.is_empty() {
        return Err(CommandError::new(
            "no_audio_selected",
            "请至少选择一个已通过校验的音频文件",
            false,
        ));
    }
    crate::minutes::get_template(&template_id, BUILTIN_TEMPLATE_VERSION)
        .map_err(|error| CommandError::new(error.code(), error.to_string(), false))?;
    let unique_ids = artifact_ids.iter().collect::<HashSet<_>>();
    if unique_ids.len() != artifact_ids.len() {
        return Err(CommandError::new(
            "duplicate_artifact_submission",
            "同一音频不能在一个批次中重复提交",
            false,
        ));
    }
    let active_artifacts = state
        .repository
        .list_tasks()?
        .into_iter()
        .filter(|task| task.retains_audio_artifact())
        .map(|task| task.artifact_id)
        .collect::<HashSet<_>>();
    if artifact_ids.iter().any(|id| active_artifacts.contains(id)) {
        return Err(CommandError::new(
            "artifact_already_processing",
            "所选音频已有进行中的任务",
            false,
        ));
    }
    let providers = load_processing_providers(state.inner())?;
    let registry = state.artifacts.lock().map_err(|_| {
        CommandError::new("artifact_registry_unavailable", "音频导入状态不可用", true)
    })?;
    let artifacts = resolve_batch_artifacts(&artifact_ids, &registry, state.importer.policy())?;
    drop(registry);
    let batch_id = (artifacts.len() > 1).then(|| Uuid::new_v4().to_string());
    let tasks = artifacts
        .into_iter()
        .map(|artifact| {
            let task = new_task(
                &artifact,
                batch_id.clone(),
                &template_id,
                providers.max_attempts,
            );
            (task, artifact)
        })
        .collect::<Vec<_>>();
    start_new_tasks(&state, tasks, providers)
}

/// 返回按更新时间倒序排列且符合筛选条件的任务。
#[tauri::command]
pub fn list_processing_tasks(
    state: State<'_, AppState>,
    query: TaskQuery,
) -> Result<Vec<TaskRecord>, CommandError> {
    let tasks = state.repository.list_tasks()?;
    Ok(tasks
        .into_iter()
        .filter(|task| task_matches_filter(task, &query.filter))
        .map(with_available_delete_action)
        .collect())
}

/// 请求取消正在运行的任务，并立即返回可见的 cancel_requested 状态。
#[tauri::command]
pub fn cancel_processing_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<TaskRecord, CommandError> {
    let _task_gate = state
        .task_gate
        .lock()
        .map_err(|_| CommandError::new("task_state_unavailable", "任务状态不可用", true))?;
    let mut task = find_task(&state.repository, &task_id)?;
    if task.status.is_terminal() {
        return Ok(task);
    }
    if task.status == TaskStatus::Saving {
        return Err(CommandError::new(
            "task_not_cancellable",
            "任务正在原子保存结果，当前不能取消",
            false,
        ));
    }
    let token = state
        .cancellations
        .lock()
        .map_err(|_| CommandError::new("task_state_unavailable", "任务状态不可用", true))?
        .get(&task_id)
        .cloned();
    if let Some(token) = token {
        task.status = TaskStatus::CancelRequested;
        task.updated_at = Utc::now().to_rfc3339();
        task.available_actions.clear();
        state.repository.save_task(&task)?;
        token.cancel();
    } else {
        mark_cancelled(&mut task);
        state.repository.save_task(&task)?;
    }
    Ok(task)
}

/// 重新启动仍有受管 artifact 的失败或中断任务。
#[tauri::command]
pub fn retry_processing_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<TaskRecord, CommandError> {
    let _task_gate = state
        .task_gate
        .lock()
        .map_err(|_| CommandError::new("task_state_unavailable", "任务状态不可用", true))?;
    let mut task = find_task(&state.repository, &task_id)?;
    if !matches!(task.status, TaskStatus::Failed | TaskStatus::Interrupted) {
        return Err(CommandError::new(
            "task_not_retryable",
            "该任务当前不能重试",
            false,
        ));
    }
    if task
        .error
        .as_ref()
        .is_some_and(|error| error.code == "artifact_cleanup_failed")
    {
        state
            .importer
            .remove_artifact(&task.artifact_id)
            .map_err(|_| {
                CommandError::new(
                    "artifact_cleanup_failed",
                    "暂存副本仍被占用，请关闭占用程序后重试",
                    true,
                )
            })?;
        state
            .artifacts
            .lock()
            .map_err(|_| {
                CommandError::new("artifact_registry_unavailable", "音频导入状态不可用", true)
            })?
            .remove(&task.artifact_id);
        task.status = TaskStatus::Completed;
        task.progress = Some(1.0);
        task.error = None;
        task.available_actions = vec![TaskAction::OpenMeeting];
        task.updated_at = Utc::now().to_rfc3339();
        state.repository.save_task(&task)?;
        return Ok(task);
    }
    ensure_retry_available(&mut task, &state.repository)?;
    let artifact = state
        .artifacts
        .lock()
        .map_err(|_| {
            CommandError::new("artifact_registry_unavailable", "音频导入状态不可用", true)
        })?
        .get(&task.artifact_id)
        .cloned()
        .ok_or_else(|| {
            CommandError::new(
                "artifact_reselect_required",
                "应用重启后需重新选择原音频才能重试",
                false,
            )
        })?;
    let providers = load_processing_providers(state.inner())?;
    task.status = TaskStatus::Queued;
    task.attempt = task.attempt.saturating_add(1);
    task.progress = Some(0.0);
    task.error = None;
    task.available_actions = vec![TaskAction::Cancel];
    task.updated_at = Utc::now().to_rfc3339();
    state.repository.save_task(&task)?;
    start_task(&state, task.clone(), artifact, providers)?;
    Ok(task)
}

/// 删除失败或中断的任务，并尽力清理不再被引用的受管音频副本。
#[tauri::command]
pub fn delete_processing_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<bool, CommandError> {
    let _task_gate = state
        .task_gate
        .lock()
        .map_err(|_| CommandError::new("task_state_unavailable", "任务状态不可用", true))?;
    let artifact_id = delete_task_record(&state.repository, &task_id)?;
    if let Some(artifact_id) = artifact_id {
        if let Ok(mut cancellations) = state.cancellations.lock() {
            cancellations.remove(&task_id);
        }
        cleanup_unused_artifact(&state, &artifact_id);
        return Ok(true);
    }
    Ok(false)
}

/// 通过受信任系统文件对话框重新选择媒体，并把新 artifact 绑定到原中断任务。
#[tauri::command]
pub async fn reselect_processing_task(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<TaskRecord, CommandError> {
    {
        let _task_gate = state
            .task_gate
            .lock()
            .map_err(|_| CommandError::new("task_state_unavailable", "任务状态不可用", true))?;
        let mut task = find_task(&state.repository, &task_id)?;
        if task.status != TaskStatus::Interrupted
            || !task.available_actions.contains(&TaskAction::ReselectFile)
        {
            return Err(CommandError::new(
                "task_not_reselectable",
                "该任务当前不需要重新选择音频",
                false,
            ));
        }
        ensure_retry_available(&mut task, &state.repository)?;
    }

    let selected = app
        .dialog()
        .file()
        .add_filter("音频和视频文件", &["wav", "mp3", "m4a", "mp4", "mov"])
        .blocking_pick_file();
    let Some(path) = selected.and_then(|value| value.into_path().ok()) else {
        return find_task(&state.repository, &task_id);
    };
    let candidates = crate::commands::ingest::import_paths(
        state.importer.clone(),
        state.artifacts.clone(),
        state.import_gate.clone(),
        vec![path],
        crate::ingest::ImportSelectionMode::Single,
    )
    .await?;
    let candidate = candidates.into_iter().next().ok_or_else(|| {
        CommandError::new("audio_not_selected", "没有选择可处理的媒体文件", false)
    })?;
    let artifact_id = candidate.artifact_id.ok_or_else(|| {
        CommandError::new(
            "audio_validation_failed",
            candidate
                .safe_message
                .unwrap_or_else(|| "所选媒体未通过校验".to_string()),
            false,
        )
    })?;

    let result = restart_with_reselected_artifact(&state, &task_id, &artifact_id);
    if result.is_err() {
        cleanup_unused_artifact(&state, &artifact_id);
    }
    result
}

/// 将进程重启时遗留的活动任务标记为可重试的中断状态。
pub fn recover_interrupted_tasks(
    repository: &MeetingRepository,
) -> Result<(), crate::storage::StorageError> {
    for mut task in repository.list_tasks()? {
        if task.status == TaskStatus::CancelRequested {
            let last_active_at = task.updated_at.clone();
            finish_processing_attempt_at(&mut task, &last_active_at);
            mark_cancelled(&mut task);
            repository.save_task(&task)?;
        } else if !task.status.is_terminal() && task.status != TaskStatus::Interrupted {
            let last_active_at = task.updated_at.clone();
            finish_processing_attempt_at(&mut task, &last_active_at);
            task.status = TaskStatus::Interrupted;
            task.progress = None;
            if task.attempt < task.max_attempts {
                task.error = Some(SafeTaskError {
                    code: "app_restarted".to_string(),
                    retryable: true,
                    safe_message: "应用上次退出时任务尚未完成，请重新选择文件后重试".to_string(),
                    http_status: None,
                    retry_after_ms: None,
                });
                task.available_actions = vec![TaskAction::ReselectFile];
            } else {
                task.error = Some(retry_limit_error());
                task.available_actions.clear();
            }
            task.updated_at = Utc::now().to_rfc3339();
            repository.save_task(&task)?;
        }
    }
    Ok(())
}

/// 校验任务仍有剩余尝试次数；达到上限时持久化不可重试状态。
fn ensure_retry_available(
    task: &mut TaskRecord,
    repository: &MeetingRepository,
) -> Result<(), CommandError> {
    if task.attempt < task.max_attempts {
        return Ok(());
    }
    task.status = TaskStatus::Failed;
    task.progress = None;
    task.error = Some(retry_limit_error());
    task.available_actions.clear();
    task.updated_at = Utc::now().to_rfc3339();
    repository.save_task(task)?;
    Err(CommandError::new(
        "retry_limit_reached",
        "任务已达到最大尝试次数",
        false,
    ))
}

/// 使用新导入的受管 artifact 重置中断任务并启动下一次尝试。
fn restart_with_reselected_artifact(
    state: &State<'_, AppState>,
    task_id: &str,
    artifact_id: &str,
) -> Result<TaskRecord, CommandError> {
    let _task_gate = state
        .task_gate
        .lock()
        .map_err(|_| CommandError::new("task_state_unavailable", "任务状态不可用", true))?;
    let mut task = find_task(&state.repository, task_id)?;
    if task.status != TaskStatus::Interrupted
        || !task.available_actions.contains(&TaskAction::ReselectFile)
    {
        return Err(CommandError::new(
            "task_not_reselectable",
            "该任务当前不需要重新选择音频",
            false,
        ));
    }
    ensure_retry_available(&mut task, &state.repository)?;
    let artifact_in_use = state.repository.list_tasks()?.into_iter().any(|other| {
        other.id != task_id && other.artifact_id == artifact_id && other.retains_audio_artifact()
    });
    if artifact_in_use {
        return Err(CommandError::new(
            "artifact_already_processing",
            "重新选择的音频已有进行中的任务",
            false,
        ));
    }
    let artifact = state
        .artifacts
        .lock()
        .map_err(|_| {
            CommandError::new("artifact_registry_unavailable", "音频导入状态不可用", true)
        })?
        .get(artifact_id)
        .cloned()
        .ok_or_else(|| {
            CommandError::new("artifact_not_found", "重新选择的音频暂存记录不存在", false)
        })?;
    let providers = load_processing_providers(state.inner())?;
    task.artifact_id = artifact.id.clone();
    task.display_name = artifact.display_name.clone();
    task.status = TaskStatus::Queued;
    task.attempt = task.attempt.saturating_add(1);
    task.progress = Some(0.0);
    task.error = None;
    task.available_actions = vec![TaskAction::Cancel];
    task.updated_at = Utc::now().to_rfc3339();
    state.repository.save_task(&task)?;
    start_task(state, task.clone(), artifact, providers)?;
    Ok(task)
}

/// 在重新绑定失败时尽力清理刚导入且未被任务采用的暂存副本。
fn cleanup_unused_artifact(state: &State<'_, AppState>, artifact_id: &str) {
    let is_active = state.repository.list_tasks().ok().is_some_and(|tasks| {
        tasks
            .into_iter()
            .any(|task| task.artifact_id == artifact_id && task.retains_audio_artifact())
    });
    if is_active {
        return;
    }
    let _ = state.importer.remove_artifact(artifact_id);
    if let Ok(mut artifacts) = state.artifacts.lock() {
        artifacts.remove(artifact_id);
    }
}

/// 创建一条前端可直接展示的排队任务。
fn new_task(
    artifact: &RegisteredArtifact,
    batch_id: Option<String>,
    template_id: &str,
    max_attempts: u32,
) -> TaskRecord {
    let now = Utc::now().to_rfc3339();
    TaskRecord {
        id: Uuid::new_v4().to_string(),
        artifact_id: artifact.id.clone(),
        batch_id,
        meeting_id: None,
        display_name: artifact.display_name.clone(),
        template_id: template_id.to_string(),
        status: TaskStatus::Queued,
        progress: Some(0.0),
        attempt: 1,
        max_attempts: max_attempts.max(1),
        error: None,
        created_at: now.clone(),
        updated_at: now,
        processing_started_at: None,
        processing_duration_ms: Some(0),
        available_actions: vec![TaskAction::Cancel],
    }
}

/// 聚合后台任务共享依赖，保持并发边界和函数签名清晰。
#[derive(Clone)]
struct TaskRuntime {
    repository: Arc<MeetingRepository>,
    importer: Arc<crate::ingest::OfflineAudioImporter>,
    artifacts: Arc<Mutex<std::collections::HashMap<String, RegisteredArtifact>>>,
    cancellations: Arc<Mutex<std::collections::HashMap<String, CancellationToken>>>,
    task_gate: Arc<Mutex<()>>,
}

/// 原子登记并保存一组新任务，随后为每项启动独立 Provider 处理流程。
fn start_new_tasks(
    state: &State<'_, AppState>,
    tasks: Vec<(TaskRecord, RegisteredArtifact)>,
    providers: ProcessingProviders,
) -> Result<Vec<TaskRecord>, CommandError> {
    let visible_tasks = tasks
        .iter()
        .map(|(task, _)| task.clone())
        .collect::<Vec<_>>();
    let registrations = visible_tasks
        .iter()
        .map(|task| (task.id.clone(), CancellationToken::new()))
        .collect::<Vec<_>>();
    let mut active = state
        .cancellations
        .lock()
        .map_err(|_| CommandError::new("task_state_unavailable", "任务状态不可用", true))?;
    if registrations.iter().any(|(id, _)| active.contains_key(id)) {
        return Err(CommandError::new(
            "task_already_running",
            "批次中存在已经运行的任务",
            true,
        ));
    }
    for (id, token) in &registrations {
        active.insert(id.clone(), token.clone());
    }
    if let Err(error) = state.repository.save_tasks(&visible_tasks) {
        for (id, _) in &registrations {
            active.remove(id);
        }
        return Err(CommandError::from(error));
    }
    drop(active);
    for ((task, artifact), (_, token)) in tasks.into_iter().zip(registrations) {
        spawn_registered_task(state, task, artifact, token, providers.clone());
    }
    Ok(visible_tasks)
}

/// 启动已经登记取消令牌的单个后台 Provider 处理流程。
fn spawn_registered_task(
    state: &State<'_, AppState>,
    task: TaskRecord,
    artifact: RegisteredArtifact,
    token: CancellationToken,
    providers: ProcessingProviders,
) {
    let runtime = TaskRuntime {
        repository: state.repository.clone(),
        importer: state.importer.clone(),
        artifacts: state.artifacts.clone(),
        cancellations: state.cancellations.clone(),
        task_gate: state.task_gate.clone(),
    };
    tauri::async_runtime::spawn(async move {
        if run_task(runtime, providers, task, artifact, token)
            .await
            .is_err()
        {
            eprintln!("task_terminal_state_persist_failed");
        }
    });
}

/// 注册取消令牌并在 Tauri 异步运行时启动处理流程。
fn start_task(
    state: &State<'_, AppState>,
    task: TaskRecord,
    artifact: RegisteredArtifact,
    providers: ProcessingProviders,
) -> Result<(), CommandError> {
    let token = CancellationToken::new();
    let mut active = state
        .cancellations
        .lock()
        .map_err(|_| CommandError::new("task_state_unavailable", "任务状态不可用", true))?;
    if active.contains_key(&task.id) {
        return Err(CommandError::new(
            "task_already_running",
            "该任务已有正在运行的处理流程",
            true,
        ));
    }
    active.insert(task.id.clone(), token.clone());
    drop(active);
    spawn_registered_task(state, task, artifact, token, providers);
    Ok(())
}

/// 通过 Provider 接口串联转写、Prompt、纪要验证和 SQLite 保存。
async fn run_task(
    runtime: TaskRuntime,
    providers: ProcessingProviders,
    mut task: TaskRecord,
    artifact: RegisteredArtifact,
    token: CancellationToken,
) -> Result<(), crate::storage::StorageError> {
    let result = run_provider_pipeline(
        &runtime.repository,
        runtime.importer.clone(),
        &providers,
        &mut task,
        artifact,
        token.clone(),
        &runtime.task_gate,
    )
    .await;
    let _task_guard = runtime
        .task_gate
        .lock()
        .map_err(|_| crate::storage::StorageError::LockPoisoned)?;
    let succeeded = result.is_ok();
    if token.is_cancelled() {
        mark_cancelled(&mut task);
    } else if let Err(mut error) = result {
        task.status = TaskStatus::Failed;
        task.progress = None;
        let can_retry = error.retryable && task.attempt < task.max_attempts;
        if error.retryable && !can_retry {
            error = retry_limit_error();
        }
        task.available_actions = if can_retry {
            vec![TaskAction::Retry]
        } else {
            Vec::new()
        };
        task.error = Some(error);
        task.updated_at = Utc::now().to_rfc3339();
    }
    let should_cleanup = matches!(task.status, TaskStatus::Completed | TaskStatus::Cancelled)
        || task.error.as_ref().is_some_and(|error| !error.retryable);
    if should_cleanup {
        let cleanup = runtime.importer.remove_artifact(&task.artifact_id);
        if cleanup.is_ok() {
            if let Ok(mut registry) = runtime.artifacts.lock() {
                registry.remove(&task.artifact_id);
            }
        } else if succeeded {
            task.status = TaskStatus::Failed;
            task.progress = None;
            task.available_actions = vec![TaskAction::Retry, TaskAction::OpenMeeting];
            task.error = Some(SafeTaskError {
                code: "artifact_cleanup_failed".to_string(),
                retryable: true,
                safe_message: "会议已保存，但音频暂存副本清理失败；关闭占用程序后重试清理"
                    .to_string(),
                http_status: None,
                retry_after_ms: None,
            });
            task.updated_at = Utc::now().to_rfc3339();
        }
    }
    finish_processing_attempt(&mut task);
    let persist_result = persist_terminal_task(&runtime.repository, &mut task);
    runtime
        .cancellations
        .lock()
        .map_err(|_| crate::storage::StorageError::LockPoisoned)?
        .remove(&task.id);
    persist_result
}

/// 执行可取消的 Provider 处理流水线并只持久化已验证结果。
async fn run_provider_pipeline(
    repository: &MeetingRepository,
    importer: Arc<crate::ingest::OfflineAudioImporter>,
    providers: &ProcessingProviders,
    task: &mut TaskRecord,
    artifact: RegisteredArtifact,
    token: CancellationToken,
    task_gate: &Arc<Mutex<()>>,
) -> Result<(), SafeTaskError> {
    update_task(
        repository,
        task,
        TaskStatus::Preparing,
        Some(0.08),
        task_gate,
        &token,
    )?;
    wait_stage(&token).await?;
    let artifact_ref = AudioArtifactRef {
        id: artifact.id.clone(),
        import_batch_id: task.batch_id.clone(),
        source_kind: AudioSourceKind::UserSelectedFile,
        staging_metadata: StagingMetadata {
            mime_type: artifact.mime_type,
            byte_length: artifact.byte_length,
            duration_ms: artifact.duration_ms,
            sha256: None,
            validated_at: Utc::now(),
        },
    };
    let reader_id = artifact_ref.id.clone();
    let reader = Arc::new(move || {
        importer.open_artifact_readonly(&reader_id).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "managed artifact unavailable")
        })
    });
    let managed = ManagedAudioArtifact::new(artifact_ref, reader);
    update_task(
        repository,
        task,
        TaskStatus::Transcribing,
        Some(0.28),
        task_gate,
        &token,
    )?;
    let context = ProviderCallContext::with_timeout(
        task.id.clone(),
        Uuid::new_v4().to_string(),
        token.clone(),
        Duration::from_millis(providers.transcription_timeout_ms),
    );
    let transcript = providers
        .transcription
        .transcribe(
            &context,
            TranscriptionRequest {
                artifact: managed,
                options: TranscriptionOptions::default(),
            },
            providers.transcription_credential.as_deref(),
        )
        .await
        .map_err(provider_error)?;
    update_task(
        repository,
        task,
        TaskStatus::ValidatingTranscript,
        Some(0.52),
        task_gate,
        &token,
    )?;
    let meeting_context = MeetingContext::default();
    let built_prompt = build_prompt(PromptBuildRequest {
        transcript: &transcript,
        context: &meeting_context,
        template_id: &task.template_id,
        template_version: BUILTIN_TEMPLATE_VERSION,
        validation_options: ValidationOptions::default(),
    })
    .map_err(minutes_error)?;
    update_task(
        repository,
        task,
        TaskStatus::Summarizing,
        Some(0.68),
        task_gate,
        &token,
    )?;
    let minutes_context = ProviderCallContext::with_timeout(
        task.id.clone(),
        Uuid::new_v4().to_string(),
        token.clone(),
        Duration::from_millis(providers.minutes_timeout_ms),
    );
    let candidate = providers
        .minutes
        .generate_candidate(
            &minutes_context,
            built_prompt.into_provider_request(),
            providers.minutes_credential.as_deref(),
        )
        .await
        .map_err(provider_error)?;
    update_task(
        repository,
        task,
        TaskStatus::ValidatingMinutes,
        Some(0.82),
        task_gate,
        &token,
    )?;
    let minutes = validate_provider_candidate(
        candidate,
        MEETING_MINUTES_SCHEMA_VERSION,
        &transcript,
        &meeting_context,
        ValidationOptions::default(),
    )
    .map_err(minutes_error)?;
    if token.is_cancelled() {
        return Err(cancelled_error());
    }
    task.available_actions.clear();
    update_task(
        repository,
        task,
        TaskStatus::Saving,
        Some(0.94),
        task_gate,
        &token,
    )?;
    let meeting_id = Uuid::new_v4().to_string();
    let title = minutes
        .title
        .clone()
        .unwrap_or_else(|| display_stem(&task.display_name));
    repository
        .save_completed_meeting(&PersistedMeetingInput {
            id: meeting_id.clone(),
            source_name: task.display_name.clone(),
            title,
            template_id: task.template_id.clone(),
            transcript: transcript.text.clone(),
            transcript_segments: serde_json::to_value(&transcript).map_err(|_| local_error())?,
            minutes: serde_json::to_value(&minutes).map_err(|_| local_error())?,
            schema_version: MEETING_MINUTES_SCHEMA_VERSION.to_string(),
        })
        .map_err(|_| local_error())?;
    if token.is_cancelled() {
        let _ = repository.delete_meeting(&meeting_id);
        return Err(cancelled_error());
    }
    task.meeting_id = Some(meeting_id);
    task.status = TaskStatus::Completed;
    task.progress = Some(1.0);
    task.error = None;
    task.available_actions = vec![TaskAction::OpenMeeting];
    task.updated_at = Utc::now().to_rfc3339();
    Ok(())
}

/// 在阶段之间提供短暂取消窗口。
async fn wait_stage(token: &CancellationToken) -> Result<(), SafeTaskError> {
    tokio::select! {
        _ = token.cancelled() => Err(cancelled_error()),
        _ = tokio::time::sleep(Duration::from_millis(80)) => Ok(()),
    }
}

/// 更新任务状态并先写入 SQLite。
fn update_task(
    repository: &MeetingRepository,
    task: &mut TaskRecord,
    status: TaskStatus,
    progress: Option<f64>,
    task_gate: &Arc<Mutex<()>>,
    token: &CancellationToken,
) -> Result<(), SafeTaskError> {
    let _task_guard = task_gate.lock().map_err(|_| local_error())?;
    if token.is_cancelled() {
        return Err(cancelled_error());
    }
    if task.processing_started_at.is_none() {
        task.processing_started_at = Some(Utc::now().to_rfc3339());
    }
    task.status = status;
    task.progress = progress;
    task.updated_at = Utc::now().to_rfc3339();
    repository.save_task(task).map_err(|_| local_error())
}

/// 按前端筛选语义判断任务是否可见。
fn task_matches_filter(task: &TaskRecord, filter: &str) -> bool {
    match filter {
        "active" => !task.status.is_terminal(),
        "failed" => matches!(task.status, TaskStatus::Failed | TaskStatus::Interrupted),
        "completed" => task.status == TaskStatus::Completed,
        _ => true,
    }
}

/// 判断任务是否可独立删除，避免绕过会议记录的级联删除流程。
fn task_can_be_deleted(task: &TaskRecord) -> bool {
    task.meeting_id.is_none() && matches!(task.status, TaskStatus::Failed | TaskStatus::Interrupted)
}

/// 校验并删除任务快照，成功时返回需要尝试清理的受管 artifact ID。
fn delete_task_record(
    repository: &MeetingRepository,
    task_id: &str,
) -> Result<Option<String>, CommandError> {
    let task = find_task(repository, task_id)?;
    if !task_can_be_deleted(&task) {
        return Err(CommandError::new(
            "task_not_deletable",
            "仅允许删除未生成会议记录的失败任务",
            false,
        ));
    }
    let deleted = repository.delete_task(task_id)?;
    Ok(deleted.then_some(task.artifact_id))
}

/// 为旧版持久化的失败任务补充删除动作，不改写数据库历史快照。
fn with_available_delete_action(mut task: TaskRecord) -> TaskRecord {
    if task_can_be_deleted(&task) && !task.available_actions.contains(&TaskAction::Delete) {
        task.available_actions.push(TaskAction::Delete);
    }
    task
}

/// 从 SQLite 快照中查找任务。
fn find_task(repository: &MeetingRepository, task_id: &str) -> Result<TaskRecord, CommandError> {
    repository
        .list_tasks()?
        .into_iter()
        .find(|task| task.id == task_id)
        .ok_or_else(|| CommandError::new("task_not_found", "任务不存在或已被移除", false))
}

/// 把 Provider 安全错误转换为前端任务错误。
fn provider_error(error: ProviderError) -> SafeTaskError {
    SafeTaskError {
        code: error.code,
        retryable: error.retryable,
        safe_message: error.safe_message,
        http_status: error.http_status,
        retry_after_ms: error.retry_after_ms,
    }
}

/// 把纪要安全错误转换为前端任务错误。
fn minutes_error(error: crate::minutes::MinutesError) -> SafeTaskError {
    SafeTaskError {
        code: error.code().to_string(),
        retryable: false,
        safe_message: error.to_string(),
        http_status: None,
        retry_after_ms: None,
    }
}

/// 创建不包含数据库细节的本地持久化错误。
fn local_error() -> SafeTaskError {
    SafeTaskError {
        code: "local_storage_error".to_string(),
        retryable: true,
        safe_message: "无法保存本地会议记录".to_string(),
        http_status: None,
        retry_after_ms: None,
    }
}

/// 创建表示任务已耗尽配置尝试次数的安全错误。
fn retry_limit_error() -> SafeTaskError {
    SafeTaskError {
        code: "retry_limit_reached".to_string(),
        retryable: false,
        safe_message: "任务已达到最大尝试次数，请检查配置或重新创建任务".to_string(),
        http_status: None,
        retry_after_ms: None,
    }
}

/// 保存终态；首次失败时把任务降级为可观察的本地持久化失败并再尝试一次。
fn persist_terminal_task(
    repository: &MeetingRepository,
    task: &mut TaskRecord,
) -> Result<(), crate::storage::StorageError> {
    if repository.save_task(task).is_ok() {
        return Ok(());
    }
    task.status = TaskStatus::Failed;
    task.progress = None;
    task.error = Some(SafeTaskError {
        code: "terminal_state_persist_failed".to_string(),
        retryable: true,
        safe_message: "任务结果状态未能完整保存，请重启应用后检查本地记录".to_string(),
        http_status: None,
        retry_after_ms: None,
    });
    task.available_actions = if task.meeting_id.is_some() {
        vec![TaskAction::OpenMeeting]
    } else {
        Vec::new()
    };
    task.updated_at = Utc::now().to_rfc3339();
    repository.save_task(task)
}

/// 创建一致的取消错误。
fn cancelled_error() -> SafeTaskError {
    SafeTaskError {
        code: "cancelled".to_string(),
        retryable: false,
        safe_message: "任务已取消".to_string(),
        http_status: None,
        retry_after_ms: None,
    }
}

/// 将任务设置为已取消终态。
fn mark_cancelled(task: &mut TaskRecord) {
    finish_processing_attempt(task);
    task.status = TaskStatus::Cancelled;
    task.progress = None;
    task.error = None;
    task.available_actions.clear();
    task.updated_at = Utc::now().to_rfc3339();
}

/// Closes the current attempt and accumulates only time spent actively processing.
fn finish_processing_attempt(task: &mut TaskRecord) {
    let ended_at = Utc::now().to_rfc3339();
    finish_processing_attempt_at(task, &ended_at);
}

/// Uses an explicit end bound during startup recovery so offline time is never counted.
fn finish_processing_attempt_at(task: &mut TaskRecord, ended_at: &str) {
    let Some(started_at) = task.processing_started_at.take() else {
        return;
    };
    let elapsed_ms = chrono::DateTime::parse_from_rfc3339(ended_at)
        .ok()
        .zip(chrono::DateTime::parse_from_rfc3339(&started_at).ok())
        .map(|(ended, started)| ended.signed_duration_since(started).num_milliseconds())
        .unwrap_or(0)
        .max(0) as u64;
    task.processing_duration_ms = Some(
        task.processing_duration_ms
            .unwrap_or_default()
            .saturating_add(elapsed_ms),
    );
}

/// 从显示文件名生成不含扩展名的安全标题。
fn display_stem(display_name: &str) -> String {
    std::path::Path::new(display_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("会议纪要")
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::ingest::{ImportRequest, ImportSelectionMode, IngestPolicy, OfflineAudioImporter};
    use crate::providers::{MockConfig, MockProvider, MockScenario};

    /// 生成 100 毫秒、16 kHz、单声道 PCM WAV 测试音频。
    fn valid_wav() -> Vec<u8> {
        let pcm = vec![0u8; 3_200];
        let riff_size = 36u32 + pcm.len() as u32;
        let mut bytes = Vec::with_capacity(44 + pcm.len());
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&riff_size.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&32_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&pcm);
        bytes
    }

    /// 构造用于批次资源限制测试的安全受管音频元数据。
    fn registered_artifact(id: &str, byte_length: u64) -> RegisteredArtifact {
        RegisteredArtifact {
            id: id.to_string(),
            display_name: format!("{id}.wav"),
            mime_type: "audio/wav".to_string(),
            byte_length,
            duration_ms: None,
        }
    }

    /// Injects the same deterministic mock behind both production provider interfaces.
    fn mock_processing_providers() -> ProcessingProviders {
        let candidate = json!({
            "schemaVersion": MEETING_MINUTES_SCHEMA_VERSION,
            "title": "Mock 会议纪要",
            "titleSource": "generated",
            "meetingTime": {"startAt": null, "endAt": null},
            "participants": [],
            "summary": "Mock Provider 已完成离线音频处理流程验证。",
            "topics": [],
            "conclusions": [],
            "decisions": [],
            "actionItems": [],
            "risksAndIssues": []
        });
        let provider = Arc::new(
            MockProvider::new(
                MockConfig {
                    scenario: MockScenario::Success,
                    delay_ms: 0,
                },
                "这是 Mock Provider 生成的匿名测试逐字稿，用于验证本地处理闭环。".to_string(),
                candidate,
                MEETING_MINUTES_SCHEMA_VERSION.to_string(),
            )
            .expect("create mock provider"),
        );
        let transcription: Arc<dyn TranscriptionProvider> = provider.clone();
        let minutes: Arc<dyn MinutesProvider> = provider;
        ProcessingProviders {
            transcription,
            minutes,
            transcription_credential: None,
            minutes_credential: None,
            transcription_timeout_ms: 5_000,
            minutes_timeout_ms: 5_000,
            max_attempts: 3,
        }
    }

    /// 验证多次追加后的逻辑批次仍受累计数量和总大小限制。
    #[test]
    fn rejects_logical_batch_over_cumulative_limits() {
        let registry = HashMap::from([
            ("a".to_string(), registered_artifact("a", 60)),
            ("b".to_string(), registered_artifact("b", 60)),
            ("c".to_string(), registered_artifact("c", 10)),
        ]);
        let count_policy = IngestPolicy::new(100, 2, 200).expect("count policy");
        let count_error = resolve_batch_artifacts(
            &["a".to_string(), "b".to_string(), "c".to_string()],
            &registry,
            count_policy,
        )
        .expect_err("reject cumulative item count");
        assert_eq!(count_error.code, "batch_limit_exceeded");

        let bytes_policy = IngestPolicy::new(100, 3, 100).expect("bytes policy");
        let bytes_error =
            resolve_batch_artifacts(&["a".to_string(), "b".to_string()], &registry, bytes_policy)
                .expect_err("reject cumulative bytes");
        assert_eq!(bytes_error.code, "batch_limit_exceeded");
    }

    /// 验证带重试动作的失败任务仍被视为占用受管音频。
    #[test]
    fn retryable_failed_task_retains_audio_artifact() {
        let artifact = registered_artifact("retryable", 60);
        let mut task = new_task(&artifact, None, "standard_meeting", 3);
        task.status = TaskStatus::Failed;
        task.available_actions = vec![TaskAction::Retry];

        assert!(task.retains_audio_artifact());

        task.available_actions.clear();
        assert!(!task.retains_audio_artifact());
    }

    /// 验证只有未关联会议的失败或中断任务可由队列独立删除。
    #[test]
    fn delete_action_is_limited_to_failed_tasks_without_meetings() {
        let artifact = registered_artifact("deletable", 60);
        let mut task = new_task(&artifact, None, "standard_meeting", 3);
        task.status = TaskStatus::Failed;
        assert!(task_can_be_deleted(&task));
        assert!(with_available_delete_action(task.clone())
            .available_actions
            .contains(&TaskAction::Delete));

        task.status = TaskStatus::Completed;
        assert!(!task_can_be_deleted(&task));
        task.status = TaskStatus::Failed;
        task.meeting_id = Some("meeting-protected".to_string());
        assert!(!task_can_be_deleted(&task));
    }

    /// 验证后端删除函数拒绝活动任务，并实际移除失败任务快照。
    #[test]
    fn deletes_only_failed_task_records() {
        let repository = MeetingRepository::in_memory().expect("create repository");
        let artifact = registered_artifact("delete-record", 60);
        let mut task = new_task(&artifact, None, "standard_meeting", 3);
        repository.save_task(&task).expect("save active task");

        let error = delete_task_record(&repository, &task.id).expect_err("reject active task");
        assert_eq!(error.code, "task_not_deletable");

        task.status = TaskStatus::Failed;
        repository.save_task(&task).expect("save failed task");
        assert_eq!(
            delete_task_record(&repository, &task.id).expect("delete failed task"),
            Some(artifact.id)
        );
        assert!(repository.list_tasks().expect("list tasks").is_empty());
    }

    /// 验证真实导入模块、MockProvider、纪要校验和 SQLite 保存形成完整闭环。
    #[tokio::test]
    async fn mock_pipeline_persists_transcript_and_minutes() {
        let source_root = TempDir::new().expect("create source tempdir");
        let staging_root = TempDir::new().expect("create staging tempdir");
        let source_path = source_root.path().join("integration.wav");
        fs::write(&source_path, valid_wav()).expect("write audio fixture");
        let importer = Arc::new(
            OfflineAudioImporter::new(
                staging_root.path(),
                IngestPolicy::new(8 * 1024 * 1024, 4, 16 * 1024 * 1024).expect("create policy"),
            )
            .expect("create importer"),
        );
        let imported = importer.import_selected_files(
            ImportRequest {
                selection_mode: ImportSelectionMode::Single,
            },
            vec![source_path],
        );
        let reference = imported.items[0]
            .artifact
            .as_ref()
            .expect("import artifact")
            .clone();
        let artifact = RegisteredArtifact {
            id: reference.id.clone(),
            display_name: "integration.wav".to_string(),
            mime_type: reference.staging_metadata.mime_type,
            byte_length: reference.staging_metadata.byte_length,
            duration_ms: reference.staging_metadata.duration_ms,
        };
        let repository = Arc::new(MeetingRepository::in_memory().expect("create repository"));
        let task = new_task(&artifact, None, "standard_meeting", 3);
        let task_id = task.id.clone();
        let artifacts = Arc::new(Mutex::new(std::collections::HashMap::from([(
            artifact.id.clone(),
            artifact.clone(),
        )])));
        let cancellations = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let task_gate = Arc::new(Mutex::new(()));

        let runtime = TaskRuntime {
            repository: repository.clone(),
            importer,
            artifacts: artifacts.clone(),
            cancellations,
            task_gate,
        };
        run_task(
            runtime,
            mock_processing_providers(),
            task,
            artifact,
            CancellationToken::new(),
        )
        .await
        .expect("persist terminal task");

        let completed = repository
            .list_tasks()
            .expect("list tasks")
            .into_iter()
            .find(|task| task.id == task_id)
            .expect("completed task");
        assert_eq!(completed.status, TaskStatus::Completed);
        let meeting_id = completed.meeting_id.expect("meeting id");
        let detail = repository
            .get_meeting(&meeting_id)
            .expect("query meeting")
            .expect("meeting detail");
        assert!(detail.transcript.contains("Mock Provider"));
        assert_eq!(detail.minutes["schemaVersion"], "1.0.0");
        assert_eq!(
            fs::read_dir(staging_root.path())
                .expect("staging exists")
                .count(),
            0
        );
        assert!(artifacts.lock().expect("artifact registry").is_empty());
    }

    /// 验证重启会终结已请求取消的任务，并将其他活动任务标记为中断。
    #[test]
    fn recovers_cancel_requested_and_active_tasks() {
        let repository = MeetingRepository::in_memory().expect("create repository");
        let artifact = RegisteredArtifact {
            id: "artifact-recovery".to_string(),
            display_name: "recovery.wav".to_string(),
            mime_type: "audio/wav".to_string(),
            byte_length: 128,
            duration_ms: None,
        };
        let mut cancel_requested = new_task(&artifact, None, "standard_meeting", 3);
        cancel_requested.status = TaskStatus::CancelRequested;
        cancel_requested.available_actions.clear();
        repository
            .save_task(&cancel_requested)
            .expect("save cancel requested task");
        let mut active = new_task(&artifact, None, "standard_meeting", 3);
        active.id = "active-recovery".to_string();
        active.status = TaskStatus::Transcribing;
        active.processing_started_at = Some("2026-07-21T01:00:00Z".to_string());
        active.updated_at = "2026-07-21T01:00:05Z".to_string();
        repository.save_task(&active).expect("save active task");
        let mut exhausted = new_task(&artifact, None, "standard_meeting", 1);
        exhausted.id = "exhausted-recovery".to_string();
        exhausted.status = TaskStatus::Summarizing;
        repository
            .save_task(&exhausted)
            .expect("save exhausted task");

        recover_interrupted_tasks(&repository).expect("recover tasks");

        let recovered = repository.list_tasks().expect("list recovered tasks");
        let cancelled = recovered
            .iter()
            .find(|task| task.id == cancel_requested.id)
            .expect("cancelled task");
        assert_eq!(cancelled.status, TaskStatus::Cancelled);
        let interrupted = recovered
            .iter()
            .find(|task| task.id == active.id)
            .expect("interrupted task");
        assert_eq!(interrupted.status, TaskStatus::Interrupted);
        assert_eq!(interrupted.processing_duration_ms, Some(5_000));
        assert!(interrupted.processing_started_at.is_none());
        assert_eq!(
            interrupted.available_actions,
            vec![TaskAction::ReselectFile]
        );
        let exhausted = recovered
            .iter()
            .find(|task| task.id == "exhausted-recovery")
            .expect("exhausted task");
        assert_eq!(exhausted.status, TaskStatus::Interrupted);
        assert!(exhausted.available_actions.is_empty());
        assert_eq!(
            exhausted.error.as_ref().map(|error| error.code.as_str()),
            Some("retry_limit_reached")
        );
    }

    /// 验证重试次数达到上限后会清除动作并持久化稳定错误。
    #[test]
    fn rejects_retry_after_max_attempts() {
        let repository = MeetingRepository::in_memory().expect("create repository");
        let artifact = RegisteredArtifact {
            id: "artifact-retry-limit".to_string(),
            display_name: "retry-limit.wav".to_string(),
            mime_type: "audio/wav".to_string(),
            byte_length: 128,
            duration_ms: None,
        };
        let mut task = new_task(&artifact, None, "standard_meeting", 1);
        task.status = TaskStatus::Failed;
        task.available_actions = vec![TaskAction::Retry];
        repository.save_task(&task).expect("save failed task");

        let error = ensure_retry_available(&mut task, &repository).expect_err("reject retry");

        assert_eq!(error.code, "retry_limit_reached");
        let persisted = find_task(&repository, &task.id).expect("load task");
        assert!(persisted.available_actions.is_empty());
        assert_eq!(
            persisted.error.as_ref().map(|value| value.code.as_str()),
            Some("retry_limit_reached")
        );
    }

    /// 验证已经发出取消信号的后台流程只能落为 cancelled，并移除活动令牌。
    #[tokio::test]
    async fn cancelled_pipeline_persists_terminal_state_and_releases_token() {
        let source_root = TempDir::new().expect("create source tempdir");
        let staging_root = TempDir::new().expect("create staging tempdir");
        let source_path = source_root.path().join("cancel.wav");
        fs::write(&source_path, valid_wav()).expect("write audio fixture");
        let importer = Arc::new(
            OfflineAudioImporter::new(
                staging_root.path(),
                IngestPolicy::new(8 * 1024 * 1024, 4, 16 * 1024 * 1024).expect("create policy"),
            )
            .expect("create importer"),
        );
        let imported = importer.import_selected_files(
            ImportRequest {
                selection_mode: ImportSelectionMode::Single,
            },
            vec![source_path],
        );
        let reference = imported.items[0]
            .artifact
            .as_ref()
            .expect("import artifact")
            .clone();
        let artifact = RegisteredArtifact {
            id: reference.id.clone(),
            display_name: "cancel.wav".to_string(),
            mime_type: reference.staging_metadata.mime_type,
            byte_length: reference.staging_metadata.byte_length,
            duration_ms: reference.staging_metadata.duration_ms,
        };
        let task = new_task(&artifact, None, "standard_meeting", 3);
        let task_id = task.id.clone();
        let repository = Arc::new(MeetingRepository::in_memory().expect("create repository"));
        repository.save_task(&task).expect("save task");
        let artifacts = Arc::new(Mutex::new(std::collections::HashMap::from([(
            artifact.id.clone(),
            artifact.clone(),
        )])));
        let token = CancellationToken::new();
        token.cancel();
        let cancellations = Arc::new(Mutex::new(std::collections::HashMap::from([(
            task_id.clone(),
            token.clone(),
        )])));

        let runtime = TaskRuntime {
            repository: repository.clone(),
            importer,
            artifacts: artifacts.clone(),
            cancellations: cancellations.clone(),
            task_gate: Arc::new(Mutex::new(())),
        };
        run_task(runtime, mock_processing_providers(), task, artifact, token)
            .await
            .expect("persist cancellation");

        let cancelled = find_task(&repository, &task_id).expect("load cancelled task");
        assert_eq!(cancelled.status, TaskStatus::Cancelled);
        assert!(cancellations.lock().expect("lock cancellations").is_empty());
        assert!(artifacts.lock().expect("lock artifacts").is_empty());
    }
}
