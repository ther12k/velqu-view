# Architecture Decision Records

Numbered, immutable once accepted; supersede explicitly.

| # | Decision | Status |
|---|---|---|
| [0001](0001-m1-paint-backend.md) | M1 paint backend: winit + softbuffer + fontdue CPU rasterizer | accepted |
| [0002](0002-offscreen-determinism.md) | Offscreen-first deterministic rendering; pixel-hash fixtures | accepted |
| [0003](0003-api-boundary.md) | VelquView API boundary: backend types never leak | accepted |
| [0004](0004-m11-api-resource-hardening.md) | M1.1: source identity, viewport invariants, host-side assets, concept-level CSS classification | accepted |

Future decisions expected: M2 layout/paint engine choice (Taffy/Parley/
Vello/Blitz components vs. custom), UI QuickJS embedding crate, host
capability trait stabilization.
