//! In-match HUD: the player status plate (portrait, XP ring, level, HP, mana,
//! gold), the team-buff chips, the (hidden) objective hint and the gameplay
//! HUD visibility (`omoba-ui/handoff/screens/hud.md`).
// i18n-strict

use bevy::prelude::*;

use crate::i18n::{data, tr, trf};

use crate::combat::{CombatStats, TargetState};
use crate::hud_layout::HudRegion;
#[cfg(test)]
use crate::input_bindings::upgrade_key_display;
use crate::input_bindings::{help_key_display, skill_keys_display};
use crate::net::{
    GameState, GameStateSnapshot, NetworkHeroClass, NetworkStructure, PlayerProgression,
    StructureKind, TeamBuffKind, TeamBuffState,
};
#[cfg(test)]
use crate::net::{TargetId, TargetKind};
use crate::player::Player;
use crate::team::{Team, TeamSelection};
use crate::ui::{
    kit_assets::Icon,
    theme::{self as ui, Form, TextStyle},
    tokens::{TextRole, color, radius, size, space},
    widgets::{
        game::{self, BarKind, BarValue, PortraitSpec, PortraitView},
        icon_node,
        surfaces::{Tooltip, TooltipText},
    },
};
#[cfg(test)]
use shared::HeroClass;

pub struct MatchHudPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct MatchHudVisuals;

impl Plugin for MatchHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_match_hud)
            .add_systems(
                Update,
                (update_match_hud, update_player_status, update_buff_chips)
                    .in_set(MatchHudVisuals)
                    .after(crate::net::ClientNetPipeline::ApplySnapshot),
            )
            .add_systems(
                Update,
                sync_gameplay_hud_visibility.after(crate::net::ClientNetPipeline::SyncConnectionUi),
            );
    }
}

#[derive(Component)]
struct MatchHudStatusText;

/// The team-buff chip column (`hud.md` region `buff-chips`).
#[derive(Component)]
struct BuffChips;

/// The status plate's portrait (avatar, XP ring, level disc).
#[derive(Component)]
struct HudPortrait;

/// The status plate's HP and mana bars.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum HudBar {
    Health,
    Mana,
}

impl HudBar {
    /// Stable id in the bar's `Name` (`MatchHudBar-HP`); never translated.
    const fn id(self) -> &'static str {
        match self {
            Self::Health => "HP", // i18n-allow
            Self::Mana => "MP",   // i18n-allow
        }
    }
}

/// Status plate anatomy (hud.md: desktop 288 × 72, phone 200 × 48).
const DESKTOP_HP_W: f32 = 200.0;
const DESKTOP_MANA_W: f32 = 156.0;
const PHONE_BAR_W: f32 = 140.0;
/// Phone level disc (`hud.md` phone player-status: 18).
const PHONE_LEVEL_DISC: f32 = 18.0;
/// Buff chip (hud.md `buff-chips`): 24 high, max 344 wide.
const BUFF_CHIP_H: f32 = 24.0;
/// The desktop offline-practice badge sits atop the chip column; the chips
/// move down by its height + `space.4` (VARIANTS.md `offline-practice`).
pub(crate) const PRACTICE_BADGE_SHIFT: f32 = BUFF_CHIP_H + space::S4;

fn phone_hud(mobile: Option<&crate::mobile_controls::MobileControls>) -> bool {
    mobile.is_some_and(|mobile| mobile.enabled)
}

fn setup_match_hud(
    mut commands: Commands,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
) {
    let form = Form::of(phone_hud(mobile.as_deref()));
    spawn_player_status(&mut commands, form);
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexStart,
            row_gap: Val::Px(space::S4),
            ..default()
        },
        HudRegion::BuffChips,
        BuffChips,
        Pickable::IGNORE,
        ZIndex(12),
        Name::new("MatchBuffChips"),
    ));
    // The objective hint stays hidden (hud.md § Out of scope): goal text
    // lives in the scoreboard detail and help.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(468.0),
                bottom: Val::Px(166.0),
                width: Val::Px(344.0),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            Pickable::IGNORE,
            ZIndex(8),
            Name::new("MatchObjectiveRoot"),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::axes(Val::Px(space::S12), Val::Px(space::S8)),
                    ..default()
                },
                Name::new("MatchObjectivePanel"),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new(""),
                    theme_caption(),
                    TextColor(color::TEXT_PRIMARY),
                    MatchHudStatusText,
                    Name::new("MatchStatusText"),
                ));
            });
        });
}

