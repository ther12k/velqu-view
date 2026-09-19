//! The reactive turn machine (M5c, ADR 0017).
//!
//! A reactive turn is **atomic across both reactive state and rendered
//! mutations**: committed Rust-owned state is snapshotted into a fresh
//! candidate JS object, the event's model write and handlers run, jobs
//! drain, all bindings reevaluate, outputs are normalized and diffed
//! into a semantic [`Mutation`] batch — and only then does anything
//! commit. Any failure (exception, deadline, job budget, invalid state,
//! oversized output, mutation limit) rolls the whole turn back: the
//! candidate is discarded, no mutation leaves the machine, the previous
//! UI stays.
//!
//! Executable units (initializers, handlers, binding evaluators) are
//! host-compiled **once per document generation** into parenthesized
//! arrow wrappers — host evaluation is the privileged path M5a.1
//! preserved; scripts still cannot reach `eval`/`Function`.

use rquickjs::{Function, IntoJs, Object, Value};

use crate::plan::{BindingKind, ReactiveDocument};
use crate::state::{self, ReactiveValue};
use crate::{JsFailure, JsLimits, ReactiveRuntime};

/// How many diagnostics the machine retains before dropping the oldest
/// (a spamming turn must not grow host memory through diagnostics).
const MAX_TURN_DIAGNOSTICS: usize = 64;

/// One plain entry value in a sanitized event payload.
#[derive(Debug, Clone, PartialEq)]
pub enum PayloadValue {
    /// `null`
    Null,
    /// A boolean.
    Bool(bool),
    /// A number.
    Number(f64),
    /// A string.
    Str(String),
}

/// An immutable event payload: ordered plain entries the view derives
/// from an M4 event. Handlers receive it frozen; mutating `event.value`
/// affects neither M4 state nor other handlers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EventPayload {
    /// `(key, value)` pairs in construction order.
    pub entries: Vec<(String, PayloadValue)>,
}

impl EventPayload {
    /// Builds a payload, enforcing `max_bytes` (the sum of key and
    /// string-value bytes) before anything reaches JavaScript.
    pub fn new(
        entries: Vec<(String, PayloadValue)>,
        max_bytes: usize,
    ) -> Result<Self, PayloadTooLarge> {
        let size: usize = entries
            .iter()
            .map(|(key, value)| match value {
                PayloadValue::Str(text) => key.len() + text.len(),
                _ => key.len(),
            })
            .sum();
        if size > max_bytes {
            return Err(PayloadTooLarge {
                size,
                max: max_bytes,
            });
        }
        Ok(Self { entries })
    }
}

/// An event payload exceeded the byte budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayloadTooLarge {
    /// Actual size in bytes.
    pub size: usize,
    /// The configured maximum.
    pub max: usize,
}

/// One validated, semantic UI mutation. The target is the plan binding
/// index (generation-scoped through the machine); the *renderer*
/// decides what the mutation invalidates — QuickJS never does.
#[derive(Debug, Clone, PartialEq)]
pub struct Mutation {
    /// The plan binding that produced the mutation.
    pub binding: usize,
    /// What to apply.
    pub kind: MutationKind,
}

/// The semantic mutation vocabulary (M5c).
#[derive(Debug, Clone, PartialEq)]
pub enum MutationKind {
    /// The element's text content becomes the string.
    SetText(String),
    /// The element is shown/hidden (v0: inline `display:none`, one
    /// Taffy pass at most).
    SetVisible(bool),
    /// The element's `class` attribute becomes the string.
    SetClass(String),
    /// The element's `style` attribute becomes the string.
    SetStyle(String),
    /// A control's value becomes the string — applied silently, never
    /// synthesizing a user `ValueChanged`.
    SetControlValue(String),
    /// A control's disabled state.
    SetControlDisabled(bool),
    /// A control's checked state (outside the M4c1 control profile —
    /// applying it is a diagnostic, not a silent no-op).
    SetControlChecked(bool),
}

