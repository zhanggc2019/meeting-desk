use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::{
    ImportItemStatus, ImportRequest, ImportSelectionMode, IngestErrorCode, IngestPolicy,
    OfflineAudioImporter,
};

const TEST_MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const TEST_MAX_BATCH_BYTES: u64 = 128 * 1024 * 1024;

/// 创建使用独立临时 staging 目录的测试导入器。
fn create_importer(staging: &TempDir) -> OfflineAudioImporter {
    let policy = IngestPolicy::new(TEST_MAX_FILE_BYTES, 16, TEST_MAX_BATCH_BYTES)
        .expect("test policy must be valid");
    OfflineAudioImporter::new(staging.path(), policy).expect("test importer must be created")
}

/// 创建单文件导入请求。
fn single_request() -> ImportRequest {
    ImportRequest {
        selection_mode: ImportSelectionMode::Single,
    }
}

/// 创建批量导入请求。
fn batch_request() -> ImportRequest {
    ImportRequest {
        selection_mode: ImportSelectionMode::Batch,
    }
}

/// 将测试夹具写入临时目录，不输出文件内容。
fn write_fixture(root: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = root.path().join(name);
    fs::write(&path, bytes).expect("fixture must be writable");
    path
}

/// 生成 100 毫秒、16 kHz、单声道、16-bit PCM 的最小合法 WAV。
fn valid_wav() -> Vec<u8> {
    let pcm = vec![0u8; 3_200];
    let riff_size = 36u32 + pcm.len() as u32;
    let mut bytes = Vec::with_capacity(44 + pcm.len());
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
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

/// 生成两个连续 MPEG-1 Layer III frame 的最小结构夹具。
fn valid_mp3() -> Vec<u8> {
    const FRAME_LENGTH: usize = 417;
    const HEADER: [u8; 4] = [0xff, 0xfb, 0x90, 0x64];
    let mut bytes = vec![0u8; FRAME_LENGTH * 2];
    bytes[0..4].copy_from_slice(&HEADER);
    bytes[FRAME_LENGTH..FRAME_LENGTH + 4].copy_from_slice(&HEADER);
    bytes
}

/// 创建一个 ISO BMFF box。
fn iso_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = u32::try_from(8usize + payload.len()).expect("test box must fit in u32");
    let mut bytes = Vec::with_capacity(size as usize);
    bytes.extend_from_slice(&size.to_be_bytes());
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(payload);
    bytes
}

/// 生成包含单个 mp4a 音频轨、时长两秒的最小 M4A 结构夹具。
fn valid_m4a() -> Vec<u8> {
    let mut ftyp_payload = Vec::new();
    ftyp_payload.extend_from_slice(b"M4A ");
    ftyp_payload.extend_from_slice(&0u32.to_be_bytes());
    ftyp_payload.extend_from_slice(b"isom");

    let mut mdhd_payload = vec![0u8; 12];
    mdhd_payload.extend_from_slice(&16_000u32.to_be_bytes());
    mdhd_payload.extend_from_slice(&32_000u32.to_be_bytes());
    mdhd_payload.extend_from_slice(&0u32.to_be_bytes());

    let mut hdlr_payload = vec![0u8; 8];
    hdlr_payload.extend_from_slice(b"soun");
    hdlr_payload.extend_from_slice(&[0u8; 12]);

    let sample_entry = iso_box(b"mp4a", &[]);
    let mut stsd_payload = vec![0u8; 4];
    stsd_payload.extend_from_slice(&1u32.to_be_bytes());
    stsd_payload.extend_from_slice(&sample_entry);

    let stsd = iso_box(b"stsd", &stsd_payload);
    let stbl = iso_box(b"stbl", &stsd);
    let minf = iso_box(b"minf", &stbl);
    let mut mdia_payload = Vec::new();
    mdia_payload.extend_from_slice(&iso_box(b"mdhd", &mdhd_payload));
    mdia_payload.extend_from_slice(&iso_box(b"hdlr", &hdlr_payload));
    mdia_payload.extend_from_slice(&minf);
    let mdia = iso_box(b"mdia", &mdia_payload);
    let trak = iso_box(b"trak", &mdia);
    let moov = iso_box(b"moov", &trak);

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&iso_box(b"ftyp", &ftyp_payload));
    bytes.extend_from_slice(&iso_box(b"mdat", &[1, 2, 3, 4]));
    bytes.extend_from_slice(&moov);
    bytes
}

