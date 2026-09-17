//! Runtime text-editor primitives for M4c1 controls.
//!
//! Positions are UTF-8 byte offsets, but every public operation normalizes
//! them to valid boundaries and moves/deletes by extended grapheme clusters.
//! The module does not know about DOM, layout, or platform keyboard events.

use unicode_segmentation::UnicodeSegmentation;

/// A single-line or multiline editor value with UTF-8-safe selection state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditorState {
    value: String,
    anchor: usize,
    focus: usize,
}

impl EditorState {
    /// Creates an editor with a collapsed caret at the start.
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            anchor: 0,
            focus: 0,
        }
    }

    pub(crate) fn value(&self) -> &str {
        &self.value
    }

    pub(crate) fn anchor(&self) -> usize {
        self.anchor
    }

    pub(crate) fn focus(&self) -> usize {
        self.focus
    }

    pub(crate) fn set_selection(&mut self, anchor: usize, focus: usize) {
        self.anchor = self.normalize_boundary(anchor);
        self.focus = self.normalize_boundary(focus);
    }

    pub(crate) fn selected_range(&self) -> Option<(usize, usize)> {
        let start = self.anchor.min(self.focus);
        let end = self.anchor.max(self.focus);
        (start < end).then_some((start, end))
    }

    /// The selected text, or `None` when the selection is collapsed
    /// (M4c2: the copy/cut payload).
    pub(crate) fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selected_range()?;
        Some(&self.value[start..end])
    }

    pub(crate) fn collapse_to(&mut self, offset: usize) {
        let offset = self.normalize_boundary(offset);
        self.anchor = offset;
        self.focus = offset;
    }

    pub(crate) fn select_all(&mut self) {
        self.anchor = 0;
        self.focus = self.value.len();
    }

    pub(crate) fn move_left(&mut self, extend: bool) {
        let destination = if !extend && self.selected_range().is_some() {
            self.anchor.min(self.focus)
        } else {
            self.previous_grapheme(self.focus)
        };
        self.move_focus(destination, extend);
    }

    pub(crate) fn move_right(&mut self, extend: bool) {
        let destination = if !extend && self.selected_range().is_some() {
            self.anchor.max(self.focus)
        } else {
            self.next_grapheme(self.focus)
        };
        self.move_focus(destination, extend);
    }

    pub(crate) fn move_home(&mut self, extend: bool) {
        let destination = self.line_start(self.focus);
        self.move_focus(destination, extend);
    }

    pub(crate) fn move_end(&mut self, extend: bool) {
        let destination = self.line_end(self.focus);
        self.move_focus(destination, extend);
    }

    pub(crate) fn move_vertical(&mut self, direction: i32, extend: bool) {
        let current_start = self.line_start(self.focus);
        let current_column = self.focus.saturating_sub(current_start);
        let target = if direction < 0 {
            current_start
                .checked_sub(1)
                .map(|offset| self.line_start(offset))
        } else {
            self.line_end(self.focus)
                .checked_add(1)
                .filter(|offset| *offset <= self.value.len())
                .map(|offset| self.line_start(offset))
        };
        let Some(target_start) = target else {
            return;
        };
        let destination = (target_start + current_column).min(self.line_end(target_start));
        self.move_focus(destination, extend);
    }

    /// Deletes the current selection, if any (M4c2: cut). A collapsed
    /// selection deletes nothing.
    pub(crate) fn delete_selection(&mut self) -> bool {
        if let Some((start, end)) = self.selected_range() {
            self.replace_range(start, end, "");
            true
        } else {
            false
        }
    }

    pub(crate) fn backspace(&mut self) -> bool {
        if let Some((start, end)) = self.selected_range() {
            self.replace_range(start, end, "");
            return true;
        }
        if self.focus == 0 {
            return false;
        }
        let start = self.previous_grapheme(self.focus);
        self.replace_range(start, self.focus, "");
        true
    }

    pub(crate) fn delete(&mut self) -> bool {
        if let Some((start, end)) = self.selected_range() {
            self.replace_range(start, end, "");
            return true;
        }
        if self.focus >= self.value.len() {
            return false;
        }
        let end = self.next_grapheme(self.focus);
        self.replace_range(self.focus, end, "");
        true
    }

    pub(crate) fn insert_text(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let (start, end) = self.selected_range().unwrap_or((self.focus, self.focus));
        self.replace_range(start, end, text);
        true
    }

    fn move_focus(&mut self, destination: usize, extend: bool) {
        let destination = self.normalize_boundary(destination);
        if extend {
            self.focus = destination;
        } else {
            self.anchor = destination;
            self.focus = destination;
        }
    }

    /// Replaces `start..end` with `replacement`, collapsing the caret to
    /// the replacement's end (M4c3 commit path; ranges are normalized).
    pub(crate) fn replace_range(&mut self, start: usize, end: usize, replacement: &str) {
        let start = self.normalize_boundary(start);
        let end = self.normalize_boundary(end).max(start);
        self.value.replace_range(start..end, replacement);
        let caret = start + replacement.len();
        self.anchor = caret;
        self.focus = caret;
    }

    fn normalize_boundary(&self, offset: usize) -> usize {
        let offset = offset.min(self.value.len());
        if self.value.is_char_boundary(offset) {
            return offset;
        }
        let mut boundary = offset;
        while boundary > 0 && !self.value.is_char_boundary(boundary) {
            boundary -= 1;
        }
        boundary
    }

    fn previous_grapheme(&self, offset: usize) -> usize {
        let offset = self.normalize_boundary(offset);
        self.value
            .grapheme_indices(true)
            .map(|(start, _)| start)
            .take_while(|&start| start < offset)
            .last()
            .unwrap_or(0)
    }

    fn next_grapheme(&self, offset: usize) -> usize {
        let offset = self.normalize_boundary(offset);
        self.value
            .grapheme_indices(true)
            .find_map(|(start, grapheme)| (start >= offset).then_some(start + grapheme.len()))
            .unwrap_or(self.value.len())
    }

    fn line_start(&self, offset: usize) -> usize {
        let offset = self.normalize_boundary(offset);
        self.value[..offset]
            .rfind('\n')
            .map_or(0, |index| index + 1)
    }

    fn line_end(&self, offset: usize) -> usize {
        let offset = self.normalize_boundary(offset);
        self.value[offset..]
            .find('\n')
            .map_or(self.value.len(), |index| offset + index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_moves_and_deletes_by_grapheme() {
        let mut editor = EditorState::new("abc");
        editor.move_right(false);
        editor.move_right(false);
        assert_eq!(editor.focus(), 2);
        assert!(editor.backspace());
        assert_eq!(editor.value(), "ac");
        assert_eq!(editor.focus(), 1);
        assert!(editor.delete());
        assert_eq!(editor.value(), "a");
    }

    #[test]
    fn precomposed_accent_is_one_grapheme() {
        let mut editor = EditorState::new("éx");
        editor.move_right(false);
        assert_eq!(editor.focus(), "é".len());
        assert!(editor.backspace());
        assert_eq!(editor.value(), "x");
    }

    #[test]
    fn combining_sequence_is_one_grapheme() {
        let value = "a\u{301}b";
        let mut editor = EditorState::new(value);
        editor.move_right(false);
        assert_eq!(editor.focus(), "a\u{301}".len());
        assert!(editor.backspace());
        assert_eq!(editor.value(), "b");
    }

    #[test]
    fn emoji_is_one_grapheme_and_offsets_remain_boundaries() {
        let mut editor = EditorState::new("a😀b");
        editor.move_right(false);
        editor.move_right(false);
        assert_eq!(editor.focus(), "a😀".len());
        assert!(editor.backspace());
        assert_eq!(editor.value(), "ab");
        assert!(editor.value().is_char_boundary(editor.focus()));
    }

    #[test]
    fn selection_replaces_without_splitting_utf8() {
        let mut editor = EditorState::new("a😀b");
        editor.set_selection(1, 1 + "😀".len());
        assert_eq!(editor.selected_range(), Some((1, 5)));
        assert!(editor.insert_text("é"));
        assert_eq!(editor.value(), "aéb");
        assert_eq!(editor.anchor(), "aé".len());
        assert_eq!(editor.focus(), "aé".len());
    }

    #[test]
    fn home_and_end_are_line_local() {
        let mut editor = EditorState::new("ab\ncd");
        editor.set_selection(4, 4);
        editor.move_home(false);
        assert_eq!(editor.focus(), 3);
        editor.move_end(false);
        assert_eq!(editor.focus(), 5);
    }
}
