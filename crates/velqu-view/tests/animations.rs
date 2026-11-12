//! ADR 0024 conformance: Tailwind transitions and Alpine-style
//! `vx-transition` phases driven end-to-end through the public API with
//! the deterministic virtual clock (`step_animations_by`), asserting
//! pixels, layout facts, invalidation classes, and byte-identical
//! reproducibility across runs.
//!
//! What each scenario pins:
//! * **hover color transition** — the interaction path spawns from the
//!   cached box (no first-frame jump), repaints only (zero Taffy passes
//!   mid-flight), and lands on exact per-channel lerped midpoints;
//! * **reactive width transition** — the `SetClass` path captures
//!   pre-change geometry, animates through layout (the facts move), and
//!   settles at the target with the engine going idle;
//! * **vx-transition enter/leave** — the phase machine renders start
//!   frames, swaps classes, completes on settled animations, and hides
//!   on leave;
//! * **determinism** — the whole hover sequence re-run in a fresh view
//!   produces the identical frame-hash vector.

use std::time::Duration;

use velqu_view::{Color, LayoutCacheState, VelquView, Viewport};

fn viewport() -> Viewport {
    Viewport::try_new(400, 300, 1.0).unwrap()
}

fn settle(view: &mut VelquView, vp: Viewport) {
    view.render(vp).unwrap();
    view.pump_reactive(&[]);
    view.render(vp).unwrap();
    let _ = view.take_events();
}

/// Steps the clock by `dt` and renders, returning the frame hash.
fn step_and_render(view: &mut VelquView, vp: Viewport, dt_ms: u64) -> String {
    view.step_animations_by(Duration::from_millis(dt_ms));
    let result = view.render(vp).unwrap();
    result.frame.sha256_hex()
}

