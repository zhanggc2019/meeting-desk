use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::ingest::OfflineAudioImporter;
use crate::providers::CancellationToken;
use crate::storage::MeetingRepository;

/// 保存由 Tauri 管理、供命令共享的本地应用状态。
pub struct AppState {
    pub repository: Arc<MeetingRepository>,
    pub data_dir: PathBuf,
    pub importer: Arc<OfflineAudioImporter>,
    pub artifacts: Arc<Mutex<HashMap<String, RegisteredArtifact>>>,
    pub import_gate: Arc<Mutex<()>>,
    pub cancellations: Arc<Mutex<HashMap<String, CancellationToken>>>,
    pub task_gate: Arc<Mutex<()>>,
}

/// 保存任务编排所需、但不直接发送到 UI 的受管 artifact 元数据。
#[derive(Debug, Clone)]
pub struct RegisteredArtifact {
    pub id: String,
    pub display_name: String,
    pub source_path: PathBuf,
    pub mime_type: String,
    pub byte_length: u64,
    pub duration_ms: Option<u64>,
}
