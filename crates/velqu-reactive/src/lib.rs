//! # velqu-reactive
//!
//! Velqu Reactive is VelquView's Alpine-inspired reactive markup layer.
//!
//! **M0 scope (this crate today):** the frozen v0 *syntax surface* as data —
//! directive names, event names, event modifiers, and attribute-binding names
//! — plus strict parsers used for static validation and diagnostics
//! (`@click.prevent=…`, `:disabled=…`, `vx-*`). Unknown names are errors, never
//! silently ignored; that is what lets tooling and AI agents catch typos
//! before runtime.
//!
//! **M5 scope (not built yet):** state scopes, the isolated UI QuickJS
//! expression context, the binding/invalidation graph, and native fast paths.
//! The runtime will live here without changing the names below.
//!
//! The UI QuickJS context receives no browser APIs — no `document`, `window`,
//! `navigator`, storage, network, or filesystem — and never owns the DOM:
//! expressions mutate reactive state; Rust applies resulting DOM changes.

use std::fmt;

/// v0 directives (`vx-*` attribute names), per the Velqu Reactive v0 spec.
pub const DIRECTIVES: &[&str] = &[
    "vx-state",
    "vx-computed",
    "vx-text",
    "vx-show",
    "vx-if",
    "vx-for",
    "vx-key",
    "vx-model",
];

/// v0 event names usable in `@<name>` handlers.
pub const EVENTS: &[&str] = &["click", "input", "change", "submit", "keydown", "keyup"];

/// v0 event modifiers (`.prevent`, `.stop`, …).
pub const MODIFIERS: &[&str] = &["prevent", "stop", "once", "enter", "escape"];

/// v0 attribute bindings (`:<name>` bindings).
pub const ATTRIBUTE_BINDINGS: &[&str] = &["class", "style", "value", "disabled", "checked"];

/// A parsed `vx-*` directive name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Directive {
    /// `vx-state` — declares a reactive state scope.
    State,
    /// `vx-computed` — derived values (optional v0.x).
    Computed,
    /// `vx-text` — element text content binding.
    Text,
    /// `vx-show` — visibility binding.
    Show,
    /// `vx-if` — conditional tree insertion (optional v0.x).
    If,
    /// `vx-for` — list rendering, requires `vx-key` (optional v0.x).
    For,
    /// `vx-key` — stable key for list items.
    Key,
    /// `vx-model` — two-way form control binding.
    Model,
}

impl Directive {
    /// The canonical attribute name for this directive.
    pub fn attribute_name(self) -> &'static str {
        match self {
            Directive::State => "vx-state",
            Directive::Computed => "vx-computed",
            Directive::Text => "vx-text",
            Directive::Show => "vx-show",
            Directive::If => "vx-if",
            Directive::For => "vx-for",
            Directive::Key => "vx-key",
            Directive::Model => "vx-model",
        }
    }
}

/// A parsed `@event.mod1.mod2` handler attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventHandler {
    /// Base event name (e.g. `click`).
    pub event: String,
    /// Modifiers in written order (e.g. `["prevent"]`).
    pub modifiers: Vec<String>,
}

/// Static syntax errors with enough context for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactiveSyntaxError {
    /// The attribute is not a known directive (`vx-…`).
    UnknownDirective(String),
    /// The `@…` attribute names an event outside the v0 set.
    UnknownEvent(String),
    /// An event modifier outside the v0 set.
    UnknownModifier(String),
    /// A `:…` binding name outside the v0 set.
    UnknownBinding(String),
}

impl fmt::Display for ReactiveSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReactiveSyntaxError::UnknownDirective(name) => write!(
                f,
                "unknown directive {name:?}: expected one of {}",
                DIRECTIVES.join(", ")
            ),
            ReactiveSyntaxError::UnknownEvent(name) => {
                write!(
                    f,
                    "unknown event {name:?}: expected one of {}",
                    EVENTS.join(", ")
                )
            }
            ReactiveSyntaxError::UnknownModifier(name) => write!(
                f,
                "unknown event modifier {name:?}: expected one of {}",
                MODIFIERS.join(", ")
            ),
            ReactiveSyntaxError::UnknownBinding(name) => write!(
                f,
                "unknown attribute binding {name:?}: expected one of {}",
                ATTRIBUTE_BINDINGS.join(", ")
            ),
        }
    }
}

impl std::error::Error for ReactiveSyntaxError {}

