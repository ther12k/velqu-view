//! M5 exit conformance: the three reactive examples (counter, forms,
//! tabs) driven end-to-end through the public API — load with Tailwind
//! and reactive enabled, render, then real pointer/keyboard events
//! pumped as atomic reactive turns (ADR 0016/0017), asserting state,
//! layout facts, and raster digests.
//!
//! These are the milestone's exit fixtures ("counter/forms/tabs
//! examples pass reactive conformance"): the exact files a human runs
//! with `velqu-lab --tailwind --reactive examples/<name>`.

use std::path::PathBuf;

use velqu_reactive::ReactiveValue;
use velqu_view::{LayoutFacts, VelquView, Viewport};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Loads one example's `index.html` with both pipelines enabled — the
/// same configuration `velqu-lab --tailwind --reactive` runs.
fn load_example(app: &str) -> VelquView {
    let html = std::fs::read_to_string(
        workspace_root()
            .join("examples")
            .join(app)
            .join("index.html"),
    )
    .unwrap_or_else(|error| panic!("cannot read examples/{app}/index.html: {error}"));
    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();
    view.load_html(&html).expect("example loads");
    view
}

/// Renders once, settles turn zero, renders again — the steady state
/// every scenario starts from.
fn settle(view: &mut VelquView, vp: Viewport) {
    view.render(vp).unwrap();
    view.pump_reactive(&[]);
    view.render(vp).unwrap();
    let _ = view.take_events();
}

