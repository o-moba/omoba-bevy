//! Welcome screen: who you are, what you can look at, and the one button that
//! starts a match.

use bevy::prelude::*;

use super::card::{ProfileCard, spawn_card};
use super::widgets;
use super::{AppScreen, automation_bypass};
use crate::career::CareerClient;
use crate::net::{ClientConnectionState, ClientSession, NetworkCommand};
use crate::team::AvatarThumbnails;
use crate::ui::theme::{self, ButtonKind};
use crate::ui::widgets::screen_button;
use crate::ui::{Activated, UiActionAppExt, UiSet};

pub struct HomeScreenPlugin;

impl Plugin for HomeScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_ui_action::<HomeAction>()
            .add_systems(OnEnter(AppScreen::Home), spawn_home)
            .add_systems(
                Update,
                (home_actions, refresh_home)
                    .chain()
                    .after(UiSet::Dispatch)
                    .run_if(in_state(AppScreen::Home)),
            );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HomeAction {
    Play,
    HumansOnly,
    BotPractice,
    OfflinePractice,
    Card,
    Collection,
    History,
    Friends,
    Profile,
}

#[derive(Component)]
struct HomeRoot;

/// What the home screen renders from. When it changes, the screen is rebuilt.
#[derive(PartialEq, Clone)]
struct HomeSignature {
    nickname: String,
    rating: i32,
    matches: u32,
    wins: u32,
    losses: u32,
    connection: ClientConnectionState,
    card: ProfileCard,
    last_result: Option<String>,
    public_matchmaking: bool,
}

fn signature(career: &CareerClient, session: &ClientSession, card: &ProfileCard) -> HomeSignature {
    let profile = career.view.profile.as_ref();
    HomeSignature {
        nickname: profile.map_or_else(
            || career.nickname.clone(),
            |profile| profile.nickname.clone(),
        ),
        rating: profile.map_or(0, |profile| profile.rating),
        matches: profile.map_or(0, |profile| profile.matches_played),
        wins: profile.map_or(0, |profile| profile.wins),
        losses: profile.map_or(0, |profile| profile.losses),
        connection: session.state(),
        card: card.clone(),
        public_matchmaking: career.view.match_service.is_some(),
        last_result: career
            .view
            .last_result
            .as_ref()
            .map(|result| result.result_id.clone()),
    }
}

/// One line about the player's own last match, for the home screen.
pub fn last_match_line(result: &shared::career::MatchResult, profile_id: Option<&str>) -> String {
    let mine = profile_id.and_then(|id| {
        result
            .participants
            .iter()
            .find(|entry| entry.profile_id.as_deref() == Some(id))
    });
    let outcome = match (result.winner, mine.map(|entry| entry.team)) {
        (Some(winner), Some(team)) if winner == team => "Victory",
        (Some(_), Some(_)) => "Defeat",
        _ => "Match complete",
    };
    let minutes = result.duration_ms / 60_000;
    match mine {
        Some(entry) => format!(
            "{outcome} · {}/{}/{} · {} · {minutes} min",
            entry.stats.kills,
            entry.stats.deaths,
            entry.stats.assists,
            entry.hero_class.display_name(),
        ),
        None => format!("{outcome} · {minutes} min"),
    }
}

/// Shared status line: the home header and the picker header both use it.
pub(crate) fn connection_line(session: &ClientSession) -> (String, Color) {
    match session.state() {
        ClientConnectionState::Connected => ("Online · ready to play".to_owned(), theme::PRIMARY),
        ClientConnectionState::Connecting | ClientConnectionState::WaitingForServer => {
            ("Connecting…".to_owned(), theme::GOLD)
        }
        ClientConnectionState::Disconnected => {
            ("Offline · reconnecting…".to_owned(), theme::DANGER_HOVER)
        }
    }
}

