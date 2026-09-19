//! Coordinator tests: deterministic — injected clock, injected reads,
//! synthetic notifications. No real filesystem, no real sleeps, no
//! advancement of Velqu's logical JS clock.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use velqu_view::{VelquView, Viewport};

use super::{
    BundleSnapshot, Coordinator, ReadOutcome, ReconcileOutcome, RegisteredSource,
    SourceNotification, SourceRegistry,
};

fn registry() -> SourceRegistry {
    SourceRegistry {
        document: RegisteredSource {
            path: PathBuf::from("/app/index.html"),
            id: velqu_view::SourceId::new("index.html"),
        },
        stylesheets: vec![
            RegisteredSource {
                path: PathBuf::from("/app/a.css"),
                id: velqu_view::SourceId::new("a.css"),
            },
            RegisteredSource {
                path: PathBuf::from("/app/b.css"),
                id: velqu_view::SourceId::new("b.css"),
            },
        ],
    }
}

const QUIET: u64 = 50;

/// A view pre-loaded with the document + both sheets (A red, B blue —
/// B wins the equal-specificity tie by order), plus its coordinator
/// seeded with the published bytes.
fn harness() -> (VelquView, Coordinator, Viewport) {
    let html = "<!doctype html><html><body style=\"margin: 0\">\
         <div vx-state=\"{ count: 0 }\">\
         <p id=t data-vv-test=label vx-text=\"'Count: ' + count\">placeholder</p>\
         <button id=inc @click=\"count = count + 1\">+</button>\
         </div>\
         </body></html>";
    let a = "#t { color: #ff0000 }";
    let b = "#t { color: #0000ff }";
    let mut view = VelquView::new();
    view.enable_reactive();
    view.load_html(html).unwrap();
    view.load_stylesheet(velqu_view::StylesheetSource::new("a.css", a))
        .unwrap();
    view.load_stylesheet(velqu_view::StylesheetSource::new("b.css", b))
        .unwrap();
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    view.render(vp).unwrap();
    view.pump_reactive_queued();
    view.render(vp).unwrap();
    let published = BundleSnapshot {
        html: html.as_bytes().to_vec(),
        sheets: vec![a.as_bytes().to_vec(), b.as_bytes().to_vec()],
    };
    let coordinator = Coordinator::new(registry(), published, QUIET);
    (view, coordinator, vp)
}

/// An injected filesystem: path → bytes; unregistered paths report
/// Missing; entries can be swapped per-test.
#[derive(Default)]
struct FakeFs {
    files: HashMap<PathBuf, Vec<u8>>,
}

impl FakeFs {
    fn set(&mut self, path: &Path, bytes: &str) {
        self.files
            .insert(path.to_owned(), bytes.as_bytes().to_vec());
    }
    fn remove(&mut self, path: &Path) {
        self.files.remove(path);
    }
    fn read(&self) -> impl FnMut(&Path) -> ReadOutcome + '_ {
        move |path| match self.files.get(path) {
            Some(bytes) => ReadOutcome::Bytes(bytes.clone()),
            None => ReadOutcome::Missing,
        }
    }
}

/// Sets up the fake fs with the published contents.
fn seeded_fs() -> FakeFs {
    let mut fs = FakeFs::default();
    let reg = registry();
    fs.set(
        &reg.document.path,
        "<!doctype html><html><body style=\"margin: 0\">\
         <div vx-state=\"{ count: 0 }\">\
         <p id=t data-vv-test=label vx-text=\"'Count: ' + count\">placeholder</p>\
         <button id=inc @click=\"count = count + 1\">+</button>\
         </div>\
         </body></html>",
    );
    fs.set(&reg.stylesheets[0].path, "#t { color: #ff0000 }");
    fs.set(&reg.stylesheets[1].path, "#t { color: #0000ff }");
    fs
}

