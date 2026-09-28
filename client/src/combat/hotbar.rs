// i18n-strict
use crate::domain::CombatStats;
use crate::hud_layout::HudRegion;
use crate::i18n::{Localized, data, tr, trf};
use crate::input_bindings::SKILL_UPGRADE_KEY;
use crate::input_context::GameplayInputContext;
use crate::net::{
    GameState, GameStateSnapshot, NetworkCommand, NetworkHeroClass, NetworkPlayerId,
    PlayerProgression,
};
use crate::player::Player;
use crate::team::TeamSelection;
use crate::ui::{
    Activated, TestId, UiAction,
    kit_assets::Icon,
    theme::{self, Form, TextStyle},
    tokens::{TextRole, border, color, radius, size, space},
    widgets::{
        game::{self, AbilityView},
        surfaces::TOOLTIP_DELAY,
    },
};
use bevy::prelude::*;
use shared::{
    SkillSlot, TargetingMode, ability_for_class_slot, scaled_cast_range, scaled_mana_cost,
};

use super::cast::{PendingCast, queue_cast_request, within_cast_range};
use super::cooldown::{LocalCastCooldown, local_hero_class};
use super::feedback::{ActionFeedback, ActionFeedbackText};
use super::selection::TargetState;
use super::skill_card::{SkillCardView, StatusTone};

/// Ability slot diameter (`size.ability.desktop`).
pub(super) const SKILL_SLOT_SIZE: f32 = size::ABILITY.desktop;
/// Ability bar anatomy (`hud.md` `ability-bar`): first slot at +20 / +14,
/// slots `space.16` apart.
const BAR_INSET: (f32, f32) = (20.0, 14.0);
const SKILL_SLOT_GAP: f32 = space::S16;
/// Upgrade chip (`hud.md` `upgrade-chip`): 24 high.
const CHIP_H: f32 = 24.0;

/// A press on the desktop skill bar: cast the slot, spend a point on it, or
/// (the upgrade chip) spend a point on the first eligible slot, like `U`.
/// The slots are painted by the kit (`ButtonKind::Skill`, `SkillUpgrade`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HotbarAction {
    Cast(usize),
    Upgrade(usize),
    UpgradeNext,
}

#[derive(Component)]
pub(super) struct DesktopSkillIcon {
    pub(super) slot: usize,
}

#[derive(Component)]
pub(super) struct SkillUpgradeButton {
    slot: usize,
}

/// The slot's live status line (`combat.hotbar.rank`): not drawn (hud.md
/// puts no text under the abilities); it carries the status chain the
/// tooltip shows and the QA harness reads.
#[derive(Component)]
pub(super) struct SkillRankLabel {
    pub(super) slot: usize,
}

/// A hotbar slot's ability button (its [`AbilityView`] follows the kit).
#[derive(Component)]
pub(super) struct SkillSlotButton {
    slot: usize,
}

/// Ability-name carrier on a hotbar slot (not drawn; names live in the
/// tooltip); follows the selected class kit.
#[derive(Component)]
pub(super) struct SkillNameLabel {
    slot: usize,
}

/// The upgrade chip over the ability bar (shown while a point can be spent).
#[derive(Component)]
pub(super) struct SkillUpgradeChip;

/// The desktop ability tooltip (`skill-description.md`).
#[derive(Component)]
pub(super) struct SkillTooltip;

