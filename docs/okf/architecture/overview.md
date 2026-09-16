---
type: Architecture
title: VelquView Architecture Overview
description: High-level architecture for the native renderer, reactive runtime, and host boundary.
tags: [velqu-view, architecture]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# System

```text
                    Application

             HTML + Tailwind + vx-*
                       │
                       ▼
                  VelquView
                       │
        ┌──────────────┼──────────────┐
        │              │              │
      DOM/CSS       Interaction    Reactive
        │              │              │
        │           Rust hot path   UI QuickJS
        │                             expressions
        └──────────────┬──────────────┘
                       │
                   Paint/Frame
                       │
                   Native window

                       │
                optional AppHost
                       │
          ┌────────────┴────────────┐
          │                         │
     Remote API host          future Desktop host
                                     │
                                 Velqu Core
                                     │
                              backend QuickJS
```

# Likely Low-level Building Blocks

```text
HTML parsing      html5ever or backend-equivalent
CSS/cascade       Stylo
layout            Taffy
text              Parley
paint             AnyRender/Vello or equivalent
GPU               wgpu
window            Winit
accessibility     AccessKit
```

These are implementation details, never public application types.

# Runtime Roles

## VelquView Rust runtime

Owns DOM/tree, style invalidation, layout, hit testing, focus, scrolling, text input, selection, IME, repaint scheduling, and state-to-view bindings.

## Velqu Reactive UI QuickJS

Owns expression evaluation, local state mutation, and small computed values. It is not a browser runtime.

## AppHost

Owns optional external capabilities. VelquView remains useful with no AppHost.

# Platform Split

```text
Velqu Server   → native Velqu Core
Velqu Desktop  → native Velqu Core + VelquView
Velqu Web      → real browser + Velqu WASM
```
