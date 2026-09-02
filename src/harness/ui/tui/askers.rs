//! TUI askers: route permission/question requests through the App over channels,
//! so they never block on stdin. The App drains the channels and resolves them
//! via modal + oneshot replies.

use crate::harness::permission::PermissionEngine;
use crate::harness::tool::context::{PermissionAskInput, PermissionAsker, UserAsker};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// A pending permission request waiting for a UI decision.
pub struct PermissionRequest {
    pub input: PermissionAskInput,
    pub reply: oneshot::Sender<bool>,
}

/// A pending user question waiting for an answer.
pub struct QuestionRequest {
    pub question: String,
    pub options: Vec<String>,
    pub reply: oneshot::Sender<Option<String>>,
}

/// PermissionAsker that publishes requests to the App.
pub struct TuiAsker {
    engine: Arc<PermissionEngine>,
    tx: mpsc::UnboundedSender<PermissionRequest>,
}

impl TuiAsker {
    pub fn new(
        engine: Arc<PermissionEngine>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<PermissionRequest>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Arc::new(Self { engine, tx }), rx)
    }
}

#[async_trait::async_trait]
impl PermissionAsker for TuiAsker {
    async fn ask(&self, input: PermissionAskInput) -> bool {
        let (reply, reply_rx) = oneshot::channel();
        let req = PermissionRequest { input, reply };
        if self.tx.send(req).is_err() {
            return false;
        }
        reply_rx.await.unwrap_or(false)
    }
}

impl TuiAsker {
    /// Marks a tool as "always allow" for the session.
    pub fn set_always_allow(&self, tool: &str) {
        self.engine.set_always_allow(tool);
    }
}

/// UserAsker that publishes questions to the App.
pub struct TuiUserAsker {
    tx: mpsc::UnboundedSender<QuestionRequest>,
}

impl TuiUserAsker {
    pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<QuestionRequest>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Arc::new(Self { tx }), rx)
    }
}

impl Default for TuiUserAsker {
    fn default() -> Self {
        let (tx, _rx) = mpsc::unbounded_channel();
        Self { tx }
    }
}

#[async_trait::async_trait]
impl UserAsker for TuiUserAsker {
    async fn ask(&self, question: String, options: Vec<String>) -> Option<String> {
        let (reply, reply_rx) = oneshot::channel();
        let req = QuestionRequest {
            question,
            options,
            reply,
        };
        if self.tx.send(req).is_err() {
            return None;
        }
        reply_rx.await.ok().flatten()
    }
}
