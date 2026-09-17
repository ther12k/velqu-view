# ADR 0011: M4b — interaction styling; interactive state never lays out

Status: accepted (M4b)

## Context

M4a made documents respond: hit testing, hover/focus/click events, and
wheel scrolling all answered from the cached layout with zero layout
passes. But nothing *painted* differently yet, scroll state was keyed by
element `id` strings (aliasing id-less containers with the document
scroller), and reviewers set the direction: M4b before M4c, and M4b must
prove one contract above all —

> Interactive visual state may change without changing layout truth.

## Decision

### 1. Stateful selectors with class-level specificity

CSS gains `:hover`, `:focus`, and `:active` as a frozen set. They
contribute class-level specificity (`.card:hover` = 0,2,0). Matching is
right-to-left as before, with the interaction state consulted for the
pseudo-class simple selector; unknown pseudo-classes are diagnosed at
parse time and their rules skipped — never silently ignored.

### 2. Hover and active apply through the ancestor chain

CSS `:hover` and `:active` are chain properties: when the pointer is
over a card's `<span>`, `.card:hover` must activate. The view keeps the
exact hit node for events and derives a path (target first, root last)
for style state — the reviewer's `hover_path`, and the same shape that
will make `:focus-within` trivial later. `:focus` matches only the
focused element exactly.

### 3. Interaction rules are paint-only, deferred otherwise

A declaration reached through an interaction selector may only change
paint: `background-color`/`background`, `color`, `border-color`,
`border-style`, `border-radius`, `cursor`. The frozen list lives in
velqu-tailwind (`INTERACTION_PAINT_PROPERTIES`) next to the profile
concepts; the cascade and the `velqu-css-check` CLI share it and cannot
drift — the checker reports non-paint declarations under interaction
selectors as unsupported with that guidance. At cascade time a deferred
declaration produces a deterministic diagnostic (deduplicated across
frames, surfaced via `style_diagnostics()`); the paint half of the rule
still applies. This is the line in the sand: `.card:hover { width:
500px }` would make pointer motion a layout invalidation, so it is
deferred with guidance until dirty-style→dirty-layout propagation is
deliberately designed.

### 4. Two-tier render: full layout vs presentation-only repaint

`render` dispatches on what changed:

* Structural change — document, stylesheets, or viewport — runs the full
  cascade + Taffy pass (with the current interaction state, so pixels
  are correct after resize too).
* Steady state recomputes styles with the interaction state, patches the
  cached box tree's paint-only fields (node styles, inline-run colors —
  runs carry their source element's NodeId), and re-emits the display
  list. Zero Taffy passes; `LayoutStats.repaints` counts these.

`layout_facts` always computes state-free (`interaction = None`): facts
are the structural truth, state only ever reaches pixels — "facts
identical, raster differs" is now a test-pinned invariant.

### 5. Runtime state is node-keyed and transplants across relayouts

Carrying the reviewer's carry-forward: scroll offsets, hover, pressed,
and focus are keyed by DOM `NodeId` — stable for one loaded document, so
scroll position survives resize and restyle relayouts, re-clamped
centrally at apply time; only `load_html` (the one sanctioned reset)
clears it. This also fixed a real M4a defect: an id-less scroll
container's wheel offset was stored under the empty-string key, aliasing
the document scroller. Events now carry a public `ScrollTarget`
(`Document` or `Element { id }`) so id-less containers are
distinguishable, and wheel change detection compares against the offset
the target was painted with (a stored raw request can exceed the current
clamp after a relayout).

### 6. Cursor is a property, not a shell guess

`cursor: auto | default | pointer | text` parses as an inherited,
presentation-only property (the freeze list covered it from the start).
`VelquView::cursor_under(viewport, x, y)` reads the hovered element's
*computed* cursor — inherited values included, so a card with `cursor:
pointer` covers its children; the shell maps it to the platform icon
(`Auto` resolves to the plain arrow). A cursor change alone neither
repaints nor lays out.

### 7. Focus origin recorded at the source

`FocusChanged` events and `focus_origin()` carry why focus moved:
`Pointer` (click), `Keyboard` (Tab), `Programmatic` (`set_focus`).
`:focus-visible` is not supported yet; when keyboard-only rings and
accessibility behavior land, the reason will already exist.

## Consequences

* The M4a invariant survives intact and stronger: hover, focus, active,
  cursor, scroll, and hit testing coexist while Taffy sleeps; the
  `layout_passes` counter is the contract's witness in tests.
* M4b exercises everything M4c (editing/IME) will rely on — focus
  identity, state transitions, style invalidation, cursor presentation,
  repaint scheduling — without text mutation, selection ranges, or
  composition state. M4c is planned as three sub-slices (editable
  controls; clipboard behind a shell interface; IME as a platform
  integration milestone).
* Patch-and-reemit repaint is deliberately naive: correctness-first,
  instrumented, trivially deterministic. Incremental restyling waits for
  evidence of need.
* Unknown pseudo-classes, deferred layout properties in interaction
  rules, and all other apply-stage diagnostics are now surfaced
  (`style_diagnostics()`) — apply-stage diagnostics were previously
  produced and dropped, which this milestone fixed.