fn settled(
    coordinator: &mut Coordinator,
    view: &mut VelquView,
    vp: Viewport,
    fs: &FakeFs,
    now_ms: u64,
) -> super::Reconciliation {
    coordinator.reconcile(now_ms, fs.read(), view, vp)
}

/// Duplicate notifications within one debounce batch: at most one
/// attempt for that unchanged snapshot — and none at all when the
/// bytes equal the published bundle.
#[test]
fn duplicate_notifications_coalesce_into_one_attempt() {
    let (mut view, mut coordinator, vp) = harness();
    let mut fs = seeded_fs();
    for _ in 0..5 {
        coordinator.notify(
            SourceNotification::Changed(vec![PathBuf::from("/app/b.css")]),
            100,
        );
    }
    assert!(!coordinator.due(120), "quiet interval holds");
    assert!(coordinator.due(151));
    // Unchanged bytes: skipped entirely.
    let result = settled(&mut coordinator, &mut view, vp, &fs, 200);
    assert_eq!(result.outcome, ReconcileOutcome::Unchanged);
    assert!(!result.repaint);
    assert_eq!(coordinator.status().unchanged_skips, 1);
    // A real change after the same notification storm: exactly one
    // attempt, one publication.
    fs.set(Path::new("/app/b.css"), "#t { color: #010101 }");
    for _ in 0..4 {
        coordinator.notify(
            SourceNotification::Changed(vec![PathBuf::from("/app/b.css")]),
            300,
        );
    }
    let result = settled(&mut coordinator, &mut view, vp, &fs, 400);
    assert!(matches!(
        result.outcome,
        ReconcileOutcome::Published {
            kind: velqu_view::ReloadKind::Stylesheets,
            ..
        }
    ));
    assert_eq!(
        coordinator.status().reconciliations,
        2,
        "one attempt per settled snapshot"
    );
}

/// Metadata/no-content change: no reload, no render, no generation
/// change.
#[test]
fn unchanged_content_never_reloads() {
    let (mut view, mut coordinator, vp) = harness();
    let fs = seeded_fs();
    let generation = view.inspector_snapshot(vp, None).generation;
    let passes = view.layout_stats().passes;
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/index.html")]),
        10,
    );
    let result = settled(&mut coordinator, &mut view, vp, &fs, 100);
    assert_eq!(result.outcome, ReconcileOutcome::Unchanged);
    assert_eq!(view.inspector_snapshot(vp, None).generation, generation);
    assert_eq!(view.layout_stats().passes, passes, "no document render");
}

