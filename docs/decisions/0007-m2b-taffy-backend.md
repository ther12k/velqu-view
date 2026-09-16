# ADR 0007 — Taffy as the whole-tree layout backend; Velqu owns rounding, facts, and the profile

- Status: accepted (M2b, 2026-09-16)

## Decision

Taffy 0.14 is the layout-algorithm backend for the **entire** box tree —
block *and* flex (and grid, when M2c lands) — not a bolt-on flex calculator.

**Strategy (of the three considered):** the Velqu `BoxNode` tree stays
canonical; each layout pass **projects it in full** onto a private
`TaffyTree` (`taffy_backend.rs`), runs `compute_layout_with_measure`, and
writes results back. Per-flex-island projection was rejected outright:
intrinsic measurement, available-space propagation, and percentage sizing
cross engine boundaries in any nested document. The zero-copy
`LayoutPartialTree` adapter remains a contained future optimization behind
the same module seam if projection cost ever matters.

**Boundary:** `ComputedStyle`, `BoxNode`, `LayoutFacts`, the display list,
and the public API stay Velqu-owned. No Taffy type escapes
`taffy_backend.rs` (ADR 0003). Taffy capability ≠ Velqu profile capability:
`order`, baseline alignment, percentage gaps, and absolute positioning are
individually deferred or excluded by the M2b profile below, even where
Taffy supports them.

## Rounding policy — one owner

Taffy's rounding is **disabled**. Layout computes in device pixels (CSS px
scaled at projection) and stays unrounded `f32` through `LayoutFacts`; the
painter snaps rect edges at raster time. Consequence, adopted explicitly:
flex facts at 2× need not numerically equal 2 × the 1× facts (three equal
children in 101 logical px cannot all have integer widths at every scale).
The invariant is **deterministic layout per viewport, exact containment
(item edges tile the container), and this documented rounding policy**. The
flex-101 fixture asserts the 101/3 split and tiling at 1× and 1.25×.

## Page model migration (intentional, documented)

M2a added adjacent margins (no collapsing). With Taffy's CSS block
algorithm, **sibling margin collapsing now applies** — this is a deliberate
correction toward CSS semantics. `html`/`body` map to Taffy `FlowRoot` so
page-root margins do not collapse through. A viewport-root wrapper node
restores root margins (Taffy ignores root-node margins). The M2a regression
gate passed: every M2a structural fact unchanged; the hello 1× raster is
byte-identical. The hello-2× raster moved (unrounded f32 baselines;
sub-pixel glyph placement), facts unchanged — regenerated and documented in
the fixture.

## Text leaves and measurement

Boxes with only inline content project as **leaves** measured through
`compute_layout_with_measure`: the measure function returns the wrapped
text **content size** (Taffy adds its own padding/border), and CSS
size/min/max handling is delegated to Taffy's `compute_leaf_layout`.
Boxes with both words and block children get an anonymous words node (a
plain block) preceding the children — matching M2a's inline-above-block
ordering.

## Baseline alignment: deferred, loudly

`align-items/align-self: baseline` parses, records, and **diagnoses**
("deferred in the M2b profile; laid out as flex-start"). It is never a
silent flex-start fallback. Implementing it needs real font baselines
(current layout uses constant DejaVu metric ratios); it lands with the
M2b+ text overhaul. The trap fixture (12px/26px/16px runs under baseline
alignment) is queued for that work.

## M2b CSS profile (frozen)

Supported: `display: flex`; `flex-direction` (row/column/reverses);
`flex-wrap`; `flex-grow/shrink/basis`; `justify-content`
(flex-start/center/flex-end/space-between); `align-items`/`align-self`
(stretch/flex-start/center/flex-end); `gap`/`row-gap`/`column-gap` (px/rem;
percentage gaps deferred); width/height + min/max; padding/margin/border;
block children inside flex items; nested flex; deterministic text
measurement under constrained width; `overflow: visible | hidden | clip`
(layout containment in Taffy; clipping executes in the display list).

Deferred (each needs an explicit profile revision): `order`, baseline
alignment, percentage gaps, scrollbar behavior, absolute positioning,
fragmented inline decorations (inline backgrounds/borders that wrap across
lines — waits for a real inline formatting model).
