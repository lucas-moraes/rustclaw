//! Command palette + slash autocomplete.

use super::theme::Theme;

#[derive(Clone, Debug)]
pub struct PaletteItem {
    pub id: String,
    pub label: String,
    pub description: String,
    pub kind: PaletteKind,
    /// Text inserted / command executed on select.
    pub payload: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteKind {
    Command,
    Agent,
    Theme,
    Action,
}

#[derive(Clone, Debug)]
pub struct PaletteState {
    pub query: String,
    pub selected: usize,
    pub items: Vec<PaletteItem>,
    pub filtered: Vec<usize>,
}

impl PaletteState {
    #[allow(dead_code)] // convenience wrapper kept for API symmetry
    pub fn open(seed: &str) -> Self {
        Self::open_with(seed, &[])
    }

    /// `extra_agents`: custom agents (name, description) discovered from
    /// `.agents/agents/*.md`, appended after the builtin agent entries.
    pub fn open_with(seed: &str, extra_agents: &[(String, String)]) -> Self {
        let mut items = all_items();
        for (name, desc) in extra_agents {
            items.push(item(
                &format!("agent-{name}"),
                &format!("agent · {name}"),
                &desc.clone(),
                PaletteKind::Agent,
                &format!("/agent {name}"),
            ));
        }
        let mut s = Self {
            query: seed.to_string(),
            selected: 0,
            items,
            filtered: Vec::new(),
        };
        s.refilter();
        s
    }

    pub fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                if q.is_empty() {
                    return true;
                }
                it.label.to_lowercase().contains(&q)
                    || it.description.to_lowercase().contains(&q)
                    || it.id.to_lowercase().contains(&q)
                    || it.payload.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }

    pub fn move_sel(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as i32;
        let cur = self.selected as i32;
        self.selected = ((cur + delta).rem_euclid(len)) as usize;
    }

    pub fn current(&self) -> Option<&PaletteItem> {
        self.filtered
            .get(self.selected)
            .and_then(|i| self.items.get(*i))
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
        self.refilter();
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
        self.refilter();
    }
}

/// Inline autocomplete when typing `/…` in the prompt.
#[derive(Clone, Debug)]
pub struct AutoComplete {
    pub selected: usize,
    pub matches: Vec<PaletteItem>,
}

impl AutoComplete {
    pub fn from_input(input: &str) -> Option<Self> {
        if !input.starts_with('/') {
            return None;
        }
        // Don't show after a space (args mode) unless still matching command name.
        let cmd_part = input.split_whitespace().next().unwrap_or(input);
        let q = cmd_part.to_lowercase();
        let matches: Vec<PaletteItem> = all_items()
            .into_iter()
            .filter(|it| {
                it.kind == PaletteKind::Command
                    && (it.payload.to_lowercase().starts_with(&q)
                        || it.id.to_lowercase().contains(q.trim_start_matches('/')))
            })
            .collect();
        if matches.is_empty() {
            return None;
        }
        Some(Self {
            selected: 0,
            matches,
        })
    }

    pub fn move_sel(&mut self, delta: i32) {
        if self.matches.is_empty() {
            return;
        }
        let len = self.matches.len() as i32;
        self.selected = ((self.selected as i32 + delta).rem_euclid(len)) as usize;
    }

    pub fn current(&self) -> Option<&PaletteItem> {
        self.matches.get(self.selected)
    }
}

