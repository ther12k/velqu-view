# ADR 0024: The transition engine — deterministic animations on a virtual clock

Status: accepted (M8)

## Context

Phase 1 closed with a renderer whose every frame is a pure function of
state: interactions repaint from cached geometry (ADR 0011), reactive
turns batch invalidation (ADR 0017/0018), and fixtures verify
bit-identical bytes (ADR 0002). Changes, however, land instantly — a
sidebar snaps open, a hover flashes. Motion is table stakes for a UI
toolkit, and the constraint set that shaped it is unusual:

* no browser engine: taffy computes layout, a CPU painter rasterizes;
* zero idle CPU is a contract (`ControlFlow::Wait`, never `Poll`);
* determinism is the differentiator — animation must not break fixture
  reproducibility;
* ADR 0011 already froze the rule that interactive state never lays out.

## Decision

### 1. Animations are cascade overrides, not a parallel style system

The engine (`velqu-view/src/anim.rs`) owns in-flight animations and
derives per-node `PropertyOverrides` — mid-flight values for width,
height, opacity, border-radius, the three color properties, and the 2D
transform. Both render paths apply the overrides **after** the cascade
(`layout::apply_animation_overrides`), before boxes are built or
patched. When an animation settles, its final sample equals the
cascade's target, so nothing shifts when the override goes away; the
finished value stays pinned in the map so a later recompute never sees
a phantom change to respawn.

### 2. Dual-tier invalidation extends the M4b rule

Paint properties (opacity, colors, radius, transform) animate through
`repaint_presentation`: zero Taffy passes per frame, exactly like hover
styling. `opacity` and 2D `transform` therefore join
`INTERACTION_PAINT_PROPERTIES`. Width/height animate through the
structure path — one Taffy pass per frame, the documented cost of
layout animation, chosen explicitly per transition (Tailwind's
`transition-all` covers both tiers).

### 3. The engine runs on a virtual clock; hosts own wall time

`AnimationEngine` never reads `Instant`. Its clock advances only by
explicit ticks (`VelquView::step_animations_by`). The shell derives
`dt` from real redraw timestamps; headless hosts step exact
discretes (`--step-ms 16`). Identical schedules produce identical
frames — CI animation fixtures run on the same guarantee as static
ones, proven by `tests/animations.rs` (identical hash vectors across
runs). Easing is a fixed-iteration Newton+bisection cubic-Bézier solver
(IEEE-exact arithmetic only); the spring curve uses libm transcendentals
and is deliberately unreachable from any v0 utility or profile property.

### 4. The differ spawns from on-screen truth

Interaction changes diff fresh cascade output against the cached box
styles. Structural class/style changes (`:class`, `:style`, and
vx-transition phase swaps) capture pre-change snapshots before mutating,
diff in the render's structure branch *before* layout applies the
change, and seed overrides at `t = 0` — the first frame always shows
the from-state, never a jump to the target. A running animation whose
target is unchanged is left alone; a changed target retargets from the
current on-screen value (CSS restart semantics, delay included). Turn
zero is exempt: initial classes and initial visibility are the
document's starting state, not changes — nothing animates on load.

### 5. Paint-time opacity, complete 2D transforms, and transformed hit-testing

The display list gains `PushOpacity`/`PopOpacity` (source-alpha
modulation at every primitive, images and glyph coverage included) and
generalizes `PushTransform` to a 2D affine `[a b c d e f]` (scroll
translation is the degenerate case; pure translations compose
bit-exactly, so existing fixtures are unchanged). 2D transforms support
independent axis scaling, rotation (`rotate-*` in degrees), custom anchor
points (`transform-origin: origin-*`), and translation.

Rotated shapes are rasterized in software using inverse-mapped local
sampling, keeping rendering 100% deterministic with zero anti-aliasing
variance across platforms.

**Transformed hit-testing:** Hit testing (`input::hit_at`) inverts the
element's 2D affine matrix ($M^{-1}$), mapping viewport coordinates into
the element's local coordinate space before checking padding and border
boxes. Scaled, translated, and rotated elements receive pixel-perfect
clicks and `:hover` states.

### 6. Looping animations (`animate-*`)

Tailwind preset looping animations (`animate-spin`, `animate-pulse`,
`animate-bounce`, `animate-none`) are supported via
`AnimationRepeat::Infinite`. Continuous rotation loops seamlessly through
360 degrees, while pulse and bounce oscillate back and forth. When the
window is minimized or occluded, `velqu-shell` pauses frame pumping in
`ControlFlow::Wait` to guarantee 0% background CPU consumption.

### 7. Alpine-style `vx-transition`

`vx-show` elements may declare phase classes
(`vx-transition:enter`, `:enter-start`, `:enter-end`, and the leave
trio; a bare `vx-transition` selects engine defaults mirroring
Alpine's). The compiler stores them as a static `TransitionPlan` —
no JS evaluation — and diagnoses a plan without a sibling `vx-show`.
The state machine renders the start frame, swaps to the end classes on
the next tick (the differ spawns from that rendered start state),
completes when the node's animations settle, then hides (leave) or
restores the base classes (enter). Direction reversals cancel the
in-flight phase and animate back from the current on-screen state.
Phase classes are injected into the `class` attribute; conflicting
utilities resolve by the generated sheet's first-use order (ADR 0009
semantics — phase classes should not fight the base classes).

### 8. The shell pumps frames only while animating

`velqu-shell` paces redraws at ~16 ms deadlines (`WaitUntil`, never
`Poll`) while `has_active_animations()` holds, then returns to pure
`Wait` — 0% idle CPU is preserved by construction, and an
`exit_after` deadline always outranks frame pacing.

## Consequences

* Tailwind's `transition-*`, `duration-*`, `ease-*`, `delay-*`,
  `opacity-*`, `scale-*`, `rotate-*` (incl. negatives), `origin-*`,
  `animate-*`, and `translate-x|y-*` families compile on named scales;
  `transform-origin` and `animation` join profile v0.
* `velqu-lab --headless --step-ms N` renders stepped animation
  sequences and verifies determinism by running the sequence twice on
  fresh loads and comparing every frame hash.
* Transformed hit testing evaluates element geometry in inverted local
  space, providing pixel-accurate interaction on transformed buttons,
  cards, and chevrons.
* Layout-affecting transitions are opt-in per element (the author wrote
  `transition-all` or `transition-property: width`) — pointer motion
  alone still can never re-layout (ADR 0011 intact: interaction rules
  remain paint-only).
* Animation state is keyed by DOM node ids and dies with the document
  generation; the virtual clock survives (schedule time, not document
  state).
