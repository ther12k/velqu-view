# ADR 0028: Native keyboard focus and animation frame pacing

Status: accepted

## Context

The native input gate (ADR 0010) originally cycled through DOM elements
with IDs, later adding editable controls (ADR 0012). DOM membership alone
is not enough: an input hidden by `vx-show` can still exist in the DOM.
The shell also offered no reverse traversal or keyboard path for a focused
button's click action.

ADR 0024 calls for animation redraws at roughly 16 ms intervals. The shell
requested a redraw while installing each wait deadline, so the request could
wake the event loop before the deadline and immediately schedule another.

## Decision

- Keep keyboard input on the Rust side. Traverse in document order in either
  direction, skipping hidden and disabled targets. Preserve visible ID-based
  targets and include native buttons without an ID. Reactive hides clear
  focus and IME immediately, even during a leave transition. CSS hiding is
  known after the layout cascade; keyboard input does not initiate layout.
- A focused, enabled native button can enqueue the same `Click` event as
  pointer activation. The shell routes Shift+Tab backward and activates with
  an initial, unmodified Enter or Space press. Editors keep their existing
  Enter and Space behavior. The host still owns event draining and dispatch
  (ADR 0019).
- Keep an animation deadline in the shell. Request a scheduled redraw only
  when the deadline is due; an early wake must not advance that deadline or
  request an animation frame. Input, resize, and accepted reloads can still
  redraw immediately. The shell advances the engine by actual elapsed time
  on redraw, not a fixed per-frame increment, and never queues catch-up
  frames. Idle and occluded animation loops wait; an exit deadline can
  wake them independently.

This corrects the focus traversal and activation contract in ADR 0010/0012
and the frame-pump clause in ADR 0024 without changing offscreen rendering
or the virtual animation clock. It does not add a platform accessibility
tree or qualify live IME behavior.

## Verification

The core path is testable through the public focus and input APIs. The shell
also has a live Lab regression: a 96×64 looping pulse must present more than
one frame, but no more than 70 in a one-second Xvfb session. This tests the
window event loop; headless animation stepping cannot detect a redraw loop.