fn spawn_home(
    mut commands: Commands,
    career: Res<CareerClient>,
    session: Res<ClientSession>,
    card: Res<ProfileCard>,
    thumbnails: Res<AvatarThumbnails>,
    mut preview: ResMut<super::preview::AvatarPreview>,
    platform: Res<crate::ui::UiPlatform>,
) {
    if automation_bypass() {
        return;
    }
    // The home screen shows the card's hero in 3D.
    if let Some(slug) = card.showcase_avatar.as_deref() {
        preview.show_portrait(slug);
    }
    let preview_image = preview.image.clone();
    let phone = platform.is_mobile();
    let (status, status_color) = connection_line(&session);
    let profile = career.view.profile.clone();
    let last_match = career
        .view
        .last_result
        .as_ref()
        .map(|result| last_match_line(result, career.public_profile_id.as_deref()));
    commands
        .spawn((
            widgets::screen_root(AppScreen::Home, "HomeScreen"),
            HomeRoot,
        ))
        .with_children(|root| {
            root.spawn(Node {
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|header| {
                header
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(2.0),
                        ..default()
                    })
                    .with_children(|title| {
                        title.spawn(widgets::heading("OMOBA", 36.0));
                        title.spawn(widgets::label("THE VERDANT ARENA", 14.0, theme::MUTED));
                    });
                header.spawn((
                    widgets::label(&status, 15.0, status_color),
                    Node {
                        // The phone bar (?, MENU, SERVER) owns the corner.
                        margin: UiRect::right(Val::Px(if phone { 400.0 } else { 0.0 })),
                        ..default()
                    },
                    Name::new("HomeConnectionStatus"),
                ));
            });

            root.spawn((
                Node {
                    flex_grow: 1.0,
                    column_gap: Val::Px(20.0),
                    min_height: Val::Px(0.0),
                    padding: UiRect::all(Val::Px(if phone { 12.0 } else { 20.0 })),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(theme::PANEL_OPAQUE),
                BorderColor::all(theme::PANEL_EDGE),
            ))
            .with_children(|body| {
                body.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(12.0),
                        width: Val::Px(320.0),
                        flex_shrink: 0.0,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    Name::new("HomeIdentity"),
                ))
                .with_children(|column| {
                    column.spawn(widgets::label("PLAYER PROFILE", 12.0, theme::GOLD));
                    spawn_card(
                        column,
                        &card,
                        profile.as_ref(),
                        &career.nickname,
                        &thumbnails,
                    );
                    column
                        .spawn(Node {
                            column_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|row| {
                            screen_button(
                                row,
                                "Customize card",
                                ButtonKind::Secondary,
                                HomeAction::Card,
                                "HomeCustomizeCard",
                            );
                            screen_button(
                                row,
                                "Account",
                                ButtonKind::Secondary,
                                HomeAction::Profile,
                                "HomeAccount",
                            );
                        });
                    if let Some(last) = last_match.as_deref() {
                        column
                            .spawn((widgets::panel_row(), Name::new("HomeLastMatch")))
                            .with_children(|panel| {
                                panel.spawn(widgets::label("Last match", 12.0, theme::MUTED));
                                panel.spawn(widgets::label(last, 14.0, theme::IVORY));
                            });
                    }
                });

                // Live showcase of the card's hero, between the identity and
                // the actions.
                body.spawn((
                    Node {
                        flex_grow: 1.0,
                        // Zero basis: the showcase takes what is left after the
                        // card and the action rail, never pushing them out.
                        flex_basis: Val::Px(0.0),
                        min_width: Val::Px(0.0),
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    Name::new("HomeShowcase"),
                ))
                .with_children(|column| {
                    column.spawn(widgets::label("YOUR CHAMPION", 12.0, theme::GOLD));
                    column.spawn((
                        ImageNode::new(preview_image),
                        Node {
                            // Sized from the window height so the showcase
                            // scales with the window and never overflows it.
                            width: Val::Vh(46.0),
                            max_width: Val::Percent(100.0),
                            min_width: Val::Px(0.0),
                            aspect_ratio: Some(0.742),
                            border_radius: BorderRadius::all(Val::Px(14.0)),
                            ..default()
                        },
                        Name::new("HomeShowcaseImage"),
                    ));
                    let avatar_name = card
                        .showcase_avatar
                        .as_deref()
                        .and_then(omoba_passport::avatars::avatar_definition)
                        .map_or("Your hero", |avatar| avatar.display_name.as_str());
                    column.spawn(widgets::heading(avatar_name, 22.0));
                    column.spawn(widgets::label(
                        card.main_class.display_name(),
                        13.0,
                        theme::MUTED,
                    ));
                });

                body.spawn((
                    Node {
                        width: Val::Px(260.0),
                        flex_shrink: 0.0,
                        padding: UiRect::vertical(Val::Px(if phone { 0.0 } else { 24.0 })),
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(if phone { 10.0 } else { 16.0 }),
                        ..default()
                    },
                    Name::new("HomePlayColumn"),
                ))
                .with_children(|column| {
                    if !phone {
                        column.spawn(widgets::label("ENTER THE ARENA", 12.0, theme::GOLD));
                    }
                    column.spawn(widgets::heading("Your next battle", 24.0));
                    if !phone {
                        column.spawn(widgets::label(
                            "5 versus 5 · Team strategy",
                            13.0,
                            theme::MUTED,
                        ));
                    }
                    screen_button(
                        column,
                        if career.view.match_service.is_some() { "QUICK MATCH" } else { "PLAY" },
                        ButtonKind::Primary,
                        HomeAction::Play,
                        "HomePlay",
                    );
                    screen_button(column, "Offline practice", ButtonKind::Secondary, HomeAction::OfflinePractice, "HomeOfflinePractice");
                    column.spawn(widgets::label("No internet needed · No rating or rewards", 12.0, theme::MUTED));
                    if career.view.match_service.is_some() {
                        screen_button(column, "Wait for players", ButtonKind::Secondary, HomeAction::HumansOnly, "HomeHumansOnly");
                        screen_button(column, "Play with bots", ButtonKind::Secondary, HomeAction::BotPractice, "HomeBotPractice");
                        column.spawn(widgets::label("Quick match fills empty seats with bots.\nProgress in every match · rating in PvP.", 12.0, theme::MUTED));
                    }
                    if !phone {
                        column.spawn(widgets::label(
                            "Choose a hero. Make your mark.",
                            14.0,
                            theme::MUTED,
                        ));
                    }
                    if !phone {
                    column
                        .spawn(Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Stretch,
                            width: Val::Percent(100.0),
                            row_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|row| {
                            screen_button(
                                row,
                                "Avatars",
                                ButtonKind::Secondary,
                                HomeAction::Collection,
                                "HomeCollection",
                            );
                            screen_button(
                                row,
                                "Match history",
                                ButtonKind::Secondary,
                                HomeAction::History,
                                "HomeHistory",
                            );
                            screen_button(
                                row,
                                "Friends",
                                ButtonKind::Secondary,
                                HomeAction::Friends,
                                "HomeFriends",
                            );
                        });
                    }
                });
            });

            if phone {
                root.spawn(Node {
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    column_gap: Val::Px(12.0),
                    ..default()
                }).with_children(|footer| {
                    footer.spawn(widgets::label("MENU opens settings · SERVER sets the address", 12.0, theme::MUTED));
                    footer.spawn(Node { column_gap: Val::Px(12.0), ..default() })
                        .with_children(|navigation| {
                            screen_button(navigation, "Avatars", ButtonKind::Secondary, HomeAction::Collection, "HomeCollection");
                            screen_button(navigation, "Match history", ButtonKind::Secondary, HomeAction::History, "HomeHistory");
                            screen_button(navigation, "Friends", ButtonKind::Secondary, HomeAction::Friends, "HomeFriends");
                        });
                });
            } else {
            root.spawn(widgets::label(
                "Escape · Settings                         OMOBA · Verdant Arena",
                12.0,
                theme::MUTED,
            ));
            }
        });
}

