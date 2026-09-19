//! The inspector trace (M6a, ADR 0020).
//!
//! The inspector **reads recorded outcomes; it never reruns work to
//! discover what happened.** Recording happens inside the view as it
//! works — events observed by a pumped batch, reactive turn attempts
//! (attempted vs committed mutations, rollbacks), invalidation
//! requests with their causes, and completed render outcomes — into a
//! byte- and count-bounded buffer. Snapshots read cached results only;
//! when there is no fresh cached layout they report that, rather than
//! silently producing one.
//!
//! Determinism discipline: sequence ids, causes, outcomes, mutation
//! counts, and pass deltas are deterministic and test-comparable;
//! durations are host-side `Instant` measurements that never affect
//! ordering. Inspecting never pumps, drains, renders, or advances the
//! JS logical clock.

use std::collections::VecDeque;
use std::time::Duration;

/// Retention and capture limits for the trace (M6a, ADR 0020).
///
/// Records are bounded three ways: how many, how much they may retain
/// in total, and how large any single record (and any embedded value
/// preview) may be. Limits apply while constructing records — a state
/// dump is never fully materialized just to be truncated afterwards.
///
/// Trace overflow may discard observational records; it must never
/// discard application events — the caller-owned batch (ADR 0019)
/// stays authoritative regardless of inspector retention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectorLimits {
    /// Maximum retained trace records.
    pub max_records: usize,
    /// Maximum total retained bytes (estimated; evicts oldest first).
    pub max_retained_bytes: usize,
    /// Maximum size of any single record; larger payloads truncate.
    pub max_record_bytes: usize,
    /// Maximum length of any embedded value preview.
    pub max_preview_bytes: usize,
    /// Record user-controlled text (input values, mutation payloads,
    /// error details)? Default is metadata only: target identity,
    /// operation, and lengths.
    pub capture_values: bool,
}

impl Default for InspectorLimits {
    fn default() -> Self {
        Self {
            max_records: 1024,
            max_retained_bytes: 256 * 1024,
            max_record_bytes: 2 * 1024,
            max_preview_bytes: 120,
            capture_values: false,
        }
    }
}

/// A rollback point for the trace (speculative staging, M6b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TraceCheckpoint {
    len: usize,
    seq: u64,
    appended: u64,
    evicted: u64,
    truncated: u64,
}

/// One trace record. `seq` is monotonically increasing across the
/// view's lifetime and never renumbers; evicted records leave visible
/// gaps in the retained window.
#[derive(Debug, Clone)]
pub struct TraceRecord {
    /// Monotonic record id (stable across evictions).
    pub seq: u64,
    /// The record body.
    pub kind: TraceRecordKind,
}

/// The four record families, linked by `seq` where a real causal
/// relationship exists — a rigid pipeline is deliberately not assumed
/// (events with no handler, initialization turns with no event,
/// several turns per frame, presentation updates without any turn).
/// Reloads (M6b) append their own record: published or rejected, with
/// generations — a rejected candidate's speculative work never enters
/// the trace because it ran on the candidate, not the application.
#[derive(Debug, Clone)]
pub enum TraceRecordKind {
    /// An event the pump observed in a caller-owned batch.
    Event(EventRecord),
    /// One reactive turn attempt (deliberate replay of a batch is a
    /// new attempt, never an overwrite).
    Turn(TurnRecord),
    /// Work Velqu **requested** (with causes); distinct from what a
    /// completed render actually did.
    Invalidation(InvalidationRecord),
    /// One completed render's actual work (the only source of
    /// pass-count deltas).
    Render(RenderRecord),
    /// One reload attempt's outcome (M6b, ADR 0021).
    Reload(ReloadTraceRecord),
}

/// One reload attempt in the trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadTraceRecord {
    /// Monotonic attempt id.
    pub attempt: u64,
    /// "document" or "stylesheets".
    pub kind: &'static str,
    /// Whether the attempt published.
    pub published: bool,
    /// Active generation before the attempt.
    pub generation_before: u64,
    /// Active generation after the attempt (unchanged on rejection).
    pub generation_after: u64,
    /// Rejecting stage label (empty on publication).
    pub stage: &'static str,
}

