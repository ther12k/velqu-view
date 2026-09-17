# M4a verification — the input gate (ADR 0010)

Scope: hit testing on the cached laid-out tree, runtime interaction
state (hover/pressed/focus), the event queue, wheel scrolling with zero
relayout, and velqu-shell wiring. Established gates held: every M2a/M2b
/M2c/M3 fixture fact and raster is unchanged (no migrations this
milestone — no painting code changed).

## What the tests pin

`crates/velqu-view/src/lib.rs` tests (the `input_view()` document: a
200×100 scrollable pane `a` with a 400×60 child and a 50×40 sibling,
then a 200×100 pane `b`):

* **`hit_test_reports_topmost_and_clip_scoped_elements`** — topmost-wins
  (the red child over pane a's background), the green sibling over the
  same container, the clipped-away region NOT hit (a point inside the
  child's 400px geometry but outside pane a's 200px clip falls through
  to the page), pane `b` by id, and nothing identifiable past the
  document.
* **`hit_test_honors_scrolled_offsets`** — after scrolling pane a by
  150px, viewport points map through the applied (clamped) offset: the
  content that moved under the clip is what gets hit.
* **`pointer_events_track_hover_and_click`** — enter/leave on id
  transitions only, no repeated events while stationary, click requires
  press+release on the same id, click focuses the id, and release off
  the press target clicks nothing.
* **`wheel_scrolls_nearest_container_with_zero_layout`** — wheel over a
  non-scrollable element targets the document scroller (viewport 150px
  vs a 200px document so it has range); wheel over pane a scrolls the
  pane; repeated ticks clamp at the extent (200px max for a 400px
  content in a 200px scrollport); `layout_stats().passes` never moves
  for the wheel calls themselves — one ordinary pass per render, as any
  frame.
* **`focus_cycles_and_reports`** — Tab-order cycle over id-carrying
  elements, wrapping, `FocusChanged` events, no event on a no-op
  `set_focus`.
* **`input_state_resets_with_the_document`** — hover/pressed/focus/
  events and the cached layout are all cleared by `load_html`.

## The consecutive-wheel invariant

A real pointer delivers many wheel events between frames. The first
implementation invalidated the cached layout on every wheel, which
would have silently dropped all but the first. As shipped, a wheel that
changes an offset **bakes** the clamped value into the cached tree
(`bake_scroll`): the container's `applied_scroll` (or the document root
offset) updates in place, mirroring what the next render's
`apply_scroll_offsets` computes from the stored request. Accumulation
across events without renders is exercised by the clamp loop in the
wheel test (renders between ticks only to paint).

## Shell wiring (velqu-shell)

* `CursorMoved` → `pointer_move` (viewport device px = physical window
  px), `CursorLeft` → `pointer_exit`.
* Left `MouseInput` press/release → `pointer_press`/`pointer_release`
  at the last tracked pointer position (NaN until the first move, so
  early clicks hit nothing).
* `MouseWheel` → `wheel` + dirty redraw. winit's deltas are
  opposite-signed to the browser convention (checked against winit
  0.30's docs and both the X11 — button 5 → `LineDelta(0, -1)` — and
  Wayland backends), so the shell negates; `LineDelta` notches scale by
  a 40px line height, `PixelDelta` passes through as exact px.
* Tab (non-repeat) → `focus_next`; Escape still closes.
* Redraw is scheduled only when pixels can change (wheel). Hover/focus
  do not alter painting yet — that is the M4b styling slice.

## Final verification at freeze

* `cargo test --workspace --locked`: all green — 118 velqu-view lib
  tests (6 new input-gate tests), 30 velqu-tailwind tests, 17 visual
  fixtures (facts + rasters byte-identical to phase 1), shell, lab,
  cross-instance determinism, doctests.
* `cargo fmt --all -- --check`: clean.
* `cargo clippy --workspace --all-targets --locked`: clean (a context
  struct replaced the 8-argument `wheel_target`; elidable lifetimes
  removed).
* `cargo +1.87.0 check --workspace --all-targets --locked`: clean.
* Headless smoke (`velqu-lab --headless --tailwind --frames 3` on
  `examples/tailwind-dashboard`): 3/3 frames identical, no diagnostics.
* Window smoke: `velqu-lab --window --exit-after-ms 2500` presented
  frames on Wayland (`wayland-0`) and forced X11 (`DISPLAY=:1`).

## Honest scope notes (deferred, diagnosed — not silent)

* Hover/focus/**pressed do not paint yet**; `:hover`-style selection is
  the M4b styling slice. Events carry the data it will need.
* Focus is id-based document order — no `tabindex`, no roving focus,
  and elements without `id` are not focusable.
* No scroll chaining (a container at its end does not spill the
  remainder to the page), no scrollbars, no momentum, no
  keyboard scrolling (PageDown etc.) — M4 follow-ups.
* No text editing, selection, or IME (M4c scope per the milestone
  plan).
* Hit testing covers the frozen layout profile; transformed/reordered
  (z-index) painting does not exist yet, so there is nothing to
  inverse-map beyond DOM paint order.
