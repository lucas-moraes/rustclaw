//! Input helpers and key binding documentation for the TUI.

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
            let last_len = rows.last().map(|r| r.idxs.len()).unwrap_or(0);
            if let Some(b) = break_at.filter(|b| *b + 1 < last_len) {
                // Move the trailing word (after the break space) to a new row.
                if let Some(last) = rows.last_mut() {
                    let cut: Vec<usize> = last.idxs.drain(b + 1..).collect();
                    if let Some(&first) = cut.first() {
                        rows.push(VisualRow {
                            start: first,
                            idxs: cut,
                        });
                    }
                }
            } else {
                rows.push(VisualRow {
                    idxs: Vec::new(),
                    start: i,
                });
            }
            row_len = rows.last().map(|r| r.idxs.len()).unwrap_or(0);
            break_at = None;
        }
        if let Some(last) = rows.last_mut() {
            last.idxs.push(i);
        }
        row_len += 1;
        if c == ' ' {
            break_at = Some(
                rows.last()
                    .map(|r| r.idxs.len())
                    .unwrap_or(0)
                    .saturating_sub(1),
            );
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
pub fn row_char_idx(row: &VisualRow, col: usize, input: &str) -> usize {
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

/// Key binding help text (also mirrored in draw/help.rs).
#[allow(dead_code)]
pub const HELP: &str = "\
  Ctrl+C     quit (exit the project)
  Enter      send prompt
  Shift/Alt+Enter  line break
  Ctrl+J     line break (macOS fallback)
  Esc        cancel streaming/run · close overlay · clear draft
  Up/Down    history (single-line) / move between lines
  Ctrl+A/E   line start / line end
  Ctrl+U/W   kill to line start / kill word
  Ctrl+Z     reset prompt input
  Del        delete char at cursor
  PgUp/PgDn  scroll transcript
  x          expand/collapse thinking (reasoning) blocks
  Drag       select transcript text (auto-copy on release)
  Ctrl+C     copy selection · quit (no selection)
  Esc        (same: cancel in-flight action first)
  Ctrl+P     command palette
  Ctrl+T     cycle theme
  Ctrl+L     clear transcript
  Ctrl+Y     copy last code block
  Ctrl+S     save last code block to file
  ? / F1     help overlay
  Tab        autocomplete (in /) / cycle mode
  y/n/a      permission modal
  1..n / type  question modal (pick option or free-text + Enter)
  /help      slash commands
";

#[cfg(test)]
mod tests {
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
