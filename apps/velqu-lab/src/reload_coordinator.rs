//! The reload coordinator (M6c, ADR 0022).
//!
//! **Filesystem notifications say that sources may be stale. The
//! current source contents determine whether — and what — to reload.**
//!
//! The coordinator owns the host-side reconciliation state: a source
//! registry (stable identity per logical path, established cascade
//! order), a bounded dirty set + quiet interval (debounce against an
//! injected clock), and three distinct snapshots —
//!
//! * **observed** — the inputs just read from disk;
//! * **last attempted** — the last bundle handed to a reload, with its
//!   outcome (repeat attempts against the same deterministically
//!   invalid input are skipped, without blocking recovery when any
//!   registered input later changes);
//! * **published** — the inputs the active application runs; only a
//!   successful publication advances it.
//!
//! Notifications never reload directly: they mark sources dirty and
//! wake the host. Reconciliation takes a dirty-set snapshot (the
//! M6a.1 lesson — notifications arriving meanwhile populate the next
//! set), reads one bounded snapshot of the registered sources, and
//! routes to the existing transactional reload APIs:
//! stylesheet-only changes → one `reload_stylesheets`; any document
//! change (alone or with CSS) → one `reload_bundle` — never a
//! published CSS intermediate followed by a document attempt.
//! Missing or failed reads are **not** empty strings: the bundle
//! defers (the old application keeps running, the paths stay
//! eligible), never attempting a reload against partial observation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use velqu_view::{ReloadKind, ReloadRejection, SourceId, VelquView, Viewport};

/// One registered input: the source at a logical path (a stable
/// identity that survives editor replacement saves — the watched
/// object is "the source at this path", not the original inode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredSource {
    /// The logical path the host maps notifications onto.
    pub path: PathBuf,
    /// The stable `SourceId` used in every reload transaction.
    pub id: SourceId,
}

/// The tracked source set: the document and the author stylesheets in
/// their established cascade order. Everything else the watcher may
/// see is ignored (v1: explicitly registered inputs only — no
/// dependency discovery).
#[derive(Debug, Clone)]
pub struct SourceRegistry {
    /// The HTML/reactive document source.
    pub document: RegisteredSource,
    /// Author stylesheets; the Vec order is the cascade order.
    pub stylesheets: Vec<RegisteredSource>,
}

impl SourceRegistry {
    /// Every registered path (document first, then sheets in order).
    pub fn paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.document.path.clone()];
        paths.extend(self.stylesheets.iter().map(|sheet| sheet.path.clone()));
        paths
    }
}

/// A watcher wakeup: paths that may be stale, or a rescan request
/// (`need_rescan` / channel overflow — events may have been missed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceNotification {
    /// The paths' contents may have changed (deduplicated by the
    /// coordinator; only registered paths matter).
    Changed(Vec<PathBuf>),
    /// Any tracked source may have changed: reread them all.
    Rescan,
}

/// One injected filesystem read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOutcome {
    /// The current bytes.
    Bytes(Vec<u8>),
    /// The path does not exist right now (replacement in flight).
    Missing,
    /// The read failed (transient; never memoized as content).
    Error(String),
}

/// The observed or published input bundle: exact bytes in registry
/// order. Byte equality *is* the fingerprint — the coordinator hashes
/// nothing it does not compare, and compares what it actually passes
/// to the reload APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleSnapshot {
    /// Document HTML bytes.
    pub html: Vec<u8>,
    /// Stylesheet bytes in cascade order.
    pub sheets: Vec<Vec<u8>>,
}

impl BundleSnapshot {
    fn same_as(&self, other: &BundleSnapshot) -> bool {
        self == other
    }
}

/// What one reconciliation did.
#[derive(Debug, Clone, PartialEq)]
pub enum ReconcileOutcome {
    /// Nothing was dirty.
    NothingDirty,
    /// The settled bytes equal the published bundle: no reload, no
    /// render (metadata-only saves land here).
    Unchanged,
    /// The settled bytes equal the last **rejected** attempt: skipped
    /// to avoid a repeated initialization/error storm (any registered
    /// input changing breaks the equality and unblocks it).
    RepeatRejected,
    /// A read was missing or failed: the old application keeps
    /// running; the paths stay eligible for a later reconciliation.
    Deferred,
    /// A reload was attempted and rejected; the application is
    /// unchanged (the ledger and inspector carry the diagnostics —
    /// no document repaint).
    Rejected(ReloadRejection),
    /// A reload published; the frame should be presented.
    Published {
        /// Which transaction ran.
        kind: ReloadKind,
        /// The new active generation (full reloads).
        generation: Option<u64>,
    },
}

/// One reconciliation's result (outcome + whether the host should
/// redraw the document).
#[derive(Debug, Clone, PartialEq)]
pub struct Reconciliation {
    /// What happened.
    pub outcome: ReconcileOutcome,
    /// Whether a successful publication changed the document (a
    /// watcher wakeup alone never forces a render).
    pub repaint: bool,
}

