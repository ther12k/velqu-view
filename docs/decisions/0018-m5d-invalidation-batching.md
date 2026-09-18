# ADR 0018: M5d — invalidation batching; measured renderer cost per turn

Status: accepted (M5d)

## Context

M5c froze reactive semantics: atomic turns, whole-batch validation,
silent control writes. One reviewer claim remained unproven:

> M5d gets a wonderfully clean job: prove batching/invalidation
> efficiency — presentation-only turn → 0 Taffy, structural turn → at
> most 1 Taffy pass, no-op turn → 0 repaint.

The M5c appliers already classified mutations (structural →
`structure_dirty`, presentation → control paint rebuild), but the
renderer's steady-state path **always** re-emitted the display list on
every render. The three claims were unprovable: a no-op turn could not
cost zero repaints because *every* render repainted.

## Decision

### 1. Two dirty flags gate `render`

- `structure_dirty` (existing): full Taffy pass + display-list
  re-emission.
- `presentation_dirty` (new): recompute styles, patch the cached tree's
  paint fields, re-emit the display list — zero Taffy.
- Neither: **paint the cached display list unchanged**. The frame is
  byte-identical; `layout_stats()` records no pass and no repaint.
  Painting still happens (the caller needs pixels); what is skipped is
  the re-derivation.

`run_layout` and `repaint_presentation` clear the flag (a full pass or
a repaint subsumes pending presentation changes). `rebuild_control_presentation`
sets it: every presentation mutation that re-emits control paint is
dirty by construction — editing, IME composition, drag selection, and
reactive `SetControlValue`/`SetControlDisabled` all funnel through it.

### 2. The invariant

> Renderer work is a function of what changed, not of what was
> requested. A render nobody dirtied is free; a presentation change
> never pays Taffy; however many structural changes accumulate between
> frames, one render pays one pass.

Scroll offsets were already baked into the cached tree; wheel and
programmatic scrolls now mark the presentation repaint they always
implicitly required.

### 3. Interaction dirtiness is precise, not conservative

Before M5d, any hover/press/focus change implied a full display-list
re-emission on the next render, because `repaint_presentation`
recomputed styles unconditionally. Now a memoized probe
(`interaction_paint`) scans the cascade for interaction selectors
(`:hover`/`:focus`/`:active`; the UA sheet is empty). Without one:

- hover/press changes dirty nothing (stateful selector matching cannot
  change any computed style);
- focus transfers dirty only when a **control** is involved (the
  caret's editor paint reads focus) or an interaction selector exists.

The memo drops on every structural change (`mark_structure_dirty`);
a stylesheet loaded after render both forces the full pass it always
documented (see below) and recomputes the probe.

### 4. Fixes the gating exposed

Steady-state renders used to repaint unconditionally, which silently
absorbed two latent gaps:

- `load_stylesheet` never marked anything dirty; a sheet loaded after
  the first render only took effect as a side effect of the next
  unconditional repaint (and its layout-affecting declarations were
  never re-laid-out, contradicting the documented "forces a full
  relayout"). It now marks structure dirty.
- `enable_tailwind` after a render had the same shape; it now marks
  structure dirty too.
- An oversized event payload was silently swapped for an empty payload
  (`unwrap_or_default`) in the turn mapper; the budget contract says
  reject. It now records a diagnostic and skips the turn.

### 5. What QuickJS still never decides

Invalidation classification stays entirely in the appliers (ADR 0017);
M5d only made the renderer's *consumption* of that classification
honest. The mutation vocabulary is unchanged.

## Consequences

- The three claims are pinned by tests (`m5d_*`):
  presentation-only turn → 0 passes / +1 repaint; structural turn →
  exactly +1 pass however many bindings changed / +0 repaints; no-op
  turn (handlers ran, state committed, empty diff) → byte-identical
  frame, +0/+0.
- Batching across turns is pinned: N queued turns settle in one render
  at one Taffy pass.
- Hover over a document without stateful selectors is free — relevant
  for dashboards that restyle on data, not on pointer state.
- A frame returned from an idle render is byte-identical to the
  previous frame; hosts may present it or skip presentation (damage
  tracking stays a host concern, M6+).
- The repaint counter (`LayoutStats::repaints`) changes meaning
  slightly: it now counts *necessary* presentation re-emissions, not
  every steady-state render. Fixture digests are unaffected.
