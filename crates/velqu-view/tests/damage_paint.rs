//! Partial-damage painting equivalence: a frame rendered through the
//! damage path (previous frame + diff) must be pixel-identical to the same
//! state rendered from scratch, and the damage rect must stay within the
//! viewport. Covers geometry morphs (width/height transitions), color-only
//! repaints, scroll, and the nothing-changed frame.

use std::time::Duration;

use velqu_view::{ReactiveValue, VelquView, Viewport};

const DOC: &str = r#"
<!doctype html>
<html>
<body style="margin: 0">
  <main vx-state="{ expanded: false, tint: false, scrolled: false }"
        style="width:300px;height:400px" class="flex flex-col justify-start items-center">
    <div data-vv-test="pill" class="h-[46px] w-[240px] rounded-[22px] bg-blue-500 text-white"
         :class="tint ? 'bg-red-500' : 'bg-blue-500'">pill text</div>
    <div
      data-vv-test="card"
      class="mt-2 overflow-hidden transition-all duration-500 ease-out bg-green-600 text-white"
      :style="'width:' + (expanded ? '280px' : '120px') + ';height:' + (expanded ? '300px' : '0px')"
    >card body text line</div>
    <div data-vv-test="after" class="w-[100px] h-[40px] bg-amber-400"></div>
  </main>
</body>
</html>"#;

/// The damage path's core contract: render(→damage) equals a fresh render.
#[test]
fn damaged_frames_equal_fresh_renders() {
    let vp = Viewport::try_new(300, 400, 1.0).unwrap();

    // Sequence A: incremental renders on one instance (damage path).
    let mut incremental = VelquView::new();
    incremental.enable_tailwind();
    incremental.enable_reactive();
    incremental.load_html(DOC).unwrap();
    incremental.render(vp).unwrap();
    incremental.pump_reactive(&[]);
    incremental.render(vp).unwrap();

    // Nothing changed: damage is None-or-empty and pixels match the first.
    let idle = incremental.render(vp).unwrap();
    assert!(idle.stats.damage.is_none(), "first damage-capable frame is full");
    let baseline = idle.frame.pixels().to_vec();
    let again = incremental.render(vp).unwrap();
    assert_eq!(again.frame.pixels(), baseline, "idle frame is unchanged");
    assert!(again.stats.damage.is_none() || again.stats.damage == Some((0, 0, 0, 0)));

    // Color-only flip (hover-equivalent): the pill repaints, rest stands.
    incremental.write_state("tint", ReactiveValue::Bool(true)).unwrap();
    let tinted = incremental.render(vp).unwrap();
    let Some((_, _, dw, dh)) = tinted.stats.damage else {
        panic!("color flip must report a damage window");
    };
    assert!(u32::from(dw) * u32::from(dh) < 300 * 400, "color flip damages a subregion");
    let tinted_pixels = tinted.frame.pixels().to_vec();

    // Width/height morph: the card animates; each damaged frame must equal
    // a from-scratch render of the same animation state.
    incremental.write_state("expanded", ReactiveValue::Bool(true)).unwrap();
    let mut step = 0;
    loop {
        incremental.step_animations_by(Duration::from_millis(16));
        let active = incremental.has_active_animations();
        let damaged = incremental.render(vp).unwrap();
        if let Some((_, _, w, h)) = damaged.stats.damage {
            assert!(u32::from(w) <= 300 && u32::from(h) <= 400);
        }
        // Fresh instance driven through the identical schedule.
        let fresh = fresh_render_at(step);
        assert_eq!(
            damaged.frame.pixels(),
            fresh,
            "morph frame {step}: damage path diverged from scratch"
        );
        step += 1;
        if !active && step > 2 {
            break;
        }
        assert!(step < 200, "morph never settled");
    }
    let settled_pixels = incremental.render(vp).unwrap().frame.pixels().to_vec();
    assert_ne!(settled_pixels, tinted_pixels, "morph must change pixels");
    assert_ne!(settled_pixels, baseline, "expand must change pixels");
}

/// A fresh instance that performs the same turns and animation ticks as
/// the incremental one, then renders once — the damage-path oracle.
fn fresh_render_at(step: usize) -> Vec<u8> {
    let mut view = VelquView::new();
    let vp = Viewport::try_new(300, 400, 1.0).unwrap();
    view.enable_tailwind();
    view.enable_reactive();
    view.load_html(DOC).unwrap();
    view.render(vp).unwrap();
    view.pump_reactive(&[]);
    view.render(vp).unwrap();
    // Mirror the incremental sequence up to `step` morph ticks.
    view.write_state("tint", ReactiveValue::Bool(true)).unwrap();
    view.render(vp).unwrap();
    view.write_state("expanded", ReactiveValue::Bool(true)).unwrap();
    view.render(vp).unwrap();
    for _ in 0..step {
        view.step_animations_by(Duration::from_millis(16));
        view.render(vp).unwrap();
    }
    view.render(vp).unwrap().frame.pixels().to_vec()
}
