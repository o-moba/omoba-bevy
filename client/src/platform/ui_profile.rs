//! Compile-target interface policy; deliberately independent of windows and Bevy.

/// Interface family is a platform policy, independent of viewport and input
/// devices. A touch-screen laptop still uses the desktop interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UiProfile {
    Desktop,
    Mobile,
}

/// Resolve once at application initialization. `std::env::consts::OS` describes
/// the compiled Rust target, including when cross-compiling on a desktop host.
pub(crate) fn ui_profile() -> UiProfile {
    select_ui_profile(
        std::env::consts::OS,
        cfg!(debug_assertions),
        std::env::var("OMOBA_TOUCH_CONTROLS").as_deref() == Ok("1"),
    )
}

fn select_ui_profile(target_os: &str, development_build: bool, mobile_preview: bool) -> UiProfile {
    if matches!(target_os, "android" | "ios") || (development_build && mobile_preview) {
        UiProfile::Mobile
    } else {
        UiProfile::Desktop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_platform_wins_and_preview_is_development_only() {
        for target in ["android", "ios", "windows", "macos", "linux"] {
            for development in [false, true] {
                for preview in [false, true] {
                    let expected = match target {
                        "android" | "ios" => UiProfile::Mobile,
                        _ if development && preview => UiProfile::Mobile,
                        _ => UiProfile::Desktop,
                    };
                    assert_eq!(
                        select_ui_profile(target, development, preview),
                        expected,
                        "target={target} development={development} preview={preview}"
                    );
                }
            }
        }
    }
}
