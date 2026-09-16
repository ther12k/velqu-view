---
type: Specification
title: Velqu Tailwind/CSS Profile v0
description: Tier-1 CSS capabilities required for Tailwind-first application UIs.
tags: [velqu-view, tailwind, css, profile]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Contract

VelquView supports **compiled Tailwind CSS within the Velqu CSS Profile**, not Tailwind utility names directly.

# Pipeline

```text
Tailwind classes
    ↓
Tailwind compiler
    ↓
CSS
    ↓
Velqu CSS check/normalization
    ↓
VelquView renderer
```

# Tier-1 Capabilities

## Layout

```text
display: block/inline/none
flex
grid
position: relative/absolute
gap
alignment
width/height
min/max size
margin
padding
```

## Visual

```text
background colors
text colors
border
border-radius
box-shadow
opacity
overflow/clipping
```

## Typography

```text
font family
font size
font weight
line height
letter spacing
text alignment
text overflow
white-space handling required by fixtures
```

## State

```text
:hover :focus :active :disabled
```

## Theme/Responsive

```text
CSS custom properties
dark-mode strategy used by reference app
media-query breakpoints required by Tailwind fixtures
```

## Transform

Initial: translate, scale, rotate.

# Deferred/Experimental

```text
position: sticky
advanced filters/backdrop filters
blend modes
CSS masks
complex 3D transforms
container queries
print CSS
advanced browser-specific appearance
```

# Compatibility Tooling

`velqu css check` should report supported, normalized, unsupported, source location, and suggested replacement when known.

# Rule

If an unsupported Tailwind construct is important to normal application UI, prefer fixing the renderer/profile rather than forcing hand-rewrites in every app.
