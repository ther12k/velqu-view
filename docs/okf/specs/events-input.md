---
type: Specification
title: Events and Input
description: Input correctness and hot-path rules for VelquView.
tags: [velqu-view, input, events, ime]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Priority

Input correctness is a renderer gate, not polish.

# Required Input

Pointer: move, down, up, click, wheel, hover, drag primitives.

Keyboard: key down/up, modifiers, tab focus traversal, shortcut recognition.

Text: insertion, selection, caret, clipboard, IME composition.

Focus: focus, blur, disabled handling, focus order.

# Hot-path Rule

These stay in Rust:

```text
scroll
pointer hit testing
caret movement
selection
IME
dragging
resizing
animation-frame scheduling
layout
paint
```

# Event Dispatch

```text
platform input
    ↓
hit test
    ↓
DOM event target
    ↓
native default behavior
    ↓
optional Velqu Reactive handler
    ↓
state invalidation
    ↓
layout/paint if necessary
```

# IME

The MVP must include an explicit IME test plan. A renderer that cannot handle composition correctly is not ready for desktop application use.