pub(super) fn setup_combat_ui(
    mut commands: Commands,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
) {
    let phone = mobile.as_ref().is_some_and(|mobile| mobile.enabled);
    // Action feedback (`hud.md` `action-feedback`): a glass toast, max two
    // lines, bottom-centred over the ability bar (phone: under the target).
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            max_width: Val::Px(if phone {
                crate::hud_layout::plate::FEEDBACK_PHONE.x
            } else {
                crate::hud_layout::plate::FEEDBACK_DESKTOP.x
            }),
            padding: UiRect::axes(Val::Px(space::S12), Val::Px(space::S8)),
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            display: Display::None,
            ..default()
        },
        Text::new(""),
        theme::styled_text(TextStyle::keep_case(TextRole::Caption)),
        TextColor(color::TEXT_PRIMARY),
        TextLayout::new_with_justify(Justify::Center),
        BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
        BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)),
        UiTransform::IDENTITY,
        HudRegion::ActionFeedback,
        ZIndex(14),
        Pickable::IGNORE,
        ActionFeedbackText,
        Name::new("ActionFeedback"),
    ));
    commands
        .spawn((
            // A Button keeps world clicks off the plate between the slots.
            Button,
            game::hud_plate(false),
            HudRegion::AbilityBar,
            ZIndex(12),
            Name::new("SkillBarRoot"),
        ))
        .insert(Node {
            position_type: PositionType::Absolute,
            padding: UiRect::new(
                Val::Px(BAR_INSET.0),
                Val::Px(0.0),
                Val::Px(BAR_INSET.1),
                Val::Px(0.0),
            ),
            column_gap: Val::Px(SKILL_SLOT_GAP),
            align_items: AlignItems::FlexStart,
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            ..default()
        })
        .with_children(|row| {
            for i in 0..4 {
                let label = crate::input_bindings::SKILL_SLOT_KEY_LABELS[i];
                row.spawn((
                    Node {
                        width: Val::Px(SKILL_SLOT_SIZE),
                        height: Val::Px(SKILL_SLOT_SIZE),
                        ..default()
                    },
                    Name::new(format!("SkillColumn-{label}")),
                ))
                .with_children(|col| {
                    let view = AbilityView {
                        ability: None,
                        icon: Icon::HudAttack,
                        key: Some(label),
                        cost: Some(0),
                        rank: 1,
                        cooldown: None,
                        locked: false,
                        unlock_level: Some(shared::SLOT_UNLOCK_LEVELS[i] as u8),
                        no_mana: false,
                        pips: true,
                        ring: false,
                    };
                    let slot = game::ability_button(
                        col,
                        view,
                        SKILL_SLOT_SIZE,
                        HotbarAction::Cast(i),
                        format!("SkillSlot-{label}"),
                    );
                    let mut commands = col.commands();
                    commands.entity(slot).insert(SkillSlotButton { slot: i });
                    commands.entity(slot).with_children(|button| {
                        // Level-up +: a separate SkillUpgrade button.
                        let upgrade = game::ability_upgrade(
                            button,
                            SKILL_SLOT_SIZE,
                            HotbarAction::Upgrade(i),
                            format!("SkillUpgrade-{label}"),
                        );
                        button
                            .commands()
                            .entity(upgrade)
                            .insert(SkillUpgradeButton { slot: i });
                    });
                    // Status carriers (not drawn, see `SkillRankLabel`).
                    col.spawn((
                        Text::new(""),
                        Node {
                            display: Display::None,
                            ..default()
                        },
                        SkillNameLabel { slot: i },
                        Name::new(format!("SkillName-{label}")),
                    ));
                    col.spawn((
                        Text::new(trf("combat.hotbar.level", &[("level", &1)])),
                        Node {
                            display: Display::None,
                            ..default()
                        },
                        SkillRankLabel { slot: i },
                        Name::new(format!("SkillRank-{label}")),
                    ));
                });
            }
        });
    // Upgrade chip: `combat.hotbar.upgrade` + keycap U, gold badge.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                height: Val::Px(CHIP_H),
                padding: UiRect::new(
                    Val::Px(space::S12),
                    Val::Px(space::S4),
                    Val::Px(0.0),
                    Val::Px(0.0),
                ),
                column_gap: Val::Px(space::S8),
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(color::GOLD_500),
            UiTransform::IDENTITY,
            UiAction(HotbarAction::UpgradeNext),
            TestId::new("SkillUpgradeChip"),
            HudRegion::UpgradeChip,
            SkillUpgradeChip,
            ZIndex(13),
            Name::new("SkillUpgradeChip"),
        ))
        .with_children(|chip| {
            chip.spawn((
                Localized::new("combat.hotbar.upgrade").into_text(),
                theme::role_text(TextRole::Eyebrow),
                TextColor(color::TEXT_ON_GOLD),
                TextLayout::new_with_no_wrap(),
            ));
            crate::ui::widgets::surfaces::keycap(
                chip,
                crate::input_bindings::upgrade_key_display(),
            );
        });
    if !phone {
        super::skill_card::spawn_skill_card(
            &mut commands,
            Form::Desktop,
            (SkillTooltip, Name::new("SkillTooltip")),
        );
    }
}

