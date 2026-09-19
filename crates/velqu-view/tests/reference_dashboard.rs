//! M7 conformance: the reference dashboard (`examples/reference-dashboard`)
//! driven end-to-end through VelquView's **public API only** — the exact
//! files a developer runs with `velqu-lab --tailwind --reactive`. Identity
//! comes from `data-vv-test` (facts) and HTML `id`s (interaction); pointer
//! positions are derived by hit-test scan, never fixed coordinates;
//! keyboard goes through `set_focus` + `key_command`/`insert_text`; wheel
//! through `wheel`. The acceptance matrix lives here and in
//! `docs/reference-dashboard.md`.

use std::path::PathBuf;

// Reviewed-then-frozen raster digests (docs/reference-dashboard.md);
// regeneration instructions live there.
const M7_INITIAL_DIGEST: &str = "a89813c53c651e717abfd7926bcac5974149440005055bf1f45f93cd747a8c5c";
const M7_EDITED_DIGEST: &str = "8b430f60ca82c8c0f71fd108892d7bab7adafacae3739794b63b981874b2fc44";
const M7_EMPTY_DIGEST: &str = "12a04cb68fae1605092e5e192c547758ab836d9782df02f24b855512af4f6b29";
const M7_SCROLLED_DIGEST: &str = "cae3e4aa911de89b67570ff9b5b2b2cbd780d0ecb1119e27875373aa8f2d2d3d";
const M7_SMALL_DIGEST: &str = "0fa9e6fc215f6350940644ff089b6513cc9e8605de16e88ff78968b20efce647";

use velqu_reactive::ReactiveValue;
use velqu_view::{Event, KeyCommand, KeyModifiers, ReloadStage, VelquView, Viewport};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn app_dir() -> PathBuf {
    workspace_root().join("examples/reference-dashboard")
}

/// Loads the reference dashboard exactly as the lab does: the HTML plus
/// every `*.css` sorted by name (cascade order), Tailwind + reactive on.
fn load_reference() -> VelquView {
    let html = std::fs::read_to_string(app_dir().join("index.html")).expect("index.html");
    let mut css_files: Vec<PathBuf> = std::fs::read_dir(app_dir())
        .expect("app dir")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "css"))
        .collect();
    css_files.sort();
    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();
    view.enable_inspector();
    view.load_html(&html).expect("reference html loads");
    for css in css_files {
        let sheet = std::fs::read_to_string(&css).expect("css");
        let id = css.file_name().unwrap().to_string_lossy().into_owned();
        view.load_stylesheet(velqu_view::StylesheetSource::new(id, sheet))
            .expect("sheet loads");
    }
    view
}

const VP: (u32, u32) = (1280, 800);

fn viewport(width: u32, height: u32, scale: f32) -> Viewport {
    Viewport::try_new(width, height, scale).unwrap()
}

fn settle(view: &mut VelquView, vp: Viewport) {
    view.render(vp).unwrap();
    let batch = view.take_events();
    view.pump_reactive(&batch);
    view.render(vp).unwrap();
    let _ = view.take_events();
}

/// A viewport point over an element by HTML id (hit-test scan).
fn point_over(view: &VelquView, vp: Viewport, id: &str) -> (f32, f32) {
    for y in (0..vp.height()).step_by(4) {
        for x in (0..vp.width()).step_by(8) {
            if view
                .hit_test(vp, x as f32 + 0.5, y as f32 + 0.5)
                .is_some_and(|target| target.element_id.as_deref() == Some(id))
            {
                return (x as f32 + 0.5, y as f32 + 0.5);
            }
        }
    }
    panic!("no hit target for {id}");
}

fn click_id(view: &mut VelquView, vp: Viewport, id: &str) {
    let (x, y) = point_over(view, vp, id);
    view.pointer_press(vp, x, y);
    view.pointer_release(vp, x, y);
    let batch = view.take_events();
    view.pump_reactive(&batch);
    view.render(vp).unwrap();
}

