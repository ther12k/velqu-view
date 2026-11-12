//! Void-element attribute parsing: <img id=... data-vv-test=...> must
//! carry both into the DOM.

use velqu_view::{VelquView, Viewport};

#[test]
fn img_void_element_keeps_id_and_fixture() {
    let html = r#"
<!doctype html>
<html><body style="margin:0">
<main vx-state="{}" style="width:200px;height:60px" class="flex items-center">
  <img id="grip" data-vv-test="grip" src="icons/grip.png" alt="" class="shrink-0" style="width:14px;height:14px;opacity:0.55" />
</main>
</body></html>"#;

    let vp = Viewport::try_new(200, 60, 1.0).unwrap();
    let mut view = VelquView::new();
    view.load_html(html).expect("loads");
    view.render(vp).unwrap();
    let facts = view.layout_facts(vp).unwrap();
    let grip = facts.nodes.iter().find(|f| f.fixture_id == "grip");
    println!("GRIP FACT: {:?}", grip.as_ref().map(|f| (f.x, f.y, f.width, f.height)));
    // The img sits at (0,0,14,14) in this document (top-aligned): probe
    // its center, not the row's vertical middle.
    let grip = grip.expect("img data-vv-test must produce a layout fact");
    let hit = view
        .hit_test(vp, grip.x + grip.width / 2.0, grip.y + grip.height / 2.0)
        .and_then(|t| t.element_id);
    assert_eq!(hit.as_deref(), Some("grip"), "img id must be hit-testable");
}
