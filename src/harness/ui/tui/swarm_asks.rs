//! Surfaces questions that subagents asked while the user is idle.
//!
//! `message_ask` blocks the subagent until someone replies. During a turn the
//! processor drains the bus and the parent answers; between turns nobody is
//! listening, so a subagent could sit blocked forever. This watcher polls the
//! bus while the TUI is idle and hands each new question to the app, which
//! shows a modal and posts the answer back with `message_reply` semantics.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::harness::swarm::bus::MessageBus;

/// How often the idle watcher looks for new questions.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// A question from a subagent, waiting for the user's answer.
#[derive(Clone, Debug)]
pub struct SwarmAsk {
    /// Message id on the bus (the `ask_id` a reply must reference).
    pub id: i64,
    /// Session that asked (`message_reply` goes back to it).
    pub from: String,
    /// The question body.
    pub question: String,
}

/// Polls `bus` for questions addressed to `session` and forwards each one
/// once. The task stops when the receiver is dropped (app exit).
pub fn spawn_watcher(
    bus: Arc<MessageBus>,
    session: String,
    tx: mpsc::UnboundedSender<SwarmAsk>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Questions already seen: a modal is queued or being answered, so
        // re-sending would stack duplicate dialogs.
        let mut seen: HashSet<i64> = HashSet::new();
        loop {
            match bus.pending_asks(&session) {
                Ok(asks) => {
                    for ask in asks {
                        if !seen.insert(ask.id) {
                            continue;
                        }
                        let msg = SwarmAsk {
                            id: ask.id,
                            from: ask.from,
                            question: ask.body,
                        };
                        if tx.send(msg).is_err() {
                            return; // app is gone
                        }
                    }
                }
                Err(e) => tracing::warn!("swarm ask watcher: {e}"),
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
}

/// Posts the user's answer back to the asking session.
pub fn answer(bus: &MessageBus, session: &str, ask: &SwarmAsk, text: &str) -> anyhow::Result<()> {
    use crate::harness::swarm::bus::MessageKind;

    bus.send_kind(session, &ask.from, text, MessageKind::Reply, Some(ask.id))?;
    // Closing the question keeps `pending_asks` honest, so the watcher does
    // not re-surface it.
    bus.mark_read(ask.id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus(dir: &std::path::Path) -> Arc<MessageBus> {
        Arc::new(MessageBus::open(&dir.join("swarm.db"), dir).unwrap())
    }

    #[tokio::test]
    async fn watcher_forwards_each_question_once() {
        use crate::harness::swarm::bus::MessageKind;

        let d = tempfile::tempdir().unwrap();
        let b = bus(d.path());
        b.send_kind("child", "parent", "which crate?", MessageKind::Ask, None)
            .unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = spawn_watcher(b.clone(), "parent".to_string(), tx);

        let got = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("watcher should forward the question")
            .expect("channel open");
        assert_eq!(got.from, "child");
        assert_eq!(got.question, "which crate?");
        assert_eq!(got.id, 1);

        // A second poll must not duplicate it.
        assert!(
            tokio::time::timeout(Duration::from_millis(2500), rx.recv())
                .await
                .is_err(),
            "the same question must not be forwarded twice"
        );
        handle.abort();
    }

    #[tokio::test]
    async fn answering_closes_the_question_and_reaches_the_asker() {
        use crate::harness::swarm::bus::MessageKind;

        let d = tempfile::tempdir().unwrap();
        let b = bus(d.path());
        b.send_kind("child", "parent", "A or B?", MessageKind::Ask, None)
            .unwrap();
        let ask = SwarmAsk {
            id: 1,
            from: "child".to_string(),
            question: "A or B?".to_string(),
        };

        answer(&b, "parent", &ask, "use B").unwrap();

        // The question no longer pends for the parent...
        assert!(b.pending_asks("parent").unwrap().is_empty());
        // ...and the child receives a Reply tied to the question.
        let inbox = b.inbox("child").unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].kind, MessageKind::Reply);
        assert_eq!(inbox[0].reply_to, Some(1));
        assert_eq!(inbox[0].body, "use B");
        assert_eq!(inbox[0].from, "parent");
    }
}