/// Replaces an input's text through the public keyboard path. The
/// current value is read back through `control_value` (public runtime
/// state), so clearing works without relying on selection semantics:
/// move the caret to the end (programmatic focus parks it at the
/// value start, where Backspace can do nothing), backspace until
/// empty, then type. The guard turns a no-progress clear into a loud
/// failure instead of a hang.
fn type_into(view: &mut VelquView, vp: Viewport, id: &str, text: &str) {
    view.set_focus(Some(id));
    let _ = view.take_events();
    view.key_command(KeyCommand::End, KeyModifiers::default());
    let _ = view.take_events();
    let handle = {
        let (x, y) = point_over(view, vp, id);
        view.hit_test(vp, x, y).unwrap().handle
    };
    let mut guard = 0;
    while view
        .control_value(handle)
        .is_some_and(|value| !value.is_empty())
    {
        guard += 1;
        assert!(guard < 256, "clearing {id} made no progress");
        view.key_command(KeyCommand::Backspace, KeyModifiers::default());
        let batch = view.take_events();
        view.pump_reactive(&batch);
        view.render(vp).unwrap();
    }
    if !text.is_empty() {
        view.insert_text(text);
        let batch = view.take_events();
        view.pump_reactive(&batch);
        view.render(vp).unwrap();
    }
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

fn fixture_present(view: &mut VelquView, vp: Viewport, fixture: &str) -> bool {
    view.layout_facts(vp)
        .unwrap()
        .nodes
        .iter()
        .any(|fact| fact.fixture_id == fixture)
}

fn state_str(view: &VelquView, path: &str) -> String {
    match view.reactive_state().unwrap().get_path(path) {
        ReactiveValue::String(text) => text,
        other => panic!("state {path} is not a string: {other:?}"),
    }
}

/// 1. Initial dashboard: correct summary values, selected section,
///    layout relationships, reviewed raster (deterministic).
#[test]
fn m7_initial_dashboard() {
    let vp = viewport(VP.0, VP.1, 1.0);
    let mut view = load_reference();
    settle(&mut view, vp);
    // Zero diagnostics: nothing silently unsupported.
    assert!(
        view.tailwind_diagnostics().is_empty(),
        "{:?}",
        view.tailwind_diagnostics()
    );
    assert!(
        view.reactive_diagnostics().is_empty(),
        "{:?}",
        view.reactive_diagnostics()
    );
    // Sections: shipments live, others hidden.
    assert!(fixture_present(&mut view, vp, "section-shipments"));
    assert!(fixture_present(&mut view, vp, "nav-shipments"));
    assert!(!fixture_present(&mut view, vp, "section-gates"));
    assert!(!fixture_present(&mut view, vp, "section-reports"));
    // Summary cards: 6 records, 0 unsaved, 2 waiting.
    assert_eq!(text_of(&mut view, vp, "summary-shown"), ["6"]);
    assert_eq!(text_of(&mut view, vp, "summary-unsaved"), ["0"]);
    assert_eq!(text_of(&mut view, vp, "summary-waiting"), ["2"]);
    // Default selection: TRK-8841's detail is live, others hidden.
    assert!(fixture_present(&mut view, vp, "detail-8841"));
    assert!(!fixture_present(&mut view, vp, "detail-2210"));
    // Reviewed raster (docs/reference-dashboard.md): deterministic.
    let first = view.render(vp).unwrap().frame.sha256_hex();
    let second = view.render(vp).unwrap().frame.sha256_hex();
    assert_eq!(first, second);
    assert_eq!(first, M7_INITIAL_DIGEST);
}

/// 2. Filter and clear: matching records appear; the no-results state
///    is useful; clearing restores all six.
#[test]
fn m7_filter_and_clear() {
    let vp = viewport(VP.0, VP.1, 1.0);
    let mut view = load_reference();
    settle(&mut view, vp);

    type_into(&mut view, vp, "search", "waiting");
    assert_eq!(text_of(&mut view, vp, "summary-shown"), ["2"]);
    assert!(fixture_present(&mut view, vp, "row-2210"));
    assert!(fixture_present(&mut view, vp, "row-7742"));
    assert!(!fixture_present(&mut view, vp, "row-8841"));

    type_into(&mut view, vp, "search", "zzz");
    assert_eq!(text_of(&mut view, vp, "summary-shown"), ["0"]);
    assert!(fixture_present(&mut view, vp, "no-results"));

    type_into(&mut view, vp, "search", "");
    assert_eq!(text_of(&mut view, vp, "summary-shown"), ["6"]);
    assert!(!fixture_present(&mut view, vp, "no-results"));
    assert!(fixture_present(&mut view, vp, "row-9158"));
}

/// 3./4. Select and edit: details follow the selection; Save updates
///    every dependent view; a disabled Save neither activates nor
///    focuses.
#[test]
fn m7_select_edit_save_disabled() {
    let vp = viewport(VP.0, VP.1, 1.0);
    let mut view = load_reference();
    settle(&mut view, vp);

    click_id(&mut view, vp, "row-2210");
    assert!(fixture_present(&mut view, vp, "detail-2210"));
    assert!(!fixture_present(&mut view, vp, "detail-8841"));

    // Edit the name: unsaved becomes 1 (card + save state agree).
    type_into(&mut view, vp, "edit-name-2210", "Siti Rahayu (vip)");
    assert_eq!(text_of(&mut view, vp, "summary-unsaved"), ["1"]);
    assert_eq!(state_str(&view, "name_2210"), "Siti Rahayu (vip)");

    // Save: everything agrees again.
    click_id(&mut view, vp, "save-2210");
    assert_eq!(text_of(&mut view, vp, "summary-unsaved"), ["0"]);
    assert!(fixture_present(&mut view, vp, "detail-2210"));

    // The disabled Save does not activate: no state change, no new
    // focus, no click event.
    view.set_focus(None);
    let _ = view.take_events();
    let (x, y) = point_over(&view, vp, "save-2210");
    view.pointer_press(vp, x, y);
    view.pointer_release(vp, x, y);
    let events = view.take_events();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::Click { .. } | Event::FocusChanged { .. })),
        "disabled Save neither clicks nor focuses: {events:?}"
    );
    let batch = Vec::new();
    view.pump_reactive(&batch);
    assert_eq!(text_of(&mut view, vp, "summary-unsaved"), ["0"]);
}

