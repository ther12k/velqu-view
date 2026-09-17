//! M5a gate battery (ADR 0015): budgets, isolation, determinism, and the
//! absent host surface. Every hostile script must terminate with a
//! classified failure and leave the host untouched.

use super::*;
use std::time::Duration;

/// Tight budgets so hostile tests run in milliseconds, not seconds.
fn test_limits() -> JsLimits {
    JsLimits {
        max_heap_bytes: 4 * 1024 * 1024,
        max_stack_bytes: 256 * 1024,
        max_execution_time: Duration::from_millis(100),
        max_source_bytes: 64 * 1024,
        max_event_payload_bytes: 16 * 1024,
        max_mutations_per_turn: 64,
        max_output_string_bytes: 8 * 1024,
        max_pending_jobs: 16,
    }
}

fn runtime() -> ReactiveRuntime {
    ReactiveRuntime::new(1, test_limits()).expect("runtime builds")
}

#[test]
fn zero_limits_are_rejected() {
    let limits = JsLimits {
        max_heap_bytes: 0,
        ..JsLimits::default()
    };
    assert!(limits.try_new().is_err());
    assert!(JsLimits::default().try_new().is_ok());
}

#[test]
fn infinite_loop_is_interrupted_and_the_runtime_survives() {
    let rt = runtime();
    let started = Instant::now();
    let failure = rt.evaluate("while (true) { }").expect_err("terminated");
    assert!(
        matches!(failure, JsFailure::Interrupted),
        "classified as an interrupt: {failure:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the deadline bounded the loop"
    );
    // The context stays usable after the uncatchable interrupt.
    rt.evaluate("var ok = 1 + 1").expect("runtime still usable");
}

