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
            .add_systems(OnEnter(AppScreen::Home), (spawn_home_backdrop, spawn_home))
            .add_systems(Update, layout_home_chrome.run_if(in_state(AppScreen::Home)))
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
    Resume,
    BotPractice,
    OfflinePractice,
    Card,
    Collection,
    History,
    Profile,
    Party,
    Help,
    Settings,
    Server,
    AcceptInvite(u64),
    DeclineInvite(u64),
}

#[derive(Component)]
struct HomeRoot;

#[derive(Component)]
enum HomeChrome {
    Frame,
    Scrim,
    BuildInfo,
}

// Chrome belongs to the physical viewport, not the centered 16:9 content canvas.
fn layout_home_chrome(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    platform: Res<crate::ui::UiPlatform>,
    scale: Res<UiScale>,
    mut chrome: Query<(&HomeChrome, &mut Node, Option<&mut TextFont>)>,
) {
    let Ok(window) = windows.single() else { return };
    let scale = scale.0.max(0.1);
    let compact = platform.is_mobile() && window.height() < 600.0;
    // Reserve a separate footer outside the ornament, above the home indicator.
    let footer_gutter = if platform.is_mobile() { 20.0 } else { 0.0 };
    for (kind, mut node, font) in &mut chrome {
        match kind {
            HomeChrome::Frame => {
                let display = if compact {
                    Display::None
                } else {
                    Display::Flex
                };
                if node.display != display {
                    node.display = display;
                }
                let inset = Val::Px(12.0 / scale);
                if [node.left, node.right, node.top]
                    .iter()
                    .any(|edge| *edge != inset)
                {
                    node.left = inset;
                    node.right = inset;
                    node.top = inset;
                }
                node.bottom = Val::Px((footer_gutter + 32.0) / scale);
            }
            HomeChrome::Scrim => {
                let height = Val::Px(if compact { 96.0 / scale } else { 160.0 / scale });
                if node.height != height {
                    node.height = height;
                }
            }
            HomeChrome::BuildInfo => {
                node.left = Val::Px(24.0 / scale);
                node.right = Val::Px(24.0 / scale);
                node.bottom = Val::Px((footer_gutter + 6.0) / scale);
                node.height = Val::Px(18.0 / scale);
                if let Some(mut font) = font {
                    font.font_size = (12.0 / scale).into();
                }
            }
        }
    }
}

fn spawn_home_backdrop(mut commands: Commands, platform: Res<crate::ui::UiPlatform>) {
    commands.spawn((
        Text::new(crate::build_info::label()),
        TextFont {
            font_size: 12.0.into(),
            ..default()
        },
        TextColor(theme::IVORY),
        TextShadow {
            offset: Vec2::splat(1.0),
            color: Color::srgba(0.0, 0.0, 0.0, 0.9),
        },
        TextLayout::default().with_justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            ..default()
        },
        HomeChrome::BuildInfo,
        GlobalZIndex(theme::SCREEN_Z + 1),
        Pickable::IGNORE,
        bevy::ui::FocusPolicy::Pass,
        bevy::state::state_scoped::DespawnOnExit(AppScreen::Home),
        Name::new("HomeBuildInfo"),
    ));
    commands
        .spawn(widgets::screen_root(AppScreen::Home, "HomeBackdrop"))
        .insert(ZIndex(theme::SCREEN_Z - 1))
        .with_children(|root| {
            living_background::spawn(
                root,
                LivingScene::Stage,
                LivingBands::default(),
                theme::Form::of(platform.is_mobile()),
            );
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Px(0.0),
                    height: Val::Px(160.0),
                    ..default()
                },
                BackgroundGradient::from(LinearGradient::to_bottom(vec![
                    ColorStop::percent(
                        theme::perceptual(color::SCRIM_LIVING).with_alpha(0.65),
                        0.0,
                    ),
                    ColorStop::percent(Color::NONE, 100.0),
                ])),
                Pickable::IGNORE,
                HomeChrome::Scrim,
                Name::new("HomeHeaderScrim"),
            ));
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(12.0),
                    right: Val::Px(12.0),
                    top: Val::Px(12.0),
                    bottom: Val::Px(12.0),
                    ..default()
                },
                crate::ui::kit_assets::KitImage::frame(crate::ui::kit_assets::Frame::Ornament),
                Pickable::IGNORE,
                HomeChrome::Frame,
                Name::new("HomeOrnamentFrame"),
            ));
        });
}

