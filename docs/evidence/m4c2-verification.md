# M4c2 verification — clipboard behind a host interface (ADR 0013)

Scope: the fallible `ClipboardProvider` trait with its null default,
Copy/Cut/Paste commands with transactional cut, AltGr-safe shell
routing, arboard installation and explicit teardown, and CRLF
normalization. Established gates held: all prior tests stayed green and
no fixture raster or fact changed (the clipboard adds no painting of
its own — cut/paste reuse M4c1's presentation-only edit path).

Review hardening (applied before freeze): the provider is fallible
(`Result`-based) and **cut is transactional** — a failed write keeps
the selection; the null provider refuses writes so a hostless Ctrl+X
cannot destroy text; shell routing gained the AltGr tiers with Alt
disqualifying chords and text suppression; arboard is torn down
explicitly at shell exit.

## The host interface

* `NullClipboardProvider` reads `Ok(None)` and **fails writes**
  (`clipboard.rs` unit test) — the asymmetry that makes null-host cut
  non-destructive; `set_clipboard_provider` flips the
  `custom_clipboard_provider` flag surfaced in `Debug`
  (`m4c2_readonly_and_null_clipboard_policies`).
* `velqu-shell` installs an arboard-backed adapter (text-only build,
  Wayland data-control enabled, X11 via the tree's existing x11rb)
  only when the platform clipboard initializes; headless sessions keep
  the null provider. Platform failures map to `ClipboardError` so the
  renderer's transactional rules apply.

## Command semantics, as tests (recording + failing providers)

* `m4c2_copy_writes_the_selection_and_changes_nothing` — Ctrl+A then
  Copy: the selection reaches the provider; value, events, and layout
  passes are untouched; a collapsed caret copies nothing.
* `m4c2_cut_deletes_and_paste_inserts_through_the_provider` — Cut
  writes `"hello"` and empties the value with the pinned
  `ValueChanged`→`SelectionChanged` order; Paste inserts the
  provider's text (`"pasté"`); an empty clipboard pastes nothing.
* `m4c2_cut_is_transactional_when_the_clipboard_fails` — a provider
  whose write always `Err`s (contention shape): Cut returns false,
  keeps `"precious"` intact, emits nothing; Copy failing is harmless.
* `m4c2_cut_is_grapheme_safe_and_paste_normalizes_crlf` — Shift+Right
  over `😀` then Cut removes it as one grapheme (clipboard receives
  `"😀"`, value `a😀b`→`ab`); `"two\r\nthree"` pastes into a textarea
  as `"two\nthree"` and into an input as `"onetwo"` (newlines dropped).
* `m4c2_readonly_and_null_clipboard_policies` — readonly copies but
  neither cuts (no write, no change) nor pastes; with the default null
  provider, paste reads nothing and **cut preserves the value** —
  Ctrl/X without a usable clipboard is a no-op, not data loss.

## Shell routing (pure `route_keyboard_input` tests)

* Named keys: Enter routes as `Command(Enter)` even with winit's
  `"\r"` text; Tab/Escape likewise — their text never inserts.
* Chords: Ctrl/Cmd+C/X/V map by logical key and by physical key when
  the layout labels it differently (`û` on physical V still pastes);
  plain `c` without modifiers stays text.
* Suppression: Ctrl+K with produced text `"k"` routes `Ignored` —
  modified non-shortcut keys cannot leak text.
* **AltGr regression (reviewer-set):** Ctrl+Alt+Q + text `"@"` routes
  `InsertText("@")` whether the logical key arrives as `q` or `@`;
  `shortcut_chord` never fires under Alt, so nothing is copied.
  Textless Ctrl+Alt+A routes `Ignored`.

## Lifecycle

`run()` releases the provider after the event loop returns: the view
is reset to the null provider and the shell's `Rc` dropped, so the
arboard handle (and the Wayland/X11 clipboard ownership it hosts)
drops deterministically before the process exits.

## Gates

All run and green on this slice:

* `cargo fmt --all -- --check`
* `cargo clippy --workspace --all-targets --locked` (clean)
* `cargo test --workspace --locked`
* `cargo +1.87.0 check --workspace --all-targets --locked` (MSRV lane)
* windowed shell smoke (Wayland): `velqu-lab -- examples/hello
  --exit-after-ms 1500` — 2 frames presented, clean close; headless
  dashboard smoke reproduces the pre-M4c2 digest exactly
  (`770b933b…`), pinning that clipboard code changes no raster.
* a throwaway probe (run, then removed — it requires a display)
  confirmed a real arboard round-trip on this Wayland session:
  `set_text("velqu-probe")` then `get_text()` returned it.

Real OS-clipboard round-trips during *editing* need interactive input;
the deterministic battery pins the renderer through recording and
failing providers and the shell through the pure routing tests.
Landing: implementation + docs committed to `main` and pushed; GitHub
CI green on the pushed commits (see the repo's Actions history).
