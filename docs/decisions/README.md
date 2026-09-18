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
| [0011](0011-m4b-interaction-styling.md) | M4b: interaction styling (:hover/:focus/:active frozen to paint) — interactive state never lays out | accepted |
| [0012](0012-m4c1-editable-controls.md) | M4c1: editable controls — opaque element identity, runtime control state, grapheme-safe editing | accepted |
| [0013](0013-m4c2-clipboard.md) | M4c2: clipboard — Copy/Cut/Paste through a host `ClipboardProvider`; arboard confined to the shell | accepted |
| [0014](0014-m4c3-ime.md) | M4c3: IME — preedit is presentation state, commit the only value mutation; session-scoped composition; shell owns enablement | accepted |
| [0015](0015-m5a-runtime-gate.md) | M5a: isolated QuickJS runtime gate — hard budgets, per-generation isolation, deterministic clock/random, no ambient I/O | accepted |
| [0016](0016-m5b-binding-compiler.md) | M5b: reactive binding compiler — vx-* markup lowers to a validated Rust-owned plan; JS never discovers the DOM | accepted |
| [0017](0017-m5c-reactive-turns.md) | M5c: reactive turns — atomic state+UI commits, plain-data state, compile-once units, model-before-handler, silent control writes | accepted |

Future decisions expected: text shaping engine (Parley candidate — see
[`docs/research/blitz-notes.md`](../research/blitz-notes.md) for how
blitz delegates white-space/wrapping to Parley and what MSRV that
implies), inline fragmentation strategy (blitz's custom-Taffy-mode
approach documented in the same note), UI QuickJS embedding crate, host
capability trait stabilization, compiled-Tailwind ingestion, and the
Tailwind conformance-corpus runner (WPT-style reftests).
