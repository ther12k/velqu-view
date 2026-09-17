# M4c3 verification — IME composition (ADR 0014)

Scope: preedit as presentation state, atomic commit through the shared
filter, session-scoped composition ownership, keyboard suppression
during composition, platform index clamping, shell-side enablement and
candidate anchoring. The gate was pre-registered in
[`m4c3-gate.md`](m4c3-gate.md); every scenario below names its test in
`crates/velqu-view/src/lib.rs`. Established gates held: all prior tests
green, and documents without compositions paint byte-identically.

## The gate scenarios, as tests

1. **Preedit changes pixels but not value; update replaces, not
   appends** — `m4c3_preedit_paints_without_touching_the_value`:
   the composed raster differs from the focused baseline, the value
   stays `abc`, no events fire, zero layout passes; `"にほ"` → `"ほ"`
   paints differently with the caret left of the previous position;
   cancel restores the baseline hash exactly.
2. **Commit inserts exactly once and clears composition** —
   `m4c3_commit_inserts_exactly_once_and_clears_the_composition`:
   exactly one `ValueChanged` + one `SelectionChanged` (pinned order),
   a second commit is inert.
3. **Composition over a selection replaces it** —
   `m4c3_composition_over_a_selection_replaces_it`: Ctrl+A then
   preedit then commit ⇒ the value is the commit.
4. **Blur cancels preedit without committing** —
   `m4c3_blur_cancels_and_a_stale_commit_cannot_touch_the_new_control`
   (focus a→b and focus→None): the cancelled composition leaves no
   pixels (hash equals the focused-b baseline).
5. **Stale Commit after focus transfer cannot modify the new control**
   — same test: the commit is refused; neither `a` nor `b` changes.
6. **Invalid/out-of-range preedit cursor indexes cannot panic** —
   `m4c3_preedit_cursor_indexes_are_clamped_not_trusted`: `(1,1)` into
   `"にほ"` clamps to the preceding boundary (caret at the content
   left), `(99,0)` and unordered `(5,3)` clamp and order; render OK.
7. **Unicode composition remains UTF-8 safe** — byte-offset cursor
   handling throughout; the clamp test plus
   `m4c3_commit_inserts_exactly_once_and_clears_the_composition`
   (`"日本"` commit, byte-offset selection).
8. **Readonly never starts IME state** —
   `m4c3_readonly_controls_never_start_a_composition`: `wants_ime()`
   false, preedit refused, commit inert, no candidate rect.
9. **Ancestor scrolling updates candidate coordinates** —
   `m4c3_ancestor_scrolling_moves_the_candidate_rect`: wheeling the
   enclosing pane 40px moves `ime_cursor_rect` up with the content.
10. **Active composition causes zero Taffy passes** — scenario 1 pins
    `layout_stats().passes` across preedit, update, and cancel.
11. **Stray KeyboardInput during preedit cannot double-insert** —
    `m4c3_stray_keyboard_input_during_composition_cannot_double_insert`
    (the Windows-shape regression): `Preedit("a")` + stray
    `insert_text("a")` + Backspace/Enter/SelectAll commands all
    refused; `Commit("あ")` ⇒ `"あabc"`.

## Shell integration

`Ime::Preedit/Commit/Disabled` map onto
`ime_preedit/ime_commit/ime_cancel`; `Focused(false)` cancels.
`set_ime_allowed` follows `wants_ime()` change-gated;
`set_ime_cursor_area` receives physical pixels from
`ime_cursor_rect`, re-read after keyboard/pointer input, wheel,
redraw, focus, and IME events (deduplicated). No winit type crosses
into `velqu-view`.

## Gates

* `cargo fmt --all -- --check`; `cargo clippy --workspace
  --all-targets --locked` (clean); `cargo test --workspace --locked`
  (165 renderer tests incl. the 8-test IME battery); `cargo +1.87.0
  check --workspace --all-targets --locked`.
* Headless dashboard smoke: digest unchanged (`770b933b…`) — IME code
  adds no raster without a composition.
* Windowed smoke (Wayland): a document with an input renders and exits
  cleanly with the IME enablement path exercised (allowed=false while
  unfocused).
* Interactive IME smoke (real candidate windows, preedit round-trips)
  on Wayland/X11 requires a human at the display; it is the remaining
  platform evidence this automation cannot synthesize, per the
  pre-registered gate.

Landing: `2da9d6c` (implementation + gate battery) + `0fa77cf` (docs)
pushed to `main`; GitHub CI green — run
[35227952541](https://github.com/ther12k/velqu-view/actions/runs/35227952541)
(fmt/clippy/test/build + MSRV 1.87).
