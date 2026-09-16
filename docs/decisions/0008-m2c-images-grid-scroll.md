# ADR 0008: M2c — images as replaced elements, the grid profile, and paint-side scrolling

Status: accepted (M2c)

## Context

M2b froze a whole-tree Taffy backend with Velqu-owned CSS semantics
(ADR 0007). M2c was authorized with a re-ordered scope — **images before
grid** — because replaced-element intrinsic sizing is an *input* to track
sizing, not a paint feature. Scroll completes the milestone on the
PushClip foundation. The standing rule continues: **Taffy capability ≠
Velqu capability**; only the frozen profile below maps onto Taffy, and
everything else stays unsupported with deterministic diagnostics.

## Decision

### 1. Images: bounded decoding in the core

Hosts provide bytes through the existing `AssetResolver` seam (ADR 0004);
the core never performs I/O. Decoding happens only in `image.rs`, under
independent resource limits — `ImageLimits { max_encoded_bytes,
max_decoded_width, max_decoded_height, max_decoded_pixels }`, validated at
construction like `Viewport` (ADR 0004) with defaults 32 MiB / 16384 /
16384 / 2^28 px:

* only the PNG and JPEG decoders are compiled (`image` 0.25,
  `default-features = false, features = ["png", "jpeg"]`; no `rayon`, so
  decode is single-threaded and deterministic);
* format detection is explicit magic-byte matching — an unsupported
  format is a deterministic diagnostic, never a decoder guess (SVG and
  animated formats are deferred);
* the header is read **before** any pixel allocation; over-limit
  dimensions and pixel counts reject without decoding;
* outcomes (success and failure) are cached per `src` per document, so
  repeated frames never re-decode and never re-diagnose.

Failure categories, each deterministic: `missing`, `unsupported format`,
`encoded too large`, `over dimensions`, `over pixels`, `decode failed`.
They surface through `VelquView::image_diagnostics()` in document order.

### 2. Images: replaced-element layout and paint

`<img>` is a real box, never flattened into inline words:

* block participation: `item_is_replaced` is set on the Taffy node, so an
  auto width sizes by content instead of stretching;
* flex participation: flex-basis comes from content; cross-axis
  `align-items: stretch` still stretches the item, as in browsers;
* sizing (Velqu-owned, decided from the **author CSS**, never from
  algorithm-resolved dimensions Taffy passes through — a stretched cross
  size must not retro-apply the ratio):
  * an absolute `width`/`height` wins; the auto companion dimension
    follows the intrinsic ratio;
  * both auto: intrinsic size (image pixels treated as logical CSS px,
    scaled at projection);
  * percentage size: resolves via the style in Taffy; the auto companion
    keeps the intrinsic size (documented profile limitation);
  * no ratio (broken/missing): the auto dimension keeps the **CSS default
    object size 300×150** — a broken image keeps a deterministic layout
    footprint and paints nothing;
* HTML `width`/`height` attributes, `srcset`, `object-fit`, inline text
  flow around images, and SVG are deferred;
* painting is `DisplayItem::DrawImage` (nearest-neighbor integer
  sampling, `object-fit: fill` semantics, clipped like every item).

### 3. Grid: the frozen profile

`display: grid` plus exactly:

* `grid-template-columns` / `grid-template-rows`: `px`, `%`, `fr`,
  `auto`, `minmax(length|auto, length|%|fr|auto)`,
  `repeat(<fixed integer>, …)` (no nested repeats);
* `grid-column` / `grid-row`: `auto | <positive integer line> |
  span n`, as `<start> / <end>`;
* `grid-auto-flow`: `row | column`;
* `gap` / `row-gap` / `column-gap` (absolute lengths, as in M2b);
* `justify-items` / `justify-self` sharing the flex alignment value
  space; baseline alignment remains deferred with a diagnostic.

Deferred (each diagnosed, none silent): named grid lines,
`grid-template-areas`, `auto-fill`/`auto-fit`, dense packing, subgrid,
masonry, percentage/calc gaps, negative line numbers. A grid container
whose children are all inline keeps its tracks: the inline content
becomes an anonymous grid item (flex containers with only words keep the
M2b leaf measurement, which keeps M2b rasters byte-identical).

Supporting fix: declaration values are now captured as the **raw source
slice** between colon and semicolon. The previous token re-serialization
dropped closing parens of nested function blocks (`repeat(2, repeat(2,
1fr))` read back as `repeat(2, repeat(`) — all prior fixtures pass
unchanged.

### 4. Scroll: runtime presentation state

`overflow: auto | scroll` (behaving identically — no scrollbar gutter
modeling) makes a box a scroll container:

* **Layout geometry is the unscrolled truth.** Scroll extents
  (`scroll_width`/`scroll_height` per container; document-level extents on
  `LayoutFacts`) are derived from laid-out child geometry in the same
  pass — scrolling requires **zero** additional Taffy passes, and
  LayoutFacts are byte-identical across any scroll change.
* Offsets enter through `VelquView::set_scroll_offset(None | Some(id), x,
  y)`: the empty key is the document-level scroller, element `id`
  attributes are the public target identity (`data-vv-test` remains
  fixture-only). Offsets are *requests*: clamped centrally to
  `0..=extent − scrollport` at apply time; unknown targets never apply;
  non-finite offsets are rejected.
* Painting emits `PushClip(scrollport)` + `PushTransform(−offset)` …
  `PopTransform` + `PopClip` per container (the document scroller wraps
  the whole frame). The painter carries transform and clip **stacks**;
  clips pushed inside a transform scope live in parent space; snapping
  happens after translation. At offset zero, emission and pixels are
  identical to M2b.
* Mouse wheel / touch / keyboard scrolling is explicitly **out of scope**
  — input events belong to the M4 milestone. Scrolling here is
  programmatic only.

### 5. Instrumentation before optimization

Whole-tree projection stays unoptimized, but measurable:
`VelquView::layout_stats()` reports `passes`, `nodes_last_pass`, and
`duration_last_pass` (indicative only, never in pixel output). When M4/M6
introduce frequent mutations, the cost of whole-tree reconstruction will
be measured, not imagined; the zero-copy Taffy adapter remains the
fallback optimization behind the module seam.

## Consequences

* The M2c freeze point: everything in this ADR is supported contract;
  everything deferred is diagnosed.
* LayoutFacts stays **v1** with additive fields only
  (`scroll_width`/`scroll_height` per node; `document_scroll_width`/
  `document_scroll_height` at the top level).
* Regression gate honored: every M2a/M2b fixture kept identical facts and
  rasters; the only CSS-side behavior change (raw-slice declaration
  values) is invisible to every previously supported value grammar.
* Future image work (object-fit, srcset, SVG, attributes) extends
  `image.rs` and the sizing rules without touching the pipeline shape.
