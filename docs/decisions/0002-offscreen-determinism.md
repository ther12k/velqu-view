# ADR 0002 — Offscreen-first deterministic rendering; pixel-hash fixtures

- Status: accepted (M0/M1, 2026-09-16)

## Context

The testing strategy (OKF `engineering/testing-conformance.md`) requires
renderer fixtures with expected layout facts and reference images, run "in
CI broadly, offscreen rendering where supported". The benchmark plan
additionally requires retained, reproducible evidence. Both need renders
that produce identical bytes for identical inputs, on any machine.

## Decision

1. `VelquView::render(viewport)` is fully offscreen; opening a window is
   optional and lives in `velqu-shell`. A window is one sink for a `Frame`.
2. Determinism rules, enforced by construction and tested:
   - DPI is a pure function of the `Viewport` (no ambient screen state);
   - fonts are bundled in-repo (DejaVu), never discovered from the system;
   - the fontdue rasterizer uses its scalar path (see ADR 0001);
   - logical→device conversion is `round(v * scale_factor)`;
   - pixel output never depends on frame counters, timings, or locale.
3. Fixtures pin a SHA-256 of the RGBA buffer plus exact-color pixel probes
   (`tests/visual/*/fixture.toml`), and commit a human-checkable
   `baseline.png` next to the manifest.
4. `velqu-lab --headless --frames N` re-renders and fails if any frame
   differs, making the determinism contract executable from the CLI.

## Consequences

- The visual fixture suite runs anywhere Cargo runs; no display, GPU, or
  screenshot plumbing in CI.
- Changing raster output intentionally (e.g. the M2 backend) requires
  regenerating hashes and baselines — deliberately a visible, reviewable
  step: `cargo run -p velqu-lab -- --headless …` then update the manifest.
- Pixel hashes would not survive cross-version fontdue changes; the lockfile
  plus bundled assets keep that pinned. If it ever flakes, the exact-color
  probes and (M2) layout facts still carry the semantic contract.
