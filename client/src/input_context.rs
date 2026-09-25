//! One frame-level input policy, resolved after modal toggles and before world actions.
use bevy::prelude::*;

use crate::debug::DebugConsole;
use crate::help_overlay::HelpOverlayVisible;
use crate::net::{GameState, GameStateSnapshot};
use crate::ui::{ModalAppExt, ModalId, ModalStack};

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum InputContextSet {
    Social,
    Modal,
    Resolve,
    Actions,
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CombatPointerInputSet;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WorldMovementInputSet;

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
                    InputContextSet::Social,
                    InputContextSet::Modal,
                    InputContextSet::Resolve,
                    InputContextSet::Actions,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                resolve_input_context
                    .in_set(InputContextSet::Resolve)
                    .after(crate::ui::modal::ModalSet::Late),
            );
        register_modals(app);
    }
}

/// The overlays that are modal panels, in one list next to the policy that
/// reads them. Each root carries `ui::ModalRoot` with the same id; a missing
/// resource (no `ServerEntry` on desktop) counts as closed.
fn register_modals(app: &mut App) {
    app.register_modal::<crate::pause_menu::PauseMenuState>(ModalId::Pause, |menu| menu.open)
        .register_modal::<crate::career::CareerClient>(
            ModalId::Career,
            crate::career::CareerClient::modal_open,
        )
        .register_modal::<crate::shop::ShopState>(ModalId::Shop, |shop| shop.open)
        .register_modal::<crate::supporter::SupporterUiState>(ModalId::Supporter, |state| {
            state.open
        })
        .register_modal::<crate::edge_hud::ScoreboardState>(ModalId::Scoreboard, |state| state.open)
        .register_modal::<crate::mobile_ui::ServerEntry>(ModalId::ServerEntry, |entry| entry.open);
}

/// Gameplay is blocked while any registered modal is open (`ModalStack`:
/// pause menu, career, shop, supporter, scoreboard, phone server entry) and
/// by the checks below, which are not "a modal panel is open" and so stay
/// here: the help overlay only blocks during a running match, the sandbox
/// also blocks in teleport/edit modes without a panel, social also blocks
/// for the chat wheel and one frame after a send, the front-end is a screen
/// state, the hero picker is detected by its root entity, and a phone blocks
/// while it is portrait or unfocused.
fn resolve_input_context(
    keyboard: Res<ButtonInput<KeyCode>>,
    modals: Option<Res<ModalStack>>,
    help: Option<Res<HelpOverlayVisible>>,
    game: Option<Res<GameStateSnapshot>>,
    session: Option<Res<crate::net::ClientSession>>,
    debug: Option<Res<DebugConsole>>,
    join_ui: Query<Entity, With<crate::team::TeamSelectRoot>>,
    mut context: ResMut<GameplayInputContext>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    social: Option<Res<crate::social::SocialClient>>,
    sandbox: Option<Res<crate::sandbox::SandboxClient>>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
) {
    context.running = game
        .as_ref()
        .is_some_and(|game| matches!(game.state, GameState::Running))
        && session
            .as_ref()
            .is_none_or(|session| session.join_confirmed());
    // Every front-end screen is modal: the world keeps simulating behind it,
    // but nothing the player does on a menu may reach gameplay.
    let front_end_open = screen.as_ref().is_some_and(|screen| screen.get().is_menu());
    context.modal_open = modals.as_ref().is_some_and(|modals| modals.is_open())
        || sandbox.as_ref().is_some_and(|s| s.blocks_world())
        || front_end_open
        || mobile
            .as_ref()
            .is_some_and(|mobile| mobile.enabled && (!mobile.landscape || !mobile.focused))
        || social
            .as_ref()
            .is_some_and(|social| social.blocks_gameplay())
        || !join_ui.is_empty()
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
    use crate::pause_menu::PauseMenuState;

    #[test]
    fn gameplay_and_camera_are_blocked_while_any_modal_is_open() {
        type Toggle = fn(&mut World, bool);
        let modals: [(ModalId, Toggle); 6] = [
            (ModalId::Pause, |world, open| {
                world.resource_mut::<PauseMenuState>().open = open
            }),
            (ModalId::Career, |world, open| {
                world.resource_mut::<crate::career::CareerClient>().modal = if open {
                    crate::career::CareerModal::Profile
                } else {
                    crate::career::CareerModal::Closed
                }
            }),
            (ModalId::Shop, |world, open| {
                world.resource_mut::<crate::shop::ShopState>().open = open
            }),
            (ModalId::Supporter, |world, open| {
                world
                    .resource_mut::<crate::supporter::SupporterUiState>()
                    .open = open
            }),
            (ModalId::Scoreboard, |world, open| {
                world
                    .resource_mut::<crate::edge_hud::ScoreboardState>()
                    .open = open
            }),
            (ModalId::ServerEntry, |world, open| {
                world.resource_mut::<crate::mobile_ui::ServerEntry>().open = open
            }),
        ];
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .init_resource::<PauseMenuState>()
            .init_resource::<crate::career::CareerClient>()
            .init_resource::<crate::shop::ShopState>()
            .init_resource::<crate::supporter::SupporterUiState>()
            .init_resource::<crate::edge_hud::ScoreboardState>()
            .init_resource::<crate::mobile_ui::ServerEntry>()
            .add_plugins(InputContextPlugin);
        app.update();
        assert!(
            app.world()
                .resource::<GameplayInputContext>()
                .gameplay_allowed()
        );
        for (id, toggle) in modals {
            toggle(app.world_mut(), true);
            app.update();
            assert_eq!(app.world().resource::<ModalStack>().top(), Some(id));
            let context = app.world().resource::<GameplayInputContext>();
            assert!(!context.gameplay_allowed(), "{id:?}");
            assert!(!context.camera_allowed(), "{id:?}");
            toggle(app.world_mut(), false);
            app.update();
            assert!(!app.world().resource::<ModalStack>().is_open());
            assert!(
                app.world()
                    .resource::<GameplayInputContext>()
                    .gameplay_allowed(),
                "{id:?}"
            );
        }
    }

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
    #[test]
    fn career_history_blocks_live_combat_and_camera_without_a_player_entity() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .init_resource::<crate::career::CareerClient>()
            .add_plugins(InputContextPlugin);
        app.world_mut()
            .resource_mut::<crate::career::CareerClient>()
            .modal = crate::career::CareerModal::History;
        app.update();
        let context = app.world().resource::<GameplayInputContext>();
        assert!(!context.gameplay_allowed());
        assert!(!context.camera_allowed());
        app.world_mut()
            .resource_mut::<crate::career::CareerClient>()
            .modal = crate::career::CareerModal::Closed;
        app.update();
        assert!(
            app.world()
                .resource::<GameplayInputContext>()
                .gameplay_allowed()
        );
    }
}
