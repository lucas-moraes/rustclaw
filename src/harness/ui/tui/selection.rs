//! Transcript text selection: hit-testing, range math, plain-text extract and
//! highlight styling for mouse drag-to-select + auto-copy.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Absolute cell inside the rendered transcript (row = plain_rows index).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellPos {
    pub row: usize,
    pub col: usize,
}

impl CellPos {
    pub fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }

    /// Chebyshev-ish distance used as drag threshold (row+col manhattan).
    pub fn manhattan(self, other: CellPos) -> usize {
        self.row.abs_diff(other.row) + self.col.abs_diff(other.col)
    }
}

/// Active (or just-finished) selection inside the transcript viewport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSelection {
    pub anchor: CellPos,
    pub head: CellPos,
    /// True while the primary button is held and a drag has started.
    pub dragging: bool,
}

impl TextSelection {
    pub fn new(anchor: CellPos) -> Self {
        Self {
            anchor,
            head: anchor,
            dragging: false,
        }
    }

    pub fn set_head(&mut self, head: CellPos) {
        self.head = head;
    }

    /// Inclusive ordered endpoints `(start, end)` in reading order.
    pub fn normalized(&self) -> (CellPos, CellPos) {
        normalize_range(self.anchor, self.head)
    }

    pub fn is_empty(&self, plain_rows: &[String]) -> bool {
        extract_text(plain_rows, self.anchor, self.head)
            .trim()
            .is_empty()
    }
}

/// Transient mouse gesture state while deciding click vs drag.
#[derive(Clone, Debug, Default)]
pub struct PendingClick {
    pub pos: CellPos,
    /// Absolute mouse coords at Down (for threshold in screen space too).
    pub screen_col: u16,
    pub screen_row: u16,
    /// `lines` index under the cursor at Down, if any.
    pub line_idx: Option<usize>,
}

/// Minimum manhattan cell distance before a Down becomes a drag-select.
pub const DRAG_THRESHOLD: usize = 2;

/// Orders two cell positions into reading-order start/end.
pub fn normalize_range(a: CellPos, b: CellPos) -> (CellPos, CellPos) {
    if a.row < b.row || (a.row == b.row && a.col <= b.col) {
        (a, b)
    } else {
        (b, a)
    }
}

/// Maps a screen mouse position to a transcript cell, or `None` if outside.
pub fn hit_test(
    area: Rect,
    scroll: usize,
    plain_rows: &[String],
    mx: u16,
    my: u16,
) -> Option<CellPos> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    if mx < area.x || mx >= area.x.saturating_add(area.width) {
        return None;
    }
    if my < area.y || my >= area.y.saturating_add(area.height) {
        return None;
    }
    let row = scroll + (my - area.y) as usize;
    if row >= plain_rows.len() {
        // Allow selecting past the last content row by clamping to EOF.
        if plain_rows.is_empty() {
            return None;
        }
        let last = plain_rows.len() - 1;
        let col = plain_rows[last].chars().count();
        return Some(CellPos::new(last, col));
    }
    let col_raw = (mx - area.x) as usize;
    let len = plain_rows[row].chars().count();
    let col = col_raw.min(len);
    Some(CellPos::new(row, col))
}

/// Concatenates span contents into a plain string (no styles).
pub fn line_to_plain(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>()
}

/// Extracts the selected plain text from rendered rows (char-indexed, not bytes).
pub fn extract_text(plain_rows: &[String], a: CellPos, b: CellPos) -> String {
    if plain_rows.is_empty() {
        return String::new();
    }
    let (start, end) = normalize_range(a, b);
    let last_idx = plain_rows.len() - 1;
    let start_row = start.row.min(last_idx);
    let end_row = end.row.min(last_idx);

    if start_row == end_row {
        let row = &plain_rows[start_row];
        let chars: Vec<char> = row.chars().collect();
        let s = start.col.min(chars.len());
        let e = end.col.min(chars.len());
        if s >= e {
            return String::new();
        }
        return chars[s..e].iter().collect();
    }

    let mut out = String::new();
    // First row: from start.col to end of line.
    {
        let row = &plain_rows[start_row];
        let chars: Vec<char> = row.chars().collect();
        let s = start.col.min(chars.len());
        out.extend(chars[s..].iter());
        out.push('\n');
    }
    // Middle rows: full lines.
    for row in plain_rows[(start_row + 1)..end_row].iter() {
        out.push_str(row);
        out.push('\n');
    }
    // Last row: from 0 to end.col.
    {
        let row = &plain_rows[end_row];
        let chars: Vec<char> = row.chars().collect();
        let e = end.col.min(chars.len());
        out.extend(chars[..e].iter());
    }
    out
}

