//! Pre-join selection flow: hero class -> roster avatar -> team.
//!
//! The roster avatars (CC0 VRM models under `assets/avatars/`) are the primary
//! cosmetic path for the demo; the legacy SDK character stays in
//! [`TeamSelection::character`] as the fallback model when no roster avatar is
//! selected. Picking a team commits the join (class + avatar + team in one
//! packet).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use shared::{HeroClass, avatar_roster};
use std::collections::HashMap;

use crate::net::{ClientConnectionState, ClientSession, NetworkCommand, SessionUiCommand};
pub use ekza_bevy_sdk::EkzaCharacter as CharacterChoice;

const TEAM_BUTTON_SIZE: f32 = 110.0;
const TEAM_BUTTON_GAP: f32 = 28.0;
const CLASS_BUTTON_WIDTH: f32 = 190.0;
const CLASS_BUTTON_HEIGHT: f32 = 60.0;
const CLASS_BUTTON_GAP: f32 = 12.0;
const AVATAR_BUTTON_WIDTH: f32 = 76.0;
const AVATAR_BUTTON_HEIGHT: f32 = 92.0;
const AVATAR_THUMBNAIL_SIZE: f32 = 56.0;
const AVATAR_GRID_GAP: f32 = 8.0;
const AVATAR_GRID_COLUMNS: usize = 8;
const TEAM_OVERLAY_COLOR: Color = Color::srgba(0.02, 0.02, 0.02, 0.55);
const SELECT_BUTTON_COLOR: Color = Color::srgba(0.18, 0.18, 0.18, 0.95);
const SELECT_BUTTON_HOVER_COLOR: Color = Color::srgba(0.25, 0.25, 0.25, 0.98);
const SELECT_BUTTON_SELECTED_COLOR: Color = Color::srgba(0.78, 0.62, 0.18, 0.98);

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

#[derive(Resource)]
pub struct TeamSelection {
    pub team: Option<Team>,
    /// Legacy SDK character; used as the model when no roster avatar is picked.
    pub character: CharacterChoice,
    /// Selected hero class (gameplay kit).
    pub hero_class: HeroClass,
    /// Selected roster avatar slug (cosmetic; primary demo path).
    pub avatar: Option<String>,
}

impl Default for TeamSelection {
    fn default() -> Self {
        Self {
            team: None,
            character: CharacterChoice::default(),
            hero_class: HeroClass::default(),
            // Preselect the first shipped avatar so a plain "click a team"
            // flow already exercises the roster path.
            avatar: avatar_roster().first().map(|avatar| avatar.slug.clone()),
        }
    }
}

/// Thumbnail image handles for the avatar grid, keyed by slug.
#[derive(Resource, Default)]
pub struct AvatarThumbnails(pub HashMap<String, Handle<Image>>);

pub struct TeamSelectPlugin;

impl Plugin for TeamSelectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TeamSelection>()
            .init_resource::<AvatarThumbnails>()
            .add_systems(
                Startup,
                setup_team_select_ui.after(crate::persistence::load_persistent_client_settings),
            )
            .add_systems(Update, team_select_ui_system)
            .add_systems(Update, attach_avatar_thumbnails)
            .add_systems(Update, autojoin_from_env);
    }
}

#[derive(Component)]
pub struct TeamSelectRoot;

#[derive(Component)]
struct TeamSelectButton {
    team: Team,
}

#[derive(Component)]
struct ClassSelectButton {
    class: HeroClass,
}

#[derive(Component)]
struct AvatarSelectButton {
    slug: String,
}

/// Placeholder node inside an avatar button that receives the thumbnail image
/// once [`AvatarThumbnails`] has a handle for the slug.
#[derive(Component)]
struct AvatarThumbnailSlot {
    slug: String,
}

