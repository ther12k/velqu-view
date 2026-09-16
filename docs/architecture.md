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
| `velqu-view` | public renderer API, scene model, M1 CPU painter, bundled fonts, `Frame`/capture, source identity, viewport validation, asset-resolution seam | windows, events, HTML semantics (M2), any I/O |
| `velqu-shell` | native window, event loop, DPI/resize, frame presentation | rendering decisions, document state |
| `velqu-reactive` | frozen vx-* syntax surface, static validation | any runtime (M5) |
| `velqu-tailwind` | CSS Profile v0 manifest, concept-level classification (declaration/at-rule, three tiers) | CSS parsing/engine work (M3) |
| `velqu-lab` (app) | loading local app dirs (with source identity + directory asset resolver), window preview, headless fixture capture | — |

Dependency direction: `velqu-lab → velqu-shell → velqu-view`.
`velqu-reactive` and `velqu-tailwind` are leaves; nothing depends on
windowing or GPU crates except `velqu-shell`.

## Render pipeline (M2a)

```
DocumentSource / StylesheetSource          sources carry identity (SourceId);
                                           stylesheets upsert by id (ADR 0004)
        ↓
html5ever → dom::Dom                       spec-correct tokenization/tree building
                                           lowered into the small Velqu tree;
                                           data-vv-test extracted as fixture identity
        ↓
cssparser → css::Stylesheet                rules, selectors + specificity,
                                           declarations + source lines; profile
                                           diagnostics for anything skipped
        ↓
style::Cascade                             UA defaults → author sheets → inline;
                                           inheritance; per-property diagnostics
        ↓
layout.rs                                  box tree → block flow → wrapped lines;
                                           LayoutFacts v1 (data-vv-test keyed) and
                                           a DisplayList (fills + text runs)
        ↓
painter.rs                                 rasterizes the display list — no layout
                                           decisions; fallible, pixel-bounded alloc
        ↓
Frame                                       pixels + sha256 + PNG encode
        ↓
velqu-shell                                 winit window; softbuffer blit
```

Identity is separated per stage (ADR 0005): DOM `NodeId`s never reach
fixtures (which use `data-vv-test`) and the painter never sees the DOM.

`Viewport` is validated at construction (`try_new`: non-zero dimensions,
finite positive scale, `width*height <= MAX_PIXELS`), so layout and paint
consume a target whose invariants cannot be violated.

Rendering is fully offscreen and deterministic: identical state + viewport
produce identical bytes. A window is only one possible sink for a `Frame`.
This is what makes the fixtures (`tests/visual/…`) run in CI with no
display server (`docs/decisions/0002-offscreen-determinism.md`).

## Renderer strategy

Phase A (done): proved the pipeline with minimal, replaceable pieces —
winit + softbuffer + fontdue — behind the Velqu-owned API
(`docs/decisions/0001-m1-paint-backend.md`).

Phase B (M2a done, M2b/M2c next): HTML parsing (html5ever), CSS syntax
(cssparser), cascade, and **block layout** now run behind the same API
(ADR 0005/0006). Flex and grid are the next layout milestones; **Taffy is
the adopted candidate** (its 0.14 line implements block/flex/grid and its
MSRV fits the 1.87 floor — verified). Text shaping (Parley candidate) is
deferred; the current deterministic Latin subset is documented in
ADR 0006. None of the engine types may appear in the public API
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

Host capabilities: core VelquView has no ambient external I/O — this already
applies to assets: documents declare an opaque `base`, and relative
references resolve through a host-installed `AssetResolver`
(`velqu-lab` installs a directory-scoped one; the default resolves
nothing). A future broader `AppHost` trait (`supports` / `request`) will
broker network, filesystem, clipboard, dialogs, etc. Not stabilized until
after the renderer POC.

## Non-goals (enforced)

No iframes, Service Workers, browser navigation/history, extensions,
WebRTC/WebBluetooth/WebUSB/WebXR, browser localStorage semantics, arbitrary
browser JS, WebGL/WebGPU web API, or general Canvas compatibility. See
`docs/okf/product/non-goals.md`.
