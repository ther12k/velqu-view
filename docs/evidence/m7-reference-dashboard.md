# M7 verification — reference dashboard (conformance, baselines, resource baseline)

The exit criterion: a developer can run, interact with, inspect, and
safely restyle a useful reference dashboard using only VelquView's
documented public surface — and the shipped example
(`examples/reference-dashboard`) is the conformance fixture.

## What landed

* `examples/reference-dashboard` — `index.html` + `app.css`: navigation
  rail, summary cards, searchable authored-row records list
  (`vx-show` predicates; `vx-for` stays deferred and documented),
  per-record detail/edit panels, Save disabled-when-saved, records card
  `overflow-y-auto`. Zero Tailwind/reactive diagnostics by
  construction; conditional `:class` bindings name complete token
  alternatives. Profile: [`docs/reference-dashboard.md`](../reference-dashboard.md).
* `velqu-tailwind` — reserved `vv-` author-hook namespace (ADR 0023):
  `vv-`-prefixed classes are skipped by the utility compiler (never
  compiled, never diagnosed); unknown unprefixed utilities still
  diagnose. Unit-pinned.
* Layout correctness (regressions before the dependent example):
  CSS Overflow §3 visible-axis coercion; CSS2 §10.5 percent-height
  definiteness; Flexbox §9.9 zero flex-basis projection.
* `crates/velqu-view/tests/reference_dashboard.rs` — the acceptance
  matrix through the public API only; reviewed-and-frozen visual
  baselines; `m7_regenerate` (ignored) digest/PNG regeneration tool;
  `m7_resource_journey` (ignored) measurement tool.

## Conformance matrix (all green)

| Acceptance row | Test |
|---|---|
| Initial dashboard: zero diagnostics, summaries 6/0/2, default selection, deterministic raster | `m7_initial_dashboard` |
| Filter and clear, useful no-results state | `m7_filter_and_clear` |
| Select + edit; Save updates dependents; disabled Save neither activates nor focuses | `m7_select_edit_save_disabled` |
| Hover/focus/scroll correct with **zero Taffy passes** (cached inspector counters) | `m7_hover_focus_scroll_are_presentation_only` |
| Filtering settles in at most one document layout | `m7_filter_costs_at_most_one_layout` |
| CSS reload preserves model/selection/focus/live state | `m7_css_reload_preserves_live_state` |
| Rejected full reload: app usable, generation unchanged | `m7_full_reload_rejection_then_publication` |
| Successful full reload: fresh state, stale handle/event inert | `m7_full_reload_rejection_then_publication` |

## Visual baselines

Five states regenerated, PNGs visually reviewed, then digests frozen
(`M7_*_DIGEST`): initial, edited, empty, scrolled (paint-side wheel,
no relayout), and 800×600 — recorded in
[`docs/reference-dashboard.md`](../reference-dashboard.md) with the
SMALL-viewport boundary statement and the regeneration procedure.
The lab's own render of the fixture reproduces the frozen `INITIAL`
digest byte-for-byte (`a89813c5…` from `velqu-lab --headless … --inspect`).

Review took two iterations, both producing real changes: the first
flagged the detail panel's Save/caption clipped at the viewport bottom
(fixture pair column too tall → bounded list); the second flagged the
nav rail ending above the content bottom, which the engine
investigation below proved to be correct rendering of a
browser-differing pattern — after the layout fixes the fixture pair
auto-heights and the final PNGs were re-reviewed (rail full-height and
flush with the content at both viewports; badge/ID spacing clean; the
800×600 state browser-equivalent, the cramped list clipping inside its
own card). Conformance facts are asserted next to the frozen SMALL
digest: `w-64` rail keeps 256px, stretches to main-content height, and
the detail panel stays inside the viewport.

## Engine fixes landed with M7 (focused regressions)

The dashboard exposed three layout-conformance gaps; each is pinned by
a named regression:

1. **Overflow computed-value normalization** (CSS Overflow §3) —
   `visible` on one axis with a clipping value on the other computes to
   `auto`, letting an `overflow-y-auto` flex item drop its
   content-based automatic minimum (Flexbox §4.5).
   `style::tests::overflow_visible_coerces_to_auto_on_the_other_axis`.
