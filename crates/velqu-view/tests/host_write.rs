//! ADR 0025 conformance: host-initiated state writes through the public
//! API — the data-in seam for hosts that own external state (clocks,
//! media metadata, sensors) the document cannot produce from user
//! input alone. Every scenario drives `VelquView::write_state` against
//! a live document and asserts state, layout facts, and raster digests
//! under the same guarantees event turns have (ADR 0017): bounded,
//! transactional, non-reentrant.

use velqu_view::{HostWriteError, ReactiveValue, VelquView, Viewport};

/// The notch-shaped fixture: nested media state, a vx-show panel, and
/// a class binding — everything a host writes at runtime.
const DOC: &str = r#"
<!doctype html>
<html>
<body>
  <main vx-state="{ clock: '00:00', media: { playing: false, title: 'Nothing' }, expanded: false }">
    <p id="clock" data-vv-test="clock" vx-text="clock"></p>
    <p id="title" data-vv-test="title" vx-text="media.title"></p>
    <p id="panel" data-vv-test="panel" vx-show="expanded">Panel</p>
  </main>
</body>
</html>"#;

fn view_with(html: &str) -> VelquView {
    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();
    view.load_html(html).expect("document loads");
    view
}

fn settle(view: &mut VelquView, vp: Viewport) {
    view.render(vp).unwrap();
    view.pump_reactive(&[]);
    view.render(vp).unwrap();
    let _ = view.take_events();
}

fn text_of(view: &mut VelquView, vp: Viewport, fixture: &str) -> Vec<String> {
    view.layout_facts(vp)
        .unwrap()
        .nodes
        .into_iter()
        .find(|fact| fact.fixture_id == fixture)
        .unwrap_or_else(|| panic!("no fact {fixture}"))
        .text_runs
}

fn has_fact(view: &mut VelquView, vp: Viewport, fixture: &str) -> bool {
    view.layout_facts(vp)
        .unwrap()
        .nodes
        .iter()
        .any(|fact| fact.fixture_id == fixture)
}

fn state_of(view: &VelquView, path: &str) -> ReactiveValue {
    view.reactive_state()
        .unwrap_or_else(|| panic!("no reactive state"))
        .get_path(path)
}

#[test]
fn host_write_updates_text_output_and_state() {
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    let mut view = view_with(DOC);
    settle(&mut view, vp);

    view.write_state("clock", ReactiveValue::String("07:42".to_owned()))
        .expect("write commits");
    assert_eq!(
        state_of(&view, "clock"),
        ReactiveValue::String("07:42".to_owned())
    );
    assert_eq!(text_of(&mut view, vp, "clock"), ["07:42"]);
}

#[test]
fn host_write_supports_every_value_kind_and_nested_paths() {
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    let mut view = view_with(DOC);
    settle(&mut view, vp);

    // A nested object path the document never declared: set_path
    // creates the intermediates, bindings reevaluate against them.
    view.write_state(
        "media",
        ReactiveValue::Object(vec![
            (
                "title".to_owned(),
                ReactiveValue::String("Low Tide".to_owned()),
            ),
            ("playing".to_owned(), ReactiveValue::Bool(true)),
            ("track".to_owned(), ReactiveValue::Number(3.0)),
            (
                "tags".to_owned(),
                ReactiveValue::Array(vec![ReactiveValue::String("ambient".to_owned())]),
            ),
        ]),
    )
    .expect("write commits");
    assert_eq!(text_of(&mut view, vp, "title"), ["Low Tide"]);
    assert_eq!(state_of(&view, "media.track"), ReactiveValue::Number(3.0));
    assert!(matches!(
        state_of(&view, "media.playing"),
        ReactiveValue::Bool(true)
    ));
    // Numbers and bools pass through untouched — no string coercion.
    view.write_state("media.track", ReactiveValue::Number(9.0))
        .expect("write commits");
    assert_eq!(state_of(&view, "media.track"), ReactiveValue::Number(9.0));
}

#[test]
fn host_write_drives_vx_show_without_synthesizing_events() {
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    let mut view = view_with(DOC);
    settle(&mut view, vp);
    assert!(!has_fact(&mut view, vp, "panel"), "panel starts hidden");

    view.write_state("expanded", ReactiveValue::Bool(true))
        .expect("write commits");
    assert!(
        has_fact(&mut view, vp, "panel"),
        "vx-show followed the write"
    );

    // The no-feedback-loop contract (ADR 0017): a host write applies
    // mutations silently and never queues observable events.
    assert!(view.take_events().is_empty());
}

