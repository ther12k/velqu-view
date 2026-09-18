# M6a.1 verification — event ownership

The M6-opening fix the reviewer required before the inspector:
`pump_reactive` became a **caller-owned batch** API; the peek-then-drain
choreography that M5's conformance run proved too easy to misuse is
gone (ADR 0019).

## The change

* `VelquView::pump_reactive(&events: &[Event])` — processes exactly the
  given batch; never reads or writes the internal queue. Turn-zero
  (initial binding outputs) still applies on the first pump with an
  empty batch.
* `VelquView::take_events()` — the drain; the returned batch is the
  caller's plain data (observe it, pump it, or drop it).
* `VelquView::pump_reactive_queued()` — convenience wrapper
  (drain + pump) for embedders/tests that observe no events; the
  ownership model is the documented primary.
* The shell's redraw and the lab's headless loop now do
  `let batch = take_events(); pump_reactive(&batch);` — the drain hack
  from the M5 fix is replaced by the real model.
* All 35 prior test call sites migrated to the explicit idiom.

## Pinned semantics (`m6a_*` tests)

* `m6a_pump_owns_the_batch_not_the_queue` — a click drained into a
  batch processes; a second click queued **after** the drain is
  untouched by that pump and survives to the next drain, advancing the
  state exactly once more.
* `m6a_repassing_a_batch_is_explicit_replay` — the same batch passed
  twice processes twice (no hidden memo); replay is visible at the call
  site; neither pass queues anything.
* `m6a_queued_wrapper_drains_then_pumps` — the wrapper leaves the queue
  empty; pumping again at rest is a no-op (the old peek-pump re-ran the
  undrained click).

The reviewer's "nasty case" — a turn-generated event destroyed by an
indiscriminate drain — is impossible by construction: the drain happens
before the pump, generation during it, the next drain observes it.
(M5 mutations are silent today, so no intra-turn generation exists
yet; the contract is structural.)

## Gates

* `cargo fmt --all -- --check`; `cargo clippy --workspace
  --all-targets --locked` (clean); `cargo test --workspace --locked`
  (285 tests: 3 ownership + all prior suites);
  `cargo +1.87.0 check --workspace --all-targets --locked`; headless
  dashboard smoke digest unchanged (`770b933b…`); the three reactive
  examples still conform (their harness pumps drained batches).
