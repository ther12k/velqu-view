# M2b Verification Report

- Date: 2026-09-16
- Scope: M2b — Taffy as the whole-tree layout backend, flex CSS profile,
  display-list clipping, overflow.
- Machine: 13th Gen Intel Core i5-13420H, 12 logical CPUs, 16 GiB RAM,
  Linux 7.0.0 x86_64
- Toolchains: rustc 1.96.0 (evidence), 1.87.0 (MSRV lane)

## Commits

| Commit | Scope |
|---|---|
| `3e5381f` | Taffy whole-tree backend, flex profile, PushClip/PopClip, M2a regression gate |
| `08348b5` | Flex fixtures (101px @1x/1.25x, mixed nesting, overflow), lab scale fix |
| (this commit) | ADR 0007, evidence |

## Architecture delivered (ADR 0007)

- **One layout engine**: block and flex both run through Taffy 0.14 via
  full per-pass projection of the Velqu box tree; zero Taffy types escape
  `taffy_backend.rs`.
- **One rounding owner**: Taffy unrounded; the painter snaps at raster.
  Flex facts at 2× are not required to equal 2 × 1× facts; determinism,
  containment, and the rounding policy are the contract (asserted by
  flex-101 at 1× and 1.25×).
- **Overflow is paint-side**: `overflow: hidden|clip` maps to layout
  containment in Taffy and `PushClip`/`PopClip` in the display list; the
  painter intersects the clip stack. Layout truth keeps full child sizes.

## Regression gate (the pre-agreed M2b acceptance condition)

Every M2a block-only fixture retains identical `LayoutFacts`. Raster:
hello @1× is **byte-identical** (`b6af24a4…`). hello-2× facts identical;
its raster moved because Taffy computes unrounded f32 and 2× glyph
placement is sub-pixel — baseline regenerated, migration documented in the
fixture and ADR 0007. Intentional semantic migration recorded: CSS sibling
margin collapsing now applies (M2a added margins); page roots use
`FlowRoot`.

## Fixtures

| Fixture | Asserts |
|---|---|
| `flex-101` | 3 × grow:1 in 101px: exact 101/3 widths, tiling (a.right == b.left, sum == 101), cross-axis stretch |
| `flex-101-125` | Same at 1.25× (42.0833… widths) — catches integer assumptions |
| `flex-mixed` | block → flex → text/block/flex-column: ordering, blk size 50×30, column stacking |
| `flex-overflow` | Child keeps full 300px layout truth; pixel probes inside/outside the clip |
| `hello`, `hello-2x` | M2a gate (unchanged facts; 1× hash identical) |

## Bugs the fixtures caught (and fixed)

1. The flex declaration arms silently failed to apply — `flex-grow` stayed
   0 while `display:flex` worked (a perl substitution had mismatched its
   pattern). Now covered by `flex_grow_distributes_fractional_widths` and
   the updated unknown-property test (`order` as the unknown case).
2. `velqu-lab` truncated fractional scales (`1.25 as u32 == 1`) when
   computing the physical viewport — headless 1.25× renders were the wrong
   size. Fixed with per-axis rounding.

## Verification

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| `cargo test --workspace --locked` | **105 passed, 0 failed** |
| Window smoke | Wayland + forced X11, flex and mixed examples |
| Determinism | fixture harness cross-instance facts+pixels equality |

Reproduce:

```bash
cargo test --workspace --locked
cargo run -p velqu-lab -- --headless --size 400x300 examples/flex
cargo run -p velqu-lab -- --headless --size 400x300 --scale 1.25 examples/flex
```

## Known limitations (ADR 0007)

- Baseline alignment deferred with loud diagnostics (needs real font
  baselines); the 12/26/16px trap fixture is queued for the text overhaul.
- `order`, percentage gaps, scrollbar behavior, absolute positioning:
  deferred by profile, not by accident.
- Inline boxes still flatten (no fragmented inline decorations until a real
  inline formatting model).
- Projection copies the tree per pass; the zero-copy `LayoutPartialTree`
  adapter is the contained optimization path if profiling demands it.
- Taffy is wired for grid too, but grid stays outside the profile until
  M2c maps it deliberately.
