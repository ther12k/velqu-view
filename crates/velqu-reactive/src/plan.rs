//! The reactive binding compiler (M5b, ADR 0016).
//!
//! **Reactive markup is compiled into a capability-limited execution
//! plan; runtime JavaScript never discovers or traverses the DOM.**
//!
//! The compiler is a pure Rust pass over a host DOM (the
//! [`ReactiveDom`] trait — `velqu-view` implements it for its parsed
//! document). No QuickJS executes here: every recognized `vx-*`
//! directive, `:attr` binding, and `@event` handler lowers to a typed
//! entry carrying the internal node, its lexical scope, the raw
//! expression/handler source, and a deterministic source span.
//! Malformed, unknown, conflicting, or semantically incompatible
//! markup produces diagnostics — never silent treatment as ordinary
//! HTML, and never a runtime surprise deferred to M5c.
//!
//! The frozen v0 *runtime* surface (what lowers to bindings):
//! `vx-state`, `vx-text`, `vx-show`, `vx-model`, `:class/:style/:
//! value/:disabled/:checked`, and `@click/@input/@change/@submit/
//! @keydown/@keyup`. `vx-if`/`vx-for`/`vx-key`/`vx-computed` are known
//! names whose dynamic-tree semantics are deferred: they compile to a
//! distinct diagnostic, not a binding.

use std::fmt;

use crate::{Directive, parse_attribute_binding, parse_directive, parse_event_handler};

/// A host DOM the compiler can walk. Implemented by `velqu-view` for its
/// parsed document and by tests for synthetic trees; the compiler sees
/// only this surface — never the host's DOM types.
pub trait ReactiveDom {
    /// Opaque node identity (the host's internal id; never crosses into
    /// JavaScript).
    type Node: Copy + Eq + fmt::Debug;

    /// The document root to walk (preorder).
    fn root(&self) -> Self::Node;
    /// `node`'s element children, in document order.
    fn children(&self, node: Self::Node) -> Vec<Self::Node>;
    /// `node`'s tag name, lowercased ("" for non-elements).
    fn tag(&self, node: Self::Node) -> &str;
    /// `node`'s attributes in source order as `(name, value)` pairs.
    /// Programmatically built DOMs may repeat names; the compiler
    /// detects and diagnoses duplicates deterministically.
    fn attributes(&self, node: Self::Node) -> Vec<(String, String)>;
}

/// What a binding computes and applies (M5c semantics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingKind {
    /// `vx-text` — the element's text content.
    Text,
    /// `vx-show` — visibility (presentation-only when hidden via paint).
    Show,
    /// `:class` — the class attribute value (string).
    Class,
    /// `:style` — the style attribute value (string).
    Style,
    /// `:value` — a control's value.
    Value,
    /// `:disabled` — a control's disabled state.
    Disabled,
    /// `:checked` — a checkbox-like control's checked state.
    Checked,
    /// `vx-model` — two-way control binding.
    Model,
}

/// One compiled state scope (`vx-state`): a node and the initializer
/// expression that will seed its reactive state (M5c evaluates it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopePlan<N> {
    /// The node carrying `vx-state`.
    pub node: N,
    /// Index of the enclosing scope (nearest ancestor `vx-state`),
    /// `None` for roots.
    pub parent: Option<usize>,
    /// The initializer expression source, verbatim.
    pub initializer_source: String,
    /// Where the directive occurred (document-order ordinal + name).
    pub span: SourceSpan,
}

/// One compiled binding: evaluate `expression_source` in `scope` and
/// apply the result to `node` as `kind` dictates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding<N> {
    /// The target node.
    pub node: N,
    /// The binding's lexical scope (nearest enclosing `vx-state`).
    pub scope: usize,
    /// What the result applies to.
    pub kind: BindingKind,
    /// The expression source, verbatim.
    pub expression_source: String,
    /// Where the binding occurred.
    pub span: SourceSpan,
}

/// One compiled event handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventBinding<N> {
    /// The node carrying the handler.
    pub node: N,
    /// The handler's lexical scope.
    pub scope: usize,
    /// The parsed `@event.modifiers` name.
    pub handler: crate::EventHandler,
    /// The handler source, verbatim.
    pub handler_source: String,
    /// Where the handler occurred.
    pub span: SourceSpan,
}

