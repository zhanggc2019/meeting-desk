use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::{Duration, Local, NaiveDate};
use regex::{Captures, Regex};
use thiserror::Error;

pub const LOG_RETENTION_DAYS: i64 = 15;
const LOG_FILE_PREFIX: &str = "MeetingDesk-";
const LOG_FILE_SUFFIX: &str = ".log";
const REDACTED_VALUE: &str = "[REDACTED]";
const REDACTED_PATH: &str = "[PATH]";

/// 表示日志初始化完成后的安全统计信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoggingInitReport {
    pub removed_expired_files: usize,
}

/// 表示日志目录或全局日志器初始化失败。
#[derive(Debug, Error)]
pub enum LoggingInitError {
    #[error("日志文件不可用: {0}")]
    Io(#[from] io::Error),
    #[error("全局日志器已被其他组件初始化")]
    LoggerAlreadyInitialized,
}

/// 生成指定日期对应的应用日志文件名。
fn daily_log_file_name(date: NaiveDate) -> String {
    format!(
        "{LOG_FILE_PREFIX}{}{LOG_FILE_SUFFIX}",
        date.format("%Y-%m-%d")
    )
}

/// 清理超过保留期限且命名符合规范的应用日志。
fn cleanup_expired_logs(log_dir: &Path, today: NaiveDate) -> io::Result<usize> {
    let cutoff = today - Duration::days(LOG_RETENTION_DAYS - 1);
    let mut removed = 0;
    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(date) = parse_managed_log_date(&file_name.to_string_lossy()) else {
            continue;
        };
        if date < cutoff {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// 对可能包含凭据或本地路径的日志消息执行脱敏。
fn sanitize_log_message(message: &str) -> String {
    let normalized = message
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let sanitized = sensitive_header_regex()
        .replace_all(&normalized, |captures: &Captures<'_>| {
            format!("{}{}{REDACTED_VALUE}", &captures[1], &captures[2])
        })
        .into_owned();
    let sanitized = bearer_regex()
        .replace_all(&sanitized, format!("Bearer {REDACTED_VALUE}"))
        .into_owned();
    let sanitized = sensitive_value_regex()
        .replace_all(&sanitized, |captures: &Captures<'_>| {
            format!("{}{}{REDACTED_VALUE}", &captures[1], &captures[2])
        })
        .into_owned();
    let sanitized = double_quoted_windows_path_regex()
        .replace_all(&sanitized, format!("\"{REDACTED_PATH}\""))
        .into_owned();
    let sanitized = single_quoted_windows_path_regex()
        .replace_all(&sanitized, format!("'{REDACTED_PATH}'"))
        .into_owned();
    unquoted_windows_path_regex()
        .replace_all(&sanitized, REDACTED_PATH)
        .into_owned()
}

/// 返回用于整段移除 Cookie 与 Authorization 头值的已编译正则表达式。
fn sensitive_header_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\b(authorization|cookie)\b(\s*[:=]\s*).*$")
            .expect("valid sensitive header regex")
    })
}

/// 从严格匹配的应用日志文件名中解析日期。
fn parse_managed_log_date(file_name: &str) -> Option<NaiveDate> {
    let date_text = file_name
        .strip_prefix(LOG_FILE_PREFIX)?
        .strip_suffix(LOG_FILE_SUFFIX)?;
    if date_text.len() != 10 {
        return None;
    }
    let date = NaiveDate::parse_from_str(date_text, "%Y-%m-%d").ok()?;
    (date.format("%Y-%m-%d").to_string() == date_text).then_some(date)
}

/// 返回用于识别 Bearer 凭据的已编译正则表达式。
fn bearer_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]+").expect("valid bearer regex")
    })
}

/// 返回用于识别敏感键值对的已编译正则表达式。
fn sensitive_value_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?i)\b(api[_-]?key|access[_-]?token|refresh[_-]?token|token|authorization|cookie|secret|password)\b(\s*[:=]\s*)(?:\"[^\"]*\"|'[^']*'|[^\s,;]+)"#,
        )
        .expect("valid sensitive value regex")
    })
}

/// 返回用于识别双引号 Windows 绝对路径的已编译正则表达式。
fn double_quoted_windows_path_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)\"(?:\\\\\?\\|[a-z]:\\|\\\\)[^\"\r\n]*\""#)
            .expect("valid double quoted path regex")
    })
}

/// 返回用于识别单引号 Windows 绝对路径的已编译正则表达式。
fn single_quoted_windows_path_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)'(?:\\\\\?\\|[a-z]:\\|\\\\)[^'\r\n]*'"#)
            .expect("valid single quoted path regex")
    })
}

