//! App UI state: `App`, modals and picker overlays.
//!
//! Draw modules import these types via `crate::harness::ui::tui::app::{...}`
//! re-exports from `mod.rs`.

use crate::harness::provider::Usage;
use crate::harness::runtime::SessionRuntime;
use crate::harness::session::Session;
use crate::harness::skill::PromptSkillToggle;
use crate::harness::tool::context::AbortSignal;
use crate::harness::ui::tui::anim::{Particle, SplashState};
use crate::harness::ui::tui::askers::{PermissionRequest, QuestionRequest};
use crate::harness::ui::tui::palette::{AutoComplete, PaletteState};
use crate::harness::ui::tui::selection::{self, CellPos, PendingClick, TextSelection};
use crate::harness::ui::tui::subagent::SubagentPanel;
use crate::harness::ui::tui::theme::{self, Theme};
use crate::harness::ui::tui::transcript::{ActiveTool, LineKind, ToolBatch, TranscriptLine};
use anyhow::Result;
use tokio::sync::mpsc;

use super::pickers::persist_custom_model;
use super::undo::copy_to_clipboard;
use super::MODES;

pub struct App {
    pub runtime: SessionRuntime,
    pub session: Session,
    pub cwd: std::path::PathBuf,
    pub lines: Vec<TranscriptLine>,
    pub streaming: Option<String>,
    pub input: String,
    pub input_cursor: usize,
    /// Inner width of the prompt box (refreshed by the draw pass); used for
    /// soft-wrap aware cursor movement between frames.
    pub input_inner_width: u16,
    pub history: Vec<String>,
    pub history_pos: Option<usize>,
    pub scroll: usize,
    pub stick_bottom: bool,
    pub running: bool,
    pub abort: AbortSignal,
    pub last_iterations: usize,
    /// When the current turn started (elapsed-time indicator in the status bar).
    pub turn_started_at: Option<std::time::Instant>,
    /// Tokens from the last completed turn.
    pub last_usage: Usage,
    /// Cumulative tokens for the current UI session.
    pub session_usage: Usage,
    pub status_msg: Option<String>,
    pub show_help: bool,
    pub help_section: usize,
    pub modal: Option<Modal>,
    /// Pending permission/question modals waiting to be shown once the current
    /// one closes. Prevents concurrent asks (parallel tools) from overwriting
    /// each other and silently dropping the oneshot sender.
    pub modal_queue: std::collections::VecDeque<Modal>,
    pub theme: Theme,
    pub theme_id: usize,
    pub tick: u64,
    pub splash: Option<SplashState>,
    pub palette: Option<PaletteState>,
    pub autocomplete: Option<AutoComplete>,
    pub active_tools: Vec<ActiveTool>,
    /// Transient state of the current tool batch (see [`ToolBatch`]).
    pub tool_status: Option<ToolBatch>,
    pub particles: Vec<Particle>,
    /// Open skill picker (shown on new session).
    pub skill_picker: Option<SkillPickerState>,
    /// Open `/models` picker (provider → model, opencode-style).
    pub model_picker: Option<ModelPickerState>,
    /// Open `/auth` token prompt (masked input).
    pub auth_prompt: Option<AuthPromptState>,
    /// Open `/resume` session picker.
    pub resume_picker: Option<ResumePickerState>,
    /// Per-row mapping of rendered transcript rows → `lines` index (rebuilt
    /// each draw; used for mouse click hit-testing).
    pub transcript_row_map: Vec<usize>,
    /// Last committed transcript scroll offset (used by click mapping).
    pub transcript_scroll: usize,
    /// Bounds of the transcript viewport (set during draw).
    pub transcript_area: ratatui::layout::Rect,
    /// Plain-text snapshot of every rendered transcript row (rebuilt each draw).
    /// Used for hit-testing and clipboard extraction.
    pub transcript_plain_rows: Vec<String>,
    /// Active drag/click selection inside the transcript, if any.
    pub selection: Option<TextSelection>,
    /// Mouse-down gesture waiting to become a click or a drag-select.
    pub pending_click: Option<PendingClick>,
    /// Per-turn skill checkboxes; `None` = not yet initialized (use session defaults).
    pub prompt_toggles: Option<Vec<PromptSkillToggle>>,
    /// Whether the skill chips (not the text input) currently hold focus.
    pub skills_focused: bool,
    /// Index of the highlighted skill chip when chips are focused.
    pub skills_idx: usize,
    pub events_tx: crate::harness::event::EventSender,
    pub events_rx: crate::harness::event::EventReceiver,
    /// Live subagent panels, keyed by the `task` tool call id.
    pub subagent_panels: Vec<(String, SubagentPanel)>,
    pub permission_rx: mpsc::UnboundedReceiver<PermissionRequest>,
    pub question_rx: mpsc::UnboundedReceiver<QuestionRequest>,
    /// Open Ctrl+F transcript search modal.
    pub search: Option<SearchState>,
    /// Whether collapsible thinking (reasoning) blocks are expanded.
    pub thinking_expanded: bool,
}