fn pixel_hex(view: &mut VelquView, vp: Viewport, x: u32, y: u32) -> String {
    let result = view.render(vp).unwrap();
    let Color { r, g, b, .. } = result.frame.pixel(x, y).expect("in bounds");
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn width_of(view: &mut VelquView, vp: Viewport, fixture: &str) -> f32 {
    let facts = view.layout_facts(vp).unwrap();
    facts
        .nodes
        .iter()
        .find(|node| node.fixture_id == fixture)
        .unwrap_or_else(|| panic!("fixture {fixture} missing"))
        .width
}

// ---------------------------------------------------------------------------

#[test]
fn hover_color_transition_repaints_without_layout_passes() {
    let vp = viewport();
    // The base color lives in an author rule (inline styles outrank
    // author rules, so a `:hover` target must not sit inline); the
    // transition spec travels inline — nothing overrides it.
    let html = r#"<html><body>
        <div id="card" data-vv-test="card"
             style="width: 200px; height: 100px;
                    transition: background-color 200ms linear"></div>
    </body></html>"#;
    let css = "#card { background-color: #0000ff; }\
               #card:hover { background-color: #ff0000; }";
    let mut view = VelquView::new();
    view.load_html(html).unwrap();
    view.load_css(css).unwrap();
    settle(&mut view, vp);

    // Baseline: the card is solid blue, nothing animates.
    assert_eq!(pixel_hex(&mut view, vp, 100, 50), "#0000ff");
    assert!(!view.has_active_animations());

    let passes_before = view.inspector_snapshot(vp, None).counters.layout_passes;

    // Hover: the differ spawns from the cached box, so this frame still
    // shows blue (t = 0 hold), with the engine now active.
    view.pointer_move(vp, 100.0, 50.0);
    assert_eq!(pixel_hex(&mut view, vp, 100, 50), "#0000ff");
    assert!(view.has_active_animations());

    // Midpoint under linear easing: exact per-channel lerp.
    step_and_render(&mut view, vp, 100);
    assert_eq!(pixel_hex(&mut view, vp, 100, 50), "#800080");

    // Completion: the target color, engine idle again.
    step_and_render(&mut view, vp, 100);
    assert_eq!(pixel_hex(&mut view, vp, 100, 50), "#ff0000");
    assert!(!view.has_active_animations());

    // The whole hover animation ran presentation-only: zero Taffy passes
    // since the hover began (ADR 0011's paint-only rule extends to
    // transitions — ADR 0024).
    let snapshot = view.inspector_snapshot(vp, None);
    assert_eq!(
        snapshot.counters.layout_passes, passes_before,
        "color transitions must never re-layout"
    );
    assert!(snapshot.counters.repaints > 0);

    // Un-hover retargets from the current (red) state back to blue.
    view.pointer_exit();
    assert_eq!(pixel_hex(&mut view, vp, 100, 50), "#ff0000");
    step_and_render(&mut view, vp, 100);
    assert_eq!(pixel_hex(&mut view, vp, 100, 50), "#800080");
    step_and_render(&mut view, vp, 100);
    assert_eq!(pixel_hex(&mut view, vp, 100, 50), "#0000ff");
    assert!(!view.has_active_animations());
}

#[test]
fn reactive_width_transition_moves_layout_facts() {
    let vp = viewport();
    let html = r#"<html><body>
        <div vx-state="{ open: false }">
            <div data-vv-test="panel"
                 :class="open
                     ? 'w-96 transition-all duration-200 ease-linear bg-green-500 h-10'
                     : 'w-64 transition-all duration-200 ease-linear bg-green-500 h-10'">
            </div>
            <button id="toggle" @click="open = !open" class="h-6 w-24">toggle</button>
        </div>
    </body></html>"#;
    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();
    view.load_html(html).unwrap();
    settle(&mut view, vp);

    // w-64 = 256px.
    assert_eq!(width_of(&mut view, vp, "panel"), 256.0);
    assert!(!view.has_active_animations());

    // Click the toggle: SetClass captures the pre-change width and the
    // first frame after the change still measures 256 (the override at
    // t = 0), never a jump to 384.
    click_id(&mut view, vp, "toggle");
    assert!(view.has_active_animations());
    assert_eq!(
        width_of(&mut view, vp, "panel"),
        256.0,
        "no first-frame jump"
    );

    // Halfway under linear: 256 + (384 - 256) / 2 = 320.
    step_and_render(&mut view, vp, 100);
    assert_eq!(width_of(&mut view, vp, "panel"), 320.0);

    // Settled at w-96 = 384, engine idle.
    step_and_render(&mut view, vp, 100);
    assert_eq!(width_of(&mut view, vp, "panel"), 384.0);
    assert!(!view.has_active_animations());
}

/// Clicks the element with `id` by scanning the laid-out hit targets
/// (the same technique as the M5 conformance suite).
fn click_id(view: &mut VelquView, vp: Viewport, id: &str) {
    let mut found = None;
    for y in (0..vp.height()).step_by(2) {
        for x in (0..vp.width()).step_by(2) {
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

#[test]
fn vx_transition_enter_and_leave_phases_run_the_machine() {
    let vp = viewport();
    let html = r#"<html><body style="margin: 0">
        <div vx-state="{ open: false }" style="margin: 0">
            <div data-vv-test="panel"
                 class="bg-red-500"
                 style="width: 200px; height: 100px; margin: 0"
                 vx-show="open"
                 vx-transition:enter="transition duration-200 ease-linear"
                 vx-transition:enter-start="opacity-0"
                 vx-transition:enter-end="opacity-100"
                 vx-transition:leave="transition duration-200 ease-linear"
                 vx-transition:leave-end="opacity-0">
            </div>
            <button id="toggle" @click="open = !open" class="h-6 w-24">toggle</button>
        </div>
    </body></html>"#;
    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();
    view.load_html(html).unwrap();
    settle(&mut view, vp);

    // Turn zero hid the panel instantly (no leave flash on load).
    assert!(!view.has_active_animations());
    assert_eq!(
        pixel_hex(&mut view, vp, 100, 50),
        "#ffffff",
        "panel hidden at rest"
    );

    // Open: EnterStart frame renders the start state (opacity 0).
    click_id(&mut view, vp, "toggle");
    assert_eq!(pixel_hex(&mut view, vp, 100, 50), "#ffffff");
    // The next tick swaps to the end classes and spawns; the frame at
    // spawn still holds the from-state.
    step_and_render(&mut view, vp, 0);
    assert!(view.has_active_animations());
    assert_eq!(pixel_hex(&mut view, vp, 100, 50), "#ffffff");

    // Midpoint: bg-red-500 (#ef4444) at 50% alpha over white — the exact
    // per-channel composite of a half-modulated source (r ≈ 246, g ≈ 161).
    step_and_render(&mut view, vp, 100);
    let mid = pixel_hex(&mut view, vp, 100, 50);
    assert!(
        mid == "#f6a1a1" || mid == "#f7a1a1",
        "half-opacity red-500 over white, got {mid}"
    );

    // Settled: fully opaque red-500.
    step_and_render(&mut view, vp, 100);
    assert!(!view.has_active_animations());
    assert_eq!(pixel_hex(&mut view, vp, 100, 50), "#ef4444");
    // The completion tick restores the base classes (opacity 100 ==
    // base state: no visual change).
    step_and_render(&mut view, vp, 0);
    assert_eq!(pixel_hex(&mut view, vp, 100, 50), "#ef4444");

    // Close: leave phases run the reverse arc, then hide.
    click_id(&mut view, vp, "toggle");
    step_and_render(&mut view, vp, 0); // swap to leave-end + spawn
    assert!(view.has_active_animations());
    assert_eq!(pixel_hex(&mut view, vp, 100, 50), "#ef4444");
    step_and_render(&mut view, vp, 100);
    let mid = pixel_hex(&mut view, vp, 100, 50);
    assert!(
        mid == "#f6a1a1" || mid == "#f7a1a1",
        "half-opacity red-500 fading out, got {mid}"
    );
    step_and_render(&mut view, vp, 100);
    assert!(!view.has_active_animations());
    // Completion hides the panel (one more tick advances the machine).
    step_and_render(&mut view, vp, 0);
    assert_eq!(
        pixel_hex(&mut view, vp, 100, 50),
        "#ffffff",
        "panel hidden after leave completes"
    );
}

#[test]
fn transition_sequences_are_bit_identical_across_runs() {
    fn run_hover_sequence() -> Vec<String> {
        let vp = viewport();
        let html = r#"<html><body>
            <div id="card" data-vv-test="card"
                 style="width: 200px; height: 100px;
                        transition: background-color 200ms linear"></div>
        </body></html>"#;
        let css = "#card { background-color: #0000ff; }\
                   #card:hover { background-color: #ff0000; }";
        let mut view = VelquView::new();
        view.load_html(html).unwrap();
        view.load_css(css).unwrap();
        settle(&mut view, vp);
        let mut hashes = vec![view.render(vp).unwrap().frame.sha256_hex()];
        view.pointer_move(vp, 100.0, 50.0);
        hashes.push(view.render(vp).unwrap().frame.sha256_hex());
        for _ in 0..4 {
            hashes.push(step_and_render(&mut view, vp, 50));
        }
        view.pointer_exit();
        hashes.push(view.render(vp).unwrap().frame.sha256_hex());
        for _ in 0..4 {
            hashes.push(step_and_render(&mut view, vp, 50));
        }
        hashes
    }
    let first = run_hover_sequence();
    let second = run_hover_sequence();
    assert_eq!(
        first, second,
        "identical schedules must produce identical frame bytes"
    );
    // The sequence actually animates: the hashes are not all equal
    // (mid-flight frames differ) and it ends idle.
    assert!(first.iter().any(|hash| hash != &first[0]));
}

#[test]
fn animation_state_dies_with_the_document_generation() {
    let vp = viewport();
    let html = r#"<html><body>
        <div id="card" style="width: 100px; height: 50px;
               transition: background-color 200ms linear"></div>
    </body></html>"#;
    let css = "#card { background-color: #0000ff; }\
               #card:hover { background-color: #ff0000; }";
    let mut view = VelquView::new();
    view.load_html(html).unwrap();
    view.load_css(css).unwrap();
    settle(&mut view, vp);
    view.pointer_move(vp, 50.0, 25.0);
    view.render(vp).unwrap();
    assert!(view.has_active_animations());

    // A new document clears the engine (node ids are per-parse).
    view.load_html(html).unwrap();
    assert!(!view.has_active_animations());
    // The fresh document renders cleanly from scratch.
    let snapshot = view.inspector_snapshot(vp, None);
    assert_eq!(snapshot.layout, LayoutCacheState::NotAvailable);
}

#[test]
fn transformed_hit_testing_follows_visual_surface() {
    let vp = viewport();
    // A button at (10, 10) with width 40, height 20.
    // Untransformed, it spans [10..50] horizontally and [10..30] vertically.
    // Transformed with translate(100px, 50px) and scale(2), its center (30, 20)
    // shifts by (100, 50) -> (130, 70), and width doubles to 80, height to 40.
    // The transformed box covers [90..170] x [50..90].
    let html = r#"<html><body style="margin: 0">
        <button id="btn" data-vv-test="btn"
                style="position: relative; margin: 10px; width: 40px; height: 20px;
                       transform: translate(100px, 50px) scale(2)">
            click
        </button>
    </body></html>"#;
    let mut view = VelquView::new();
    view.load_html(html).unwrap();
    settle(&mut view, vp);

    // Old untransformed spot (15, 15) must MISS.
    let hit_old = view.hit_test(vp, 15.0, 15.0);
    assert_ne!(
        hit_old.as_ref().and_then(|h| h.element_id.as_deref()),
        Some("btn"),
        "old untransformed spot must not hit"
    );

    // New transformed spot (130, 70) must HIT.
    let hit_new = view.hit_test(vp, 130.0, 70.0);
    assert_eq!(
        hit_new.as_ref().and_then(|h| h.element_id.as_deref()),
        Some("btn"),
        "transformed spot must hit the button"
    );
}

#[test]
fn looping_animations_pulse_and_spin_run_infinitely() {
    let vp = viewport();
    let html = r#"<html><body style="margin: 0">
        <div id="spinner" class="animate-spin w-10 h-10 bg-blue-500"></div>
        <div id="pulsing" class="animate-pulse w-10 h-10 bg-red-500"></div>
    </body></html>"#;
    let mut view = VelquView::new();
    view.enable_tailwind();
    view.load_html(html).unwrap();
    settle(&mut view, vp);

    // Both looping animations are active.
    assert!(view.has_active_animations());

    // Step several times across full duration cycles (1s for spin, 2s for pulse).
    for _ in 0..10 {
        step_and_render(&mut view, vp, 500);
        assert!(
            view.has_active_animations(),
            "looping animations must never finish on their own"
        );
    }
}

/// A looping animation inside a hidden subtree (vx-show false on an
/// ancestor) must not keep the frame pump awake: hidden nodes never
/// paint, so their infinite animations are stopped until they paint.
#[test]
fn looping_animation_in_hidden_subtree_stays_idle() {
    let vp = viewport();
    let html = r#"<html><body style="margin: 0">
        <main vx-state="{ open: false }">
            <div id="panel" vx-show="open">
                <div id="pulsing" class="animate-pulse w-10 h-10 bg-red-500"></div>
            </div>
        </main>
    </body></html>"#;
    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();
    view.load_html(html).unwrap();
    settle(&mut view, vp);

    // The panel is hidden: the pulse must NOT be running.
    assert!(
        !view.has_active_animations(),
        "an invisible looping animation must not keep the pump awake"
    );

    // Show the panel: sync restarts the pulse (visible again).
    view.write_state("open", velqu_view::ReactiveValue::Bool(true))
        .unwrap();
    view.pump_reactive(&[]);
    view.render(vp).unwrap();
    assert!(
        view.has_active_animations(),
        "a visible looping animation must run"
    );

    // Hide it again: the pump goes back to sleep.
    view.write_state("open", velqu_view::ReactiveValue::Bool(false))
        .unwrap();
    view.pump_reactive(&[]);
    view.render(vp).unwrap();
    assert!(
        !view.has_active_animations(),
        "re-hiding must stop the loop again"
    );
}
