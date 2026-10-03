//! Compact match chrome, selected-target health and the live two-team scoreboard.
// i18n-strict
use crate::{
    combat::{CombatStats, TargetState},
    hud_layout::HudRegion,
    i18n::{Locale, Localized, UiLabel, data, tr, trf},
    input_context::InputContextSet,
    mobile_controls::MobileControls,
    net::{
        ClientSession, GameState, GameStateSnapshot, NetworkAvatar, NetworkHeroClass,
        NetworkMapStructure, NetworkMinionId, NetworkMinionKind, NetworkNeutralCampType,
        NetworkNeutralId, NetworkPlayerId, NetworkStructureId, NetworkStructureProtected,
        StructureKind, TargetKind,
    },
    player::Player,
    ui::{
        Activated, ModalId, ModalRoot, ScrollArea, TestId, UiAction, UiActionAppExt,
        kit_assets::{Icon, KitImage},
        test_id::node_key,
        theme::{self as ui, ButtonKind, Form, TextStyle},
        tokens::{TextRole, border, color, radius, size, space},
        widgets::{
            KitParts, controls,
            game::{self, BarKind, BarValue, PortraitSpec, PortraitView},
            icon_node,
            surfaces::Tooltip,
        },
    },
};
use bevy::{prelude::*, window::PrimaryWindow};
use shared::{
    live_score::{LiveScorePlayer, LiveScoreboard},
    map::Team,
};

pub(crate) struct EdgeHudPlugin;
#[derive(Resource, Default)]
pub(crate) struct ScoreboardState {
    pub open: bool,
    namespace: Option<(u64, u64)>,
}
/// Presses on the match chrome and the scoreboard. The buttons keep their
/// fixed panel colours (no `ButtonStyle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EdgeAction {
    /// The score strip toggles the scoreboard.
    Score,
    /// The ≡ button opens the pause menu.
    Menu,
    /// The scoreboard's Close button and its backdrop.
    Close,
}
#[derive(Component)]
struct EdgePart;
/// The team kill totals of the score strip.
#[derive(Component, Clone, Copy)]
enum ScoreLabel {
    Green,
    Blue,
}
#[derive(Component)]
struct KdaLabel;
#[derive(Component)]
struct TargetLabel;
#[derive(Component)]
struct TargetValue;
#[derive(Component)]
struct TargetFill;
/// The target plate's parts (`target-*.md` § Plate anatomy).
#[derive(Component)]
struct TargetPortrait;
#[derive(Component)]
struct TargetNameRow;
#[derive(Component)]
struct TargetKindIcon;
#[derive(Component)]
struct TargetBadge;
#[derive(Component)]
struct TargetBadgeText;
#[derive(Component)]
struct TargetHpBar;
/// The kit bar inside [`TargetHpBar`] (its `BarValue` drives the trail).
#[derive(Component)]
struct TargetBar;
#[derive(Component)]
struct TargetManaLine;
#[derive(Component)]
struct TargetManaFill;
#[derive(Component)]
struct ScoreRows(Team);
#[derive(Component)]
struct ScoreScroll;
#[derive(Component)]
struct ScoreDetail;

impl Plugin for EdgeHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScoreboardState>()
            .add_ui_action::<EdgeAction>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                actions
                    .in_set(InputContextSet::Modal)
                    .after(crate::ui::UiSet::Dispatch)
                    .before(crate::help_overlay::HelpOverlaySet::Input)
                    .before(crate::shop::ShopModalSet),
            )
            .add_systems(
                Update,
                (update, render_rows)
                    .chain()
                    .after(InputContextSet::Resolve),
            )
            .add_systems(PostUpdate, layout.before(bevy::ui::UiSystems::Layout));
    }
}
fn button_node(width: f32) -> Node {
    Node {
        width: Val::Px(width),
        height: Val::Px(44.0),
        flex_shrink: 0.0,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        ..default()
    }
}
fn text(parent: &mut ChildSpawnerCommands, value: impl UiLabel, size: f32, color: Color) {
    parent.spawn((value.into_text(), ui::text(size), TextColor(color)));
}
/// Target plate anatomy (`target-hero.md` § Plate anatomy): portrait at
/// (8, 8) desktop / (4, 4) phone, text column from x 56 / 48, name row
/// ending at plate x 270 / 212, HP bar 214 / 164 wide.
struct TargetAnatomy {
    portrait: PortraitSpec,
    portrait_at: f32,
    column_x: f32,
    column_w: f32,
    name_top_hero: f32,
    name_top: f32,
    name_h: f32,
    hp_top_hero: f32,
    hp_top: f32,
    mana_top: f32,
    badge_h: f32,
}

impl TargetAnatomy {
    fn of(form: Form) -> Self {
        match form {
            Form::Desktop => Self {
                portrait: PortraitSpec {
                    side: size::PORTRAIT_SM,
                    ring: false,
                    disc: TARGET_LEVEL_DISC.desktop,
                    disc_at: Vec2::new(-8.0, -6.0),
                    icon: size::ICON_MD,
                },
                portrait_at: space::S8,
                column_x: 56.0,
                column_w: 214.0,
                name_top_hero: 4.0,
                name_top: 6.0,
                name_h: 20.0,
                hp_top_hero: 28.0,
                hp_top: 32.0,
                mana_top: 44.0,
                badge_h: 16.0,
            },
            Form::Phone => Self {
                portrait: PortraitSpec {
                    side: 36.0,
                    ring: false,
                    disc: TARGET_LEVEL_DISC.phone,
                    disc_at: Vec2::new(-4.0, -4.0),
                    icon: size::ICON_SM,
                },
                portrait_at: space::S4,
                column_x: 48.0,
                column_w: 128.0,
                name_top_hero: 3.0,
                name_top: 3.0,
                name_h: 18.0,
                hp_top_hero: 23.0,
                hp_top: 23.0,
                mana_top: 0.0,
                badge_h: 16.0,
            },
        }
    }
}

/// Target level disc (`target-hero.md`: 20 desktop, 18 phone).
const TARGET_LEVEL_DISC: crate::ui::tokens::Metric = crate::ui::tokens::Metric::new(20.0, 18.0);
/// Protected structure: the bar at 55 % opacity (`target-structure.md`).
const PROTECTED_ALPHA: f32 = 0.55;