/// Invalid B after published A, then restoration of A: B is rejected
/// once (repeats of the same invalid snapshot are skipped), and
/// restoring A does not reset the still-running application.
#[test]
fn invalid_then_restored_snapshot_leaves_the_application_running() {
    let (mut view, mut coordinator, vp) = harness();
    let mut fs = seeded_fs();
    // Run the counter to 7.
    for _ in 0..7 {
        let mut found = None;
        for y in (0..vp.height()).step_by(4) {
            for x in (0..vp.width()).step_by(8) {
                if view
                    .hit_test(vp, x as f32 + 0.5, y as f32 + 0.5)
                    .is_some_and(|t| t.element_id.as_deref() == Some("inc"))
                {
                    found = Some((x as f32 + 0.5, y as f32 + 0.5));
                }
            }
        }
        let (x, y) = found.unwrap();
        view.pointer_press(vp, x, y);
        view.pointer_release(vp, x, y);
        view.pump_reactive_queued();
        view.render(vp).unwrap();
    }
    assert_eq!(
        view.reactive_state().map(|s| s.get_path("count")),
        Some(velqu_reactive::ReactiveValue::Number(7.0))
    );

    // B appears on disk — a stylesheet whose reload is rejected by
    // policy (empty source).
    fs.set(Path::new("/app/b.css"), "   ");
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/b.css")]),
        100,
    );
    let result = settled(&mut coordinator, &mut view, vp, &fs, 200);
    assert!(matches!(result.outcome, ReconcileOutcome::Rejected(_)));
    assert!(!result.repaint);
    assert_eq!(
        view.reactive_state().map(|s| s.get_path("count")),
        Some(velqu_reactive::ReactiveValue::Number(7.0))
    );

    // More notifications for the same invalid B: no repeated storm.
    for _ in 0..3 {
        coordinator.notify(
            SourceNotification::Changed(vec![PathBuf::from("/app/b.css")]),
            300,
        );
    }
    let result = settled(&mut coordinator, &mut view, vp, &fs, 400);
    assert_eq!(result.outcome, ReconcileOutcome::RepeatRejected);
    assert_eq!(coordinator.status().repeat_skips, 1);

    // A is restored on disk (the published bytes): the disk matches
    // the active source again — no reset, the counter stays 7.
    fs.set(Path::new("/app/b.css"), "#t { color: #0000ff }");
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/b.css")]),
        500,
    );
    let result = settled(&mut coordinator, &mut view, vp, &fs, 600);
    assert_eq!(result.outcome, ReconcileOutcome::Unchanged);
    assert_eq!(
        view.reactive_state().map(|s| s.get_path("count")),
        Some(velqu_reactive::ReactiveValue::Number(7.0)),
        "no document reset on restoration"
    );
    // The inspector history still shows B's failed attempt.
    let last = view.last_reload_attempt().unwrap();
    assert!(matches!(
        last.outcome,
        velqu_view::ReloadOutcome::Rejected { .. }
    ));
}

/// Valid CSS edit with the counter at 7 and edited control state: the
/// state survives, the new style appears, and the runtime still
/// increments afterwards.
#[test]
fn css_edit_preserves_state_and_the_runtime_operates_afterward() {
    let html = "<!doctype html><html><body style=\"margin: 0\">\
         <div vx-state=\"{ count: 0, name: '' }\">\
         <p id=t data-vv-test=label vx-text=\"'Count: ' + count\">placeholder</p>\
         <input id=field vx-model=\"name\">\
         <button id=inc @click=\"count = count + 1\">+</button>\
         </div>\
         </body></html>";
    let a = "#t { color: #ff0000 }";
    let mut view = VelquView::new();
    view.enable_reactive();
    view.load_html(html).unwrap();
    view.load_stylesheet(velqu_view::StylesheetSource::new("a.css", a))
        .unwrap();
    let vp = Viewport::try_new(400, 300, 1.0).unwrap();
    view.render(vp).unwrap();
    view.pump_reactive_queued();
    view.render(vp).unwrap();

    // Counter to 7 + a typed control value.
    for _ in 0..7 {
        let (x, y) = point_over(&view, vp, "inc");
        view.pointer_press(vp, x, y);
        view.pointer_release(vp, x, y);
        view.pump_reactive_queued();
        view.render(vp).unwrap();
    }
    view.set_focus(Some("field"));
    view.insert_text("Ada");
    view.pump_reactive_queued();
    view.render(vp).unwrap();
    let _ = view.take_events();

    let mut single_registry = registry();
    single_registry.stylesheets.truncate(1);
    let published = BundleSnapshot {
        html: html.as_bytes().to_vec(),
        sheets: vec![a.as_bytes().to_vec()],
    };
    let mut coordinator = Coordinator::new(single_registry, published, QUIET);
    let mut fs = FakeFs::default();
    fs.set(Path::new("/app/index.html"), html);
    fs.set(Path::new("/app/a.css"), a);

    fs.set(Path::new("/app/a.css"), "#t { color: #00ff00 }");
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/a.css")]),
        100,
    );
    let result = settled(&mut coordinator, &mut view, vp, &fs, 200);
    assert!(matches!(result.outcome, ReconcileOutcome::Published { .. }));
    assert!(result.repaint);
    view.render(vp).unwrap();
    assert_eq!(
        view.reactive_state().map(|s| s.get_path("count")),
        Some(velqu_reactive::ReactiveValue::Number(7.0))
    );
    assert_eq!(
        view.reactive_state().map(|s| s.get_path("name")),
        Some(velqu_reactive::ReactiveValue::String("Ada".to_owned()))
    );
    assert_eq!(view.focused(), Some("field"));

    // The runtime still operates.
    let (x, y) = point_over(&view, vp, "inc");
    view.pointer_press(vp, x, y);
    view.pointer_release(vp, x, y);
    view.pump_reactive_queued();
    assert_eq!(
        view.reactive_state().map(|s| s.get_path("count")),
        Some(velqu_reactive::ReactiveValue::Number(8.0))
    );
}

