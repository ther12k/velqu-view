# M2c verification evidence — images, grid, scroll

Scope: M2c as authorized with the review's re-ordering — images first
(replaced-element intrinsic sizing feeds grid), then grid, then scroll.
Profile frozen in [ADR 0008](../decisions/0008-m2c-images-grid-scroll.md).

## Commits

| Commit | Content |
|---|---|
| `4a40688` | m2c: add bounded asset decoding and image resources |
| `3b44c3c` | m2c: model img as an intrinsic replaced element |
| `d9db08b` | m2c: paint images and pin image fixtures |
| `c88c7e0` | m2c: map the frozen grid profile to Taffy |
| `6471a83` | m2c: pin the required grid fixtures |
| `2a3e975` | m2c: add scroll extents, runtime scroll state, and instrumentation |
| `1dd0e41` | m2c: paint scrolling with PushTransform and pin scrolled fixtures |
| *(this commit)* | m2c: freeze the CSS profile (ADR 0008) and record evidence |

## MSRV evidence

`cargo +1.87.0 check -p velqu-view` passes with the new `image 0.25.9`
dependency in the lockstep tree (image declares `rust-version = 1.85.0`;
verified, not assumed). Full `cargo +1.87.0 check --workspace
--all-targets --locked` re-run at freeze time — see the final
verification block below.

## Regression gate

**Every M2a/M2b fixture kept identical LayoutFacts and raster hashes.**
The 1× hashes `hello = b6af24a4…`, `flex-101 = b07012fa…`,
`flex-mixed = f4d5d97a…`, `flex-overflow = 6e70f3f5…` are unchanged
through all seven M2c commits (`hello-2x` stayed at its documented M2b
Taffy-migration hash `2f49acba…`). Two intentional, contained behavior
notes:

1. `scroll`/`auto` boxes now emit a clip scope (hidden/clip geometry
   unchanged); no pre-M2c fixture used them.
2. Declaration **values** are captured as raw source slices instead of
   token re-serialization (a real css.rs bug: nested function blocks lost
   closing parens). Every previously supported value grammar parses to
   the same result — all pre-existing fixtures and unit tests pass
   unmodified.

## New fixtures (facts-first, then raster)

| Fixture | Pins |
|---|---|
| `image-block` | intrinsic 40×20; CSS `width: 60px` → ratio height 30; `DrawImage` probes |
| `image-flex` | intrinsic main size 40 (flex-basis from content), stretched cross size 30, grow sibling at exactly x=40 |
| `image-broken` | missing asset → default object size 300×150 footprint, nothing painted |
| `grid-fr` / `-125` / `-2x` | 101px, `1fr 2fr`: exact fractional split 33.666668 / 67.333336 logical; 42.083332 / 84.166664 at 1.25×; 67.333336 / 134.666672 at 2× (rounding policy: 2×-independence accepted, determinism pinned) |
| `grid-image` | auto column sized by a JPEG's intrinsic 320×180 (second decode format in CI), 10px gap probe, text in the 1fr remainder |
| `grid-nested` | grid → flex → grid → text in one Taffy tree; whole-tree dispatch the island architecture cannot do |
| `scroll-page` | document scroller at y=100 over a red/blue page: raster moved, facts unscrolled |
| `scroll-pane` | 120×40 pane into 300px of content, scrolled 50px: unscrolled facts (content at x=0/150, extents 300×40), shifted raster, clip holding outside the pane |

Fixture baselines are re-generatable: `pixels_sha256 = "PENDING"`
re-saves `baseline.png` and fails with the digest to pin.

## The scrolling regression (review-mandated), pinned in test

`scroll_offset_is_runtime_state_without_new_layout` proves, in order:

1. `layout_facts` before scrolling == after (byte-identical facts);
2. `set_scroll_offset` runs **zero** layout passes
   (`layout_stats().passes` unchanged);
3. the raster differs after scrolling;
4. scrolling back to 0 reproduces the original frame byte-for-byte;
5. scroll 400 and scroll 100000 clamp to the same maximum-offset raster;
6. non-finite offsets are rejected (`VelquError::InvalidScrollOffset`).

## Bugs the new fixtures/tests caught

1. **Stretch ≠ intrinsic in flex** — the first image flex test failed
   until the profile decision was explicit: replaced items stretch on
   the cross axis (as in browsers) but keep intrinsic main size; ratio
   resolution reads the author CSS only (a stretched cross size must not
   retro-apply the ratio).
2. **Block auto width stretched images** — fixed with Taffy's
   `item_is_replaced` (compressible replaced sizing).
3. **Grid containers with only inline children projected as leaves** —
   their tracks never reached Taffy (innergrid text-sized to 76.9px
   instead of its 60px+40px tracks). Grid containers now keep tracks and
   wrap inline content into an anonymous grid item.
4. **css.rs `read_value` dropped closing parens** of nested function
   blocks during token re-serialization — found by the grid track
   parser; fixed with raw source-slice capture.
5. **Painter `pop_clip` reset clipping to unclipped** instead of
   restoring the stack — a latent M2b bug nested clips would have
   exposed; now a proper stack (no existing fixture nested clips, so
   rasters unchanged).

## Final verification at freeze

* `cargo test --workspace`: all green — 107 velqu-view lib tests, 16
  visual fixtures (facts + hashes + probes), cross-instance
  determinism, reactive/tailwind/shell suites.
* `cargo fmt --all -- --check`: clean.
* `cargo clippy --workspace --all-targets` (warnings as tracked): clean.
* `cargo +1.87.0 check --workspace --all-targets --locked`: clean.
* `velqu-lab` headless smokes: `examples/images` (real DirAssets host,
  image diagnostics empty), `examples/grid` (1fr/2fr gallery with
  centered, spanning cells); window smoke on Wayland + forced X11.

## Known limitations (frozen, diagnosed)

* Images: HTML width/height attributes, `srcset`, `object-fit`, inline
  text flow, SVG/animated formats deferred.
* Grid: named lines, template areas, auto-fill/auto-fit, dense packing,
  subgrid, masonry, percentage/calc gaps, negative lines deferred.
* Scroll: wheel/touch/keyboard input is M4; scrollbars are not modeled;
  scroll extents use direct-children geometry (a documented
  simplification while overflow of unclipped descendants is unused).
* Whole-tree Taffy reconstruction per pass is deliberately unoptimized;
  `layout_stats()` exists so M4/M6 can measure before deciding.
