//! VelquView Lab — the development host.
//!
//! M1 surface: load a local app directory (`index.html` + `*.css`), render it
//! in a native window, or render offscreen for fixtures/evidence:
//!
//! ```text
//! velqu-lab ./examples/hello                        # native window
//! velqu-lab --headless --size 800x600 --frames 5 \
//!            --out frame.png ./examples/hello       # offscreen capture
//! ```
//!
//! Inspection (DOM/style/layout/reactive/event trace), reload, and Tailwind
//! compatibility warnings are M3+/M6 work; the flags below are the stable
//! harness those features will hang off.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use velqu_view::{VelquView, Viewport};

struct Args {
    app_dir: PathBuf,
    headless: bool,
    size: (u32, u32),
    scale: f32,
    out: Option<PathBuf>,
    frames: u32,
    exit_after: Option<Duration>,
    tailwind: bool,
    reactive: bool,
    inspect: bool,
}

const USAGE: &str = "\
velqu-lab — VelquView development host

USAGE:
    velqu-lab [OPTIONS] [APP_DIR]

ARGS:
    APP_DIR            App directory containing index.html (default: .)

OPTIONS:
    --headless         Render offscreen; no window (fixtures, CI)
    --tailwind         Compile Tailwind utility classes into CSS (ADR 0009)
    --reactive         Enable Velqu Reactive: compile vx-* markup into the
                       bounded execution plan and drive reactive turns (ADR 0016/0017)
    --inspect          Enable the inspector trace (ADR 0020) and print the
                       snapshot + retained trace after the headless run
    --size WxH         Logical viewport size (default 1024x640)
    --scale F          DPI scale factor (default 1.0)
    --frames N         Headless: render N frames and verify determinism (default 1)
    --out PATH         Headless: PNG output path (default <APP_DIR>/out/frame.png)
    --exit-after-ms N  Window: auto-close after N milliseconds (smoke tests)
    -h, --help         Print this help
";

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        app_dir: PathBuf::from("."),
        headless: false,
        size: (1024, 640),
        scale: 1.0,
        out: None,
        frames: 1,
        exit_after: None,
        tailwind: false,
        reactive: false,
        inspect: false,
    };
    let mut positional: Vec<String> = Vec::new();
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--headless" => args.headless = true,
            "--window" => args.headless = false,
            "--tailwind" => args.tailwind = true,
            "--reactive" => args.reactive = true,
            "--inspect" => args.inspect = true,
            "--size" => {
                let value = argv.next().ok_or("--size requires WxH")?;
                args.size = parse_size(&value)?;
            }
            "--scale" => {
                let value = argv.next().ok_or("--scale requires a number")?;
                args.scale = value
                    .parse()
                    .map_err(|_| format!("invalid scale {value:?}"))?;
                if !args.scale.is_finite() || args.scale <= 0.0 {
                    return Err(format!("scale must be > 0, got {value:?}"));
                }
            }
            "--frames" => {
                let value = argv.next().ok_or("--frames requires a count")?;
                args.frames = value
                    .parse()
                    .map_err(|_| format!("invalid frame count {value:?}"))?;
                if args.frames == 0 {
                    return Err("frame count must be >= 1".into());
                }
            }
            "--out" => {
                let value = argv.next().ok_or("--out requires a path")?;
                args.out = Some(PathBuf::from(value));
            }
            "--exit-after-ms" => {
                let value = argv.next().ok_or("--exit-after-ms requires a count")?;
                let ms: u64 = value
                    .parse()
                    .map_err(|_| format!("invalid duration {value:?}"))?;
                args.exit_after = Some(Duration::from_millis(ms));
            }
            other if other.starts_with('-') => return Err(format!("unknown option {other:?}")),
            other => positional.push(other.to_owned()),
        }
    }
    match positional.len() {
        0 => {}
        1 => args.app_dir = PathBuf::from(&positional[0]),
        n => return Err(format!("expected one app directory, got {n}")),
    }
    Ok(args)
}

