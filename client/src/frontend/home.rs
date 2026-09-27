//! Welcome screen: who you are, what you can look at, and the one button that
//! starts a match.
//!
//! Text comes from the `home` dictionary; the render key includes the locale
//! generation, so a language change rebuilds the screen.
// i18n-strict

use bevy::prelude::*;

use super::card::{ProfileCard, spawn_card};
use super::widgets;
use super::{AppScreen, automation_bypass};
use crate::career::CareerClient;
use crate::i18n::{Locale, data, tr, trf};
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
    Profile,
    Party,
    AcceptInvite(u64),
    DeclineInvite(u64),
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
    /// (party id, inviter) of the newest pending party invite.
    invite: Option<(u64, String)>,
    /// Party size and leader, when in a party.
    party: Option<(usize, String, bool)>,
    /// The locale generation: a language change rebuilds the screen.
    locale: u32,
}

fn signature(
    career: &CareerClient,
    session: &ClientSession,
    card: &ProfileCard,
    party: &crate::party::PartyClient,
    locale: Option<&Locale>,
) -> HomeSignature {
    let profile = career.view.profile.as_ref();
    HomeSignature {
        locale: locale.map_or(0, Locale::generation),
        invite: party
            .view
            .invites
            .last()
            .map(|i| (i.party_id, i.from_nickname.clone())),
        party: party.view.party.as_ref().map(|p| {
            let leader = p
                .members
                .iter()
                .find(|m| m.leader)
                .map_or_else(String::new, |m| m.nickname.clone());
            (p.members.len(), leader, p.leader == party.view.you)
        }),
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
        (Some(winner), Some(team)) if winner == team => tr("home.outcome.victory"),
        (Some(_), Some(_)) => tr("home.outcome.defeat"),
        _ => tr("home.outcome.complete"),
    };
    let minutes = result.duration_ms / 60_000;
    match mine {
        Some(entry) => trf(
            "home.last_match.line",
            &[
                ("outcome", &outcome),
                ("kills", &entry.stats.kills),
                ("deaths", &entry.stats.deaths),
                ("assists", &entry.stats.assists),
                ("hero", &data::hero_name(entry.hero_class)),
                ("minutes", &minutes),
            ],
        ),
        None => trf(
            "home.last_match.short",
            &[("outcome", &outcome), ("minutes", &minutes)],
        ),
    }
}

