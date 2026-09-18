//! M5c machine battery (ADR 0017): atomic turns, model ordering,
//! rollback, `.once`/`.stop`, batch semantics — at the machine level,
//! with a synthetic plan. The view-level battery pins wiring.

use super::*;
use crate::plan::{BindingKind, ReactiveDom, compile};
use std::time::Duration;

fn limits() -> JsLimits {
    JsLimits {
        max_heap_bytes: 4 * 1024 * 1024,
        max_stack_bytes: 256 * 1024,
        max_execution_time: Duration::from_millis(100),
        max_source_bytes: 64 * 1024,
        max_event_payload_bytes: 16 * 1024,
        max_mutations_per_turn: 64,
        max_output_string_bytes: 8 * 1024,
        max_pending_jobs: 16,
    }
}

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
    fn new() -> Self {
        Self {
            nodes: vec![TestNode {
                tag: "body",
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

/// The canonical counter document: a scope, a text binding, and a click
/// handler (optionally customized).
fn counter_dom(handler: &str) -> TestDom {
    let mut dom = TestDom::new();
    let scope = dom.add(0, "div");
    dom.attr(scope, "vx-state", "{ count: 0 }");
    let p = dom.add(scope, "p");
    dom.attr(p, "vx-text", "'Count: ' + count");
    let button = dom.add(scope, "button");
    dom.attr(button, "@click", handler);
    dom
}

fn click_payload() -> EventPayload {
    EventPayload::new(
        vec![
            ("type".to_owned(), PayloadValue::Str("click".to_owned())),
            ("button".to_owned(), PayloadValue::Number(0.0)),
        ],
        1024,
    )
    .expect("budgeted")
}

fn click_handlers<N>(plan: &crate::plan::ReactiveDocument<N>) -> Vec<usize> {
    plan.events.iter().map(|_| 0).collect()
}

#[test]
fn initial_state_evaluates_the_bindings_once() {
    let dom = counter_dom("count = count + 1");
    let plan = compile(&dom);
    let (machine, _initial) = ReactiveMachine::new(1, limits(), &plan).expect("machine");
    assert_eq!(
        machine.state().get_path("count"),
        ReactiveValue::Number(0.0)
    );
}

#[test]
fn a_click_commits_state_and_emits_the_diff() {
    let dom = counter_dom("count = count + 1");
    let plan = compile(&dom);
    let handlers = click_handlers(&plan);
    let (mut machine, _initial) = ReactiveMachine::new(1, limits(), &plan).expect("machine");

    let payload = click_payload();
    let pending = match machine.prepare(Some(&payload), &handlers, None) {
        TurnOutcome::Prepared(pending) => pending,
        other => panic!("{other:?}"),
    };
    // Not committed until commit(): state is still the old one.
    assert_eq!(
        machine.state().get_path("count"),
        ReactiveValue::Number(0.0)
    );
    assert_eq!(
        pending
            .mutations
            .iter()
            .filter(|m| matches!(m.kind, MutationKind::SetText(ref t) if t == "Count: 1"))
            .count(),
        1
    );
    machine.commit(pending);
    assert_eq!(
        machine.state().get_path("count"),
        ReactiveValue::Number(1.0)
    );

    // The second click is a real change: exactly one diffed mutation.
    let payload = click_payload();
    let pending = match machine.prepare(Some(&payload), &handlers, None) {
        TurnOutcome::Prepared(pending) => pending,
        other => panic!("{other:?}"),
    };
    assert_eq!(pending.mutations.len(), 1);
    machine.commit(pending);

    // A model write that changes nothing the bindings can see emits no
    // mutations: the diff against the applied baseline is empty.
    let pending = match machine.prepare(None, &[], Some(("count", "2"))) {
        TurnOutcome::Prepared(pending) => pending,
        other => panic!("{other:?}"),
    };
    assert!(
        pending.mutations.is_empty(),
        "no-ops produce no mutations: {:?}",
        pending.mutations
    );
}

#[test]
fn a_throwing_handler_rolls_the_whole_turn_back() {
    let dom = counter_dom("count = count + 1; throw new Error('boom')");
    let plan = compile(&dom);
    let handlers = click_handlers(&plan);
    let (mut machine, _initial) = ReactiveMachine::new(1, limits(), &plan).expect("machine");
    let payload = click_payload();
    match machine.prepare(Some(&payload), &handlers, None) {
        TurnOutcome::RolledBack(message) => assert!(message.contains("boom"), "{message}"),
        other => panic!("expected rollback, got {other:?}"),
    }
    assert_eq!(
        machine.state().get_path("count"),
        ReactiveValue::Number(0.0),
        "state rolled back"
    );
    assert!(!machine.diagnostics().is_empty());
}

#[test]
fn a_throwing_binding_rolls_state_back_too() {
    // The handler succeeds; the binding throws on the new state.
    let dom = counter_dom("oops = {}");
    let mut dom = dom;
    // Rewrite the text binding to read a missing property of a number.
    let scope = dom.nodes[0].children[0];
    let p = dom.nodes[scope].children[0];
    dom.nodes[p].attrs.clear();
    dom.attr(p, "vx-text", "count + missing.deeper.value");
    let plan = compile(&dom);
    let handlers = click_handlers(&plan);
    let (mut machine, _initial) = ReactiveMachine::new(1, limits(), &plan).expect("machine");
    let payload = click_payload();
    match machine.prepare(Some(&payload), &handlers, None) {
        TurnOutcome::RolledBack(message) => {
            assert!(message.contains("binding 0 threw"), "{message}")
        }
        other => panic!("expected rollback, got {other:?}"),
    }
    assert_eq!(
        machine.state().get_path("oops"),
        ReactiveValue::Null,
        "the candidate (including the handler's write) was discarded"
    );
}

#[test]
fn a_job_bomb_rolls_the_turn_back() {
    let dom = counter_dom("count = 1; function chain() { Promise.resolve().then(chain); } chain()");
    // (the count write happens before the flood; rollback discards it)
    let plan = compile(&dom);
    let handlers = click_handlers(&plan);
    let (mut machine, _initial) = ReactiveMachine::new(1, limits(), &plan).expect("machine");
    let payload = click_payload();
    match machine.prepare(Some(&payload), &handlers, None) {
        TurnOutcome::RolledBack(message) => {
            assert!(message.contains("jobs"), "{message}")
        }
        other => panic!("expected rollback, got {other:?}"),
    }
    assert_eq!(
        machine.state().get_path("count"),
        ReactiveValue::Number(0.0),
        "the handler's write was rolled back with the turn"
    );
}

#[test]
fn model_write_lands_before_the_input_handler() {
    let mut dom = TestDom::new();
    let scope = dom.add(0, "div");
    dom.attr(scope, "vx-state", "{ name: '', seen: '' }");
    let input = dom.add(scope, "input");
    dom.attr(input, "vx-model", "name");
    dom.attr(input, "@input", "seen = name");
    let span = dom.add(scope, "span");
    dom.attr(span, "vx-text", "seen");
    let plan = compile(&dom);
    let input_handler = plan
        .events
        .iter()
        .position(|event| event.handler.event == "input")
        .expect("input handler");
    let span_binding = plan
        .bindings
        .iter()
        .position(|b| b.kind == BindingKind::Text)
        .expect("text binding");
    let (mut machine, _initial) = ReactiveMachine::new(1, limits(), &plan).expect("machine");

    let payload = EventPayload::new(
        vec![
            ("type".to_owned(), PayloadValue::Str("input".to_owned())),
            ("value".to_owned(), PayloadValue::Str("Alice".to_owned())),
        ],
        1024,
    )
    .expect("budgeted");
    let pending = match machine.prepare(Some(&payload), &[input_handler], Some(("name", "Alice"))) {
        TurnOutcome::Prepared(pending) => pending,
        other => panic!("{other:?}"),
    };
    machine.commit(pending);
    // The handler observed the model value, and the sibling binding
    // received it in the same commit.
    assert_eq!(
        machine.state().get_path("seen"),
        ReactiveValue::String("Alice".into())
    );
    assert_eq!(
        machine.state().get_path("name"),
        ReactiveValue::String("Alice".into())
    );
    let _ = span_binding;
}

#[test]
fn five_state_changes_commit_as_one_batch() {
    let mut dom = TestDom::new();
    let scope = dom.add(0, "div");
    dom.attr(scope, "vx-state", "{ a: 0, b: 0, c: 0, d: 0, e: 0 }");
    for key in ["a", "b", "c", "d", "e"] {
        let p = dom.add(scope, "p");
        dom.attr(p, "vx-text", key);
    }
    let button = dom.add(scope, "button");
    dom.attr(button, "@click", "a = 1; b = 2; c = 3; d = 4; e = 5");
    let plan = compile(&dom);
    let handlers = click_handlers(&plan);
    let (mut machine, _initial) = ReactiveMachine::new(1, limits(), &plan).expect("machine");
    let payload = click_payload();
    let pending = match machine.prepare(Some(&payload), &handlers, None) {
        TurnOutcome::Prepared(pending) => pending,
        other => panic!("{other:?}"),
    };
    assert_eq!(pending.mutations.len(), 5, "one batch, five SetText");
    machine.commit(pending);
    for (key, value) in [("a", 1.0), ("e", 5.0)] {
        assert_eq!(machine.state().get_path(key), ReactiveValue::Number(value));
    }
}

#[test]
fn once_handlers_fire_exactly_once_and_only_on_commit() {
    let dom = counter_dom("count = count + 1");
    let mut dom = dom;
    let scope = dom.nodes[0].children[0];
    let button = dom.nodes[scope].children[1];
    dom.nodes[button].attrs.clear();
    dom.attr(button, "@click.once", "count = count + 1");
    let plan = compile(&dom);
    let handlers = click_handlers(&plan);
    let (mut machine, _initial) = ReactiveMachine::new(1, limits(), &plan).expect("machine");

    let payload = click_payload();
    let pending = match machine.prepare(Some(&payload), &handlers, None) {
        TurnOutcome::Prepared(pending) => pending,
        other => panic!("{other:?}"),
    };
    machine.commit(pending);
    assert_eq!(
        machine.state().get_path("count"),
        ReactiveValue::Number(1.0)
    );

    // The once is consumed: the handler does not fire again (a turn
    // where every handler is skipped is NoChange).
    let payload = click_payload();
    assert!(matches!(
        machine.prepare(Some(&payload), &handlers, None),
        TurnOutcome::NoChange
    ));
    assert_eq!(
        machine.state().get_path("count"),
        ReactiveValue::Number(1.0)
    );
}

#[test]
fn stop_ends_the_ancestor_walk() {
    let mut dom = TestDom::new();
    let scope = dom.add(0, "div");
    dom.attr(scope, "vx-state", "{ order: '' }");
    let mid = dom.add(scope, "section");
    dom.attr(mid, "@click", "order = order + 'm'");
    let leaf = dom.add(mid, "button");
    dom.attr(leaf, "@click.stop", "order = order + 'l'");
    let plan = compile(&dom);
    // Handler order: target (leaf) first, then ancestors.
    let leaf_handler = plan
        .events
        .iter()
        .position(|e| e.handler.modifiers.iter().any(|m| m == "stop"))
        .expect("leaf handler");
    let mid_handler = plan
        .events
        .iter()
        .position(|e| !e.handler.modifiers.iter().any(|m| m == "stop"))
        .expect("mid handler");
    let (mut machine, _initial) = ReactiveMachine::new(1, limits(), &plan).expect("machine");
    let payload = click_payload();
    let pending = match machine.prepare(Some(&payload), &[leaf_handler, mid_handler], None) {
        TurnOutcome::Prepared(pending) => pending,
        other => panic!("{other:?}"),
    };
    machine.commit(pending);
    assert_eq!(
        machine.state().get_path("order"),
        ReactiveValue::String("l".into()),
        "the ancestor handler did not run after .stop"
    );
}

#[test]
fn event_payloads_are_frozen() {
    let dom = counter_dom("__velquEvent.value = 'hacked'; count = 1");
    let plan = compile(&dom);
    let handlers = click_handlers(&plan);
    let (mut machine, _initial) = ReactiveMachine::new(1, limits(), &plan).expect("machine");
    let payload = EventPayload::new(
        vec![("value".to_owned(), PayloadValue::Str("real".to_owned()))],
        1024,
    )
    .expect("budgeted");
    // Sloppy mode + Object.freeze: the write is silently contained —
    // the payload is Rust-side data and never reflected back to M4 —
    // and the rest of the turn still commits.
    let pending = match machine.prepare(Some(&payload), &handlers, None) {
        TurnOutcome::Prepared(pending) => pending,
        other => panic!("contained write must not fail the turn: {other:?}"),
    };
    machine.commit(pending);
    assert_eq!(
        machine.state().get_path("count"),
        ReactiveValue::Number(1.0)
    );
    assert_eq!(
        payload.entries[0].1,
        PayloadValue::Str("real".to_owned()),
        "the Rust payload is untouched by definition"
    );
}

#[test]
fn oversized_payloads_are_refused_before_js() {
    assert!(
        EventPayload::new(
            vec![("value".to_owned(), PayloadValue::Str("x".repeat(2048)))],
            1024,
        )
        .is_err()
    );
}

#[test]
fn state_capture_rejects_functions_and_exotic_values() {
    let dom = counter_dom("count = () => 1");
    let plan = compile(&dom);
    let handlers = click_handlers(&plan);
    let (mut machine, _initial) = ReactiveMachine::new(1, limits(), &plan).expect("machine");
    let payload = click_payload();
    match machine.prepare(Some(&payload), &handlers, None) {
        TurnOutcome::RolledBack(message) => {
            assert!(message.contains("function"), "{message}")
        }
        other => panic!("expected rollback, got {other:?}"),
    }
}