/// Parses a `vx-*` attribute name into a [`Directive`].
///
/// ```
/// use velqu_reactive::{parse_directive, Directive};
/// assert_eq!(parse_directive("vx-show").unwrap(), Directive::Show);
/// assert!(parse_directive("vx-toggled").is_err());
/// ```
pub fn parse_directive(attribute: &str) -> Result<Directive, ReactiveSyntaxError> {
    let directive = match attribute {
        "vx-state" => Directive::State,
        "vx-computed" => Directive::Computed,
        "vx-text" => Directive::Text,
        "vx-show" => Directive::Show,
        "vx-if" => Directive::If,
        "vx-for" => Directive::For,
        "vx-key" => Directive::Key,
        "vx-model" => Directive::Model,
        other => {
            return Err(ReactiveSyntaxError::UnknownDirective(other.to_owned()));
        }
    };
    Ok(directive)
}

/// Parses the name part of an event handler attribute (what follows `@`).
///
/// ```
/// use velqu_reactive::parse_event_handler;
/// let handler = parse_event_handler("submit.prevent").unwrap();
/// assert_eq!(handler.event, "submit");
/// assert_eq!(handler.modifiers, ["prevent"]);
/// assert!(parse_event_handler("tap").is_err());
/// ```
pub fn parse_event_handler(name: &str) -> Result<EventHandler, ReactiveSyntaxError> {
    let mut parts = name.split('.');
    let event = parts.next().unwrap_or_default();
    if !EVENTS.contains(&event) {
        return Err(ReactiveSyntaxError::UnknownEvent(event.to_owned()));
    }
    let mut modifiers = Vec::new();
    for modifier in parts {
        if !MODIFIERS.contains(&modifier) {
            return Err(ReactiveSyntaxError::UnknownModifier(modifier.to_owned()));
        }
        modifiers.push(modifier.to_owned());
    }
    Ok(EventHandler {
        event: event.to_owned(),
        modifiers,
    })
}

/// Parses the name part of an attribute binding (what follows `:`).
///
/// ```
/// use velqu_reactive::parse_attribute_binding;
/// assert_eq!(parse_attribute_binding("disabled").unwrap(), "disabled");
/// assert!(parse_attribute_binding("href").is_err());
/// ```
pub fn parse_attribute_binding(name: &str) -> Result<&'static str, ReactiveSyntaxError> {
    ATTRIBUTE_BINDINGS
        .iter()
        .find(|binding| *binding == &name)
        .copied()
        .ok_or_else(|| ReactiveSyntaxError::UnknownBinding(name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_sets_are_frozen() {
        // Exact-match assertions keep the v0 surface from drifting silently.
        assert_eq!(
            DIRECTIVES,
            &[
                "vx-state",
                "vx-computed",
                "vx-text",
                "vx-show",
                "vx-if",
                "vx-for",
                "vx-key",
                "vx-model"
            ]
        );
        assert_eq!(
            EVENTS,
            &["click", "input", "change", "submit", "keydown", "keyup"]
        );
        assert_eq!(MODIFIERS, &["prevent", "stop", "once", "enter", "escape"]);
        assert_eq!(
            ATTRIBUTE_BINDINGS,
            &["class", "style", "value", "disabled", "checked"]
        );
    }

    #[test]
    fn directive_round_trip() {
        for name in DIRECTIVES {
            let directive = parse_directive(name).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(directive.attribute_name(), *name);
        }
    }

    #[test]
    fn event_handlers_parse_with_modifiers() {
        let plain = parse_event_handler("click").unwrap();
        assert_eq!(plain.modifiers.len(), 0);

        let fancy = parse_event_handler("keydown.enter.once").unwrap();
        assert_eq!(fancy.event, "keydown");
        assert_eq!(fancy.modifiers, ["enter", "once"]);

        assert_eq!(
            parse_event_handler("click.self").unwrap_err(),
            ReactiveSyntaxError::UnknownModifier("self".into())
        );
    }

    #[test]
    fn attribute_bindings_validate() {
        for name in ATTRIBUTE_BINDINGS {
            assert_eq!(parse_attribute_binding(name).unwrap(), *name);
        }
        assert!(matches!(
            parse_attribute_binding("title"),
            Err(ReactiveSyntaxError::UnknownBinding(_))
        ));
    }

    #[test]
    fn errors_display_lists_expectations() {
        let err = parse_directive("vx-magic").unwrap_err();
        assert!(err.to_string().contains("vx-state"));
    }
}
