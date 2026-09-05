use bevy::{asset::AssetPlugin, prelude::*};

mod bosses;
mod camera;
mod combat;
mod debug_console;
mod decor;
mod game_state;
mod god_mode;
mod help_overlay;
mod input_bindings;
mod input_context;
mod maps;
mod match_hud;
mod minimap;
mod minions;
mod model_scale;
mod net;
mod pause_menu;
mod persistence;
mod player;
mod presentation2d;
mod presentation3d;
mod session_config;
mod sprite;
mod team;
mod world;
mod world2d;

use bosses::BossesPlugin;
use camera::CameraPlugin;
use combat::CombatPlugin;
use debug_console::DebugConsolePlugin;
use decor::DecorPlugin;
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

fn main() {
    // Headless model size analyzer (prints a bind-pose height table and exits).
    if std::env::var("OMOBA_MEASURE_MODELS").is_ok_and(|value| value == "1") {
        model_scale::run_model_measurement_analyzer();
        return;
    }
    let asset_root = shared::client_asset_root();
    eprintln!("Omoba asset root: {}", asset_root.display());
    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin {
            file_path: asset_root.to_string_lossy().into_owned(),
            ..default()
        }))
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
        .add_plugins((
            input_context::InputContextPlugin,
            HelpOverlayPlugin,
            DebugConsolePlugin,
            PauseMenuPlugin,
            GodModePlugin,
            ModelScalePlugin,
            MinionVisualsPlugin,
        ))
        // Separate call: the plugin tuple above is at Bevy's 15-element limit.
        .add_plugins((DecorPlugin, presentation3d::Presentation3dPlugin))
        .run();
}
