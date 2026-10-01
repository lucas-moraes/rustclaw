//! Shared list scroll / viewport layout for pickers and modals.

/// Keeps `selected` within the `[scroll_offset, scroll_offset + view_h)` window.
pub fn ensure_visible(selected: usize, scroll_offset: usize, view_h: usize, total: usize) -> usize {
    if view_h == 0 || total == 0 {
        return scroll_offset;
    }
    let mut off = scroll_offset;
    if selected < off {
        off = selected;
    } else if selected >= off + view_h {
        off = selected + 1 - view_h;
    }
    let max_offset = total.saturating_sub(view_h);
    off.min(max_offset)
}

/// Layout for a scrollable list with a fixed footer (hint) and optional position row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListViewport {
    /// How many item rows fit above the footer.
    pub item_rows: usize,
    /// Whether to reserve a row for `↑ first–last of total ↓`.
    pub show_position: bool,
}

/// Computes how many item rows fit in `inner_height`, always reserving `hint_rows`
/// at the bottom. When `total` exceeds the space, one row is reserved for the
/// position indicator. `item_rows` is always at least 1 when `inner_height > 0`
/// and `total > 0`.
pub fn list_viewport(inner_height: usize, total: usize, hint_rows: usize) -> ListViewport {
    if total == 0 || inner_height == 0 {
        return ListViewport {
            item_rows: 0,
            show_position: false,
        };
    }
    let after_hint = inner_height.saturating_sub(hint_rows);
    if after_hint == 0 {
        return ListViewport {
            item_rows: 1,
            show_position: total > 1,
        };
    }
    if total <= after_hint {
        return ListViewport {
            item_rows: after_hint.max(1).min(total),
            show_position: false,
        };
    }
    let item_rows = after_hint.saturating_sub(1).max(1);
    ListViewport {
        item_rows,
        show_position: true,
    }
}

/// Formats `↑ 16–30 of 42 ↓` (arrows only when more content exists above/below).
pub fn format_position_indicator(scroll_offset: usize, item_rows: usize, total: usize) -> String {
    if total == 0 {
        return String::new();
    }
    let first = scroll_offset + 1;
    let last = (scroll_offset + item_rows).min(total);
    let has_above = scroll_offset > 0;
    let has_below = scroll_offset + item_rows < total;
    let mut s = String::from("  ");
    if has_above {
        s.push('↑');
        s.push(' ');
    }
    s.push_str(&format!("{}–{} of {}", first, last, total));
    if has_below {
        s.push(' ');
        s.push('↓');
    }
    s
}

/// Adaptive modal height for the cursor model picker (borders + hint + items).
pub fn cursor_model_modal_height(item_count: usize, terminal_height: u16) -> u16 {
    let n = item_count as u16;
    // +1 row for the filter input line above the list.
    let h = (6 + n).min(terminal_height.saturating_sub(2)).min(24);
    h.max(8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_viewport_fits_all_without_indicator() {
        let v = list_viewport(10, 5, 1);
        assert_eq!(v.item_rows, 5);
        assert!(!v.show_position);
    }

    #[test]
    fn list_viewport_reserves_indicator_row() {
        let v = list_viewport(10, 20, 1);
        assert!(v.show_position);
        assert_eq!(v.item_rows, 8);
    }

    #[test]
    fn list_viewport_minimum_one_row() {
        let v = list_viewport(2, 50, 1);
        assert_eq!(v.item_rows, 1);
        assert!(v.show_position);
    }

    #[test]
    fn ensure_visible_scrolls_down() {
        assert_eq!(ensure_visible(15, 0, 5, 20), 11);
    }

    #[test]
    fn ensure_visible_scrolls_up() {
        assert_eq!(ensure_visible(2, 10, 5, 20), 2);
    }

    #[test]
    fn format_position_indicator_arrows() {
        assert_eq!(format_position_indicator(0, 5, 20), "  1–5 of 20 ↓");
        assert_eq!(format_position_indicator(15, 5, 20), "  ↑ 16–20 of 20");
        assert_eq!(format_position_indicator(5, 5, 20), "  ↑ 6–10 of 20 ↓");
    }
}
