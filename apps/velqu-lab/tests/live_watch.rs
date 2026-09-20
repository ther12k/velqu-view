//! Live watch regression (post-closure defect repair): the real lab
//! binary, a real window, and **no injected input of any kind** — the
//! window must reconcile and publish on its own scheduling.
//!
//! This is the test the injected-clock coordinator suite cannot be:
//! it establishes that the *host actually schedules* reconciliation
//! inside a waiting event loop. It needs a display session and is
//! therefore `#[ignore]`d (CI has no display lane); run locally with:
//!
//! ```sh
//! cargo test -p velqu-lab --test live_watch -- --ignored --nocapture
//! ```
//!
//! Coverage against the correction record's gate matrix:
//!
//! * single registered save, then nothing — must reconcile (native +
//!   polling backends); this exact test failed on the defective
//!   scheduling for the window's whole lifetime;
//! * burst: two saves inside the quiet interval settle to exactly one
//!   publication of the latest contents;
//! * rejected edit then repair: the old UI stays up (rejection is
//!   printed, not fatal) and the repair publishes without restart;
//! * shutdown with a pending deadline: clean termination, no hang,
//!   no rescheduling storm.
//!
//! The startup rescan pass is asserted along the way (it counts as
//! one unchanged-skip reconciliation) — the registration gap closes
//! live, not only under injected clocks.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Generous settle for first presentation on a slow machine.
const SETTLE: Duration = Duration::from_secs(4);
/// How long a save has to make it to screen.
const RECONCILE_BUDGET: Duration = Duration::from_secs(15);

fn unique_app_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("velqu-live-watch-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp app dir");
    dir
}

fn write_app(dir: &Path) {
    std::fs::write(
        dir.join("index.html"),
        "<!doctype html><html><head><meta charset=\"utf-8\"></head>\
         <body vx-state=\"{ n: 1 }\">\
         <p id=t data-vv-test=t vx-text=\"n\">1</p>\
         </body></html>",
    )
    .expect("index.html");
    std::fs::write(dir.join("app.css"), "p { color: #000000 }").expect("app.css");
}

#[derive(Clone)]
struct Output {
    lines: Arc<Mutex<Vec<String>>>,
}

