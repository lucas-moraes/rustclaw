//! Picker/modal overlay states extracted from `state.rs` (R6 split).
//!
//! Draw modules reach these via the `app::mod` re-exports.

use crate::harness::ui::tui::theme::Theme;
use crate::harness::ui::tui::transcript::TranscriptLine;

use super::state::{search_lines, App};

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
    /// First visible row index in the scrollable list.
    pub scroll_offset: usize,
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
            scroll_offset: 0,
        })
    }

    pub fn move_sel(&mut self, delta: i32) {
        if self.ids.is_empty() {
            return;
        }
        let len = self.ids.len() as i32;
        self.selected = ((self.selected as i32 + delta).rem_euclid(len)) as usize;
    }

    /// Keeps the selected row within the visible window, scrolling as needed.
    pub fn ensure_selected_visible(&mut self, visible: usize) {
        if visible == 0 || self.ids.is_empty() {
            return;
        }
        self.scroll_offset = crate::harness::ui::tui::scroll::ensure_visible(
            self.selected,
            self.scroll_offset,
            visible,
            self.ids.len(),
        );
    }

    /// Scrolls the list by `delta` rows (mouse wheel), keeping selection.
    pub fn scroll_by(&mut self, delta: i32) {
        if self.ids.is_empty() {
            return;
        }
        let max_offset = self.ids.len().saturating_sub(1);
        self.scroll_offset =
            (self.scroll_offset as i32 + delta).clamp(0, max_offset as i32) as usize;
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
    /// First visible row index in the scrollable list.
    pub scroll_offset: usize,
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
            scroll_offset: 0,
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

    /// Keeps the selected row within the visible window, scrolling as needed.
    pub fn ensure_selected_visible(&mut self, visible: usize) {
        if self.custom_input.is_some() || self.add_provider.is_some() {
            return;
        }
        let len = self.items().len();
        if visible == 0 || len == 0 {
            return;
        }
        self.scroll_offset = crate::harness::ui::tui::scroll::ensure_visible(
            self.selected,
            self.scroll_offset,
            visible,
            len,
        );
    }

    /// Scrolls the list by `delta` rows (mouse wheel), keeping selection.
    pub fn scroll_by(&mut self, delta: i32) {
        if self.custom_input.is_some() || self.add_provider.is_some() {
            return;
        }
        let len = self.items().len();
        if len == 0 {
            return;
        }
        let max_offset = len.saturating_sub(1);
        self.scroll_offset =
            (self.scroll_offset as i32 + delta).clamp(0, max_offset as i32) as usize;
    }
}

/// State of the theme picker overlay (Ctrl+T / `/theme`).
pub struct ThemePickerState {
    /// Row highlighted for keyboard navigation.
    pub selected: usize,
    /// First visible row index in the scrollable list.
    pub scroll_offset: usize,
    /// Theme name active when the picker opened (to restore on Esc).
    pub original: String,
}

impl ThemePickerState {
    /// Opens the picker with the current theme pre-selected.
    pub fn open(app: &App) -> Self {
        let names = Theme::names();
        let selected = names.iter().position(|n| *n == app.theme.name).unwrap_or(0);
        let mut list = crate::harness::ui::tui::fuzzy::FuzzyList::new(names);
        list.set_filter("");
        list.selected = selected;
        Self {
            selected: list.selected,
            scroll_offset: list.scroll_offset,
            original: app.theme.name.to_string(),
        }
    }