/// An observed event: metadata by default, never captured user text
/// unless the limits opt in.
#[derive(Debug, Clone)]
pub struct EventRecord {
    /// Generation of the document the pump was running for.
    pub generation: u64,
    /// The event's handle generation when it carries one — a stale
    /// batch pumped after a reload shows the mismatch here.
    pub event_generation: Option<u64>,
    /// `click`, `input`, `focus`, …
    pub kind: &'static str,
    /// The target's HTML id, if any (descriptive, not identity).
    pub target_id: Option<String>,
    /// Value length in bytes for value-carrying events (metadata).
    pub value_len: Option<usize>,
    /// Value preview, only when `capture_values` opted in.
    pub value_preview: Option<String>,
}

/// The outcome of one reactive turn attempt.
#[derive(Debug, Clone)]
pub struct TurnRecord {
    /// Document generation the turn ran against.
    pub generation: u64,
    /// The triggering `EventRecord` seq; `None` for initialization.
    pub trigger: Option<u64>,
    /// Mutations the turn proposed (before validation).
    pub attempted_mutations: usize,
    /// Mutation kinds proposed, bounded, in order.
    pub mutation_kinds: Vec<&'static str>,
    /// Committed mutations, validation outcome, rollback reason.
    pub outcome: TurnOutcomeRecord,
    /// Reactive state revision before the attempt.
    pub state_revision_before: u64,
    /// Reactive state revision after the attempt (unchanged on
    /// rejection/rollback).
    pub state_revision_after: u64,
    /// Host-measured JS+turn duration (never affects ordering).
    pub duration: Option<Duration>,
}

/// What became of a turn's proposed mutation batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcomeRecord {
    /// The batch validated and committed.
    Committed {
        /// How many mutations were applied.
        count: usize,
    },
    /// The host rejected the batch (a target failed validation): state
    /// and UI untouched.
    Rejected,
    /// JS failed (exception, budget, deadline): candidate state and
    /// the batch were rolled back.
    RolledBack,
    /// Nothing reactive matched: no turn ran for the event.
    NoMatch,
}

/// Work requested of the renderer, with causes. When several causes
/// coalesce into one frame the record keeps a bounded set (plus a
/// truncation count) instead of letting the last cause win.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidationRecord {
    /// Document generation.
    pub generation: u64,
    /// What kind of work was requested.
    pub classification: InvalidationClass,
    /// Bounded cause summaries (`"reactive SetText"`, `"hover"`,
    /// `"stylesheet app.css"`, …).
    pub causes: Vec<String>,
    /// Causes dropped by the bound.
    pub truncated_causes: usize,
}

/// The invalidation classification (ADR 0018).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidationClass {
    /// Presentation-only: cached geometry, re-emitted pixels.
    Presentation,
    /// Structural: a full layout pass.
    Structural,
}

/// One completed render: the **actual** deltas, associated with the
/// invalidation records it settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRecord {
    /// Document generation.
    pub generation: u64,
    /// The frame index this render produced.
    pub frame_index: u64,
    /// Taffy passes this render ran (completed work, not requests).
    pub layout_pass_delta: u64,
    /// Presentation repaints this render ran.
    pub repaint_delta: u64,
    /// `InvalidationRecord` seqs this render settled (bounded).
    pub settled: Vec<u64>,
    /// Settled invalidation records dropped by the link bound.
    pub settled_truncated: usize,
}

/// Retention state of the trace window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TraceSummary {
    /// Total records ever appended.
    pub appended: u64,
    /// Seq of the oldest retained record.
    pub first_retained: u64,
    /// Retained record count.
    pub retained: usize,
    /// Records evicted by the bounds (total, over the view lifetime).
    pub evicted: u64,
    /// Payloads truncated by `max_record_bytes`/`max_preview_bytes`.
    pub truncated: u64,
}

/// The bounded trace store. Owned by the view; every mutation is a
/// recording hook, every read is a copy — the inspector never holds
/// references into live application state.
#[derive(Debug, Clone)]
pub(crate) struct Trace {
    limits: InspectorLimits,
    seq: u64,
    appended: u64,
    evicted: u64,
    truncated: u64,
    bytes: usize,
    records: VecDeque<TraceRecord>,
}

