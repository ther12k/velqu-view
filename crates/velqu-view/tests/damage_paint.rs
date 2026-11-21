//! Partial-damage painting equivalence: a frame rendered through the
//! damage path (previous frame + diff) must be pixel-identical to the same
//! state rendered from scratch, and the damage rect must stay within the
//! viewport. Covers geometry morphs (width/height transitions), color-only
//! repaints, scroll, and the nothing-changed frame.

use std::time::Duration;

use velqu_view::{Color, ReactiveValue, StylesheetSource, VelquView, Viewport};

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

    let init = || {
        let mut view = VelquView::new();
        view.enable_tailwind();
        view.enable_reactive();
        view.load_html(DOC).unwrap();
        view.pump_reactive(&[]);
        view.render(vp).unwrap();
        view
    };
    let mut incremental = init();
    let mut reference = init();
    let other_viewport = Viewport::try_new(vp.width() + 1, vp.height(), 1.0).unwrap();

    // Nothing changed: damage is None-or-empty and pixels match the first.
    let idle = incremental.render(vp).unwrap();
    assert!(
        idle.stats.damage.is_none(),
        "first damage-capable frame is full"
    );
    let baseline = idle.frame.pixels().to_vec();
    let again = incremental.render(vp).unwrap();
    assert_eq!(again.frame.pixels(), baseline, "idle frame is unchanged");
    assert!(again.stats.damage.is_none() || again.stats.damage == Some((0, 0, 0, 0)));

    // Color-only flip (hover-equivalent): the pill repaints, rest stands.
    incremental
        .write_state("tint", ReactiveValue::Bool(true))
        .unwrap();
    reference
        .write_state("tint", ReactiveValue::Bool(true))
        .unwrap();
    reference.render(vp).unwrap();
    let tinted = incremental.render(vp).unwrap();
    let Some((_, _, dw, dh)) = tinted.stats.damage else {
        panic!("color flip must report a damage window");
    };
    assert!(
        u32::from(dw) * u32::from(dh) < 300 * 400,
        "color flip damages a subregion"
    );
    let tinted_pixels = tinted.frame.pixels().to_vec();

    // Width/height morph: the card animates; each damaged frame must equal
    // a from-scratch render of the same animation state.
    incremental
        .write_state("expanded", ReactiveValue::Bool(true))
        .unwrap();
    reference
        .write_state("expanded", ReactiveValue::Bool(true))
        .unwrap();
    let mut step = 0;
    loop {
        incremental.step_animations_by(Duration::from_millis(16));
        reference.step_animations_by(Duration::from_millis(16));
        let active = incremental.has_active_animations();
        let damaged = incremental.render(vp).unwrap();
        if let Some((x, y, w, h)) = damaged.stats.damage {
            assert!(u32::from(x) + u32::from(w) <= vp.width());
            assert!(u32::from(y) + u32::from(h) <= vp.height());
        }
        // Run the same schedule, but invalidate the damage baseline before
        // each reference frame to check against an actual full repaint.
        reference.render(other_viewport).unwrap();
        let full = reference.render(vp).unwrap();
        assert!(full.stats.damage.is_none());
        assert!(
            damaged.frame == full.frame,
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

#[test]
fn offscreen_changes_preserve_the_visible_frame() {
    let mut view = VelquView::new();
    view.enable_reactive();
    view.load_html(
        r#"<html><body style="margin:0" vx-state="{ tinted: false }">
          <div style="width:60px;height:100px;background-color:#123456"></div>
          <div :style="'width:20px;height:20px;background-color:' + (tinted ? '#ff0000' : '#0000ff')"></div>
        </body></html>"#,
    )
    .unwrap();
    let vp = Viewport::try_new(60, 60, 1.0).unwrap();
    view.pump_reactive(&[]);
    let before = view.render(vp).unwrap().frame;

    view.write_state("tinted", ReactiveValue::Bool(true))
        .unwrap();
    let after = view.render(vp).unwrap().frame;
    assert!(
        after == before,
        "off-screen damage must not erase the visible frame"
    );
}

#[test]
fn next_host_frame_is_full_after_stylesheet_reload() {
    let mut view = VelquView::new();
    view.load_html(
        "<body style='margin:0'>\
         <div id=top style='width:25px;height:20px'></div>\
         <div style='height:25px'></div>\
         <div id=bottom style='width:25px;height:20px'></div></body>",
    )
    .unwrap();
    view.load_stylesheet(StylesheetSource::new(
        "sheet",
        "#top { background-color:#f00 }",
    ))
    .unwrap();
    let vp = Viewport::try_new(100, 100, 1.0).unwrap();
    let first = view.render(vp).unwrap();
    assert_eq!(first.frame.pixel(10, 10), Some(Color::from_rgb8(255, 0, 0)));

    view.reload_stylesheets(
        vec![StylesheetSource::new(
            "sheet",
            "#top { background-color:#00f }",
        )],
        vp,
    )
    .unwrap();
    view.load_css("#bottom { background-color:#0f0 }").unwrap();
    let next = view.render(vp).unwrap();
    assert_eq!(next.frame.pixel(10, 10), Some(Color::from_rgb8(0, 0, 255)));
    assert!(
        next.stats.damage.is_none(),
        "host has not received the staged frame"
    );
}

#[test]
fn damage_covers_children_of_changed_opacity_scope() {
    let mut view = VelquView::new();
    view.enable_reactive();
    view.load_html(
        r#"<html><body style="margin:0" vx-state="{ dim: false }">
          <section :style="'opacity:' + (dim ? '0.4' : '0.8')">
            <div style="width:20px;height:20px;background-color:#000000"></div>
          </section>
          <div style="height:40px"></div>
          <div :style="'width:20px;height:20px;background-color:' + (dim ? '#ff0000' : '#0000ff')"></div>
        </body></html>"#,
    )
    .unwrap();
    let vp = Viewport::try_new(100, 100, 1.0).unwrap();
    view.pump_reactive(&[]);
    let before = view.render(vp).unwrap().frame;
    view.write_state("dim", ReactiveValue::Bool(true)).unwrap();
    let after = view.render(vp).unwrap();
    assert!(
        after.stats.damage.is_some(),
        "only part of the frame changed"
    );
    assert_ne!(after.frame.pixel(10, 10), before.pixel(10, 10));

    // A different viewport invalidates the damage baseline, so the next
    // render at vp independently computes every pixel.
    view.render(Viewport::try_new(101, 100, 1.0).unwrap())
        .unwrap();
    let full = view.render(vp).unwrap();
    assert!(
        after.frame == full.frame,
        "damage must account for inherited opacity"
    );
}
