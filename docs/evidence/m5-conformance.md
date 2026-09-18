# M5 exit — reactive conformance (counter / forms / tabs)

M5's exit criterion: "counter/forms/tabs examples pass reactive
conformance." The three example apps are now real, human-runnable,
and machine-pinned end-to-end: the exact `index.html` files a person
runs with `velqu-lab --tailwind --reactive examples/<name>` are loaded
by `crates/velqu-view/tests/reactive_examples.rs`, driven through
public-API pointer and keyboard events, and asserted on state, layout
facts, and raster digests.

## Running them

```text
velqu-lab --tailwind --reactive examples/counter   # window, click +/-
velqu-lab --tailwind --reactive examples/forms     # type, submit enables
velqu-lab --tailwind --reactive examples/tabs      # switch tabs
```

`--reactive` (new in this slice) enables the reactive pipeline in the
lab; the headless mode pumps one turn batch per frame — the same
redraw order the shell uses.

## What each example proves

* **counter** (OKF-spec transcription, now with `id`/`data-vv-test`
  hooks): click `+`, `+`, `−` → three atomic turns → text and state
  both read `1`; deterministic across a fresh identical session
  (same drive → same raster digest); the two utilities the
  transcription uses that sit outside CSS profile v0 (`min-h-screen`,
  `mx-auto`) are diagnosed deterministically and nothing else is.
* **forms**: `vx-model` two-way binding through real keystrokes (each
  character is one `ValueChanged` → one turn → the model write lands
  before any `@input` handler); live `vx-text` derivations repaint;
  `:disabled` on the submit button actually disables it — a click on
  the disabled button produces no event and no state change — until
  both fields are non-empty; `vx-show` reveals the confirmation.
* **tabs**: `@click` switches a `tab` key; `:class` restyles the
  active tab; `vx-show` flips panels — one structural pass per switch
  (render + one `layout_facts` observation); counter state survives
  being hidden and re-shown.

## Fixes the conformance pass surfaced

1. **`SetControlDisabled` on non-controls was rejected wholesale.**
   The M5b compiler's frozen surface compiles `:disabled` on any
   element (its determinism test pins a button), but the M5c runtime
   validator demanded an input/textarea control — one such binding
   rejected the *entire initial mutation batch* (the forms example
   surfaced it as `initial mutation batch rejected`). `:disabled` now
   routes: controls → runtime state (presentation); other elements →
   the HTML `disabled` attribute (structural). A disabled element
   neither clicks nor receives focus (`node_is_disabled` unifies the
   control-state and attribute checks across `pointer_release`,
   `set_focus_node_with`, and `focusable_nodes`). Pinned by
   `m5_disabled_buttons_do_not_activate`.
2. **The shell and lab never drained the event queue.** `pump_reactive`
   reads the queue without consuming it (the frozen test ordering is
   pump → `take_events`), so undrained turn-driving events re-mapped
   to fresh turns on every later redraw — a click would re-run its
   handler once per frame — and the queue grew unboundedly in long
   sessions. The shell's redraw and the lab's headless loop now drain
   after pumping; the pump's doc states the caller contract.

## Gates

* `cargo fmt --all -- --check`; `cargo clippy --workspace
  --all-targets --locked` (clean); `cargo test --workspace --locked`
  (282 tests: 3 conformance + 1 disabled-button + all prior suites);
  `cargo +1.87.0 check --workspace --all-targets --locked`; headless
  smokes: all three examples render deterministically with reactive
  enabled; dashboard digest unchanged (`770b933b…`).

Landing: `27db6c0` pushed to `main`; GitHub CI green — run
[35378279214](https://github.com/ther12k/velqu-view/actions/runs/35378279214)
(fmt/clippy/test/build + MSRV 1.87, both jobs).
