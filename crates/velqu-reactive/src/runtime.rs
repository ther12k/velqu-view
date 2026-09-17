//! The isolated QuickJS runtime gate for Velqu Reactive (M5a, ADR 0015).
//!
//! QuickJS is Velqu's **bounded UI-computation engine** — nothing else. It
//! never becomes `window`, `document`, `fetch`, a filesystem, Node, or a
//! browser compatibility layer; a reactive document gets one isolated
//! runtime per document generation, destroyed on reload, so stale
//! callbacks, old globals, and queued jobs cannot cross navigation.
//!
//! Hard budgets are part of the contract from commit one
//! ([`JsLimits`]): heap, stack, wall-clock execution deadline (enforced
//! through QuickJS's interrupt handler), source and payload sizes,
//! mutation counts, output string length, and pending-job count. A
//! hostile component cannot hang or balloon the host shell.
//!
//! Determinism is explicit (ADR 0015): wall-clock time and entropy are
//! host-controlled — `Date.now()` reads a logical per-turn clock and
//! `Math.random()` is a seeded xorshift — so deterministic scripts render
//! identically across instances and runs.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::{Context, Ctx, Error as JsError, Function, Object, Runtime, Value};

/// The deterministic epoch `Date` reports: 2000-01-01T00:00:00Z. Fixing it
/// keeps date rendering independent of the host's wall clock.
pub const LOGICAL_EPOCH_MS: u64 = 946_684_800_000;

/// How far the logical clock advances per reactive turn (one tick).
const CLOCK_STEP_MS: u64 = 1_000;

/// Cap on retained diagnostic lines regardless of other limits, so a
/// spamming component cannot grow host memory through the sink alone.
const MAX_DIAGNOSTIC_LINES: usize = 256;

/// Hard budgets for one isolated runtime (M5a, ADR 0015). All limits are
/// enforced; zero values are rejected by [`JsLimits::try_new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsLimits {
    /// Maximum QuickJS heap bytes (`JS_SetMemoryLimit`).
    pub max_heap_bytes: usize,
    /// Maximum native stack bytes (`JS_SetMaxStackSize`).
    pub max_stack_bytes: usize,
    /// Wall-clock deadline per evaluation/drain phase, enforced by the
    /// interrupt handler as an uncatchable interrupt.
    pub max_execution_time: Duration,
    /// Maximum accepted script source size in bytes (checked before
    /// compilation).
    pub max_source_bytes: usize,
    /// Maximum accepted event payload size in bytes (M5c).
    pub max_event_payload_bytes: usize,
    /// Maximum mutations one reactive turn may request (M5c/M5d); the
    /// rest of the batch is refused.
    pub max_mutations_per_turn: usize,
    /// Maximum length of any single string crossing the boundary out of
    /// JS (diagnostic lines, binding outputs).
    pub max_output_string_bytes: usize,
    /// Maximum pending jobs (microtasks) drained per turn.
    pub max_pending_jobs: usize,
}

impl Default for JsLimits {
    /// Development defaults: a 16 MiB heap, 1 MiB stack, 16 ms deadline,
    /// 256 KiB sources, 64 KiB payloads/output strings, 1024 mutations,
    /// 256 jobs. Hosts may tighten per document.
    fn default() -> Self {
        Self {
            max_heap_bytes: 16 * 1024 * 1024,
            max_stack_bytes: 1024 * 1024,
            max_execution_time: Duration::from_millis(16),
            max_source_bytes: 256 * 1024,
            max_event_payload_bytes: 64 * 1024,
            max_mutations_per_turn: 1024,
            max_output_string_bytes: 64 * 1024,
            max_pending_jobs: 256,
        }
    }
}