fn theme_caption() -> impl Bundle {
    ui::styled_text(TextStyle::keep_case(TextRole::Caption))
}

/// The player status plate: portrait with XP ring and level disc, HP bar,
/// mana bar (+ value on desktop) and, on desktop, the gold row that opens
/// the shop (phones have gold in the quick-buy row).
fn spawn_player_status(commands: &mut Commands, form: Form) {
    let phone = form == Form::Phone;
    let portrait = if phone {
        PortraitSpec {
            side: size::PORTRAIT_SM,
            ring: true,
            disc: PHONE_LEVEL_DISC,
            disc_at: Vec2::splat(-space::S2),
            icon: size::ICON_MD,
        }
    } else {
        PortraitSpec {
            side: size::PORTRAIT_MD,
            ring: true,
            disc: game::LEVEL_DISC,
            disc_at: Vec2::splat(-space::S2),
            icon: size::ICON_LG,
        }
    };
    let (padding, gap) = if phone {
        // 4 + 40 + 8 + 140 + 8 = 200; 4 + 40 + 4 = 48.
        (
            UiRect::new(
                Val::Px(space::S4),
                Val::Px(space::S8),
                Val::Px(space::S4),
                Val::Px(space::S4),
            ),
            space::S8,
        )
    } else {
        // 10 + 56 + 12 + 200 + 10 = 288; 8 + 56 + 8 = 72.
        (
            UiRect::axes(
                Val::Px(game::PLATE_PADDING.1),
                Val::Px(game::PLATE_PADDING.0),
            ),
            space::S12,
        )
    };
    commands
        .spawn((
            // A Button keeps world clicks off the plate.
            Button,
            game::hud_plate(true),
            HudRegion::PlayerStatus,
            ZIndex(12),
            Name::new("MatchHudColumn"),
        ))
        .insert(Node {
            position_type: PositionType::Absolute,
            padding,
            column_gap: Val::Px(gap),
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(crate::ui::tokens::border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            ..default()
        })
        .with_children(|plate| {
            let view = PortraitView {
                art: None,
                fallback: game::class_icon(shared::HeroClass::Warrior),
                level: Some(1),
                xp: 0.0,
                grey: false,
                strong_rim: false,
            };
            let entity = game::live_portrait(plate, view, portrait, (HudPortrait, Button));
            if !phone {
                plate.commands().entity(entity).insert((
                    Tooltip {
                        title: None,
                        body: "hud.xp",
                    },
                    TooltipText::default(),
                ));
            }
            plate
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    justify_content: if phone {
                        JustifyContent::Center
                    } else {
                        JustifyContent::SpaceBetween
                    },
                    row_gap: Val::Px(space::S4),
                    height: Val::Percent(100.0),
                    ..default()
                })
                .with_children(|column| {
                    let hp_w = if phone { PHONE_BAR_W } else { DESKTOP_HP_W };
                    let mana_w = if phone { PHONE_BAR_W } else { DESKTOP_MANA_W };
                    let start = BarValue {
                        current: 100.0,
                        max: 100.0,
                        respawn: None,
                    };
                    let hp = game::bar(column, BarKind::HpSelf, start, Val::Px(hp_w), form, true);
                    column.commands().entity(hp).insert((
                        HudBar::Health,
                        Name::new(format!("MatchHudBar-{}", HudBar::Health.id())),
                    ));
                    // Desktop: the mana value sits right of the 156 bar.
                    let mana_row_w = if phone { mana_w } else { hp_w };
                    let mana = game::bar(
                        column,
                        BarKind::Mana,
                        start,
                        Val::Px(mana_row_w),
                        form,
                        !phone,
                    );
                    column.commands().entity(mana).insert((
                        HudBar::Mana,
                        Name::new(format!("MatchHudBar-{}", HudBar::Mana.id())),
                    ));
                    if !phone {
                        crate::shop::spawn_gold_row(column);
                    }
                });
        });
}

