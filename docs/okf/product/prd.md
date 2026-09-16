---
type: Product Requirements Document
title: VelquView PRD
description: Requirements for a native Tailwind-first local HTML application renderer and reactive UI runtime.
tags: [velqu-view, prd, desktop, tailwind, reactive]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# 1. Product Summary

**VelquView** is a native application UI runtime that renders local HTML and CSS, with first-class support for Tailwind-generated CSS and a small Alpine-inspired reactive layer called **Velqu Reactive**.

VelquView is **not a web browser**. It does not aim to load arbitrary websites or implement the complete Web Platform. HTML and CSS are used as a familiar authoring format for native application interfaces.

The initial standalone executable is **VelquView Lab**, a local app runner and development environment used to prove the renderer, reactive layer, input correctness, diagnostics, and performance before building Velqu Desktop.

# 2. Product Thesis

Developers already know HTML and Tailwind. Electron and Tauri make those skills useful for desktop development, but their UI rendering ultimately relies on browser/webview technology. VelquView tests a different trade-off:

> Keep the HTML/Tailwind authoring model, but replace the browser engine with a deliberately restricted native application renderer.

Expected benefits are hypotheses until measured:

* lower baseline memory;
* fast startup and first paint;
* direct control over input, invalidation, and painting;
* predictable Tailwind compatibility rather than arbitrary browser compatibility;
* an AI-agent-friendly UI language;
* an independent local UI runtime that can later be hosted by Velqu Desktop or connected to a remote API.

# 3. Core Product Invariants

1. **Native by default.** VelquView is native Rust. WASM is not an internal execution layer.
2. **WASM is for the real web target only.**
3. **Application UI, not websites.** No promise that arbitrary internet pages render.
4. **Tailwind-first, CSS-profile internally.** VelquView renders CSS; it does not hard-code Tailwind utility semantics.
5. **No mandatory frontend JavaScript.** Basic UI and standard interactions must work without script.
6. **Velqu Reactive is optional and constrained.** QuickJS evaluates UI expressions/state; it does not expose a browser DOM.
7. **External I/O is host-provided.** Filesystem, network, process execution, database access, clipboard, dialogs, and similar capabilities are outside the core renderer.
8. **Velqu Desktop is a future host.** VelquView must remain independently useful.
9. **Renderer backend details are private.** Apps never depend directly on Blitz, Stylo, Taffy, Parley, Vello, or another engine.
10. **Performance claims require retained evidence.**

# 4. Primary User

The first target user is a developer or AI coding agent building a local desktop-style application UI with:

```text
HTML
+ Tailwind
+ Velqu Reactive
+ optional host actions
```

The user should not need to learn a Rust UI DSL for ordinary interfaces.

# 5. Primary Use Cases

## 5.1 Standalone local UI

Examples: kiosk UI, calculator, local prototype, settings utility, locally reactive dashboard, or interactive design preview.

No server or Velqu Core is required.

## 5.2 API-backed local application

VelquView can be hosted with an explicit network capability and communicate with any remote backend. The backend does not have to be Velqu.

## 5.3 Future Velqu Desktop application

Velqu Desktop hosts VelquView and adds native capabilities plus direct integration with Velqu Core and backend QuickJS. No localhost server is required for local Velqu routes.

# 6. Developer Experience

```html
<div
  vx-state="{ count: 0 }"
  class="min-h-screen bg-zinc-950 p-8 text-zinc-100"
>
  <button
    @click="count++"
    class="rounded-lg bg-blue-600 px-4 py-2 font-medium hover:bg-blue-500"
  >
    Increment
  </button>

  <span class="ml-3" vx-text="count"></span>
</div>
```

Run locally:

```bash
velqu-lab ./my-app
```

The development host should provide reload, Tailwind compatibility diagnostics, DOM/style/layout inspection, reactive state inspection, event traces, and basic frame/repaint diagnostics.

# 7. Velqu Reactive

Velqu Reactive borrows Alpine's markup ergonomics but does not attempt Alpine runtime compatibility.

Initial surface:

```text
vx-state
vx-computed
vx-text
vx-show
vx-if
vx-model

@click
@input
@change
@submit
@keydown
@keyup

:class
:style
:value
:disabled
:checked
```

QuickJS is used as an expression engine for complex expressions.

The UI QuickJS runtime is isolated from backend QuickJS, receives no filesystem/process/network/browser APIs, and does not receive `document`, `window`, `navigator`, or DOM mutation APIs.

Simple bindings may later receive Rust-native fast paths.

# 8. Styling

Tailwind is the Tier-1 authoring framework.

```text
Tailwind source
    ↓
Tailwind compiler
    ↓
CSS
    ↓
Velqu compatibility validation/normalization
    ↓
VelquView
```

Plain CSS is supported when it fits the same Velqu CSS Profile.

# 9. Host Model

Core VelquView has no privileged external I/O.

A host may provide network, filesystem, clipboard, dialogs, database, process, notification, or system capabilities. Every capability is explicit and can be permission-scoped.

# 10. MVP Success Criteria

The MVP is successful when:

* a real Tailwind dashboard renders correctly enough to be useful;
* resize, scroll, pointer, keyboard, focus, editable text, and IME are reliable;
* Velqu Reactive drives local state without browser DOM APIs;
* a Mini IDE shell can be built using normal HTML/Tailwind plus standard text input;
* VelquView Lab exposes enough inspection data to debug layout and state;
* Tailwind Profile v0 is mechanically testable;
* matched Tauri/Electron benchmarks are reproducible;
* the measured advantage is large enough to justify renderer ownership.

# 11. Performance Hypotheses

No fixed public performance claim is made before measurement.

The POC should attempt to demonstrate materially lower whole-process memory, faster cold startup/first useful paint, near-zero idle CPU, competitive input/scroll latency, and predictable memory growth.

A small improvement is not sufficient justification for owning a renderer stack.

# 12. MVP Exclusions

Not MVP:

* general internet browsing;
* arbitrary browser JavaScript;
* npm/browser framework compatibility;
* React/Vue/Svelte runtime support;
* WebRTC/WebBluetooth/WebUSB/WebXR;
* Service Workers;
* browser navigation/history;
* browser extensions;
* full Canvas/WebGL/WebGPU web APIs;
* iframe compatibility;
* public native widget SDK;
* VS Code extension compatibility;
* LSP/debugger/terminal/Git/AI in the Mini IDE benchmark.

See [Non-goals](non-goals.md).

# 13. Delivery Strategy

Development starts in an independent `ther12k/velqu-view` repository.

```text
velqu-view ─────┐
                ├── velqu-desktop
velqu ──────────┘
```

`velqu-desktop` should not be created until VelquView passes the renderer and benchmark decision gates.

# 14. Product Decision

Proceed as an evidence-gated POC.

The real test is whether a restricted application renderer can deliver enough UI correctness and enough resource advantage over Tauri/Electron to justify long-term maintenance.
