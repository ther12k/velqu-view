# counter

The Velqu Reactive v0 example, transcribed from the OKF spec
(`examples` → counter in the bundle; `docs/okf/`).

Interactive since M5c/M5d: `velqu-lab --tailwind --reactive
examples/counter` opens a window where the buttons drive bounded
reactive turns (state + UI commit atomically; each structural turn
costs exactly one layout pass). The `vx-*` / `@click` surface is the
frozen v0 directive set (see `crates/velqu-reactive`, ADR 0016/0017).

Conformance is pinned by `crates/velqu-view/tests/reactive_examples.rs`
(this document, event-driven, digest-checked).