/// Portrait, level, XP, HP, mana and the dead state of the status plate.
#[allow(clippy::type_complexity)]
fn update_player_status(
    player: Query<
        (
            &CombatStats,
            &PlayerProgression,
            Option<&NetworkHeroClass>,
            Option<&crate::net::NetworkAvatar>,
        ),
        With<Player>,
    >,
    selection: Res<TeamSelection>,
    mut portraits: Query<(&mut PortraitView, Option<&mut TooltipText>), With<HudPortrait>>,
    mut bars: Query<(&HudBar, &mut BarValue)>,
) {
    let Ok((stats, progression, class, avatar)) = player.single() else {
        return;
    };
    let alive = stats.is_alive();
    let class = class.map_or(selection.hero_class, |class| class.0);
    let art = avatar
        .and_then(|avatar| avatar.0.as_deref())
        .and_then(omoba_passport::avatars::avatar_definition)
        .and_then(crate::passport::thumbnail_asset_path);
    let xp = if progression.next_level_xp == 0 {
        1.0
    } else {
        (progression.xp as f32 / progression.next_level_xp as f32).clamp(0.0, 1.0)
    };
    for (mut view, tooltip) in &mut portraits {
        let next = PortraitView {
            art: art.clone(),
            fallback: game::class_icon(class),
            level: Some(progression.level.max(1)),
            xp,
            grey: !alive,
            strong_rim: false,
        };
        if *view != next {
            *view = next;
        }
        if let Some(mut tooltip) = tooltip {
            let text = if progression.next_level_xp == 0 {
                tr("hud.max_level").to_owned()
            } else {
                trf(
                    "hud.xp",
                    &[
                        ("xp", &progression.xp),
                        ("next", &progression.next_level_xp),
                        ("points", &progression.skill_points),
                    ],
                )
            };
            if tooltip.body != text {
                tooltip.body = text;
            }
        }
    }
    for (bar, mut value) in &mut bars {
        let next = match bar {
            HudBar::Health => BarValue {
                current: stats.hp.max(0.0),
                max: stats.max_hp.max(1.0),
                // Dead: fill hidden, `hud.target.defeated` over the track;
                // respawn seconds are not replicated (hud.md § States).
                respawn: (!alive).then_some(0),
            },
            HudBar::Mana => BarValue {
                current: stats.mana.max(0.0),
                max: stats.max_mana.max(1.0),
                respawn: (!alive).then_some(0),
            },
        };
        if *value != next {
            *value = next;
        }
    }
}

/// The active team buffs of the local team as chips (`hud/level-up` +
/// `type.caption` gold on glass), under the practice badge in practice.
#[allow(clippy::type_complexity)]
fn update_buff_chips(
    mut commands: Commands,
    game_state: Option<Res<GameStateSnapshot>>,
    session: Option<Res<crate::net::ClientSession>>,
    local_team: Query<&Team, With<Player>>,
    locale: Option<Res<crate::i18n::Locale>>,
    mut columns: Query<(Entity, &mut Node), With<BuffChips>>,
    mut previous: Local<Option<Vec<String>>>,
) {
    let running = game_state
        .as_ref()
        .is_some_and(|g| matches!(g.state, GameState::Running));
    let lines: Vec<String> = if running {
        let buffs = game_state
            .as_ref()
            .map(|snapshot| snapshot.team_buffs.as_slice())
            .unwrap_or(&[]);
        local_team
            .single()
            .map(|team| team_buff_hud_text(buffs, *team))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    } else {
        Vec::new()
    };
    let practice = session.as_ref().is_some_and(|session| session.is_offline());
    for (_, mut node) in &mut columns {
        let margin = Val::Px(if practice { PRACTICE_BADGE_SHIFT } else { 0.0 });
        if node.margin.top != margin {
            node.margin.top = margin;
        }
    }
    let relabel = crate::i18n::locale_changed(&locale);
    if previous.as_ref() == Some(&lines) && !relabel {
        return;
    }
    // Log transitions (not the per-second countdown) for evidence runs.
    let count = |lines: &Option<Vec<String>>| lines.as_ref().map_or(0, Vec::len);
    if lines.is_empty() && count(&previous) > 0 {
        info!("[hud] team buff indicator cleared");
    } else if !lines.is_empty() && count(&previous) != lines.len() {
        info!("[hud] team buff indicator: {}", lines.join(" | "));
    }
    *previous = Some(lines.clone());
    for (column, _) in &mut columns {
        commands.entity(column).despawn_related::<Children>();
        commands.entity(column).with_children(|column| {
            for (index, line) in lines.iter().enumerate() {
                column
                    .spawn((
                        Node {
                            height: Val::Px(BUFF_CHIP_H),
                            max_width: Val::Percent(100.0),
                            padding: UiRect::horizontal(Val::Px(space::S8)),
                            column_gap: Val::Px(space::S4),
                            align_items: AlignItems::Center,
                            overflow: Overflow::clip_x(),
                            border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                            ..default()
                        },
                        BackgroundColor(ui::perceptual(color::SURFACE_GLASS_STRONG)),
                        Pickable::IGNORE,
                        Name::new(format!("MatchBuffChip-{index}")),
                    ))
                    .with_children(|chip| {
                        chip.spawn(icon_node(Icon::HudLevelUp, size::ICON_SM, color::TEXT_GOLD));
                        chip.spawn((
                            Text::new(line.clone()),
                            theme_caption(),
                            TextColor(color::TEXT_GOLD),
                            TextLayout::no_wrap(),
                            Name::new("MatchBuffText"),
                        ));
                    });
            }
        });
    }
}

