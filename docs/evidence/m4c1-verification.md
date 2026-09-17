# M4c1 verification — editable controls; opaque identity, runtime state (ADR 0012)

Scope: `ElementHandle` identity, runtime control values/selection with a
state-free `LayoutFacts`, replaced control boxes, grapheme-safe editing,
the command/text keyboard split, readonly/disabled policy, pointer
capture selection, scroll-to-caret, and presentation-only editing.
Established gates held: every prior test stayed green and **no fixture
raster or fact changed** (controls paint runtime items only where the
author styled neither background nor border; documents without controls
emit byte-identical display lists).

## Handle identity and stale-handle behavior

* Two id-less scroll containers emit distinct `ScrollTarget::Element`
  handles with `id: None` (`idless_scroll_containers_have_independent_
  accumulating_state`, strengthened in M4c1).
* Handles survive resize/restyle relayouts and die with the document:
  resolution checks generation, node range, and element-ness;
  `set_focus_handle` with a stale handle is a safe no-op.
* Events carry `ElementTarget { handle, id }` — pointer, focus, scroll,
  value, and selection share one identity model; the optional HTML id
  rides along as metadata (`m4c1_editing_keeps_runtime_value_out_of_
  dom_and_layout_facts` asserts `SelectionChanged`/`ValueChanged`
  targets).

## Initial values vs current values; facts vs raster

* `<input value=abc>` initializes from the attribute; typing produces
  `"éabc"` in `control_value(handle)` while `dom.attribute(.., "value")`
  still reads `abc` — the DOM is immutable input.
* `LayoutFacts` deep-equal across editing (`m4c1_editing_...`); the
  frame raster changes (`m4c1_editing_repaints_presentation_without_
  layout`). Facts stay structural; control truth is the separate
  `ControlFacts` snapshot (kind, value length, selection anchor/focus,
  caret rect, visible range, internal scroll).
* Textarea initializes from child text and accepts newlines; unsupported
  input types produce deterministic `control_diagnostics()` entries and
  are not editable.

## The no-Taffy editing path

Every editing and selection operation rebuilds control paint from the
cached outer box and re-emits the display list; the `layout_passes`
counter is frozen in tests while `repaints` increments:

* `m4c1_editing_keeps_runtime_value_out_of_dom_and_layout_facts`
  (insert), `m4c1_commands_filter_named_text_and_allow_textarea_
  newlines` (End/Backspace/Enter/insert), `m4c1_editing_repaints_
  presentation_without_layout` (frame hash changes, passes equal,
  repaints grow), `m4c1_click_places_caret_and_focuses_the_control`
  and `m4c1_drag_selects_with_capture_even_outside_the_control`
  (pointer paths).

## Unicode safety

`editor.rs` unit tests pin the grapheme contract: ASCII stepping,
precomposed `é` as one grapheme, `a` + U+0301 combining accent as one
grapheme (backspace removes both), `😀` as one grapheme with offsets
remaining valid UTF-8 boundaries, and selection replacement never
splitting a code point. `m4c1_editing_...` inserts `"é"` through the
public API and asserts byte-offset selections.

## Command filtering (`KeyEvent.text`)

* Shell unit tests (`velqu-shell`): Enter — even with winit's
  `"\r"` text — maps to `KeyCommand::Enter`, Tab/Escape to commands,
  Ctrl/Cmd+A to SelectAll (Alt+A does not), and a character key
  (`"é"`) yields no command so its text is inserted.
* Renderer-side defense: `insert_text("\r")` on an input is rejected;
  textarea accepts newline text; Enter as a *command* inserts `\n`
  only in textarea (`m4c1_commands_filter_...`).

## Readonly and disabled behavior

`m4c1_readonly_and_disabled_controls_reject_value_changes`: readonly
rejects value mutation but allows caret movement; disabled is skipped
by `focus_next`, refused by `set_focus`/`set_focus_handle`/pointer
press, and rejects editing.

## Caret placement, drag selection, capture, scroll-to-caret

* `m4c1_click_places_caret_and_focuses_the_control` — press+release on
  an input: pointer-origin focus, collapsed caret at offset 0, `Click`
  + `SelectionChanged{0,0}`, no `ValueChanged`, zero layout passes.
* `m4c1_drag_selects_with_capture_even_outside_the_control` — dragging
  past the 200px input extends selection to `(0, 5)` with `PointerLeave`
  preceding `SelectionChanged` (hover follows the real pointer while
  capture holds); release ends capture — subsequent moves emit hover
  events only; the selection itself is still `(0, 5)` afterwards.
* `m4c1_control_facts_report_caret_and_scroll_to_caret` — an 80-char
  input: Home keeps `scroll_offset (0,0)`; End scrolls horizontally
  (`scroll_offset.0 > 0`) with the caret rect inside the content box
  and the visible range spanning the line.

## Resize/restyle preservation and reload reset

Control state is node-keyed like scroll offsets: it transplants across
relayouts (the editor rehydrates against the new outer content box on
the next render), and `load_html` clears values, selections, capture,
focus, and hover together (`input_state_resets_with_the_document` and
the generation bump invalidating handles).

## Gates

All run and green on this slice:

* `cargo fmt --all -- --check`
* `cargo clippy --workspace --all-targets --locked` (clean, no warnings)
* `cargo test --workspace --locked` — 201 tests across the workspace
  (151 in velqu-view incl. the M4c1 battery, 4 shell key-mapping tests,
  visual fixtures ×2, tailwind, reactive, lab)
* `cargo +1.87.0 check --workspace --all-targets --locked` (MSRV lane)
* headless dashboard smoke: `velqu-lab -- examples/tailwind-dashboard
  --headless` — 37 items / 180 glyphs, stable digest, PNG written
* windowed shell smoke (Wayland): `velqu-lab -- examples/hello
  --exit-after-ms 1500` — window opened, 2 frames presented, clean
  close. Key routing itself is covered by the pure `key_command_for`
  unit tests; synthesizing real keypresses needs the M6 tooling.
