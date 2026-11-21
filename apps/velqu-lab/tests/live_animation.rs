//! The real window loop must pace a lightweight looping animation even
//! when repainting it takes much less than a frame interval. Run with:
//!
//! ```sh
//! xvfb-run -a cargo test -p velqu-lab --test live_animation -- --ignored
//! ```

use std::path::Path;
use std::process::Command;

#[test]
#[ignore = "live window session; run with --ignored"]
fn live_pulse_respects_frame_deadline() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pulse");
    let output = Command::new(env!("CARGO_BIN_EXE_velqu-lab"))
        .args(["--tailwind", "--size", "96x64", "--exit-after-ms", "1000"])
        .arg(fixture)
        .output()
        .expect("launch velqu-lab");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "window failed: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let frames: u32 = stdout
        .lines()
        .find_map(|line| line.strip_prefix("window closed: "))
        .and_then(|line| line.split_whitespace().next())
        .and_then(|count| count.parse().ok())
        .expect("window exit reports the frame count");

    // A 16 ms cadence allows about 63 frames per second. Leave room for
    // initial window-system redraws but catch an unthrottled redraw loop.
    assert!(
        (2..=70).contains(&frames),
        "expected a paced animation, got {frames} frames: {stdout}"
    );
}
