---
type: Product Specification
title: VelquView MVP
description: The minimum POC required to prove or reject the VelquView product thesis.
tags: [velqu-view, mvp, poc]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# MVP Objective

Prove that a restricted native renderer can support serious Tailwind application UI, local reactive interactions, and desktop-quality input without embedding Chromium/WebView2.

# Required Rendering

* native window and DPI-aware resize;
* HTML tree loading;
* CSS cascade;
* block, Flexbox, and Grid;
* typography/wrapping;
* CSS variables;
* backgrounds, borders, radius, shadows, opacity;
* clipping and scrolling;
* images and SVG icon support sufficient for fixtures.

# Required Interaction

* hit testing and hover;
* click;
* focus;
* keyboard navigation;
* editable text;
* selection;
* clipboard;
* IME composition;
* disabled controls;
* wheel/trackpad scrolling.

# Required Velqu Reactive v0

```text
vx-state
vx-text
vx-show
vx-model
@click
@input
:class
:value
:disabled
```

Stretch:

```text
vx-computed
vx-if
vx-for + vx-key
event modifiers
transitions
```

# Required Development Host

`velqu-lab` must run a local app directory and expose reload, DOM inspection, style/layout details, reactive state, event logs, compatibility warnings, and basic frame diagnostics.

# Reference Apps

1. Hello/layout fixture.
2. Counter/reactive fixture.
3. Forms/input fixture.
4. Tailwind dashboard.
5. Mini IDE shell.

# Explicit MVP Non-goals

* Velqu Core integration is not required to prove the renderer.
* No WASM in VelquView.
* No arbitrary `<script>` execution.
* No browser DOM compatibility.
* No public native widget SDK.

# Exit Criteria

**GO** if UI correctness is acceptable and benchmark evidence shows a material reason to continue.

**NO-GO** if ordinary Tailwind UI needs pervasive one-off hacks, input correctness remains unreliable, or the resource improvement is too small to justify renderer maintenance.