/// Input family does not determine layout: an iPad has touch input and a
/// spacious layout. The authored canvas is fitted and centered in the window.
fn home_canvas(mobile: bool, viewport: Vec2, ui_scale: f32) -> (bool, Node, UiTransform) {
    let phone = mobile && viewport.y < 600.0;
    let reference = if phone {
        Vec2::new(844.0, 390.0)
    } else {
        Vec2::new(1280.0, 720.0)
    };
    let scale = ui_scale.max(0.1);
    let fit = (viewport.x / reference.x).min(viewport.y / reference.y);
    (
        phone,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px((viewport.x / scale - reference.x) * 0.5),
            top: Val::Px((viewport.y / scale - reference.y) * 0.5),
            width: Val::Px(reference.x),
            height: Val::Px(reference.y),
            ..default()
        },
        UiTransform::from_scale(Vec2::splat(fit / scale)),
    )
}

// Authored content stays inside the visible ornament, including its corners.
// The header/footer share one physical inset even on a tall tablet canvas.
const HOME_INSET: f32 = 64.0;
fn header_top(fit: f32, outer_y: f32) -> f32 {
    HOME_INSET / fit - outer_y
}

fn home_control_size(authored: f32, mobile: bool, fit: f32) -> f32 {
    if mobile {
        authored.max(theme::metric::TOUCH_MIN / fit.max(0.1))
    } else {
        authored
    }
}

fn spawn_home_utilities(
    parent: &mut ChildSpawnerCommands,
    mobile: bool,
    form: theme::Form,
    fit: f32,
) {
    for (icon, action, desktop_id, phone_id) in [
        (
            Icon::NavHelpCircle,
            HomeAction::Help,
            "HomeHelp",
            "PhoneHelpButton",
        ),
        (
            Icon::NavSettings,
            HomeAction::Settings,
            "HomeSettings",
            "PhoneMenuButton",
        ),
        (
            Icon::NavLink,
            HomeAction::Server,
            "HomeServer",
            "PhoneServerButton",
        ),
    ] {
        let button = kit::controls::sized_icon_button(
            parent,
            icon,
            form,
            ButtonKind::Secondary,
            action,
            if mobile { phone_id } else { desktop_id },
        );
        let side = home_control_size(if mobile { 44.0 } else { 40.0 }, mobile, fit);
        parent
            .commands()
            .entity(button)
            .insert(kit::controls::icon_button_node(side, radius::PILL));
    }
}

/// What the home screen renders from. When it changes, the screen is rebuilt.
#[derive(PartialEq, Clone)]
struct HomeSignature {
    resume_available: bool,
    nickname: String,
    rating: i32,
    matches: u32,
    wins: u32,
    losses: u32,
    connection: ClientConnectionState,
    compatibility_issue: Option<shared::compatibility::CompatibilityIssue>,
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
    viewport_bits: (u32, u32),
}