/// The outcome of preparing one turn.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnOutcome {
    /// Nothing reactive matched: no JS ran, nothing changed.
    NoChange,
    /// The turn is validated and ready; [`ReactiveMachine::commit`]
    /// applies it, anything else discards it.
    Prepared(PendingTurn),
    /// The turn failed; state and UI are untouched. The string is the
    /// diagnostic.
    RolledBack(String),
}

/// A validated, uncommitted turn.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingTurn {
    /// The candidate state, extracted and constrained.
    pub state: ReactiveValue,
    /// The normalized per-binding outputs (index-aligned with the
    /// plan's bindings).
    pub outputs: Vec<ReactiveValue>,
    /// The diffed mutation batch.
    pub mutations: Vec<Mutation>,
    /// Handler indices that ran (their `.once` registration commits
    /// with the turn).
    pub fired_handlers: Vec<usize>,
}

/// The reactive machine: one per document generation.
pub struct ReactiveMachine {
    generation: u64,
    limits: JsLimits,
    runtime: ReactiveRuntime,
    /// Committed state (the single shared v0 namespace; see ADR 0017).
    state: ReactiveValue,
    /// Last committed output per plan binding.
    applied: Vec<Option<ReactiveValue>>,
    /// Handler indices consumed by `.once` (committed turns only).
    fired_once: std::collections::HashSet<usize>,
    /// The binding kinds, index-aligned with the plan.
    binding_kinds: Vec<BindingKind>,
    /// Handler modifier sets, index-aligned with the plan's events.
    handler_modifiers: Vec<Vec<String>>,
    /// Poisoned units: indices whose compilation failed; evaluating or
    /// invoking one rolls the turn back deterministically.
    poisoned_bindings: std::collections::HashMap<usize, String>,
    poisoned_handlers: std::collections::HashMap<usize, String>,
    /// Initializers that failed during construction (M6b observability).
    initializer_failures: u32,
    /// Bounded turn diagnostics, newest last.
    diagnostics: Vec<String>,
}

impl ReactiveMachine {
    /// Compiles the plan's executable units under `limits` and runs the
    /// scope initializers (document order, merged into the shared state
    /// namespace). Failing initializers produce diagnostics, not a
    /// failed machine.
    pub fn new<N>(
        generation: u64,
        limits: JsLimits,
        plan: &ReactiveDocument<N>,
    ) -> Result<(Self, Vec<Mutation>), JsFailure> {
        let runtime = ReactiveRuntime::new(generation, limits)?;
        let mut machine = Self {
            generation,
            limits,
            runtime,
            state: ReactiveValue::Object(Vec::new()),
            applied: plan.bindings.iter().map(|_| None).collect(),
            fired_once: std::collections::HashSet::new(),
            binding_kinds: plan.bindings.iter().map(|b| b.kind.clone()).collect(),
            handler_modifiers: plan
                .events
                .iter()
                .map(|e| e.handler.modifiers.clone())
                .collect(),
            poisoned_bindings: std::collections::HashMap::new(),
            poisoned_handlers: std::collections::HashMap::new(),
            initializer_failures: 0,
            diagnostics: Vec::new(),
        };
        machine.compile_units(plan);
        machine.run_initializers(plan);
        // Turn zero: evaluate the bindings once against the initial
        // state, commit it, and hand the host the initial mutations so
        // the first render shows initial values.
        let initial = match machine.prepare_inner(None, &[], None, true) {
            TurnOutcome::Prepared(pending) => {
                let mutations = pending.mutations.clone();
                machine.commit(pending);
                mutations
            }
            _ => Vec::new(),
        };
        Ok((machine, initial))
    }

    /// The document generation this machine belongs to.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The committed state (plain data).
    pub fn state(&self) -> &ReactiveValue {
        &self.state
    }

    /// Bounded turn diagnostics, oldest first.
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    /// How many scope initializers failed (threw, returned non-plain
    /// data, or failed state capture) during construction. M5c
    /// semantics keep these as diagnostics-not-failures for a normal
    /// load; reload acceptance (M6b) reads the counter to classify a
    /// candidate as unpublishable.
    pub fn initializer_failures(&self) -> u32 {
        self.initializer_failures
    }

