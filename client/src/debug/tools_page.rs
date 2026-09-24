//! Practice sandbox page of the pause menu. Only a bot practice match
//! (`match_mode` `"practice"` or
//! [`OFFLINE_PRACTICE_MODE`](shared::debug::OFFLINE_PRACTICE_MODE), i.e.
//! `DebugAccess::for_match_mode(..).practice`) shows it; every action is a
//! server command, so the local client never fakes bots, levels or
//! invulnerability. The god mode line reads and writes [`DebugToggles`].
use bevy::prelude::*;
use shared::debug::{DebugAccess, DebugCommand};
use shared::practice::{
    DUEL_GOLD_STEP, DUEL_MAX_GOLD, DUEL_MAX_LEVEL, DUEL_MIN_LEVEL, PracticeCommand,
};

use super::DebugToggles;
use crate::combat::ActionFeedback;
use crate::net::{ClientSession, GameStateSnapshot, NetworkCommand};
use crate::pause_menu::{PauseAction, PauseMenuSet, PauseMenuState};
use crate::ui::{Activated, UiActionAppExt, theme, theme::ButtonKind, widgets};

pub(crate) struct PracticeSandboxPlugin;

impl Plugin for PracticeSandboxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PracticeSandboxState>()
            .init_resource::<DebugToggles>()
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

/// Menu-side sandbox choices. God mode is not here: it is
/// [`DebugToggles::god_mode`], shared with the HUD.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PracticeSandboxState {
    pub(crate) open: bool,
    pub(crate) duel_level: u32,
    pub(crate) duel_gold: u32,
}

impl Default for PracticeSandboxState {
    fn default() -> Self {
        Self {
            open: false,
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
                god_mode_label(DebugToggles::default().god_mode),
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
        && snapshot.is_some_and(|s| DebugAccess::for_match_mode(&s.match_mode).practice)
}

/// The main-page entry exists only in a practice match; leaving one also
/// closes the page and forgets the requested god mode (the new match starts
/// without it on the server too). God mode is cleared on the edge out of a
/// practice match only, so a HUD toggle outside practice (dev) is kept.
fn sync_practice_availability(
    snapshot: Option<Res<GameStateSnapshot>>,
    session: Res<ClientSession>,
    mut was_practice: Local<bool>,
    mut state: ResMut<PracticeSandboxState>,
    mut toggles: ResMut<DebugToggles>,
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
    if !practice && state.open {
        state.open = false;
    }
    if !practice && *was_practice && toggles.god_mode {
        toggles.god_mode = false;
    }
    *was_practice = practice;
}

/// The main-page entry (`PauseAction::OpenPractice`) and the page's own
/// controls; everything is ignored outside an open menu in a practice match.
fn apply_practice_actions(
    mut opened: MessageReader<Activated<PauseAction>>,
    mut activated: MessageReader<Activated<PracticeAction>>,
    menu: Res<PauseMenuState>,
    snapshot: Option<Res<GameStateSnapshot>>,
    session: Res<ClientSession>,
    mut state: ResMut<PracticeSandboxState>,
    mut toggles: ResMut<DebugToggles>,
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
                toggles.god_mode = !toggles.god_mode;
                commands.write(NetworkCommand::Debug(DebugCommand::GodMode(
                    toggles.god_mode,
                )));
                say(if toggles.god_mode {
                    "God mode on: the server skips damage to you."
                } else {
                    "God mode off."
                });
            }
            PracticeAction::Roster => {
                commands.write(NetworkCommand::Debug(DebugCommand::Practice(
                    PracticeCommand::Roster,
                )));
                say("Standard practice bots restored.");
            }
            PracticeAction::ClearBots => {
                commands.write(NetworkCommand::Debug(DebugCommand::Practice(
                    PracticeCommand::ClearBots,
                )));
                say("All bots removed.");
            }
            PracticeAction::SpawnDummy => {
                commands.write(NetworkCommand::Debug(DebugCommand::Practice(
                    PracticeCommand::SpawnDummy,
                )));
                say("Target dummy placed in front of you.");
            }
            PracticeAction::StartDuel => {
                commands.write(NetworkCommand::Debug(DebugCommand::Practice(
                    state.duel_command(),
                )));
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
    toggles: Res<DebugToggles>,
    mut labels: Query<(&PracticeLabel, &mut Text)>,
) {
    if !state.is_changed() && !toggles.is_changed() {
        return;
    }
    for (label, mut text) in &mut labels {
        text.0 = match label {
            PracticeLabel::GodMode => god_mode_label(toggles.god_mode).to_owned(),
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
            .init_resource::<DebugToggles>()
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
            [NetworkCommand::Debug(DebugCommand::Practice(
                PracticeCommand::SpawnDummy
            ))]
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

    /// The page's god mode line is `DebugToggles::god_mode`: toggling it sends
    /// the command, and leaving a practice match clears it. A toggle made
    /// outside practice (the HUD in a dev match) is not cleared.
    #[test]
    fn leaving_practice_clears_god_mode_but_a_dev_toggle_survives() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(PauseMenuState {
                open: true,
                ..default()
            })
            .init_resource::<PracticeSandboxState>()
            .init_resource::<DebugToggles>()
            .insert_resource(ClientSession::admitted_for_test())
            .init_resource::<GameStateSnapshot>()
            .add_message::<NetworkCommand>()
            .add_message::<Activated<PauseAction>>()
            .add_message::<Activated<PracticeAction>>()
            .add_systems(
                Update,
                (sync_practice_availability, apply_practice_actions).chain(),
            );
        let set_mode = |app: &mut App, mode: &str| {
            app.world_mut()
                .resource_mut::<GameStateSnapshot>()
                .match_mode = mode.into();
        };
        let god_mode = |app: &App| app.world().resource::<DebugToggles>().god_mode;

        // Dev: the HUD's god mode is kept; the page is not available.
        set_mode(&mut app, "dev");
        app.world_mut().resource_mut::<DebugToggles>().god_mode = true;
        app.update();
        app.update();
        assert!(god_mode(&app));
        app.world_mut().resource_mut::<DebugToggles>().god_mode = false;

        // Offline practice: the page's toggle turns it on and sends it.
        set_mode(&mut app, shared::debug::OFFLINE_PRACTICE_MODE);
        app.update();
        let source = app.world_mut().spawn_empty().id();
        app.world_mut().write_message(Activated {
            action: PracticeAction::GodMode,
            source,
        });
        app.update();
        assert!(god_mode(&app));
        let sent: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect();
        assert!(matches!(
            sent.as_slice(),
            [NetworkCommand::Debug(DebugCommand::GodMode(true))]
        ));

        // Leaving the practice match forgets it.
        set_mode(&mut app, "release");
        app.update();
        assert!(!god_mode(&app));
    }
}
