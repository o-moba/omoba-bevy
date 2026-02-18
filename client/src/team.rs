use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::net::NetworkCommand;

const TEAM_BUTTON_SIZE: f32 = 140.0;
const TEAM_BUTTON_GAP: f32 = 28.0;
const CHARACTER_BUTTON_WIDTH: f32 = 120.0;
const CHARACTER_BUTTON_HEIGHT: f32 = 42.0;
const CHARACTER_BUTTON_GAP: f32 = 12.0;
const TEAM_OVERLAY_COLOR: Color = Color::srgba(0.02, 0.02, 0.02, 0.55);
const CHARACTER_BUTTON_COLOR: Color = Color::srgba(0.18, 0.18, 0.18, 0.95);
const CHARACTER_BUTTON_HOVER_COLOR: Color = Color::srgba(0.25, 0.25, 0.25, 0.98);
const CHARACTER_BUTTON_SELECTED_COLOR: Color = Color::srgba(0.78, 0.62, 0.18, 0.98);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Team {
    Green,
    Blue,
}

impl Team {
    pub fn as_str(self) -> &'static str {
        match self {
            Team::Green => "Green",
            Team::Blue => "Blue",
        }
    }

    pub fn ui_color(self) -> Color {
        match self {
            Team::Green => Color::srgba(0.14, 0.55, 0.22, 0.95),
            Team::Blue => Color::srgba(0.18, 0.35, 0.75, 0.95),
        }
    }

    pub fn ui_hover_color(self) -> Color {
        match self {
            Team::Green => Color::srgba(0.18, 0.65, 0.28, 0.98),
            Team::Blue => Color::srgba(0.22, 0.45, 0.85, 0.98),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterChoice {
    Ipfs,
    Toka,
    Wang,
    Cube,
}

impl CharacterChoice {
    pub fn as_str(self) -> &'static str {
        match self {
            CharacterChoice::Ipfs => "IPFS",
            CharacterChoice::Toka => "Toka",
            CharacterChoice::Wang => "Wang",
            CharacterChoice::Cube => "Cube",
        }
    }
}

#[derive(Resource, Default)]
pub struct TeamSelection {
    pub team: Option<Team>,
    pub character: CharacterChoice,
}

impl Default for CharacterChoice {
    fn default() -> Self {
        Self::Ipfs
    }
}

pub struct TeamSelectPlugin;

impl Plugin for TeamSelectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TeamSelection>()
            .add_systems(Startup, setup_team_select_ui)
            .add_systems(Update, team_select_ui_system);
    }
}

#[derive(Component)]
pub struct TeamSelectRoot;

#[derive(Component)]
struct TeamSelectButton {
    team: Team,
}

#[derive(Component)]
struct CharacterSelectButton {
    choice: CharacterChoice,
}

fn setup_team_select_ui(selection: Res<TeamSelection>, mut commands: Commands) {
    spawn_team_select_ui(&mut commands, selection.character);
}