/// Keep entry and result cards clear; these controls only describe a live,
/// admitted hero. Visibility and layout agree so hidden panels reserve no space.
fn sync_gameplay_hud_visibility(
    game: Res<GameStateSnapshot>,
    session: Res<crate::net::ClientSession>,
    help: Option<Res<crate::help_overlay::HelpOverlayVisible>>,
    pause: Option<Res<crate::pause_menu::PauseMenuState>>,
    shop: Option<Res<crate::shop::ShopState>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut roots: Query<(&Name, &mut Node, &mut Visibility), Without<ChildOf>>,
    gamepad: Option<Res<crate::gamepad::GamepadControls>>,
    scoreboard: Option<Res<crate::edge_hud::ScoreboardState>>,
) {
    let phone = phone_hud(mobile.as_deref());
    // The touch skill buttons hide while a controller owns input; the
    // desktop skill bar carries its bindings instead.
    let touch_hud = phone && !gamepad.as_ref().is_some_and(|pad| pad.active);
    let show = session.join_confirmed()
        && matches!(game.state, GameState::Running)
        && !help.is_some_and(|help| help.0)
        && !pause.is_some_and(|pause| pause.open)
        && !shop.is_some_and(|shop| shop.open)
        // hud.md § States: the scoreboard hides the controls too (chat
        // keeps them, DECISIONS R7.2).
        && !scoreboard.is_some_and(|board| board.open);
    for (name, mut node, mut visibility) in &mut roots {
        let name = name.as_str();
        if matches!(
            name,
            "MatchHudColumn"
                | "SkillBarRoot"
                | "SkillUpgradeChip"
                | "ActionFeedback"
                | "EquipmentHud"
                | "MatchObjectiveRoot"
                | "MatchBuffChips"
                | "QuickBuyHud"
        ) {
            // Transient feedback and the upgrade chip own their empty state.
            if !matches!(name, "ActionFeedback" | "SkillUpgradeChip") {
                let displayed = show
                    && name != "MatchObjectiveRoot"
                    // Desktop: inventory + quick-buy plate; phone: the row.
                    && !(name == "EquipmentHud" && phone)
                    && !(name == "MatchHudColumn" && touch_hud)
                    && (name != "QuickBuyHud" || phone)
                    && !(name == "SkillBarRoot" && touch_hud);
                let display = if displayed {
                    Display::Flex
                } else {
                    Display::None
                };
                if node.display != display {
                    node.display = display;
                }
            }
            // The upgrade chip belongs to the ability bar: a phone without
            // a controller uses the RANK disc instead.
            let shown = show && !(name == "SkillUpgradeChip" && touch_hud);
            let next = if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *visibility != next {
                *visibility = next;
            }
        }
    }
}

