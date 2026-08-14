use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::error::IngestError;

const MP3_SCAN_BYTES: u64 = 256 * 1024;
const MAX_ISO_BOXES_PER_LEVEL: usize = 16_384;

/// 导入模块支持的音频或视频容器类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    Wav,
    Mp3,
    M4a,
    Mp4,
    Mov,
}

impl AudioFormat {
    /// 返回该容器的规范文件扩展名。
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
            Self::M4a => "m4a",
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
        }
    }

    /// 返回跨 Provider 使用的内部规范 MIME。
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Wav => "audio/wav",
            Self::Mp3 => "audio/mpeg",
            Self::M4a => "audio/mp4",
            Self::Mp4 => "video/mp4",
            Self::Mov => "video/quicktime",
        }
    }

    /// 返回文件内容预期使用的底层容器族。
    fn container(self) -> DetectedContainer {
        match self {
            Self::Wav => DetectedContainer::Wav,
            Self::Mp3 => DetectedContainer::Mp3,
            Self::M4a | Self::Mp4 | Self::Mov => DetectedContainer::IsoBmff,
        }
    }
}

/// 文件头可可靠区分的底层容器族。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetectedContainer {
    Wav,
    Mp3,
    IsoBmff,
}

/// 经过结构校验后得到的非敏感音频技术元数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioInspection {
    pub format: AudioFormat,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct IsoBox {
    kind: [u8; 4],
    payload_start: u64,
    end: u64,
}

#[derive(Debug, Clone, Copy)]
struct IsoAudioTrack {
    duration_ms: Option<u64>,
    supported_codec: bool,
}

#[derive(Debug, Clone, Copy)]
struct Mp3FrameInfo {
    byte_length: usize,
    sample_rate: u32,
    samples_per_frame: u32,
}

/// ISO BMFF 中和转写相关的媒体轨类型。
enum IsoMediaTrack {
    Audio(IsoAudioTrack),
    Video,
}

/// 根据大小写不敏感的扩展名确定用户声明的容器类型。
pub fn expected_format(path: &Path) -> Result<AudioFormat, IngestError> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or(IngestError::UnsupportedExtension)?;

    match extension.as_str() {
        "wav" => Ok(AudioFormat::Wav),
        "mp3" => Ok(AudioFormat::Mp3),
        "m4a" => Ok(AudioFormat::M4a),
        "mp4" => Ok(AudioFormat::Mp4),
        "mov" => Ok(AudioFormat::Mov),
        _ => Err(IngestError::UnsupportedExtension),
    }
}

/// 校验暂存文件的真实容器、关键结构和扩展名一致性。
pub fn inspect_audio(path: &Path, expected: AudioFormat) -> Result<AudioInspection, IngestError> {
    let mut file = File::open(path).map_err(|_| IngestError::AudioStorageFailed)?;
    let byte_length = file
        .metadata()
        .map_err(|_| IngestError::AudioStorageFailed)?
        .len();
    if byte_length == 0 {
        return Err(IngestError::EmptyAudio);
    }

    let actual = detect_format(&mut file, byte_length)?;
    if actual != expected.container() {
        return Err(IngestError::ExtensionContentMismatch);
    }

    let duration_ms = match actual {
        DetectedContainer::Wav => validate_wav(&mut file, byte_length)?,
        DetectedContainer::Mp3 => Some(inspect_mp3_duration(&mut file, byte_length)?),
        DetectedContainer::IsoBmff => validate_iso_bmff(&mut file, byte_length)?,
    };

    Ok(AudioInspection {
        format: expected,
        duration_ms,
    })
}

