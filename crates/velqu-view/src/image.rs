//! Bounded, deterministic image decoding (M2c-a).
//!
//! Hosts provide encoded bytes through the [`crate::AssetResolver`] seam;
//! Velqu decodes them **only** through this module, under explicit resource
//! limits ([`ImageLimits`], ADR 0008) — the same defense-in-depth posture as
//! [`crate::Viewport`] for frame buffers. Decoded output is RGBA8, tightly
//! packed, and a function of the input bytes alone.
//!
//! Profile rules (ADR 0008):
//! * only PNG and JPEG decoders are compiled (`image` with
//!   `default-features = false`); anything else is an
//!   [`ImageFailure::UnsupportedFormat`] — SVG/animated formats are deferred;
//! * encoded size, decoded dimensions, and decoded pixel count are all
//!   bounded *before* a full-size decode allocation happens (the header is
//!   read first via `into_dimensions`);
//! * decode is single-threaded (no `rayon`) and deterministic.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use ::image::{ImageFormat, ImageReader};

/// Upper bound on encoded bytes accepted for one image by default.
///
/// Deliberately generous for local app UI assets while keeping a hostile
/// asset from forcing a large read into the decoder pipeline.
pub const DEFAULT_MAX_ENCODED_BYTES: usize = 32 * 1024 * 1024;

/// Upper bound on either decoded dimension by default (16384 px, matching
/// the largest viewport [`crate::Viewport`] accepts on one axis).
pub const DEFAULT_MAX_DIMENSION: u32 = 16384;

/// Upper bound on decoded pixels by default (2^28 = a 1 GiB RGBA8 buffer,
/// matching [`crate::MAX_PIXELS`]).
pub const DEFAULT_MAX_PIXELS: u64 = 1 << 28;

/// CSS default object size, in logical px, for an image with no intrinsic
/// dimensions — a broken or missing asset keeps a deterministic layout
/// footprint instead of collapsing to zero.
pub(crate) const DEFAULT_OBJECT_SIZE: (f32, f32) = (300.0, 150.0);

/// Resource limits applied to every image decode (ADR 0008).
///
/// Images are untrusted input sized by their header, so limits are enforced
/// independently of the viewport: an oversized asset becomes a broken image
/// with a deterministic diagnostic, never an unbounded allocation.
///
/// Construct via [`ImageLimits::try_new`], which rejects zero-valued budgets;
/// fields are private so limits cannot be invalidated after construction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageLimits {
    max_encoded_bytes: usize,
    max_decoded_width: u32,
    max_decoded_height: u32,
    max_decoded_pixels: u64,
}

impl ImageLimits {
    /// Creates limits, validating that no budget is zero (a zero budget
    /// would reject every image and is always a configuration mistake).
    pub fn try_new(
        max_encoded_bytes: usize,
        max_decoded_width: u32,
        max_decoded_height: u32,
        max_decoded_pixels: u64,
    ) -> Result<Self, InvalidImageLimits> {
        if max_encoded_bytes == 0 {
            return Err(InvalidImageLimits {
                reason: InvalidImageLimitsReason::ZeroEncodedBytes,
            });
        }
        if max_decoded_width == 0 || max_decoded_height == 0 {
            return Err(InvalidImageLimits {
                reason: InvalidImageLimitsReason::ZeroDecodedDimension,
            });
        }
        if max_decoded_pixels == 0 {
            return Err(InvalidImageLimits {
                reason: InvalidImageLimitsReason::ZeroDecodedPixels,
            });
        }
        Ok(Self {
            max_encoded_bytes,
            max_decoded_width,
            max_decoded_height,
            max_decoded_pixels,
        })
    }

    /// Largest accepted encoded payload, in bytes.
    pub fn max_encoded_bytes(&self) -> usize {
        self.max_encoded_bytes
    }

    /// Largest accepted decoded width, in pixels.
    pub fn max_decoded_width(&self) -> u32 {
        self.max_decoded_width
    }

