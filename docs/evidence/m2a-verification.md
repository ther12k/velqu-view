# M2a Verification Report

- Date: 2026-09-16
- Scope: M2a — HTML + supported CSS cascade deterministically produce
  inspectable layout facts; the same facts deterministically produce the
  reference raster (per the agreed M2a exit criterion).
- Machine: 13th Gen Intel Core i5-13420H, 12 logical CPUs, 16 GiB RAM,
  Linux 7.0.0 x86_64
- Toolchains: rustc 1.96.0 (evidence), 1.87.0 (MSRV lane)

## What was built (commit sequence)

| Commit | Scope |
|---|---|
| `027b900` | ci: SHA-pinned actions, `permissions: contents: read`, `persist-credentials: false` |
| `1249a36` | DOM (`dom.rs`) + html5ever `TreeSink` lowering; ADR 0005 (identity separation) |
| `2c88a33` | CSS syntax + selector model on cssparser (`css.rs`); specificity, source order, diagnostics |
| `1b88fb8` | Cascade + `ComputedStyle` (`style.rs`); UA defaults, inheritance, inline styles, profile diagnostics |
| `7c5e742` | Deterministic text measurement + greedy wrap (`text.rs`) |
| `b700289` | Box tree, block layout, `LayoutFacts` v1, display list, painter rewire (`layout.rs`, `display_list.rs`, `painter.rs`) |
| (this commit) | ADR 0006 (M2a CSS profile), docs, evidence |

## Exit criterion evidence

1. **Structural facts**: `tests/visual/hello/fixture.toml` asserts exact
   border-box/content-box geometry, device-px padding/border/margin, and
   per-line text runs for `card`/`title`/`subtitle`, keyed by
   `data-vv-test` (never `NodeId`). The 1x and 2x fixtures assert the same
   document at both scales: geometry lands at exactly 2× device
   coordinates (e.g. card border box 436×… at 1x, 872×… at 2x).
2. **Same facts → same raster**: `render()` runs the identical cascade +
   layout pass; the pixel hash (10/10 identical frames per run) and
   committed baselines cover the raster layer. The fixture harness fails
   if facts and pixels ever diverge between instances.
3. **In-distribution rendering**: the hello example renders real HTML+CSS —
   page background from `html{}`, card border/background/margins, UA `h1`
   defaults overridden by `.title`, subtitle color inherited from `.card`,
   and text wrapping at the 400px content box (asserted as two specific
   lines in the fixture).

## Verification

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| `cargo test --workspace --locked` | **102 passed, 0 failed** |
| MSRV lane | `cargo +1.87.0 check --workspace --all-targets --locked` — passes with html5ever 0.40 + cssparser 0.38 in the tree |
| Determinism | hello @1x: 10/10 identical frames; fixtures deterministic across independent instances (facts + pixels) |
| Window smoke | Wayland + forced X11 present, Esc/auto-close ok |
| Fixture probes | exact-color probes pass (page bg, card fill, card border at 1x and 2x) |

Reproduce:

```bash
cargo test --workspace --locked
cargo +1.87.0 check --workspace --all-targets --locked
cargo run -p velqu-lab -- --headless --size 800x600 examples/hello
```

## Environment note (for reproducibility)

The development container was rebuilt mid-session (Rust toolchain,
`gh`, curl, python removed; `/home/ther12k` ownership changed). The
toolchain was reinstalled to `Workspace/.toolchain` (rustup 1.96.0 + 1.87.0
plus a Debian gcc-12 sysroot extracted from pool debs for linking). This
affects nothing committed — CI runs on stock ubuntu runners — but local
evidence runs should use `Workspace/.toolchain/env.sh` until the host is
restored.

## Known limitations (recorded, per ADR 0006)

- No margin collapsing (adjacent margins add); no sticky/fixed; no
  flex/grid yet (M2b/M2c, Taffy).
- Percentage heights resolve as auto; box-sizing is content-box.
- Inline boxes flatten into text runs: inline backgrounds/borders and
  inline-block are M2b.
- Text subset: Latin/UI, space-separated words, bundled DejaVu only;
  baseline centering uses constant metric ratios (0.928/0.236 per px)
  rather than per-font metrics — M2b replaces this with real metrics
  (Parley candidate).
- `!important` is captured per spec but its interaction with UA-important
  rules is untested (UA sheet is empty in M2a).
- Taffy is adopted as the flex/grid engine for M2b/M2c but is not yet wired
  (M2a block flow is hand-rolled; the Taffy mapping lands with the flex
  milestone and its own ADR).
