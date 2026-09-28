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
use crate::ui::kit_assets::Icon;
use crate::ui::living_background::{self, LivingBands, LivingScene};
use crate::ui::theme::{self, ButtonKind, TextStyle};
use crate::ui::tokens::{Metric, TextRole, border, color, radius, space};
use crate::ui::widgets::{self as kit, ButtonSize, screen_button};
use crate::ui::{Activated, Pressable, UiActionAppExt, UiSet};

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
    BotPractice,
    OfflinePractice,
    Card,
    Collection,
    History,
    Profile,
    Party,
    Help,
    Settings,
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
    /// Phone menu scaling settles after the window exists; rebuild once it
    /// does so screen-owned redline geometry uses the final scale.
    ui_scale_bits: u32,
}

fn signature(
    career: &CareerClient,
    session: &ClientSession,
    card: &ProfileCard,
    party: &crate::party::PartyClient,
    locale: Option<&Locale>,
    ui_scale: f32,
) -> HomeSignature {
    let profile = career.view.profile.as_ref();
    HomeSignature {
        locale: locale.map_or(0, Locale::generation),
        ui_scale_bits: ui_scale.to_bits(),
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
        ClientConnectionState::Disconnected => {
            (tr("home.connection.offline").to_owned(), theme::DANGER)
        }
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
    ui_scale: Option<Res<UiScale>>,
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
    let form = theme::Form::of(phone);
    let ui_scale_value = ui_scale.as_ref().map_or(1.0, |scale| scale.0);
    let unit = if phone {
        1.0 / ui_scale_value.max(0.1)
    } else {
        1.0
    };
    let party_line = signature(
        &career,
        &session,
        &card,
        &party,
        locale.as_deref(),
        ui_scale_value,
    );
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
        .insert(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|root| {
            living_background::spawn(
                root,
                LivingScene::Stage,
                if phone {
                    LivingBands::default()
                } else {
                    LivingBands {
                        header: Some(104.0),
                        footer: Some(56.0),
                    }
                },
                form,
            );
            if !phone {
                root.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(8.0),
                        right: Val::Px(8.0),
                        top: Val::Px(8.0),
                        bottom: Val::Px(8.0),
                        ..default()
                    },
                    crate::ui::kit_assets::KitImage::frame(crate::ui::kit_assets::Frame::Ornament),
                    Pickable::IGNORE,
                    Name::new("HomeOrnamentFrame"),
                ));
                root.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(40.0),
                        top: Val::Px(28.0),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(2.0),
                        ..default()
                    },
                    Name::new("HomeBrand"),
                ))
                .with_children(|brand| {
                    brand.spawn(widgets::heading("OMOBA", 34.0)); // i18n-allow
                    brand.spawn(widgets::label(tr("home.tagline"), 13.0, theme::MUTED));
                });
                root.spawn((
                    widgets::label(&status, 15.0, status_color),
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(140.0),
                        top: Val::Px(36.0),
                        ..default()
                    },
                    Name::new("HomeConnectionStatus"),
                ));
                root.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(40.0),
                        top: Val::Px(28.0),
                        column_gap: Val::Px(space::S8),
                        ..default()
                    },
                    Name::new("HomeUtilityButtons"),
                ))
                .with_children(|buttons| {
                    kit::controls::sized_icon_button(
                        buttons,
                        Icon::NavHelpCircle,
                        form,
                        ButtonKind::Secondary,
                        HomeAction::Help,
                        "HomeHelp",
                    );
                    kit::controls::sized_icon_button(
                        buttons,
                        Icon::NavSettings,
                        form,
                        ButtonKind::Secondary,
                        HomeAction::Settings,
                        "HomeSettings",
                    );
                });
            } else {
                // The phone utility bar owns the top-right corner. Keep the
                // status in the accessibility tree while avoiding a visual
                // collision with Help / Menu / Server.
                root.spawn((
                    widgets::label(&status, 14.0, status_color),
                    Node {
                        display: Display::None,
                        ..default()
                    },
                    Name::new("HomeConnectionStatus"),
                ));
            }

            spawn_home_identity(
                root,
                phone,
                &card,
                profile.as_ref(),
                &career.nickname,
                &thumbnails,
                last_match.as_deref(),
                unit,
            );

            let avatar_name = card
                .showcase_avatar
                .as_deref()
                .and_then(omoba_passport::avatars::avatar_definition)
                .map_or(tr("home.showcase.your_hero"), |avatar| {
                    avatar.display_name.as_str()
                });
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(if phone { 300.0 * unit } else { 470.0 }),
                    top: Val::Px(if phone { 40.0 * unit } else { 96.0 }),
                    width: Val::Px(if phone { 240.0 * unit } else { 340.0 }),
                    height: Val::Px(if phone { 250.0 * unit } else { 440.0 }),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    overflow: Overflow::clip(),
                    ..default()
                },
                Name::new("HomeShowcase"),
            ))
            .with_children(|stage| {
                stage.spawn((
                    ImageNode::new(preview_image),
                    Node {
                        width: Val::Px(if phone { 185.0 * unit } else { 326.0 }),
                        aspect_ratio: Some(0.742),
                        flex_shrink: 1.0,
                        ..default()
                    },
                    Name::new("HomeShowcaseImage"),
                ));
            });
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(if phone { 330.0 * unit } else { 520.0 }),
                    top: Val::Px(if phone { 276.0 * unit } else { 540.0 }),
                    width: Val::Px(if phone { 180.0 * unit } else { 240.0 }),
                    height: Val::Px(if phone { 36.0 * unit } else { 64.0 }),
                    padding: UiRect::axes(Val::Px(space::S12), Val::Px(space::S4)),
                    flex_direction: if phone {
                        FlexDirection::Row
                    } else {
                        FlexDirection::Column
                    },
                    column_gap: Val::Px(space::S8),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(border::HAIRLINE)),
                    border_radius: BorderRadius::all(Val::Px(radius::MD)),
                    ..default()
                },
                BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
                BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)),
                Name::new("HomeHeroPlate"),
            ))
            .with_children(|plate| {
                plate.spawn(widgets::heading(
                    avatar_name,
                    if phone { 16.0 } else { 22.0 },
                ));
                plate.spawn(widgets::label(
                    data::hero_name(card.main_class),
                    12.0,
                    theme::GOLD,
                ));
            });

            let play_label = match &party_line.party {
                Some((size, _, true)) => trf("home.play.party", &[("size", size)]),
                Some(_) => tr("home.play.party_lobby").to_owned(),
                None if career.view.match_service.is_some() => {
                    tr("home.play.quick_match").to_owned()
                }
                None => tr("home.play.play").to_owned(),
            };
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(if phone { 561.0 * unit } else { 880.0 }),
                    top: Val::Px(if phone { 130.0 * unit } else { 204.0 }),
                    width: Val::Px(if phone { 220.0 * unit } else { 360.0 }),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(if phone { 10.0 * unit } else { 8.0 }),
                    ..default()
                },
                Name::new("HomePlayColumn"),
            ))
            .with_children(|column| {
                if !phone {
                    column.spawn(widgets::label(tr("home.enter_arena"), 12.0, theme::GOLD));
                }
                let play = kit::spawn_button(
                    column,
                    if phone {
                        Node {
                            width: Val::Px(220.0 * unit),
                            height: Val::Px(64.0 * unit),
                            padding: UiRect::horizontal(Val::Px(space::S24 * unit)),
                            column_gap: Val::Px(space::S8 * unit),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(Val::Px(radius::MD * unit)),
                            ..default()
                        }
                    } else {
                        kit::button_node(ButtonSize::Hero, ButtonKind::Primary, form)
                    },
                    play_label,
                    TextStyle::new(TextRole::ButtonLg),
                    ButtonKind::Primary,
                    Some(Icon::HudAttack),
                    HomeAction::Play,
                    "HomePlay".into(),
                    (),
                );
                if !matches!(session.state(), ClientConnectionState::Connected) {
                    column.commands().entity(play).insert(Pressable {
                        disabled: true,
                        ..default()
                    });
                }
                if !phone {
                    let mode = party_line.party.as_ref().and_then(|(_, leader, leads)| {
                        (!*leads).then(|| trf("home.party_leader", &[("leader", leader)]))
                    });
                    column.spawn(widgets::label(
                        mode.as_deref().unwrap_or_else(|| tr("home.mode")),
                        13.0,
                        theme::MUTED,
                    ));
                }
                let (secondary_label, secondary_action, secondary_id) =
                    if career.view.match_service.is_some() {
                        (
                            tr("home.button.bot_practice"),
                            HomeAction::BotPractice,
                            "HomeBotPractice",
                        )
                    } else {
                        (
                            tr("home.button.offline_practice"),
                            HomeAction::OfflinePractice,
                            "HomeOfflinePractice",
                        )
                    };
                kit::spawn_button(
                    column,
                    Node {
                        width: Val::Px(if phone { 180.0 * unit } else { 280.0 }),
                        height: Val::Px(if phone { 44.0 * unit } else { 46.0 }),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(space::S8 * unit),
                        border_radius: BorderRadius::all(Val::Px(radius::MD * unit)),
                        ..default()
                    },
                    secondary_label,
                    TextStyle::new(TextRole::Button),
                    ButtonKind::Secondary,
                    Some(Icon::NavBot),
                    secondary_action,
                    secondary_id.into(),
                    (),
                );
            });

            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(if phone { 63.0 * unit } else { 880.0 }),
                    top: Val::Px(if phone { 313.0 * unit } else { 500.0 }),
                    width: Val::Px(if phone { 718.0 * unit } else { 360.0 }),
                    height: Val::Px(if phone { 56.0 * unit } else { 88.0 }),
                    column_gap: Val::Px(if phone { 0.0 } else { space::S8 }),
                    border_radius: BorderRadius::all(Val::Px(radius::LG)),
                    ..default()
                },
                BackgroundColor(if phone {
                    theme::perceptual(color::SURFACE_GLASS_STRONG)
                } else {
                    Color::NONE
                }),
                Name::new("HomeNavigation"),
            ))
            .with_children(|nav| {
                spawn_home_nav(
                    nav,
                    phone,
                    tr("home.button.avatars"),
                    Icon::HudAttack,
                    HomeAction::Collection,
                    "HomeCollection",
                    unit,
                );
                spawn_home_nav(
                    nav,
                    phone,
                    tr("home.button.history"),
                    Icon::NavHistory,
                    HomeAction::History,
                    "HomeHistory",
                    unit,
                );
                spawn_home_nav(
                    nav,
                    phone,
                    tr("home.button.party"),
                    Icon::NavUsers,
                    HomeAction::Party,
                    "HomeParty",
                    unit,
                );
                if phone {
                    spawn_home_nav(
                        nav,
                        true,
                        tr("home.button.account"),
                        Icon::NavUser,
                        HomeAction::Profile,
                        "HomeAccount",
                        unit,
                    );
                } else {
                    spawn_home_nav(
                        nav,
                        false,
                        tr("pause.button.settings"),
                        Icon::NavSettings,
                        HomeAction::Settings,
                        "HomeSettingsTile",
                        unit,
                    );
                }
            });

            if let Some((party_id, from)) = &party_line.invite {
                spawn_invite_banner(root, *party_id, from);
            }
            if !phone {
                root.spawn((
                    widgets::label(tr("home.footer"), 12.0, theme::MUTED),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(40.0),
                        bottom: Val::Px(24.0),
                        ..default()
                    },
                ));
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn spawn_home_identity(
    root: &mut ChildSpawnerCommands,
    phone: bool,
    card: &ProfileCard,
    profile: Option<&shared::career::ProfileSummary>,
    fallback_nickname: &str,
    thumbnails: &AvatarThumbnails,
    last_match: Option<&str>,
    unit: f32,
) {
    if !phone {
        root.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(40.0),
                top: Val::Px(248.0),
                width: Val::Px(380.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(space::S8),
                ..default()
            },
            Name::new("HomeIdentity"),
        ))
        .with_children(|column| {
            column.spawn(widgets::label(tr("home.profile"), 12.0, theme::GOLD));
            spawn_card(column, card, profile, fallback_nickname, thumbnails);
            column
                .spawn(Node {
                    width: Val::Px(332.0),
                    column_gap: Val::Px(space::S8),
                    ..default()
                })
                .with_children(|buttons| {
                    let customize = screen_button(
                        buttons,
                        tr("home.button.customize_card"),
                        ButtonKind::Secondary,
                        HomeAction::Card,
                        "HomeCustomizeCard",
                    );
                    buttons.commands().entity(customize).insert(Node {
                        width: Val::Px(278.0),
                        height: Val::Px(46.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..kit::button_node(
                            ButtonSize::Regular,
                            ButtonKind::Secondary,
                            theme::Form::Desktop,
                        )
                    });
                    kit::controls::sized_icon_button(
                        buttons,
                        Icon::SettingsUserCog,
                        theme::Form::Desktop,
                        ButtonKind::Secondary,
                        HomeAction::Profile,
                        "HomeAccount",
                    );
                });
            if let Some(last) = last_match {
                column
                    .spawn((
                        Node {
                            width: Val::Px(332.0),
                            padding: UiRect::axes(Val::Px(space::S12), Val::Px(space::S8)),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(space::S4),
                            border: UiRect::all(Val::Px(border::HAIRLINE)),
                            border_radius: BorderRadius::all(Val::Px(radius::MD)),
                            ..default()
                        },
                        BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
                        BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)),
                        Name::new("HomeLastMatch"),
                    ))
                    .with_children(|panel| {
                        panel.spawn(widgets::label(tr("home.last_match"), 12.0, theme::MUTED));
                        panel.spawn(widgets::label(last, 13.0, theme::IVORY));
                    });
            }
        });
        return;
    }

    let accent = card.accent_color();
    let wins = profile.map_or(0, |profile| profile.wins);
    let name = profile.map_or(fallback_nickname, |profile| profile.nickname.as_str());
    let level = profile.map_or(1, shared::career::ProfileSummary::level);
    root.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(63.0 * unit),
            top: Val::Px(10.0 * unit),
            width: Val::Px(300.0 * unit),
            height: Val::Px(48.0 * unit),
            padding: UiRect::axes(Val::Px(space::S8 * unit), Val::Px(space::S4 * unit)),
            column_gap: Val::Px(space::S8 * unit),
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::PILL)),
            ..default()
        },
        BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
        BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)),
        Name::new("HomeIdentity"),
    ))
    .with_children(|chip| {
        let portrait = card
            .showcase_avatar
            .as_ref()
            .and_then(|slug| thumbnails.0.get(slug).cloned());
        let mut avatar = chip.spawn((
            Node {
                width: Val::Px(40.0 * unit),
                height: Val::Px(40.0 * unit),
                flex_shrink: 0.0,
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                ..default()
            },
            BackgroundColor(theme::TILE),
            BorderColor::all(accent),
            Name::new("HomeIdentityPortrait"),
        ));
        if let Some(image) = portrait {
            avatar.insert(ImageNode::new(image));
        }
        chip.spawn(Node {
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            min_width: Val::Px(0.0),
            ..default()
        })
        .with_children(|text| {
            text.spawn(widgets::heading(name, 16.0));
            text.spawn(widgets::label(
                &format!(
                    "{} · {}",
                    card.title_text(wins),
                    trf(
                        "card.level_class",
                        &[
                            ("level", &level),
                            ("hero", &data::hero_name(card.main_class))
                        ]
                    )
                ),
                11.0,
                accent,
            ));
        });
    });
    if let Some(last) = last_match {
        root.spawn((
            widgets::label(last, 12.0, theme::IVORY),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(63.0 * unit),
                top: Val::Px(64.0 * unit),
                width: Val::Px(260.0 * unit),
                height: Val::Px(40.0 * unit),
                padding: UiRect::horizontal(Val::Px(space::S12 * unit)),
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                ..default()
            },
            BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
            Name::new("HomeLastMatch"),
        ));
    }
}

