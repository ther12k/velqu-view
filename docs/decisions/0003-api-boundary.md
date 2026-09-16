# ADR 0003 — VelquView API boundary: backend types never leak

- Status: accepted (M0/M1, 2026-09-16)

## Context

Core invariant 4 and the renderer strategy require that applications depend
only on Velqu-owned APIs, so the backend (today winit/softbuffer/fontdue;
tomorrow possibly Stylo/Taffy/Parley/Vello or Blitz components) stays
replaceable. The risk register calls out "backend types leak publicly" as a
high-severity risk.

## Decision

`velqu-view` defines the entire public contract:

```rust
VelquView::new / load_html / load_css / render(Viewport) -> FrameResult
Viewport { width, height, scale_factor }        // plain data
Frame { pixels(), pixel(x,y), sha256_hex(), save_png() }
RenderStats / FrameResult                       // diagnostics metadata
Color / VelquError                              // shared value types
```

Rules:

- No public method, field, or error variant of any Velqu crate may expose a
  type owned by winit, softbuffer, fontdue (or later Taffy/Stylo/Parley/
  Vello/Blitz). The backend crates are `dependencies`, and their types stay
  behind private modules.
- `velqu-shell` consumes `Frame`s; it does not produce them.
- Errors crossing the API are Velqu-owned enums with actionable messages;
  backend errors are converted at the boundary (`From` impls in
  `velqu-shell`, which is itself outside the application-facing render API).
- Exchange currency between render and presentation is the plain `Frame`
  buffer, not a GPU texture or window handle.

Enforcement today: by construction and review (no backend type is re-exported
anywhere). When the M2 engine lands, add a compile-time check (e.g. a
`velqu-api` facade crate compiling only against public items).

## Consequences

- Application and test code written against M1 (`velqu-lab`, the fixture
  harness) survives backend replacement unchanged.
- Backend crates can be swapped or version-bumped without semver impact on
  VelquView's public surface.
