# ADR 0009: M3 — the v0 Tailwind-compatible pipeline (utilities → CSS → cascade)

Status: accepted (M3)

## Context

Phase 1 of VelquView is "HTML + Tailwind renders natively". The M0
contract fixed the pipeline shape: **Tailwind compiler → CSS → profile
check → renderer**, with the renderer core never hard-coding utility
names. M3 must make that pipeline real without a node toolchain or
network in the render path.

## Decision

### 1. The synthesizer lives in velqu-tailwind, not the renderer

`velqu_tailwind::compile_utilities(classes) -> TailwindBuild` maps
Tailwind v3-compatible utility classes to plain CSS text (one rule per
class, first-use order, plus a preflight-lite `* { box-sizing:
border-box; }`). `VelquView::enable_tailwind()` opts a document in: the
DOM's `class` attributes are compiled once per document load and the
generated sheet joins the cascade **first among the author sheets** —
so utilities still beat element rules (class specificity), while the
author's own class rules override utilities at equal specificity. That
matches Tailwind's layered-utilities semantics (v3 puts utilities in
`@layer utilities`; unlayered author CSS wins), and inline styles beat
everything as CSS specifies.

*(Correction during the phase-1 review: the first implementation
appended the generated sheet after the author sheets, which made
utilities un-overridable by author CSS at equal specificity — the
opposite of Tailwind's behavior. The order was flipped and is pinned by
`tailwind_author_css_overrides_utilities`.)*

This refines, not breaks, the "no utility names in the renderer" rule:
the names live in the designated compiler crate, and the renderer only
ever sees ordinary CSS.

### 2. v0 utility surface = the renderer's frozen profile

Supported: display (block/inline/flex/grid/hidden), flex
(direction/wrap/grow/shrink/flex-1…), grid (cols/rows 1–12, span,
start/end, auto-flow row|col, justify-items), gap, the 4px spacing scale
(p/m with all Tailwind side shorthands), sizing (w/h/min/max with scale,
full, auto, named max-w breakpoints), the full v3 color palette (22
families × 11 shades + white/black) for text/bg/border colors, the
typography scale (text-xs…9xl with paired line-heights, font weights,
text-align, leading, whitespace), borders (widths, solid/none,
gray-200 default color, per-side colors), rounded, and overflow. The
spacing scale is emitted as absolute px (4px per unit at Tailwind's
16px root default) rather than rem — a documented simplification while
the root font size is fixed.

Beyond it, diagnosed with guidance — never silent: variants (`hover:`),
arbitrary values (`w-[91px]`), fractional/decimal classes (`w-1/2`,
`p-2.5` — they need CSS escaping the v0 selector path does not compile),
`!important`, positioning (absolute/fixed/sticky/inset), shadows,
opacity, cursor, z-index, transitions/filters/rings, em-based tracking,
`margin: auto`, text decoration/case transforms, font families.

### 3. box-sizing became real renderer behavior

The pipeline's preflight requires `box-sizing: border-box` to mean
something, so the renderer now parses `box-sizing` and honors it in
Taffy (`BorderBox`), defaulting to `ContentBox` exactly as before.
Without the pipeline nothing changes; with it, `w-64 p-4` is a 256px
border box, as Tailwind authors expect.

### 4. Diagnostics are the checker, in-process

Every class the compiler cannot map is reported through
`VelquView::tailwind_diagnostics()` (and printed by `velqu-lab
--tailwind`). The profile classification API (ADR 0004's
`classify_declaration`/`classify_at_rule`) remains the standalone check
layer; a standalone `velqu css check` CLI on top of it is deferred — the
in-pipeline diagnostics already give authors the same verdicts where
they act on them.

### 5. SVG icon strategy (recorded, deferred)

Inline `<svg>` and `mask`/`background-image` icons are outside profile
v0 and are diagnosed as unknown/unsupported markup. Icons for v0 are
raster assets through the existing bounded `<img>` path (PNG/JPEG from
the host resolver, ADR 0008). A native SVG icon path remains open for a
later milestone if dashboards demand it.

## Consequences

* Phase 1 (M0–M3) renders HTML styled purely with Tailwind utilities —
  no CSS files, no node, no network — deterministically, with fixture
  baselines (`tailwind-hello`) and a dashboard example
  (`examples/tailwind-dashboard`).
* The palette is the Tailwind v3 default set; Tailwind v4's OKLCH values
  would enter as Normalized-tier declarations (converted to sRGB) only
  when real compiler ingestion lands.
* Real compiled-Tailwind ingestion (consuming a `tailwindcss` build
  output through the profile checker) remains the upgrade path; the
  renderer side needs no changes for it — only a better compiler behind
  the same seam.
* Rounded corners are computed but not yet painted (radius values land
  in ComputedStyle; rasterization of rounded rects is future work) —
  utilities compile and documents lay out correctly; corners render
  square. Recorded as a known visual limitation, not a profile gap.