impl Trace {
    pub(crate) fn new(limits: InspectorLimits) -> Self {
        Self {
            limits,
            seq: 0,
            appended: 0,
            evicted: 0,
            truncated: 0,
            bytes: 0,
            records: VecDeque::new(),
        }
    }

    pub(crate) fn limits(&self) -> &InspectorLimits {
        &self.limits
    }

    pub(crate) fn set_limits(&mut self, limits: InspectorLimits) {
        self.limits = limits;
        self.enforce();
    }

    /// Appends one record, assigning its seq. Returns the seq (tests
    /// and the view link causal records with it).
    pub(crate) fn push(&mut self, mut kind: TraceRecordKind) -> u64 {
        self.seq += 1;
        self.appended += 1;
        let seq = self.seq;
        if self.record_bytes(&kind) > self.limits.max_record_bytes {
            self.shrink_record(&mut kind);
            self.truncated += 1;
        }
        let size = self.record_bytes(&kind);
        self.bytes += size;
        self.records.push_back(TraceRecord { seq, kind });
        self.enforce();
        seq
    }

    /// Estimated retained size of a record.
    fn record_bytes(&self, kind: &TraceRecordKind) -> usize {
        fn text(list: &[String]) -> usize {
            list.iter().map(|c| c.len()).sum::<usize>() + list.len() * 16
        }
        const BASE: usize = 96;
        match kind {
            TraceRecordKind::Event(event) => {
                BASE + event.kind.len()
                    + event.target_id.as_ref().map_or(0, String::len)
                    + event.value_preview.as_ref().map_or(0, String::len)
            }
            TraceRecordKind::Turn(turn) => {
                BASE + turn.mutation_kinds.iter().map(|k| k.len()).sum::<usize>()
                    + turn.outcome.reason_bytes()
            }
            TraceRecordKind::Invalidation(invalidation) => BASE + text(&invalidation.causes),
            TraceRecordKind::Render(render) => BASE + render.settled.len() * 8,
            TraceRecordKind::Reload(_) => BASE,
        }
    }

    /// Truncates oversized payloads in place (causes and previews are
    /// cut to the preview bound; kinds lists drop their tail).
    fn shrink_record(&mut self, kind: &mut TraceRecordKind) {
        let preview = self.limits.max_preview_bytes;
        match kind {
            TraceRecordKind::Event(event) => {
                if let Some(value) = &mut event.value_preview {
                    truncate_utf8(value, preview);
                }
            }
            TraceRecordKind::Turn(turn) => {
                if turn.mutation_kinds.len() > 16 {
                    turn.mutation_kinds.truncate(16);
                }
            }
            TraceRecordKind::Invalidation(invalidation) => {
                for cause in &mut invalidation.causes {
                    truncate_utf8(cause, preview);
                }
            }
            TraceRecordKind::Render(_) => {}
            TraceRecordKind::Reload(_) => {}
        }
    }

    /// Evicts oldest records until both bounds hold.
    fn enforce(&mut self) {
        while self.records.len() > self.limits.max_records
            || self.bytes > self.limits.max_retained_bytes
        {
            match self.records.pop_front() {
                Some(record) => {
                    let size = self.record_bytes(&record.kind);
                    self.bytes = self.bytes.saturating_sub(size);
                    self.evicted += 1;
                }
                None => break,
            }
        }
    }