2. **Zero flex-basis projection** (Flexbox §9.9) — `flex-1`'s `0%`
   basis projects as an absolute zero; Taffy otherwise resolved it
   against content under intrinsic sizing, starving fixed siblings.
   `layout::tests::flex_zero_basis_distributes_free_space_not_content`,
   `layout::tests::scroll_container_flex_item_drops_content_minimum`
   (the dashboard's failure shape; `scroll_width` still reports the
   full extent). Honest boundary: with *visible* overflow the content
   minimum legitimately claims space — only scroll containers drop it.
3. **Percentage height needs a definite parent** (CSS2 §10.5) —
   `height: 100%` resolves against a definite parent, computes to
   `auto` otherwise, re-enabling flex align-stretch for `h-full` rails
   in auto-height rows. `layout::tests::percent_height_needs_a_definite_parent`.

Fix 3 changes the M5 dashboard's frozen raster (its `flex h-full` shell):
the rail now stretches full-height, browser-correct. Supersession:
`770b933b40dd…` → `dd94673102b2…` (1024×640 @1×; display items 37 → 48,
the larger rail background). The new raster was rendered and reviewed
before acceptance. Historical evidence docs stating "digest unchanged
(`770b933b…`)" were true at their milestone time and are superseded as
of 2026-09-19, per the documented-migration convention (M2a).

## Editor semantics exercised by the suite

The suite drives text through the public keyboard path and pins these
M4c editor semantics as the contract: programmatic `set_focus` parks
the caret at the value's **start** (pointer press places it at the
click point); `Backspace` deletes one grapheme and ignores the active
selection (SelectAll + text replaces); `insert_text("")` is a no-op, so
clearing is backspace-until-empty, with the suite asserting forward
progress. No IME claim is made (standing item, §below).

## Resource baseline (descriptive, single host — not a benchmark)

Scene: `examples/reference-dashboard`, 1280×800 @1×, Tailwind +
reactive + inspector on. Toolchain: containerized `velqu-bench:multihost`
(rustc 1.96.0), `--release` lab build. Host: shared Linux workstation,
2026-09-19. Timing numbers are the lab's own indicative wall times,
not latency targets.

Startup to first frame (release lab, `--frames 2`, 3 runs):
first frame 9.81 / 10.47 / 15.31 ms (first run cold); avg 9.30–12.28 ms;
digest and inspector identical across runs (120 display items, 1 layout
pass, 0 turns).

Scripted journey (filter → select → edit → save → clear), cumulative
counters from `m7_resource_journey --ignored --nocapture`:

| Step | turns | layouts | repaints | display items |
|---|---:|---:|---:|---:|
| initial | 0 | 2 | 0 | 120 |
| filter "waiting" | 1 | 3 | 0 | 94 |
| select TRK-2210 | 2 | 4 | 0 | 93 |
| edit name (13 chars) | 14 | 5 | 11 | 91 |
| save | 15 | 6 | 11 | 93 |
| clear filter (8 backspaces) | 22 | 8 | 16 | 121 |

Reading: per-character typing is presentation-only (12 reactive turns,
11 repaints, a single layout in the burst — M5d batching + M4b
interaction-paint); structural filters/selects cost one layout each;
the journey's whole cost is 22 turns / 8 layouts / 16 repaints.

RSS (VmRSS, same run): flat at ~50.8 MB through the entire journey;
across five bounded full reloads ~89.2 MB after the first-generation
replacement (new document + QuickJS generation), then +8 kB, +4 kB,
+0, +0 — plateaued, consistent with the bounded per-generation model.

Idle wakeups (watcher disabled / native / polling) are **not measured
here**: they require a windowed host run, which this headless lane
does not provide. The structural mechanism is M6c's — coalesced
notifications, bounded dirty set, injected-clock quiet interval,
reconciliation only on noise (see
[`docs/evidence/m6c-verification.md`](m6c-verification.md)); a windowed
wake count remains open follow-up work.

## Standing item

IME live-platform validation stays open exactly as scoped in M4c3
([`docs/evidence/m4c3-gate.md`](m4c3-gate.md)); the dashboard exercises
`insert_text` only and makes no IME claim.
