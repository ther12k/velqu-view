# M4c3 gate — IME (pre-registered before implementation)

IME is the one M4c feature whose contract cannot be established by
synthetic unit tests alone, so this gate is recorded **before** the
milestone starts; ADR 0014 will record the design and the freeze
evidence must answer every scenario below.

## Design rules (frozen up front)

* **Preedit is presentation/editor runtime state; only `Ime::Commit`
  mutates the control value.** `ControlState` gains a
  `composition: Option<CompositionState>` carrying the composition
  text, its cursor range, and the replacement range. Preedit text is
  never written into `ControlState.value`.
* Preedit changes pixels and caret/composition geometry — never
  `ValueChanged`, never `LayoutFacts`, zero Taffy passes (the M4b/M4c1
  presentation contract continues).
* **Commit is one atomic edit**: the active composition/selected range
  is replaced by the normalized, filtered committed text — the same
  `ControlKind::filter_text` shared by keyboard insertion and paste;
  no fourth normalization path. Emits `ValueChanged` then
  `SelectionChanged`, once.
* **Composition is session-scoped.** IME events are window-level and
  carry no `ElementHandle`, so composition start retains an
  `ImeSession { owner: ElementHandle, document_generation }` and later
  composition events apply only within that session. Document reload,
  owner removal, focus transfer, or the owner becoming
  readonly/disabled cancels the preedit; only an explicit Commit
  converts it into value text.
* **IME enablement belongs to the shell.** The renderer exposes
  state like `wants_ime()` and `ime_cursor_rect()` (device space); the
  shell calls `Window::set_ime_allowed` / `set_ime_cursor_area`. No
  winit types enter `velqu-view`. The candidate rect updates after
  caret movement, selection movement, internal and ancestor scrolling,
  viewport resize, scale-factor change, and composition text changes.
* **During active composition, Commit is the only text-insertion
  source.** There is an open winit/Windows issue where `KeyboardInput`
  still arrives while IME is enabled: stray `KeyEvent.text` must be
  suppressed while preedit is active, or Windows users get doubled
  characters.
* **Never trust platform indexes.** Preedit cursor/selection byte
  ranges are validated/clamped before use against the Rust string
  (winit 0.30.13 itself fixed an out-of-bounds macOS Pinyin crash on
  its side; Velqu guards its own).

## Freeze scenarios

1. Preedit changes pixels but not value; updating preedit replaces the
   old composition rather than appending.
2. Commit inserts exactly once and clears the composition.
3. Composition over an existing selection replaces it.
4. Blur cancels preedit without committing.
5. A stale Commit after focus transfer cannot modify the new control.
6. Invalid/out-of-range preedit cursor indexes cannot panic.
7. Unicode composition remains UTF-8 safe (byte offsets, grapheme
   boundaries).
8. A readonly control never starts IME state.
9. Ancestor scrolling updates candidate-window coordinates.
10. Active composition causes zero Taffy passes.
11. Windows-style stray `KeyboardInput` during preedit cannot
    double-insert: `Preedit("a")` + stray text `"a"` + `Commit("あ")`
    ⇒ `"あ"`, not `"aあ"`.

## Platform smoke

Windowed IME smoke with real input methods, at minimum Wayland and
X11; Windows and macOS eventually. Unit tests pin the renderer
contract; platform smokes establish the rest.
