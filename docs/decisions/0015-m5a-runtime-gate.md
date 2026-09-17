# ADR 0015: M5a — the isolated QuickJS runtime gate

Status: accepted (M5a; M5b–M5d build on this)

## Context

M4c froze the editing stack: opaque element identity, runtime control
state, transactional clipboard, and session-scoped IME — all with the
renderer owning editor semantics and hosts owning OS services. M5 adds
Velqu Reactive, and the first question is architectural: what *is* the
JS engine? The reviewer's line: **QuickJS is a bounded UI-computation
engine. It does not become window, document, fetch, filesystem,
Node.js, or a browser compatibility layer.** That distinction decides
whether VelquView stays small.

The M0 crate already froze the reactive *syntax surface* (`vx-*`
directives, `@event` handlers with modifiers, `:attr` bindings) as
data; this ADR gates the runtime that will later evaluate it.

## Decision

### 1. Dependency and feature set (the memory-limit trap)

`rquickjs 0.13` (QuickJS-NG bindings), `default-features = false,
features = ["std"]`. The feature list is deliberate and recorded here
because it is load-bearing:

* **No `rust-alloc`, no `allocator`** — rquickjs documents that
  `set_memory_limit` becomes a **no-op** with a custom allocator. With
  either feature enabled, a "16 MiB heap budget" would silently not
  exist. The C allocator stays active so the limit binds.
* **No `loader`, no `dyn-load`** — no ES module resolution, no native
  module loading. The only code that ever runs is source the host
  submits directly.
* MSRV probe: `cargo +1.87.0 check -p velqu-reactive` against the
  actual feature set passed before any code was accepted (rquickjs
  0.13 declares `rust-version = 1.87`; the empirical check confirmed
  it for our configuration).

### 2. Hard budgets from commit one

[`JsLimits`] bounds every evaluation: `max_heap_bytes`
(`JS_SetMemoryLimit`), `max_stack_bytes` (`JS_SetMaxStackSize`),
`max_execution_time` (enforced via QuickJS's interrupt handler as an
uncatchable interrupt — the deadline is armed before every evaluation
*and* every job-drain phase), `max_source_bytes` (checked before
compilation), `max_event_payload_bytes`, `max_mutations_per_turn`,
`max_output_string_bytes` (single strings crossing out of JS),
`max_pending_jobs` (microtasks drained per turn). Zero budgets are
rejected at construction — zero means "unlimited" to QuickJS, which is
exactly what an embedded UI runtime must never mean. Failures classify
as `JsFailure` (`Interrupted` / `OutOfMemory` / `StackOverflow` /
`Exception` / `SourceTooLarge` / `TooManyPendingJobs`); none is fatal
to the host, and the context stays usable after an interrupt.

### 3. One document generation = one isolated runtime

A `ReactiveRuntime` is constructed per document generation and
destroyed on reload (M5b wires `load_html` to this). Globals, heap,
and queued jobs die together: stale callbacks cannot touch the next
document, old handles cannot cross reloads, JS globals cannot leak
between pages. The same generation concept that scoped element
identity (ADR 0012) and IME sessions (ADR 0014) scopes script state.

### 4. The sanctioned surface is tiny — and everything else is absent

The global surface is the ECMAScript standard intrinsics with two
virtualizations plus one diagnostic object:

* `Date` — a subclass reading the host's **logical clock** (fixed
  epoch 2000-01-01T00:00:00Z, one tick per reactive turn via
  `logical_tick()`); `Date.now()` never touches the wall clock.
* `Math.random` — a seeded xorshift, seeded by the document
  generation: identical documents produce identical sequences across
  instances and runs; reloads reseed deterministically.
* `console.{log,info,warn,error}` and `velqu.log` — one sink,
  line-capped (256 lines) and length-capped per line, retained for
  host diagnostics.

The native functions behind these are captured by the profile prelude
and **deleted from the global object** — scripts reach them only
through the sanctioned names. Explicitly absent: `require`, `process`,
`window`, `document`, `fetch`, `XMLHttpRequest`, `localStorage`, any
filesystem/network/clipboard/asset access, arbitrary Rust calls. No
timers in v0. A test walks this list and fails on any leak.
Determinism is therefore a contract: deterministic scripts render
identically, and any nondeterminism a script wants must come through a
future host capability (`velqu.invoke`) — deliberately not in M5a.

### 5. Microtasks are part of the turn

One reactive turn will be: event handler → bounded pending-job drain
(same deadline, job-count budget) → binding reevaluation → mutation
commit. `Promise.resolve().then(...)` lands within the turn; a job
that throws surfaces as a classified failure; an endless promise chain
trips the job budget. M5a proves both halves (the
`microtasks_run_within_a_turn` and `job_bomb_is_bounded` tests).

### 6. Exceptions are diagnostics

Thrown errors, interrupts, and budget trips produce capped, host-safe
`JsFailure` values (M5b routes them into the existing diagnostics
pattern). A failing script never corrupts the host; the runtime stays
reusable within its generation.

## Consequences

* A hostile component cannot hang or balloon the shell: the hostile
  battery pins `while (true)`, array/string bombs, infinite recursion,
  promise-chain bombs, oversized sources, and mutation spam as bounded,
  classified outcomes.
* The engine choice stays an implementation detail: M5b compiles the
  frozen `vx-*` surface into a Rust-owned binding plan, and QuickJS
  only ever evaluates expressions against plain state — it never sees
  DOM, NodeId, or renderer types.
* Known engine-level sharp edge handled: a lexical `class Date` inside
  the shim's own scope is in TDZ while the shim reads the original —
  the subclass is named `LogicalDate` and the original is captured
  first (the test that caught this is now part of the battery).
