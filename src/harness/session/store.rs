//! SQLite persistence for harness sessions and messages (single source of truth).
//!
//! Data is partitioned per project: each project root gets its own set of
//! tables (`project_<h>_sessions` / `project_<h>_messages`). Legacy flat
//! tables (`harness_sessions` / `session_messages`) are migrated on open.

use crate::harness::project::table::table_name;
use crate::harness::session::{Message, Part, Role, Session, TodoItem};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct SessionStore {
    conn: Mutex<Connection>,
}

#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub id: String,
    pub agent: String,
    pub cwd: PathBuf,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    pub preview: String,
    /// Optional user-defined title (falls back to `preview`).
    pub title: Option<String>,
}

impl SessionStore {
    /// Opens (and migrates) the store.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create data dir {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open session db {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .context("failed to set journal mode")?;

        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate_legacy()?;
        Ok(store)
    }

    /// Creates the per-project tables for `cwd` (idempotent).
    pub fn ensure_project(&self, cwd: &Path) -> Result<()> {
        let sessions = table_name(cwd, "sessions");
        let messages = table_name(cwd, "messages");
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS {sessions} (
                id TEXT PRIMARY KEY,
                agent TEXT NOT NULL,
                cwd TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                todos_json TEXT NOT NULL DEFAULT '[]',
                skills_json TEXT NOT NULL DEFAULT '[]'
            );
            CREATE TABLE IF NOT EXISTS {messages} (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                parts_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                ord INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_{messages}_session
                ON {messages}(session_id, ord);"
        );
        conn.execute_batch(&sql)
            .with_context(|| format!("failed to ensure project tables for {}", cwd.display()))?;
        // Add the optional user-defined title column to existing installs.
        let has_title: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('{sessions}') WHERE name='title'"),
                [],
                |r| r.get(0),
            )
            .unwrap_or(1);
        if has_title == 0 {
            conn.execute(&format!("ALTER TABLE {sessions} ADD COLUMN title TEXT"), [])
                .context("failed to add title column")?;
        }
        Ok(())
    }

    /// Migrates rows from the legacy flat tables into per-project tables.
    fn migrate_legacy(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let has_sessions = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='harness_sessions'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_sessions {
            return Ok(());
        }
        drop(conn);
        self.migrate_sessions()?;
        Ok(())
    }

    fn migrate_sessions(&self) -> Result<()> {
        // Snapshot legacy rows while holding the lock, then release it.
        let (sessions, messages) = {
            let conn = self.conn.lock().unwrap();

            // Legacy `harness_sessions` may predate the skills_json column.
            let skills_col: Option<usize> = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('harness_sessions')
                     WHERE name='skills_json'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .ok()
                .filter(|n| *n > 0)
                .map(|_| 6);
            let select_cols = if skills_col.is_some() {
                "id, agent, cwd, created_at, updated_at, todos_json, skills_json"
            } else {
                "id, agent, cwd, created_at, updated_at, todos_json"
            };
            let sql = format!("SELECT {select_cols} FROM harness_sessions");
            let mut stmt = conn
                .prepare(&sql)
                .context("failed to prepare legacy session read")?;
            let sessions: Vec<(String, String, String, String, String, String, String)> = stmt
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        match skills_col {
                            Some(i) => r.get::<_, String>(i)?,
                            None => "[]".to_string(),
                        },
                    ))
                })
                .context("failed to read legacy sessions")?
                .collect::<std::result::Result<_, _>>()
                .context("failed to collect legacy sessions")?;
            drop(stmt);

            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, role, parts_json, created_at, ord
                     FROM session_messages",
                )
                .context("failed to prepare legacy message read")?;
            let messages: Vec<(String, String, String, String, String, i64)> = stmt
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })
                .context("failed to read legacy messages")?
                .collect::<std::result::Result<_, _>>()
                .context("failed to collect legacy messages")?;
            (sessions, messages)
        };

        // Ensure per-project tables exist before inserting (no lock held here).
        let mut projects = std::collections::BTreeSet::new();
        for (_, _, cwd, _, _, _, _) in &sessions {
            projects.insert(PathBuf::from(cwd));
        }
        for p in &projects {
            self.ensure_project(p)?;
        }

        // Insert phase (re-lock).
        let conn = self.conn.lock().unwrap();
        for (id, agent, cwd, created_at, updated_at, todos_json, skills_json) in sessions {
            let cwd_path = PathBuf::from(&cwd);
            let sessions_t = table_name(&cwd_path, "sessions");
            let messages_t = table_name(&cwd_path, "messages");
            conn.execute(
                &format!(
                    "INSERT OR IGNORE INTO {sessions_t}
                        (id, agent, cwd, created_at, updated_at, todos_json, skills_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
                ),
                params![
                    id,
                    agent,
                    cwd,
                    created_at,
                    updated_at,
                    todos_json,
                    skills_json
                ],
            )
            .context("failed to migrate legacy session")?;

            let own_messages: Vec<_> = messages
                .iter()
                .filter(|(_, sid, _, _, _, _)| sid == &id)
                .collect();
            for (mid, _sid, role, parts_json, msg_created_at, ord) in own_messages {
                conn.execute(
                    &format!(
                        "INSERT OR IGNORE INTO {messages_t}
                            (id, session_id, role, parts_json, created_at, ord)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
                    ),
                    params![mid, id, role, parts_json, msg_created_at, ord],
                )
                .context("failed to migrate legacy message")?;
            }
        }

        conn.execute_batch(
            "DROP TABLE IF EXISTS harness_sessions;
             DROP TABLE IF EXISTS session_messages;
             DROP INDEX IF EXISTS idx_session_messages_session;",
        )
        .context("failed to drop legacy tables")?;
        Ok(())
    }

    pub fn create_session(&self, agent: &str, cwd: &Path) -> Result<Session> {
        self.ensure_project(cwd)?;
        let session = Session::new(agent, cwd.to_path_buf());
        let sessions_t = table_name(cwd, "sessions");
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {sessions_t}
                    (id, agent, cwd, created_at, updated_at, todos_json, skills_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, '[]', '[]')"
            ),
            params![
                session.id,
                session.agent,
                session.cwd.to_string_lossy(),
                session.created_at.to_rfc3339(),
                session.updated_at.to_rfc3339()
            ],
        )
        .context("failed to insert session")?;
        Ok(session)
    }

    /// Insert or update a message for a session (idempotent on message id).
    pub fn save_message(&self, session_id: &str, cwd: &Path, msg: &Message) -> Result<()> {
        self.ensure_project(cwd)?;
        let messages_t = table_name(cwd, "messages");
        let sessions_t = table_name(cwd, "sessions");
        let conn = self.conn.lock().unwrap();
        let parts_json = serde_json::to_string(&msg.parts).context("failed to serialize parts")?;
        let created_at = msg.created_at.to_rfc3339();

        let exists: bool = conn
            .query_row(
                &format!("SELECT 1 FROM {messages_t} WHERE id = ?1 AND session_id = ?2"),
                params![msg.id, session_id],
                |_| Ok(true),
            )
            .optional()
            .context("failed to check message existence")?
            .is_some();

        if exists {
            conn.execute(
                &format!(
                    "UPDATE {messages_t} SET parts_json = ?3, created_at = ?4
                     WHERE id = ?1 AND session_id = ?2"
                ),
                params![msg.id, session_id, parts_json, created_at],
            )
            .context("failed to update message")?;
        } else {
            let ord: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COALESCE(MAX(ord), -1) + 1 FROM {messages_t} WHERE session_id = ?1"
                    ),
                    params![session_id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            conn.execute(
                &format!(
                    "INSERT INTO {messages_t} (id, session_id, role, parts_json, created_at, ord)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
                ),
                params![
                    msg.id,
                    session_id,
                    msg.role.as_str(),
                    parts_json,
                    created_at,
                    ord
                ],
            )
            .context("failed to insert message")?;
        }

        conn.execute(
            &format!("UPDATE {sessions_t} SET updated_at = ?2 WHERE id = ?1"),
            params![session_id, chrono::Utc::now().to_rfc3339()],
        )
        .context("failed to touch session")?;
        Ok(())
    }

    pub fn load_session(&self, id: &str, cwd: &Path) -> Result<Option<Session>> {
        self.ensure_project(cwd)?;
        let sessions_t = table_name(cwd, "sessions");
        let messages_t = table_name(cwd, "messages");
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                &format!(
                    "SELECT id, agent, cwd, created_at, updated_at, todos_json, skills_json
                     FROM {sessions_t} WHERE id = ?1"
                ),
                params![id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()
            .context("failed to load session")?;

        let Some((sid, agent, cwd, created_at, updated_at, todos_json, skills_json)) = row else {
            return Ok(None);
        };

        let mut stmt = conn
            .prepare(&format!(
                "SELECT id, role, parts_json, created_at FROM {messages_t}
                 WHERE session_id = ?1 ORDER BY ord ASC"
            ))
            .context("failed to prepare messages query")?;
        let mut messages = Vec::new();
        let rows = stmt
            .query_map(params![sid], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .context("failed to query messages")?;
        for row in rows {
            let (mid, role, parts_json, created_at) = row.context("failed to read message row")?;
            let parts: Vec<Part> =
                serde_json::from_str(&parts_json).context("failed to parse parts_json")?;
            let role = match role.as_str() {
                "assistant" => Role::Assistant,
                "system" => Role::System,
                _ => Role::User,
            };
            messages.push(Message {
                id: mid,
                role,
                parts,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
            });
        }
        let todos: Vec<TodoItem> = serde_json::from_str(&todos_json).unwrap_or_default();
        let skills: Vec<crate::harness::skill::SessionSkill> =
            serde_json::from_str(&skills_json).unwrap_or_default();

        Ok(Some(Session {
            id: sid,
            agent,
            cwd: PathBuf::from(cwd),
            created_at: parse_ts(&created_at),
            updated_at: parse_ts(&updated_at),
            messages,
            todos,
            skills,
        }))
    }

    /// Lists sessions of the current project only.
    pub fn list_sessions(&self, cwd: &Path) -> Result<Vec<SessionSummary>> {
        self.ensure_project(cwd)?;
        let sessions_t = table_name(cwd, "sessions");
        let messages_t = table_name(cwd, "messages");
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT s.id, s.agent, s.cwd, s.created_at, s.updated_at, s.title,
                        (SELECT COUNT(*) FROM {messages_t} m WHERE m.session_id = s.id) AS msg_count,
                        (SELECT m2.parts_json FROM {messages_t} m2
                          WHERE m2.session_id = s.id AND m2.role = 'user'
                          ORDER BY m2.ord ASC LIMIT 1) AS first_user
                 FROM {sessions_t} s ORDER BY s.updated_at DESC LIMIT 100"
            ))
            .context("failed to prepare list query")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            })
            .context("failed to query sessions")?;
        let mut out = Vec::new();
        for row in rows {
            let (id, agent, cwd, created_at, updated_at, title, msg_count, first_user) =
                row.context("failed to read session row")?;
            let preview = first_user
                .and_then(|pj| serde_json::from_str::<Vec<Part>>(&pj).ok())
                .and_then(|parts| {
                    parts
                        .iter()
                        .find_map(|p| p.as_text().map(|s| s.to_string()))
                })
                .unwrap_or_default();
            out.push(SessionSummary {
                id,
                agent,
                cwd: PathBuf::from(cwd),
                created_at,
                updated_at,
                message_count: msg_count as usize,
                preview: crate::harness::session::preview(&preview, 80),
                title,
            });
        }
        Ok(out)
    }

    /// Sets a user-defined title for a session (mirrors opencode titles).
    pub fn set_session_title(&self, id: &str, cwd: &Path, title: &str) -> Result<()> {
        self.ensure_project(cwd)?;
        let sessions_t = table_name(cwd, "sessions");
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!("UPDATE {sessions_t} SET title = ?2 WHERE id = ?1"),
            params![id, title],
        )
        .context("failed to set session title")?;
        Ok(())
    }

    /// Deletes the message `msg_id` and every later message of the session
    /// (ords are monotonic per session). Used by the "revert prompt" action.
    pub fn delete_messages_from(&self, id: &str, cwd: &Path, msg_id: &str) -> Result<()> {
        self.ensure_project(cwd)?;
        let messages_t = table_name(cwd, "messages");
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "DELETE FROM {messages_t}
                 WHERE session_id = ?1
                   AND ord >= (SELECT ord FROM {messages_t} WHERE id = ?2)"
            ),
            params![id, msg_id],
        )
        .context("failed to truncate messages")?;
        Ok(())
    }

    pub fn delete_session(&self, id: &str, cwd: &Path) -> Result<()> {
        self.ensure_project(cwd)?;
        let sessions_t = table_name(cwd, "sessions");
        let messages_t = table_name(cwd, "messages");
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!("DELETE FROM {messages_t} WHERE session_id = ?1"),
            params![id],
        )
        .context("failed to delete messages")?;
        conn.execute(
            &format!("DELETE FROM {sessions_t} WHERE id = ?1"),
            params![id],
        )
        .context("failed to delete session")?;
        Ok(())
    }

    /// Persist the whole session (messages + meta). Used after turns.
    pub fn save_session(&self, session: &Session) -> Result<()> {
        self.ensure_project(&session.cwd)?;
        let sessions_t = table_name(&session.cwd, "sessions");
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "UPDATE {sessions_t} SET agent = ?2, cwd = ?3, updated_at = ?4,
                        todos_json = ?5, skills_json = ?6
                 WHERE id = ?1"
            ),
            params![
                session.id,
                session.agent,
                session.cwd.to_string_lossy(),
                session.updated_at.to_rfc3339(),
                serde_json::to_string(&session.todos).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&session.skills).unwrap_or_else(|_| "[]".into()),
            ],
        )
        .context("failed to update session")?;
        drop(conn);
        // Save only messages not yet persisted.
        let existing = self.message_ids(&session.id, &session.cwd)?;
        for msg in &session.messages {
            if !existing.contains(&msg.id) {
                self.save_message(&session.id, &session.cwd, msg)?;
            }
        }
        Ok(())
    }

    fn message_ids(
        &self,
        session_id: &str,
        cwd: &Path,
    ) -> Result<std::collections::HashSet<String>> {
        let messages_t = table_name(cwd, "messages");
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT id FROM {messages_t} WHERE session_id = ?1"
            ))
            .context("failed to prepare ids query")?;
        let rows = stmt.query_map(params![session_id], |r| r.get::<_, String>(0))?;
        let mut set = std::collections::HashSet::new();
        for row in rows {
            set.insert(row.context("failed to read id")?);
        }
        Ok(set)
    }
}