pub fn all_items() -> Vec<PaletteItem> {
    let mut items = vec![
        item(
            "help",
            "/help",
            "Show commands and tips",
            PaletteKind::Command,
            "/help",
        ),
        item(
            "new",
            "/new",
            "Start a fresh session",
            PaletteKind::Command,
            "/new",
        ),
        item(
            "sessions",
            "/sessions",
            "Manage sessions (list · select · delete · rename)",
            PaletteKind::Command,
            "/sessions",
        ),
        item(
            "fork",
            "/fork",
            "Fork the session (copy first N messages into a new one)",
            PaletteKind::Command,
            "/fork ",
        ),
        item(
            "agent",
            "/agent",
            "Switch agent (build/plan/explore/general/chat-free)",
            PaletteKind::Command,
            "/agent ",
        ),
        item(
            "compact",
            "/compact",
            "Compact conversation context",
            PaletteKind::Command,
            "/compact",
        ),
        item(
            "theme",
            "/theme",
            "List or set color theme",
            PaletteKind::Command,
            "/theme ",
        ),
        item(
            "usage",
            "/usage",
            "Show tokens and context usage",
            PaletteKind::Command,
            "/usage",
        ),
        item(
            "skills",
            "/skills",
            "Manage this session's skill memory",
            PaletteKind::Command,
            "/skills",
        ),
        item(
            "memory",
            "/memory",
            "List / search / rm / clear project memory (remember tool)",
            PaletteKind::Command,
            "/memory ",
        ),
        item(
            "export",
            "/export",
            "Export session transcript to Markdown or JSON",
            PaletteKind::Command,
            "/export ",
        ),
        item(
            "apply-plan",
            "/apply-plan",
            "Inject the plan agent's plan and switch to build",
            PaletteKind::Command,
            "/apply-plan",
        ),
        item(
            "image",
            "/image",
            "Attach an image (png/jpeg/gif/webp) to the next prompt",
            PaletteKind::Command,
            "/image ",
        ),
        item(
            "diff",
            "/diff",
            "Show file changes since the pre-agent snapshot",
            PaletteKind::Command,
            "/diff ",
        ),
        item(
            "restore",
            "/restore",
            "Restore a file to its pre-agent snapshot",
            PaletteKind::Command,
            "/restore ",
        ),
        item(
            "mcp",
            "/mcp",
            "MCP servers: list / status / restart",
            PaletteKind::Command,
            "/mcp ",
        ),
        item(
            "models",
            "/models",
            "Switch provider/model (opencode-style picker)",
            PaletteKind::Command,
            "/models",
        ),
        item(
            "auth",
            "/auth",
            "Save an API token for a provider (auth.json)",
            PaletteKind::Command,
            "/auth ",
        ),
        item(
            "settings",
            "/settings",
            "View / edit global limits and theme (config.json)",
            PaletteKind::Command,
            "/settings",
        ),
        item(
            "model",
            "/model",
            "Set the model for this project",
            PaletteKind::Command,
            "/model ",
        ),
        item(
            "provider",
            "/provider",
            "Switch provider (default model)",
            PaletteKind::Command,
            "/provider ",
        ),
        item(
            "permissions",
            "/permissions",
            "Set per-tool permission rules (allow · ask · deny)",
            PaletteKind::Command,
            "/permissions ",
        ),
        item(
            "allow-all-permissions",
            "/allow-all-permissions",
            "Grant all in-project permissions",
            PaletteKind::Command,
            "/allow-all-permissions",
        ),
        item(
            "jobs",
            "/jobs",
            "Background bash jobs: list / show output",
            PaletteKind::Command,
            "/jobs ",
        ),
        item(
            "undo",
            "/undo",
            "Revert to before the last prompt",
            PaletteKind::Command,
            "/undo",
        ),
        item(
            "skills-picker",
            "skills · picker",
            "Reopen the skill selection overlay",
            PaletteKind::Action,
            "__skills__",
        ),
        item(
            "exit",
            "/exit",
            "Quit RustClaw",
            PaletteKind::Command,
            "/exit",
        ),
        item(
            "agent-build",
            "agent · build",
            "Implementation-focused agent",
            PaletteKind::Agent,
            "/agent build",
        ),
        item(
            "agent-plan",
            "agent · plan",
            "Planning / design agent",
            PaletteKind::Agent,
            "/agent plan",
        ),
        item(
            "agent-explore",
            "agent · explore",
            "Read-only codebase explorer",
            PaletteKind::Agent,
            "/agent explore",
        ),
        item(
            "agent-general",
            "agent · general",
            "General-purpose agent",
            PaletteKind::Agent,
            "/agent general",
        ),
        item(
            "agent-chat-free",
            "agent · chat-free",
            "Free-form conversational assistant (Gemini/ChatGPT style)",
            PaletteKind::Agent,
            "/agent chat-free",
        ),
        item(
            "action-help",
            "toggle help",
            "Open the help overlay",
            PaletteKind::Action,
            "__help__",
        ),
        item(
            "action-clear",
            "clear transcript",
            "Clear local transcript view",
            PaletteKind::Action,
            "__clear__",
        ),
        item(
            "action-theme",
            "cycle theme",
            "Switch to next color theme",
            PaletteKind::Action,
            "__theme_cycle__",
        ),
        item(
            "action-quit",
            "quit",
            "Exit when idle",
            PaletteKind::Action,
            "__quit__",
        ),
    ];

    for name in Theme::names() {
        items.push(item(
            &format!("theme-{name}"),
            &format!("theme · {name}"),
            &format!("Apply the {name} theme"),
            PaletteKind::Theme,
            &format!("/theme {name}"),
        ));
    }

    items
}

fn item(id: &str, label: &str, description: &str, kind: PaletteKind, payload: &str) -> PaletteItem {
    PaletteItem {
        id: id.to_string(),
        label: label.to_string(),
        description: description.to_string(),
        kind,
        payload: payload.to_string(),
    }
}

pub fn kind_label(k: PaletteKind) -> &'static str {
    match k {
        PaletteKind::Command => "cmd",
        PaletteKind::Agent => "agent",
        PaletteKind::Theme => "theme",
        PaletteKind::Action => "action",
    }
}
