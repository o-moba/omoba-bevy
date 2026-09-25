//! Debug tools page of the pause menu, driven by [`ClientDebugAccess`]
//! (step 11e; the server's `Snapshot.debug_access`, or the `match_mode`
//! table for an older server):
//! - the toggles (god mode, speed boost) wherever they are allowed: dev,
//!   local and offline practice; not in release, a worker round or Combat
//!   Test;
//! - the bots and 1v1 section only where practice commands are allowed;
//! - in Combat Test only an entry that opens the Combat Test panel, whose
//!   actor config owns the toggles there.
//!
//! The main-page entry is hidden when none of these apply. Every action is a
//! server command, so the local client never fakes bots, levels or
//! invulnerability. The toggle lines read and write [`DebugToggles`]. Entity
//! `Name`s keep their `PauseMenuPractice*` prefix.
use bevy::prelude::*;
use shared::debug::DebugCommand;
use shared::practice::{
    DUEL_GOLD_STEP, DUEL_MAX_GOLD, DUEL_MAX_LEVEL, DUEL_MIN_LEVEL, PracticeCommand,
};

use super::{ClientDebugAccess, DebugAccessSet, DebugToggles};
use crate::combat::ActionFeedback;
use crate::net::NetworkCommand;
use crate::pause_menu::{PauseAction, PauseMenuSet, PauseMenuState};
use crate::ui::{Activated, UiActionAppExt, theme, theme::ButtonKind, widgets};

pub(crate) struct PracticeSandboxPlugin;

impl Plugin for PracticeSandboxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PracticeSandboxState>()
            .init_resource::<DebugToggles>()
            .init_resource::<ClientDebugAccess>()
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
                    .after(DebugAccessSet)
                    .after(PauseMenuSet::Taps)
                    .in_set(PauseMenuSet::Visuals),
            );
    }
}

/// Menu-side sandbox choices. The toggles are not here: they are
/// [`DebugToggles`], shared with the HUD.
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

/// Main-page entry; hidden where the page would be empty.
#[derive(Component)]
pub(crate) struct PracticeOpenButton;

/// A part of the page shown only with the matching access.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum ToolsPart {
    Toggles,
    Practice,
    CombatTest,
}

impl ToolsPart {
    fn shown(self, access: ClientDebugAccess) -> bool {
        match self {
            Self::Toggles => access.toggles(),
            Self::Practice => access.practice(),
            Self::CombatTest => access.combat_test,
        }
    }
}

/// Controls of the sandbox page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PracticeAction {
    Back,
    GodMode,
    SpeedBoost,
    OpenCombatTest,
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
    SpeedBoost,
    Level,
    Gold,
}

