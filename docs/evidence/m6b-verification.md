# M6b verification — transactional reload

The exit criterion: a reload either publishes a fully prepared,
coherent replacement, or leaves the active application usable and
unchanged except for reload diagnostics; CSS-only replacement
preserves document identity and runtime state, subject only to
necessary post-style reconciliation.

## What landed

* `VelquView::reload_document(source, viewport)` — a candidate view
  (host services transferred in, one candidate, existing budgets)
  prepares through parse → Tailwind/reactive compilation → runtime +
  initializers → initial mutations → first-frame assets → layout →
  raster; only a fully prepared candidate publishes. Publication moves
  document-owned state and adopts the prepared frame's accounting;
  host lifetime (resolver, flags, limits, inspector, ledger)
  survives. Generation ids come from one lifetime mint counter —
  failed candidates leave gaps, never collisions.
* `VelquView::reload_stylesheets(replacements, viewport)` — in-place
  upserts by `SourceId` (cascade position preserved) staged against
  the committed live document: no reparse, no initializers, no turn
  zero, no new runtime. Renderer-visible staging effects are
  snapshotted and restored wholesale on rejection; success reconciles
  interaction state with the new layout.
* `reload.rs` — the attempt ledger (`ReloadAttempt` with stage and
  generations), plus one `Reload` trace record per outcome; the
  inspector snapshot carries `last_reload`.
* `velqu-reactive` observability: `initializer_failures()` and
  `poisoned_units()` counters (M5 semantics unchanged — reload
  acceptance is stricter than load, by policy).
* Acceptance policy table in ADR 0021; rejection stages: Source,
  ReactiveInitialization, InitialMutations, FirstFrame.

## Battery (all in `crates/velqu-view/src/lib.rs`, `m6b_*`)

* **`m6b_counter_probe_end_to_end`** — the reviewer's full sequence:
  1. counter to 7 with a typed value, selection, focus, and a
     scrolled pane;
  2. a policy-rejected CSS replacement → same generation, same state,
     same control facts, same color, same focus;
  3. a valid color-only replacement → count still 7, value/selection/
     focus intact, sheet order `[A, B]` preserved with B still winning
     the equal-specificity tie, scroll offset still accumulating
     (wheel continues from 40 to 80), reloading B in place shows its
     new color;
  4. click → 8 (the preserved runtime still operates);
  5. throwing-initializer HTML → rejected at ReactiveInitialization,
     active generation unchanged, the old UI still accepts input (9);
  6. valid replacement HTML → generation jumped past the failed
     attempt's gap, source-defined state (100), the old selection
     handle reports its stale generation, the ledger records the
     publication with `generation_before`.
* **`m6b_full_reload_publishes_a_fully_prepared_generation`** — the
  published frame is presentable without another pass; a stale
  pre-reload batch is inert against the replacement; the trace
  records the publication.
* **`m6b_initial_batch_rejection_blocks_reload`** — three
  parse-succeeds-but-unpublishable candidates: `:checked` (initial
  mutations), `(+)` (poisoned unit — balanced, so the shape check
  passes, invalid as JavaScript), and an empty source; after all
  three, the old document still runs.
* **`m6b_rejected_css_reload_restores_the_previous_presentation`** —
  bit-identical raster, unchanged counters and state, and a trace
  explaining exactly the rejection record plus the verification
  render.
* **`m6b_reload_during_composition`** — a rejected reload keeps the
  composition committable; a successful full reload kills the session
  (a stale commit is a no-op; the replacement starts at its
  source-defined value).

## Engine notes

* The M5b shape check catches unbalanced expressions at plan compile
  (a diagnostic, load-acceptable) — the poisoned-unit reload fixture
  needs a *balanced* invalid expression (`(+)`), which is exactly the
  class that reaches unit compilation.
* An `overflow: auto` pane is not itself hit-targetable when its
  content covers it; the probe wheels over the content (the wheel
  walk finds the scrollable ancestor).

## Gates

* `cargo fmt --all -- --check`; `cargo clippy --workspace
  --all-targets --locked` (clean); `cargo test --workspace --locked`
  (304 tests); `cargo +1.87.0 check --workspace --all-targets
  --locked`; headless dashboard smoke digest unchanged
  (`770b933b…`); counter/forms/tabs conformance unchanged.

Landing: `a36832a` pushed to `main`; GitHub CI green — run
[35428449895](https://github.com/ther12k/velqu-view/actions/runs/35428449895)
(fmt/clippy/test/build + MSRV 1.87, both jobs).