fn parse_ts(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::session::{Part, ToolPart, ToolStatus};

    fn temp_store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("test.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn test_create_and_load_session() {
        let (_dir, store) = temp_store();
        let session = store
            .create_session("build", Path::new("/tmp/proj"))
            .unwrap();
        assert_eq!(session.agent, "build");
        let loaded = store
            .load_session(&session.id, Path::new("/tmp/proj"))
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.agent, "build");
        assert_eq!(loaded.cwd, PathBuf::from("/tmp/proj"));
        assert!(loaded.messages.is_empty());
        assert!(loaded.todos.is_empty());
    }

    #[test]
    fn test_projects_are_isolated() {
        let (_dir, store) = temp_store();
        let s1 = store.create_session("build", Path::new("/proj/a")).unwrap();
        let s2 = store
            .create_session("explore", Path::new("/proj/b"))
            .unwrap();
        // Session from project B is not visible when loading from project A.
        assert!(store
            .load_session(&s1.id, Path::new("/proj/a"))
            .unwrap()
            .is_some());
        assert!(store
            .load_session(&s2.id, Path::new("/proj/a"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_persist_skills() {
        let (_dir, store) = temp_store();
        let mut session = store.create_session("build", Path::new("/tmp")).unwrap();
        session
            .skills
            .push(crate::harness::skill::SessionSkill::new("frontend", true));
        session
            .skills
            .push(crate::harness::skill::SessionSkill::new("backend", false));
        store.save_session(&session).unwrap();

        let loaded = store
            .load_session(&session.id, Path::new("/tmp"))
            .unwrap()
            .unwrap();
        assert_eq!(loaded.skills.len(), 2);
        assert_eq!(loaded.skills[0].skill_id, "frontend");
        assert!(loaded.skills[0].include_by_default);
        assert!(!loaded.skills[1].include_by_default);
    }

    #[test]
    fn test_save_and_load_messages() {
        let (_dir, store) = temp_store();
        let session = store.create_session("build", Path::new("/tmp")).unwrap();

        let mut user = Message::user("list files");
        user.parts.push(Part::text("second part"));
        store
            .save_message(&session.id, Path::new("/tmp"), &user)
            .unwrap();

        let mut assistant = Message::new(
            Role::Assistant,
            vec![
                Part::text("done"),
                Part::Tool(ToolPart {
                    id: "tc1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "ls"}),
                    status: ToolStatus::Completed,
                    output: "out".into(),
                    title: "ls".into(),
                    error: None,
                }),
            ],
        );
        store
            .save_message(&session.id, Path::new("/tmp"), &assistant)
            .unwrap();

        // Update assistant (tool completed) - upsert should not duplicate
        assistant.parts.push(Part::text("extra"));
        store
            .save_message(&session.id, Path::new("/tmp"), &assistant)
            .unwrap();

        let loaded = store
            .load_session(&session.id, Path::new("/tmp"))
            .unwrap()
            .unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].id, user.id);
        assert_eq!(loaded.messages[0].parts.len(), 2);
        assert_eq!(loaded.messages[1].id, assistant.id);
        assert_eq!(loaded.messages[1].parts.len(), 3); // updated in place, not duplicated
        assert!(loaded.messages[1].has_tool_calls());
    }

    #[test]
    fn test_list_only_current_project() {
        let (_dir, store) = temp_store();
        let s1 = store.create_session("build", Path::new("/a")).unwrap();
        store.create_session("explore", Path::new("/b")).unwrap();
        store
            .save_message(&s1.id, Path::new("/a"), &Message::user("first prompt"))
            .unwrap();

        let list_a = store.list_sessions(Path::new("/a")).unwrap();
        assert_eq!(list_a.len(), 1);
        assert_eq!(list_a[0].message_count, 1);

        let list_b = store.list_sessions(Path::new("/b")).unwrap();
        assert_eq!(list_b.len(), 1);
    }

    #[test]
    fn test_delete_session() {
        let (_dir, store) = temp_store();
        let s1 = store.create_session("build", Path::new("/a")).unwrap();
        store
            .save_message(&s1.id, Path::new("/a"), &Message::user("hi"))
            .unwrap();
        store.delete_session(&s1.id, Path::new("/a")).unwrap();
        assert!(store
            .load_session(&s1.id, Path::new("/a"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_delete_messages_from_reverts() {
        let (_dir, store) = temp_store();
        let s1 = store.create_session("build", Path::new("/a")).unwrap();
        let m1 = Message::user("first");
        store.save_message(&s1.id, Path::new("/a"), &m1).unwrap();
        let m2 = Message::user("second");
        store.save_message(&s1.id, Path::new("/a"), &m2).unwrap();
        let a1 = Message::new(Role::Assistant, vec![Part::text("reply")]);
        store.save_message(&s1.id, Path::new("/a"), &a1).unwrap();

        // Revert to the second user prompt: itself + assistant disappear.
        store
            .delete_messages_from(&s1.id, Path::new("/a"), &m2.id)
            .unwrap();
        let loaded = store
            .load_session(&s1.id, Path::new("/a"))
            .unwrap()
            .unwrap();
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].parts[0].as_text(), Some("first"));
    }

    #[test]
    fn test_set_session_title() {
        let (_dir, store) = temp_store();
        let s1 = store.create_session("build", Path::new("/a")).unwrap();
        store
            .save_message(&s1.id, Path::new("/a"), &Message::user("primeiro prompt"))
            .unwrap();

        // Without a title, the list exposes only the preview.
        let list = store.list_sessions(Path::new("/a")).unwrap();
        assert_eq!(list[0].title, None);
        assert_eq!(list[0].preview, "primeiro prompt");

        // Renaming sets the user-defined title.
        store
            .set_session_title(&s1.id, Path::new("/a"), "Meu título")
            .unwrap();
        let list = store.list_sessions(Path::new("/a")).unwrap();
        assert_eq!(list[0].title.as_deref(), Some("Meu título"));
        // The preview stays intact.
        assert_eq!(list[0].preview, "primeiro prompt");

        // Reopening the store keeps the title (schema migration is idempotent).
        drop(store);
        let store = SessionStore::open(&_dir.path().join("test.db")).unwrap();
        let list = store.list_sessions(Path::new("/a")).unwrap();
        assert_eq!(list[0].title.as_deref(), Some("Meu título"));
    }

    #[test]
    fn test_migrates_legacy_flat_tables() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");

        // Build a legacy flat DB the old way.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE harness_sessions (
                    id TEXT PRIMARY KEY, agent TEXT NOT NULL, cwd TEXT NOT NULL,
                    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                    todos_json TEXT NOT NULL DEFAULT '[]'
                );
                CREATE TABLE session_messages (
                    id TEXT PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL,
                    parts_json TEXT NOT NULL, created_at TEXT NOT NULL, ord INTEGER NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO harness_sessions (id, agent, cwd, created_at, updated_at)
                 VALUES ('legacy1', 'build', '/proj/legacy', '2024-01-01T00:00:00+00:00', '2024-01-01T00:00:00+00:00')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO session_messages
                    (id, session_id, role, parts_json, created_at, ord)
                 VALUES ('m1', 'legacy1', 'user', '[{\"type\":\"text\",\"text\":\"legacy prompt\"}]', '2024-01-01T00:00:00+00:00', 0)",
                [],
            )
            .unwrap();
        }

        // Opening the store migrates legacy rows into the per-project table.
        let store = SessionStore::open(&db).unwrap();
        let loaded = store
            .load_session("legacy1", Path::new("/proj/legacy"))
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent, "build");
        assert_eq!(loaded.messages[0].text_content(), "legacy prompt");

        // Legacy flat tables are gone.
        let conn = Connection::open(&db).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN
                 ('harness_sessions', 'session_messages')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}