pub(crate) fn spawn_practice_open_button(main: &mut ChildSpawnerCommands) {
    let button = widgets::button(
        main,
        "Debug tools",
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
                Text::new("Debug tools"),
                theme::text(20.0),
                TextColor(theme::IVORY),
                Name::new("PauseMenuPracticeTitle"),
            ));
            section.spawn((
                Text::new("Nothing here counts toward career or rating."),
                theme::text(13.0),
                TextColor(theme::MUTED),
                Name::new("PauseMenuPracticeNote"),
            ));
            part(section, ToolsPart::Toggles, "PauseMenuPracticeToggles").with_children(
                |toggles| {
                    widgets::toggle_row(
                        toggles,
                        "God mode",
                        on_off(DebugToggles::default().god_mode),
                        PracticeLabel::GodMode,
                        PracticeAction::GodMode,
                        "PauseMenuPracticeGodMode",
                    );
                    widgets::toggle_row(
                        toggles,
                        "Speed boost",
                        on_off(DebugToggles::default().speed_boost),
                        PracticeLabel::SpeedBoost,
                        PracticeAction::SpeedBoost,
                        "PauseMenuPracticeSpeedBoost",
                    );
                },
            );
            part(section, ToolsPart::Practice, "PauseMenuPracticeBots").with_children(|bots| {
                bots.spawn((
                    Text::new("Bots"),
                    theme::text(18.0),
                    TextColor(theme::GOLD),
                    Name::new("PauseMenuPracticeBotsTitle"),
                ));
                widgets::button(
                    bots,
                    "Standard bots (lanes)",
                    ButtonKind::Secondary,
                    PracticeAction::Roster,
                    "PauseMenuPracticeRosterButton",
                );
                widgets::button(
                    bots,
                    "Clear all bots",
                    ButtonKind::Secondary,
                    PracticeAction::ClearBots,
                    "PauseMenuPracticeClearButton",
                );
                widgets::button(
                    bots,
                    "Spawn target dummy",
                    ButtonKind::Secondary,
                    PracticeAction::SpawnDummy,
                    "PauseMenuPracticeDummyButton",
                );
                bots.spawn((
                    Text::new("1v1 opponent"),
                    theme::text(18.0),
                    TextColor(theme::GOLD),
                    Name::new("PauseMenuPracticeDuelTitle"),
                ));
                widgets::adjust_row(
                    bots,
                    "Level",
                    defaults.duel_level.to_string(),
                    PracticeLabel::Level,
                    PracticeAction::LevelDown,
                    PracticeAction::LevelUp,
                    "PauseMenuPracticeLevelControls",
                );
                widgets::adjust_row(
                    bots,
                    "Gold",
                    defaults.duel_gold.to_string(),
                    PracticeLabel::Gold,
                    PracticeAction::GoldDown,
                    PracticeAction::GoldUp,
                    "PauseMenuPracticeGoldControls",
                );
                widgets::button(
                    bots,
                    "Start 1v1 on mid",
                    ButtonKind::Secondary,
                    PracticeAction::StartDuel,
                    "PauseMenuPracticeDuelButton",
                );
            });
            part(
                section,
                ToolsPart::CombatTest,
                "PauseMenuPracticeCombatTest",
            )
            .with_children(|combat| {
                combat.spawn((
                    Text::new("God mode and speed belong to the actor config here."),
                    theme::text(13.0),
                    TextColor(theme::MUTED),
                    Name::new("PauseMenuPracticeCombatTestNote"),
                ));
                widgets::button(
                    combat,
                    "Combat Test panel (F6)",
                    ButtonKind::Secondary,
                    PracticeAction::OpenCombatTest,
                    "PauseMenuPracticeCombatTestButton",
                );
            });
            widgets::button(
                section,
                "Back",
                ButtonKind::Secondary,
                PracticeAction::Back,
                "PauseMenuPracticeBackButton",
            );
        });
}

/// A column of the page that `sync_practice_availability` shows or hides.
fn part<'a>(
    section: &'a mut ChildSpawnerCommands,
    part: ToolsPart,
    name: &'static str,
) -> EntityCommands<'a> {
    section.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            align_items: AlignItems::Center,
            display: Display::None,
            ..default()
        },
        Visibility::Hidden,
        part,
        Name::new(name),
    ))
}

fn on_off(enabled: bool) -> &'static str {
    if enabled { "ON" } else { "OFF" }
}