/// 5. Hover, focus, and scroll under a stationary pointer: correct
///    interaction state with **zero Taffy passes** (counters read
///    through cached inspector snapshots only).
#[test]
fn m7_hover_focus_scroll_are_presentation_only() {
    let vp = viewport(VP.0, VP.1, 1.0);
    let mut view = load_reference();
    settle(&mut view, vp);
    // Baseline through cached data.
    let before = view.inspector_snapshot(vp, None).counters;

    let (row_x, row_y) = point_over(&view, vp, "row-8841");
    view.pointer_move(vp, row_x, row_y);
    let _ = view.take_events();
    view.render(vp).unwrap();
    let after_hover = view.inspector_snapshot(vp, None).counters;
    assert_eq!(
        after_hover.layout_passes, before.layout_passes,
        "hover: 0 Taffy"
    );
    assert!(after_hover.repaints > before.repaints, "hover repaints");

    view.set_focus(Some("search"));
    let _ = view.take_events();
    view.render(vp).unwrap();
    let after_focus = view.inspector_snapshot(vp, None).counters;
    assert_eq!(
        after_focus.layout_passes, before.layout_passes,
        "focus: 0 Taffy"
    );

    view.wheel(vp, row_x, row_y, 0.0, 60.0);
    let events = view.take_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Scrolled { .. })),
        "the records card scrolls: {events:?}"
    );
    view.render(vp).unwrap();
    let after_scroll = view.inspector_snapshot(vp, None).counters;
    assert_eq!(
        after_scroll.layout_passes, before.layout_passes,
        "scroll: 0 Taffy"
    );
}

/// Filtering is structural but bounded: at most one document layout
/// per settled turn batch.
#[test]
fn m7_filter_costs_at_most_one_layout() {
    let vp = viewport(VP.0, VP.1, 1.0);
    let mut view = load_reference();
    settle(&mut view, vp);
    let _ = view.layout_facts(vp); // let the facts pass leave the baseline
    view.render(vp).unwrap();
    let before = view.inspector_snapshot(vp, None).counters;

    type_into(&mut view, vp, "search", "waiting");
    let after = view.inspector_snapshot(vp, None).counters;
    assert!(
        after.layout_passes <= before.layout_passes + 1,
        "one filter turn settled in at most one layout: {} → {}",
        before.layout_passes,
        after.layout_passes
    );
}