pub fn spawn_team_select_ui(commands: &mut Commands, selected_character: CharacterChoice) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(20.0),
                ..default()
            },
            BackgroundColor(TEAM_OVERLAY_COLOR),
            TeamSelectRoot,
            Name::new("TeamSelectOverlay"),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Choose Character"),
                TextFont {
                    font_size: 22.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                Name::new("CharacterSelectTitle"),
            ));

            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(CHARACTER_BUTTON_GAP),
                        ..default()
                    },
                    Name::new("CharacterButtonsRow"),
                ))
                .with_children(|row| {
                    for choice in [
                        CharacterChoice::Ipfs,
                        CharacterChoice::Toka,
                        CharacterChoice::Wang,
                        CharacterChoice::Cube,
                    ] {
                        row.spawn((
                            Button,
                            Node {
                                width: Val::Px(CHARACTER_BUTTON_WIDTH),
                                height: Val::Px(CHARACTER_BUTTON_HEIGHT),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(if choice == selected_character {
                                CHARACTER_BUTTON_SELECTED_COLOR
                            } else {
                                CHARACTER_BUTTON_COLOR
                            }),
                            CharacterSelectButton { choice },
                            Name::new(format!("CharacterButton-{}", choice.as_str())),
                        ))
                        .with_children(|button| {
                            button.spawn((
                                Text::new(choice.as_str()),
                                TextFont {
                                    font_size: 18.0,
                                    ..default()
                                },
                                TextColor(Color::WHITE),
                            ));
                        });
                    }
                });

            parent.spawn((
                Text::new("Choose Team"),
                TextFont {
                    font_size: 22.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                Name::new("TeamSelectTitle"),
            ));

            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(TEAM_BUTTON_GAP),
                        ..default()
                    },
                    Name::new("TeamButtonsRow"),
                ))
                .with_children(|row| {
                    row.spawn((
                        Button,
                        Node {
                            width: Val::Px(TEAM_BUTTON_SIZE),
                            height: Val::Px(TEAM_BUTTON_SIZE),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(Team::Green.ui_color()),
                        TeamSelectButton { team: Team::Green },
                        Name::new("TeamGreenButton"),
                    ));
                    row.spawn((
                        Button,
                        Node {
                            width: Val::Px(TEAM_BUTTON_SIZE),
                            height: Val::Px(TEAM_BUTTON_SIZE),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(Team::Blue.ui_color()),
                        TeamSelectButton { team: Team::Blue },
                        Name::new("TeamBlueButton"),
                    ));
                });
        });
}

fn team_select_ui_system(
    mut commands: Commands,
    mut selection: ResMut<TeamSelection>,
    mut interaction_sets: ParamSet<(
        Query<
            (&Interaction, &TeamSelectButton, &mut BackgroundColor),
            (
                Changed<Interaction>,
                With<Button>,
                Without<CharacterSelectButton>,
            ),
        >,
        Query<
            (&Interaction, &CharacterSelectButton, &mut BackgroundColor),
            (
                Changed<Interaction>,
                With<Button>,
                Without<TeamSelectButton>,
            ),
        >,
    )>,
    character_buttons: Query<
        (Entity, &CharacterSelectButton),
        (With<Button>, Without<TeamSelectButton>),
    >,
    overlay_query: Query<Entity, With<TeamSelectRoot>>,
    mut command_writer: MessageWriter<NetworkCommand>,
) {
    if selection.team.is_some() {
        return;
    }

    let mut selected_character_changed = false;
    {
        let mut character_interactions = interaction_sets.p1();
        for (interaction, button, mut color) in character_interactions.iter_mut() {
            match *interaction {
                Interaction::Pressed => {
                    selection.character = button.choice;
                    selected_character_changed = true;
                }
                Interaction::Hovered => {
                    if selection.character != button.choice {
                        *color = CHARACTER_BUTTON_HOVER_COLOR.into();
                    }
                }
                Interaction::None => {
                    if selection.character != button.choice {
                        *color = CHARACTER_BUTTON_COLOR.into();
                    }
                }
            }
        }
    }

    if selected_character_changed {
        for (entity, button) in &character_buttons {
            let color = if button.choice == selection.character {
                CHARACTER_BUTTON_SELECTED_COLOR
            } else {
                CHARACTER_BUTTON_COLOR
            };
            commands.entity(entity).insert(BackgroundColor(color));
        }
    }

    {
        let mut team_interactions = interaction_sets.p0();
        for (interaction, button, mut color) in team_interactions.iter_mut() {
            match *interaction {
                Interaction::Pressed => {
                    selection.team = Some(button.team);
                    command_writer.write(NetworkCommand::Join { team: button.team });
                    if let Ok(overlay) = overlay_query.single() {
                        commands
                            .entity(overlay)
                            .despawn_related::<Children>()
                            .despawn();
                    }
                }
                Interaction::Hovered => {
                    *color = button.team.ui_hover_color().into();
                }
                Interaction::None => {
                    *color = button.team.ui_color().into();
                }
            }
        }
    }
}
