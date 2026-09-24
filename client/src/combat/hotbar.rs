use crate::domain::CombatStats;
use crate::input_bindings::SKILL_UPGRADE_KEY;
use crate::input_context::GameplayInputContext;
use crate::net::{
    GameState, GameStateSnapshot, NetworkCommand, NetworkHeroClass, NetworkPlayerId,
    PlayerProgression,
};
use crate::player::Player;
use crate::team::TeamSelection;
use bevy::prelude::*;
use shared::{
    MAX_ABILITY_RANK, SkillSlot, TargetingMode, ability_for_class_slot, scaled_cast_range,
    scaled_mana_cost,
};

use super::cast::{PendingCast, queue_cast_request, within_cast_range};
use super::cooldown::{LocalCastCooldown, local_hero_class};
use super::feedback::{ActionFeedback, ActionFeedbackText};
use super::selection::TargetState;

pub(super) const SKILL_SLOT_SIZE: f32 = 80.0;
const SKILL_SLOT_GAP: f32 = 8.0;
const SKILL_BUTTON_COLOR: Color = crate::ui_theme::PANEL;
const SKILL_BUTTON_HOVER_COLOR: Color = crate::ui_theme::HOVER;
const SKILL_BUTTON_PRESS_COLOR: Color = crate::ui_theme::TILE;
const SKILL_UPGRADE_READY_COLOR: Color = Color::srgba(0.20, 0.62, 0.26, 0.95);
const SKILL_UPGRADE_HOVER_COLOR: Color = Color::srgba(0.26, 0.72, 0.32, 0.98);
const SKILL_UPGRADE_IDLE_COLOR: Color = Color::srgba(0.16, 0.16, 0.18, 0.55);

#[derive(Component)]
pub(super) struct SkillBarSlot {
    slot: usize,
}

#[derive(Component)]
pub(super) struct DesktopSkillIcon {
    pub(super) slot: usize,
}

#[derive(Component)]
pub(super) struct SkillUpgradeButton {
    slot: usize,
}

#[derive(Component)]
pub(super) struct SkillRankLabel {
    pub(super) slot: usize,
}

/// Ability-name caption on a hotbar slot; follows the selected class kit.
#[derive(Component)]
pub(super) struct SkillNameLabel {
    slot: usize,
}

pub(super) fn setup_combat_ui(mut commands: Commands, asset_server: Option<Res<AssetServer>>) {
    let skill_atlas = asset_server.map(|assets| assets.load(crate::skill_icons::ATLAS_PATH));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(258.0),
            left: Val::Px(468.0),
            max_width: Val::Px(344.0),
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.88, 0.5)),
        BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.8)),
        ZIndex(14),
        ActionFeedbackText,
        Name::new("ActionFeedback"),
    ));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(468.0),
                bottom: Val::Px(16.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(SKILL_SLOT_GAP),
                align_items: AlignItems::FlexEnd,
                ..default()
            },
            ZIndex(12),
            Name::new("SkillBarRoot"),
        ))
        .with_children(|row| {
            for i in 0..4 {
                let label = crate::input_bindings::SKILL_SLOT_KEY_LABELS[i];
                row.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(4.0),
                        ..default()
                    },
                    Name::new(format!("SkillColumn-{label}")),
                ))
                .with_children(|col| {
                    // Upgrade arrow above the slot; bright when a point can be spent.
                    col.spawn((
                        Button,
                        Node {
                            width: Val::Px(SKILL_SLOT_SIZE),
                            height: Val::Px(22.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(SKILL_UPGRADE_IDLE_COLOR),
                        SkillUpgradeButton { slot: i },
                        Name::new(format!("SkillUpgrade-{label}")),
                    ))
                    .with_children(|arrow| {
                        arrow.spawn((
                            Text::new("+  UPGRADE"),
                            TextFont {
                                font_size: 11.0,
                                ..default()
                            },
                            TextColor::WHITE,
                        ));
                    });

                    col.spawn((
                        Button,
                        Node {
                            width: Val::Px(SKILL_SLOT_SIZE),
                            height: Val::Px(112.0),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(8.0)),
                            padding: UiRect::all(Val::Px(4.0)),
                            flex_direction: FlexDirection::Column,
                            justify_content: JustifyContent::FlexEnd,
                            align_items: AlignItems::Center,
                            row_gap: Val::Px(1.0),
                            ..default()
                        },
                        BackgroundColor(SKILL_BUTTON_COLOR),
                        BorderColor::all(crate::ui_theme::EDGE),
                        SkillBarSlot { slot: i },
                        Name::new(format!("SkillSlot-{label}")),
                    ))
                    .with_children(|slot| {
                        // The atlas is presentation only; shortcuts and status remain
                        // live text below it, including while the art is loading.
                        slot.spawn((
                            ImageNode {
                                image: skill_atlas.clone().unwrap_or_default(),
                                ..default()
                            },
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px((SKILL_SLOT_SIZE - 48.0) * 0.5 - 1.0),
                                top: Val::Px(1.0),
                                width: Val::Px(48.0),
                                height: Val::Px(48.0),
                                border_radius: BorderRadius::all(Val::Px(7.0)),
                                display: Display::None,
                                ..default()
                            },
                            bevy::ui::FocusPolicy::Pass,
                            DesktopSkillIcon { slot: i },
                            Name::new(format!("SkillIcon-{label}")),
                        ));
                        slot.spawn((
                            Text::new(label),
                            TextLayout::new_with_justify(Justify::Center),
                            TextFont {
                                font_size: 14.0,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.01, 0.02, 0.03, 0.78)),
                            Node {
                                position_type: PositionType::Absolute,
                                top: Val::Px(5.0),
                                left: Val::Px(5.0),
                                padding: UiRect::horizontal(Val::Px(3.0)),
                                border_radius: BorderRadius::all(Val::Px(3.0)),
                                ..default()
                            },
                            TextColor::WHITE,
                        ));
                        slot.spawn((
                            Text::new(""),
                            TextFont {
                                font_size: 11.0,
                                ..default()
                            },
                            TextColor(Color::srgba(0.88, 0.90, 0.94, 1.0)),
                            TextLayout::new_with_justify(Justify::Center),
                            Node {
                                width: Val::Percent(100.0),
                                ..default()
                            },
                            SkillNameLabel { slot: i },
                            Name::new(format!("SkillName-{label}")),
                        ));
                        slot.spawn((
                            Text::new("Lv 1"),
                            TextFont {
                                font_size: 11.0,
                                ..default()
                            },
                            TextColor(Color::srgba(0.82, 0.84, 0.90, 1.0)),
                            TextLayout::new_with_justify(Justify::Center),
                            Node {
                                width: Val::Percent(100.0),
                                ..default()
                            },
                            SkillRankLabel { slot: i },
                            Name::new(format!("SkillRank-{label}")),
                        ));
                    });
                });
            }
        });
}

