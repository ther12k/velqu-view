# M6c verification — file watching as host-side reconciliation

The exit criterion: the host turns noisy, potentially incomplete
filesystem notifications into bounded reconciliation attempts,
invokes only the existing transactional reload APIs, and preserves
the running application whenever an attempted update fails.

## What landed

* `apps/velqu-lab/src/reload_coordinator.rs` — the reconciliation
  state: source registry (stable `SourceId` per logical path,
  established cascade order), bounded dirty set + quiet interval
  against an **injected clock**, and the observed / last-attempted /
  published snapshot triple (byte-exact; missing ≠ empty — read
  failures defer, never become empty strings). Routing:
  stylesheet-only → one `reload_stylesheets`; any document change →
  one `reload_bundle` (the M6c view addition: document + sheets as
  one transaction, publishing the candidate's sheet state);
  unchanged → nothing; manual → forced full bundle.
* `apps/velqu-lab/src/watch.rs` — notify 8.2 wrapped in the host
  boundary: **parent-directory** watching (temp-file replacement
  safe), content-bearing event kinds only, coalesced notifications,
  `need_rescan`/overflow → one bounded rescan, `PollWatcher` option,
  explicit shutdown (late callbacks inert).
* `velqu-shell` — `run_with_watch`: `EventLoopProxy<ShellUserEvent>`
  wakes the `Wait` loop; the host hook runs on the main thread with
  the view; only a hook-returned "repaint" redraws the document.
* `velqu-lab --watch` / `--watch=poll` — window-mode hot reload over
  the app directory (`index.html` + sorted `*.css`).
* `velqu-view::reload_bundle` — the combined transaction (ADR 0021's
  acceptance policy unchanged).

## Battery

Deterministic coordinator tests (injected clock + injected reads +
synthetic notifications; `reload_coordinator/tests.rs`):

| Scenario | Test |
|---|---|
| Duplicate notifications in one batch → one attempt (and none for unchanged bytes) | `duplicate_notifications_coalesce_into_one_attempt` |
| Metadata/no-content change → no reload/render/generation | `unchanged_content_never_reloads` |
| Invalid B after A, then A restored → B rejected once, no storm, no reset | `invalid_then_restored_snapshot_leaves_the_application_running` |
| Valid CSS edit at counter 7 + control state → survives, still increments | `css_edit_preserves_state_and_the_runtime_operates_afterward` |
| HTML + CSS in one batch → one full transaction, no CSS intermediate | `combined_batch_is_one_full_transaction` |
| Notification during preparation → pending, reconciled after | `notifications_during_reconciliation_remain_pending` |
| Missing source then recreation → defers, recovers | `missing_source_defers_then_recovers_on_recreation` |
| Rescan signal → tracked set reread, latest contents found | `rescan_rereads_the_tracked_set` |
| Shutdown → no work; late notifications inert | `shutdown_stops_all_work` |
| Quiet interval gates; restarts on new noise | `quiet_interval_gates_reconciliation` |
| Read errors transient (not memoized) | `read_errors_are_transient` |
| Manual reload bypasses unchanged suppression | `manual_reload_bypasses_unchanged_suppression` |

Native integration tests (real notify backends, temporary
directories, bounded 8s waits; `watch/tests.rs`):

| Scenario | Test |
|---|---|
| **Replacement save twice** (temp+rename): both detected, identity + cascade position preserved, final color proves both landed | `native_css_replacement_save_twice` |
| Polling backend → same coordinator, same semantics | `polling_backend_feeds_the_same_reconciliation` |
| Document replacement save routes to the full bundle | `native_document_save_routes_to_a_full_bundle` |
| Shutdown inert (save after shutdown → no reconciliation, old color) | `shutdown_inert` |

## Engine notes

* **Polling startup vs timestamp-only detection — the precise causes
  (M6c.1 review follow-up, verified against the notify 8.2.0
  source).** Two distinct mechanisms, two distinct fixes:
  1. **Startup ordering**: registering a path builds the initial
     `WatchData` snapshot synchronously — a source edited between the
     host's initial read and registration produces no event ever (the
     registration *is* the baseline). Fix: the watcher thread sends
     one forced `Rescan` immediately after registering its directories
     (ADR 0022's startup reconciliation), so the coordinator compares
     disk against what the application actually published,
     independently of any event. Pinned by
     `startup_reconciliation_closes_the_registration_gap`
     (coordinator) and
     `native_startup_registration_gap_closes_without_a_save` (the real
     sequence under the native backend).
  2. **Timestamp-only change detection**: notify truncates mtimes to
     whole seconds and compares "newer mtime or different content
     hash"; content comparison is off by default. An in-place edit
     inside the same recorded second is invisible to timestamp-only
     polling. Fix: the poll backend enables
     `Config::with_compare_contents(true)` (cost bounded by the
     watched app directories). Pinned by
     `polling_detects_same_second_edits_via_content_comparison`,
     which writes the baseline and the edit inside one wall second and
     asserts the whole-second mtimes match (so only content comparison
     can converge).
* `PollWatcher::poll()` sends a request to its worker; its return is
  not a completion barrier — the tests wait for the observable
  outcome (a publication), never treating it as synchronization.
* Native watcher evidence is Linux evidence.
* notify 8.2.0 resolved from crates.io; MSRV 1.87 verified against
  the actual feature set (lockfile evidence in CI's msrv job).

## The focused reload_bundle regression (M6c.1)

`m6c_reload_bundle_preserves_author_sheets_or_publishes_nothing`
(velqu-view): an HTML-only edit published through the full-bundle
route with the intended sheet set keeps A and B present, in order,
with their computed-style effect visible in the replacement; a
combined edit that rejects (empty sheet in the bundle) publishes
**neither** the HTML nor the CSS change; and the convenience APIs'
replacement semantics are documented and pinned
(`reload_document` resets the sheet set; `reload_stylesheets`
upserts in place; `reload_bundle` replaces with the given set).

## Gates

* `cargo fmt --all -- --check`; `cargo clippy --workspace
  --all-targets --locked` (clean); `cargo test --workspace --locked`
  (320 tests: 12 coordinator + 4 native + all prior suites);
  `cargo +1.87.0 check --workspace --all-targets --locked`; headless
  dashboard smoke digest unchanged (`770b933b…`); M5 conformance and
  the M6b battery unchanged.

Landing: `d8d49f1` pushed to `main`; GitHub CI green — run
[35431174182](https://github.com/ther12k/velqu-view/actions/runs/35431174182)
(fmt/clippy/test/build + MSRV 1.87 with notify 8.2.0, both jobs).
