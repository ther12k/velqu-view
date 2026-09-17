# Architecture Decision Records

Numbered, immutable once accepted; supersede explicitly.

| # | Decision | Status |
|---|---|---|
| [0001](0001-m1-paint-backend.md) | M1 paint backend: winit + softbuffer + fontdue CPU rasterizer | accepted |
| [0002](0002-offscreen-determinism.md) | Offscreen-first deterministic rendering; pixel-hash fixtures | accepted |
| [0003](0003-api-boundary.md) | VelquView API boundary: backend types never leak | accepted |
| [0004](0004-m11-api-resource-hardening.md) | M1.1: source identity, viewport invariants, host-side assets, concept-level CSS classification | accepted |
| [0005](0005-identity-separation.md) | DOM/style/layout/paint identity are distinct; fixtures use data-vv-test keys | accepted |
| [0006](0006-m2a-css-profile.md) | M2a named CSS profile; out-of-profile CSS is diagnosed, never ignored | accepted |
| [0007](0007-m2b-taffy-backend.md) | M2b: Taffy whole-tree backend; Velqu owns rounding/facts/profile | accepted |
| [0008](0008-m2c-images-grid-scroll.md) | M2c: bounded image decoding, replaced-element sizing, frozen grid profile, paint-side scrolling | accepted |
| [0009](0009-m3-tailwind-pipeline.md) | M3: v0 Tailwind-compatible pipeline (utilities → CSS → cascade) in velqu-tailwind | accepted |
| [0010](0010-m4a-input-gate.md) | M4a: input gate — hit testing as inverse of paint, runtime interaction state, browser-signed wheel with zero relayout | accepted |

Future decisions expected: text shaping engine (Parley candidate — see
[`docs/research/blitz-notes.md`](../research/blitz-notes.md) for how
blitz delegates white-space/wrapping to Parley and what MSRV that
implies), inline fragmentation strategy (blitz's custom-Taffy-mode
approach documented in the same note), UI QuickJS embedding crate, host
capability trait stabilization, compiled-Tailwind ingestion, and the
Tailwind conformance-corpus runner (WPT-style reftests).
