//! # velqu-tailwind
//!
//! Owns the **Velqu Tailwind/CSS Profile** — the machine-readable description
//! of which CSS capabilities VelquView commits to — plus classification of
//! *parsed CSS concepts* (declarations, at-rules) against that profile. It
//! must stay data + diagnostics: it is not a second CSS engine, and VelquView
//! never hard-codes Tailwind utility names (the pipeline is Tailwind compiler
//! → CSS → profile check → renderer).
//!
//! **M0 scope:** the v0 profile manifest transcribed from the spec
//! (`docs/okf/specs/tailwind-profile-v0.md`).
//!
//! **M1.1 scope (ADR 0004):** classification operates on declaration
//! *values*, not just property names — `position: sticky` and complex 3D
//! transforms are distinguishable from their supported siblings — and the
//! three-tier result (`Supported` / `Normalized` / `Unsupported`) plus
//! replacement suggestions match what the M3 `velqu css check` must report.
//!
//! **M3 scope (not built yet):** real compiled-Tailwind ingestion, source
//! locations, and the checker CLI built on this model.

use std::fmt;

/// How a CSS concept relates to the profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compatibility {
    /// Within the profile; rendered natively as authored.
    Supported,
    /// Accepted, but VelquView's pipeline rewrites it into an equivalent
    /// supported form before rendering (e.g. color-space conversion).
    Normalized,
    /// Outside the v0 profile.
    Unsupported,
}

impl fmt::Display for Compatibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Compatibility::Supported => "supported",
            Compatibility::Normalized => "normalized",
            Compatibility::Unsupported => "unsupported",
        })
    }
}

/// The verdict for one classified CSS concept.
///
/// Mirrors what the M3 checker reports per construct: tier plus a suggested
/// replacement where one is known (spec: "supported, normalized, unsupported,
/// source location, and suggested replacement when known").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classification {
    /// Compatibility tier.
    pub compatibility: Compatibility,
    /// Known-good alternative or migration hint, if any.
    pub replacement: Option<&'static str>,
}

