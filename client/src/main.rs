use bevy::{app::AppExit, asset::AssetPlugin, prelude::*};

mod camera;
mod net;
mod player;
mod world;

use camera::CameraPlugin;
use net::NetworkingPlugin;
use player::PlayerPlugin;
use world::SetupPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin {
            file_path: format!("{}/assets", env!("CARGO_MANIFEST_DIR")),
            ..default()
        }))
        .add_plugins((CameraPlugin, PlayerPlugin, SetupPlugin, NetworkingPlugin))
        .add_systems(Update, handle_exit_input)
        .run();
}

fn handle_exit_input(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut app_exit_writer: EventWriter<AppExit>,
) {
    if keyboard_input.just_pressed(KeyCode::Escape) {
        info!("Escape pressed, exiting application.");
        app_exit_writer.send(AppExit::Success);
    }
}
