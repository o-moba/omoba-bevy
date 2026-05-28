pub(crate) mod combat;

use bevy::prelude::*;

pub(crate) struct GameplayPlugin;

impl Plugin for GameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(combat::CombatPlugin);
    }
}