impl Output {
    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
    fn wait_for(&self, predicate: &dyn Fn(&str) -> bool, budget: Duration, what: &str) -> String {
        let started = Instant::now();
        loop {
            if let Some(line) = self
                .lines()
                .into_iter()
                .find(|line| predicate(line.as_str()))
            {
                return line;
            }
            assert!(
                started.elapsed() < budget,
                "no {what} within {budget:?}.\noutput so far: {:?}",
                self.lines()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn launch(backend: &str, dir: &Path, exit_after_ms: u64) -> (Child, Output) {
    let bin = env!("CARGO_BIN_EXE_velqu-lab");
    let mut child = Command::new(bin)
        .arg("--tailwind")
        .arg("--reactive")
        .arg(backend)
        .arg("--exit-after-ms")
        .arg(exit_after_ms.to_string())
        .arg(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch velqu-lab");
    let lines = Arc::new(Mutex::new(Vec::new()));
    let mut streams: Vec<Box<dyn std::io::Read + Send>> = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        streams.push(Box::new(stdout));
    }
    if let Some(stderr) = child.stderr.take() {
        streams.push(Box::new(stderr));
    }
    for stream in streams {
        let sink = Arc::clone(&lines);
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                sink.lock().unwrap().push(line);
            }
        });
    }
    (child, Output { lines })
}

/// The single-save / no-further-input scenario against the real
/// binary. `backend` is `--watch` or `--watch=poll`.
fn single_edit_reconciles(backend: &str) {
    let dir = unique_app_dir(backend.trim_start_matches("--watch").trim_matches('='));
    write_app(&dir);

    let total = SETTLE + RECONCILE_BUDGET + Duration::from_secs(5);
    let (mut child, output) = launch(backend, &dir, total.as_millis() as u64);

    // First presentation settle. Nothing is injected into the window.
    std::thread::sleep(SETTLE);
    assert!(
        output
            .lines()
            .iter()
            .all(|line| !line.contains("reconciled")),
        "no reconcile may happen before the edit"
    );

    // The single registered save. One write, one flush, nothing else.
    std::fs::write(dir.join("app.css"), "p { color: #ff0000 }").expect("edit app.css");
    let reconciled = output.wait_for(
        &|line: &str| line.contains("watch: reconciled"),
        RECONCILE_BUDGET,
        "reconciliation of a single registered save with no further input — \
         the window never wakes on its own",
    );

    // The publication evidence: a stylesheet transaction (state
    // preserved), not a full-document restart. Exactly two
    // reconciliations are deterministic: the startup rescan pass
    // (disk == published there, counted as an unchanged skip) and
    // this edit's publication.
    assert!(
        reconciled.contains("kind=Stylesheets"),
        "expected the stylesheet transaction path, got: {reconciled}"
    );
    assert!(
        reconciled.contains("reconciliations: 2"),
        "startup rescan + the single edit, got: {reconciled}"
    );
    assert!(
        reconciled.contains("unchanged_skips: 1"),
        "the startup pass found the settled bundle unchanged: {reconciled}"
    );

    // Clean exit on the same process — no restart anywhere.
    let status = child.wait().expect("lab exits via --exit-after-ms");
    assert_eq!(status.code(), Some(0), "clean exit");
    let closed = output
        .lines()
        .into_iter()
        .find(|line| line.contains("window closed"))
        .expect("window closed line");
    assert!(
        closed.contains("frame(s)"),
        "frame evidence in the exit line: {closed}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "live window session; run with --ignored"]
fn live_single_css_edit_reconciles_native() {
    single_edit_reconciles("--watch");
}

#[test]
#[ignore = "live window session; run with --ignored"]
fn live_single_css_edit_reconciles_poll() {
    single_edit_reconciles("--watch=poll");
}

/// Burst saves inside the quiet interval settle to exactly ONE
/// publication carrying the latest contents — the deadline extends,
/// it does not fire per save.
#[test]
#[ignore = "live window session; run with --ignored"]
fn live_burst_edits_settle_to_one_publication_of_the_latest_contents() {
    let dir = unique_app_dir("burst");
    write_app(&dir);

    let total = SETTLE + RECONCILE_BUDGET + Duration::from_secs(5);
    let (mut child, output) = launch("--watch", &dir, total.as_millis() as u64);

    std::thread::sleep(SETTLE);
    // Two saves 60 ms apart — both inside the 120 ms quiet interval.
    std::fs::write(dir.join("app.css"), "p { color: #ff0000 }").expect("burst save 1");
    std::thread::sleep(Duration::from_millis(60));
    std::fs::write(dir.join("app.css"), "p { color: #00ff00 }").expect("burst save 2");

    output.wait_for(
        &|line: &str| line.contains("watch: reconciled"),
        RECONCILE_BUDGET,
        "burst publication",
    );
    std::thread::sleep(Duration::from_millis(500));
    let reconciles = output
        .lines()
        .into_iter()
        .filter(|line| line.contains("watch: reconciled"))
        .count();
    assert_eq!(
        reconciles,
        1,
        "one publication for the settled burst, got {} lines: {:?}",
        reconciles,
        output.lines()
    );

    let status = child.wait().expect("lab exits via --exit-after-ms");
    assert_eq!(status.code(), Some(0), "clean exit");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rejected edit (whitespace-only stylesheet) leaves the old UI
/// running and prints the rejection; repairing the file publishes on
/// the same process without a restart.
#[test]
#[ignore = "live window session; run with --ignored"]
fn live_rejected_edit_then_repair_publishes_without_restart() {
    let dir = unique_app_dir("rejected");
    write_app(&dir);

    let total = SETTLE + RECONCILE_BUDGET * 2 + Duration::from_secs(8);
    let (mut child, output) = launch("--watch", &dir, total.as_millis() as u64);

    std::thread::sleep(SETTLE);
    // The invalid save (empty source is rejected by reload policy).
    std::fs::write(dir.join("app.css"), "   ").expect("invalid save");
    output.wait_for(
        &|line: &str| line.starts_with("watch: ") && !line.contains("reconciled"),
        RECONCILE_BUDGET,
        "rejection of the invalid stylesheet",
    );

    // The repair.
    std::fs::write(dir.join("app.css"), "p { color: #ff0000 }").expect("repair save");
    let reconciled = output.wait_for(
        &|line: &str| line.contains("watch: reconciled"),
        RECONCILE_BUDGET,
        "publication of the repaired stylesheet",
    );
    assert!(
        reconciled.contains("kind=Stylesheets"),
        "the repair publishes a stylesheet transaction: {reconciled}"
    );

    let status = child.wait().expect("lab exits via --exit-after-ms");
    assert_eq!(status.code(), Some(0), "the old UI ran until exit");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Unregistered-file activity inside the watched directory must not
/// postpone registered-source work: the noise wakes the host but never
/// moves the debounce deadline.
#[test]
#[ignore = "live window session; run with --ignored"]
fn live_unregistered_noise_does_not_postpone_the_edit() {
    let dir = unique_app_dir("noise");
    write_app(&dir);

    let total = SETTLE + RECONCILE_BUDGET + Duration::from_secs(5);
    let (mut child, output) = launch("--watch", &dir, total.as_millis() as u64);

    std::thread::sleep(SETTLE);
    // The registered save, then a burst of unregistered noise inside
    // the quiet interval (each write is a watcher wake).
    std::fs::write(dir.join("app.css"), "p { color: #ff0000 }").expect("edit app.css");
    for at in [30, 60, 90] {
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(dir.join("notes.txt"), format!("noise {at}")).expect("noise write");
    }

    let reconciled = output.wait_for(
        &|line: &str| line.contains("watch: reconciled"),
        RECONCILE_BUDGET,
        "reconciliation despite unregistered-file noise",
    );
    assert!(
        reconciled.contains("kind=Stylesheets"),
        "the stylesheet transaction still publishes: {reconciled}"
    );
    std::thread::sleep(Duration::from_millis(300));
    let reconciles = output
        .lines()
        .into_iter()
        .filter(|line| line.contains("watch: reconciled"))
        .count();
    assert_eq!(reconciles, 1, "noise creates no publications of its own");

    let status = child.wait().expect("lab exits via --exit-after-ms");
    assert_eq!(status.code(), Some(0), "clean exit");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Shutdown with a pending debounce deadline: the save lands close
/// enough to the exit deadline that the timer is still armed when the
/// loop exits — the process must still terminate cleanly and quickly.
#[test]
#[ignore = "live window session; run with --ignored"]
fn live_shutdown_with_pending_deadline_exits_cleanly() {
    let dir = unique_app_dir("shutdown");
    write_app(&dir);

    // Exit at 5 s; the save at 4.85 s arms a deadline at ~4.97 s…
    // keep it past the exit: save at 4.95 s → deadline ~5.07 s.
    let (mut child, output) = launch("--watch", &dir, 5_000);
    std::thread::sleep(Duration::from_millis(4_950));
    std::fs::write(dir.join("app.css"), "p { color: #ff0000 }").expect("last-gasp save");

    // Watchdog: exit within 15 s of the deadline or kill + fail.
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "process did not terminate with a pending timer\noutput: {:?}",
            output.lines()
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(status.code(), Some(0), "clean exit with a timer pending");
    let _ = std::fs::remove_dir_all(&dir);
}
