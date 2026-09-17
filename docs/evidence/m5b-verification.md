# M5b verification — the reactive binding compiler (ADR 0016)

Scope: the pure Rust compile pass (`velqu-reactive::plan`), the
`ReactiveDom` seam, the deterministic validation rules, the
velqu-view integration (opt-in compile, generation-scoped binding
ids, diagnostics), and the static-document byte-identity contract.
The M5a runtime gate is untouched except the profile hardening below.

## Reviewer profile hardening (applied first, ADR 0015 amendment)

Dynamic code generation is refused at every script-reachable handle.
The natural kill (no Eval intrinsic) is unavailable — QuickJS-NG routes
the host's own `Ctx::eval` through the same `eval_internal` hook (the
probe: the profile prelude itself failed with `eval is not supported`)
— so the refusal is handle-level: `eval`/`Function` globals deleted;
`Function.prototype.constructor` replaced with a throwing stub
(capture-before-delete ordering matters); dynamic `import()`, `@keydown`
… see the battery. Pinned by
`dynamic_code_generation_is_refused_at_every_handle`, including the
promise-machinery subtlety that a throw inside a reaction is absorbed
into the derived promise (asserted via capture, not a job exception).

## The compiler battery (`velqu-reactive/src/plan/tests.rs`, 13 tests)

1. plain document → empty plan (`plain_document_compiles_to_an_empty_plan`).
2. unknown `vx-magic` → one diagnostic naming the expectation, never
   silent (`unknown_directive_is_a_diagnostic_never_silent`).
3. nested `vx-state` scopes → explicit parentage; a binding under both
   resolves to the **nearest** scope
   (`nested_scopes_carry_parentage_and_bindings_resolve_nearest`).
4. malformed sources (empty, unbalanced, unterminated string, handler)
   → four diagnostics tied to node+attribute, nothing lowered
   (`malformed_expressions_diagnose_at_the_node_and_attribute`).
5. duplicate `vx-text` → diagnostic, first occurrence wins.
6. `vx-model` + `:value` → conflict diagnostic, model wins, value
   binding dropped.
7. `vx-model` on `<div>` → compile diagnostic.
8. `@input` on `<h1>` → form-event diagnostic; `@click` on a button is
   legal.
9. `vx-if`/`vx-for`/`vx-computed` → known-but-deferred diagnostics.
10. `:value` on a `<span>` → not-a-control diagnostic.
11. orphan bindings (no scope) → diagnostic.
12. determinism: two compiles of the same DOM deep-equal; the kitchen-
    sink document lowers 1 scope / 4 bindings / 2 events / 0
    diagnostics.
13. `binding_without_a_scope_is_diagnosed`.

## The host-integration battery (`velqu-view` lib tests)

* `m5b_static_documents_stay_byte_identical` — the exit gate's exact
  wording: with reactive enabled, a zero-directive document produces an
  empty plan, identical `LayoutFacts`, and an identical raster hash;
  and a document **with** reactive markup renders byte-identically
  with vs. without the plan compiled (compile is a pure read).
* `m5b_plan_compiles_from_the_dom_with_deterministic_shape` — the
  scope/binding/event shape from real parsed HTML, modifiers preserved
  (`@click.prevent`), unknown `vx-typo` surfacing through
  `reactive_diagnostics()`, and reload-to-same-document compiling the
  identical plan.
* `m5b_reload_invalidates_old_binding_handles` — a `ReactiveBindingId`
  resolves before reload and is a safe no-op after; the new plan mints
  fresh generation-scoped ids.
* `m5b_semantic_rules_surface_as_view_diagnostics` — model-on-div,
  form-event-on-heading, deferred directive, and the model/value
  conflict all surface as public diagnostics with the deterministic
  resolutions (value binding dropped, model kept).

## Gates

* `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets
  --locked` (clean); `cargo test --workspace --locked`; `cargo +1.87.0
  check --workspace --all-targets --locked`; headless dashboard smoke
  digest unchanged (`770b933b…`).

Landing: implementation + docs pushed to `main`; GitHub CI green (run
recorded in the evidence addendum).