    /// How many executable units (bindings/handlers) failed to compile
    /// and are poisoned. Turn-time semantics are frozen (a poisoned
    /// unit rolls back its turns); reload acceptance reads the count.
    pub fn poisoned_units(&self) -> usize {
        self.poisoned_bindings.len() + self.poisoned_handlers.len()
    }

    /// Records a host-side turn diagnostic (batch rejection, invalid
    /// model paths) into the same bounded stream.
    pub fn record_host_diagnostic(&mut self, message: &str) {
        self.push_diagnostic(message.to_owned());
    }

    /// Prepares one reactive turn. `payload` is the sanitized event
    /// (already byte-budgeted), `handlers` the plan event indices to
    /// invoke in order (target → ancestors), `model_write` an optional
    /// `(state path, value)` applied **before** the handlers (the
    /// vx-model ordering contract).
    ///
    /// Preparing never mutates committed state — a caller that cannot
    /// apply the batch simply drops the [`PendingTurn`].
    pub fn prepare(
        &mut self,
        payload: Option<&EventPayload>,
        handlers: &[usize],
        model_write: Option<(&str, &str)>,
    ) -> TurnOutcome {
        self.prepare_inner(payload, handlers, model_write, false)
    }

    /// The initial evaluation (machine construction) runs the bindings
    /// even with no event: first render shows initial state.
    fn prepare_inner(
        &mut self,
        payload: Option<&EventPayload>,
        handlers: &[usize],
        model_write: Option<(&str, &str)>,
        force: bool,
    ) -> TurnOutcome {
        if !force && handlers.is_empty() && model_write.is_none() {
            return TurnOutcome::NoChange;
        }
        if let Some((size, max)) = self.too_large_hint() {
            return TurnOutcome::RolledBack(format!(
                "state exceeds the {max}-byte output budget by {size}"
            ));
        }
        // Runtime-level calls (drain_jobs) cannot run inside a context
        // `with` scope, so the turn runs in two phases with the
        // candidate anchored in the JS heap between them.
        let context = self.runtime.context().clone();

        // Phase 1: snapshot → model write → handlers. The candidate
        // state object is anchored at __velquUnits.state.
        self.runtime.arm();
        let handler_phase = context.with(|ctx| -> Result<Vec<usize>, String> {
            let core: Object = ctx
                .globals()
                .get("__velquCore")
                .map_err(|_| "profile core helpers missing".to_owned())?;
            let units: Object = ctx
                .globals()
                .get("__velquUnits")
                .map_err(|_| "compiled units missing".to_owned())?;
            let mut candidate = self.state.clone();
            if let Some((path, value)) = model_write {
                candidate.set_path(path, ReactiveValue::String(value.to_owned()));
            }
            let state_obj: Object = candidate
                .clone()
                .into_js(&ctx)
                .ok()
                .and_then(|value| value.into_object())
                .ok_or_else(|| "state root must be an object".to_owned())?;
            let event_obj: Object = match payload {
                None => {
                    Object::new(ctx.clone()).map_err(|_| "payload allocation failed".to_owned())?
                }
                Some(payload) => {
                    let object = Object::new(ctx.clone())
                        .map_err(|_| "payload allocation failed".to_owned())?;
                    for (key, value) in &payload.entries {
                        let value = match value {
                            PayloadValue::Null => Value::new_null(ctx.clone()),
                            PayloadValue::Bool(v) => Value::new_bool(ctx.clone(), *v),
                            PayloadValue::Number(v) => Value::new_number(ctx.clone(), *v),
                            PayloadValue::Str(v) => v
                                .as_str()
                                .into_js(&ctx)
                                .map_err(|_| "payload failed".to_owned())?,
                        };
                        object
                            .set(key.as_str(), value)
                            .map_err(|_| "payload failed")?;
                    }
                    let freeze: Function =
                        core.get("freeze").map_err(|_| "freeze helper missing")?;
                    freeze
                        .call::<_, Object>((object,))
                        .map_err(|_| "freeze failed".to_owned())?
                }
            };
            // Anchor the candidate; phase 2 re-reads it after the drain.
            units
                .set("state", state_obj.clone())
                .map_err(|_| "anchor failed".to_owned())?;
            units
                .set("event", event_obj.clone())
                .map_err(|_| "anchor failed".to_owned())?;

            let mut ran: Vec<usize> = Vec::new();
            for &index in handlers {
                if self.fired_once.contains(&index) {
                    continue;
                }
                if let Some(reason) = self.poisoned_handlers.get(&index) {
                    return Err(format!("handler failed to compile: {reason}"));
                }
                let handler_units: Object = units
                    .get("handlers")
                    .map_err(|_| "handler units missing".to_owned())?;
                let function: Function = handler_units
                    .get(index.to_string())
                    .map_err(|_| format!("handler {index} missing"))?;
                function
                    .call::<_, Value>((state_obj.clone(), event_obj.clone()))
                    .map_err(|error| self.call_failure(&ctx, &format!("handler {index}"), error))?;
                ran.push(index);
                if self
                    .handler_modifiers
                    .get(index)
                    .is_some_and(|modifiers| modifiers.iter().any(|m| m == "stop"))
                {
                    break;
                }
            }
            Ok(ran)
        });

        let ran = match handler_phase {
            Ok(ran) => ran,
            Err(message) => {
                self.push_diagnostic(message.clone());
                return TurnOutcome::RolledBack(message);
            }
        };
        // Every handler was skipped (`.once` consumed) and no model write:
        // nothing ran, nothing can differ — a turn did not happen.
        if !force && ran.is_empty() && model_write.is_none() {
            return TurnOutcome::NoChange;
        }

        // Phase 1.5: bounded microtask drain (runtime-level).
        if let Err(failure) = self.runtime.drain_jobs() {
            let message = format!("turn jobs: {failure}");
            self.push_diagnostic(message.clone());
            return TurnOutcome::RolledBack(message);
        }

        // Phase 2: capture the candidate and evaluate every binding.
        self.runtime.arm();
        let binding_phase = context.with(
            |ctx| -> Result<(ReactiveValue, Vec<ReactiveValue>), String> {
                let core: Object = ctx
                    .globals()
                    .get("__velquCore")
                    .map_err(|_| "profile core helpers missing".to_owned())?;
                let is_plain: Function = core.get("isPlain").map_err(|_| "isPlain missing")?;
                let string_of: Function =
                    core.get("string").map_err(|_| "string helper missing")?;
                let truthy: Function = core.get("truthy").map_err(|_| "truthy helper missing")?;
                let units: Object = ctx
                    .globals()
                    .get("__velquUnits")
                    .map_err(|_| "compiled units missing".to_owned())?;
                let state_obj: Object = units
                    .get("state")
                    .map_err(|_| "candidate state missing".to_owned())?;

                let captured = state::capture(
                    &ctx,
                    state_obj.as_value(),
                    &is_plain,
                    self.limits.max_output_string_bytes,
                )
                .map_err(|error| format!("candidate state: {error}"))?;

                let binding_units: Object = units
                    .get("bindings")
                    .map_err(|_| "binding units missing".to_owned())?;
                let mut outputs = Vec::with_capacity(self.binding_kinds.len());
                for index in 0..self.binding_kinds.len() {
                    if let Some(reason) = self.poisoned_bindings.get(&index) {
                        return Err(format!("binding failed to compile: {reason}"));
                    }
                    let function: Function = binding_units
                        .get(index.to_string())
                        .map_err(|_| format!("binding {index} missing"))?;
                    let value: Value =
                        function
                            .call::<_, Value>((state_obj.clone(),))
                            .map_err(|error| {
                                self.call_failure(&ctx, &format!("binding {index}"), error)
                            })?;
                    let normalized = match self.binding_kinds[index] {
                        BindingKind::Show | BindingKind::Disabled | BindingKind::Checked => {
                            let truth: bool = truthy
                                .call((value.clone(),))
                                .map_err(|_| "truthiness failed".to_owned())?;
                            ReactiveValue::Bool(truth)
                        }
                        _ => {
                            let text: String = string_of
                                .call((value.clone(),))
                                .map_err(|_| "string conversion failed".to_owned())?;
                            if text.len() > self.limits.max_output_string_bytes {
                                return Err(format!(
                                    "binding output of {} bytes exceeds the {}-byte budget",
                                    text.len(),
                                    self.limits.max_output_string_bytes
                                ));
                            }
                            ReactiveValue::String(text)
                        }
                    };
                    outputs.push(normalized);
                }
                Ok((captured, outputs))
            },
        );

        let (candidate, outputs, ran) = match binding_phase {
            Ok(value) => (value.0, value.1, ran),
            Err(message) => {
                self.push_diagnostic(message.clone());
                return TurnOutcome::RolledBack(message);
            }
        };

        // 7. Diff into a semantic batch, bounded by the mutation budget.
        let mut mutations = Vec::new();
        for (index, output) in outputs.iter().enumerate() {
            if self.applied.get(index) != Some(&Some(output.clone())) {
                mutations.push(Mutation {
                    binding: index,
                    kind: match (&self.binding_kinds[index], output) {
                        (BindingKind::Text, ReactiveValue::String(text)) => {
                            MutationKind::SetText(text.clone())
                        }
                        (BindingKind::Show, ReactiveValue::Bool(value)) => {
                            MutationKind::SetVisible(*value)
                        }
                        (BindingKind::Class, ReactiveValue::String(text)) => {
                            MutationKind::SetClass(text.clone())
                        }
                        (BindingKind::Style, ReactiveValue::String(text)) => {
                            MutationKind::SetStyle(text.clone())
                        }
                        (BindingKind::Value | BindingKind::Model, ReactiveValue::String(text)) => {
                            MutationKind::SetControlValue(text.clone())
                        }
                        (BindingKind::Disabled, ReactiveValue::Bool(value)) => {
                            MutationKind::SetControlDisabled(*value)
                        }
                        (BindingKind::Checked, ReactiveValue::Bool(value)) => {
                            MutationKind::SetControlChecked(*value)
                        }
                        // Normalization guarantees these pairings; the
                        // fall-through keeps the diff total.
                        (_, other) => MutationKind::SetText(format!("{other:?}")),
                    },
                });
            }
        }
        if mutations.len() > self.limits.max_mutations_per_turn {
            let message = format!(
                "turn requested {} mutations; the budget is {}",
                mutations.len(),
                self.limits.max_mutations_per_turn
            );
            self.push_diagnostic(message.clone());
            return TurnOutcome::RolledBack(message);
        }

        TurnOutcome::Prepared(PendingTurn {
            state: candidate,
            outputs,
            mutations,
            fired_handlers: ran,
        })
    }

