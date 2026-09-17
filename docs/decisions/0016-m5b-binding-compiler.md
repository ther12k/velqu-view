# ADR 0016: M5b — the reactive binding compiler; a capability-limited execution plan

Status: accepted (M5b)

## Context

M5a froze the bounded QuickJS runtime (per-generation isolation, hard
budgets, deterministic clock/random, no ambient I/O). The reviewer's
direction for the next slice: keep the compiler **deliberately boring
and deterministic** — pure Rust, no QuickJS execution, no fake DOM —
and prove that reactive markup lowers to a validated Rust-owned plan
while non-reactive documents remain behaviorally and byte-for-byte
unchanged. The architectural sentence to freeze:

> **Reactive markup is compiled into a capability-limited execution
> plan; runtime JavaScript never discovers or traverses the DOM.**

That rule is what keeps Velqu from gradually acquiring querySelector,
live node wrappers, mutation observers, and eventually a browser-shaped
JS environment.

## Decision

### 1. The DOM is seen through a one-way trait, and only by the compiler

`ReactiveDom` is a minimal read-only surface (root, element children in
order, tag, attributes as written). `velqu-view` implements it for its
parsed document; the compiler walks it and nothing else. JS never sees
nodes, `NodeId`s, handles, or this trait — the plan's node references
are internal ids that cross into public API only as opaque,
generation-checked `ReactiveBindingId`s resolving to `ElementTarget`
(the same identity discipline as ADR 0012).

### 2. The plan is Rust-owned data

`ReactiveDocument { scopes, bindings, events, diagnostics }`:

* `ScopePlan` — a `vx-state` node, its parent scope index (explicit
  parentage; nested scopes chain), and the initializer source verbatim.
* `Binding` — node, lexical scope (nearest enclosing `vx-state`), a
  typed `BindingKind` (Text/Show/Class/Style/Value/Disabled/Checked/
  Model), the expression source verbatim, and a deterministic
  `SourceSpan` (preorder node ordinal + attribute name; byte-accurate
  parser spans are deliberately deferred).
* `EventBinding` — node, scope, the parsed `@event.modifiers` name,
  the handler source verbatim, and its span.

Compilation is a pure function of the DOM: identical documents compile
to identical plans (test-pinned). No code executes during compilation.

### 3. The frozen runtime surface — smaller than the syntax surface

The M0 name sets are the *syntax* surface; M5b lowers this runtime
subset: `vx-state`, `vx-text`, `vx-show`, `vx-model`, the five `:attr`
bindings, and the six `@event` handlers. `vx-if`/`vx-for`/`vx-key`/
`vx-computed` are known names whose dynamic-tree semantics are
**deferred with a distinct diagnostic** — never silently treated as
ordinary HTML, never half-implemented. No `vx-for`, no template
cloning, no arbitrary node insertion/removal in this slice; no refs,
watchers, lifecycle hooks, timers, or async helpers are added.

### 4. Deterministic validation, never silent

* Unknown `vx-*`/`@…`/`:…` → diagnostic (via the frozen M0 parsers).
* Duplicate reactive attributes → diagnostic, **first occurrence wins**
  (deterministic regardless of how the DOM was built).
* `vx-model` + `:value` on one element → conflict diagnostic,
  **vx-model wins**, the value binding is dropped.
* Semantic compatibility at compile time, not runtime: `vx-model` only
  on `<input>`/`<textarea>` (the M4c1 profile); `vx-text` not on
  controls; `:value`/`:checked` only on controls; form events
  (`@input`/`@change`/`@submit`) only on form elements.
* Malformed expression sources (empty, unbalanced delimiters,
  unterminated strings, NUL) → a diagnostic tied to node and attribute.
  This is a cheap shape check, not a JS parse; QuickJS remains the
  semantic gate when M5c evaluates — malformed-by-construction markup
  fails at compile time instead of surprising the runtime.
* Bindings/handlers outside any `vx-state` scope → diagnostic.

### 5. One generation, one plan

The plan is compiled at `enable_reactive` and on every document load
while enabled, stamped with the document generation. Reload replaces it
wholesale — the sanctioned reset, exactly like scroll/hover/focus/
control state — and `ReactiveBindingId`s from the old generation
resolve to nothing (generation-checked, like `ElementHandle`).

### 6. Static documents take the exact M4 path

Reactive is opt-in (`enable_reactive`, mirroring Tailwind). Compilation
is a pure read: it touches no rendering path, so documents with zero
reactive markup produce an empty plan and byte-identical facts and
raster — and because reactive attributes are inert to styling and
layout (unknown attributes never enter the cascade), documents *with*
reactive markup also render byte-identically whether or not the plan
was compiled. Both are test-pinned.

## Consequences

* M5c's runtime receives exactly this plan: evaluate expressions
  against scope state, collect a validated mutation batch, commit
  through the existing invalidation model. The JS side gains no DOM
  vocabulary — it evaluates `state + expression + event data` and
  returns results; Rust knows which node each binding belongs to.
* AI-generated Velqu UI has one sanctioned update path (the plan) to
  reason about, rather than arbitrary imperative mutation.
* The `keydown`/`keyup` handler names compile but have no M4 input
  event to dispatch yet; M5c wires what the input gate emits
  (click/input/change via M4 events) and documents the rest — they
  fire never, not wrongly.
* Deferred parser spans mean diagnostics identify markup by node
  ordinal + attribute name; byte offsets can arrive later without
  changing plan shape.