/// 6. CSS reload after editing: new styling publishes while model
///    values, selection, focus, and valid scroll state survive.
#[test]
fn m7_css_reload_preserves_live_state() {
    let vp = viewport(VP.0, VP.1, 1.0);
    let mut view = load_reference();
    settle(&mut view, vp);
    // Live state: a filter, a selection, an unsaved edit, focus.
    type_into(&mut view, vp, "search", "waiting");
    click_id(&mut view, vp, "row-2210");
    view.set_focus(Some("edit-name-2210"));
    view.key_command(KeyCommand::SelectAll, KeyModifiers::default());
    let _ = view.take_events();
    view.insert_text("Siti R.");
    let batch = view.take_events();
    view.pump_reactive(&batch);
    view.render(vp).unwrap();
    assert_eq!(text_of(&mut view, vp, "summary-unsaved"), ["1"]);

    // A valid CSS-only replacement (hover color change).
    let css = std::fs::read_to_string(app_dir().join("app.css")).unwrap();
    let replaced = css.replace("#1e293b; /* slate-800 */", "#0f172a; /* slate-900 */");
    assert_ne!(css, replaced, "the fixture changed");
    view.reload_stylesheets(
        vec![velqu_view::StylesheetSource::new("app.css", replaced)],
        vp,
    )
    .unwrap();

    // Everything survived; the runtime still operates.
    assert_eq!(
        text_of(&mut view, vp, "summary-shown"),
        ["2"],
        "filter survived"
    );
    assert!(
        fixture_present(&mut view, vp, "detail-2210"),
        "selection survived"
    );
    assert_eq!(
        text_of(&mut view, vp, "summary-unsaved"),
        ["1"],
        "edit survived"
    );
    assert_eq!(view.focused(), Some("edit-name-2210"), "focus survived");
    click_id(&mut view, vp, "save-2210");
    assert_eq!(text_of(&mut view, vp, "summary-unsaved"), ["0"]);
}

/// 7./8. Rejected full-bundle reload keeps the app usable; a
///    successful full reload publishes fresh source-defined state and
///    stale handles/events cannot affect the replacement.
#[test]
fn m7_full_reload_rejection_then_publication() {
    let vp = viewport(VP.0, VP.1, 1.0);
    let mut view = load_reference();
    settle(&mut view, vp);
    type_into(&mut view, vp, "search", "waiting");
    let generation = view.inspector_snapshot(vp, None).generation;
    let stale_handle = {
        let (x, y) = point_over(&view, vp, "row-2210");
        view.hit_test(vp, x, y).unwrap().handle
    };
    let sheets = view.stylesheets().to_vec();

    // A bundle whose reactive initializer throws: rejected, app usable.
    let throwing = "<!doctype html><html><body style=\"margin: 0\">\
        <main vx-state=\"{ boom: (function () { throw new Error('x') })() }\">\
        <p data-vv-test=broken vx-text=\"boom\">x</p>\
        </main></body></html>";
    let rejection = view
        .reload_bundle(
            velqu_view::DocumentSource::new("index.html", throwing),
            sheets.clone(),
            vp,
        )
        .unwrap_err();
    assert_eq!(rejection.stage, ReloadStage::ReactiveInitialization);
    assert_eq!(
        view.inspector_snapshot(vp, None).generation,
        generation,
        "the active generation is unchanged"
    );
    // The old application still accepts input.
    type_into(&mut view, vp, "search", "cleared");
    assert_eq!(text_of(&mut view, vp, "summary-shown"), ["3"]);

    // A valid full reload: fresh source-defined state; the stale
    // handle reports its generation; a stale event batch is inert.
    view.set_focus(Some("search"));
    let _ = view.take_events();
    view.insert_text("zzz");
    let stale_batch = view.take_events();
    let html = std::fs::read_to_string(app_dir().join("index.html")).unwrap();
    let new_generation = view
        .reload_bundle(
            velqu_view::DocumentSource::new("index.html", html),
            sheets,
            vp,
        )
        .unwrap();
    assert_ne!(new_generation, generation);
    assert_eq!(
        state_str(&view, "search"),
        "",
        "source-defined initial state"
    );
    assert_eq!(text_of(&mut view, vp, "summary-shown"), ["6"]);
    let snapshot = view.inspector_snapshot(vp, Some(stale_handle));
    assert_eq!(
        snapshot.selection_note,
        Some("the selection belongs to an older generation")
    );
    view.pump_reactive(&stale_batch);
    assert_eq!(state_str(&view, "search"), "", "stale events are inert");
}