fn parse_size(value: &str) -> Result<(u32, u32), String> {
    let (w, h) = value
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("size must look like 800x600, got {value:?}"))?;
    let parse = |v: &str| v.parse().map_err(|_| format!("invalid size {value:?}"));
    let (w, h) = (parse(w)?, parse(h)?);
    if w == 0 || h == 0 {
        return Err(format!("size must be non-zero, got {value:?}"));
    }
    Ok((w, h))
}

/// The lab's host-side asset resolver: maps renderer asset requests onto
/// the app directory. This is where filesystem access lives — the renderer
/// itself never touches the disk (ADR 0004).
struct DirAssets {
    root: PathBuf,
}

impl velqu_view::AssetResolver for DirAssets {
    fn resolve(&self, request: velqu_view::AssetRequest<'_>) -> Option<velqu_view::Asset> {
        let rel = request.path.trim_start_matches("./");
        // Stay inside the app directory: reject traversal and absolute refs.
        if rel.starts_with('/') || rel.split(['/', '\\']).any(|segment| segment == "..") {
            return None;
        }
        let path = self.root.join(rel);
        let bytes = std::fs::read(&path).ok()?;
        Some(velqu_view::Asset {
            id: velqu_view::SourceId::new(rel),
            bytes,
        })
    }
}

/// Loads `index.html` plus every `*.css` in `app_dir` (sorted by name),
/// with source identity taken from the file names and a host asset
/// resolver installed over the app directory.
fn load_app(view: &mut VelquView, app_dir: &Path) -> Result<(), String> {
    let index = app_dir.join("index.html");
    let html = std::fs::read_to_string(&index)
        .map_err(|e| format!("cannot read {}: {e}", index.display()))?;
    let base = app_dir.to_string_lossy().into_owned();
    let document =
        velqu_view::DocumentSource::new(index.to_string_lossy().into_owned(), html).with_base(base);
    view.load_document(document).map_err(|e| e.to_string())?;
    view.set_asset_resolver(std::rc::Rc::new(DirAssets {
        root: app_dir.to_owned(),
    }));

    let mut css_files: Vec<PathBuf> = match std::fs::read_dir(app_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "css"))
            .collect(),
        Err(e) => return Err(format!("cannot read {}: {e}", app_dir.display())),
    };
    css_files.sort();
    for css_file in css_files {
        let css = std::fs::read_to_string(&css_file)
            .map_err(|e| format!("cannot read {}: {e}", css_file.display()))?;
        let sheet = velqu_view::StylesheetSource::new(css_file.to_string_lossy().into_owned(), css);
        view.load_stylesheet(sheet).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn human_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else {
        format!("{:.1} KiB", n as f64 / 1024.0)
    }
}

