# ADR 0022: M6c — file watching as host-side reconciliation

Status: accepted (M6c)

## Context

M6b froze the transactional reload APIs. M6c turns noisy, potentially
incomplete filesystem notifications into bounded reconciliation
attempts against them. The architectural rule:

> Filesystem notifications say that sources **may be stale**. The
> current source contents determine whether — and what — to reload.

## Decision

### 1. The shape

```
native watcher / polling backend (notify)
        ↓
bounded dirty-source set + wake (event-loop proxy)
        ↓
host-side quiet interval (debounce, injectable clock)
        ↓
read one bounded snapshot of the registered sources
        ↓
compare against attempted / published inputs
        ↓
reload_stylesheets(…) OR reload_bundle(…)
        ↓
record outcome; present only successful results
```

Everything filesystem-shaped — notify, reads, debounce timers, path
mapping — lives in the **host** (`velqu-lab`), outside velqu-view. The
renderer receives the same explicit source objects M6b accepts. The
watcher never reloads: it marks sources dirty and wakes the loop
(`EventLoopProxy<ShellUserEvent>`); the hook runs on the main thread
with the view, and only a published reload redraws.

### 2. Registry, dirty set, and the M6a.1 lesson

A host-owned registry maps logical path → stable `SourceId` → role →
established cascade order. The watched object is "the source at this
logical path", not the original inode: an editor replacing
`styles.css` (temp file + rename) keeps the id and the cascade
position. Watchers attach to **parent directories** (notify documents
surprising behavior for individually watched files under rename).

Reconciliation **takes** the dirty set (a snapshot); notifications
arriving meanwhile populate the next set — the same ownership fix
reactive pumping needed. A dirty set is never cleared after a reload.

### 3. Observed ≠ attempted ≠ published

Three snapshots, byte-exact (equality is the fingerprint; the
coordinator compares the bytes it actually passes to M6b):

* **observed** — just read;
* **last attempted** (+ the view's ledger outcome) — repeat attempts
  against the same deterministically invalid input are skipped (no
  initialization/error storm); because the snapshot covers **all**
  registered inputs, any dependency change breaks the equality and
  unblocks recovery;
* **published** — only successful publication advances it, so
  restoring the published bytes after a rejected edit produces **no
  reload at all** (the running application is untouched).

A manual reload deliberately bypasses unchanged suppression and routes
as one full-bundle transaction — automatic "nothing changed" and a
user-requested restart are different operations.

### 4. Routing (narrow and explicit)

| Changed inputs | Operation |
|---|---|
| Registered stylesheets only | one `reload_stylesheets`, order preserved |
| Document source (± CSS) | one `reload_bundle` — never a published CSS intermediate followed by a document attempt |
| Bytes identical to published | no reload, no render |
| Reads missing/failed | deferred: the old application keeps running, the paths stay eligible — **missing is not empty** |

v1 watches explicitly registered inputs only; dependency discovery
(and its repair trap: new HTML referencing a not-yet-existing sheet)
is out of profile and stated as a limitation.

### 5. Notification loss and backend honesty

`need_rescan()` — and the coordinator's own overflow signal — mean
"any tracked source may have changed": one bounded reread of the
registered set, never event reconstruction. Raw events coalesce into
one notification per wakeup and live only in bounded diagnostics.

Native watching is an optimization, not a guarantee (network
filesystems, containers): `--watch=poll` selects `PollWatcher`, and
both backends feed the **same** reconciliation logic — polling has no
separate reload semantics.

### 6. What the debounce proves

A quiet interval reduces noise; it does not prove a save is complete,
and the host cannot infer editor intent from event sequences. The
pinned claim is: **duplicate notifications for the same settled input
snapshot cause at most one attempt** — plus no-notification-loss
during earlier work. A change during preparation belongs to the next
reconciliation (retained, processed after).

## Consequences

* The acceptance battery splits into deterministic coordinator tests
  (injected clock + injected reads + synthetic notifications) and
  native integration tests (temporary directories, bounded waits for
  outcomes — never exact event counts; Linux watcher evidence is
  Linux evidence).
* The inspector additions stay small: `WatcherStatus` (dirty count,
  unchanged/repeat skips, deferrals, rescans, reconciliations) lives
  beside the existing reload ledger — no parallel tracing.
* Tailwind inputs regenerate through the ordinary load path inside a
  reload transaction; the watcher does not watch generated output.