/// Where a directive occurred, deterministically: the node's
/// document-order ordinal plus the attribute name. Byte-accurate source
/// offsets would require parser span tracking and are deliberately
/// deferred (ADR 0016).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    /// Preorder ordinal of the element within the walked tree.
    pub node_ordinal: usize,
    /// The attribute as written (e.g. `vx-text`, `@click.prevent`).
    pub attribute: String,
}

/// A compile diagnostic, in deterministic document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactiveDiagnostic {
    /// Where the offending markup occurred.
    pub span: SourceSpan,
    /// What was wrong and (where applicable) the deterministic
    /// resolution the compiler applied.
    pub message: String,
}

impl fmt::Display for ReactiveDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "node {} {}: {}",
            self.span.node_ordinal, self.span.attribute, self.message
        )
    }
}

/// The compiled plan for one document: scopes, bindings, and event
/// handlers, plus the diagnostics collected along the way. Pure data —
/// the runtime (M5c) consumes it; JavaScript never sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactiveDocument<N> {
    /// `vx-state` scopes, in document order.
    pub scopes: Vec<ScopePlan<N>>,
    /// Bindings, in document order.
    pub bindings: Vec<Binding<N>>,
    /// Event handlers, in document order.
    pub events: Vec<EventBinding<N>>,
    /// Diagnostics, in first-seen document order.
    pub diagnostics: Vec<ReactiveDiagnostic>,
}