/// Clicks a known element by scanning for its hit target (buttons have
/// no intrinsic position knowledge pre-render).
fn click_element(view: &mut VelquView, vp: Viewport, id: &str) {
    let mut found = None;
    for y in (0..vp.height()).step_by(4) {
        for x in (0..vp.width()).step_by(8) {
            if view
                .hit_test(vp, x as f32 + 0.5, y as f32 + 0.5)
                .is_some_and(|target| target.element_id.as_deref() == Some(id))
            {
                found = Some((x as f32 + 0.5, y as f32 + 0.5));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let (x, y) = found.unwrap_or_else(|| panic!("no hit target for {id}"));
    view.pointer_press(vp, x, y);
    view.pointer_release(vp, x, y);
    let events = view.take_events();
    view.pump_reactive(&events);
    view.render(vp).unwrap();
}

/// Types text into a control one character per turn — each keystroke is
/// one ValueChanged event, exactly like a real keyboard.
fn type_into(view: &mut VelquView, vp: Viewport, id: &str, text: &str) {
    view.set_focus(Some(id));
    for character in text.chars() {
        view.insert_text(&character.to_string());
        let events = view.take_events();
        view.pump_reactive(&events);
        view.render(vp).unwrap();
    }
}

fn text_of(facts: &LayoutFacts, fixture: &str) -> Vec<String> {
    facts
        .nodes
        .iter()
        .find(|fact| fact.fixture_id == fixture)
        .unwrap_or_else(|| panic!("no fact {fixture}"))
        .text_runs
        .clone()
}

fn state_of(view: &VelquView, path: &str) -> ReactiveValue {
    view.reactive_state()
        .unwrap_or_else(|| panic!("no reactive state"))
        .get_path(path)
}

#[test]
fn counter_example_conforms() {
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    let mut view = load_example("counter");
    settle(&mut view, vp);

    // Turn zero painted the initial state.
    assert_eq!(text_of(&view.layout_facts(vp).unwrap(), "value"), ["0"]);
    assert_eq!(state_of(&view, "count"), ReactiveValue::Number(0.0));

    // +, +, − through real pointer events: three atomic turns.
    click_element(&mut view, vp, "inc");
    click_element(&mut view, vp, "inc");
    click_element(&mut view, vp, "dec");
    assert_eq!(text_of(&view.layout_facts(vp).unwrap(), "value"), ["1"]);
    assert_eq!(state_of(&view, "count"), ReactiveValue::Number(1.0));

    // The OKF transcription uses two utilities outside profile v0; both
    // are diagnosed deterministically (and nothing else is).
    let diagnostics = view.tailwind_diagnostics();
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert!(
        view.reactive_diagnostics().is_empty(),
        "{:?}",
        view.reactive_diagnostics()
    );

    // A fresh identical session is pixel-identical (same drive, same
    // raster): conformance doubles as a determinism fixture.
    let digest_a = view.render(vp).unwrap().frame.sha256_hex();
    let mut view_b = load_example("counter");
    settle(&mut view_b, vp);
    click_element(&mut view_b, vp, "inc");
    click_element(&mut view_b, vp, "inc");
    click_element(&mut view_b, vp, "dec");
    let digest_b = view_b.render(vp).unwrap().frame.sha256_hex();
    assert_eq!(digest_a, digest_b);
}

#[test]
fn forms_example_conforms() {
    let vp = Viewport::try_new(400, 600, 1.0).unwrap();
    let mut view = load_example("forms");
    settle(&mut view, vp);

    // Initial derivations from empty state.
    let facts = view.layout_facts(vp).unwrap();
    assert_eq!(text_of(&facts, "greeting"), ["Hello, stranger"]);
    assert_eq!(text_of(&facts, "count"), ["0 character(s) typed"]);
    assert!(
        view.tailwind_diagnostics().is_empty(),
        "{:?}",
        view.tailwind_diagnostics()
    );

    // The submit button carries :disabled="!name || !email": while
    // either field is empty a click cannot activate it.
    click_element(&mut view, vp, "submit");
    assert_eq!(state_of(&view, "submitted"), ReactiveValue::Bool(false));
    assert!(
        !view
            .layout_facts(vp)
            .unwrap()
            .nodes
            .iter()
            .any(|fact| fact.fixture_id == "done"),
        "the confirmation stays hidden"
    );

    // Typing drives vx-model: each keystroke is one turn, the model
    // write lands, and the derived bindings repaint.
    type_into(&mut view, vp, "name", "Ada");
    assert_eq!(
        state_of(&view, "name"),
        ReactiveValue::String("Ada".to_owned())
    );
    assert_eq!(
        text_of(&view.layout_facts(vp).unwrap(), "greeting"),
        ["Hello, Ada"]
    );
    assert_eq!(
        text_of(&view.layout_facts(vp).unwrap(), "count"),
        ["3 character(s) typed"]
    );

    // Still disabled without an email.
    click_element(&mut view, vp, "submit");
    assert_eq!(state_of(&view, "submitted"), ReactiveValue::Bool(false));

    type_into(&mut view, vp, "email", "ada@lovelace.dev");
    assert_eq!(
        state_of(&view, "email"),
        ReactiveValue::String("ada@lovelace.dev".to_owned())
    );

    // Both fields filled: the button activates and its handler commits.
    click_element(&mut view, vp, "submit");
    assert_eq!(state_of(&view, "submitted"), ReactiveValue::Bool(true));
    assert_eq!(
        text_of(&view.layout_facts(vp).unwrap(), "done"),
        ["Signed up: ada@lovelace.dev"]
    );
    assert!(
        view.reactive_diagnostics().is_empty(),
        "{:?}",
        view.reactive_diagnostics()
    );
}

#[test]
fn tabs_example_conforms() {
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    let mut view = load_example("tabs");
    settle(&mut view, vp);
    assert!(
        view.tailwind_diagnostics().is_empty(),
        "{:?}",
        view.tailwind_diagnostics()
    );

    // The counter panel is the initial tab; the about panel is hidden
    // (display:none leaves no facts).
    let visible = |view: &mut VelquView| -> Vec<String> {
        view.layout_facts(vp)
            .unwrap()
            .nodes
            .iter()
            .filter(|fact| fact.fixture_id.starts_with("panel-"))
            .map(|fact| fact.fixture_id.clone())
            .collect()
    };
    assert_eq!(visible(&mut view), ["panel-count"]);

    // Counter works inside the tab: two turns, text and state agree.
    click_element(&mut view, vp, "inc");
    click_element(&mut view, vp, "inc");
    assert_eq!(text_of(&view.layout_facts(vp).unwrap(), "value"), ["2"]);
    assert_eq!(state_of(&view, "count"), ReactiveValue::Number(2.0));

    // Switch to about: one structural vx-show flip each way. Each
    // switch costs one render pass; each `visible()` observation runs
    // layout_facts, which is its own (structural-truth) pass.
    click_element(&mut view, vp, "tab-about");
    assert_eq!(visible(&mut view), ["panel-about"]);
    let passes_before = view.layout_stats().passes;
    click_element(&mut view, vp, "tab-count");
    assert_eq!(visible(&mut view), ["panel-count"]);
    assert_eq!(
        view.layout_stats().passes,
        passes_before + 2,
        "one render + one facts pass for the switch-back"
    );

    // State survives being hidden and re-shown.
    assert_eq!(text_of(&view.layout_facts(vp).unwrap(), "value"), ["2"]);
    assert!(
        view.reactive_diagnostics().is_empty(),
        "{:?}",
        view.reactive_diagnostics()
    );
}
