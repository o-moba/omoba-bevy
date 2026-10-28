//! Native platform boundaries: packaged assets, writable preferences, and window policy.
//!
//! Android enters through `#[bevy_main]` in the library, so its application handle is
//! available before these functions run. Preferences never write into packaged assets.

use bevy::prelude::*;
use std::path::PathBuf;

mod ui_profile;
pub(crate) use ui_profile::{UiProfile, ui_profile};

pub fn configure_app(app: &mut App) {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    app.insert_resource(bevy::winit::WinitSettings::mobile());
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let _ = app;
}

pub fn window_plugin() -> WindowPlugin {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        WindowPlugin {
            primary_window: Some(Window {
                title: "Omoba Beta".to_string(),
                resizable: false,
                mode: bevy::window::WindowMode::BorderlessFullscreen(MonitorSelection::Primary),
                // VSync and a 60Hz reactive loop bound work when the app is idle.
                present_mode: bevy::window::PresentMode::AutoVsync,
                prefers_home_indicator_hidden: true,
                prefers_status_bar_hidden: true,
                preferred_screen_edges_deferring_system_gestures: bevy::window::ScreenEdge::Bottom,
                ..default()
            }),
            ..default()
        }
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    match desktop_window_title(CONFIG_DIR_SUFFIX) {
        // A contour build says so in its title bar; production keeps Bevy's default window.
        Some(title) => WindowPlugin {
            primary_window: Some(Window { title, ..default() }),
            ..default()
        },
        None => WindowPlugin::default(),
    }
}

/// `OMOBA DEV` for a dev-contour desktop build, `None` (unchanged default) for production.
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
fn desktop_window_title(suffix: Option<&str>) -> Option<String> {
    suffix
        .filter(|suffix| !suffix.is_empty())
        .map(|suffix| format!("OMOBA {}", suffix.to_ascii_uppercase()))
}

/// Build-time suffix for the default settings directory. A dev-contour tester
/// build (`scripts/release.py build <platform> --contour dev`) sets `dev`, so it
/// keeps its session, saved server, identity and caches in `omoba-bevy-dev` and
/// never reads or overwrites a production install's `omoba-bevy`. Unset or empty
/// keeps the production name. A runtime `OMOBA_CLIENT_CONFIG_DIR` still wins.
const CONFIG_DIR_SUFFIX: Option<&str> = option_env!("OMOBA_DEFAULT_CONFIG_DIR_SUFFIX");

// An invalid suffix fails the build instead of silently sharing production state.
const _: () = assert!(
    valid_config_dir_suffix(CONFIG_DIR_SUFFIX),
    "OMOBA_DEFAULT_CONFIG_DIR_SUFFIX must be 1-16 lowercase ASCII letters, digits or inner '-'"
);

/// One path component: lowercase ASCII letters, digits and inner hyphens, at most 16 bytes.
const fn valid_config_dir_suffix(suffix: Option<&str>) -> bool {
    let Some(suffix) = suffix else {
        return true;
    };
    let bytes = suffix.as_bytes();
    if bytes.is_empty() {
        return true;
    }
    if bytes.len() > 16 || bytes[0] == b'-' || bytes[bytes.len() - 1] == b'-' {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-') {
            return false;
        }
        index += 1;
    }
    true
}

/// `omoba-bevy`, then `-<market>` for a market other than global
/// (`shared::market::Market::storage_suffix`), then `-<suffix>` for a contour
/// build: `omoba-bevy`, `omoba-bevy-dev`, `omoba-bevy-ru`, `omoba-bevy-ru-dev`.
/// Each market has its own account service, so its folder never holds another
/// market's device keys or caches.
fn config_directory_name(market: shared::market::Market, suffix: Option<&str>) -> String {
    let mut name = "omoba-bevy".to_owned();
    for part in [
        market.storage_suffix(),
        suffix.filter(|suffix| !suffix.is_empty()),
    ]
    .into_iter()
    .flatten()
    {
        name.push('-');
        name.push_str(part);
    }
    name
}

/// The OS-owned writable directory for settings, excluding the filename.
/// The explicit OMOBA_CLIENT_CONFIG_DIR override remains in persistence.rs.
pub fn preferences_directory() -> Option<PathBuf> {
    preferences_directory_for(shared::region::market())
}

