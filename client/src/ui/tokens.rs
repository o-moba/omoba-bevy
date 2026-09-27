//! Verdant Crown design tokens, typed.
//!
//! The data is `client/ui/tokens/verdant-crown.json`, a byte copy of
//! `omoba-ui/handoff/tokens.json` kept in sync by `scripts/sync_ui_tokens.py`
//! (DECISIONS R4: tokens stay data). `client/build.rs` generates the
//! constants below from it (`build/ui_tokens.rs` documents the naming), so a
//! token change is a data change and a missing token is a compile error.
//! Kit code names tokens (`color::SURFACE_1`, `size::BUTTON_HEIGHT.at(form)`,
//! `TextRole::Button`); it does not hold colour or size literals.
#![allow(dead_code)] // The generated table covers every token; screens adopt them step by step.

use bevy::color::Color;

use super::theme::metric::Form;

/// A size with a desktop and a phone value (`<key>.desktop` / `<key>.phone`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Metric {
    pub desktop: f32,
    pub phone: f32,
}

impl Metric {
    pub const fn new(desktop: f32, phone: f32) -> Self {
        Self { desktop, phone }
    }

    /// The value for a layout family.
    pub const fn at(self, form: Form) -> f32 {
        match form {
            Form::Desktop => self.desktop,
            Form::Phone => self.phone,
        }
    }
}

/// A CSS `cubic-bezier(x1, y1, x2, y2)` easing from `motion.easing.*`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CubicBezier {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl CubicBezier {
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// Eased progress for linear progress `t` in `0..=1` (CSS semantics: the
    /// curve's x is time; solve x(s) = t by bisection, return y(s)).
    pub fn ease(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let bezier = |a: f32, b: f32, s: f32| {
            let inverse = 1.0 - s;
            3.0 * inverse * inverse * s * a + 3.0 * inverse * s * s * b + s * s * s
        };
        let (mut low, mut high) = (0.0_f32, 1.0_f32);
        for _ in 0..24 {
            let middle = (low + high) * 0.5;
            if bezier(self.x1, self.x2, middle) < t {
                low = middle;
            } else {
                high = middle;
            }
        }
        bezier(self.y1, self.y2, (low + high) * 0.5)
    }
}

/// `type.<style>.case`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextCase {
    /// Latin upper case; CJK is never transformed.
    Upper,
    /// As written (`none`).
    AsWritten,
}

/// A named type style (`type.<style>.*`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TypeStyle {
    pub family: FontFamily,
    pub size: Metric,
    /// Multiplier of the font size.
    pub line_height: f32,
    /// Tracking in em. Bevy 0.18 text has no letter spacing; kept as data
    /// (see `docs/ui-kit.md`, deviations).
    pub letter_spacing_em: f32,
    pub case: TextCase,
    /// The face that replaces `family` when the text needs CJK glyphs.
    pub cjk_family: FontFamily,
    pub cjk_letter_spacing_em: f32,
}

include!(concat!(env!("OUT_DIR"), "/ui_tokens.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> serde_json::Map<String, serde_json::Value> {
        serde_json::from_str(include_str!("../../ui/tokens/verdant-crown.json")).unwrap()
    }

    fn hex(color: Color) -> String {
        let srgba = color.to_srgba();
        let channel = |value: f32| (value * 255.0).round() as u8;
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            channel(srgba.red),
            channel(srgba.green),
            channel(srgba.blue),
            channel(srgba.alpha)
        )
    }

    /// Every key of the JSON is in exactly one generated table with the same
    /// value, and the tables hold nothing else: the constants are the data.
    #[test]
    fn generated_tokens_round_trip_the_json() {
        let data = data();
        let mut seen = std::collections::BTreeSet::new();
        for (key, color) in COLOR_TOKENS {
            assert_eq!(data[*key].as_str().unwrap(), hex(*color), "{key}");
            assert!(seen.insert(*key), "{key} generated twice");
        }
        for (key, value) in NUMBER_TOKENS {
            let expected = data[*key].as_f64().unwrap() as f32;
            assert!(
                (expected - value).abs() < 1e-6,
                "{key}: {value} != {expected}"
            );
            assert!(seen.insert(*key), "{key} generated twice");
        }
        for (key, value) in TEXT_TOKENS {
            assert_eq!(data[*key].as_str().unwrap(), *value, "{key}");
            assert!(seen.insert(*key), "{key} generated twice");
        }
        let keys: std::collections::BTreeSet<&str> = data
            .keys()
            .map(String::as_str)
            .filter(|key| !key.starts_with('$'))
            .collect();
        assert_eq!(keys, seen, "every token is typed, and only tokens are");
    }

    #[test]
    fn metrics_resolve_by_layout_family() {
        assert_eq!(size::BUTTON_HEIGHT.at(Form::Desktop), 46.0);
        assert_eq!(size::BUTTON_HEIGHT.at(Form::Phone), 44.0);
        assert_eq!(size::BUTTON_LG_MIN_WIDTH.at(Form::Phone), 200.0);
        assert_eq!(space::SCREEN_MARGIN.at(Form::Phone), 16.0);
        assert_eq!(size::ABILITY_ATTACK_PHONE, 96.0);
        assert_eq!(size::BUTTON_MIN_WIDTH, 120.0);
        assert_eq!(space::S8, 8.0);
        let title = TextRole::TitleXl.style();
        assert_eq!(title.size.at(Form::Desktop), 48.0);
        assert_eq!(title.size.at(Form::Phone), 32.0);
        assert_eq!(title.family, FontFamily::DisplayBold);
        assert_eq!(title.cjk_family, FontFamily::CjkDisplay);
        assert_eq!(title.case, TextCase::Upper);
        assert_eq!(TextRole::Body.style().case, TextCase::AsWritten);
        assert_eq!(TextRole::Body.style().cjk_family, FontFamily::CjkBody);
        // Numbers keep Barlow in Chinese: digits are Latin.
        assert_eq!(
            TextRole::NumberXl.style().cjk_family,
            FontFamily::NumberBold
        );
        assert_eq!(TextRole::ALL.len(), 14);
    }

    #[test]
    fn families_load_from_the_installed_fonts() {
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        for family in FontFamily::ALL {
            assert!(
                assets.join(family.asset_path()).is_file(),
                "{:?} {}",
                family,
                family.asset_path()
            );
            assert_eq!(FontFamily::from_token(family.token()), Some(family));
        }
        assert_eq!(
            FontFamily::CjkBody.asset_path(),
            "ui/NotoSansCJKsc-Regular.otf"
        );
        assert_eq!(
            FontFamily::Display.asset_path(),
            "ui/verdant/fonts/Cinzel-SemiBold.ttf"
        );
    }

    #[test]
    fn easings_follow_css_cubic_bezier() {
        for easing in [
            motion::EASING_STANDARD,
            motion::EASING_ENTER,
            motion::EASING_EXIT,
        ] {
            assert!(easing.ease(0.0).abs() < 1e-3);
            assert!((easing.ease(1.0) - 1.0).abs() < 1e-3);
            let mut last = 0.0;
            for step in 1..=20 {
                let value = easing.ease(step as f32 / 20.0);
                assert!(value + 1e-4 >= last, "monotonic");
                last = value;
            }
        }
        // Standard decelerates: well past half-way at the half-time mark.
        assert!(motion::EASING_STANDARD.ease(0.5) > 0.7);
        assert_eq!(
            motion::DURATION_TOAST_HOLD,
            std::time::Duration::from_secs(3)
        );
    }
}
