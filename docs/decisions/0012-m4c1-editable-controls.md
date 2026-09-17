# ADR 0012: M4c1 — editable controls; opaque identity and runtime control state

Status: accepted (M4c1)

## Context

M4b froze interaction styling: hover/focus/active paint changes, cursor,
and scroll all happen with zero Taffy passes, and runtime state is
node-keyed so it transplants across relayouts. But public events still
identified elements by an optional HTML `id` string — id-less elements
were indistinguishable in `FocusChanged` and aliased nothing useful — and
the renderer had no text editing at all.

The reviewer set two rules before any editing behavior, and this ADR
freezes them first:

1. **Element identity in public events is opaque and
   document-scoped.** HTML ids are descriptive metadata, not identity.
2. **Control state is runtime state.** A control's current value,
   selection, and internal scroll live next to hover/focus/scroll state —
   never in the DOM, never in `LayoutFacts`.

## Decision

### 1. `ElementHandle`: opaque, generation-scoped identity

`ElementHandle` is a `Copy` token carrying a private document-generation
counter plus the parse-local node slot. Its fields and constructor are
crate-private; callers obtain handles only from the view (`HitTarget`,
events, `ControlFact`s) and hand them back. Guarantees:

* Stable across resize/restyle relayouts (the DOM and generation do not
  change when only geometry is recomputed).
* Invalid after document replacement: every successful load increments
  the generation, and resolution checks generation, node range, and
  element-ness. Stale handles are safe no-ops.

`Event` variants (`PointerEnter`/`PointerLeave`/`Click`,
`FocusChanged`, `ValueChanged`, `SelectionChanged`) and
`ScrollTarget::Element` carry `ElementTarget { handle, id }` — one
identity model for every event, with the optional author id alongside as
metadata. Id-based convenience APIs (`set_focus(Some(id))`,
`set_scroll_offset(Some(id), ..)`) remain; `set_focus_handle` is the
handle-based counterpart. Two id-less scroll containers now emit
distinct identities (test-pinned).

### 2. Runtime control state, separate from the DOM and from facts

`<input>` (no type / `type="text"`) and `<textarea>` are the M4c1
profile. On document load the view discovers them and initializes
`ControlState { kind, editor, readonly, disabled, dirty,
scroll_offset }` from DOM attributes/child text — the *initial* content.
From then on the DOM is immutable input: editing mutates only runtime
state (`EditorState`), and `control_value(handle)` is the only way to
read the live value. Unsupported input types produce deterministic
diagnostics (`control_diagnostics()`), not editable controls.

`ControlFacts` is a separate snapshot (never touching `LayoutFacts v1`):
per control it reports kind, UTF-8 value length, selection anchor/focus,
caret rect, visible byte range, and internal scroll offset — read from
the cached outer layout plus runtime state, empty on cache miss
(the no-implicit-layout rule).

### 3. Replaced outer boxes; the editor lives outside document layout

Controls are replaced leaves in the box tree (the M2c image path): no
child flattening, projected to Taffy with deterministic intrinsic sizes
(input 200×32, textarea 240×96 device-px-scaled; author width/height
still win). Taffy lays out only the outer box. The editor's line
layout, selection, caret, and internal scroll are computed by
`control.rs` from the fixed content box — never part of the document
tree, never measured to size the document.

Editing is presentation-only after the first layout: value/selection
changes rebuild the control paint items and geometry from the *cached*
outer box (`rebuild_control_presentation`) and re-emit the display list —
the same zero-Taffy contract as M4b interaction repaints
(`layout_passes` is the test witness). Control chrome (white background,
1px gray ring) paints only where the author styled neither background
nor border.

### 4. UTF-8 byte offsets, grapheme-safe operations

`EditorState` stores `value: String` with `anchor`/`focus` as UTF-8
byte offsets; every operation normalizes to valid boundaries and
moves/deletes by extended grapheme cluster (`unicode-segmentation`).
Pinned: ASCII, precomposed `é`, `a` + combining accent, `😀` — no byte
splitting, offsets always land on char boundaries.

### 5. Keyboard: backend-independent commands, text filtered

`KeyCommand` (Backspace/Delete/arrows/Home/End/Enter/Tab/Escape/
SelectAll) + `KeyModifiers { ctrl, command, shift, alt }` are the
renderer's vocabulary — no winit types cross the API. The shell maps
winit's `logical_key` (and physical `KeyA` for layouts where the
logical key is unlabeled) to commands and forwards `KeyEvent.text` only
for non-command keys: winit delivers `"\r"` for Enter and `"\t"` for
Tab, so **named commands are dispatched before text insertion** — that
is the filter, and the renderer additionally drops control characters
and newlines single-line inputs cannot accept. Ctrl/Cmd+A maps to
SelectAll; Enter inserts a newline only in textarea; Tab keeps
focus-cycling semantics; Escape keeps close semantics. IME events are
deliberately not enabled — `Ime::Preedit`/`Commit` are M4c3.

Only the focused, enabled control receives input. Readonly controls
navigate/select but reject value mutations; disabled controls are
skipped by focus traversal, rejected by every focus entry point, and
refuse editing. Mutations emit `ValueChanged { target, value }` then
`SelectionChanged { target, anchor, focus }` — deterministic order,
test-pinned.

### 6. Pointer capture for selection

Press, hover, and capture are separate runtime concepts. Pressing an
enabled control focuses it (pointer origin), collapses the caret at the
nearest grapheme boundary, records the drag anchor, and captures the
pointer to that node. While captured, moves extend the selection — the
point is mapped through the same document/scroll transforms as hit
testing (clipping deliberately ignored, so selection continues after
the pointer leaves the box). Release ends capture; hover is always
re-derived from the real pointer position independently. Reload clears
capture with all other document-keyed state. None of it lays out.

## Consequences

* One identity model serves scroll, focus, value, and selection events;
  id-less elements are first-class everywhere.
* The M4b state-free facts contract is untouched: `layout_facts`
  deep-equal across editing is test-pinned; control truth has its own
  snapshot (`control_facts`).
* Editing exercises exactly the seams M4c2 (clipboard) and M4c3 (IME)
  will need: value mutation through runtime state, selection ranges in
  byte offsets, caret geometry for `set_ime_cursor_area`, and
  presentation-only invalidation.
* Fixed profile colors for control chrome and selection highlight are
  deliberate (M4b's interaction freeze is not reopened); author CSS
  background/border on controls paint through the structural path.
* Horizontal visibility inside a wrapped line is not narrowed
  (`visible_text_range` is line-based); complex shaping, bidi, password
  masking, spellcheck, and contenteditable remain outside the profile,
  as do clipboard and undo (M4c2) and IME (M4c3).
