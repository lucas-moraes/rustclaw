//! "Last code block" helpers: extract the most recent fenced code block from
//! the transcript and copy it to the clipboard (Ctrl+Y) or save it to a file
//! (Ctrl+S). Extracted from `app.rs`.

use crate::harness::ui::tui::app::{copy_to_clipboard, App, LineKind};

impl App {
    /// Extracts the most recent fenced code block (```…```) from assistant
    /// lines, walking backwards. Returns `None` when no complete block exists.
    pub fn last_code_block(&self) -> Option<String> {
        // Walk assistant lines in reverse, collecting fenced content.
        let mut fence_lines: Vec<String> = Vec::new();
        let mut in_fence = false;
        for line in self.lines.iter().rev() {
            if line.kind != LineKind::Assistant {
                if in_fence {
                    break;
                }
                continue;
            }
            for raw in line.text.lines().rev() {
                let trimmed = raw.trim_start();
                if trimmed.starts_with("```") {
                    if in_fence {
                        // Opening fence found: block complete.
                        fence_lines.reverse();
                        return Some(fence_lines.join("\n"));
                    }
                    in_fence = true;
                    continue;
                }
                if in_fence {
                    fence_lines.push(raw.to_string());
                }
            }
            if in_fence {
                break;
            }
        }
        None
    }

    /// Copies the last code block to the clipboard (Ctrl+Y).
    pub fn copy_last_code_block(&mut self) {
        match self.last_code_block() {
            Some(code) => {
                if copy_to_clipboard(&code) {
                    let n = code.chars().count();
                    self.status_msg = Some(format!("copied code block ({n} chars)"));
                } else {
                    self.push(LineKind::Error, "[error] clipboard unavailable".to_string());
                }
            }
            None => {
                self.status_msg = Some("no code block found".to_string());
            }
        }
    }

    /// Saves the last code block to a file (Ctrl+S). Writes to
    /// `rustclaw-code-<n>.txt` in the project cwd (or appends a counter).
    pub fn save_last_code_block(&mut self) {
        let Some(code) = self.last_code_block() else {
            self.status_msg = Some("no code block found".to_string());
            return;
        };
        for n in 1..=999 {
            let path = self.cwd.join(format!("rustclaw-code-{n}.txt"));
            if path.exists() {
                continue;
            }
            match std::fs::write(&path, &code) {
                Ok(()) => {
                    self.status_msg = Some(format!("saved code block → {}", path.display()));
                }
                Err(e) => {
                    self.push(LineKind::Error, format!("[error] failed to save: {e}"));
                }
            }
            return;
        }
        self.push(
            LineKind::Error,
            "[error] too many code files (999+)".to_string(),
        );
    }
}