impl Classification {
    /// A plain verdict with no suggestion.
    pub const fn tier(compatibility: Compatibility) -> Self {
        Self {
            compatibility,
            replacement: None,
        }
    }
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
    /// Property-name-level classification against this profile.
    ///
    /// **Concept-level note:** this answers "is the property known at all",
    /// not "is this declaration supported" — [`classify_declaration`]
    /// distinguishes values (e.g. `position: sticky`). Use this for manifest
    /// membership questions only.
    ///
    /// `--*` in the manifest matches any custom property; `var()` matches
    /// `var(…)` values.
    pub fn classify_property(&self, property: &str) -> Compatibility {
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

/// Property-level classification against [`PROFILE_V0`].
///
/// ```
/// use velqu_tailwind::{classify_property, Compatibility};
/// assert_eq!(classify_property("display"), Compatibility::Supported);
/// assert_eq!(classify_property("--brand-500"), Compatibility::Supported);
/// assert_eq!(classify_property("backdrop-filter"), Compatibility::Unsupported);
/// ```
pub fn classify_property(property: &str) -> Compatibility {
    PROFILE_V0.classify_property(property)
}

/// CSS properties whose values are colors (for value-level color checks).
const COLOR_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "border-color",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "outline-color",
    "text-decoration-color",
];

/// Transform tokens that indicate complex 3D (deferred per profile v0).
const TRANSFORM_3D_TOKENS: &[&str] = &[
    "matrix3d",
    "translate3d",
    "scale3d",
    "rotate3d",
    "rotatex",
    "rotatey",
    "rotatez",
    "perspective",
];

/// Classifies one parsed declaration (property + value) against profile v0.
///
/// This is the concept-level check the M3 checker will call per declaration.
/// Property membership is the baseline; specific properties refine by value:
///
/// * `position` — `relative`/`absolute`/`static` supported; `sticky`
///   (deferred by the spec) and `fixed` unsupported with suggestions;
/// * `display` — the profile's block/inline/none/flex/grid values;
/// * `transform` — 2D translate/scale/rotate/matrix supported, 3D forms
///   deferred;
/// * color properties — wide-gamut functions (`oklch`, `oklab`,
///   `color(…)`) are **Normalized**: accepted and converted by the Velqu
///   pipeline rather than rejected (Tailwind v4 emits them by default).
///
/// ```
/// use velqu_tailwind::{classify_declaration, Compatibility};
///
/// let pos = classify_declaration("position", "sticky");
/// assert_eq!(pos.compatibility, Compatibility::Unsupported);
/// assert!(pos.replacement.is_some());
///
/// assert_eq!(
///     classify_declaration("position", "absolute").compatibility,
///     Compatibility::Supported
/// );
/// assert_eq!(
///     classify_declaration("color", "oklch(0.7 0.1 250)").compatibility,
///     Compatibility::Normalized
/// );
/// ```
pub fn classify_declaration(property: &str, value: &str) -> Classification {
    let property = property.trim().to_ascii_lowercase();
    let value = value.trim().to_ascii_lowercase();

    if property.starts_with("--") {
        return Classification::tier(Compatibility::Supported);
    }
    if PROFILE_V0.classify_property(&property) == Compatibility::Unsupported {
        return Classification::tier(Compatibility::Unsupported);
    }

    match property.as_str() {
        "position" => match value.as_str() {
            "relative" | "absolute" | "static" => Classification::tier(Compatibility::Supported),
            "sticky" => Classification {
                compatibility: Compatibility::Unsupported,
                replacement: Some(
                    "position: sticky is deferred in profile v0; restructure with a scroll \
                     container or fixed side layout",
                ),
            },
            "fixed" => Classification {
                compatibility: Compatibility::Unsupported,
                replacement: Some(
                    "position: fixed is deferred in profile v0; place the element at the \
                     document root with position: absolute",
                ),
            },
            _ => Classification::tier(Compatibility::Unsupported),
        },
        "display" => match value.as_str() {
            "block" | "inline" | "none" | "flex" | "grid" => {
                Classification::tier(Compatibility::Supported)
            }
            _ => Classification {
                compatibility: Compatibility::Unsupported,
                replacement: Some(
                    "display value outside profile v0 (block/inline/none/flex/grid); adjust \
                     layout structure",
                ),
            },
        },
        "transform" => {
            if TRANSFORM_3D_TOKENS
                .iter()
                .any(|token| value.contains(token))
            {
                Classification {
                    compatibility: Compatibility::Unsupported,
                    replacement: Some(
                        "complex 3D transforms are deferred in profile v0; use 2D \
                         translate/scale/rotate",
                    ),
                }
            } else {
                Classification::tier(Compatibility::Supported)
            }
        }
        _ if COLOR_PROPERTIES.contains(&property.as_str()) => {
            if value.contains("oklch(")
                || value.contains("oklab(")
                || value.contains("color(")
                || value.contains("lab(")
                || value.contains("lch(")
            {
                Classification {
                    compatibility: Compatibility::Normalized,
                    replacement: Some(
                        "wide-gamut color; the Velqu pipeline converts it to sRGB (velqu css \
                         check reports the converted value)",
                    ),
                }
            } else {
                Classification::tier(Compatibility::Supported)
            }
        }
        _ => Classification::tier(Compatibility::Supported),
    }
}

/// Classifies one parsed at-rule by name (with or without `@`).
///
/// ```
/// use velqu_tailwind::{classify_at_rule, Compatibility};
/// assert_eq!(
///     classify_at_rule("media").compatibility,
///     Compatibility::Supported
/// );
/// assert_eq!(
///     classify_at_rule("keyframes").compatibility,
///     Compatibility::Unsupported
/// );
/// ```
pub fn classify_at_rule(name: &str) -> Classification {
    let name = name.trim().trim_start_matches('@').to_ascii_lowercase();
    match name.as_str() {
        "media" => Classification::tier(Compatibility::Supported),
        "keyframes" => Classification {
            compatibility: Compatibility::Unsupported,
            replacement: Some(
                "CSS animations are outside profile v0; drive motion from state bindings or a \
                 future native animation primitive",
            ),
        },
        "font-face" => Classification {
            compatibility: Compatibility::Unsupported,
            replacement: Some(
                "@font-face is outside profile v0; use the bundled fonts or wait for host font \
                 loading",
            ),
        },
        "supports" | "container" | "layer" | "import" | "scope" => Classification {
            compatibility: Compatibility::Unsupported,
            replacement: Some("at-rule outside profile v0; flatten at build time"),
        },
        _ => Classification::tier(Compatibility::Unsupported),
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
                classify_property(property),
                Compatibility::Supported,
                "{property} should be in profile v0"
            );
        }
    }

    #[test]
    fn custom_properties_match_any_name() {
        assert_eq!(classify_property("--BRAND-500"), Compatibility::Supported);
        assert_eq!(
            classify_property("--velqu-radius"),
            Compatibility::Supported
        );
    }

    #[test]
    fn deferred_properties_are_unsupported() {
        for property in [
            "backdrop-filter",
            "mix-blend-mode",
            "mask-image",
            "container-type",
            "-webkit-appearance",
        ] {
            assert_eq!(classify_property(property), Compatibility::Unsupported);
        }
    }

    #[test]
    fn classification_normalizes_input() {
        assert_eq!(classify_property("  FONT-SIZE "), Compatibility::Supported);
    }

    #[test]
    fn profile_lists_deferred_work() {
        assert!(PROFILE_V0.deferred.contains(&"position: sticky"));
        assert!(!PROFILE_V0.deferred.is_empty());
    }

    #[test]
    fn position_values_are_distinguished() {
        for value in ["relative", "absolute", "static"] {
            assert_eq!(
                classify_declaration("position", value).compatibility,
                Compatibility::Supported,
                "position: {value}"
            );
        }
        let sticky = classify_declaration("position", "sticky");
        assert_eq!(sticky.compatibility, Compatibility::Unsupported);
        assert!(sticky.replacement.unwrap().contains("sticky"));
        let fixed = classify_declaration("position", " FIXED ");
        assert_eq!(fixed.compatibility, Compatibility::Unsupported);
    }

    #[test]
    fn transform_values_are_distinguished() {
        assert_eq!(
            classify_declaration("transform", "translate(4px, 8px)").compatibility,
            Compatibility::Supported
        );
        assert_eq!(
            classify_declaration("transform", "rotate(45deg) scale(1.5)").compatibility,
            Compatibility::Supported
        );
        for value in [
            "translate3d(0,0,4px)",
            "matrix3d(1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1)",
        ] {
            let verdict = classify_declaration("transform", value);
            assert_eq!(verdict.compatibility, Compatibility::Unsupported);
            assert!(verdict.replacement.is_some());
        }
    }

    #[test]
    fn display_values_follow_the_profile() {
        for value in ["block", "inline", "none", "flex", "grid"] {
            assert_eq!(
                classify_declaration("display", value).compatibility,
                Compatibility::Supported
            );
        }
        assert_eq!(
            classify_declaration("display", "inline-block").compatibility,
            Compatibility::Unsupported
        );
    }

    #[test]
    fn wide_gamut_colors_are_normalized_not_rejected() {
        for property in ["color", "background-color", "border-color"] {
            assert_eq!(
                classify_declaration(property, "oklch(0.7 0.1 250)").compatibility,
                Compatibility::Normalized,
                "{property}"
            );
        }
        let verdict = classify_declaration("background-color", "oklch(0.21 0.006 285.9)");
        assert!(verdict.replacement.unwrap().contains("sRGB"));
        // Plain sRGB values stay plain-supported.
        assert_eq!(
            classify_declaration("color", "#10141c").compatibility,
            Compatibility::Supported
        );
        assert_eq!(
            classify_declaration("color", "rgb(59 130 246)").compatibility,
            Compatibility::Supported
        );
    }

    #[test]
    fn unknown_property_is_unsupported_regardless_of_value() {
        let verdict = classify_declaration("backdrop-filter", "blur(4px)");
        assert_eq!(verdict.compatibility, Compatibility::Unsupported);
        assert_eq!(verdict.replacement, None);
    }

    #[test]
    fn custom_property_declarations_are_supported() {
        assert_eq!(
            classify_declaration("--brand", "#3b82f6").compatibility,
            Compatibility::Supported
        );
    }

    #[test]
    fn at_rules_classify_with_names_not_sigils() {
        assert_eq!(
            classify_at_rule("@media").compatibility,
            Compatibility::Supported
        );
        assert_eq!(
            classify_at_rule("Media").compatibility,
            Compatibility::Supported
        );
        for name in ["keyframes", "font-face", "supports", "container"] {
            let verdict = classify_at_rule(name);
            assert_eq!(verdict.compatibility, Compatibility::Unsupported);
            assert!(
                verdict.replacement.is_some(),
                "{name} should explain itself"
            );
        }
    }
}
