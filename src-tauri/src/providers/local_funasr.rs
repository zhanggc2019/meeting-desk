use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{
    CapabilityEvidence, OperationOutcome, ProviderCallContext, ProviderCredential, ProviderError,
    ProviderErrorCategory, ProviderMetadata, ReplaySafety, Transcript, TranscriptSegment,
    TranscriptionCapabilities, TranscriptionProvider, TranscriptionRequest,
};

const ADAPTER_ID: &str = "local_funasr_python";
const ADAPTER_VERSION: &str = "3";
const APP_DATA_DIRECTORY: &str = "com.internal.meetingdesk";
const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;
const REQUIRED_MODEL_FILES: [&str; 3] = ["config.yaml", "model.pt", "tokens.json"];
const VAD_MODEL_DIRECTORY_NAME: &str = "fsmn-vad";
const REQUIRED_VAD_MODEL_FILES: [&str; 4] =
    ["config.yaml", "configuration.json", "model.pt", "am.mvn"];

/// 返回 Windows `CREATE_NO_WINDOW` 标志，避免本地 Python 推理弹出终端窗口。
#[cfg(windows)]
const fn windows_subprocess_creation_flags() -> u32 {
    0x0800_0000
}

/// Non-secret paths and model identity required by the local FunASR adapter.
#[derive(Clone)]
pub struct LocalFunAsrConfig {
    python_executable: OsString,
    script_path: PathBuf,
    model_dir: PathBuf,
    vad_model_dir: PathBuf,
    model_name: String,
}

impl fmt::Debug for LocalFunAsrConfig {
    /// Redacts local filesystem paths while retaining the safe model label.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalFunAsrConfig")
            .field("python_executable", &"[LOCAL]")
            .field("script_path", &"[LOCAL]")
            .field("model_dir", &"[LOCAL]")
            .field("vad_model_dir", &"[LOCAL]")
            .field("model_name", &self.model_name)
            .finish()
    }
}

/// Runs SenseVoiceSmall locally through a cancellable Python subprocess.
#[derive(Clone)]
pub struct LocalFunAsrProvider {
    config: LocalFunAsrConfig,
}

impl fmt::Debug for LocalFunAsrProvider {
    /// Formats only non-sensitive adapter metadata.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalFunAsrProvider")
            .field("adapter_id", &ADAPTER_ID)
            .field("model", &self.config.model_name)
            .finish()
    }
}

#[derive(Debug, Deserialize)]
struct LocalInferenceOutput {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    segments: Vec<LocalInferenceSegment>,
}

#[derive(Debug, Deserialize)]
struct LocalInferenceSegment {
    #[serde(default)]
    start_ms: Option<u64>,
    #[serde(default)]
    end_ms: Option<u64>,
    text: String,
}

impl LocalFunAsrProvider {
    /// Discovers the repository-local model, Python runtime, and bundled inference script.
    pub fn discover(model_name: impl Into<String>) -> Self {
        Self::discover_with_model_directory(model_name, None)
    }