/// The live status of a slot (the `hotbar.rs` status chain of
/// `skill-description.md` § Content), with its tone.
#[allow(clippy::too_many_arguments)]
fn slot_status(
    slot: usize,
    prog: &PlayerProgression,
    cooldowns: &LocalCastCooldown,
    stats: Option<&CombatStats>,
    cost: f32,
    pending: &PendingCast,
    definition: &shared::AbilityDefinition,
    out_of_range: bool,
    has_target: bool,
) -> (String, StatusTone) {
    if !prog.unlocked()[slot] {
        (
            trf(
                "combat.hotbar.locked",
                &[("level", &shared::SLOT_UNLOCK_LEVELS[slot])],
            ),
            StatusTone::Muted,
        )
    } else if cooldowns.remaining_secs[slot] > 0.0 {
        (
            trf(
                "combat.hotbar.cooldown",
                &[("seconds", &format!("{:.1}", cooldowns.remaining_secs[slot]))],
            ),
            StatusTone::Secondary,
        )
    } else if stats.is_some_and(|stats| stats.mana < cost) {
        (
            tr("combat.hotbar.need_mana").to_string(),
            StatusTone::Danger,
        )
    } else if pending
        .request
        .is_some_and(|request| request.slot == slot && request.approach_announced)
    {
        (
            tr("combat.hotbar.approaching").to_string(),
            StatusTone::Warning,
        )
    } else if definition.targeting == TargetingMode::UnitTarget && !has_target {
        (
            tr("combat.hotbar.select_target").to_string(),
            StatusTone::Warning,
        )
    } else if definition.targeting == TargetingMode::UnitTarget && out_of_range {
        (
            tr("combat.hotbar.out_of_range").to_string(),
            StatusTone::Warning,
        )
    } else {
        (tr("combat.hotbar.ready").to_string(), StatusTone::Ready)
    }
}

