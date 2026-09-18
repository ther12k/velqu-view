# M6a verification — the inspector

ADR 0020's promise: the inspector explains what Velqu actually did
without becoming another participant in application execution.

## What landed

* `crates/velqu-view/src/inspect.rs` — the four record families
  (Event/Turn/Invalidation/Render), `InspectorLimits` (records, total
  bytes, per-record bytes, preview bytes, value-capture opt-in), and
  the bounded `Trace` store with monotonic seqs and honest eviction
  reporting.
* View recording hooks: state/layout revisions, turn counters, per-turn
  attempted vs committed mutations with outcomes (Committed/Rejected/
  RolledBack), coalesced invalidation causes (structural and
  presentation cause stacks capped at 32 with drop counts), and
  completed-work deltas per render.
* `inspector_snapshot(viewport, selection)` — `&self`, cached-only:
  generation/revision coherence, `LayoutCacheState` (Fresh/Stale/
  NotAvailable), awaiting flags, pending causes, counters, subsystem-
  tagged diagnostics, and the selected element's effective style + box
  geometry + interaction flags *now*.
* `velqu-lab --inspect` — prints the snapshot and retained trace after
  a headless run. Sample (counter example):

  ```text
  inspector: generation 1, state revision 0, layout revision 1, frame 1
  inspector: layout fresh
  inspector: 1 pass(es), 0 repaint(s), 0 turn(s), 8 display item(s)
  trace: 2 record(s) retained (first #1, 2 appended, 0 evicted, 0 truncated)
  #1 invalidation structural: ["document load", "reactive SetText", "viewport"]
  #2 render frame 1: +1 pass, +0 repaint (settled #1)
  ```

  The "why did layout happen" answer is right there: three coalesced
  causes, one pass, settled by one render.

## Acceptance battery (all pinned in `crates/velqu-view/src/lib.rs`)

| Reviewer scenario | Test |
|---|---|
| Repeated idle snapshot reads change nothing | `m6a_snapshot_reads_are_inert` |
| Capture on vs off: identical state, events, facts, raster | `m6a_capture_leaves_the_application_identical` |
| Failed transaction: attempt recorded, zero committed, rollback visible | `m6a_failed_turn_records_zero_committed` |
| Several turns → one render, causes attributable, work counted once | `m6a_several_turns_settle_in_one_render` |
| Explicit replay: separate attempts, no hidden dedup | `m6a_replay_records_distinct_attempts` |
| Old batch after replacement cannot operate on it (generation mismatch recorded) | `m6a_stale_batch_cannot_touch_the_replacement` |
| Retention overflow: bounds hold, loss reported, application intact | `m6a_retention_overflow_keeps_the_application_intact` |
| Hover under a stationary pointer: presentation without JS or layout | `m6a_hover_is_presentation_without_turns` |
| Intermediate coherence exposed, not rendered away | `m6a_snapshot_exposes_intermediate_coherence` |
| Metadata by default, values only by opt-in | `m6a_records_metadata_not_user_text` |

Plus the storage unit tests in `inspect.rs` (monotonic seqs across
eviction, oldest-first byte eviction, truncate-not-drop, UTF-8-safe
cuts).

## Determinism discipline

Tests compare seqs, causes, outcomes, counts, and deltas exactly;
durations are `Instant`-measured and recorded but never asserted.
Snapshot reads cannot pump, drain, render, or advance the JS logical
clock — the API is `&self`, so the compiler enforces most of it.

## Gates

* `cargo fmt --all -- --check`; `cargo clippy --workspace
  --all-targets --locked` (clean); `cargo test --workspace --locked`
  (299 tests); `cargo +1.87.0 check --workspace --all-targets
  --locked`; headless dashboard smoke digest unchanged
  (`770b933b…`); counter/forms/tabs conformance unchanged; the lab's
  `--inspect` run shown above.

Landing: `4471722` pushed to `main`; GitHub CI green — run
[35397858504](https://github.com/ther12k/velqu-view/actions/runs/35397858504)
(fmt/clippy/test/build + MSRV 1.87, both jobs).