    /// Largest accepted decoded height, in pixels.
    pub fn max_decoded_height(&self) -> u32 {
        self.max_decoded_height
    }

    /// Largest accepted decoded pixel count (`width * height`).
    pub fn max_decoded_pixels(&self) -> u64 {
        self.max_decoded_pixels
    }
}

impl Default for ImageLimits {
    fn default() -> Self {
        Self::try_new(
            DEFAULT_MAX_ENCODED_BYTES,
            DEFAULT_MAX_DIMENSION,
            DEFAULT_MAX_DIMENSION,
            DEFAULT_MAX_PIXELS,
        )
        .expect("documented default limits are valid")
    }
}

/// Why [`ImageLimits::try_new`] rejected a limit set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidImageLimitsReason {
    /// The encoded-byte budget was zero.
    ZeroEncodedBytes,
    /// A decoded-dimension cap (width or height) was zero.
    ZeroDecodedDimension,
    /// The decoded-pixel-count cap was zero.
    ZeroDecodedPixels,
}

impl fmt::Display for InvalidImageLimitsReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            InvalidImageLimitsReason::ZeroEncodedBytes => {
                "max encoded bytes must be > 0 (a zero budget rejects every image)"
            }
            InvalidImageLimitsReason::ZeroDecodedDimension => "decoded dimension caps must be > 0",
            InvalidImageLimitsReason::ZeroDecodedPixels => "the decoded pixel cap must be > 0",
        })
    }
}

/// A limit set rejected by [`ImageLimits::try_new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidImageLimits {
    /// Which invariant failed.
    pub reason: InvalidImageLimitsReason,
}

impl fmt::Display for InvalidImageLimits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid image limits: {}", self.reason)
    }
}

impl std::error::Error for InvalidImageLimits {}

/// One decoded image: dimensions plus tightly packed RGBA8 pixels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl DecodedImage {
    /// The intrinsic aspect ratio (`width / height`), or `None` for a
    /// degenerate zero-area image.
    pub(crate) fn aspect_ratio(&self) -> Option<f32> {
        if self.width == 0 || self.height == 0 {
            None
        } else {
            Some(self.width as f32 / self.height as f32)
        }
    }
}

/// Why an image could not be used. Every variant is deterministic for a
/// given (bytes, limits) pair and becomes a diagnostic; the element renders
/// as a broken image regardless of variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImageFailure {
    /// The host resolver returned nothing for the reference.
    Missing,
    /// The bytes are neither PNG nor JPEG (the M2c-supported formats).
    UnsupportedFormat,
    /// The encoded payload exceeds `max_encoded_bytes`.
    EncodedTooLarge,
    /// The header declares dimensions beyond the per-axis caps.
    OverDimensions,
    /// The header-declared pixel count exceeds `max_decoded_pixels`.
    OverPixels,
    /// The decoder rejected the bitstream (corrupt or truncated data).
    DecodeFailed,
}

impl ImageFailure {
    /// Deterministic human-readable reason for diagnostics.
    pub(crate) fn message(&self) -> &'static str {
        match self {
            ImageFailure::Missing => "not provided by the host asset resolver",
            ImageFailure::UnsupportedFormat => {
                "unsupported image format (M2c profile supports PNG and JPEG)"
            }
            ImageFailure::EncodedTooLarge => "encoded size exceeds the image byte limit",
            ImageFailure::OverDimensions => "declared dimensions exceed the image size limits",
            ImageFailure::OverPixels => "declared pixel count exceeds the image pixel limit",
            ImageFailure::DecodeFailed => "image data is corrupt or truncated",
        }
    }
}

/// A resolved-and-decoded image, or the deterministic reason it failed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImageEntry {
    Loaded(Rc<DecodedImage>),
    Failed(ImageFailure),
}

/// Per-`src` decoded-image cache. Decoding happens once per asset per
/// document load; every later frame reuses the decoded pixels, so repeated
/// rendering never re-pays decode.
#[derive(Debug, Default)]
pub(crate) struct ImageStore {
    entries: HashMap<String, ImageEntry>,
}

