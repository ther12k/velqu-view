//! Transactional reload (M6b, ADR 0021).
//!
//! Both reload paths are transactions with one publication rule:
//! **nothing becomes live until preparation succeeds.** A rejected
//! attempt changes reload diagnostics — never application state.
//!
//! * Full-document reload prepares a candidate **through its first
//!   rendered frame** (parse → compile → runtime → initial mutations →
//!   layout → raster) and publishes a new generation atomically.
//! * CSS-only reload stages replacement sheets against the **current
//!   committed document** (the live DOM with its reactive mutations,
//!   control state, and scroll — never a reparse of the original
//!   source), validates the resulting frame, and publishes within the
//!   existing generation.
//!
//! Reload acceptance is a policy, distinct from parser recovery and
//! from compatibility diagnostics (ADR 0021's table).

/// How many reload attempts the ledger retains.
const MAX_RELOAD_HISTORY: usize = 32;

/// Which reload path an attempt took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadKind {
    /// A full document replacement (new generation on success).
    FullDocument,
    /// A stylesheet-set replacement (same generation, state preserved).
    Stylesheets,
}

impl ReloadKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ReloadKind::FullDocument => "document",
            ReloadKind::Stylesheets => "stylesheets",
        }
    }
}

/// Where a candidate failed — the transaction boundary that rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadStage {
    /// The source bundle was unusable under the acceptance policy
    /// (unreadable/empty per the existing primitive).
    Source,
    /// Reactive initialization failed (an initializer threw, a unit
    /// failed to compile, or the state capture rejected).
    ReactiveInitialization,
    /// The initial mutation batch failed validation.
    InitialMutations,
    /// First-frame preparation (style/layout/paint) returned an error.
    FirstFrame,
}

impl ReloadStage {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ReloadStage::Source => "source",
            ReloadStage::ReactiveInitialization => "reactive initialization",
            ReloadStage::InitialMutations => "initial mutations",
            ReloadStage::FirstFrame => "first frame",
        }
    }
}

/// What an attempt did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// Published: the active generation after the swap.
    Published {
        /// The new active generation.
        generation: u64,
    },
    /// Rejected at the stage; the active application is unchanged.
    Rejected {
        /// The transaction boundary that refused to publish.
        stage: ReloadStage,
    },
}

/// One recorded reload attempt (the inspector surface: "active
/// generation 12; last attempt 19 rejected at reactive initialization;
/// active generation still 12").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadAttempt {
    /// Monotonic attempt id (view lifetime).
    pub attempt: u64,
    /// Which path.
    pub kind: ReloadKind,
    /// Published or rejected (with the rejecting stage).
    pub outcome: ReloadOutcome,
    /// The active generation before the attempt.
    pub generation_before: u64,
    /// The active generation after the attempt (equal to `before` on
    /// rejection; the new generation on publication).
    pub generation_after: u64,
    /// Stage label / detail for rejected attempts.
    pub detail: Option<String>,
}

/// The rejection a reload API call returns: the transaction refused to
/// publish, application state untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadRejection {
    /// Which path was refused.
    pub kind: ReloadKind,
    /// The rejecting stage.
    pub stage: ReloadStage,
    /// Human-readable reason.
    pub message: String,
}

impl std::fmt::Display for ReloadRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "reload ({}) rejected at {}: {}",
            self.kind.label(),
            self.stage.label(),
            self.message
        )
    }
}

impl std::error::Error for ReloadRejection {}

/// Bounded attempt history, newest last.
#[derive(Debug, Clone, Default)]
pub(crate) struct Ledger {
    next: u64,
    history: std::collections::VecDeque<ReloadAttempt>,
}

impl Ledger {
    pub(crate) fn record(
        &mut self,
        kind: ReloadKind,
        outcome: ReloadOutcome,
        generation_before: u64,
        generation_after: u64,
        detail: Option<String>,
    ) -> u64 {
        self.next += 1;
        let attempt = self.next;
        if self.history.len() >= MAX_RELOAD_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(ReloadAttempt {
            attempt,
            kind,
            outcome,
            generation_before,
            generation_after,
            detail,
        });
        attempt
    }

    pub(crate) fn last(&self) -> Option<&ReloadAttempt> {
        self.history.back()
    }
}