fn setup_team_select_ui(
    selection: Res<TeamSelection>,
    asset_server: Res<AssetServer>,
    mut thumbnails: ResMut<AvatarThumbnails>,
    mut commands: Commands,
) {
    for avatar in avatar_roster() {
        if let Some(thumbnail) = avatar.thumbnail.as_deref() {
            thumbnails.0.insert(
                avatar.slug.clone(),
                asset_server.load(format!("avatars/{thumbnail}")),
            );
        }
    }
    spawn_team_select_ui(&mut commands, &selection);
}

pub fn spawn_team_select_ui(commands: &mut Commands, selection: &TeamSelection) {
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
                row_gap: Val::Px(14.0),
                ..default()
            },
            BackgroundColor(TEAM_OVERLAY_COLOR),
            TeamSelectRoot,
            Name::new("TeamSelectOverlay"),
        ))
        .with_children(|parent| {
            spawn_section_title(parent, "Choose Class", "ClassSelectTitle");

            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(CLASS_BUTTON_GAP),
                        ..default()
                    },
                    Name::new("ClassButtonsRow"),
                ))
                .with_children(|row| {
                    for class in HeroClass::ALL {
                        spawn_class_button(row, class, selection.hero_class == class);
                    }
                });

            spawn_section_title(parent, "Choose Avatar", "AvatarSelectTitle");

            parent
                .spawn((
                    Node {
                        width: Val::Px(
                            AVATAR_GRID_COLUMNS as f32 * (AVATAR_BUTTON_WIDTH + AVATAR_GRID_GAP),
                        ),
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        justify_content: JustifyContent::Center,
                        column_gap: Val::Px(AVATAR_GRID_GAP),
                        row_gap: Val::Px(AVATAR_GRID_GAP),
                        ..default()
                    },
                    Name::new("AvatarGrid"),
                ))
                .with_children(|grid| {
                    for avatar in avatar_roster() {
                        spawn_avatar_button(
                            grid,
                            &avatar.slug,
                            &avatar.display_name,
                            selection.avatar.as_deref() == Some(avatar.slug.as_str()),
                        );
                    }
                });

            spawn_section_title(parent, "Choose Team", "TeamSelectTitle");

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
                    spawn_team_button(row, Team::Green, "TeamGreenButton");
                    spawn_team_button(row, Team::Blue, "TeamBlueButton");
                });

            parent.spawn((
                Text::new("Pick a class and an avatar, then a team to join the match."),
                TextFont {
                    font_size: 15.0,
                    ..default()
                },
                TextColor(Color::srgba(0.78, 0.80, 0.86, 1.0)),
                Name::new("TeamSelectHint"),
            ));
        });
}

fn spawn_section_title(parent: &mut ChildSpawnerCommands, title: &str, name: &str) {
    parent.spawn((
        Text::new(title),
        TextFont {
            font_size: 20.0,
            ..default()
        },
        TextColor(Color::WHITE),
        Name::new(name.to_owned()),
    ));
}

