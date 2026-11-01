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

/// What a message is for. `Ask` blocks the sender until a `Reply` arrives;
/// `Note` is fire-and-forget and never needs an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    /// Status update / finding. No reply expected.
    Note,
    /// A question that the recipient must answer with `message_reply`.
    Ask,
    /// The answer to an `Ask`, carrying `reply_to`.
    Reply,
}

impl MessageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MessageKind::Note => "note",
            MessageKind::Ask => "ask",
            MessageKind::Reply => "reply",
        }
    }

    fn parse(s: &str) -> MessageKind {
        match s {
            "ask" => MessageKind::Ask,
            "reply" => MessageKind::Reply,
            _ => MessageKind::Note,
        }
    }
}

/// A single message on the bus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub from: String,
    pub to: String,
    pub body: String,
    pub created_at: String,
    pub kind: MessageKind,
    /// For `Reply`: the id of the `Ask` being answered.
    pub reply_to: Option<i64>,
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
                read_at TEXT,
                kind TEXT NOT NULL DEFAULT 'note',
                reply_to INTEGER
            );
            CREATE INDEX IF NOT EXISTS {}_inbox ON {} (recipient, read_at);",
            self.table, self.table, self.table
        ))
        .with_context(|| format!("failed to create table {}", self.table))?;
        // Databases created before the conversation protocol lack these
        // columns; `CREATE TABLE IF NOT EXISTS` leaves them as they are.
        for sql in [
            format!(
                "ALTER TABLE {} ADD COLUMN kind TEXT NOT NULL DEFAULT 'note'",
                self.table
            ),
            format!("ALTER TABLE {} ADD COLUMN reply_to INTEGER", self.table),
        ] {
            // "duplicate column name" is the expected error on a fresh table.
            let _ = conn.execute_batch(&sql);
        }
        Ok(())
    }

    /// Appends a note. Returns its id.
    pub fn send(&self, from: &str, to: &str, body: &str) -> Result<i64> {
        self.send_kind(from, to, body, MessageKind::Note, None)
    }

    /// Appends a message of any kind. Returns its id.
    pub fn send_kind(
        &self,
        from: &str,
        to: &str,
        body: &str,
        kind: MessageKind,
        reply_to: Option<i64>,
    ) -> Result<i64> {
        let conn = self.conn.lock().expect("message db mutex poisoned");
        conn.execute(
            &format!(
                "INSERT INTO {} (sender, recipient, body, kind, reply_to)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                self.table
            ),
            params![from, to, body, kind.as_str(), reply_to],
        )
        .context("failed to insert message")?;
        Ok(conn.last_insert_rowid())
    }

    /// Sends a question and waits (with `timeout`) for a reply.
    ///
    /// This is the blocking half of the conversation protocol: the requester
    /// parks until the recipient calls `message_reply`. Polling keeps the
    /// implementation simple and the reply durable in SQLite, so a crash of
    /// either side leaves a readable record instead of a dangling wait.
    pub async fn ask(
        &self,
        from: &str,
        to: &str,
        body: &str,
        timeout: std::time::Duration,
    ) -> Result<Option<String>> {
        let ask_id = self.send_kind(from, to, body, MessageKind::Ask, None)?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(reply) = self.take_reply(from, ask_id)? {
                return Ok(Some(reply));
            }
            if std::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    }

    /// Returns the (first) unread reply to `ask_id` addressed to `session`,
    /// marking it read. Used by `ask`; also handy for tests.
    pub fn take_reply(&self, session: &str, ask_id: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("message db mutex poisoned");
        let row: Option<(i64, String)> = conn
            .query_row(
                &format!(
                    "SELECT id, body FROM {}
                     WHERE recipient = ?1 AND reply_to = ?2 AND read_at IS NULL
                     ORDER BY id ASC LIMIT 1",
                    self.table
                ),
                params![session, ask_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .context("failed to look up reply")?;
        let Some((id, body)) = row else {
            return Ok(None);
        };
        conn.execute(
            &format!(
                "UPDATE {} SET read_at = datetime('now') WHERE id = ?1",
                self.table
            ),
            params![id],
        )
        .context("failed to mark reply read")?;
        Ok(Some(body))
    }

    /// Loads a single message by id.
    pub fn get(&self, id: i64) -> Result<Option<Message>> {
        let conn = self.conn.lock().expect("message db mutex poisoned");
        let msg = conn
            .query_row(
                &format!(
                    "SELECT id, sender, recipient, body, created_at, kind, reply_to
                     FROM {} WHERE id = ?1",
                    self.table
                ),
                params![id],
                |row| {
                    Ok(Message {
                        id: row.get(0)?,
                        from: row.get(1)?,
                        to: row.get(2)?,
                        body: row.get(3)?,
                        created_at: row.get(4)?,
                        kind: MessageKind::parse(&row.get::<_, String>(5)?),
                        reply_to: row.get(6)?,
                    })
                },
            )
            .optional()
            .context("failed to load message")?;
        Ok(msg)
    }

    /// Marks a message as read without returning its body.
    pub fn mark_read(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("message db mutex poisoned");
        conn.execute(
            &format!(
                "UPDATE {} SET read_at = datetime('now') WHERE id = ?1 AND read_at IS NULL",
                self.table
            ),
            params![id],
        )
        .context("failed to mark message read")?;
        Ok(())
    }

    /// Unread `Ask` messages waiting for `session` to answer.
    pub fn pending_asks(&self, session: &str) -> Result<Vec<Message>> {
        let conn = self.conn.lock().expect("message db mutex poisoned");
        let cutoff = format!("-{} seconds", self.ttl_secs);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT id, sender, recipient, body, created_at, kind, reply_to FROM {}
                 WHERE recipient = ?1 AND read_at IS NULL AND kind = 'ask'
                   AND created_at >= datetime('now', ?2)
                 ORDER BY id ASC",
                self.table
            ))
            .context("failed to prepare pending_asks query")?;
        let rows = stmt
            .query_map(params![session, cutoff], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    from: row.get(1)?,
                    to: row.get(2)?,
                    body: row.get(3)?,
                    created_at: row.get(4)?,
                    kind: MessageKind::parse(&row.get::<_, String>(5)?),
                    reply_to: row.get(6)?,
                })
            })
            .context("failed to read pending asks")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("failed to decode ask row")?);
        }
        Ok(out)
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
                    "SELECT id, sender, recipient, body, created_at, kind, reply_to FROM {}
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
                        kind: MessageKind::parse(&row.get::<_, String>(5)?),
                        reply_to: row.get(6)?,
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

    #[test]
    fn ask_and_reply_complete_a_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let bus = bus(d.path());

        // The asker parks on `ask` while the responder answers from another
        // task, mimicking a subagent blocked on its parent.
        // The bus holds a Mutex<Connection>, so a second handle shares it.
        let responder = MessageBus::open(&d.path().join("test.db"), d.path()).unwrap();
        let handle = std::thread::spawn(move || {
            // Wait for the question to land, then answer it.
            for _ in 0..100 {
                let asks = responder.pending_asks("parent").unwrap();
                if let Some(ask) = asks.first() {
                    responder
                        .send_kind(
                            "parent",
                            &ask.from,
                            "use the JSON schema",
                            MessageKind::Reply,
                            Some(ask.id),
                        )
                        .unwrap();
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("question never arrived");
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let answer = rt.block_on(bus.ask(
            "child",
            "parent",
            "which schema?",
            std::time::Duration::from_secs(5),
        ));
        handle.join().unwrap();
        assert_eq!(answer.unwrap().as_deref(), Some("use the JSON schema"));
    }

    #[test]
    fn ask_times_out_without_a_reply() {
        let d = tempfile::tempdir().unwrap();
        let bus = bus(d.path());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let answer = rt.block_on(bus.ask(
            "child",
            "parent",
            "anyone?",
            std::time::Duration::from_millis(50),
        ));
        assert_eq!(answer.unwrap(), None);
        // The question survives the timeout so a late answer is still useful.
        assert_eq!(bus.pending_asks("parent").unwrap().len(), 1);
    }

    #[test]
    fn pending_asks_ignores_notes_and_read_questions() {
        let d = tempfile::tempdir().unwrap();
        let bus = bus(d.path());
        let ask = bus
            .send_kind("child", "parent", "q?", MessageKind::Ask, None)
            .unwrap();
        bus.send("child", "parent", "just a note").unwrap();
        assert_eq!(bus.pending_asks("parent").unwrap().len(), 1);

        bus.mark_read(ask).unwrap();
        assert!(bus.pending_asks("parent").unwrap().is_empty());
    }

    #[test]
    fn inbox_reports_the_message_kind() {
        let d = tempfile::tempdir().unwrap();
        let bus = bus(d.path());
        bus.send_kind("c", "p", "q?", MessageKind::Ask, None)
            .unwrap();
        bus.send_kind("c", "p", "a!", MessageKind::Reply, Some(1))
            .unwrap();
        let got = bus.inbox("p").unwrap();
        assert_eq!(got[0].kind, MessageKind::Ask);
        assert_eq!(got[1].kind, MessageKind::Reply);
        assert_eq!(got[1].reply_to, Some(1));
    }
}
