# M4b verification — interaction styling; state never lays out (ADR 0011)

Scope: `:hover`/`:focus`/`:active` selectors with class-level
specificity, the paint-only freeze, the presentation-only repaint path,
NodeId-keyed runtime state with relayout transplant, `cursor`, and
focus origin. Established gates held: no fixture raster or fact changed
(no migrations — the full-layout path is untouched when state is empty).

## The reviewer's carry-forward: bake_scroll invariants

Pinned as tests early in the milestone (`crates/velqu-view/src/lib.rs`,
the "scroll-state invariants" section):

1. **scroll → facts identical**: `scroll_leaves_layout_facts_untouched`
   (deep-equality across `set_scroll_offset` + wheel).
2. **scroll → hit testing sees translated children**: pinned in M4a
   (`hit_test_honors_scrolled_offsets`), still green.
3. **re-render at the same viewport → offset neither lost nor applied
   twice**: `scroll_applies_once_and_stays_across_repeated_renders`
   (three identical hashes, distinct from the unscrolled hash).
4. **scroll → resize/reflow → position survives, re-clamped**:
   `scroll_position_survives_resize_and_reclamps` (a wheel at the new
   max is a no-op; a wheel-up moves exactly 40 from the re-clamped
   value) and `scroll_position_survives_a_restyle_relayout` (offset
   accumulates across a `load_css` relayout). Reload resets — pinned
   since M4a.

The reviewer's NodeId suggestion is implemented verbatim: runtime
scroll state keys on DOM node identity (`None` = the document scroller),
so the transplant needs no copying — the keys simply outlive box-tree
rebuilds. This also fixed a real M4a defect: id-less scroll containers
previously stored wheel offsets under the empty-string key, aliasing the
document scroller; `idless_scroll_containers_have_independent_
accumulating_state` pins the fix.

## The headline contract, as tests

* `hover_changes_pixels_without_touching_layout` — hover changes the
  raster, LayoutFacts deep-equal, `layout_passes` frozen, `repaints`
  incremented. Structural truth and pixels are different documents now.
* `wheel_scrolls_nearest_container_with_zero_layout` — strengthened:
  the render after a wheel runs **zero** layout passes (previously one).
* `focus_paint_moves_with_tab_and_never_lays_out` — Tab A→B moves the
  `:focus` paint across boxes with zero passes.
* `scrolling_under_a_stationary_pointer_moves_hover` — wheel with the
  pointer parked: `PointerLeave{a}`/`PointerEnter{b}` events, hover
  paint follows, zero passes.
* `layout_hover_deferred_with_deduped_diagnostics` — `:hover { width:
  500px; background-color: … }`: the paint applies, the width is
  deferred, the card keeps 100px, one deduplicated diagnostic.

## Stateful selectors

* `hover_styles_follow_the_pointer_chain` — hovering the *label* turns
  the card red (`.card:hover` through the ancestor chain) and the label
  green (`.card:hover .label` descendant rule); everything reverts
  off-card. Repaints only.
* `hover_specificity_is_class_level` — `.card:hover` (0,2,0) beats
  `.card` (0,1,0) regardless of source order.
* `focus_matches_the_focused_element_exactly` — `:focus` does not chain.
* `overlapping_siblings_hover_the_topmost_only` — the topmost painted
  box takes hover and click (ADR 0010's amended paint-order contract).
* `clipped_content_is_never_hovered` — content hanging outside a scroll
  container's clip is not hoverable, pixels untouched.
* `unsupported_pseudo_classes_are_diagnosed_and_skipped` — `:first-child`
  rules are skipped with a named diagnostic, never silently ignored.

## Cursor and focus origin

* `cursor_is_inherited_and_read_under_the_pointer` — `cursor: pointer`
  on a card reads as `Pointer` over its id-less child (computed,
  inherited) and `Auto` outside; reading never lays out.
* `focus_origin_tracks_why_focus_moved` — Tab → `Keyboard`, `set_focus`
  → `Programmatic`, click → `Pointer`.

## Shell

* Cursor maps to the platform icon (`Auto` → arrow); a cursor change
  alone neither repaints nor lays out.
* The shell repaints only when the view reports a pixel-relevant change
  (`pointer_move`/`press`/`release` return it; wheel implies it) —
  stationary-pointer moves without hover changes cost nothing.

## Final verification at freeze

* `cargo test --workspace --locked`: all green — 138 velqu-view lib
  tests (20 new for M4b), 31 velqu-tailwind tests (checker interaction
  rule included), 17 visual fixtures byte-identical, shell, lab,
  cross-instance determinism, doctests.
* `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
  --locked`: clean.
* `cargo +1.87.0 check --workspace --all-targets --locked`: clean.
* Headless smoke (`velqu-lab --headless --tailwind --frames 3` on the
  dashboard example): 3/3 frames identical, no diagnostics.
* Window smoke: frames presented on Wayland (`wayland-0`) and forced
  X11 (`DISPLAY=:1`).
* CI: both lanes green on the M4b push.

## Honest scope notes (deferred, diagnosed — not silent)

* Interaction selectors are frozen to paint properties; layout-affecting
  ones are deferred with diagnostics. Widening requires the
  dirty-style→dirty-layout propagation design and is deliberately not
  improvised here.
* Tailwind *variants* (`hover:bg-red-500`) still diagnose as unsupported
  in the compiler; interaction styling flows through the CSS route.
  Variants are a natural follow-up now that matching exists.
* `:focus-visible`, `:focus-within`, `:active` scroll-chaining,
  scrollbars, momentum, keyboard scrolling: future M4 work; the state
  shapes (focus origin, chain paths) were chosen to make them cheap.
* Presentation repaint re-emits the whole display list each time —
  correctness-first; incremental restyling waits for evidence.
* Editing/selection/IME are M4c, split into c1 (editable controls),
  c2 (clipboard behind a shell interface), c3 (IME as a platform
  integration milestone with dedicated platform verification).
