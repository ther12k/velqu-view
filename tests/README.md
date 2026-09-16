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

# Optional: synthetic image assets for <img src> (M2c). The key is the src
# reference exactly as written in the HTML; bytes are generated at test time
# (deterministic; no binary blobs in the repository).
[assets."red.png"]
format = "png"                         # png | jpeg
width = 40
height = 20
color = "#ef4444"

[expect]
pixels_sha256 = "…"        # exact digest of the RGBA buffer ("PENDING"
                           # regenerates baseline.png and fails with the digest)

[[expect.pixel]]           # exact-color probes in device pixels
pos = [44, 144]
color = "#EF4444"
```

- Deterministic by construction: bundled fonts, scalar rasterizer, no
  timings/counters in pixel output.
- `baseline.png` next to a fixture is the human-checkable reference
  (regenerate by setting `pixels_sha256 = "PENDING"` and running the suite).
- M2 adds `layout_facts` (expected geometry) to this schema.
- M2c adds image facts (replaced elements size by intrinsic ratio; broken
  images keep a 300×150 footprint and paint nothing).
