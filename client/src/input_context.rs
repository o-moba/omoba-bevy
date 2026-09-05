//! One frame-level input policy, resolved after modal toggles and before world actions.
use bevy::prelude::*;

use crate::debug_console::DebugConsole;
use crate::help_overlay::HelpOverlayVisible;
use crate::net::{GameState, GameStateSnapshot};
use crate::pause_menu::PauseMenuState;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum InputContextSet {
    Modal,
    Resolve,
    Actions,
}

#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct GameplayInputContext {
    pub modal_open: bool,
    pub debug_flight: bool,
    pub running: bool,
}

impl Default for GameplayInputContext {
    fn default() -> Self {
        Self {
            modal_open: false,
            debug_flight: false,
            running: true,
        }
    }
}

impl GameplayInputContext {
    pub fn gameplay_allowed(&self) -> bool {
        self.running && !self.modal_open && !self.debug_flight
    }

    pub fn camera_allowed(&self) -> bool {
        !self.modal_open
    }
}

pub(crate) struct InputContextPlugin;

impl Plugin for InputContextPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameplayInputContext>()
            .configure_sets(
                Update,
                (
                    InputContextSet::Modal,
                    InputContextSet::Resolve,
                    InputContextSet::Actions,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                resolve_input_context.in_set(InputContextSet::Resolve),
            );
    }
}

fn resolve_input_context(
    keyboard: Res<ButtonInput<KeyCode>>,
    pause: Option<Res<PauseMenuState>>,
    help: Option<Res<HelpOverlayVisible>>,
    game: Option<Res<GameStateSnapshot>>,
    session: Option<Res<crate::net::ClientSession>>,
    debug: Option<Res<DebugConsole>>,
    join_ui: Query<Entity, With<crate::team::TeamSelectRoot>>,
    mut context: ResMut<GameplayInputContext>,
) {
    context.running = game
        .as_ref()
        .is_some_and(|game| matches!(game.state, GameState::Running))
        && session
            .as_ref()
            .is_none_or(|session| session.join_confirmed());
    context.modal_open = !join_ui.is_empty()
        || pause.as_ref().is_some_and(|pause| pause.open)
        || (context.running && help.as_ref().is_some_and(|help| help.0));
    let debug_enabled = debug.as_ref().is_some_and(|debug| debug.ui_enabled);
    if !debug_enabled || (!context.modal_open && keyboard.just_pressed(KeyCode::Space)) {
        context.debug_flight = false;
    } else if !context.modal_open && keyboard.just_pressed(KeyCode::F8) {
        context.debug_flight = !context.debug_flight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_frame_modal_toggle_blocks_actions_and_debug_flight_requires_opt_in() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<PauseMenuState>()
            .init_resource::<HelpOverlayVisible>()
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .add_plugins(InputContextPlugin)
            .add_systems(
                Update,
                (|keys: Res<ButtonInput<KeyCode>>, mut help: ResMut<HelpOverlayVisible>| {
                    if keys.just_pressed(KeyCode::F1) {
                        help.0 = !help.0;
                    }
                })
                .in_set(InputContextSet::Modal),
            );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F1);
        app.update();
        assert!(
            !app.world()
                .resource::<GameplayInputContext>()
                .gameplay_allowed()
        );
        assert!(
            !app.world()
                .resource::<GameplayInputContext>()
                .camera_allowed()
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut().resource_mut::<HelpOverlayVisible>().0 = false;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F8);
        app.update();
        assert!(!app.world().resource::<GameplayInputContext>().debug_flight);
        let mut debug = DebugConsole::default();
        debug.ui_enabled = true;
        app.insert_resource(debug);
        app.update();
        assert!(app.world().resource::<GameplayInputContext>().debug_flight);
        assert!(
            !app.world()
                .resource::<GameplayInputContext>()
                .gameplay_allowed()
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Space);
        app.update();
        assert!(
            app.world()
                .resource::<GameplayInputContext>()
                .gameplay_allowed()
        );
    }

    #[derive(Resource, Default)]
    struct PipelineTrace(Vec<String>);

    #[test]
    fn production_pipeline_applies_snapshot_before_autohelp_and_sends_current_frame_input() {
        use crate::net::{ClientNetPipeline, NetworkCommand, TargetId, TargetKind};
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<GameStateSnapshot>()
            .init_resource::<PipelineTrace>()
            .add_message::<NetworkCommand>()
            .add_plugins((InputContextPlugin, crate::help_overlay::HelpOverlayPlugin));
        crate::net::configure_network_pipeline(&mut app);
        app.add_systems(
            Update,
            (|mut snapshot: ResMut<GameStateSnapshot>, mut trace: ResMut<PipelineTrace>| {
                snapshot.state = GameState::Running;
                trace.0.push("snapshot".into());
            })
            .in_set(ClientNetPipeline::ApplySnapshot),
        );
        app.add_systems(
            Update,
            (|context: Res<GameplayInputContext>,
              mut commands: MessageWriter<NetworkCommand>,
              mut trace: ResMut<PipelineTrace>| {
                if context.gameplay_allowed() {
                    commands.write(NetworkCommand::Cast {
                        target: TargetId {
                            kind: TargetKind::Player,
                            id: 2,
                        },
                        slot: 0,
                    });
                    trace.0.push("gameplay".into());
                } else {
                    trace.0.push("blocked".into());
                }
            })
            .in_set(InputContextSet::Actions),
        );
        app.add_systems(
            Update,
            (|mut commands: MessageReader<NetworkCommand>, mut trace: ResMut<PipelineTrace>| {
                let count = commands.read().count();
                trace.0.push(format!("send:{count}"));
            })
            .in_set(ClientNetPipeline::SendCommands),
        );
        app.update();
        assert_eq!(
            app.world().resource::<PipelineTrace>().0,
            ["snapshot", "blocked", "send:0"]
        );
        app.world_mut().resource_mut::<PipelineTrace>().0.clear();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F1);
        app.update();
        assert_eq!(
            app.world().resource::<PipelineTrace>().0,
            ["snapshot", "gameplay", "send:1"]
        );
    }
}