/// 返回用于识别未加引号 Windows 绝对路径的已编译正则表达式。
fn unquoted_windows_path_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)(?:\\\\\?\\|[a-z]:\\|\\\\)[^\s,;\"']+"#)
            .expect("valid unquoted path regex")
    })
}

/// 按本地自然日写入日志，并在日期变化时切换目标文件。
struct DailyLogWriter {
    log_dir: PathBuf,
    current_date: NaiveDate,
    file: File,
}

impl DailyLogWriter {
    /// 创建按日日志写入器，并清理超过保留期限的受管日志。
    fn new(log_dir: &Path, today: NaiveDate) -> io::Result<Self> {
        fs::create_dir_all(log_dir)?;
        cleanup_expired_logs(log_dir, today)?;
        let file = open_daily_log_file(log_dir, today)?;
        Ok(Self {
            log_dir: log_dir.to_path_buf(),
            current_date: today,
            file,
        })
    }

    /// 将字节写入指定日期的日志，必要时完成跨日切换。
    fn write_for_date(&mut self, date: NaiveDate, buffer: &[u8]) -> io::Result<usize> {
        if date != self.current_date {
            self.file.flush()?;
            cleanup_expired_logs(&self.log_dir, date)?;
            self.file = open_daily_log_file(&self.log_dir, date)?;
            self.current_date = date;
        }
        self.file.write(buffer)
    }
}

impl Write for DailyLogWriter {
    /// 将日志记录写入当前本地日期对应的文件。
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.write_for_date(Local::now().date_naive(), buffer)
    }

    /// 将文件缓冲区同步到操作系统。
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// 以追加模式打开指定日期的日志文件。
fn open_daily_log_file(log_dir: &Path, date: NaiveDate) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join(daily_log_file_name(date)))
}

