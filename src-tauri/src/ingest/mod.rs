mod error;
mod format;

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub use error::{IngestError, IngestErrorCode};
pub use format::AudioFormat;

use format::{expected_format, inspect_audio};

const COPY_BUFFER_BYTES: usize = 64 * 1024;

/// Tauri 命令层可接受的文件选择模式；实际路径必须来自原生文件对话框。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSelectionMode {
    Single,
    Batch,
}

/// 供 Tauri 命令包装器反序列化的安全请求，不接受任意路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub selection_mode: ImportSelectionMode,
}

/// 离线导入的资源限制，所有限制均由可信后端配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestPolicy {
    pub max_file_bytes: u64,
    pub max_batch_items: usize,
    pub max_batch_total_bytes: u64,
}

impl IngestPolicy {
    /// 创建并验证非零、内部一致的离线导入限制。
    pub fn new(
        max_file_bytes: u64,
        max_batch_items: usize,
        max_batch_total_bytes: u64,
    ) -> Result<Self, IngestError> {
        if max_file_bytes == 0
            || max_batch_items == 0
            || max_batch_total_bytes == 0
            || max_file_bytes > max_batch_total_bytes
        {
            return Err(IngestError::InvalidPolicy);
        }
        Ok(Self {
            max_file_bytes,
            max_batch_items,
            max_batch_total_bytes,
        })
    }
}

/// 音频来源类型；当前仅允许用户通过文件对话框选择的离线文件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioSourceKind {
    UserSelectedFile,
}

/// Provider 可消费的受管音频元数据；完整哈希不会被序列化到 IPC。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagingMetadata {
    pub mime_type: String,
    pub byte_length: u64,
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing, default)]
    pub sha256: Option<String>,
    pub validated_at: DateTime<Utc>,
}

/// 后端 Provider 与任务层使用的不透明受管音频引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioArtifactRef {
    pub id: String,
    pub import_batch_id: Option<String>,
    pub source_kind: AudioSourceKind,
    pub staging_metadata: StagingMetadata,
}

/// 单个选择项的终态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportItemStatus {
    Ready,
    Duplicate,
    Failed,
}

/// 可安全发送给 UI 的导入错误，不包含路径或底层错误文本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportItemError {
    pub code: IngestErrorCode,
    pub safe_message_key: String,
}

impl ImportItemError {
    /// 从内部错误创建不含敏感上下文的公开错误。
    fn from_error(error: &IngestError) -> Self {
        Self {
            code: error.code(),
            safe_message_key: error.safe_message_key().to_owned(),
        }
    }
}

/// 一个文件选择项的导入结果；selection_index 用于 UI 对齐原顺序。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportItemResult {
    pub selection_index: usize,
    pub status: ImportItemStatus,
    pub artifact: Option<AudioArtifactRef>,
    pub duplicate_of_artifact_id: Option<String>,
    pub error: Option<ImportItemError>,
}

/// 单个原生文件对话框选择批次的逐文件结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportBatchResponse {
    pub batch_id: String,
    pub items: Vec<ImportItemResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ArtifactKey {
    sha256: String,
    byte_length: u64,
}

#[derive(Debug, Clone)]
struct ArtifactRecord {
    reference: AudioArtifactRef,
    managed_path: PathBuf,
}

/// 纯离线音频导入服务；调用方应在阻塞线程池中执行文件 IO。
pub struct OfflineAudioImporter {
    staging_root: PathBuf,
    policy: IngestPolicy,
    artifacts: Mutex<HashMap<ArtifactKey, ArtifactRecord>>,
}

impl OfflineAudioImporter {
    /// 在可信受管根创建导入器，并确保根目录存在且可解析。
    pub fn new(
        staging_root: impl Into<PathBuf>,
        policy: IngestPolicy,
    ) -> Result<Self, IngestError> {
        let requested_root = staging_root.into();
        fs::create_dir_all(&requested_root).map_err(|_| IngestError::AudioStorageFailed)?;
        let canonical_root =
            fs::canonicalize(&requested_root).map_err(|_| IngestError::AudioStorageFailed)?;
        let metadata =
            fs::metadata(&canonical_root).map_err(|_| IngestError::AudioStorageFailed)?;
        if !metadata.is_dir() {
            return Err(IngestError::AudioStorageFailed);
        }
        Ok(Self {
            staging_root: canonical_root,
            policy,
            artifacts: Mutex::new(HashMap::new()),
        })
    }

    /// 返回当前导入器使用的资源限制，供任务提交边界复核累计批次。
    pub fn policy(&self) -> IngestPolicy {
        self.policy
    }

