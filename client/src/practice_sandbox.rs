//! Practice sandbox page of the pause menu. Only a local bot practice match
//! (`match_mode == "practice"`) shows it; every action is a server command,
//! so the local client never fakes bots, levels or invulnerability.
use bevy::prelude::*;
use shared::practice::{
    DUEL_GOLD_STEP, DUEL_MAX_GOLD, DUEL_MAX_LEVEL, DUEL_MIN_LEVEL, PracticeCommand,
};

use crate::combat::ActionFeedback;
use crate::net::{ClientSession, GameStateSnapshot, NetworkCommand};
use crate::pause_menu::{PauseAction, PauseMenuSet, PauseMenuState};
use crate::ui::{Activated, UiActionAppExt, theme, theme::ButtonKind, widgets};

pub(crate) struct PracticeSandboxPlugin;

impl Plugin for PracticeSandboxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PracticeSandboxState>()
            .add_ui_action::<PracticeAction>()
            .add_systems(
                Update,
                (
                    sync_practice_availability,
                    apply_practice_actions,
                    sync_practice_section,
                    update_practice_labels,
                )
                    .chain()
                    .after(PauseMenuSet::Taps)
                    .in_set(PauseMenuSet::Visuals),
            );
    }
}

/// Menu-side sandbox choices. `god_mode` mirrors what was last requested;
/// the server owns the actual flag and re-applies it per match.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PracticeSandboxState {
    pub(crate) open: bool,
    pub(crate) god_mode: bool,
    pub(crate) duel_level: u32,
    pub(crate) duel_gold: u32,
}

impl Default for PracticeSandboxState {
    fn default() -> Self {
        Self {
            open: false,
            god_mode: false,
            duel_level: 5,
            duel_gold: 500,
        }
    }
}

impl PracticeSandboxState {
    fn adjust_level(&mut self, delta: i32) {
        self.duel_level = (self.duel_level as i32 + delta)
            .clamp(DUEL_MIN_LEVEL as i32, DUEL_MAX_LEVEL as i32) as u32;
    }

    fn adjust_gold(&mut self, delta: i32) {
        let next = self.duel_gold as i64 + delta as i64 * DUEL_GOLD_STEP as i64;
        self.duel_gold = next.clamp(0, DUEL_MAX_GOLD as i64) as u32;
    }

    fn duel_command(&self) -> PracticeCommand {
        PracticeCommand::duel(self.duel_level, self.duel_gold)
    }
}

#[derive(Component)]
pub(crate) struct PracticeSection;

/// Main-page entry; hidden outside practice matches.
#[derive(Component)]
pub(crate) struct PracticeOpenButton;

/// Controls of the sandbox page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PracticeAction {
    Back,
    GodMode,
    Roster,
    ClearBots,
    SpawnDummy,
    StartDuel,
    LevelDown,
    LevelUp,
    GoldDown,
    GoldUp,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum PracticeLabel {
    GodMode,
    Level,
    Gold,
}

pub(crate) fn spawn_practice_open_button(main: &mut ChildSpawnerCommands) {
    let button = widgets::button(
        main,
        "Practice sandbox",
        ButtonKind::Secondary,
        PauseAction::OpenPractice,
        "PauseMenuPracticeButton",
    );
    main.commands().entity(button).insert(PracticeOpenButton);
}

pub(crate) fn spawn_practice_section(panel: &mut ChildSpawnerCommands) {
    let defaults = PracticeSandboxState::default();
    panel
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                display: Display::None,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::FlexStart,
                flex_shrink: 0.0,
                ..default()
            },
            Visibility::Hidden,
            PracticeSection,
            Name::new("PauseMenuPracticeSection"),
        ))
        .with_children(|section| {
            section.spawn((
                Text::new("Practice sandbox"),
                theme::text(20.0),
                TextColor(theme::IVORY),
                Name::new("PauseMenuPracticeTitle"),
            ));
            section.spawn((
                Text::new("Local bots only. Nothing here counts toward career or rating."),
                theme::text(13.0),
                TextColor(theme::MUTED),
                Name::new("PauseMenuPracticeNote"),
            ));
            widgets::toggle_row(
                section,
                "God mode",
                god_mode_label(defaults.god_mode),
                PracticeLabel::GodMode,
                PracticeAction::GodMode,
                "PauseMenuPracticeGodMode",
            );
            section.spawn((
                Text::new("Bots"),
                theme::text(18.0),
                TextColor(theme::GOLD),
                Name::new("PauseMenuPracticeBotsTitle"),
            ));
            widgets::button(
                section,
                "Standard bots (lanes)",
                ButtonKind::Secondary,
                PracticeAction::Roster,
                "PauseMenuPracticeRosterButton",
            );
            widgets::button(
                section,
                "Clear all bots",
                ButtonKind::Secondary,
                PracticeAction::ClearBots,
                "PauseMenuPracticeClearButton",
            );
            widgets::button(
                section,
                "Spawn target dummy",
                ButtonKind::Secondary,
                PracticeAction::SpawnDummy,
                "PauseMenuPracticeDummyButton",
            );
            section.spawn((
                Text::new("1v1 opponent"),
                theme::text(18.0),
                TextColor(theme::GOLD),
                Name::new("PauseMenuPracticeDuelTitle"),
            ));
            widgets::adjust_row(
                section,
                "Level",
                defaults.duel_level.to_string(),
                PracticeLabel::Level,
                PracticeAction::LevelDown,
                PracticeAction::LevelUp,
                "PauseMenuPracticeLevelControls",
            );
            widgets::adjust_row(
                section,
                "Gold",
                defaults.duel_gold.to_string(),
                PracticeLabel::Gold,
                PracticeAction::GoldDown,
                PracticeAction::GoldUp,
                "PauseMenuPracticeGoldControls",
            );
            widgets::button(
                section,
                "Start 1v1 on mid",
                ButtonKind::Secondary,
                PracticeAction::StartDuel,
                "PauseMenuPracticeDuelButton",
            );
            widgets::button(
                section,
                "Back",
                ButtonKind::Secondary,
                PracticeAction::Back,
                "PauseMenuPracticeBackButton",
            );
        });
}

