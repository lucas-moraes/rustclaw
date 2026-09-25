//! Prompt input editor: cursor movement, editing keys and history.
//!
//! These are `impl App` methods extracted from `app.rs` so the editor logic
//! (soft-wrap aware cursor movement, kill/delete, history browse) lives in its
//! own module instead of bloating the main application state file.

use crate::harness::ui::tui::app::App;
use crate::harness::ui::tui::input::{row_char_idx, visual_row_col, wrap_visual};

impl App {
    /// Inserts a char at the cursor (typing).
    pub fn insert_char_fixed(&mut self, c: char) {
        let mut chars: Vec<char> = self.input.chars().collect();
        let idx = self.input_cursor.min(chars.len());
        chars.insert(idx, c);
        self.input = chars.into_iter().collect();
        self.input_cursor += 1;
        self.refresh_autocomplete();
        self.mark_dirty();
    }

    /// Removes the char at the cursor (Delete key).
    pub fn delete_forward(&mut self) {
        if self.input_cursor < self.input.chars().count() {
            let mut chars: Vec<char> = self.input.chars().collect();
            chars.remove(self.input_cursor.min(chars.len() - 1));
            self.input = chars.into_iter().collect();
            self.refresh_autocomplete();
            self.mark_dirty();
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
        self.mark_dirty();
    }

    /// Moves the cursor to the end of the current logical line (End/Ctrl+E).
    pub fn cursor_line_end(&mut self) {
        let (_, end) = self.current_line_bounds();
        self.input_cursor = end;
        self.mark_dirty();
    }

    fn store_kill(&mut self, text: String) {
        if !text.is_empty() {
            self.kill_ring = Some(text);
        }
    }

    /// Ctrl+U: delete from the current line start up to the cursor.
    pub fn kill_to_line_start(&mut self) {
        let (start, _) = self.current_line_bounds();
        if start < self.input_cursor {
            let mut chars: Vec<char> = self.input.chars().collect();
            let killed: String = chars[start..self.input_cursor].iter().collect();
            chars.drain(start..self.input_cursor);
            self.input = chars.into_iter().collect();
            self.input_cursor = start;
            self.store_kill(killed);
            self.refresh_autocomplete();
            self.mark_dirty();
        }
    }

    /// Ctrl+K: delete from the cursor to the end of the current logical line.
    pub fn kill_to_line_end(&mut self) {
        let (_, end) = self.current_line_bounds();
        if self.input_cursor < end {
            let mut chars: Vec<char> = self.input.chars().collect();
            let killed: String = chars[self.input_cursor..end].iter().collect();
            chars.drain(self.input_cursor..end);
            self.input = chars.into_iter().collect();
            self.store_kill(killed);
            self.refresh_autocomplete();
            self.mark_dirty();
        }
    }

    /// Ctrl+Y: insert the kill-ring at the cursor.
    pub fn yank_kill_ring(&mut self) {
        let Some(text) = self.kill_ring.clone() else {
            return;
        };
        for c in text.chars() {
            self.insert_char_fixed(c);
        }
    }

    /// Word left: whitespace/`\n` boundary; does not cross the current line start.
    pub fn cursor_word_left(&mut self) {
        let (line_start, _) = self.current_line_bounds();
        if self.input_cursor <= line_start {
            return;
        }
        let chars: Vec<char> = self.input.chars().collect();
        let mut i = self.input_cursor.min(chars.len());
        while i > line_start && chars[i - 1].is_whitespace() && chars[i - 1] != '\n' {
            i -= 1;
        }
        while i > line_start && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        self.input_cursor = i;
        self.mark_dirty();
    }

    /// Word right: skip the current word then following whitespace (may cross `\n`).
    pub fn cursor_word_right(&mut self) {
        let chars: Vec<char> = self.input.chars().collect();
        let len = chars.len();
        let mut i = self.input_cursor.min(len);
        while i < len && !chars[i].is_whitespace() {
            i += 1;
        }
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        self.input_cursor = i;
        self.mark_dirty();
    }

    /// Alt+D: kill the word after the cursor (doesn't cross `\n`).
    pub fn kill_word_forward(&mut self) {
        let chars: Vec<char> = self.input.chars().collect();
        let len = chars.len();
        let start = self.input_cursor.min(len);
        let mut i = start;
        while i < len && chars[i].is_whitespace() && chars[i] != '\n' {
            i += 1;
        }
        while i < len && !chars[i].is_whitespace() {
            i += 1;
        }
        if i > start {
            let mut chars = chars;
            let killed: String = chars[start..i].iter().collect();
            chars.drain(start..i);
            self.input = chars.into_iter().collect();
            self.store_kill(killed);
            self.refresh_autocomplete();
            self.mark_dirty();
        }
    }

    /// Ctrl+Z: clear the prompt editor (text, cursor, history browse, autocomplete).
    pub fn clear_prompt_input(&mut self) {
        self.input.clear();
        self.input_cursor = 0;
        self.history_pos = None;
        self.autocomplete = None;
        self.mark_dirty();
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
        if i < self.input_cursor {
            let killed: String = chars[i..self.input_cursor].iter().collect();
            chars.drain(i..self.input_cursor);
            self.input = chars.into_iter().collect();
            self.input_cursor = i;
            self.store_kill(killed);
            self.refresh_autocomplete();
            self.mark_dirty();
        }
    }

    /// Visual cursor position after soft-wrapping: (row, col).
    /// Public helper; kept for API completeness (not currently consumed).
    #[allow(dead_code)]
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
        self.mark_dirty();
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
        self.mark_dirty();
    }

    pub fn backspace(&mut self) {
        if self.input_cursor > 0 {
            let mut chars: Vec<char> = self.input.chars().collect();
            chars.remove(self.input_cursor - 1);
            self.input = chars.into_iter().collect();
            self.input_cursor -= 1;
            self.refresh_autocomplete();
            self.mark_dirty();
        }
    }

    pub fn cursor_left(&mut self) {
        self.input_cursor = self.input_cursor.saturating_sub(1);
        self.mark_dirty();
    }
    pub fn cursor_right(&mut self) {
        if self.input_cursor < self.input.chars().count() {
            self.input_cursor += 1;
            self.mark_dirty();
        }
    }
    /// Move cursor to start of input. Public helper; kept for API completeness.
    #[allow(dead_code)]
    pub fn cursor_home(&mut self) {
        self.input_cursor = 0;
    }
    /// Move cursor to end of input. Public helper; kept for API completeness.
    #[allow(dead_code)]
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
        self.mark_dirty();
    }
    pub fn history_down(&mut self) {
        let Some(pos) = self.history_pos else { return };
        if pos + 1 >= self.history.len() {
            self.history_pos = None;
            self.input.clear();
            self.input_cursor = 0;
            self.refresh_autocomplete();
            self.mark_dirty();
            return;
        }
        let new_pos = pos + 1;
        self.history_pos = Some(new_pos);
        self.input = self.history[new_pos].clone();
        self.input_cursor = self.input.chars().count();
        self.refresh_autocomplete();
        self.mark_dirty();
    }

    /// Inserts pasted text, keeping newlines (`\r\n` / `\r` → `\n`).
    pub fn paste_text(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        for c in normalized.chars() {
            self.insert_char_fixed(c);
        }
    }
}
