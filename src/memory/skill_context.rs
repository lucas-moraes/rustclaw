use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct SkillContextStore {
    conn: Connection,
}

impl SkillContextStore {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS active_skills (
                chat_id INTEGER PRIMARY KEY,
                skill_name TEXT NOT NULL DEFAULT 'general',
                activated_at TEXT NOT NULL,
                last_used TEXT NOT NULL
            );
            
            CREATE INDEX IF NOT EXISTS idx_active_skills_chat ON active_skills(chat_id);
            "#,
        )?;
        Ok(())
    }

    pub fn save_active_skill(&self, chat_id: i64, skill_name: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();

        self.conn.execute(
            r#"
            INSERT INTO active_skills (chat_id, skill_name, activated_at, last_used)
            VALUES (?1, ?2, ?3, ?3)
            ON CONFLICT(chat_id) DO UPDATE SET
                skill_name = excluded.skill_name,
                last_used = excluded.last_used
            "#,
            params![chat_id, skill_name, now],
        )?;

        Ok(())
    }

    pub fn get_active_skill(&self, chat_id: i64) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT skill_name FROM active_skills WHERE chat_id = ?1")?;

        let result: Result<Option<String>, _> =
            stmt.query_row([chat_id], |row| row.get(0)).optional();

        Ok(result?)
    }

    #[allow(dead_code)]
    pub fn update_last_used(&self, chat_id: i64) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();

        self.conn.execute(
            "UPDATE active_skills SET last_used = ?1 WHERE chat_id = ?2",
            params![now, chat_id],
        )?;

        Ok(())
    }
}
