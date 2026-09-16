# VelquView

**VelquView** is a native local-application UI runtime: it renders local
HTML + (Tailwind-derived) CSS with a small Alpine-inspired reactive layer
(*Velqu Reactive*), **without embedding Chromium, WebView2, WebKit, or any
other browser engine**.

It is an *application renderer*, not a web browser. No iframes, no Service
Workers, no arbitrary browser JavaScript, no browser storage or navigation.
The point is efficient, predictable rendering of serious application UI
using the authoring model developers already know.

The initial executable is **VelquView Lab** (`apps/velqu-lab`), the
development host used to load, test, inspect, and eventually benchmark
VelquView apps.

```
HTML + Tailwind + Velqu Reactive
        ↓
    VelquView (native Rust)
        ↓
native lightweight application UI
```

## Status

| Milestone | Scope | State |
|---|---|---|
| M0 | workspace, CI, fixtures, decision log, evidence format | **done** |
| M1 | native window, rectangle/text paint, resize/DPI, frame capture | **done** |
| M1.1 | API/resource hardening: source identity, viewport invariants, host-side assets, MSRV/licenses ([ADR 0004](docs/decisions/0004-m11-api-resource-hardening.md)) | **done** |
| M2a | HTML parsing, cascade, block layout, text, display list, layout facts | **done** |
| M2b | Taffy whole-tree backend, flex profile, display-list clipping, overflow | **done** |
| M2c | grid, images/assets, scroll | next |
| M3 | Tailwind pipeline + compatibility profile fixtures | |
| M4 | input gate (pointer/keyboard/focus/text/selection/IME) | |
| M5 | Velqu Reactive v0 (isolated UI QuickJS) | |
| M6 | VelquView Lab inspector + reload | |
| M7 | reference Tailwind dashboard | |
| M8 | Mini IDE benchmark shell | |
| M9 | Electron/Tauri/VelquView comparative benchmark | |
| M10 | GO / NO-GO decision | |

Full roadmap: [`docs/okf/planning/milestones.md`](docs/okf/planning/milestones.md).

## Layout

```
crates/
  velqu-view/       public renderer API + M1 CPU paint backend (crates/velqu-view)
  velqu-reactive/   vx-* syntax surface (runtime lands M5)
  velqu-tailwind/   Velqu CSS Profile v0 manifest + classification
  velqu-shell/      winit window + softbuffer presentation, DPI/resize
apps/
  velqu-lab/        development host (window + headless fixture capture)
examples/           hello (M1 fixture), counter, forms/dashboard/mini-ide (later)
tests/              visual fixtures + future tailwind/reactive/input corpora
docs/               architecture, decisions, evidence, and the OKF spec bundle
```

## Quickstart

```bash
cargo test --workspace                    # everything, no display needed

# native window (Esc to close)
cargo run -p velqu-lab -- examples/hello

# deterministic offscreen capture (fixtures/CI)
cargo run -p velqu-lab -- --headless --size 800x600 --frames 5 \
    --out frame.png examples/hello
```

The headless run prints the frame's pixel digest and per-frame wall time, and
fails if frames differ — that determinism check is the M1 exit gate
(`tests/visual/hello`).

## Architecture invariants

1. **Native Rust runtime** — no WASM inside VelquView.
2. **Application renderer, not browser** — no browser compatibility surface.
3. **Tailwind via compiled CSS** — VelquView never hard-codes utility names;
   Tailwind compiles to CSS, Velqu validates it against the CSS Profile.
4. **Velqu owns the public API** — no Blitz/Stylo/Taffy/Parley/Vello/winit
   types leak to applications; the backend stays replaceable.
5. **Existing technology first** — fastest credible path now (winit +
   softbuffer + fontdue), evidence-gated engine integration later.
6. **Renderer mechanics stay in Rust** — layout, paint, hit testing,
   scrolling, focus, caret, selection, IME never route through QuickJS.
7. **No ambient I/O in core** — future capabilities flow through an explicit
   `AppHost` broker; VelquView works standalone with none.

Details: [`docs/architecture.md`](docs/architecture.md),
decisions in [`docs/decisions/`](docs/decisions/README.md).

## Source of truth

The product/architecture specification lives in [`docs/okf/`](docs/okf/)
(the OKF bundle this repository was seeded from). Implementation evidence
lives in [`docs/evidence/`](docs/evidence/).

## Licensing

Code: `MIT OR Apache-2.0` at your option — see [LICENSE-MIT](LICENSE-MIT)
and [LICENSE-APACHE](LICENSE-APACHE). Bundled fonts are DejaVu
(`crates/velqu-view/assets/fonts/LICENSE-dejavu.txt`).
