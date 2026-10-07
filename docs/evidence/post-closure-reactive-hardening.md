# Post-closure correction 0002: reactive profile hardening + initial-turn reload acceptance

Status: repaired (2026-10-07) · Scope: `crates/velqu-reactive` (profile
preludes, diagnostic sink, machine) + one acceptance check in
`crates/velqu-view` — no shell, lab, CSS, or layout changes · Linked
from [`phase1-closure.md`](phase1-closure.md).

Found by a whole-system review of the `3c1ee34` tree: four defects,
each reproduced against the framework's own locked dependencies through
the public API before any fix was written, and each now pinned by a
focused regression that fails on `3c1ee34`.

## Defect 1 — oversized multi-byte logging panicked the host (P1)

`console.log('€'.repeat(22000))` under default limits (byte cap 65536,
`8192 % 3 != 0`-style boundaries land mid-code-point) reached
`String::truncate` in the sink closure and **panicked the host process**
— violating the "failures are diagnostics, never fatal" contract
(ADR 0015 §6). Any application logging non-ASCII text could hit it; no
hostile bundle required.

Fix: the sink caps through the existing char-boundary-safe
`cap_string`. Regression: `unicode_logging_is_capped_on_char_boundaries`
(3-byte and 4-byte code points, caps chosen to land mid-glyph).

## Defect 2 — reload published candidates whose turn zero rolled back (P1)

`ReactiveMachine::new` mapped a rolled-back initial evaluation to "no
initial mutations" and returned success; `reload_bundle` checked
initializer failures, poisoned units, and mutation-batch validity — but
not initial-evaluation failure. Both candidates published, replacing a
working application with one whose bindings never committed:

- `<p vx-text="missingIdentifier">` — binding throws at turn zero;
- `<p :class="'x'.repeat(70000)">` — output exceeds the byte budget.

Fix: the machine retains turn zero's rollback reason
(`initial_turn_failure()`); `reload_bundle` rejects such candidates at
the existing `ReactiveInitialization` stage. Regression:
`m6b_initial_binding_failure_blocks_reload` (both shapes reject; the
active document stays interactive). ADR 0021's acceptance table gains
the corresponding row (amendment, below).

Not changed: ordinary compatibility diagnostics remain non-blocking, and
a *first load* of such a document still renders with diagnostics — only
reload acceptance tightens.

## Defect 3 — async/generator constructors rebuilt code (P2)

The M5a.1 refusal patched `Function.prototype.constructor`, but the
async, generator, and async-generator families carry **their own**
prototype objects, each with a live compiler. All three compiled and
returned:

```js
(async function(){}).constructor('return 42')   // Ok
(function*(){}).constructor('yield 42')         // Ok
(async function*(){}).constructor('yield 42')   // Ok
```

The ADR 0015 amendment claimed these paths were covered; the claim was
wrong, not the goal. Fix: the prelude patches `constructor` on all four
family prototypes. Regression: the three probes (plus an async arrow)
join `dynamic_code_generation_is_refused_at_every_handle`. No capability
beyond script-side compilation was demonstrated — this closes a policy
bypass, not an OS-sandbox hole (the runtime never had ambient I/O to
reach).

## Defect 4 — the native wall clock leaked through the Date shim (P2)

`LogicalDate` subclassed the native `Date`, so the native constructor
stayed reachable through the subclass's [[Prototype]]:

```js
Object.getPrototypeOf(Date).now()                  // wall clock
new (Object.getPrototypeOf(Date))().getTime()       // wall clock
```

determinism contract broken for any state or binding touching those
handles. Fix: the shim severs both reachable handles — the subclass
prototype is re-parented to the (deleted) function prototype, and
`Date.prototype`'s inherited `constructor` re-points at the shim; the
pure statics `parse`/`UTC` are copied across the sever. Regression:
`the_native_clock_is_unreachable_from_the_profile` walks every handle,
asserts the severed prototype refuses compilation, and pins `Date.UTC`.

## Evidence

Each fix verified through the same locked-dependency probe harness that
reproduced the defects (public `velqu-reactive`/`velqu-view` API only):

| Probe (public API) | `3c1ee34` | repair |
| --- | --- | --- |
| `console.log('€'.repeat(22000))` | host panic (exit 101) | `Ok`, capped diagnostic |
| `reload_document` of a throwing-binding candidate | `Ok(2)` published | `Err`, stage `reactive initialization` |
| `reload_document` of an oversized-output candidate | `Ok(2)` published | `Err`, stage `reactive initialization` |
| valid candidate reload | `Ok` | `Ok` (unchanged) |
| async/generator `.constructor(...)` ×3 | compiled | refused `TypeError` |
| `Object.getPrototypeOf(Date).now()` | wall-clock ms | `TypeError: not a function` |

Full gate at the repair commit: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace --locked` (all green, including the frozen
reference-dashboard digests — **no baseline raster moved**), and
`cargo +1.87.0 check --workspace --all-targets --locked`.

## Explicitly not closed by this repair

- The external consumers pin `1d0a562` and still carry all four
  defects. The operator application does not exercise any of the four
  paths (no oversized non-ASCII logging, a fixed valid template, no
  async-constructor or Date-prototype use), so the pilot and the
  supervised-human-session candidate are unaffected. Adopting this
  repair there is a future owner-directed repin, not done here.
- Wayland, live IME, and idle-wakeup qualification remain exactly as
  scoped in the closure record — untouched by this change.
