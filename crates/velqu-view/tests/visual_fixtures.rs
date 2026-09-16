//! Visual fixture harness: renders every `tests/visual/*/fixture.toml`
//! offscreen and asserts the expectations inside.
//!
//! See `tests/README.md` at the repository root for the fixture schema.
//! Fixtures live outside this crate; they are located relative to the
//! workspace root so the same files drive tests and `velqu-lab` capture.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use velqu_view::{Color, VelquView, Viewport};

#[derive(Deserialize)]
struct Fixture {
    #[allow(dead_code)]
    name: String,
    html: PathBuf,
    #[serde(default)]
    css: Vec<PathBuf>,
    viewport: ViewportSpec,
    expect: Expect,
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
}

#[derive(Deserialize)]
struct PixelProbe {
    pos: [u32; 2],
    color: String,
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

fn run_fixture(dir: &Path) {
    let fixture = load_fixture(dir);
    let mut view = VelquView::new();

    let html_path = workspace_root().join(&fixture.html);
    let html = std::fs::read_to_string(&html_path)
        .unwrap_or_else(|e| panic!("{}: {e}", html_path.display()));
    view.load_html(&html).expect("fixture html loads");

    for css_rel in &fixture.css {
        let css_path = workspace_root().join(css_rel);
        let css = std::fs::read_to_string(&css_path)
            .unwrap_or_else(|e| panic!("{}: {e}", css_path.display()));
        view.load_css(&css).expect("fixture css loads");
    }

    let viewport = Viewport::new(
        fixture.viewport.width,
        fixture.viewport.height,
        fixture.viewport.scale,
    );
    let result = view.render(viewport).expect("fixture renders");

    assert_eq!(result.frame.width(), viewport.width, "frame width");
    assert_eq!(result.frame.height(), viewport.height, "frame height");

    let actual_hash = result.frame.sha256_hex();
    match fixture.expect.pixels_sha256.as_deref() {
        Some("PENDING") | None => {
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
    // produce byte-identical frames (no hidden global state).
    for dir in fixture_dirs() {
        let fixture = load_fixture(&dir);
        let mut views = [VelquView::new(), VelquView::new()];
        for view in &mut views {
            let html = std::fs::read_to_string(workspace_root().join(&fixture.html)).unwrap();
            view.load_html(&html).unwrap();
            for css_rel in &fixture.css {
                let css = std::fs::read_to_string(workspace_root().join(css_rel)).unwrap();
                view.load_css(&css).unwrap();
            }
        }
        let viewport = Viewport::new(
            fixture.viewport.width,
            fixture.viewport.height,
            fixture.viewport.scale,
        );
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
