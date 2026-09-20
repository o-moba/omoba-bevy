//! The player's profile card: the identity shown on the home screen, and the
//! screen that customizes it.
//!
//! The card is local-first. Career profiles (`ProfileSummary`) carry the facts
//! the server owns — nickname, rating, level, wins — while the card carries the
//! presentation the player chooses. It is stored next to the client
//! preferences in its own file so the preferences schema stays untouched.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use shared::HeroClass;
use shared::career::ProfileSummary;
use std::path::PathBuf;

use super::AppScreen;
use super::widgets::{self, ButtonKind, MenuButton};
use crate::team::AvatarThumbnails;

const CARD_FILE: &str = "profile_card.json";

/// Titles a player can put on the card. The requirement is the number of
/// matches won; index 0 is always available.
pub const TITLES: [(&str, u32); 6] = [
    ("Newcomer", 0),
    ("Lane Regular", 5),
    ("Jungle Warden", 15),
    ("Tower Breaker", 30),
    ("Verdant Veteran", 60),
    ("Ancient Champion", 120),
];

pub fn title_unlocked(index: usize, wins: u32) -> bool {
    TITLES.get(index).is_some_and(|(_, needed)| wins >= *needed)
}

/// Chosen presentation of the player's identity.
#[derive(Resource, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ProfileCard {
    /// The hero the card shows off.
    pub main_class: HeroClass,
    /// Avatar slug displayed on the card, if the player picked one.
    pub showcase_avatar: Option<String>,
    /// Index into [`widgets::ACCENTS`].
    pub accent: usize,
    /// Index into [`TITLES`].
    pub title: usize,
}

impl Default for ProfileCard {
    fn default() -> Self {
        Self {
            main_class: HeroClass::default(),
            showcase_avatar: crate::passport::selectable_avatars()
                .first()
                .map(|avatar| avatar.slug.clone()),
            accent: 0,
            title: 0,
        }
    }
}

impl ProfileCard {
    pub fn accent_color(&self) -> Color {
        widgets::accent_color(self.accent)
    }

    pub fn title_text(&self, wins: u32) -> &'static str {
        if title_unlocked(self.title, wins) {
            TITLES[self.title.min(TITLES.len() - 1)].0
        } else {
            TITLES[0].0
        }
    }

    /// Keeps a card read from disk inside the ranges the UI can render.
    pub fn sanitized(mut self) -> Self {
        self.accent = self.accent.min(widgets::ACCENTS.len() - 1);
        self.title = self.title.min(TITLES.len() - 1);
        self.showcase_avatar = self
            .showcase_avatar
            .filter(|slug| shared::avatar_definition(slug).is_some());
        self
    }
}

fn card_path() -> Option<PathBuf> {
    crate::platform::preferences_file_path(
        std::env::var("OMOBA_CLIENT_CONFIG_DIR").ok().as_deref(),
        crate::platform::preferences_directory(),
        CARD_FILE,
    )
}

fn load_card(mut commands: Commands) {
    let card = card_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<ProfileCard>(&bytes).ok())
        .map(ProfileCard::sanitized)
        .unwrap_or_default();
    commands.insert_resource(card);
}

fn save_card(card: Res<ProfileCard>, mut ready: Local<bool>) {
    // The load system inserts the resource, which counts as a change; the
    // first frame must not write that back out.
    if !*ready {
        *ready = true;
        return;
    }
    if !card.is_changed() {
        return;
    }
    let Some(path) = card_path() else {
        return;
    };
    match serde_json::to_vec_pretty(card.as_ref()) {
        Ok(bytes) => {
            if let Some(parent) = path.parent()
                && let Err(error) = std::fs::create_dir_all(parent)
            {
                warn!("Failed to create the profile card directory: {error}");
                return;
            }
            if let Err(error) = std::fs::write(&path, bytes) {
                warn!("Failed to save the profile card: {error}");
            }
        }
        Err(error) => warn!("Failed to encode the profile card: {error}"),
    }
}

pub struct ProfileCardPlugin;

impl Plugin for ProfileCardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProfileCard>()
            .add_systems(Startup, load_card)
            .add_systems(Update, save_card)
            .add_systems(OnEnter(AppScreen::Card), spawn_card_screen)
            .add_systems(
                Update,
                (card_actions, refresh_card_screen)
                    .chain()
                    .run_if(in_state(AppScreen::Card)),
            );
    }
}