/// 返回结果中的首个公开错误码。
fn first_error_code(response: &super::ImportBatchResponse) -> IngestErrorCode {
    response.items[0]
        .error
        .as_ref()
        .expect("result must contain an error")
        .code
}

/// 验证 WAV 被复制到受管目录、计算哈希并保留源文件内容。
#[test]
fn imports_valid_wav_without_modifying_source() {
    let source_root = TempDir::new().expect("source tempdir must be created");
    let staging = TempDir::new().expect("staging tempdir must be created");
    let source_bytes = valid_wav();
    let source = write_fixture(&source_root, "meeting.wav", &source_bytes);
    let importer = create_importer(&staging);

    let response = importer.import_selected_files(single_request(), vec![source.clone()]);

    assert_eq!(response.items.len(), 1);
    assert_eq!(response.items[0].status, ImportItemStatus::Ready);
    let artifact = response.items[0]
        .artifact
        .as_ref()
        .expect("ready result must include artifact");
    assert_eq!(artifact.staging_metadata.mime_type, "audio/wav");
    assert_eq!(
        artifact.staging_metadata.byte_length,
        source_bytes.len() as u64
    );
    assert_eq!(artifact.staging_metadata.duration_ms, Some(100));
    assert!(artifact.staging_metadata.sha256.is_some());
    assert_eq!(
        fs::read(source).expect("source must remain readable"),
        source_bytes
    );

    let mut managed = Vec::new();
    importer
        .open_artifact_readonly(&artifact.id)
        .expect("artifact must be openable")
        .read_to_end(&mut managed)
        .expect("artifact must be readable");
    assert_eq!(managed, source_bytes);
}

/// 验证最小 MP3 夹具可以通过连续 frame 校验。
#[test]
fn imports_valid_mp3_fixture() {
    let source_root = TempDir::new().expect("source tempdir must be created");
    let staging = TempDir::new().expect("staging tempdir must be created");
    let source = write_fixture(&source_root, "meeting.MP3", &valid_mp3());
    let importer = create_importer(&staging);

    let response = importer.import_selected_files(single_request(), vec![source]);

    assert_eq!(response.items[0].status, ImportItemStatus::Ready);
    let artifact = response.items[0]
        .artifact
        .as_ref()
        .expect("ready result must include artifact");
    assert_eq!(artifact.staging_metadata.mime_type, "audio/mpeg");
    assert_eq!(artifact.staging_metadata.duration_ms, None);
}

/// 验证最小 M4A 夹具可以识别 mp4a 音频轨和时长。
#[test]
fn imports_valid_m4a_fixture() {
    let source_root = TempDir::new().expect("source tempdir must be created");
    let staging = TempDir::new().expect("staging tempdir must be created");
    let source = write_fixture(&source_root, "meeting.m4a", &valid_m4a());
    let importer = create_importer(&staging);

    let response = importer.import_selected_files(single_request(), vec![source]);

    assert_eq!(response.items[0].status, ImportItemStatus::Ready);
    let artifact = response.items[0]
        .artifact
        .as_ref()
        .expect("ready result must include artifact");
    assert_eq!(artifact.staging_metadata.mime_type, "audio/mp4");
    assert_eq!(artifact.staging_metadata.duration_ms, Some(2_000));
}

/// 验证空音频返回稳定错误且不留下 part 文件。
#[test]
fn rejects_empty_audio_and_cleans_part_file() {
    let source_root = TempDir::new().expect("source tempdir must be created");
    let staging = TempDir::new().expect("staging tempdir must be created");
    let source = write_fixture(&source_root, "empty.wav", &[]);
    let importer = create_importer(&staging);

    let response = importer.import_selected_files(single_request(), vec![source]);

    assert_eq!(first_error_code(&response), IngestErrorCode::EmptyAudio);
    assert_eq!(
        fs::read_dir(staging.path())
            .expect("staging must exist")
            .count(),
        0
    );
}

