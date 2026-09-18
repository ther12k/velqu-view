# forms

The reactive forms example: two-way `vx-model` bindings over `<input>`
controls, live `vx-text` derivations, `:disabled`/`:class` attribute
bindings, and `vx-show` confirmation — all styled with Tailwind
utilities and no CSS files.

Run: `velqu-lab --tailwind --reactive examples/forms`.

Typing goes through the M4c1 editor (grapheme-safe, IME-aware); each
keystroke is one ValueChanged event → one atomic reactive turn → the
model write lands before any `@input` handler. Conformance is pinned
by `crates/velqu-view/tests/reactive_examples.rs`.
