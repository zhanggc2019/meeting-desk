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
    #[error("分页参数无效")]
    InvalidPagination,
}

/// 封装应用唯一的 SQLite 连接，并以互斥锁串行化事务。
pub struct MeetingRepository {
    connection: Mutex<Connection>,
}

/// 描述一次关联记录删除的结果，并把待释放的受管音频引用交给命令层。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RelatedRecordsDeletion {
    pub deleted: bool,
    pub artifact_ids: Vec<String>,
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

    /// 在 SQLite 中按任务状态筛选并分页，同时返回筛选后的总数。
    pub fn list_tasks_page(
        &self,
        filter: &str,
        offset: u64,
        limit: u64,
    ) -> Result<(Vec<TaskRecord>, u64), StorageError> {
        let (offset, limit) = Self::pagination_values(offset, limit)?;
        let status_condition = match filter {
            "active" => {
                "json_extract(record_json, '$.status') NOT IN ('completed', 'failed', 'cancelled')"
            }
            "failed" => "json_extract(record_json, '$.status') IN ('failed', 'interrupted')",
            "completed" => "json_extract(record_json, '$.status') = 'completed'",
            _ => "1 = 1",
        };
        let connection = self.lock()?;
        let total = connection.query_row(
            &format!("SELECT COUNT(*) FROM tasks WHERE {status_condition}"),
            [],
            |row| row.get::<_, u64>(0),
        )?;
        let mut statement = connection.prepare(&format!(
            "SELECT record_json FROM tasks WHERE {status_condition} \
             ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2"
        ))?;
        let rows = statement.query_map(params![limit, offset], |row| row.get::<_, String>(0))?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(serde_json::from_str(&row?)?);
        }
        Ok((tasks, total))
    }

    /// 按 ID 删除单条任务快照，不影响会议记录或用户原始文件。
    pub fn delete_task(&self, id: &str) -> Result<bool, StorageError> {
        Ok(self
            .lock()?
            .execute("DELETE FROM tasks WHERE id = ?1", [id])?
            > 0)
    }

    /// 在单个事务中删除任务；有关联会议时同时清理会议正文、纪要和其他关联任务。
    pub fn delete_task_with_related_records(
        &self,
        id: &str,
    ) -> Result<RelatedRecordsDeletion, StorageError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let record_json = transaction
            .query_row("SELECT record_json FROM tasks WHERE id = ?1", [id], |row| {
                row.get::<_, String>(0)
            })
            .optional()?;
        let Some(record_json) = record_json else {
            transaction.commit()?;
            return Ok(RelatedRecordsDeletion::default());
        };
        let task: TaskRecord = serde_json::from_str(&record_json)?;
        let mut artifact_ids = Vec::new();
        if let Some(meeting_id) = task.meeting_id.as_deref() {
            let related_tasks = Self::tasks_for_meeting(&transaction, meeting_id)?;
            for (task_id, artifact_id) in related_tasks {
                transaction.execute("DELETE FROM tasks WHERE id = ?1", [task_id])?;
                if !artifact_ids.contains(&artifact_id) {
                    artifact_ids.push(artifact_id);
                }
            }
            transaction.execute("DELETE FROM meetings WHERE id = ?1", [meeting_id])?;
        } else {
            transaction.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
            artifact_ids.push(task.artifact_id);
        }
        transaction.commit()?;
        Ok(RelatedRecordsDeletion {
            deleted: true,
            artifact_ids,
        })
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

    /// 在 SQLite 中搜索会议索引、纪要和逐字稿并分页，同时返回搜索后的总数。
    pub fn search_meetings_page(
        &self,
        query: &str,
        offset: u64,
        limit: u64,
    ) -> Result<(Vec<MeetingListItem>, u64), StorageError> {
        let (offset, limit) = Self::pagination_values(offset, limit)?;
        let escaped = query
            .trim()
            .to_lowercase()
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let from_and_filter = "FROM meetings m JOIN transcripts t ON t.meeting_id = m.id \
             JOIN minutes n ON n.meeting_id = m.id \
             WHERE ?1 = '%%' OR LOWER(m.title) LIKE ?1 ESCAPE '\\' \
             OR LOWER(m.source_name) LIKE ?1 ESCAPE '\\' \
             OR LOWER(t.full_text) LIKE ?1 ESCAPE '\\' \
             OR LOWER(n.minutes_json) LIKE ?1 ESCAPE '\\'";
        let connection = self.lock()?;
        let total = connection.query_row(
            &format!("SELECT COUNT(*) {from_and_filter}"),
            [pattern.as_str()],
            |row| row.get::<_, u64>(0),
        )?;
        let mut statement = connection.prepare(&format!(
            "SELECT m.id, m.title, m.source_name, m.template_id, m.created_at, m.updated_at \
             {from_and_filter} ORDER BY m.updated_at DESC LIMIT ?2 OFFSET ?3"
        ))?;
        let rows = statement.query_map(params![pattern, limit, offset], |row| {
            Ok(MeetingListItem {
                id: row.get(0)?,
                title: row.get(1)?,
                source_name: row.get(2)?,
                template_id: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        let items = rows.collect::<Result<Vec<_>, _>>()?;
        Ok((items, total))
    }

    /// 返回指向指定会议的已完成任务，用于分页会议列表派生处理耗时。
    pub fn get_completed_task_for_meeting(
        &self,
        meeting_id: &str,
    ) -> Result<Option<TaskRecord>, StorageError> {
        let record_json = self
            .lock()?
            .query_row(
                "SELECT record_json FROM tasks \
                 WHERE json_extract(record_json, '$.status') = 'completed' \
                 AND json_extract(record_json, '$.meetingId') = ?1 \
                 ORDER BY updated_at DESC LIMIT 1",
                [meeting_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        record_json
            .map(|value| serde_json::from_str(&value).map_err(StorageError::from))
            .transpose()
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
        Ok(self.delete_meeting_with_related_tasks(id)?.deleted)
    }

    /// 在单个事务中删除会议及其全部关联任务，并返回待释放的受管音频引用。
    pub fn delete_meeting_with_related_tasks(
        &self,
        id: &str,
    ) -> Result<RelatedRecordsDeletion, StorageError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let related_tasks = Self::tasks_for_meeting(&transaction, id)?;
        let mut artifact_ids = Vec::with_capacity(related_tasks.len());
        for (task_id, artifact_id) in related_tasks {
            transaction.execute("DELETE FROM tasks WHERE id = ?1", [task_id])?;
            if !artifact_ids.contains(&artifact_id) {
                artifact_ids.push(artifact_id);
            }
        }
        let deleted = transaction.execute("DELETE FROM meetings WHERE id = ?1", [id])? > 0;
        transaction.commit()?;
        Ok(RelatedRecordsDeletion {
            deleted,
            artifact_ids,
        })
    }

    /// 读取指向指定会议的任务 ID 和 artifact ID，供事务内级联清理使用。
    fn tasks_for_meeting(
        transaction: &Transaction<'_>,
        meeting_id: &str,
    ) -> Result<Vec<(String, String)>, StorageError> {
        let task_rows = {
            let mut statement = transaction.prepare("SELECT id, record_json FROM tasks")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut related = Vec::new();
        for (task_id, record_json) in task_rows {
            let task: TaskRecord = serde_json::from_str(&record_json)?;
            if task.meeting_id.as_deref() == Some(meeting_id) {
                related.push((task_id, task.artifact_id));
            }
        }
        Ok(related)
    }

    /// 把无符号分页参数安全转换为 SQLite 的有符号 LIMIT/OFFSET。
    fn pagination_values(offset: u64, limit: u64) -> Result<(i64, i64), StorageError> {
        if !(1..=100).contains(&limit) {
            return Err(StorageError::InvalidPagination);
        }
        let offset = i64::try_from(offset).map_err(|_| StorageError::InvalidPagination)?;
        let limit = i64::try_from(limit).map_err(|_| StorageError::InvalidPagination)?;
        Ok((offset, limit))
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

    /// 验证任务可按 ID 删除，且重复删除不会误报成功。
    #[test]
    fn deletes_task_by_id() {
        let repository = MeetingRepository::in_memory().expect("create database");
        repository
            .save_task(&task_fixture("failed-task"))
            .expect("save task");

        assert!(repository.delete_task("failed-task").expect("delete task"));
        assert!(repository.list_tasks().expect("list tasks").is_empty());
        assert!(!repository
            .delete_task("failed-task")
            .expect("delete missing task"));
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

    /// 验证删除成功任务会在同一事务内清除会议、逐字稿、纪要及全部关联任务。
    #[test]
    fn deletes_completed_task_and_all_related_records_atomically() {
        let repository = MeetingRepository::in_memory().expect("create database");
        repository
            .save_completed_meeting(&fixture())
            .expect("save meeting");
        let mut requested = task_fixture("completed-task");
        requested.status = TaskStatus::Completed;
        requested.meeting_id = Some("meeting-1".to_string());
        let mut related = task_fixture("related-task");
        related.status = TaskStatus::Completed;
        related.meeting_id = Some("meeting-1".to_string());
        repository
            .save_tasks(&[
                requested.clone(),
                related.clone(),
                task_fixture("unrelated-task"),
            ])
            .expect("save tasks");

        let outcome = repository
            .delete_task_with_related_records(&requested.id)
            .expect("delete completed task");

        assert!(outcome.deleted);
        assert_eq!(outcome.artifact_ids.len(), 2);
        assert!(outcome.artifact_ids.contains(&requested.artifact_id));
        assert!(outcome.artifact_ids.contains(&related.artifact_id));
        assert!(repository
            .get_meeting("meeting-1")
            .expect("get deleted meeting")
            .is_none());
        let remaining = repository.list_tasks().expect("list remaining tasks");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "unrelated-task");
    }

    /// 验证会议删除会返回关联 artifact 供命令层释放，并清除关联任务。
    #[test]
    fn deletes_meeting_with_related_tasks_and_reports_artifacts() {
        let repository = MeetingRepository::in_memory().expect("create database");
        repository
            .save_completed_meeting(&fixture())
            .expect("save meeting");
        let mut task = task_fixture("meeting-task");
        task.status = TaskStatus::Completed;
        task.meeting_id = Some("meeting-1".to_string());
        repository.save_task(&task).expect("save related task");

        let outcome = repository
            .delete_meeting_with_related_tasks("meeting-1")
            .expect("delete meeting records");

        assert!(outcome.deleted);
        assert_eq!(outcome.artifact_ids, vec![task.artifact_id]);
        assert!(repository.list_tasks().expect("list tasks").is_empty());
    }

    /// 验证关联任务数据损坏时整个删除事务回滚，不留下半清理状态。
    #[test]
    fn rolls_back_related_record_deletion_on_invalid_task_data() {
        let repository = MeetingRepository::in_memory().expect("create database");
        repository
            .save_completed_meeting(&fixture())
            .expect("save meeting");
        let mut task = task_fixture("completed-task");
        task.status = TaskStatus::Completed;
        task.meeting_id = Some("meeting-1".to_string());
        repository.save_task(&task).expect("save task");
        repository
            .lock()
            .expect("lock database")
            .execute(
                "INSERT INTO tasks(id, record_json, updated_at) VALUES (?1, ?2, ?3)",
                params!["invalid-task", "{invalid", "2026-08-14T00:00:00Z"],
            )
            .expect("save invalid task fixture");

        let error = repository
            .delete_task_with_related_records(&task.id)
            .expect_err("reject invalid related data");

        assert!(matches!(error, StorageError::InvalidJson(_)));
        assert!(repository
            .get_meeting("meeting-1")
            .expect("get meeting after rollback")
            .is_some());
        let list_error = repository
            .list_tasks()
            .err()
            .expect("invalid task remains after rollback");
        assert!(list_error.to_string().contains("本地记录格式无效"));
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

    /// 验证任务状态筛选、总数和 LIMIT/OFFSET 由存储层统一返回。
    #[test]
    fn lists_filtered_task_page_with_total() {
        let repository = MeetingRepository::in_memory().expect("create database");
        let tasks = (0..5)
            .map(|index| {
                let mut task = task_fixture(&format!("task-{index}"));
                task.status = if index < 3 {
                    TaskStatus::Completed
                } else {
                    TaskStatus::Failed
                };
                task.updated_at = format!("2026-08-14T00:00:0{index}Z");
                task
            })
            .collect::<Vec<_>>();
        repository.save_tasks(&tasks).expect("save tasks");

        let (page, total) = repository
            .list_tasks_page("completed", 2, 2)
            .expect("list task page");

        assert_eq!(total, 3);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, "task-0");
    }

    /// 验证会议分页在数据库中搜索逐字稿，并返回搜索后的准确总数。
    #[test]
    fn searches_meeting_page_with_total() {
        let repository = MeetingRepository::in_memory().expect("create database");
        for index in 0..3 {
            let mut meeting = fixture();
            meeting.id = format!("meeting-{index}");
            meeting.title = format!("会议 {index}");
            meeting.transcript = if index < 2 {
                format!("包含分页关键字 needle {index}")
            } else {
                "不匹配的正文".to_string()
            };
            repository
                .save_completed_meeting(&meeting)
                .expect("save meeting");
        }

        let (page, total) = repository
            .search_meetings_page("needle", 0, 1)
            .expect("search meeting page");

        assert_eq!(total, 2);
        assert_eq!(page.len(), 1);
    }
}