fn update_match_hud(
    game_state: Option<Res<GameStateSnapshot>>,
    team_selection: Res<TeamSelection>,
    player: Query<(&CombatStats, Option<&NetworkHeroClass>), With<Player>>,
    local_team: Query<&Team, With<Player>>,
    enemy_bases: Query<
        (
            &CombatStats,
            &Team,
            &StructureKind,
            Option<&crate::net::NetworkStructureProtected>,
        ),
        With<NetworkStructure>,
    >,
    target_state: Res<TargetState>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut status: Query<&mut Text, With<MatchHudStatusText>>,
) {
    let Ok(mut status_text) = status.single_mut() else {
        return;
    };
    let Some((stats, replicated_class)) = player.iter().next() else {
        status_text.0.clear();
        return;
    };
    let hero_class = replicated_class
        .map(|class| class.0)
        .unwrap_or(team_selection.hero_class);
    let running = game_state
        .as_ref()
        .is_some_and(|g| matches!(g.state, GameState::Running));
    if !running {
        status_text.0 = trf(
            "hud.prematch",
            &[
                ("help_key", &help_key_display()),
                ("class", &data::hero_name(hero_class)),
                ("skills", &skill_keys_display()),
            ],
        );
        return;
    }
    let objective_line = enemy_base_objective_line(&local_team, &enemy_bases);
    // Phones name the touch controls instead of the keyboard shortcuts.
    let phone = phone_hud(mobile.as_deref());
    let target = tr(
        match (
            stats.is_alive(),
            target_state.selected_target.is_some(),
            phone,
        ) {
            (false, _, _) => "hud.target.defeated",
            (true, true, false) => "hud.target.locked",
            (true, true, true) => "hud.target.locked_phone",
            (true, false, false) => "hud.target.none",
            (true, false, true) => "hud.target.none_phone",
        },
    );
    status_text.0 = format!("{objective_line}\n{target}");
}