/// Coordinator status for the inspector surface (small; links to the
/// view's reload ledger rather than duplicating it).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WatcherStatus {
    /// Dirty registered paths awaiting the quiet interval.
    pub dirty: usize,
    /// Whether a rescan is pending.
    pub rescan_pending: bool,
    /// Reconciliations run.
    pub reconciliations: u64,
    /// Settled-but-unchanged snapshots skipped.
    pub unchanged_skips: u64,
    /// Repeat attempts of a rejected snapshot skipped.
    pub repeat_skips: u64,
    /// Read-miss deferrals (missing/failed reads).
    pub deferrals: u64,
    /// Rescans performed.
    pub rescans: u64,
}

/// The reconciliation coordinator. Host-owned; the renderer never
/// sees filesystem events, only explicit source objects.
pub struct Coordinator {
    registry: SourceRegistry,
    dirty: BTreeSet<PathBuf>,
    rescan_pending: bool,
    quiet_ms: u64,
    last_change_ms: Option<u64>,
    published: BundleSnapshot,
    last_attempted: Option<BundleSnapshot>,
    force_pending: bool,
    status: WatcherStatus,
    enabled: bool,
}

impl Coordinator {
    /// Seeds the coordinator with the published bundle (the bytes the
    /// application was loaded from) and the quiet interval.
    pub fn new(registry: SourceRegistry, published: BundleSnapshot, quiet_ms: u64) -> Self {
        Self {
            registry,
            dirty: BTreeSet::new(),
            rescan_pending: false,
            quiet_ms,
            last_change_ms: None,
            published,
            last_attempted: None,
            force_pending: false,
            status: WatcherStatus::default(),
            enabled: true,
        }
    }

