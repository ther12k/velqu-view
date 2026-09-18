# M5d verification — invalidation batching

Claim set (reviewer's M5 exit for this slice):

1. **Presentation-only turn → 0 Taffy passes.**
2. **Structural turn → at most 1 Taffy pass.**
3. **No-op turn → 0 repaint.**

All three were unprovable before M5d: the steady-state render path
re-emitted the display list unconditionally, so *every* render counted
a repaint. M5d adds the `presentation_dirty` gate (ADR 0018) and makes
the claims literal, test-pinned properties of `layout_stats()`.

## Mechanism

* `render()` three-way switch: `structure_dirty` (or missing/viewport-
  mismatched cache) → full pass; else `presentation_dirty` →
  presentation repaint (zero Taffy); else → paint the cached display
  list unchanged (zero accounting, byte-identical frame).
* `rebuild_control_presentation()` — the funnel every control-paint
  change flows through (editing, IME preedit/commit/cancel, drag
  selection, reactive `SetControlValue`/`SetControlDisabled`) — marks
  the flag; the layout and repaint paths clear it.
* Wheel and programmatic scroll bake offsets into the cached tree and
  mark the presentation repaint they always implicitly needed.
* Interaction dirtiness is **precise**: a memoized `interaction_paint`
  probe scans the cascade (author sheets, generated utilities,
  `<style>` blocks; UA sheet is empty) for `:hover`/`:focus`/`:active`.
  Without a match, hover/press changes dirty nothing; focus transfers
  dirty only when a control is involved (caret paint reads focus).
  The memo drops on every structural change.

## Test battery (`crates/velqu-view/src/lib.rs`, `m5d_*`)

* `m5d_presentation_only_turn_costs_zero_taffy_one_repaint` — a click
  handler rewrites `vx-model` state; the only mutation is
  `SetControlValue`: `passes` unchanged, `repaints + 1`, control facts
  show the new value, and no `ValueChanged` was synthesized.
* `m5d_structural_turn_runs_exactly_one_taffy_pass` — one handler,
  five `vx-text` mutations: `passes + 1` exactly, `repaints + 0` (a
  layout pass is not a repaint), all five texts updated.
* `m5d_noop_turn_repaints_nothing` — handler advances an unbound state
  key: the turn commits (`spare = 1`) but the diff is empty; the next
  render is byte-identical with `+0` passes and `+0` repaints.
* `m5d_idle_render_costs_nothing` — steady state, nothing dirty:
  repeated renders are byte-identical and move neither counter.
* `m5d_multiple_turns_settle_in_one_pass` — two queued clicks → two
  atomic turns → **one** settling render at `passes + 1`; final text
  and state both reflect the second turn.
* `m5d_hover_without_stateful_paint_is_free` — hovering across
  elements in a document without interaction selectors queues
  enter/leave events but repaints nothing; after `load_css` adds a
  `:hover` rule (structural pass + memo invalidation), hover becomes
  exactly one presentation repaint with no new pass.

## Fixes the gating exposed (previously masked by unconditional repaint)

* `load_stylesheet` after the first render now marks structure dirty —
  a post-load sheet can change layout-affecting properties; before, it
  only reached pixels as a side effect, and the documented "forces a
  full relayout" never actually ran. Pinned by the hover test's
  `load_css` leg (the restyle pass happens).
* `enable_tailwind` after a render marks structure dirty (utility
  sheet injection is a restyle).
* Oversized event payloads are no longer silently swapped for empty
  ones in the turn mapper: `PayloadTooLarge` records a diagnostic and
  skips the turn (the M5a budget contract).

## Prior-suite compatibility

All 272 pre-M5d tests pass unchanged: M4b interaction tests exercise
documents *with* `:hover`/`:focus` rules (probe true → dirty), M4c1
editing tests dirty through control paint, and scroll tests dirty
through the baked-offset path. Visual fixture baselines and the
dashboard digest are unchanged — gating only removes re-derivations,
never pixels.

## Gates

* `cargo fmt --all -- --check`; `cargo clippy --workspace
  --all-targets --locked` (clean); `cargo test --workspace --locked`
  (278 tests: 6 new M5d + all prior suites);
  `cargo +1.87.0 check --workspace --all-targets --locked`; headless
  dashboard smoke digest unchanged (`770b933b…`).

Landing: `07fca43` pushed to `main`; GitHub CI green — run
[35372713562](https://github.com/ther12k/velqu-view/actions/runs/35372713562)
(fmt/clippy/test/build + MSRV 1.87, both jobs).
