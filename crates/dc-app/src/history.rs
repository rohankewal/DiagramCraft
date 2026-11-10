//! Undo and redo.
//!
//! History stores whole-document snapshots rather than inverse commands. The
//! format caps a document at 2 MB, so a bounded stack of snapshots costs little
//! and cannot drift out of sync with the document the way hand-written inverse
//! operations do.

use dc_model::Document;

/// How many undo steps are retained.
pub const DEFAULT_LIMIT: usize = 128;

#[derive(Clone, Debug)]
pub struct History {
    past: Vec<Document>,
    future: Vec<Document>,
    limit: usize,
}

impl Default for History {
    fn default() -> Self {
        History::new(DEFAULT_LIMIT)
    }
}

impl History {
    pub fn new(limit: usize) -> Self {
        History { past: Vec::new(), future: Vec::new(), limit: limit.max(1) }
    }

    /// Records the state before a change. Call this immediately before
    /// mutating the document.
    ///
    /// A redo branch is discarded, since editing after undoing creates a new
    /// line of history.
    pub fn record(&mut self, current: &Document) {
        self.future.clear();
        self.past.push(current.clone());
        if self.past.len() > self.limit {
            self.past.remove(0);
        }
    }

    /// Steps back, returning the document to restore.
    pub fn undo(&mut self, current: &Document) -> Option<Document> {
        let previous = self.past.pop()?;
        self.future.push(current.clone());
        Some(previous)
    }

    /// Steps forward, returning the document to restore.
    pub fn redo(&mut self, current: &Document) -> Option<Document> {
        let next = self.future.pop()?;
        self.past.push(current.clone());
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// How many steps can still be undone.
    #[cfg(test)]
    pub fn undo_depth(&self) -> usize {
        self.past.len()
    }

    /// Forgets all history, as when a new document is opened.
    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::{BoundingBox, Shape, ShapeType};

    fn doc_with(n: usize) -> Document {
        let mut d = Document::new();
        for i in 0..n {
            d.pages[0].add_shape(Shape::new(
                format!("s{i}"),
                ShapeType::Rectangle,
                BoundingBox::new(0.0, 0.0, 10.0, 10.0),
            ));
        }
        d
    }

    #[test]
    fn a_fresh_history_has_nothing_to_undo() {
        let h = History::default();
        assert!(!h.can_undo());
        assert!(!h.can_redo());
    }

    #[test]
    fn undo_restores_the_recorded_state() {
        let mut h = History::default();
        let before = doc_with(1);
        h.record(&before);
        let after = doc_with(2);

        let restored = h.undo(&after).unwrap();
        assert_eq!(restored.pages[0].shapes().len(), 1);
        assert!(h.can_redo());
    }

    #[test]
    fn redo_returns_to_the_newer_state() {
        let mut h = History::default();
        let before = doc_with(1);
        h.record(&before);
        let after = doc_with(2);

        let undone = h.undo(&after).unwrap();
        let redone = h.redo(&undone).unwrap();
        assert_eq!(redone.pages[0].shapes().len(), 2);
    }

    #[test]
    fn multiple_steps_unwind_in_order() {
        let mut h = History::default();
        let mut current = doc_with(0);
        for n in 1..=3 {
            h.record(&current);
            current = doc_with(n);
        }

        assert_eq!(h.undo_depth(), 3);
        for expected in [2, 1, 0] {
            current = h.undo(&current).unwrap();
            assert_eq!(current.pages[0].shapes().len(), expected);
        }
        assert!(!h.can_undo());
    }

    #[test]
    fn editing_after_undo_discards_the_redo_branch() {
        let mut h = History::default();
        let v1 = doc_with(1);
        h.record(&v1);
        let v2 = doc_with(2);

        let back = h.undo(&v2).unwrap();
        assert!(h.can_redo());

        // A new edit from here replaces the abandoned future.
        h.record(&back);
        assert!(!h.can_redo(), "the stale redo branch should be gone");
    }

    #[test]
    fn history_is_bounded() {
        let mut h = History::new(3);
        let mut current = doc_with(0);
        for n in 1..=10 {
            h.record(&current);
            current = doc_with(n);
        }
        assert_eq!(h.undo_depth(), 3, "older entries should be dropped");

        // The three retained steps still unwind correctly.
        for expected in [9, 8, 7] {
            current = h.undo(&current).unwrap();
            assert_eq!(current.pages[0].shapes().len(), expected);
        }
        assert!(!h.can_undo());
    }

    #[test]
    fn a_zero_limit_is_raised_to_one() {
        let mut h = History::new(0);
        h.record(&doc_with(1));
        assert!(h.can_undo());
    }

    #[test]
    fn undo_on_empty_history_returns_nothing() {
        let mut h = History::default();
        assert!(h.undo(&doc_with(1)).is_none());
        assert!(h.redo(&doc_with(1)).is_none());
    }

    #[test]
    fn clear_forgets_everything() {
        let mut h = History::default();
        h.record(&doc_with(1));
        let undone = h.undo(&doc_with(2)).unwrap();
        let _ = undone;

        h.clear();
        assert!(!h.can_undo() && !h.can_redo());
    }
}
