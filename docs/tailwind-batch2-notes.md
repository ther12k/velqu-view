# Tailwind batch 2 — synthesis vs. renderer support

Batch 2 added interaction variants, fractional sizes and insets, negative
insets, cursor utilities, and `border-[#hex]`. Later renderer work unlocked
more utilities, but emitting a CSS declaration is not proof VelquView applies it.

## Which target to use

- `compile_utilities` is the renderer target, used by VelquView and `tw-audit`.
  It diagnoses `position`, inset offsets, and `z-index` without emitting them.
  The earlier profile spec lists positioning as a goal; ADR 0007/0009 defer it.
  `apply_declaration` has no matching properties, the Taffy projection does
  not set position or inset, and paint does not sort by `z-index`.
- `compile_css_utilities` is the standalone CSS target. It still synthesizes
  `static`/`relative`/`absolute`, inset utilities (including negative,
  fractional, and arbitrary px forms), and the `z-*` scale. `fixed` and
  `sticky` remain outside the synthesizer's subset. The standalone output is
  not a statement of compatibility with VelquView; `velqu-css-check` checks
  the renderer profile.
- Both targets emit `animation` declarations for the named presets. Only
  standalone CSS includes their `@keyframes`: VelquView skips those at-rules
  and drives `spin`, `pulse`, and `bounce` natively (ADR 0024).

## What runs in the renderer

- `hover:`, `focus:`, `active:`, and `focus-visible:` use interaction selectors
  restricted to paint properties (ADR 0011). `:focus-visible` matches
  keyboard-origin focus. Layout-changing variants are diagnosed.
- Outer `shadow-*` presets and `shadow-[…]` support single or multiple layers.
  Arbitrary shadows decode underscores as spaces and support hex and numeric
  `rgb()`/`rgba()` colors. RGB channels are 0–255; optional alpha is 0–1.
- Grab/grabbing cursors, `select-none`, `truncate` (single-line ellipsis), and
  negative px margins are supported. A leading dash in `-mt-[3.5px]` is a
  value sign, just like in `-mt-2`.

## Still deferred

| Utility family | Renderer gap |
| --- | --- |
| `shadow-inner`, `ring-*`, `blur-*` | Only outer shadows are painted; Tailwind rings compose with CSS variables, and blur utilities use filters. |
| `disabled:*`, `group-hover:*`, `placeholder:*` | The renderer has no matching disabled/group state or pseudo-element support for these variants. |
| `sm:*`, `md:*`, responsive variants | The runtime CSS parser skips `@media` blocks. |
| `pointer-events-none`/`-auto` | Hit testing owns pointer routing; `pointer-events` is not applied. |
| `outline-none`, `tabular-nums` | Outline properties and `font-variant-numeric` are not applied. |
| Fractional `translate-*` (`-translate-x-1/2`) | Percentages would need to resolve against the element's own box; translation utilities currently emit px. |