fn show(node: &mut Node, visibility: &mut Visibility, shown: bool) {
    let display = if shown { Display::Flex } else { Display::None };
    if node.display != display {
        node.display = display;
        *visibility = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// The main-page entry exists only where the page has something to show,
/// and each part of the page follows its access. Losing all access closes
/// the page; the toggles themselves are reset by `sync_debug_access`.
fn sync_practice_availability(
    access: Res<ClientDebugAccess>,
    mut state: ResMut<PracticeSandboxState>,
    mut buttons: Query<(&mut Node, &mut Visibility), With<PracticeOpenButton>>,
    mut parts: Query<(&ToolsPart, &mut Node, &mut Visibility), Without<PracticeOpenButton>>,
) {
    let access = *access;
    for (mut node, mut visibility) in &mut buttons {
        show(&mut node, &mut visibility, access.any());
    }
    for (part, mut node, mut visibility) in &mut parts {
        show(&mut node, &mut visibility, part.shown(access));
    }
    if !access.any() && state.open {
        state.open = false;
    }
}

/// The main-page entry (`PauseAction::OpenPractice`) and the page's own
/// controls; everything is ignored outside an open menu, and each action
/// only where its access allows it.
fn apply_practice_actions(
    mut opened: MessageReader<Activated<PauseAction>>,
    mut activated: MessageReader<Activated<PracticeAction>>,
    mut menu: ResMut<PauseMenuState>,
    access: Res<ClientDebugAccess>,
    mut state: ResMut<PracticeSandboxState>,
    mut toggles: ResMut<DebugToggles>,
    mut feedback: Option<ResMut<ActionFeedback>>,
    mut sandbox: Option<ResMut<crate::sandbox::SandboxClient>>,
    mut commands: MessageWriter<NetworkCommand>,
) {
    let access = *access;
    let page = menu.open && access.any();
    for Activated { action, .. } in opened.read() {
        if *action == PauseAction::OpenPractice && page {
            state.open = true;
        }
    }
    for Activated { action, .. } in activated.read() {
        let allowed = page
            && match action {
                PracticeAction::GodMode | PracticeAction::SpeedBoost => access.toggles(),
                PracticeAction::OpenCombatTest => access.combat_test,
                PracticeAction::Back => true,
                PracticeAction::Roster
                | PracticeAction::ClearBots
                | PracticeAction::SpawnDummy
                | PracticeAction::StartDuel
                | PracticeAction::LevelDown
                | PracticeAction::LevelUp
                | PracticeAction::GoldDown
                | PracticeAction::GoldUp => access.practice(),
            };
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
            PracticeAction::SpeedBoost => {
                toggles.speed_boost = !toggles.speed_boost;
                commands.write(NetworkCommand::Debug(DebugCommand::SpeedBoost(
                    toggles.speed_boost,
                )));
                say(if toggles.speed_boost {
                    "Speed boost on."
                } else {
                    "Speed boost off."
                });
            }
            PracticeAction::OpenCombatTest => {
                if let Some(sandbox) = sandbox.as_deref_mut() {
                    sandbox.open = true;
                }
                state.open = false;
                menu.open = false;
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
            PracticeLabel::GodMode => on_off(toggles.god_mode).to_owned(),
            PracticeLabel::SpeedBoost => on_off(toggles.speed_boost).to_owned(),
            PracticeLabel::Level => state.duel_level.to_string(),
            PracticeLabel::Gold => state.duel_gold.to_string(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::sync_debug_access;
    use crate::net::{ClientSession, GameStateSnapshot};
    use crate::ui::{UiAction, action::dispatch_actions};
    use shared::debug::DebugAccess;

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
            .init_resource::<ClientDebugAccess>()
            .init_resource::<ClientSession>()
            .init_resource::<GameStateSnapshot>()
            .add_message::<NetworkCommand>()
            .add_message::<Activated<PauseAction>>()
            .add_message::<Activated<PracticeAction>>()
            .add_systems(
                Update,
                (
                    sync_debug_access,
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

    fn page_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(PauseMenuState {
                open: true,
                ..default()
            })
            .init_resource::<PracticeSandboxState>()
            .init_resource::<DebugToggles>()
            .init_resource::<ClientDebugAccess>()
            .insert_resource(ClientSession::admitted_for_test())
            .init_resource::<GameStateSnapshot>()
            .add_message::<NetworkCommand>()
            .add_message::<Activated<PauseAction>>()
            .add_message::<Activated<PracticeAction>>()
            .add_systems(
                Update,
                (
                    sync_debug_access,
                    sync_practice_availability,
                    apply_practice_actions,
                )
                    .chain(),
            );
        app
    }

    fn set_mode(app: &mut App, mode: &str, access: Option<DebugAccess>) {
        let mut snapshot = app.world_mut().resource_mut::<GameStateSnapshot>();
        snapshot.match_mode = mode.into();
        snapshot.debug_access = access;
    }

    fn press(app: &mut App, action: PracticeAction) -> Vec<NetworkCommand> {
        let source = app.world_mut().spawn_empty().id();
        app.world_mut().write_message(Activated { action, source });
        app.update();
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect()
    }

    fn spawn_parts(app: &mut App) -> [Entity; 4] {
        let mut spawn = |part: Option<ToolsPart>| {
            let mut entity = app
                .world_mut()
                .spawn((Node::default(), Visibility::Inherited));
            match part {
                Some(part) => entity.insert(part),
                None => entity.insert(PracticeOpenButton),
            };
            entity.id()
        };
        [
            spawn(None),
            spawn(Some(ToolsPart::Toggles)),
            spawn(Some(ToolsPart::Practice)),
            spawn(Some(ToolsPart::CombatTest)),
        ]
    }

    fn shown(app: &App, entities: [Entity; 4]) -> [bool; 4] {
        entities.map(|e| app.world().get::<Node>(e).unwrap().display != Display::None)
    }

    /// Step 11e: the page follows access. Dev (without `OMOBA_DEBUG_UI`)
    /// gets the toggles only; practice gets everything; a worker round
    /// (new server: `Some(all false)`) and release get nothing.
    #[test]
    fn the_page_shows_what_access_allows() {
        let mut app = page_app();
        let parts = spawn_parts(&mut app);

        set_mode(&mut app, "dev", None);
        app.update();
        assert_eq!(shown(&app, parts), [true, true, false, false], "dev");
        assert!(matches!(
            press(&mut app, PracticeAction::SpeedBoost).as_slice(),
            [NetworkCommand::Debug(DebugCommand::SpeedBoost(true))]
        ));
        assert!(app.world().resource::<DebugToggles>().speed_boost);
        assert!(
            press(&mut app, PracticeAction::SpawnDummy).is_empty(),
            "no bots in dev"
        );

        set_mode(&mut app, "practice", None);
        app.update();
        assert_eq!(shown(&app, parts), [true, true, true, false], "practice");
        assert!(matches!(
            press(&mut app, PracticeAction::SpawnDummy).as_slice(),
            [NetworkCommand::Debug(DebugCommand::Practice(
                PracticeCommand::SpawnDummy
            ))]
        ));

        set_mode(&mut app, "practice", Some(DebugAccess::default()));
        app.update();
        assert_eq!(shown(&app, parts), [false; 4], "worker round");
        assert!(press(&mut app, PracticeAction::GodMode).is_empty());
        assert!(press(&mut app, PracticeAction::Roster).is_empty());

        set_mode(&mut app, "release", None);
        app.update();
        assert_eq!(shown(&app, parts), [false; 4], "release");
    }

    /// The page's toggle lines are `DebugToggles`: toggling sends the
    /// command, and leaving a practice match resets both (in
    /// `sync_debug_access`). A toggle in a dev match is kept.
    #[test]
    fn leaving_practice_resets_the_toggles_but_a_dev_toggle_survives() {
        let mut app = page_app();
        let god_mode = |app: &App| app.world().resource::<DebugToggles>().god_mode;

        // Dev: god mode is kept.
        set_mode(&mut app, "dev", None);
        app.update();
        app.world_mut().resource_mut::<DebugToggles>().god_mode = true;
        app.update();
        app.update();
        assert!(god_mode(&app));
        app.world_mut().resource_mut::<DebugToggles>().god_mode = false;

        // Offline practice: the page's toggle turns it on and sends it.
        set_mode(&mut app, shared::debug::OFFLINE_PRACTICE_MODE, None);
        app.update();
        let sent = press(&mut app, PracticeAction::GodMode);
        assert!(god_mode(&app));
        assert!(matches!(
            sent.as_slice(),
            [NetworkCommand::Debug(DebugCommand::GodMode(true))]
        ));

        // Leaving the practice match forgets it and closes the page.
        app.world_mut().resource_mut::<PracticeSandboxState>().open = true;
        set_mode(&mut app, "release", None);
        app.update();
        assert!(!god_mode(&app));
        assert!(!app.world().resource::<PracticeSandboxState>().open);
    }

    /// Combat Test: no toggles (the actor config owns them), only the entry
    /// that opens the Combat Test panel and closes the menu.
    #[test]
    fn combat_test_offers_only_the_panel_entry() {
        let mut app = page_app();
        app.init_resource::<crate::sandbox::SandboxClient>();
        app.world_mut()
            .resource_mut::<crate::sandbox::SandboxClient>()
            .open = false;
        let parts = spawn_parts(&mut app);
        set_mode(&mut app, "dev", None);
        app.world_mut().resource_mut::<GameStateSnapshot>().sandbox =
            Some(shared::sandbox::SandboxSnapshot {
                config: Default::default(),
                ack: None,
                last_request_id: 0,
                actors: vec![],
                analytics: Default::default(),
                simulation_secs: 0.0,
                frame: 1,
            });
        app.update();
        assert_eq!(shown(&app, parts), [true, false, false, true]);
        assert!(press(&mut app, PracticeAction::GodMode).is_empty());
        assert!(press(&mut app, PracticeAction::OpenCombatTest).is_empty());
        assert!(app.world().resource::<crate::sandbox::SandboxClient>().open);
        assert!(!app.world().resource::<PauseMenuState>().open);
    }
}
