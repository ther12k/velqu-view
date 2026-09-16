# VelquView Architecture

Short form. The authoritative spec set is the OKF bundle in `docs/okf/`;
this file maps it onto what exists in the repository today.

## System

```
            Application

     HTML + Tailwind + vx-*
                 │
                 ▼
            VelquView (crates/velqu-view)
                 │
    ┌────────────┼────────────────┐
    │            │                │
 document/    interaction     reactive layer
 styling      (Rust hot path)  (M5: isolated UI QuickJS,
 (M2)                          expressions + state only)
    │            │                │
    └────────────┼────────────────┘
                 ▼
         paint (offscreen Frame)
                 │
        velqu-shell: native window
        (winit + softbuffer presentation)
                 │
          optional AppHost (future)
```

## Crate map (current)

| Crate | Owns | Does not own |
|---|---|---|
| `velqu-view` | public renderer API, scene model, M1 CPU painter, bundled fonts, `Frame`/capture | windows, events, HTML semantics (M2) |
| `velqu-shell` | native window, event loop, DPI/resize, frame presentation | rendering decisions, document state |
| `velqu-reactive` | frozen vx-* syntax surface, static validation | any runtime (M5) |
| `velqu-tailwind` | CSS Profile v0 manifest, classification | CSS parsing/engine work (M3) |
| `velqu-lab` (app) | loading local app dirs, window preview, headless fixture capture | — |

Dependency direction: `velqu-lab → velqu-shell → velqu-view`.
`velqu-reactive` and `velqu-tailwind` are leaves; nothing depends on
windowing or GPU crates except `velqu-shell`.

## M1 paint pipeline (what runs today)

```
VelquView::load_html / load_css      sources retained + validated
        ↓
probe scene (src/probe.rs)           explicit debug scene: rects, outline,
                                     text in two weights, viewport/DPI status,
                                     document size echo
        ↓
painter (src/painter.rs)             CPU raster into an RGBA8 buffer;
                                     logical→device = round(v * scale);
                                     solid rects, fontdue glyphs (scalar path)
        ↓
Frame                                 pixels + sha256 + PNG encode
        ↓
velqu-shell                           winit window; softbuffer blit
                                      (native-endian 0x00RRGGBB)
```

Rendering is fully offscreen and deterministic: identical state + viewport
produce identical bytes. A window is only one possible sink for a `Frame`.
This is what makes the M1 fixture (`tests/visual/hello`) run in CI with no
display server (`docs/decisions/0002-offscreen-determinism.md`).

## Renderer strategy

Phase A (now): prove the pipeline with minimal, replaceable pieces —
winit + softbuffer + fontdue — behind the Velqu-owned API
(`docs/decisions/0001-m1-paint-backend.md`).

Phase B (M2): build HTML parsing, cascade, and layout (block/flex/grid)
behind the same API. Candidate engines (html5ever, Stylo, Taffy, Parley,
Vello/AnyRender, AccessKit) may be adopted when milestone evidence
justifies them; selected Blitz components are explicitly allowed as a
feasibility shortcut. None of their types may appear in the public API
(`docs/decisions/0003-api-boundary.md`).

Phase C (M9/M10): comparative benchmark against matched Electron and
Tauri/system-webview apps; GO/NO-GO on Velqu Desktop.

## Runtime boundaries

Always-native hot paths (never routed through QuickJS): pointer movement,
scrolling, caret, selection, IME, focus, hit testing, dragging, resize,
animation-frame scheduling, layout, paint.

Velqu Reactive (M5) gets an isolated frontend QuickJS context that evaluates
expressions against reactive state only. It receives no `document`, `window`,
`navigator`, storage, network, or filesystem, and never mutates the DOM
directly — Rust applies state-driven changes:

```
QuickJS expression → reactive state → Rust binding engine → Velqu DOM → layout/paint
```

Host capabilities: core VelquView has no ambient external I/O. A future
`AppHost` trait (`supports` / `request`) will broker network, filesystem,
clipboard, dialogs, etc. Not stabilized until after the renderer POC.

## Non-goals (enforced)

No iframes, Service Workers, browser navigation/history, extensions,
WebRTC/WebBluetooth/WebUSB/WebXR, browser localStorage semantics, arbitrary
browser JS, WebGL/WebGPU web API, or general Canvas compatibility. See
`docs/okf/product/non-goals.md`.