/// Prints the inspector snapshot and the retained trace (ADR 0020) —
/// the lab's developer surface for "what did Velqu actually do".
/// Reads recorded outcomes only; it runs after the frames, so it can
/// never perturb them.
fn print_inspection(view: &VelquView, viewport: velqu_view::Viewport) {
    use velqu_view::{LayoutCacheState, TraceRecordKind};

    let snapshot = view.inspector_snapshot(viewport, None);
    println!(
        "inspector: generation {}, state revision {}, layout revision {}, frame {}",
        snapshot.generation,
        snapshot.state_revision,
        snapshot.layout_revision,
        snapshot.frame_index
    );
    let layout = match snapshot.layout {
        LayoutCacheState::Fresh => "fresh",
        LayoutCacheState::Stale => "stale (different viewport)",
        LayoutCacheState::NotAvailable => "not available yet",
    };
    let mut state = format!("layout {layout}");
    if snapshot.awaiting_relayout {
        state.push_str(", relayout pending");
    }
    if snapshot.awaiting_repaint {
        state.push_str(", repaint pending");
    }
    println!("inspector: {state}");
    println!(
        "inspector: {} pass(es), {} repaint(s), {} turn(s), {} display item(s)",
        snapshot.counters.layout_passes,
        snapshot.counters.repaints,
        snapshot.counters.reactive_turns,
        snapshot.counters.display_items_last
    );
    if !snapshot.pending.structural.is_empty() || !snapshot.pending.presentation.is_empty() {
        println!(
            "inspector: pending causes: structural {:?}, presentation {:?}",
            snapshot.pending.structural, snapshot.pending.presentation
        );
    }
    for diagnostic in &snapshot.diagnostics {
        println!(
            "inspector: [{}] {}",
            diagnostic.subsystem, diagnostic.message
        );
    }

    let summary = view.inspector_trace_summary();
    println!(
        "trace: {} record(s) retained (first #{}, {} appended, {} evicted, {} truncated)",
        summary.retained,
        summary.first_retained,
        summary.appended,
        summary.evicted,
        summary.truncated
    );
    const SHOWN: usize = 24;
    let records = view.inspector_records();
    let start = records.len().saturating_sub(SHOWN);
    if start > 0 {
        println!("trace: … {} earlier record(s) elided", start);
    }
    for record in &records[start..] {
        match &record.kind {
            TraceRecordKind::Event(event) => {
                let target = event.target_id.as_deref().unwrap_or("-");
                let generation = match event.event_generation {
                    Some(generation) if generation != event.generation => {
                        format!(" (gen {generation}→{})", event.generation)
                    }
                    _ => String::new(),
                };
                let value = event
                    .value_len
                    .map(|len| format!(" value[{len}]"))
                    .unwrap_or_default();
                println!(
                    "#{} event {} {}{generation}{value}",
                    record.seq, event.kind, target
                );
            }
            TraceRecordKind::Turn(turn) => {
                let trigger = turn
                    .trigger
                    .map_or_else(|| "init".to_owned(), |seq| format!("#{seq}"));
                match &turn.outcome {
                    velqu_view::TurnOutcomeRecord::Committed { count } => {
                        println!(
                            "#{} turn ←{trigger}: {} proposed, {count} committed (revision {}→{})",
                            record.seq,
                            turn.attempted_mutations,
                            turn.state_revision_before,
                            turn.state_revision_after
                        );
                    }
                    velqu_view::TurnOutcomeRecord::Rejected => {
                        println!(
                            "#{} turn ←{trigger}: {} proposed, REJECTED",
                            record.seq, turn.attempted_mutations
                        );
                    }
                    velqu_view::TurnOutcomeRecord::RolledBack => {
                        println!("#{} turn ←{trigger}: rolled back", record.seq);
                    }
                    velqu_view::TurnOutcomeRecord::NoMatch => {
                        println!("#{} turn ←{trigger}: no match", record.seq);
                    }
                }
            }
            TraceRecordKind::Invalidation(invalidation) => {
                let class = match invalidation.classification {
                    velqu_view::InvalidationClass::Presentation => "presentation",
                    velqu_view::InvalidationClass::Structural => "structural",
                };
                let dropped = invalidation.truncated_causes;
                println!(
                    "#{} invalidation {class}: {:?}{}",
                    record.seq,
                    invalidation.causes,
                    (dropped > 0)
                        .then_some(format!(" (+{dropped} truncated)"))
                        .unwrap_or_default()
                );
            }
            TraceRecordKind::Render(render) => {
                let settled = render
                    .settled
                    .iter()
                    .map(|seq| format!("#{seq}"))
                    .collect::<Vec<_>>()
                    .join(",");
                println!(
                    "#{} render frame {}: +{} pass, +{} repaint (settled {settled})",
                    record.seq, render.frame_index, render.layout_pass_delta, render.repaint_delta
                );
            }
        }
    }
}

