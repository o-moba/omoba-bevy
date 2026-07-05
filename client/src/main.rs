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
mod maps;
mod match_hud;
mod minimap;
mod model_scale;
mod net;
mod pause_menu;
mod persistence;
mod player;
mod session_config;
mod team;
mod world;

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
use model_scale::ModelScalePlugin;
use net::NetworkingPlugin;
use pause_menu::PauseMenuPlugin;
use persistence::ClientPersistencePlugin;
use player::PlayerPlugin;
use team::TeamSelectPlugin;
use world::SetupPlugin;

fn main() {
    // Headless model size analyzer (prints a bind-pose height table and exits).
    if std::env::var("OMOBA_MEASURE_MODELS").is_ok_and(|value| value == "1") {
        model_scale::run_model_measurement_analyzer();
        return;
    }
    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin {
            file_path: format!("{}/assets", env!("CARGO_MANIFEST_DIR")),
            ..default()
        }))
        .add_plugins((
            CameraPlugin,
            PlayerPlugin,
            MapsPlugin,
            ClientPersistencePlugin,
            SetupPlugin,
            NetworkingPlugin,
            BossesPlugin,
            MinimapPlugin,
            CombatPlugin,
            MatchHudPlugin,
            TeamSelectPlugin,
            GameStateUiPlugin,
        ))
        .add_plugins((
            HelpOverlayPlugin,
            DebugConsolePlugin,
            PauseMenuPlugin,
            GodModePlugin,
            ModelScalePlugin,
        ))
        // Separate call: the plugin tuple above is at Bevy's 15-element limit.
        .add_plugins(DecorPlugin)
        .run();
}