/// Regenerates the five frozen digests and their review PNGs.
/// Not part of the gate: run explicitly with
/// `cargo test -p velqu-view --test reference_dashboard m7_regenerate -- --ignored --nocapture`
/// after an intentional fixture or engine change, review the PNGs in
/// /tmp/ref-states, then pin the printed digests above
/// (docs/reference-dashboard.md records the procedure).
#[test]
#[ignore = "digest/PNG regeneration tool; see docs/reference-dashboard.md"]
fn m7_regenerate() {
    std::fs::create_dir_all("/tmp/ref-states").unwrap();
    let primary = viewport(VP.0, VP.1, 1.0);
    let mut view = load_reference();
    settle(&mut view, primary);
    let frame = view.render(primary).unwrap();
    println!("INITIAL {}", frame.frame.sha256_hex());
    frame
        .frame
        .save_png(std::path::Path::new("/tmp/ref-states/initial.png"))
        .unwrap();
    click_id(&mut view, primary, "row-2210");
    type_into(&mut view, primary, "edit-name-2210", "Siti Rahayu (vip)");
    let frame = view.render(primary).unwrap();
    println!("EDITED {}", frame.frame.sha256_hex());
    frame
        .frame
        .save_png(std::path::Path::new("/tmp/ref-states/edited.png"))
        .unwrap();
    type_into(&mut view, primary, "search", "zzz");
    let frame = view.render(primary).unwrap();
    println!("EMPTY {}", frame.frame.sha256_hex());
    frame
        .frame
        .save_png(std::path::Path::new("/tmp/ref-states/empty.png"))
        .unwrap();
    type_into(&mut view, primary, "search", "");
    println!(
        "post-clear search={:?} shown={:?}",
        state_str(&view, "search"),
        text_of(&mut view, primary, "summary-shown")
    );
    view.set_focus(None);
    let _ = view.take_events();
    let (row_x, row_y) = point_over(&view, primary, "row-8841");
    view.wheel(primary, row_x, row_y, 0.0, 80.0);
    let _ = view.take_events();
    let frame = view.render(primary).unwrap();
    println!("SCROLLED {}", frame.frame.sha256_hex());
    frame
        .frame
        .save_png(std::path::Path::new("/tmp/ref-states/scrolled.png"))
        .unwrap();
    let small = viewport(800, 600, 1.0);
    let mut small_view = load_reference();
    settle(&mut small_view, small);
    let frame = small_view.render(small).unwrap();
    println!("SMALL {}", frame.frame.sha256_hex());
    frame
        .frame
        .save_png(std::path::Path::new("/tmp/ref-states/small.png"))
        .unwrap();
}

