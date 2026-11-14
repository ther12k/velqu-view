//! The transition engine (ADR 0024): easing curves, active animations,
//! and the override values layout/paint consume mid-flight.
//!
//! Determinism rule: the engine runs on a **virtual clock** — a `Duration`
//! the host advances explicitly ([`AnimationEngine::tick_by`]). It never
//! reads `Instant`, so identical host schedules produce bit-identical
//! interpolated values and frames across runs and platforms. Named and
//! `cubic-bezier` easings use only IEEE-exact add/multiply; the physical
//! spring driver (ADR 0027) integrates with the same restricted arithmetic
//! — no transcendentals — so spring frames are fixture-safe too.
//!
//! Invalidation is dual-tier, like the renderer's own paths (ADR 0011/
//! 0018): `Width`/`Height` animations dirty structure (a Taffy pass per
//! tick); every paint property (opacity, colors, radius, transform)
//! repaints from cached geometry with zero Taffy passes.

use std::collections::HashMap;

use crate::color::Color;
use crate::dom::NodeId;
use crate::style::{BoxShadow, Transform2D};

/// One easing curve over normalized progress `t ∈ [0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Easing {
    /// Constant speed.
    Linear,
    /// The CSS `cubic-bezier(x1, y1, x2, y2)` family (Tailwind's `ease-*`
    /// and author CSS). Solved with a fixed Newton + bisection schedule,
    /// so sampling is a pure arithmetic function of the parameters.
    CubicBezier {
        /// First control point x (CSS constrains it to `[0, 1]`).
        x1: f32,
        /// First control point y.
        y1: f32,
        /// Second control point x (CSS constrains it to `[0, 1]`).
        x2: f32,
        /// Second control point y.
        y2: f32,
    },
    // (The ADR 0024 reserved `Spring` curve variant never shipped: the
    // author-facing spring is the physical driver
    // [`TransitionDriver::Spring`], ADR 0027.)
}

impl Easing {
    /// Tailwind `ease-in`: `cubic-bezier(0.4, 0, 1, 1)`.
    pub(crate) const EASE_IN: Easing = Easing::CubicBezier {
        x1: 0.4,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
    };
    /// Tailwind `ease-out`: `cubic-bezier(0, 0, 0.2, 1)`.
    pub(crate) const EASE_OUT: Easing = Easing::CubicBezier {
        x1: 0.0,
        y1: 0.0,
        x2: 0.2,
        y2: 1.0,
    };
    /// Tailwind `ease-in-out`: `cubic-bezier(0.4, 0, 0.2, 1)`.
    pub(crate) const EASE_IN_OUT: Easing = Easing::CubicBezier {
        x1: 0.4,
        y1: 0.0,
        x2: 0.2,
        y2: 1.0,
    };
    /// Tailwind's transition default (`ease-in-out` curve).
    pub(crate) const DEFAULT: Easing = Easing::EASE_IN_OUT;

    /// Samples the curve at raw progress `t` (clamped to `[0, 1]`).
    pub(crate) fn sample(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match *self {
            Easing::Linear => t,
            Easing::CubicBezier { x1, y1, x2, y2 } => sample_cubic_bezier(x1, y1, x2, y2, t),
        }
    }
}

/// Cubic Bézier with fixed endpoints (0,0)→(1,1): `x(u)`/`y(u)`.
fn bez_component(u: f32, p1: f32, p2: f32) -> f32 {
    let one_minus_u = 1.0 - u;
    3.0 * one_minus_u * one_minus_u * u * p1 + 3.0 * one_minus_u * u * u * p2 + u * u * u
}

/// Derivative of [`bez_component`] w.r.t. `u`.
fn bez_component_derivative(u: f32, p1: f32, p2: f32) -> f32 {
    let one_minus_u = 1.0 - u;
    3.0 * one_minus_u * one_minus_u * p1
        + 6.0 * one_minus_u * u * (p2 - p1)
        + 3.0 * u * u * (1.0 - p2)
}

/// Solves the Bézier easing curve at progress `t`: finds `u` with
/// `x(u) = t` (Newton, clamped, then guaranteed bisection), then returns
/// `y(u)`. Fixed iteration counts keep the result a deterministic
/// arithmetic function of the parameters.
fn sample_cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    // Identity curve short-circuit (linear, and degenerate flat control
    // points): keeps the solver out of the common path.
    if x1 == y1 && x2 == y2 {
        return t;
    }
    let mut u = t;
    for _ in 0..8 {
        let x = bez_component(u, x1, x2) - t;
        if x.abs() < 1e-5 {
            break;
        }
        let d = bez_component_derivative(u, x1, x2);
        if d.abs() < 1e-6 {
            break;
        }
        u = (u - x / d).clamp(0.0, 1.0);
    }
    if !(0.0..=1.0).contains(&u) || u.is_nan() {
        // Bisection fallback: x(u) is monotonic for x ordinates in [0, 1],
        // so 32 halvings resolve far below f32 precision.
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..32 {
            u = (lo + hi) * 0.5;
            if bez_component(u, x1, x2) < t {
                lo = u;
            } else {
                hi = u;
            }
        }
        u = (lo + hi) * 0.5;
    }
    bez_component(u, y1, y2)
}

/// Physical spring parameters (ADR 0027): the damped harmonic oscillator
/// `a = (−k·(x − target) − c·v) / m` — framer-motion's model. Stiffness
/// `k` and damping `c` carry per-second² units; `m` scales inertia.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SpringSpec {
    /// Angular stiffness: higher springs snap harder.
    pub(crate) stiffness: f32,
    /// Velocity loss per second; 0 normalizes to critical damping.
    pub(crate) damping: f32,
    /// Inertia; ≤ 0 falls back to 1.
    pub(crate) mass: f32,
}

