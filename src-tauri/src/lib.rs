pub mod app_state;
pub mod commands;
pub mod config;
pub mod domain;
pub mod ingest;
pub mod minutes;
pub mod providers;
pub mod secrets;
pub mod storage;

use std::sync::Arc;
use tauri::Manager;

/// 初始化插件并启动 Tauri 应用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .max_file_size(5_000_000) // 5 MB per file
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            let repository = storage::MeetingRepository::open(&data_dir.join("meetings.sqlite3"))?;
            commands::tasks::recover_interrupted_tasks(&repository)?;
            let policy =
                ingest::IngestPolicy::new(2 * 1024 * 1024 * 1024, 32, 8 * 1024 * 1024 * 1024)?;
            let importer = ingest::OfflineAudioImporter::new(data_dir.join("audio"), policy)?;
            // 启动清理采用 best-effort；只持久化待重试状态，不记录受管文件名或路径。
            let cleanup_pending = importer
                .clear_staged_files()
                .map(|report| report.failed > 0)
                .unwrap_or(true);
            let _ = repository.set_setting(
                "staging_cleanup_pending",
                if cleanup_pending { "true" } else { "false" },
            );
            app.manage(app_state::AppState {
                repository: Arc::new(repository),
                data_dir,
                importer: Arc::new(importer),
                artifacts: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
                import_gate: Arc::new(std::sync::Mutex::new(())),
                cancellations: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
                task_gate: Arc::new(std::sync::Mutex::new(())),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ingest::select_audio_files,
            commands::ingest::release_audio_artifact,
            commands::meetings::list_meetings,
            commands::meetings::get_meeting_detail,
            commands::meetings::get_meeting_markdown_preview,
            commands::meetings::delete_meeting,
            commands::meetings::export_meeting_markdown,
            commands::meetings::list_minutes_templates,
            commands::settings::get_public_settings,
            commands::settings::select_local_model_directory,
            commands::settings::save_provider_settings,
            commands::settings::delete_provider_secret,
            commands::settings::test_provider_connection,
            commands::tasks::create_processing_tasks,
            commands::tasks::list_processing_tasks,
            commands::tasks::cancel_processing_task,
            commands::tasks::retry_processing_task,
            commands::tasks::delete_processing_task,
            commands::tasks::reselect_processing_task,
        ])
        .run(tauri::generate_context!())
        .expect("启动听见纪要失败");
}
