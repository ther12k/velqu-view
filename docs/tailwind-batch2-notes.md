# Tailwind batch 2 — renderer-side limits

Batch 2 of the tailwind gap work (follow-up to `feat(tailwind): compile
color opacity modifiers, positioning, px arbitrary values, and animation
keyframes`) added the interaction variants, negative insets, fractional
sizes/insets, `cursor-*`, and `border-[#hex]` to `velqu-tailwind`. This
file records the utilities that **stay deferred because the renderer does
not implement them yet**, with the evidence checked before implementing —
so a future renderer slice knows exactly what to land to unlock them.

## What was checked before implementing

- **Interaction pseudo-classes** (`crates/velqu-view/src/css.rs`,
  `style.rs`, ADR 0011): the renderer's selector parser accepts a frozen
  set `["hover", "focus", "active"]` and the cascade matches
  `Compound[Class, PseudoClass]` against `InteractionState`
  (`hover_path`/`active_path` ancestor chains, exact `focus`). Interaction
  rules are **paint-only**: any other property inside a stateful rule is
  deferred at runtime with a diagnostic. The compiler therefore wraps
  `hover:`/`focus:`/`active:` utilities in
  `.hover\:class:hover`-style selectors and validates every declaration
  against `INTERACTION_PAINT_PROPERTIES` (the frozen list in
  `velqu-tailwind`) at build time.
- **Colors** (`crates/velqu-view/src/color.rs`, `style.rs::parse_color`):
  `#RGB`/`#RRGGBB`/`#RRGGBBAA` hex, 3-digit hex expansion, `rgb(r, g, b)`
  without alpha, and a handful of names. **No `rgba()` function** — an
  alpha must be an 8-digit hex channel (`compose_alpha` in the compiler
  already produces that shape for opacity modifiers).

## Still deferred, waiting on renderer work

| Utility family | Renderer gap | Unlock |
| --- | --- | --- |
| `shadow-*`, `shadow-[…]`, `ring-*` | `apply_declaration` has no `box-shadow` handler; `ComputedStyle` has no shadow field; the painter draws no shadows | Add `box-shadow` parsing + a painter pass. When it lands, the arbitrary form needs underscores → spaces and `rgba(r, g, b, a)` args converted to `#RRGGBBAA` (or emitted as-is if the parser grows `rgba()` support) |
| `focus-visible:*`, `disabled:*` | `:focus-visible`/`:disabled` are outside the frozen `INTERACTION_PSEUDO_CLASSES`; such rules are skipped with `UnsupportedPseudoClass` | Grow the frozen set + state tracking; the compiler's `VARIANT_PSEUDO_CLASSES` table is the single place to extend |
| `sm:*`, `md:*`, responsive | `@media` blocks are skipped wholesale (M2a diagnostics) | Media-query support in the css parser + cascade |
| `group-hover:*`, `placeholder:*` | No group/ancestor-state or pseudo-element matching | Selector engine work |
| `cursor-grab`, `cursor-grabbing` | `CursorStyle` is `Auto/Default/Pointer/Text` only (`style.rs`) | Add grab variants + shell cursor mapping. `cursor-pointer` already compiles |
| `pointer-events-none`/`-auto` | No `pointer-events` property in `apply_declaration`; hit testing owns pointer routing | Property + hit-test integration |
| `select-none` | No `user-select` property | Property + selection integration |
| `outline-none` | No outline properties at all | Focus-ring story first |
| `truncate` | No `text-overflow`; the renderer cannot paint an ellipsis. `overflow-hidden` + `whitespace-nowrap` already clip without the ellipsis | Text-shaping-level ellipsis support |
| `tabular-nums` | No `font-variant-numeric` | Font feature support in text shaping |
| `w-1/2`-style fractions on `translate-*` (`-translate-x-1/2`) | Percent-of-own-box transforms need layout-time resolution the transform engine does not own (existing deferral) | The `scale`/`inset` percentage paths show the pattern |
| `-mr-1` (negative margins) | Negative spacing stays v0-deferred (only insets negate this round) | Extend `NEGATABLE_INSETS`-style gating to the margin families |

Nothing above is silently dropped: each compiles to a named
`UtilityDiagnostic` that names the renderer gap.