impl<N> Default for ReactiveDocument<N> {
    fn default() -> Self {
        Self {
            scopes: Vec::new(),
            bindings: Vec::new(),
            events: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

impl<N> ReactiveDocument<N> {
    /// True when nothing reactive was found and nothing was diagnosed:
    /// the document takes the exact non-reactive path.
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
            && self.bindings.is_empty()
            && self.events.is_empty()
            && self.diagnostics.is_empty()
    }
}

/// The deferred-but-known directives: valid v0 names whose runtime
/// semantics (dynamic tree insertion/removal, derived state) are not in
/// M5b/M5c's static plan. They compile to a diagnostic, never silently.
fn deferred_directive(name: &str) -> bool {
    matches!(name, "vx-if" | "vx-for" | "vx-key" | "vx-computed")
}

/// A cheap, deterministic expression shape check (M5b does not run
/// JavaScript): nonempty after trimming, no NUL bytes, and balanced
/// `()`/`[]`/`{}` and quotes. QuickJS remains the semantic gate at
/// evaluation time (M5c) — this catches the malformed-by-construction
/// cases at compile time, tied to node and attribute.
fn check_expression_shape(source: &str) -> Result<(), &'static str> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err("expression is empty");
    }
    if source.contains('\0') {
        return Err("expression contains a NUL byte");
    }
    let mut depth_paren = 0i32;
    let mut depth_bracket = 0i32;
    let mut depth_brace = 0i32;
    let mut quote: Option<char> = None;
    for character in trimmed.chars() {
        if let Some(q) = quote {
            if character == q {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' | '`' => quote = Some(character),
            '(' => depth_paren += 1,
            ')' => {
                depth_paren -= 1;
                if depth_paren < 0 {
                    return Err("expression has an unmatched ')'");
                }
            }
            '[' => depth_bracket += 1,
            ']' => {
                depth_bracket -= 1;
                if depth_bracket < 0 {
                    return Err("expression has an unmatched ']'");
                }
            }
            '{' => depth_brace += 1,
            '}' => {
                depth_brace -= 1;
                if depth_brace < 0 {
                    return Err("expression has an unmatched '}'");
                }
            }
            _ => {}
        }
    }
    if quote.is_some() {
        return Err("expression has an unterminated string literal");
    }
    if depth_paren != 0 {
        return Err("expression has unbalanced parentheses");
    }
    if depth_bracket != 0 {
        return Err("expression has unbalanced brackets");
    }
    if depth_brace != 0 {
        return Err("expression has unbalanced braces");
    }
    Ok(())
}

/// Is `tag` a control `vx-model` may target (the M4c1 profile)?
fn model_target(tag: &str) -> bool {
    matches!(tag, "input" | "textarea")
}

/// Is `tag` a form-associated element for `@input`/`@change`/`@submit`?
fn form_element(tag: &str) -> bool {
    matches!(tag, "input" | "textarea" | "select" | "form" | "button")
}

/// Compiles a reactive document: walks `dom` in preorder, lowering the
/// frozen surface into the plan and collecting diagnostics. Pure and
/// deterministic — identical DOMs compile to identical plans.
pub fn compile<N: Copy + Eq>(dom: &impl ReactiveDom<Node = N>) -> ReactiveDocument<N> {
    let mut plan = ReactiveDocument::default();
    let mut scope_stack: Vec<usize> = Vec::new(); // enclosing scope indices
    let mut ordinal = 0usize; // true preorder counter
    walk(dom, dom.root(), &mut scope_stack, &mut ordinal, &mut plan);
    plan
}

fn walk<N: Copy + Eq>(
    dom: &impl ReactiveDom<Node = N>,
    node: N,
    scope_stack: &mut Vec<usize>,
    ordinal: &mut usize,
    plan: &mut ReactiveDocument<N>,
) {
    let this_ordinal = *ordinal;
    *ordinal += 1;
    let tag = dom.tag(node).to_owned();
    let attributes = dom.attributes(node);
    let span_for = |attribute: &str| SourceSpan {
        node_ordinal: this_ordinal,
        attribute: attribute.to_owned(),
    };
    let current_scope = scope_stack.last().copied();
    let scopes_before = plan.scopes.len();
    let mut seen: Vec<&str> = Vec::new();
    let mut has_model = false;
    let mut value_binding_attribute: Option<String> = None;

    // Pass 1: lower this node's attributes; duplicates are diagnosed
    // deterministically (first occurrence wins).
    for (name, value) in &attributes {
        let is_reactive = name.starts_with("vx-") || name.starts_with('@') || name.starts_with(':');
        if !is_reactive {
            continue;
        }
        let span = span_for(name);
        if seen.contains(&name.as_str()) {
            plan.diagnostics.push(ReactiveDiagnostic {
                span,
                message: "duplicate reactive attribute: the first occurrence wins".to_owned(),
            });
            continue;
        }
        seen.push(name);

        // Known names only: unknown vx-*/@*/:* diagnose, never pass as
        // ordinary markup.
        if name.starts_with("vx-") {
            let directive = match parse_directive(name) {
                Ok(directive) => directive,
                Err(error) => {
                    plan.diagnostics.push(ReactiveDiagnostic {
                        span,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            if deferred_directive(name) {
                plan.diagnostics.push(ReactiveDiagnostic {
                    span,
                    message: format!(
                        "{name} is a known v0 directive whose dynamic-tree semantics are deferred (not in M5b)"
                    ),
                });
                continue;
            }
            lower_directive(node, &tag, directive, value, &span, current_scope, plan);
            if directive == Directive::Model {
                has_model = true;
            }
            continue;
        }
        if let Some(event_name) = name.strip_prefix('@') {
            let handler = match parse_event_handler(event_name) {
                Ok(handler) => handler,
                Err(error) => {
                    plan.diagnostics.push(ReactiveDiagnostic {
                        span,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            if matches!(handler.event.as_str(), "input" | "change" | "submit")
                && !form_element(&tag)
            {
                plan.diagnostics.push(ReactiveDiagnostic {
                    span,
                    message: format!(
                        "@{} is a form event; <{}> is not a form element",
                        handler.event, tag
                    ),
                });
                continue;
            }
            if let Err(reason) = check_expression_shape(value) {
                plan.diagnostics.push(ReactiveDiagnostic {
                    span,
                    message: format!("handler {reason}"),
                });
                continue;
            }
            match current_scope {
                Some(scope) => plan.events.push(EventBinding {
                    node,
                    scope,
                    handler,
                    handler_source: value.clone(),
                    span,
                }),
                None => plan.diagnostics.push(ReactiveDiagnostic {
                    span,
                    message: "event handler has no enclosing vx-state scope".to_owned(),
                }),
            }
            continue;
        }
        if let Some(binding_name) = name.strip_prefix(':') {
            let binding = match parse_attribute_binding(binding_name) {
                Ok(binding) => binding,
                Err(error) => {
                    plan.diagnostics.push(ReactiveDiagnostic {
                        span,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let kind = match binding {
                "class" => BindingKind::Class,
                "style" => BindingKind::Style,
                "value" => BindingKind::Value,
                "disabled" => BindingKind::Disabled,
                "checked" => BindingKind::Checked,
                _ => unreachable!("parse_attribute_binding returns frozen names"),
            };
            if matches!(kind, BindingKind::Value | BindingKind::Checked)
                && !model_target(&tag)
                && tag != "select"
                && tag != "option"
            {
                plan.diagnostics.push(ReactiveDiagnostic {
                    span,
                    message: format!(
                        ":{binding} targets a control property; <{tag}> is not a control"
                    ),
                });
                continue;
            }
            if kind == BindingKind::Value {
                value_binding_attribute = Some(name.clone());
            }
            if let Err(reason) = check_expression_shape(value) {
                plan.diagnostics.push(ReactiveDiagnostic {
                    span,
                    message: format!("expression {reason}"),
                });
                continue;
            }
            match current_scope {
                Some(scope) => plan.bindings.push(Binding {
                    node,
                    scope,
                    kind,
                    expression_source: value.clone(),
                    span,
                }),
                None => plan.diagnostics.push(ReactiveDiagnostic {
                    span,
                    message: "binding has no enclosing vx-state scope".to_owned(),
                }),
            }
        }
    }

    // Deterministic conflict rule: vx-model owns the control's value;
    // a :value binding on the same node is diagnosed and dropped.
    if has_model {
        if let Some(attribute) = value_binding_attribute {
            plan.bindings
                .retain(|binding| binding.node != node || binding.kind != BindingKind::Value);
            plan.diagnostics.push(ReactiveDiagnostic {
                span: span_for(&attribute),
                message:
                    "conflicts with vx-model on the same element: vx-model wins, the :value binding is dropped"
                        .to_owned(),
            });
        }
    }

    // Pass 2: descend, with this node's scope (if it declared one, it is
    // the only scope pushed while processing this node) enclosing.
    let declared_scope = (plan.scopes.len() > scopes_before).then_some(scopes_before);
    if let Some(index) = declared_scope {
        scope_stack.push(index);
    }
    for child in dom.children(node) {
        walk(dom, child, scope_stack, ordinal, plan);
    }
    if declared_scope.is_some() {
        scope_stack.pop();
    }
}

fn lower_directive<N>(
    node: N,
    tag: &str,
    directive: Directive,
    value: &str,
    span: &SourceSpan,
    current_scope: Option<usize>,
    plan: &mut ReactiveDocument<N>,
) {
    match directive {
        Directive::State => {
            if let Err(reason) = check_expression_shape(value) {
                plan.diagnostics.push(ReactiveDiagnostic {
                    span: span.clone(),
                    message: format!("scope initializer {reason}"),
                });
                return;
            }
            plan.scopes.push(ScopePlan {
                node,
                parent: current_scope,
                initializer_source: value.to_owned(),
                span: span.clone(),
            });
        }
        Directive::Text | Directive::Show | Directive::Model => {
            let kind = match directive {
                Directive::Text => BindingKind::Text,
                Directive::Show => BindingKind::Show,
                _ => BindingKind::Model,
            };
            let target_error = match kind {
                BindingKind::Model if !model_target(tag) => Some(format!(
                    "vx-model targets <input>/<textarea>; <{tag}> is unsupported"
                )),
                BindingKind::Text if model_target(tag) || tag == "select" => Some(format!(
                    "vx-text replaces text content; <{tag}> renders its value instead"
                )),
                _ => None,
            };
            if let Some(message) = target_error {
                plan.diagnostics.push(ReactiveDiagnostic {
                    span: span.clone(),
                    message,
                });
                return;
            }
            if let Err(reason) = check_expression_shape(value) {
                plan.diagnostics.push(ReactiveDiagnostic {
                    span: span.clone(),
                    message: format!("expression {reason}"),
                });
                return;
            }
            match current_scope {
                Some(scope) => plan.bindings.push(Binding {
                    node,
                    scope,
                    kind,
                    expression_source: value.to_owned(),
                    span: span.clone(),
                }),
                None => plan.diagnostics.push(ReactiveDiagnostic {
                    span: span.clone(),
                    message: "binding has no enclosing vx-state scope".to_owned(),
                }),
            }
        }
        // Deferred directives were diagnosed before lowering.
        Directive::Computed | Directive::If | Directive::For | Directive::Key => {}
    }
}

#[cfg(test)]
mod tests;
