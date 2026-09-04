//! TUI application state and main loop.

use crate::harness::event::{HarnessEvent, ToolStatus};
use crate::harness::provider::{format_tokens, Usage};
use crate::harness::runtime::{PromptResult, SessionRuntime};
use crate::harness::session::Session;
use crate::harness::skill::PromptSkillToggle;
use crate::harness::tool::context::AbortSignal;
use crate::harness::ui::commands::CommandOutcome;
use crate::harness::ui::tui::anim::{Particle, SplashState};
use crate::harness::ui::tui::askers::{PermissionRequest, QuestionRequest};
use crate::harness::ui::tui::palette::{AutoComplete, PaletteState};
use crate::harness::ui::tui::theme::{self, Theme};
use anyhow::Result;
use crossterm::event::KeyEvent;
use tokio::sync::mpsc;

fn preview(s: &str, max: usize) -> String {
    crate::harness::session::preview(s, max)
}

/// One soft-wrapped visual row: the logical char indices it displays plus the
/// smallest char index whose cursor position falls on this row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualRow {
    pub idxs: Vec<usize>,
    pub start: usize,
}

/// Word-wrapped visual rows of the prompt input (indices only).
pub fn wrap_input_rows(input: &str, width: usize) -> Vec<Vec<usize>> {
    wrap_visual(input, width)
        .into_iter()
        .map(|r| r.idxs)
        .collect()
}

/// Word-wrapped visual rows with logical start positions.
pub fn wrap_visual(input: &str, width: usize) -> Vec<VisualRow> {
    let width = width.max(4);
    let mut rows: Vec<VisualRow> = vec![VisualRow {
        idxs: Vec::new(),
        start: 0,
    }];
    let mut row_len = 0usize;
    // Index into the current row where the last word break (space) sits.
    let mut break_at: Option<usize> = None;

    for (i, c) in input.chars().enumerate() {
        if c == '\n' {
            rows.push(VisualRow {
                idxs: Vec::new(),
                start: i + 1,
            });
            row_len = 0;
            break_at = None;
            continue;
        }
        if row_len + 1 > width {
            if let Some(b) = break_at.filter(|b| *b + 1 < rows.last().unwrap().idxs.len()) {
                // Move the trailing word (after the break space) to a new row.
                let cut: Vec<usize> = rows.last_mut().unwrap().idxs.drain(b + 1..).collect();
                rows.push(VisualRow {
                    start: cut[0],
                    idxs: cut,
                });
            } else {
                rows.push(VisualRow {
                    idxs: Vec::new(),
                    start: i,
                });
            }
            row_len = rows.last().unwrap().idxs.len();
            break_at = None;
        }
        rows.last_mut().unwrap().idxs.push(i);
        row_len += 1;
        if c == ' ' {
            break_at = Some(rows.last().unwrap().idxs.len() - 1);
        }
    }
    rows
}

/// Maps a logical cursor (char index) to its visual (row, col) in `rows`.
pub fn visual_row_col(rows: &[VisualRow], cursor: usize) -> (usize, usize) {
    if rows.is_empty() {
        return (0, 0);
    }
    // The cursor belongs to the last row whose start <= cursor.
    let mut r = 0;
    for (i, row) in rows.iter().enumerate() {
        if row.start <= cursor {
            r = i;
        } else {
            break;
        }
    }
    let col = rows[r].idxs.iter().filter(|i| **i < cursor).count();
    (r, col)
}

/// Compacts the wrapped visual rows into a display window (opencode-style):
/// keeps the first 2 rows from the top, collapses the middle into a hidden
/// marker (rendered dim by the caller) and keeps the cursor row plus whatever
/// fits below it visible at the bottom. `Some((head, from, to))` describes the
/// visible ranges: rows `0..head` at the top, marker between, rows
/// `from..to` at the bottom (cursor row always inside `from..to`). `None`
/// when everything fits (no compaction).
pub fn compact_window(total: usize, crow: usize, max: usize) -> Option<(usize, usize, usize)> {
    if total <= max || max < 4 {
        return None;
    }
    let upper = 2usize.min(max.saturating_sub(2));
    if crow < upper {
        // Cursor in the top region: the head window already shows it.
        return None;
    }
    let bottom_keep = max.saturating_sub(upper + 1).max(1);
    let to = (crow + bottom_keep).min(total);
    let from = to.saturating_sub(bottom_keep).max(upper);
    Some((upper, from, to))
}

/// Char index for a (row, col) target, clamped; col == row length puts the
/// cursor just after the row's last char (before a `\n` or at end of input).
fn row_char_idx(row: &VisualRow, col: usize, input: &str) -> usize {
    let total = input.chars().count();
    if row.idxs.is_empty() {
        return row.start.min(total);
    }
    if col >= row.idxs.len() {
        (row.idxs[row.idxs.len() - 1] + 1).min(total)
    } else {
        row.idxs[col]
    }
}

/// Modes cycled by the prompt mode selector (like opencode's primary agents).
/// `general` stays available via `/agent general` and the palette.
pub const MODES: &[&str] = &["build", "plan", "explore", "general"];

/// Kind of a transcript line, used to pick colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    User,
    Assistant,
    Reasoning,
    ToolStart,
    ToolOk,
    ToolError,
    System,
    Error,
    Diff,
}

#[derive(Clone, Debug)]
pub struct TranscriptLine {
    pub kind: LineKind,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct ActiveTool {
    pub name: String,
}

/// Transient state of the current parallel tool batch. Instead of pushing one
/// line per tool call, a single status line is overwritten while the batch is
/// running and a unique summary line is emitted when the batch completes.
#[derive(Clone, Debug, Default)]
pub struct ToolBatch {
    /// Per-tool completion counts, e.g. [("read", 3), ("bash", 1)].
    pub counts: Vec<(String, usize)>,
    /// Path/args preview of the most recently started tool.
    pub last_path: String,
    /// Last started tool name (used for the transient "running" line).
    pub last_name: String,
    pub done: usize,
    pub failed: usize,
    pub pending: usize,
}

impl ToolBatch {
    pub fn start(&mut self, name: &str, path: String) {
        self.last_name = name.to_string();
        self.last_path = path;
        self.pending += 1;
        if let Some(e) = self.counts.iter_mut().find(|(n, _)| n == name) {
            e.1 += 1;
        } else {
            self.counts.push((name.to_string(), 1));
        }
    }

    /// Summary like `read ×3 · bash ×1`.
    pub fn summary(&self) -> String {
        self.counts
            .iter()
            .map(|(n, c)| format!("{} ×{}", n, c))
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// Transient label while running: `read src/x (2/4)`.
    pub fn live_label(&self) -> String {
        format!(
            "{} {} ({}/{})",
            self.last_name,
            self.last_path,
            self.done + self.failed,
            self.pending + self.done + self.failed
        )
    }
}

/// App UI state.
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
    /// Tokens from the last completed turn.
    pub last_usage: Usage,
    /// Cumulative tokens for the current UI session.
    pub session_usage: Usage,
    pub status_msg: Option<String>,
    pub show_help: bool,
    pub help_section: usize,
    pub modal: Option<Modal>,
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
    /// Per-turn skill checkboxes; `None` = not yet initialized (use session defaults).
    pub prompt_toggles: Option<Vec<PromptSkillToggle>>,
    /// Whether the skill chips (not the text input) currently hold focus.
    pub skills_focused: bool,
    /// Index of the highlighted skill chip when chips are focused.
    pub skills_idx: usize,
    pub events_tx: crate::harness::event::EventSender,
    pub events_rx: crate::harness::event::EventReceiver,
    pub permission_rx: mpsc::UnboundedReceiver<PermissionRequest>,
    pub question_rx: mpsc::UnboundedReceiver<QuestionRequest>,
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
}

impl ModelPickerState {
    pub fn new() -> Self {
        Self {
            stage_models: false,
            selected: 0,
            provider: String::new(),
            custom_input: None,
        }
    }

    /// Items of the current stage (providers, or models + custom entry).
    pub fn items(&self) -> Vec<String> {
        if !self.stage_models {
            crate::harness::provider::catalog::provider_names()
                .into_iter()
                .map(str::to_string)
                .collect()
        } else {
            let mut v: Vec<String> = crate::harness::provider::catalog::models_for(&self.provider)
                .into_iter()
                .map(str::to_string)
                .collect();
            v.push("custom…".to_string());
            v
        }
    }

