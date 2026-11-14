# ADR 0027: Physical springs — momentum-preserving transitions on the virtual clock

Status: accepted (host-write-api branch)

## Context

The transition engine (ADR 0024) drives every animation from a duration
and an easing curve: progress is a pure function of the virtual clock.
That model cannot express the feel the notch port needs — the pill
morph reads "laggy" — because retargeting a curve restarts the whole
curve from the new endpoints. When a user taps mid-morph, the pill
decelerates to zero, snaps to the new curve's start, and accelerates
again. Every retarget throws away the motion the user was watching.

Framer-motion's answer is a physical model: a damped harmonic
oscillator with state — position and velocity integrated per tick —
where the duration is emergent and a retarget only replaces the target.
Velocity carries through; the motion never resets. The work list called
for exactly this: `x'' = −k(x − target) − c·x'`, stiffness/damping/mass
exposed per transition.

ADR 0024 reserved a normalized `Easing::Spring` *curve* for this role
but never shipped it: a curve over a fixed duration cannot preserve
momentum, so it would have reintroduced the lag it was meant to fix.

## Decision

### 1. Springs are a driver, not a curve

`ActiveAnimation` gains a `TransitionDriver`: `Timed` (the existing
duration + easing path) or `Spring { spec, progress, velocity }`. The
spring integrates the *normalized* progress `s` (0 = `from`, 1 = `to`)
with semi-implicit Euler — velocity first, then position with the fresh
velocity — in fixed substeps of 1/240 s per tick, so any host tick
length integrates identically and no plausible stiffness destabilizes
the solver. Because `AnimProperty::write_override` already accepts any
`t` (springs overshoot), every animatable property — width, height,
opacity, radius, colors, transform — is spring-capable without new
override plumbing.

### 2. Momentum handover is the contract

A retarget re-anchors `from` at the current on-screen value (progress
restarts at 0) and converts the old spring's physical velocity into the
new span's units: `v' = span_old·v / span_new`. Scalar properties
(width, height, opacity, radius) carry momentum; per-channel properties
(colors, transform) restart at rest — channel-wise velocity transfer is
ambiguous and their retargets are rare. Timed→spring handovers start at
rest at the current sample.

### 3. Determinism: IEEE-exact arithmetic only

The integrator uses add/multiply/divide — no transcendentals (the
libm-variance caveat that fenced off ADR 0024's analytic spring
disappears). Identical host schedules produce bit-identical spring
states, so fixtures may depend on springs; the settle test is
`|s − 1| < 1e-3 ∧ |v| < 1e-3`, and two guards keep the frame pump
honest: non-positive damping normalizes to critical damping `2√(k·m)`
(so an authored `spring(300, 0)` cannot oscillate forever), and a 10 s
cap force-settles pathological parameters. Zero stiffness animates
nothing (target applies instantly, like `duration: 0`).

### 4. Authoring: `transition-timing-function: spring(k, d[, m])`

The timing-function slot holds either a curve or a spring — the last
declaration wins; each form clears the other. `transition` shorthand
accepts `spring(...)` in the curve position. Spawn paths (differ,
pending style diffs, vx-transition phases) pick the driver from the
spec; enter/leave machinery is untouched. `box-shadow` (first layer)
joins the animatable set at the same time — the pill morph transitions
shadow, radius, and size together.

## Consequences

* Mid-morph taps keep momentum: the pill morph stops being "laggy" by
  construction, not by tuning.
* Duration is no longer authored for springs — `duration-*` is ignored
  on the spring path (delay still applies).
* The ADR 0024 reserved `Easing::Spring` variant is removed; one spring
  model, not two.
* The engine's override rebuild now mutates spring state per tick; the
  virtual-clock determinism contract is unchanged (state is a pure
  function of the tick schedule).