    pub(crate) fn summary(&self) -> TraceSummary {
        TraceSummary {
            appended: self.appended,
            first_retained: self.records.front().map_or(self.seq + 1, |r| r.seq),
            retained: self.records.len(),
            evicted: self.evicted,
            truncated: self.truncated,
        }
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = &TraceRecord> {
        self.records.iter()
    }

    pub(crate) fn clear(&mut self) {
        self.records.clear();
        self.bytes = 0;
    }

    /// Drops records appended after `len` (a rejected speculative
    /// attempt's records never happened to the application, M6b).
    /// Eviction counters only ever grow; seqs stay monotonic.
    pub(crate) fn truncate_to(&mut self, len: usize) {
        while self.records.len() > len {
            if let Some(record) = self.records.pop_back() {
                let size = self.record_bytes(&record.kind);
                self.bytes = self.bytes.saturating_sub(size);
                self.appended = self.appended.saturating_sub(1);
                if self.seq > 0 {
                    self.seq -= 1;
                }
            }
        }
    }

    /// Captures the trace's full bookkeeping for a rollback point
    /// (speculative CSS staging, M6b).
    pub(crate) fn checkpoint(&self) -> TraceCheckpoint {
        TraceCheckpoint {
            len: self.records.len(),
            seq: self.seq,
            appended: self.appended,
            evicted: self.evicted,
            truncated: self.truncated,
        }
    }

    /// Restores a checkpoint: speculative records vanish as if they
    /// never happened; ids rewound here were never observed outside.
    pub(crate) fn restore(&mut self, checkpoint: TraceCheckpoint) {
        self.truncate_to(checkpoint.len);
        self.seq = checkpoint.seq;
        self.appended = checkpoint.appended;
        self.evicted = checkpoint.evicted;
        self.truncated = checkpoint.truncated;
    }
}

impl TurnOutcomeRecord {
    fn reason_bytes(&self) -> usize {
        match self {
            TurnOutcomeRecord::Committed { count } => 8 + count.to_string().len(),
            _ => 32,
        }
    }
}

/// The inspector snapshot (M6a, ADR 0020): a **read of recorded
/// outcomes** — cached layout, committed state revision, counters,
/// diagnostics, and pending work. Producing one never pumps, drains,
/// renders, lays out, or advances the JS logical clock; when the cached
/// layout is missing or stale the snapshot says so instead of building
/// one.
#[derive(Debug, Clone, PartialEq)]
pub struct InspectorSnapshot {
    /// The document generation the numbers below describe.
    pub generation: u64,
    /// Committed reactive turns for this generation.
    pub state_revision: u64,
    /// Completed layout passes for this generation.
    pub layout_revision: u64,
    /// The last frame index the view produced (view lifetime).
    pub frame_index: u64,
    /// State of the cached layout the geometry/style below came from.
    pub layout: LayoutCacheState,
    /// A committed turn has not been laid out yet.
    pub awaiting_relayout: bool,
    /// A presentation change has not been re-emitted yet.
    pub awaiting_repaint: bool,
    /// Work requested but not yet settled by a render.
    pub pending: PendingCauses,
    /// Lifetime counters (cheap; kept without detailed capture).
    pub counters: InspectorCounters,
    /// Normalized diagnostics from every subsystem, tagged.
    pub diagnostics: Vec<DiagnosticEntry>,
    /// The selected element's inspection (cached results only).
    pub selected: Option<ElementInspection>,
    /// Why the selection is absent, when it is.
    pub selection_note: Option<&'static str>,
    /// The last reload attempt, if any (host lifetime; survives
    /// generation swaps) — M6b, ADR 0021.
    pub last_reload: Option<crate::reload::ReloadAttempt>,
}

/// What the cached layout is relative to the asked viewport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutCacheState {
    /// A cached layout matches this viewport exactly.
    Fresh,
    /// A cached layout exists but was built for a different viewport.
    Stale,
    /// No layout has been produced for this generation yet.
    NotAvailable,
}

/// Requested-but-unsettled invalidation causes (bounded).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingCauses {
    /// Structural causes waiting for the next render.
    pub structural: Vec<String>,
    /// Presentation causes waiting for the next render.
    pub presentation: Vec<String>,
}

/// Cheap lifetime counters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InspectorCounters {
    /// Completed Taffy passes (view lifetime).
    pub layout_passes: u64,
    /// Completed presentation repaints.
    pub repaints: u64,
    /// Reactive turn attempts (committed, rejected, or rolled back).
    pub reactive_turns: u64,
    /// Display items in the last completed render.
    pub display_items_last: usize,
}

/// One normalized diagnostic: subsystem-tagged message. Detailed
/// payloads stay subsystem-specific; the envelope is for inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticEntry {
    /// `css`, `tailwind`, `reactive`, `image`, `control`, …
    pub subsystem: &'static str,
    /// The subsystem's diagnostic text.
    pub message: String,
}