    /// Commits a prepared turn: candidate becomes committed state, the
    /// outputs become the applied baseline, and `.once` handlers
    /// register. This is the only state-mutating step.
    pub fn commit(&mut self, pending: PendingTurn) {
        self.state = pending.state;
        self.applied = pending.outputs.into_iter().map(Some).collect();
        for index in pending.fired_handlers {
            if self
                .handler_modifiers
                .get(index)
                .is_some_and(|modifiers| modifiers.iter().any(|m| m == "once"))
            {
                self.fired_once.insert(index);
            }
        }
    }

    /// A diagnostic surface helper: the committed state's total byte
    /// size, if it already exceeds the output budget (a backstop for
    /// states grown across many turns).
    fn too_large_hint(&self) -> Option<(usize, usize)> {
        fn size(value: &ReactiveValue) -> usize {
            match value {
                ReactiveValue::String(text) => text.len(),
                ReactiveValue::Array(items) => items.iter().map(size).sum(),
                ReactiveValue::Object(entries) => entries
                    .iter()
                    .map(|(key, value)| key.len() + size(value))
                    .sum(),
                _ => 0,
            }
        }
        let total = size(&self.state);
        (total > self.limits.max_output_string_bytes)
            .then_some((total, self.limits.max_output_string_bytes))
    }