impl SpringSpec {
    /// Normalizes authored values: non-positive stiffness animates
    /// nothing (the spawn path applies the target directly), non-positive
    /// mass falls back to 1, and non-positive damping takes the critical
    /// value `2√(k·m)` so the integrator can never oscillate forever.
    pub(crate) fn normalized(stiffness: f32, damping: f32, mass: f32) -> Self {
        let mass = if mass > 0.0 { mass } else { 1.0 };
        let damping = if damping > 0.0 {
            damping
        } else {
            2.0 * stiffness.max(0.0).sqrt() * mass
        };
        Self {
            stiffness,
            damping,
            mass,
        }
    }

    /// True when the spec animates nothing (the target applies directly).
    pub(crate) fn is_instant(&self) -> bool {
        self.stiffness <= 0.0
    }
}

/// What produces one active animation's progress. Keyframes sample an
/// easing curve; springs integrate state per tick (ADR 0027).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum TransitionDriver {
    /// Eased progress over `duration` (the CSS/Tailwind model).
    Timed,
    /// Semi-implicit Euler integration of the normalized oscillator:
    /// `progress` is 0 at `from`, 1 at `to`, overshooting both for
    /// underdamped springs; `velocity` is its derivative in progress
    /// units per second.
    Spring {
        spec: SpringSpec,
        progress: f32,
        velocity: f32,
    },
}

/// Spring integration substep (s): fixed slices of each tick keep
/// explicit Euler stable for any plausible stiffness (ω·h ≪ 2) and make
/// identical schedules bit-identical regardless of tick length.
const SPRING_SUBSTEP_SECS: f32 = 1.0 / 240.0;

/// Progress/velocity below which a spring counts as settled.
const SPRING_SETTLE_EPSILON: f32 = 1e-3;

/// Wall-clock cap on a spring's active phase: pathological parameters
/// (tiny stiffness, near-zero damping) can never hold the frame pump
/// hostage; the cap force-settles at the target.
const SPRING_SETTLE_CAP_SECS: f32 = 10.0;

/// Which property an animation drives; the retarget and completion keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PropKind {
    Width,
    Height,
    Opacity,
    BorderRadius,
    BackgroundColor,
    BorderColor,
    Color,
    Transform,
    /// The first `box-shadow` layer (multi-layer transitions are out of
    /// profile); paint-only.
    BoxShadow,
}

/// One animated property with its from/to endpoints (all values logical
/// CSS units; colors lerp per channel).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum AnimProperty {
    /// Border-box width in logical px.
    Width {
        from: f32,
        to: f32,
    },
    /// Border-box height in logical px.
    Height {
        from: f32,
        to: f32,
    },
    Opacity {
        from: f32,
        to: f32,
    },
    /// Logical px radius.
    BorderRadius {
        from: f32,
        to: f32,
    },
    BackgroundColor {
        from: Color,
        to: Color,
    },
    BorderColor {
        from: Color,
        to: Color,
    },
    Color {
        from: Color,
        to: Color,
    },
    Transform {
        from: Transform2D,
        to: Transform2D,
    },
    /// The first `box-shadow` layer, all length fields + color lerped
    /// (paint-only; the shadow list beyond layer one stays static).
    BoxShadow {
        from: BoxShadow,
        to: BoxShadow,
    },
    /// An oscillating opacity loop between max and min (ADR 0024, `animate-pulse`).
    PulseOpacity {
        min: f32,
        max: f32,
    },
    /// An oscillating vertical bounce loop (ADR 0024, `animate-bounce`).
    BounceTranslateY {
        distance: f32,
    },
}

impl AnimProperty {
    pub(crate) fn kind(&self) -> PropKind {
        match self {
            AnimProperty::Width { .. } => PropKind::Width,
            AnimProperty::Height { .. } => PropKind::Height,
            AnimProperty::Opacity { .. } | AnimProperty::PulseOpacity { .. } => PropKind::Opacity,
            AnimProperty::BorderRadius { .. } => PropKind::BorderRadius,
            AnimProperty::BackgroundColor { .. } => PropKind::BackgroundColor,
            AnimProperty::BorderColor { .. } => PropKind::BorderColor,
            AnimProperty::Color { .. } => PropKind::Color,
            AnimProperty::Transform { .. } | AnimProperty::BounceTranslateY { .. } => {
                PropKind::Transform
            }
            AnimProperty::BoxShadow { .. } => PropKind::BoxShadow,
        }
    }

    /// Layout-affecting animations dirty structure (a Taffy pass per
    /// tick); everything else repaints from cached geometry.
    pub(crate) fn affects_layout(&self) -> bool {
        matches!(
            self,
            AnimProperty::Width { .. } | AnimProperty::Height { .. }
        )
    }

    /// The animation's target as a comparable value (the differ skips
    /// re-spawning an animation whose target has not changed — a per-tick
    /// respawn would restart progress forever).
    pub(crate) fn target_value(self) -> AnimValue {
        match self {
            AnimProperty::Width { to, .. }
            | AnimProperty::Height { to, .. }
            | AnimProperty::Opacity { to, .. }
            | AnimProperty::BorderRadius { to, .. } => AnimValue::Num(to),
            AnimProperty::PulseOpacity { min, .. } => AnimValue::Num(min),
            AnimProperty::BounceTranslateY { distance } => AnimValue::Num(distance),
            AnimProperty::BackgroundColor { to, .. }
            | AnimProperty::BorderColor { to, .. }
            | AnimProperty::Color { to, .. } => AnimValue::Col(to),
            AnimProperty::Transform { to, .. } => AnimValue::Xf(to),
            AnimProperty::BoxShadow { to, .. } => AnimValue::Shadow(to),
        }
    }