    /// Disables all work (host shutting down or watch turned off).
    /// Late notifications are recorded by no-ops.
    // Test- and host-facing lifecycle API; the lab forgets its watcher
    // for the loop lifetime, so the non-test binary sees this as unused.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn shutdown(&mut self) {
        self.enabled = false;
        self.dirty.clear();
        self.rescan_pending = false;
    }

    /// Whether the coordinator is active.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The tracked registry (for watcher setup: parent directories).
    pub fn registry(&self) -> &SourceRegistry {
        &self.registry
    }

    /// Current status (the small inspector surface).
    pub fn status(&self) -> WatcherStatus {
        let mut status = self.status.clone();
        status.dirty = self.dirty.len();
        status.rescan_pending = self.rescan_pending;
        status
    }

    /// Records a wakeup: marks registered paths dirty and restarts the
    /// quiet interval. Never reloads. Unknown paths are ignored
    /// (bounded by the registered source set).
    pub fn notify(&mut self, notification: SourceNotification, now_ms: u64) {
        if !self.enabled {
            return;
        }
        match notification {
            SourceNotification::Changed(paths) => {
                for path in paths {
                    if self
                        .registry
                        .paths()
                        .iter()
                        .any(|registered| registered == &path)
                    {
                        self.dirty.insert(path);
                    }
                }
            }
            SourceNotification::Rescan => {
                self.rescan_pending = true;
                for path in self.registry.paths() {
                    self.dirty.insert(path);
                }
            }
        }
        if !self.dirty.is_empty() {
            self.last_change_ms = Some(now_ms);
        }
    }

    /// Forces the next reconciliation to reread everything and bypass
    /// unchanged-content suppression — a **manual** reload is a
    /// different operation from automatic "nothing changed".
    // No key binding wires this yet; the semantics are pinned by tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn request_manual_reload(&mut self, now_ms: u64) {
        if !self.enabled {
            return;
        }
        self.rescan_pending = true;
        self.last_attempted = None;
        self.force_pending = true;
        for path in self.registry.paths() {
            self.dirty.insert(path);
        }
        self.last_change_ms = Some(now_ms);
    }

    /// Whether a reconciliation is due (something dirty and the quiet
    /// interval has elapsed since the last change).
    pub fn due(&self, now_ms: u64) -> bool {
        if !self.enabled {
            return false;
        }
        if self.dirty.is_empty() {
            return false;
        }
        match self.last_change_ms {
            Some(last) => now_ms.saturating_sub(last) >= self.quiet_ms,
            None => true,
        }
    }

    /// Runs one reconciliation against an injected reader. The dirty
    /// set is **taken** (a snapshot): notifications arriving during the
    /// reconciliation populate the next set — the M6a.1 lesson, never a
    /// shared-clear-after-reload.
    pub fn reconcile<R>(
        &mut self,
        _now_ms: u64,
        mut read: R,
        view: &mut VelquView,
        viewport: Viewport,
    ) -> Reconciliation
    where
        R: FnMut(&Path) -> ReadOutcome,
    {
        if !self.enabled || self.dirty.is_empty() {
            return Reconciliation {
                outcome: ReconcileOutcome::NothingDirty,
                repaint: false,
            };
        }
        let dirty: BTreeSet<PathBuf> = std::mem::take(&mut self.dirty);
        let rescan = self.rescan_pending;
        self.rescan_pending = false;
        if rescan {
            self.status.rescans += 1;
        }
        self.status.reconciliations += 1;

        // Read the dirty sources (all of them on rescan); a missing or
        // failed read defers the whole bundle — the old application
        // keeps running and the paths stay eligible.
        let mut observed = self.published.clone();
        let mut deferred_paths: Vec<PathBuf> = Vec::new();
        let mut read_one = |path: &Path, deferred: &mut Vec<PathBuf>| match read(path) {
            ReadOutcome::Bytes(bytes) => Some(bytes),
            ReadOutcome::Missing => {
                deferred.push(path.to_owned());
                None
            }
            ReadOutcome::Error(_) => {
                deferred.push(path.to_owned());
                None
            }
        };
        if rescan || dirty.contains(&self.registry.document.path) {
            match read_one(&self.registry.document.path.clone(), &mut deferred_paths) {
                Some(bytes) => observed.html = bytes,
                None => {
                    self.defer(deferred_paths);
                    return Reconciliation {
                        outcome: ReconcileOutcome::Deferred,
                        repaint: false,
                    };
                }
            }
        }
        for sheet in self.registry.stylesheets.clone() {
            if rescan || dirty.contains(&sheet.path) {
                match read_one(&sheet.path, &mut deferred_paths) {
                    Some(bytes) => {
                        let index = self
                            .registry
                            .stylesheets
                            .iter()
                            .position(|candidate| candidate.path == sheet.path)
                            .expect("registry sheets are unique");
                        observed.sheets[index] = bytes;
                    }
                    None => {
                        self.defer(deferred_paths);
                        return Reconciliation {
                            outcome: ReconcileOutcome::Deferred,
                            repaint: false,
                        };
                    }
                }
            }
        }

        // Compare against what is live and what already failed.
        // A manual reload deliberately bypasses unchanged suppression —
        // automatic "nothing changed" and a user-requested restart are
        // different operations.
        let forced = self.force_pending;
        self.force_pending = false;
        if !forced && observed.same_as(&self.published) {
            self.status.unchanged_skips += 1;
            return Reconciliation {
                outcome: ReconcileOutcome::Unchanged,
                repaint: false,
            };
        }
        if self
            .last_attempted
            .as_ref()
            .is_some_and(|attempted| observed.same_as(attempted))
            && view.last_reload_attempt().is_some_and(|attempt| {
                !matches!(attempt.outcome, velqu_view::ReloadOutcome::Published { .. })
            })
        {
            self.status.repeat_skips += 1;
            return Reconciliation {
                outcome: ReconcileOutcome::RepeatRejected,
                repaint: false,
            };
        }

        // Route: any document change (alone or with CSS, or a forced
        // manual restart) is one full-bundle transaction — the bundle
        // carries **every** registered sheet, so a published candidate's
        // sheet set is the registry, in order. Stylesheet-only changes
        // are one stylesheet transaction with the established order.
        let document_changed = forced || observed.html != self.published.html;
        let attempt = if document_changed {
            let source = velqu_view::DocumentSource::new(
                self.registry.document.id.to_string(),
                String::from_utf8_lossy(&observed.html).into_owned(),
            );
            let sheets = self
                .registry
                .stylesheets
                .iter()
                .enumerate()
                .map(|(index, registered)| {
                    velqu_view::StylesheetSource::new(
                        registered.id.to_string(),
                        String::from_utf8_lossy(&observed.sheets[index]).into_owned(),
                    )
                })
                .collect::<Vec<_>>();
            view.reload_bundle(source, sheets, viewport)
                .map(|generation| (ReloadKind::FullDocument, Some(generation)))
        } else {
            let changed = self
                .registry
                .stylesheets
                .iter()
                .enumerate()
                .filter(|(index, _)| observed.sheets[*index] != self.published.sheets[*index])
                .map(|(index, registered)| {
                    velqu_view::StylesheetSource::new(
                        registered.id.to_string(),
                        String::from_utf8_lossy(&observed.sheets[index]).into_owned(),
                    )
                })
                .collect::<Vec<_>>();
            view.reload_stylesheets(changed, viewport)
                .map(|_| (ReloadKind::Stylesheets, None))
        };

        self.last_attempted = Some(observed.clone());
        match attempt {
            Ok((kind, generation)) => {
                self.published = observed;
                Reconciliation {
                    outcome: ReconcileOutcome::Published { kind, generation },
                    repaint: true,
                }
            }
            Err(rejection) => Reconciliation {
                outcome: ReconcileOutcome::Rejected(rejection),
                repaint: false,
            },
        }
    }

    /// A deferred reconciliation keeps its unreadable paths eligible
    /// for the next wakeup (bounded by the registered set; deferral is
    /// a cheap reread, never an error storm).
    fn defer(&mut self, paths: Vec<PathBuf>) {
        self.status.deferrals += 1;
        for path in paths {
            self.dirty.insert(path);
        }
    }
}

#[cfg(test)]
mod tests;
