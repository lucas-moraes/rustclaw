//! Generic fuzzy-filterable list used by TUI pickers (models, skills, resume,
//! theme, palette). Subsequence match is case-insensitive; no extra crate.

/// An item that can be filtered by [`fuzzy_match`] on a haystack string.
pub trait FuzzyItem {
    fn haystack(&self) -> &str;
}

impl FuzzyItem for String {
    fn haystack(&self) -> &str {
        self
    }
}

impl FuzzyItem for &str {
    fn haystack(&self) -> &str {
        self
    }
}

/// Case-insensitive subsequence match. Returns a score (lower is better) when
/// every character of `query` appears in order in `candidate`. Empty query
/// matches everything with score `0`.
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let cand: Vec<char> = candidate.to_lowercase().chars().collect();
    let mut idx = 0usize;
    let mut score = 0i32;
    let mut last = 0usize;
    for (qi, qc) in query.to_lowercase().chars().enumerate() {
        let mut found = None;
        while idx < cand.len() {
            if cand[idx] == qc {
                found = Some(idx);
                idx += 1;
                break;
            }
            idx += 1;
        }
        let pos = found?;
        if qi > 0 {
            score += (pos.saturating_sub(last + 1)) as i32;
        }
        last = pos;
    }
    Some(score)
}

/// Filterable, scrollable list of `T`.
#[derive(Clone, Debug)]
pub struct FuzzyList<T> {
    pub items: Vec<T>,
    pub filter: String,
    pub selected: usize,
    pub scroll_offset: usize,
}

impl<T: FuzzyItem> FuzzyList<T> {
    pub fn new(items: Vec<T>) -> Self {
        Self {
            items,
            filter: String::new(),
            selected: 0,
            scroll_offset: 0,
        }
    }

    /// Original indices of items that match the current filter, in list order.
    pub fn filtered_indices(&self) -> Vec<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| fuzzy_match(&self.filter, it.haystack()).is_some())
            .map(|(i, _)| i)
            .collect()
    }

    pub fn visible_items(&self) -> Vec<(usize, &T)> {
        self.filtered_indices()
            .into_iter()
            .filter_map(|i| self.items.get(i).map(|it| (i, it)))
            .collect()
    }

    pub fn set_filter(&mut self, filter: impl Into<String>) {
        self.filter = filter.into();
        let n = self.filtered_indices().len();
        if n == 0 {
            self.selected = 0;
        } else if self.selected >= n {
            self.selected = n - 1;
        }
    }

    pub fn move_sel(&mut self, delta: i32) {
        let n = self.filtered_indices().len() as i32;
        if n == 0 {
            return;
        }
        self.selected = ((self.selected as i32 + delta).rem_euclid(n)) as usize;
    }

    pub fn ensure_visible(&mut self, view_h: usize) {
        let n = self.filtered_indices().len();
        if view_h == 0 || n == 0 {
            return;
        }
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + view_h {
            self.scroll_offset = self.selected + 1 - view_h;
        }
        let max_offset = n.saturating_sub(view_h);
        self.scroll_offset = self.scroll_offset.min(max_offset);
    }

    pub fn scroll_by(&mut self, delta: i32) {
        let n = self.filtered_indices().len();
        if n == 0 {
            return;
        }
        let max_offset = n.saturating_sub(1);
        self.scroll_offset =
            (self.scroll_offset as i32 + delta).clamp(0, max_offset as i32) as usize;
    }

    /// Original index of the highlighted filtered row.
    #[allow(dead_code)]
    pub fn current_index(&self) -> Option<usize> {
        self.filtered_indices().get(self.selected).copied()
    }

    pub fn current(&self) -> Option<&T> {
        self.current_index().and_then(|i| self.items.get(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_filters_subsequence() {
        assert!(fuzzy_match("cw", "cyberclaw").is_some());
        assert!(fuzzy_match("Cyb", "cyberclaw").is_some());
        assert!(fuzzy_match("xyz", "cyberclaw").is_none());
        assert!(fuzzy_match("ml", "model").is_some());
        assert!(fuzzy_match("lm", "model").is_none());
    }

    #[test]
    fn test_fuzzy_empty_query_shows_all() {
        let list = FuzzyList::new(vec!["alpha".to_string(), "beta".to_string()]);
        assert_eq!(list.filtered_indices(), vec![0, 1]);
        assert_eq!(fuzzy_match("", "anything"), Some(0));
    }

    #[test]
    fn test_move_sel_clamps() {
        let mut list = FuzzyList::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        list.move_sel(1);
        assert_eq!(list.selected, 1);
        list.move_sel(10);
        assert_eq!(list.selected, 2); // wraps via rem_euclid: 1+10=11, 11%3=2
        list.move_sel(-1);
        assert_eq!(list.selected, 1);
        list.selected = 0;
        list.move_sel(-1);
        assert_eq!(list.selected, 2);
    }

    #[test]
    fn test_ensure_visible_scrolls() {
        let mut list = FuzzyList::new((0..20).map(|i| format!("i{i}")).collect());
        list.selected = 15;
        list.ensure_visible(5);
        assert_eq!(list.scroll_offset, 11); // 15 + 1 - 5
        list.scroll_by(1);
        assert_eq!(list.scroll_offset, 12);
        assert!(list.selected >= list.scroll_offset);
        assert!(list.selected < list.scroll_offset + 5);
        list.selected = 2;
        list.ensure_visible(5);
        assert_eq!(list.scroll_offset, 2);
    }

    #[test]
    fn test_enter_selects_filtered_index() {
        let mut list = FuzzyList::new(vec![
            "apple".to_string(),
            "apricot".to_string(),
            "banana".to_string(),
        ]);
        list.set_filter("ap");
        assert_eq!(list.filtered_indices(), vec![0, 1]);
        list.selected = 1;
        assert_eq!(list.current_index(), Some(1));
        assert_eq!(list.current().map(|s| s.as_str()), Some("apricot"));

        list.set_filter("ban");
        assert_eq!(list.current_index(), Some(2));
        assert_eq!(list.current().map(|s| s.as_str()), Some("banana"));
    }
}