    /// Writes the eased sample into an override bundle at eased progress
    /// `t` (0 = `from`, 1 = `to`; springs may overshoot both).
    fn write_override(&self, t: f32, overrides: &mut PropertyOverrides) {
        match *self {
            AnimProperty::Width { from, to } => {
                overrides.width = Some(lerp(from, to, t).max(0.0));
            }
            AnimProperty::Height { from, to } => {
                overrides.height = Some(lerp(from, to, t).max(0.0));
            }
            AnimProperty::Opacity { from, to } => {
                overrides.opacity = Some(lerp(from, to, t).clamp(0.0, 1.0));
            }
            AnimProperty::BorderRadius { from, to } => {
                overrides.border_radius = Some(lerp(from, to, t).max(0.0));
            }
            AnimProperty::BackgroundColor { from, to } => {
                overrides.background_color = Some(lerp_color(from, to, t));
            }
            AnimProperty::BorderColor { from, to } => {
                overrides.border_color = Some(lerp_color(from, to, t));
            }
            AnimProperty::Color { from, to } => {
                overrides.color = Some(lerp_color(from, to, t));
            }
            AnimProperty::Transform { from, to } => {
                overrides.transform = Some(Transform2D {
                    scale_x: lerp(from.scale_x, to.scale_x, t),
                    scale_y: lerp(from.scale_y, to.scale_y, t),
                    rotate: lerp(from.rotate, to.rotate, t),
                    translate_x: lerp(from.translate_x, to.translate_x, t),
                    translate_y: lerp(from.translate_y, to.translate_y, t),
                });
            }
            AnimProperty::BoxShadow { from, to } => {
                overrides.box_shadow = Some(BoxShadow {
                    offset_x: lerp(from.offset_x, to.offset_x, t),
                    offset_y: lerp(from.offset_y, to.offset_y, t),
                    blur: lerp(from.blur, to.blur, t).max(0.0),
                    spread: lerp(from.spread, to.spread, t),
                    color: lerp_color(from.color, to.color, t),
                });
            }
            AnimProperty::PulseOpacity { min, max } => {
                let ping_pong = if t < 0.5 { t * 2.0 } else { (1.0 - t) * 2.0 };
                let opacity = lerp(max, min, ping_pong).clamp(0.0, 1.0);
                overrides.opacity = Some(opacity);
            }
            AnimProperty::BounceTranslateY { distance } => {
                let ping_pong = if t < 0.5 { t * 2.0 } else { (1.0 - t) * 2.0 };
                let dy = -distance * (1.0 - ping_pong);
                overrides.transform = Some(Transform2D {
                    scale_x: 1.0,
                    scale_y: 1.0,
                    rotate: 0.0,
                    translate_x: 0.0,
                    translate_y: dy,
                });
            }
        }
    }

    /// The `from → to` span in value units for scalar properties — the
    /// scale between normalized spring progress and physical units.
    /// `None` for per-channel properties (colors, transforms): their
    /// spring retargets restart at rest, since per-channel velocity
    /// transfer to a scalar oscillator is ambiguous.
    fn scalar_span(&self) -> Option<f32> {
        match *self {
            AnimProperty::Width { from, to }
            | AnimProperty::Height { from, to }
            | AnimProperty::Opacity { from, to }
            | AnimProperty::BorderRadius { from, to } => Some(to - from),
            AnimProperty::BackgroundColor { .. }
            | AnimProperty::BorderColor { .. }
            | AnimProperty::Color { .. }
            | AnimProperty::Transform { .. }
            | AnimProperty::BoxShadow { .. }
            | AnimProperty::PulseOpacity { .. }
            | AnimProperty::BounceTranslateY { .. } => None,
        }
    }

    /// The same property with its `from` endpoint replaced by the current
    /// override sample (CSS retargeting: a change mid-flight starts from
    /// what is currently on screen). Kinds must match; the differ only
    /// retargets like with like. The override bundle always holds the
    /// running animation's latest sample (seeded at spawn, rewritten per
    /// tick), and when a field is absent the caller-authored `from` —
    /// captured from the cached box tree — is already the on-screen value.
    fn retargeted_from(&self, current: PropertyOverrides) -> Self {
        match *self {
            AnimProperty::Width { from, to } => AnimProperty::Width {
                from: current.width.unwrap_or(from),
                to,
            },
            AnimProperty::Height { from, to } => AnimProperty::Height {
                from: current.height.unwrap_or(from),
                to,
            },
            AnimProperty::Opacity { from, to } => AnimProperty::Opacity {
                from: current.opacity.unwrap_or(from),
                to,
            },
            AnimProperty::BorderRadius { from, to } => AnimProperty::BorderRadius {
                from: current.border_radius.unwrap_or(from),
                to,
            },
            AnimProperty::BackgroundColor { from, to } => AnimProperty::BackgroundColor {
                from: current.background_color.unwrap_or(from),
                to,
            },
            AnimProperty::BorderColor { from, to } => AnimProperty::BorderColor {
                from: current.border_color.unwrap_or(from),
                to,
            },
            AnimProperty::Color { from, to } => AnimProperty::Color {
                from: current.color.unwrap_or(from),
                to,
            },
            AnimProperty::Transform { from, to } => AnimProperty::Transform {
                from: current.transform.unwrap_or(from),
                to,
            },
            AnimProperty::BoxShadow { from, to } => AnimProperty::BoxShadow {
                from: current.box_shadow.unwrap_or(from),
                to,
            },
            AnimProperty::PulseOpacity { min, max } => AnimProperty::PulseOpacity { min, max },
            AnimProperty::BounceTranslateY { distance } => {
                AnimProperty::BounceTranslateY { distance }
            }
        }
    }
}

fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

fn lerp_channel(from: u8, to: u8, t: f32) -> u8 {
    (from as f32 + (to as f32 - from as f32) * t).round() as u8
}

