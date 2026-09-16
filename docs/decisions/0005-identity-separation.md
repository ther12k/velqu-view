# ADR 0005 — DOM identity, style identity, layout identity, and paint identity are distinct

- Status: accepted (M2a start, 2026-09-16)

## Decision

VelquView keeps four identity spaces separate:

| Space | Identity | Lives/dies with |
|---|---|---|
| DOM | `NodeId` (index into the parsed tree) | one document parse |
| Style | per-node `ComputedStyle` produced by cascade | one (DOM, stylesheet set) pair |
| Layout | box/fragment positions and sizes | one (style, viewport) layout pass |
| Paint | display-list items and their order | one frame |

No stage stores or exposes another stage's identity. Fixtures and external
tools must never observe `NodeId` either — they use fixture-owned keys
(`data-vv-test="card"` attributes) that survive parser and internal
representation changes.

## Why

Pseudo-states (`:hover`), Velqu Reactive mutations (M5), scrolling, partial
invalidation, and hot reload all change exactly one of these spaces without
touching the others:

- a hover state re-runs **style + layout + paint** for affected nodes, not a
  DOM reparse;
- a reactive `vx-text` mutation re-runs **style (for that node) + layout +
  paint**, keeping the same DOM identity so bindings keyed by node survive;
- hot reload (M6) replaces the **DOM** wholesale but wants to reuse the
  stylesheet set;
- scrolling re-runs **paint** (and sometimes layout of scroll contents), not
  style.

If one shared identity leaked across spaces, each of those features would
need to re-derive or re-map identities globally.

## Consequences

- The DOM is a pure tree (`NodeId` + nodes); it never holds layout boxes or
  computed values.
- The box tree/layout tree is a separate structure allocated per layout pass
  (M2a: rebuilt from scratch; invalidation comes later, behind this seam).
- The display list is a flat, ordered list built from the finished layout
  tree; the painter consumes it and makes no layout decisions.
- `data-vv-test` attributes are extracted at parse time into the DOM and are
  the only node-level identity exposed to fixtures (via the layout-facts
  schema).
