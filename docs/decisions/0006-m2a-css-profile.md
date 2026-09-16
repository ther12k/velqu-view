# ADR 0006 — M2a named CSS profile; the contract is tested, not grown

- Status: accepted (M2a, 2026-09-16)

## Decision

M2a implements a **named, frozen CSS subset** — the "M2a CSS Profile" — and
everything outside it produces a deterministic compatibility diagnostic
(`velqu css check` surfaces them; nothing is silently ignored). The profile
grows only by explicit profile revisions, never by "implement CSS until the
Tailwind examples look right".

## M2a CSS Profile

Selectors: type, class, id, universal; descendant and child combinators.
(Pseudo-classes, attributes, sibling combinators: later profiles.)

Cascade: specificity (ids/classes/elements) → source order → inline style;
`!important` is captured and outranks normal declarations of the same
element/property. Origin order: UA defaults < author sheets < inline.

Units: `px`, `%` (widths/margins/padding), `rem` (against the 16px root).
`em` is deferred.

Layout: `display: block | inline | none`; normal block flow with
margin/padding/border (all four sides + shorthand), `width/height`,
`min/max-width/height`. Documented simplifications: no margin collapsing
(adjacent margins add), percentage heights resolve as auto, content-box
sizing, inline boxes flatten into text runs.

Text: `font-size`, `font-weight` (numeric + normal/bold), `line-height`
(normal/number/px), `color`, `text-align` (left/center/right/start/end),
`white-space` subset (normal/pre/pre-wrap/nowrap). Supported text content is
the Latin/UI subset with space-separated words — complex shaping and bidi
are explicitly out of the M2a contract.

Colors: `#rgb`, `#rrggbb`, `rgb(r,g,b)`, a small named set,
`transparent`. Wide-gamut functions classify as **Normalized** (M3
conversion) per the velqu-tailwind tier model.

## Consequences

- Compatibility results are mechanical: `StyleDiagnostic` (cascade) and
  `CssDiagnostic` (parser) name the construct, source, and line.
- The profile gives the conformance fixtures a fixed target; fixtures test
  the contract, not incidental behavior.
- Growth is a reviewable event: a profile revision changes this ADR and the
  fixtures together.