fn home_actions(
    mut next: ResMut<NextState<AppScreen>>,
    mut career: ResMut<CareerClient>,
    mut requests: MessageWriter<NetworkCommand>,
    mut session_ui: MessageWriter<crate::net::SessionUiCommand>,
    session: Res<ClientSession>,
    mut matchmaking: ResMut<crate::match_service::MatchServiceClient>,
    mut activated: MessageReader<Activated<HomeAction>>,
) {
    for Activated { action, .. } in activated.read() {
        match action {
            HomeAction::OfflinePractice => {
                session_ui.write(crate::net::SessionUiCommand::StartOffline);
                next.set(AppScreen::HeroSelect);
            }
            HomeAction::Play | HomeAction::HumansOnly | HomeAction::BotPractice => {
                // Backing out of the practice picker restores the saved server first.
                if session.is_offline() {
                    session_ui.write(crate::net::SessionUiCommand::LeaveMatch);
                }

                matchmaking.preference = match *action {
                    HomeAction::HumansOnly => shared::match_service::MatchPreference::HumansOnly,
                    HomeAction::BotPractice => shared::match_service::MatchPreference::BotPractice,
                    _ => shared::match_service::MatchPreference::Quick,
                };
                next.set(AppScreen::HeroSelect);
            }
            HomeAction::Card => next.set(AppScreen::Card),
            HomeAction::Collection => next.set(AppScreen::Collection),
            HomeAction::History => crate::career::open_history_modal(&mut career, &mut requests),
            HomeAction::Friends => crate::career::open_friends_modal(&mut career, &mut requests),
            HomeAction::Profile => career.open_profile_modal(),
        }
    }
}