fn lerp_color(from: Color, to: Color, t: f32) -> Color {
    Color::from_rgba8(
        lerp_channel(from.r, to.r, t),
        lerp_channel(from.g, to.g, t),
        lerp_channel(from.b, to.b, t),
        lerp_channel(from.a, to.a, t),
    )
}

/// Mid-flight animated values for one node, keyed per property. Applied
/// over the cascade's computed styles in both render paths, so animated
/// values always win until the animation settles (where the computed
/// target already equals the final sample).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct PropertyOverrides {
    pub(crate) width: Option<f32>,
    pub(crate) height: Option<f32>,
    pub(crate) opacity: Option<f32>,
    pub(crate) border_radius: Option<f32>,
    pub(crate) background_color: Option<Color>,
    pub(crate) border_color: Option<Color>,
    pub(crate) color: Option<Color>,
    pub(crate) transform: Option<Transform2D>,
    pub(crate) box_shadow: Option<BoxShadow>,
}

impl PropertyOverrides {
    /// The overridden value for `kind`, if any (the differ's settled-state
    /// guard: a pinned completion value equals the cascade target).
    pub(crate) fn value_for(&self, kind: PropKind) -> Option<AnimValue> {
        match kind {
            PropKind::Width => self.width.map(AnimValue::Num),
            PropKind::Height => self.height.map(AnimValue::Num),
            PropKind::Opacity => self.opacity.map(AnimValue::Num),
            PropKind::BorderRadius => self.border_radius.map(AnimValue::Num),
            PropKind::BackgroundColor => self.background_color.map(AnimValue::Col),
            PropKind::BorderColor => self.border_color.map(AnimValue::Col),
            PropKind::Color => self.color.map(AnimValue::Col),
            PropKind::Transform => self.transform.map(AnimValue::Xf),
            PropKind::BoxShadow => self.box_shadow.map(AnimValue::Shadow),
        }
    }
}

/// Animated values per node, keyed for the render paths.
pub(crate) type PropertyOverrideMap = HashMap<NodeId, PropertyOverrides>;

/// One comparable property value (the differ's retarget guard).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum AnimValue {
    Num(f32),
    Col(Color),
    Xf(Transform2D),
    Shadow(BoxShadow),
}

/// How many times an animation runs (ADR 0024: transitions run once;
/// Tailwind `animate-spin`/`animate-pulse`/`animate-bounce` loop infinitely).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AnimationRepeat {
    #[default]
    Once,
    Infinite,
}

/// One in-flight animation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActiveAnimation {
    pub(crate) node: NodeId,
    pub(crate) property: AnimProperty,
    /// Virtual-clock timestamp the delay counts from.
    started_at: Duration64,
    delay: Duration64,
    duration: Duration64,
    easing: Easing,
    repeat: AnimationRepeat,
    /// How progress is produced. Springs ignore `duration`/`easing`.
    driver: TransitionDriver,
}

/// `std::time::Duration` alias keeping the struct signatures short.
type Duration64 = std::time::Duration;

impl ActiveAnimation {
    /// Integrates the spring driver one tick: fixed substeps of the
    /// caller's virtual-clock delta (stable explicit Euler, ADR 0027).
    /// Returns `true` when the spring has settled (or hit the settle cap)
    /// and the animation is done.
    fn step_spring(&mut self, now: Duration64, dt_secs: f32) -> bool {
        let TransitionDriver::Spring {
            spec,
            progress,
            velocity,
        } = &mut self.driver
        else {
            return false;
        };
        let elapsed = now.saturating_sub(self.started_at);
        let Some(active) = elapsed.checked_sub(self.delay) else {
            return false; // still inside the delay: hold the from-value
        };
        if active.as_secs_f32() >= SPRING_SETTLE_CAP_SECS {
            *progress = 1.0;
            *velocity = 0.0;
            return true;
        }
        if dt_secs <= 0.0 {
            return false;
        }
        let mut remaining = dt_secs;
        while remaining > 0.0 {
            let h = remaining.min(SPRING_SUBSTEP_SECS);
            let acceleration =
                (-spec.stiffness * (*progress - 1.0) - spec.damping * *velocity) / spec.mass;
            // Semi-implicit Euler: velocity first, then position with the
            // fresh velocity.
            *velocity += acceleration * h;
            *progress += *velocity * h;
            remaining -= h;
        }
        if !progress.is_finite() || !velocity.is_finite() {
            *progress = 1.0;
            *velocity = 0.0;
            return true;
        }
        (*progress - 1.0).abs() < SPRING_SETTLE_EPSILON
            && velocity.abs() < SPRING_SETTLE_EPSILON
    }

    /// Raw (uneased) progress at virtual time `now`: 0 during the delay,
    /// then linear over the duration, clamped to 1 (or cycling in [0, 1) for infinite loops).
    fn raw_progress(&self, now: Duration64) -> f32 {
        let elapsed = now.saturating_sub(self.started_at);
        let Some(after_delay) = elapsed.checked_sub(self.delay) else {
            return 0.0;
        };
        if self.duration.is_zero() {
            return 1.0;
        }
        let total_secs = after_delay.as_secs_f32();
        let dur_secs = self.duration.as_secs_f32();
        match self.repeat {
            AnimationRepeat::Once => (total_secs / dur_secs).clamp(0.0, 1.0),
            AnimationRepeat::Infinite => (total_secs % dur_secs) / dur_secs,
        }
    }

    fn is_finished(&self, now: Duration64) -> bool {
        match self.repeat {
            AnimationRepeat::Once => self.raw_progress(now) >= 1.0,
            AnimationRepeat::Infinite => false,
        }
    }
}

