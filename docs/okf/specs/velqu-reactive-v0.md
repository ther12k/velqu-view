---
type: Specification
title: Velqu Reactive v0
description: Alpine-inspired constrained reactive markup optimized for VelquView and AI-generated UI.
tags: [velqu-view, reactive, quickjs, alpine]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Goal

Provide Alpine-like local UI ergonomics without implementing a browser DOM.

# Principles

* familiar markup;
* small surface;
* deterministic behavior;
* static validation where possible;
* no ambient browser/OS APIs;
* Rust owns DOM and rendering;
* QuickJS evaluates expressions and mutates reactive state only.

# Runtime

```text
Velqu DOM
   │
binding graph
   │
UI QuickJS
   │
state/expression
```

QuickJS does not receive `document` or `window`.

# v0 Directives

## `vx-state`

```html
<div vx-state="{ open: false, count: 0 }">
```

## `vx-text`

```html
<span vx-text="count"></span>
```

## `vx-show`

```html
<div vx-show="open"></div>
```

## `vx-model`

```html
<input vx-model="query">
```

# v0 Events

```html
<button @click="count++"></button>
<input @input="query = $event.value">
<form @submit.prevent="submitted = true"></form>
```

Required events:

```text
click input change submit keydown keyup
```

Initial modifiers:

```text
prevent stop once enter escape
```

# v0 Attribute Bindings

```html
<button :disabled="loading"></button>
<div :class="{ 'opacity-50': loading }"></div>
<input :value="query">
```

Required:

```text
class style value disabled checked
```

# Expression Environment

Allowed concepts:

```text
local state
objects
arrays
strings
numbers
booleans
basic functions
$event
```

Not provided:

```text
document window navigator network filesystem process DOM mutation browser storage
```

# Optional v0.x

## `vx-computed`

```html
<div
  vx-state="{ query: '', loading: false }"
  vx-computed="{ canSearch: query.length > 2 && !loading }"
>
```

## `vx-if`

Conditional tree insertion/removal.

## `vx-for` + mandatory `vx-key`

```html
<template vx-for="item in items" vx-key="item.id">
  <div vx-text="item.name"></div>
</template>
```

# Native Fast Paths

The engine may optimize simple expressions such as `vx-show="open"` or `vx-text="count"` into Rust-native state bindings. Complex expressions may continue through QuickJS. This optimization is invisible to application code.

# AI-Agent Design Rules

1. One canonical syntax per feature where possible.
2. Unknown state variables are check errors.
3. Dynamic lists require stable keys.
4. Attribute ordering can be normalized by formatter.
5. Capabilities are machine-readable.
6. No hidden global browser state.
