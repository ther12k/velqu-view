---
type: Architecture
title: Runtime Boundaries
description: Rules separating renderer mechanics, local reactive state, host I/O, and application logic.
tags: [velqu-view, runtime, boundaries]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Principle

Use the cheapest correct execution layer for each class of work.

# Always Rust / Native Hot Path

```text
pointer movement
scrolling
caret movement
selection
IME composition
focus
hit testing
dragging
splitter movement
animation frame scheduling
layout
paint
```

These events must not round-trip through QuickJS.

# Local Application-facing UI State

Velqu Reactive handles open/closed state, selected tabs, small counters, form state, local filters, conditional classes, and simple computed state.

```html
<div vx-state="{ tab: 'editor' }">
  <button @click="tab = 'editor'">Editor</button>
  <button @click="tab = 'preview'">Preview</button>
  <section vx-show="tab === 'editor'">...</section>
  <section vx-show="tab === 'preview'">...</section>
</div>
```

# External/Application Work

Filesystem, database, remote HTTP, process execution, system dialogs, notifications, and business rules are not assumed by core VelquView. They flow through an explicit host capability.

# Future Velqu Desktop

```text
hx-* / action
    ↓
Velqu Desktop host
    ↓
direct Velqu Core dispatch
    ↓
backend QuickJS
```

No localhost transport is required.

# Security Boundary

Frontend QuickJS is an expression runtime, not a hostile-code sandbox. It receives no ambient OS capabilities. Host capabilities are explicit and permission-scoped.
