// Text centering regression (ADR 0026 follow-up): a flex-centered 10px
// line in a 32px pill must place its ink within ~1px of the box center —
// the baseline model uses the face's real hhea metrics (font.rs
// `verticals`), not the old constant ratios that sat text ~2px low.
use velqu_view::{VelquView, Viewport};

#[test]
fn pill_text_centers_in_flex_box() {
    let vp = Viewport::try_new(120, 40, 1.0).unwrap();
    let html = r#"<html><head><style>
      button { width:100px; height:32px; background:#6ee7b7; display:flex;
               align-items:center; justify-content:center; padding:0 10px;
               font-size:10px; font-weight:800; }
    </style></head><body style="margin:0">
      <button><span>To-do</span></button>
    </body></html>"#;
    let mut view = VelquView::new();
    view.load_html(html).unwrap();
    let r = view.render(vp).unwrap();
    let mut ink_ys = Vec::new();
    let mut mint_ys = Vec::new();
    for y in 0..40 {
        for x in 0..120 {
            let c = r.frame.pixel(x, y).unwrap();
            if c.r < 100 && c.g < 100 && c.b < 120 {
                ink_ys.push(y);
            }
            if c.g > 180 && c.r < 180 {
                mint_ys.push(y);
            }
        }
    }
    let ink_center = (*ink_ys.iter().min().unwrap() + *ink_ys.iter().max().unwrap()) as f32 / 2.0;
    let box_center = (*mint_ys.iter().min().unwrap() + *mint_ys.iter().max().unwrap()) as f32 / 2.0;
    let offset = ink_center - box_center;
    assert!(
        offset.abs() <= 1.5,
        "text ink must center in the pill: offset {offset:+.1}px"
    );
}
