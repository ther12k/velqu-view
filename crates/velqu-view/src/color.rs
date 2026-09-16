//! Colors used by the renderer and exposed to applications as plain data.

use std::fmt;

/// An 8-bit RGBA color. Plain data: no color-space metadata in M1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Color {
    /// Red component, 0–255.
    pub r: u8,
    /// Green component, 0–255.
    pub g: u8,
    /// Blue component, 0–255.
    pub b: u8,
    /// Alpha component, 0–255, where 255 is opaque.
    pub a: u8,
}

impl Color {
    /// Fully transparent black.
    pub const TRANSPARENT: Color = Color::from_rgba8(0, 0, 0, 0);
    /// Opaque black (`#000000`).
    pub const BLACK: Color = Color::from_rgb8(0, 0, 0);
    /// Opaque white (`#FFFFFF`).
    pub const WHITE: Color = Color::from_rgb8(0xFF, 0xFF, 0xFF);

    /// Creates an opaque color from 8-bit RGB components.
    pub const fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self::from_rgba8(r, g, b, 0xFF)
    }

    /// Creates a color from 8-bit RGBA components.
    pub const fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Parses `#RRGGBB`, `#RRGGBBAA` (with or without the leading `#`).
    ///
    /// This is the same shape CSS uses, so fixture expectations can be copied
    /// straight from stylesheets.
    pub fn from_hex(s: &str) -> Result<Self, ColorParseError> {
        let hex = s.strip_prefix('#').unwrap_or(s);
        if hex.len() != 6 && hex.len() != 8 {
            return Err(ColorParseError {
                input: s.to_owned(),
                reason: ColorParseErrorReason::Length,
            });
        }
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ColorParseError {
                input: s.to_owned(),
                reason: ColorParseErrorReason::HexDigit,
            });
        }
        let channel = |lo: usize| u8::from_str_radix(&hex[lo..lo + 2], 16).unwrap_or(0);
        let (r, g, b, a) = match hex.len() {
            6 => (channel(0), channel(2), channel(4), 0xFF),
            _ => (channel(0), channel(2), channel(4), channel(6)),
        };
        Ok(Self { r, g, b, a })
    }

    /// Composites `self` over `dst` (source-over), the only blend M1 needs.
    pub fn blend_over(self, dst: Color) -> Color {
        if self.a == 0xFF {
            return self;
        }
        if self.a == 0 {
            return dst;
        }
        let sa = self.a as u32;
        let da = dst.a as u32;
        let out_a = sa + da * (255 - sa) / 255;
        let mix = |sc: u8, dc: u8| {
            let v = (sc as u32 * sa + dc as u32 * da * (255 - sa) / 255) / out_a.max(1);
            v.min(255) as u8
        };
        Color::from_rgba8(
            mix(self.r, dst.r),
            mix(self.g, dst.g),
            mix(self.b, dst.b),
            out_a as u8,
        )
    }

    /// The color as a `0xAARRGGBB` u32 (big-endian numeric form).
    ///
    /// Shell backends convert to whatever their surface expects from this.
    pub fn to_argb32(self) -> u32 {
        (self.a as u32) << 24 | (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if self.a != 0xFF {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

/// Why a hex color failed to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorParseErrorReason {
    /// The string was not 6 or 8 hex digits.
    Length,
    /// A non-hex digit was found.
    HexDigit,
}

/// A failed [`Color::from_hex`] parse, echoing the offending input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorParseError {
    /// The rejected input string.
    pub input: String,
    /// Why it was rejected.
    pub reason: ColorParseErrorReason,
}

impl fmt::Display for ColorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.reason {
            ColorParseErrorReason::Length => write!(
                f,
                "invalid color {:?}: expected 6 or 8 hex digits (#RRGGBB[AA])",
                self.input
            ),
            ColorParseErrorReason::HexDigit => {
                write!(f, "invalid color {:?}: non-hex digit found", self.input)
            }
        }
    }
}

impl std::error::Error for ColorParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(
            Color::from_hex("#10141C").unwrap(),
            Color::from_rgb8(0x10, 0x14, 0x1c)
        );
        assert_eq!(
            Color::from_hex("3B82F6").unwrap(),
            Color::from_rgb8(0x3b, 0x82, 0xf6)
        );
        assert_eq!(
            Color::from_hex("#3B82F680").unwrap(),
            Color::from_rgba8(0x3b, 0x82, 0xf6, 0x80)
        );
    }

    #[test]
    fn hex_errors_carry_reasons() {
        assert_eq!(
            Color::from_hex("#12345").unwrap_err().reason,
            ColorParseErrorReason::Length
        );
        assert_eq!(
            Color::from_hex("#12345G").unwrap_err().reason,
            ColorParseErrorReason::HexDigit
        );
    }

    #[test]
    fn display_round_trips_through_parse() {
        let c = Color::from_rgba8(0x12, 0x34, 0x56, 0x78);
        assert_eq!(Color::from_hex(&c.to_string()).unwrap(), c);
        let opaque = Color::from_rgb8(1, 2, 3);
        assert_eq!(Color::from_hex(&opaque.to_string()).unwrap(), opaque);
    }

    #[test]
    fn blend_over_semantics() {
        let red = Color::from_rgb8(255, 0, 0);
        let blue = Color::from_rgb8(0, 0, 255);
        assert_eq!(red.blend_over(blue), red);
        assert_eq!(Color::TRANSPARENT.blend_over(blue), blue);
        // 50% red over blue stays within channel bounds.
        let half = Color::from_rgba8(255, 0, 0, 128);
        let mixed = half.blend_over(blue);
        assert_eq!(mixed.a, 255);
        assert_eq!((mixed.r, mixed.b), (128, 127));
    }

    #[test]
    fn argb32_layout() {
        assert_eq!(Color::from_rgb8(0x10, 0x14, 0x1c).to_argb32(), 0xFF10141C);
    }
}
