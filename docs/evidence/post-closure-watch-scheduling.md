# Post-closure correction 0001: live-window watch scheduling

Status: repaired (2026-09-20) · Scope: `apps/velqu-lab` only — no
renderer, shell, CSS, or reactive changes · Linked from
[`phase1-closure.md`](phase1-closure.md).

## The defect

**Live-window `velqu-lab --watch` reconciliation was broken at
`1d0a562`: the window never reconciled a save on its own — for any
backend, any number of saves, with or without unrelated input.** The
M6c evidence base (coordinator and transaction tests with injected
clocks and injected reads) does not establish end-to-end windowed
delivery, and the closure record's "complete dev loop" claim carried
this hidden qualification until this repair.

Found by the external-consumer check: the standalone starter app
(`velqu-view-starter`, its `docs/verification.md` §3) reproduced the
failure across seven lab runs while proving each half of the chain
sound in isolation (the shell's `Watch` proxy→hook path delivers
within ~10 ms; notify delivers inotify events on the watched
directory; the lab's exact composition through the public API runs
the hook at edit time).

### Root cause (both parts in velqu-lab)

1. **The debounce clock restarted on every wake.** The startup rescan
   leaves the coordinator's dirty set non-empty (it drains only
   inside a reconcile), and `notify` set `last_change_ms := now` on
   *every* notification while the dirty set was non-empty — even ones
   matching no registered path. The host hook runs only at wake
   instants, so `now − last_change ≈ 0` at every invocation and
   `due()` (quiet ≥ 120 ms) could never hold.
2. **Nothing woke the loop at the deadline.** Under
   `ControlFlow::Wait` the event loop sleeps until an event; no timer
   or delayed re-send existed for "the quiet interval elapsed".
   Unrelated input does not help — the shell runs the watch hook only
   on `Watch` wakeups.

## The repair

The two concepts the correction requires — a **relevant filesystem
notification** changes the deadline; a **wake** only checks it:

* `Coordinator::notify` moves the deadline only for relevant
  notifications: a changed path that is registered — **including a
  re-edit of an already-dirty source** (bursts extend) — or a rescan.
  Irrelevant wakes never postpone registered-source work.
* `Coordinator::next_deadline_ms` exposes the pending deadline (the
  thing the host must schedule); quiescence is `None`.
* `Coordinator::defer` schedules exactly one bounded retry (deadline
  = deferral + quiet), so a missing/failed read neither hot-loops nor
  waits for a filesystem event that may never come.
* `watch::DeadlineScheduler` — one worker per watched session, a
  single current deadline (`Instant`, monotonic), a condition
  variable whose wait rechecks the predicate (spurious/interrupted
  wakes are safe), replace-extends semantics for burst re-edits,
  explicit idempotent shutdown (Drop), and idle-when-no-deadline. It
  invokes the existing loop-proxy wake; it never sends recursively
  and never turns native watching into polling.
* The `run_window_watch` hook drains, reconciles when due, then
  re-arms the scheduler from coordinator truth — pending debounce or
  deferral retry get a real future wake; quiescence arms nothing.

Startup reconciliation is now real: the registration-gap rescan
completes as a scheduled pass (counted, then quiescent) instead of
restarting the quiet interval forever.

## Evidence

The decisive regression is
[`apps/velqu-lab/tests/live_watch.rs`](../apps/velqu-lab/tests/live_watch.rs)
— the **actual lab binary**, a real window, one registered save, and
**no further input, second save, or injected events of any kind**
(CI has no display lane; run with `--ignored` locally).

* **Fails on `1d0a562`** (both backends): *"no reconciliation within
  15 s of a single registered save and no further input — the window
  never wakes."*
* **Passes on the repair** — the full gate matrix:

| Scenario | Result |
| --- | --- |
| One edit, then no further input (native **and** polling) | reconciles and publishes `kind=Stylesheets` after the quiet interval |
| Second edit to the same path before expiry | deadline extends; exactly one publication of the latest contents |
| Early timer wake / unrelated input | deadline not reset; pending work completes (unit: irrelevant wakes never move it; scheduler: extend replaces) |
| Unregistered-file activity | does not postpone the registered edit; no publications of its own |
| Startup rescan, no later edits | completes (one unchanged-skip) and returns to quiescence |
| Rejected edit, then valid repair | old UI runs until exit; repair publishes on the same process |
| Shutdown with a timer pending | clean exit, no hang, no rescheduling |

Live lanes: **X11 verified** (`DISPLAY=:1`). **Wayland lane
unavailable** on the verification machine (no compositor socket,
`WAYLAND_DISPLAY` unset) — recorded, not substituted.

Coordinator policy stays pinned by the injected-clock suite (27 lab
unit tests, including the counter-at-7 state-preservation probes
`invalid_then_restored_snapshot_leaves_the_application_running` and
`css_edit_preserves_state_and_the_runtime_operates_afterward`, which
prove the scheduler reaches the transactional stylesheet path —
state survives — rather than a full-document restart). The live
publication lines carry `kind=Stylesheets generation=None` as the
same evidence at the binary level.

Full gate at the repair commit: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked` (zero warnings),
`cargo test --workspace --locked` (all green), `cargo +1.87.0 check
--workspace --all-targets --locked`; `git diff` confined to
`apps/velqu-lab` and this record.

## Explicitly not closed by this repair

The **idle-wakeup count / resource-cost qualification** under
`--watch` (closure record, open item 2) remains open: this repair
closes a *functional* scheduling defect, and the scheduler adds one
idle worker thread that wakes only at deadlines. Measuring wakeup
counts and long-run memory behavior is still release-qualification
work against the external starter.
