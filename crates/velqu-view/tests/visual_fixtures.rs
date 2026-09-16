//! M2a fixture harness.
//!
//! Two layers of assertions per `tests/visual/*/fixture.toml`:
//!
//! 1. **Structural first** — `layout_facts` expectations (exact geometry,
//!    box model, text runs) keyed by author-written `data-vv-test` ids
//!    (ADR 0005: internal `NodeId`s never leak into fixtures).
//! 2. **Raster second** — the pixel digest answers only "did the final
//!    raster change?" and exact-color probes pin deterministic colors.
//!
//! See `tests/README.md` for the fixture schema.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde::Deserialize;
use velqu_view::{Color, LayoutFacts, VelquView, Viewport};

#[derive(Deserialize)]
struct Fixture {
    #[allow(dead_code)]
    name: String,
    html: PathBuf,
    #[serde(default)]
    css: Vec<PathBuf>,
    viewport: ViewportSpec,
    expect: Expect,
    /// Synthetic image assets: `<img src="KEY">` resolves to a solid-color
    /// PNG/JPEG generated at test time. Deterministic bytes, no binary
    /// blobs in the repository.
    #[serde(default)]
    assets: BTreeMap<String, AssetSpec>,
    /// Programmatic scroll offsets (M2c): the key is the scroll target —
    /// the empty string is the document-level scroller, anything else an
    /// element `id` — and the value is `[x, y]`. Offsets are runtime state:
    /// facts stay unscrolled, only the raster moves.
    #[serde(default)]
    scroll: BTreeMap<String, [f32; 2]>,
}

#[derive(Deserialize)]
struct AssetSpec {
    /// "png" or "jpeg" (the M2c-supported formats).
    format: String,
    width: u32,
    height: u32,
    color: String,
}

fn encode_asset(spec: &AssetSpec) -> Vec<u8> {
    let color = Color::from_hex(&spec.color).unwrap_or_else(|e| panic!("asset color: {e}"));
    let img = image::RgbaImage::from_pixel(
        spec.width,
        spec.height,
        image::Rgba([color.r, color.g, color.b, color.a]),
    );
    let mut out = std::io::Cursor::new(Vec::new());
    match spec.format.as_str() {
        "png" => img
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("png asset encodes"),
        "jpeg" | "jpg" => image::DynamicImage::ImageRgba8(img)
            .to_rgb8()
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .expect("jpeg asset encodes"),
        other => panic!("unsupported asset format {other:?}"),
    }
    out.into_inner()
}

/// Serves fixture-declared assets to the renderer's `<img src>` requests.
struct FixtureAssets {
    map: std::collections::HashMap<String, Vec<u8>>,
}

impl velqu_view::AssetResolver for FixtureAssets {
    fn resolve(&self, request: velqu_view::AssetRequest<'_>) -> Option<velqu_view::Asset> {
        self.map.get(request.path).map(|bytes| velqu_view::Asset {
            id: velqu_view::SourceId::new(request.path),
            bytes: bytes.clone(),
        })
    }
}

#[derive(Deserialize)]
struct ViewportSpec {
    width: u32,
    height: u32,
    #[serde(default = "one")]
    scale: f32,
}

fn one() -> f32 {
    1.0
}

#[derive(Deserialize)]
struct Expect {
    #[serde(default)]
    pixels_sha256: Option<String>,
    #[serde(default)]
    pixel: Vec<PixelProbe>,
    /// Structural assertions per `data-vv-test` id.
    #[serde(default)]
    facts: Vec<FactExpect>,
}

#[derive(Deserialize)]
struct PixelProbe {
    pos: [u32; 2],
    color: String,
}

