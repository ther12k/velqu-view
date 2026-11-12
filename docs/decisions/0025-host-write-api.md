# ADR 0025: Host-initiated state writes — the data-in seam

Status: accepted (host-write-api branch)

## Context

ADR 0017 froze a single direction for reactive state: state changes
enter **only** through event turns — a user interaction pumps a batch,
a `vx-model` write or handler mutates the candidate, bindings
reevaluate, mutations commit atomically. `reactive_state()` is
deliberately read-only; no host path reaches `set_path`.

That boundary is correct for documents, but a host that owns external
state — the process rendering a clock, media metadata from an OS
service, sensor readings — has no way to surface it. The workarounds
are all bad: reloading the document per tick re-parses and re-lays-out
on every clock second; synthesizing `ValueChanged` events is
impossible by design (`ElementHandle` is opaque) and would lie about
provenance anyway; baking values into the HTML source re-parses per
change.

The first real consumer is a notch-bar overlay (lekuvo) whose host
process owns the window, the media session, and the mascot state
machine, while the document owns presentation. It needs to push
`clock`, `media.title`, and `expanded` continuously.

## Decision

### 1. `VelquView::write_state(path, value)` runs a real reactive turn

The host write enters through the same `ReactiveMachine` machinery an
event turn uses: `prepare_host_write(path, value)` is `prepare_inner`
with the model write supplied by the host and no payload or handlers.
The candidate state is snapshotted, `set_path` applies the write
(creating intermediate objects), **all** bindings reevaluate, outputs
are normalized and diffed into the mutation batch, and only a fully
valid batch commits. Every budget, the atomicity, the non-reentrancy,
and the rollback semantics of ADR 0017 apply unchanged. A rolled-back
write is a no-op surfaced as `HostWriteError::RolledBack(reason)`; the
reason also lands in `reactive_diagnostics()`.

`prepare`'s `model_write` generalized from `(&str, &str)` to
`(&str, ReactiveValue)`: host writes carry every value kind (numbers,
bools, arrays, objects), not just the strings a control produces.

### 2. Writes are changes, not loads: transitions run

`write_state` applies its mutations with transitions enabled — a host
write that flips a `vx-show` bound element plays its `vx-transition`
phases exactly as a click-driven flip would. Turn zero remains exempt
(ADR 0024): the pending initial batch, when a write arrives before the
first pump, applies first without transitions so a write's outputs are
never clobbered by a stale initial batch (and an initially hidden
element never flashes a leave animation).

### 3. No events out: the no-feedback-loop contract holds

A write applies mutations silently. `SetControlValue` updates control
runtime state without synthesizing a `ValueChanged`; nothing a write
does is observable in `take_events()`. Hosts poll `reactive_state()`
for echo purposes and never risk re-entrancy from their own writes.

### 4. `NoReactiveDocument` mirrors `reactive_state`

Writing into a view with reactive disabled, no document, no reactive
markup, or a machine awaiting reload is `HostWriteError::NoReactiveDocument`
— nothing ran. The write API never enables reactive implicitly:
turning the pipeline on remains the host's explicit act.

## Consequences

* Hosts own the cadence: a per-second clock write is one bounded turn
  (~one clone of plain state, one binding pass), not a reload.
  Fixture-class documents stay microsecond-cheap.
* `ReactiveValue` is re-exported from `velqu_view` so hosts name the
  value type without depending on `velqu-reactive` directly.
* Determinism is preserved: identical write schedules produce
  identical frames (pinned by `tests/host_write.rs`), because the
  engine still never reads wall time.
* The inspector turn trace remains event-driven; host writes bump
  `state_revision`/`reactive_turns` but do not synthesize trace
  triggers. If a host-write trace becomes necessary, it extends the
  tracer without touching this contract.
* The seam is deliberately narrow: one path, one value, one turn. Bulk
  hydration, computed state, or host-side watchers stay out until a
  consumer demonstrates the need.