/// Renders the card itself. Shared by the home screen (read-only) and the
/// customization screen (live preview), so both always agree.
pub fn spawn_card(
    parent: &mut ChildSpawnerCommands,
    card: &ProfileCard,
    profile: Option<&ProfileSummary>,
    nickname: &str,
    thumbnails: &AvatarThumbnails,
) {
    let accent = card.accent_color();
    let wins = profile.map_or(0, |profile| profile.wins);
    parent
        .spawn((
            Node {
                width: Val::Px(360.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(18.0)),
                row_gap: Val::Px(12.0),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(16.0)),
                ..default()
            },
            BackgroundColor(widgets::PANEL),
            BorderColor::all(accent),
            Name::new("ProfileCard"),
        ))
        .with_children(|card_node| {
            card_node
                .spawn(Node {
                    column_gap: Val::Px(14.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| {
                    let portrait = card
                        .showcase_avatar
                        .as_ref()
                        .and_then(|slug| thumbnails.0.get(slug).cloned());
                    let mut avatar_node = row.spawn((
                        Node {
                            width: Val::Px(84.0),
                            height: Val::Px(84.0),
                            border: UiRect::all(Val::Px(2.0)),
                            border_radius: BorderRadius::all(Val::Px(42.0)),
                            ..default()
                        },
                        BackgroundColor(widgets::TILE),
                        BorderColor::all(accent),
                        Name::new("ProfileCardPortrait"),
                    ));
                    if let Some(image) = portrait {
                        avatar_node.insert(ImageNode::new(image));
                    }
                    row.spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(4.0),
                            ..default()
                        },
                        Name::new("ProfileCardIdentity"),
                    ))
                    .with_children(|column| {
                        let name = profile.map_or(nickname, |profile| profile.nickname.as_str());
                        column.spawn(widgets::heading(name, 24.0));
                        column.spawn(widgets::label(card.title_text(wins), 14.0, accent));
                        let level = profile.map_or(1, ProfileSummary::level);
                        column.spawn(widgets::label(
                            &format!("Level {level} · {}", card.main_class.display_name()),
                            13.0,
                            widgets::MUTED,
                        ));
                    });
                });
            let stats = match profile {
                Some(profile) => format!(
                    "Rating {} · {} matches · {}W / {}L",
                    profile.rating, profile.matches_played, profile.wins, profile.losses
                ),
                None => "Career profile not loaded yet".to_owned(),
            };
            card_node.spawn(widgets::label(&stats, 13.0, widgets::IVORY));
            if profile.is_some_and(ProfileSummary::newcomer) {
                card_node.spawn(widgets::label(
                    "Newcomer placement: matched with other new players",
                    12.0,
                    widgets::MUTED,
                ));
            }
        });
}

#[derive(Component, Clone, Copy)]
enum CardAction {
    Back,
    Class(HeroClass),
    Accent(usize),
    Title(usize),
    Showcase,
}

#[derive(Component)]
struct CardPreviewSlot;

fn spawn_card_screen(
    mut commands: Commands,
    card: Res<ProfileCard>,
    career: Res<crate::career::CareerClient>,
    thumbnails: Res<AvatarThumbnails>,
) {
    let wins = career
        .view
        .profile
        .as_ref()
        .map_or(0, |profile| profile.wins);
    commands
        .spawn(widgets::screen_root(AppScreen::Card, "CardScreen"))
        .with_children(|root| {
            root.spawn(Node {
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|header| {
                header.spawn(widgets::heading("Profile card", 30.0));
                widgets::button(
                    header,
                    "Back",
                    ButtonKind::Secondary,
                    CardAction::Back,
                    "CardBack",
                );
            });
            root.spawn(Node {
                column_gap: Val::Px(24.0),
                ..default()
            })
            .with_children(|body| {
                body.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(12.0),
                        ..default()
                    },
                    CardPreviewSlot,
                    Name::new("CardPreviewSlot"),
                ))
                .with_children(|slot| {
                    spawn_card(
                        slot,
                        &card,
                        career.view.profile.as_ref(),
                        &career.nickname,
                        &thumbnails,
                    );
                });
                body.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(18.0),
                        flex_grow: 1.0,
                        ..default()
                    },
                    Name::new("CardEditors"),
                ))
                .with_children(|editors| {
                    editors.spawn(widgets::label("Main hero", 16.0, widgets::MUTED));
                    editors
                        .spawn(Node {
                            column_gap: Val::Px(8.0),
                            flex_wrap: FlexWrap::Wrap,
                            row_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|row| {
                            for class in HeroClass::ALL {
                                widgets::tile(
                                    row,
                                    class.display_name(),
                                    class == card.main_class,
                                    CardAction::Class(class),
                                    &format!("CardClass-{}", class.id()),
                                );
                            }
                        });
                    editors.spawn(widgets::label("Accent", 16.0, widgets::MUTED));
                    editors
                        .spawn(Node {
                            column_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|row| {
                            for (index, (name, color)) in widgets::ACCENTS.iter().enumerate() {
                                row.spawn((
                                    Button,
                                    Node {
                                        width: Val::Px(46.0),
                                        height: Val::Px(46.0),
                                        border: UiRect::all(Val::Px(if index == card.accent {
                                            3.0
                                        } else {
                                            1.0
                                        })),
                                        border_radius: BorderRadius::all(Val::Px(23.0)),
                                        ..default()
                                    },
                                    BackgroundColor(*color),
                                    BorderColor::all(if index == card.accent {
                                        widgets::IVORY
                                    } else {
                                        widgets::PANEL_EDGE
                                    }),
                                    CardAction::Accent(index),
                                    Name::new(format!("CardAccent-{name}")),
                                ));
                            }
                        });
                    editors.spawn(widgets::label("Title", 16.0, widgets::MUTED));
                    editors
                        .spawn(Node {
                            column_gap: Val::Px(8.0),
                            row_gap: Val::Px(8.0),
                            flex_wrap: FlexWrap::Wrap,
                            ..default()
                        })
                        .with_children(|row| {
                            for (index, (name, needed)) in TITLES.iter().enumerate() {
                                let unlocked = wins >= *needed;
                                let text = if unlocked {
                                    (*name).to_owned()
                                } else {
                                    format!("{name} · {needed} wins")
                                };
                                widgets::tile(
                                    row,
                                    &text,
                                    index == card.title && unlocked,
                                    CardAction::Title(index),
                                    &format!("CardTitle-{name}"),
                                );
                            }
                        });
                    editors.spawn(widgets::label(
                        "Showcase avatar is picked in the collection.",
                        13.0,
                        widgets::MUTED,
                    ));
                    editors
                        .spawn(Node {
                            column_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|row| {
                            widgets::button(
                                row,
                                "Open collection",
                                ButtonKind::Secondary,
                                CardAction::Showcase,
                                "CardOpenCollection",
                            );
                        });
                });
            });
        });
}