/// 初始化全局脱敏日志器，按本地日期写入并保留最近十五天。
pub fn initialize_application_logger(
    log_dir: &Path,
) -> Result<LoggingInitReport, LoggingInitError> {
    let today = Local::now().date_naive();
    fs::create_dir_all(log_dir)?;
    let removed_expired_files = cleanup_expired_logs(log_dir, today)?;
    let writer = DailyLogWriter::new(log_dir, today)?;
    let dispatch = tauri_plugin_log::fern::Dispatch::new()
        .format(|out, message, record| {
            let sanitized = sanitize_log_message(&message.to_string());
            out.finish(format_args!(
                "[{}][{}][{}] {}",
                Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                record.level(),
                record.target(),
                sanitized
            ))
        })
        .level(log::LevelFilter::Warn)
        .level_for("meeting_desk_lib", log::LevelFilter::Info)
        .level_for("meeting_desk", log::LevelFilter::Info)
        .level_for("app", log::LevelFilter::Info)
        .level_for("hyper", log::LevelFilter::Error)
        .level_for("reqwest", log::LevelFilter::Error)
        .level_for("rusqlite", log::LevelFilter::Error)
        .chain(tauri_plugin_log::fern::Output::writer(
            Box::new(writer),
            "\n",
        ));
    let (max_level, logger) = dispatch.into_log();
    tauri_plugin_log::attach_logger(max_level, logger)
        .map_err(|_| LoggingInitError::LoggerAlreadyInitialized)?;
    Ok(LoggingInitReport {
        removed_expired_files,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::Duration;
    use tempfile::tempdir;

    use super::*;

    /// 创建一个空测试文件，并确保父目录已经存在。
    fn touch(path: &Path) {
        fs::write(path, b"").expect("test file should be created");
    }

    #[test]
    fn daily_log_file_name_uses_calendar_date() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 14).expect("valid date");

        assert_eq!(daily_log_file_name(date), "MeetingDesk-2026-08-14.log");
    }

    #[test]
    fn cleanup_keeps_today_and_previous_fourteen_days() {
        let directory = tempdir().expect("temporary directory should be created");
        let today = NaiveDate::from_ymd_opt(2026, 8, 14).expect("valid date");
        for age in 0..=16 {
            touch(
                &directory
                    .path()
                    .join(daily_log_file_name(today - Duration::days(age))),
            );
        }

        let removed =
            cleanup_expired_logs(directory.path(), today).expect("managed logs should be cleaned");

        assert_eq!(removed, 2);
        for age in 0..LOG_RETENTION_DAYS {
            assert!(directory
                .path()
                .join(daily_log_file_name(today - Duration::days(age)))
                .is_file());
        }
        assert!(!directory
            .path()
            .join(daily_log_file_name(today - Duration::days(15)))
            .exists());
    }

    #[test]
    fn cleanup_does_not_delete_unmanaged_entries() {
        let directory = tempdir().expect("temporary directory should be created");
        let today = NaiveDate::from_ymd_opt(2026, 8, 14).expect("valid date");
        let unrelated = directory.path().join("support-notes.txt");
        let malformed = directory.path().join("MeetingDesk-2026-8-1.log");
        let legacy = directory.path().join("MeetingDesk.log");
        let matching_directory = directory.path().join("MeetingDesk-2020-01-01.log");
        touch(&unrelated);
        touch(&malformed);
        touch(&legacy);
        fs::create_dir(&matching_directory).expect("test directory should be created");

        cleanup_expired_logs(directory.path(), today).expect("cleanup should succeed");

        assert!(unrelated.is_file());
        assert!(malformed.is_file());
        assert!(legacy.is_file());
        assert!(matching_directory.is_dir());
    }

    #[test]
    fn cleanup_is_idempotent() {
        let directory = tempdir().expect("temporary directory should be created");
        let today = NaiveDate::from_ymd_opt(2026, 8, 14).expect("valid date");
        touch(
            &directory
                .path()
                .join(daily_log_file_name(today - Duration::days(30))),
        );

        assert_eq!(cleanup_expired_logs(directory.path(), today).unwrap(), 1);
        assert_eq!(cleanup_expired_logs(directory.path(), today).unwrap(), 0);
    }

    #[test]
    fn sanitizer_redacts_credentials_and_authorization_headers() {
        let message = "api_key=secret-one token:secret-two Authorization: Bearer abc.def Cookie=session=value password='secret-three'";

        let sanitized = sanitize_log_message(message);

        assert!(!sanitized.contains("secret-one"));
        assert!(!sanitized.contains("secret-two"));
        assert!(!sanitized.contains("abc.def"));
        assert!(!sanitized.contains("session=value"));
        assert!(!sanitized.contains("secret-three"));
        assert!(sanitized.contains("[REDACTED]"));
    }

    #[test]
    fn sanitizer_redacts_complete_cookie_and_authorization_headers() {
        let cookie = sanitize_log_message("Cookie: session=secret-one; refresh=secret-two");
        let authorization = sanitize_log_message("Authorization: Basic secret-three==");

        assert!(!cookie.contains("secret-one"));
        assert!(!cookie.contains("secret-two"));
        assert!(!authorization.contains("secret-three"));
        assert!(cookie.ends_with(REDACTED_VALUE));
        assert!(authorization.ends_with(REDACTED_VALUE));
    }

    #[test]
    fn sanitizer_redacts_windows_paths_but_preserves_safe_diagnostics() {
        let message = r#"task_id=task-123 error_code=provider_timeout duration_ms=4200 source_path=\"C:\Users\Alice Zhang\meeting.mp3\" model_path=D:\models\speech\model.pt"#;

        let sanitized = sanitize_log_message(message);

        assert!(!sanitized.contains("Alice Zhang"));
        assert!(!sanitized.contains("D:\\models"));
        assert!(sanitized.contains("task_id=task-123"));
        assert!(sanitized.contains("error_code=provider_timeout"));
        assert!(sanitized.contains("duration_ms=4200"));
    }

    #[test]
    fn sanitizer_prevents_multiline_log_injection() {
        let sanitized = sanitize_log_message("model=safe-model\r\n[ERROR] forged-entry\tcode=bad");

        assert!(!sanitized.contains('\r'));
        assert!(!sanitized.contains('\n'));
        assert!(!sanitized.contains('\t'));
        assert!(sanitized.contains("model=safe-model"));
        assert!(sanitized.contains("code=bad"));
    }

    #[test]
    fn writer_switches_to_a_new_file_when_calendar_date_changes() {
        let directory = tempdir().expect("temporary directory should be created");
        let first_day = NaiveDate::from_ymd_opt(2026, 8, 14).expect("valid date");
        let second_day = first_day + Duration::days(1);
        let mut writer =
            DailyLogWriter::new(directory.path(), first_day).expect("writer should open");

        writer
            .write_for_date(first_day, b"first day\n")
            .expect("first write should succeed");
        writer
            .write_for_date(second_day, b"second day\n")
            .expect("second write should succeed");
        writer.flush().expect("logs should flush");

        assert_eq!(
            fs::read_to_string(directory.path().join(daily_log_file_name(first_day))).unwrap(),
            "first day\n"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join(daily_log_file_name(second_day))).unwrap(),
            "second day\n"
        );
    }
}
