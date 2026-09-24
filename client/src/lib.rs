use bevy::{asset::AssetPlugin, prelude::*};

mod audio_settings;
mod battlefield_atmosphere;
mod bosses;
mod camera;
mod career;
mod career_identity;
mod combat;
mod combat_feedback;
mod combat_visuals;
mod creatures3d;
mod debug_console;
mod decor;
mod domain;
mod edge_hud;
mod frontend;
mod game_audio;
mod game_state;
mod game_vfx;
mod god_mode;
mod help_overlay;
mod humanoid;
mod input_bindings;
mod input_context;
mod jungle;
mod map_visuals;
mod maps;
mod match_hud;
mod match_service;
mod minimap;
mod minimap_route;
mod minions;
mod mobile_controls;
mod mobile_ui;
mod model_scale;
mod navigation;
mod net;
mod passport;
mod pause_menu;
mod persistence;
mod platform;
mod player;
mod plugins;
mod practice_sandbox;
mod presentation2d;
mod presentation3d;
mod projectile_visuals;
#[cfg(feature = "qa")]
mod qa;
mod reaction_visuals;
mod sandbox;
mod session_config;
mod shop;
mod skill_icons;
mod social;
mod sprite;
mod supporter;
mod supporter_storekit;
mod team;
mod team_vision;
mod ui;
mod ui_theme;
mod verdant3d;
mod world;
mod world2d;

pub(crate) use combat::targeting;

use plugins::{GameplayPlugins, NetPlugins, PresentationPlugins, UiPlugins};
use sprite::PlayerVisualMode;

#[bevy_main]
pub fn main() {
    shared::catalog::ensure_loaded();
    #[cfg(feature = "qa")]
    if let Some(directory) = std::env::var_os("OMOBA_ANIMATION_QA") {
        qa::animation_qa::run(directory.into());
        return;
    }
    if let Err(error) = sandbox::validate_launch() {
        eprintln!("Combat Test: {error}");
        return;
    }
    passport::initialize();
    // Headless model size analyzer (prints a bind-pose height table and exits).
    if std::env::var("OMOBA_MEASURE_MODELS").is_ok_and(|value| value == "1") {
        model_scale::run_model_measurement_analyzer();
        return;
    }
    let asset_root = shared::client_asset_root();
    eprintln!("Omoba asset root: {}", asset_root.display());
    let mut app = App::new();
    platform::configure_app(&mut app);
    // Purchased Ekza avatars live outside the (possibly read-only) bundle.
    // Asset sources must exist before AssetPlugin is added.
    if let Some(store_root) = passport::initialize_store() {
        app.register_asset_source(
            omoba_passport::store::ASSET_SOURCE,
            bevy::asset::io::AssetSourceBuilder::platform_default(
                &store_root.to_string_lossy(),
                None,
            ),
        );
    }
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: asset_root.to_string_lossy().into_owned(),
                ..default()
            })
            .set(platform::window_plugin()),
    )
    // The render backend is chosen once, before any plugin builds. After
    // `DefaultPlugins` so an invalid value's warning reaches the log.
    .insert_resource(PlayerVisualMode::from_environment())
    .add_plugins((NetPlugins, UiPlugins, GameplayPlugins, PresentationPlugins));
    #[cfg(feature = "qa")]
    app.add_plugins(qa::QaPlugins);
    app.run();
}
