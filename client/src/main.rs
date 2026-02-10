use bevy::{app::AppExit, asset::AssetPlugin, prelude::*};

mod camera;
mod combat;
mod debug_console;
mod game_state;
mod maps;
mod minimap;
mod net;
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
        ))
        .add_systems(Update, handle_exit_input)
        .run();
}

fn handle_exit_input(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut app_exit_writer: MessageWriter<AppExit>,
) {
    if keyboard_input.just_pressed(KeyCode::Escape) {
        info!("Escape pressed, exiting application.");
        app_exit_writer.write(AppExit::Success);
    }
}
