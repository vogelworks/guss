//! Command/search history with navigation.

use std::collections::HashMap;

pub struct History {
    items: HashMap<String, Vec<String>>,
    current_index: Option<usize>,
}

impl History {
    pub fn new() -> Self {
        History {
            items: HashMap::new(),
            current_index: None,
        }
    }

    /// Add an item to history for a context.
    pub fn add(&mut self, context: &str, item: String) {
        self.items.entry(context.to_string()).or_insert_with(Vec::new).push(item);
        self.current_index = None;
    }

    /// Navigate back in history (left arrow).
    pub fn prev(&mut self, context: &str) -> Option<String> {
        if let Some(hist) = self.items.get(context) {
            if hist.is_empty() {
                return None;
            }
            let new_index = match self.current_index {
                None => hist.len() - 1,
                Some(i) if i > 0 => i - 1,
                Some(i) => i,
            };
            self.current_index = Some(new_index);
            return Some(hist[new_index].clone());
        }
        None
    }

    /// Navigate forward in history (right arrow).
    pub fn next(&mut self, context: &str) -> Option<String> {
        if let Some(hist) = self.items.get(context) {
            if hist.is_empty() {
                return None;
            }
            let new_index = match self.current_index {
                Some(i) if i < hist.len() - 1 => i + 1,
                Some(_i) => {
                    self.current_index = None;
                    return None;
                }
                None => return None,
            };
            self.current_index = Some(new_index);
            return Some(hist[new_index].clone());
        }
        None
    }

    /// Get current item without advancing.
    pub fn current(&self, context: &str) -> Option<String> {
        if let Some(hist) = self.items.get(context) {
            if let Some(idx) = self.current_index {
                return Some(hist[idx].clone());
            }
        }
        None
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigates_per_context() {
        let mut h = History::new();
        h.add("root", "a".into());
        h.add("root", "b".into());
        h.add("docs", "x".into());

        assert_eq!(h.next("root"), None, "next before prev is a no-op");
        assert_eq!(h.prev("root").as_deref(), Some("b"));
        assert_eq!(h.prev("root").as_deref(), Some("a"));
        assert_eq!(h.prev("root").as_deref(), Some("a"), "stops at oldest");
        assert_eq!(h.next("root").as_deref(), Some("b"));
        assert_eq!(h.next("root"), None, "past newest resets");
        assert_eq!(h.current("root"), None);
    }
}
