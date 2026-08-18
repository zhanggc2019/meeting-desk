pub mod app_logging;
pub mod app_state;
pub mod commands;
pub mod config;
pub mod domain;
pub mod ingest;
pub mod meeting_export;
pub mod minutes;
pub mod providers;
pub mod secrets;
pub mod storage;

use std::sync::Arc;
use std::time::Instant;
use tauri::Manager;

/// 初始化插件并启动 Tauri 应用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_log::Builder::new().skip_logger().build())
        .setup(|app| {
            let setup_started = Instant::now();
            let logging_report = app
                .path()
                .app_log_dir()
                .ok()
                .and_then(|log_dir| app_logging::initialize_application_logger(&log_dir).ok());
            if let Some(report) = logging_report {
                log::info!(
                    target: "app.lifecycle",
                    "application_start version={} platform=windows retention_days={} expired_logs_removed={}",
                    app.package_info().version,
                    app_logging::LOG_RETENTION_DAYS,
                    report.removed_expired_files,
                );
            }
            let data_dir = app.path().app_local_data_dir()?;
            let repository = storage::MeetingRepository::open(&data_dir.join("meetings.sqlite3"))?;
            log::info!(target: "app.storage", "database_ready backend=sqlite");
            commands::tasks::recover_interrupted_tasks(&repository)?;
            log::info!(target: "app.tasks", "interrupted_task_recovery_completed");
            let policy =
                ingest::IngestPolicy::new(2 * 1024 * 1024 * 1024, 32, 8 * 1024 * 1024 * 1024)?;
            let importer = ingest::OfflineAudioImporter::new(data_dir.join("audio"), policy)?;
            // 启动清理采用 best-effort；只持久化待重试状态，不记录受管文件名或路径。
            let cleanup_pending = match importer.clear_staged_files() {
                Ok(report) => {
                    log::info!(
                        target: "app.ingest",
                        "staging_cleanup_completed removed_count={} failed_count={}",
                        report.removed,
                        report.failed,
                    );
                    report.failed > 0
                }
                Err(_) => {
                    log::warn!(
                        target: "app.ingest",
                        "staging_cleanup_failed error_code=audio_storage_failed retryable=true"
                    );
                    true
                }
            };
            if repository
                .set_setting(
                "staging_cleanup_pending",
                if cleanup_pending { "true" } else { "false" },
                )
                .is_err()
            {
                log::warn!(
                    target: "app.storage",
                    "setting_persist_failed setting=staging_cleanup_pending retryable=true"
                );
            }
            app.manage(app_state::AppState {
                repository: Arc::new(repository),
                data_dir,
                importer: Arc::new(importer),
                artifacts: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
                import_gate: Arc::new(std::sync::Mutex::new(())),
                cancellations: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
                task_gate: Arc::new(std::sync::Mutex::new(())),
            });
            log::info!(
                target: "app.lifecycle",
                "application_ready setup_elapsed_ms={}",
                setup_started.elapsed().as_millis(),
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ingest::select_audio_files,
            commands::ingest::release_audio_artifact,
            commands::meetings::list_meetings,
            commands::meetings::list_meetings_page,
            commands::meetings::get_meeting_detail,
            commands::meetings::get_meeting_markdown_preview,
            commands::meetings::play_meeting_media,
            commands::meetings::delete_meeting,
            commands::meetings::export_meeting_markdown,
            commands::meetings::export_meeting_document_command,
            commands::meetings::list_minutes_templates,
            commands::settings::get_public_settings,
            commands::settings::select_local_model_directory,
            commands::settings::save_provider_settings,
            commands::settings::delete_provider_secret,
            commands::settings::test_provider_connection,
            commands::tasks::create_processing_tasks,
            commands::tasks::list_processing_tasks,
            commands::tasks::list_processing_tasks_page,
            commands::tasks::cancel_processing_task,
            commands::tasks::retry_processing_task,
            commands::tasks::delete_processing_task,
            commands::tasks::reselect_processing_task,
        ])
        .run(tauri::generate_context!())
        .expect("启动听见纪要失败");
}
