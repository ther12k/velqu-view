# Perf deep-dive handoff (frame smoothness: pill↔card morph + bar drag)

Status: the performance work below is **complete and was fully green
(`cargo test -p velqu-view` 250 lib + all integration suites, including the
pixel-hash fixtures) before the gradient/spring edits landed in the shared
tree.** (This was a mid-flight note: the gradient/spring stream has since
finished, and the combined tree verified green and landed together — see
the batch commit on `host-write-api`. The perf stream needed no changes
for it; the files simply interleave.)

## What is perf work (this stream)

New files:

* `crates/velqu-view/src/perf.rs` — `LEKUVO_PERF=1`/`VV_PERF=1` per-frame
  instrumentation: one stderr line per render with css-parse ms, style
  compute ms (in-pass and presentation), layout total + style/boxes/taffy/
  emit sub-phases, presentation repaint ms, paint ms, item/glyph counts,
  damage rect/area, viewport, frame interval. Observational only.
* `crates/velqu-view/tests/damage_paint.rs` — the damage-path equivalence
  contract: a frame rendered through the damage path (previous frame +
  display-list diff) is pixel-identical to a from-scratch render of the
  same state; covers color-only flips, width/height morphs, idle frames.

Edits, by area:

1. **Tailwind cascade order** (`crates/velqu-tailwind/src/compile.rs`):
   compiled utility rules are now stable-sorted so `duration-*`/`delay-*`/
   `ease-*` modifiers (rules without `transition-property`) sort **after**
   `transition-*` property carriers. First-appearance order made a card's
   `transition-all` silently override an earlier element's `duration-500`
   back to the 150ms base (the lekuvo card ran its 500ms morph at 150ms).
   Adds `is_transition_modifier()` + test
   `transition_modifiers_sort_after_property_rules`.

2. **`layout_facts` cache** (`crates/velqu-view/src/lib.rs`): facts reuse
   the cached layout when structure is clean and the viewport matches —
   hosts polling facts every frame (mascot compositing, input regions) cost
   a tree walk instead of a full re-parse + relayout (was ~5.8ms per call;
   lekuvo calls it 3×/frame). Relayout happens only on `structure_dirty`
   or viewport mismatch. Test `reactive_examples::tabs_example_conforms`
   updated: a clean-cache facts observation now costs 0 passes (was 1).

3. **Cascade style cache** (`layout.rs` `StyleCache` + lib.rs): exact-keyed
   `(node, parent-style-id, tag+full-attribute-text)` computed styles with
   a monotonic per-epoch parent id (never an address). Animation frames
   freeze attributes/sheets, so all nodes but the animating one hit the
   cache (style compute on morph frames: ~4.5ms → ~0.2ms). Invalidation
   (`StyleCache::invalidate`) fires on: attribute/text edits
   (`apply_mutations` structural/tailwind paths), vx-transition phase
   swaps/completions, sheet changes (`rebuild_css`, `rebuild_tailwind`,
   `collect_style_blocks`, `enable_tailwind`), interaction changes
   (`update_hover`, pointer press/release/exit, `set_focus_node_with`),
   document load. `layout_document`/`compute_all_styles` gained a
   `&mut StyleCache` parameter (all call sites updated, tests pass a fresh
   cache).

4. **Painter row spans + damage painting** (`crates/velqu-view/src/painter.rs`):
   * Axis-aligned `fill_rounded`/`fill_rounded_ring`/`fill_rect` are
     row-scoped: per scanline the inside interval is derived from the same
     corner math (conservative ±2px) and the exact per-pixel predicate
     still runs inside the span — **pixel-identical output** (all pixel-hash
     fixtures pass) while the outside-span scan disappears. The box-shadow
     ring stack (8 rings over the blurred area) was the top paint cost:
     5→23ms growing with card area during the morph.
   * `paint_document` takes `previous: Option<(&[u8], Rect)>`: with a
     previous frame it copies its pixels and clears+repaints only the
     damage rect; `blend_pixel` has a damage-window safety net, text/image
     loops skip rows outside the window.
   * `display_list_bounds` produces index-aligned conservative device-px
     bounds per display item (transform scopes resolved, shadows + blur
     reach, 1em/char text estimate, 3px margin).

5. **Damage diff in `render`** (`crates/velqu-view/src/lib.rs`): the display
   list is diffed against the last painted one (item equality **plus**
   transform-resolved bounds — scroll shifts bounds while items stay
   equal); the union of divergent items' bounds (both old and new) is the
   repaint window, clamped into a `(x, y, w, h)` damage exposed as
   `RenderStats.damage` (new public field; `None` = full surface — first
   frame, viewport/document/background change, or full-surface change).
   `VelquView` keeps `prev_paint` (`PaintedFrame`: cloned display list,
   bounds, background, frame pixels, viewport).

