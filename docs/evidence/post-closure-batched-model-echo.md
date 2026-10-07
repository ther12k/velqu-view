# Post-closure correction 0003: batched model echo write-backs corrupt live editing

Status: repaired (2026-10-07) · Scope: `crates/velqu-view` — the reactive
pump's mutation application and one `editor.rs` guard · Linked from
[`phase1-closure.md`](phase1-closure.md).

## The defect

**Typing into a `vx-model` control while two or more `ValueChanged`
events accumulate in one pumped batch corrupts the control's text
deterministically** — characters drop mid-string and reappear at the
end ("Correction Candidate" became "Crrection Candidatee"). One event
per batch (a redraw after every keystroke) was unaffected, which is
why the M5/M7 conformance suites and the paced pilot runs never saw
it.

Found by the pilot's simulated technical acceptance (2026-10-07,
`velqu-view-pilot/SIMULATED-ACCEPTANCE-2026-10-07.md`, disposition
revise): fast synthetic typing produced wrong stored values, and a
screenshot showed the malformed text *before* Save. Isolated
headlessly through the public API (no X11, no synthetic input):
grouped `insert_text` + drain + `pump_reactive` reproduces it on
`3c1ee34` for batch groupings of 2+ `ValueChanged` events, with the
exact corruption shape the pilot observed.

## Root cause (both parts)

A `ValueChanged` turn's model round-trip re-applies the event's own
value as a silent `SetControlValue` on the same control — an **echo**.
With several edits in one batch the editor's live value is ahead of
each snapshot, and:

1. `apply_mutations` wrote every echo back through
   `Editor::set_value`, which replaces the value wholesale;
2. `set_value` clamps the caret *down* into the new (shorter, stale)
   value and never restores it, so after the batch the caret sits
   `k−1` graphemes left of the end for `k` echoed snapshots — every
   later keystroke inserts one position early.

## The repair

* **Echo screening in the pump** (`pump_reactive`): for a
  `ValueChanged` turn, `SetControlValue` mutations on the event's
  origin control whose value equals the event's value are dropped
  before application. The editor's live value is the newest truth for
  user input; the model still converges through the ordered model
  writes. A binding output that *differs* from the event's value (a
  handler changed the model) is a real programmatic update and still
  applies — pinned below.
* **`Editor::set_value` equal-value no-op**: writing the identical
  value is now a complete no-op instead of re-clamping the selection —
  defense in depth for any other caller.

## Evidence

Headless public-API probe (grouped typing of "Correction Candidate",
groups = ValueChanged events per pumped batch):

| Grouping | `3c1ee34` | repair |
| --- | --- | --- |
| 1 per batch | `"Correction Candidate"` | unchanged |
| 2 per batch | `"Correction Candidate"` | unchanged |
| 4 per batch | **`"Crrection Candidateo"`** | `"Correction Candidate"` |
| 8 per batch | corrupt | `"Correction Candidate"` |
| all in one batch | corrupt | `"Correction Candidate"` |

After the repair the probe also confirms: the caret stays at the
value's end for every grouping; a *stale* echo batch pumped after
further typing no longer rewinds the live editor (previously it
clamped to its last snapshot); interleaved two-control editing
converges; mid-string insertion (`Home`, then typing) stays exact
under batching; and the application persists exactly what
`insert_text` receives (the pilot's stored garbage is fully explained
by this defect — the synthetic input lane delivered ordered batches
and is not the culprit).

Regressions (fail on `3c1ee34`, pass on the repair):
`m5c_batched_value_changed_turns_keep_value_and_caret_exact` and
`m5c_model_echo_write_backs_never_rewind_a_live_editor`
(`crates/velqu-view/src/lib.rs`).

Full gate at the repair commit: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace --locked` (all green; frozen digests
unchanged), `cargo +1.87.0 check --workspace --all-targets --locked`.

## Explicitly not closed by this repair

The operator application (`velqu-view-starter`, candidate `bba9ddd`)
pins runtime `1d0a562` and still carries the defect. Adopting this
repair is the separately named follow-up candidate (a runtime repin
plus differential windowed re-run, then re-freezing the pilot session
boundary). The supervised human session remains separate and
unaffected in its preparation-only status.