    pub fn move_sel(&mut self, delta: i32) {
        if self.custom_input.is_some() {
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
}

impl ResumePickerState {
    pub fn new(app: &App) -> anyhow::Result<Self> {
        let sessions = app.runtime.list_sessions().unwrap_or_default();
        Ok(Self {
            sessions,
            selected: 0,
            rename_input: None,
        })
    }

    pub fn move_sel(&mut self, delta: i32) {
        if self.sessions.is_empty() {
            return;
        }
        let len = self.sessions.len() as i32;
        self.selected = ((self.selected as i32 + delta).rem_euclid(len)) as usize;
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
            client: reqwest::Client::new(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: String::new(),
        };
        let provider =
            crate::harness::provider::opencode_go::build_provider("deepinfra", http).unwrap();
        let runtime = SessionRuntime::new(
            provider,
            crate::harness::tool::registry::ToolRegistry::builder().build(),
            crate::harness::runtime::HarnessConfig::default(),
            &_keep_dir.path().join("test.db"),
            Arc::new(AllowAsker),
            Arc::new(NoUserAsker),
        )
        .unwrap();
        let session = crate::harness::session::Session::new("build", "/tmp/proj".into());
        let (perm_tx, perm_rx) = mpsc::unbounded_channel::<PermissionRequest>();
        let (quest_tx, quest_rx) = mpsc::unbounded_channel::<QuestionRequest>();
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
            abort: AbortSignal::new(),
            last_iterations: 0,
            last_usage: Usage::default(),
            session_usage: Usage::default(),
            status_msg: None,
            show_help: false,
            help_section: 0,
            modal: None,
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
            prompt_toggles: None,
            skills_focused: false,
            skills_idx: 0,
            events_tx,
            events_rx,
            permission_rx,
            question_rx,
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

    /// Cycles the active agent mode (build → plan → explore → general → build),
    /// mirroring opencode's primary-agent selector.
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

    fn push(&mut self, kind: LineKind, text: impl Into<String>) {
        self.lines.push(TranscriptLine {
            kind,
            text: text.into(),
        });
    }

    /// Emits the single summary line for a completed tool batch and clears the
    /// transient state (unless errors remain, in which case they are listed).
    fn finish_tool_batch(&mut self) {
        if let Some(batch) = self.tool_status.take() {
            let (kind, mark) = if batch.failed == 0 {
                (LineKind::ToolOk, "✓")
            } else if batch.done == 0 {
                (LineKind::ToolError, "✗")
            } else {
                (LineKind::ToolError, "✓/✗")
            };
            self.push(kind, format!("  {} {}", mark, batch.summary()));
        }
    }

    pub fn apply_event(&mut self, ev: HarnessEvent) {
        match ev {
            HarnessEvent::TextDelta { delta, .. } => {
                self.streaming
                    .get_or_insert_with(String::new)
                    .push_str(&delta);
            }
            HarnessEvent::ReasoningDelta { delta, .. } => {
                if delta.chars().count() >= 40 {
                    self.flush_streaming();
                    self.push(LineKind::Reasoning, delta);
                }
            }
            HarnessEvent::MessageUpdated { .. } => {
                self.flush_streaming();
            }
            HarnessEvent::ToolStart { name, input, .. } => {
                self.flush_streaming();
                self.status_msg = Some(format!("running: {}", name));
                self.active_tools.push(ActiveTool { name: name.clone() });
                let batch = self.tool_status.get_or_insert_with(ToolBatch::default);
                batch.start(&name, preview(&input.to_string(), 60));
            }
            HarnessEvent::ToolEnd {
                name, status, diff, ..
            } => {
                self.active_tools.retain(|t| t.name != name);
                if let Some(batch) = self.tool_status.as_mut() {
                    match status {
                        ToolStatus::Completed => batch.done += 1,
                        ToolStatus::Error => batch.failed += 1,
                        _ => {}
                    }
                    batch.pending = batch.pending.saturating_sub(1);
                    // Batch complete → emit a single summary line.
                    if batch.pending == 0 {
                        self.finish_tool_batch();
                    }
                }
                if let Some(d) = diff {
                    if !d.trim().is_empty() {
                        self.push(LineKind::Diff, d);
                    }
                }
                self.status_msg = None;
            }
            HarnessEvent::CompactionStarted { .. } => {
                self.flush_streaming();
                self.push(LineKind::System, "[compacting context…]".to_string());
            }
            HarnessEvent::CompactionFinished {
                summarized_messages,
                ..
            } => {
                self.push(
                    LineKind::System,
                    format!(
                        "[compaction: {} message(s) summarized]",
                        summarized_messages
                    ),
                );
            }
            HarnessEvent::Error { message, .. } => {
                self.flush_streaming();
                self.push(LineKind::Error, format!("[error] {}", message));
            }
            HarnessEvent::RunStarted { .. } => {
                self.running = true;
                self.status_msg = Some("running…".to_string());
            }
            HarnessEvent::RunFinished { .. } => {
                self.running = false;
                self.status_msg = None;
                self.active_tools.clear();
                self.flush_streaming();
            }
            HarnessEvent::UserMessage { .. } => {}
            HarnessEvent::PermissionAsk { .. } | HarnessEvent::PermissionResolved { .. } => {}
        }
    }

    pub fn flush_streaming(&mut self) {
        if let Some(s) = self.streaming.take() {
            if !s.trim().is_empty() {
                self.push(LineKind::Assistant, s);
            }
        }
    }

    pub fn add_user_prompt(&mut self, text: &str) {
        self.push(LineKind::User, text.to_string());
    }

    pub fn add_system(&mut self, text: &str) {
        self.push(LineKind::System, text.to_string());
    }

    pub fn insert_char_fixed(&mut self, c: char) {
        let mut chars: Vec<char> = self.input.chars().collect();
        let idx = self.input_cursor.min(chars.len());
        chars.insert(idx, c);
        self.input = chars.into_iter().collect();
        self.input_cursor += 1;
        self.refresh_autocomplete();
    }

    /// Removes the char at the cursor (Delete key).
    pub fn delete_forward(&mut self) {
        if self.input_cursor < self.input.chars().count() {
            let mut chars: Vec<char> = self.input.chars().collect();
            chars.remove(self.input_cursor.min(chars.len() - 1));
            self.input = chars.into_iter().collect();
            self.refresh_autocomplete();
        }
    }

    /// Char bounds of the logical line containing `self.input_cursor`.
    fn current_line_bounds(&self) -> (usize, usize) {
        let total = self.input.chars().count();
        let mut line_start = 0usize;
        for (i, c) in self.input.chars().enumerate() {
            if i >= self.input_cursor {
                break;
            }
            if c == '\n' {
                line_start = i + 1;
            }
        }
        let line_end = self
            .input
            .chars()
            .enumerate()
            .find(|(i, c)| c == &'\n' && *i >= self.input_cursor)
            .map(|(i, _)| i)
            .unwrap_or(total);
        (line_start, line_end)
    }

    /// Moves the cursor to the start of the current logical line (Home/Ctrl+A).
    pub fn cursor_line_start(&mut self) {
        self.input_cursor = self.current_line_bounds().0;
    }

    /// Moves the cursor to the end of the current logical line (End/Ctrl+E).
    pub fn cursor_line_end(&mut self) {
        let (_, end) = self.current_line_bounds();
        self.input_cursor = end;
    }

    /// Ctrl+U: delete from the current line start up to the cursor.
    pub fn kill_to_line_start(&mut self) {
        let (start, _) = self.current_line_bounds();
        if start < self.input_cursor {
            let mut chars: Vec<char> = self.input.chars().collect();
            chars.drain(start..self.input_cursor);
            self.input = chars.into_iter().collect();
            self.input_cursor = start;
            self.refresh_autocomplete();
        }
    }

    /// Ctrl+W: delete the word before the cursor (doesn't cross lines).
    pub fn kill_word_back(&mut self) {
        let mut chars: Vec<char> = self.input.chars().collect();
        let mut i = self.input_cursor.min(chars.len());
        // Skip spaces right before the cursor.
        while i > 0 && chars.get(i - 1) == Some(&' ') {
            i -= 1;
        }
        // Then skip the word chars.
        while i > 0 && chars.get(i - 1) != Some(&' ') && chars.get(i - 1) != Some(&'\n') {
            i -= 1;
        }
        chars.drain(i..self.input_cursor);
        self.input = chars.into_iter().collect();
        self.input_cursor = i;
        self.refresh_autocomplete();
    }

    /// Visual cursor position after soft-wrapping: (row, col).
    pub fn visual_cursor(&self, width: usize) -> (usize, usize) {
        let width = if width == 0 {
            self.input_inner_width.max(20) as usize
        } else {
            width
        };
        let rows = wrap_visual(&self.input, width);
        visual_row_col(&rows, self.input_cursor)
    }

    /// Up: move to the previous visual row (opencode-style multi-line editor).
    pub fn cursor_visual_up(&mut self) {
        let width = self.input_inner_width.max(20) as usize;
        let rows = wrap_visual(&self.input, width);
        if rows.len() < 2 {
            return;
        }
        let (r, col) = visual_row_col(&rows, self.input_cursor);
        if r == 0 {
            return;
        }
        self.input_cursor = row_char_idx(&rows[r - 1], col, &self.input);
    }

    /// Down: move to the next visual row.
    pub fn cursor_visual_down(&mut self) {
        let width = self.input_inner_width.max(20) as usize;
        let rows = wrap_visual(&self.input, width);
        if rows.len() < 2 {
            return;
        }
        let (r, col) = visual_row_col(&rows, self.input_cursor);
        if r + 1 >= rows.len() {
            return;
        }
        self.input_cursor = row_char_idx(&rows[r + 1], col, &self.input);
    }

    pub fn backspace(&mut self) {
        if self.input_cursor > 0 {
            let mut chars: Vec<char> = self.input.chars().collect();
            chars.remove(self.input_cursor - 1);
            self.input = chars.into_iter().collect();
            self.input_cursor -= 1;
            self.refresh_autocomplete();
        }
    }

    pub fn cursor_left(&mut self) {
        self.input_cursor = self.input_cursor.saturating_sub(1);
    }
    pub fn cursor_right(&mut self) {
        if self.input_cursor < self.input.chars().count() {
            self.input_cursor += 1;
        }
    }
    pub fn cursor_home(&mut self) {
        self.input_cursor = 0;
    }
    pub fn cursor_end(&mut self) {
        self.input_cursor = self.input.chars().count();
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let pos = self.history_pos.unwrap_or(self.history.len());
        if pos == 0 {
            return;
        }
        let new_pos = pos - 1;
        self.history_pos = Some(new_pos);
        self.input = self.history[new_pos].clone();
        self.input_cursor = self.input.chars().count();
        self.refresh_autocomplete();
    }
    pub fn history_down(&mut self) {
        let Some(pos) = self.history_pos else { return };
        if pos + 1 >= self.history.len() {
            self.history_pos = None;
            self.input.clear();
            self.input_cursor = 0;
            self.refresh_autocomplete();
            return;
        }
        let new_pos = pos + 1;
        self.history_pos = Some(new_pos);
        self.input = self.history[new_pos].clone();
        self.input_cursor = self.input.chars().count();
        self.refresh_autocomplete();
    }

    pub fn scroll_by(&mut self, delta: i32) {
        self.stick_bottom = false;
        let new = (self.scroll as i32 + delta).max(0) as usize;
        self.scroll = new;
    }

    pub fn clamp_scroll(&mut self, total: usize, view_height: usize) {
        let max = total.saturating_sub(view_height);
        if self.stick_bottom {
            self.scroll = max;
        } else {
            self.scroll = self.scroll.min(max);
            if self.scroll >= max {
                self.stick_bottom = true;
            }
        }
    }

    pub fn clear_transcript(&mut self) {
        self.lines.clear();
        self.streaming = None;
        self.tool_status = None;
        self.scroll = 0;
        self.stick_bottom = true;
        self.add_system("transcript cleared");
    }

    /// (Re)builds per-turn prompt toggles from the session's skill memory.
    pub fn sync_prompt_toggles(&mut self) {
        self.prompt_toggles = Some(
            self.session
                .skills
                .iter()
                .map(|s| PromptSkillToggle {
                    skill_id: s.skill_id.clone(),
                    include: s.include_by_default,
                })
                .collect(),
        );
    }

    /// Toggles the include flag for the skill at `index` (in prompt_toggles order).
    pub fn toggle_prompt_skill(&mut self, index: usize) {
        if let Some(toggles) = &mut self.prompt_toggles {
            if let Some(t) = toggles.get_mut(index) {
                t.include = !t.include;
            }
        }
    }

    /// Cycle which UI element is focused: input <-> skill chips.
    pub fn cycle_focus(&mut self) {
        self.skills_focused = !self.skills_focused;
    }

    /// Skill ids currently checked for the upcoming prompt.
    pub fn enabled_skill_ids(&self) -> Vec<String> {
        match &self.prompt_toggles {
            Some(toggles) => toggles
                .iter()
                .filter(|t| t.include)
                .map(|t| t.skill_id.clone())
                .collect(),
            None => self
                .session
                .skills
                .iter()
                .filter(|s| s.include_by_default)
                .map(|s| s.skill_id.clone())
                .collect(),
        }
    }

    /// Applies the picker selection to the session and persists it.
    pub fn apply_skill_picker(&mut self) -> Result<()> {
        if let Some(picker) = self.skill_picker.take() {
            self.session.skills = picker.build_skills();
            self.runtime.store.save_session(&self.session)?;
        }
        self.sync_prompt_toggles();
        self.skills_focused = false;
        Ok(())
    }

    /// Opens the skill picker for a fresh session (or `/new`). Returns true if opened.
    pub fn open_skill_picker(&mut self) -> bool {
        if let Some(state) = SkillPickerState::open(self) {
            self.skill_picker = Some(state);
            true
        } else {
            self.sync_prompt_toggles();
            false
        }
    }

    /// Handles `/skills [list|add <id>|rm <id>|default <id> on|off]`.
    pub fn handle_skills_command(&mut self, text: &str) -> Result<()> {
        let mut parts = text.splitn(3, char::is_whitespace);
        let _cmd = parts.next();
        let sub = parts.next().unwrap_or("").trim().to_string();
        let arg = parts.next().unwrap_or("").trim().to_string();

        match sub.as_str() {
            "" => {
                if self.runtime.skills.skills.is_empty() {
                    self.add_system(
                        "no skills discovered (look for .agents/skills/SKILL.md or RUSTCLAW_SKILLS_DIR)",
                    );
                } else {
                    self.open_skill_picker();
                }
            }
            "list" => {
                let lines: Vec<String> = self
                    .session
                    .skills
                    .iter()
                    .map(|s| {
                        let on = if s.include_by_default { "on" } else { "off" };
                        format!("  {} [{}]", s.skill_id, on)
                    })
                    .collect();
                for l in lines {
                    self.add_system(&l);
                }
            }
            "add" => {
                let mut existing: std::collections::HashSet<String> = self
                    .session
                    .skills
                    .iter()
                    .map(|s| s.skill_id.clone())
                    .collect();
                let mut added = 0;
                let mut unknown: Vec<String> = Vec::new();
                for id in arg.split(',') {
                    let id = id.trim();
                    if id.is_empty() {
                        continue;
                    }
                    if self.runtime.skills.get(id).is_none() {
                        unknown.push(id.to_string());
                        continue;
                    }
                    if existing.insert(id.to_string()) {
                        self.session
                            .skills
                            .push(crate::harness::skill::SessionSkill::new(id, true));
                        added += 1;
                    }
                }
                if !unknown.is_empty() {
                    self.add_system(&format!(
                        "unknown: {} (available: {})",
                        unknown.join(", "),
                        self.runtime.skills.names().join(", ")
                    ));
                }
                self.runtime.store.save_session(&self.session)?;
                self.sync_prompt_toggles();
                self.add_system(&format!("skills updated (+{})", added));
            }
            "rm" => {
                let remove: std::collections::HashSet<String> = arg
                    .split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect();
                self.session
                    .skills
                    .retain(|s| !remove.contains(&s.skill_id));
                self.runtime.store.save_session(&self.session)?;
                self.sync_prompt_toggles();
                self.add_system("skills updated");
            }
            "default" => {
                let mut it = arg.split_whitespace();
                let id = it.next().unwrap_or("").trim().to_string();
                let on = it.next().unwrap_or("").trim().to_string();
                if id.is_empty() {
                    self.add_system("usage: /skills default <id> on|off");
                    return Ok(());
                }
                let val = on == "on" || on == "true";
                let found = self.session.skills.iter().position(|s| s.skill_id == id);
                match found {
                    Some(idx) => {
                        self.session.skills[idx].include_by_default = val;
                        self.runtime.store.save_session(&self.session)?;
                        self.sync_prompt_toggles();
                        self.add_system(&format!(
                            "{} default → {}",
                            id,
                            if val { "on" } else { "off" }
                        ));
                    }
                    None => self.add_system(&format!("skill not in session memory: {}", id)),
                }
            }
            "picker" | "open" => {
                self.open_skill_picker();
            }
            _ => self.add_system(
                "usage: /skills [list] [add <id>] [rm <id>] [default <id> on|off] [picker]",
            ),
        }
        Ok(())
    }

    pub fn record_usage(&mut self, usage: Usage, iterations: usize) {
        self.last_iterations = iterations;
        self.last_usage = usage;
        self.session_usage.add_assign(usage);
    }

    pub fn reset_usage(&mut self) {
        self.last_usage = Usage::default();
        self.session_usage = Usage::default();
        self.last_iterations = 0;
    }

    pub fn context_tokens(&self) -> usize {
        self.session.approx_tokens()
    }

    pub fn max_context_tokens(&self) -> usize {
        self.runtime.config.max_context_tokens
    }

    /// Multi-line usage report for `/usage` and turn summaries.
    pub fn usage_report(&self) -> Vec<String> {
        let ctx = self.context_tokens();
        let max = self.max_context_tokens();
        let pct = if max == 0 { 0 } else { (ctx * 100) / max };
        vec![
            format!(
                "last turn · in {} · out {} · total {} · {} iter(s)",
                format_tokens(self.last_usage.input_tokens),
                format_tokens(self.last_usage.output_tokens),
                format_tokens(self.last_usage.total()),
                self.last_iterations
            ),
            format!(
                "session  · in {} · out {} · total {}",
                format_tokens(self.session_usage.input_tokens),
                format_tokens(self.session_usage.output_tokens),
                format_tokens(self.session_usage.total())
            ),
            format!(
                "context  · ~{} / {} ({}%)",
                format_tokens(ctx as u64),
                format_tokens(max as u64),
                pct
            ),
        ]
    }
}

/// Main TUI loop.
pub async fn run_tui(
    runtime: SessionRuntime,
    session: Session,
    cwd: std::path::PathBuf,
    permission_rx: mpsc::UnboundedReceiver<PermissionRequest>,
    question_rx: mpsc::UnboundedReceiver<QuestionRequest>,
) -> Result<()> {
    use crossterm::event::{self, Event};
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use std::io::stdout;

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    crossterm::execute!(stdout, crossterm::event::EnableMouseCapture)?;
    let _ = crossterm::execute!(stdout, crossterm::event::EnableBracketedPaste);
    // Kitty keyboard protocol: lets Shift+Enter reach the app as a distinct
    // key so multi-line prompts work (terminals without support just stay
    // on plain Enter). DISAMBIGUATE is what disambiguates Shift+Enter;
    // REPORT_EVENT_TYPES is intentionally omitted — we only handle presses.
    let _ = crossterm::execute!(
        stdout,
        crossterm::event::PushKeyboardEnhancementFlags(
            crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | crossterm::event::KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        )
    );
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let _guard = TerminalGuard;

    let mut app = App::new(runtime, session, cwd, permission_rx, question_rx);
    // Unconfigured boot → onboarding wizard: with no model selected yet, open
    // the /models picker first (the auth prompt follows automatically after
    // the model choice, see apply_model_choice). When a model is already
    // selected in the settings, skip setup and go straight to the token
    // prompt for the resolved provider.
    if !app.runtime.config.is_configured() {
        let settings = crate::config::GlobalSettings::load();
        if settings.provider.is_empty() && settings.model.is_empty() {
            app.open_models_picker();
            app.add_system("RustClaw needs a provider/model and an API token — configure now");
        } else {
            let provider = app.runtime.config.provider.clone();
            app.add_system(
                "model already selected — RustClaw just needs an API token for this provider",
            );
            app.auth_prompt = Some(AuthPromptState::new(&provider));
        }
    }

    let mut prompt_task: Option<tokio::task::JoinHandle<Result<(PromptResult, Session)>>> = None;

    loop {
        app.tick = app.tick.wrapping_add(1);

        // Splash finished?
        if let Some(s) = &app.splash {
            if s.done() {
                app.splash = None;
                let fresh = app.session.messages.is_empty();
                if fresh && app.skill_picker.is_none() {
                    // New session → choose skills for this session's memory.
                    app.open_skill_picker();
                } else {
                    app.sync_prompt_toggles();
                }
            }
        }

        while let Ok(ev) = app.events_rx.try_recv() {
            app.apply_event(ev);
        }
        while let Ok(req) = app.permission_rx.try_recv() {
            app.flush_streaming();
            app.push(
                LineKind::System,
                format!(
                    "[permission] {} {}",
                    req.input.tool,
                    preview(&req.input.args_summary, 120)
                ),
            );
            app.modal = Some(Modal::Permission(req));
        }
        while let Ok(req) = app.question_rx.try_recv() {
            app.flush_streaming();
            app.push(LineKind::System, format!("[question] {}", req.question));
            app.modal = Some(Modal::Question {
                req,
                draft: String::new(),
                cursor: 0,
            });
        }

        if let Some(handle) = prompt_task.take() {
            if handle.is_finished() {
                match handle.await {
                    Ok(Ok((r, updated))) => {
                        app.session = updated;
                        app.record_usage(r.usage, r.iterations);
                        app.running = false;
                        app.status_msg = None;
                        app.active_tools.clear();
                        app.flush_streaming();
                        if r.usage.total() > 0 || r.iterations > 0 {
                            app.add_system(&format!(
                                "turn · in {} · out {} · Σ {} · ctx ~{}/{} · {} iter(s)",
                                format_tokens(r.usage.input_tokens),
                                format_tokens(r.usage.output_tokens),
                                format_tokens(r.usage.total()),
                                format_tokens(app.context_tokens() as u64),
                                format_tokens(app.max_context_tokens() as u64),
                                r.iterations
                            ));
                        }
                    }
                    Ok(Err(e)) => {
                        app.push(LineKind::Error, format!("[error] {}", e));
                        app.running = false;
                        app.status_msg = None;
                        app.active_tools.clear();
                        app.flush_streaming();
                    }
                    Err(e) => {
                        app.push(LineKind::Error, format!("[error] task: {}", e));
                        app.running = false;
                        app.active_tools.clear();
                    }
                }
            } else {
                prompt_task = Some(handle);
            }
        }

        terminal.draw(|frame| crate::harness::ui::tui::draw::draw(frame, &mut app))?;

        let poll_ms = if app.needs_anim() { 50 } else { 120 };
        if event::poll(std::time::Duration::from_millis(poll_ms))? {
            match event::read()? {
                Event::Key(key) => {
                    // Kitty protocol reports releases/repeats as well; only
                    // handle press events (otherwise Shift+Enter fires twice).
                    if key.kind != crossterm::event::KeyEventKind::Press {
                        continue;
                    }
                    // Skip splash on any key.
                    if app.splash.is_some() {
                        app.splash = None;
                        let fresh = app.session.messages.is_empty();
                        if fresh && app.skill_picker.is_none() {
                            app.open_skill_picker();
                        } else {
                            app.sync_prompt_toggles();
                        }
                        continue;
                    }
                    let quit = handle_key(&mut app, key, &mut prompt_task).await?;
                    if quit {
                        break;
                    }
                }
                Event::Mouse(m) => {
                    use crossterm::event::MouseEventKind;
                    if app.splash.is_some() {
                        continue;
                    }
                    match m.kind {
                        MouseEventKind::ScrollUp => app.scroll_by(-3),
                        MouseEventKind::ScrollDown => app.scroll_by(3),
                        MouseEventKind::Down(button)
                            if button == crossterm::event::MouseButton::Left =>
                        {
                            let area = app.transcript_area;
                            // Only clicks inside the transcript viewport hit a bubble.
                            if area.width > 0 && m.row >= area.y && m.row < area.y + area.height {
                                let row = app.transcript_scroll
                                    + (m.row - area.y).saturating_sub(0) as usize;
                                if let Some(&li) = app.transcript_row_map.get(row) {
                                    if let Some(line) = app.lines.get(li) {
                                        if line.kind == LineKind::User && app.modal.is_none() {
                                            // One prompt popup at a time.
                                            app.autocomplete = None;
                                            app.modal = Some(Modal::UserPrompt { line_idx: li });
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Event::Paste(text) => {
                    if app.splash.is_some() || app.modal.is_some() || app.palette.is_some() {
                        continue;
                    }
                    for c in text.chars() {
                        if c == '\n' || c == '\r' {
                            continue;
                        }
                        app.insert_char_fixed(c);
                    }
                }
                Event::Resize(..) => {}
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    drop(terminal);
    let mut stdout = std::io::stdout();
    execute!(stdout, crossterm::event::DisableMouseCapture)?;
    execute!(stdout, LeaveAlternateScreen)?;
    let _ = crossterm::execute!(stdout, crossterm::event::PopKeyboardEnhancementFlags);
    Ok(())
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        use crossterm::execute;
        use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::event::PopKeyboardEnhancementFlags
        );
    }
}

async fn handle_key(
    app: &mut App,
    key: KeyEvent,
    prompt_task: &mut Option<tokio::task::JoinHandle<Result<(PromptResult, Session)>>>,
) -> Result<bool> {
    use crossterm::event::{KeyCode, KeyModifiers};

    if app.skill_picker.is_some() {
        return handle_skill_picker_key(app, key);
    }

    if app.model_picker.is_some() {
        return handle_model_picker_key(app, key);
    }

    if app.auth_prompt.is_some() {
        return handle_auth_prompt_key(app, key);
    }

    if app.resume_picker.is_some() {
        return handle_resume_picker_key(app, key);
    }

    if app.modal.is_some() {
        return handle_modal_key(app, key);
    }

    if app.palette.is_some() {
        return handle_palette_key(app, key, prompt_task).await;
    }

    if app.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::F(1) => {
                app.show_help = false;
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                app.help_section =
                    (app.help_section + 1) % crate::harness::ui::tui::draw::help::section_count();
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                let n = crate::harness::ui::tui::draw::help::section_count();
                app.help_section = (app.help_section + n - 1) % n;
            }
            _ => {}
        }
        return Ok(false);
    }

    // Global shortcuts
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.running {
                app.abort.abort();
            }
            return Ok(true);
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.palette = Some(PaletteState::open(""));
            app.autocomplete = None;
            return Ok(false);
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cycle_theme();
            return Ok(false);
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_transcript();
            return Ok(false);
        }
        // Opencode-style line editing (prompt editor).
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_line_start();
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_line_end();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.kill_to_line_start();
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.kill_word_back();
        }
        KeyCode::F(1) => {
            app.show_help = true;
            return Ok(false);
        }
        KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.session.skills.is_empty() {
                return Ok(false);
            }
            app.cycle_focus();
            return Ok(false);
        }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.session.skills.is_empty() {
                return Ok(false);
            }
            app.cycle_focus();
            return Ok(false);
        }
        _ => {}
    }

    // Autocomplete navigation
    if let Some(ref mut ac) = app.autocomplete {
        match key.code {
            KeyCode::Up => {
                ac.move_sel(-1);
                return Ok(false);
            }
            KeyCode::Down => {
                ac.move_sel(1);
                return Ok(false);
            }
            KeyCode::Tab => {
                if let Some(item) = ac.current().cloned() {
                    app.input = item.payload;
                    app.input_cursor = app.input.chars().count();
                    app.autocomplete = AutoComplete::from_input(&app.input);
                }
                return Ok(false);
            }
            _ => {}
        }
    }

    // Skill chips focused: navigate/toggle chips instead of input.
    if app.skills_focused {
        match key.code {
            KeyCode::Up | KeyCode::Down => app.cycle_focus(),
            KeyCode::Char(' ') => app.toggle_prompt_skill(app.skills_idx),
            KeyCode::Left => {
                let n = app.prompt_toggles.as_ref().map(|t| t.len()).unwrap_or(0);
                if n > 0 {
                    app.skills_idx = (app.skills_idx + n - 1) % n;
                }
            }
            KeyCode::Right => {
                let n = app.prompt_toggles.as_ref().map(|t| t.len()).unwrap_or(0);
                if n > 0 {
                    app.skills_idx = (app.skills_idx + 1) % n;
                }
            }
            KeyCode::Enter => app.cycle_focus(),
            _ => return Ok(false),
        }
        return Ok(false);
    }

    match key.code {
        // Enter + any modifier inserts a newline (multi-line prompt / list
        // building): Shift+Enter, Shift+Return, Alt+Enter and Ctrl+Enter all
        // work here. On macOS terminals without the kitty keyboard protocol
        // (default Terminal.app), Ctrl+J is the reliable fallback.
        KeyCode::Enter if !key.modifiers.is_empty() => {
            app.insert_char_fixed('\n');
        }
        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.insert_char_fixed('\n');
        }
        KeyCode::Enter => {
            if submit_input(app, prompt_task).await? {
                return Ok(true);
            }
        }
        KeyCode::Char(c) => {
            if c == '?' && app.input.is_empty() {
                app.show_help = !app.show_help;
            } else {
                app.insert_char_fixed(c);
            }
        }
        KeyCode::Backspace => app.backspace(),
        KeyCode::Delete => app.delete_forward(),
        KeyCode::Left => app.cursor_left(),
        KeyCode::Right => app.cursor_right(),
        // Home/End (and Ctrl+A/E) act inside the current logical line.
        KeyCode::Home => app.cursor_line_start(),
        KeyCode::End => app.cursor_line_end(),
        KeyCode::Tab => {
            // Tab cycles the active mode when not navigating `/` autocomplete
            // (which is handled above when autocomplete is Some).
            if app.autocomplete.is_none() && !app.running {
                app.cycle_mode();
            }
        }
        KeyCode::Up => {
            if app.autocomplete.is_none() {
                // Multi-line input → move inside the editor; single-line falls
                // back to prompt history (opencode behavior).
                if app.input.contains('\n') {
                    app.cursor_visual_up();
                } else if !app.running {
                    app.history_up();
                }
            }
        }
        KeyCode::Down => {
            if app.autocomplete.is_none() {
                if app.input.contains('\n') {
                    app.cursor_visual_down();
                } else if !app.running {
                    app.history_down();
                }
            }
        }
        KeyCode::PageUp => app.scroll_by(-8),
        KeyCode::PageDown => app.scroll_by(8),
        KeyCode::Esc => {
            if app.autocomplete.is_some() {
                app.autocomplete = None;
            } else if app.running {
                // Cancel the running turn (opencode-style Esc cancel).
                app.abort.abort();
                app.status_msg = Some("cancelling…".to_string());
                app.push(LineKind::System, "[run cancelled by user]".to_string());
            } else if !app.input.is_empty() {
                app.input.clear();
                app.input_cursor = 0;
                app.autocomplete = None;
            }
        }
        _ => {}
    }
    Ok(false)
}

async fn handle_palette_key(
    app: &mut App,
    key: KeyEvent,
    prompt_task: &mut Option<tokio::task::JoinHandle<Result<(PromptResult, Session)>>>,
) -> Result<bool> {
    use crossterm::event::KeyCode;

    let action = {
        let pal = match app.palette.as_mut() {
            Some(p) => p,
            None => return Ok(false),
        };
        match key.code {
            KeyCode::Esc => {
                app.palette = None;
                return Ok(false);
            }
            KeyCode::Up => {
                pal.move_sel(-1);
                return Ok(false);
            }
            KeyCode::Down => {
                pal.move_sel(1);
                return Ok(false);
            }
            KeyCode::Backspace => {
                pal.backspace();
                return Ok(false);
            }
            KeyCode::Char(c) => {
                pal.push_char(c);
                return Ok(false);
            }
            KeyCode::Enter => pal.current().map(|i| i.payload.clone()),
            _ => return Ok(false),
        }
    };

    app.palette = None;
    if let Some(payload) = action {
        match payload.as_str() {
            "__help__" => app.show_help = true,
            "__clear__" => app.clear_transcript(),
            "__theme_cycle__" => app.cycle_theme(),
            "__skills__" => {
                app.open_skill_picker();
            }
            "__quit__" => return Ok(true),
            other if other.starts_with('/') => {
                app.input = other.to_string();
                app.input_cursor = app.input.chars().count();
                // If payload ends with space, leave for args; else submit.
                if !other.ends_with(' ') && submit_input(app, prompt_task).await? {
                    return Ok(true);
                }
            }
            _ => {}
        }
    }
    Ok(false)
}

/// Reads the system clipboard (best effort). Used for Ctrl+V in masked inputs.
fn paste_clipboard() -> Option<String> {
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut cb| cb.get_text().ok())
}

/// Handles a key while the session-memory skill picker is open.
fn handle_skill_picker_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use crossterm::event::KeyCode;
    let Some(picker) = app.skill_picker.as_mut() else {
        return Ok(false);
    };
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => picker.move_sel(-1),
        KeyCode::Down | KeyCode::Char('j') => picker.move_sel(1),
        KeyCode::Char(' ') => picker.toggle(),
        KeyCode::Char('a') => picker.toggle_all(),
        KeyCode::Enter => {
            app.apply_skill_picker()?;
            return Ok(false);
        }
        KeyCode::Esc => {
            app.apply_skill_picker()?;
            return Ok(false);
        }
        _ => {}
    }
    Ok(false)
}

/// `/settings` — show/update global limits (config.json), no modal needed.
fn handle_settings_command(app: &mut App, text: &str) {
    let rest = text.strip_prefix("/settings").unwrap_or("").trim();
    let mut parts = rest.split_whitespace();
    match parts.next() {
        None => {
            let c = &app.runtime.config;
            app.add_system(&format!(
                "settings · iterations {} · context {} · theme {} · provider {} · model {}",
                c.max_iterations, c.max_context_tokens, app.theme.name, c.provider, c.model
            ));
            app.add_system("usage: /settings iterations <n> · context <n> · theme <name>");
        }
        Some("iterations") => {
            let Some(n) = parts.next().and_then(|v| v.parse::<usize>().ok()) else {
                app.add_system("usage: /settings iterations <n> (e.g. 50)");
                return;
            };
            match app.runtime.update_settings(Some(n), None) {
                Ok(()) => app.add_system(&format!("settings · max_iterations = {}", n)),
                Err(e) => app.add_system(&format!("[error] {}", e)),
            }
        }
        Some("context") => {
            let Some(n) = parts.next().and_then(|v| v.parse::<usize>().ok()) else {
                app.add_system("usage: /settings context <tokens> (e.g. 100000)");
                return;
            };
            match app.runtime.update_settings(None, Some(n)) {
                Ok(()) => app.add_system(&format!("settings · max_context_tokens = {}", n)),
                Err(e) => app.add_system(&format!("[error] {}", e)),
            }
        }
        Some("theme") => match parts.next() {
            Some(name) if app.set_theme(name) => {
                app.add_system(&format!(
                    "theme → {} (saved to config.json)",
                    app.theme.name
                ));
            }
            Some(_) => app.add_system(&format!(
                "unknown theme (options: {})",
                Theme::names().join(", ")
            )),
            None => app.add_system(&format!("current theme: {}", app.theme.name)),
        },
        Some(other) => app.add_system(&format!(
            "unknown setting: {} (iterations · context · theme)",
            other
        )),
    }
}

/// Handles a key while the `/models` picker is open.
fn handle_model_picker_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let Some(picker) = app.model_picker.as_mut() else {
        return Ok(false);
    };