#[test]
fn memory_bomb_is_terminated() {
    let rt = runtime();
    let failure = rt
        .evaluate(r#"const hoard = []; for (;;) { hoard.push("xxxxxxxxxxxxxxxx"); }"#)
        .expect_err("terminated");
    assert!(
        matches!(
            failure,
            JsFailure::OutOfMemory | JsFailure::Interrupted | JsFailure::StackOverflow
        ),
        "classified bounded failure: {failure:?}"
    );
}

#[test]
fn deep_recursion_hits_the_stack_limit() {
    let rt = runtime();
    let failure = rt
        .evaluate("function f() { return f() + 1; } f()")
        .expect_err("terminated");
    assert!(
        matches!(failure, JsFailure::StackOverflow),
        "classified as stack exhaustion: {failure:?}"
    );
}

#[test]
fn oversized_strings_are_refused() {
    let rt = runtime();
    // Beyond QuickJS's maximum string length: an immediate RangeError.
    let too_long = rt.evaluate("('x').repeat(2 ** 31)").expect_err("refused");
    assert!(matches!(
        too_long,
        JsFailure::Exception(_) | JsFailure::OutOfMemory
    ));
    // Within the length cap but over the heap budget.
    let heap_hog = rt
        .evaluate("('y').repeat(64 * 1024 * 1024)")
        .expect_err("refused");
    assert!(matches!(
        heap_hog,
        JsFailure::OutOfMemory | JsFailure::Exception(_)
    ));
}

#[test]
fn job_bomb_is_bounded() {
    let rt = runtime();
    rt.evaluate("function chain() { Promise.resolve().then(chain); } chain()")
        .expect_err("the drain trips a budget");
    // The failure surfaces on the drain inside evaluate(); the count-based
    // budget specifically on an explicit drain:
    let failure = rt.drain_jobs().expect_err("still bounded");
    assert!(
        matches!(failure, JsFailure::TooManyPendingJobs { .. }),
        "job budget enforced: {failure:?}"
    );
}

#[test]
fn microtasks_run_within_a_turn() {
    let rt = runtime();
    rt.evaluate("var landed = 0; Promise.resolve().then(() => { landed = 1; });")
        .expect("bounded chain evaluates");
    rt.evaluate("if (landed !== 1) { throw new Error('microtask did not run'); }")
        .expect("the microtask executed during the turn's drain");
}

#[test]
fn oversized_source_is_rejected_before_evaluation() {
    let rt = runtime();
    let limits = test_limits();
    let source = "var x = 1;\n".repeat(limits.max_source_bytes / 11 + 1);
    assert!(source.len() > limits.max_source_bytes);
    let failure = rt.evaluate(&source).expect_err("rejected");
    assert_eq!(
        failure,
        JsFailure::SourceTooLarge {
            size: source.len(),
            max: limits.max_source_bytes,
        }
    );
}

#[test]
fn exception_diagnostics_are_captured_not_fatal() {
    let rt = runtime();
    let failure = rt
        .evaluate("throw new Error('boom')")
        .expect_err("captured");
    match failure {
        JsFailure::Exception(text) => assert!(text.contains("boom"), "{text}"),
        other => panic!("expected an exception failure, got {other:?}"),
    }
    rt.evaluate("var after = true")
        .expect("runtime still usable");
}

#[test]
fn reload_destroys_the_js_world() {
    let first = ReactiveRuntime::new(7, test_limits()).unwrap();
    first.evaluate("globalThis.marker = 42;").unwrap();
    drop(first);
    let second = ReactiveRuntime::new(8, test_limits()).unwrap();
    let kind = second
        .ctx
        .with(|ctx| ctx.eval::<String, _>("typeof globalThis.marker"))
        .expect("typeof evaluates");
    assert_eq!(kind, "undefined", "globals die with the runtime");
    assert_eq!(second.generation(), 8);
}

#[test]
fn no_ambient_host_surface_exists() {
    let rt = runtime();
    rt.evaluate(
        r#"
        for (const name of ["require", "process", "window", "document",
                            "fetch", "XMLHttpRequest", "localStorage",
                            "__velquEmit", "__velquNow", "__velquClock",
                            "__velquNowInternal"]) {
            if (typeof globalThis[name] !== "undefined") {
                throw new Error("ambient surface leaked: " + name);
            }
        }
        const velquKeys = Object.keys(velqu).sort().join(",");
        if (velquKeys !== "log") { throw new Error("velqu surface: " + velquKeys); }
        "#,
    )
    .expect("the profile is exactly the sanctioned surface");
}

#[test]
fn clock_and_random_are_deterministic() {
    let a = ReactiveRuntime::new(3, test_limits()).unwrap();
    let b = ReactiveRuntime::new(3, test_limits()).unwrap();
    let probe = "Date.now() + ' ' + Math.random() + ' ' + Math.random()";
    let sa = a.ctx.with(|c| c.eval::<String, _>(probe)).unwrap();
    let sb = b.ctx.with(|c| c.eval::<String, _>(probe)).unwrap();
    assert_eq!(sa, sb, "identical generations render identically");
    assert!(sa.starts_with(&LOGICAL_EPOCH_MS.to_string()), "{sa}");

    // A tick advances the logical clock; a new generation reseeds.
    a.logical_tick();
    let advanced = a.ctx.with(|c| c.eval::<f64, _>("Date.now()")).unwrap();
    assert_eq!(advanced as u64, LOGICAL_EPOCH_MS + 1_000);
    let c = ReactiveRuntime::new(4, test_limits()).unwrap();
    let pa = a.ctx.with(|x| x.eval::<f64, _>("Math.random()")).unwrap();
    let pc = c.ctx.with(|x| x.eval::<f64, _>("Math.random()")).unwrap();
    assert_ne!(pa, pc, "reload reseeds the sequence");
}

#[test]
fn console_sink_is_recorded_and_capped() {
    let rt = runtime();
    rt.evaluate("console.log('hello', 42); velqu.log('via velqu'); console.error('bad', true);")
        .expect("logging never fails");
    let lines = rt.diagnostics();
    assert_eq!(lines[..3].to_vec(), ["hello 42", "via velqu", "bad true"]);
    rt.evaluate("for (let i = 0; i < 10000; i++) { console.log('spam'); }")
        .expect("spam is not an error");
    assert!(
        rt.diagnostics().len() <= 256,
        "the sink is line-capped: {}",
        rt.diagnostics().len()
    );
}

#[test]
fn output_lines_are_length_capped() {
    let rt = runtime();
    rt.evaluate("console.log('z'.repeat(100 * 1024))").unwrap();
    let line = &rt.diagnostics()[0];
    assert!(
        line.len() <= test_limits().max_output_string_bytes + 3,
        "{}",
        line.len()
    );
}
