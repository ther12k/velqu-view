# Phase 1 closure record — VelquView

Status: **implementation and reference-application conformance complete.**
Release qualification remains explicitly scoped (§Open items).

Objective: *"make this velqu view phase 1 finished, it should can be
used to show html with tailwind"* — a native Rust HTML renderer
(winit + softbuffer + fontdue, private Taffy, rquickjs reactive
runtime; no browser engine) that renders HTML styled with Tailwind
utilities, interactive, inspectable, and safely restylable through the
documented public surface.

## Baseline

| Item | Value |
|---|---|
| Baseline commit (M7 acceptance) | `199ccb0335f8121802582b483e2a0de25a3814f2` |
| Closure commit (this record + 0%-basis scoping) | recorded in git at the tip that carries this file |
| CI on baseline | run `35447937753` — fmt/clippy(-D warnings)/test/release-build and MSRV 1.87 lanes, both green (2m16s) |
| Gate at closure | `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets --locked` zero warnings; `cargo test --workspace --locked` 338 passed / 0 failed; `cargo +1.87.0 check --workspace --all-targets --locked` clean |
| Reference application | `examples/reference-dashboard` (ADR 0023; profile: [`docs/reference-dashboard.md`](../reference-dashboard.md); evidence: [`m7-reference-dashboard.md`](m7-reference-dashboard.md)) |

Milestone table: M1 foundation → M2a/b/c layout/text/images → M3
Tailwind pipeline → M4a/b/c interactions and controls → M5 reactive
runtime (ADR 0015–0018) → M6a.1/a/b/c inspector, transactional reload,
file watching (ADR 0019–0022) → M7 reference dashboard (ADR 0023).
Each milestone has its ADR and evidence document under
`docs/decisions/` and `docs/evidence/`.

## Frozen raster baselines (durable identity record)

Two digest sets, deliberately kept separate — they describe different
fixtures at different viewports and must not be cross-read.

**Legacy conformance fixture** — `examples/tailwind-dashboard`,
1024×640 @1×, `velqu-lab --tailwind --headless`:

| Digest | Commit | Cause |
|---|---|---|
| `770b933b40dd4d20efd57267d3805fc3856508530ea76e34b9f7899dcb200622` | M2b → M6c.1 era | original freeze and all invariance claims through M6c.1 |
| `dd94673102b291d05bc3d25e11f93f573ee25e8ebd16c1db3dde904019b3c331` | M7 (`7ba3e66`, superseded at `199ccb0`) | percentage-height/stretch correction: full-height rail; display items 37 → 48; replacement raster rendered and visually reviewed before acceptance |

Historical evidence documents asserting the old digest were true at
their milestone time; superseded 2026-09-19 per the documented-
migration convention (M2a). Incorrect pixels are not preserved merely
because they were once frozen.

**M7 reference application** — `examples/reference-dashboard`,
`crates/velqu-view/tests/reference_dashboard.rs` drives the real files
through the public API; each state was rendered, its PNG visually
reviewed, then the digest pinned. Regeneration is the documented
intentional-change procedure (`m7_regenerate --ignored`), never silent.

| State | Viewport/scale | Digest (sha256) |
|---|---|---|
| INITIAL | 1280×800 @1× | `a89813c53c651e717abfd7926bcac5974149440005055bf1f45f93cd747a8c5c` |
| EDITED | 1280×800 @1× | `8b430f60ca82c8c0f71fd108892d7bab7adafacae3739794b63b981874b2fc44` |
| EMPTY | 1280×800 @1× | `12a04cb68fae1605092e5e192c547758ab836d9782df02f24b855512af4f6b29` |
| SCROLLED | 1280×800 @1× | `cae3e4aa911de89b67570ff9b5b2b2cbd780d0ecb1119e27875373aa8f2d2d3d` |
| SMALL | 800×600 @1× | `0fa9e6fc215f6350940644ff089b6513cc9e8605de16e88ff78968b20efce647` |

Both digest sets are unchanged by the closure-commit basis rescoping
(§below) — verified by rerunning the conformance suite and the lab
render before committing.

## Scope boundaries carried into the closure commit

The M7 review asked for one narrow engine-correctness resolution
before unconditional sign-off; it is closed here, and the two
precision items alongside it:

