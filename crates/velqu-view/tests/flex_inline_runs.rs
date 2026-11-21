//! Flex item geometry and paint regressions for direct text and inline elements.

use velqu_view::{LayoutFacts, LayoutNodeFact, VelquView, Viewport};

fn fact<'a>(facts: &'a LayoutFacts, id: &str) -> &'a LayoutNodeFact {
    facts.nodes.iter().find(|f| f.fixture_id == id).unwrap()
}

#[test]
fn inline_elements_are_separate_flex_column_items() {
    let mut view = VelquView::new();
    view.load_html(
        r#"<body style="margin:0">
        <div data-vv-test="column" style="display:flex;flex-direction:column;
             width:100px;height:80px;padding:7px;border:2px solid black;gap:5px">
            <span data-vv-test="first" style="width:11px;height:6px"></span>
            <span data-vv-test="second" style="width:13px;height:9px"></span>
        </div></body>"#,
    )
    .unwrap();
    let vp = Viewport::try_new(200, 120, 1.0).unwrap();
    let facts = view.layout_facts(vp).unwrap();
    let column = fact(&facts, "column");
    let first = fact(&facts, "first");
    let second = fact(&facts, "second");
    assert_eq!((first.x, first.y), (column.content_x, column.content_y));
    assert_eq!((second.x, second.y), (first.x, first.y + 6.0 + 5.0));
}

#[test]
fn direct_text_keeps_its_source_position_between_inline_elements() {
    let vp = Viewport::try_new(200, 120, 1.0).unwrap();
    let build = |items: &str| {
        let mut view = VelquView::new();
        view.load_html(&format!(
            r#"<body style="margin:0"><div data-vv-test="row"
               style="display:flex;align-items:center;gap:6px;height:40px;">
               {items}</div></body>"#,
        ))
        .unwrap();
        view
    };
    let mut actual = build(
        " A \n<!-- join -->U <span data-vv-test=middle> B </span> C \
         <span data-vv-test=end>D</span> \n",
    );
    let mut reference = build(
        "<div>A U</div><div data-vv-test=middle>B</div><div>C</div><div data-vv-test=end>D</div>",
    );
    let actual_facts = actual.layout_facts(vp).unwrap();
    let reference_facts = reference.layout_facts(vp).unwrap();
    let positions = |facts: &LayoutFacts| [fact(facts, "middle").x, fact(facts, "end").x];
    assert_eq!(positions(&actual_facts), positions(&reference_facts));
    assert_eq!(fact(&actual_facts, "row").text_runs, ["A U", "C"]);
    assert_eq!(
        actual.render(vp).unwrap().frame.sha256_hex(),
        reference.render(vp).unwrap().frame.sha256_hex(),
    );
}

#[test]
fn anonymous_flex_text_does_not_repeat_parent_chrome_on_layout_or_repaint() {
    let vp = Viewport::try_new(150, 100, 1.0).unwrap();
    let build = |items: &str| {
        let mut view = VelquView::new();
        view.load_html(&format!(
            r#"<body style="margin:0"><div id="tile" data-vv-test="tile">{items}</div></body>"#,
        ))
        .unwrap();
        view.load_css(
            "#tile { display:flex; width:80px; height:28px; padding:8px; align-items:center;
                     font-size:16px; line-height:1; color:#000; background-color:#00f;
                     opacity:0.5; transform:translate(12px,4px); box-shadow:4px 4px #f00 }
             #tile:hover { color:#fff; background-color:#008000; opacity:0.75;
                           transform:translate(16px,4px); box-shadow:6px 8px #f00 }",
        )
        .unwrap();
        view
    };
    let mut actual = build("Hello");
    // The explicit (undecorated) flex item lets the parent paint its own
    // effects once, and a span makes its text color refresh on hover.
    let mut reference = build("<div><span>Hello</span></div>");
    let before = actual.render(vp).unwrap().frame.sha256_hex();
    assert_eq!(
        before,
        reference.render(vp).unwrap().frame.sha256_hex(),
        "initial layout"
    );
    assert_eq!(
        fact(&actual.layout_facts(vp).unwrap(), "tile").text_runs,
        ["Hello"]
    );

    let stats = actual.layout_stats();
    assert!(actual.pointer_move(vp, 14.0, 6.0));
    assert!(reference.pointer_move(vp, 14.0, 6.0));
    let after = actual.render(vp).unwrap().frame.sha256_hex();
    assert_eq!(
        after,
        reference.render(vp).unwrap().frame.sha256_hex(),
        "hover repaint"
    );
    assert_ne!(after, before);
    assert_eq!(actual.layout_stats().passes, stats.passes);
    assert!(actual.layout_stats().repaints > stats.repaints);
}

#[test]
fn ellipsis_fits_using_the_retained_run_size_and_weight() {
    let mut view = VelquView::new();
    view.load_html(
        r#"<body style="margin:0">
        <div style="display:flex;gap:8px;font-size:16px">
            <div data-vv-test="mixed" class="cut">A B <span style="font-size:80px">X</span></div>
            <div data-vv-test="bold" class="cut" style="font-weight:700">WWWW</div>
        </div></body>"#,
    )
    .unwrap();
    view.load_css(
        ".cut { width:66px; white-space:nowrap; text-overflow:ellipsis; overflow:hidden }",
    )
    .unwrap();
    let vp = Viewport::try_new(200, 120, 1.0).unwrap();
    let facts = view.layout_facts(vp).unwrap();
    // The 80px trailing run's mark is too wide, but the retained 16px
    // run's mark fits. Three bundled DejaVu Ws plus a mark fit in 66px
    // in regular weight, but not in bold.
    let lines = ["mixed", "bold"].map(|id| fact(&facts, id).text_runs.join("\n"));
    assert_eq!(lines, ["A B …", "WW …"]);
}
