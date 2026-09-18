# ADR 0019: M6a — event ownership; the pump takes a batch, not the queue

Status: accepted (M6a.1)

## Context

M5 froze reactive semantics. Its conformance run exposed that the
host integration choreography was too easy to misuse:

```
pump_reactive()   // peeks the internal queue
take_events()     // the caller must remember to drain
```

Neither the shell nor the lab drained — so every undrained
turn-driving event re-mapped to a fresh turn on each later redraw (a
click re-ran its handler once per frame), and the queue grew
unboundedly. The M5 fix patched the consumers. The reviewer froze M5
anyway but refused to freeze the choreography:

> There's a nastier future case too: existing events → pump → a
> reactive commit generates a new event → drain. If the drain is
> indiscriminate, the newly generated event can be removed before the
> next reactive turn/inspector pass.

M6's event inspector is a second consumer of the same events; it must
record an immutable batch, not "whatever happens to remain in a queue
after another subsystem has touched it."

## Decision

### 1. The batch is caller-owned data

```rust
let batch = view.take_events();   // drain: the caller now owns it
view.pump_reactive(&batch);       // process exactly this batch
inspector.record(&batch);         // (M6a) observe the same data
```

`pump_reactive(&[Event])` processes the given batch as turns — one
event = one bounded, non-reentrant, transactional turn (ADR 0017
semantics unchanged). The pump never reads or writes the internal
queue:

- events queued between the drain and the pump are a different batch;
- events a turn generates during the pump stay queued for the **next**
  drain — the reviewer's nasty case is impossible by construction
  (drain-before-pump, not pump-then-drain);
- an empty batch still applies turn zero exactly once (initial binding
  outputs), so `pump_reactive(&[])` after load is meaningful.

### 2. A batch is plain data: replay is explicit

Passing the same batch twice processes it twice. There is no view-side
memo, because consuming a batch is the caller's act of handing it
over. Replay is visible at the call site instead of accidental — the
opposite failure mode of the peek-pump, which replayed silently.

### 3. Convenience wrapper, not the primary

`pump_reactive_queued()` = drain + pump, for embedders and tests that
observe no events. The ownership model is the documented primary; the
shell, the lab, and every test use take-then-pump explicitly.

## Consequences

- The shell's redraw and the lab's headless loop drain a batch, pump
  it, and drop it (they observe nothing); anything a turn generates
  survives to the next redraw.
- The M6a inspector will receive the same immutable `&[Event]` batch
  the pump processed — deterministic tracing, bounded retention, no
  queue-shape sensitivity.
- `Event` batches are `Clone` plain data already; no new types.
- M5's frozen turn semantics are untouched: this is purely the
  orchestration boundary above them.
