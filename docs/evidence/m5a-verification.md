# M5a verification — the isolated QuickJS runtime gate (ADR 0015)

Scope: the rquickjs 0.13 dependency under the deliberate feature set,
`JsLimits` enforcement, hostile-script termination, the sanctioned
global surface, deterministic clock/random virtualization, and
per-generation isolation. The M5 slices: this is 5a (runtime gate); the
binding compiler (5b), state + events (5c), and invalidation batching
(5d) follow.

## The dependency probe

* `rquickjs 0.13`, `default-features = false, features = ["std"]`.
  Neither `rust-alloc` nor `allocator` is enabled — the documented
  condition under which `set_memory_limit` silently no-ops — so the C
  allocator is active and the heap limit binds.
  `loader`/`dyn-load` are absent: no module resolution or native
  loading exists to reach.
* MSRV: `cargo +1.87.0 check -p velqu-reactive` against the actual
  feature set passed before implementation continued (18s stable /
  14s MSRV compile of the QuickJS sys crate).

## The hostile battery (`crates/velqu-reactive/src/runtime/tests.rs`)

All under tight test budgets (4 MiB heap, 256 KiB stack, 100 ms
deadline, 16-job cap):

* `infinite_loop_is_interrupted_and_the_runtime_survives` —
  `while (true) {}` raises the uncatchable interrupt, classifies as
  `Interrupted`, finishes well under the bound, and the context
  evaluates again afterwards.
* `memory_bomb_is_terminated` — endless array-of-strings growth ends
  as a classified bounded failure.
* `deep_recursion_hits_the_stack_limit` — `f() { return f() + 1 }`
  classifies as `StackOverflow`.
* `oversized_strings_are_refused` — `repeat(2**31)` (engine length
  cap) and `repeat(64 MiB)` (heap budget) both terminate.
* `job_bomb_is_bounded` — an endless `Promise.resolve().then(chain)`
  trips `TooManyPendingJobs`; `microtasks_run_within_a_turn` proves a
  legal microtask lands inside the same evaluation's drain.
* `oversized_source_is_rejected_before_evaluation` — source over
  `max_source_bytes` never compiles.
* `zero_limits_are_rejected` — zero budgets are refused (zero means
  "unlimited" to QuickJS).
* `exception_diagnostics_are_captured_not_fatal` — a throw surfaces as
  capped text; the runtime remains usable.

## Isolation, surface, determinism

* `reload_destroys_the_js_world` — a global set in generation 7 is
  `undefined` in a fresh generation-8 runtime; queued jobs die with it.
* `no_ambient_host_surface_exists` — the script walks
  `require`/`process`/`window`/`document`/`fetch`/`XMLHttpRequest`/
  `localStorage` and the internal sink/clock globals (all must be
  `undefined` — the profile prelude captures and deletes them) and
  asserts `Object.keys(velqu)` is exactly `["log"]`.
* `clock_and_random_are_deterministic` — two same-generation runtimes
  return identical `Date.now()`/`Math.random()` sequences; a
  `logical_tick()` advances `Date.now()` by exactly one step; a new
  generation reseeds. `Date` and `Math.random` never touch the wall
  clock.
* `console_sink_is_recorded_and_capped` /
  `output_lines_are_length_capped` — the diagnostic sink records
  `console.*`/`velqu.log` output, caps at 256 lines, and truncates
  over-long lines on char boundaries.

## Engine sharp edge caught by the battery

The Date shim initially subclassed as `class Date extends OriginDate`
inside its own scope: the lexical binding is in TDZ while the shim
reads the original, producing `ReferenceError: Date is not
initialized` on ~every construction path. The battery caught it
immediately; the fix (capture `OriginDate` before declaring
`LogicalDate`) is documented in ADR 0015 §6.

## Gates

* `cargo fmt --all -- --check`; `cargo clippy --workspace
  --all-targets --locked` (clean, `-D warnings` lane equivalent);
  `cargo test --workspace --locked` (232 tests incl. the 19-test M5a
  battery); `cargo +1.87.0 check --workspace --all-targets --locked`.
* Headless dashboard smoke: digest unchanged (`770b933b…`) — M5a adds
  no rendering behavior; documents without reactive attributes are
  byte-identical to M4 (the runtime is not yet wired into
  `load_html`; that lands with M5b's binding plan).

Landing: implementation + docs pushed to `main`; GitHub CI green (run
recorded in the evidence addendum).