    /// Host-compiles every unit once. Wrappers are whole-program
    /// parentheses, so a "binding expression" cannot smuggle statements
    /// past its arrow's expression body; handler bodies are statements
    /// by contract. Each wrapper obeys the source budget.
    fn compile_units<N>(&mut self, plan: &ReactiveDocument<N>) {
        type UnitErrors = Vec<(usize, String)>;
        let context = self.runtime.context().clone();
        let outcome: (UnitErrors, UnitErrors, UnitErrors) = context.with(|ctx| {
            let core: Object = ctx
                .globals()
                .get::<_, Object>("__velquCore")
                .expect("profile installed");
            let units: Object = ctx
                .globals()
                .get("__velquUnits")
                .expect("unit holder installed");
            let init_store: Object = units.get("init").expect("init store");
            let bind_store: Object = units.get("bindings").expect("bind store");
            let handler_store: Object = units.get("handlers").expect("handler store");
            let _ = &core;

            let max_source = self.limits.max_source_bytes;
            let mut init_errors: Vec<(usize, String)> = Vec::new();
            let mut binding_errors: Vec<(usize, String)> = Vec::new();
            let mut handler_errors: Vec<(usize, String)> = Vec::new();
            for (index, scope) in plan.scopes.iter().enumerate() {
                let wrapper = scope_wrapper(&scope.initializer_source);
                if let Err(reason) = compile_unit(
                    &self.runtime,
                    &ctx,
                    &init_store,
                    index.to_string(),
                    wrapper,
                    max_source,
                ) {
                    init_errors.push((index, reason));
                }
            }
            for (index, binding) in plan.bindings.iter().enumerate() {
                let wrapper = scope_wrapper(&binding.expression_source);
                if let Err(reason) = compile_unit(
                    &self.runtime,
                    &ctx,
                    &bind_store,
                    index.to_string(),
                    wrapper,
                    max_source,
                ) {
                    binding_errors.push((index, reason));
                }
            }
            for (index, event) in plan.events.iter().enumerate() {
                let wrapper = handler_wrapper(&event.handler_source);
                if let Err(reason) = compile_unit(
                    &self.runtime,
                    &ctx,
                    &handler_store,
                    index.to_string(),
                    wrapper,
                    max_source,
                ) {
                    handler_errors.push((index, reason));
                }
            }
            (init_errors, binding_errors, handler_errors)
        });
        // Applied here, after the `with` scope: poisoning is &mut self.
        let (init_errors, binding_errors, handler_errors) = outcome;
        for (_index, reason) in init_errors {
            self.push_diagnostic(format!("scope initializer failed to compile: {reason}"));
        }
        for (index, reason) in binding_errors {
            self.poisoned_bindings.insert(index, reason);
        }
        for (index, reason) in handler_errors {
            self.poisoned_handlers.insert(index, reason);
        }
    }