fn spawn_home_nav(
    parent: &mut ChildSpawnerCommands,
    phone: bool,
    label: &'static str,
    icon: Icon,
    action: HomeAction,
    id: &'static str,
    unit: f32,
) {
    kit::spawn_button(
        parent,
        Node {
            width: Val::Px(if phone { 179.5 * unit } else { 84.0 }),
            height: Val::Px(if phone { 56.0 * unit } else { 88.0 }),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: Val::Px(space::S4),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            ..default()
        },
        label,
        TextStyle::new(TextRole::Button).sized(Metric::new(13.0, 12.0)),
        ButtonKind::Secondary,
        Some(icon),
        action,
        id.into(),
        (),
    );
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
    mut pause: Option<ResMut<crate::pause_menu::PauseMenuState>>,
    mut help: Option<ResMut<crate::help_overlay::HelpOverlayVisible>>,
) {
    let in_party = party.as_ref().is_some_and(|p| p.in_party());
    let leads = party.as_ref().is_some_and(|p| p.view.is_leader());
    for Activated { action, .. } in activated.read() {
        match action {
            HomeAction::Play | HomeAction::BotPractice if in_party => {
                // A party plays together: the leader launches it, a member
                // waits for the leader in the lobby.
                if leads {
                    let preference = match *action {
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
            HomeAction::Play | HomeAction::BotPractice => {
                // Backing out of the practice picker restores the saved server first.
                if session.is_offline() {
                    session_ui.write(crate::net::SessionUiCommand::LeaveMatch);
                }

                matchmaking.preference = match *action {
                    HomeAction::BotPractice => shared::match_service::MatchPreference::BotPractice,
                    _ => shared::match_service::MatchPreference::Quick,
                };
                next.set(AppScreen::HeroSelect);
            }
            HomeAction::Card => next.set(AppScreen::Card),
            HomeAction::Collection => next.set(AppScreen::Collection),
            HomeAction::History => crate::career::open_history_modal(&mut career, &mut requests),
            HomeAction::Profile => career.open_profile_modal(),
            HomeAction::Help => {
                if let Some(help) = help.as_deref_mut() {
                    help.0 = true;
                }
            }
            HomeAction::Settings => {
                if let Some(pause) = pause.as_deref_mut() {
                    pause.open = true;
                    pause.in_settings = true;
                }
            }
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
    ui_scale: Option<Res<UiScale>>,
    party: Res<crate::party::PartyClient>,
    roots: Query<Entity, With<HomeRoot>>,
    locale: Option<Res<Locale>>,
    mut last: Local<Option<HomeSignature>>,
) {
    let current = signature(
        &career,
        &session,
        &card,
        &party,
        locale.as_deref(),
        ui_scale.as_ref().map_or(1.0, |scale| scale.0),
    );
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
        commands, career, session, card, thumbnails, preview, platform, ui_scale, party, locale,
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
            app.world()
                .get::<Children>(button)
                .unwrap()
                .iter()
                .find_map(|child| app.world().get::<Text>(child))
                .unwrap()
                .0
                .clone()
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