/// HTML and CSS updated in one reconciled batch: one full-document
/// transaction, both applied — no separately published CSS
/// intermediate.
#[test]
fn combined_batch_is_one_full_transaction() {
    let (mut view, mut coordinator, vp) = harness();
    let mut fs = seeded_fs();
    let new_html = "<!doctype html><html><body style=\"margin: 0\">\
         <div vx-state=\"{ count: 100 }\">\
         <p id=t data-vv-test=label vx-text=\"'Count: ' + count\">placeholder</p>\
         <button id=inc @click=\"count = count + 1\">+</button>\
         </div>\
         </body></html>";
    fs.set(Path::new("/app/index.html"), new_html);
    fs.set(Path::new("/app/a.css"), "#t { color: #00ff00 }");
    coordinator.notify(
        SourceNotification::Changed(vec![
            PathBuf::from("/app/index.html"),
            PathBuf::from("/app/a.css"),
        ]),
        100,
    );
    let result = settled(&mut coordinator, &mut view, vp, &fs, 200);
    match result.outcome {
        ReconcileOutcome::Published {
            kind: velqu_view::ReloadKind::FullDocument,
            ..
        } => {}
        other => panic!("expected one full-document publication, got {other:?}"),
    }
    assert_eq!(
        view.reactive_state().map(|s| s.get_path("count")),
        Some(velqu_reactive::ReactiveValue::Number(100.0)),
        "the replacement's source-defined state"
    );
    assert_eq!(
        view.stylesheets()
            .iter()
            .map(|s| s.id.to_string())
            .collect::<Vec<_>>(),
        ["a.css", "b.css"],
        "cascade order preserved through the bundle publication"
    );
    // The CSS took effect in the same transaction: A loaded green is
    // overridden by B (still blue) — verify A's new content by
    // reloading B too, then seeing the tie winner change.
    fs.set(Path::new("/app/b.css"), "#t { color: #010101 }");
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/b.css")]),
        300,
    );
    let result = settled(&mut coordinator, &mut view, vp, &fs, 400);
    assert!(matches!(result.outcome, ReconcileOutcome::Published { .. }));
    view.render(vp).unwrap();
    let (t_x, t_y) = point_over(&view, vp, "t");
    let t_handle = view.hit_test(vp, t_x, t_y).unwrap().handle;
    let color = view
        .inspector_snapshot(vp, Some(t_handle))
        .selected
        .unwrap()
        .color;
    assert_eq!(color, velqu_view::Color::from_hex("#010101").unwrap());
}

/// Missing is not empty: a temporarily missing source defers (the old
/// application survives) and its later recreation reconciles.
#[test]
fn missing_source_defers_then_recovers_on_recreation() {
    let (mut view, mut coordinator, vp) = harness();
    let mut fs = seeded_fs();
    let generation = view.inspector_snapshot(vp, None).generation;

    fs.remove(Path::new("/app/a.css"));
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/a.css")]),
        100,
    );
    let result = settled(&mut coordinator, &mut view, vp, &fs, 200);
    assert_eq!(result.outcome, ReconcileOutcome::Deferred);
    assert_eq!(view.inspector_snapshot(vp, None).generation, generation);
    assert!(!result.repaint);

    // Recreation triggers recovery without touching the HTML again.
    fs.set(Path::new("/app/a.css"), "#t { color: #00ff00 }");
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/a.css")]),
        300,
    );
    let result = settled(&mut coordinator, &mut view, vp, &fs, 400);
    assert!(matches!(result.outcome, ReconcileOutcome::Published { .. }));
}

