//! Selection state of a list, independent of any toolkit.
//!
//! Same API as ratatui's `ListState` for the parts the shell uses. The
//! offset (first visible row) belongs to the renderer: the TUI copies it
//! to and from ratatui's state at each frame.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListState {
    selected: Option<usize>,
    offset: usize,
}

impl ListState {
    pub fn with_selected(mut self, selected: Option<usize>) -> ListState {
        self.select(selected);
        self
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn select(&mut self, index: Option<usize>) {
        self.selected = index;
        if index.is_none() {
            self.offset = 0;
        }
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn set_offset(&mut self, offset: usize) {
        self.offset = offset;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deselect_resets_offset() {
        let mut s = ListState::default().with_selected(Some(3));
        s.set_offset(2);
        assert_eq!(s.selected(), Some(3));
        s.select(None);
        assert_eq!((s.selected(), s.offset()), (None, 0));
    }
}
