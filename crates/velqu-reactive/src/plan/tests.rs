//! M5b compiler battery (ADR 0016). A tiny synthetic DOM exercises the
//! pure pass; velqu-view's integration tests pin the host side.

use super::*;

/// A minimal test DOM: nodes are usize slots, built by id.
#[derive(Default)]
struct TestDom {
    nodes: Vec<TestNode>,
}

#[derive(Default)]
struct TestNode {
    tag: &'static str,
    attrs: Vec<(String, String)>,
    children: Vec<usize>,
}

impl TestDom {
    fn new(root_tag: &'static str) -> Self {
        Self {
            nodes: vec![TestNode {
                tag: root_tag,
                ..TestNode::default()
            }],
        }
    }

    fn add(&mut self, parent: usize, tag: &'static str) -> usize {
        let id = self.nodes.len();
        self.nodes.push(TestNode {
            tag,
            ..TestNode::default()
        });
        self.nodes[parent].children.push(id);
        id
    }

    fn attr(&mut self, node: usize, name: &str, value: &str) {
        self.nodes[node]
            .attrs
            .push((name.to_owned(), value.to_owned()));
    }
}

impl ReactiveDom for TestDom {
    type Node = usize;

    fn root(&self) -> usize {
        0
    }

    fn children(&self, node: usize) -> Vec<usize> {
        self.nodes[node].children.clone()
    }

    fn tag(&self, node: usize) -> &str {
        self.nodes[node].tag
    }

    fn attributes(&self, node: usize) -> Vec<(String, String)> {
        self.nodes[node].attrs.clone()
    }
}

#[test]
fn plain_document_compiles_to_an_empty_plan() {
    let mut dom = TestDom::new("div");
    let p = dom.add(0, "p");
    dom.attr(p, "class", "x");
    let plan = compile(&dom);
    assert!(plan.is_empty(), "{plan:?}");
}