impl JsLimits {
    /// Validates that every budget is nonzero; zero would silently mean
    /// "unlimited" (heap/stack) or "impossible" (time), neither of which
    /// belongs in an embedded UI runtime.
    pub fn try_new(self) -> Result<Self, InvalidJsLimits> {
        let message = if self.max_heap_bytes == 0 {
            Some("max_heap_bytes must be nonzero")
        } else if self.max_stack_bytes == 0 {
            Some("max_stack_bytes must be nonzero")
        } else if self.max_execution_time.is_zero() {
            Some("max_execution_time must be nonzero")
        } else if self.max_source_bytes == 0 {
            Some("max_source_bytes must be nonzero")
        } else if self.max_event_payload_bytes == 0 {
            Some("max_event_payload_bytes must be nonzero")
        } else if self.max_mutations_per_turn == 0 {
            Some("max_mutations_per_turn must be nonzero")
        } else if self.max_output_string_bytes == 0 {
            Some("max_output_string_bytes must be nonzero")
        } else if self.max_pending_jobs == 0 {
            Some("max_pending_jobs must be nonzero")
        } else {
            None
        };
        match message {
            Some(message) => Err(InvalidJsLimits {
                message: message.to_owned(),
            }),
            None => Ok(self),
        }
    }
}

/// Why a [`JsLimits`] was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidJsLimits {
    /// Human-readable reason.
    pub message: String,
}

impl fmt::Display for InvalidJsLimits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InvalidJsLimits {}

/// How a budgeted evaluation failed. Every variant is a bounded, host-safe
/// outcome; none leaves the host process worse than it found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsFailure {
    /// The source exceeded `max_source_bytes` before compilation.
    SourceTooLarge {
        /// Actual source size in bytes.
        size: usize,
        /// The configured maximum.
        max: usize,
    },
    /// The execution deadline elapsed; QuickJS raised the uncatchable
    /// interrupt.
    Interrupted,
    /// The heap limit was reached.
    OutOfMemory,
    /// The stack limit was reached (deep or infinite recursion).
    StackOverflow,
    /// JavaScript threw. The text is the coerced exception (message +
    /// stack where available), capped by `max_output_string_bytes`.
    Exception(String),
    /// The pending-job drain exceeded `max_pending_jobs`.
    TooManyPendingJobs {
        /// Jobs executed before the budget tripped.
        executed: usize,
    },
}

impl fmt::Display for JsFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsFailure::SourceTooLarge { size, max } => {
                write!(f, "script source too large: {size} bytes > {max}")
            }
            JsFailure::Interrupted => {
                write!(f, "script exceeded its execution deadline")
            }
            JsFailure::OutOfMemory => write!(f, "script exceeded its memory budget"),
            JsFailure::StackOverflow => write!(f, "script exceeded its stack budget"),
            JsFailure::Exception(text) => write!(f, "uncaught exception: {text}"),
            JsFailure::TooManyPendingJobs { executed } => {
                write!(f, "pending-job budget exceeded after {executed} jobs")
            }
        }
    }
}

impl std::error::Error for JsFailure {}

/// One isolated QuickJS runtime, owned by one document generation
/// (M5a, ADR 0015). Dropping it — or loading the next document — destroys
/// the entire JS world: globals, heap, and queued jobs.
pub struct ReactiveRuntime {
    generation: u64,
    limits: JsLimits,
    runtime: Runtime,
    ctx: Context,
    /// Deadline shared with the interrupt handler; armed before every
    /// evaluation/drain phase.
    deadline: Rc<Cell<Instant>>,
    /// Logical clock ticks (each turn advances one step).
    clock: Rc<Cell<u64>>,
    /// Seeded xorshift state behind `Math.random`.
    rng: Rc<Cell<u32>>,
    /// Diagnostic sink shared with `console`/`velqu.log`.
    diagnostics: Rc<RefCell<Vec<String>>>,
}

impl fmt::Debug for ReactiveRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReactiveRuntime")
            .field("generation", &self.generation)
            .field("limits", &self.limits)
            .finish()
    }
}

