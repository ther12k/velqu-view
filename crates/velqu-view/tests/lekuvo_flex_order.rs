//! Isolation test: does a Tailwind flex row preserve source order when
//! children mix empty boxes, text spans, and nested box containers?

use velqu_view::{VelquView, Viewport};

#[test]
fn flex_row_source_order_with_mixed_children() {
    let html = r#"
<!doctype html>
<html><body style="margin:0">
<main vx-state="{ clock: '18:48' }" style="width:560px;height:100px">
  <div id="row" class="flex items-center gap-3 h-10 px-5 rounded-2xl">
    <div id="mascot" data-vv-test="mascot" class="w-7 h-7 shrink-0"></div>
    <span id="divider" data-vv-test="divider" class="w-px h-5 shrink-0"></span>
    <div id="labelcol" data-vv-test="labelcol" class="flex flex-col justify-center min-w-0" style="width:170px">
      <span id="label" data-vv-test="label" class="text-sm">napping</span>
    </div>
    <span id="clock" data-vv-test="clock" vx-text="clock" class="text-lg"></span>
    <div id="grip" data-vv-test="grip" class="w-7 h-7 shrink-0 text-xs">&#10431;</div>
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
    let mut rows: Vec<(String, f32, f32, f32)> = facts
        .nodes
        .iter()
        .filter(|f| f.fixture_id != "row")
        .map(|f| (f.fixture_id.clone(), f.x, f.width, f.height))
        .collect();
    rows.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    for (id, x, w, h) in &rows {
        println!("{id:8} x={x:7.1} w={w:6.1} h={h:5.1}");
    }
    // Source order at the top level: mascot, divider, labelcol, clock,
    // grip (`label` nests inside `labelcol`, sharing its x).
    let order: Vec<&str> = rows.iter().map(|(id, ..)| id.as_str()).collect();
    assert_eq!(
        order,
        vec!["mascot", "divider", "labelcol", "label", "clock", "grip"],
        "flex children out of source order: {order:?}"
    );
    let label_x = rows.iter().find(|(id, ..)| id == "label").unwrap().1;
    let col_x = rows.iter().find(|(id, ..)| id == "labelcol").unwrap().1;
    assert_eq!(label_x, col_x, "nested label must sit inside labelcol");
}