/// A modal dialog waiting for user input.
pub enum Modal {
    Permission(PermissionRequest),
    /// Question from the agent. Free-text answer is always allowed; when
    /// `options` is non-empty the user can also pick with `1..n`.
    Question {
        req: QuestionRequest,
        /// Draft free-text answer being typed in the modal.
        draft: String,
        /// Cursor position (char index) inside `draft`.
        cursor: usize,
    },
    /// Pop-up for a clicked user prompt: revert or copy it.
    UserPrompt {
        line_idx: usize,
    },
}

/// Case-insensitive substring search over transcript lines.
/// Returns the indices of the lines whose text contains `query`.
pub fn search_lines(lines: &[TranscriptLine], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.text.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

/// State of the Ctrl+F transcript search modal.
pub struct SearchState {
    /// Query being typed.
    pub input: String,
    /// Indices into `App::lines` matching the current query.
    pub matches: Vec<usize>,
    /// Highlighted entry in the match list.
    pub selected: usize,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            matches: Vec::new(),
            selected: 0,
        }
    }

    pub fn push_char(&mut self, c: char) {
        self.input.push(c);
    }

    pub fn backspace(&mut self) {
        self.input.pop();
    }

    /// Recomputes matches against `lines` for the current query.
    pub fn refresh(&mut self, lines: &[TranscriptLine]) {
        self.matches = search_lines(lines, &self.input);
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
    }

    pub fn move_sel(&mut self, delta: i32) {
        if self.matches.is_empty() {
            return;
        }
        let len = self.matches.len() as i32;
        self.selected = ((self.selected as i32 + delta).rem_euclid(len)) as usize;
    }

    /// Line index of the highlighted match, if any.
    pub fn current(&self) -> Option<usize> {
        self.matches.get(self.selected).copied()
    }
}

/// State of the session-memory skill picker overlay.
pub struct SkillPickerState {
    /// Selected row for keyboard navigation.
    pub selected: usize,
    /// Parallel to `runtime.skills.skills`: which skills are checked.
    pub checked: Vec<bool>,
    /// The catalog snapshot (id -> display) this picker was built from.
    pub ids: Vec<String>,
}

impl SkillPickerState {
    pub fn open(app: &App) -> Option<Self> {
        let catalog = &app.runtime.skills;
        if catalog.skills.is_empty() {
            return None;
        }
        let ids: Vec<String> = catalog.skills.iter().map(|s| s.id.clone()).collect();
        let checked = ids
            .iter()
            .map(|id| app.session.skills.iter().any(|s| s.skill_id == *id))
            .collect();
        Some(Self {
            selected: 0,
            checked,
            ids,
        })
    }

    pub fn move_sel(&mut self, delta: i32) {
        if self.ids.is_empty() {
            return;
        }
        let len = self.ids.len() as i32;
        self.selected = ((self.selected as i32 + delta).rem_euclid(len)) as usize;
    }

    pub fn toggle(&mut self) {
        if let Some(c) = self.checked.get_mut(self.selected) {
            *c = !*c;
        }
    }

    pub fn toggle_all(&mut self) {
        let on = self.checked.iter().any(|c| !*c);
        for c in self.checked.iter_mut() {
            *c = on;
        }
    }

    /// Builds the SessionSkill list from the current checkbox selection.
    pub fn build_skills(&self) -> Vec<crate::harness::skill::SessionSkill> {
        self.ids
            .iter()
            .zip(self.checked.iter())
            .enumerate()
            .filter(|(_, (_, c))| **c)
            .map(|(i, (id, _))| {
                let mut ss = crate::harness::skill::SessionSkill::new(id.clone(), true);
                ss.ord = i as u32;
                ss
            })
            .collect()
    }
}

