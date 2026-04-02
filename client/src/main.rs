use bevy::{asset::AssetPlugin, prelude::*};

mod camera;
mod combat;
mod debug_console;
mod game_state;
mod maps;
mod minimap;
mod net;
mod pause_menu;
mod persistence;
mod player;
mod session_config;
mod team;
mod world;

use camera::CameraPlugin;
use combat::CombatPlugin;
use debug_console::DebugConsolePlugin;
use game_state::GameStateUiPlugin;
use maps::MapsPlugin;
use minimap::MinimapPlugin;
use net::NetworkingPlugin;
use pause_menu::PauseMenuPlugin;
use persistence::ClientPersistencePlugin;
use player::PlayerPlugin;
use team::TeamSelectPlugin;
use world::SetupPlugin;

fn main() {
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
            DebugConsolePlugin,
            NetworkingPlugin,
            MinimapPlugin,
            CombatPlugin,
            TeamSelectPlugin,
            GameStateUiPlugin,
            PauseMenuPlugin,
        ))
        .run();
}