impl ImageStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Drops every entry (called when the document source is replaced: image
    /// identity is per-document, like the DOM).
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn get(&self, src: &str) -> Option<&ImageEntry> {
        self.entries.get(src)
    }

    /// Records that the host resolver returned nothing for `src`.
    pub(crate) fn mark_missing(&mut self, src: &str) {
        self.entries
            .insert(src.to_owned(), ImageEntry::Failed(ImageFailure::Missing));
    }

    /// Decodes `bytes` under `limits` and caches the outcome for `src`
    /// (successes and failures alike, so a broken asset is diagnosed once).
    /// Decodes `bytes` under `limits` and caches the outcome for `src`
    /// (successes and failures alike, so a broken asset is diagnosed once).
    /// Returns the failure, if any, for diagnostics.
    pub(crate) fn load(
        &mut self,
        src: &str,
        bytes: &[u8],
        limits: &ImageLimits,
    ) -> Option<ImageFailure> {
        let entry = match decode_bounded(bytes, limits) {
            Ok(image) => ImageEntry::Loaded(Rc::new(image)),
            Err(failure) => ImageEntry::Failed(failure.clone()),
        };
        let failure = match &entry {
            ImageEntry::Loaded(_) => None,
            ImageEntry::Failed(failure) => Some(failure.clone()),
        };
        self.entries.insert(src.to_owned(), entry);
        failure
    }
}

/// Detects the M2c-supported formats by magic bytes. Format detection is
/// Velqu's job (the resolver only fetches), and explicit detection keeps the
/// "unsupported format" failure deterministic instead of decoder-guessed.
fn detect_format(bytes: &[u8]) -> Option<ImageFormat> {
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    const JPEG_MAGIC: [u8; 3] = [0xff, 0xd8, 0xff];
    if bytes.starts_with(&PNG_MAGIC) {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&JPEG_MAGIC) {
        Some(ImageFormat::Jpeg)
    } else {
        None
    }
}