pub(crate) fn preferences_directory_for(market: shared::market::Market) -> Option<PathBuf> {
    let name = config_directory_name(market, CONFIG_DIR_SUFFIX);
    #[cfg(target_os = "android")]
    {
        bevy::android::ANDROID_APP
            .get()
            .and_then(|app| app.internal_data_path())
            .map(|base| base.join(name))
    }
    #[cfg(target_os = "ios")]
    {
        // iOS HOME is the app sandbox; Application Support survives relaunches and
        // keeps preferences out of the signed, read-only .app bundle.
        std::env::var_os("HOME").map(|base| {
            PathBuf::from(base)
                .join("Library")
                .join("Application Support")
                .join(name)
        })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join(name))
    }
    #[cfg(not(any(target_os = "android", target_os = "ios", target_os = "windows")))]
    {
        std::env::var_os("HOME").map(|base| PathBuf::from(base).join(".config").join(name))
    }
}

/// Explicit test/launcher overrides win; an empty override keeps OS-private storage.
/// This pure boundary avoids changing process environment in parallel ECS tests.
pub fn preferences_file_path(
    override_directory: Option<&str>,
    platform_directory: Option<PathBuf>,
    filename: &str,
) -> Option<PathBuf> {
    runtime_directory(override_directory, platform_directory)
        .map(|directory| directory.join(filename))
}

/// The settings folder: a non-empty override, else the OS-private one.
pub fn settings_directory_from(
    override_directory: Option<&str>,
    platform_directory: Option<PathBuf>,
) -> Option<PathBuf> {
    override_directory
        .map(str::trim)
        .filter(|directory| !directory.is_empty())
        .map(PathBuf::from)
        .or(platform_directory)
}

/// This process's settings folder (`OMOBA_CLIENT_CONFIG_DIR`, else the OS one).
fn runtime_directory(
    override_directory: Option<&str>,
    platform_directory: Option<PathBuf>,
) -> Option<PathBuf> {
    let directory = settings_directory_from(override_directory, platform_directory)?;
    if override_directory.is_some_and(|s| !s.trim().is_empty()) {
        Some(crate::account_region::override_directory(
            directory,
            shared::region::RegionId::from_market(shared::region::market())
                .unwrap_or(shared::region::RegionId::Global),
            shared::market::MARKET,
        ))
    } else {
        Some(directory)
    }
}

pub fn settings_directory() -> Option<PathBuf> {
    runtime_directory(
        std::env::var("OMOBA_CLIENT_CONFIG_DIR").ok().as_deref(),
        preferences_directory(),
    )
}

/// Whether this platform can start a fresh copy of the game itself (desktop);
/// a phone app is closed and reopened by the player.
pub const CAN_RELAUNCH: bool = !cfg!(any(target_os = "android", target_os = "ios"));

/// Starts a fresh copy of this game with the same arguments, so a choice that
/// applies at startup (the connection profile) takes effect; the caller exits.
pub fn relaunch() -> std::io::Result<()> {
    if !CAN_RELAUNCH {
        return Err(std::io::Error::from(std::io::ErrorKind::Unsupported));
    }
    let executable = std::env::current_exe()?;
    // The new process outlives this one; nothing waits for it.
    let _child = std::process::Command::new(executable)
        .args(std::env::args_os().skip(1))
        .spawn()?;
    Ok(())
}

/// The game owns native browser presentation; the SDK still validates the URL.
pub fn open_external_url(raw: &str) -> Result<(), String> {
    let url = ekza_bevy_sdk::passport::pairing::safe_url_with_query(raw)?;
    #[cfg(target_os = "ios")]
    {
        let url = std::ffi::CString::new(url).map_err(|_| "Invalid browser link")?;
        unsafe extern "C" {
            fn omoba_browser_open(url: *const std::ffi::c_char);
        }
        // Swift copies the string synchronously, then opens UIKit on the main queue.
        unsafe { omoba_browser_open(url.as_ptr()) };
        Ok(())
    }
    #[cfg(not(target_os = "ios"))]
    omoba_passport::open_in_browser(&url)
}