    fn as_list(&self) -> crate::harness::ui::tui::fuzzy::FuzzyList<&'static str> {
        let mut list = crate::harness::ui::tui::fuzzy::FuzzyList::new(Theme::names());
        list.selected = self.selected;
        list.scroll_offset = self.scroll_offset;
        list
    }

    fn apply_list(&mut self, list: crate::harness::ui::tui::fuzzy::FuzzyList<&'static str>) {
        self.selected = list.selected;
        self.scroll_offset = list.scroll_offset;
    }

    pub fn items(&self) -> Vec<&'static str> {
        Theme::names()
    }

    pub fn move_sel(&mut self, delta: i32) {
        let mut list = self.as_list();
        list.move_sel(delta);
        self.apply_list(list);
    }

    /// Keeps the selected row within the visible window, scrolling as needed.
    pub fn ensure_selected_visible(&mut self, visible: usize) {
        let mut list = self.as_list();
        list.ensure_visible(visible);
        self.apply_list(list);
    }

    /// Name of the highlighted theme, if any.
    pub fn current(&self) -> Option<&'static str> {
        self.as_list().current().copied()
    }

    /// Scrolls the list by `delta` rows (mouse wheel), keeping selection.
    pub fn scroll_by(&mut self, delta: i32) {
        let mut list = self.as_list();
        list.scroll_by(delta);
        self.apply_list(list);
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
        self.scroll_offset = crate::harness::ui::tui::scroll::ensure_visible(
            self.selected,
            self.scroll_offset,
            visible,
            self.sessions.len(),
        );
    }

    /// Scrolls the list by `delta` rows (mouse wheel), keeping selection.
    pub fn scroll_by(&mut self, delta: i32) {
        if self.sessions.is_empty() {
            return;
        }
        let max_offset = self.sessions.len().saturating_sub(1);
        self.scroll_offset =
            (self.scroll_offset as i32 + delta).clamp(0, max_offset as i32) as usize;
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

/// Tool services that take a token but are not model providers.
pub(crate) const TOOL_SERVICES: &[&str] = &["tavily"];

/// One selectable row in the `/auth` picker.
pub struct AuthItem {
    /// Token key stored in auth.json (e.g. "deepinfra", "tavily").
    pub name: String,
    /// Section label: "model providers" | "tool services" | "other stored tokens".
    pub section: &'static str,
    /// Whether a token is already stored for this key.
    pub has_token: bool,
}

/// `/auth` picker: lists model providers and tool services with a ✓/✗
/// marker, letting the user pick which token to set (or overwrite).
pub struct AuthPickerState {
    pub items: Vec<AuthItem>,
    pub selected: usize,
    pub scroll_offset: usize,
}

impl AuthPickerState {
    /// Builds the picker from the builtin provider catalog, the known tool
    /// services and any other tokens already present in `auth.json`.
    pub fn new() -> Self {
        let store = crate::harness::auth::AuthStore::load();
        let providers = crate::harness::provider::catalog::provider_names();
        let mut items: Vec<AuthItem> = Vec::new();

        for name in &providers {
            items.push(AuthItem {
                name: name.clone(),
                section: "model providers",
                has_token: store.get_key(name).is_some(),
            });
        }
        for name in TOOL_SERVICES {
            items.push(AuthItem {
                name: (*name).to_string(),
                section: "tool services",
                has_token: store.get_key(name).is_some(),
            });
        }
        for name in store.entries.keys() {
            let known = providers.iter().any(|p| p.eq_ignore_ascii_case(name))
                || TOOL_SERVICES.iter().any(|s| s.eq_ignore_ascii_case(name));
            if !known {
                items.push(AuthItem {
                    name: name.clone(),
                    section: "other stored tokens",
                    has_token: true,
                });
            }
        }

        Self {
            items,
            selected: 0,
            scroll_offset: 0,
        }
    }

    /// Moves the selection by `delta`, wrapping around the list.
    pub fn move_sel(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let len = self.items.len() as i32;
        self.selected = ((self.selected as i32 + delta).rem_euclid(len)) as usize;
    }

    /// Keeps the selected row within the visible window, scrolling as needed.
    pub fn ensure_selected_visible(&mut self, visible: usize) {
        if visible == 0 || self.items.is_empty() {
            return;
        }
        self.scroll_offset = crate::harness::ui::tui::scroll::ensure_visible(
            self.selected,
            self.scroll_offset,
            visible,
            self.items.len(),
        );
    }

    /// Scrolls the list by `delta` rows (mouse wheel), keeping selection.
    pub fn scroll_by(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let max_offset = self.items.len().saturating_sub(1);
        self.scroll_offset =
            (self.scroll_offset as i32 + delta).clamp(0, max_offset as i32) as usize;
    }
}

impl Default for AuthPickerState {
    fn default() -> Self {
        Self::new()
    }
}
