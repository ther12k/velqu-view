//! The Tailwind v3 default color palette, verbatim, as `(shade, hex)`
//! tables. v0 of the Velqu pipeline embeds the palette so utility classes
//! (`bg-rose-600`) compile without a network or node toolchain; the
//! profile check still applies to everything a declaration carries.

/// One color family: name plus shades 50–950 (11 entries, in Tailwind's
/// ascending order).
pub struct Family {
    /// Tailwind family name (`bg-<name>-<shade>`).
    pub name: &'static str,
    /// Hex values for shades `50, 100, 200, 300, 400, 500, 600, 700, 800,
    /// 900, 950`.
    pub shades: [&'static str; 11],
}

/// Shade index for a Tailwind shade number.
///
/// The shade is written as 2–3 digits (`50`, `950`); anything else is not
/// in the default palette.
pub fn shade_index(shade: &str) -> Option<usize> {
    match shade {
        "50" => Some(0),
        "100" => Some(1),
        "200" => Some(2),
        "300" => Some(3),
        "400" => Some(4),
        "500" => Some(5),
        "600" => Some(6),
        "700" => Some(7),
        "800" => Some(8),
        "900" => Some(9),
        "950" => Some(10),
        _ => None,
    }
}

/// The 22 default Tailwind v3 families, alphabetical.
pub const FAMILIES: &[Family] = &[
    Family {
        name: "amber",
        shades: [
            "#fffbeb", "#fef3c7", "#fde68a", "#fcd34d", "#fbbf24", "#f59e0b", "#d97706", "#b45309",
            "#92400e", "#78350f", "#451a03",
        ],
    },
    Family {
        name: "blue",
        shades: [
            "#eff6ff", "#dbeafe", "#bfdbfe", "#93c5fd", "#60a5fa", "#3b82f6", "#2563eb", "#1d4ed8",
            "#1e40af", "#1e3a8a", "#172554",
        ],
    },
    Family {
        name: "cyan",
        shades: [
            "#ecfeff", "#cffafe", "#a5f3fc", "#67e8f9", "#22d3ee", "#06b6d4", "#0891b2", "#0e7490",
            "#155e75", "#164e63", "#083344",
        ],
    },
    Family {
        name: "emerald",
        shades: [
            "#ecfdf5", "#d1fae5", "#a7f3d0", "#6ee7b7", "#34d399", "#10b981", "#059669", "#047857",
            "#065f46", "#064e3b", "#022c22",
        ],
    },
    Family {
        name: "fuchsia",
        shades: [
            "#fdf4ff", "#fae8ff", "#f5d0fe", "#f0abfc", "#e879f9", "#d946ef", "#c026d3", "#a21caf",
            "#86198f", "#701a75", "#4a044e",
        ],
    },
    Family {
        name: "gray",
        shades: [
            "#f9fafb", "#f3f4f6", "#e5e7eb", "#d1d5db", "#9ca3af", "#6b7280", "#4b5563", "#374151",
            "#1f2937", "#111827", "#030712",
        ],
    },
    Family {
        name: "green",
        shades: [
            "#f0fdf4", "#dcfce7", "#bbf7d0", "#86efac", "#4ade80", "#22c55e", "#16a34a", "#15803d",
            "#166534", "#14532d", "#052e16",
        ],
    },
    Family {
        name: "indigo",
        shades: [
            "#eef2ff", "#e0e7ff", "#c7d2fe", "#a5b4fc", "#818cf8", "#6366f1", "#4f46e5", "#4338ca",
            "#3730a3", "#312e81", "#1e1b4b",
        ],
    },
    Family {
        name: "lime",
        shades: [
            "#f7fee7", "#ecfccb", "#d9f99d", "#bef264", "#a3e635", "#84cc16", "#65a30d", "#4d7c0f",
            "#3f6212", "#365314", "#1a2e05",
        ],
    },
    Family {
        name: "neutral",
        shades: [
            "#fafafa", "#f5f5f5", "#e5e5e5", "#d4d4d4", "#a3a3a3", "#737373", "#525252", "#404040",
            "#262626", "#171717", "#0a0a0a",
        ],
    },
    Family {
        name: "orange",
        shades: [
            "#fff7ed", "#ffedd5", "#fed7aa", "#fdba74", "#fb923c", "#f97316", "#ea580c", "#c2410c",
            "#9a3412", "#7c2d12", "#431407",
        ],
    },
    Family {
        name: "pink",
        shades: [
            "#fdf2f8", "#fce7f3", "#fbcfe8", "#f9a8d4", "#f472b6", "#ec4899", "#db2777", "#be185d",
            "#9d174d", "#831843", "#500724",
        ],
    },
    Family {
        name: "purple",
        shades: [
            "#faf5ff", "#f3e8ff", "#e9d5ff", "#d8b4fe", "#c084fc", "#a855f7", "#9333ea", "#7e22ce",
            "#6b21a8", "#581c87", "#3b0764",
        ],
    },
    Family {
        name: "red",
        shades: [
            "#fef2f2", "#fee2e2", "#fecaca", "#fca5a5", "#f87171", "#ef4444", "#dc2626", "#b91c1c",
            "#991b1b", "#7f1d1d", "#450a0a",
        ],
    },
    Family {
        name: "rose",
        shades: [
            "#fff1f2", "#ffe4e6", "#fecdd3", "#fda4af", "#fb7185", "#f43f5e", "#e11d48", "#be123c",
            "#9f1239", "#881337", "#4c0519",
        ],
    },
    Family {
        name: "sky",
        shades: [
            "#f0f9ff", "#e0f2fe", "#bae6fd", "#7dd3fc", "#38bdf8", "#0ea5e9", "#0284c7", "#0369a1",
            "#075985", "#0c4a6e", "#082f49",
        ],
    },
    Family {
        name: "slate",
        shades: [
            "#f8fafc", "#f1f5f9", "#e2e8f0", "#cbd5e1", "#94a3b8", "#64748b", "#475569", "#334155",
            "#1e293b", "#0f172a", "#020617",
        ],
    },
    Family {
        name: "stone",
        shades: [
            "#fafaf9", "#f5f5f4", "#e7e5e4", "#d6d3d1", "#a8a29e", "#78716c", "#57534e", "#44403c",
            "#292524", "#1c1917", "#0c0a09",
        ],
    },
    Family {
        name: "teal",
        shades: [
            "#f0fdfa", "#ccfbf1", "#99f6e4", "#5eead4", "#2dd4bf", "#14b8a6", "#0d9488", "#0f766e",
            "#115e59", "#134e4a", "#042f2e",
        ],
    },
    Family {
        name: "violet",
        shades: [
            "#f5f3ff", "#ede9fe", "#ddd6fe", "#c4b5fd", "#a78bfa", "#8b5cf6", "#7c3aed", "#6d28d9",
            "#5b21b6", "#4c1d95", "#2e1065",
        ],
    },
    Family {
        name: "yellow",
        shades: [
            "#fefce8", "#fef9c3", "#fef08a", "#fde047", "#facc15", "#eab308", "#ca8a04", "#a16207",
            "#854d0e", "#713f12", "#422006",
        ],
    },
    Family {
        name: "zinc",
        shades: [
            "#fafafa", "#f4f4f5", "#e4e4e7", "#d4d4d8", "#a1a1aa", "#71717a", "#52525b", "#3f3f46",
            "#27272a", "#18181b", "#09090b",
        ],
    },
];

/// Looks up `<name>-<shade>` (e.g. `rose-600`) in the default palette.
pub fn lookup(name_shade: &str) -> Option<&'static str> {
    let (name, shade) = name_shade.split_once('-')?;
    let index = shade_index(shade)?;
    let family = FAMILIES.iter().find(|f| f.name == name)?;
    Some(family.shades[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_colors_match_tailwind_v3() {
        assert_eq!(lookup("red-500"), Some("#ef4444"));
        assert_eq!(lookup("green-500"), Some("#22c55e"));
        assert_eq!(lookup("blue-500"), Some("#3b82f6"));
        assert_eq!(lookup("slate-900"), Some("#0f172a"));
        assert_eq!(lookup("amber-200"), Some("#fde68a"));
        assert_eq!(lookup("sky-500"), Some("#0ea5e9"));
    }

    #[test]
    fn unknown_families_and_shades_are_none() {
        assert_eq!(lookup("chartreuse-500"), None);
        assert_eq!(lookup("red-550"), None);
        assert_eq!(lookup("red"), None);
    }

    #[test]
    fn every_family_has_eleven_hex_shades() {
        for family in FAMILIES {
            assert_eq!(family.shades.len(), 11, "{}", family.name);
            for hex in family.shades {
                assert_eq!(hex.len(), 7, "{} {}", family.name, hex);
                assert!(hex.starts_with('#'));
            }
        }
    }
}
