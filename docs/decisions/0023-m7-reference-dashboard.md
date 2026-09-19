# ADR 0023: M7 — reference dashboard and the reserved `vv-` author-hook namespace

Status: accepted (M7)

## Context

M7's exit criterion: a developer can run, interact with, inspect, and
safely restyle a useful reference dashboard using only VelquView's
documented public surface — and the shipped example itself is the
conformance fixture. That example needs a place for two kinds of
classes the utility compiler must not touch:

1. **Semantic/test hooks.** `data-vv-test` (ADR 0005) keys identity for
   fixtures; real documents also want stable class names for their own
   structure — a rail, a row, a selected row — without each name being
   a utility.
2. **Interaction-state styling.** Tailwind variants (`hover:`, `focus:`)
   are deferred with a distinct diagnostic in the v0 profile (ADR 0009).
   Author CSS restricted to the interaction paint properties (ADR 0011)
   is the supported equivalent.

Before this ADR, a document using a `vv-…` class through the Tailwind
path collected an *unknown utility* diagnostic for each one. The only
workarounds were renaming hooks to collide with no utility (fragile) or
disabling diagnostics (forbidden — profile refusals must be visible).

## Decision

### 1. `vv-` is a reserved author-hook namespace

Classes whose name starts with `vv-` are skipped by
`velqu-tailwind`'s utility compiler — never compiled, never diagnosed.
This is a documented convention, not a disabled check: unknown
**unprefixed** utilities still diagnose exactly as before. The prefix
pairs with `data-vv-test` as the document's own naming plane; the
compiler owns everything else.

Rule for authors: `vv-` classes carry no declarations from the utility
pipeline. Anything they style must come from the document's author CSS
sheets, or from utilities listed alongside them in the same `class`
attribute.

### 2. The reference dashboard is `examples/reference-dashboard`

One app directory (`index.html` + `app.css`), exercised by the lab CLI
and the public-API conformance tests:

- **Structure**: navigation rail, summary-card row, searchable records
  list, selected-record detail panel with an editable name/notes form.
  The detail panel is ordinary layout — no modal.
- **Rows are authored, not generated.** `vx-for` is deferred in the
  reactive v0 profile; the dashboard ships a modest, explicitly
  authored set of rows filtered through supported bindings (`vx-show`
  predicates over `search`). This is a documented limitation of the
  fixture, not an endorsement of hand-unrolling large datasets.
- **Conditional classes are complete alternatives.** Every `:class`
  expression names whole token sets for both branches (selected /
  unselected rows, enabled / disabled Save). No branch relies on
  removing half a utility's effect.
- **Tailwind claim is executable.** The dashboard's visible styling
  comes from compiled utilities on the normal pipeline path — zero
  diagnostics, nothing silently disappearing. Deferred variants are
  replaced by author CSS restricted to the interaction paint
  properties (`.vv-nav:hover`, `.vv-row:hover`, `.vv-input:focus` in
  `app.css`); that boundary is stated in
  [`docs/reference-dashboard.md`](../reference-dashboard.md), not left
  implicit.
- **Scrolling is real.** The records card is `overflow-y-auto` inside a
  bounded column, so wheel scrolling (ADR 0008/0010) is part of the
  fixture's acceptance, not an afterthought.

### 3. Out of scope

The dashboard is fixed local data. No server, network, auth,
persistence, or new dependency surface (no crawler/async builder/remote
watcher). Capabilities the dashboard needs but the profile lacks
become separately named profile-change proposals — never silent
additions smuggled through the fixture.

## Consequences

- `compile_utilities` skips `vv-`-prefixed names before matching; a unit
  test pins that the skip is prefix-scoped and that unknown unprefixed
  utilities still diagnose.
- The M7 acceptance matrix (conformance test + visual baselines) drives
  this example through the public API only — the fixture and the tests
  share `data-vv-test` hooks and `vv-` styling classes.
- `docs/reference-dashboard.md` maintains the reference profile: the
  utilities, bindings, controls, and asset formats the example actually
  uses, plus every deferred feature it works around.