fn signature(
    career: &CareerClient,
    session: &ClientSession,
    card: &ProfileCard,
    party: &crate::party::PartyClient,
    locale: Option<&Locale>,
    ui_scale: f32,
    viewport: Vec2,
) -> HomeSignature {
    let profile = career.view.profile.as_ref();
    HomeSignature {
        resume_available: false,
        locale: locale.map_or(0, Locale::generation),
        ui_scale_bits: ui_scale.to_bits(),
        viewport_bits: (viewport.x.to_bits(), viewport.y.to_bits()),
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
        compatibility_issue: session.compatibility_issue,
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
    if let Some(issue) = session.compatibility_issue {
        let key = if issue == shared::compatibility::CompatibilityIssue::Unavailable {
            "net.compatibility.unverified_short"
        } else {
            "net.compatibility.incompatible_short"
        };
        return (tr(key).to_owned(), color::TEXT_DANGER);
    }
    match session.state() {
        ClientConnectionState::Connected => (tr("home.connection.online").to_owned(), theme::JADE),
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
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    resume: Option<Res<crate::net::recovery::ResumeMatchState>>,
) {
    let can_resume = resume.as_ref().is_some_and(|resume| resume.saved.is_some());
    if automation_bypass() {
        return;
    }
    // The home screen shows the card's hero in 3D.
    if let Some(slug) = card.showcase_avatar.as_deref() {
        preview.show(slug);
    }
    let preview_image = preview.image.clone();
    let ui_scale_value = ui_scale.as_ref().map_or(1.0, |scale| scale.0);
    let viewport = windows.single().map_or(Vec2::new(1280.0, 720.0), |w| {
        Vec2::new(w.width(), w.height())
    });
    let (phone, canvas, transform) = home_canvas(platform.is_mobile(), viewport, ui_scale_value);
    let form = theme::Form::of(phone);
    let unit = 1.0;
    let reference = if phone {
        Vec2::new(844.0, 390.0)
    } else {
        Vec2::new(1280.0, 720.0)
    };
    let fit = (viewport.x / reference.x).min(viewport.y / reference.y);
    let outer_y = (viewport.y / fit - reference.y) * 0.5;
    // The plate is cover-cropped, while controls are contain-fitted. Locate
    // its painted platform (640, 500 in the 1280x720 painting) in the canvas.
    let cover = (viewport.x / 1280.0).max(viewport.y / 720.0);
    let stage_y = (viewport.y * 0.5
        + 140.0 * cover * crate::ui::tokens::motion::LIVING_SCALE_PLATE
        - (viewport.y - reference.y * fit) * 0.5)
        / fit;
    // Enlarge the rendered panel without changing its ground anchor. The
    // transparent camera padding permits a small panel overscan above the canvas.
    let hero_height = (if phone { 250.0_f32 } else { 440.0_f32 } * 1.35)
        .min((stage_y + 16.0) / super::preview::PREVIEW_GROUND_ANCHOR);
    let hero_top = stage_y - hero_height * super::preview::PREVIEW_GROUND_ANCHOR;
    let hero_width = hero_height * (460.0 / 620.0);
    let party_line = signature(
        &career,
        &session,
        &card,
        &party,
        locale.as_deref(),
        ui_scale_value,
        viewport,
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
        .insert((canvas, transform, BackgroundColor(Color::NONE)))
        .with_children(|root| {
            if !phone {
                root.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(HOME_INSET),
                        right: Val::Px(HOME_INSET),
                        top: Val::Px(header_top(fit, outer_y)),
                        height: Val::Px(64.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        column_gap: Val::Px(space::S24),
                        ..default()
                    },
                    Name::new("HomeHeader"),
                ))
                .with_children(|header| {
                    header
                        .spawn((
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(space::S4),
                                ..default()
                            },
                            Name::new("HomeBrand"),
                        ))
                        .with_children(|brand| {
                            brand.spawn((Text::new("OMOBA"), theme::role_text(TextRole::Title))); // i18n-allow
                            brand.spawn((
                                Text::new(tr("home.tagline")),
                                theme::role_text(TextRole::Eyebrow),
                            ));
                        });
                    header
                        .spawn((
                            Node {
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(space::S8),
                                ..default()
                            },
                            Name::new("HomeUtilityButtons"),
                        ))
                        .with_children(|buttons| {
                            buttons
                                .spawn((
                                    Node {
                                        max_width: Val::Px(240.0),
                                        margin: UiRect::right(Val::Px(space::S16)),
                                        padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                                        border_radius: BorderRadius::all(Val::Px(20.0)),
                                        ..default()
                                    },
                                    BackgroundColor(
                                        theme::perceptual(color::SURFACE_1).with_alpha(0.94),
                                    ),
                                    Name::new("HomeConnectionPill"),
                                ))
                                .with_children(|pill| {
                                    pill.spawn((
                                        widgets::label(&status, 13.0, status_color),
                                        Name::new("HomeConnectionStatus"),
                                    ));
                                });
                            spawn_home_utilities(
                                buttons,
                                platform.is_mobile(),
                                theme::Form::of(platform.is_mobile()),
                                fit,
                            );
                        });
                });
            } else {
                root.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(63.0),
                        top: Val::Px(12.0),
                        column_gap: Val::Px(space::S8),
                        ..default()
                    },
                    Name::new("HomeUtilityButtons"),
                ))
                .with_children(|buttons| {
                    spawn_home_utilities(buttons, true, theme::Form::Phone, fit);
                });
                root.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(63.0),
                        top: Val::Px(66.0),
                        max_width: Val::Px(240.0),
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                        border_radius: BorderRadius::all(Val::Px(16.0)),
                        ..default()
                    },
                    BackgroundColor(theme::perceptual(color::SURFACE_1).with_alpha(0.94)),
                    Name::new("HomeConnectionPill"),
                ))
                .with_children(|pill| {
                    pill.spawn((
                        widgets::label(&status, 12.0, status_color),
                        Name::new("HomeConnectionStatus"),
                    ));
                });
            }

            spawn_home_identity(
                root,
                phone,
                &card,
                profile.as_ref(),
                &career.nickname,
                &thumbnails,
                if phone && party_line.invite.is_some() {
                    None
                } else {
                    last_match.as_deref()
                },
                unit,
                platform.is_mobile(),
                fit,
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
                    left: Val::Px(if phone { 420.0 } else { 640.0 } - hero_width * 0.5),
                    top: Val::Px(hero_top),
                    width: Val::Px(hero_width),
                    height: Val::Px(hero_height),
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
                        width: Val::Percent(100.0),
                        aspect_ratio: Some(460.0 / 620.0),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    Name::new("HomeShowcaseImage"),
                    super::preview::InteractivePreview,
                ));
            });
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(if phone { 330.0 * unit } else { 520.0 }),
                    top: Val::Px(if phone { 296.0 * unit } else { stage_y + 28.0 }),
                    width: Val::Px(if phone { 180.0 * unit } else { 240.0 }),
                    height: Val::Px(if phone { 20.0 * unit } else { 64.0 }),
                    padding: UiRect::axes(
                        Val::Px(space::S12),
                        Val::Px(if phone { 0.0 } else { space::S4 }),
                    ),
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
                    left: Val::Px(if phone { 561.0 * unit } else { 856.0 }),
                    top: Val::Px(if phone { 108.0 * unit } else { 204.0 }),
                    width: Val::Px(if phone { 220.0 * unit } else { 360.0 }),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(if phone { 6.0 * unit } else { 8.0 }),
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
                    if can_resume {
                        tr("home.play.resume").to_owned()
                    } else {
                        play_label
                    },
                    TextStyle::new(TextRole::ButtonLg),
                    ButtonKind::Primary,
                    Some(Icon::HudAttack),
                    if can_resume {
                        HomeAction::Resume
                    } else {
                        HomeAction::Play
                    },
                    "HomePlay".into(),
                    (),
                );
                if !can_resume && !matches!(session.state(), ClientConnectionState::Connected) {
                    column.commands().entity(play).insert(Pressable {
                        disabled: true,
                        ..default()
                    });
                }
                if session.compatibility_issue.is_some() {
                    column.spawn((
                        widgets::label(
                            crate::net::link_status(&session)
                                .detail()
                                .unwrap_or_default(),
                            if phone { 10.0 * unit } else { 13.0 },
                            color::TEXT_DANGER,
                        ),
                        Node {
                            width: Val::Percent(100.0),
                            padding: UiRect::all(Val::Px(4.0 * unit)),
                            ..default()
                        },
                        BackgroundColor(theme::PANEL),
                        Name::new("HomeCompatibilityDetail"),
                    ));
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
                // Local practice remains available even when a server advertises
                // matchmaking. A server bot allocation and socket-free practice
                // are distinct choices, with different progress guarantees.
                for (label, action, id) in [
                    (
                        "home.button.bot_practice",
                        HomeAction::BotPractice,
                        "HomeBotPractice",
                    ),
                    (
                        "home.button.offline_practice",
                        HomeAction::OfflinePractice,
                        "HomeOfflinePractice",
                    ),
                ] {
                    if action == HomeAction::BotPractice
                        && (can_resume || career.view.match_service.is_none())
                    {
                        continue;
                    }
                    let button = kit::spawn_button(
                        column,
                        Node {
                            width: Val::Px(if phone { 220.0 * unit } else { 280.0 }),
                            height: Val::Px(home_control_size(
                                if phone { 44.0 * unit } else { 46.0 },
                                platform.is_mobile(),
                                fit,
                            )),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(space::S8 * unit),
                            border_radius: BorderRadius::all(Val::Px(radius::MD * unit)),
                            ..default()
                        },
                        tr(label),
                        TextStyle::new(TextRole::Button),
                        ButtonKind::Secondary,
                        Some(Icon::NavBot),
                        action,
                        id.into(),
                        (),
                    );
                    if action == HomeAction::BotPractice && !session.is_connected() {
                        column.commands().entity(button).insert(Pressable {
                            disabled: true,
                            ..default()
                        });
                    }
                }
                column.spawn((
                    widgets::label(
                        tr("home.offline_hint"),
                        if phone { 11.0 * unit } else { 12.0 },
                        theme::IVORY,
                    ),
                    TextLayout::justify(Justify::Center),
                    Node {
                        max_width: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
                    Name::new("HomeOfflineNotice"),
                ));
            });

            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(if phone { 92.0 * unit } else { 856.0 }),
                    top: Val::Px(if phone { 317.0 * unit } else { 500.0 }),
                    width: Val::Px(if phone { 660.0 * unit } else { 360.0 }),
                    height: Val::Px(if phone { 52.0 * unit } else { 88.0 }),
                    column_gap: Val::Px(if phone { 12.0 } else { space::S8 }),
                    border_radius: BorderRadius::all(Val::Px(radius::LG)),
                    ..default()
                },
                BackgroundColor(Color::NONE),
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
                spawn_invite_banner(root, *party_id, from, phone, platform.is_mobile(), fit);
            }
            if !platform.is_mobile() {
                root.spawn((
                    widgets::label(tr("home.footer"), 12.0, theme::MUTED),
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(HOME_INSET),
                        bottom: Val::Px(48.0 / fit - outer_y),
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
    mobile: bool,
    fit: f32,
) {
    if !phone {
        let account_side = home_control_size(if mobile { 44.0 } else { 40.0 }, mobile, fit);
        root.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(HOME_INSET),
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
                    width: Val::Px(278.0 + space::S8 + account_side),
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
                    buttons
                        .commands()
                        .entity(customize)
                        .remove::<kit::MenuControl>()
                        .insert(Node {
                            width: Val::Px(278.0),
                            height: Val::Px(home_control_size(46.0, mobile, fit)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..kit::button_node(
                                ButtonSize::Regular,
                                ButtonKind::Secondary,
                                theme::Form::Desktop,
                            )
                        });
                    let account = kit::controls::sized_icon_button(
                        buttons,
                        Icon::SettingsUserCog,
                        theme::Form::of(mobile),
                        ButtonKind::Secondary,
                        HomeAction::Profile,
                        "HomeAccount",
                    );
                    buttons
                        .commands()
                        .entity(account)
                        .insert(kit::controls::icon_button_node(account_side, radius::PILL));
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
            width: Val::Px(if phone { 156.0 * unit } else { 84.0 }),
            height: Val::Px(if phone { 52.0 * unit } else { 88.0 }),
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
    mut server_entry: Option<ResMut<crate::mobile_ui::ServerEntry>>,
) {
    let in_party = party.as_ref().is_some_and(|p| p.in_party());
    let leads = party.as_ref().is_some_and(|p| p.view.is_leader());
    for Activated { action, .. } in activated.read() {
        match action {
            HomeAction::Resume => {
                session_ui.write(crate::net::SessionUiCommand::ResumeMatch);
                next.set(AppScreen::Searching);
            }
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
            HomeAction::Server => {
                if let Some(entry) = server_entry.as_deref_mut() {
                    entry.open_for(&session);
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
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    resume: Option<Res<crate::net::recovery::ResumeMatchState>>,
) {
    let mut current = signature(
        &career,
        &session,
        &card,
        &party,
        locale.as_deref(),
        ui_scale.as_ref().map_or(1.0, |scale| scale.0),
        windows.single().map_or(Vec2::new(1280.0, 720.0), |w| {
            Vec2::new(w.width(), w.height())
        }),
    );
    current.resume_available = resume.as_ref().is_some_and(|resume| resume.saved.is_some());
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
        windows, resume,
    );
}

/// A bounded invitation in the left column. On phones it replaces the recent
/// match chip, leaving both the top utility row and the hero stage clear.
fn spawn_invite_banner(
    parent: &mut ChildSpawnerCommands,
    party_id: u64,
    from: &str,
    phone: bool,
    mobile: bool,
    fit: f32,
) {
    let button_height = home_control_size(44.0, mobile, fit);
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(if phone { 68.0 } else { 178.0 - button_height }),
                left: Val::Px(if phone { 63.0 } else { HOME_INSET }),
                width: Val::Px(if phone { 230.0 } else { 380.0 }),
                height: Val::Px(66.0 + button_height),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(space::S8),
                padding: UiRect::all(Val::Px(space::S8)),
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::all(Val::Px(radius::MD)),
                ..default()
            },
            BackgroundColor(theme::TILE_SELECTED),
            BorderColor::all(theme::GOLD),
            Name::new("HomePartyInvite"),
        ))
        .with_children(|banner| {
            banner.spawn((
                Text::new(trf("home.invite", &[("name", &from)])),
                theme::role_text(TextRole::Caption),
                TextLayout::new(Justify::Left, LineBreak::WordOrCharacter),
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(40.0),
                    flex_shrink: 0.0,
                    overflow: Overflow::clip(),
                    ..default()
                },
                Name::new("HomePartyInviteLabel"),
            ));
            banner
                .spawn(Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(button_height),
                    flex_shrink: 0.0,
                    column_gap: Val::Px(space::S8),
                    ..default()
                })
                .with_children(|actions| {
                    for (label, action, id) in [
                        (
                            "home.button.accept",
                            HomeAction::AcceptInvite(party_id),
                            "HomeAcceptInvite",
                        ),
                        (
                            "home.button.decline",
                            HomeAction::DeclineInvite(party_id),
                            "HomeDeclineInvite",
                        ),
                    ] {
                        kit::spawn_button(
                            actions,
                            Node {
                                flex_grow: 1.0,
                                flex_basis: Val::Px(0.0),
                                min_width: Val::Px(0.0),
                                height: Val::Px(button_height),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                padding: UiRect::horizontal(Val::Px(space::S4)),
                                ..default()
                            },
                            tr(label),
                            TextStyle::new(TextRole::Button).sized(Metric::new(13.0, 13.0)),
                            ButtonKind::Secondary,
                            None,
                            action,
                            id.into(),
                            (),
                        );
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tablet_home_controls_retain_touch_size_and_invites_stay_between_header_and_profile() {
        use crate::ui::test_id::harness;
        for viewport in [
            Vec2::new(1024.0, 768.0),
            Vec2::new(1180.0, 820.0),
            Vec2::new(1366.0, 1024.0),
        ] {
            let fit = (viewport.x / 1280.0).min(viewport.y / 720.0);
            let mut app = harness::kit_app();
            harness::spawn_ui(app.world_mut(), |root| {
                spawn_home_utilities(root, true, theme::Form::Phone, fit);
                spawn_home_identity(
                    root,
                    false,
                    &ProfileCard::default(),
                    None,
                    "Player",
                    &AvatarThumbnails::default(),
                    None,
                    1.0,
                    true,
                    fit,
                );
                spawn_invite_banner(root, 1, "AReallyLongInviterName1234", false, true, fit);
            });
            for id in [
                "PhoneHelpButton",
                "PhoneMenuButton",
                "PhoneServerButton",
                "HomeAccount",
                "HomeCustomizeCard",
                "HomeAcceptInvite",
                "HomeDeclineInvite",
            ] {
                let entity = harness::find(app.world_mut(), id).unwrap();
                let node = app.world().get::<Node>(entity).unwrap();
                let Val::Px(height) = node.height else {
                    panic!("{id} has no explicit hit height")
                };
                assert!(
                    height * fit >= 43.99,
                    "{id} shrinks below44px on {viewport:?}"
                );
                if let Val::Px(width) = node.width {
                    assert!(width * fit >= 43.99, "{id} shrinks below44px wide");
                }
            }
            let (_, invite) = app
                .world_mut()
                .query::<(&Name, &Node)>()
                .iter(app.world())
                .find(|(name, _)| name.as_str() == "HomePartyInvite")
                .unwrap();
            let (Val::Px(top), Val::Px(height)) = (invite.top, invite.height) else {
                panic!()
            };
            let outer_y = (viewport.y / fit - 720.0) * 0.5;
            assert!(top >= header_top(fit, outer_y) + 64.0);
            assert!(top + height < 248.0, "invite must not cover the profile");
        }
    }

    #[test]
    fn phone_long_name_invite_stays_in_its_left_slot_and_wraps() {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        let inviter = "AReallyLongInviterName1234";
        harness::spawn_ui(app.world_mut(), |root| {
            spawn_invite_banner(root, 1, inviter, true, true, 1.0);
        });
        let (_, invite) = app
            .world_mut()
            .query::<(&Name, &Node)>()
            .iter(app.world())
            .find(|(name, _)| name.as_str() == "HomePartyInvite")
            .unwrap();
        let (Val::Px(left), Val::Px(top), Val::Px(width)) = (invite.left, invite.top, invite.width)
        else {
            panic!()
        };
        assert!(
            left >= 63.0 && left + width < 300.0,
            "invite must stay left of the hero"
        );
        assert!(top >= 64.0, "invite must stay below identity and utilities");
        let (_, label, layout, node) = app
            .world_mut()
            .query::<(&Name, &Text, &TextLayout, &Node)>()
            .iter(app.world())
            .find(|(name, _, _, _)| name.as_str() == "HomePartyInviteLabel")
            .unwrap();
        assert!(label.0.contains(inviter));
        assert_eq!(layout.linebreak, LineBreak::WordOrCharacter);
        assert_eq!(node.width, Val::Percent(100.0));
        assert_eq!(node.overflow, Overflow::clip());
    }

    #[test]
    fn tablet_uses_the_spacious_canvas_centered_in_its_viewport() {
        for viewport in [
            Vec2::new(1024.0, 768.0),
            Vec2::new(1180.0, 820.0),
            Vec2::new(1366.0, 1024.0),
        ] {
            let (phone, node, transform) = home_canvas(true, viewport, 1.0);
            assert!(!phone, "tablet must not use the fixed phone composition");
            let (Val::Px(left), Val::Px(top)) = (node.left, node.top) else {
                panic!()
            };
            let center = Vec2::new(left, top) + Vec2::new(640.0, 360.0);
            assert!(center.abs_diff_eq(viewport * 0.5, 0.01));
            let extent = Vec2::new(1280.0, 720.0) * transform.scale;
            assert!(extent.x <= viewport.x + 0.01 && extent.y <= viewport.y + 0.01);
            assert!((extent.x - viewport.x).abs() < 0.01);
        }
        assert!(home_canvas(true, Vec2::new(844.0, 390.0), 1.0).0);
    }

    #[test]
    fn viewport_chrome_keeps_equal_insets_across_tablet_sizes_and_ui_scales() {
        for (size, scale) in [
            (Vec2::new(1180.0, 820.0), 1.0),
            (Vec2::new(1024.0, 768.0), 0.8),
            (Vec2::new(1366.0, 1024.0), 1.5),
            (Vec2::new(844.0, 390.0), 1.0),
        ] {
            let mut app = App::new();
            app.insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Mobile))
                .insert_resource(UiScale(scale))
                .add_systems(Update, layout_home_chrome);
            let mut window = Window::default();
            window.resolution.set(size.x, size.y);
            app.world_mut().spawn((window, bevy::window::PrimaryWindow));
            let frame = app
                .world_mut()
                .spawn((Node::default(), HomeChrome::Frame))
                .id();
            let scrim = app
                .world_mut()
                .spawn((Node::default(), HomeChrome::Scrim))
                .id();
            let footer = app
                .world_mut()
                .spawn((Node::default(), TextFont::default(), HomeChrome::BuildInfo))
                .id();
            app.update();
            let node = app.world().get::<Node>(frame).unwrap();
            for edge in [node.left, node.right, node.top] {
                let Val::Px(edge) = edge else {
                    panic!("viewport inset must use pixels")
                };
                assert!((edge * scale - 12.0).abs() < 0.01);
            }
            assert_eq!(node.bottom, Val::Px(52.0 / scale));
            let footer = app.world().get::<Node>(footer).unwrap();
            assert_eq!(footer.left, footer.right);
            assert_eq!(footer.bottom, Val::Px(26.0 / scale));
            assert_eq!(footer.height, Val::Px(18.0 / scale));
            assert_eq!(
                node.display,
                if size.y < 600.0 {
                    Display::None
                } else {
                    Display::Flex
                }
            );
            assert_eq!(
                app.world().get::<Node>(scrim).unwrap().height,
                Val::Px(if size.y < 600.0 { 96.0 } else { 160.0 } / scale)
            );
        }
    }

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
    fn connected_home_offers_separate_server_and_socket_free_practice() {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<AppScreen>()
            .init_resource::<Assets<Image>>()
            .init_resource::<super::super::preview::AvatarPreview>()
            .init_resource::<CareerClient>()
            .init_resource::<ClientSession>()
            .init_resource::<ProfileCard>()
            .init_resource::<AvatarThumbnails>()
            .init_resource::<crate::party::PartyClient>()
            .init_resource::<crate::match_service::MatchServiceClient>()
            .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
            .add_message::<NetworkCommand>()
            .add_message::<crate::net::SessionUiCommand>()
            .add_ui_action::<HomeAction>()
            .add_systems(Startup, spawn_home)
            .add_systems(Update, home_actions.after(UiSet::Dispatch));
        app.world_mut()
            .resource_mut::<CareerClient>()
            .view
            .match_service = Some(shared::match_service::MatchServiceView::Idle);
        app.world_mut()
            .resource_mut::<ClientSession>()
            .set_state_for_test(ClientConnectionState::Connected);
        app.update();
        for id in ["HomeBotPractice", "HomeOfflinePractice"] {
            let button = harness::find(app.world_mut(), id).unwrap();
            assert!(
                !app.world()
                    .get::<Pressable>(button)
                    .is_some_and(|p| p.disabled)
            );
        }
        assert!(
            app.world_mut()
                .query::<&Name>()
                .iter(app.world())
                .any(|name| name.as_str() == "HomeOfflineNotice")
        );
        harness::press(app.world_mut(), "HomeOfflinePractice");
        app.update();
        assert!(
            app.world_mut()
                .resource_mut::<Messages<crate::net::SessionUiCommand>>()
                .drain()
                .any(|command| matches!(command, crate::net::SessionUiCommand::StartOffline))
        );
        assert!(
            app.world()
                .resource::<Messages<NetworkCommand>>()
                .is_empty()
        );
    }

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
            .add_systems(Startup, (spawn_home_backdrop, spawn_home))
            .add_systems(Update, refresh_home);
        app.update();
        let backdrop = app
            .world_mut()
            .query::<(Entity, &Name)>()
            .iter(app.world())
            .find(|(_, name)| name.as_str() == "HomeBackdrop")
            .unwrap()
            .0;
        app.world_mut()
            .resource_mut::<ClientSession>()
            .set_state_for_test(ClientConnectionState::Connecting);
        app.update();
        assert!(
            app.world().get_entity(backdrop).is_ok(),
            "status refresh must preserve the painting and its fade"
        );
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
            connection_line(app.world().resource::<ClientSession>()).0
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
