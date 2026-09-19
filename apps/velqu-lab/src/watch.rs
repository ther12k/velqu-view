//! The watcher backend (M6c, ADR 0022): notify wrapped in the
//! host-side boundary. It **never reloads** — it watches the parent
//! directories of the registered sources (editors save by truncation
//! or by temp-file replacement; watching the file itself is the
//! documented footgun), coalesces raw events into bounded
//! [`SourceNotification`]s, and wakes the host through a channel. A
//! `need_rescan` (or channel overflow — the same meaning) becomes one
//! [`SourceNotification::Rescan`]: reread the tracked set, never
//! reconstruct the missing event sequence.
//!
//! Native watching is an optimization over polling, not a guarantee:
//! notify documents silent-event-loss environments (network
//! filesystems, some containers). [`WatchBackend::Poll`] exists for
//! exactly those, and both backends feed the same coordinator —
//! polling has no separate reload semantics.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use notify::event::{Event as NotifyEvent, EventKind};

use crate::reload_coordinator::SourceNotification;

/// Which backend to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchBackend {
    /// The platform's native watcher (inotify/FSEvents/ReadDirectoryChangesW).
    Native,
    /// Polling fallback for filesystems without reliable native events.
    Poll,
}

/// A started watcher. Dropping the handle stops the thread; `shutdown`
/// stops it explicitly and makes any already-queued wakeups inert
/// (late callbacks cannot trigger reloads after shutdown).
pub struct WatcherHandle {
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WatcherHandle {
    /// Stops the watcher; later wakeups arrive as `Err(Disconnected)`
    /// and must be ignored by the host.
    // The lab forgets its watcher for the loop lifetime; tests exercise
    // explicit shutdown.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn shutdown(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Watches the parent directories of `sources` and sends coalesced
/// notifications over `wakeups`. Raw events drain into one
/// notification per wakeup batch; the registered set bounds what is
/// ever reported. (The no-wake form; the lab uses `spawn_with_wake`,
/// tests use this.)
#[cfg_attr(not(test), allow(dead_code))]
pub fn spawn(
    backend: WatchBackend,
    sources: &[PathBuf],
    wakeups: Sender<SourceNotification>,
) -> Result<WatcherHandle, String> {
    spawn_with_wake(backend, sources, wakeups, Arc::new(|| {}))
}

/// [`spawn`] plus a wake callback invoked after every notification
/// send (the host forwards it to its event loop's proxy).
pub fn spawn_with_wake(
    backend: WatchBackend,
    sources: &[PathBuf],
    wakeups: Sender<SourceNotification>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<WatcherHandle, String> {
    let dirs: Vec<PathBuf> = parent_dirs(sources);
    let (events_tx, events_rx) = channel::<notify::Result<NotifyEvent>>();
    let mut notify_watcher: Box<dyn notify::Watcher + Send> = match backend {
        WatchBackend::Native => {
            let watcher = notify::recommended_watcher(move |event| {
                // The sender outlives the watcher thread only; a send
                // failure means shutdown — drop the event.
                let _ = events_tx.send(event);
            })
            .map_err(|error| format!("native watcher: {error}"))?;
            Box::new(watcher)
        }
        WatchBackend::Poll => {
            let watcher = notify::PollWatcher::new(
                move |event| {
                    let _ = events_tx.send(event);
                },
                notify::Config::default().with_poll_interval(Duration::from_millis(250)),
            )
            .map_err(|error| format!("poll watcher: {error}"))?;
            Box::new(watcher)
        }
    };

    let stopped = Arc::new(AtomicBool::new(false));
    let thread_stopped = Arc::clone(&stopped);
    let thread = std::thread::Builder::new()
        .spawn(move || {
            // Watch the directories recursively: sources may sit in
            // nested app folders.
            for dir in dirs {
                let _ = notify_watcher.watch(&dir, notify::RecursiveMode::Recursive);
            }
            while !thread_stopped.load(Ordering::SeqCst) {
                match events_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(event) => {
                        // Coalesce: drain everything already queued,
                        // then send one notification.
                        let mut paths = Vec::new();
                        let mut rescan = match event {
                            Ok(event) => collect(&event, &mut paths),
                            Err(_) => false,
                        };
                        while let Ok(more) = events_rx.try_recv() {
                            match more {
                                Ok(event) => rescan |= collect(&event, &mut paths),
                                Err(_) => {
                                    rescan = true;
                                }
                            }
                        }
                        let notification = if rescan {
                            SourceNotification::Rescan
                        } else {
                            SourceNotification::Changed(dedup(paths))
                        };
                        if wakeups.send(notification).is_err() {
                            break; // host gone
                        }
                        wake();
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .map_err(|error| format!("watch thread: {error}"))?;
    Ok(WatcherHandle {
        stopped,
        thread: Some(thread),
    })
}

/// The unique parent directories of the tracked sources.
fn parent_dirs(sources: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs: HashSet<PathBuf> = HashSet::new();
    for source in sources {
        if let Some(parent) = source.parent() {
            if parent == Path::new("") {
                dirs.insert(PathBuf::from("."));
            } else {
                dirs.insert(parent.to_owned());
            }
        }
    }
    dirs.into_iter().collect()
}

/// Collects one notify event's relevant paths; returns whether a
/// rescan was requested (`need_rescan`: events may have been missed).
fn collect(event: &NotifyEvent, paths: &mut Vec<PathBuf>) -> bool {
    if event.need_rescan() {
        return true;
    }
    // Content-bearing changes only; access/metadata noise is dropped
    // (a metadata-only save becomes an Unchanged skip at worst).
    match event.kind {
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {
            paths.extend(event.paths.iter().cloned());
            false
        }
        _ => false,
    }
}

fn dedup(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// The host side of the wakeup channel: non-blocking receive of
/// pending notifications (bounded drains; raw events never queue
/// here).
pub struct Wakeups {
    receiver: Receiver<SourceNotification>,
}

impl Wakeups {
    /// Creates the channel the watcher reports into.
    pub fn channel() -> (Sender<SourceNotification>, Self) {
        let (sender, receiver) = channel();
        (sender, Self { receiver })
    }

    /// Drains pending notifications into the coordinator (the wake
    /// itself; the quiet interval does the debouncing).
    pub fn drain_into(
        &self,
        coordinator: &mut crate::reload_coordinator::Coordinator,
        now_ms: u64,
    ) {
        while let Ok(notification) = self.receiver.try_recv() {
            coordinator.notify(notification, now_ms);
        }
    }
}

#[cfg(test)]
mod tests;
