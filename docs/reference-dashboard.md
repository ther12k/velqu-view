# Reference dashboard (M7)

`examples/reference-dashboard` is the M7 reference application and the
conformance fixture for VelquView's documented public surface. A
developer can run it, interact with it, inspect it, and safely restyle
it using only that surface; the public-API conformance tests
(`crates/velqu-view/tests/reference_dashboard.rs`) and the visual
baselines drive these same files.

This document is the **reference profile**: what the fixture actually
uses, and every deferred feature it deliberately works around. If the
example needs something outside the profile, that need becomes a
separately named profile-change proposal — it is never smuggled in.

## Running it

```sh
velqu-lab --headless --tailwind --reactive --size 1280x800 \
  --frames 2 --out out --inspect examples/reference-dashboard
```

The app directory is `index.html` plus every `*.css` sorted by name
(cascade order): `app.css` only. Interactive runs use the same command
without `--headless`.

## What it demonstrates

A small operations dashboard over a fixed local dataset (six gate
shipments): navigation rail, summary-card row, searchable records list,
selected-record detail panel, and an editable name/notes form in
ordinary layout — no modal. Every visible control either works or is
clearly disabled (Save while nothing to save).

## Bindings used (reactive v0 profile)

| Surface | Used here | Deferred (distinct diagnostic) |
|---|---|---|
| Directives | `vx-state`, `vx-text`, `vx-show`, `vx-model` | `vx-if`, `vx-for`, `vx-key`, `vx-computed` |
| Attribute bindings | `:class`, `:disabled` | `:value`, `:checked`, `:style` (supported by the runtime, not needed here) |
| Event handlers | `@click`, `@input` | `@change`, `@submit`, `@keydown`, `@keyup` (supported by the runtime, not needed here) |

Semantics per ADR 0016/0017: `vx-model` writes before `@input`
handlers run; state is plain data on dotted paths; the sandbox exposes
full ECMAScript minus `eval`/`Function`/wall-clock `Date`.

**Rows are authored, not generated.** `vx-for` is deferred, so the
records list and the per-record detail panels are an explicitly
authored set of six, filtered through `vx-show` predicates over
`search` (plain JS `String.prototype.includes`). This is a documented
limitation of the fixture: a modest, explicitly authored row set is the
supported way to build lists today; it is not an endorsement of
hand-unrolling large datasets.

**Conditional classes are complete alternatives.** Every `:class`
expression names whole token sets for both branches — selected row
(`bg-indigo-50 border-indigo-400` …) vs unselected (`bg-white
border-slate-200` …), enabled Save (`bg-indigo-600 text-white`) vs
disabled (`bg-slate-200 text-slate-400`). No branch depends on
"half-removing" a utility from the other branch.

## Utilities used (Tailwind v0 profile)

All visible styling compiles through the normal utility pipeline — the
reference run produces **zero diagnostics**; nothing disappears
silently. Families used (representative tokens, not exhaustive):

- Flex/grid layout: `flex`, `flex-1`, `grid`, `grid-cols-3`, `gap-4`,
  `items-center`, `justify-between`, `block`
- Spacing/size scale: `p-2…p-8`, `px-2/3`, `py-2/6`, `mt-1…mt-8`,
  `mr-6`, `w-16/24/64/72/96`, `w-full`, `h-3/20/80`, `h-full`
- Color + background: `bg-white`, `bg-slate-100/200/900`,
  `bg-indigo-50/600`, `bg-emerald-600`, `bg-red-500`; `text-white`,
  `text-slate-400…900`, `text-indigo`-family accents, status colors
  (`text-emerald-600`, `text-amber-500/600`, `text-red-500`)
- Borders/radius: `border`, `border-slate-200/300`,
  `border-indigo-400`, `rounded`, `rounded-md`, `rounded-lg`
- Typography: `text-xs…text-3xl`, `font-medium/semibold/bold`,
  `text-left`, `text-right`, `text-center`
- Overflow: `overflow-y-auto` — the records card scrolls

Known profile refusals exercised during development (each produced its
targeted diagnostic, and the classes were replaced): `border-dashed`
(border-style supports `solid`/`none` only) and fractional widths
`w-2/3`, `w-1/6`, `w-1/2` (not in v0; replaced with `w-72`, `w-16`,
`w-24`).

## The author-CSS boundary, stated plainly

Tailwind **variants** (`hover:`, `focus:`, `dark:`, `md:` …) are
deferred in the v0 profile and are diagnosed, not compiled. Where the
dashboard needs an interaction state, it uses author CSS in `app.css`
restricted to the interaction paint properties (ADR 0011):