1. **`flex-basis: 0%` vs `0px` distinction preserved** (the WPT
   `flex-one-sets-flex-basis-to-zero-px.html` concern). The M7-era
   projection converted every `0%` basis to an absolute zero — too
   broad: a percentage basis is content-based when the container's
   main size is indefinite, exactly where browsers distinguish it from
   `0px`. The conversion is now scoped to **scroll-container flex
   items only** (computed overflow neither `visible` nor `clip` on an
   axis), where browsers size the intrinsic contribution at zero and
   Taffy's content-based measurement diverged — the dashboard's
   original failure. Pinned by
   `explicit_zero_percent_basis_stays_content_based` (auto-height
   column: `0%` item content-based, `0px` item zero-based,
   `min-height: 0` on both; definite height: identical). Non-scroll
   items keep their declared percentage through layout.
2. **Overflow normalization stated precisely.** When one axis is
   neither `visible` nor `clip`: the other axis's `visible` computes
   to `auto` and its `clip` computes to `hidden`; `visible`/`clip`
   paired together stays as specified. The automatic-minimum effect is
   about the *automatic* minimum only (Flexbox §4.5); explicit
   min-width/min-height remain separate author constraints. The v0
   profile folds the `scroll` keyword into `auto` at parse time. All
   pairing rows are unit-pinned.
3. **Percentage height is used-value language.** Against a
   content-sized parent the percentage behaves as `auto` at the layout
   step; the computed style keeps the declaration. Definiteness
   tracks declared length/percentage chains — flex-acquired or
   stretched heights are definite per Flexbox §3 but not tracked by
   the projection walk: a documented limitation, not a claimed
   equivalence.

## Resource-measurement boundaries

Full detail in [`m7-reference-dashboard.md`](m7-reference-dashboard.md)
§Resource baseline. Scope statements that travel with the numbers:

- First-frame times are the lab's measured first-frame **render wall
  time** in an already-started process — not cold launch to first
  screen presentation.
- Journey counters (22 turns / 8 layouts / 16 repaints) are an exact
  baseline for that one scripted sequence; "typing is
  presentation-only" holds for the tested edit path only.
- Memory figures are **VmRSS** (current resident), not VmHWM. The
  five-reload plateau (~89.2 MB) is observed behavior over five
  reloads, not a long-run leak-free guarantee.
- Watcher idle wakeups are **not measured**; M6c coordinator/watcher
  tests are logical evidence only. A windowed wake count is open
  follow-up work.

## Carried review items — closed

The two narrow M6c.1 checks carried forward by the M6b review are
closed and pinned (evidence: [`m6c-verification.md`](m6c-verification.md)
§Engine notes, landed at commit `246c679`, all present in the closure
gate):

- **Startup reconciliation** (registration is the baseline; an edit
  between initial read and registration produces no event):
  `startup_reconciliation_closes_the_registration_gap`
  (`apps/velqu-lab/src/reload_coordinator/tests.rs`),
  `native_startup_registration_gap_closes_without_a_save`
  (`apps/velqu-lab/src/watch/tests.rs`).
- **Preserved-timestamp polling edits** (notify truncates mtimes to
  whole seconds; same-second edits invisible to timestamp-only
  comparison):
  `polling_detects_same_second_edits_via_content_comparison`
  (`apps/velqu-lab/src/watch/tests.rs`).

## Open qualification items (release-scoped, not Phase-1 blockers)

1. **IME live-platform validation** (Wayland/X11, Windows, macOS) —
   scoped in M4c3 ([`m4c3-gate.md`](m4c3-gate.md)); VelquView exposes
   `insert_text` and makes no IME claim.
2. **Live-window idle-wakeup counts** under `--watch` / `--watch=poll`
   — requires a windowed host run; not measured by the headless lane.
3. **Long-run reload memory qualification** — a longer bounded run
   with retired-runtime/resource counts; the five-reload RSS plateau
   is indicative only.

## Disposition

Phase 1 is closed as a completed implementation, not an unlimited
compatibility promise: the frozen scope is the documented v0 profiles
(Tailwind utilities, reactive v0, CSS named profile) plus the
deviation records above. No M8 is opened by this record; renderer
rewrites, new reactive subsystems, or profile expansions are
separately named future proposals.
