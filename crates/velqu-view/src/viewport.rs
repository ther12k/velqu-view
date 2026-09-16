//! Physical render target with enforced invariants.
//!
//! M1.1 hardening (ADR 0004): a `Viewport` can no longer hold documentary
//! "must be positive" comments as its only defense. Construction validates
//! dimensions, scale, and total pixel count, so every downstream consumer
//! (layout in M2, the painter today) can rely on:
//!
//! * `width > 0 && height > 0`;
//! * `scale_factor` finite and `> 0`;
//! * `width * height <= Viewport::MAX_PIXELS`, bounding frame allocations.

use std::fmt;

/// A physical render target description.
///
/// `width`/`height` are in physical (device) pixels; `scale_factor` is the
/// device-pixels-per-logical-pixel ratio (1.0 = 96-dpi nominal). All document
/// geometry is authored in logical pixels and scaled here, so a DPI change is
/// a viewport change — no hidden global DPI state.
///
/// Construct via [`Viewport::try_new`], which enforces the invariants above;
/// fields are private so they cannot be invalidated after construction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    width: u32,
    height: u32,
    scale_factor: f32,
}

/// Upper bound on `width * height` accepted by [`Viewport::try_new`].
///
/// 2^28 pixels = a 1 GiB RGBA8 buffer. Deliberately generous (covers any
/// realistic multi-monitor 8K surface) while keeping a hostile or buggy
/// viewport from triggering uncontrolled allocations.
pub const MAX_PIXELS: u64 = 1 << 28;

impl Viewport {
    /// Creates a viewport from physical dimensions and a scale factor,
    /// validating invariants.
    ///
    /// Errors when dimensions are zero, the scale factor is not a finite
    /// positive number, or the pixel count exceeds [`MAX_PIXELS`].
    pub fn try_new(width: u32, height: u32, scale_factor: f32) -> Result<Self, InvalidViewport> {
        if width == 0 || height == 0 {
            return Err(InvalidViewport {
                reason: InvalidViewportReason::ZeroDimensions,
            });
        }
        if !scale_factor.is_finite() {
            return Err(InvalidViewport {
                reason: InvalidViewportReason::NonFiniteScaleFactor,
            });
        }
        if scale_factor <= 0.0 {
            return Err(InvalidViewport {
                reason: InvalidViewportReason::NonPositiveScaleFactor,
            });
        }
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(InvalidViewport {
                reason: InvalidViewportReason::TooManyPixels,
            });
        }
        Ok(Self {
            width,
            height,
            scale_factor,
        })
    }

    /// Width in physical pixels; always > 0.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in physical pixels; always > 0.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Device pixels per logical pixel; always finite and > 0.
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Logical width (`width / scale_factor`); division is safe by
    /// construction.
    pub fn logical_width(&self) -> f32 {
        self.width as f32 / self.scale_factor
    }

    /// Logical height (`height / scale_factor`).
    pub fn logical_height(&self) -> f32 {
        self.height as f32 / self.scale_factor
    }
}

/// Why a viewport was rejected by [`Viewport::try_new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidViewportReason {
    /// `width` or `height` was zero.
    ZeroDimensions,
    /// `scale_factor` was NaN or infinite.
    NonFiniteScaleFactor,
    /// `scale_factor` was zero or negative.
    NonPositiveScaleFactor,
    /// `width * height` exceeded [`MAX_PIXELS`].
    TooManyPixels,
}

impl fmt::Display for InvalidViewportReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            InvalidViewportReason::ZeroDimensions => "width and height must be non-zero",
            InvalidViewportReason::NonFiniteScaleFactor => "scale factor must be a finite number",
            InvalidViewportReason::NonPositiveScaleFactor => "scale factor must be > 0",
            InvalidViewportReason::TooManyPixels => {
                "viewport exceeds the maximum supported pixel count"
            }
        })
    }
}

/// A viewport rejected by [`Viewport::try_new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidViewport {
    /// Which invariant failed.
    pub reason: InvalidViewportReason,
}

impl fmt::Display for InvalidViewport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid viewport: {}", self.reason)
    }
}

impl std::error::Error for InvalidViewport {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_new_accepts_valid_viewports() {
        let vp = Viewport::try_new(800, 600, 1.0).unwrap();
        assert_eq!(
            (vp.width(), vp.height(), vp.scale_factor()),
            (800, 600, 1.0)
        );
        assert!((vp.logical_width() - 800.0).abs() < f32::EPSILON);

        let hidpi = Viewport::try_new(1600, 1200, 2.0).unwrap();
        assert!((hidpi.logical_width() - 800.0).abs() < f32::EPSILON);
        assert!((hidpi.logical_height() - 600.0).abs() < f32::EPSILON);
    }

    #[test]
    fn zero_dimensions_are_rejected() {
        for (w, h) in [(0, 600), (800, 0), (0, 0)] {
            assert_eq!(
                Viewport::try_new(w, h, 1.0).unwrap_err().reason,
                InvalidViewportReason::ZeroDimensions
            );
        }
    }

    #[test]
    fn bad_scale_factors_are_rejected() {
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let err = Viewport::try_new(10, 10, scale).unwrap_err();
            assert!(
                err.reason == InvalidViewportReason::NonFiniteScaleFactor
                    || err.reason == InvalidViewportReason::NonPositiveScaleFactor,
                "unexpected reason for scale {scale}"
            );
        }
        assert_eq!(
            Viewport::try_new(10, 10, f32::NAN).unwrap_err().reason,
            InvalidViewportReason::NonFiniteScaleFactor
        );
        assert_eq!(
            Viewport::try_new(10, 10, 0.0).unwrap_err().reason,
            InvalidViewportReason::NonPositiveScaleFactor
        );
    }

    #[test]
    fn pixel_count_is_bounded() {
        // Exactly at the cap: allowed by try_new…
        assert!(Viewport::try_new(16384, 16384, 1.0).is_ok());
        // …one row over: rejected without any allocation happening.
        let err = Viewport::try_new(16384, 16385, 1.0).unwrap_err();
        assert_eq!(err.reason, InvalidViewportReason::TooManyPixels);
        assert!(err.to_string().contains("maximum"));
    }

    #[test]
    fn fractional_scales_are_fine() {
        let vp = Viewport::try_new(750, 1111, 1.25).unwrap();
        assert_eq!(vp.scale_factor(), 1.25);
    }
}