    /// 导入原生文件对话框返回的路径，并对每个选择项独立给出结果。
    pub fn import_selected_files(
        &self,
        request: ImportRequest,
        selected_paths: Vec<PathBuf>,
    ) -> ImportBatchResponse {
        let batch_id = Uuid::new_v4().to_string();
        if selected_paths.is_empty() {
            return ImportBatchResponse {
                batch_id,
                items: Vec::new(),
            };
        }

        if (request.selection_mode == ImportSelectionMode::Single && selected_paths.len() != 1)
            || selected_paths.len() > self.policy.max_batch_items
        {
            let error = if selected_paths.len() > self.policy.max_batch_items {
                IngestError::BatchLimitExceeded
            } else {
                IngestError::InvalidSelection
            };
            return ImportBatchResponse {
                batch_id,
                items: selected_paths
                    .iter()
                    .enumerate()
                    .map(|(index, _)| failed_item(index, &error))
                    .collect(),
            };
        }

        if self.batch_total_exceeds_limit(&selected_paths) {
            return ImportBatchResponse {
                batch_id,
                items: selected_paths
                    .iter()
                    .enumerate()
                    .map(|(index, _)| failed_item(index, &IngestError::BatchLimitExceeded))
                    .collect(),
            };
        }

        let items = selected_paths
            .iter()
            .enumerate()
            .map(|(index, path)| match self.import_one(path, &batch_id) {
                Ok(item) => ImportItemResult {
                    selection_index: index,
                    status: item.status,
                    duplicate_of_artifact_id: item.duplicate_of_artifact_id,
                    artifact: Some(item.artifact),
                    error: None,
                },
                Err(error) => failed_item(index, &error),
            })
            .collect();

        ImportBatchResponse { batch_id, items }
    }

    /// 为可信 Provider resolver 打开受管 artifact 的只读文件句柄。
    pub fn open_artifact_readonly(&self, artifact_id: &str) -> Result<File, IngestError> {
        let artifacts = self
            .artifacts
            .lock()
            .map_err(|_| IngestError::AudioStorageFailed)?;
        let record = artifacts
            .values()
            .find(|record| record.reference.id == artifact_id)
            .ok_or(IngestError::ArtifactNotFound)?;
        File::open(&record.managed_path).map_err(|_| IngestError::ArtifactNotFound)
    }

    /// 删除一个精确受管 artifact；永远不接触用户原始文件。
    pub fn remove_artifact(&self, artifact_id: &str) -> Result<bool, IngestError> {
        let found = {
            let artifacts = self
                .artifacts
                .lock()
                .map_err(|_| IngestError::AudioStorageFailed)?;
            artifacts
                .iter()
                .find(|(_, record)| record.reference.id == artifact_id)
                .map(|(key, record)| (key.clone(), record.clone()))
        };
        let Some((key, record)) = found else {
            return Ok(false);
        };
        match fs::remove_file(&record.managed_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(IngestError::AudioStorageFailed),
        }?;
        self.artifacts
            .lock()
            .map_err(|_| IngestError::AudioStorageFailed)?
            .remove(&key);
        Ok(true)
    }

    /// 启动时清理受管根目录中的直接文件，不递归且不跟随符号链接。
    pub fn clear_staged_files(&self) -> Result<usize, IngestError> {
        let mut removed = 0usize;
        for entry in
            fs::read_dir(&self.staging_root).map_err(|_| IngestError::AudioStorageFailed)?
        {
            let entry = entry.map_err(|_| IngestError::AudioStorageFailed)?;
            let file_type = entry
                .file_type()
                .map_err(|_| IngestError::AudioStorageFailed)?;
            if !file_type.is_file() {
                continue;
            }
            fs::remove_file(entry.path()).map_err(|_| IngestError::AudioStorageFailed)?;
            removed = removed.saturating_add(1);
        }
        self.artifacts
            .lock()
            .map_err(|_| IngestError::AudioStorageFailed)?
            .clear();
        Ok(removed)
    }

    /// 在复制前以 checked arithmetic 判断批次已知文件大小是否超限。
    fn batch_total_exceeds_limit(&self, selected_paths: &[PathBuf]) -> bool {
        let mut total = 0u64;
        for path in selected_paths {
            let Ok(metadata) = fs::metadata(path) else {
                continue;
            };
            let Some(updated) = total.checked_add(metadata.len()) else {
                return true;
            };
            total = updated;
            if total > self.policy.max_batch_total_bytes {
                return true;
            }
        }
        false
    }

    /// 将一个用户源文件只读复制到受管 staging，完成结构校验、哈希与去重。
    fn import_one(&self, source_path: &Path, batch_id: &str) -> Result<ImportedItem, IngestError> {
        let expected = expected_format(source_path)?;
        let path_metadata = match fs::metadata(source_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(IngestError::SourceNotFound)
            }
            Err(_) => return Err(IngestError::SourceUnreadable),
        };
        if !path_metadata.is_file() {
            return Err(IngestError::SourceNotFile);
        }
        if path_metadata.len() == 0 {
            return Err(IngestError::EmptyAudio);
        }
        if path_metadata.len() > self.policy.max_file_bytes {
            return Err(IngestError::FileTooLarge);
        }