/// Reflect the selected class kit + server ranks on the hotbar and light the
/// upgrade arrows when the local player has a skill point to spend and the
/// slot is below the shared max rank.
pub(super) fn update_skill_bar_system(
    progression: Query<
        (
            &PlayerProgression,
            Option<&NetworkHeroClass>,
            &CombatStats,
            &Transform,
        ),
        With<Player>,
    >,
    cooldowns: Res<LocalCastCooldown>,
    pending: Res<PendingCast>,
    target: Res<TargetState>,
    targets: Query<&Transform, Without<Player>>,
    team_selection: Res<TeamSelection>,
    mut rank_labels: Query<(&SkillRankLabel, &mut Text), Without<SkillNameLabel>>,
    mut name_labels: Query<(&SkillNameLabel, &mut Text), Without<SkillRankLabel>>,
    images: Option<Res<Assets<Image>>>,
    mut icons: Query<(&DesktopSkillIcon, &mut ImageNode, &mut Node), Without<SkillUpgradeButton>>,
    mut upgrade_buttons: Query<
        (
            &SkillUpgradeButton,
            &Interaction,
            &mut BackgroundColor,
            &mut Node,
        ),
        With<Button>,
    >,
) {
    let local = progression.iter().next();
    let prog = local.map(|(prog, ..)| *prog).unwrap_or_default();
    let class = local_hero_class(local.map(|(_, class, ..)| class), &team_selection);

    for (icon, mut image, mut node) in &mut icons {
        let Some(slot) = SkillSlot::from_index(icon.slot as u8) else {
            node.display = Display::None;
            continue;
        };
        let definition = ability_for_class_slot(class, slot);
        image.rect = images
            .as_ref()
            .and_then(|images| images.get(&image.image))
            .and_then(|atlas| crate::skill_icons::icon_rect(definition.id, atlas.size().as_vec2()));
        node.display = if image.rect.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        let rank = prog.ranks[icon.slot].max(1);
        let available = prog.unlocked()[icon.slot]
            && cooldowns.remaining_secs[icon.slot] <= 0.0
            && local
                .is_none_or(|(_, _, stats, _)| stats.mana >= scaled_mana_cost(definition, rank));
        image.color = if available {
            Color::WHITE
        } else {
            Color::srgb(0.3, 0.3, 0.3)
        };
    }

    for (label, mut text) in &mut name_labels {
        let Some(slot) = SkillSlot::from_index(label.slot as u8) else {
            continue;
        };
        let next = ability_for_class_slot(class, slot).name;
        if text.0 != next {
            text.0 = next.to_string();
        }
    }

    for (label, mut text) in &mut rank_labels {
        let rank = prog.ranks.get(label.slot).copied().unwrap_or(1).max(1);
        let slot = SkillSlot::from_index(label.slot as u8).expect("hotbar slot");
        let definition = ability_for_class_slot(class, slot);
        let cost = scaled_mana_cost(definition, rank);
        let status = if !prog.unlocked()[label.slot] {
            format!("Locked Lv {}", shared::SLOT_UNLOCK_LEVELS[label.slot])
        } else if cooldowns.remaining_secs[label.slot] > 0.0 {
            format!("{:.1}s", cooldowns.remaining_secs[label.slot])
        } else if local.is_some_and(|(_, _, stats, _)| stats.mana < cost) {
            "Need mana".to_string()
        } else if pending
            .request
            .is_some_and(|request| request.slot == label.slot && request.approach_announced)
        {
            "Approaching".to_string()
        } else if definition.targeting == TargetingMode::UnitTarget
            && target.selected_entity.is_none()
        {
            "Select target".to_string()
        } else if definition.targeting == TargetingMode::UnitTarget
            && local
                .zip(
                    target
                        .selected_entity
                        .and_then(|entity| targets.get(entity).ok()),
                )
                .is_some_and(|((_, _, _, player), target)| {
                    !within_cast_range(
                        player.translation,
                        target.translation,
                        scaled_cast_range(definition, rank),
                    )
                })
        {
            "Out of range".to_string()
        } else {
            "Ready".to_string()
        };
        let next = format!("R{rank} · {cost:.0} MP\n{status}");
        if text.0 != next {
            text.0 = next;
        }
    }

    for (button, interaction, mut color, mut node) in &mut upgrade_buttons {
        let rank = prog.ranks.get(button.slot).copied().unwrap_or(1).max(1);
        let can_upgrade =
            prog.skill_points > 0 && rank < MAX_ABILITY_RANK && prog.unlocked()[button.slot];
        // Arrow only shows when a point can actually be spent on this slot.
        let display = if can_upgrade {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        let next_color = if matches!(interaction, Interaction::Hovered | Interaction::Pressed) {
            SKILL_UPGRADE_HOVER_COLOR
        } else {
            SKILL_UPGRADE_READY_COLOR
        };
        *color = next_color.into();
    }
}

/// Arrow click or the upgrade key spends a point on the matching slot. The server
/// is authoritative: it ignores the request when no point is available.
pub(super) fn skill_upgrade_input_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    progression: Query<&PlayerProgression, With<Player>>,
    upgrade_buttons: Query<
        (&SkillUpgradeButton, &Interaction),
        (Changed<Interaction>, With<Button>),
    >,
    mut command_writer: MessageWriter<NetworkCommand>,
    context: Res<GameplayInputContext>,
) {
    if !context.gameplay_allowed() {
        return;
    }

    let Some(prog) = progression.iter().next() else {
        return;
    };
    let unlocked = prog.unlocked();
    let eligible = |slot: usize| {
        prog.skill_points > 0 && unlocked[slot] && prog.ranks[slot] < MAX_ABILITY_RANK
    };
    if keyboard.just_pressed(SKILL_UPGRADE_KEY) {
        if let Some(slot) = (0..4).find(|slot| eligible(*slot)) {
            command_writer.write(NetworkCommand::UpgradeSkill { slot: slot as u8 });
        }
    }
    for (button, interaction) in &upgrade_buttons {
        if matches!(interaction, Interaction::Pressed) && eligible(button.slot) {
            command_writer.write(NetworkCommand::UpgradeSkill {
                slot: button.slot as u8,
            });
        }
    }
}

