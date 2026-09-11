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
    WindowPlugin::default()
}

/// The OS-owned writable directory for settings, excluding the filename.
/// The explicit OMOBA_CLIENT_CONFIG_DIR override remains in persistence.rs.
pub fn preferences_directory() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        bevy::android::ANDROID_APP
            .get()
            .and_then(|app| app.internal_data_path())
            .map(|base| base.join("omoba-bevy"))
    }
    #[cfg(target_os = "ios")]
    {
        // iOS HOME is the app sandbox; Application Support survives relaunches and
        // keeps preferences out of the signed, read-only .app bundle.
        std::env::var_os("HOME").map(|base| {
            PathBuf::from(base)
                .join("Library")
                .join("Application Support")
                .join("omoba-bevy")
        })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join("omoba-bevy"))
    }
    #[cfg(not(any(target_os = "android", target_os = "ios", target_os = "windows")))]
    {
        std::env::var_os("HOME").map(|base| PathBuf::from(base).join(".config").join("omoba-bevy"))
    }
}

/// Explicit test/launcher overrides win; an empty override keeps OS-private storage.
/// This pure boundary avoids changing process environment in parallel ECS tests.
pub fn preferences_file_path(
    override_directory: Option<&str>,
    platform_directory: Option<PathBuf>,
    filename: &str,
) -> Option<PathBuf> {
    override_directory
        .map(str::trim)
        .filter(|directory| !directory.is_empty())
        .map(PathBuf::from)
        .or(platform_directory)
        .map(|directory| directory.join(filename))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
