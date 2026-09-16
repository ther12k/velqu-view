# M0/M1 Verification Report

- Date: 2026-09-16
- Milestones: M0 (bootstrap) + M1 (window / paint)
- Machine: 13th Gen Intel Core i5-13420H, 12 logical CPUs, 16 GiB RAM,
  Linux 7.0.0 x86_64 (Wayland session with XWayland)
- Toolchain: rustc 1.96.0 (ac68faa20 2026-05-25)

## M0 exit criteria — "reproducible local and CI build"

| Item | Evidence |
|---|---|
| Workspace builds | `cargo build --workspace --locked` — clean, 5 members |
| Formatting | `cargo fmt --all -- --check` — clean |
| Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` — clean |
| Tests | `cargo test --workspace --locked` — **36 passed, 0 failed** (unit + doctests + 2 fixture integration tests) |
| CI | `.github/workflows/ci.yml` — fmt/clippy/test/release-build on ubuntu-latest with X11/Wayland dev libs |
| Fixture convention | `tests/README.md` + 2 visual fixtures with pinned hashes and committed baselines |
| Decision log | `docs/decisions/` — ADRs 0001–0003 |
| Evidence format | `docs/evidence/benchmark-format.md` |
| Spec retention | OKF bundle committed at `docs/okf/` |

## M1 exit criteria — "deterministic Hello fixture"

Native window, renderer surface, text/rectangle, resize/DPI, screenshot
capture:

| Item | Evidence |
|---|---|
| Native window | `velqu-lab examples/hello` presents on **Wayland** and (forced) **X11**; Esc / close exits cleanly |
| Rectangle | device-pixel-snapped solid fills + outline; exact-color probes asserted in fixtures |
| Text | two bundled weights (DejaVu regular/bold), 5 text runs, 173 glyphs per hello frame; visually verified against `tests/visual/hello/baseline.png` |
| Resize | offscreen: 200×150 … 1600×1000 viewports render correctly; window smoke at 200×150 / 640×400 / 1024×640 / 1600×1000 presents cleanly; `Resized`/`ScaleFactorChanged` request redraws |
| DPI | 2x fixture (`tests/visual/hello-2x`): swatch centers land at exactly 2× the 1x coordinates; text re-rasterized at 2× pixel size (ink coverage ≈ 4×, unit-tested) |
| Frame capture | `Frame::save_png` + `sha256_hex`; headless CLI writes PNGs and enforces determinism |

Headless determinism runs (release build, 20 frames each):

```text
800x600 @ 1.0x → 20/20 identical, sha256 9f59e9fc1027924ff53a8135c84867c211ebd3afcb562be89aca2673d21c5345
1600x1200 @ 2.0x → 20/20 identical, sha256 a8e904435977e1c2bd9edb416cc18ffecaa69fa9e2f97b50e4cf81ad03dbffe5
```

Indicative wall times (not benchmarks; no isolation, single machine):

```text
800x600 @ 1.0x:  first 1.40 ms, avg 513 µs / frame
1600x1200 @ 2.0x: first 4.88 ms, avg 2.26 ms / frame
```

Commands (reproduce from the repository root):

```bash
cargo build --release --workspace --locked
./target/release/velqu-lab --headless --size 800x600 --frames 20 examples/hello
./target/release/velqu-lab --headless --size 800x600 --scale 2 --frames 20 examples/hello
./target/release/velqu-lab --exit-after-ms 1200 examples/hello            # window, Wayland
WINIT_UNIX_BACKEND=x11 ./target/release/velqu-lab --exit-after-ms 1200 examples/hello
```

## What was proven

1. The Velqu-owned API (`VelquView`, `Viewport`, `Frame`) renders fully
   offscreen with byte-level determinism; windows are just one frame sink.
2. Deterministic fixtures (pixel hash + exact-color probes) run in plain
   `cargo test` with no display server — the CI shape all later milestones
   can keep using.
3. DPI is a pure viewport parameter: 2× scale re-rasterizes text and
   re-places geometry at exact 2× device coordinates.
4. winit + softbuffer presentation works on both Linux backends available
   here, with auto-close and Esc exit for smoke automation.

## Known limitations (recorded, not hidden)

- **HTML/CSS are not rendered yet.** `load_html`/`load_css` validate and
  retain sources; the visible scene is the explicit M1 probe. This is the
  M2 milestone, not a silent scope change.
- **Primitives are un-antialiased** (solid rects snapped to device pixels);
  text is coverage-blended. Raster quality is revisited with the M2 backend.
- **No anti-aliased clipping, transforms, scrolling, or input dispatch** —
  M2/M4 scope.
- **Interactive live-resize was not automated** on this machine (xdotool
  cannot see the XWayland window); verified instead via multiple window
  sizes + offscreen viewport sweeps. Manual interactive resize/Escape close
  remain for the developer to feel locally.
- **Timing numbers are indicative only** — no repetition statistics, no
  isolated machine; per `benchmark-format.md` they are not claims.
- **fontdue scalar path only** (ADR 0001) — glyph rasterization speed is
  traded for cross-architecture byte determinism at this stage.
- Release binary is 33.8 MB with `debug = "line-tables-only"` retained for
  future evidence runs; size optimization is deliberately not an M1 goal.

## Decision boundary status

No discrepancies found between the OKF bundle assumptions and the M1
implementation evidence. Nothing in M1 forces a browser-compatibility
feature. ADRs 0001–0003 record the backend, determinism, and API-boundary
decisions.

## Next milestone recommendation

**M2 — HTML/CSS renderer spike** behind the existing API: html5ever-style
parsing (or equivalent), style cascade over the CSS Profile v0 property
set, block + flex layout (Taffy candidate), text layout (Parley candidate),
backgrounds/borders, overflow + scrolling, and `layout_facts` added to the
fixture schema. The `Scene → Frame` seam and the fixture harness built here
are the integration points; the M2 engine choice (direct Taffy/Parley vs.
selected Blitz components) should be its own ADR once the first layout
fixtures exist.
