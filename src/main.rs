use bevy::{app::AppExit, prelude::*};

mod camera;
mod player;
mod setup;

use camera::CameraPlugin;
use player::PlayerPlugin;
use setup::SetupPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins((CameraPlugin, PlayerPlugin, SetupPlugin))
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
