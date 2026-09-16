# Tests

Test layers and their homes:

| Layer | Location | Runs in CI |
|---|---|---|
| Unit (API, painter, colors, fonts) | `crates/*/src/**` `#[cfg(test)]` | yes |
| Reactive syntax surface | `crates/velqu-reactive` | yes |
| CSS profile classification | `crates/velqu-tailwind` | yes |
| Visual fixtures (offscreen render) | `tests/visual/*/fixture.toml` via `crates/velqu-view/tests/visual_fixtures.rs` | yes |
| Tailwind conformance corpus | `tests/tailwind/` (M3) | later |
| Reactive conformance | `tests/reactive/` (M5) | later |
| Input matrix | `tests/input/` (M4) | later |

## Visual fixture convention

Each fixture directory contains a `fixture.toml`:

```toml
name = "hello"
html = "examples/hello/index.html"     # relative to the repository root
css  = ["examples/hello/app.css"]
viewport = { width = 800, height = 600, scale = 1.0 }

[expect]
pixels_sha256 = "…"        # exact digest of the RGBA buffer

[[expect.pixel]]           # exact-color probes in device pixels
pos = [44, 144]
color = "#EF4444"
```

- Deterministic by construction: bundled fonts, scalar rasterizer, no
  timings/counters in pixel output.
- `baseline.png` next to a fixture is the human-checkable reference
  (regenerate with `velqu-lab --headless`).
- M2 adds `layout_facts` (expected geometry) to this schema.