/// 通过受限头部读取和结构探测识别真实容器类型。
fn detect_format(file: &mut File, byte_length: u64) -> Result<DetectedContainer, IngestError> {
    let mut header = [0u8; 12];
    file.seek(SeekFrom::Start(0))
        .map_err(|_| IngestError::CorruptAudio)?;
    let bytes_read = file
        .read(&mut header)
        .map_err(|_| IngestError::CorruptAudio)?;

    if bytes_read >= 12 && &header[0..4] == b"RIFF" && &header[8..12] == b"WAVE" {
        return Ok(DetectedContainer::Wav);
    }
    if bytes_read >= 12 && &header[4..8] == b"ftyp" {
        return Ok(DetectedContainer::IsoBmff);
    }
    if find_mp3_frame(file, byte_length).is_ok() {
        return Ok(DetectedContainer::Mp3);
    }

    Err(IngestError::CorruptAudio)
}

/// 校验 RIFF/WAVE 的 fmt、data chunk 及基础音频参数。
fn validate_wav(file: &mut File, byte_length: u64) -> Result<Option<u64>, IngestError> {
    let mut riff_header = [0u8; 12];
    read_exact_at(file, 0, &mut riff_header)?;
    if &riff_header[0..4] != b"RIFF" || &riff_header[8..12] != b"WAVE" {
        return Err(IngestError::CorruptAudio);
    }

    let declared_size = u32::from_le_bytes(riff_header[4..8].try_into().unwrap()) as u64;
    let riff_end = declared_size
        .checked_add(8)
        .ok_or(IngestError::CorruptAudio)?;
    if riff_end < 12 || riff_end > byte_length {
        return Err(IngestError::CorruptAudio);
    }

    let mut offset = 12u64;
    let mut byte_rate = None;
    let mut data_size = None;
    while offset < riff_end {
        if riff_end - offset < 8 {
            return Err(IngestError::CorruptAudio);
        }
        let mut chunk_header = [0u8; 8];
        read_exact_at(file, offset, &mut chunk_header)?;
        let chunk_size = u32::from_le_bytes(chunk_header[4..8].try_into().unwrap()) as u64;
        let payload_start = offset.checked_add(8).ok_or(IngestError::CorruptAudio)?;
        let payload_end = payload_start
            .checked_add(chunk_size)
            .ok_or(IngestError::CorruptAudio)?;
        if payload_end > riff_end {
            return Err(IngestError::CorruptAudio);
        }

        if &chunk_header[0..4] == b"fmt " {
            if chunk_size < 16 {
                return Err(IngestError::CorruptAudio);
            }
            let mut format = [0u8; 16];
            read_exact_at(file, payload_start, &mut format)?;
            let audio_format = u16::from_le_bytes(format[0..2].try_into().unwrap());
            let channels = u16::from_le_bytes(format[2..4].try_into().unwrap());
            let sample_rate = u32::from_le_bytes(format[4..8].try_into().unwrap());
            let parsed_byte_rate = u32::from_le_bytes(format[8..12].try_into().unwrap());
            let block_align = u16::from_le_bytes(format[12..14].try_into().unwrap());
            let bits_per_sample = u16::from_le_bytes(format[14..16].try_into().unwrap());
            if audio_format == 0
                || channels == 0
                || channels > 32
                || sample_rate == 0
                || parsed_byte_rate == 0
                || block_align == 0
                || bits_per_sample == 0
                || bits_per_sample > 64
            {
                return Err(IngestError::CorruptAudio);
            }
            byte_rate = Some(parsed_byte_rate as u64);
        } else if &chunk_header[0..4] == b"data" {
            if chunk_size == 0 {
                return Err(IngestError::EmptyAudio);
            }
            data_size = Some(chunk_size);
        }

        offset = payload_end
            .checked_add(chunk_size & 1)
            .ok_or(IngestError::CorruptAudio)?;
        if offset > riff_end {
            return Err(IngestError::CorruptAudio);
        }
    }

    let byte_rate = byte_rate.ok_or(IngestError::CorruptAudio)?;
    let data_size = data_size.ok_or(IngestError::CorruptAudio)?;
    let duration_ms = (u128::from(data_size) * 1_000u128) / u128::from(byte_rate);
    u64::try_from(duration_ms)
        .map(Some)
        .map_err(|_| IngestError::CorruptAudio)
}