        let mut source = File::open(source_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                IngestError::SourceNotFound
            } else {
                IngestError::SourceUnreadable
            }
        })?;
        let before = source
            .metadata()
            .map_err(|_| IngestError::SourceUnreadable)?;

        let artifact_id = Uuid::new_v4().to_string();
        let part_path = self.staging_root.join(format!(".{artifact_id}.part"));
        let mut part_guard = PartFileGuard::new(part_path.clone());
        let mut destination = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part_path)
            .map_err(|_| IngestError::AudioStorageFailed)?;

        let mut hasher = Sha256::new();
        let mut copied = 0u64;
        let mut buffer = [0u8; COPY_BUFFER_BYTES];
        loop {
            let bytes_read = source
                .read(&mut buffer)
                .map_err(|_| IngestError::SourceUnreadable)?;
            if bytes_read == 0 {
                break;
            }
            copied = copied
                .checked_add(bytes_read as u64)
                .ok_or(IngestError::FileTooLarge)?;
            if copied > self.policy.max_file_bytes || copied > before.len() {
                return Err(IngestError::SourceChangedDuringImport);
            }
            hasher.update(&buffer[..bytes_read]);
            destination
                .write_all(&buffer[..bytes_read])
                .map_err(|_| IngestError::AudioStorageFailed)?;
        }
        destination
            .sync_all()
            .map_err(|_| IngestError::AudioStorageFailed)?;
        drop(destination);

        let after_handle = source
            .metadata()
            .map_err(|_| IngestError::SourceUnreadable)?;
        let after_path =
            fs::metadata(source_path).map_err(|_| IngestError::SourceChangedDuringImport)?;
        if copied != before.len()
            || after_handle.len() != before.len()
            || after_path.len() != before.len()
            || metadata_modified_changed(&before, &after_handle)
            || metadata_modified_changed(&before, &after_path)
        {
            return Err(IngestError::SourceChangedDuringImport);
        }

        let inspection = inspect_audio(&part_path, expected)?;
        let sha256 = hex::encode(hasher.finalize());
        let key = ArtifactKey {
            sha256: sha256.clone(),
            byte_length: copied,
        };
        let mut artifacts = self
            .artifacts
            .lock()
            .map_err(|_| IngestError::AudioStorageFailed)?;
        if let Some(existing) = artifacts.get(&key) {
            if existing.managed_path.is_file() {
                return Ok(ImportedItem {
                    status: ImportItemStatus::Duplicate,
                    duplicate_of_artifact_id: Some(existing.reference.id.clone()),
                    artifact: existing.reference.clone(),
                });
            }
        }
        artifacts.remove(&key);

        let final_path = self
            .staging_root
            .join(format!("{artifact_id}.{}", inspection.format.extension()));
        fs::rename(&part_path, &final_path).map_err(|_| IngestError::AudioStorageFailed)?;
        part_guard.commit();

        let reference = AudioArtifactRef {
            id: artifact_id,
            import_batch_id: Some(batch_id.to_owned()),
            source_kind: AudioSourceKind::UserSelectedFile,
            staging_metadata: StagingMetadata {
                mime_type: inspection.format.mime_type().to_owned(),
                byte_length: copied,
                duration_ms: inspection.duration_ms,
                sha256: Some(sha256),
                validated_at: Utc::now(),
            },
        };
        artifacts.insert(
            key,
            ArtifactRecord {
                reference: reference.clone(),
                managed_path: final_path,
            },
        );

        Ok(ImportedItem {
            status: ImportItemStatus::Ready,
            artifact: reference,
            duplicate_of_artifact_id: None,
        })
    }
}

struct ImportedItem {
    status: ImportItemStatus,
    artifact: AudioArtifactRef,
    duplicate_of_artifact_id: Option<String>,
}

struct PartFileGuard {
    path: PathBuf,
    committed: bool,
}

impl PartFileGuard {
    /// 创建失败时自动清理单个精确 `.part` 路径的守卫。
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    /// 标记暂存文件已在同一受管目录内成功提升为最终 artifact。
    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for PartFileGuard {
    /// 在未提交时仅删除本 guard 持有的精确暂存路径。
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// 判断复制前后的可用修改时间是否发生变化。
fn metadata_modified_changed(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    match (before.modified(), after.modified()) {
        (Ok(before_time), Ok(after_time)) => before_time != after_time,
        _ => false,
    }
}

/// 创建保持原选择顺序的失败结果。
fn failed_item(selection_index: usize, error: &IngestError) -> ImportItemResult {
    ImportItemResult {
        selection_index,
        status: ImportItemStatus::Failed,
        artifact: None,
        duplicate_of_artifact_id: None,
        error: Some(ImportItemError::from_error(error)),
    }
}

#[cfg(test)]
mod tests;
