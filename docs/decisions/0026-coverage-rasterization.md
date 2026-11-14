# ADR 0026: Coverage rasterization — the painter anti-aliases at f32

Status: accepted (host-write-api branch)

## Context

The Lekuvo notch port rendered visibly wrong against the reference UI in
two coupled ways:

* hairline borders (`border-white/10` on rounded rects) wobbled or
  disappeared — a ring is in/out at pixel centers, so at fractional
  positions whole pixel rows of the ring dropped out;
* fractional spacing (`gap-2.5`, `p-[2.5px]`, 15.5px text) never reached
  the pixels — the reference reads as having a different rhythm.

ADR 0007 assigned Velqu the rounding role: Taffy is unrounded, and "the
painter snaps rect edges to whole pixels at raster time." That policy
was chosen alongside a painter with **no** anti-aliasing — every shape
was a strict pixel-center in/out test — because binary coverage cannot
vary. The policy delivered determinism but also hard-snapped every edge:
layout's f32 geometry was destroyed at the last step, and 1px rings at
odd coordinates degenerated.

Gradients (the other P0 blocker) inherit this decision: a corner radial
wash painted through binary coverage would show staircase edges, and
AA-free gradients would look worse than the flat approximation they
replace.

## Decision

### 1. The painter rasterizes at full f32 precision with per-pixel coverage

The display list stays in logical/device f32 (layout output, unchanged);
the painter no longer snaps shape edges to integers. Each pixel receives
a coverage fraction and blends with `source_alpha × coverage`:

* axis-aligned rectangles: **exact** 1D span fractions per axis
  (`fx·fy`) — no sampling, interior pixels take the full-strength path;
* rounded rectangles and rings: a **signed-distance classification**
  (rounded-box SDF, pure `f32` `sqrt`) short-circuits pixels whose
  center is more than `√2/2` from the boundary; boundary-band pixels
  evaluate a fixed **4×4 subsample grid** at `(i + 0.5)/4` offsets;
  rings blend `(cov_outer − cov_inner)`, which renders 1px hairlines at
  their true partial coverage at any fractional position.

### 2. Determinism is preserved by fixed grids, not by binary tests

Every coverage input is a pure `f32` function of the display item and a
fixed sample grid — no transcendentals, no adaptive sampling. Identical
display lists produce bit-identical frames, so the fixture contract
(ADR 0002) holds; the raster digests change once, deliberately, and are
re-pinned after visual audit (interior color probes are unaffected;
edge probes re-pin with the new AA values).

### 3. What stays raster-integer

Text glyphs remain fontdue's whole-pixel rasterizer (its deterministic
contract, ADR 0007 intact); image blits stay nearest-neighbor at snapped
edges; clip scopes stay binary masks. Fractional-DPI *text* rasterization
(scale-factor ≠ 1 subpixel glyph placement) remains P2 work on top of
this ADR. One consequence to watch: a fractional clip edge cuts AA'd
content with a hard line — acceptable at v1, revisit if artifacts
appear in the notch's rounded-clip cards.

## Consequences

* Subpixel layout reaches the screen: `gap-2.5`/`p-[2.5px]` spacing and
  fractional positions render as authored; the reference rhythm
  reproduces without per-pixel layout hacks.
* Hairline rings are stable — same radius, same color, every frame.
* Gradients (ADR companion work) render smooth through the same
  coverage path.
* Boundary-band pixels pay 16 subsample evaluations; interior pixels
  pay one SDF evaluation. Notch-scale surfaces stay well inside frame
  budget (measured via the `LEKUVO_PERF` paint timer).
* ADR 0007's "painter snaps edges" clause is superseded by this ADR;
  Taffy stays unrounded and Velqu remains the single rounding owner.
