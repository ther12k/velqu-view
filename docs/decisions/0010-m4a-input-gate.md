# ADR 0010: M4a — the input gate (hit testing, interaction state, wheel)

Status: accepted (M4a)

## Context

Phases M0–M3 render documents; nothing responds to them yet. The scroll
milestone (ADR 0008) explicitly deferred wheel/input handling to the
input milestone. M4a is the slice that makes a rendered document
*respond*: the pointer selects things, the wheel scrolls, Tab moves
focus — with the invariant that **input never triggers layout**. Text
editing, IME, selection, and hover/focus *painting* stay in later M4
slices.

## Decision

### 1. Hit testing is the inverse of painting

`input::hit_at` walks the cached laid-out box tree with the same
ordering rules the painter uses, run backwards:

* **Later siblings win** over earlier ones and over their parent's own
  chrome (children are tested before the parent's background).
* **Clip scopes exclude**: a scroll container's children are only
  reachable when the point is inside the container's padding box; the
  clip test uses the *incoming-space* point, so clipped-away content is
  never hit even when the raw geometry would contain the point.
* **Scroll offsets translate on descent only**: child geometry lives in
  the container's content space, so a container's applied (clamped)
  scroll offset translates the point when recursing into children — not
  for the container's own border/padding test.
* The document-level scroll offset maps the caller's viewport-space
  point into page space once, at the root.

The result is a [`HitTarget`](`element_id`, `tag`,
`scroll_container`); the element `id` is the public interaction
identity, matching scroll targets (ADR 0008).

### 2. Interaction state is runtime presentation state

Hover, pressed, and focus live on `VelquView` next to the scroll
offsets: they change no layout facts and are lost with the document
(`load_html` resets them). They produce an ordered event log drained via
`take_events()`: `PointerEnter`/`PointerLeave`/`Click`/`FocusChanged`
(plus `Scrolled` from ADR 0008's machinery). Events carry element ids —
the renderer's public surface stays id/address-based (ADR 0005); no
node pointers leak out. Click requires press and release on the same
id; a click also focuses the clicked element when it has an id. Tab
cycles the elements that carry an `id`, in document order, wrapping.
Nothing paints differently for hover/focus yet — that is the M4b
styling slice, which is why the shell does not redraw for them.

### 3. Input reads a cached layout; wheel bakes, never invalidates

`render`/`layout_facts` already store the last laid-out tree
(`last_laid` + `last_viewport`). Input APIs validate the cached
viewport (same dimensions and scale) and answer from it — a pointer
move, press, or wheel is O(hit-walk), with **zero layout passes** (pinned
by `wheel_scrolls_nearest_container_with_zero_layout`).

The one subtle point is consecutive wheel events. A real pointer
delivers many scroll events between frames; if each wheel invalidated
the cached layout until the next render, all but the first would be
silently dropped. Instead, a wheel that changes an offset **bakes** the
clamped offset into the cached tree (`bake_scroll`): the target
container's `applied_scroll` — or the document root offset — updates in
place, mirroring exactly what the next render's `apply_scroll_offsets`
will compute from the stored request. The cache stays valid, events
accumulate, and the next render reproduces the same pixels with one
ordinary layout pass.

### 4. Wheel semantics are browser-signed device px

`VelquView::wheel(viewport, x, y, dx, dy)` takes **device px** with
browser sign convention: positive dy scrolls forward (view moves down),
positive dx scrolls the view right. The gesture targets the nearest
scrollable ancestor (`overflow: auto`/`scroll`) of the element under
the pointer, else the document-level scroller; the delta is clamped
centrally against that container's extent (ADR 0008's single clamp
site). Offsetting only, never chaining: a container at its scroll end
does not hand the remainder to the page (scroll chaining is deferred).

The shell converts platform deltas: winit's are opposite-signed
("positive = content moves down", per winit's docs and both the X11 and
Wayland backends), so the shell negates, and `LineDelta` notches scale
by a 40 px line height (`WHEEL_LINE_PX`). `PixelDelta` (trackpads)
passes through negated as exact px.

### 5. The shell forwards, the view decides

velqu-shell maps `CursorMoved` → `pointer_move`, `CursorLeft` →
`pointer_exit`, left `MouseInput` → `pointer_press`/`pointer_release`,
`MouseWheel` → `wheel` + dirty redraw, Tab → `focus_next`, Escape →
close. It tracks the last pointer position (NaN until the first
`CursorMoved`, so early clicks hit nothing) and builds the input
viewport from the live window size. Input before the window exists or
while minimized is dropped. Cursor shaping (pointer over links) waits
for the CSS `cursor` property — not in the profile, so not faked.

## Consequences

* A document is now interactive at the granularity of ids: hover/focus/
  click/wheel events flow out; scroll position responds to the wheel
  with no relayout. This is the seam M4b (hover/focus styling, e.g.
  `:hover`) and M4c (editing/IME/selection) build on.
* Hit testing inherits the frozen layout profile exactly; there is no
  second geometry source to drift. Regions outside the layout profile
  (transforms, z-ordering beyond DOM order) hit-test the same way they
  paint.
* Focus is id-based and document-ordered — adequate for dashboards;
  DOM order is not tab order semantics (no `tabindex`) and is recorded
  as a limitation.
* `pointer_exit` covers the window-exit case; per-element `pointerout`
  bubbling and capture phases are out of scope for v0.