/// Returns true when `abs_row` intersects the inclusive selection range.
pub fn row_in_selection(abs_row: usize, start: CellPos, end: CellPos) -> bool {
    abs_row >= start.row && abs_row <= end.row
}

/// Column range (start inclusive, end exclusive) selected on `abs_row`.
pub fn selected_cols(
    abs_row: usize,
    start: CellPos,
    end: CellPos,
    row_len: usize,
) -> (usize, usize) {
    if abs_row < start.row || abs_row > end.row {
        return (0, 0);
    }
    if start.row == end.row {
        let s = start.col.min(row_len);
        let e = end.col.min(row_len);
        return (s.min(e), s.max(e));
    }
    if abs_row == start.row {
        let s = start.col.min(row_len);
        return (s, row_len);
    }
    if abs_row == end.row {
        let e = end.col.min(row_len);
        return (0, e);
    }
    (0, row_len)
}

/// Style applied to selected cells.
pub fn selection_style(accent: ratatui::style::Color, bg: ratatui::style::Color) -> Style {
    Style::default()
        .bg(accent)
        .fg(bg)
        .add_modifier(Modifier::BOLD)
}

/// Rebuilds a ratatui `Line` applying selection highlight on `[col_start, col_end)`.
pub fn apply_highlight(
    line: Line<'static>,
    col_start: usize,
    col_end: usize,
    style: Style,
) -> Line<'static> {
    if col_start >= col_end {
        return line;
    }
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut cursor = 0usize;
    for span in line.spans {
        let content = span.content.as_ref();
        let len = content.chars().count();
        if len == 0 {
            continue;
        }
        let span_end = cursor + len;
        // No overlap with selection.
        if span_end <= col_start || cursor >= col_end {
            out.push(span);
            cursor = span_end;
            continue;
        }
        // Split this span into before / selected / after.
        let chars: Vec<char> = content.chars().collect();
        let local_sel_start = col_start.saturating_sub(cursor).min(len);
        let local_sel_end = col_end.saturating_sub(cursor).min(len);

        if local_sel_start > 0 {
            let before: String = chars[..local_sel_start].iter().collect();
            out.push(Span::styled(before, span.style));
        }
        if local_sel_end > local_sel_start {
            let mid: String = chars[local_sel_start..local_sel_end].iter().collect();
            out.push(Span::styled(mid, style));
        }
        if local_sel_end < len {
            let after: String = chars[local_sel_end..].iter().collect();
            out.push(Span::styled(after, span.style));
        }
        cursor = span_end;
    }
    Line::from(out)
}