/// What one virtual-clock tick did.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TickOutcome {
    /// Any animation is still pending or running (the host keeps pumping
    /// frames only while this is true).
    pub(crate) more: bool,
    /// A width/height override exists: the next render needs a Taffy pass.
    pub(crate) layout_dirty: bool,
    /// Any override exists: the next render repaints (no Taffy pass).
    pub(crate) paint_dirty: bool,
    /// `(node, property)` pairs that reached completion on this tick (the
    /// vx-transition state machine advances phases on them).
    pub(crate) finished: Vec<(NodeId, PropKind)>,
}

/// The per-document animation registry. One instance lives on
/// [`crate::VelquView`]; it dies with the document generation (node ids
/// are per-parse).
#[derive(Debug, Clone, Default)]
pub(crate) struct AnimationEngine {
    /// The virtual clock; advanced only by [`AnimationEngine::tick_by`].
    clock: Duration64,
    /// In-flight animations in spawn order.
    pub(crate) active: Vec<ActiveAnimation>,
    /// Current mid-flight values per node (rebuilt on each tick; also
    /// seeded at spawn so the first frame holds the from-value).
    overrides: HashMap<NodeId, PropertyOverrides>,
}

impl AnimationEngine {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Whether any animation is pending or running.
    pub(crate) fn has_active(&self) -> bool {
        !self.active.is_empty()
    }

    /// Whether any **layout-affecting** animation (width/height) is in
    /// flight — the host's fast-frame signal: such animations change the
    /// silhouette every tick and need the shortest frame cadence, while
    /// paint-only loops (pulses, fades) tolerate the ambient 30fps.
    pub(crate) fn has_layout_animation(&self) -> bool {
        self.active
            .iter()
            .any(|anim| anim.property.affects_layout())
    }

    /// Whether the node has any animation in flight.
    pub(crate) fn has_active_for(&self, node: NodeId) -> bool {
        self.active.iter().any(|anim| anim.node == node)
    }

    /// The current target of the node's animation for `kind`, if any.
    /// The differ uses this to leave a running animation alone while its
    /// target is unchanged (and to retarget it cleanly when it does).
    pub(crate) fn active_target(&self, node: NodeId, kind: PropKind) -> Option<AnimValue> {
        self.active
            .iter()
            .find(|anim| anim.node == node && anim.property.kind() == kind)
            .map(|anim| anim.property.target_value())
    }

    /// Drops everything (document generation change: node ids are
    /// per-parse and must never leak across loads).
    pub(crate) fn clear(&mut self) {
        self.active.clear();
        self.overrides.clear();
        // The clock survives: it is document-agnostic schedule time. A
        // fresh generation starting mid-stream keeps monotonic timestamps.
    }

    /// Starts (or retargets) one animation. A zero duration applies no
    /// animation — the computed target lands immediately (CSS
    /// `transition-duration: 0s`). An existing animation on the same
    /// node+property is replaced, starting from its current interpolated
    /// value at the current clock; per CSS, the delay and duration restart.
    pub(crate) fn start(
        &mut self,
        node: NodeId,
        property: AnimProperty,
        delay: Duration64,
        duration: Duration64,
        easing: Easing,
    ) {
        self.start_with_repeat(
            node,
            property,
            delay,
            duration,
            easing,
            AnimationRepeat::Once,
        );
    }

    /// Starts (or retargets) an animation with an explicit repeat policy
    /// (e.g. `AnimationRepeat::Infinite` for Tailwind loading animations).
    pub(crate) fn start_with_repeat(
        &mut self,
        node: NodeId,
        property: AnimProperty,
        delay: Duration64,
        duration: Duration64,
        easing: Easing,
        repeat: AnimationRepeat,
    ) {
        if duration.is_zero() {
            return;
        }
        let kind = property.kind();
        if let Some(index) = self
            .active
            .iter()
            .position(|anim| anim.node == node && anim.property.kind() == kind)
        {
            let _existing = self.active.remove(index);
            let current = self.overrides.get(&node).copied().unwrap_or_default();
            let retargeted = property.retargeted_from(current);
            self.seed_override(node, &retargeted);
            self.active.push(ActiveAnimation {
                node,
                property: retargeted,
                started_at: self.clock,
                delay,
                duration,
                easing,
                repeat,
                driver: TransitionDriver::Timed,
            });
            return;
        }
        self.seed_override(node, &property);
        self.active.push(ActiveAnimation {
            node,
            property,
            started_at: self.clock,
            delay,
            duration,
            easing,
            repeat,
            driver: TransitionDriver::Timed,
        });
    }

    /// Starts (or retargets) a physical spring animation (ADR 0027). An
    /// existing spring on the same node+property hands over its momentum:
    /// the retargeted animation restarts from the current on-screen value
    /// with the physical velocity preserved (scalars only — per-channel
    /// properties restart at rest). A zero-stiffness spec animates
    /// nothing; the spawn path applies the target directly instead.
    pub(crate) fn start_spring(
        &mut self,
        node: NodeId,
        property: AnimProperty,
        delay: Duration64,
        spec: SpringSpec,
    ) {
        if spec.is_instant() {
            return;
        }
        let kind = property.kind();
        if let Some(index) = self
            .active
            .iter()
            .position(|anim| anim.node == node && anim.property.kind() == kind)
        {
            let existing = self.active.remove(index);
            let current = self.overrides.get(&node).copied().unwrap_or_default();
            let retargeted = property.retargeted_from(current);
            self.seed_override(node, &retargeted);
            // Momentum handover: convert the old normalized velocity into
            // physical value-units, then back into the new span's units.
            let velocity = match (
                existing.driver,
                existing.property.scalar_span(),
                retargeted.scalar_span(),
            ) {
                (
                    TransitionDriver::Spring { velocity, .. },
                    Some(span_old),
                    Some(span_new),
                ) if span_new.abs() > f32::EPSILON => span_old * velocity / span_new,
                _ => 0.0,
            };
            self.active.push(ActiveAnimation {
                node,
                property: retargeted,
                started_at: self.clock,
                delay,
                duration: Duration64::ZERO,
                easing: Easing::DEFAULT,
                repeat: AnimationRepeat::Once,
                driver: TransitionDriver::Spring {
                    spec,
                    progress: 0.0,
                    velocity,
                },
            });
            return;
        }
        self.seed_override(node, &property);
        self.active.push(ActiveAnimation {
            node,
            property,
            started_at: self.clock,
            delay,
            duration: Duration64::ZERO,
            easing: Easing::DEFAULT,
            repeat: AnimationRepeat::Once,
            driver: TransitionDriver::Spring {
                spec,
                progress: 0.0,
                velocity: 0.0,
            },
        });
    }

