//! EQ-bar shape repro: a span with inline `display:flex` holding three
//! fixed-size bar spans, inside a flex row.

use velqu_view::{VelquView, Viewport};

#[test]
fn inline_flex_container_lays_bars_in_a_row() {
    let html = r#"
<!doctype html>
<html><body style="margin:0">
<main vx-state="{ playing: true }" style="width:560px;height:100px" class="flex items-center">
  <div class="flex items-center gap-2 min-w-0">
    <span id="label" data-vv-test="label" class="text-sm">Midnight City</span>
    <span id="eq" data-vv-test="eq" style="display:flex;align-items:flex-end;gap:2px;height:12px">
      <span id="b1" data-vv-test="b1" style="width:3px;height:12px;background-color:#8EA7F7"></span>
      <span id="b2" data-vv-test="b2" style="width:3px;height:9px;background-color:#8EA7F7"></span>
      <span id="b3" data-vv-test="b3" style="width:3px;height:7px;background-color:#8EA7F7"></span>
    </span>
  </div>
</main>
</body></html>"#;

    let vp = Viewport::try_new(560, 100, 1.0).unwrap();
    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();
    view.load_html(html).expect("loads");
    view.render(vp).unwrap();
    view.pump_reactive(&[]);
    view.render(vp).unwrap();

    let facts = view.layout_facts(vp).unwrap();
    for id in ["eq", "b1", "b2", "b3"] {
        if let Some(f) = facts.nodes.iter().find(|f| f.fixture_id == id) {
            println!(
                "FACT {id}: display={} x={:.1} y={:.1} w={:.1} h={:.1}",
                f.display, f.x, f.y, f.width, f.height
            );
        } else {
            println!("FACT {id}: ABSENT");
        }
    }
    let b1 = facts
        .nodes
        .iter()
        .find(|f| f.fixture_id == "b1")
        .expect("b1");
    let b2 = facts
        .nodes
        .iter()
        .find(|f| f.fixture_id == "b2")
        .expect("b2");
    let b3 = facts
        .nodes
        .iter()
        .find(|f| f.fixture_id == "b3")
        .expect("b3");
    assert!(
        (b1.height - 12.0).abs() < 1.0,
        "b1 height {} != 12",
        b1.height
    );
    assert!(
        (b2.height - 9.0).abs() < 1.0,
        "b2 height {} != 9",
        b2.height
    );
    assert!(
        (b3.height - 7.0).abs() < 1.0,
        "b3 height {} != 7",
        b3.height
    );
    assert!(
        b2.y > b1.y && b3.y > b2.y,
        "bars must be bottom-aligned (flex-end): b1.y={} b2.y={} b3.y={}",
        b1.y,
        b2.y,
        b3.y
    );
    assert!(
        (b1.x - b2.x).abs() > 2.0,
        "bars must sit in a horizontal row: b1.x={} b2.x={}",
        b1.x,
        b2.x
    );
}