    /// Runs the scope initializers in document order against the shared
    /// namespace, merging each plain-object result.
    fn run_initializers<N>(&mut self, plan: &ReactiveDocument<N>) {
        if plan.scopes.is_empty() {
            return;
        }
        let context = self.runtime.context().clone();
        let mut failures: u32 = 0;
        let outcome = context.with(|ctx| {
            let core: Object = ctx
                .globals()
                .get("__velquCore")
                .map_err(|_| "profile core helpers missing".to_owned())?;
            let units: Object = ctx
                .globals()
                .get("__velquUnits")
                .map_err(|_| "compiled units missing".to_owned())?;
            let init_store: Object = units
                .get("init")
                .map_err(|_| "init units missing".to_owned())?;
            let is_plain: Function = core.get("isPlain").map_err(|_| "isPlain missing")?;
            let object_ctor: Function = ctx
                .globals()
                .get("Object")
                .map_err(|_| "Object missing".to_owned())?;
            let assign: Function = object_ctor
                .get("assign")
                .map_err(|_| "Object.assign missing".to_owned())?;

            let mut state_obj = Object::new(ctx.clone()).map_err(|e| format!("{e}"))?;
            for index in 0..plan.scopes.len() {
                let function: Function = init_store
                    .get(index.to_string())
                    .map_err(|_| format!("initializer {index} missing"))?;
                let result: Value = match function.call::<_, Value>((state_obj.clone(),)) {
                    Ok(value) => value,
                    Err(error) => {
                        failures += 1;
                        return Err(self.call_failure(
                            &ctx,
                            &format!("initializer {index}"),
                            error,
                        ));
                    }
                };
                let plain: bool = is_plain
                    .call((result.clone(),))
                    .map_err(|_| "isPlain failed".to_owned())?;
                if !plain {
                    failures += 1;
                    self.push_diagnostic(format!(
                        "scope initializer {index} returned a non-plain object; skipped"
                    ));
                    continue;
                }
                state_obj = assign
                    .call((state_obj, result))
                    .map_err(|_| "merge failed".to_owned())?;
            }
            match state::capture(
                &ctx,
                &state_obj.into_value(),
                &is_plain,
                self.limits.max_output_string_bytes,
            ) {
                Ok(captured) => Ok(captured),
                Err(error) => {
                    failures += 1;
                    Err(format!("initial state: {error}"))
                }
            }
        });
        self.initializer_failures += failures;
        match outcome {
            Ok(state) => self.state = state,
            Err(message) => {
                // Initial-state failure keeps the empty namespace; the
                // first turn starts from it.
                self.push_diagnostic(message);
            }
        }
    }

