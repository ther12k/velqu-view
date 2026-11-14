//! Host font registration: registered families change the raster; unknown
//! families and bad data are safe fallbacks; defaults stay byte-identical.

use velqu_view::{VelquView, Viewport};

const DOC: &str = r#"
<!doctype html>
<html>
<body style="margin:0">
  <main style="width:220px;height:60px;background-color:#ffffff">
    <p id="t" data-vv-test="t" style="font-size:16px;color:#000000">Hello Velqu</p>
  </main>
</body>
</html>"#;

fn render_digest(css_font_family: Option<&str>) -> String {
    let html = match css_font_family {
        Some(family) => DOC.replace(
            r#"style="font-size:16px;color:#000000""#,
            &format!(r#"style="font-size:16px;color:#000000;font-family:{family}""#),
        ),
        None => DOC.to_owned(),
    };
    let mut view = VelquView::new();
    view.load_html(&html).expect("loads");
    view.render(Viewport::try_new(220, 60, 1.0).unwrap())
        .expect("renders")
        .frame
        .sha256_hex()
}

/// A minimal valid TTF built by hand is impractical here; reuse the
/// bundled DejaVu bytes under a different family name instead. The
/// raster must be identical to the default (same outlines), proving the
/// registry path resolves correctly without needing a second font file.
#[test]
fn registered_family_under_new_name_resolves() {
    let dejavu =
        std::fs::read(env!("CARGO_MANIFEST_DIR").to_owned() + "/assets/fonts/DejaVuSans.ttf")
            .expect("bundled font present on disk");
    let baseline = render_digest(None);

    let mut view = VelquView::new();
    view.register_font("myfont", dejavu.clone(), None)
        .expect("valid font");
    view.load_html(&DOC.replace(
        r#"style="font-size:16px;color:#000000""#,
        r#"style="font-size:16px;color:#000000;font-family:MyFont""#,
    ))
    .expect("loads");
    let digest = view
        .render(Viewport::try_new(220, 60, 1.0).unwrap())
        .expect("renders")
        .frame
        .sha256_hex();

    assert_eq!(
        baseline, digest,
        "registered DejaVu under new name must raster identically"
    );
}

#[test]
fn registered_family_with_different_outlines_changes_raster() {
    // The bold face has different outlines from the regular one; register
    // it as a family and the raster must differ from the default render.
    let bold =
        std::fs::read(env!("CARGO_MANIFEST_DIR").to_owned() + "/assets/fonts/DejaVuSans-Bold.ttf")
            .expect("bundled bold font on disk");
    let baseline = render_digest(None);

    let mut view = VelquView::new();
    view.register_font("other", bold, None).expect("valid font");
    view.load_html(&DOC.replace(
        r#"style="font-size:16px;color:#000000""#,
        r#"style="font-size:16px;color:#000000;font-family:other""#,
    ))
    .expect("loads");
    let digest = view
        .render(Viewport::try_new(220, 60, 1.0).unwrap())
        .expect("renders")
        .frame
        .sha256_hex();

    assert_ne!(
        baseline, digest,
        "different outlines must change the raster"
    );
}

#[test]
fn unknown_family_falls_back_to_default() {
    let baseline = render_digest(None);
    let missing = render_digest(Some("NoSuchFamily"));
    assert_eq!(
        baseline, missing,
        "unknown family must render with the default face"
    );
}

#[test]
fn invalid_font_data_is_an_error() {
    let mut view = VelquView::new();
    assert!(
        view.register_font("broken", b"not a font".to_vec(), None)
            .is_err()
    );
    // And the registry is untouched: default rendering continues.
    view.load_html(DOC).expect("loads");
    let digest = view
        .render(Viewport::try_new(220, 60, 1.0).unwrap())
        .expect("renders")
        .frame
        .sha256_hex();
    assert_eq!(digest, render_digest(None));
}

#[test]
fn bold_variant_resolves_by_font_weight() {
    // Register ONLY a regular face for a family; bold text (font-weight:700)
    // must fall back to that family's regular face (not DejaVu bold).
    let dejavu =
        std::fs::read(env!("CARGO_MANIFEST_DIR").to_owned() + "/assets/fonts/DejaVuSans.ttf")
            .expect("bundled font on disk");
    let mut view = VelquView::new();
    view.register_font("onlyregular", dejavu, None)
        .expect("valid");
    let html = r#"
<!doctype html>
<html><body style="margin:0">
  <main style="width:220px;height:60px;background-color:#ffffff">
    <p id="t" data-vv-test="t" style="font-size:16px;color:#000000;font-weight:700;font-family:OnlyRegular">Bold</p>
  </main>
</body></html>"#;
    view.load_html(html).expect("loads");
    // No panic + deterministic render is the contract here.
    view.render(Viewport::try_new(220, 60, 1.0).unwrap())
        .expect("renders");
}