impl ReactiveRuntime {
    /// Creates an isolated runtime for `generation` under `limits`.
    ///
    /// The global surface is exactly: the ECMAScript builtins with
    /// host-controlled `Date`/`Math.random`, a capped `console` sink, and
    /// a `velqu` object carrying the same sink. No I/O, no module loader,
    /// no `window`/`document`, no native calls beyond these.
    pub fn new(generation: u64, limits: JsLimits) -> Result<Self, JsFailure> {
        let limits = limits.try_new().map_err(|InvalidJsLimits { message }| {
            JsFailure::Exception(format!("invalid runtime limits: {message}"))
        })?;
        let runtime = Runtime::new().map_err(|error| JsFailure::Exception(error.to_string()))?;
        // The C allocator must stay active for the memory limit to bind:
        // neither the "rust-alloc" nor "allocator" feature is enabled
        // (ADR 0015 records the trap).
        runtime.set_memory_limit(limits.max_heap_bytes);
        runtime.set_max_stack_size(limits.max_stack_bytes);
        // Armed at construction (the profile prelude runs under the same
        // budget); every evaluate/drain re-arms it afterwards.
        let deadline = Rc::new(Cell::new(Instant::now() + limits.max_execution_time));
        {
            let deadline = Rc::clone(&deadline);
            runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline.get())));
        }
        let clock = Rc::new(Cell::new(0u64));
        let rng = Rc::new(Cell::new(seed_for(generation)));
        let diagnostics = Rc::new(RefCell::new(Vec::new()));
        // `full` intrinsics: the host itself evaluates sources through
        // `Ctx::eval`, which rides the same `eval_internal` hook as
        // script-side dynamic codegen (QuickJS-NG design — dropping the
        // Eval intrinsic kills host compilation too; ADR 0015 amendment
        // records the probe). Dynamic codegen is instead killed at the
        // handle level in `install_profile`: every script-reachable path
        // to the compiler is removed or made to throw.
        let ctx =
            Context::full(&runtime).map_err(|error| JsFailure::Exception(error.to_string()))?;
        let this = Self {
            generation,
            limits,
            runtime,
            ctx,
            deadline,
            clock,
            rng,
            diagnostics,
        };
        this.install_profile()?;
        Ok(this)
    }

    /// The document generation this runtime belongs to.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The budgets in force.
    pub fn limits(&self) -> &JsLimits {
        &self.limits
    }

    /// Evaluates `source` as a program under the armed budgets, then
    /// drains pending jobs (microtasks) within the same deadline and the
    /// job-count budget. Errors are classified, never fatal to the host.
    pub fn evaluate(&self, source: &str) -> Result<(), JsFailure> {
        if source.len() > self.limits.max_source_bytes {
            return Err(JsFailure::SourceTooLarge {
                size: source.len(),
                max: self.limits.max_source_bytes,
            });
        }
        self.arm_deadline();
        let outcome = self.ctx.with(|ctx| {
            ctx.eval::<(), _>(source)
                .map_err(|error| self.classify(&ctx, error))
        });
        outcome?;
        self.drain_jobs()
    }

    /// Drains pending microtasks under a fresh deadline and the
    /// job-count budget. One reactive turn is: handler → this drain →
    /// binding reevaluation (M5c).
    pub fn drain_jobs(&self) -> Result<(), JsFailure> {
        self.arm_deadline();
        let mut executed = 0usize;
        while self.runtime.is_job_pending() {
            if executed >= self.limits.max_pending_jobs {
                return Err(JsFailure::TooManyPendingJobs { executed });
            }
            if Instant::now() >= self.deadline.get() {
                return Err(JsFailure::Interrupted);
            }
            match self.runtime.execute_pending_job() {
                // false: the queue emptied between the check and the call.
                Ok(false) => break,
                Ok(true) => executed += 1,
                Err(exception) => {
                    // The job threw; extract the exception text from the
                    // context the engine handed back, then stop draining.
                    // (JobException is not nameable through the facade, so
                    // its public Context field is used right here.)
                    let text = exception
                        .0
                        .with(|ctx| coerce_to_string(&ctx, &ctx.catch()))
                        .unwrap_or_else(|| "error in microtask".to_owned());
                    return Err(self.exception_failure(&text));
                }
            }
        }
        Ok(())
    }

    /// Advances the deterministic logical clock one tick and returns the
    /// new tick count. `Date.now()` moves with it.
    pub fn logical_tick(&self) -> u64 {
        let next = self.clock.get().saturating_add(1);
        self.clock.set(next);
        next
    }

    /// The sink's retained lines (capped), in order: `console`/`velqu.log`
    /// output and, later, reactive diagnostics.
    pub fn diagnostics(&self) -> Vec<String> {
        self.diagnostics.borrow().clone()
    }

    /// Arms a fresh execution deadline for the next phase.
    fn arm_deadline(&self) {
        self.deadline
            .set(Instant::now() + self.limits.max_execution_time);
    }

    /// Installs the Velqu deterministic profile into the fresh context.
    fn install_profile(&self) -> Result<(), JsFailure> {
        let output_cap = self.limits.max_output_string_bytes;
        self.ctx
            .with(|ctx| {
                // The diagnostic sink's native boundary is a plain Rust
                // `String` (lifetime-free FromJs); formatting/coercion happens
                // in the JS prelude so no `Value<'js>` crosses a closure.
                let emit = {
                    let diagnostics = Rc::clone(&self.diagnostics);
                    move |line: String| {
                        let mut line = line;
                        if line.len() > output_cap {
                            line.truncate(output_cap);
                            line.push('…');
                        }
                        let mut sink = diagnostics.borrow_mut();
                        if sink.len() < MAX_DIAGNOSTIC_LINES {
                            sink.push(line);
                        }
                    }
                };
                let emit = Function::new(ctx.clone(), emit)
                    .expect("sink function allocatable under fresh limits");
                let prelude_console = r#"(() => {
                const emit = globalThis.__velquEmit;
                delete globalThis.__velquEmit;
                const send = (args) => {
                    try { emit(args.map(String).join(" ")); }
                    catch (error) { emit(String(error)); }
                };
                const forward = (...args) => send(args);
                globalThis.console = { log: forward, info: forward, warn: forward, error: forward };
                globalThis.velqu = { log: forward };
            })();"#;
                ctx.globals()
                    .set("__velquEmit", emit)
                    .expect("sink installed");
                // The prelude captures the sink and removes the global, so
                // scripts reach it only through console/velqu.
                ctx.eval::<(), _>(prelude_console)
                    .map_err(|_| self.take_exception(&ctx, "console prelude failed"))?;

                let random = {
                    let rng = Rc::clone(&self.rng);
                    move || -> f64 {
                        // xorshift32 → uniform in [0,1): deterministic per
                        // generation seed, identical across identical runs.
                        let mut state = rng.get();
                        state ^= state << 13;
                        state ^= state >> 17;
                        state ^= state << 5;
                        rng.set(state);
                        f64::from(state) / f64::from(u32::MAX)
                    }
                };
                let math: Object = ctx.globals().get("Math").expect("Math builtin present");
                math.set(
                    "random",
                    Function::new(ctx.clone(), random).expect("Math.random allocatable"),
                )
                .expect("Math.random virtualized");

                let clock = Rc::clone(&self.clock);
                // The tick is raw milliseconds since the logical epoch;
                // the JS shim adds the epoch constant itself.
                let now = move || -> f64 { (clock.get() * CLOCK_STEP_MS) as f64 };
                ctx.globals()
                    .set(
                        "__velquNow",
                        Function::new(ctx.clone(), now).expect("clock fn allocatable"),
                    )
                    .expect("clock installed");
                // The Date shim: wall-clock reads become logical-clock reads;
                // constructing with explicit components stays pure computation.
                // The native tick function is captured and removed, like the
                // diagnostic sink, so only the shim holds it.
                ctx.eval::<(), _>(
                    // NB: the subclass must not be named `Date` — a lexical
                // `class Date` binding would shadow the global during its
                // own TDZ and break the `OriginDate` read above it.
                r#"(() => {
                    const tick = globalThis.__velquNow;
                    delete globalThis.__velquNow;
                    const epoch = 946684800000;
                    const OriginDate = Date;
                    class LogicalDate extends OriginDate {
                        constructor(...args) {
                            if (args.length === 0) { super(epoch + tick()); }
                            else { super(...args); }
                        }
                        static now() { return epoch + tick(); }
                    }
                    globalThis.Date = LogicalDate;
                })();"#,
                )
                .map_err(|_| self.take_exception(&ctx, "date prelude failed"))?;
                // Dynamic codegen is refused at the handle level (ADR 0015
                // amendment): the Eval intrinsic must stay because the
                // host's own `Ctx::eval` rides the same engine hook, so
                // every script-reachable compiler handle is removed or
                // made to throw instead.
                //   - `eval` / `Function` globals: deleted (direct and
                //     indirect eval become ReferenceErrors).
                //   - `Function.prototype.constructor`: replaced with a
                //     throwing stub, so `(function(){}).constructor(...)`
                //     and class-constructor chains cannot rebuild Function.
                //   - dynamic `import(...)`: no module loader exists
                //     (loader feature off); the engine rejects it.
                ctx.eval::<(), _>(
                    r#"(() => {
                        const refused = () => { throw new TypeError("dynamic code generation is not supported"); };
                        // Capture before deleting: the identifiers resolve
                        // through the globals being removed.
                        const functionPrototype = Function.prototype;
                        Object.defineProperty(functionPrototype, "constructor", {
                            value: refused, writable: true, configurable: true,
                        });
                        delete globalThis.eval;
                        delete globalThis.Function;
                    })();"#,
                )
                .map_err(|_| self.take_exception(&ctx, "codegen refusal failed"))?;
                Ok(())
            })
            .map_err(|failure: JsFailure| failure)?;
        Ok(())
    }

    /// Drains a pending exception into a text-bearing failure (used where
    /// the failing call's own error is uninformative, like prelude evals).
    fn take_exception(&self, ctx: &Ctx<'_>, context: &str) -> JsFailure {
        if ctx.has_exception() {
            let value = ctx.catch();
            let text = coerce_to_string(ctx, &value).unwrap_or_else(|| format!("{value:?}"));
            JsFailure::Exception(format!("{context}: {text}"))
        } else {
            JsFailure::Exception(context.to_owned())
        }
    }

    /// Maps an rquickjs error to a classified failure, consuming any
    /// pending exception so the context stays usable afterwards.
    fn classify(&self, ctx: &Ctx<'_>, error: JsError) -> JsFailure {
        if !ctx.has_exception() {
            return JsFailure::Exception(error.to_string());
        }
        let value = ctx.catch();
        let text = coerce_to_string(ctx, &value).unwrap_or_else(|| format!("{value:?}"));
        self.exception_failure(&text)
    }

    /// Classifies coerced exception text.
    fn exception_failure(&self, text: &str) -> JsFailure {
        let capped = cap_string(text, self.limits.max_output_string_bytes);
        let lower = capped.to_ascii_lowercase();
        if lower.contains("memory") || lower.contains("allocation") {
            JsFailure::OutOfMemory
        } else if lower.contains("stack")
            && (lower.contains("overflow") || lower.contains("maximum call stack"))
        {
            JsFailure::StackOverflow
        } else if lower.contains("interrupted") {
            JsFailure::Interrupted
        } else {
            JsFailure::Exception(capped)
        }
    }
}

/// Deterministic seed for `Math.random`: derived from the generation so
/// identical documents produce identical sequences across instances, and
/// reloads (new generations) reseed.
fn seed_for(generation: u64) -> u32 {
    // Any nonzero odd seed; mix the generation so sequences differ per
    // reload while staying a pure function of it.
    (generation as u32).wrapping_mul(0x9E37_79B9) | 1
}

/// Coerces a JS value to its string form via `String(v)` (the ECMAScript
/// ToString the console itself would use).
fn coerce_to_string<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Option<String> {
    let string_ctor: Function = ctx.globals().get("String").ok()?;
    let coerced: String = string_ctor.call((value.clone(),)).ok()?;
    Some(coerced)
}

/// Caps a string to `max` bytes on a char boundary.
fn cap_string(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut capped = text[..end].to_owned();
    capped.push('…');
    capped
}

#[cfg(test)]
mod tests;
