//! Deterministic text measurement and greedy line breaking.
//!
//! M2a keeps the M1 font path (bundled DejaVu, fontdue scalar rasterizer) so
//! layout math and pixels stay byte-reproducible. The supported text subset
//! is deliberately narrow and documented: **Latin/UI text with
//! space-separated words** — no complex shaping, no bidi, no hyphenation.
//! Script coverage beyond that is an M2b+ concern behind this same seam.

//! consumed by the box tree/layout stages in the next commit.
#![allow(dead_code)]

use crate::font::FontStore;
use crate::scene::FontWeight;

/// One laid-out line: its text plus measured width in device pixels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Line {
    pub text: String,
    pub width: f32,
}

/// Collapses CSS `white-space: normal` whitespace: runs of whitespace become
/// single spaces and the result is trimmed.
pub(crate) fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            in_space = !out.is_empty();
        } else {
            if in_space {
                out.push(' ');
                in_space = false;
            }
            out.push(ch);
        }
    }
    out
}

/// Measures one run's advance width in device pixels.
pub(crate) fn measure_run(fonts: &mut FontStore, text: &str, px: u16, weight: FontWeight) -> f32 {
    text.chars()
        .map(|ch| fonts.glyph(weight, ch, px).advance)
        .sum()
}

/// Greedy word wrap. `\n` forces a break; a word wider than `max_width`
/// occupies its own line (no hyphenation — overflow is allowed and visible).
pub(crate) fn wrap_lines(
    fonts: &mut FontStore,
    text: &str,
    max_width: f32,
    px: u16,
    weight: FontWeight,
) -> Vec<Line> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let collapsed = collapse_whitespace(paragraph);
        let words: Vec<&str> = collapsed.split_ascii_whitespace().collect();
        if words.is_empty() {
            if !paragraph.is_empty() || lines.is_empty() {
                lines.push(Line {
                    text: String::new(),
                    width: 0.0,
                });
            }
            continue;
        }
        let space_width = measure_run(fonts, " ", px, weight);
        let mut current = String::new();
        let mut current_width = 0.0;
        for word in words {
            let word_width = measure_run(fonts, word, px, weight);
            if current.is_empty() {
                current = word.to_owned();
                current_width = word_width;
                continue;
            }
            if current_width + space_width + word_width <= max_width {
                current.push(' ');
                current.push_str(word);
                current_width += space_width + word_width;
            } else {
                lines.push(Line {
                    width: current_width,
                    text: std::mem::take(&mut current),
                });
                current = word.to_owned();
                current_width = word_width;
            }
        }
        if !current.is_empty() || lines.is_empty() {
            lines.push(Line {
                width: current_width,
                text: current,
            });
        }
    }
    lines
}

/// Maps a CSS numeric font-weight to the bundled face.
pub(crate) fn face_weight(font_weight: u16) -> FontWeight {
    if font_weight >= 600 {
        FontWeight::Bold
    } else {
        FontWeight::Regular
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measurer() -> FontStore {
        FontStore::bundled()
    }

    #[test]
    fn collapse_runs_and_trim() {
        assert_eq!(collapse_whitespace("  a \n\t b  "), "a b");
        assert_eq!(collapse_whitespace("plain"), "plain");
        assert_eq!(collapse_whitespace("   "), "");
    }

    #[test]
    fn measurement_is_positive_and_deterministic() {
        let mut fonts = measurer();
        let a = measure_run(&mut fonts, "Hello Velqu", 16, FontWeight::Regular);
        let b = measure_run(&mut fonts, "Hello Velqu", 16, FontWeight::Regular);
        assert!(a > 0.0);
        assert_eq!(a, b);
        // Bold is wider than regular for DejaVu.
        let bold = measure_run(&mut fonts, "Hello Velqu", 16, FontWeight::Bold);
        assert!(bold > a);
        assert_eq!(measure_run(&mut fonts, "", 16, FontWeight::Regular), 0.0);
    }

    #[test]
    fn wrapping_breaks_at_word_boundaries() {
        let mut fonts = measurer();
        // Chosen so two words fit and the third wraps.
        let width = measure_run(&mut fonts, "one two", 16, FontWeight::Regular);
        let lines = wrap_lines(&mut fonts, "one two three", width, 16, FontWeight::Regular);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "one two");
        assert_eq!(lines[1].text, "three");
    }

    #[test]
    fn hard_breaks_always_split() {
        let mut fonts = measurer();
        let lines = wrap_lines(&mut fonts, "a\nb\nc", 1000.0, 16, FontWeight::Regular);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].text, "b");
    }

    #[test]
    fn oversized_word_gets_its_own_line() {
        let mut fonts = measurer();
        let lines = wrap_lines(
            &mut fonts,
            "tiny enormousword",
            10.0,
            16,
            FontWeight::Regular,
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "tiny");
        assert_eq!(lines[1].text, "enormousword");
    }

    #[test]
    fn empty_text_yields_single_empty_line() {
        let mut fonts = measurer();
        let lines = wrap_lines(&mut fonts, "", 100.0, 16, FontWeight::Regular);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].width, 0.0);
    }

    #[test]
    fn weight_mapping_threshold() {
        assert_eq!(face_weight(400), FontWeight::Regular);
        assert_eq!(face_weight(500), FontWeight::Regular);
        assert_eq!(face_weight(600), FontWeight::Bold);
        assert_eq!(face_weight(700), FontWeight::Bold);
    }
}
