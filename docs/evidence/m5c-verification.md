# M5c verification — reactive turns; atomic state + UI commits (ADR 0017)

Scope: the turn machine (constrained state, compile-once units, frozen
payloads, bounded jobs, validated batches), the velqu-view pump
(model-before-handler ordering, silent control writes, whole-batch
validation, no reentrancy), and the exit criterion:

> M4 events drive bounded, non-reentrant, transactional reactive turns
> whose state changes and validated UI mutations commit atomically,
> with no DOM discovery or ambient host capabilities.

## The reviewer's battery, as tests

Machine level (`velqu-reactive/src/machine/tests.rs`, 13 tests):

* `a_click_commits_state_and_emits_the_diff` — prepare does NOT commit
  (state still old until `commit`); the diff is exactly
  `SetText("Count: 1")`; a turn that changes nothing the bindings see
  emits an empty batch.
* `a_throwing_handler_rolls_the_whole_turn_back` — state write before
  the throw is discarded.
* `a_throwing_binding_rolls_state_back_too` — handler succeeded,
  binding threw on the candidate: the candidate (including the
  handler's write) is discarded.
* `a_job_bomb_rolls_the_turn_back` — an endless promise chain trips
  the job budget; the handler's write rolls back with the turn.
* `state_capture_rejects_functions_and_exotic_values` — `count = () =>
  1` rolls back as unsupported state.
* `model_write_lands_before_the_input_handler` — the handler observed
  the model write, and the sibling binding committed in the same turn.
* `five_state_changes_commit_as_one_batch` — one batch, five
  `SetText` mutations.
* `once_handlers_fire_exactly_once_and_only_on_commit` — a rolled-back
  turn does not consume `.once`; a committed one does; an all-skipped
  turn is `NoChange`.
* `stop_ends_the_ancestor_walk` — `.stop` on the target ends the walk.
* `event_payloads_are_frozen` — a handler writing `__velquEvent.value`
  is silently contained; the rest of the turn commits.
* `oversized_payloads_are_refused_before_js` — the byte budget rejects
  before construction.
* `initial_state_evaluates_the_bindings_once`, `zero_limits` (via
  JsLimits::try_new), `no ambient` (unchanged, plus the `__velquCore`
  surface pinned to exactly `freeze,isPlain,string,truthy`).

View level (`velqu-view` lib tests, 10 tests):

* `m5c_click_to_state_to_text_end_to_end` — real click → pump →
  render: turn zero replaces the placeholder with `Count: 0`; a click
  commits `Count: 1` through the facts.
* `m5c_binding_throw_rolls_back_state_and_ui` — the always-throwing
  binding rolls back turn zero and every click: raster hash identical,
  state `count == 0`, the diagnostic names `binding 0 threw`.
* `m5c_job_bomb_rolls_back_state_and_ui` — same, via the job budget.
* `m5c_model_write_precedes_the_input_handler` — real typed input:
  the handler saw `name == "Alice"`, the sibling `vx-text` shows it.
* `m5c_state_to_model_updates_the_control_without_new_events` — a
  click handler sets state; the input shows the new value and the
  event queue contains **no** `ValueChanged`.
* `m5c_five_properties_commit_in_one_layout_pass` — one handler, five
  text mutations: exactly **one** Taffy pass (`passes + 1`).
* `m5c_ancestors_run_target_first_and_stop_ends_the_walk` — ancestor
  clicks append `po`; with `.stop` on the target, only `p`.
* `m5c_reload_restarts_state_and_handlers` — reload resets state to
  the initializers and re-arms the `.once` handler in the new
  generation.
* `m5c_static_documents_pump_as_a_noop` — zero-markup document:
  identical facts and raster with reactive on, clicks pump as no-ops,
  `reactive_state()` is `None`.
* `m5b_semantic_rules…` (unchanged) plus new semantic coverage: an
  event handler carrying no scope is diagnosed; a scope node's own
  handlers resolve against their own scope (compiler walk restructure,
  caught by the M5c battery — the old walk deferred the scope push to
  child descent and silently dropped `@click` on the `vx-state` node).

## Engine findings recorded along the way

* QuickJS-NG parses `with` in sloppy mode only — rquickjs's eval
  default is strict, so units compile with an explicit
  `strict: false`; the containment story for frozen payloads is
  documented in the ADR.
* rquickjs runtime-level calls (job drain) cannot run inside a
  context `with` scope — the turn is structured as two phases with
  the candidate anchored in the JS heap between them.

## Gates

* `cargo fmt --all -- --check`; `cargo clippy --workspace
  --all-targets --locked` (clean); `cargo test --workspace --locked`
  (272 tests: 13 machine + 10 view M5c + all prior suites);
  `cargo +1.87.0 check --workspace --all-targets --locked`; headless
  dashboard smoke digest unchanged (`770b933b…`).

Landing: `433162e` + `1a3bbac` pushed to `main`; GitHub CI green —
run [35366406880](https://github.com/ther12k/velqu-view/actions/runs/35366406880)
(fmt/clippy/test/build + MSRV 1.87).