pub fn browser_approval_hint() -> &'static str {
    #[cfg(target_os = "ios")]
    {
        unsafe extern "C" {
            fn omoba_browser_failed() -> bool;
        }
        if unsafe { omoba_browser_failed() } {
            return crate::i18n::tr("account.browser.failed");
        }
    }
    crate::i18n::tr("account.browser.confirm")
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::market::Market;

    #[test]
    fn preferences_override_and_private_storage_are_separate_from_assets() {
        let private = PathBuf::from("/data/user/0/space.ekza.omoba.beta/files/omoba-bevy");
        assert_eq!(
            preferences_file_path(None, Some(private.clone()), "prefs.json"),
            Some(private.join("prefs.json"))
        );
        assert_eq!(
            preferences_file_path(Some("  /tmp/Omoba QA  "), Some(private), "prefs.json"),
            Some(PathBuf::from("/tmp/Omoba QA/prefs.json"))
        );
    }

    #[test]
    fn blank_override_uses_private_storage_and_missing_storage_stays_unavailable() {
        let private = PathBuf::from("/app/Library/Application Support/omoba-bevy");
        assert_eq!(
            preferences_file_path(Some(" \t"), Some(private.clone()), "prefs.json"),
            Some(private.join("prefs.json"))
        );
        // Never fall back into the installed bundle when an OS directory is missing.
        assert_eq!(preferences_file_path(Some(""), None, "prefs.json"), None);
        assert_eq!(preferences_file_path(None, None, "prefs.json"), None);
    }

    #[test]
    fn the_settings_directory_follows_the_same_override_rule() {
        let private = PathBuf::from("/app/Library/Application Support/omoba-bevy-dev");
        assert_eq!(
            settings_directory_from(None, Some(private.clone())),
            Some(private.clone())
        );
        assert_eq!(
            settings_directory_from(Some(" /tmp/qa "), Some(private.clone())),
            Some(PathBuf::from("/tmp/qa"))
        );
        assert_eq!(
            settings_directory_from(Some(""), Some(private.clone())),
            Some(private)
        );
        assert_eq!(settings_directory_from(None, None), None);
        assert_eq!(
            CAN_RELAUNCH,
            !cfg!(any(target_os = "android", target_os = "ios"))
        );
    }

    #[test]
    fn production_builds_keep_the_original_settings_directory() {
        assert_eq!(config_directory_name(Market::Global, None), "omoba-bevy");
        assert_eq!(
            config_directory_name(Market::Global, Some("")),
            "omoba-bevy"
        );
    }

    #[test]
    fn a_contour_build_gets_its_own_settings_directory() {
        assert_eq!(
            config_directory_name(Market::Global, Some("dev")),
            "omoba-bevy-dev"
        );
        assert_ne!(
            config_directory_name(Market::Global, Some("dev")),
            config_directory_name(Market::Global, None)
        );
    }

    #[test]
    fn each_market_gets_its_own_settings_directory_before_the_contour() {
        assert_eq!(config_directory_name(Market::Ru, None), "omoba-bevy-ru");
        assert_eq!(config_directory_name(Market::Ru, Some("")), "omoba-bevy-ru");
        assert_eq!(
            config_directory_name(Market::Ru, Some("dev")),
            "omoba-bevy-ru-dev"
        );
        assert_eq!(config_directory_name(Market::Cn, None), "omoba-bevy-cn");
        assert_eq!(
            config_directory_name(Market::Cn, Some("dev")),
            "omoba-bevy-cn-dev"
        );
        let names: std::collections::BTreeSet<_> = Market::ALL
            .into_iter()
            .flat_map(|market| {
                [None, Some("dev")].map(|suffix| config_directory_name(market, suffix))
            })
            .collect();
        assert_eq!(names.len(), Market::ALL.len() * 2);
    }

    #[test]
    fn only_a_contour_build_marks_its_window_title() {
        assert_eq!(desktop_window_title(None), None);
        assert_eq!(desktop_window_title(Some("")), None);
        assert_eq!(
            desktop_window_title(Some("dev")).as_deref(),
            Some("OMOBA DEV")
        );
    }

    #[test]
    fn config_dir_suffix_is_one_safe_path_component() {
        for good in [
            None,
            Some(""),
            Some("dev"),
            Some("dev-2"),
            Some("abcdefghij012345"),
        ] {
            assert!(valid_config_dir_suffix(good), "{good:?}");
        }
        for bad in [
            "../prod",
            "a/b",
            "a\\b",
            "Dev",
            "-dev",
            "dev-",
            "dev ",
            ".",
            "abcdefghij0123456",
        ] {
            assert!(!valid_config_dir_suffix(Some(bad)), "{bad:?}");
        }
    }

    #[test]
    fn preferences_directory_uses_the_compiled_suffix() {
        // Matches whatever this test binary was compiled with: `omoba-bevy` normally,
        // `omoba-bevy-dev` when OMOBA_DEFAULT_CONFIG_DIR_SUFFIX=dev, `omoba-bevy-ru`
        // when OMOBA_MARKET=ru.
        let expected = config_directory_name(
            shared::region::market(),
            option_env!("OMOBA_DEFAULT_CONFIG_DIR_SUFFIX"),
        );
        if let Some(directory) = preferences_directory() {
            assert_eq!(
                directory.file_name().and_then(|name| name.to_str()),
                Some(expected.as_str())
            );
        }
    }
}
