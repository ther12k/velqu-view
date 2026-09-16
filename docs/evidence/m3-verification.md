# M3 verification evidence — the Tailwind pipeline (phase 1 finish)

Scope: make VelquView render HTML styled with Tailwind utilities
natively — the phase-1 exit. Pipeline shape per ADR 0009: utilities →
CSS → cascade; the renderer core stays generic.

## Commits

| Commit | Content |
|---|---|
| `e88f01f` | m3: add the v0 Tailwind-compatible utility synthesizer |
| *(this commit)* | m3: wire the pipeline into VelquView, dashboard example, fixture, ADR 0009 + evidence |

## The phase-1 demo

`velqu-lab --tailwind examples/tailwind-dashboard` renders a complete
dashboard — sidebar, active nav item, 3-column stat-card grid, status
table — from `index.html` utility classes with **zero CSS files**:
60 paint items, `tailwind: all utility classes compiled`, deterministic
SHA over frames, headless and window (Wayland + X11) paths.

## Pipeline evidence

* **Compile:** `velqu_tailwind::compile_utilities` maps the renderer's
  frozen profile as utilities: layout (flex/grid/spacing/sizing), the
  full v3 palette (22 families × 11 shades, unit-tested anchor values),
  typography, borders/radius, overflow. 23 crate tests cover mappings,
  palette anchors, dedup/order, and override semantics.
* **Cascade:** the generated sheet joins after every author sheet.
  Pinned by `tailwind_utilities_lose_to_inline_but_beat_element_rules`:
  utilities beat element rules (class specificity), inline styles beat
  utilities.
* **Opt-in:** without `enable_tailwind()`, utilities do nothing and
  produce no diagnostics (`tailwind_is_opt_in_…`).
* **Loud failures:** unsupported classes (`shadow-md`, `hover:flex`,
  `w-1/2`, `mx-auto`, `absolute`, …) each get a deterministic,
  guidance-bearing diagnostic via `tailwind_diagnostics()`; the lab
  prints them. Nothing is dropped silently.
* **box-sizing:** the preflight rule is real renderer behavior now —
  `box-sizing: border-box` is parsed and honored in Taffy; the default
  remains `content-box`, so every pre-Tailwind fixture is unchanged
  (regression gate: all M2a/M2b/M2c facts + rasters identical).

## Fixture

`tests/visual/tailwind-hello` — a document with **no CSS files**:
`bg-slate-100 m-0 p-4` page, `bg-white border rounded-lg p-4 w-64`
card (border-box: exactly 256px wide), `text-xl font-bold text-slate-900`
title, `text-sm text-slate-500 mt-2` body. Facts + pixel probes pin the
palette colors (page `#f1f5f9`, border `#e5e7eb`, card white) and the
border-box width.

## Final verification at freeze

* `cargo test --workspace --locked`: all green — 110 velqu-view lib
  tests, 23 velqu-tailwind tests, 17 visual fixtures, cross-instance
  determinism, doctests.
* `cargo fmt --all -- --check`: clean.
* `cargo clippy --workspace --all-targets`: clean.
* `cargo +1.87.0 check --workspace --all-targets --locked`: clean.
* `velqu-lab --headless --tailwind` on the dashboard example: deterministic
  frames, no diagnostics; window smoke on Wayland + forced X11.
* CI: both lanes green on the phase-1 freeze push.

## Honest scope notes (M3 items not in this pass)

* The compatibility **checker CLI** (`velqu css check`) is deferred: the
  classification API (ADR 0004) and the in-pipeline utility diagnostics
  already produce the verdicts; the CLI is a thin wrapper for a later
  commit.
* Real **compiled-Tailwind ingestion** (consuming an actual
  `tailwindcss` build output) is the upgrade path behind the same seam;
  v0 ships the built-in synthesizer instead of a node dependency.
* **SVG icons** are recorded as raster-img-only for v0 (ADR 0009 §5).
* `border-radius` values compile and compute but the rasterizer does not
  paint rounded corners yet (corners render square); noted in ADR 0009
  as a known visual limitation, not a profile gap.
