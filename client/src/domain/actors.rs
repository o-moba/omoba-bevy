//! Hero markers and the movement intents input writes and prediction consumes.

use bevy::prelude::*;

#[derive(Component)]
pub struct Player;

#[derive(Component)]
pub struct PlayerBody;

#[derive(Component, Default)]
pub struct VerticalVelocity(pub f32);

#[derive(Component)]
pub struct RemotePlayer;

#[derive(Component)]
pub(crate) struct MovementTarget {
    pub(crate) target: Vec3,
}

#[derive(Component, Debug)]
pub(crate) struct MovementRoute {
    pub(crate) requested_target: Vec3,
    pub(crate) structure_revision: u64,
    pub(crate) destination: Vec3,
    pub(crate) waypoints: Vec<Vec3>,
}
