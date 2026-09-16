---
type: Engineering Plan
title: Repository Skeleton
description: Proposed initial and future workspace layout for github.com/ther12k/velqu-view.
tags: [velqu-view, repository, rust, skeleton]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Repository Decision

Create:

```text
github.com/ther12k/velqu-view
```

Do not create `velqu-desktop` yet.

# Initial Workspace

```text
velqu-view/
├── Cargo.toml
├── README.md
├── crates/
│   ├── velqu-view/
│   ├── velqu-reactive/
│   ├── velqu-tailwind/
│   └── velqu-shell/
├── apps/
│   └── velqu-lab/
├── examples/
│   ├── hello/
│   ├── counter/
│   ├── forms/
│   ├── dashboard/
│   └── mini-ide/
├── tests/
│   ├── visual/
│   ├── tailwind/
│   ├── reactive/
│   └── input/
└── docs/
```

# Crate Responsibilities

## `velqu-view`

Public renderer API and, initially, most internal DOM/style/layout/paint glue. Do not prematurely split every subsystem.

## `velqu-reactive`

Owns `vx-*` parsing, binding graph, state scopes, UI QuickJS context, expression evaluation, and reactive invalidation.

## `velqu-tailwind`

Owns compatibility profile, CSS validation/normalization, diagnostics, and fixture metadata. It must not become a second CSS engine.

## `velqu-shell`

Owns native window, event loop, DPI, platform input, clipboard hooks needed by core UI, and render-surface lifecycle.

## `velqu-lab`

Development app runner and inspector.

# Future Extraction

Only after boundaries become real:

```text
velqu-dom
velqu-html
velqu-style
velqu-layout
velqu-text
velqu-paint
velqu-host
```

# Dependency Rule

Application code imports VelquView APIs only. No public API may require Blitz/Taffy/Stylo/Parley types.

# Later Repository Topology

After GO:

```text
ther12k/velqu
ther12k/velqu-view
ther12k/velqu-desktop
```