/// State of the two-stage `/models` picker: provider → model.
pub struct ModelPickerState {
    /// `false` = choosing provider, `true` = choosing model.
    pub stage_models: bool,
    /// Row highlighted for keyboard navigation.
    pub selected: usize,
    /// Provider chosen in stage 1.
    pub provider: String,
    /// When `Some`, a free-text custom model is being typed.
    pub custom_input: Option<String>,
    /// When `Some`, a "add provider" form is being filled (name, base_url,
    /// default_model). Each entry is one field.
    pub add_provider: Option<AddProviderForm>,
}

/// Multi-field form for adding a user-defined provider via the picker.
#[derive(Clone, Debug)]
pub struct AddProviderForm {
    pub fields: [String; 3],
    /// Index of the field currently being edited (0=name, 1=base_url,
    /// 2=default_model).
    pub field: usize,
}

impl AddProviderForm {
    pub fn new() -> Self {
        Self {
            fields: [String::new(), String::new(), String::new()],
            field: 0,
        }
    }
}

impl ModelPickerState {
    pub fn new() -> Self {
        Self {
            stage_models: false,
            selected: 0,
            provider: String::new(),
            custom_input: None,
            add_provider: None,
        }
    }

    /// Items of the current stage (providers, or models + custom entry).
    pub fn items(&self) -> Vec<String> {
        if !self.stage_models {
            let mut v = crate::harness::provider::catalog::provider_names();
            v.push("add provider…".to_string());
            v
        } else {
            let mut v: Vec<String> = crate::harness::provider::catalog::models_for(&self.provider);
            v.push("custom…".to_string());
            v
        }
    }

    pub fn move_sel(&mut self, delta: i32) {
        if self.custom_input.is_some() || self.add_provider.is_some() {
            return;
        }
        let len = self.items().len() as i32;
        if len == 0 {
            return;
        }
        self.selected = ((self.selected as i32 + delta).rem_euclid(len)) as usize;
    }

    /// Advances to the model stage after a provider is chosen.
    pub fn pick_provider(&mut self, name: String) {
        self.provider = name;
        self.stage_models = true;
        self.selected = 0;
        self.custom_input = None;
    }

    /// Returns the model to apply, consuming any custom input. `None` when
    /// the selection is the "custom…" entry (which opens the input).
    pub fn pick_model(&mut self) -> Option<String> {
        if let Some(input) = self.custom_input.take() {
            let trimmed = input.trim().to_string();
            return if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            };
        }
        let items = self.items();
        let picked = items.get(self.selected)?.clone();
        if picked == "custom…" {
            self.custom_input = Some(String::new());
            return None;
        }
        Some(picked)
    }
}

/// Modal waiting for a token (masked input) for `/auth`.
pub struct AuthPromptState {
    pub provider: String,
    pub input: String,
}

impl AuthPromptState {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            input: String::new(),
        }
    }

    pub fn push_char(&mut self, c: char) {
        self.input.push(c);
    }

    pub fn backspace(&mut self) {
        self.input.pop();
    }
}

/// `/sessions` manager picker: lets the user choose a saved session by title
/// (first user message preview) without exposing the raw session id. Also
/// supports `d` (delete session) and `r` (rename session).
pub struct ResumePickerState {
    pub sessions: Vec<crate::harness::session::store::SessionSummary>,
    pub selected: usize,
    /// When `Some`, an inline rename input is being edited for the selected
    /// session (pre-filled with the current title).
    pub rename_input: Option<String>,
    /// First visible row index in the scrollable list.
    pub scroll_offset: usize,
}

impl ResumePickerState {
    pub fn new(app: &App) -> anyhow::Result<Self> {
        let sessions = app.runtime.list_sessions().unwrap_or_default();
        Ok(Self {
            sessions,
            selected: 0,
            rename_input: None,
            scroll_offset: 0,
        })
    }

    pub fn move_sel(&mut self, delta: i32) {
        if self.sessions.is_empty() {
            return;
        }
        let len = self.sessions.len() as i32;
        self.selected = ((self.selected as i32 + delta).rem_euclid(len)) as usize;
    }

    /// Keeps the selected row within the visible window, scrolling as needed.
    pub fn ensure_selected_visible(&mut self, visible: usize) {
        if visible == 0 || self.sessions.is_empty() {
            return;
        }
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + visible {
            self.scroll_offset = self.selected + 1 - visible;
        }
        let max_offset = self.sessions.len().saturating_sub(visible);
        self.scroll_offset = self.scroll_offset.min(max_offset);
    }

