# ADR 0013: M4c2 — clipboard behind a host interface

Status: accepted (M4c2, hardened before freeze per review)

## Context

M4c1 gave the renderer editable controls: runtime values, grapheme-safe
editing, selection with pointer capture, and a backend-independent
command vocabulary. Copy/cut/paste were deliberately absent. The
clipboard, however, is platform I/O — exactly the kind of thing ADR 0004
keeps out of the core renderer (no ambient I/O; hosts provide bytes
through `AssetResolver`). The milestone line was set as "clipboard:
copy/cut/paste behind a shell interface".

## Decision

### 1. A fallible host clipboard interface

`ClipboardProvider` is a trait in `velqu-view` whose operations can
fail:

```rust
fn read(&self) -> Result<Option<String>, ClipboardError>;
fn write(&self, text: &str) -> Result<(), ClipboardError>;
```

It is shared-reference callable (like the resolver) so the view can
reach it mid-command. Hosts install it through
`set_clipboard_provider`; the default `NullClipboardProvider` reads
`Ok(None)` but **refuses writes** — see §4. `arboard` (text-only, no
default features, plus the Wayland data-control backend) lives
exclusively in `velqu-shell`, which installs it in `run()` when the
platform clipboard initializes; headless sessions keep the null
provider. Platform failures (contention, a non-text payload, an
unsupported environment) surface as `Err` rather than being swallowed.

### 2. Copy, Cut, Paste join the command vocabulary

Three `KeyCommand` variants map from Ctrl/Cmd+C/X/V in the shell. With
Ctrl/Cmd held, non-shortcut keys produce no command *and no text* —
modified keys can never leak into controls. Browser-consistent
semantics in the renderer:

* **Copy** writes the selection through the provider and changes no
  runtime state (returns `false`, no events, no repaint). A collapsed
  caret copies nothing. Readonly controls copy — they are selectable.
  A failed write is harmless: the editor is untouched either way.
* **Cut is transactional.** It requires a selection and an editable
  control; it writes the selection through the provider and deletes
  the selection **only if the write succeeded**. A failed write leaves
  value and selection untouched: with no undo stack, deleting text the
  clipboard never captured would be unrecoverable data loss (a failed
  asset load never destroys user data either — the null-host analogy
  stops at I/O, not at destructiveness). Successful cut deletes
  grapheme-safely (an emoji selection cuts as one unit) and emits the
  standard `ValueChanged`→`SelectionChanged` pair. Readonly cut is a
  complete no-op, as in browsers.
* **Paste** reads the provider: `Ok(Some(text))` replaces the current
  selection (after filtering); `Ok(None)` (no text) and `Err`
  (unreachable) both no-op. Readonly paste is a no-op.

### 3. CRLF normalization at the filter

Pasted `\r` always drops while `\n` survives in textarea, so Windows
CRLF clipboards arrive as plain `\n`. Single-line inputs lose both.
The filter (`ControlKind::filter_text`) is shared by `insert_text` and
paste, so the insertion paths cannot drift.

### 4. Null-host asymmetry is deliberate

`NullClipboardProvider::read` returns `Ok(None)` (paste safely no-ops),
but `write` returns `Err` ("no clipboard provider installed"). A null
host cannot accept clipboard data, so null-host cut does not delete —
Ctrl/Cmd+X with no usable clipboard is a no-op, not a data-destroying
one. Editing behavior stays *safe* with or without a host clipboard.

### 5. AltGr-safe shell routing

winit 0.30 has no distinct AltGr flag — AltGr arrives as Ctrl+Alt, and
`KeyEvent.text` carries the actually-produced glyph (`@` for Ctrl+Alt+Q
on German layouts). The shell therefore routes in tiers:

1. Named editing/navigation keys are commands before any text
   consideration (Enter's `"\r"`, Tab's `"\t"` never insert).
2. An exact shortcut chord — Ctrl/Cmd + A/C/X/V — is a command. **Alt
   disqualifies the chord**, so Ctrl+Alt+Q can never alias Copy even
   when the physical key matches.
3. A primary shortcut modifier (Ctrl/Cmd) without Alt and without a
   matched chord suppresses text entirely (Ctrl+K inserts nothing).
4. Everything else — plain keys and AltGr-like Ctrl+Alt chords —
   defers to `KeyEvent.text`.

The decision is a pure function (`route_keyboard_input`), unit-tested
including the German-layout regression: Ctrl+Alt+Q + text `"@"`
inserts `"@"`, never Copy.

### 6. Explicit clipboard teardown

arboard's guidance for frameworks that own the event loop (as winit
does) is not to rely on the handle dropping naturally at process exit —
and on Wayland/X11 the application hosts the clipboard selection. The
shell therefore releases the provider (resetting the view to null and
dropping its own `Rc`) immediately after the event loop returns, before
`run()` completes.

## Consequences

* The renderer gains no I/O: everything platform-specific stays behind
  the two-method trait, and the arboard dependency is confined to the
  shell.
* Cut/paste ride M4c1's `finish_control_change` — events, ordering,
  presentation-only repaint, and the no-Taffy guarantee all come free.
* The trait is text-only by design; image/RTF clipboard formats are out
  of profile. Undo/redo remains deferred (it needs a value-history
  design, not a clipboard).
* Real OS-clipboard round-trips can't be synthesized headlessly; the
  deterministic battery pins the renderer through a recording provider
  and a failing provider (transactional cut), and the shell through
  pure routing tests, with interactive verification left to windowed
  smoke.
