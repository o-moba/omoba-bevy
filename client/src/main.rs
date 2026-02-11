use bevy::{asset::AssetPlugin, prelude::*};

mod camera;
mod combat;
mod debug_console;
mod game_state;
mod maps;
mod minimap;
mod net;
mod pause_menu;
mod player;
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
            SetupPlugin,
            NetworkingPlugin,
            MinimapPlugin,
            CombatPlugin,
            TeamSelectPlugin,
            GameStateUiPlugin,
            DebugConsolePlugin,
            PauseMenuPlugin,
        ))
        .run();
}