#[test]
fn host_write_before_first_pump_applies_turn_zero_first() {
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    let mut view = view_with(DOC);
    view.render(vp).unwrap();

    // No pump yet: the document still owes its initial outputs. The
    // write must apply them first, then land on top.
    view.write_state("clock", ReactiveValue::String("09:59".to_owned()))
        .expect("write commits");
    assert_eq!(text_of(&mut view, vp, "clock"), ["09:59"]);

    // A later empty pump must not replay the stale initial batch over
    // the write.
    view.pump_reactive(&[]);
    view.render(vp).unwrap();
    assert_eq!(text_of(&mut view, vp, "clock"), ["09:59"]);
    assert_eq!(text_of(&mut view, vp, "title"), ["Nothing"]);
}

#[test]
fn host_write_rolls_back_leaving_state_and_ui_untouched() {
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    // A binding whose expression compiles but throws on evaluation
    // rolls every later turn — including host writes — back.
    let html = DOC.replace(
        r#"<p id="clock""#,
        r#"<p vx-text="clock + missing.deeper.value"></p><p id="clock""#,
    );
    let mut view = view_with(&html);
    settle(&mut view, vp);
    let digest_before = view.render(vp).unwrap().frame.sha256_hex();

    match view.write_state("clock", ReactiveValue::String("11:11".to_owned())) {
        Err(HostWriteError::RolledBack(_)) => {}
        other => panic!("expected rollback, got {other:?}"),
    }
    assert_eq!(
        state_of(&view, "clock"),
        ReactiveValue::String("00:00".to_owned())
    );
    let digest_after = view.render(vp).unwrap().frame.sha256_hex();
    assert_eq!(digest_before, digest_after, "the UI is unchanged");
    assert!(
        !view.reactive_diagnostics().is_empty(),
        "the rollback reason is retained"
    );
}

#[test]
fn host_write_without_reactive_markup_is_no_reactive_document() {
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    let plain = "<!doctype html><html><body><p>static</p></body></html>";
    let mut view = view_with(plain);
    settle(&mut view, vp);
    assert_eq!(
        view.write_state("clock", ReactiveValue::String("1".to_owned())),
        Err(HostWriteError::NoReactiveDocument)
    );

    // Reactive never enabled at all: same answer, nothing ran.
    let mut bare = VelquView::new();
    bare.enable_tailwind();
    bare.load_html(plain).unwrap();
    assert_eq!(
        bare.write_state("clock", ReactiveValue::String("1".to_owned())),
        Err(HostWriteError::NoReactiveDocument)
    );
}

#[test]
fn host_write_and_event_turns_share_one_state() {
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    let html = r#"
<!doctype html>
<html>
<body>
  <main vx-state="{ count: 0 }">
    <span id="value" data-vv-test="value" vx-text="count"></span>
    <button id="inc" @click="count++">+</button>
  </main>
</body>
</html>"#;
    let mut view = view_with(html);
    settle(&mut view, vp);

    // The host seeds state; a real click builds on it — one state
    // store, two entry paths, ordered turns.
    view.write_state("count", ReactiveValue::Number(41.0))
        .expect("write commits");
    assert_eq!(text_of(&mut view, vp, "value"), ["41"]);

    let mut found = None;
    for y in (0..vp.height()).step_by(4) {
        for x in (0..vp.width()).step_by(8) {
            if view
                .hit_test(vp, x as f32 + 0.5, y as f32 + 0.5)
                .is_some_and(|target| target.element_id.as_deref() == Some("inc"))
            {
                found = Some((x as f32 + 0.5, y as f32 + 0.5));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let (x, y) = found.expect("no hit target for inc");
    view.pointer_press(vp, x, y);
    view.pointer_release(vp, x, y);
    let events = view.take_events();
    view.pump_reactive(&events);
    view.render(vp).unwrap();
    assert_eq!(text_of(&mut view, vp, "value"), ["42"]);
    assert!(view.reactive_diagnostics().is_empty());
}

#[test]
fn host_writes_are_deterministic_across_sessions() {
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    let digest = |mut view: VelquView| -> String {
        settle(&mut view, vp);
        view.write_state("clock", ReactiveValue::String("23:59".to_owned()))
            .expect("write commits");
        view.write_state("expanded", ReactiveValue::Bool(true))
            .expect("write commits");
        view.render(vp).unwrap().frame.sha256_hex()
    };
    assert_eq!(digest(view_with(DOC)), digest(view_with(DOC)));
}