    /// Stops any animation of `kind` running on `node`.
    #[allow(dead_code)]
    pub(crate) fn stop_for(&mut self, node: NodeId, kind: PropKind) {
        self.active
            .retain(|anim| !(anim.node == node && anim.property.kind() == kind));
    }

    /// Stops any infinite looping animation running on `node` (e.g. when an
    /// element's `animate-*` class is removed).
    pub(crate) fn stop_infinite_for(&mut self, node: NodeId) {
        self.active
            .retain(|anim| !(anim.node == node && anim.repeat == AnimationRepeat::Infinite));
    }

    /// Seeds the override bundle at `t = 0` so the frame the animation
    /// spawns in already shows the from-value (no first-frame jump to the
    /// target).
    fn seed_override(&mut self, node: NodeId, property: &AnimProperty) {
        let entry = self.overrides.entry(node).or_default();
        property.write_override(0.0, entry);
    }

    /// Read-only override access for the render paths.
    pub(crate) fn overrides(&self) -> &HashMap<NodeId, PropertyOverrides> {
        &self.overrides
    }

    /// Advances the virtual clock by `dt` and re-derives every override.
    /// Timed animations sample their easing at the clock; springs
    /// integrate their state by `dt` (ADR 0027). Finished animations pin
    /// their final (`t = 1`) samples into the override map — equal to the
    /// cascade's targets — so a later style recompute sees no delta and
    /// never respawns a settled transition.
    pub(crate) fn tick_by(&mut self, dt: Duration64) -> TickOutcome {
        if self.active.is_empty() {
            return TickOutcome::default();
        }
        self.clock += dt;
        let dt_secs = dt.as_secs_f32();
        let mut finished = Vec::new();
        let mut completed: Vec<(NodeId, AnimProperty)> = Vec::new();
        self.active.retain_mut(|animation| {
            let done = match &mut animation.driver {
                TransitionDriver::Timed => animation.is_finished(self.clock),
                TransitionDriver::Spring { .. } => animation.step_spring(self.clock, dt_secs),
            };
            if done {
                finished.push((animation.node, animation.property.kind()));
                completed.push((animation.node, animation.property));
            }
            !done
        });
        self.overrides.clear();
        let mut layout_dirty = false;
        let mut paint_dirty = false;
        for animation in &self.active {
            let eased = match &animation.driver {
                TransitionDriver::Timed => {
                    animation.easing.sample(animation.raw_progress(self.clock))
                }
                TransitionDriver::Spring { progress, .. } => *progress,
            };
            let entry = self.overrides.entry(animation.node).or_default();
            animation.property.write_override(eased, entry);
            if animation.property.affects_layout() {
                layout_dirty = true;
            } else {
                paint_dirty = true;
            }
        }
        // Finished animations pin their `t = 1` sample permanently: it
        // equals the cascade's target, so the next style recompute sees
        // no delta and never respawns a finished transition (the cached
        // box would otherwise hold the last mid-flight value, forever
        // looking like a fresh change). The entry dies with the document
        // generation (or is rewritten by the next retarget).
        for (node, property) in completed {
            let entry = self.overrides.entry(node).or_default();
            property.write_override(1.0, entry);
        }
        // A completion must dirty too: the final frame renders the
        // settled state, and a finished layout animation needs its
        // settling pass at the target size.
        if !finished.is_empty() {
            let layout_finished = finished
                .iter()
                .any(|(_, kind)| matches!(kind, PropKind::Width | PropKind::Height));
            if layout_finished {
                layout_dirty = true;
            } else {
                paint_dirty = true;
            }
        }
        TickOutcome {
            more: !self.active.is_empty(),
            layout_dirty,
            paint_dirty,
            finished,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(r: u8, g: u8, b: u8) -> Color {
        Color::from_rgb8(r, g, b)
    }

    #[test]
    fn linear_and_bezier_endpoints_and_midpoints() {
        assert_eq!(Easing::Linear.sample(0.0), 0.0);
        assert_eq!(Easing::Linear.sample(1.0), 1.0);
        assert_eq!(Easing::Linear.sample(0.25), 0.25);

        // The identity bezier is exactly linear.
        assert_eq!(
            Easing::CubicBezier {
                x1: 0.0,
                y1: 0.0,
                x2: 1.0,
                y2: 1.0
            }
            .sample(0.25),
            0.25
        );

        // The symmetric CSS `ease-in-out` curve passes through the
        // midpoint exactly; Tailwind's 0.4/0.2 curve does not (it runs
        // fast through the middle — ~0.776 at t = 0.5, verified against
        // the Bézier math by hand).
        let symmetric = Easing::CubicBezier {
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        };
        let mid = symmetric.sample(0.5);
        assert!(
            (mid - 0.5).abs() < 1e-2,
            "symmetric ease-in-out(0.5) ≈ 0.5, got {mid}"
        );
        let tailwind_mid = Easing::EASE_IN_OUT.sample(0.5);
        assert!(
            (tailwind_mid - 0.7756).abs() < 1e-2,
            "Tailwind ease-in-out(0.5) ≈ 0.7756, got {tailwind_mid}"
        );

        // Endpoints clamp exactly regardless of curve.
        assert_eq!(Easing::EASE_IN.sample(0.0), 0.0);
        assert_eq!(Easing::EASE_IN.sample(1.0), 1.0);
        assert_eq!(Easing::EASE_OUT.sample(0.0), 0.0);
        assert_eq!(Easing::EASE_OUT.sample(1.0), 1.0);

        // ease-in is below linear, ease-out above it, in the first half.
        assert!(Easing::EASE_IN.sample(0.4) < 0.4);
        assert!(Easing::EASE_OUT.sample(0.4) > 0.4);
    }

    #[test]
    fn bezier_solver_is_monotonic_for_named_curves() {
        for easing in [Easing::EASE_IN, Easing::EASE_OUT, Easing::EASE_IN_OUT] {
            let mut last = -1.0;
            for step in 0..=20 {
                let t = step as f32 / 20.0;
                let value = easing.sample(t);
                assert!(value >= last, "curve must not reverse");
                last = value;
            }
        }
    }

    const FRAME: Duration64 = Duration64::from_millis(16);

    #[test]
    fn spring_settles_and_pins_target() {
        let mut engine = AnimationEngine::new();
        engine.start_spring(
            1,
            AnimProperty::BorderRadius {
                from: 0.0,
                to: 24.0,
            },
            Duration64::ZERO,
            SpringSpec::normalized(170.0, 26.0, 1.0),
        );
        assert_eq!(engine.overrides().get(&1).unwrap().border_radius, Some(0.0));
        for _ in 0..600 {
            let tick = engine.tick_by(FRAME);
            if !tick.more {
                break;
            }
            // Never NaN, never below zero (the property clamps at 0).
            let value = engine.overrides().get(&1).unwrap().border_radius.unwrap();
            assert!(value.is_finite() && value >= 0.0);
        }
        assert!(!engine.has_active(), "spring must settle");
        let pinned = engine.overrides().get(&1).unwrap().border_radius;
        assert_eq!(pinned, Some(24.0), "completion pins the target");
    }

    #[test]
    fn spring_overshoots_with_low_damping() {
        let mut engine = AnimationEngine::new();
        engine.start_spring(
            2,
            AnimProperty::Width {
                from: 0.0,
                to: 100.0,
            },
            Duration64::ZERO,
            SpringSpec::normalized(170.0, 5.0, 1.0),
        );
        let mut overshot = false;
        for _ in 0..600 {
            let tick = engine.tick_by(FRAME);
            let value = engine.overrides().get(&2).unwrap().width.unwrap();
            if value > 100.0 {
                overshot = true;
            }
            if !tick.more {
                break;
            }
        }
        assert!(overshot, "underdamped spring must overshoot the target");
    }

    #[test]
    fn spring_retarget_preserves_momentum() {
        let mut engine = AnimationEngine::new();
        engine.start_spring(
            3,
            AnimProperty::Width {
                from: 0.0,
                to: 100.0,
            },
            Duration64::ZERO,
            SpringSpec::normalized(120.0, 12.0, 1.0),
        );
        // Run until the value is clearly moving upward.
        let mut mid_value = 0.0;
        for _ in 0..50 {
            engine.tick_by(FRAME);
            mid_value = engine.overrides().get(&3).unwrap().width.unwrap();
            if mid_value > 20.0 {
                break;
            }
        }
        assert!(mid_value > 20.0, "spring made progress, got {mid_value}");
        // Retarget downward: the value must not jump and momentum must
        // briefly carry it further up before the pull of the new target.
        engine.start_spring(
            3,
            AnimProperty::Width {
                from: 0.0,
                to: 0.0,
            },
            Duration64::ZERO,
            SpringSpec::normalized(120.0, 12.0, 1.0),
        );
        assert_eq!(engine.active.len(), 1, "retarget replaces, not stacks");
        let after_retarget = engine.overrides().get(&3).unwrap().width.unwrap();
        assert!(
            (after_retarget - mid_value).abs() < 1e-3,
            "retarget must not jump: {after_retarget} vs {mid_value}"
        );
        engine.tick_by(FRAME);
        let next = engine.overrides().get(&3).unwrap().width.unwrap();
        assert!(
            next > after_retarget,
            "momentum carries past the retarget: {next} vs {after_retarget}"
        );
        // The reversed spring still settles at the new target.
        for _ in 0..600 {
            if !engine.tick_by(FRAME).more {
                break;
            }
        }
        assert!(!engine.has_active());
        assert_eq!(engine.overrides().get(&3).unwrap().width, Some(0.0));
    }

    #[test]
    fn spring_delay_holds_from_value() {
        let mut engine = AnimationEngine::new();
        engine.start_spring(
            4,
            AnimProperty::Opacity {
                from: 0.0,
                to: 1.0,
            },
            Duration64::from_millis(100),
            SpringSpec::normalized(170.0, 26.0, 1.0),
        );
        let tick = engine.tick_by(Duration64::from_millis(50));
        assert!(tick.more && tick.paint_dirty);
        assert_eq!(engine.overrides().get(&4).unwrap().opacity, Some(0.0));
        // Past the delay the spring moves.
        engine.tick_by(Duration64::from_millis(200));
        let value = engine.overrides().get(&4).unwrap().opacity.unwrap();
        assert!(value > 0.0, "spring moves after the delay, got {value}");
    }

    #[test]
    fn spring_zero_stiffness_animates_nothing() {
        let mut engine = AnimationEngine::new();
        engine.start_spring(
            5,
            AnimProperty::Opacity {
                from: 0.0,
                to: 1.0,
            },
            Duration64::ZERO,
            SpringSpec::normalized(0.0, 26.0, 1.0),
        );
        assert!(!engine.has_active());
        assert!(engine.overrides().is_empty());
    }

    #[test]
    fn spring_identical_schedules_are_bit_identical() {
        let run = || {
            let mut engine = AnimationEngine::new();
            engine.start_spring(
                9,
                AnimProperty::Width {
                    from: 0.0,
                    to: 64.0,
                },
                Duration64::ZERO,
                SpringSpec::normalized(200.0, 18.0, 1.2),
            );
            let mut samples = Vec::new();
            for _ in 0..120 {
                engine.tick_by(FRAME);
                samples.push(engine.overrides().get(&9).unwrap().width.unwrap().to_bits());
            }
            samples
        };
        let a = run();
        let b = run();
        assert_eq!(a, b, "identical schedules must be bit-identical");
    }

    #[test]
    fn delay_holds_from_value_and_completion_lands_on_to() {
        let mut engine = AnimationEngine::new();
        engine.start(
            7,
            AnimProperty::Opacity { from: 0.0, to: 1.0 },
            Duration64::from_millis(100),
            Duration64::from_millis(200),
            Easing::Linear,
        );
        // Spawn seeds the from-value immediately: no first-frame jump.
        let seeded = engine.overrides().get(&7).copied().unwrap();
        assert_eq!(seeded.opacity, Some(0.0));

        // During the delay nothing moves (clock 50ms < 100ms delay).
        let tick = engine.tick_by(Duration64::from_millis(50));
        assert!(tick.more && tick.paint_dirty && !tick.layout_dirty);
        assert_eq!(engine.overrides().get(&7).unwrap().opacity, Some(0.0));

        // The delay end is still progress 0; 50ms into the active phase
        // (clock 150ms) linear progress is 0.25.
        engine.tick_by(Duration64::from_millis(50));
        assert_eq!(engine.overrides().get(&7).unwrap().opacity, Some(0.0));
        engine.tick_by(Duration64::from_millis(50));
        assert_eq!(engine.overrides().get(&7).unwrap().opacity, Some(0.25));

        // At exactly delay+duration (clock 300ms) the animation finishes
        // and pins its final sample: the override holds `to` (equal to
        // the cascade's target), so nothing respawns later.
        let done = engine.tick_by(Duration64::from_millis(150));
        assert!(!done.more);
        assert_eq!(done.finished, vec![(7, PropKind::Opacity)]);
        assert!(done.paint_dirty, "the settling frame must repaint");
        assert_eq!(engine.overrides().get(&7).unwrap().opacity, Some(1.0));
        assert!(!engine.has_active());
    }

    #[test]
    fn width_animations_dirty_layout_and_clamp_negative() {
        let mut engine = AnimationEngine::new();
        engine.start(
            3,
            AnimProperty::Width {
                from: 100.0,
                to: 50.0,
            },
            Duration64::ZERO,
            Duration64::from_millis(100),
            Easing::Linear,
        );
        let tick = engine.tick_by(Duration64::from_millis(50));
        assert!(tick.layout_dirty && tick.more);
        assert_eq!(engine.overrides().get(&3).unwrap().width, Some(75.0));
    }

    #[test]
    fn retarget_mid_flight_starts_from_current_value() {
        let mut engine = AnimationEngine::new();
        engine.start(
            1,
            AnimProperty::Opacity { from: 0.0, to: 1.0 },
            Duration64::ZERO,
            Duration64::from_millis(100),
            Easing::Linear,
        );
        engine.tick_by(Duration64::from_millis(50));
        assert_eq!(engine.overrides().get(&1).unwrap().opacity, Some(0.5));

        // Reverse the target: the new animation starts at 0.5.
        engine.start(
            1,
            AnimProperty::Opacity { from: 0.5, to: 0.0 },
            Duration64::ZERO,
            Duration64::from_millis(100),
            Easing::Linear,
        );
        assert_eq!(engine.active.len(), 1, "retarget replaces, not stacks");
        assert_eq!(engine.overrides().get(&1).unwrap().opacity, Some(0.5));
        engine.tick_by(Duration64::from_millis(50));
        assert_eq!(engine.overrides().get(&1).unwrap().opacity, Some(0.25));
    }

    #[test]
    fn zero_duration_applies_nothing() {
        let mut engine = AnimationEngine::new();
        engine.start(
            2,
            AnimProperty::Opacity { from: 0.0, to: 1.0 },
            Duration64::ZERO,
            Duration64::ZERO,
            Easing::Linear,
        );
        assert!(!engine.has_active());
        assert!(engine.overrides().is_empty());
    }

    #[test]
    fn clear_drops_everything() {
        let mut engine = AnimationEngine::new();
        engine.start(
            4,
            AnimProperty::BackgroundColor {
                from: color(0, 0, 0),
                to: color(255, 255, 255),
            },
            Duration64::ZERO,
            Duration64::from_millis(100),
            Easing::Linear,
        );
        engine.tick_by(Duration64::from_millis(50));
        assert!(engine.has_active_for(4));
        engine.clear();
        assert!(!engine.has_active());
        assert!(engine.overrides().is_empty());
    }

    #[test]
    fn colors_lerp_per_channel() {
        let mut engine = AnimationEngine::new();
        engine.start(
            5,
            AnimProperty::BackgroundColor {
                from: color(0, 0, 0),
                to: color(200, 100, 0),
            },
            Duration64::ZERO,
            Duration64::from_millis(100),
            Easing::Linear,
        );
        engine.tick_by(Duration64::from_millis(50));
        let lerped = engine.overrides().get(&5).unwrap().background_color;
        assert_eq!(lerped, Some(color(100, 50, 0)));
    }
}
