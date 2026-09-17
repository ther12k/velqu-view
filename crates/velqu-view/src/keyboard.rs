//! Backend-independent keyboard input for M4c controls.
//!
//! Platform shells translate their native key events into this small command
//! vocabulary and pass text separately. Command keys therefore never become
//! accidental text insertion (for example Enter carrying `"\r"`).

/// Named editing/navigation commands understood by M4c controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCommand {
    /// Delete the grapheme immediately before the caret.
    Backspace,
    /// Delete the grapheme immediately after the caret.
    Delete,
    /// Move left by one grapheme.
    Left,
    /// Move right by one grapheme.
    Right,
    /// Move up within a textarea line layout (profile fallback is line-local).
    Up,
    /// Move down within a textarea line layout (profile fallback is line-local).
    Down,
    /// Move to the current line's start.
    Home,
    /// Move to the current line's end.
    End,
    /// Insert a newline when the focused control accepts it.
    Enter,
    /// Move focus to the next control; never inserted as text.
    Tab,
    /// Clear transient keyboard handling; never inserted as text.
    Escape,
    /// Select the entire current value.
    SelectAll,
    /// Copy the current selection to the host clipboard (M4c2).
    Copy,
    /// Copy the selection and delete it, when the control is editable
    /// (M4c2). Readonly controls ignore cut entirely, as browsers do.
    Cut,
    /// Replace the current selection with the host clipboard's text
    /// (M4c2).
    Paste,
}

/// Modifier state after platform translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyModifiers {
    /// Control modifier (Windows/Linux convention).
    pub ctrl: bool,
    /// Command modifier (macOS convention).
    pub command: bool,
    /// Shift modifier, used for extending selection.
    pub shift: bool,
    /// Alt/Option modifier.
    pub alt: bool,
}

impl KeyModifiers {
    /// Whether this is a select-all shortcut modifier combination.
    pub(crate) fn select_all(self) -> bool {
        (self.ctrl || self.command) && !self.alt
    }
}
