use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::domain::{MeetingDetail, MeetingListItem, PersistedMeetingInput, TaskRecord};

/// SQLite 访问失败时返回的安全错误。
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("无法访问本地会议数据库")]
    Database(#[from] rusqlite::Error),
    #[error("本地数据库正忙，请稍后重试")]
    LockPoisoned,
    #[error("本地记录格式无效")]
    InvalidJson(#[from] serde_json::Error),
}

/// 封装应用唯一的 SQLite 连接，并以互斥锁串行化事务。
pub struct MeetingRepository {
    connection: Mutex<Connection>,
}

impl MeetingRepository {
    /// 打开磁盘数据库并执行幂等迁移。
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|_| rusqlite::Error::InvalidPath(path.into()))?;
        }
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    /// 创建仅用于测试的内存数据库。
    #[cfg(test)]
    pub fn in_memory() -> Result<Self, StorageError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    /// 从现有连接创建仓库并初始化 Schema。
    fn from_connection(connection: Connection) -> Result<Self, StorageError> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        let repository = Self {
            connection: Mutex::new(connection),
        };
        repository.migrate()?;
        Ok(repository)
    }

    /// 创建 MVP 所需的表和索引。
    fn migrate(&self) -> Result<(), StorageError> {
        self.lock()?.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meetings (
                id TEXT PRIMARY KEY,
                source_name TEXT NOT NULL,
                title TEXT NOT NULL,
                template_id TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS transcripts (
                meeting_id TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
                full_text TEXT NOT NULL,
                segments_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS minutes (
                meeting_id TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
                minutes_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                record_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS app_settings (
                setting_key TEXT PRIMARY KEY,
                setting_value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_meetings_updated_at ON meetings(updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_meetings_title ON meetings(title);
            "#,
        )?;
        Ok(())
    }

    /// 原子保存转写、结构化纪要及会议索引信息。
    pub fn save_completed_meeting(
        &self,
        input: &PersistedMeetingInput,
    ) -> Result<(), StorageError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        Self::upsert_meeting(&transaction, input, &now)?;
        transaction.execute(
            "INSERT INTO transcripts(meeting_id, full_text, segments_json) VALUES (?1, ?2, ?3)
             ON CONFLICT(meeting_id) DO UPDATE SET full_text = excluded.full_text, segments_json = excluded.segments_json",
            params![input.id, input.transcript, serde_json::to_string(&input.transcript_segments)?],
        )?;
        transaction.execute(
            "INSERT INTO minutes(meeting_id, minutes_json) VALUES (?1, ?2)
             ON CONFLICT(meeting_id) DO UPDATE SET minutes_json = excluded.minutes_json",
            params![input.id, serde_json::to_string(&input.minutes)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// 写入或更新会议主记录，同时保留首次创建时间。
    fn upsert_meeting(
        transaction: &Transaction<'_>,
        input: &PersistedMeetingInput,
        now: &str,
    ) -> Result<(), StorageError> {
        transaction.execute(
            "INSERT INTO meetings(id, source_name, title, template_id, schema_version, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(id) DO UPDATE SET source_name = excluded.source_name, title = excluded.title,
             template_id = excluded.template_id, schema_version = excluded.schema_version, updated_at = excluded.updated_at",
            params![input.id, input.source_name, input.title, input.template_id, input.schema_version, now],
        )?;
        Ok(())
    }

    /// 保存不包含秘密和会议正文的任务快照。
    pub fn save_task(&self, task: &TaskRecord) -> Result<(), StorageError> {
        self.lock()?.execute(
            "INSERT INTO tasks(id, record_json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET record_json = excluded.record_json, updated_at = excluded.updated_at",
            params![task.id, serde_json::to_string(task)?, task.updated_at],
        )?;
        Ok(())
    }

    /// 在单个 SQLite 事务中保存一组任务，避免批量创建只落盘部分项目。
    pub fn save_tasks(&self, tasks: &[TaskRecord]) -> Result<(), StorageError> {
        let encoded = tasks
            .iter()
            .map(|task| {
                Ok((
                    task.id.clone(),
                    serde_json::to_string(task)?,
                    task.updated_at.clone(),
                ))
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        for (id, record_json, updated_at) in encoded {
            transaction.execute(
                "INSERT INTO tasks(id, record_json, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET record_json = excluded.record_json, updated_at = excluded.updated_at",
                params![id, record_json, updated_at],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// 返回按更新时间倒序排列的任务快照。
    pub fn list_tasks(&self) -> Result<Vec<TaskRecord>, StorageError> {
        let connection = self.lock()?;
        let mut statement =
            connection.prepare("SELECT record_json FROM tasks ORDER BY updated_at DESC")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(serde_json::from_str(&row?)?);
        }
        Ok(tasks)
    }

    /// 搜索会议标题和源文件名；空查询返回全部历史。
    pub fn search_meetings(&self, query: &str) -> Result<Vec<MeetingListItem>, StorageError> {
        let pattern = format!("%{}%", query.trim().replace('%', "\\%").replace('_', "\\_"));
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, title, source_name, template_id, created_at, updated_at
             FROM meetings WHERE ?1 = '%%' OR title LIKE ?1 ESCAPE '\\' OR source_name LIKE ?1 ESCAPE '\\'
             ORDER BY updated_at DESC",
        )?;
        let rows = statement.query_map([pattern], |row| {
            Ok(MeetingListItem {
                id: row.get(0)?,
                title: row.get(1)?,
                source_name: row.get(2)?,
                template_id: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    /// 读取指定会议的完整逐字稿和结构化纪要。
    pub fn get_meeting(&self, id: &str) -> Result<Option<MeetingDetail>, StorageError> {
        let connection = self.lock()?;
        let row = connection
            .query_row(
                "SELECT m.id, m.source_name, m.template_id, t.full_text, t.segments_json,
                        n.minutes_json, m.created_at, m.updated_at
                 FROM meetings m JOIN transcripts t ON t.meeting_id = m.id
                 JOIN minutes n ON n.meeting_id = m.id WHERE m.id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()?;
        row.map(|value| {
            Ok(MeetingDetail {
                id: value.0,
                source_name: value.1,
                template_id: value.2,
                transcript: value.3,
                transcript_segments: serde_json::from_str(&value.4)?,
                minutes: serde_json::from_str(&value.5)?,
                created_at: value.6,
                updated_at: value.7,
            })
        })
        .transpose()
    }

    /// 删除会议、级联正文和关联任务，不接触用户原始音频。
    pub fn delete_meeting(&self, id: &str) -> Result<bool, StorageError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let task_rows = {
            let mut statement = transaction.prepare("SELECT id, record_json FROM tasks")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (task_id, record_json) in task_rows {
            let task: TaskRecord = serde_json::from_str(&record_json)?;
            if task.meeting_id.as_deref() == Some(id) {
                transaction.execute("DELETE FROM tasks WHERE id = ?1", [task_id])?;
            }
        }
        let deleted = transaction.execute("DELETE FROM meetings WHERE id = ?1", [id])? > 0;
        transaction.commit()?;
        Ok(deleted)
    }

    /// 保存非秘密设置字符串。
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.lock()?.execute(
            "INSERT INTO app_settings(setting_key, setting_value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(setting_key) DO UPDATE SET setting_value = excluded.setting_value, updated_at = excluded.updated_at",
            params![key, value, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// 读取非秘密设置字符串。
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        self.lock()?
            .query_row(
                "SELECT setting_value FROM app_settings WHERE setting_key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::from)
    }

    /// 获取数据库互斥锁，并把锁污染转换为安全错误。
    fn lock(&self) -> Result<MutexGuard<'_, Connection>, StorageError> {
        self.connection
            .lock()
            .map_err(|_| StorageError::LockPoisoned)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::{TaskAction, TaskStatus};

    /// 构造不包含真实会议内容的测试记录。
    fn fixture() -> PersistedMeetingInput {
        PersistedMeetingInput {
            id: "meeting-1".into(),
            source_name: "sample.mp3".into(),
            title: "示例会议".into(),
            template_id: "standard_meeting".into(),
            transcript: "这是一段人工测试文本。".into(),
            transcript_segments: json!([]),
            minutes: json!({"schemaVersion": "1.0.0", "title": "示例会议"}),
            schema_version: "1.0.0".into(),
        }
    }

    /// 构造不包含会议正文的任务持久化样例。
    fn task_fixture(id: &str) -> TaskRecord {
        let now = Utc::now().to_rfc3339();
        TaskRecord {
            id: id.to_string(),
            artifact_id: format!("artifact-{id}"),
            batch_id: Some("batch-test".to_string()),
            meeting_id: None,
            display_name: format!("{id}.wav"),
            template_id: "standard_meeting".to_string(),
            status: TaskStatus::Queued,
            progress: Some(0.0),
            attempt: 0,
            max_attempts: 3,
            error: None,
            created_at: now.clone(),
            updated_at: now,
            processing_started_at: None,
            processing_duration_ms: Some(0),
            available_actions: vec![TaskAction::Cancel],
        }
    }

    /// 验证会议保存、查询和删除使用同一事务事实源。
    #[test]
    fn persists_searches_and_deletes_meeting() {
        let repository = MeetingRepository::in_memory().expect("create database");
        repository
            .save_completed_meeting(&fixture())
            .expect("save meeting");

        let meetings = repository.search_meetings("示例").expect("search meeting");
        assert_eq!(meetings.len(), 1);
        let detail = repository.get_meeting("meeting-1").expect("get meeting");
        assert_eq!(detail.expect("detail").transcript, "这是一段人工测试文本。");
        let mut task = task_fixture("meeting-task");
        task.meeting_id = Some("meeting-1".to_string());
        task.status = TaskStatus::Completed;
        task.available_actions = vec![TaskAction::OpenMeeting];
        repository.save_task(&task).expect("save related task");
        assert!(repository
            .delete_meeting("meeting-1")
            .expect("delete meeting"));
        assert!(repository
            .get_meeting("meeting-1")
            .expect("get deleted")
            .is_none());
        assert!(repository.list_tasks().expect("list tasks").is_empty());
    }

    /// 验证设置写入不会要求任何秘密字段。
    #[test]
    fn stores_public_setting() {
        let repository = MeetingRepository::in_memory().expect("create database");
        repository
            .set_setting("provider", "mock")
            .expect("set setting");
        assert_eq!(
            repository
                .get_setting("provider")
                .expect("get setting")
                .as_deref(),
            Some("mock")
        );
    }

    /// 验证批量任务通过单个事务全部写入并可完整读取。
    #[test]
    fn saves_task_batch_in_one_transaction() {
        let repository = MeetingRepository::in_memory().expect("create database");
        let tasks = vec![task_fixture("task-a"), task_fixture("task-b")];

        repository.save_tasks(&tasks).expect("save task batch");

        let stored = repository.list_tasks().expect("list task batch");
        assert_eq!(stored.len(), 2);
        assert!(stored
            .iter()
            .all(|task| task.batch_id.as_deref() == Some("batch-test")));
    }
}
