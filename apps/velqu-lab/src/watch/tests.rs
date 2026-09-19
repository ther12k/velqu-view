//! Native-watcher integration tests: real `notify` backends, real
//! temporary directories, bounded waits for outcomes — never exact
//! counts or platform notification ordering. (Linux watcher evidence
//! is Linux evidence.) The coordinator semantics live in the synthetic
//! battery; these establish native delivery.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use velqu_view::{VelquView, Viewport};

use crate::reload_coordinator::{
    BundleSnapshot, Coordinator, ReadOutcome, ReconcileOutcome, RegisteredSource, SourceRegistry,
};
use crate::watch::{Wakeups, WatchBackend, WatcherHandle, spawn};

/// A unique temporary app directory (best-effort cleanup on drop).
struct TempApp {
    dir: PathBuf,
}

impl TempApp {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = u64::from(std::process::id()) * 1000 + COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("velqu-watch-{tag}-{unique}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        Self { dir }
    }

    fn write(&self, name: &str, bytes: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, bytes).expect("write");
        path
    }

    /// An editor-style replacement save: write a sibling temp file,
    /// then rename it over the original (the watcher stays attached
    /// to the directory entry, not the old inode).
    fn replace_save(&self, name: &str, bytes: &str) {
        let target = self.dir.join(name);
        let temp = self.dir.join(format!("{name}.velqu-tmp"));
        std::fs::write(&temp, bytes).expect("write temp");
        std::fs::rename(&temp, &target).expect("rename over original");
    }
}

impl Drop for TempApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const HTML: &str = "<!doctype html><html><body style=\"margin: 0\">\
     <div vx-state=\"{ count: 0 }\">\
     <p id=t data-vv-test=label vx-text=\"'Count: ' + count\">placeholder</p>\
     <button id=inc @click=\"count = count + 1\">+</button>\
     </div>\
     </body></html>";

fn loaded_view(app: &TempApp) -> VelquView {
    let mut view = VelquView::new();
    view.enable_reactive();
    view.load_html(HTML).unwrap();
    view.load_stylesheet(velqu_view::StylesheetSource::new(
        "a.css",
        std::fs::read_to_string(app.dir.join("a.css")).unwrap(),
    ))
    .unwrap();
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    view.render(vp).unwrap();
    view.pump_reactive_queued();
    view.render(vp).unwrap();
    view
}

fn registry(app: &TempApp) -> SourceRegistry {
    SourceRegistry {
        document: RegisteredSource {
            path: app.dir.join("index.html"),
            id: velqu_view::SourceId::new("index.html"),
        },
        stylesheets: vec![RegisteredSource {
            path: app.dir.join("a.css"),
            id: velqu_view::SourceId::new("a.css"),
        }],
    }
}