    // Typing a custom model name.
    if picker.custom_input.is_some() {
        match key.code {
            KeyCode::Esc => {
                picker.custom_input = None;
            }
            KeyCode::Backspace => {
                if let Some(inp) = picker.custom_input.as_mut() {
                    inp.pop();
                }
            }
            KeyCode::Enter => {
                if let Some(model) = picker.pick_model() {
                    let provider = picker.provider.clone();
                    app.model_picker = None;
                    app.apply_model_choice(&provider, &model)?;
                } else if picker.custom_input.is_none() {
                    // empty input → back to the model list
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(inp) = picker.custom_input.as_mut() {
                    inp.push(c);
                }
            }
            // Ctrl+V pastes from the system clipboard.
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(pasted) = paste_clipboard() {
                    if let Some(inp) = picker.custom_input.as_mut() {
                        inp.push_str(&pasted);
                    }
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => picker.move_sel(-1),
        KeyCode::Down | KeyCode::Char('j') => picker.move_sel(1),
        KeyCode::Esc => {
            if picker.stage_models {
                // Back to the provider stage.
                picker.stage_models = false;
                picker.selected = 0;
            } else {
                app.model_picker = None;
            }
        }
        KeyCode::Enter => {
            if !picker.stage_models {
                if let Some(name) = picker.items().get(picker.selected).cloned() {
                    picker.pick_provider(name);
                }
            } else if let Some(model) = picker.pick_model() {
                let provider = picker.provider.clone();
                app.model_picker = None;
                app.apply_model_choice(&provider, &model)?;
            }
        }
        _ => {}
    }
    Ok(false)
}

/// Handles a key while the session manager picker (/sessions) is open.
fn handle_resume_picker_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let Some(picker) = app.resume_picker.as_mut() else {
        return Ok(false);
    };
    if picker.sessions.is_empty() {
        app.resume_picker = None;
        return Ok(false);
    }

    // Inline rename editing.
    if let Some(text) = picker.rename_input.as_mut() {
        match key.code {
            KeyCode::Esc => {
                picker.rename_input = None;
            }
            KeyCode::Enter => {
                let new_title = text.trim().to_string();
                picker.rename_input = None;
                if !new_title.is_empty() {
                    let id = picker.sessions[picker.selected].id.clone();
                    app.runtime.set_session_title(&id, &new_title)?;
                    app.resume_picker = Some(ResumePickerState::new(app)?);
                    app.add_system("session renamed");
                }
            }
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                text.push(c);
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(pasted) = paste_clipboard() {
                    text.push_str(&pasted);
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => picker.move_sel(-1),
        KeyCode::Down | KeyCode::Char('j') => picker.move_sel(1),
        KeyCode::PageUp => picker.move_sel(-10),
        KeyCode::PageDown => picker.move_sel(10),
        KeyCode::Esc => {
            app.resume_picker = None;
        }
        KeyCode::Enter => {
            let id = picker.sessions[picker.selected].id.clone();
            app.resume_picker = None;
            if let Some(loaded) = app.runtime.load_session(&id)? {
                app.session = loaded;
                app.reset_usage();
                app.sync_prompt_toggles();
                app.add_system("session selected");
            } else {
                app.add_system("session not found");
            }
        }
        // Delete the selected session.
        KeyCode::Char('d') | KeyCode::Delete => {
            let id = picker.sessions[picker.selected].id.clone();
            let keep = picker.selected;
            app.resume_picker = None;
            app.runtime.delete_session(&id)?;
            app.add_system("session deleted");
            match ResumePickerState::new(app) {
                Ok(mut next) if !next.sessions.is_empty() => {
                    next.selected = keep.min(next.sessions.len() - 1);
                    app.resume_picker = Some(next);
                }
                Ok(_) => {}
                Err(e) => app.add_system(&format!("[error] failed to refresh sessions: {e}")),
            }
        }
        // Rename the selected session.
        KeyCode::Char('r') => {
            picker.rename_input = Some(picker.selected_title());
        }
        _ => {}
    }
    Ok(false)
}

/// Handles a key while the `/auth` token prompt is open.
fn handle_auth_prompt_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let Some(prompt) = app.auth_prompt.as_mut() else {
        return Ok(false);
    };
    match key.code {
        KeyCode::Esc => {
            app.auth_prompt = None;
            app.add_system("auth cancelled");
        }
        KeyCode::Enter => {
            let provider = prompt.provider.clone();
            let token = prompt.input.trim().to_string();
            app.auth_prompt = None;
            if token.is_empty() {
                app.add_system("[error] empty token — auth cancelled");
            } else {
                let mut store = crate::harness::auth::AuthStore::load();
                store.set_key(&provider, token.clone());
                match store.save() {
                    Ok(()) => {
                        // Point the runtime at this provider and rebuild the
                        // provider so the freshly saved token is live. This
                        // also force-enables the prompt (`is_configured`).
                        let model = if app.runtime.config.provider == provider {
                            app.runtime.config.model.clone()
                        } else {
                            crate::harness::provider::catalog::default_model(&provider)
                                .map(str::to_string)
                                .unwrap_or_default()
                        };
                        if let Err(e) = app.runtime.switch_model(&provider, &model) {
                            app.add_system(&format!("[error] applying provider: {}", e));
                        }
                        app.runtime.config.api_key = token.clone();
                        app.add_system(&format!(
                            "token saved for provider `{}` (auth.json, 0600){}",
                            provider,
                            if app.runtime.config.is_configured() {
                                " — ready to go"
                            } else {
                                ""
                            }
                        ));
                    }
                    Err(e) => app.add_system(&format!("[error] failed to save token: {}", e)),
                }
            }
        }
        KeyCode::Backspace => prompt.backspace(),
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => prompt.push_char(c),
        // Ctrl+V pastes from the system clipboard.
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(pasted) = paste_clipboard() {
                prompt.input.push_str(&pasted);
            }
        }
        _ => {}
    }
    Ok(false)
}

fn handle_modal_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use crossterm::event::KeyCode;
    match app.modal.take() {
        Some(Modal::Permission(req)) => {
            let reply = match key.code {
                KeyCode::Char('y') | KeyCode::Enter => true,
                KeyCode::Char('a') => {
                    app.runtime.permission.set_always_allow(&req.input.tool);
                    true
                }
                KeyCode::Char('n') | KeyCode::Esc => false,
                _ => {
                    app.modal = Some(Modal::Permission(req));
                    return Ok(false);
                }
            };
            let _ = req.reply.send(reply);
            app.push(
                LineKind::System,
                format!("[permission] {}", if reply { "allowed" } else { "denied" }),
            );
        }
        Some(Modal::Question {
            req,
            mut draft,
            mut cursor,
        }) => {
            use crossterm::event::KeyModifiers;

            let put_back = |app: &mut App, req: QuestionRequest, draft: String, cursor: usize| {
                app.modal = Some(Modal::Question { req, draft, cursor });
            };
            let finish = |app: &mut App, req: QuestionRequest, answer: Option<String>| {
                let _ = req.reply.send(answer.clone());
                match answer {
                    Some(a) => app.push(
                        LineKind::System,
                        format!("[question] answered: {}", preview(&a, 80)),
                    ),
                    None => app.push(LineKind::System, "[question] no answer".to_string()),
                }
            };
            let insert_char = |draft: &mut String, cursor: &mut usize, c: char| {
                let mut chars: Vec<char> = draft.chars().collect();
                let at = (*cursor).min(chars.len());
                chars.insert(at, c);
                *draft = chars.into_iter().collect();
                *cursor = at + 1;
            };

            match key.code {
                KeyCode::Esc => {
                    finish(app, req, None);
                }
                KeyCode::Enter => {
                    let trimmed = draft.trim();
                    if trimmed.is_empty() {
                        // Keep the modal open until the user types something or Esc.
                        put_back(app, req, draft, cursor);
                        return Ok(false);
                    }
                    let answer = if let Ok(num) = trimmed.parse::<usize>() {
                        // Bare number selects a predefined option when present.
                        if num >= 1 && num <= req.options.len() {
                            req.options[num - 1].clone()
                        } else {
                            trimmed.to_string()
                        }
                    } else {
                        trimmed.to_string()
                    };
                    finish(app, req, Some(answer));
                }
                // Every printable character goes into the free-text draft.
                // Option shortcuts still work: type "1" + Enter, or just "1" + Enter.
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                        && !key.modifiers.contains(KeyModifiers::SUPER) =>
                {
                    insert_char(&mut draft, &mut cursor, c);
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Backspace => {
                    if cursor > 0 {
                        let mut chars: Vec<char> = draft.chars().collect();
                        chars.remove(cursor - 1);
                        draft = chars.into_iter().collect();
                        cursor -= 1;
                    }
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Delete => {
                    let mut chars: Vec<char> = draft.chars().collect();
                    if cursor < chars.len() {
                        chars.remove(cursor);
                        draft = chars.into_iter().collect();
                    }
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Left => {
                    cursor = cursor.saturating_sub(1);
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Right => {
                    let len = draft.chars().count();
                    if cursor < len {
                        cursor += 1;
                    }
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Home => {
                    cursor = 0;
                    put_back(app, req, draft, cursor);
                }
                KeyCode::End => {
                    cursor = draft.chars().count();
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    draft.clear();
                    cursor = 0;
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let chars: Vec<char> = draft.chars().collect();
                    let mut i = cursor.min(chars.len());
                    while i > 0 && chars[i - 1].is_whitespace() {
                        i -= 1;
                    }
                    while i > 0 && !chars[i - 1].is_whitespace() {
                        i -= 1;
                    }
                    let mut kept = chars;
                    kept.drain(i..cursor.min(kept.len()));
                    draft = kept.into_iter().collect();
                    cursor = i;
                    put_back(app, req, draft, cursor);
                }
                _ => {
                    put_back(app, req, draft, cursor);
                }
            }
        }
        Some(Modal::UserPrompt { line_idx }) => {
            let mut continue_modal = true;
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    app.add_system("prompt action dismissed");
                }
                KeyCode::Char('c') => {
                    let text = user_prompt_text(app, line_idx);
                    if copy_to_clipboard(&text) {
                        app.push(LineKind::System, "prompt copied to clipboard".to_string());
                    } else {
                        app.push(LineKind::Error, "[error] clipboard unavailable".to_string());
                    }
                    // One-shot action: the popup closes after executing.
                    continue_modal = false;
                }
                KeyCode::Char('r') => {
                    match revert_to_prompt(app, line_idx) {
                        Ok(()) => app.add_system("session reverted to before this prompt"),
                        Err(e) => app.add_system(&format!("[error] revert failed: {}", e)),
                    }
                    continue_modal = false;
                }
                _ => {
                    app.modal = Some(Modal::UserPrompt { line_idx });
                }
            }
            if !continue_modal {
                app.modal = None;
            }
        }
        None => {}
    }
    Ok(false)
}

/// Text of the user prompt visually rendered at `line_idx` (clamped).
fn user_prompt_text(app: &App, line_idx: usize) -> String {
    app.lines
        .get(line_idx)
        .map(|l| l.text.clone())
        .unwrap_or_default()
}

fn copy_to_clipboard(text: &str) -> bool {
    match arboard::Clipboard::new() {
        Ok(mut cb) => cb.set_text(text.to_string()).is_ok(),
        Err(_) => false,
    }
}

/// Removes the clicked user prompt and everything after it (memory + SQLite).
fn revert_to_prompt(app: &mut App, line_idx: usize) -> Result<()> {
    // The clicked User bubble is the nth user prompt in the transcript.
    let nth = app
        .lines
        .iter()
        .take(line_idx + 1)
        .filter(|l| l.kind == LineKind::User)
        .count()
        .saturating_sub(1);
    let mut user_seen = 0usize;
    let mut cut_msg_index = None;
    for (i, m) in app.session.messages.iter().enumerate() {
        if m.role.as_str() == "user" {
            if user_seen == nth {
                cut_msg_index = Some(i);
                break;
            }
            user_seen += 1;
        }
    }
    let Some(msg_index) = cut_msg_index else {
        anyhow::bail!("user prompt not found");
    };
    let msg_id = app.session.messages[msg_index].id.clone();
    // DB: drop rows with ordinal >= this message.
    app.runtime
        .store
        .delete_messages_from(&app.session.id, &app.session.cwd, &msg_id)?;
    // Memory: drop messages from this prompt on.
    app.session.messages.truncate(msg_index);
    app.runtime.store.save_session(&app.session)?;
    app.tool_status = None;
    Ok(())
}

/// Returns Ok(true) when the app should quit.
async fn submit_input(
    app: &mut App,
    prompt_task: &mut Option<tokio::task::JoinHandle<Result<(PromptResult, Session)>>>,
) -> Result<bool> {
    let text = app.input.trim().to_string();
    if text.is_empty() {
        return Ok(false);
    }
    app.flush_streaming();
    app.input.clear();
    app.input_cursor = 0;
    app.autocomplete = None;
    app.history.push(text.clone());
    app.history_pos = None;

    if text.starts_with('/') {
        if text == "/theme" || text.starts_with("/theme ") {
            let arg = text.strip_prefix("/theme").unwrap_or("").trim();
            if arg.is_empty() {
                app.add_system(&format!(
                    "themes: {}  (current: {})",
                    Theme::names().join(", "),
                    app.theme.name
                ));
            } else if app.set_theme(arg) {
                app.add_system(&format!("theme → {}", app.theme.name));
            } else {
                app.add_system(&format!(
                    "unknown theme: {} (try {})",
                    arg,
                    Theme::names().join(", ")
                ));
            }
            return Ok(false);
        }

        if text == "/usage" || text == "/tokens" {
            for line in app.usage_report() {
                app.add_system(&line);
            }
            return Ok(false);
        }

        if text == "/skills" || text.starts_with("/skills ") {
            app.handle_skills_command(&text)?;
            return Ok(false);
        }

        // /models picker, /model and /provider direct switch, /auth token input.
        // /settings: show or update global limits/theme (persisted in config.json).
        if text == "/settings" || text.starts_with("/settings ") {
            handle_settings_command(app, &text);
            return Ok(false);
        }
        if text == "/models" || text.starts_with("/models ") {
            let arg = text.strip_prefix("/models").unwrap_or("").trim();
            if arg.is_empty() {
                app.open_models_picker();
            } else {
                // List the models of a provider in the transcript.
                let models = crate::harness::provider::catalog::models_for(arg);
                if models.is_empty() {
                    app.add_system(&format!("unknown provider: {} (try /models)", arg));
                } else {
                    app.add_system(&format!("models for {} ({}):", arg, models.len()));
                    for m in models {
                        app.add_system(&format!("  {}", m));
                    }
                }
            }
            return Ok(false);
        }
        if let Some(rest) = text.strip_prefix("/model ") {
            let model = rest.trim();
            if model.is_empty() {
                app.add_system(&format!(
                    "current model: {} (provider {})",
                    app.runtime.config.model, app.runtime.config.provider
                ));
            } else if app.running {
                app.add_system("[busy] cannot switch model while a turn is running");
            } else {
                let provider = app.runtime.config.provider.clone();
                app.apply_model_choice(&provider, model)?;
            }
            return Ok(false);
        }
        if text == "/model" {
            app.add_system(&format!(
                "current model: {} (provider {}) · usage: /model <name>",
                app.runtime.config.model, app.runtime.config.provider
            ));
            return Ok(false);
        }
        if let Some(rest) = text.strip_prefix("/provider ") {
            let provider = rest.trim();
            if provider.is_empty() {
                app.add_system(&format!(
                    "current provider: {}",
                    app.runtime.config.provider
                ));
            } else if app.running {
                app.add_system("[busy] cannot switch provider while a turn is running");
            } else if let Some(default_model) =
                crate::harness::provider::catalog::default_model(provider)
            {
                app.apply_model_choice(provider, default_model)?;
            } else {
                app.add_system(&format!(
                    "unknown provider: {} (options: {})",
                    provider,
                    crate::harness::provider::catalog::provider_names().join(", ")
                ));
            }
            return Ok(false);
        }
        if text == "/provider" {
            app.add_system(&format!(
                "current provider: {} (model {}) · usage: /provider <name>",
                app.runtime.config.provider, app.runtime.config.model
            ));
            return Ok(false);
        }
        if text == "/auth" || text.starts_with("/auth ") {
            let arg = text.strip_prefix("/auth").unwrap_or("").trim();
            // No argument → update the token for the current provider/model.
            let provider = if arg.is_empty() {
                app.runtime.config.provider.clone()
            } else {
                arg.to_string()
            };
            if provider.is_empty() {
                let store = crate::harness::auth::AuthStore::load();
                app.add_system("usage: /auth <provider> — stored providers:");
                let names: Vec<&str> = store.entries.keys().map(|s| s.as_str()).collect();
                if names.is_empty() {
                    app.add_system("  (none)");
                } else {
                    app.add_system(&format!("  {}", names.join(", ")));
                }
            } else if app.running {
                app.add_system("[busy] cannot run /auth while a turn is running");
            } else {
                app.auth_prompt = Some(AuthPromptState::new(provider));
            }
            return Ok(false);
        }
        if text == "/sessions" {
            if app.running {
                app.add_system("[busy] cannot switch session while a turn is running");
            } else {
                let picker = ResumePickerState::new(app)?;
                if picker.sessions.is_empty() {
                    app.add_system("no sessions to manage");
                } else {
                    app.resume_picker = Some(picker);
                }
            }
            return Ok(false);
        }

        match crate::harness::ui::commands::handle(&mut app.runtime, &mut app.session, &text)
            .await?
        {
            CommandOutcome::Exit => return Ok(true),
            CommandOutcome::Continue(lines) => {
                for l in lines {
                    app.add_system(&l);
                }
            }
        }
        // Fresh session counters + skill picker when switching sessions.
        if text == "/new" {
            app.reset_usage();
            app.open_skill_picker();
        } else if text == "/sessions" || text.starts_with("/sessions select ") {
            app.reset_usage();
            app.sync_prompt_toggles();
        }
        return Ok(false);
    }

    // Prompt input is hidden until a provider/model/token is configured.
    if !app.runtime.config.is_configured() {
        app.add_system("configure provider/model + token first — use /models and /auth");
        return Ok(false);
    }

    if app.running {
        app.add_system("[busy] still running a turn");
        return Ok(false);
    }

    app.add_user_prompt(&text);
    app.running = true;
    app.abort = AbortSignal::new();
    app.last_iterations = 0;

    let abort = app.abort.clone();
    let runtime = app.runtime.clone_shareable();
    let mut task_session = app.session.clone();
    let tx = app.events_tx.clone();
    let enabled_skills = app.enabled_skill_ids();
    app.skills_focused = false;

    let handle = tokio::spawn(async move {
        let result = runtime
            .prompt(&mut task_session, &tx, &text, abort, Some(&enabled_skills))
            .await;
        result.map(|r| (r, task_session))
    });
    *prompt_task = Some(handle);
    Ok(false)
}

#[cfg(test)]
mod input_tests {
    use super::*;

    fn row_text(input: &str, r: &VisualRow) -> String {
        let chars: Vec<char> = input.chars().collect();
        r.idxs.iter().map(|i| chars[*i]).collect()
    }

    #[test]
    fn test_soft_wrap_row_counts() {
        let long = "aaaa ".repeat(10);
        let w = 10usize;
        let rows = wrap_input_rows(&long, w);
        assert!(rows.iter().all(|r| r.len() <= w));
        assert!(rows.len() > 1);
        let flat: Vec<usize> = rows.iter().flatten().copied().collect();
        assert_eq!(flat, (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn test_word_wrap_moves_whole_words() {
        let input = "ola mundo ola";
        let rows = wrap_visual(input, 6);
        let texts: Vec<String> = rows.iter().map(|r| row_text(input, r)).collect();
        assert!(texts.iter().all(|t| !t.ends_with("mun")));
        assert!(texts.iter().any(|t| t.contains("mundo")));
    }

    #[test]
    fn test_cursor_mapping_round_trip() {
        let inputs = ["abc def", "top\n\nbottom", "one two three\nfour five"];
        for input in inputs {
            for w in [4usize, 6, 20] {
                let rows = wrap_visual(input, w);
                let last = input.chars().count();
                for cursor in 0..=last {
                    let (r, c) = visual_row_col(&rows, cursor);
                    assert!(r < rows.len(), "row out of bounds");
                    let idx = row_char_idx(&rows[r], c, input);
                    let (r2, c2) = visual_row_col(&rows, idx);
                    assert_eq!(
                        (r, c),
                        (r2, c2),
                        "cursor {} -> ({},{}); idx {} -> ({},{})",
                        cursor,
                        r,
                        c,
                        idx,
                        r2,
                        c2
                    );
                }
            }
        }
    }

    #[test]
    fn test_newlines_create_empty_rows() {
        let input = "top\n\nbottom";
        let rows = wrap_visual(input, 20);
        assert_eq!(rows.len(), 3);
        assert!(rows[1].idxs.is_empty());
        assert_eq!(visual_row_col(&rows, 4), (1, 0));
        assert_eq!(visual_row_col(&rows, 5), (2, 0));
    }

    #[test]
    fn test_visual_up_down_move_between_rows() {
        let mut app = App::inline_for_tests("one two three\nfour five");
        app.input_inner_width = 5;
        let rows = wrap_visual(&app.input, 5);
        assert!(rows.len() > 2);
        let last_row = rows.len() - 1;
        app.input_cursor = app.input.chars().count();
        app.cursor_visual_up();
        let (r, _) = visual_row_col(&rows, app.input_cursor);
        assert!(r < last_row, "expected to move up, row={}", r);
        app.cursor_visual_down();
        let (r2, _) = visual_row_col(&rows, app.input_cursor);
        assert_eq!(r2, last_row);
    }

    #[test]
    fn test_line_boundaries_editing_keys() {
        let mut app = App::inline_for_tests("first line\nsecond line");
        app.input_cursor = 14;
        app.cursor_line_start();
        assert_eq!(app.input_cursor, 11);

        app.cursor_line_end();
        assert_eq!(app.input_cursor, 22);

        app.kill_to_line_start();
        assert_eq!(app.input, "first line\n");

        app.input_cursor = 10;
        app.kill_word_back();
        assert_eq!(app.input, "first \n");
        assert_eq!(app.input_cursor, 6);
    }

    #[test]
    fn test_delete_forward() {
        let mut app = App::inline_for_tests("abc");
        app.input_cursor = 1;
        app.delete_forward();
        assert_eq!(app.input, "ac");
        app.input_cursor = 2;
        app.delete_forward();
        assert_eq!(app.input, "ac");
    }

    #[test]
    fn test_history_only_when_single_line() {
        let mut app = App::inline_for_tests("");
        app.history = vec!["old prompt".to_string()];
        assert!(!app.input.contains('\n'));
        app.history_up();
        assert_eq!(app.input, "old prompt");

        let mut app = App::inline_for_tests("linha1\nlinha2");
        app.history = vec!["old prompt".to_string()];
        let before = app.input.clone();
        app.cursor_visual_up();
        assert_eq!(app.history_pos, None);
        assert_eq!(app.input, before);
    }

    #[test]
    fn test_compact_window_small_no_hidden() {
        assert_eq!(compact_window(3, 1, 6), None);
        assert_eq!(compact_window(6, 5, 6), None);
    }

    #[test]
    fn test_compact_window_cursor_middle() {
        // 20 rows, cursor in the middle, window of 4.
        let (head, from, to) = compact_window(20, 10, 4).unwrap();
        assert_eq!(head, 2);
        // Cursor row (10) is inside the visible bottom region.
        assert!(from <= 10 && 10 < to);
        // Window math: head (2) + marker (1) + bottom rows ≤ max.
        assert_eq!(head + 1 + (to - from), 4);
    }

    #[test]
    fn test_compact_window_cursor_at_end() {
        let (head, from, to) = compact_window(20, 19, 6).unwrap();
        assert_eq!(head, 2);
        assert!(from <= 19 && 19 < to);
        assert_eq!(to, 20);
    }

    #[test]
    fn test_compact_window_cursor_always_visible_property() {
        for total in [4usize, 8, 15, 50] {
            for crow in 0..total {
                for max in [4usize, 6, 10] {
                    match compact_window(total, crow, max) {
                        None => assert!(crow < max, "cursor would be cut off"),
                        Some((_head, from, to)) => {
                            assert!(from <= crow && crow < to);
                            assert!(from >= 2, "marker must hide the middle, not the head");
                        }
                    }
                }
            }
        }
    }
}
