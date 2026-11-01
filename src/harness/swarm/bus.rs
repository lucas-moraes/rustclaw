//! Message bus between agents of the same project (SQLite-backed).
//!
//! Messages are addressed to a session id (`to`) and carry the sender's
//! session id (`from`). Delivery is pull-based: `send` appends, `inbox` drains
//! the unread messages for a session. Messages expire after a TTL so a long
//! session does not accumulate stale traffic.

use crate::harness::project::table::table_name;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

/// Default lifetime of a message: 1 hour.
pub const DEFAULT_MESSAGE_TTL_SECS: i64 = 3600;

/// A single message on the bus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub from: String,
    pub to: String,
    pub body: String,
    pub created_at: String,
}

/// SQLite-backed message bus, partitioned per project root.
pub struct MessageBus {
    conn: Mutex<Connection>,
    table: String,
    ttl_secs: i64,
}

impl MessageBus {
    /// Opens (or creates) the bus for `cwd` inside the database at `db_path`.
    pub fn open(db_path: &Path, cwd: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create data dir {}", parent.display()))?;
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("failed to open message db {}", db_path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .context("failed to set journal mode")?;
        conn.execute_batch("PRAGMA busy_timeout=5000;")
            .context("failed to set busy timeout")?;
        let bus = Self {
            conn: Mutex::new(conn),
            table: table_name(cwd, "swarm_messages"),
            ttl_secs: DEFAULT_MESSAGE_TTL_SECS,
        };
        bus.ensure_schema()?;
        Ok(bus)
    }

    /// Overrides the message TTL (used by tests).
    #[cfg(test)]
    pub fn with_ttl(mut self, ttl_secs: i64) -> Self {
        self.ttl_secs = ttl_secs;
        self
    }

    fn ensure_schema(&self) -> Result<()> {
        let conn = self.conn.lock().expect("message db mutex poisoned");
        // `table` is hex-only (see project::table), so interpolation is safe.
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {} (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sender TEXT NOT NULL,
                recipient TEXT NOT NULL,
                body TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                read_at TEXT
            );
            CREATE INDEX IF NOT EXISTS {}_inbox ON {} (recipient, read_at);",
            self.table, self.table, self.table
        ))
        .with_context(|| format!("failed to create table {}", self.table))?;
        Ok(())
    }

    /// Appends a message. Returns its id.
    pub fn send(&self, from: &str, to: &str, body: &str) -> Result<i64> {
        let conn = self.conn.lock().expect("message db mutex poisoned");
        conn.execute(
            &format!(
                "INSERT INTO {} (sender, recipient, body) VALUES (?1, ?2, ?3)",
                self.table
            ),
            params![from, to, body],
        )
        .context("failed to insert message")?;
        Ok(conn.last_insert_rowid())
    }

    /// Drains the unread messages addressed to `session_id`, marking them read.
    ///
    /// Expired messages are dropped without being returned.
    pub fn inbox(&self, session_id: &str) -> Result<Vec<Message>> {
        let mut conn = self.conn.lock().expect("message db mutex poisoned");
        let tx = conn.transaction().context("failed to open transaction")?;

        let mut out = Vec::new();
        {
            let mut stmt = tx
                .prepare(&format!(
                    "SELECT id, sender, recipient, body, created_at FROM {}
                     WHERE recipient = ?1 AND read_at IS NULL
                       AND created_at >= datetime('now', ?2)
                     ORDER BY id ASC",
                    self.table
                ))
                .context("failed to prepare inbox query")?;
            let cutoff = format!("-{} seconds", self.ttl_secs);
            let rows = stmt
                .query_map(params![session_id, cutoff], |row| {
                    Ok(Message {
                        id: row.get(0)?,
                        from: row.get(1)?,
                        to: row.get(2)?,
                        body: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                })
                .context("failed to read inbox")?;
            for row in rows {
                out.push(row.context("failed to decode message row")?);
            }
        }

        // Mark everything addressed to this session as read, including the
        // expired ones: they will never be delivered, so keeping them unread
        // would only make the next drain scan them again.
        tx.execute(
            &format!(
                "UPDATE {} SET read_at = datetime('now')
                 WHERE recipient = ?1 AND read_at IS NULL",
                self.table
            ),
            params![session_id],
        )
        .context("failed to mark messages read")?;
        tx.commit().context("failed to commit inbox drain")?;
        Ok(out)
    }

    /// Number of unread, non-expired messages for `session_id`.
    pub fn unread_count(&self, session_id: &str) -> Result<usize> {
        let conn = self.conn.lock().expect("message db mutex poisoned");
        let cutoff = format!("-{} seconds", self.ttl_secs);
        let count: Option<i64> = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM {}
                     WHERE recipient = ?1 AND read_at IS NULL
                       AND created_at >= datetime('now', ?2)",
                    self.table
                ),
                params![session_id, cutoff],
                |row| row.get(0),
            )
            .optional()
            .context("failed to count unread messages")?;
        Ok(count.unwrap_or(0) as usize)
    }

    /// Deletes messages older than the TTL. Returns how many were removed.
    pub fn gc(&self) -> Result<usize> {
        let conn = self.conn.lock().expect("message db mutex poisoned");
        let cutoff = format!("-{} seconds", self.ttl_secs);
        let removed = conn
            .execute(
                &format!(
                    "DELETE FROM {} WHERE created_at < datetime('now', ?1)",
                    self.table
                ),
                params![cutoff],
            )
            .context("failed to gc messages")?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus(dir: &Path) -> MessageBus {
        MessageBus::open(&dir.join("test.db"), dir).unwrap()
    }

    #[test]
    fn send_then_inbox_delivers_once() {
        let d = tempfile::tempdir().unwrap();
        let bus = bus(d.path());
        bus.send("parent", "child", "hello").unwrap();

        let got = bus.inbox("child").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].from, "parent");
        assert_eq!(got[0].body, "hello");

        // Draining is destructive: a second call returns nothing.
        assert!(bus.inbox("child").unwrap().is_empty());
    }

    #[test]
    fn inbox_is_scoped_to_the_recipient() {
        let d = tempfile::tempdir().unwrap();
        let bus = bus(d.path());
        bus.send("a", "child", "for child").unwrap();
        bus.send("b", "other", "for other").unwrap();

        let got = bus.inbox("child").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].body, "for child");
        assert_eq!(bus.unread_count("other").unwrap(), 1);
    }

    #[test]
    fn messages_are_returned_in_send_order() {
        let d = tempfile::tempdir().unwrap();
        let bus = bus(d.path());
        for i in 0..3 {
            bus.send("p", "c", &format!("m{i}")).unwrap();
        }
        let bodies: Vec<String> = bus
            .inbox("c")
            .unwrap()
            .into_iter()
            .map(|m| m.body)
            .collect();
        assert_eq!(bodies, vec!["m0", "m1", "m2"]);
    }

    #[test]
    fn expired_messages_are_not_delivered() {
        let d = tempfile::tempdir().unwrap();
        let bus = bus(d.path()).with_ttl(0);
        bus.send("p", "c", "stale").unwrap();
        // TTL 0 means "created_at >= now", which a message from a previous
        // second fails; force the boundary by sleeping past the second.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(bus.inbox("c").unwrap().is_empty());
        assert_eq!(bus.unread_count("c").unwrap(), 0);
    }

    #[test]
    fn gc_removes_expired_messages() {
        let d = tempfile::tempdir().unwrap();
        let bus = bus(d.path()).with_ttl(0);
        bus.send("p", "c", "stale").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert_eq!(bus.gc().unwrap(), 1);
    }

    #[test]
    fn two_buses_on_the_same_db_share_messages() {
        let d = tempfile::tempdir().unwrap();
        let sender = bus(d.path());
        let receiver = bus(d.path());
        sender.send("parent", "child", "cross-instance").unwrap();
        let got = receiver.inbox("child").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].body, "cross-instance");
    }
}