/// 校验 MP3 连续帧并根据每帧采样数计算总播放时长。
fn inspect_mp3_duration(file: &mut File, byte_length: u64) -> Result<u64, IngestError> {
    let first_offset = find_mp3_frame(file, byte_length)?;
    file.seek(SeekFrom::Start(first_offset))
        .map_err(|_| IngestError::CorruptAudio)?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut consumed_bytes = 0u64;
    let mut frame_count = 0u64;
    let mut duration_ns = 0u128;
    loop {
        let mut header = [0u8; 4];
        if reader.read_exact(&mut header).is_err() {
            break;
        }
        let Some(frame) = parse_mp3_frame(&header) else {
            break;
        };
        let frame_length = frame.byte_length as u64;
        let frame_end = first_offset
            .checked_add(consumed_bytes)
            .and_then(|offset| offset.checked_add(frame_length))
            .ok_or(IngestError::CorruptAudio)?;
        if frame_end > byte_length {
            break;
        }
        duration_ns = duration_ns.saturating_add(
            u128::from(frame.samples_per_frame).saturating_mul(1_000_000_000)
                / u128::from(frame.sample_rate),
        );
        frame_count = frame_count.saturating_add(1);
        consumed_bytes = consumed_bytes.saturating_add(frame_length);
        let remaining_frame_bytes = i64::try_from(frame.byte_length.saturating_sub(4))
            .map_err(|_| IngestError::CorruptAudio)?;
        reader
            .seek(SeekFrom::Current(remaining_frame_bytes))
            .map_err(|_| IngestError::CorruptAudio)?;
    }
    if frame_count < 2 {
        return Err(IngestError::CorruptAudio);
    }
    u64::try_from(duration_ns / 1_000_000).map_err(|_| IngestError::CorruptAudio)
}

/// 跳过可选 ID3v2 tag 后查找两个连续有效的 MP3 frame。
fn find_mp3_frame(file: &mut File, byte_length: u64) -> Result<u64, IngestError> {
    if byte_length < 8 {
        return Err(IngestError::CorruptAudio);
    }

    let mut id3_header = [0u8; 10];
    read_exact_at(file, 0, &mut id3_header)?;
    let mut scan_start = 0u64;
    if &id3_header[0..3] == b"ID3" {
        if id3_header[6..10].iter().any(|byte| byte & 0x80 != 0) {
            return Err(IngestError::CorruptAudio);
        }
        let tag_size = ((u32::from(id3_header[6]) << 21)
            | (u32::from(id3_header[7]) << 14)
            | (u32::from(id3_header[8]) << 7)
            | u32::from(id3_header[9])) as u64;
        let footer_size = if id3_header[5] & 0x10 != 0 { 10 } else { 0 };
        scan_start = 10u64
            .checked_add(tag_size)
            .and_then(|value| value.checked_add(footer_size))
            .ok_or(IngestError::CorruptAudio)?;
        if scan_start >= byte_length {
            return Err(IngestError::CorruptAudio);
        }
    }

    let scan_length = (byte_length - scan_start).min(MP3_SCAN_BYTES);
    let mut buffer = vec![0u8; usize::try_from(scan_length).unwrap_or(0)];
    if buffer.len() < 8 {
        return Err(IngestError::CorruptAudio);
    }
    read_exact_at(file, scan_start, &mut buffer)?;

    for index in 0..=buffer.len() - 4 {
        let Some(frame) = parse_mp3_frame(&buffer[index..index + 4]) else {
            continue;
        };
        let first_offset = scan_start
            .checked_add(index as u64)
            .ok_or(IngestError::CorruptAudio)?;
        let next_offset = first_offset
            .checked_add(frame.byte_length as u64)
            .ok_or(IngestError::CorruptAudio)?;
        if next_offset
            .checked_add(4)
            .ok_or(IngestError::CorruptAudio)?
            > byte_length
        {
            continue;
        }
        let mut next_header = [0u8; 4];
        read_exact_at(file, next_offset, &mut next_header)?;
        let Some(next_frame) = parse_mp3_frame(&next_header) else {
            continue;
        };
        if next_offset
            .checked_add(next_frame.byte_length as u64)
            .ok_or(IngestError::CorruptAudio)?
            <= byte_length
        {
            return Ok(first_offset);
        }
    }

    Err(IngestError::CorruptAudio)
}