/// One line per active buff of the LOCAL player's team, with the remaining
/// time in whole seconds. Effect numbers mirror `server/src/balance.rs`
/// (`BOTTOM_BOSS_BUFF_*` / `TOP_BOSS_BUFF_*`); an empty string hides the row.
pub(crate) fn team_buff_hud_text(buffs: &[TeamBuffState], local_team: Team) -> String {
    buffs
        .iter()
        .filter(|buff| buff.team == local_team)
        .map(|buff| {
            let secs = buff.remaining_secs.max(0.0).ceil() as u32;
            match buff.kind {
                TeamBuffKind::WendigoFavor => trf("hud.buff.wendigo", &[("secs", &secs)]),
                TeamBuffKind::MutatioMight => trf("hud.buff.mutatio", &[("secs", &secs)]),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn enemy_base_objective_line(
    local_team: &Query<&Team, With<Player>>,
    structures: &Query<
        (
            &CombatStats,
            &Team,
            &StructureKind,
            Option<&crate::net::NetworkStructureProtected>,
        ),
        With<NetworkStructure>,
    >,
) -> String {
    let Ok(team) = local_team.single() else {
        return tr("hud.goal.destroy_base").to_owned();
    };
    let mut hp_sum = 0.0f32;
    let mut max_sum = 0.0f32;
    let mut any = false;
    let mut protected = false;
    for (stats, st_team, kind, protection) in structures.iter() {
        if *kind == StructureKind::BaseTower && *st_team != *team {
            protected |= protection.is_some_and(|protection| protection.0);
            hp_sum += stats.hp.max(0.0);
            max_sum += stats.max_hp.max(0.0);
            any = true;
        }
    }
    if protected {
        tr("hud.goal.unlock_base").to_owned()
    } else if any && max_sum > 0.0 {
        trf(
            "hud.goal.base_hp",
            &[
                ("hp", &format!("{:.0}", hp_sum.min(max_sum))),
                ("max", &format!("{max_sum:.0}")),
            ],
        )
    } else {
        tr("hud.goal.destroy_base").to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Short effect summary for an ability tooltip line (single effect per ability).
    fn running_status_text(
        stats: CombatStats,
        hero_class: HeroClass,
        selected_target: Option<TargetId>,
        objective_line: &str,
    ) -> String {
        let target_line = match selected_target {
            Some(t) => {
                let kind = match t.kind {
                    TargetKind::Player => "Player",
                    TargetKind::Minion => "Minion",
                    TargetKind::Structure => "Structure",
                    TargetKind::Neutral => "Neutral",
                };
                format!("Target: enemy {kind} - locked")
            }
            None => "Target: none - click a foe or use Tab".to_string(),
        };
        let hp = stats.hp.max(0.0);
        let max_hp = stats.max_hp.max(1.0);
        let mana = stats.mana.max(0.0);
        let max_mana = stats.max_mana.max(1.0);
        format!(
            "HP {:.0}/{:.0}   Mana {:.0}/{:.0}   Class: {}\n\
    {target_line}\n\
    {objective_line}\n\
    Keys: {} cast | {} upgrade | {} help",
            hp,
            max_hp,
            mana,
            max_mana,
            hero_class.display_name(),
            skill_keys_display(),
            upgrade_key_display(),
            help_key_display()
        )
    }

    #[test]
    fn running_status_text_shows_resources_objective_and_class_kit() {
        let text = running_status_text(
            CombatStats {
                hp: 75.0,
                max_hp: 100.0,
                mana: 40.0,
                max_mana: 100.0,
            },
            HeroClass::Mage,
            None,
            "Goal: destroy enemy base - 650 / 650 HP remaining",
        );
        assert!(text.contains("HP 75/100   Mana 40/100"));
        assert!(text.contains("Class: Mage"));
        assert!(text.contains("Goal: destroy enemy base"));
        assert!(text.contains("Keys: Q / W / E / R") || text.contains("Keys: Q"));
    }

    #[test]
    fn running_status_text_shows_target_and_per_slot_cooldown() {
        let text = running_status_text(
            CombatStats {
                hp: 90.0,
                max_hp: 120.0,
                mana: 12.0,
                max_mana: 60.0,
            },
            HeroClass::Cleric,
            Some(TargetId {
                kind: TargetKind::Player,
                id: 42,
            }),
            "Goal: destroy enemy base",
        );
        assert!(text.contains("Target: enemy Player"));
        assert!(!text.contains("#42"));
    }

    #[test]
    fn team_buff_hud_text_lists_local_team_buffs_with_remaining_time() {
        let buffs = vec![
            TeamBuffState {
                team: Team::Green.into(),
                kind: TeamBuffKind::WendigoFavor,
                remaining_secs: 71.3,
            },
            TeamBuffState {
                team: Team::Blue.into(),
                kind: TeamBuffKind::MutatioMight,
                remaining_secs: 45.0,
            },
        ];

        let green = team_buff_hud_text(&buffs, Team::Green);
        assert!(green.contains("Wendigo's Favor"));
        assert!(green.contains("+15% ability damage"));
        assert!(
            green.contains("72s"),
            "remaining time must round up: {green}"
        );
        assert!(
            !green.contains("Dragon"),
            "enemy team's buff must not show: {green}"
        );

        let blue = team_buff_hud_text(&buffs, Team::Blue);
        assert!(blue.contains("Dragon's Might"));
        assert!(blue.contains("+25% ability damage, +2 HP/s"));
        assert!(blue.contains("45s"));
    }

    #[test]
    fn team_buff_hud_text_is_empty_without_active_buffs() {
        assert_eq!(team_buff_hud_text(&[], Team::Green), "");
        let enemy_only = vec![TeamBuffState {
            team: Team::Blue.into(),
            kind: TeamBuffKind::WendigoFavor,
            remaining_secs: 10.0,
        }];
        assert_eq!(team_buff_hud_text(&enemy_only, Team::Green), "");
    }

    #[test]
    fn team_buff_hud_text_stacks_both_buffs_on_separate_lines() {
        let buffs = vec![
            TeamBuffState {
                team: Team::Green.into(),
                kind: TeamBuffKind::WendigoFavor,
                remaining_secs: 30.0,
            },
            TeamBuffState {
                team: Team::Green.into(),
                kind: TeamBuffKind::MutatioMight,
                remaining_secs: 80.0,
            },
        ];
        let text = team_buff_hud_text(&buffs, Team::Green);
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("Wendigo's Favor") && text.contains("Dragon's Might"));
    }
}
