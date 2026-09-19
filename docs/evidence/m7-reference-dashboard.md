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

## Resource baseline (descriptive, single host — not a benchmark)

Scene: `examples/reference-dashboard`, 1280×800 @1×, Tailwind +
reactive + inspector on. Toolchain: containerized `velqu-bench:multihost`
(rustc 1.96.0), `--release` lab build. Host: shared Linux workstation,
2026-09-19. Timing numbers are the lab's own indicative wall times,
not latency targets.

Startup to first frame (release lab, `--frames 2`, 3 runs):
first frame 13.91 / 14.18 / 14.77 ms; avg 13.27–13.74 ms; digest and
inspector identical across runs (120 display items, 1 layout pass,
0 turns).

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

RSS (VmRSS, same run): flat at ~55.2 MB through the entire journey;
across five bounded full reloads ~93.6 MB after the first-generation
replacement (new document + QuickJS generation), then +36 kB, +8 kB,
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
