---
type: Example
title: Tailwind Dashboard Fixture
description: Reference application for Tailwind layout, state styles, forms, icons, and responsive behavior.
tags: [velqu-view, example, tailwind, dashboard]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Purpose

The dashboard fixture is the primary realistic CSS/rendering corpus before the Mini IDE.

# Required UI

```text
sidebar
top bar
metric cards
data table
search input
select
checkbox
dialog
tabs
dropdown
toast/status area
dark/light tokens
SVG icons
responsive resize
nested scroll region
```

# State

Use Velqu Reactive only for local state: sidebar open, active tab, dialog open, search field, and selected rows.

# Acceptance

* no overlapping controls;
* expected Flex/Grid structure;
* icons inherit expected size/color;
* focus/hover states visible;
* forms accept text correctly;
* scrolling stable;
* resize leaves no stale paint;
* memory stable after repeated local interactions.