6. **Ticker signal** (`anim.rs`, `lib.rs`): `AnimationEngine::
   has_layout_animation()` / `VelquView::has_layout_animation()` — true
   while a width/height animation is in flight; hosts run their shortest
   cadence then and the ambient 30fps otherwise.

Perf-only, no-impact plumbing: `perf` field on `VelquView` (inert without
the env flag), `PassTimings` out-param threaded through `layout_document`
(timing reads only).

## What is the other stream (user, mid-edit — NOT perf work)

* `crates/velqu-view/src/gradient.rs` (new, untracked) — P0 item 1 of
  `docs/velqu-worklist.md`: `linear-gradient`/`radial-gradient` background
  layers (parse → `ResolvedGradient` → display-list emission → paint).
* `anim.rs` (most of the +459 lines), `style.rs`, `display_list.rs`,
  `lib.rs`, `painter.rs` hunks — ADR 0027 spring transitions
  (`transition-timing-function: spring(...)`, `SpringSpec`), `BoxShadow`
  as an animatable property (`AnimValue::Shadow` WIP), `GradientFill`
  display item + `ResolvedGradient`, `background_image` style property.
  Shared-file hunks interleave with the perf work above (notably `lib.rs`
  render/prev-paint region and `painter.rs` match arms).

## Known compile blockers at handoff time (theirs, mid-edit)

* `gradient.rs:236` — `match` arm body without braces
  (`Some(len) => RadialSize::Ellipse { ... }` multi-statement arm).
* `gradient.rs:340,343,…` — `parse_gradient_length(tokens[1])` passed a
  `String` where `&str` is expected.
* `anim.rs:321` — `AnimValue::Shadow` variant referenced but not yet added
  to the enum (BoxShadow property WIP).

## Expected combined verification once the user's edits compile

`cargo test` (all suites — pixel-hash fixtures must stay green; the damage
path is pixel-identical by construction and pinned by `damage_paint.rs`),
`cargo clippy` both repos, then a combined commit on host-write-api. The
perf stream's benchmarks live in lekuvo (`LEKUVO_BENCH_MORPH`,
`LEKUVO_BENCH_DRAG`, `LEKUVO_PERF`) and were validated against a snapshot
of this tree with the gradient WIP minimally brace/type-fixed (see the
lekuvo commit message for the measured numbers).

---

## Final state (session close)

The user's commits absorbed the perf work directly:
- `8699042 feat(renderer): coverage rasterization, gradients, springs, damage
  painting` — carried the perf stream (style cache, facts cache, damage
  painting, painter row spans, perf instrumentation) together with the
  visual-quality work, "verified green as one tree".
- `ff495f3 perf(painter): single-pass shadow falloff + scoped morph damage` —
  folded the late band-ring/damage fixes (prefix-tolerant diff,
  clip-aware bounds, animated reach, ring exclusion) plus a single-pass
  shadow falloff replacing the 8-ring stack.

Lint follow-ups: `5d07311` added the `walk_styles` argument-count
allowance; the branch review picked up the `CompiledRule` and
`sort_by_key` cleanup in `crates/velqu-tailwind/src/compile.rs`.

### Verified final numbers (lekuvo @ 88f4731 against this tree)

- Morph (24 expand/collapse, 62.5fps host cadence): render p50 10.0ms /
  p95 15.5ms / max 31.3ms; frame interval p50=p95 16.0ms (cadence-bound);
  **gate met** (>=45fps sustained, interval p95 <= 22ms). Baseline before
  the session: render p50 30.6ms / p95 42.8ms, interval p95 64.6ms
  (~15fps), and the morph itself never ran (cascade-order bug) until the
  tailwind rule-order fix.
- layout_facts x3/frame: 17.5ms -> 0.05ms. Style compute per morph frame:
  4.5ms -> 0.3ms. Damage during the morph: the animated card region
  (~100-320k px), never the full 425,600-px surface.
- Drag (180 moves at 60Hz, Xvfb): move_window 21us/event (reply round
  trip removed); the loop performs **zero** renders/presents during a
  drag (`LEKUVO_PERF renders (during drag 0)`); the old echo-coupled
  shape cost 5.6-7ms render + present per move. Gate met.
- Idle: 0% of one core over 8s (12 frames total, zero renders/presents
  per second). Gate met (<=2%).

Note: an untracked scratch test `morph_check.rs` that existed briefly in
this tree was removed during scratch cleanup; it was a small repro and
its coverage is superseded by `tests/damage_paint.rs`.