    fn call_failure(&self, ctx: &rquickjs::Ctx<'_>, unit: &str, error: rquickjs::Error) -> String {
        if ctx.has_exception() {
            let value = ctx.catch();
            let text = value
                .as_string()
                .and_then(|text| text.to_string().ok())
                .unwrap_or_else(|| format!("{value:?}"));
            format!("{unit} threw: {text}")
        } else {
            format!("{unit} failed: {error}")
        }
    }

    fn push_diagnostic(&mut self, message: String) {
        if self.diagnostics.len() >= MAX_TURN_DIAGNOSTICS {
            self.diagnostics.remove(0);
        }
        self.diagnostics.push(message);
    }
}

/// An expression wrapper: the state snapshot is `with`-scoped so bare
/// identifiers read *and write* state properties (Alpine-style authoring);
/// a whole-program paren keeps this a single expression. Sloppy mode is
/// required for `with` — see ADR 0017 for the trade-offs (writes to keys
/// absent from state fall through to the global and are not captured).
fn scope_wrapper(source: &str) -> String {
    format!(
        "( (__velquState, __velquEvent) => {{ with (__velquState) {{ return ( {} ); }} }} )",
        source
    )
}

/// A handler wrapper: statements by contract, same `with` scoping.
fn handler_wrapper(source: &str) -> String {
    format!(
        "( (__velquState, __velquEvent) => {{ with (__velquState) {{ {} }} }} )",
        source
    )
}

/// Host-compiles one unit into `store[key]`: a whole-program
/// parenthesized wrapper (an expression cannot smuggle statements past
/// its arrow body; handler bodies are statements by contract), checked
/// against the source budget. A syntax failure poisons the unit — turns
/// touching it roll back with the reason.
fn compile_unit<'js>(
    runtime: &ReactiveRuntime,
    ctx: &rquickjs::Ctx<'js>,
    store: &Object<'js>,
    key: String,
    wrapper: String,
    max_source: usize,
) -> Result<(), String> {
    if wrapper.len() > max_source {
        return Err(format!(
            "generated source of {} bytes exceeds the {}-byte budget",
            wrapper.len(),
            max_source
        ));
    }
    match runtime.eval_function(ctx, &wrapper) {
        Ok(function) => {
            store.set(key, function).expect("unit stored");
            Ok(())
        }
        Err(failure) => Err(format!("{failure}")),
    }
}

#[cfg(test)]
mod tests;