#[test]
fn unknown_directive_is_a_diagnostic_never_silent() {
    let mut dom = TestDom::new("div");
    let span = dom.add(0, "span");
    dom.attr(span, "vx-magic", "1");
    let plan = compile(&dom);
    assert!(plan.bindings.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert!(
        plan.diagnostics[0].message.contains("unknown directive"),
        "{plan:?}"
    );
    assert_eq!(plan.diagnostics[0].span.attribute, "vx-magic");
}

#[test]
fn nested_scopes_carry_parentage_and_bindings_resolve_nearest() {
    let mut dom = TestDom::new("div");
    let outer = dom.add(0, "div");
    dom.attr(outer, "vx-state", "{ a: 1 }");
    let inner = dom.add(outer, "section");
    dom.attr(inner, "vx-state", "{ b: 2 }");
    let text = dom.add(inner, "p");
    dom.attr(text, "vx-text", "a + b");
    let plan = compile(&dom);

    assert_eq!(plan.scopes.len(), 2);
    assert_eq!(plan.scopes[0].parent, None, "outer is a root scope");
    assert_eq!(plan.scopes[1].parent, Some(0), "inner's parent is outer");
    assert_eq!(plan.scopes[0].initializer_source, "{ a: 1 }");
    assert_eq!(plan.scopes[1].initializer_source, "{ b: 2 }");

    // The vx-text under both scopes resolves to the inner (nearest).
    let text = plan
        .bindings
        .iter()
        .find(|b| b.kind == BindingKind::Text)
        .expect("text binding lowered");
    assert_eq!(text.scope, 1);
    assert_eq!(text.expression_source, "a + b");
}

#[test]
fn binding_without_a_scope_is_diagnosed() {
    let mut dom = TestDom::new("div");
    let p = dom.add(0, "p");
    dom.attr(p, "vx-text", "orphan");
    let plan = compile(&dom);
    assert!(plan.bindings.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert!(
        plan.diagnostics[0]
            .message
            .contains("no enclosing vx-state")
    );
}

#[test]
fn malformed_expressions_diagnose_at_the_node_and_attribute() {
    let mut dom = TestDom::new("div");
    let scope = dom.add(0, "div");
    dom.attr(scope, "vx-state", "{ unmatched: (");
    let p = dom.add(0, "p");
    dom.attr(p, "vx-text", "");
    dom.attr(p, "vx-show", "name === 'unterminated");
    let input = dom.add(0, "input");
    dom.attr(input, "@click", "count++ }");
    let plan = compile(&dom);
    assert!(
        plan.bindings.is_empty() && plan.events.is_empty() && plan.scopes.is_empty(),
        "{plan:?}"
    );
    assert_eq!(plan.diagnostics.len(), 4, "{:#?}", plan.diagnostics);
    for diagnostic in &plan.diagnostics {
        assert!(
            diagnostic.message.contains("expression")
                || diagnostic.message.contains("handler")
                || diagnostic.message.contains("initializer"),
            "{}",
            diagnostic.message
        );
    }
}

#[test]
fn duplicate_directives_fail_deterministically_first_wins() {
    let mut dom = TestDom::new("div");
    let scope = dom.add(0, "div");
    dom.attr(scope, "vx-state", "{ n: 1 }");
    let p = dom.add(scope, "p");
    dom.attr(p, "vx-text", "first");
    dom.attr(p, "vx-text", "second");
    let plan = compile(&dom);
    assert_eq!(plan.bindings.len(), 1);
    assert_eq!(plan.bindings[0].expression_source, "first");
    assert_eq!(plan.diagnostics.len(), 1);
    assert!(plan.diagnostics[0].message.contains("duplicate"));
}

#[test]
fn model_and_value_conflict_resolves_deterministically() {
    let mut dom = TestDom::new("div");
    let scope = dom.add(0, "div");
    dom.attr(scope, "vx-state", "{ name: '' }");
    let input = dom.add(scope, "input");
    dom.attr(input, "vx-model", "name");
    dom.attr(input, ":value", "'literal'");
    let plan = compile(&dom);
    // vx-model wins; the :value binding is gone.
    assert_eq!(
        plan.bindings
            .iter()
            .filter(|b| b.kind == BindingKind::Value)
            .count(),
        0
    );
    assert_eq!(
        plan.bindings
            .iter()
            .filter(|b| b.kind == BindingKind::Model)
            .count(),
        1
    );
    assert!(
        plan.diagnostics
            .iter()
            .any(|d| d.message.contains("vx-model wins"))
    );
}

#[test]
fn model_on_unsupported_element_diagnoses_at_compile_time() {
    let mut dom = TestDom::new("div");
    let scope = dom.add(0, "div");
    dom.attr(scope, "vx-state", "{ x: 1 }");
    let div = dom.add(0, "div");
    dom.attr(div, "vx-model", "x");
    let plan = compile(&dom);
    assert!(plan.bindings.is_empty());
    assert!(
        plan.diagnostics
            .iter()
            .any(|d| d.span.attribute == "vx-model" && d.message.contains("<div> is unsupported"))
    );
}

#[test]
fn form_events_on_non_form_elements_diagnose() {
    let mut dom = TestDom::new("div");
    let scope = dom.add(0, "div");
    dom.attr(scope, "vx-state", "{ v: '' }");
    let h1 = dom.add(scope, "h1");
    dom.attr(h1, "@input", "v = $event");
    let button = dom.add(scope, "button");
    dom.attr(button, "@click", "v = 'ok'");
    let plan = compile(&dom);
    assert_eq!(plan.events.len(), 1, "@click on a button is legal");
    assert_eq!(plan.events[0].handler.event, "click");
    assert!(
        plan.diagnostics
            .iter()
            .any(|d| d.span.attribute == "@input" && d.message.contains("form event"))
    );
}

#[test]
fn deferred_directives_diagnose_as_known_but_deferred() {
    let mut dom = TestDom::new("div");
    let ul = dom.add(0, "ul");
    dom.attr(ul, "vx-for", "item in items");
    let p = dom.add(0, "p");
    dom.attr(p, "vx-if", "open");
    dom.attr(p, "vx-computed", "x");
    let plan = compile(&dom);
    assert!(plan.bindings.is_empty());
    assert_eq!(plan.diagnostics.len(), 3);
    assert!(
        plan.diagnostics
            .iter()
            .all(|d| d.message.contains("deferred"))
    );
}

#[test]
fn value_binding_on_non_control_diagnoses() {
    let mut dom = TestDom::new("div");
    let scope = dom.add(0, "div");
    dom.attr(scope, "vx-state", "{ v: '' }");
    let span = dom.add(0, "span");
    dom.attr(span, ":value", "v");
    let plan = compile(&dom);
    assert!(plan.bindings.is_empty());
    assert!(plan.diagnostics[0].message.contains("not a control"));
}

#[test]
fn compilation_is_deterministic() {
    let build_dom = || {
        let mut dom = TestDom::new("div");
        let scope = dom.add(0, "div");
        dom.attr(scope, "vx-state", "{ n: 0, open: false }");
        let p = dom.add(scope, "p");
        dom.attr(p, "vx-text", "'count: ' + n");
        dom.attr(p, "vx-show", "open");
        let button = dom.add(scope, "button");
        dom.attr(button, "@click.prevent", "n = n + 1");
        dom.attr(button, ":disabled", "n > 9");
        let input = dom.add(scope, "input");
        dom.attr(input, "vx-model", "open");
        dom.attr(input, "@input", "noop");
        dom
    };
    let a = compile(&build_dom());
    let b = compile(&build_dom());
    assert_eq!(a, b);
    assert_eq!(a.scopes.len(), 1);
    assert_eq!(a.bindings.len(), 4); // text, show, disabled, model
    assert_eq!(a.events.len(), 2); // click.prevent, input
    assert!(a.diagnostics.is_empty(), "{:#?}", a.diagnostics);
}
