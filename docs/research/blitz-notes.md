# Research notes: blitz (DioxusLabs) as a reference project

Source surveyed: `DioxusLabs/blitz` @ `0.3.0-beta.2` (shallow clone, this
session). Blitz is the closest existing analog to VelquView: a native-Rust
HTML/CSS renderer with no full browser engine, built on Stylo (Servo's
style system), Taffy (layout), Parley (text), and Vello/peniko (GPU
paint). These notes map what they do onto our open decisions — they
validate several of our choices and sharpen the path for the deferred
ones.

## Architecture mapping

| Concern | blitz | VelquView | Verdict |
|---|---|---|---|
| Style | Stylo (Servo's full CSS engine) | cssparser + own cascade + named profile | Different bets: they buy browser-complete CSS; we buy a small auditable profile with loud diagnostics. Both are coherent; ours is the OKF contract. |
| Layout glue | `stylo_taffy` crate: `convert.rs` (`to_taffy_style`) + `wrapper.rs` | private `taffy_backend.rs`: `map_style` + projection | Same shape — an isolated translate layer between style and Taffy. Our private-module version of their standalone-crate version. Validated. |
| Taffy | **fork of Taffy** (git-rev-pinned `DioxusLabs/taffy@c17b313`) | upstream `taffy 0.14` | They fork because they implement **inline layout and tables as custom Taffy layout modes** (`layout/inline.rs`, `layout/table.rs` call `compute_child_layout` from inside the engine). We stay on upstream; see inline finding below. |
| Text | Parley 0.11.1 (`default-features = false, features = ["std"]` — the same feature discipline as our fontdue) | fontdue + hand-rolled greedy line breaker | Their approach is where our text milestone should land; see white-space finding. |
| Paint | Vello/peniko GPU scenes | deterministic CPU rasterizer | Ours is a fixture contract (pixel hashes); GPU remains a far-future backend behind the display list. |
| MSRV | workspace `rust-version = 1.91`, CI has a dedicated `build-msrv` lane pinning 1.91 | `1.87` with the same CI lane pattern | Same discipline. Note: their stack (Parley 0.11 etc.) requires ≥1.88 — when our text overhaul adopts Parley, the MSRV bump to match is an explicit, evidenced decision, exactly like the fontdue/1.87 bump. |

## Finding 1 — white-space: delegate to the text engine, don't patch the word model

`blitz-dom/src/stylo_to_parley.rs` maps Stylo's computed `white-space`
into Parley attributes per run:

```text
stylo WhiteSpaceCollapse::Collapse      -> parley Collapse
stylo WhiteSpaceCollapse::Preserve      -> parley Preserve
stylo PreserveBreaks (pre-line)         -> parley Preserve  (+ wrap attr)
stylo BreakSpaces                       -> parley Preserve
```

Blitz contains **zero hand-written wrapping or whitespace collapsing** —
that is Parley's job, fed by attributes. Implication for us: implementing
`white-space: pre` inside our word-splitting model would be rebuilding a
slice of a text engine badly. The correct fix is the already-planned
text-shaping overhaul (Parley is our recorded candidate), and this is
the exact seam it plugs into: computed `white_space` → text-engine
attributes. Our current state (`nowrap` honored, `pre` deferred with
diagnostics) is the right holding pattern.

## Finding 2 — inline fragmentation is engine work, which is why we were right to defer it

Blitz's inline layout lives *inside a Taffy fork* as a custom layout mode
(`layout/inline.rs`), with baselines, fragments, and child layout
computed via `compute_child_layout` during the Taffy pass. That is the
level of commitment real inline fragmentation takes. Our M2b decision —
anonymous text items, fragmented inline decorations out of profile,
baseline deferral diagnosed loudly — remains the proportionate choice
for a profile-based renderer, and upgrading later means extending the
backend seam, not the public API.

## Finding 3 — our rounded-border simplification is honest; border.rs is the reference for more

`blitz-paint/src/render/border.rs` implements full CSS border semantics:
bevel/groove/ridge color derivation, dashed/dotted via path sampling
along the border path, and careful corner geometry — including the
`0 < radius < border-width` case, where CSS draws a rounded outer edge
against a sharp inner corner. Our `RoundedBorder` implements the solid
case with per-corner inner radii (clamped at 0), which approximates that
edge case the same way (inner corner goes sharp). If we ever add
border-style variety, that file is the roadmap.

## Finding 4 — damage flags: the template for our future incremental relayout

`blitz-dom/src/layout/damage.rs` defines restyle damage as bitflags —
`CONSTRUCT_BOX`, `CONSTRUCT_FC`, `CONSTRUCT_DESCENDENT`,
`ONLY_RELAYOUT` — propagated down the tree to decide *what* work a
mutation requires. This is the concrete shape our `layout_stats()`
instrumentation was built to grow into: when M4/M6 add mutations, a
damage-flags model (rather than ad-hoc dirty booleans) is the proven
design to copy.

## Finding 5 — their conformance strategy mirrors ours, at web scale

Blitz runs the **Web Platform Tests** suite (`wpt/runner`, with reftests
via `<link rel=match|mismatch>` and testharness.js, results posted from
CI). Our facts+pixel-hash fixtures are miniature reftests; the Tailwind
conformance corpus (`tests/tailwind/`, still future) can adopt the same
runner idea later: shared HTML fixtures, expected-match/mismatch
metadata, CI-posted results.

## What we deliberately do not adopt

* **Stylo** — a browser-complete style system contradicts the OKF
  profile contract (loud, diagnosed, small surface) and dwarfs the
  codebase.
* **Vello/GPU paint** — the CPU rasterizer is our determinism contract;
  GPU stays a far-future backend behind the display list boundary.
* **A Taffy fork** — we have no custom layout modes (yet); upstream 0.14
  plus our projection covers the frozen profile. If inline ever enters
  the profile, upstream first, fork only with evidence.

## Actions taken from this research

None in code (nothing here changes the frozen M0–M3 surface). This note
is recorded so the text milestone, the mutation milestones, and the
Tailwind corpus start from proven designs instead of fresh guesses.