/// Decodes `bytes` under `limits`, refusing before any large allocation
/// when a bound is exceeded. The header is read first (cheap, no pixel
/// buffers) so dimension/pixel limits reject oversized assets without
/// decoding them.
pub(crate) fn decode_bounded(
    bytes: &[u8],
    limits: &ImageLimits,
) -> Result<DecodedImage, ImageFailure> {
    if bytes.len() > limits.max_encoded_bytes() {
        return Err(ImageFailure::EncodedTooLarge);
    }
    let Some(format) = detect_format(bytes) else {
        return Err(ImageFailure::UnsupportedFormat);
    };

    // Pass 1: header only. Rejects over-limit dimensions and pixel counts
    // before the decoder allocates any pixel buffer.
    let header_reader = ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let (width, height) = match header_reader.into_dimensions() {
        Ok(dims) => dims,
        Err(err) => {
            return Err(match err {
                ::image::ImageError::Limits(limit) => match limit.kind() {
                    ::image::error::LimitErrorKind::InsufficientMemory => ImageFailure::OverPixels,
                    _ => ImageFailure::OverDimensions,
                },
                _ => ImageFailure::DecodeFailed,
            });
        }
    };
    if width == 0 || height == 0 {
        // Degenerate images have no intrinsic size or ratio: broken.
        return Err(ImageFailure::DecodeFailed);
    }
    if u64::from(width) * u64::from(height) > limits.max_decoded_pixels() {
        return Err(ImageFailure::OverPixels);
    }

    // Pass 2: full decode. The decoder's own limits stay on as defense in
    // depth (decoders may allocate intermediate buffers beyond the output).
    let mut reader = ImageReader::with_format(std::io::Cursor::new(bytes), format);
    // `image::Limits` is #[non_exhaustive]: mutate the defaults instead of
    // constructing a literal.
    let mut ilimits = ::image::Limits::default();
    ilimits.max_image_width = Some(limits.max_decoded_width());
    ilimits.max_image_height = Some(limits.max_decoded_height());
    ilimits.max_alloc = Some(limits.max_decoded_pixels() * 4);
    reader.limits(ilimits);
    let dynamic = match reader.decode() {
        Ok(image) => image,
        Err(err) => {
            return Err(match err {
                ::image::ImageError::Limits(_) => ImageFailure::OverDimensions,
                _ => ImageFailure::DecodeFailed,
            });
        }
    };
    let rgba = dynamic.to_rgba8();
    Ok(DecodedImage {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

// `::image` (extern crate) is spelled with a leading `::` because a
// crate-root `mod image` shadows the crate name in plain paths.

/// Encodes a solid-color PNG for tests in other modules (deterministic
/// bytes; the visual fixture harness uses the same shape of input).
#[cfg(test)]
pub(crate) fn test_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let img = ::image::RgbaImage::from_pixel(width, height, ::image::Rgba(rgba));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png)
        .expect("png encode succeeds");
    out.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encode(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let img = ::image::RgbaImage::from_fn(width, height, |x, y| {
            ::image::Rgba([(x * 7 % 256) as u8, (y * 13 % 256) as u8, 0x40, 0xff])
        });
        // JPEG has no alpha channel: the encoder rejects Rgba8 buffers.
        let mut out = Cursor::new(Vec::new());
        match format {
            ImageFormat::Jpeg => {
                ::image::DynamicImage::ImageRgba8(img)
                    .to_rgb8()
                    .write_to(&mut out, format)
                    .expect("encode succeeds");
            }
            _ => img.write_to(&mut out, format).expect("encode succeeds"),
        }
        out.into_inner()
    }

    #[test]
    fn png_decodes_to_expected_dimensions_and_pixels() {
        let bytes = encode(ImageFormat::Png, 7, 5);
        let decoded = decode_bounded(&bytes, &ImageLimits::default()).unwrap();
        assert_eq!((decoded.width, decoded.height), (7, 5));
        assert_eq!(decoded.rgba.len(), 7 * 5 * 4);
        // Opaque output: the decoder expands to RGBA with full alpha here.
        assert_eq!(decoded.rgba[3], 0xff);
        assert_eq!(decoded.aspect_ratio(), Some(1.4));
    }

    #[test]
    fn jpeg_decodes_to_expected_dimensions() {
        let bytes = encode(ImageFormat::Jpeg, 9, 4);
        let decoded = decode_bounded(&bytes, &ImageLimits::default()).unwrap();
        assert_eq!((decoded.width, decoded.height), (9, 4));
        assert_eq!(decoded.rgba.len(), 9 * 4 * 4);
    }

    #[test]
    fn decoding_is_deterministic() {
        let bytes = encode(ImageFormat::Png, 11, 6);
        let a = decode_bounded(&bytes, &ImageLimits::default()).unwrap();
        let b = decode_bounded(&bytes, &ImageLimits::default()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn unknown_magic_is_an_unsupported_format() {
        assert_eq!(
            decode_bounded(b"not an image at all", &ImageLimits::default()).unwrap_err(),
            ImageFailure::UnsupportedFormat
        );
        // JPEG magic truncated to two bytes: not detectable, not decodable.
        assert_eq!(
            decode_bounded(&[0xff, 0xd8], &ImageLimits::default()).unwrap_err(),
            ImageFailure::UnsupportedFormat
        );
    }

    #[test]
    fn corrupt_payload_after_valid_magic_is_a_decode_failure() {
        let mut bytes = encode(ImageFormat::Png, 4, 4);
        bytes.truncate(20); // valid signature, broken body
        assert_eq!(
            decode_bounded(&bytes, &ImageLimits::default()).unwrap_err(),
            ImageFailure::DecodeFailed
        );
    }

    #[test]
    fn encoded_byte_limit_refuses_before_any_decode() {
        let bytes = encode(ImageFormat::Png, 4, 4);
        let tight = ImageLimits::try_new(bytes.len() - 1, 4096, 4096, 1 << 20).unwrap();
        assert_eq!(
            decode_bounded(&bytes, &tight).unwrap_err(),
            ImageFailure::EncodedTooLarge
        );
    }

    #[test]
    fn width_limit_refuses_on_the_header_without_decoding() {
        let bytes = encode(ImageFormat::Png, 7, 5);
        let tight = ImageLimits::try_new(bytes.len(), 6, 4096, 1 << 20).unwrap();
        assert_eq!(
            decode_bounded(&bytes, &tight).unwrap_err(),
            ImageFailure::OverDimensions
        );
    }

    #[test]
    fn pixel_count_limit_refuses_square_enough_images() {
        let bytes = encode(ImageFormat::Png, 7, 5); // 35 px
        let tight = ImageLimits::try_new(bytes.len(), 4096, 4096, 34).unwrap();
        assert_eq!(
            decode_bounded(&bytes, &tight).unwrap_err(),
            ImageFailure::OverPixels
        );
        // Exactly at the cap is allowed.
        let exact = ImageLimits::try_new(bytes.len(), 4096, 4096, 35).unwrap();
        assert!(decode_bounded(&bytes, &exact).is_ok());
    }

    #[test]
    fn degenerate_dimensions_are_a_decode_failure() {
        // A valid 0×0 PNG: IHDR with zero dimensions.
        let bytes: &[u8] = &[
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
            b'D', b'R', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1d, 0x87, 0x7d, 0x51,
        ];
        assert_eq!(
            decode_bounded(bytes, &ImageLimits::default()).unwrap_err(),
            ImageFailure::DecodeFailed
        );
    }

    #[test]
    fn limits_reject_zero_budgets() {
        assert_eq!(
            ImageLimits::try_new(0, 10, 10, 100).unwrap_err().reason,
            InvalidImageLimitsReason::ZeroEncodedBytes
        );
        assert_eq!(
            ImageLimits::try_new(10, 0, 10, 100).unwrap_err().reason,
            InvalidImageLimitsReason::ZeroDecodedDimension
        );
        assert_eq!(
            ImageLimits::try_new(10, 10, 0, 100).unwrap_err().reason,
            InvalidImageLimitsReason::ZeroDecodedDimension
        );
        assert_eq!(
            ImageLimits::try_new(10, 10, 10, 0).unwrap_err().reason,
            InvalidImageLimitsReason::ZeroDecodedPixels
        );
        assert!(ImageLimits::try_new(1, 1, 1, 1).is_ok());
    }

    #[test]
    fn store_caches_successes_and_failures_per_src() {
        let mut store = ImageStore::new();
        let bytes = encode(ImageFormat::Png, 3, 2);
        store.load("a.png", &bytes, &ImageLimits::default());
        store.load("broken.png", b"junk", &ImageLimits::default());
        store.mark_missing("gone.png");

        assert!(matches!(
            store.get("a.png"),
            Some(ImageEntry::Loaded(image)) if (image.width, image.height) == (3, 2)
        ));
        assert_eq!(
            store.get("broken.png"),
            Some(&ImageEntry::Failed(ImageFailure::UnsupportedFormat))
        );
        assert_eq!(
            store.get("gone.png"),
            Some(&ImageEntry::Failed(ImageFailure::Missing))
        );
        assert!(store.get("other.png").is_none());

        // Re-loading the same src replaces the entry (asset replacement).
        store.load("broken.png", &bytes, &ImageLimits::default());
        assert!(matches!(
            store.get("broken.png"),
            Some(ImageEntry::Loaded(_))
        ));

        store.clear();
        assert!(store.get("a.png").is_none());
    }

    #[test]
    fn failure_messages_are_distinct() {
        let messages = [
            ImageFailure::Missing.message(),
            ImageFailure::UnsupportedFormat.message(),
            ImageFailure::EncodedTooLarge.message(),
            ImageFailure::OverDimensions.message(),
            ImageFailure::OverPixels.message(),
            ImageFailure::DecodeFailed.message(),
        ];
        for (i, a) in messages.iter().enumerate() {
            for b in &messages[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