```css
.vv-nav:hover   { background-color: #1e293b; color: #e2e8f0; }
.vv-row:hover   { background-color: #f1f5f9; }
.vv-input:focus { border-color: #6366f1; }
```

These properties (background/color/border-color/border-style/
border-radius/cursor family) repaint without relayout. Author CSS for
anything structural (layout, sizing, non-paint properties) is outside
this boundary and belongs to the named CSS profile (ADR 0006), which
this fixture does not extend.

`vv-`-prefixed classes are the document's own author-hook namespace
(ADR 0023): the utility compiler skips them (never compiles, never
diagnoses); unknown unprefixed utilities still diagnose. They carry no
declarations from the utility pipeline — their styling comes from
`app.css` or from utilities listed alongside them.

## Controls and inputs used

`<button>` (nav, row select, Save with `:disabled`), `<input>` (search,
driver name via `vx-model`), `<textarea>` (notes via `vx-model`),
`<label for>` (static text pairing). Editing is grapheme-safe with
pointer-capture selection (ADR 0012); IME remains a separately tracked
validation item and is not claimed by this fixture.

## Data

Fixed, deterministic, local: the six-shipment dataset lives in the
`vx-state` initializer in `index.html`. No server, network, auth, or
persistence; no images or other binary assets.

## Visual baselines

Five application states are frozen as reviewed raster digests and
pinned in `crates/velqu-view/tests/reference_dashboard.rs`
(`M7_*_DIGEST`). Each digest was regenerated, its PNG inspected, and
only then accepted:

| State | What it shows |
|---|---|
| `INITIAL` 1280×800 @1× | default selection TRK-8841, 6/0/2 summary cards, Save disabled |
| `EDITED` | TRK-2210 selected, name edited (focused field), Save enabled, unsaved = 1 |
| `EMPTY` | search `zzz`: 0 records, no-results block, detail state intact |
| `SCROLLED` | records card wheel-scrolled (paint-side), selection still highlighted |
| `SMALL` 800×600 @1× | rail + summary + detail intact; the records list narrows |

The `SMALL` boundary, stated plainly: at 800×600 the fixed-width detail
panel and rail leave the records list a narrow column whose text wraps.
This viewport is a stable layout baseline, not a responsive claim —
Tailwind variants (`md:`) are deferred in the v0 profile, so the
fixture does not restyle itself per viewport.

Digests are regenerated with an intentional-change procedure, never
silently:

```sh
cargo test -p velqu-view --test reference_dashboard m7_regenerate \
  -- --ignored --nocapture   # prints digests, writes /tmp/ref-states/*.png
```

Review the PNGs, then pin the printed digests as the new constants.

## Developing against it

Interactive run (renders a real window):

```sh
velqu-lab --tailwind --reactive --size 1280x800 examples/reference-dashboard
```

Add `--watch` (native notifications) or `--watch=poll` to hot-reload.
The dev loop uses the transactional reload APIs (ADR 0021/0022):

- **CSS edit** → `reload_stylesheets`: styling publishes only when it
  accepts; model values, selection, focus, and live scroll state
  survive (pinned by `m7_css_reload_preserves_live_state`).
- **HTML edit** → `reload_bundle` (document + sheets as one
  transaction): a candidate that fails acceptance (e.g. a reactive
  initializer that throws) is rejected and the running application
  stays usable on its generation; a valid one publishes fresh
  source-defined state (pinned by
  `m7_full_reload_rejection_then_publication`).

Run `--inspect` headless to print the inspector record (generations,
layout passes, repaints, turns, display items) alongside the frame
digest; `--out DIR` writes PNGs. The headless digest is byte-identical
to the test harness's, which is what makes the frozen baselines in
`crates/velqu-view/tests/reference_dashboard.rs` reviewable from
either path.

Restyling guidance: change utilities in `index.html`, or interaction
states in `app.css` (restricted to the interaction paint properties
above). Unknown unprefixed utilities are diagnosed, never silently
dropped; `vv-`-prefixed classes are the document's own hook namespace
(ADR 0023) and carry no utility declarations.

## Acceptance

The M7 acceptance matrix — initial dashboard, filter and clear search,
select and edit with Save disabled-when-saved, hover/focus/scroll with
zero Taffy passes, CSS-only reload preserving live state, rejected
full-bundle reload leaving the app usable, and successful full reload —
is encoded in `crates/velqu-view/tests/reference_dashboard.rs` and the
visual baselines, which drive this directory's real files through the
public API only (`data-vv-test` hooks for identity; pointer/keyboard/
wheel input derived from those hooks and returned geometry, never fixed
coordinates).
