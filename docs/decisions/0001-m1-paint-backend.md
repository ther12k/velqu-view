# ADR 0001 — M1 paint backend: winit + softbuffer + fontdue CPU rasterizer

- Status: accepted (M0/M1, 2026-09-16)
- Scope: M1 window/paint milestone only; M2 backend choice is a separate,
  evidence-gated decision.

## Context

The M1 exit gate is "deterministic Hello fixture": native window, rectangle,
text, resize, DPI, frame capture — no reactive JS, no HTML semantics yet.
The renderer strategy (OKF `architecture/renderer-strategy.md`) calls for the
fastest credible path based on existing Rust technology, with the option of
selected Blitz components, and forbids exposing backend types publicly.

Candidate paths for M1:

1. wgpu + Vello (GPU) — heavyweight dependency tree for a rect+text probe;
   output can vary with GPU/driver; frame capture less direct.
2. Blitz components — pulls the full HTML/Stylo/Taffy stack, which M1
   explicitly does not exercise yet; larger surface than the milestone needs.
3. winit + softbuffer + fontdue (CPU) — small dependency tree, works on
   machines without working GPU acceleration, and frame "capture" *is* the
   frame buffer, making determinism trivially checkable.

## Decision

For M1, render with a small CPU rasterizer (bundled DejaVu fonts via
fontdue) into an RGBA8 `Frame`, and present via winit + softbuffer.

Constraints kept for the M2 swap:

- All rendering happens in `velqu-view` against the `Scene` primitive list;
  `velqu-shell` only moves finished frames to the window.
- The painter is crate-private; nothing outside `velqu-view` knows how pixels
  are produced.
- fontdue is compiled with `default-features = false, features = ["std"]`:
  its default `simd` feature gates an x86-only code path that could
  rasterize differently from the scalar path on other architectures, which
  would break cross-machine fixture hashes.

## Consequences

- No GPU dependency; CI renders offscreen with zero system requirements
  beyond a Rust toolchain for the test suite.
- Rectangles are un-antialiased (device-pixel snapped); text is coverage-
  blended only. Acceptable for the M1 probe; M2 revisits raster quality with
  the real backend.
- Presentation cost is a full-window blit per frame; fine for M1 scale,
  measured again at M2+.
- The M2 decision (Taffy/Parley/Vello, Blitz components, or hybrid) must
  preserve the `Scene → Frame` seam or replace it wholesale behind the same
  public API.
