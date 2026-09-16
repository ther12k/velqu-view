# Phase-1 review fixes — evidence

The phase-1 self-review (commit `473ec72`) and the follow-up fix round
(`308cf7c`, `bb64f58`, this commit) against M3 (ADR 0009).

## Review-round findings (all fixed and pinned)

1. **Cascade order was backwards** — the generated utility sheet sorted
   after author sheets, so author CSS could never override utilities at
   equal specificity (opposite of Tailwind's layered-utilities
   semantics). The sheet now sorts first among author sheets. Pinned by
   `tailwind_author_css_overrides_utilities`; ADR 0009 corrected
   pre-acceptance with the correction recorded in it.
2. **`<style>` blocks were silently dropped** — the earlier element-rule
   cascade test passed vacuously. Style elements are now extracted per
   document into author sheets.
3. **`whitespace-*` utilities missing; `white-space: nowrap` had no
   layout effect** — the compiler gained the utilities, and line
   breaking (measure + positioning) honors nowrap. Pinned by
   `tailwind_whitespace_utilities_map_into_the_profile`.
4. **Stale crate docs; `flex-wrap-reverse` now diagnosed explicitly.**

## Follow-up fix round (the review's known gaps)

* **Rounded corners are painted** (`308cf7c`): `RoundedFill`,
  `RoundedBorder`, `PushClipRounded`; strict per-pixel coverage on
  pixel centers, no anti-aliasing, deterministic. Zero-radius paths are
  untouched — every square fixture stayed byte-identical;
  `tailwind-hello`'s hash migrated with a documented note (its
  `rounded-lg` card now clips corners). New `rounded` fixture pins a
  stadium fill, a border ring with an unpainted hole, and a rounded
  overflow clip. The dashboard example visibly rounds its cards and
  nav pill.
* **`velqu-css-check` CLI** (`bb64f58`): `check_css` classifies every
  declaration and at-rule with source lines (`@media` recursed, other
  at-rule blocks skipped as already-diagnosed); the binary prints
  verdicts with suggestions, summaries, and exits non-zero on
  unsupported constructs so it can gate builds. Seven tests.

## Still deferred (recorded, not forgotten)

* `white-space: pre` preserved-whitespace text layout (the text engine
  collapses whitespace; `nowrap` is honored, `pre`/`pre-line`/`pre-wrap`
  currently behave as `normal`-wrapping with collapsed spaces).
* Spacing utilities emit absolute px (Tailwind's 4px-per-unit at a 16px
  root default), not rem-relative to a configurable root font size.
* Variant classes (`hover:`), arbitrary values, and real compiled-
  Tailwind ingestion remain behind diagnostics / the same seam.
* SVG icons: raster-only for v0 (ADR 0009 §5).

## Verification at the end of the round

* `cargo test --workspace --locked`: all green (112 velqu-view lib
  tests incl. the two new cascade/whitespace pins, 30 velqu-tailwind
  tests incl. 7 checker tests, 18 visual fixtures, cross-instance
  determinism).
* `cargo fmt --all -- --check` clean; `cargo clippy --workspace
  --all-targets` clean.
* `cargo +1.87.0 check --workspace --all-targets --locked` clean.
* CI: both lanes green on the push.
