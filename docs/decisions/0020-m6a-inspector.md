# ADR 0020: M6a — the inspector reads recorded outcomes; it never reruns work

Status: accepted (M6a)

## Context

M6a.1 froze event ownership (ADR 0019): the pump takes a caller-owned
batch, so reactive processing and inspection can observe the same
input without competing to consume it. M6a proper builds the inspector
on that foundation. The reviewer's central constraint:

> The inspector reads recorded outcomes; it does not rerun work to
> discover what happened. … It explains what Velqu actually did
> without becoming another participant in application execution.

## Decision

### 1. Four record families, linked — not a fictional pipeline

`EventRecord` (an observed batch event), `TurnRecord` (one turn
attempt), `InvalidationRecord` (requested work + causes), and
`RenderRecord` (one completed render's actual deltas) link by
monotonic `seq` where a real causal relationship exists. The model
accommodates: events with no handler (an `EventRecord` with no turn),
initialization turns (trigger `None`), several turns per frame (each
attempt attributable, one render counting the completed work once),
presentation updates without any turn (hover/scroll invalidations),
and failed turns (attempt recorded, zero committed).

### 2. Attempted ≠ committed; requested ≠ completed

A turn records its proposed mutation count and kinds, its outcome
(`Committed { count }` / `Rejected` / `RolledBack`), and the state
revision before/after. A failed transaction cannot appear as a
successful UI update. Only a completed `RenderRecord` reports
pass-count deltas; a pending invalidation is exposed as pending, not
as work that ran.

### 3. Coalesced causes, bounded

Invalidatees carry bounded cause sets (`"reactive SetText"`,
`"hover"`, `"stylesheet app.css"`, `"viewport"`, …) plus a truncation
count — the last cause never silently wins. This is the "why did
layout happen" panel: cause (what changed), classification
(presentation/structural), outcome (what a completed render actually
did).

### 4. Byte- and count-bounded retention, honest loss

`InspectorLimits { max_records, max_retained_bytes, max_record_bytes,
max_preview_bytes, capture_values }` apply while constructing records.
Eviction is oldest-first; seqs never renumber; the summary exposes
`first_retained`, `evicted`, and `truncated`. Overflow may discard
observational records — never application events (the caller-owned
batch is untouched by retention).

### 5. Metadata by default

Value-carrying records store target identity, operation, and lengths;
full text (event values, previews) only under the explicit
`capture_values` opt-in. No permissions subsystem — a capture policy.

### 6. Snapshots are observational

`inspector_snapshot(viewport, selection)` uses shared access to
cached, recorded data. Its implementation and regression tests
establish that it does not execute application work: no pumping,
draining, rendering, layout, or JS-runtime access, and the logical
clock and deterministic random stream cannot advance through
inspection. (The `&self` signature is a guardrail that reinforces
this, not proof of purity — shared references permit interior
mutability in general; the tests are the evidence.)
Geometry and styles come from the **cached** box tree (effective,
interaction-patched, as of the last paint — with the interaction flags
*now* shown alongside); a missing cache reports `NotAvailable`, a
different viewport `Stale`. Intermediate coherence is exposed, not
hidden: state revision vs layout revision, `awaiting_relayout`,
`awaiting_repaint`, and pending causes.

### 7. Capture is gated

Every recording hook no-ops when capture is disabled; cause strings
are never formatted just to be discarded. Cheap counters
(passes/repaints/turns/items) run regardless. Durations are
`Instant`-measured and recorded but never affect ordering; tests
compare only the deterministic fields.

## Consequences

* The observation gate is pinned: passive inspection causes no
  document layout, reactive execution, event consumption, or painting
  (`m6a_snapshot_reads_are_inert`, `m6a_capture_leaves_the_application_identical`).
* Replay of a batch is a new attempt record (ADR 0019 made replay
  explicit; the trace shows both attempts with distinct triggers).
* A stale batch pumped after a reload records its generation mismatch
  and cannot run turns against the replacement — pinned, reusing the
  existing identity/generation system.
* The lab's `--inspect` prints the snapshot and retained trace after a
  headless run — the developer surface for now; interactive panels,
  style editing, a JS console, replay controls, and remote protocols
  stay out of M6a scope.
* M6b's reload outcomes will extend the same record model
  (candidate built / swapped / rejected) rather than a parallel one.