/// 验证损坏但带支持扩展名的文件被拒绝。
#[test]
fn rejects_corrupt_audio() {
    let source_root = TempDir::new().expect("source tempdir must be created");
    let staging = TempDir::new().expect("staging tempdir must be created");
    let source = write_fixture(&source_root, "broken.mp3", b"not an mp3 stream");
    let importer = create_importer(&staging);

    let response = importer.import_selected_files(single_request(), vec![source]);

    assert_eq!(first_error_code(&response), IngestErrorCode::CorruptAudio);
    assert_eq!(
        fs::read_dir(staging.path())
            .expect("staging must exist")
            .count(),
        0
    );
}

/// 验证真实容器与文件扩展名不一致时返回伪装文件错误。
#[test]
fn rejects_extension_content_mismatch() {
    let source_root = TempDir::new().expect("source tempdir must be created");
    let staging = TempDir::new().expect("staging tempdir must be created");
    let source = write_fixture(&source_root, "disguised.mp3", &valid_wav());
    let importer = create_importer(&staging);

    let response = importer.import_selected_files(single_request(), vec![source]);

    assert_eq!(
        first_error_code(&response),
        IngestErrorCode::ExtensionContentMismatch
    );
}

/// 验证单文件上限在复制前生效。
#[test]
fn rejects_file_over_configured_limit() {
    let source_root = TempDir::new().expect("source tempdir must be created");
    let staging = TempDir::new().expect("staging tempdir must be created");
    let source = write_fixture(&source_root, "meeting.wav", &valid_wav());
    let policy = IngestPolicy::new(100, 2, 10_000).expect("test policy must be valid");
    let importer =
        OfflineAudioImporter::new(staging.path(), policy).expect("test importer must be created");

    let response = importer.import_selected_files(single_request(), vec![source]);

    assert_eq!(first_error_code(&response), IngestErrorCode::FileTooLarge);
}

/// 验证批量导入逐文件返回成功或失败，不因单项错误中止整个批次。
#[test]
fn batch_import_reports_per_file_results() {
    let source_root = TempDir::new().expect("source tempdir must be created");
    let staging = TempDir::new().expect("staging tempdir must be created");
    let good = write_fixture(&source_root, "good.wav", &valid_wav());
    let bad = write_fixture(&source_root, "bad.m4a", b"broken");
    let importer = create_importer(&staging);

    let response = importer.import_selected_files(batch_request(), vec![good, bad]);

    assert_eq!(response.items.len(), 2);
    assert_eq!(response.items[0].selection_index, 0);
    assert_eq!(response.items[0].status, ImportItemStatus::Ready);
    assert_eq!(response.items[1].selection_index, 1);
    assert_eq!(response.items[1].status, ImportItemStatus::Failed);
    assert_eq!(
        response.items[1]
            .error
            .as_ref()
            .expect("failed item must include error")
            .code,
        IngestErrorCode::CorruptAudio
    );
}

/// 验证相同内容不会创建第二份受管音频。
#[test]
fn deduplicates_identical_audio_by_streaming_hash() {
    let source_root = TempDir::new().expect("source tempdir must be created");
    let staging = TempDir::new().expect("staging tempdir must be created");
    let bytes = valid_wav();
    let first = write_fixture(&source_root, "first.wav", &bytes);
    let second = write_fixture(&source_root, "second.wav", &bytes);
    let importer = create_importer(&staging);

    let response = importer.import_selected_files(batch_request(), vec![first, second]);

    assert_eq!(response.items[0].status, ImportItemStatus::Ready);
    assert_eq!(response.items[1].status, ImportItemStatus::Duplicate);
    assert_eq!(
        response.items[1].duplicate_of_artifact_id,
        response.items[0]
            .artifact
            .as_ref()
            .map(|artifact| artifact.id.clone())
    );
    assert_eq!(
        fs::read_dir(staging.path())
            .expect("staging must exist")
            .count(),
        1
    );
}

