//! # velqu-tailwind
//!
//! Owns the **Velqu Tailwind/CSS Profile** — the machine-readable description
//! of which CSS properties and feature areas VelquView commits to — plus
//! classification helpers. It must stay data + diagnostics: it is not a
//! second CSS engine, and VelquView never hard-codes Tailwind utility names
//! (the pipeline is Tailwind compiler → CSS → profile check → renderer).
//!
//! **M0 scope (this crate today):** the v0 profile manifest transcribed from
//! the spec (`docs/okf/specs/tailwind-profile-v0.md`) and property-level
//! classification.
//!
//! **M3 scope (not built yet):** real compiled-Tailwind ingestion, source
//! locations, suggested replacements, and the `velqu css check` prototype
//! built on this manifest.

use std::fmt;

/// How a CSS feature relates to the profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compatibility {
    /// Within the profile; rendered natively.
    Supported,
    /// Outside the v0 profile.
    Unsupported,
}

/// One capability area of the profile (layout, typography, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureArea {
    /// Short slug, e.g. `layout`.
    pub slug: &'static str,
    /// Human-readable area name.
    pub title: &'static str,
    /// CSS property names and feature tokens in this area.
    pub properties: &'static [&'static str],
}

/// The Velqu Tailwind/CSS Profile manifest.
#[derive(Debug, Clone, Copy)]
pub struct CssProfile {
    /// Profile identifier, e.g. `velqu-css-profile-v0`.
    pub id: &'static str,
    /// Capability areas.
    pub areas: &'static [FeatureArea],
    /// Features intentionally deferred beyond v0.
    pub deferred: &'static [&'static str],
}

/// Feature areas of profile v0, per the spec.
pub const PROFILE_V0: CssProfile = CssProfile {
    id: "velqu-css-profile-v0",
    areas: &[
        FeatureArea {
            slug: "layout",
            title: "Layout",
            properties: &[
                "display",
                "position",
                "top",
                "right",
                "bottom",
                "left",
                "gap",
                "row-gap",
                "column-gap",
                "flex-direction",
                "flex-wrap",
                "flex-grow",
                "flex-shrink",
                "flex-basis",
                "grid-template-columns",
                "grid-template-rows",
                "grid-column",
                "grid-row",
                "align-items",
                "align-self",
                "align-content",
                "justify-content",
                "justify-self",
                "justify-items",
                "width",
                "height",
                "min-width",
                "min-height",
                "max-width",
                "max-height",
                "margin",
                "padding",
                "box-sizing",
                "inset",
                "z-index",
                "overflow",
                "overflow-x",
                "overflow-y",
            ],
        },
        FeatureArea {
            slug: "visual",
            title: "Visual",
            properties: &[
                "background-color",
                "color",
                "border-width",
                "border-style",
                "border-color",
                "border-radius",
                "box-shadow",
                "opacity",
                "cursor",
                "pointer-events",
            ],
        },
        FeatureArea {
            slug: "typography",
            title: "Typography",
            properties: &[
                "font-family",
                "font-size",
                "font-weight",
                "line-height",
                "letter-spacing",
                "text-align",
                "text-overflow",
                "white-space",
                "text-decoration-line",
                "text-transform",
            ],
        },
        FeatureArea {
            slug: "state",
            title: "State",
            properties: &[":hover", ":focus", ":active", ":disabled"],
        },
        FeatureArea {
            slug: "theme",
            title: "Theme/Responsive",
            properties: &[
                "--*",
                "var()",
                "@media",
                "prefers-color-scheme",
                "prefers-reduced-motion",
            ],
        },
        FeatureArea {
            slug: "transform",
            title: "Transform",
            properties: &["transform", "translate", "scale", "rotate"],
        },
    ],
    deferred: &[
        "position: sticky",
        "filters / backdrop filters",
        "blend modes",
        "CSS masks",
        "complex 3D transforms",
        "container queries",
        "print CSS",
        "browser-specific appearance",
    ],
};

impl CssProfile {
    /// Classifies a CSS property or feature token against this profile.
    ///
    /// `--*` in the manifest matches any custom property
    /// (`--*` = custom properties, `var()` references).
    ///
    /// ```
    /// use velqu_tailwind::{classify, Compatibility};
    /// # fn main() {
    /// assert_eq!(classify("display"), Compatibility::Supported);
    /// assert_eq!(classify("--brand-500"), Compatibility::Supported);
    /// assert_eq!(classify("backdrop-filter"), Compatibility::Unsupported);
    /// # }
    /// ```
    pub fn classify(&self, property: &str) -> Compatibility {
        let normalized = property.trim().to_ascii_lowercase();
        let is_custom_property = normalized.starts_with("--");
        for area in self.areas {
            for known in area.properties {
                match *known {
                    "--*" if is_custom_property => return Compatibility::Supported,
                    "var()" if normalized.starts_with("var(") => return Compatibility::Supported,
                    _ if *known == normalized => return Compatibility::Supported,
                    _ => {}
                }
            }
        }
        Compatibility::Unsupported
    }
}

/// Classifies `property` against [`PROFILE_V0`].
pub fn classify(property: &str) -> Compatibility {
    PROFILE_V0.classify(property)
}

impl fmt::Display for Compatibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Compatibility::Supported => "supported",
            Compatibility::Unsupported => "unsupported",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_one_properties_are_supported() {
        for property in [
            "display",
            "flex-direction",
            "grid-template-columns",
            "gap",
            "width",
            "margin",
            "padding",
            "background-color",
            "color",
            "border-radius",
            "box-shadow",
            "opacity",
            "overflow",
            "font-size",
            "line-height",
            "text-align",
            "transform",
            ":hover",
            ":focus",
            "@media",
            "prefers-color-scheme",
        ] {
            assert_eq!(
                classify(property),
                Compatibility::Supported,
                "{property} should be in profile v0"
            );
        }
    }

    #[test]
    fn custom_properties_match_any_name() {
        assert_eq!(classify("--BRAND-500"), Compatibility::Supported);
        assert_eq!(classify("--velqu-radius"), Compatibility::Supported);
    }

    #[test]
    fn deferred_features_are_unsupported() {
        for property in [
            "backdrop-filter",
            "mix-blend-mode",
            "mask-image",
            "container-type",
            "-webkit-appearance",
        ] {
            assert_eq!(classify(property), Compatibility::Unsupported);
        }
    }

    #[test]
    fn classification_normalizes_input() {
        assert_eq!(classify("  FONT-SIZE "), Compatibility::Supported);
    }

    #[test]
    fn profile_lists_deferred_work() {
        assert!(PROFILE_V0.deferred.contains(&"position: sticky"));
        assert!(!PROFILE_V0.deferred.is_empty());
    }
}
