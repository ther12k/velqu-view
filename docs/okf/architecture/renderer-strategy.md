---
type: Architecture Decision
title: Renderer Strategy
description: Use a Velqu-owned API with a fast backend spike before committing to direct engine integration.
tags: [velqu-view, renderer, blitz, decision]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Decision

Own the **VelquView API and contracts immediately**. Do not initially own every renderer integration layer.

# Phase A - Fast Feasibility Backend

Use selected Blitz components or Blitz as a reference/backend behind a private adapter to prove Tailwind rendering, text, input, scroll, resize, memory, startup, and paint invalidation.

The application must never import Blitz APIs.

# Phase B - Evidence Review

After the benchmark, decide among:

1. keep Blitz-backed integration;
2. keep some Blitz crates and replace specific layers;
3. integrate Stylo/Taffy/Parley directly;
4. stop the project.

# Why Not Fork First

A full fork creates immediate maintenance obligations while the product thesis is unproven.

# Why Not Build Everything Directly First

The hard work is integration correctness: invalidation, intrinsic sizing, inline layout, text shaping, font fallback, scrolling, selection, IME, focus, hit testing, accessibility, paint invalidation, DPI, and platform input.

# Public Abstraction

Conceptual:

```rust
pub struct VelquView { /* private backend */ }

impl VelquView {
    pub fn load_html(&mut self, html: &str) -> Result<()>;
    pub fn load_css(&mut self, css: &str) -> Result<()>;
    pub fn dispatch_input(&mut self, input: InputEvent);
    pub fn frame(&mut self) -> FrameResult;
}
```