    /// Human title for a session (no id).
    pub fn title(&self, i: usize) -> String {
        if let Some(s) = self.sessions.get(i) {
            if let Some(t) = &s.title {
                if !t.trim().is_empty() {
                    return t.clone();
                }
            }
            if !s.preview.trim().is_empty() {
                return s.preview.clone();
            }
        }
        "untitled session".to_string()
    }

    /// The displayed title for the selected row (used to prefill rename).
    pub fn selected_title(&self) -> String {
        self.title(self.selected)
    }
}

impl App {
    /// Test-only lightweight App editor instance.
    #[cfg(test)]
    pub fn inline_for_tests(input: &str) -> Self {
        use crate::harness::runtime::SessionRuntime;
        use crate::harness::tool::context::{PermissionAskInput, PermissionAsker, UserAsker};
        use std::sync::Arc;

        struct AllowAsker;
        #[async_trait::async_trait]
        impl PermissionAsker for AllowAsker {
            async fn ask(&self, _req: PermissionAskInput) -> bool {
                true
            }
        }
        struct NoUserAsker;
        #[async_trait::async_trait]
        impl UserAsker for NoUserAsker {
            async fn ask(&self, _q: String, _o: Vec<String>) -> Option<String> {
                None
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let _keep_dir = Box::leak(Box::new(dir)); // db file stays alive for the test
        let http = crate::harness::provider::HttpConfig {
            client: crate::harness::provider::build_http_client(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: String::new(),
        };
        let provider =
            crate::harness::provider::opencode_go::build_provider("deepinfra", http, false)
                .unwrap();
        let runtime = SessionRuntime::new_in(
            _keep_dir.path(),
            provider,
            crate::harness::tool::registry::ToolRegistry::builder().build(),
            crate::config::RuntimeConfig::default(),
            &_keep_dir.path().join("test.db"),
            Arc::new(crate::harness::permission::PermissionEngine::default()),
            Arc::new(AllowAsker),
            Arc::new(NoUserAsker),
        )
        .unwrap();
        let session = crate::harness::session::Session::new("build", "/tmp/proj".into());
        let (_perm_tx, perm_rx) = mpsc::unbounded_channel::<PermissionRequest>();
        let (_quest_tx, quest_rx) = mpsc::unbounded_channel::<QuestionRequest>();
        let mut app = Self::new(runtime, session, "/tmp/proj".into(), perm_rx, quest_rx);
        app.input = input.to_string();
        app.input_cursor = input.chars().count();
        app
    }

    pub fn new(
        runtime: SessionRuntime,
        session: Session,
        cwd: std::path::PathBuf,
        permission_rx: mpsc::UnboundedReceiver<PermissionRequest>,
        question_rx: mpsc::UnboundedReceiver<QuestionRequest>,
    ) -> Self {
        let (events_tx, events_rx) = crate::harness::event::event_channel();
        let (theme, theme_id) = theme::initial_theme();
        Self {
            runtime,
            session,
            cwd,
            lines: Vec::new(),
            streaming: None,
            input: String::new(),
            input_cursor: 0,
            input_inner_width: 80,
            history: Vec::new(),
            history_pos: None,
            scroll: 0,
            stick_bottom: true,
            running: false,
            turn_started_at: None,
            abort: AbortSignal::new(),
            last_iterations: 0,
            last_usage: Usage::default(),
            session_usage: Usage::default(),
            status_msg: None,
            show_help: false,
            help_section: 0,
            modal: None,
            modal_queue: std::collections::VecDeque::new(),
            theme,
            theme_id,
            tick: 0,
            splash: Some(SplashState::new()),
            palette: None,
            autocomplete: None,
            active_tools: Vec::new(),
            tool_status: None,
            particles: Vec::new(),
            skill_picker: None,
            model_picker: None,
            auth_prompt: None,
            resume_picker: None,
            transcript_row_map: Vec::new(),
            transcript_scroll: 0,
            transcript_area: ratatui::layout::Rect::default(),
            transcript_plain_rows: Vec::new(),
            selection: None,
            pending_click: None,
            prompt_toggles: None,
            skills_focused: false,
            skills_idx: 0,
            events_tx,
            events_rx,
            subagent_panels: Vec::new(),
            permission_rx,
            question_rx,
            search: None,
            thinking_expanded: false,
        }
    }

    pub fn needs_anim(&self) -> bool {
        self.splash.is_some()
            || self.running
            || self.palette.is_some()
            || self.show_help
            || self.modal.is_some()
            || self.input.is_empty()
            || self.streaming.is_some()
            || self
                .tool_status
                .as_ref()
                .map(|b| b.pending > 0)
                .unwrap_or(false)
            || !self.particles.is_empty()
            || self.autocomplete.is_some()
            || self.skill_picker.is_some()
            || self.model_picker.is_some()
            || self.auth_prompt.is_some()
            || self.resume_picker.is_some()
            || self.search.is_some()
            || self.selection.as_ref().map(|s| s.dragging).unwrap_or(false)
    }

    /// Opens the `/models` picker (only while idle).
    pub fn open_models_picker(&mut self) {
        if self.running {
            self.add_system("[busy] cannot switch model while a turn is running");
            return;
        }
        self.autocomplete = None;
        self.model_picker = Some(ModelPickerState::new());
    }

    /// Applies a provider/model selection and closes the picker.
    pub fn apply_model_choice(&mut self, provider: &str, model: &str) -> Result<()> {
        // Persist free-form ("custom…") models in the user store so they show
        // up in the picker next time. Best effort: a failure here must not
        // block the switch itself.
        if let Err(e) = persist_custom_model(provider, model) {
            tracing::warn!("failed to persist custom model: {}", e);
        }
        match self.runtime.switch_model(provider, model) {
            Ok(()) => {
                let name = self.runtime.provider.name().to_string();
                self.add_system(&format!(
                    "model → {} ({}) · provider {}",
                    model, name, provider
                ));
            }
            Err(e) => self.add_system(&format!("[error] model switch failed: {}", e)),
        }
        // Onboarding wizard continuation: no token for this provider yet →
        // open the (masked) auth prompt right away. This also covers switching
        // from a configured provider to a new one that has no token stored.
        if !self.runtime.has_token_for(provider) && self.auth_prompt.is_none() {
            self.add_system(&format!(
                "no token for provider `{provider}` — paste it via /auth"
            ));
            self.auth_prompt = Some(AuthPromptState::new(provider));
        }
        Ok(())
    }

    pub fn cycle_theme(&mut self) {
        self.theme_id = (self.theme_id + 1) % Theme::all().len();
        self.theme = Theme::from_index(self.theme_id);
        self.add_system(&format!("theme → {}", self.theme.name));
        self.persist_theme();
    }

    /// Saves the current theme into the global config.json (best effort).
    pub fn persist_theme(&self) {
        let mut s = crate::config::GlobalSettings::load();
        s.theme = self.theme.name.to_string();
        if let Err(e) = s.save() {
            tracing::warn!("failed to persist theme: {}", e);
        }
    }

    /// Cycles the active agent mode (build → plan → explore → general →
    /// chat-free → build), mirroring opencode's primary-agent selector.
    pub fn cycle_mode(&mut self) {
        let current = MODES
            .iter()
            .position(|m| *m == self.session.agent)
            .map(|i| (i + 1) % MODES.len())
            .unwrap_or(0);
        self.session.agent = MODES[current].to_string();
        self.autocomplete = None;
        if let Err(e) = self.runtime.store.save_session(&self.session) {
            tracing::warn!("failed to persist agent mode: {}", e);
        }
    }

    pub fn set_theme(&mut self, name: &str) -> bool {
        if let Some(t) = Theme::by_name(name) {
            self.theme_id = Theme::index_of(t.name);
            self.theme = t;
            self.persist_theme();
            true
        } else {
            false
        }
    }

    pub fn refresh_autocomplete(&mut self) {
        self.autocomplete = AutoComplete::from_input(&self.input);
    }

    /// Custom agents (name, description) for the palette, sorted by name.
    pub fn custom_agent_items(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = self
            .runtime
            .custom_agents
            .iter()
            .map(|(name, spec)| (name.clone(), spec.description.clone()))
            .collect();
        v.sort();
        v
    }

    pub(crate) fn push(&mut self, kind: LineKind, text: impl Into<String>) {
        self.lines.push(TranscriptLine {
            kind,
            text: text.into(),
        });
    }

    /// Enqueues a modal to be shown. If none is currently open, shows it
    /// immediately; otherwise it waits in the queue so concurrent asks are
    /// never dropped.
    pub(crate) fn enqueue_modal(&mut self, modal: Modal) {
        if self.modal.is_none() {
            self.modal = Some(modal);
        } else {
            self.modal_queue.push_back(modal);
        }
    }

    /// Closes the current modal and opens the next queued one, if any.
    pub(crate) fn close_modal(&mut self) {
        self.modal = self.modal_queue.pop_front();
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.pending_click = None;
    }

    /// True when a non-empty text selection is active.
    pub fn has_text_selection(&self) -> bool {
        self.selection
            .as_ref()
            .map(|s| !s.is_empty(&self.transcript_plain_rows))
            .unwrap_or(false)
    }

    /// Plain text covered by the current selection (empty if none).
    pub fn selected_text(&self) -> String {
        let Some(sel) = &self.selection else {
            return String::new();
        };
        selection::extract_text(&self.transcript_plain_rows, sel.anchor, sel.head)
    }

    /// Copies the current selection to the system clipboard. Returns `true` on
    /// success. No-ops when the selection is empty/whitespace.
    pub fn copy_selection_to_clipboard(&mut self) -> bool {
        let text = self.selected_text();
        if text.trim().is_empty() {
            return false;
        }
        if copy_to_clipboard(&text) {
            let n = text.chars().count();
            self.status_msg = Some(format!("copied {n} chars"));
            true
        } else {
            self.push(LineKind::Error, "[error] clipboard unavailable".to_string());
            false
        }
    }

    /// Extracts the last code block (``` fenced) from the transcript.
    /// Returns the code content without the fence markers.
    /// Maps screen mouse coords to a transcript cell, if inside the viewport.
    pub fn hit_test_transcript(&self, mx: u16, my: u16) -> Option<CellPos> {
        selection::hit_test(
            self.transcript_area,
            self.transcript_scroll,
            &self.transcript_plain_rows,
            mx,
            my,
        )
    }

    /// `lines` index under a rendered cell, if any.
    pub fn line_idx_at_cell(&self, pos: CellPos) -> Option<usize> {
        self.transcript_row_map.get(pos.row).copied()
    }

    /// Opens the Ctrl+F transcript search modal.
    pub fn open_search(&mut self) {
        self.search = Some(SearchState::new());
    }

    /// Jumps the transcript viewport so the given `lines` index is visible.
    /// Uses the per-row mapping built during the last draw pass.
    pub fn jump_to_line(&mut self, line_idx: usize) {
        let Some(row) = self.transcript_row_map.iter().position(|&r| r == line_idx) else {
            return;
        };
        let view_h = self.transcript_area.height as usize;
        if view_h == 0 {
            return;
        }
        self.stick_bottom = false;
        // Center the match in the viewport when possible.
        let half = view_h / 2;
        self.scroll = row.saturating_sub(half);
    }
}

#[cfg(test)]
mod resume_picker_tests {
    use super::ResumePickerState;
    use crate::harness::session::store::SessionSummary;

    fn picker(n: usize) -> ResumePickerState {
        let sessions = (0..n)
            .map(|i| SessionSummary {
                id: format!("s{i}"),
                agent: "build".to_string(),
                cwd: std::path::PathBuf::new(),
                created_at: String::new(),
                updated_at: String::new(),
                message_count: 0,
                preview: String::new(),
                title: Some(format!("session {i}")),
                parent_id: None,
            })
            .collect();
        ResumePickerState {
            sessions,
            selected: 0,
            rename_input: None,
            scroll_offset: 0,
        }
    }

    #[test]
    fn test_scroll_follows_selection_down() {
        let mut p = picker(50);
        p.selected = 30;
        p.ensure_selected_visible(10);
        assert_eq!(p.scroll_offset, 21); // 30 + 1 - 10
    }

    #[test]
    fn test_scroll_follows_selection_up() {
        let mut p = picker(50);
        p.selected = 30;
        p.scroll_offset = 30;
        p.ensure_selected_visible(10);
        assert_eq!(p.scroll_offset, 30); // selected < offset? no; selected >= offset+10? 30>=40? no
        p.selected = 5;
        p.ensure_selected_visible(10);
        assert_eq!(p.scroll_offset, 5);
    }

    #[test]
    fn test_scroll_clamps_to_max_offset() {
        let mut p = picker(50);
        p.selected = 49;
        p.ensure_selected_visible(10);
        assert_eq!(p.scroll_offset, 40); // max_offset = 50-10 = 40
    }

    #[test]
    fn test_scroll_noop_when_fits() {
        let mut p = picker(5);
        p.selected = 3;
        p.ensure_selected_visible(10);
        assert_eq!(p.scroll_offset, 0);
    }
}