pub(super) fn skill_button_system(
    mut interactions: Query<
        (&Interaction, &SkillBarSlot, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>),
    >,
    game_state: Option<Res<GameStateSnapshot>>,
    team_selection: Res<TeamSelection>,
    local_player: Query<
        (
            &CombatStats,
            Option<&PlayerProgression>,
            Option<&NetworkPlayerId>,
            Option<&NetworkHeroClass>,
        ),
        With<Player>,
    >,
    target_state: Res<TargetState>,
    mut pending_cast: ResMut<PendingCast>,
    mut feedback: ResMut<ActionFeedback>,
    context: Res<GameplayInputContext>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
) {
    if !context.gameplay_allowed() || mobile.as_ref().is_some_and(|mobile| mobile.enabled) {
        return;
    }

    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    for (interaction, bar_slot, mut color) in interactions.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *color = SKILL_BUTTON_PRESS_COLOR.into();
                let Ok((_stats, _prog, _net_id, class)) = local_player.single() else {
                    continue;
                };
                let class = local_hero_class(Some(class), &team_selection);
                queue_cast_request(
                    bar_slot.slot,
                    class,
                    &target_state,
                    &mut pending_cast,
                    &mut feedback,
                );
            }
            Interaction::Hovered => {
                *color = SKILL_BUTTON_HOVER_COLOR.into();
            }
            Interaction::None => {
                *color = SKILL_BUTTON_COLOR.into();
            }
        }
    }
}