fn spawn_class_button(row: &mut ChildSpawnerCommands, class: HeroClass, selected: bool) {
    row.spawn((
        Button,
        Node {
            width: Val::Px(CLASS_BUTTON_WIDTH),
            height: Val::Px(CLASS_BUTTON_HEIGHT),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: Val::Px(3.0),
            padding: UiRect::axes(Val::Px(6.0), Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(if selected {
            SELECT_BUTTON_SELECTED_COLOR
        } else {
            SELECT_BUTTON_COLOR
        }),
        ClassSelectButton { class },
        Name::new(format!("ClassButton-{}", class.id())),
    ))
    .with_children(|button| {
        button.spawn((
            Text::new(class.display_name()),
            TextFont {
                font_size: 17.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
        button.spawn((
            Text::new(class.tagline()),
            TextFont {
                font_size: 10.5,
                ..default()
            },
            TextColor(Color::srgba(0.82, 0.84, 0.90, 1.0)),
        ));
    });
}

fn spawn_avatar_button(
    grid: &mut ChildSpawnerCommands,
    slug: &str,
    display_name: &str,
    selected: bool,
) {
    grid.spawn((
        Button,
        Node {
            width: Val::Px(AVATAR_BUTTON_WIDTH),
            height: Val::Px(AVATAR_BUTTON_HEIGHT),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: Val::Px(3.0),
            ..default()
        },
        BackgroundColor(if selected {
            SELECT_BUTTON_SELECTED_COLOR
        } else {
            SELECT_BUTTON_COLOR
        }),
        AvatarSelectButton {
            slug: slug.to_owned(),
        },
        Name::new(format!("AvatarButton-{slug}")),
    ))
    .with_children(|button| {
        button.spawn((
            Node {
                width: Val::Px(AVATAR_THUMBNAIL_SIZE),
                height: Val::Px(AVATAR_THUMBNAIL_SIZE),
                ..default()
            },
            BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.9)),
            AvatarThumbnailSlot {
                slug: slug.to_owned(),
            },
        ));
        button.spawn((
            Text::new(display_name),
            TextFont {
                font_size: 10.5,
                ..default()
            },
            TextColor(Color::srgba(0.86, 0.88, 0.92, 1.0)),
        ));
    });
}

/// Attaches the thumbnail image to avatar buttons once (avatars without a
/// shipped thumbnail keep the dark placeholder tile + name).
fn attach_avatar_thumbnails(
    mut commands: Commands,
    thumbnails: Res<AvatarThumbnails>,
    slots: Query<(Entity, &AvatarThumbnailSlot), Without<ImageNode>>,
) {
    for (entity, slot) in &slots {
        if let Some(handle) = thumbnails.0.get(&slot.slug) {
            // try_insert: the overlay (and these slots) can be despawned by a
            // join in the same frame; inserting into a dead entity must not panic.
            commands
                .entity(entity)
                .try_insert(ImageNode::new(handle.clone()));
        }
    }
}

fn spawn_team_button(row: &mut ChildSpawnerCommands, team: Team, name: &str) {
    row.spawn((
        Button,
        Node {
            width: Val::Px(TEAM_BUTTON_SIZE),
            height: Val::Px(TEAM_BUTTON_SIZE),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(team.ui_color()),
        TeamSelectButton { team },
        Name::new(name.to_owned()),
    ))
    .with_children(|button| {
        button.spawn((
            Text::new(team.as_str()),
            TextFont {
                font_size: 26.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
    });
}

#[allow(clippy::too_many_arguments)]
fn team_select_ui_system(
    mut commands: Commands,
    client_session: Res<ClientSession>,
    mut selection: ResMut<TeamSelection>,
    mut interaction_sets: ParamSet<(
        Query<
            (&Interaction, &TeamSelectButton, &mut BackgroundColor),
            (Changed<Interaction>, With<Button>),
        >,
        Query<
            (&Interaction, &ClassSelectButton, &mut BackgroundColor),
            (Changed<Interaction>, With<Button>),
        >,
        Query<
            (&Interaction, &AvatarSelectButton, &mut BackgroundColor),
            (Changed<Interaction>, With<Button>),
        >,
    )>,
    class_buttons: Query<(Entity, &ClassSelectButton), With<Button>>,
    avatar_buttons: Query<(Entity, &AvatarSelectButton), With<Button>>,
    overlay_query: Query<Entity, With<TeamSelectRoot>>,
    mut command_writer: MessageWriter<NetworkCommand>,
    mut session_ui_writer: MessageWriter<SessionUiCommand>,
) {
    if selection.team.is_some() {
        return;
    }

    let mut class_changed = false;
    for (interaction, button, mut color) in interaction_sets.p1().iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                selection.hero_class = button.class;
                class_changed = true;
            }
            Interaction::Hovered => {
                if selection.hero_class != button.class {
                    *color = SELECT_BUTTON_HOVER_COLOR.into();
                }
            }
            Interaction::None => {
                if selection.hero_class != button.class {
                    *color = SELECT_BUTTON_COLOR.into();
                }
            }
        }
    }
    if class_changed {
        for (entity, button) in &class_buttons {
            let color = if button.class == selection.hero_class {
                SELECT_BUTTON_SELECTED_COLOR
            } else {
                SELECT_BUTTON_COLOR
            };
            commands.entity(entity).try_insert(BackgroundColor(color));
        }
    }

    let mut avatar_changed = false;
    for (interaction, button, mut color) in interaction_sets.p2().iter_mut() {
        let is_selected = selection.avatar.as_deref() == Some(button.slug.as_str());
        match *interaction {
            Interaction::Pressed => {
                selection.avatar = Some(button.slug.clone());
                avatar_changed = true;
            }
            Interaction::Hovered => {
                if !is_selected {
                    *color = SELECT_BUTTON_HOVER_COLOR.into();
                }
            }
            Interaction::None => {
                if !is_selected {
                    *color = SELECT_BUTTON_COLOR.into();
                }
            }
        }
    }
    if avatar_changed {
        for (entity, button) in &avatar_buttons {
            let color = if selection.avatar.as_deref() == Some(button.slug.as_str()) {
                SELECT_BUTTON_SELECTED_COLOR
            } else {
                SELECT_BUTTON_COLOR
            };
            commands.entity(entity).try_insert(BackgroundColor(color));
        }
    }

    for (interaction, button, mut color) in interaction_sets.p0().iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                if client_session.join_flow_committed {
                    continue;
                }
                // Dead transport: a Join written now would be silently lost and
                // the overlay would be gone with no way back. Trigger the same
                // recovery as the Retry button and keep the select screen up.
                if client_session.state == ClientConnectionState::Disconnected {
                    info!(
                        "[omoba:cli] event=join_deferred reason=disconnected \
                         msg=\"Reconnecting to server; pick a team again once connected.\""
                    );
                    session_ui_writer.write(SessionUiCommand::Retry);
                    continue;
                }
                selection.team = Some(button.team);
                info!(
                    "[omoba:cli] event=join_request team={:?} class={} avatar={:?} character={:?}",
                    button.team,
                    selection.hero_class.id(),
                    selection.avatar,
                    selection.character
                );
                command_writer.write(NetworkCommand::Join {
                    team: button.team,
                    character: selection.character,
                    hero_class: selection.hero_class,
                    avatar: selection.avatar.clone(),
                });
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

/// Debug/automation hook: `OMOBA_AUTOJOIN="<class>:<avatar-slug|->:<team>"`
/// (e.g. `mage:agnes:green`) joins immediately without UI interaction. Used by
/// the headless evidence runs; ignored when unset.
fn autojoin_from_env(
    mut done: Local<bool>,
    mut commands: Commands,
    client_session: Res<ClientSession>,
    mut selection: ResMut<TeamSelection>,
    overlay_query: Query<Entity, With<TeamSelectRoot>>,
    mut command_writer: MessageWriter<NetworkCommand>,
) {
    if *done {
        return;
    }
    let Ok(raw) = std::env::var("OMOBA_AUTOJOIN") else {
        *done = true;
        return;
    };
    if selection.team.is_some() || client_session.join_flow_committed {
        *done = true;
        return;
    }
    *done = true;

    let mut parts = raw.split(':');
    let class = HeroClass::from_id(parts.next().unwrap_or("")).unwrap_or_default();
    let avatar = shared::normalize_avatar_slug(parts.next()).map(str::to_owned);
    let team = match parts.next() {
        Some("blue") => Team::Blue,
        _ => Team::Green,
    };

    selection.hero_class = class;
    selection.avatar = avatar.clone();
    selection.team = Some(team);
    info!(
        "[omoba:cli] event=autojoin team={:?} class={} avatar={:?}",
        team,
        class.id(),
        avatar
    );
    command_writer.write(NetworkCommand::Join {
        team,
        character: selection.character,
        hero_class: class,
        avatar,
    });
    for overlay in &overlay_query {
        commands
            .entity(overlay)
            .despawn_related::<Children>()
            .despawn();
    }
}