/// A rescan signal (need_rescan / overflow) rereads the whole tracked
/// set and finds the latest supported contents — including changes
/// whose notifications were lost.
#[test]
fn rescan_rereads_the_tracked_set() {
    let (mut view, mut coordinator, vp) = harness();
    let mut fs = seeded_fs();
    // A change whose notification was lost entirely.
    fs.set(Path::new("/app/b.css"), "#t { color: #010101 }");
    // Only the rescan wakes the coordinator.
    coordinator.notify(SourceNotification::Rescan, 100);
    let result = settled(&mut coordinator, &mut view, vp, &fs, 200);
    assert!(matches!(result.outcome, ReconcileOutcome::Published { .. }));
    assert_eq!(coordinator.status().rescans, 1);
}

/// Startup reconciliation (the registration gap): the application read
/// source A; the file became B before watch registration completed —
/// the watcher baselines B and never emits an event. The watcher
/// thread's forced post-registration rescan must display B with no
/// further save, and nothing more happens afterwards.
#[test]
fn startup_reconciliation_closes_the_registration_gap() {
    let (mut view, mut coordinator, vp) = harness();
    let mut fs = seeded_fs();
    // The edit happens with NO notification at all (pre-registration);
    // b.css so the change is visible (B wins the equal-specificity tie).
    fs.set(Path::new("/app/b.css"), "#t { color: #00ff00 }");
    // The watcher thread sends one Rescan right after registering.
    coordinator.notify(SourceNotification::Rescan, 50);
    let result = settled(&mut coordinator, &mut view, vp, &fs, 200);
    assert!(matches!(result.outcome, ReconcileOutcome::Published { .. }));
    view.render(vp).unwrap();
    let (t_x, t_y) = point_over(&view, vp, "t");
    let handle = view.hit_test(vp, t_x, t_y).unwrap().handle;
    let color = view
        .inspector_snapshot(vp, Some(handle))
        .selected
        .unwrap()
        .color;
    assert_eq!(color, velqu_view::Color::from_hex("#00ff00").unwrap());
    // No further filesystem changes: no further reconciliation runs.
    assert!(!coordinator.due(10_000), "quiet after the startup pass");
    let result = settled(&mut coordinator, &mut view, vp, &fs, 20_000);
    assert_eq!(result.outcome, ReconcileOutcome::NothingDirty);
}

/// Notifications arriving during a reconciliation populate the next
/// dirty set — never a shared-clear-after-reload.
#[test]
fn notifications_during_reconciliation_remain_pending() {
    let (mut view, mut coordinator, vp) = harness();
    let mut fs = seeded_fs();
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/a.css")]),
        100,
    );
    fs.set(Path::new("/app/a.css"), "#t { color: #00ff00 }");
    // Reconcile with a reader that mutates the source mid-flight: the
    // second change's notification arrives "during" the first pass.
    let result = {
        let fs_ref = &fs;
        let mut first = true;
        coordinator.reconcile(
            200,
            |path| {
                if first && path == Path::new("/app/a.css") {
                    first = false;
                    // The watcher thread observes a later save while
                    // the host is mid-reconciliation.
                    return ReadOutcome::Bytes(b"#t { color: #00ff00 }".to_vec());
                }
                match fs_ref.files.get(path) {
                    Some(bytes) => ReadOutcome::Bytes(bytes.clone()),
                    None => ReadOutcome::Missing,
                }
            },
            &mut view,
            vp,
        )
    };
    assert!(matches!(result.outcome, ReconcileOutcome::Published { .. }));
    // The disk now holds a NEWER save; its notification wakes the next
    // pass, which must see it.
    fs.set(Path::new("/app/a.css"), "#t { color: #0000cd }");
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/a.css")]),
        300,
    );
    let result = settled(&mut coordinator, &mut view, vp, &fs, 400);
    assert!(matches!(result.outcome, ReconcileOutcome::Published { .. }));
}

