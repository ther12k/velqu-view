---
type: Engineering Plan
title: VelquView Implementation Plan
description: Evidence-first implementation sequence from window spike to benchmarked local app runtime.
tags: [velqu-view, implementation, roadmap]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Phase 0 - Workspace and Evidence Harness

Rust workspace, CI, fixture convention, measurement scripts, retained benchmark result format.

# Phase 1 - Native Window and Paint

Prove Winit window, renderer surface, text, resize, DPI, and deterministic frame capture.

# Phase 2 - HTML/CSS Renderer Spike

Behind `velqu-view` API: HTML load, style, block/flex/grid, text, backgrounds/borders, images, scrolling. Use the fastest credible integration path, potentially selected Blitz pieces.

# Phase 3 - Tailwind Profile

Add real Tailwind build, compatibility checker, dashboard corpus, screenshot comparisons.

# Phase 4 - Input Correctness

Mouse, focus, keyboard, editing, selection, clipboard, IME, scroll, disabled state.

# Phase 5 - Velqu Reactive v0

Add isolated frontend QuickJS expression runtime and only the frozen v0 directive/event/binding set.

# Phase 6 - VelquView Lab

Local app loader, file watching/reload, DOM/style/layout/reactive inspector, event trace, compatibility warnings, frame counters.

# Phase 7 - Reference Dashboard

Sidebar, header, cards, table, forms, dialog, tabs, dropdown, dark theme, responsive resize, icons.

# Phase 8 - Mini IDE Benchmark

File explorer, tabs, editable text, save mock/local action, find, command palette, shortcuts, theme.

No extensions, LSP, debugger, terminal, Git, or AI.

# Phase 9 - Matched Benchmark

Equivalent Electron, Tauri, and VelquView Mini IDE shells with retained measurements.

# Phase 10 - Decision

GO: stabilize API, design host capabilities, plan Velqu Desktop.

NO-GO: document evidence and stop or narrow the renderer project.
