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
 (M2–M3)                      expressions + state only)
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
| `velqu-view` | public renderer API, scene model, CPU painter, bundled fonts, `Frame`/capture, source identity, viewport validation, asset-resolution seam, input/interaction state, editable controls, transactional reload, inspector trace | windows, events, any I/O |
| `velqu-shell` | native window, event loop, DPI/resize, frame presentation, IME/clipboard wiring | rendering decisions, document state |
| `velqu-reactive` | vx-* syntax surface, plan compiler, isolated QuickJS runtime + reactive turn machine (budgeted, deterministic, plain-data state) | renderer mechanics, DOM access, script-side I/O |
| `velqu-tailwind` | Tailwind utility compilation to the Velqu CSS Profile (v0), classification, palette, profile checker | CSS parsing/engine work |
| `velqu-lab` (app) | loading local app dirs (with source identity + directory asset resolver), window preview, headless fixture capture, inspector print, `--watch` hot reload (coordinator + watcher + deadline scheduler) | — |

Dependency direction: `velqu-lab → velqu-shell → velqu-view`.
`velqu-reactive` and `velqu-tailwind` are leaves; nothing depends on
windowing or GPU crates except `velqu-shell`. Phase 1 (M1–M7) is
closed; see [`docs/evidence/phase1-closure.md`](evidence/phase1-closure.md)
and its linked post-closure corrections.

## Render pipeline

```
DocumentSource / StylesheetSource          sources carry identity (SourceId);
                                           stylesheets upsert by id (ADR 0004)
        ↓
velqu-tailwind (opt-in)                    Tailwind utilities compile to CSS
                                           under the v0 profile; diagnostics,
                                           never silent drops (ADR 0009)
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
layout.rs / taffy_backend.rs               box tree → Taffy whole-tree layout
                                           (block/flex/grid profiles, ADR 0007/
                                           0008) → display-list clipping and
                                           overflow; LayoutFacts (data-vv-test
                                           keyed)
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

Phase A (done, M1): proved the pipeline with minimal, replaceable
pieces — winit + softbuffer + fontdue — behind the Velqu-owned API
(`docs/decisions/0001-m1-paint-backend.md`).

Phase B (done, M2–M7 / Phase 1 closed): HTML parsing (html5ever), CSS
syntax (cssparser), cascade, **Taffy whole-tree layout** (block, flex,
and grid profiles behind a private backend — ADR 0007/0008), the
Tailwind utility pipeline (ADR 0009), the input/interaction stack
(ADR 0010–0014), Velqu Reactive (ADR 0015–0018), and the Lab dev loop
(inspector, transactional reload, file watching — ADR 0019–0022). Text
shaping remains the deterministic Latin subset documented in ADR 0006;
none of the engine types appear in the public API
(`docs/decisions/0003-api-boundary.md`). Deferred boundaries (variants,
dynamic lists, IME live-platform validation, and the recorded layout
deviations) are scoped in the Phase-1 closure record, not reopened
here.

Phase C (M9/M10, not started): comparative benchmark against matched
Electron and Tauri/system-webview apps; GO/NO-GO on Velqu Desktop.

## Development loop (M6)

`velqu-lab` adds the developer surfaces on top of the same public API:
an inspector that records outcomes (events, reactive turns,
invalidation causes, render passes — never rerunning work, ADR 0020),
transactional reload for both full documents and stylesheets (ADR 0021),
and host-side file watching (`--watch`, native or polling) reconciled
through the reload APIs (ADR 0022). A reload either publishes a fully
prepared replacement or leaves the running application untouched.

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