fn setup(mut commands: Commands, mobile: Option<Res<MobileControls>>) {
    let form = Form::from_mobile(mobile.as_deref());
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            HudRegion::ScoreStrip,
            EdgePart,
            ZIndex(14),
            Name::new("MatchScoreStrip"),
        ))
        .with_children(|strip| {
            strip
                .spawn(crate::ui::widgets::plate_button(
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        padding: UiRect::horizontal(Val::Px(space::S8 + border::FRAME)),
                        column_gap: Val::Px(space::S8),
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    EdgeAction::Score,
                    "MatchScoreButton",
                    KitParts::default(),
                ))
                .with_children(|button| {
                    button
                        .spawn(Node {
                            column_gap: Val::Px(space::S4),
                            align_items: AlignItems::Center,
                            flex_shrink: 0.0,
                            ..default()
                        })
                        .with_children(|scores| {
                            for (label, ink) in [
                                (Some(ScoreLabel::Green), color::TEAM_GREEN),
                                (None, color::TEXT_MUTED),
                                (Some(ScoreLabel::Blue), color::TEAM_BLUE),
                            ] {
                                let mut text = scores.spawn((
                                    Text::new(if label.is_some() { "—" } else { ":" }),
                                    ui::role_text(if form == Form::Phone {
                                        TextRole::NumberSm
                                    } else {
                                        TextRole::NumberLg
                                    }),
                                    TextColor(ink),
                                    TextLayout::new_with_no_wrap(),
                                ));
                                if let Some(label) = label {
                                    text.insert(label);
                                }
                            }
                        });
                    button.spawn((
                        Node {
                            width: Val::Px(border::HAIRLINE),
                            height: Val::Px(space::S24),
                            ..default()
                        },
                        BackgroundColor(color::BORDER_SUBTLE),
                    ));
                    button
                        .spawn(Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            flex_grow: 1.0,
                            ..default()
                        })
                        .with_children(|column| {
                            column.spawn((
                                Localized::new("edge.kda").into_text(),
                                ui::role_text(TextRole::Eyebrow),
                                TextColor(color::TEXT_MUTED),
                                TextLayout::new_with_no_wrap(),
                            ));
                            column.spawn((
                                Text::new("—/—/—"),
                                ui::role_text(TextRole::NumberSm),
                                TextColor(color::TEXT_PRIMARY),
                                TextLayout::new_with_no_wrap(),
                                KdaLabel,
                                Name::new("MatchKdaText"),
                            ));
                        });
                });
        });
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            HudRegion::MenuButton,
            ZIndex(14),
            Name::new("MatchMenuRoot"),
        ))
        .with_children(|root| {
            let menu = controls::sized_icon_button(
                root,
                Icon::NavMenu,
                Form::Desktop,
                ButtonKind::Secondary,
                EdgeAction::Menu,
                "MatchMenuButton",
            );
            root.commands().entity(menu).insert((
                EdgePart,
                Tooltip {
                    title: None,
                    body: "pause.title",
                },
            ));
        });
    spawn_target_frame(&mut commands, form);
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                display: Display::None,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.015, 0.02, 0.72)),
            ZIndex(90),
            ModalRoot(ModalId::Scoreboard),
            Name::new("ScoreboardRoot"),
        ))
        .with_children(|overlay| {
            overlay.spawn((
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                UiAction(EdgeAction::Close),
                TestId::new("ScoreboardBackdrop"),
            ));
            overlay
                .spawn((
                    Node {
                        width: Val::Px(1088.0),
                        max_width: Val::Percent(94.0),
                        max_height: Val::Percent(92.0),
                        padding: UiRect::all(Val::Px(14.0)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.0),
                        ..ui::panel_node()
                    },
                    KitImage::frame(crate::ui::kit_assets::Frame::Panel),
                    BorderColor::all(ui::EDGE),
                    ZIndex(1),
                    Name::new("ScoreboardPanel"),
                ))
                .with_children(|panel| {
                    panel
                        .spawn((Node {
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        },))
                        .with_children(|row| {
                            text(row, Localized::new("edge.scoreboard.title"), 24.0, ui::GOLD);
                            row.spawn(Node {
                                column_gap: Val::Px(12.0),
                                align_items: AlignItems::Center,
                                ..default()
                            })
                            .with_children(|score| {
                                for (label, ink) in [
                                    (ScoreLabel::Green, color::TEAM_GREEN),
                                    (ScoreLabel::Blue, color::TEAM_BLUE),
                                ] {
                                    if matches!(label, ScoreLabel::Blue) {
                                        text(score, ":", 24.0, ui::GOLD);
                                    }
                                    score.spawn((
                                        Text::new("—"),
                                        ui::role_text(TextRole::NumberLg),
                                        TextColor(ink),
                                        label,
                                    ));
                                }
                            });
                            row.spawn((
                                Button,
                                button_node(64.0),
                                BackgroundColor(ui::TILE),
                                BorderColor::all(ui::EDGE),
                                UiAction(EdgeAction::Close),
                                TestId::new("ScoreboardCloseButton"),
                            ))
                            .with_children(|p| {
                                text(p, Localized::new("edge.scoreboard.close"), 14.0, ui::IVORY)
                            });
                        });
                    panel
                        .spawn((Node {
                            column_gap: Val::Px(12.0),
                            ..default()
                        },))
                        .with_children(|tables| {
                            for team in [Team::Green, Team::Blue] {
                                tables
                                    .spawn((Node {
                                        width: Val::Percent(50.0),
                                        min_width: Val::Px(0.0),
                                        flex_direction: FlexDirection::Column,
                                        row_gap: Val::Px(6.0),
                                        ..default()
                                    },))
                                    .with_children(|col| {
                                        text(
                                            col,
                                            Localized::new(if team == Team::Green {
                                                "edge.team.green"
                                            } else {
                                                "edge.team.blue"
                                            }),
                                            20.0,
                                            if team == Team::Green {
                                                ui::JADE
                                            } else {
                                                Color::srgb(0.50, 0.72, 1.0)
                                            },
                                        );
                                        spawn_score_row(
                                            col,
                                            [
                                                "edge.column.player",
                                                "edge.column.kda",
                                                "edge.column.gold",
                                            ]
                                            .map(Localized::new),
                                        );
                                        col.spawn((
                                            Node {
                                                height: Val::Px(190.0),
                                                flex_direction: FlexDirection::Column,
                                                overflow: Overflow::scroll_y(),
                                                ..default()
                                            },
                                            // Wheel under the cursor, touch
                                            // drag from the first pixel.
                                            ScrollArea::wheel(28.0).hover_only().touch_drag(0.0),
                                            ScoreRows(team),
                                            ScoreScroll,
                                            Name::new(if team == Team::Green {
                                                "ScoreboardGreenRows"
                                            } else {
                                                "ScoreboardBlueRows"
                                            }),
                                        ));
                                    });
                            }
                        });
                    panel.spawn((
                        Text::new(""),
                        ui::text(12.0),
                        TextColor(ui::MUTED),
                        ScoreDetail,
                        Name::new("ScoreboardDetail"),
                    ));
                });
        });
}
/// The target plate (`target-hero|minion|neutral|structure.md`): plate
/// `color.surface.glass.strong`, portrait disc (avatar or kind icon, level
/// disc), name row (name, class / lock icon, BOSS or lane badge), HP bar with
/// value and, for heroes on desktop, the mana line. Not interactive.
fn spawn_target_frame(commands: &mut Commands, form: Form) {
    let anatomy = TargetAnatomy::of(form);
    let absolute = |left: f32, top: f32| Node {
        position_type: PositionType::Absolute,
        left: Val::Px(left),
        top: Val::Px(top),
        ..default()
    };
    commands
        .spawn((
            game::hud_plate(true),
            HudRegion::TargetFrame,
            EdgePart,
            ZIndex(14),
            Pickable::IGNORE,
            Name::new("TargetHealthRoot"),
        ))
        .insert(Node {
            position_type: PositionType::Absolute,
            display: Display::None,
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            ..default()
        })
        .with_children(|plate| {
            plate
                .spawn((
                    absolute(anatomy.portrait_at, anatomy.portrait_at),
                    Pickable::IGNORE,
                ))
                .with_children(|slot| {
                    game::live_portrait(
                        slot,
                        PortraitView {
                            art: None,
                            fallback: Icon::HudMinion,
                            level: None,
                            xp: 0.0,
                            grey: false,
                            strong_rim: false,
                        },
                        anatomy.portrait,
                        TargetPortrait,
                    );
                });
            plate
                .spawn((
                    Node {
                        width: Val::Px(anatomy.column_w),
                        height: Val::Px(anatomy.name_h),
                        column_gap: Val::Px(space::S4),
                        align_items: AlignItems::Center,
                        ..absolute(anatomy.column_x, anatomy.name_top)
                    },
                    TargetNameRow,
                    Pickable::IGNORE,
                ))
                .with_children(|row| {
                    row.spawn((
                        Node {
                            max_width: Val::Px(anatomy.column_w),
                            min_width: Val::Px(0.0),
                            flex_shrink: 1.0,
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ))
                    .with_children(|clip| {
                        clip.spawn((
                            Text::new(""),
                            ui::styled_text(TextStyle::keep_case(TextRole::Label)),
                            TextColor(color::TEXT_PRIMARY),
                            TextLayout::new_with_no_wrap(),
                            TargetLabel,
                            Name::new("TargetHealthName"),
                        ));
                    });
                    row.spawn((
                        icon_node(Icon::NavLock, size::ICON_SM, color::TEXT_MUTED),
                        TargetKindIcon,
                    ));
                    row.spawn((
                        Node {
                            flex_grow: 1.0,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    row.spawn((
                        Node {
                            height: Val::Px(anatomy.badge_h),
                            padding: UiRect::horizontal(Val::Px(space::S4 + border::FRAME)),
                            flex_shrink: 0.0,
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                            display: Display::None,
                            ..default()
                        },
                        BackgroundColor(color::SURFACE_3),
                        TargetBadge,
                        Pickable::IGNORE,
                    ))
                    .with_children(|badge| {
                        badge.spawn((
                            Text::new(""),
                            ui::styled_text(
                                TextStyle::keep_case(TextRole::Label).sized(TARGET_BADGE_TEXT),
                            ),
                            TextColor(color::TEXT_SECONDARY),
                            TargetBadgeText,
                        ));
                    });
                });
            let hp_value = BarValue {
                current: 1.0,
                max: 1.0,
                respawn: None,
            };
            plate
                .spawn((
                    absolute(anatomy.column_x, anatomy.hp_top),
                    TargetHpBar,
                    Pickable::IGNORE,
                ))
                .with_children(|slot| {
                    let (bar, parts) = game::bar_parts(
                        slot,
                        BarKind::HpEnemy,
                        hp_value,
                        Val::Px(anatomy.column_w),
                        form,
                        true,
                    );
                    let mut commands = slot.commands();
                    commands.entity(bar).insert(TargetBar);
                    if let Some(fill) = parts.fill {
                        commands
                            .entity(fill)
                            .insert((TargetFill, Name::new("TargetHealthFill")));
                    }
                    if let Some(label) = parts.label {
                        commands
                            .entity(label)
                            .insert((TargetValue, Name::new("TargetHealthValue")));
                    }
                });
            if form == Form::Desktop {
                plate
                    .spawn((
                        Node {
                            width: Val::Px(anatomy.column_w),
                            height: Val::Px(size::BAR_XP),
                            border_radius: BorderRadius::all(Val::Px(radius::SM)),
                            overflow: Overflow::clip(),
                            ..absolute(anatomy.column_x, anatomy.mana_top)
                        },
                        BackgroundColor(ui::perceptual(color::BAR_TRACK)),
                        TargetManaLine,
                        Pickable::IGNORE,
                    ))
                    .with_child((
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(color::BAR_MANA),
                        TargetManaFill,
                    ));
            }
        });
}

/// Target badge text: `type.caption` semibold at 11 (`target-neutral.md`).
const TARGET_BADGE_TEXT: crate::ui::tokens::Metric = crate::ui::tokens::Metric::new(11.0, 11.0);

fn actions(
    mut back: crate::ui::BackInput,
    session: Res<ClientSession>,
    game: Res<GameStateSnapshot>,
    mut state: ResMut<ScoreboardState>,
    mut pause: ResMut<crate::pause_menu::PauseMenuState>,
    help: Res<crate::help_overlay::HelpOverlayVisible>,
    shop: Option<Res<crate::shop::ShopState>>,
    social: Option<Res<crate::social::SocialClient>>,
    career: Option<Res<crate::career::CareerClient>>,
    supporter: Option<Res<crate::supporter::SupporterUiState>>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    entry: Option<Res<crate::mobile_ui::ServerEntry>>,
    mobile: Option<Res<MobileControls>>,
    mut activated: MessageReader<Activated<EdgeAction>>,
    pad_menu: (
        Option<Res<crate::gamepad::GamepadControls>>,
        Option<Res<crate::ui::ModalStack>>,
    ),
) {
    let (gamepad, modals) = pad_menu;
    let identity = (game.meta.server_epoch, game.meta.match_id);
    // A controller's Start/Options is the `≡` button: it opens the pause
    // menu where `≡` could, and closes the menu when it is the top modal.
    let start = gamepad
        .as_ref()
        .is_some_and(|pad| pad.active && pad.menu_pressed);
    let allowed = session.join_confirmed() && matches!(game.state, GameState::Running);
    if state.namespace != Some(identity) || !allowed {
        state.open = false;
        state.namespace = Some(identity);
    }
    let other_modal = pause.open
        || help.0
        || shop.as_ref().is_some_and(|s| s.open)
        || social.as_ref().is_some_and(|s| s.blocks_gameplay())
        || career.as_ref().is_some_and(|s| s.modal_open())
        || supporter.as_ref().is_some_and(|s| s.open)
        || screen.as_ref().is_some_and(|s| s.get().is_menu())
        || entry.as_ref().is_some_and(|s| s.open)
        || mobile
            .as_ref()
            .is_some_and(|s| s.enabled && (!s.landscape || !s.focused));
    if other_modal {
        state.open = false;
    }
    let can_open = allowed && !other_modal;
    if state.open && back.just_pressed() {
        state.open = false;
        back.consume();
    }
    if can_open && back.keys().just_pressed(KeyCode::Tab) {
        state.open = !state.open;
        back.keys_mut().clear_just_pressed(KeyCode::Tab);
    }
    if start
        && pause.open
        && modals
            .as_ref()
            .is_some_and(|modals| modals.top() == Some(ModalId::Pause))
    {
        pause.open = false;
        pause.in_settings = false;
    }
    let pad_menu = (start && can_open).then_some(EdgeAction::Menu);
    for action in activated.read().map(|a| a.action).chain(pad_menu) {
        match action {
            EdgeAction::Close => state.open = false,
            EdgeAction::Score if can_open => state.open = !state.open,
            EdgeAction::Menu if can_open => {
                state.open = false;
                pause.open = true;
                pause.in_settings = false;
            }
            _ => {}
        }
    }
}
/// Team kill totals (sum of `LiveScorePlayer.kills`), `None` before data.
fn team_kills(board: Option<&LiveScoreboard>) -> Option<(u32, u32)> {
    let board = board?;
    let sum = |team| {
        board
            .players
            .iter()
            .filter(|p| p.team == team)
            .fold(0u32, |n, p| n.saturating_add(p.kills))
    };
    Some((sum(Team::Green), sum(Team::Blue)))
}
fn scores(board: Option<&LiveScoreboard>, local: Option<u64>) -> (String, String) {
    let Some((green, blue)) = team_kills(board) else {
        return ("— : —".into(), "—/—/—".into());
    };
    let kda = board
        .into_iter()
        .flat_map(|board| board.players.iter())
        .find(|p| Some(p.player_id) == local)
        .map_or_else(
            || "—/—/—".into(),
            |p| format!("{}/{}/{}", p.kills, p.deaths, p.assists),
        );
    (format!("{green} : {blue}"), kda) // i18n-allow: numbers
}

/// What the target plate shows for the selected entity.
#[derive(Clone, PartialEq, Debug)]
struct TargetDetails {
    kind: TargetKind,
    name: String,
    /// Disconnected enemy: the name is muted.
    muted: bool,
    hp: f32,
    max: f32,
    /// Heroes: `mana / max_mana` for the mana line.
    mana: Option<f32>,
    portrait: PortraitView,
    /// Class icon (hero) or lock (protected structure) after the name.
    icon: Option<(Icon, Color)>,
    /// BOSS (gold) or lane (muted) badge.
    badge: Option<(String, bool)>,
    protected: bool,
}

type TargetItem<'a> = (
    &'a CombatStats,
    Option<&'a NetworkPlayerId>,
    Option<&'a NetworkMinionId>,
    Option<&'a NetworkStructureId>,
    Option<&'a NetworkNeutralId>,
    Option<&'a NetworkHeroClass>,
    Option<&'a NetworkAvatar>,
    Option<&'a NetworkMinionKind>,
    Option<&'a NetworkNeutralCampType>,
    Option<&'a StructureKind>,
    Option<&'a NetworkMapStructure>,
    Option<&'a NetworkStructureProtected>,
);

/// The plate content for a target, or `None` when nothing may show (wrong
/// identity, dead, invalid HP).
fn target_details(
    id: crate::net::TargetId,
    item: TargetItem<'_>,
    board: Option<&LiveScoreboard>,
) -> Option<TargetDetails> {
    let (
        stats,
        player,
        minion,
        structure,
        neutral,
        class,
        avatar,
        minion_kind,
        camp,
        structure_kind,
        map_structure,
        protected,
    ) = item;
    let matches = match id.kind {
        TargetKind::Player => player.map(|p| p.0) == Some(id.id),
        TargetKind::Minion => minion.map(|p| p.0) == Some(id.id),
        TargetKind::Structure => structure.map(|p| p.0) == Some(id.id),
        TargetKind::Neutral => neutral.map(|p| p.0) == Some(id.id),
    };
    if !matches || !stats.is_alive() || !stats.max_hp.is_finite() || stats.max_hp <= 0.0 {
        return None;
    }
    let disc = |icon: Icon, strong: bool| PortraitView {
        art: None,
        fallback: icon,
        level: None,
        xp: 0.0,
        grey: false,
        strong_rim: strong,
    };
    let protected = id.kind == TargetKind::Structure && protected.is_some_and(|p| p.0);
    let mut muted = false;
    let (name, portrait, icon, badge) = match id.kind {
        TargetKind::Player => {
            let row = board.and_then(|s| s.players.iter().find(|p| p.player_id == id.id));
            let class = class.map(|class| class.0);
            // Fallback: the class name, `edge.target.hero` only without one.
            let fallback = class.map_or(tr("edge.target.hero"), data::hero_name);
            let name = match row {
                Some(row) if !row.connected => {
                    muted = true;
                    trf("edge.scoreboard.offline", &[("name", &row.nickname)])
                }
                Some(row) => row.nickname.clone(),
                None => fallback.to_owned(),
            };
            let art = avatar
                .and_then(|avatar| avatar.0.as_deref())
                .and_then(omoba_passport::avatars::avatar_definition)
                .and_then(crate::passport::thumbnail_asset_path);
            let icon = class.map(game::class_icon).unwrap_or(Icon::NavUser);
            (
                name,
                PortraitView {
                    art,
                    fallback: icon,
                    level: row.map(|row| row.level),
                    xp: 0.0,
                    grey: muted,
                    strong_rim: false,
                },
                class.map(|class| (game::class_icon(class), color::TEXT_MUTED)),
                None,
            )
        }
        TargetKind::Minion => (
            tr(match minion_kind.map(|kind| kind.0) {
                Some(shared::combat::MinionKind::Melee) => "edge.target.minion_melee",
                Some(shared::combat::MinionKind::Caster) => "edge.target.minion_caster",
                None => "edge.target.minion",
            })
            .to_owned(),
            disc(Icon::HudMinion, false),
            None,
            None,
        ),
        TargetKind::Neutral => {
            let boss = camp.map(|camp| camp.0).filter(|camp| camp.is_boss());
            match boss {
                Some(camp) => (
                    tr(data::boss_key(camp)).to_owned(),
                    disc(Icon::NavCrown, true),
                    None,
                    Some((tr("edge.target.boss").to_owned(), true)),
                ),
                None => (
                    tr("edge.target.neutral").to_owned(),
                    disc(Icon::HudSkull, false),
                    None,
                    None,
                ),
            }
        }
        TargetKind::Structure => {
            let base = structure_kind == Some(&StructureKind::BaseTower);
            let name = tr(match structure_kind {
                Some(StructureKind::Tower) => "edge.target.tower",
                Some(StructureKind::BaseTower) => "edge.target.base",
                None => "edge.target.structure",
            });
            let lane = (!base)
                .then(|| map_structure.and_then(|structure| structure.lane))
                .flatten()
                .map(|lane| (tr(data::lane_key(lane)).to_owned(), false));
            (
                name.to_owned(),
                disc(Icon::HudTower, base),
                protected.then_some((Icon::NavLock, color::TEXT_MUTED)),
                lane,
            )
        }
    };
    Some(TargetDetails {
        kind: id.kind,
        name,
        muted,
        hp: stats.hp.max(0.0),
        max: stats.max_hp,
        mana: (id.kind == TargetKind::Player && stats.max_mana > 0.0)
            .then(|| (stats.mana / stats.max_mana).clamp(0.0, 1.0)),
        portrait,
        icon,
        badge,
        protected,
    })
}

#[derive(bevy::ecs::system::SystemParam)]
struct TargetParts<'w, 's> {
    portraits: Query<'w, 's, &'static mut PortraitView, With<TargetPortrait>>,
    icons: Query<'w, 's, (&'static mut KitImage, &'static mut Node), With<TargetKindIcon>>,
    badges: Query<
        'w,
        's,
        (&'static mut Node, &'static mut BackgroundColor),
        (With<TargetBadge>, Without<TargetKindIcon>),
    >,
    badge_texts: Query<
        'w,
        's,
        (&'static mut Text, &'static mut TextColor),
        (
            With<TargetBadgeText>,
            Without<TargetLabel>,
            Without<TargetValue>,
        ),
    >,
    rows: Query<
        'w,
        's,
        (
            &'static mut Node,
            Has<TargetNameRow>,
            Has<TargetHpBar>,
            Has<TargetManaLine>,
        ),
        (
            Or<(With<TargetNameRow>, With<TargetHpBar>, With<TargetManaLine>)>,
            Without<TargetKindIcon>,
            Without<TargetBadge>,
            Without<TargetBar>,
        ),
    >,
    bars: Query<'w, 's, &'static mut BarValue, With<TargetBar>>,
    fills: Query<
        'w,
        's,
        (&'static mut Node, &'static mut BackgroundColor),
        (
            With<TargetFill>,
            Without<TargetKindIcon>,
            Without<TargetBadge>,
            Without<TargetNameRow>,
            Without<TargetHpBar>,
            Without<TargetManaLine>,
        ),
    >,
    mana: Query<
        'w,
        's,
        &'static mut Node,
        (
            With<TargetManaFill>,
            Without<TargetFill>,
            Without<TargetKindIcon>,
            Without<TargetBadge>,
            Without<TargetNameRow>,
            Without<TargetHpBar>,
            Without<TargetManaLine>,
        ),
    >,
}

#[allow(clippy::too_many_arguments)]
fn update(
    state: Res<ScoreboardState>,
    game: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    context: Res<crate::input_context::GameplayInputContext>,
    mobile: Option<Res<MobileControls>>,
    target: Res<TargetState>,
    local: Query<&NetworkPlayerId, With<Player>>,
    local_team: Query<&crate::team::Team, With<Player>>,
    targets: Query<TargetItem>,
    mut labels: Query<
        (
            &mut Text,
            &mut TextColor,
            Option<&ScoreLabel>,
            Option<&KdaLabel>,
            Option<&TargetLabel>,
            Option<&TargetValue>,
            Option<&ScoreDetail>,
        ),
        Without<TargetBadgeText>,
    >,
    mut nodes: Query<
        (Option<&Name>, Option<&TestId>, &mut Node),
        (
            Without<TargetFill>,
            Without<TargetKindIcon>,
            Without<TargetBadge>,
            Without<TargetNameRow>,
            Without<TargetHpBar>,
            Without<TargetManaLine>,
            Without<TargetManaFill>,
        ),
    >,
    mut parts: TargetParts,
) {
    let form = Form::from_mobile(mobile.as_deref());
    let anatomy = TargetAnatomy::of(form);
    let (_, kda) = scores(game.scoreboard.as_ref(), local.single().ok().map(|id| id.0));
    let kills = team_kills(game.scoreboard.as_ref());
    let details = target
        .selected_entity
        .zip(target.selected_target)
        .and_then(|(entity, id)| {
            target_details(id, targets.get(entity).ok()?, game.scoreboard.as_ref())
        });
    for (mut text, mut ink, score_label, kda_label, target_label, value, detail) in &mut labels {
        let next = if let Some(label) = score_label {
            kills.map_or_else(
                || "—".to_owned(),
                |(green, blue)| match label {
                    ScoreLabel::Green => green.to_string(),
                    ScoreLabel::Blue => blue.to_string(),
                },
            )
        } else if kda_label.is_some() {
            kda.clone()
        } else if target_label.is_some() {
            let muted = details.as_ref().is_some_and(|d| d.muted);
            let next_ink = if muted {
                color::TEXT_MUTED
            } else {
                color::TEXT_PRIMARY
            };
            if ink.0 != next_ink {
                ink.0 = next_ink;
            }
            details
                .as_ref()
                .map_or_else(String::new, |details| details.name.clone())
        } else if value.is_some() {
            let alpha = if details.as_ref().is_some_and(|d| d.protected) {
                PROTECTED_ALPHA
            } else {
                1.0
            };
            let next_ink = color::TEXT_PRIMARY.with_alpha(alpha);
            if ink.0 != next_ink {
                ink.0 = next_ink;
            }
            details.as_ref().map_or_else(String::new, |d| {
                format!("{:.0} / {:.0}", d.hp, d.max) // i18n-allow: numbers
            })
        } else if detail.is_some() {
            let mut line = tr("edge.scoreboard.detail").to_owned();
            let buffs = local_team
                .single()
                .map(|team| crate::match_hud::team_buff_hud_text(&game.team_buffs, *team))
                .unwrap_or_default();
            if !buffs.is_empty() {
                line.push('\n');
                line.push_str(&buffs);
            }
            line
        } else {
            continue;
        };
        if text.0 != next {
            text.0 = next;
        }
    }
    let resting =
        session.join_confirmed() && matches!(game.state, GameState::Running) && !context.modal_open;
    for (name, id, mut node) in &mut nodes {
        let Some(name) = node_key(name, id) else {
            continue;
        };
        let show = match name {
            "MatchScoreStrip" | "MatchMenuButton" => Some(resting),
            "TargetHealthRoot" => Some(resting && details.is_some()),
            "ScoreboardRoot" => Some(state.open),
            _ => None,
        };
        if let Some(show) = show {
            let display = if show { Display::Flex } else { Display::None };
            if node.display != display {
                node.display = display;
            }
        }
    }
    let Some(details) = details else {
        return;
    };
    // HP: the kit bar owns the trail; the fill and value are also written
    // here so the plate is right in the frame the target changes.
    let fraction = (details.hp / details.max).clamp(0.0, 1.0);
    for mut value in &mut parts.bars {
        let next = BarValue {
            current: details.hp,
            max: details.max,
            respawn: None,
        };
        if *value != next {
            *value = next;
        }
    }
    let alpha = if details.protected {
        PROTECTED_ALPHA
    } else {
        1.0
    };
    for (mut node, mut fill) in &mut parts.fills {
        let width = Val::Percent(fraction * 100.0);
        if node.width != width {
            node.width = width;
        }
        let next = color::BAR_HP_ENEMY.with_alpha(alpha);
        if fill.0 != next {
            fill.0 = next;
        }
    }
    for mut view in &mut parts.portraits {
        if *view != details.portrait {
            *view = details.portrait.clone();
        }
    }
    for (mut image, mut node) in &mut parts.icons {
        let display = if details.icon.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        if let Some((icon, tint)) = details.icon {
            let next = KitImage::icon(icon, tint);
            if *image != next {
                *image = next;
            }
        }
    }
    for (mut node, mut fill) in &mut parts.badges {
        let display = if details.badge.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        let next = if details.badge.as_ref().is_some_and(|(_, gold)| *gold) {
            color::GOLD_500
        } else {
            color::SURFACE_3
        };
        if fill.0 != next {
            fill.0 = next;
        }
    }
    for (mut text, mut ink) in &mut parts.badge_texts {
        let (label, gold) = details.badge.clone().unwrap_or_default();
        if text.0 != label {
            text.0 = label;
        }
        let next = if gold {
            color::TEXT_ON_GOLD
        } else {
            color::TEXT_SECONDARY
        };
        if ink.0 != next {
            ink.0 = next;
        }
    }
    // Heroes carry the mana line under a raised HP bar; other kinds centre
    // name and bar in the plate (`target-*.md` § Plate anatomy).
    let hero = details.kind == TargetKind::Player;
    let mana_shown = hero && details.mana.is_some() && form == Form::Desktop;
    for (mut node, name_row, hp_bar, mana_line) in &mut parts.rows {
        if name_row {
            let top = Val::Px(if hero {
                anatomy.name_top_hero
            } else {
                anatomy.name_top
            });
            if node.top != top {
                node.top = top;
            }
        } else if hp_bar {
            let top = Val::Px(if hero {
                anatomy.hp_top_hero
            } else {
                anatomy.hp_top
            });
            if node.top != top {
                node.top = top;
            }
        } else if mana_line {
            let display = if mana_shown {
                Display::Flex
            } else {
                Display::None
            };
            if node.display != display {
                node.display = display;
            }
        }
    }
    for mut node in &mut parts.mana {
        let width = Val::Percent(details.mana.unwrap_or(0.0) * 100.0);
        if node.width != width {
            node.width = width;
        }
    }
}
/// Header aligned with the identity, K/D/A and gold cells. Level is on the portrait.
fn spawn_score_row<L: UiLabel>(parent: &mut ChildSpawnerCommands, cells: [L; 3]) {
    parent
        .spawn((Node {
            height: Val::Px(29.0),
            min_height: Val::Px(29.0),
            column_gap: Val::Px(8.0),
            padding: UiRect::horizontal(Val::Px(8.0)).with_left(Val::Px(11.0)),
            align_items: AlignItems::Center,
            ..default()
        },))
        .with_children(|row| {
            for (value, (width, grow)) in
                cells
                    .into_iter()
                    .zip([(0.0, 1.0), (64.0, 0.0), (52.0, 0.0)])
            {
                row.spawn((
                    value.into_text(),
                    ui::text(12.0),
                    TextColor(ui::MUTED),
                    TextLayout::new_with_justify(if grow > 0.0 {
                        Justify::Left
                    } else {
                        Justify::Right
                    }),
                    Node {
                        width: if grow > 0.0 {
                            Val::Auto
                        } else {
                            Val::Px(width)
                        },
                        min_width: Val::Px(0.0),
                        flex_grow: grow,
                        flex_shrink: if grow > 0.0 { 1.0 } else { 0.0 },
                        overflow: Overflow::clip(),
                        ..default()
                    },
                ));
            }
        });
}
fn spawn_live_score_row(
    parent: &mut ChildSpawnerCommands,
    player: &LiveScorePlayer,
    name: String,
    own: bool,
    art: Option<Handle<Image>>,
) {
    let team = if player.team == Team::Green {
        color::TEAM_GREEN
    } else {
        color::TEAM_BLUE
    };
    let ink = if player.connected {
        color::TEXT_PRIMARY
    } else {
        color::TEXT_DISABLED
    };
    parent
        .spawn((
            Node {
                height: Val::Px(48.0),
                min_height: Val::Px(48.0),
                flex_shrink: 0.0,
                width: Val::Percent(100.0),
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
                border: UiRect::left(Val::Px(3.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(if own {
                color::SURFACE_SELECTED
            } else {
                color::SURFACE_2
            }),
            BorderColor::all(if own { color::GOLD_500 } else { team }),
        ))
        .with_children(|row| {
            row.spawn((
                Node {
                    width: Val::Px(36.0),
                    height: Val::Px(36.0),
                    flex_shrink: 0.0,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(color::SURFACE_3),
            ))
            .with_children(|portrait| {
                if let Some(art) = art {
                    portrait.spawn((
                        ImageNode::new(art),
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                    ));
                } else {
                    portrait.spawn(icon_node(
                        game::class_icon(player.hero_class),
                        24.0,
                        color::TEXT_GOLD,
                    ));
                }
                portrait.spawn((
                    Text::new(player.level.to_string()),
                    ui::role_text(TextRole::Caption),
                    TextColor(color::TEXT_GOLD),
                    BackgroundColor(color::SURFACE_0),
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(-2.0),
                        bottom: Val::Px(-2.0),
                        ..default()
                    },
                ));
            });
            row.spawn(Node {
                flex_grow: 1.0,
                min_width: Val::Px(0.0),
                flex_basis: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                overflow: Overflow::clip_x(),
                ..default()
            })
            .with_children(|identity| {
                identity.spawn((
                    Text::new(name),
                    ui::role_text(TextRole::Label),
                    TextColor(ink),
                    TextLayout::new_with_no_wrap(),
                ));
                identity.spawn((
                    Text::new(data::hero_name(player.hero_class)),
                    ui::role_text(TextRole::Caption),
                    TextColor(color::TEXT_MUTED),
                    TextLayout::new_with_no_wrap(),
                ));
            });
            for (value, width, ink) in [
                (
                    format!("{}/{}/{}", player.kills, player.deaths, player.assists),
                    64.0,
                    ink,
                ),
                (player.earned_gold.to_string(), 52.0, color::TEXT_GOLD),
            ] {
                row.spawn((
                    Text::new(value),
                    ui::role_text(TextRole::Number),
                    TextColor(ink),
                    Node {
                        width: Val::Px(width),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    TextLayout::new_with_justify(Justify::Right),
                ));
            }
        });
}

fn render_rows(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    local: Query<&NetworkPlayerId, With<Player>>,
    rows: Query<(Entity, &ScoreRows)>,
    locale: Option<Res<Locale>>,
    mut previous: Local<Option<(Option<LiveScoreboard>, Option<u64>, u32)>>,
    avatars: Query<(&NetworkPlayerId, &NetworkAvatar)>,
    thumbnails: Option<Res<crate::team::AvatarThumbnails>>,
) {
    let id = local.single().ok().map(|p| p.0);
    let generation = locale.as_ref().map_or(0, |locale| locale.generation());
    let next = (game.scoreboard.clone(), id, generation);
    if previous.as_ref() == Some(&next) {
        return;
    }
    *previous = Some(next);
    for (entity, team) in &rows {
        commands.entity(entity).despawn_related::<Children>();
        commands.entity(entity).with_children(|parent| {
            let Some(board) = game.scoreboard.as_ref() else {
                text(parent, tr("edge.scoreboard.waiting"), 12.0, ui::MUTED);
                return;
            };
            let mut players: Vec<&LiveScorePlayer> =
                board.players.iter().filter(|p| p.team == team.0).collect();
            players.sort_by_key(|p| (std::cmp::Reverse(p.kills), p.player_id));
            if players.is_empty() {
                text(parent, tr("edge.scoreboard.empty"), 12.0, ui::MUTED);
            }
            for p in players {
                let name = if p.connected {
                    p.nickname.clone()
                } else {
                    trf("edge.scoreboard.offline", &[("name", &p.nickname)])
                };
                let art = avatars
                    .iter()
                    .find(|(id, _)| id.0 == p.player_id)
                    .and_then(|(_, avatar)| avatar.0.as_ref())
                    .and_then(|slug| thumbnails.as_ref().and_then(|thumbs| thumbs.0.get(slug)))
                    .cloned();
                spawn_live_score_row(parent, p, name, Some(p.player_id) == id, art);
            }
        });
    }
}
fn layout(
    windows: Query<&Window, With<PrimaryWindow>>,
    mobile: Res<MobileControls>,
    mut nodes: Query<(Option<&Name>, Option<&TestId>, &mut Node)>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let phone = mobile.enabled && window.height() < 600.0;
    for (name, id, mut node) in &mut nodes {
        match node_key(name, id).unwrap_or_default() {
            "ScoreboardRoot" => {
                node.padding = if phone {
                    UiRect {
                        left: Val::Px(mobile.safe.left),
                        right: Val::Px(mobile.safe.right),
                        top: Val::Px(mobile.safe.top),
                        bottom: Val::Px(mobile.safe.bottom),
                    }
                } else {
                    UiRect::ZERO
                };
            }
            "ScoreboardPanel" => {
                node.max_width = Val::Percent(if phone { 100.0 } else { 94.0 });
                node.max_height = Val::Percent(if phone { 100.0 } else { 92.0 });
                node.width = Val::Px(if phone {
                    window.width() - mobile.safe.left - mobile.safe.right - 32.0
                } else {
                    1088.0
                });
                node.padding = UiRect::all(Val::Px(if phone { 12.0 } else { 16.0 }));
            }
            "ScoreboardGreenRows" | "ScoreboardBlueRows" => {
                node.height = Val::Px(if phone {
                    if window.height() <= 340.0 {
                        135.0
                    } else {
                        220.0
                    }
                } else {
                    280.0
                });
            }
            _ => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::{
        mouse::{MouseScrollUnit, MouseWheel},
        touch::{TouchInput, TouchPhase},
    };
    fn app() -> App {
        let mut app = App::new();
        app.insert_resource(ClientSession::admitted_for_test())
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<crate::pause_menu::PauseMenuState>()
            .init_resource::<crate::help_overlay::HelpOverlayVisible>()
            .init_resource::<MobileControls>()
            .init_resource::<TargetState>()
            .add_message::<MouseWheel>()
            .add_message::<TouchInput>()
            .add_plugins((crate::input_context::InputContextPlugin, EdgeHudPlugin))
            .add_systems(
                Update,
                crate::pause_menu::toggle_pause_menu
                    .in_set(InputContextSet::Modal)
                    .after(crate::shop::ShopModalSet),
            );
        app
    }
    fn named(app: &mut App, name: &str) -> Entity {
        app.world_mut()
            .query::<(Entity, Option<&Name>, Option<&TestId>)>()
            .iter(app.world())
            .find(|(_, n, id)| node_key(*n, *id) == Some(name))
            .unwrap()
            .0
    }
    #[test]
    fn controller_start_opens_pause_like_the_menu_button_and_east_only_closes_it() {
        let mut app = app();
        app.init_resource::<crate::ui::BackPress>()
            .add_systems(Last, crate::ui::back::clear_back_press);
        let mut pad = crate::gamepad::GamepadControls::default();
        pad.active = true;
        app.insert_resource(pad);
        app.update();
        let pause = |app: &App| {
            app.world()
                .resource::<crate::pause_menu::PauseMenuState>()
                .open
        };
        let start = |app: &mut App, down: bool| {
            app.world_mut()
                .resource_mut::<crate::gamepad::GamepadControls>()
                .menu_pressed = down;
        };
        // East in play is not Esc: it never opens the menu.
        app.world_mut()
            .resource_mut::<crate::ui::BackPress>()
            .press();
        app.update();
        assert!(!pause(&app));
        start(&mut app, true);
        app.update();
        assert!(
            pause(&app),
            "Start opens the menu where the menu button can"
        );
        start(&mut app, false);
        app.update();
        assert!(pause(&app));
        start(&mut app, true);
        app.update();
        assert!(!pause(&app), "Start closes the menu when it is on top");
        start(&mut app, false);
        app.update();
        assert!(!pause(&app));
        start(&mut app, true);
        app.update();
        start(&mut app, false);
        assert!(pause(&app));
        app.world_mut()
            .resource_mut::<crate::ui::BackPress>()
            .press();
        app.update();
        assert!(!pause(&app), "East closes the menu like Esc");
        // The scoreboard is closed by East too.
        app.world_mut().resource_mut::<ScoreboardState>().open = true;
        app.update();
        app.world_mut()
            .resource_mut::<crate::ui::BackPress>()
            .press();
        app.update();
        assert!(!app.world().resource::<ScoreboardState>().open);
        assert!(!pause(&app), "consumed by the scoreboard");
    }
    #[test]
    fn score_button_blocks_same_frame_escape_restores_and_round_change_closes() {
        let mut app = app();
        app.update();
        let score = named(&mut app, "MatchScoreButton");
        app.world_mut()
            .get_mut::<Interaction>(score)
            .unwrap()
            .clone_from(&Interaction::Pressed);
        app.update();
        assert!(app.world().resource::<ScoreboardState>().open);
        assert!(
            !app.world()
                .resource::<crate::input_context::GameplayInputContext>()
                .gameplay_allowed()
        );
        app.world_mut()
            .get_mut::<Interaction>(score)
            .unwrap()
            .clone_from(&Interaction::None);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        assert!(!app.world().resource::<ScoreboardState>().open);
        assert!(
            !app.world()
                .resource::<crate::pause_menu::PauseMenuState>()
                .open
        );
        assert!(
            app.world()
                .resource::<crate::input_context::GameplayInputContext>()
                .gameplay_allowed()
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .get_mut::<Interaction>(score)
            .unwrap()
            .clone_from(&Interaction::Pressed);
        app.update();
        assert!(app.world().resource::<ScoreboardState>().open);
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .meta
            .match_id += 1;
        app.update();
        assert!(!app.world().resource::<ScoreboardState>().open);
    }
    #[test]
    fn open_scoreboard_rows_scroll_by_hovered_wheel_and_touch_drag() {
        let mut app = app();
        app.insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
            .add_systems(
                Update,
                crate::ui::scroll::scroll_areas.after(InputContextSet::Resolve),
            );
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        app.update();
        app.world_mut().resource_mut::<ScoreboardState>().open = true;
        app.update();
        assert_eq!(
            app.world().resource::<crate::ui::ModalStack>().top(),
            Some(crate::ui::ModalId::Scoreboard)
        );
        let rows = named(&mut app, "ScoreboardGreenRows");
        app.world_mut().entity_mut(rows).insert((
            ComputedNode {
                size: Vec2::new(300.0, 190.0),
                content_size: Vec2::new(300.0, 600.0),
                inverse_scale_factor: 1.0,
                ..default()
            },
            UiGlobalTransform::from_translation(Vec2::new(300.0, 300.0)),
            InheritedVisibility::VISIBLE,
        ));
        let y = |app: &App| app.world().get::<ScrollPosition>(rows).unwrap().y;
        let wheel = |app: &mut App| {
            app.world_mut().write_message(MouseWheel {
                unit: MouseScrollUnit::Line,
                x: 0.0,
                y: -1.0,
                window,
            });
            app.update();
        };
        wheel(&mut app);
        assert_eq!(y(&app), 0.0, "the wheel needs the cursor over the rows");
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(300.0, 300.0)));
        wheel(&mut app);
        assert_eq!(y(&app), 28.0);
        for (phase, dy) in [
            (TouchPhase::Started, 0.0),
            (TouchPhase::Moved, 40.0),
            (TouchPhase::Ended, 40.0),
        ] {
            app.world_mut().write_message(TouchInput {
                window,
                id: 1,
                phase,
                position: Vec2::new(300.0, 300.0 - dy),
                force: None,
            });
        }
        app.update();
        assert_eq!(y(&app), 68.0);
    }
    #[test]
    fn scoreboard_cannot_reopen_over_chat_or_frontend_from_key_or_button() {
        for frontend in [false, true] {
            let mut app = app();
            if frontend {
                app.insert_resource(State::new(crate::frontend::AppScreen::Home));
            } else {
                let mut social = crate::social::SocialClient::default();
                social.chat_open = true;
                app.insert_resource(social);
            }
            app.update();
            let score = named(&mut app, "MatchScoreButton");
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Tab);
            app.update();
            assert!(!app.world().resource::<ScoreboardState>().open);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .reset_all();
            *app.world_mut().get_mut::<Interaction>(score).unwrap() = Interaction::Pressed;
            app.update();
            assert!(!app.world().resource::<ScoreboardState>().open);
            if frontend {
                app.insert_resource(State::new(crate::frontend::AppScreen::InMatch));
            } else {
                app.world_mut()
                    .resource_mut::<crate::social::SocialClient>()
                    .chat_open = false;
            }
            *app.world_mut().get_mut::<Interaction>(score).unwrap() = Interaction::None;
            app.update();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Tab);
            app.update();
            assert!(app.world().resource::<ScoreboardState>().open);
        }
    }
    #[test]
    fn target_health_follows_each_kind_and_clears_invalid_or_dead_locks() {
        let mut app = app();
        app.update();
        let root = named(&mut app, "TargetHealthRoot");
        let fill = named(&mut app, "TargetHealthFill");
        let value = named(&mut app, "TargetHealthValue");
        for kind in [
            TargetKind::Player,
            TargetKind::Minion,
            TargetKind::Structure,
            TargetKind::Neutral,
        ] {
            let mut entity = app.world_mut().spawn(CombatStats {
                hp: 100.0,
                max_hp: 300.0,
                mana: 0.0,
                max_mana: 0.0,
            });
            match kind {
                TargetKind::Player => {
                    entity.insert(NetworkPlayerId(42));
                }
                TargetKind::Minion => {
                    entity.insert(NetworkMinionId(42));
                }
                TargetKind::Structure => {
                    entity.insert(NetworkStructureId(42));
                }
                TargetKind::Neutral => {
                    entity.insert(NetworkNeutralId(42));
                }
            }
            let entity = entity.id();
            {
                let mut target = app.world_mut().resource_mut::<TargetState>();
                target.selected_entity = Some(entity);
                target.selected_target = Some(crate::net::TargetId { kind, id: 42 });
            }
            app.update();
            assert_eq!(app.world().get::<Text>(value).unwrap().0, "100 / 300");
            assert_eq!(
                app.world().get::<Node>(root).unwrap().display,
                Display::Flex
            );
            let Val::Percent(percent) = app.world().get::<Node>(fill).unwrap().width else {
                panic!("health fill must be proportional")
            };
            assert!((percent - 100.0 / 3.0).abs() < 0.01);
            app.world_mut().get_mut::<CombatStats>(entity).unwrap().hp = 63.0;
            app.update();
            assert_eq!(app.world().get::<Text>(value).unwrap().0, "63 / 300");
            app.world_mut()
                .resource_mut::<TargetState>()
                .selected_target
                .as_mut()
                .unwrap()
                .id = 43;
            app.update();
            assert_eq!(
                app.world().get::<Node>(root).unwrap().display,
                Display::None
            );
            app.world_mut()
                .resource_mut::<TargetState>()
                .selected_target
                .as_mut()
                .unwrap()
                .id = 42;
            app.world_mut().get_mut::<CombatStats>(entity).unwrap().hp = 0.0;
            app.update();
            assert_eq!(
                app.world().get::<Node>(root).unwrap().display,
                Display::None
            );
            app.world_mut().despawn(entity);
        }
    }
    /// target-hero|minion|neutral|structure.md: every kind names itself,
    /// carries its portrait disc, badge, lock and (hero, desktop) level disc
    /// and mana line; a protected structure dims its bar to 55 %.
    #[test]
    fn target_plate_names_each_kind_with_its_badge_lock_level_and_mana() {
        use crate::net::{
            NetworkMapStructure, NetworkMinionKind, NetworkNeutralCampType,
            NetworkStructureProtected, NeutralCampType, StructureKind,
        };
        let mut app = app();
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .scoreboard = Some(LiveScoreboard {
            kills: Vec::new(),
            players: vec![LiveScorePlayer {
                avatar: None,
                player_id: 7,
                nickname: "DarkSentinel".into(),
                team: Team::Blue,
                hero_class: shared::HeroClass::Warden,
                kills: 0,
                deaths: 0,
                assists: 0,
                earned_gold: 0,
                level: 5,
                connected: true,
            }],
        });
        app.update();
        let stats = CombatStats {
            hp: 130.0,
            max_hp: 180.0,
            mana: 30.0,
            max_mana: 60.0,
        };
        struct Expect {
            name: &'static str,
            badge: Option<&'static str>,
            icon: Option<Icon>,
            level: Option<u32>,
            mana: bool,
            portrait: Icon,
            strong: bool,
            alpha: f32,
        }
        let cases: Vec<(TargetKind, Box<dyn Fn(&mut EntityWorldMut)>, Expect)> = vec![
            (
                TargetKind::Player,
                Box::new(|e: &mut EntityWorldMut| {
                    e.insert((
                        NetworkPlayerId(7),
                        NetworkHeroClass(shared::HeroClass::Warden),
                    ));
                }),
                Expect {
                    name: "DarkSentinel",
                    badge: None,
                    icon: Some(Icon::ClassWarden),
                    level: Some(5),
                    mana: true,
                    portrait: Icon::ClassWarden,
                    strong: false,
                    alpha: 1.0,
                },
            ),
            (
                TargetKind::Minion,
                Box::new(|e: &mut EntityWorldMut| {
                    e.insert((
                        NetworkMinionId(7),
                        NetworkMinionKind(shared::combat::MinionKind::Caster),
                    ));
                }),
                Expect {
                    name: "Caster minion",
                    badge: None,
                    icon: None,
                    level: None,
                    mana: false,
                    portrait: Icon::HudMinion,
                    strong: false,
                    alpha: 1.0,
                },
            ),
            (
                TargetKind::Neutral,
                Box::new(|e: &mut EntityWorldMut| {
                    e.insert((
                        NetworkNeutralId(7),
                        NetworkNeutralCampType(NeutralCampType::KingMutatioBoss),
                    ));
                }),
                Expect {
                    name: "King Mutatio",
                    badge: Some("BOSS"),
                    icon: None,
                    level: None,
                    mana: false,
                    portrait: Icon::NavCrown,
                    strong: true,
                    alpha: 1.0,
                },
            ),
            (
                TargetKind::Structure,
                Box::new(|e: &mut EntityWorldMut| {
                    e.insert((
                        NetworkStructureId(7),
                        StructureKind::Tower,
                        NetworkMapStructure {
                            lane: Some(shared::map::Lane::Mid),
                            ..default()
                        },
                        NetworkStructureProtected(true),
                    ));
                }),
                Expect {
                    name: "Tower",
                    badge: Some("MID"),
                    icon: Some(Icon::NavLock),
                    level: None,
                    mana: false,
                    portrait: Icon::HudTower,
                    strong: false,
                    alpha: PROTECTED_ALPHA,
                },
            ),
            (
                TargetKind::Structure,
                Box::new(|e: &mut EntityWorldMut| {
                    e.insert((NetworkStructureId(7), StructureKind::BaseTower));
                }),
                Expect {
                    name: "Base",
                    badge: None,
                    icon: None,
                    level: None,
                    mana: false,
                    portrait: Icon::HudTower,
                    strong: true,
                    alpha: 1.0,
                },
            ),
        ];
        for (kind, insert, expect) in cases {
            let mut entity = app.world_mut().spawn(stats);
            insert(&mut entity);
            let entity = entity.id();
            {
                let mut target = app.world_mut().resource_mut::<TargetState>();
                target.selected_entity = Some(entity);
                target.selected_target = Some(crate::net::TargetId { kind, id: 7 });
            }
            app.update();
            let world = app.world_mut();
            let name = world
                .query_filtered::<&Text, With<TargetLabel>>()
                .single(world)
                .unwrap()
                .0
                .clone();
            assert_eq!(name, expect.name);
            let (badge_node, badge_text) = (
                world
                    .query_filtered::<&Node, With<TargetBadge>>()
                    .single(world)
                    .unwrap()
                    .display,
                world
                    .query_filtered::<&Text, With<TargetBadgeText>>()
                    .single(world)
                    .unwrap()
                    .0
                    .clone(),
            );
            match expect.badge {
                Some(badge) => {
                    assert_eq!(badge_node, Display::Flex, "{}", expect.name);
                    assert_eq!(badge_text, badge);
                }
                None => assert_eq!(badge_node, Display::None, "{}", expect.name),
            }
            let (icon, icon_node) = world
                .query_filtered::<(&KitImage, &Node), With<TargetKindIcon>>()
                .single(world)
                .unwrap();
            match expect.icon {
                Some(expected) => {
                    assert_eq!(icon_node.display, Display::Flex, "{}", expect.name);
                    assert_eq!(*icon, KitImage::icon(expected, color::TEXT_MUTED));
                }
                None => assert_eq!(icon_node.display, Display::None, "{}", expect.name),
            }
            let portrait = world
                .query_filtered::<&PortraitView, With<TargetPortrait>>()
                .single(world)
                .unwrap()
                .clone();
            assert_eq!(portrait.fallback, expect.portrait, "{}", expect.name);
            assert_eq!(portrait.level, expect.level, "{}", expect.name);
            assert_eq!(portrait.strong_rim, expect.strong, "{}", expect.name);
            let mana = world
                .query_filtered::<&Node, With<TargetManaLine>>()
                .single(world)
                .unwrap()
                .display;
            assert_eq!(mana == Display::Flex, expect.mana, "{}", expect.name);
            let fill = world
                .query_filtered::<&BackgroundColor, With<TargetFill>>()
                .single(world)
                .unwrap()
                .0;
            assert_eq!(fill, color::BAR_HP_ENEMY.with_alpha(expect.alpha));
            app.world_mut().despawn(entity);
        }
    }

    #[test]
    fn live_score_totals_use_both_teams_and_distinguish_missing_data() {
        let player = |id, team, kills| LiveScorePlayer {
            avatar: None,
            player_id: id,
            nickname: "Player".into(),
            team,
            hero_class: shared::HeroClass::Warrior,
            kills,
            deaths: 2,
            assists: 3,
            earned_gold: 57,
            level: 4,
            connected: true,
        };
        let board = LiveScoreboard {
            kills: Vec::new(),
            players: vec![
                player(1, Team::Green, 4),
                player(2, Team::Green, 5),
                player(3, Team::Blue, 6),
            ],
        };
        assert_eq!(
            scores(Some(&board), Some(2)),
            ("9 : 6".into(), "5/2/3".into())
        );
        assert_eq!(scores(None, Some(2)), ("— : —".into(), "—/—/—".into()));
        assert_eq!(
            scores(Some(&LiveScoreboard::default()), None),
            ("0 : 0".into(), "—/—/—".into())
        );
    }

    #[test]
    fn scoreboard_headings_follow_the_language() {
        use crate::i18n::{Locale, LocaleId, relabel_localized};
        let mut app = app();
        app.insert_resource(Locale::detached(LocaleId::parse("zh-Hans").unwrap()))
            .add_systems(PostUpdate, relabel_localized);
        app.update();
        let texts: Vec<String> = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|text| text.0.clone())
            .collect();
        for expected in ["对局比分", "关闭", "绿队", "蓝队", "玩家", "KDA"] {
            assert!(
                texts.iter().any(|text| text == expected),
                "{expected}: {texts:?}"
            );
        }
        assert!(!texts.iter().any(|text| text == "MATCH SCORE"));
    }
}