fn god_mode_label(enabled: bool) -> &'static str {
    if enabled { "ON" } else { "OFF" }
}

/// Server bot practice and the socket-free offline practice both accept the
/// sandbox commands; every other match mode hides the page.
fn is_practice(snapshot: Option<&GameStateSnapshot>, session: &ClientSession) -> bool {
    session.join_confirmed()
        && snapshot
            .is_some_and(|s| matches!(s.match_mode.as_str(), "practice" | "offline_practice"))
}

/// The main-page entry exists only in a practice match; leaving one also
/// closes the page and forgets the requested god mode (the new match starts
/// without it on the server too).
fn sync_practice_availability(
    snapshot: Option<Res<GameStateSnapshot>>,
    session: Res<ClientSession>,
    mut state: ResMut<PracticeSandboxState>,
    mut buttons: Query<(&mut Node, &mut Visibility), With<PracticeOpenButton>>,
) {
    let practice = is_practice(snapshot.as_deref(), &session);
    for (mut node, mut visibility) in &mut buttons {
        let display = if practice {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
            *visibility = if practice {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
    }
    if !practice && (state.open || state.god_mode) {
        state.open = false;
        state.god_mode = false;
    }
}

/// The main-page entry (`PauseAction::OpenPractice`) and the page's own
/// controls; everything is ignored outside an open menu in a practice match.
#[allow(clippy::too_many_arguments)]
fn apply_practice_actions(
    mut opened: MessageReader<Activated<PauseAction>>,
    mut activated: MessageReader<Activated<PracticeAction>>,
    menu: Res<PauseMenuState>,
    snapshot: Option<Res<GameStateSnapshot>>,
    session: Res<ClientSession>,
    mut state: ResMut<PracticeSandboxState>,
    mut debug: Option<ResMut<crate::god_mode::DebugToggleState>>,
    mut feedback: Option<ResMut<ActionFeedback>>,
    mut commands: MessageWriter<NetworkCommand>,
) {
    let allowed = menu.open && is_practice(snapshot.as_deref(), &session);
    for Activated { action, .. } in opened.read() {
        if *action == PauseAction::OpenPractice && allowed {
            state.open = true;
        }
    }
    for Activated { action, .. } in activated.read() {
        if !allowed {
            continue;
        }
        let mut say = |line: &str| {
            if let Some(feedback) = feedback.as_deref_mut() {
                feedback.push_line(line);
            }
        };
        match action {
            PracticeAction::Back => state.open = false,
            PracticeAction::GodMode => {
                state.god_mode = !state.god_mode;
                if let Some(debug) = debug.as_deref_mut() {
                    debug.god_mode = state.god_mode;
                }
                commands.write(NetworkCommand::SetGodMode {
                    enabled: state.god_mode,
                });
                say(if state.god_mode {
                    "God mode on: the server skips damage to you."
                } else {
                    "God mode off."
                });
            }
            PracticeAction::Roster => {
                commands.write(NetworkCommand::Practice {
                    command: PracticeCommand::Roster,
                });
                say("Standard practice bots restored.");
            }
            PracticeAction::ClearBots => {
                commands.write(NetworkCommand::Practice {
                    command: PracticeCommand::ClearBots,
                });
                say("All bots removed.");
            }
            PracticeAction::SpawnDummy => {
                commands.write(NetworkCommand::Practice {
                    command: PracticeCommand::SpawnDummy,
                });
                say("Target dummy placed in front of you.");
            }
            PracticeAction::StartDuel => {
                commands.write(NetworkCommand::Practice {
                    command: state.duel_command(),
                });
                say("1v1: an opponent is coming down mid.");
            }
            PracticeAction::LevelDown => state.adjust_level(-1),
            PracticeAction::LevelUp => state.adjust_level(1),
            PracticeAction::GoldDown => state.adjust_gold(-1),
            PracticeAction::GoldUp => state.adjust_gold(1),
        }
    }
}

/// The page shows while the menu is open on it; settings or closing the
/// menu return to the main page like every other section.
fn sync_practice_section(
    menu: Res<PauseMenuState>,
    mut state: ResMut<PracticeSandboxState>,
    mut sections: Query<(&mut Visibility, &mut Node), With<PracticeSection>>,
) {
    if state.open && (!menu.open || menu.in_settings) {
        state.open = false;
    }
    if !menu.is_changed() && !state.is_changed() {
        return;
    }
    for (mut visibility, mut node) in &mut sections {
        let show = menu.open && state.open;
        *visibility = if show {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        node.display = if show { Display::Flex } else { Display::None };
    }
}

fn update_practice_labels(
    state: Res<PracticeSandboxState>,
    mut labels: Query<(&PracticeLabel, &mut Text)>,
) {
    if !state.is_changed() {
        return;
    }
    for (label, mut text) in &mut labels {
        text.0 = match label {
            PracticeLabel::GodMode => god_mode_label(state.god_mode).to_owned(),
            PracticeLabel::Level => state.duel_level.to_string(),
            PracticeLabel::Gold => state.duel_gold.to_string(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{UiAction, action::dispatch_actions};

    #[test]
    fn duel_settings_clamp_and_build_the_wire_command() {
        let mut state = PracticeSandboxState::default();
        for _ in 0..20 {
            state.adjust_level(1);
            state.adjust_gold(1);
        }
        assert_eq!(
            (state.duel_level, state.duel_gold),
            (DUEL_MAX_LEVEL, DUEL_MAX_GOLD)
        );
        for _ in 0..40 {
            state.adjust_level(-1);
            state.adjust_gold(-1);
        }
        assert_eq!((state.duel_level, state.duel_gold), (DUEL_MIN_LEVEL, 0));
        state.adjust_level(3);
        state.adjust_gold(4);
        assert_eq!(
            state.duel_command(),
            PracticeCommand::StartDuel {
                level: 4,
                gold: 400
            }
        );
    }

    #[test]
    fn practice_entry_only_shows_in_practice_and_commands_reach_the_network() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<PauseMenuState>()
            .init_resource::<PracticeSandboxState>()
            .init_resource::<ClientSession>()
            .init_resource::<GameStateSnapshot>()
            .add_message::<NetworkCommand>()
            .add_message::<Activated<PauseAction>>()
            .add_message::<Activated<PracticeAction>>()
            .add_systems(
                Update,
                (
                    dispatch_actions::<PauseAction>,
                    dispatch_actions::<PracticeAction>,
                    sync_practice_availability,
                    apply_practice_actions,
                    sync_practice_section,
                    update_practice_labels,
                )
                    .chain(),
            );
        let open_button = app
            .world_mut()
            .spawn((
                Button,
                UiAction(PauseAction::OpenPractice),
                Node::default(),
                Visibility::Inherited,
                Interaction::None,
                BackgroundColor(theme::TILE),
                PracticeOpenButton,
            ))
            .id();
        let dummy_button = app
            .world_mut()
            .spawn((
                Button,
                UiAction(PracticeAction::SpawnDummy),
                Interaction::None,
                BackgroundColor(theme::TILE),
            ))
            .id();
        let section = app
            .world_mut()
            .spawn((Visibility::Hidden, Node::default(), PracticeSection))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<Node>(open_button).unwrap().display,
            Display::None,
            "hidden until a practice match is confirmed"
        );
        app.insert_resource(ClientSession::admitted_for_test());
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .match_mode = "practice".into();
        app.world_mut().resource_mut::<PauseMenuState>().open = true;
        app.update();
        assert_eq!(
            app.world().get::<Node>(open_button).unwrap().display,
            Display::Flex
        );
        *app.world_mut().get_mut::<Interaction>(open_button).unwrap() = Interaction::Pressed;
        app.update();
        assert!(app.world().resource::<PracticeSandboxState>().open);
        assert_eq!(
            app.world().get::<Node>(section).unwrap().display,
            Display::Flex
        );
        *app.world_mut()
            .get_mut::<Interaction>(dummy_button)
            .unwrap() = Interaction::Pressed;
        app.update();
        let sent: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect();
        assert!(matches!(
            sent.as_slice(),
            [NetworkCommand::Practice {
                command: PracticeCommand::SpawnDummy
            }]
        ));
        // Closing the menu leaves the page; a release match hides the entry again.
        app.world_mut().resource_mut::<PauseMenuState>().open = false;
        app.update();
        assert!(!app.world().resource::<PracticeSandboxState>().open);
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .match_mode = "release".into();
        app.update();
        assert_eq!(
            app.world().get::<Node>(open_button).unwrap().display,
            Display::None
        );
    }
}