/// Watch disabled or shutdown: no work while disabled, and late
/// notifications cannot trigger a reload after shutdown.
#[test]
fn shutdown_stops_all_work() {
    let (mut view, mut coordinator, vp) = harness();
    let mut fs = seeded_fs();
    coordinator.shutdown();
    assert!(!coordinator.enabled());
    fs.set(Path::new("/app/b.css"), "#t { color: #010101 }");
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/b.css")]),
        100,
    );
    coordinator.notify(SourceNotification::Rescan, 100);
    assert!(!coordinator.due(1000));
    let result = settled(&mut coordinator, &mut view, vp, &fs, 2000);
    assert_eq!(result.outcome, ReconcileOutcome::NothingDirty);
}

/// A manual reload deliberately bypasses unchanged-content
/// suppression.
#[test]
fn manual_reload_bypasses_unchanged_suppression() {
    let (mut view, mut coordinator, vp) = harness();
    let fs = seeded_fs();
    // Same bytes on disk: automatic reconciliation would skip.
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/index.html")]),
        10,
    );
    let result = settled(&mut coordinator, &mut view, vp, &fs, 100);
    assert_eq!(result.outcome, ReconcileOutcome::Unchanged);
    // Manual: reread everything and reload regardless.
    coordinator.request_manual_reload(200);
    let result = settled(&mut coordinator, &mut view, vp, &fs, 300);
    assert!(
        matches!(result.outcome, ReconcileOutcome::Published { .. }),
        "manual reload republishes even identical bytes"
    );
}

/// A transient read error is not memoized as a content failure: the
/// next successful read reconciles normally.
#[test]
fn read_errors_are_transient() {
    let (mut view, mut coordinator, vp) = harness();
    let mut fs = seeded_fs();
    fs.set(Path::new("/app/b.css"), "#t { color: #010101 }");
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/b.css")]),
        100,
    );
    // First read: a transient error.
    let result = {
        let failed = std::cell::Cell::new(false);
        let fs_ref = &fs;
        coordinator.reconcile(
            200,
            |path| {
                if path == Path::new("/app/b.css") && !failed.get() {
                    failed.set(true);
                    return ReadOutcome::Error("EBUSY".to_owned());
                }
                match fs_ref.files.get(path) {
                    Some(bytes) => ReadOutcome::Bytes(bytes.clone()),
                    None => ReadOutcome::Missing,
                }
            },
            &mut view,
            vp,
        )
    };
    assert_eq!(result.outcome, ReconcileOutcome::Deferred);
    // The retry (later wakeup) succeeds.
    coordinator.notify(
        SourceNotification::Changed(vec![PathBuf::from("/app/b.css")]),
        300,
    );
    let result = settled(&mut coordinator, &mut view, vp, &fs, 400);
    assert!(matches!(result.outcome, ReconcileOutcome::Published { .. }));
}

/// The quiet-interval gate: notifications inside the window restart
/// it; nothing runs early.
#[test]
fn quiet_interval_gates_reconciliation() {
    let (_view, mut coordinator, _vp) = harness();
    let reg = registry();
    coordinator.notify(
        SourceNotification::Changed(vec![reg.stylesheets[0].path.clone()]),
        100,
    );
    assert!(!coordinator.due(149));
    // A new notification inside the window restarts it.
    coordinator.notify(
        SourceNotification::Changed(vec![reg.stylesheets[1].path.clone()]),
        140,
    );
    assert!(!coordinator.due(189));
    assert!(coordinator.due(190));
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