/// The selected element, read from cached results. Styles are the
/// **effective** (interaction-patched) values as of the last paint —
/// the active interaction flags tell the reader which states fed them.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementInspection {
    /// The document generation this inspection describes.
    pub generation: u64,
    /// Tag name.
    pub tag: String,
    /// HTML `id`, if any.
    pub id: Option<String>,
    /// `data-vv-test` fixture id, if any.
    pub fixture_id: Option<String>,
    /// Effective computed `display`.
    pub display: String,
    /// Effective colors (as of the last paint).
    pub background_color: crate::color::Color,
    /// Effective text color.
    pub color: crate::color::Color,
    /// Effective font size (px).
    pub font_size: f32,
    /// Effective font weight.
    pub font_weight: u16,
    /// Border box (x, y, w, h) in device px, from the cached layout.
    pub border_box: (f32, f32, f32, f32),
    /// Content box (x, y, w, h).
    pub content_box: (f32, f32, f32, f32),
    /// Interaction state **now** (the snapshot's moment).
    pub hovered: bool,
    /// Focused now.
    pub focused: bool,
    /// Pressed/active now.
    pub active: bool,
    /// Control kind when the element is an editable control.
    pub control: Option<&'static str>,
    /// Runtime value length in bytes (never the value itself).
    pub value_len: Option<usize>,
}

/// Truncates a string to at most `max_bytes` on a UTF-8 boundary.
pub(crate) fn truncate_utf8(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_cause(trace: &mut Trace, seq_cause: &str) -> u64 {
        trace.push(TraceRecordKind::Invalidation(InvalidationRecord {
            generation: 1,
            classification: InvalidationClass::Presentation,
            causes: vec![seq_cause.to_owned()],
            truncated_causes: 0,
        }))
    }

    #[test]
    fn seq_is_monotonic_across_eviction() {
        let mut trace = Trace::new(InspectorLimits {
            max_records: 3,
            ..InspectorLimits::default()
        });
        for i in 0..10 {
            push_cause(&mut trace, &format!("cause-{i}"));
        }
        let summary = trace.summary();
        assert_eq!(summary.appended, 10);
        assert_eq!(summary.evicted, 7);
        assert_eq!(summary.retained, 3);
        assert_eq!(summary.first_retained, 8, "ids never renumber");
        let seqs: Vec<u64> = trace.records().map(|r| r.seq).collect();
        assert_eq!(seqs, [8, 9, 10]);
    }

    #[test]
    fn byte_bound_evicts_oldest_first() {
        let mut trace = Trace::new(InspectorLimits {
            max_retained_bytes: 400,
            ..InspectorLimits::default()
        });
        // Each record ≈ 96 base + 9 text + 16 list overhead = 121 bytes;
        // 400 retains the newest 3.
        for i in 0..8 {
            push_cause(&mut trace, &format!("cause-{i:03}"));
        }
        let summary = trace.summary();
        assert_eq!(summary.retained, 3, "{summary:?}");
        assert_eq!(summary.evicted, 5);
        assert_eq!(summary.first_retained, 6);
        let seqs: Vec<u64> = trace.records().map(|r| r.seq).collect();
        assert_eq!(seqs, [6, 7, 8], "oldest evicted first");
    }

    #[test]
    fn oversized_records_truncate_not_drop() {
        let mut trace = Trace::new(InspectorLimits {
            max_record_bytes: 128,
            max_preview_bytes: 40,
            ..InspectorLimits::default()
        });
        let big = "x".repeat(500);
        trace.push(TraceRecordKind::Invalidation(InvalidationRecord {
            generation: 1,
            classification: InvalidationClass::Structural,
            causes: vec![big],
            truncated_causes: 0,
        }));
        let summary = trace.summary();
        assert_eq!(summary.truncated, 1);
        assert_eq!(summary.retained, 1, "truncated, not dropped");
        match trace.records().next().map(|r| &r.kind) {
            Some(TraceRecordKind::Invalidation(record)) => {
                assert!(record.causes[0].len() <= 40);
            }
            other => panic!("unexpected record: {other:?}"),
        }
    }

    #[test]
    fn utf8_truncation_stays_on_boundaries() {
        let mut text = "é".repeat(100); // 2 bytes per char
        truncate_utf8(&mut text, 41); // mid-char cut
        assert!(text.len() <= 41);
        assert_eq!(text.chars().count(), 20);
    }
}
