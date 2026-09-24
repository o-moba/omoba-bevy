use bevy::{asset::AssetPlugin, prelude::*};

mod animation_qa;
mod audio_qa;
mod audio_settings;
mod battlefield_atmosphere;
mod beta_ui_qa;
mod bosses;
mod camera;
mod career;
mod career_identity;
mod career_visual_qa;
mod combat;
mod combat_feedback;
mod combat_qa;
mod combat_visuals;
mod creatures3d;
mod debug_console;
mod decor;
mod domain;
mod edge_hud;
mod forest_pickup_qa;
mod frontend;
mod frontend_flow_qa;
mod frontend_qa;
mod game_audio;
mod game_state;
mod game_vfx;
mod god_mode;
mod help_overlay;
mod humanoid;
mod input_bindings;
mod input_context;
mod jungle;
mod map_qa;
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
mod navigation_qa;
mod net;
mod offline_qa;
mod passport;
mod pause_menu;
mod persistence;
mod platform;
mod player;
mod practice_sandbox;
mod presentation2d;
mod presentation3d;
mod projectile_visuals;
mod reaction_visuals;
mod sandbox;
mod session_config;
mod shop;
mod skill_icons;
mod social;
mod social_qa;
mod sprite;
mod supporter;
mod supporter_storekit;
mod targeting_qa;
mod team;
mod team_vision;
mod team_vision_qa;
mod ui;
mod ui_theme;
mod verdant3d;
mod visual_qa;
mod world;
mod world2d;

pub(crate) use combat::targeting;

use bosses::BossesPlugin;
use camera::CameraPlugin;
use combat::CombatPlugin;
use debug_console::DebugConsolePlugin;
use decor::DecorPlugin;
use frontend::FrontendPlugin;
use game_state::GameStateUiPlugin;
use god_mode::GodModePlugin;
use help_overlay::HelpOverlayPlugin;
use maps::MapsPlugin;
use match_hud::MatchHudPlugin;
use minimap::MinimapPlugin;
use minions::MinionVisualsPlugin;
use model_scale::ModelScalePlugin;
use net::NetworkingPlugin;
use pause_menu::PauseMenuPlugin;
use persistence::ClientPersistencePlugin;
use player::PlayerPlugin;
use presentation2d::Presentation2dPlugin;
use sprite::SpriteVisualsPlugin;
use team::TeamSelectPlugin;
use world::SetupPlugin;
use world2d::World2dPlugin;

#[bevy_main]
pub fn main() {
    if let Some(directory) = std::env::var_os("OMOBA_ANIMATION_QA") {
        animation_qa::run(directory.into());
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
    .add_plugins((
        CameraPlugin,
        PlayerPlugin,
        SpriteVisualsPlugin,
        Presentation2dPlugin,
        MapsPlugin,
        ClientPersistencePlugin,
        SetupPlugin,
        World2dPlugin,
        NetworkingPlugin,
        BossesPlugin,
        MinimapPlugin,
        CombatPlugin,
        MatchHudPlugin,
        TeamSelectPlugin,
        GameStateUiPlugin,
    ))
    .add_plugins(sandbox::SandboxPlugin)
    .add_plugins(edge_hud::EdgeHudPlugin)
    .add_plugins(match_service::MatchServicePlugin)
    .add_plugins((FrontendPlugin, frontend_qa::FrontendQaPlugin))
    .add_plugins((
        input_context::InputContextPlugin,
        ui::UiKitPlugin,
        shop::ShopPlugin,
        HelpOverlayPlugin,
        DebugConsolePlugin,
        PauseMenuPlugin,
        practice_sandbox::PracticeSandboxPlugin,
        GodModePlugin,
        ModelScalePlugin,
        MinionVisualsPlugin,
    ))
    // Separate call: the plugin tuple above is at Bevy's 15-element limit.
    .add_plugins((
        DecorPlugin,
        presentation3d::Presentation3dPlugin,
        jungle::JungleVisualsPlugin,
    ))
    .add_plugins((verdant3d::Verdant3dPlugin, visual_qa::VisualQaPlugin))
    .add_plugins((
        mobile_controls::MobileControlsPlugin,
        mobile_ui::MobileUiPlugin,
    ))
    .add_plugins((
        combat_visuals::CombatVisualsPlugin,
        projectile_visuals::ProjectileVisualsPlugin,
        combat_feedback::CombatFeedbackPlugin,
    ))
    .add_plugins((
        social::SocialPlugin,
        reaction_visuals::ReactionVisualsPlugin,
        social_qa::SocialQaPlugin,
    ))
    .add_plugins(career::CareerPlugin)
    .add_plugins(supporter::SupporterPlugin)
    .add_plugins(supporter_storekit::SupporterStoreKitPlugin)
    .add_plugins(game_audio::GameAudioPlugin)
    .add_plugins(game_vfx::GameVfxPlugin)
    .add_plugins(battlefield_atmosphere::BattlefieldAtmospherePlugin)
    .add_plugins(team_vision::TeamVisionPlugin)
    .add_plugins(team_vision_qa::TeamVisionQaPlugin)
    .add_plugins(audio_qa::AudioQaPlugin)
    .add_plugins(offline_qa::OfflineQaPlugin)
    .add_plugins(career_identity::CareerIdentityPlugin)
    .add_plugins(career_visual_qa::CareerVisualQaPlugin)
    .add_plugins(map_visuals::MapVisualsPlugin)
    .add_plugins(map_qa::MapQaPlugin)
    .add_plugins(combat_qa::CombatQaPlugin)
    .add_plugins(forest_pickup_qa::ForestPickupQaPlugin)
    .add_plugins(targeting_qa::TargetingQaPlugin)
    .run();
}