    /// Discovers local runtime resources while honoring a user-selected model directory.
    pub fn discover_with_model_directory(
        model_name: impl Into<String>,
        configured_model_dir: Option<PathBuf>,
    ) -> Self {
        let model_name = model_name.into();
        let roots = discovery_roots();
        let model_dir = environment_path("MEETING_DESK_ASR_MODEL_DIR")
            .or(configured_model_dir)
            .unwrap_or_else(|| {
                first_existing(
                    roots
                        .iter()
                        .flat_map(|root| {
                            [
                                root.join("model").join(&model_name),
                                root.join("resources").join("model").join(&model_name),
                            ]
                        })
                        .collect(),
                )
            });
        let script_path = environment_path("MEETING_DESK_FUNASR_SCRIPT").unwrap_or_else(|| {
            first_existing(
                roots
                    .iter()
                    .flat_map(|root| {
                        [
                            root.join("src-tauri")
                                .join("python")
                                .join("local_funasr.py"),
                            root.join("python").join("local_funasr.py"),
                            root.join("resources")
                                .join("python")
                                .join("local_funasr.py"),
                        ]
                    })
                    .collect(),
            )
        });
        let python_executable = std::env::var_os("MEETING_DESK_FUNASR_PYTHON")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| {
                let mut runtime_roots = model_dir
                    .ancestors()
                    .take(5)
                    .map(Path::to_path_buf)
                    .collect::<Vec<_>>();
                for root in &roots {
                    if !runtime_roots.contains(root) {
                        runtime_roots.push(root.clone());
                    }
                }
                let candidates = python_runtime_candidates(&runtime_roots);
                let discovered = first_existing(candidates);
                if discovered.exists() {
                    discovered.into_os_string()
                } else {
                    OsString::from("python")
                }
            });
        Self::from_paths(python_executable, script_path, model_dir, model_name)
    }

    /// Validates sibling SenseVoiceSmall and FSMN-VAD directories without exposing paths.
    pub(crate) fn validate_model_directory(model_dir: &Path) -> Result<(), ProviderError> {
        if !model_dir.is_dir() {
            return Err(local_configuration_error(
                "local_funasr_model_missing",
                "所选目录不是有效的 SenseVoiceSmall 模型目录",
            ));
        }
        for file_name in REQUIRED_MODEL_FILES {
            let valid = model_dir
                .join(file_name)
                .metadata()
                .map(|metadata| metadata.is_file() && metadata.len() > 0)
                .unwrap_or(false);
            if !valid {
                return Err(local_configuration_error(
                    "local_funasr_model_incomplete",
                    "模型目录不完整，需要包含 config.yaml、model.pt 和 tokens.json",
                ));
            }
        }
        let vad_model_dir = vad_model_directory(model_dir);
        if !vad_model_dir.is_dir() {
            return Err(local_configuration_error(
                "local_funasr_vad_model_missing",
                "缺少同级 fsmn-vad 模型目录，请将其放在 SenseVoiceSmall 旁边",
            ));
        }
        for file_name in REQUIRED_VAD_MODEL_FILES {
            let valid = vad_model_dir
                .join(file_name)
                .metadata()
                .map(|metadata| metadata.is_file() && metadata.len() > 0)
                .unwrap_or(false);
            if !valid {
                return Err(local_configuration_error(
                    "local_funasr_vad_model_incomplete",
                    "fsmn-vad 模型目录不完整，需要包含 config.yaml、configuration.json、model.pt 和 am.mvn",
                ));
            }
        }
        Ok(())
    }

    /// Creates an adapter with explicit paths for tests and controlled deployments.
    pub fn from_paths(
        python_executable: impl Into<OsString>,
        script_path: impl Into<PathBuf>,
        model_dir: impl Into<PathBuf>,
        model_name: impl Into<String>,
    ) -> Self {
        let model_dir = model_dir.into();
        let vad_model_dir = vad_model_directory(&model_dir);
        Self {
            config: LocalFunAsrConfig {
                python_executable: python_executable.into(),
                script_path: script_path.into(),
                model_dir,
                vad_model_dir,
                model_name: model_name.into(),
            },
        }
    }

    /// Verifies model files and asks Python to load the model without processing audio.
    pub async fn check_runtime(&self, timeout: Duration) -> Result<(), ProviderError> {
        self.validate_files()?;
        let token = super::CancellationToken::new();
        let args = [
            OsString::from("--check"),
            OsString::from("--model-dir"),
            self.config.model_dir.as_os_str().to_owned(),
            OsString::from("--vad-model-dir"),
            self.config.vad_model_dir.as_os_str().to_owned(),
        ];
        self.run_process(&args, &token, timeout, "local_runtime_check_failed")
            .await
    }

    /// Validates local files using stable messages that do not reveal absolute paths.
    fn validate_files(&self) -> Result<(), ProviderError> {
        if !self.config.script_path.is_file() {
            return Err(local_configuration_error(
                "local_funasr_script_missing",
                "本地 FunASR 推理脚本缺失，请重新安装应用",
            ));
        }
        Self::validate_model_directory(&self.config.model_dir)
    }

    /// Copies an ingest-managed file into an isolated temporary directory with cancellation.
    async fn stage_audio(
        &self,
        context: &ProviderCallContext,
        request: &TranscriptionRequest,
        temp_dir: &TempDir,
    ) -> Result<PathBuf, ProviderError> {
        if request.artifact.reference.staging_metadata.byte_length == 0 {
            return Err(ProviderError::input("empty_audio", "音频文件为空"));
        }
        let suffix = media_suffix(&request.artifact.reference.staging_metadata.mime_type)?;
        let audio_path = temp_dir.path().join(format!("input.{suffix}"));
        let source =
            request.artifact.reader.open_readonly().map_err(|_| {
                local_resource_error("local_audio_unavailable", "无法读取本地音频文件")
            })?;
        let mut source = tokio::fs::File::from_std(source);
        let mut destination = tokio::fs::File::create(&audio_path).await.map_err(|_| {
            local_resource_error("local_audio_staging_failed", "无法准备本地转写文件")
        })?;
        let remaining = context.remaining();
        if remaining.is_zero() {
            return Err(local_timeout_error());
        }
        let copied = tokio::select! {
            _ = context.cancellation_token.cancelled() => return Err(ProviderError::cancelled()),
            _ = tokio::time::sleep(remaining) => return Err(local_timeout_error()),
            result = tokio::io::copy(&mut source, &mut destination) => result,
        }
        .map_err(|_| local_resource_error("local_audio_staging_failed", "无法准备本地转写文件"))?;
        destination.flush().await.map_err(|_| {
            local_resource_error("local_audio_staging_failed", "无法准备本地转写文件")
        })?;
        if copied != request.artifact.reference.staging_metadata.byte_length {
            return Err(ProviderError::input(
                "audio_metadata_mismatch",
                "音频文件已发生变化，请重新导入",
            ));
        }
        Ok(audio_path)
    }

    /// Executes the trusted inference script and terminates it on cancellation or timeout.
    async fn run_process(
        &self,
        args: &[OsString],
        token: &super::CancellationToken,
        timeout: Duration,
        fallback_code: &'static str,
    ) -> Result<(), ProviderError> {
        if timeout.is_zero() {
            return Err(local_timeout_error());
        }
        let mut command = Command::new(&self.config.python_executable);
        command
            .arg(&self.config.script_path)
            .args(args)
            .env("PYTHONUTF8", "1")
            .env("HF_HUB_OFFLINE", "1")
            .env("MODELSCOPE_OFFLINE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(windows_subprocess_creation_flags());
        let mut child = command.spawn().map_err(|_| {
            local_configuration_error(
                "local_funasr_python_missing",
                "内置 FunASR 运行环境缺失，请重新安装应用",
            )
        })?;
        let status = tokio::select! {
            _ = token.cancelled() => {
                let _ = child.kill().await;
                return Err(ProviderError::cancelled());
            }
            _ = tokio::time::sleep(timeout) => {
                let _ = child.kill().await;
                return Err(local_timeout_error());
            }
            result = child.wait() => result.map_err(|_| {
                local_resource_error(fallback_code, "本地 FunASR 进程执行失败")
            })?,
        };
        if status.success() {
            return Ok(());
        }
        Err(match status.code() {
            Some(20) => local_configuration_error(
                "local_funasr_dependency_missing",
                "内置 Python 环境缺少 FunASR 依赖，请重新安装应用",
            ),
            Some(21) => local_configuration_error(
                "local_funasr_model_load_failed",
                "本地 SenseVoiceSmall 或 FSMN-VAD 模型加载失败，请检查模型文件",
            ),
            Some(22) => ProviderError::input(
                "local_audio_decode_failed",
                "无法解码该媒体文件，请检查格式或 FFmpeg 环境",
            ),
            _ => local_resource_error(fallback_code, "本地语音转写失败"),
        })
    }

    /// Parses the bounded local JSON result into the provider-neutral transcript contract.
    async fn read_output(
        &self,
        output_path: &Path,
        started_at: chrono::DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<Transcript, ProviderError> {
        let metadata = tokio::fs::metadata(output_path).await.map_err(|_| {
            local_resource_error("local_funasr_output_missing", "本地转写未生成结果")
        })?;
        if metadata.len() == 0 || metadata.len() > MAX_OUTPUT_BYTES {
            return Err(ProviderError::protocol(
                "invalid_local_funasr_output",
                "本地转写结果大小无效",
            ));
        }
        let bytes = tokio::fs::read(output_path).await.map_err(|_| {
            local_resource_error("local_funasr_output_unavailable", "无法读取本地转写结果")
        })?;
        let output = serde_json::from_slice::<LocalInferenceOutput>(&bytes).map_err(|_| {
            ProviderError::protocol("invalid_local_funasr_output", "本地转写结果格式无效")
        })?;
        if output.text.trim().is_empty() {
            return Err(ProviderError::input(
                "empty_transcript",
                "本地模型未检测到可转写语音",
            ));
        }
        let segments = output
            .segments
            .into_iter()
            .enumerate()
            .filter(|(_, segment)| !segment.text.trim().is_empty())
            .map(|(index, segment)| TranscriptSegment {
                id: format!("s{:04}", index + 1),
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                speaker_label: None,
                text: segment.text,
                confidence: None,
            })
            .collect();
        Ok(Transcript {
            schema_version: "1".to_string(),
            text: output.text,
            language: output.language,
            duration_ms,
            segments,
            provider_metadata: ProviderMetadata {
                provider_id: "local_funasr".to_string(),
                adapter_id: ADAPTER_ID.to_string(),
                adapter_version: ADAPTER_VERSION.to_string(),
                model: self.config.model_name.clone(),
                remote_request_id: None,
                started_at,
                completed_at: Utc::now(),
            },
        })
    }
}

#[async_trait]
impl TranscriptionProvider for LocalFunAsrProvider {
    /// Declares local replay, cancellation, and VAD-derived segment timestamp behavior.
    fn capabilities(&self) -> TranscriptionCapabilities {
        TranscriptionCapabilities {
            evidence: CapabilityEvidence::Verified,
            accepted_media_types: vec![
                "audio/mpeg".to_string(),
                "audio/wav".to_string(),
                "audio/mp4".to_string(),
                "video/mp4".to_string(),
                "video/quicktime".to_string(),
            ],
            max_audio_bytes: None,
            max_duration_ms: None,
            supports_async_jobs: false,
            supports_timestamps: true,
            supports_speaker_labels: false,
            supports_confidence: false,
            supports_remote_cancel: false,
            supports_remote_urls: false,
            replay_safety: ReplaySafety::VerifiedAlwaysSafe,
        }
    }

    /// Stages one managed offline file and runs SenseVoiceSmall entirely on this computer.
    async fn transcribe(
        &self,
        context: &ProviderCallContext,
        request: TranscriptionRequest,
        _credential: Option<&ProviderCredential>,
    ) -> Result<Transcript, ProviderError> {
        self.validate_files()?;
        if context.cancellation_token.is_cancelled() {
            return Err(ProviderError::cancelled());
        }
        let started_at = Utc::now();
        let duration_ms = request.artifact.reference.staging_metadata.duration_ms;
        let temp_dir = tempfile::Builder::new()
            .prefix("meeting-desk-local-asr-")
            .tempdir()
            .map_err(|_| {
                local_resource_error("local_temp_unavailable", "无法创建本地转写临时目录")
            })?;
        let audio_path = self.stage_audio(context, &request, &temp_dir).await?;
        let output_path = temp_dir.path().join("result.json");
        let language = request
            .options
            .language_hint
            .as_deref()
            .filter(|value| matches!(*value, "zh" | "en" | "yue" | "ja" | "ko"))
            .unwrap_or("auto");
        let args = vec![
            OsString::from("--model-dir"),
            self.config.model_dir.as_os_str().to_owned(),
            OsString::from("--vad-model-dir"),
            self.config.vad_model_dir.as_os_str().to_owned(),
            OsString::from("--audio"),
            audio_path.as_os_str().to_owned(),
            OsString::from("--output"),
            output_path.as_os_str().to_owned(),
            OsString::from("--language"),
            OsString::from(language),
        ];
        self.run_process(
            &args,
            &context.cancellation_token,
            context.remaining(),
            "local_funasr_inference_failed",
        )
        .await?;
        self.read_output(&output_path, started_at, duration_ms)
            .await
    }
}

/// Returns candidate roots for development, installed resources, and explicit overrides.
fn discovery_roots() -> Vec<PathBuf> {
    let current_dir = std::env::current_dir().ok();
    let current_executable = std::env::current_exe().ok();
    let local_app_data = environment_path("LOCALAPPDATA");
    discovery_roots_from(
        current_dir.as_deref(),
        current_executable.as_deref(),
        local_app_data.as_deref(),
    )
}

/// Builds stable discovery roots with installed app data ahead of development locations.
fn discovery_roots_from(
    current_dir: Option<&Path>,
    current_executable: Option<&Path>,
    local_app_data: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(local_app_data) = local_app_data {
        roots.push(local_app_data.join(APP_DATA_DIRECTORY));
    }
    if let Some(current) = current_dir {
        for root in current.ancestors().take(5) {
            let root = root.to_path_buf();
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    if let Some(executable) = current_executable {
        if let Some(parent) = executable.parent() {
            for root in parent.ancestors().take(5) {
                let root = root.to_path_buf();
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
        }
    }
    roots
}

/// Reads one non-empty path override without logging its value.
fn environment_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Resolves the fixed FSMN-VAD directory beside the selected SenseVoice model.
fn vad_model_directory(model_dir: &Path) -> PathBuf {
    model_dir
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(VAD_MODEL_DIRECTORY_NAME)
}

/// Selects the first existing candidate, falling back deterministically when none exist.
fn first_existing(candidates: Vec<PathBuf>) -> PathBuf {
    candidates
        .iter()
        .find(|candidate| candidate.exists())
        .cloned()
        .or_else(|| candidates.into_iter().next())
        .unwrap_or_default()
}

/// Builds runtime candidates with bundled Python ahead of development virtual environments.
fn python_runtime_candidates(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .map(|root| root.join("runtime").join("python").join("python.exe"))
        .chain(
            roots
                .iter()
                .map(|root| root.join(".venv").join("Scripts").join("python.exe")),
        )
        .collect()
}

/// Maps a validated ingest MIME type to a safe temporary filename suffix.
fn media_suffix(mime_type: &str) -> Result<&'static str, ProviderError> {
    match mime_type {
        "audio/mpeg" => Ok("mp3"),
        "audio/wav" | "audio/x-wav" => Ok("wav"),
        "audio/mp4" => Ok("m4a"),
        "video/mp4" => Ok("mp4"),
        "video/quicktime" => Ok("mov"),
        _ => Err(ProviderError::input(
            "unsupported_audio",
            "本地模型不支持该媒体格式",
        )),
    }
}

/// Creates a sanitized local configuration error.
fn local_configuration_error(code: &'static str, message: &'static str) -> ProviderError {
    ProviderError::configuration(code, message)
}

/// Creates a sanitized local resource error without paths or process output.
fn local_resource_error(code: &'static str, message: &'static str) -> ProviderError {
    ProviderError::new(
        code,
        ProviderErrorCategory::LocalResource,
        false,
        true,
        message,
        None,
        None,
        OperationOutcome::Failed,
    )
}

/// Creates a local inference timeout error with provider-neutral semantics.
fn local_timeout_error() -> ProviderError {
    ProviderError::new(
        "local_funasr_timeout",
        ProviderErrorCategory::Timeout,
        false,
        true,
        "本地语音转写超过设置的超时时间",
        None,
        None,
        OperationOutcome::Unknown,
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use chrono::Utc;

    use super::*;
    use crate::ingest::{AudioArtifactRef, AudioSourceKind, StagingMetadata};
    use crate::providers::{ManagedAudioArtifact, TranscriptionOptions};

    /// Creates the minimum valid SenseVoice marker files used by subprocess tests.
    fn create_sensevoice_model_fixture(root: &Path) {
        fs::create_dir_all(root).expect("model fixture directory");
        for name in REQUIRED_MODEL_FILES {
            fs::write(root.join(name), b"fixture").expect("model fixture file");
        }
    }

    /// Creates the minimum local FSMN-VAD files expected beside SenseVoiceSmall.
    fn create_vad_model_fixture(model_dir: &Path) {
        let vad_dir = model_dir.parent().expect("model parent").join("fsmn-vad");
        fs::create_dir_all(&vad_dir).expect("VAD fixture directory");
        for name in ["config.yaml", "configuration.json", "model.pt", "am.mvn"] {
            fs::write(vad_dir.join(name), b"fixture").expect("VAD fixture file");
        }
    }

    /// Creates a complete local ASR bundle with sibling recognition and VAD models.
    fn create_model_fixture(root: &Path) {
        create_sensevoice_model_fixture(root);
        create_vad_model_fixture(root);
    }

    /// Creates a managed audio request without exposing its source path to the provider DTO.
    fn request_for(path: PathBuf, bytes: u64) -> TranscriptionRequest {
        let artifact = AudioArtifactRef {
            id: "artifact-local-test".to_string(),
            import_batch_id: None,
            source_kind: AudioSourceKind::UserSelectedFile,
            staging_metadata: StagingMetadata {
                mime_type: "audio/wav".to_string(),
                byte_length: bytes,
                duration_ms: Some(1_000),
                sha256: None,
                validated_at: Utc::now(),
            },
        };
        TranscriptionRequest {
            artifact: ManagedAudioArtifact::new(
                artifact,
                Arc::new(move || std::fs::File::open(&path)),
            ),
            options: TranscriptionOptions::default(),
        }
    }

    /// Verifies that local filesystem paths never appear in Debug output.
    #[test]
    fn debug_redacts_local_paths() {
        let provider = LocalFunAsrProvider::from_paths(
            "private-python.exe",
            "private-script.py",
            "private-model",
            "SenseVoiceSmall",
        );
        let debug = format!("{:?}", provider.config);
        assert!(!debug.contains("private-python"));
        assert!(!debug.contains("private-script"));
        assert!(!debug.contains("private-model"));
    }

    /// Verifies installed builds prefer the writable application data root.
    #[test]
    fn discovery_roots_prioritize_local_app_data() {
        let local_app_data = Path::new(r"C:\Users\tester\AppData\Local");
        let roots = discovery_roots_from(
            Some(Path::new(r"D:\work\funasr-demo")),
            Some(Path::new(r"D:\Program Files\MeetingDesk\meeting-desk.exe")),
            Some(local_app_data),
        );

        assert_eq!(
            roots.first(),
            Some(&local_app_data.join("com.internal.meetingdesk")),
        );
    }

    /// Verifies a project virtual environment remains the fallback when no bundle exists.
    #[test]
    fn nearby_virtualenv_is_used_when_bundled_runtime_is_missing() {
        let fixture = tempfile::tempdir().expect("fixture");
        let python = fixture
            .path()
            .join(".venv")
            .join("Scripts")
            .join("python.exe");
        std::fs::create_dir_all(python.parent().expect("python parent")).expect("venv");
        std::fs::write(&python, b"fixture").expect("python executable");

        let candidates = python_runtime_candidates(&[fixture.path().to_path_buf()]);

        assert_eq!(candidates.get(1), Some(&python));
        assert_eq!(first_existing(candidates), python);
    }

    /// Verifies installed builds prefer the bundled Python runtime over development venvs.
    #[test]
    fn bundled_python_is_first_runtime_candidate() {
        let root = PathBuf::from(r"D:\Program Files\MeetingDesk");
        let candidates = python_runtime_candidates(std::slice::from_ref(&root));

        assert_eq!(
            candidates.first(),
            Some(&root.join("runtime").join("python").join("python.exe")),
        );
        assert_eq!(
            candidates.get(1),
            Some(&root.join(".venv").join("Scripts").join("python.exe")),
        );
    }

    /// Verifies the runtime check passes the validated model directory to Python for loading.
    #[tokio::test]
    async fn runtime_check_passes_asr_and_vad_model_directories() {
        let fixture = tempfile::tempdir().expect("fixture directory");
        let model_dir = fixture.path().join("model");
        create_model_fixture(&model_dir);
        let script_path = fixture.path().join("check_funasr.py");
        fs::write(
            &script_path,
            r#"import argparse, pathlib
p = argparse.ArgumentParser()
p.add_argument('--check', action='store_true', required=True)
p.add_argument('--model-dir', required=True)
p.add_argument('--vad-model-dir', required=True)
a = p.parse_args()
model_dir = pathlib.Path(a.model_dir)
vad_dir = pathlib.Path(a.vad_model_dir)
asr_required = ('config.yaml', 'model.pt', 'tokens.json')
vad_required = ('config.yaml', 'configuration.json', 'model.pt', 'am.mvn')
valid = all((model_dir / name).is_file() for name in asr_required)
valid = valid and all((vad_dir / name).is_file() for name in vad_required)
raise SystemExit(0 if a.check and valid else 2)
"#,
        )
        .expect("script fixture");
        let provider =
            LocalFunAsrProvider::from_paths("python", script_path, model_dir, "SenseVoiceSmall");

        provider
            .check_runtime(Duration::from_secs(10))
            .await
            .expect("runtime check should receive the model directory");
    }

    /// Verifies the real subprocess boundary with a deterministic offline Python fixture.
    #[tokio::test]
    async fn subprocess_result_is_normalized_without_credentials() {
        let fixture = tempfile::tempdir().expect("fixture directory");
        let model_dir = fixture.path().join("model");
        create_model_fixture(&model_dir);
        let audio_path = fixture.path().join("source.wav");
        fs::write(&audio_path, b"safe-audio-fixture").expect("audio fixture");
        let script_path = fixture.path().join("fake_funasr.py");
        fs::write(
            &script_path,
            r#"import argparse, json
p = argparse.ArgumentParser()
p.add_argument('--model-dir')
p.add_argument('--vad-model-dir')
p.add_argument('--audio')
p.add_argument('--output')
p.add_argument('--language')
p.add_argument('--check', action='store_true')
a = p.parse_args()
if not a.check:
    with open(a.output, 'w', encoding='utf-8') as f:
        json.dump({'text':'first paragraph\nsecond paragraph','language':'zh','segments':[{'start_ms':120,'end_ms':930,'text':'first paragraph'},{'start_ms':1100,'end_ms':2300,'text':'second paragraph'}]}, f)
"#,
        )
        .expect("script fixture");
        let provider =
            LocalFunAsrProvider::from_paths("python", script_path, model_dir, "SenseVoiceSmall");
        let context = ProviderCallContext::with_timeout(
            "task-local-test",
            "operation-local-test",
            super::super::CancellationToken::new(),
            Duration::from_secs(10),
        );
        let transcript = provider
            .transcribe(
                &context,
                request_for(audio_path, b"safe-audio-fixture".len() as u64),
                None,
            )
            .await
            .expect("local subprocess transcript");
        assert_eq!(transcript.text, "first paragraph\nsecond paragraph");
        assert_eq!(transcript.segments.len(), 2);
        assert_eq!(transcript.segments[0].start_ms, Some(120));
        assert_eq!(transcript.segments[1].end_ms, Some(2_300));
        assert_eq!(transcript.provider_metadata.provider_id, "local_funasr");
        assert_eq!(transcript.provider_metadata.adapter_version, "3");
        assert!(!format!("{transcript:?}").contains("first paragraph"));
    }

    /// 验证本地适配器只声明已经由 VAD 句段结果提供的能力。
    #[test]
    fn capabilities_include_vad_segment_timestamps() {
        let provider = LocalFunAsrProvider::from_paths(
            "python",
            "local_funasr.py",
            "model",
            "SenseVoiceSmall",
        );

        let capabilities = provider.capabilities();
        assert!(capabilities.supports_timestamps);
        assert!(!capabilities.supports_speaker_labels);
        assert!(!capabilities.supports_confidence);
    }

    /// 验证 Windows 本地推理子进程使用禁止创建控制台窗口的启动标志。
    #[cfg(windows)]
    #[test]
    fn subprocess_uses_no_window_creation_flag() {
        assert_eq!(windows_subprocess_creation_flags(), 0x0800_0000);
    }

    /// Verifies missing model files fail before starting Python.
    #[tokio::test]
    async fn missing_model_is_a_sanitized_configuration_error() {
        let fixture = tempfile::tempdir().expect("fixture directory");
        let script_path = fixture.path().join("fake_funasr.py");
        fs::write(&script_path, "raise SystemExit(0)").expect("script fixture");
        let provider = LocalFunAsrProvider::from_paths(
            "python",
            script_path,
            fixture.path().join("missing-model"),
            "SenseVoiceSmall",
        );
        let error = provider
            .check_runtime(Duration::from_secs(1))
            .await
            .expect_err("missing model must fail");
        assert_eq!(error.code, "local_funasr_model_missing");
        assert!(error.safe_message.contains("模型目录"));
        assert!(!error
            .safe_message
            .contains(fixture.path().to_string_lossy().as_ref()));
    }

    /// Verifies a valid SenseVoice directory is rejected until its sibling VAD model exists.
    #[test]
    fn missing_vad_model_is_a_sanitized_configuration_error() {
        let fixture = tempfile::tempdir().expect("fixture directory");
        let model_dir = fixture.path().join("SenseVoiceSmall");
        create_sensevoice_model_fixture(&model_dir);

        let error = LocalFunAsrProvider::validate_model_directory(&model_dir)
            .expect_err("missing sibling VAD model must fail");

        assert_eq!(error.code, "local_funasr_vad_model_missing");
        assert!(error.safe_message.contains("fsmn-vad"));
        assert!(!error
            .safe_message
            .contains(fixture.path().to_string_lossy().as_ref()));
    }

    /// Verifies cancellation terminates an active local inference subprocess.
    #[tokio::test]
    async fn cancellation_stops_local_subprocess() {
        let fixture = tempfile::tempdir().expect("fixture directory");
        let model_dir = fixture.path().join("model");
        create_model_fixture(&model_dir);
        let audio_path = fixture.path().join("source.wav");
        fs::write(&audio_path, b"safe-audio-fixture").expect("audio fixture");
        let script_path = fixture.path().join("slow_funasr.py");
        fs::write(&script_path, "import time\ntime.sleep(30)\n").expect("script fixture");
        let provider =
            LocalFunAsrProvider::from_paths("python", script_path, model_dir, "SenseVoiceSmall");
        let token = super::super::CancellationToken::new();
        let cancel_token = token.clone();
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel_token.cancel();
        });
        let context = ProviderCallContext::with_timeout(
            "task-cancel-test",
            "operation-cancel-test",
            token,
            Duration::from_secs(10),
        );
        let error = provider
            .transcribe(
                &context,
                request_for(audio_path, b"safe-audio-fixture".len() as u64),
                None,
            )
            .await
            .expect_err("cancelled local process must fail");
        cancel_task.await.expect("cancel task");
        assert_eq!(error.code, "cancelled");
    }
}