/// Structural expectation: only the fields present in the TOML are
/// compared (f32 compares are exact — layout is deterministic).
#[derive(Deserialize)]
struct FactExpect {
    id: String,
    tag: Option<String>,
    display: Option<String>,
    x: Option<f32>,
    y: Option<f32>,
    width: Option<f32>,
    height: Option<f32>,
    content_x: Option<f32>,
    content_y: Option<f32>,
    content_width: Option<f32>,
    content_height: Option<f32>,
    #[serde(default)]
    padding: Option<[f32; 4]>,
    #[serde(default)]
    border: Option<[f32; 4]>,
    #[serde(default)]
    margin: Option<[f32; 4]>,
    #[serde(default)]
    text_runs: Option<Vec<String>>,
    /// Scroll-container extents (M2c); omitted for non-scroll boxes.
    #[serde(default)]
    scroll_width: Option<f32>,
    #[serde(default)]
    scroll_height: Option<f32>,
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate is nested in workspace")
}

fn visual_fixtures_root() -> PathBuf {
    workspace_root().join("tests").join("visual")
}

fn load_fixture(dir: &Path) -> Fixture {
    let manifest = std::fs::read_to_string(dir.join("fixture.toml"))
        .unwrap_or_else(|e| panic!("{}: {e}", dir.join("fixture.toml").display()));
    toml::from_str(&manifest).unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
}

fn build_view(fixture: &Fixture) -> VelquView {
    let mut view = VelquView::new();
    let html_path = workspace_root().join(&fixture.html);
    let html = std::fs::read_to_string(&html_path)
        .unwrap_or_else(|e| panic!("{}: {e}", html_path.display()));
    view.load_document(
        velqu_view::DocumentSource::new(fixture.html.to_string_lossy().into_owned(), html)
            .with_base(
                html_path
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
    )
    .expect("fixture html loads");
    for css_rel in &fixture.css {
        let css_path = workspace_root().join(css_rel);
        let css = std::fs::read_to_string(&css_path)
            .unwrap_or_else(|e| panic!("{}: {e}", css_path.display()));
        view.load_stylesheet(velqu_view::StylesheetSource::new(
            css_rel.to_string_lossy().into_owned(),
            css,
        ))
        .expect("fixture css loads");
    }
    if !fixture.assets.is_empty() {
        let mut map = std::collections::HashMap::new();
        for (name, spec) in &fixture.assets {
            map.insert(name.clone(), encode_asset(spec));
        }
        view.set_asset_resolver(Rc::new(FixtureAssets { map }));
    }
    for (target, offset) in &fixture.scroll {
        let target = if target.is_empty() {
            None
        } else {
            Some(target.as_str())
        };
        view.set_scroll_offset(target, offset[0], offset[1])
            .expect("fixture scroll offset is valid");
    }
    view
}

fn run_fixture(dir: &Path) {
    let fixture = load_fixture(dir);
    let mut view = build_view(&fixture);

    let viewport = Viewport::try_new(
        fixture.viewport.width,
        fixture.viewport.height,
        fixture.viewport.scale,
    )
    .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));

    // Layer 1: structural facts.
    let facts: LayoutFacts = view.layout_facts(viewport).expect("fixture lays out");
    assert_eq!(
        facts.schema_version,
        velqu_view::LAYOUT_FACTS_SCHEMA_VERSION,
        "{}: schema version",
        dir.display()
    );
    assert_eq!(facts.viewport_width, viewport.width(), "facts width");
    assert_eq!(facts.viewport_height, viewport.height(), "facts height");

    for expected in &fixture.expect.facts {
        let actual = facts
            .nodes
            .iter()
            .find(|n| n.fixture_id == expected.id)
            .unwrap_or_else(|| {
                panic!(
                    "{}: no layout fact for data-vv-test={:?} (have {:?})",
                    dir.display(),
                    expected.id,
                    facts
                        .nodes
                        .iter()
                        .map(|n| &n.fixture_id)
                        .collect::<Vec<_>>()
                )
            });
        let FactExpect {
            id,
            tag,
            display,
            x,
            y,
            width,
            height,
            content_x,
            content_y,
            content_width,
            content_height,
            padding,
            border,
            margin,
            text_runs,
            scroll_width,
            scroll_height,
        } = expected;
        assert_eq!(Some(&actual.tag), tag.as_ref(), "{id}: tag");
        assert_eq!(Some(&actual.display), display.as_ref(), "{id}: display");
        assert_f32(id, "x", actual.x, *x);
        assert_f32(id, "y", actual.y, *y);
        assert_f32(id, "width", actual.width, *width);
        assert_f32(id, "height", actual.height, *height);
        assert_f32(id, "content_x", actual.content_x, *content_x);
        assert_f32(id, "content_y", actual.content_y, *content_y);
        assert_f32(id, "content_width", actual.content_width, *content_width);
        assert_f32(id, "content_height", actual.content_height, *content_height);
        if let Some(expected) = padding {
            assert_eq!(actual.padding, *expected, "{id}: padding");
        }
        if let Some(expected) = border {
            assert_eq!(actual.border, *expected, "{id}: border");
        }
        if let Some(expected) = margin {
            assert_eq!(actual.margin, *expected, "{id}: margin");
        }
        if let Some(expected) = text_runs {
            assert_eq!(&actual.text_runs, expected, "{id}: text_runs");
        }
        if let Some(expected) = scroll_width {
            assert_eq!(actual.scroll_width, Some(*expected), "{id}: scroll_width");
        }
        if let Some(expected) = scroll_height {
            assert_eq!(actual.scroll_height, Some(*expected), "{id}: scroll_height");
        }
    }

    // Layer 2: raster.
    let result = view.render(viewport).expect("fixture renders");
    assert_eq!(result.frame.width(), viewport.width(), "frame width");
    assert_eq!(result.frame.height(), viewport.height(), "frame height");

    let actual_hash = result.frame.sha256_hex();
    match fixture.expect.pixels_sha256.as_deref() {
        Some("PENDING") | None => {
            // Regenerate the human-checkable reference while the hash is
            // still pending, then fail with the digest to pin.
            let _ = result.frame.save_png(&dir.join("baseline.png"));
            panic!(
                "{}: fixture has no baseline hash; set pixels_sha256 = {actual_hash}",
                dir.display()
            );
        }
        Some(expected) => assert_eq!(
            expected,
            actual_hash,
            "{}: pixel digest changed (update baseline.png + hash after review)",
            dir.display()
        ),
    }

    for probe in &fixture.expect.pixel {
        let [x, y] = probe.pos;
        let expected = Color::from_hex(&probe.color).expect("probe color parses");
        let actual = result
            .frame
            .pixel(x, y)
            .unwrap_or_else(|| panic!("{}: probe ({x},{y}) outside frame", dir.display()));
        assert_eq!(
            actual,
            expected,
            "{}: probe at ({x},{y}) expected {expected}, got {actual}",
            dir.display()
        );
    }
}