/// Applies selection highlight to a visible row when it intersects the range.
pub fn highlight_visible_row(
    line: Line<'static>,
    abs_row: usize,
    start: CellPos,
    end: CellPos,
    style: Style,
) -> Line<'static> {
    if !row_in_selection(abs_row, start, end) {
        return line;
    }
    let plain = line_to_plain(&line);
    let row_len = plain.chars().count();
    let (cs, ce) = selected_cols(abs_row, start, end, row_len);
    apply_highlight(line, cs, ce, style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn test_normalize_range_orders() {
        let a = CellPos::new(2, 5);
        let b = CellPos::new(1, 3);
        let (s, e) = normalize_range(a, b);
        assert_eq!(s, b);
        assert_eq!(e, a);
        let (s2, e2) = normalize_range(b, a);
        assert_eq!(s2, b);
        assert_eq!(e2, a);
    }

    #[test]
    fn test_extract_text_single_line() {
        let rows = vec!["hello world".to_string()];
        let t = extract_text(&rows, CellPos::new(0, 0), CellPos::new(0, 5));
        assert_eq!(t, "hello");
        let t2 = extract_text(&rows, CellPos::new(0, 6), CellPos::new(0, 11));
        assert_eq!(t2, "world");
        let empty = extract_text(&rows, CellPos::new(0, 3), CellPos::new(0, 3));
        assert_eq!(empty, "");
    }

    #[test]
    fn test_extract_text_multi_line() {
        let rows = vec![
            "alpha".to_string(),
            "bravo".to_string(),
            "charlie".to_string(),
        ];
        let t = extract_text(&rows, CellPos::new(0, 2), CellPos::new(2, 3));
        assert_eq!(t, "pha\nbravo\ncha");
    }

    #[test]
    fn test_extract_text_clamps_cols() {
        let rows = vec!["ab".to_string()];
        let t = extract_text(&rows, CellPos::new(0, 0), CellPos::new(0, 99));
        assert_eq!(t, "ab");
    }

    #[test]
    fn test_hit_test_outside() {
        let area = Rect {
            x: 10,
            y: 5,
            width: 40,
            height: 10,
        };
        let rows = vec!["hello".to_string()];
        assert!(hit_test(area, 0, &rows, 0, 0).is_none());
        assert!(hit_test(area, 0, &rows, 10, 4).is_none());
        assert!(hit_test(area, 0, &rows, 50, 5).is_none());
    }

    #[test]
    fn test_hit_test_inside() {
        let area = Rect {
            x: 10,
            y: 5,
            width: 40,
            height: 10,
        };
        let rows = vec!["hello".to_string(), "world".to_string()];
        let p = hit_test(area, 0, &rows, 12, 5).unwrap();
        assert_eq!(p, CellPos::new(0, 2));
        let p2 = hit_test(area, 1, &rows, 10, 5).unwrap();
        assert_eq!(p2, CellPos::new(1, 0));
        // col past end clamps to len
        let p3 = hit_test(area, 0, &rows, 10 + 20, 5).unwrap();
        assert_eq!(p3, CellPos::new(0, 5));
    }

    #[test]
    fn test_selected_cols_variants() {
        let start = CellPos::new(1, 2);
        let end = CellPos::new(3, 4);
        assert_eq!(selected_cols(0, start, end, 10), (0, 0));
        assert_eq!(selected_cols(1, start, end, 10), (2, 10));
        assert_eq!(selected_cols(2, start, end, 10), (0, 10));
        assert_eq!(selected_cols(3, start, end, 10), (0, 4));
        // same row
        let a = CellPos::new(5, 3);
        let b = CellPos::new(5, 8);
        assert_eq!(selected_cols(5, a, b, 10), (3, 8));
    }

    #[test]
    fn test_apply_highlight_splits_spans() {
        let line = Line::from(vec![
            Span::raw("hello "),
            Span::styled("world", Style::default().fg(Color::Red)),
        ]);
        let style = selection_style(Color::Cyan, Color::Black);
        // Select "lo wo" (cols 3..8)
        let out = apply_highlight(line, 3, 8, style);
        let plain: String = out.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(plain, "hello world");
        // Middle span(s) should carry the selection style.
        let selected: String = out
            .spans
            .iter()
            .filter(|s| s.style.bg == Some(Color::Cyan))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(selected, "lo wo");
    }

    #[test]
    fn test_apply_highlight_empty_range_noop() {
        let line = Line::from("abc");
        let style = selection_style(Color::Cyan, Color::Black);
        let out = apply_highlight(line.clone(), 2, 2, style);
        assert_eq!(line_to_plain(&out), "abc");
    }

    #[test]
    fn test_manhattan_threshold() {
        let a = CellPos::new(0, 0);
        assert!(a.manhattan(CellPos::new(0, 1)) < DRAG_THRESHOLD);
        assert!(a.manhattan(CellPos::new(1, 1)) >= DRAG_THRESHOLD);
    }

    #[test]
    fn test_selection_is_empty() {
        let rows = vec!["  ".to_string(), "hi".to_string()];
        let mut sel = TextSelection::new(CellPos::new(0, 0));
        sel.set_head(CellPos::new(0, 2));
        assert!(sel.is_empty(&rows));
        sel.set_head(CellPos::new(1, 2));
        assert!(!sel.is_empty(&rows));
    }
}
