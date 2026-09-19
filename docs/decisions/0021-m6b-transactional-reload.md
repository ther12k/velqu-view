# ADR 0021: M6b — transactional reload; both paths are transactions

Status: accepted (M6b)

## Context

M6a froze the inspector. M6b adds the development loop's core: a
reload either publishes a fully prepared, coherent replacement, or
leaves the active application usable and unchanged except for reload
diagnostics. The reviewer's central rule:

> Nothing becomes live until preparation succeeds. A rejected attempt
> changes reload diagnostics, not application state.

## Decision

### 1. Two operations, one publication rule

* **Full-document reload** — `reload_document(source, viewport)`:
  prepares a candidate **through its first rendered frame** and
  publishes a new generation atomically.
* **CSS-only reload** — `reload_stylesheets(replacements, viewport)`:
  stages replacement sheets against the **current committed document**
  and publishes within the existing generation.

### 2. Full-document candidates prepare through the first frame

Not only through mutation validation:

```
read source bundle
→ parse HTML (error-recovering by design)
→ compile Tailwind output + reactive plan + executable units
→ construct candidate runtime, run scope initializers
→ apply + validate initial mutations inside the candidate
→ resolve first-frame assets under the existing policy
→ compute style/layout, build display list, rasterize
→ publish (atomic generation swap)
```

Preparing stops short of nothing fallible: a candidate whose
initialization succeeds but whose first frame would fail never
replaces the working document. Missing-image placeholders remain
acceptable (the policy is "can produce a first frame", not "every
asset succeeds"). Preparation is synchronous and bounded to **one
candidate** under the host's existing source/asset/JS budgets — a
reload is never an unbounded second copy of the application. The
candidate prepares for the caller-supplied viewport; a changed
viewport before publication re-validates on the next render through
the ordinary cache-mismatch path.

### 3. Generation identity from the lifetime authority

Generation ids come from one monotonic mint counter shared by loads
and candidates. A candidate reserves its id before preparation; a
failed attempt consumes the id (a visible gap in the sequence) but
never changes the active generation, and can never re-mint an
already-published generation — old handles stay invalid forever.

Publication moves **document-owned state only** (DOM, styles, reactive
runtime and state, controls, focus/scroll/composition, layout caches,
per-generation revisions). Host lifetime — installed asset resolver,
pipeline flags, limits, inspector history and its monotonic seqs, the
reload ledger — survives every swap. Publication happens between
completed event batches; the old document's queued events do not
survive (their handles are stale by construction). A successful full
reload cancels any active IME composition with the old document; a
rejected one leaves the live control and composition untouched.

### 4. CSS-only reload stages the committed document

The subtle failure this design excludes: reparsing the original HTML
would restore initial text/classes while the JS runtime still says
`count = 7` — state and document inconsistent even though the
generation survived. Instead, the replacement sheets are staged
against the live committed DOM and runtime control state: **no
reparse, no scope initializers, no turn zero, no new QuickJS
runtime.** Each replacement upserts **in place** by `SourceId`
(cascade position preserved; order of appearance is a tie-breaker, so
moving a sheet would change results); unknown ids append; a
multi-sheet replacement publishes as one ordered set.

Staging runs on the live instance inside one synchronous call — the
DOM, controls, and reactive runtime are untouched by staging; the
renderer-visible side effects (caches, counters, trace, diagnostics,
dirty flags) are snapshotted first and **restored wholesale on
rejection**, so a rejected attempt is bit-identical to before except
for reload diagnostics. Nothing is observable mid-transaction
(single-threaded; no frame presents between stage and verdict).

Interaction state survives and is then **reconciled** with the new
layout — legitimate consequences, not resets: a focus hidden by the
new CSS clears, hover re-derives under the stationary pointer, scroll
offsets re-clamp in the next render's apply stage. The shell refreshes
the IME candidate rectangle after the next redraw (window operations
stay shell-side).

### 5. Reload acceptance policy

Parser recovery, compatibility reporting, and reload acceptance are
distinct. HTML parsing is error-recovering by design; "a DOM came
out" is not acceptance. Conversely "any diagnostic exists" is not
rejection — that would turn every deferred-feature warning into a
failed reload.

| Condition | M6b behavior |
|---|---|
| Required source unusable (empty per the existing primitives) | Reject; retain live document |
| Reactive compilation of executable units fails (poisoned units) | Reject |
| Reactive initialization fails (initializer threw / non-plain / capture) | Reject |
| Initial mutation batch fails validation | Reject |
| First-frame preparation returns an error | Reject |
| Existing non-blocking normalization/compatibility diagnostics | Accept with diagnostic |
| Malformed-but-recoverable markup/CSS | Accept (parser-defined meaning) |

The strongest fixtures are parse-succeeds-but-unpublishable
candidates: a throwing initializer, a poisoned-but-balanced expression
(`(+)`), and a `:checked` binding whose initial batch cannot validate.

### 6. Failed-attempt diagnostics are separate from the active document

The ledger (`ReloadAttempt`: monotonic id, kind, outcome, stage,
generations before/after) is host lifetime: the inspector can say
"active generation 12; last attempt 19 rejected at reactive
initialization; active generation still 12". A rejected candidate's
speculative turns, mutations, and layout never enter the application
trace — they ran on the candidate, not the view (CSS staging records
are checkpoint-rolled-back). Both outcomes append one `Reload` trace
record.

### 7. Maintenance contract for the stylesheet transaction (M6b freeze note)

Staging on the live instance with snapshot-and-restore is a
synchronous implementation choice, not a shape requirement. Its rules:

* **One restoration path.** Every handled failure exits through the
  single restore block; speculative work cannot publish
  application-visible effects before success.
* **The rollback inventory is centralized** in the one snapshot/restore
  pair at the transaction's edge. As new renderer state appears, the
  danger is a field render mutates but the snapshot forgets — any new
  renderer-visible state must join both sides.
* **Renderer rollback ≠ external effects.** Preparation may call an
  asset provider that performs reads or updates its own caches;
  restoring Velqu's state cannot undo provider-side actions. Native
  cursor/IME changes, application observer callbacks, and window
  presentation remain **after successful publication**, never inside
  speculative preparation.

## Consequences

* The reviewer's counter probe is pinned end-to-end: run to 7 with
  typed value, selection, focus, and scroll → a policy-rejected CSS
  replacement (nothing moves) → a color-only CSS replacement (state,
  control, focus, scroll, and cascade order all survive; the runtime
  still increments to 8) → a throwing-HTML rejection (old UI keeps
  accepting input) → a valid replacement (fresh generation,
  source-defined state, old handles rejected, inspector history
  intact).
* CSS reload does not construct an incremental style engine: the
  existing invalidation classification applies (a restyle is a
  structural pass); the gate is transactional correctness and state
  continuity.
* Filesystem notification, debounce, coalescing, and rename detection
  are M6c; M6b's API is the primitive they will drive.