fn card_actions(
    mut card: ResMut<ProfileCard>,
    career: Res<crate::career::CareerClient>,
    mut next: ResMut<NextState<AppScreen>>,
    buttons: Query<(&Interaction, &CardAction), Changed<Interaction>>,
) {
    let wins = career
        .view
        .profile
        .as_ref()
        .map_or(0, |profile| profile.wins);
    for (interaction, action) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            CardAction::Back => next.set(AppScreen::Home),
            CardAction::Showcase => next.set(AppScreen::Collection),
            CardAction::Class(class) => card.main_class = *class,
            CardAction::Accent(index) => card.accent = *index,
            CardAction::Title(index) => {
                if title_unlocked(*index, wins) {
                    card.title = *index;
                }
            }
        }
    }
}

/// Rebuilds the live preview whenever the card changes, so what the player
/// edits is exactly what the home screen will show.
fn refresh_card_screen(
    mut commands: Commands,
    card: Res<ProfileCard>,
    career: Res<crate::career::CareerClient>,
    thumbnails: Res<AvatarThumbnails>,
    slot: Query<Entity, With<CardPreviewSlot>>,
    mut tiles: Query<(&CardAction, &mut MenuButton)>,
) {
    if !card.is_changed() {
        return;
    }
    let wins = career
        .view
        .profile
        .as_ref()
        .map_or(0, |profile| profile.wins);
    for (action, mut button) in &mut tiles {
        let selected = match action {
            CardAction::Class(class) => *class == card.main_class,
            CardAction::Title(index) => *index == card.title && title_unlocked(*index, wins),
            _ => continue,
        };
        if button.selected != selected {
            button.selected = selected;
        }
    }
    let Ok(slot) = slot.single() else {
        return;
    };
    commands.entity(slot).despawn_related::<Children>();
    commands.entity(slot).with_children(|slot| {
        spawn_card(
            slot,
            &card,
            career.view.profile.as_ref(),
            &career.nickname,
            &thumbnails,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_unlock_by_wins_and_fall_back_when_locked() {
        assert!(title_unlocked(0, 0));
        assert!(!title_unlocked(3, 10));
        let card = ProfileCard {
            title: 3,
            ..Default::default()
        };
        assert_eq!(card.title_text(10), TITLES[0].0);
        assert_eq!(card.title_text(30), TITLES[3].0);
    }

    #[test]
    fn sanitizing_drops_unknown_avatars_and_clamps_indexes() {
        let card = ProfileCard {
            accent: 99,
            title: 99,
            showcase_avatar: Some("not-a-real-avatar".into()),
            ..Default::default()
        }
        .sanitized();
        assert_eq!(card.accent, widgets::ACCENTS.len() - 1);
        assert_eq!(card.title, TITLES.len() - 1);
        assert!(card.showcase_avatar.is_none());
    }
}