/// 解析 MPEG Layer III header，并返回帧长度和播放时长所需的采样参数。
fn parse_mp3_frame(header: &[u8]) -> Option<Mp3FrameInfo> {
    if header.len() < 4 || header[0] != 0xff || header[1] & 0xe0 != 0xe0 {
        return None;
    }
    let version_bits = (header[1] >> 3) & 0x03;
    let layer_bits = (header[1] >> 1) & 0x03;
    let bitrate_index = (header[2] >> 4) & 0x0f;
    let sample_rate_index = (header[2] >> 2) & 0x03;
    if version_bits == 0x01
        || layer_bits != 0x01
        || bitrate_index == 0
        || bitrate_index == 0x0f
        || sample_rate_index == 0x03
    {
        return None;
    }

    const MPEG1_BITRATES: [u32; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    const MPEG2_BITRATES: [u32; 16] = [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ];
    const SAMPLE_RATES: [u32; 3] = [44_100, 48_000, 32_000];

    let mpeg1 = version_bits == 0x03;
    let bitrate_kbps = if mpeg1 {
        MPEG1_BITRATES[bitrate_index as usize]
    } else {
        MPEG2_BITRATES[bitrate_index as usize]
    };
    let divisor = match version_bits {
        0x03 => 1,
        0x02 => 2,
        0x00 => 4,
        _ => return None,
    };
    let sample_rate = SAMPLE_RATES[sample_rate_index as usize] / divisor;
    let coefficient = if mpeg1 { 144 } else { 72 };
    let padding = u32::from((header[2] >> 1) & 0x01);
    let length = (coefficient * bitrate_kbps * 1_000) / sample_rate + padding;
    let byte_length = usize::try_from(length).ok().filter(|value| *value >= 24)?;
    Some(Mp3FrameInfo {
        byte_length,
        sample_rate,
        samples_per_frame: if mpeg1 { 1_152 } else { 576 },
    })
}

/// 校验 ISO BMFF 顶层 box、媒体数据及受支持音频轨。
fn validate_iso_bmff(file: &mut File, byte_length: u64) -> Result<Option<u64>, IngestError> {
    let top_level = read_boxes(file, 0, byte_length)?;
    let ftyp = top_level
        .iter()
        .find(|entry| &entry.kind == b"ftyp")
        .ok_or(IngestError::CorruptAudio)?;
    if ftyp.end - ftyp.payload_start < 8 {
        return Err(IngestError::CorruptAudio);
    }
    let has_media_data = top_level
        .iter()
        .any(|entry| &entry.kind == b"mdat" && entry.end > entry.payload_start);
    if !has_media_data {
        return Err(IngestError::CorruptAudio);
    }
    let moov = top_level
        .iter()
        .find(|entry| &entry.kind == b"moov")
        .ok_or(IngestError::CorruptAudio)?;

    let moov_children = read_boxes(file, moov.payload_start, moov.end)?;
    let mut supported_tracks = Vec::new();
    let mut saw_unsupported_audio = false;
    let mut saw_video = false;
    for trak in moov_children.iter().filter(|entry| &entry.kind == b"trak") {
        if let Some(track) = parse_iso_media_track(file, *trak)? {
            match track {
                IsoMediaTrack::Audio(track) if track.supported_codec => {
                    supported_tracks.push(track)
                }
                IsoMediaTrack::Audio(_) => saw_unsupported_audio = true,
                IsoMediaTrack::Video => saw_video = true,
            }
        }
    }

    if supported_tracks.len() > 1 {
        return Err(IngestError::UnsupportedAudioTracks);
    }
    if let Some(track) = supported_tracks.first() {
        return Ok(track.duration_ms);
    }
    if saw_unsupported_audio {
        return Err(IngestError::UnsupportedAudio);
    }
    if saw_video {
        return Err(IngestError::MissingAudioTrack);
    }
    Err(IngestError::CorruptAudio)
}

/// 解析单个 trak，返回音频或视频 handler 及必要元数据。
fn parse_iso_media_track(
    file: &mut File,
    trak: IsoBox,
) -> Result<Option<IsoMediaTrack>, IngestError> {
    let children = read_boxes(file, trak.payload_start, trak.end)?;
    for mdia in children.iter().filter(|entry| &entry.kind == b"mdia") {
        if let Some(track) = parse_mdia(file, *mdia)? {
            return Ok(Some(track));
        }
    }
    Ok(None)
}

/// 解析 mdia 的 handler、mdhd 时长和 stsd sample entry。
fn parse_mdia(file: &mut File, mdia: IsoBox) -> Result<Option<IsoMediaTrack>, IngestError> {
    let children = read_boxes(file, mdia.payload_start, mdia.end)?;
    let mut is_audio = false;
    let mut is_video = false;
    let mut duration_ms = None;
    let mut supported_codec = None;

    for child in children {
        if &child.kind == b"hdlr" {
            let mut handler = [0u8; 12];
            if child.end - child.payload_start < handler.len() as u64 {
                return Err(IngestError::CorruptAudio);
            }
            read_exact_at(file, child.payload_start, &mut handler)?;
            is_audio = &handler[8..12] == b"soun";
            is_video = &handler[8..12] == b"vide";
        } else if &child.kind == b"mdhd" {
            duration_ms = parse_mdhd_duration(file, child)?;
        } else if &child.kind == b"minf" {
            supported_codec = parse_minf_codec(file, child)?;
        }
    }

    if is_audio {
        Ok(Some(IsoMediaTrack::Audio(IsoAudioTrack {
            duration_ms,
            supported_codec: supported_codec.unwrap_or(false),
        })))
    } else if is_video {
        Ok(Some(IsoMediaTrack::Video))
    } else {
        Ok(None)
    }
}

/// 解析 mdhd full box 并以 checked arithmetic 计算毫秒时长。
fn parse_mdhd_duration(file: &mut File, mdhd: IsoBox) -> Result<Option<u64>, IngestError> {
    if mdhd.end - mdhd.payload_start < 20 {
        return Err(IngestError::CorruptAudio);
    }
    let mut version = [0u8; 1];
    read_exact_at(file, mdhd.payload_start, &mut version)?;
    let (timescale_offset, duration_offset, duration_width) = match version[0] {
        0 => (12u64, 16u64, 4usize),
        1 => (20u64, 24u64, 8usize),
        _ => return Err(IngestError::CorruptAudio),
    };
    let required = duration_offset + duration_width as u64;
    if required > mdhd.end - mdhd.payload_start {
        return Err(IngestError::CorruptAudio);
    }
    let mut timescale_bytes = [0u8; 4];
    read_exact_at(
        file,
        mdhd.payload_start + timescale_offset,
        &mut timescale_bytes,
    )?;
    let timescale = u32::from_be_bytes(timescale_bytes) as u64;
    if timescale == 0 {
        return Err(IngestError::CorruptAudio);
    }
    let duration = if duration_width == 4 {
        let mut bytes = [0u8; 4];
        read_exact_at(file, mdhd.payload_start + duration_offset, &mut bytes)?;
        u32::from_be_bytes(bytes) as u64
    } else {
        let mut bytes = [0u8; 8];
        read_exact_at(file, mdhd.payload_start + duration_offset, &mut bytes)?;
        u64::from_be_bytes(bytes)
    };
    if duration == 0 {
        return Err(IngestError::EmptyAudio);
    }
    let milliseconds = (u128::from(duration) * 1_000u128) / u128::from(timescale);
    u64::try_from(milliseconds)
        .map(Some)
        .map_err(|_| IngestError::CorruptAudio)
}

/// 在 minf/stbl/stsd 中检查 AAC 或 ALAC sample entry。
fn parse_minf_codec(file: &mut File, minf: IsoBox) -> Result<Option<bool>, IngestError> {
    let children = read_boxes(file, minf.payload_start, minf.end)?;
    for stbl in children.iter().filter(|entry| &entry.kind == b"stbl") {
        let sample_table = read_boxes(file, stbl.payload_start, stbl.end)?;
        if let Some(stsd) = sample_table.iter().find(|entry| &entry.kind == b"stsd") {
            return parse_stsd(file, *stsd).map(Some);
        }
    }
    Ok(None)
}

/// 校验 stsd entry 数量与首层 sample entry 边界，并识别 mp4a/alac。
fn parse_stsd(file: &mut File, stsd: IsoBox) -> Result<bool, IngestError> {
    if stsd.end - stsd.payload_start < 8 {
        return Err(IngestError::CorruptAudio);
    }
    let mut header = [0u8; 8];
    read_exact_at(file, stsd.payload_start, &mut header)?;
    let entry_count = u32::from_be_bytes(header[4..8].try_into().unwrap()) as usize;
    if entry_count == 0 || entry_count > MAX_ISO_BOXES_PER_LEVEL {
        return Err(IngestError::CorruptAudio);
    }

    let mut offset = stsd.payload_start + 8;
    let mut supported = false;
    for _ in 0..entry_count {
        if stsd.end - offset < 8 {
            return Err(IngestError::CorruptAudio);
        }
        let mut entry_header = [0u8; 8];
        read_exact_at(file, offset, &mut entry_header)?;
        let entry_size = u32::from_be_bytes(entry_header[0..4].try_into().unwrap()) as u64;
        if entry_size < 8 {
            return Err(IngestError::CorruptAudio);
        }
        let entry_end = offset
            .checked_add(entry_size)
            .ok_or(IngestError::CorruptAudio)?;
        if entry_end > stsd.end {
            return Err(IngestError::CorruptAudio);
        }
        if &entry_header[4..8] == b"mp4a" || &entry_header[4..8] == b"alac" {
            supported = true;
        }
        offset = entry_end;
    }
    Ok(supported)
}

/// 读取指定 ISO BMFF 范围内的直接子 box，并限制 box 数量。
fn read_boxes(file: &mut File, start: u64, end: u64) -> Result<Vec<IsoBox>, IngestError> {
    if start > end {
        return Err(IngestError::CorruptAudio);
    }
    let mut offset = start;
    let mut boxes = Vec::new();
    while offset < end {
        if boxes.len() >= MAX_ISO_BOXES_PER_LEVEL || end - offset < 8 {
            return Err(IngestError::CorruptAudio);
        }
        let mut header = [0u8; 8];
        read_exact_at(file, offset, &mut header)?;
        let size32 = u32::from_be_bytes(header[0..4].try_into().unwrap()) as u64;
        let kind: [u8; 4] = header[4..8].try_into().unwrap();
        let (box_size, header_size) = if size32 == 1 {
            if end - offset < 16 {
                return Err(IngestError::CorruptAudio);
            }
            let mut extended = [0u8; 8];
            read_exact_at(file, offset + 8, &mut extended)?;
            (u64::from_be_bytes(extended), 16u64)
        } else if size32 == 0 {
            (end - offset, 8u64)
        } else {
            (size32, 8u64)
        };
        if box_size < header_size {
            return Err(IngestError::CorruptAudio);
        }
        let box_end = offset
            .checked_add(box_size)
            .ok_or(IngestError::CorruptAudio)?;
        if box_end > end {
            return Err(IngestError::CorruptAudio);
        }
        boxes.push(IsoBox {
            kind,
            payload_start: offset + header_size,
            end: box_end,
        });
        offset = box_end;
    }
    Ok(boxes)
}

/// 从绝对偏移读取固定长度数据，不把底层路径或 IO 错误泄露给调用者。
fn read_exact_at(file: &mut File, offset: u64, buffer: &mut [u8]) -> Result<(), IngestError> {
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(buffer))
        .map_err(|_| IngestError::CorruptAudio)
}
