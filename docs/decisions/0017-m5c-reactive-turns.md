# ADR 0017: M5c — reactive turns; atomic state + UI commits

Status: accepted (M5c)

## Context

M5a froze the bounded QuickJS runtime; M5b froze the binding compiler
("reactive markup is compiled into a capability-limited execution plan;
runtime JavaScript never discovers or traverses the DOM"). M5c closes
the loop: M4 events drive **turns**. The reviewer's rule to lock before
implementation:

> A reactive turn is atomic across both reactive state and rendered
> mutations. If a handler changes `count`, then a binding throws or the
> resulting mutation batch is invalid, don't leave `count` changed while
> the screen still shows the previous value.

## Decision

### 1. State is plain data; the machine owns the only commit path

`ReactiveValue` is null/bool/number/string/array/plain-object. Capture
from JavaScript rejects functions, symbols, bigints, exotic prototypes
(via a `isPlain` prototype check), nesting beyond 32 levels (the cycle
backstop), and strings over the output budget. Committed state lives
Rust-side; each turn materializes a fresh candidate object from it. A
failed turn simply never captures the candidate — rollback without
heap snapshots. v0 uses one shared state namespace per document (all
scopes merge into it, initializers in document order); per-scope
lexical shadowing is deferred with the plan's parentage already
recorded.

### 2. The turn, in order

```
committed state ─ snapshot ─> candidate JS object
event payload (frozen)  +  vx-model write (BEFORE handlers)
handlers (plan order, bounded)
bounded microtask drain          <- runtime-level phase
capture candidate (constrained)
re-evaluate ALL bindings ─> normalize (String()/ToBoolean via helpers)
diff vs applied baseline ─> MutationBatch
validate batch (whole-batch, host-side)
COMMIT: candidate → committed; outputs → applied; .once registers
        mutations → document/control appliers
```

Any failure anywhere — handler throws, binding throws, deadline,
job budget, invalid candidate, oversized output, mutation-limit —
discards the candidate and the batch, records a bounded diagnostic,
and leaves the previous UI untouched. No reentrancy: one queued M4
event = one turn; `pump_reactive` processes the queue in order and
never enters QuickJS recursively.

### 3. Units compile once, evaluated through a `with` scope

Host compilation (the privileged path M5a.1 kept) wraps each binding
expression and handler **once per generation** into a parenthesized
arrow — an expression cannot smuggle statements past its arrow body —
evaluated in sloppy mode with `with (__velquState)`: bare-identifier
reads *and writes* hit state properties (Alpine-style authoring:
`count = count + 1`). Each generated wrapper obeys `max_source_bytes`.
Trade-off recorded: sloppy mode means writes to keys absent from state
fall through to the global object and are not captured; the state
profile (plain data) is enforced on capture, which is where it matters.
Event payloads are `Object.freeze`d, so a handler writing
`__velquEvent.value` is silently contained.

### 4. vx-model ordering, both directions, pinned

* User edit → M4 `ValueChanged` → the model's state path is written
  **before** the `@input` handlers run → the handler sees the new value
  → sibling bindings commit in the same turn.
* State → model binding output → `SetControlValue` applies **silently**:
  control runtime state only, never a synthesized user `ValueChanged`.
  No feedback loops.
* `vx-model` expressions must be dotted state paths; anything else is
  a diagnostic and the write is skipped.

### 5. Semantic mutations; the renderer classifies invalidation

The machine emits `SetText/SetVisible/SetClass/SetStyle/
SetControlValue/SetControlDisabled/SetControlChecked` — targets are
plan binding indices (generation-scoped through the machine). The view
validates the **whole batch** against the current DOM before applying
anything: stale/unknown targets, non-elements, or control mutations on
non-controls reject the batch (and with it the turn). Application
routes by kind: text/class/style changes and visibility flips mark the
document structural (next render = one cascade+Taffy pass); control
value/flag writes are presentation-only and never emit events.
QuickJS never decides invalidation.

### 6. Event payloads and the v0 event set

Payloads are host-built plain objects (`type`, `value`, `id`), frozen
deep, byte-budgeted (`max_event_payload_bytes`) before construction.
The frozen driving set for v0: `@click` and `@input` (from M4
Click/ValueChanged); `@keydown`/`@keyup`/`@submit` compile but have no
M4 event source — they fire never, not wrongly. Propagation is target →
ancestors (target first, no capture phase); `.stop` ends the walk,
`.once` is consumed only by a committed turn (a rolled-back turn does
not consume it); `.prevent` is a no-op in a renderer with no default
actions to prevent.

### 7. Reload = the sanctioned reset

The machine is built per document generation and stored with the plan;
`load_html` destroys it wholesale — state, compiled units, queued
jobs, and `.once` registrations die with the old generation. A static
document (no reactive markup) pumps as a no-op and renders
byte-identically with or without reactive enabled.

## Consequences

* M5d's job is now purely efficiency: presentation-only turns already
  avoid Taffy via the M4b repaint path (control writes), structural
  turns already cost ≤ 1 pass; proving and instrumenting that remains.
* Text/class/style mutation adds a small, sanctioned DOM-mutation
  surface (`set_text`/`set_attribute`), host-internal only — the DOM
  stays invisible to JavaScript.
* `button`/`select` became block-level in the UA sheet: reactive
  documents interactive-by-construction need hit-testable controls,
  and the counter example already assumed it.
* One scope namespace in v0 means two sibling scopes writing the same
  key merge last-writer-wins; the deferred lexical-showering semantics
  have their plan-side foundation (scope parentage) already in place.