/// 验证 IPC JSON 不包含源路径、文件名或完整 SHA-256。
#[test]
fn serialized_response_omits_sensitive_local_metadata() {
    let source_root = TempDir::new().expect("source tempdir must be created");
    let staging = TempDir::new().expect("staging tempdir must be created");
    let source = write_fixture(&source_root, "private-meeting.wav", &valid_wav());
    let importer = create_importer(&staging);

    let response = importer.import_selected_files(single_request(), vec![source]);
    let json = serde_json::to_string(&response).expect("response must serialize");

    assert!(!json.contains("private-meeting"));
    assert!(!json.contains("sha256"));
    assert!(!json.contains("staging_root"));
}

/// 验证项目根目录的真实 MP3（若存在）能够通过离线导入。
#[test]
fn imports_repository_real_mp3_when_present() {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri must have repository parent");
    let source = repository_root.join("AI视频批量生产与模板优化会议.mp3");
    if !source.is_file() {
        return;
    }

    let staging = TempDir::new().expect("staging tempdir must be created");
    let importer = create_importer(&staging);
    let response = importer.import_selected_files(single_request(), vec![source]);

    assert_eq!(response.items.len(), 1);
    assert_eq!(response.items[0].status, ImportItemStatus::Ready);
    assert_eq!(
        response.items[0]
            .artifact
            .as_ref()
            .expect("ready result must include artifact")
            .staging_metadata
            .mime_type,
        "audio/mpeg"
    );
}

/// 验证只读 artifact 查询对未知 ID 返回稳定错误。
#[test]
fn reports_unknown_artifact_id() {
    let staging = TempDir::new().expect("staging tempdir must be created");
    let importer = create_importer(&staging);

    let error = importer
        .open_artifact_readonly("missing")
        .expect_err("unknown artifact must fail");

    assert_eq!(error.code(), IngestErrorCode::ArtifactNotFound);
}

/// 验证显式释放只删除受管副本并保留用户源文件。
#[test]
fn releases_managed_artifact_without_deleting_source() {
    let source_root = TempDir::new().expect("source tempdir must be created");
    let staging = TempDir::new().expect("staging tempdir must be created");
    let source = write_fixture(&source_root, "meeting.wav", &valid_wav());
    let importer = create_importer(&staging);
    let response = importer.import_selected_files(single_request(), vec![source.clone()]);
    let artifact_id = response.items[0]
        .artifact
        .as_ref()
        .expect("ready artifact")
        .id
        .clone();

    assert!(importer
        .remove_artifact(&artifact_id)
        .expect("remove artifact"));
    assert!(source.is_file());
    assert_eq!(
        fs::read_dir(staging.path())
            .expect("staging exists")
            .count(),
        0
    );
    assert_eq!(
        importer
            .open_artifact_readonly(&artifact_id)
            .expect_err("released artifact must be unavailable")
            .code(),
        IngestErrorCode::ArtifactNotFound
    );
}

/// 验证启动清理仅移除受管目录中的直接文件。
#[test]
fn clears_staged_files_on_startup_recovery() {
    let source_root = TempDir::new().expect("source tempdir must be created");
    let staging = TempDir::new().expect("staging tempdir must be created");
    let source = write_fixture(&source_root, "meeting.wav", &valid_wav());
    let importer = create_importer(&staging);
    let response = importer.import_selected_files(single_request(), vec![source]);
    assert_eq!(response.items[0].status, ImportItemStatus::Ready);

    assert_eq!(importer.clear_staged_files().expect("clear staging"), 1);
    assert_eq!(
        fs::read_dir(staging.path())
            .expect("staging exists")
            .count(),
        0
    );
}

/// 验证类型定义没有意外要求源路径 DTO。
#[test]
fn import_request_only_contains_selection_mode() {
    let json = serde_json::to_value(single_request()).expect("request must serialize");

    assert_eq!(json, serde_json::json!({ "selectionMode": "single" }));
}

/// 验证测试辅助代码没有误依赖绝对项目路径。
#[test]
fn real_asset_path_is_derived_from_manifest_directory() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    assert!(manifest.ends_with("src-tauri"));
}