/// Visual baselines at the documented viewports and scales: reviewed
/// screenshots before frozen digests (docs/reference-dashboard.md).
#[test]
fn m7_visual_baselines() {
    let primary = viewport(VP.0, VP.1, 1.0);
    let mut view = load_reference();
    settle(&mut view, primary);
    // Initial @ 1280x800 logical @1x (the documented run).
    let initial = view.render(primary).unwrap().frame.sha256_hex();
    assert_eq!(initial, M7_INITIAL_DIGEST);

    // Edited form state (reviewed): unsaved edit on the selection.
    click_id(&mut view, primary, "row-2210");
    type_into(&mut view, primary, "edit-name-2210", "Siti Rahayu (vip)");
    let edited = view.render(primary).unwrap().frame.sha256_hex();
    assert_eq!(edited, M7_EDITED_DIGEST);

    // Empty-result state (reviewed).
    type_into(&mut view, primary, "search", "zzz");
    let empty = view.render(primary).unwrap().frame.sha256_hex();
    assert_eq!(empty, M7_EMPTY_DIGEST);

    // Scrolled records card (reviewed): back to all rows, then wheel.
    type_into(&mut view, primary, "search", "");
    view.set_focus(None);
    let _ = view.take_events();
    let (row_x, row_y) = point_over(&view, primary, "row-8841");
    view.wheel(primary, row_x, row_y, 0.0, 80.0);
    let _ = view.take_events();
    let scrolled = view.render(primary).unwrap().frame.sha256_hex();
    assert_eq!(scrolled, M7_SCROLLED_DIGEST);

    // A smaller supported viewport (reviewed): sensible layout, its own
    // baseline — not a responsive claim.
    let small = viewport(800, 600, 1.0);
    let mut small_view = load_reference();
    settle(&mut small_view, small);
    // Conformance facts, not aesthetics: the fixed nav rail keeps its
    // authored width (a `flex-1` sibling with a zero basis must not
    // starve it), and nothing escapes the viewport horizontally.
    {
        let facts = small_view.layout_facts(small).unwrap();
        let detail = facts
            .nodes
            .iter()
            .find(|fact| fact.fixture_id == "detail-8841")
            .expect("detail present at 800x600");
        assert!(
            detail.x + detail.width <= 800.0,
            "detail panel stays inside the viewport: {}+{}",
            detail.x,
            detail.width
        );
    }
    let rail = small_view
        .hit_test(small, 60.0, 300.0)
        .and_then(|hit| {
            small_view
                .inspector_snapshot(small, Some(hit.handle))
                .selected
        })
        .expect("nav rail hit at 800x600");
    assert_eq!(rail.border_box.2, 256.0, "w-64 rail keeps its width");
    assert!(
        rail.border_box.3 >= 560.0,
        "rail stretches to the main content height: {}",
        rail.border_box.3
    );
    assert_eq!(
        small_view.render(small).unwrap().frame.sha256_hex(),
        M7_SMALL_DIGEST
    );

    // Device scales 1.25x and 2x: deterministic per configuration
    // (same logical page, sharper raster; each double-render stable).
    for scale in [1.25, 2.0] {
        let physical = viewport(
            (VP.0 as f32 * scale).round() as u32,
            (VP.1 as f32 * scale).round() as u32,
            scale,
        );
        let mut scaled = load_reference();
        settle(&mut scaled, physical);
        let first = scaled.render(physical).unwrap().frame.sha256_hex();
        let second = scaled.render(physical).unwrap().frame.sha256_hex();
        assert_eq!(first, second, "scale {scale}: deterministic");
    }
}

/// Resource baseline (docs/evidence/m7-reference-dashboard.md): a
/// scripted journey's cumulative turn/layout/repaint counters and RSS
/// across bounded full reloads. Ignored — run explicitly with
/// `--ignored --nocapture` after intentional changes; counters are
/// deterministic for the journey, memory is indicative only.
#[test]
#[ignore = "resource-baseline measurement tool; see docs/evidence/m7-reference-dashboard.md"]
fn m7_resource_journey() {
    let vp = viewport(VP.0, VP.1, 1.0);
    let mut view = load_reference();
    settle(&mut view, vp);

    fn rss_kb() -> u64 {
        std::fs::read_to_string("/proc/self/status")
            .expect("/proc available")
            .lines()
            .find(|line| line.starts_with("VmRSS:"))
            .and_then(|line| line.split_whitespace().nth(1).map(String::from))
            .and_then(|kb| kb.parse().ok())
            .expect("VmRSS line")
    }

    let print_step = |view: &VelquView, label: &str| {
        let counters = view.inspector_snapshot(vp, None).counters;
        println!(
            "{label}: turns={} layouts={} repaints={} items={} rss={}kB",
            counters.reactive_turns,
            counters.layout_passes,
            counters.repaints,
            counters.display_items_last,
            rss_kb(),
        );
    };

    // The scripted journey: filter → select → edit → save → clear.
    print_step(&view, "initial");
    type_into(&mut view, vp, "search", "waiting");
    print_step(&view, "filter-waiting");
    click_id(&mut view, vp, "row-2210");
    print_step(&mut view, "select-2210");
    type_into(&mut view, vp, "edit-name-2210", "Siti Rahayu (vip)");
    print_step(&mut view, "edit-name");
    click_id(&mut view, vp, "save-2210");
    print_step(&mut view, "save");
    type_into(&mut view, vp, "search", "");
    print_step(&mut view, "clear-filter");

    // Memory across bounded full reloads (same source): indicative.
    let html = std::fs::read_to_string(app_dir().join("index.html")).unwrap();
    let sheets = view.stylesheets().to_vec();
    for cycle in 1..=5 {
        view.reload_bundle(
            velqu_view::DocumentSource::new("index.html", html.clone()),
            sheets.clone(),
            vp,
        )
        .expect("reload publishes");
        settle(&mut view, vp);
        println!(
            "reload-cycle-{cycle}: generation={} rss={}kB",
            view.inspector_snapshot(vp, None).generation,
            rss_kb()
        );
    }
}