/// Reflect the selected class kit + server ranks on the hotbar: each slot's
/// ability view (art, cost, rank pips, cooldown, lock, mana), the + badges
/// and the upgrade chip while a point can be spent, and the status carriers.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
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
        (&SkillUpgradeButton, &mut Node),
        (Without<DesktopSkillIcon>, Without<SkillUpgradeChip>),
    >,
    mut slots: Query<(&SkillSlotButton, &mut AbilityView)>,
    mut chips: Query<
        &mut Node,
        (
            With<SkillUpgradeChip>,
            Without<DesktopSkillIcon>,
            Without<SkillUpgradeButton>,
        ),
    >,
) {
    let local = progression.iter().next();
    let prog = local.map(|(prog, ..)| *prog).unwrap_or_default();
    let class = local_hero_class(local.map(|(_, class, ..)| class), &team_selection);
    let stats = local.map(|(_, _, stats, _)| stats);

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
        let next = data::ability_name(ability_for_class_slot(class, slot));
        if text.0 != next {
            text.0 = next.to_string();
        }
    }

    let has_target = target.selected_entity.is_some();
    let out_of_range = |definition: &shared::AbilityDefinition, rank: u8| {
        local
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
    };
    for (label, mut text) in &mut rank_labels {
        let rank = prog.ranks.get(label.slot).copied().unwrap_or(1).max(1);
        let slot = SkillSlot::from_index(label.slot as u8).expect("hotbar slot");
        let definition = ability_for_class_slot(class, slot);
        let cost = scaled_mana_cost(definition, rank);
        let (status, _) = slot_status(
            label.slot,
            &prog,
            &cooldowns,
            stats,
            cost,
            &pending,
            definition,
            out_of_range(definition, rank),
            has_target,
        );
        let next = trf(
            "combat.hotbar.rank",
            &[
                ("rank", &rank),
                ("cost", &format!("{cost:.0}")),
                ("status", &status),
            ],
        );
        if text.0 != next {
            text.0 = next;
        }
    }

    for (button, mut view) in &mut slots {
        let rank = prog.ranks[button.slot].max(1);
        let definition = ability_for_class_slot(class, SkillSlot::ALL[button.slot]);
        let cost = scaled_mana_cost(definition, rank);
        let remaining = cooldowns.remaining_secs[button.slot];
        let unlocked = prog.unlocked()[button.slot];
        // hud.md § States, Dead: every ability is veiled (no lock, no level).
        let dead = stats.is_some_and(|stats| !stats.is_alive());
        let next = AbilityView {
            ability: Some(definition.id),
            cost: Some(cost.round() as u32),
            // A locked slot has learned nothing yet (the redline's empty pips).
            rank: if unlocked { prog.ranks[button.slot] } else { 0 },
            cooldown: (remaining > 0.0)
                .then(|| (remaining, cooldowns.total_secs[button.slot].max(remaining))),
            locked: !unlocked || dead,
            unlock_level: (!unlocked && !dead)
                .then_some(shared::SLOT_UNLOCK_LEVELS[button.slot] as u8),
            no_mana: stats.is_some_and(|stats| stats.mana < cost),
            ..view.clone()
        };
        if *view != next {
            *view = next;
        }
    }

    let mut any = false;
    for (button, mut node) in &mut upgrade_buttons {
        // The + only shows when a point can actually be spent on this slot.
        let can_upgrade = upgrade_eligible(&prog, button.slot);
        any |= can_upgrade;
        let display = if can_upgrade {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    let any = any || (0..4).any(|slot| upgrade_eligible(&prog, slot));
    for mut node in &mut chips {
        let display = if any { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
}

/// The desktop ability tooltip: shown after the tooltip delay on a hovered
/// slot (moving between slots swaps it at once), live while open, hidden
/// when the pointer leaves, a cast starts or play is gated; a controller
/// never focuses the bar, so it never shows one.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn update_skill_tooltip(
    time: Res<Time>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    ui_scale: Option<Res<UiScale>>,
    context: Res<GameplayInputContext>,
    gamepad: Option<Res<crate::gamepad::GamepadControls>>,
    local: Query<
        (
            &PlayerProgression,
            Option<&NetworkHeroClass>,
            &CombatStats,
            &Transform,
            Option<&crate::net::PlayerEquipment>,
        ),
        With<Player>,
    >,
    state: (
        Res<TeamSelection>,
        Res<LocalCastCooldown>,
        Res<PendingCast>,
        Res<TargetState>,
        Option<Res<GameStateSnapshot>>,
    ),
    targets: Query<&Transform, Without<Player>>,
    slots: Query<(
        &SkillSlotButton,
        &Interaction,
        &ComputedNode,
        &UiGlobalTransform,
        &AbilityView,
    )>,
    chips: Query<&Node, (With<SkillUpgradeChip>, Without<SkillTooltip>)>,
    keys: Query<(&SkillSlotButton, &crate::ui::widgets::game::AbilityParts)>,
    key_texts: Query<&Text>,
    mut tooltips: Query<(&mut SkillCardView, &mut Node), With<SkillTooltip>>,
    mut hovered: Local<Option<(usize, f32)>>,
) {
    let (team_selection, cooldowns, pending, target, game) = state;
    let Ok((mut view, mut node)) = tooltips.single_mut() else {
        return;
    };
    let now = time.elapsed_secs();
    let pointer = slots
        .iter()
        .find(|(_, interaction, ..)| **interaction != Interaction::None)
        .map(|(slot, ..)| slot.slot);
    let pad = gamepad.as_ref().is_some_and(|pad| pad.active);
    let casting = pending.request.is_some();
    *hovered = match (pointer, *hovered) {
        (Some(slot), Some((was, since))) if was == slot => Some((slot, since)),
        // Moving between slots while one is shown swaps without delay.
        (Some(slot), Some(_)) if view.visible => Some((slot, now - TOOLTIP_DELAY.as_secs_f32())),
        (Some(slot), _) => Some((slot, now)),
        (None, _) => None,
    };
    let shown = hovered
        .filter(|(_, since)| now - since >= TOOLTIP_DELAY.as_secs_f32())
        .map(|(slot, _)| slot)
        .filter(|_| !pad && !casting && context.gameplay_allowed());
    let Some(slot) = shown else {
        if view.visible {
            view.visible = false;
        }
        return;
    };
    let Ok((prog, class, stats, position, equipment)) = local.single() else {
        return;
    };
    let class = local_hero_class(Some(class), &team_selection);
    let sandbox = game.as_ref().and_then(|game| game.sandbox.as_ref());
    let rank = prog.ranks[slot].max(1);
    let duration = if sandbox.is_some_and(|s| s.config.player.no_cooldowns) {
        0.0
    } else {
        super::effective_cast_duration(
            class,
            prog.level,
            rank,
            SkillSlot::ALL[slot],
            equipment.map_or_else(Default::default, |equipment| equipment.item_bonuses),
            sandbox.is_some(),
        )
    };
    let mut next = SkillCardView::of(class, prog, slot, stats.mana, duration);
    let definition = ability_for_class_slot(class, SkillSlot::ALL[slot]);
    let out_of_range = target
        .selected_entity
        .and_then(|entity| targets.get(entity).ok())
        .is_some_and(|target| {
            !within_cast_range(
                position.translation,
                target.translation,
                scaled_cast_range(definition, rank),
            )
        });
    // Dead: read-only card, no status line.
    next.status = stats.is_alive().then(|| {
        slot_status(
            slot,
            prog,
            &cooldowns,
            Some(stats),
            scaled_mana_cost(definition, rank),
            &pending,
            definition,
            out_of_range,
            target.selected_entity.is_some(),
        )
    });
    next.key = keys
        .iter()
        .find(|(button, _)| button.slot == slot)
        .and_then(|(_, parts)| parts.key)
        .and_then(|key| key_texts.get(key).ok())
        .map(|text| text.0.clone());
    if *view != next {
        *view = next;
    }
    // Centred on the slot, clamped to the window; bottom `space.8` above
    // the upgrade chip (or the bar plate without it).
    let Ok(window) = windows.single() else { return };
    let viewport = crate::hud_layout::ui_viewport(window, ui_scale.as_deref());
    let layout = crate::hud_layout::HudLayout::desktop(viewport, false);
    let Some((_, _, computed, transform, _)) =
        slots.iter().find(|(button, ..)| button.slot == slot)
    else {
        return;
    };
    let rect = crate::ui::focus::node_rect(computed, transform);
    let centre = rect.center().x * computed.inverse_scale_factor();
    let chip = chips.iter().any(|chip| chip.display != Display::None);
    let card = super::skill_card::CARD;
    let left = (centre - card.x * 0.5).clamp(
        space::S16,
        (viewport.x - space::S16 - card.x).max(space::S16),
    );
    let top = layout.tooltip_bottom(chip) - card.y;
    if node.left != Val::Px(left) {
        node.left = Val::Px(left);
    }
    if node.top != Val::Px(top) {
        node.top = Val::Px(top);
    }
}

/// Shows the controller's skill bindings (PS or generic names) in the key
/// badges while it owns input and the keyboard keys otherwise.
pub(super) fn sync_skill_key_labels(
    gamepad: Option<Res<crate::gamepad::GamepadControls>>,
    slots: Query<(&SkillSlotButton, &crate::ui::widgets::game::AbilityParts)>,
    mut texts: Query<&mut Text>,
) {
    let keys = match gamepad.as_ref().filter(|pad| pad.active) {
        Some(pad) => crate::gamepad::legend::skill_labels(pad.playstation),
        None => crate::input_bindings::SKILL_SLOT_KEY_LABELS,
    };
    for (slot, parts) in &slots {
        if let Some(Ok(mut text)) = parts.key.map(|key| texts.get_mut(key)) {
            if text.0 != keys[slot.slot] {
                text.0 = keys[slot.slot].to_owned();
            }
        }
    }
}

/// A skill point can be spent on `slot` (0..4): the one rule the arrow, the
/// upgrade key and a controller's North + skill share.
pub(crate) fn upgrade_eligible(prog: &PlayerProgression, slot: usize) -> bool {
    prog.skill_points > 0 && prog.unlocked()[slot] && prog.ranks[slot] < shared::MAX_ABILITY_RANK
}

/// Arrow click or the upgrade key spends a point on the matching slot. The server
/// is authoritative: it ignores the request when no point is available.
pub(super) fn skill_upgrade_input_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    progression: Query<&PlayerProgression, With<Player>>,
    mut activated: MessageReader<Activated<HotbarAction>>,
    mut command_writer: MessageWriter<NetworkCommand>,
    context: Res<GameplayInputContext>,
) {
    // Read every press, so one made while gameplay is gated cannot fire later.
    let presses: Vec<Option<usize>> = activated
        .read()
        .filter_map(|activated| match activated.action {
            HotbarAction::Upgrade(slot) => Some(Some(slot)),
            // The chip spends a point like the upgrade key.
            HotbarAction::UpgradeNext => Some(None),
            HotbarAction::Cast(_) => None,
        })
        .collect();
    if !context.gameplay_allowed() {
        return;
    }

    let Some(prog) = progression.iter().next() else {
        return;
    };
    let eligible = |slot: usize| upgrade_eligible(prog, slot);
    let first = || (0..4).find(|slot| eligible(*slot));
    if keyboard.just_pressed(SKILL_UPGRADE_KEY) {
        if let Some(slot) = first() {
            command_writer.write(NetworkCommand::UpgradeSkill { slot: slot as u8 });
        }
    }
    for press in presses {
        match press {
            Some(slot) if eligible(slot) => {
                command_writer.write(NetworkCommand::UpgradeSkill { slot: slot as u8 });
            }
            None => {
                if let Some(slot) = first() {
                    command_writer.write(NetworkCommand::UpgradeSkill { slot: slot as u8 });
                }
            }
            Some(_) => {}
        }
    }
}

pub(super) fn skill_button_system(
    mut activated: MessageReader<Activated<HotbarAction>>,
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
    let casts: Vec<usize> = activated
        .read()
        .filter_map(|activated| match activated.action {
            HotbarAction::Cast(slot) => Some(slot),
            HotbarAction::Upgrade(_) | HotbarAction::UpgradeNext => None,
        })
        .collect();
    if !context.gameplay_allowed() || mobile.as_ref().is_some_and(|mobile| mobile.enabled) {
        return;
    }

    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    for slot in casts {
        let Ok((_stats, _prog, _net_id, class)) = local_player.single() else {
            continue;
        };
        let class = local_hero_class(Some(class), &team_selection);
        queue_cast_request(slot, class, &target_state, &mut pending_cast, &mut feedback);
    }
}
