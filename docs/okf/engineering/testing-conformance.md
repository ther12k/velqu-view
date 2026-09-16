---
type: Engineering Plan
title: Testing and Conformance
description: Test strategy for renderer correctness, Tailwind compatibility, reactive semantics, and input.
tags: [velqu-view, testing, conformance]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Test Layers

## Unit

State scope, dependency tracking, expression evaluation, event modifiers, CSS compatibility classification, DOM mutation.

## Render Fixtures

Each fixture contains input HTML, compiled CSS, viewport/DPI, expected layout facts, and a reference image where useful.

# Tailwind Conformance Corpus

```text
layout/
flex/
grid/
spacing/
sizing/
position/
overflow/
typography/
colors/
borders/
shadows/
transforms/
hover-focus/
responsive/
dark/
svg-icons/
forms/
```

# Application Fixtures

```text
dashboard/
settings/
kanban/
chat-shell/
mini-ide/
```

# Visual Comparison

Browser reference screenshots can be an engineering oracle where useful, but arbitrary browser pixel parity is not the product contract.

# Input Matrix

```text
mouse click
hover
wheel
keyboard
Tab/Shift+Tab
selection
copy/paste
IME composition
resize during input
DPI change if testable
```

# Reactive Conformance

For each directive specify initial state, event, expected state, expected DOM mutation, and expected invalidation.

# CI

Run parser/reactive/unit tests broadly, offscreen rendering where supported, and platform-specific interactive lanes incrementally.
