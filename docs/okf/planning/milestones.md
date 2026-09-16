---
type: Roadmap
title: VelquView Milestones
description: Evidence-gated project roadmap for the POC and post-POC decision.
tags: [velqu-view, milestones, planning]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# V0 Roadmap

## M0 - Bootstrap

Repository, workspace, CI, decision log, fixture convention, evidence format.

Exit: reproducible local and CI build.

## M1 - Window / Paint

Native window, renderer surface, text/rectangle, resize/DPI, screenshot capture.

Exit: deterministic Hello fixture.

## M2 - HTML/CSS

HTML tree, style, block/flex/grid, text, borders/backgrounds, images, scrolling.

Exit: static application shell renders.

## M3 - Tailwind Profile

Tailwind build, compatibility checker v0, dashboard fixture, SVG icon strategy.

Exit: dashboard usable without one-off per-component patches.

## M4 - Input Gate

Pointer, keyboard, focus, editing, selection, clipboard, IME, scrolling.

Exit: forms/input fixture passes.

## M5 - Velqu Reactive

UI QuickJS runtime, v0 directives/events/bindings, state inspector.

Exit: counter/forms/tabs examples pass reactive conformance.

## M6 - VelquView Lab

Local app runner, reload, inspector, diagnostics.

Exit: layout/state failures can be diagnosed without browser tooling.

## M7 - Real Dashboard

Full reference dashboard.

Exit: Tailwind Profile v0 frozen for benchmark.

## M8 - Mini IDE

File tree, tabs, editable text, command palette, find, shortcuts, theme.

Exit: benchmark app is usable.

## M9 - Comparative Benchmark

Matched Electron/Tauri/VelquView runs.

Exit: retained evidence package.

## M10 - Decision

GO: stabilize API, design host capability layer, plan Velqu Desktop.

NO-GO: retain findings and freeze or narrow the project.