fn run_headless(args: &Args, view: &mut VelquView) -> Result<(), String> {
    // Physical = logical × scale, rounded per axis (1.25 × 400 = 500).
    let physical = |logical: u32| (logical as f32 * args.scale).round() as u32;
    let viewport = Viewport::try_new(physical(args.size.0), physical(args.size.1), args.scale)
        .map_err(|e| e.to_string())?;
    let mut hashes: Vec<String> = Vec::new();
    let mut durations: Vec<Duration> = Vec::new();
    let mut first: Option<velqu_view::FrameResult> = None;
    for _ in 0..args.frames {
        // One pump per frame, mirroring the shell's redraw order (M5c):
        // drain the queue into a batch, pump the batch (M6a ownership,
        // ADR 0019), then render. Turn-zero mutations land before the
        // first render; drained batches are not re-observed here.
        let batch = view.take_events();
        view.pump_reactive(&batch);
        drop(batch);
        let started = Instant::now();
        let result = view.render(viewport).map_err(|e| e.to_string())?;
        durations.push(started.elapsed());
        hashes.push(result.frame.sha256_hex());
        if first.is_none() {
            first = Some(result);
        }
    }
    let first = first.expect("frames >= 1");

    let identical = hashes.iter().filter(|h| **h == hashes[0]).count();
    println!(
        "app: {} (html {}, css {} sheet(s))",
        args.app_dir.display(),
        human_bytes(view.html().map(str::len).unwrap_or(0)),
        view.stylesheets().len()
    );
    println!(
        "viewport: {}x{} px @ {:.2}x (logical {}x{})",
        viewport.width(),
        viewport.height(),
        viewport.scale_factor(),
        args.size.0,
        args.size.1
    );
    println!(
        "frame 1: {} items, {} glyphs",
        first.stats.items, first.stats.glyphs
    );
    for message in view.image_diagnostics() {
        println!("image diagnostic: {message}");
    }
    if view.tailwind_enabled() {
        let diagnostics = view.tailwind_diagnostics();
        if diagnostics.is_empty() {
            println!("tailwind: all utility classes compiled");
        } else {
            for message in diagnostics {
                println!("tailwind diagnostic: {message}");
            }
        }
    }
    if view.reactive_enabled() {
        match view.reactive_plan() {
            Some(plan) => {
                println!(
                    "reactive: {} scope(s), {} binding(s), {} handler(s)",
                    plan.scopes.len(),
                    plan.bindings.len(),
                    plan.events.len()
                );
                for message in view.reactive_diagnostics() {
                    println!("reactive diagnostic: {message}");
                }
            }
            None => println!("reactive: no vx-* markup in this document"),
        }
    }
    println!("sha256: {}", hashes[0]);
    if args.frames > 1 {
        println!("determinism: {identical}/{} frames identical", args.frames);
    }
    let avg = durations.iter().sum::<Duration>() / durations.len() as u32;
    println!(
        "render wall time: first {:.2?}, avg {:.2?} (indicative, not a benchmark)",
        durations[0], avg
    );

    let out = args
        .out
        .clone()
        .unwrap_or_else(|| args.app_dir.join("out").join("frame.png"));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    first
        .frame
        .save_png(&out)
        .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    println!("png: {}", out.display());

    if identical != hashes.len() {
        return Err("determinism check FAILED: frames differ".into());
    }
    Ok(())
}

fn run_window(args: &Args, view: &mut VelquView) -> Result<(), String> {
    let title = format!("VelquView Lab — {}", args.app_dir.display());
    let mut config = velqu_shell::ShellConfig::new(title)
        .with_logical_size(args.size.0 as f32, args.size.1 as f32);
    if let Some(exit_after) = args.exit_after {
        config = config.with_exit_after(exit_after);
    }
    let stats = velqu_shell::run(config, view).map_err(|e| e.to_string())?;
    println!(
        "window closed: {} frame(s) presented in {:.1?}",
        stats.frames, stats.uptime
    );
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("velqu-lab: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let mut view = VelquView::new();
    if args.tailwind {
        view.enable_tailwind();
    }
    if args.reactive {
        view.enable_reactive();
    }
    if args.inspect {
        view.enable_inspector();
    }
    if let Err(message) = load_app(&mut view, &args.app_dir) {
        eprintln!("velqu-lab: {message}");
        return ExitCode::from(2);
    }

    let result = if args.headless {
        run_headless(&args, &mut view)
    } else {
        run_window(&args, &mut view)
    };
    if args.inspect && args.headless && result.is_ok() {
        let physical = |logical: u32| (logical as f32 * args.scale).round() as u32;
        if let Ok(viewport) =
            Viewport::try_new(physical(args.size.0), physical(args.size.1), args.scale)
        {
            print_inspection(&view, viewport);
        }
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("velqu-lab: {message}");
            ExitCode::FAILURE
        }
    }
}