fn real_read(path: &Path) -> ReadOutcome {
    match std::fs::read(path) {
        Ok(bytes) => ReadOutcome::Bytes(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ReadOutcome::Missing,
        Err(_) => ReadOutcome::Error("read failed".to_owned()),
    }
}

/// Runs the host pump until the predicate accepts (bounded). Returns
/// the number of **publications** observed.
fn pump_until(
    app: &TempApp,
    view: &mut VelquView,
    coordinator: &mut Coordinator,
    wakeups: &Wakeups,
    quiet_ms: u64,
    mut satisfied: impl FnMut(&ReconcileOutcome) -> bool,
) -> Option<ReconcileOutcome> {
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    let start = Instant::now();
    let mut last: Option<ReconcileOutcome> = None;
    while start.elapsed() < Duration::from_secs(8) {
        let now_ms = start.elapsed().as_millis() as u64;
        wakeups.drain_into(coordinator, now_ms);
        if coordinator.due(now_ms) {
            let result = coordinator.reconcile(now_ms, real_read, view, vp);
            if satisfied(&result.outcome) {
                return Some(result.outcome);
            }
            if !matches!(result.outcome, ReconcileOutcome::NothingDirty) {
                last = Some(result.outcome);
            }
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    let _ = quiet_ms;
    let _ = app;
    last
}

fn harness(backend: WatchBackend) -> (TempApp, VelquView, Coordinator, Wakeups, WatcherHandle) {
    let (app, mut view, mut coordinator, wakeups, watcher) = raw_harness(backend);
    // Drain the watcher's forced post-registration rescan (startup
    // reconciliation) so the tests below start from a settled baseline.
    std::thread::sleep(Duration::from_millis(500));
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(4) {
        let now_ms = start.elapsed().as_millis() as u64;
        wakeups.drain_into(&mut coordinator, now_ms);
        if coordinator.due(now_ms) {
            let _ = coordinator.reconcile(now_ms, real_read, &mut view, vp);
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    (app, view, coordinator, wakeups, watcher)
}

/// The raw harness: files written, view loaded (published = disk),
/// watcher spawned, nothing drained yet.
fn raw_harness(backend: WatchBackend) -> (TempApp, VelquView, Coordinator, Wakeups, WatcherHandle) {
    let app = TempApp::new(match backend {
        WatchBackend::Native => "native",
        WatchBackend::Poll => "poll",
    });
    app.write("index.html", HTML);
    app.write("a.css", "#t { color: #ff0000 }");
    let view = loaded_view(&app);
    let published = BundleSnapshot {
        html: HTML.as_bytes().to_vec(),
        sheets: vec![b"#t { color: #ff0000 }".to_vec()],
    };
    let coordinator = Coordinator::new(registry(&app), published, 60);
    let (sender, wakeups) = Wakeups::channel();
    let watcher = spawn(backend, &registry(&app).paths(), sender).expect("watcher starts");
    (app, view, coordinator, wakeups, watcher)
}

/// The end-to-end native probe: an actual CSS replacement save (temp
/// file + rename) flows through the native watcher into the
/// coordinator and publishes through M6b — twice, with the source
/// identity and cascade position preserved both times.
#[test]
fn native_css_replacement_save_twice() {
    let (app, mut view, mut coordinator, wakeups, mut watcher) = harness(WatchBackend::Native);
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();

    // Replacement save #1.
    app.replace_save("a.css", "#t { color: #00ff00 }");
    let outcome = pump_until(&app, &mut view, &mut coordinator, &wakeups, 60, |outcome| {
        matches!(outcome, ReconcileOutcome::Published { .. })
    })
    .expect("first replacement published within the bound");
    assert!(matches!(
        outcome,
        ReconcileOutcome::Published {
            kind: velqu_view::ReloadKind::Stylesheets,
            ..
        }
    ));
    view.render(vp).unwrap();
    assert_eq!(
        view.stylesheets()
            .iter()
            .map(|s| s.id.to_string())
            .collect::<Vec<_>>(),
        ["a.css"],
        "source identity and position preserved"
    );

    // Replacement save #2 with different bytes — a watcher that stayed
    // attached to the obsolete inode would miss it.
    app.replace_save("a.css", "#t { color: #010101 }");
    let outcome = pump_until(&app, &mut view, &mut coordinator, &wakeups, 60, |outcome| {
        matches!(outcome, ReconcileOutcome::Published { .. })
    })
    .expect("second replacement published within the bound");
    let _ = outcome;
    view.render(vp).unwrap();
    // The final color proves both saves landed (a stale watcher would
    // still show the first or none).
    let (t_x, t_y) = point_over(&view, vp, "t");
    let handle = view.hit_test(vp, t_x, t_y).unwrap().handle;
    let color = view
        .inspector_snapshot(vp, Some(handle))
        .selected
        .unwrap()
        .color;
    assert_eq!(color, velqu_view::Color::from_hex("#010101").unwrap());
    watcher.shutdown();
}

/// The polling backend reaches the same coordinator with the same
/// semantics (no separate reload behavior).
#[test]
fn polling_backend_feeds_the_same_reconciliation() {
    let (app, mut view, mut coordinator, wakeups, mut watcher) = harness(WatchBackend::Poll);
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    std::fs::write(app.dir.join("a.css"), "#t { color: #00ff00 }").unwrap();
    let outcome = pump_until(&app, &mut view, &mut coordinator, &wakeups, 60, |outcome| {
        matches!(outcome, ReconcileOutcome::Published { .. })
    })
    .expect("poll watcher published within the bound");
    assert!(matches!(
        outcome,
        ReconcileOutcome::Published {
            kind: velqu_view::ReloadKind::Stylesheets,
            ..
        }
    ));
    view.render(vp).unwrap();
    let (t_x, t_y) = point_over(&view, vp, "t");
    let handle = view.hit_test(vp, t_x, t_y).unwrap().handle;
    let color = view
        .inspector_snapshot(vp, Some(handle))
        .selected
        .unwrap()
        .color;
    assert_eq!(color, velqu_view::Color::from_hex("#00ff00").unwrap());
    watcher.shutdown();
}

/// A document replacement save routes to the full-bundle transaction.
#[test]
fn native_document_save_routes_to_a_full_bundle() {
    let (app, mut view, mut coordinator, wakeups, mut watcher) = harness(WatchBackend::Native);
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    let new_html = "<!doctype html><html><body style=\"margin: 0\">\
         <div vx-state=\"{ count: 100 }\">\
         <p id=t data-vv-test=label vx-text=\"'Count: ' + count\">placeholder</p>\
         <button id=inc @click=\"count = count + 1\">+</button>\
         </div>\
         </body></html>";
    app.replace_save("index.html", new_html);
    let outcome = pump_until(&app, &mut view, &mut coordinator, &wakeups, 60, |outcome| {
        matches!(outcome, ReconcileOutcome::Published { .. })
    })
    .expect("document replacement published within the bound");
    assert!(matches!(
        outcome,
        ReconcileOutcome::Published {
            kind: velqu_view::ReloadKind::FullDocument,
            ..
        }
    ));
    view.render(vp).unwrap();
    assert_eq!(
        view.reactive_state().map(|s| s.get_path("count")),
        Some(velqu_reactive::ReactiveValue::Number(100.0))
    );
    watcher.shutdown();
}

/// The startup registration gap, as the real sequence runs: the app
/// reads A, the file becomes B **before** the watcher registers (no
/// event will ever come), and the watcher's forced post-registration
/// rescan displays B without another save. Pinned under the native
/// backend; the coordinator-level sequence is the synthetic twin.
#[test]
fn native_startup_registration_gap_closes_without_a_save() {
    let (app, mut view, mut coordinator, wakeups, mut watcher) = raw_harness(WatchBackend::Native);
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    // Pre-registration edit: the on-disk content is B, the published
    // snapshot is A, and no notification exists or will.
    std::fs::write(app.dir.join("a.css"), "#t { color: #00ff00 }").unwrap();
    let start = Instant::now();
    let published = loop {
        let now_ms = start.elapsed().as_millis() as u64;
        wakeups.drain_into(&mut coordinator, now_ms);
        if coordinator.due(now_ms) {
            if let ReconcileOutcome::Published { .. } = coordinator
                .reconcile(now_ms, real_read, &mut view, vp)
                .outcome
            {
                break true;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "no startup reconciliation ran"
        );
        std::thread::sleep(Duration::from_millis(15));
    };
    assert!(published, "the post-registration rescan published B");
    view.render(vp).unwrap();
    let (t_x, t_y) = point_over(&view, vp, "t");
    let handle = view.hit_test(vp, t_x, t_y).unwrap().handle;
    let color = view
        .inspector_snapshot(vp, Some(handle))
        .selected
        .unwrap()
        .color;
    assert_eq!(color, velqu_view::Color::from_hex("#00ff00").unwrap());
    watcher.shutdown();
}

/// A timestamp-preserving edit under the poll backend: notify truncates
/// mtimes to whole seconds and compares "newer or content-different",
/// so an in-place edit inside the same recorded second is invisible to
/// timestamp-only polling. With content comparison enabled the poll
/// route still converges to the new bytes. (Waits are for the
/// observable outcome; `PollWatcher::poll()` is a request, not a
/// completion barrier.)
#[test]
fn polling_detects_same_second_edits_via_content_comparison() {
    let (app, mut view, mut coordinator, wakeups, mut watcher) = raw_harness(WatchBackend::Poll);
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    // Align into a fresh wall-second, then write the baseline and the
    // edit inside that one second: the file's whole-second mtime never
    // advances past the baseline's.
    wait_for_fresh_second();
    let css = app.dir.join("a.css");
    std::fs::write(&css, "#t { color: #ff0000 }").unwrap();
    let before_mtime = whole_seconds(&css);
    std::fs::write(&css, "#t { color: #010101 }").unwrap();
    let after_mtime = whole_seconds(&css);
    assert_eq!(
        before_mtime, after_mtime,
        "the edit preserved the recorded whole-second timestamp"
    );
    let start = Instant::now();
    let published = loop {
        let now_ms = start.elapsed().as_millis() as u64;
        wakeups.drain_into(&mut coordinator, now_ms);
        if coordinator.due(now_ms) {
            if let ReconcileOutcome::Published { .. } = coordinator
                .reconcile(now_ms, real_read, &mut view, vp)
                .outcome
            {
                break true;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "the content-comparing poller never converged"
        );
        std::thread::sleep(Duration::from_millis(15));
    };
    assert!(published);
    view.render(vp).unwrap();
    let (t_x, t_y) = point_over(&view, vp, "t");
    let handle = view.hit_test(vp, t_x, t_y).unwrap().handle;
    let color = view
        .inspector_snapshot(vp, Some(handle))
        .selected
        .unwrap()
        .color;
    assert_eq!(color, velqu_view::Color::from_hex("#010101").unwrap());
    watcher.shutdown();
}

/// Waits until a new wall-clock second has just begun (the test then
/// has ~1s of headroom for its two writes).
fn wait_for_fresh_second() {
    let entry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let into_second = entry.subsec_millis();
    if into_second < 900 {
        std::thread::sleep(Duration::from_millis((1000 - into_second + 20).into()));
    }
}

/// The file's whole-second mtime (notify's comparison granularity).
fn whole_seconds(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .map(|mtime| {
            mtime
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        })
        .unwrap_or(0)
}

/// Shutdown stops the work: after `shutdown`, a save produces no
/// reconciliation at all within the bound.
#[test]
fn shutdown_inert() {
    let (app, mut view, mut coordinator, wakeups, mut watcher) = harness(WatchBackend::Native);
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    watcher.shutdown();
    coordinator.shutdown();
    app.replace_save("a.css", "#t { color: #010101 }");
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        let now_ms = start.elapsed().as_millis() as u64;
        wakeups.drain_into(&mut coordinator, now_ms);
        if coordinator.due(now_ms) {
            let result = coordinator.reconcile(now_ms, real_read, &mut view, vp);
            assert_eq!(result.outcome, ReconcileOutcome::NothingDirty);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let (t_x, t_y) = point_over(&view, vp, "t");
    let handle = view.hit_test(vp, t_x, t_y).unwrap().handle;
    let color = view
        .inspector_snapshot(vp, Some(handle))
        .selected
        .unwrap()
        .color;
    assert_eq!(color, velqu_view::Color::from_hex("#ff0000").unwrap());
}

fn point_over(view: &VelquView, vp: Viewport, id: &str) -> (f32, f32) {
    for y in (0..vp.height()).step_by(4) {
        for x in (0..vp.width()).step_by(8) {
            if view
                .hit_test(vp, x as f32 + 0.5, y as f32 + 0.5)
                .is_some_and(|target| target.element_id.as_deref() == Some(id))
            {
                return (x as f32 + 0.5, y as f32 + 0.5);
            }
        }
    }
    panic!("no hit target for {id}");
}
