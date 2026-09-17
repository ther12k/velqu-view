//! Host-provided clipboard access for M4c2 (ADR 0013).
//!
//! The core renderer has no ambient I/O (ADR 0004); the clipboard is no
//! exception. Copy/cut/paste are resolved through a host-installed
//! [`ClipboardProvider`]; the default [`NullClipboardProvider`] has no
//! clipboard at all.
//!
//! Fallibility is part of the contract: **cut deletes the selection only
//! after its write succeeded**. An unavailable or contended clipboard can
//! therefore never destroy user data — unlike a failed asset load, a lost
//! clipboard write is unrecoverable (there is no undo stack yet), so the
//! provider must be able to say no.

use std::fmt;

/// Why a clipboard operation failed (host-defined: clipboard contention,
/// an unsupported environment, a non-text payload, no provider installed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardError {
    /// Human-readable description. Renderer behavior depends only on the
    /// error's existence, never its contents.
    pub message: String,
}

impl ClipboardError {
    /// Creates an error with a host-chosen description.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ClipboardError {}

/// Host-provided system clipboard access.
///
/// Installed through [`crate::VelquView::set_clipboard_provider`]. The
/// renderer calls these synchronously while handling
/// [`crate::KeyCommand::Copy`]/[`crate::KeyCommand::Cut`]/[`crate::KeyCommand::Paste`].
/// Implementations must not panic; failures surface as `Err` and the
/// renderer's transactional rules decide what that means per command.
pub trait ClipboardProvider {
    /// The clipboard's current text, or `Ok(None)` when it holds no text.
    /// `Err` means the clipboard could not be reached (paste then no-ops).
    fn read(&self) -> Result<Option<String>, ClipboardError>;

    /// Replaces the clipboard text. Returning `Err` must mean the text is
    /// **not** on the clipboard: cut is transactional and keeps the
    /// selection whenever this fails.
    fn write(&self, text: &str) -> Result<(), ClipboardError>;
}

/// The default provider: no clipboard at all (NullHost behavior).
///
/// The asymmetry is deliberate and load-bearing: `read` reports "no text"
/// (`Ok(None)`) so paste no-ops, but `write` **fails** — a null host
/// cannot accept clipboard data, and a cut whose write went nowhere must
/// not delete the selection (ADR 0013: a failed asset load never destroys
/// user data either; a silently-lossy cut would).
#[derive(Debug, Clone, Copy, Default)]
pub struct NullClipboardProvider;

impl ClipboardProvider for NullClipboardProvider {
    fn read(&self) -> Result<Option<String>, ClipboardError> {
        Ok(None)
    }

    fn write(&self, _text: &str) -> Result<(), ClipboardError> {
        Err(ClipboardError::new("no clipboard provider installed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_provider_reads_nothing_and_refuses_writes() {
        let provider = NullClipboardProvider;
        assert_eq!(provider.read(), Ok(None));
        assert!(provider.write("anything").is_err());
    }
}