/// The career view arrives over the network after the screen is already up, so
/// the screen rebuilds itself when the facts it renders change.
fn refresh_home(
    mut commands: Commands,
    career: Res<CareerClient>,
    session: Res<ClientSession>,
    card: Res<ProfileCard>,
    thumbnails: Res<AvatarThumbnails>,
    preview: ResMut<super::preview::AvatarPreview>,
    platform: Res<crate::ui::UiPlatform>,
    roots: Query<Entity, With<HomeRoot>>,
    mut last: Local<Option<HomeSignature>>,
) {
    let current = signature(&career, &session, &card);
    if last.as_ref() == Some(&current) {
        return;
    }
    *last = Some(current);
    let Ok(root) = roots.single() else {
        return;
    };
    commands
        .entity(root)
        .despawn_related::<Children>()
        .despawn();
    spawn_home(
        commands, career, session, card, thumbnails, preview, platform,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_line_reports_every_session_state() {
        for state in [
            ClientConnectionState::Connected,
            ClientConnectionState::Connecting,
            ClientConnectionState::WaitingForServer,
            ClientConnectionState::Disconnected,
        ] {
            let mut session = ClientSession::default();
            session.set_state_for_test(state);
            let (line, _) = connection_line(&session);
            assert!(!line.is_empty(), "{state:?} must have a status line");
        }
    }

    #[test]
    fn a_press_dispatches_its_action_once_and_a_disabled_button_does_nothing() {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<AppScreen>()
            .init_resource::<CareerClient>()
            .init_resource::<ClientSession>()
            .init_resource::<crate::match_service::MatchServiceClient>()
            .add_message::<NetworkCommand>()
            .add_message::<crate::net::SessionUiCommand>()
            .add_ui_action::<HomeAction>()
            .add_systems(Update, home_actions.after(UiSet::Dispatch));
        harness::spawn_ui(app.world_mut(), |root| {
            screen_button(
                root,
                "Customize card",
                ButtonKind::Secondary,
                HomeAction::Card,
                "HomeCustomizeCard",
            );
            screen_button(
                root,
                "Avatars",
                ButtonKind::Secondary,
                HomeAction::Collection,
                "HomeCollection",
            );
        });
        app.update();
        harness::press(app.world_mut(), "HomeCustomizeCard");
        app.update();
        assert_eq!(
            harness::drain_actions::<HomeAction>(app.world_mut()),
            [HomeAction::Card]
        );
        assert!(matches!(
            *app.world().resource::<NextState<AppScreen>>(),
            NextState::Pending(AppScreen::Card)
        ));
        app.update();
        assert!(harness::drain_actions::<HomeAction>(app.world_mut()).is_empty());
        assert_eq!(
            *app.world().resource::<State<AppScreen>>().get(),
            AppScreen::Card
        );
        harness::set_disabled(app.world_mut(), "HomeCollection", true);
        harness::press(app.world_mut(), "HomeCollection");
        app.update();
        assert!(harness::drain_actions::<HomeAction>(app.world_mut()).is_empty());
        assert!(matches!(
            *app.world().resource::<NextState<AppScreen>>(),
            NextState::Unchanged
        ));
    }
}