/// Shared status line: the home header and the picker header both use it.
pub(crate) fn connection_line(session: &ClientSession) -> (String, Color) {
    match session.state() {
        ClientConnectionState::Connected => {
            (tr("home.connection.online").to_owned(), theme::PRIMARY)
        }
        ClientConnectionState::Connecting | ClientConnectionState::WaitingForServer => {
            (tr("home.connection.connecting").to_owned(), theme::GOLD)
        }
        ClientConnectionState::Disconnected => (
            tr("home.connection.offline").to_owned(),
            theme::DANGER_HOVER,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_home(
    mut commands: Commands,
    career: Res<CareerClient>,
    session: Res<ClientSession>,
    card: Res<ProfileCard>,
    thumbnails: Res<AvatarThumbnails>,
    mut preview: ResMut<super::preview::AvatarPreview>,
    platform: Res<crate::ui::UiPlatform>,
    party: Res<crate::party::PartyClient>,
    locale: Option<Res<Locale>>,
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
    let party_line = signature(&career, &session, &card, &party, locale.as_deref());
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
                        title.spawn(widgets::heading("OMOBA", 36.0)); // i18n-allow
                        title.spawn(widgets::label(tr("home.tagline"), 14.0, theme::MUTED));
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
                    column.spawn(widgets::label(tr("home.profile"), 12.0, theme::GOLD));
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
                                tr("home.button.customize_card"),
                                ButtonKind::Secondary,
                                HomeAction::Card,
                                "HomeCustomizeCard",
                            );
                            screen_button(
                                row,
                                tr("home.button.account"),
                                ButtonKind::Secondary,
                                HomeAction::Profile,
                                "HomeAccount",
                            );
                        });
                    if let Some(last) = last_match.as_deref() {
                        column
                            .spawn((widgets::panel_row(), Name::new("HomeLastMatch")))
                            .with_children(|panel| {
                                panel.spawn(widgets::label(
                                    tr("home.last_match"),
                                    12.0,
                                    theme::MUTED,
                                ));
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
                    column.spawn(widgets::label(tr("home.showcase"), 12.0, theme::GOLD));
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
                        .map_or(tr("home.showcase.your_hero"), |avatar| {
                            avatar.display_name.as_str()
                        });
                    column.spawn(widgets::heading(avatar_name, 22.0));
                    column.spawn(widgets::label(
                        data::hero_name(card.main_class),
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
                        column.spawn(widgets::label(tr("home.enter_arena"), 12.0, theme::GOLD));
                    }
                    column.spawn(widgets::heading(tr("home.next_battle"), 24.0));
                    if !phone {
                        column.spawn(widgets::label(tr("home.mode"), 13.0, theme::MUTED));
                    }
                    let play_label = match &party_line.party {
                        Some((size, _, true)) => trf("home.play.party", &[("size", size)]),
                        Some(_) => tr("home.play.party_lobby").to_owned(),
                        None if career.view.match_service.is_some() => {
                            tr("home.play.quick_match").to_owned()
                        }
                        None => tr("home.play.play").to_owned(),
                    };
                    screen_button(
                        column,
                        &play_label,
                        ButtonKind::Primary,
                        HomeAction::Play,
                        "HomePlay",
                    );
                    if let Some((_, leader, false)) = &party_line.party {
                        column.spawn(widgets::label(
                            &trf("home.party_leader", &[("leader", leader)]),
                            12.0,
                            theme::GOLD,
                        ));
                    }
                    screen_button(
                        column,
                        tr("home.button.offline_practice"),
                        ButtonKind::Secondary,
                        HomeAction::OfflinePractice,
                        "HomeOfflinePractice",
                    );
                    column.spawn(widgets::label(tr("home.offline_hint"), 12.0, theme::MUTED));
                    if career.view.match_service.is_some() {
                        screen_button(
                            column,
                            tr("home.button.humans_only"),
                            ButtonKind::Secondary,
                            HomeAction::HumansOnly,
                            "HomeHumansOnly",
                        );
                        screen_button(
                            column,
                            tr("home.button.bot_practice"),
                            ButtonKind::Secondary,
                            HomeAction::BotPractice,
                            "HomeBotPractice",
                        );
                        column.spawn(widgets::label(tr("home.quick_hint"), 12.0, theme::MUTED));
                    }
                    if !phone {
                        column.spawn(widgets::label(tr("home.slogan"), 14.0, theme::MUTED));
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
                                    tr("home.button.avatars"),
                                    ButtonKind::Secondary,
                                    HomeAction::Collection,
                                    "HomeCollection",
                                );
                                screen_button(
                                    row,
                                    tr("home.button.history"),
                                    ButtonKind::Secondary,
                                    HomeAction::History,
                                    "HomeHistory",
                                );
                                screen_button(
                                    row,
                                    tr("home.button.party"),
                                    ButtonKind::Secondary,
                                    HomeAction::Party,
                                    "HomeParty",
                                );
                            });
                    }
                });
            });

            if let Some((party_id, from)) = &party_line.invite {
                spawn_invite_banner(root, *party_id, from);
            }
            if phone {
                root.spawn(Node {
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    column_gap: Val::Px(12.0),
                    ..default()
                })
                .with_children(|footer| {
                    footer.spawn(widgets::label(tr("home.phone_footer"), 12.0, theme::MUTED));
                    footer
                        .spawn(Node {
                            column_gap: Val::Px(12.0),
                            ..default()
                        })
                        .with_children(|navigation| {
                            screen_button(
                                navigation,
                                tr("home.button.avatars"),
                                ButtonKind::Secondary,
                                HomeAction::Collection,
                                "HomeCollection",
                            );
                            screen_button(
                                navigation,
                                tr("home.button.history"),
                                ButtonKind::Secondary,
                                HomeAction::History,
                                "HomeHistory",
                            );
                            screen_button(
                                navigation,
                                tr("home.button.party"),
                                ButtonKind::Secondary,
                                HomeAction::Party,
                                "HomeParty",
                            );
                        });
                });
            } else {
                root.spawn(widgets::label(tr("home.footer"), 12.0, theme::MUTED));
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
    party: Option<Res<crate::party::PartyClient>>,
) {
    let in_party = party.as_ref().is_some_and(|p| p.in_party());
    let leads = party.as_ref().is_some_and(|p| p.view.is_leader());
    for Activated { action, .. } in activated.read() {
        match action {
            HomeAction::Play | HomeAction::HumansOnly | HomeAction::BotPractice if in_party => {
                // A party plays together: the leader launches it, a member
                // waits for the leader in the lobby.
                if leads {
                    let preference = match *action {
                        HomeAction::HumansOnly => {
                            shared::match_service::MatchPreference::HumansOnly
                        }
                        HomeAction::BotPractice => {
                            shared::match_service::MatchPreference::BotPractice
                        }
                        _ => shared::match_service::MatchPreference::Quick,
                    };
                    requests.write(NetworkCommand::Party(shared::party::PartyCommand::Launch {
                        preference,
                    }));
                } else {
                    next.set(AppScreen::Lobby);
                }
            }
            HomeAction::Party => next.set(AppScreen::Lobby),
            HomeAction::AcceptInvite(party_id) => {
                requests.write(NetworkCommand::Party(shared::party::PartyCommand::Accept {
                    party_id: *party_id,
                }));
                next.set(AppScreen::Lobby);
            }
            HomeAction::DeclineInvite(party_id) => {
                requests.write(NetworkCommand::Party(
                    shared::party::PartyCommand::Decline {
                        party_id: *party_id,
                    },
                ));
            }
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
    party: Res<crate::party::PartyClient>,
    roots: Query<Entity, With<HomeRoot>>,
    locale: Option<Res<Locale>>,
    mut last: Local<Option<HomeSignature>>,
) {
    let current = signature(&career, &session, &card, &party, locale.as_deref());
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
        commands, career, session, card, thumbnails, preview, platform, party, locale,
    );
}

/// "X invites you to a party" with Accept / Decline: a toast centred at the
/// top of the screen, over the layout rather than inside a column.
fn spawn_invite_banner(parent: &mut ChildSpawnerCommands, party_id: u64, from: &str) {
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(20.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|strip| {
            strip
                .spawn((
                    Node {
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(10.0),
                        padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(10.0)),
                        ..default()
                    },
                    BackgroundColor(theme::TILE_SELECTED),
                    BorderColor::all(theme::GOLD),
                    Name::new("HomePartyInvite"),
                ))
                .with_children(|banner| {
                    banner.spawn(widgets::label(
                        &trf("home.invite", &[("name", &from)]),
                        14.0,
                        theme::IVORY,
                    ));
                    screen_button(
                        banner,
                        tr("home.button.accept"),
                        ButtonKind::Secondary,
                        HomeAction::AcceptInvite(party_id),
                        "HomeAcceptInvite",
                    );
                    screen_button(
                        banner,
                        tr("home.button.decline"),
                        ButtonKind::Secondary,
                        HomeAction::DeclineInvite(party_id),
                        "HomeDeclineInvite",
                    );
                });
        });
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

    /// The home screen is a render-key screen: a language change rebuilds it
    /// in the new language (buttons, headings and the status line).
    #[test]
    fn a_language_change_rebuilds_the_home_screen_in_that_language() {
        if crate::i18n::testing::isolated(
            "frontend::home::tests::a_language_change_rebuilds_the_home_screen_in_that_language",
        ) {
            return;
        }
        use crate::i18n::{I18nPlugin, LocaleId};
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.add_plugins((bevy::state::app::StatesPlugin, I18nPlugin::default()))
            .init_state::<AppScreen>()
            .init_resource::<Assets<Image>>()
            .init_resource::<super::super::preview::AvatarPreview>()
            .init_resource::<CareerClient>()
            .init_resource::<ClientSession>()
            .init_resource::<ProfileCard>()
            .init_resource::<AvatarThumbnails>()
            .init_resource::<crate::party::PartyClient>()
            .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
            .add_systems(Startup, spawn_home)
            .add_systems(Update, refresh_home);
        app.update();
        let label = |app: &mut App, id: &str| {
            let button = harness::find(app.world_mut(), id).unwrap();
            let child = app.world().get::<Children>(button).unwrap()[0];
            app.world().get::<Text>(child).unwrap().0.clone()
        };
        let status = |app: &mut App| {
            let (_, text) = app
                .world_mut()
                .query::<(&Name, &Text)>()
                .iter(app.world())
                .find(|(name, _)| name.as_str() == "HomeConnectionStatus")
                .map(|(name, text)| (name.clone(), text.0.clone()))
                .unwrap();
            text
        };
        assert_eq!(label(&mut app, "HomePlay"), "PLAY");
        assert_eq!(label(&mut app, "HomeCustomizeCard"), "Customize card");
        let english_status = status(&mut app);
        app.world_mut()
            .resource_mut::<Locale>()
            .set(LocaleId::parse("zh-Hans").unwrap());
        app.update();
        assert_eq!(label(&mut app, "HomePlay"), "开始游戏");
        assert_eq!(label(&mut app, "HomeCustomizeCard"), "自定义名片");
        assert_eq!(label(&mut app, "HomeHistory"), "对局记录");
        assert_ne!(status(&mut app), english_status);
        assert_eq!(
            status(&mut app),
            connection_line(&ClientSession::default()).0
        );
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