fn assert_f32(id: &str, field: &str, actual: f32, expected: Option<f32>) {
    if let Some(expected) = expected {
        assert!(
            (actual - expected).abs() < 0.01,
            "{id}: {field} expected {expected}, got {actual}"
        );
    }
}

fn fixture_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(visual_fixtures_root())
        .expect("tests/visual exists")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_dir() && path.join("fixture.toml").exists())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no fixtures found under tests/visual");
    dirs
}

#[test]
fn visual_fixtures_match_baselines() {
    for dir in fixture_dirs() {
        run_fixture(&dir);
    }
}

#[test]
fn fixtures_are_deterministic_across_instances() {
    // Independent VelquView instances over the same fixture inputs must
    // produce byte-identical frames and identical facts (no hidden state).
    for dir in fixture_dirs() {
        let fixture = load_fixture(&dir);
        let viewport = Viewport::try_new(
            fixture.viewport.width,
            fixture.viewport.height,
            fixture.viewport.scale,
        )
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let mut views = [build_view(&fixture), build_view(&fixture)];
        let facts_a = views[0].layout_facts(viewport).unwrap();
        let facts_b = views[1].layout_facts(viewport).unwrap();
        assert_eq!(facts_a, facts_b, "{}: layout facts diverged", dir.display());
        let a = views[0].render(viewport).unwrap();
        let b = views[1].render(viewport).unwrap();
        assert_eq!(
            a.frame.pixels(),
            b.frame.pixels(),
            "{}: two instances diverged",
            dir.display()
        );
    }
}
