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

Future decisions expected: Taffy adoption for flex/grid (M2b/M2c), text
shaping engine (Parley candidate), UI QuickJS embedding crate, host
capability trait stabilization.
