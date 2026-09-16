---
type: Engineering Specification
title: VelquView Lab
description: Standalone local app runner and renderer-development shell.
tags: [velqu-view, devtools, lab]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Purpose

VelquView Lab is the first executable product from the repository. It is not a general web browser.

# Launch

```bash
velqu-lab ./examples/dashboard
```

# Conceptual UI

```text
┌──────────────────────────────────────────────────────┐
│ VelquView Lab      Reload   Inspect   FPS / Memory  │
├──────────────────────────────────────────────────────┤
│                                                      │
│                  rendered application                │
│                                                      │
├──────────────────────────────────────────────────────┤
│ DOM | Style | Layout | Reactive | Events | Paint    │
└──────────────────────────────────────────────────────┘
```

# Inspector v0

DOM tree and attributes.

Computed style properties relevant to the profile.

Layout: x, y, width, height, content/padding/border, overflow.

Reactive: scope, state values, computed values, binding dependencies, last changed value.

Events: recent semantic events and targets.

Paint: frame count and dirty/repaint information available from the backend.

# Reload

Full document reload is acceptable first. CSS-only/reactive/incremental reload can come later.

# Safety

No public URL bar. The app opens local VelquView packages/directories, not arbitrary internet pages.
