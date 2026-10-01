//! Presentation and input helpers for resolved skills. No hit or resource authority.
// i18n-strict
use bevy::gizmos::config::GizmoConfigGroup;
use bevy::prelude::*;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub(super) struct SkillAimGizmos;
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(super) struct SkillEffectGizmos;

#[derive(Resource, Default)]
pub(super) struct SkillAimVector(pub Option<(Vec2, Vec2)>);

use shared::loadout::{EffectVisualKind, WeaponMode};
use shared::{HeroClass, TargetingMode};

use crate::i18n::{data, tr, trf};
use crate::net::{GameStateSnapshot, NetworkHeroClass, PlayerLoadout};
use crate::player::Player;
use crate::sprite::PlayerVisualMode;

pub(crate) fn bounded_aim(origin: Vec2, aim: Vec2, targeting: TargetingMode, range: f32) -> Vec2 {
    match targeting {
        TargetingMode::Point => origin + (aim - origin).clamp_length_max(range),
        TargetingMode::SelfTarget => origin,
        _ => aim,
    }
}

pub(crate) fn attack_range(class: HeroClass, loadout: Option<&PlayerLoadout>) -> f32 {
    loadout
        .and_then(|s| s.0.as_ref())
        .filter(|s| s.basic_attack_range > 0.0)
        .map_or_else(
            || shared::basic_attack_for_class(class).range,
            |s| s.basic_attack_range,
        )
}

pub(crate) fn movement_factor(loadout: Option<&PlayerLoadout>) -> f32 {
    loadout.and_then(|s| s.0.as_ref()).map_or(1.0, |s| {
        if s.root_remaining_secs > 0.0 {
            0.0
        } else {
            s.movement_multiplier
        }
    })
}

#[derive(Component)]
pub(super) struct StandardStatus;

pub(super) fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands
        .spawn((
            Button,
            InteractButton,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(24.0),
                bottom: Val::Px(230.0),
                padding: UiRect::all(Val::Px(12.0)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(crate::ui::theme::BACKDROP),
            Name::new("LanternInteract"),
        ))
        .with_child((
            Text::new(tr("combat.standard.interact")),
            TextFont {
                font_size: 16.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
    commands.spawn((
        Text::default(),
        TextFont {
            font: assets.load("ui/Inter.ttf"),
            font_size: 14.0,
            ..default()
        },
        TextColor(crate::ui::tokens::color::TEXT_PRIMARY),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(18.0),
            bottom: Val::Px(158.0),
            max_width: Val::Px(440.0),
            display: Display::None,
            ..default()
        },
        BackgroundColor(crate::ui::theme::BACKDROP),
        StandardStatus,
        Name::new("StandardKitStatus"),
    ));
}

pub(super) fn update_status(
    inspection: Res<super::inspection::SkillInspection>,
    local: Query<(&NetworkHeroClass, &PlayerLoadout), With<Player>>,
    mut label: Query<(&mut Text, &mut Node), With<StandardStatus>>,
    mobile: Res<crate::mobile_controls::MobileControls>,
    pad: Res<crate::gamepad::GamepadControls>,
) {
    let Ok((mut text, mut node)) = label.single_mut() else {
        return;
    };
    if inspection.slot.is_none() {
        node.display = Display::None;
        return;
    }
    let Ok((class, loadout)) = local.single() else {
        node.display = Display::None;
        return;
    };
    let Some(state) = loadout.0.as_ref().filter(|s| s.recipe.is_some()) else {
        node.display = Display::None;
        return;
    };
    node.display = Display::Flex;
    let passive = match state.recipe.as_ref().unwrap().passive {
        shared::loadout::PassiveId::Radiance => tr("combat.standard.radiance"),
        shared::loadout::PassiveId::Momentum => tr("combat.standard.momentum"),
        shared::loadout::PassiveId::Tempered => tr("combat.standard.tempered"),
        shared::loadout::PassiveId::Vitals => tr("combat.standard.vitals"),
        shared::loadout::PassiveId::Flow => tr("combat.standard.flow"),
        shared::loadout::PassiveId::Shroud => tr("combat.standard.shroud"),
        shared::loadout::PassiveId::Essence => tr("combat.standard.essence"),
        shared::loadout::PassiveId::Clockwork => tr("combat.standard.clockwork"),
        shared::loadout::PassiveId::Resonance => tr("combat.standard.resonance"),
        shared::loadout::PassiveId::Souls => tr("combat.standard.souls_passive"),
        shared::loadout::PassiveId::Concussion => tr("combat.standard.concussion_passive"),
    };
    let mut lines = vec![format!("{} · {}", data::hero_name(class.0), passive)];
    if !mobile.enabled && !pad.active {
        lines.push(tr("combat.standard.aim_hint").into());
    }
    if class.0 == HeroClass::Wildspark {
        lines.push(trf(
            "combat.standard.weapon",
            &[
                (
                    "mode",
                    &tr(match state.weapon_mode {
                        WeaponMode::Repeater => "combat.standard.repeater",
                        WeaponMode::Rockets => "combat.standard.rockets",
                    }),
                ),
                ("stacks", &state.passive_stacks),
                ("cost", &format!("{:.0}", state.basic_attack_mana_cost)),
            ],
        ));
    }
    if state.forge_remaining_secs > 0.0 {
        lines.push(trf(
            "combat.standard.forging",
            &[("seconds", &format!("{:.1}", state.forge_remaining_secs))],
        ));
    }
    if state.forge_ready {
        lines.push(tr("combat.standard.forge_ready").into());
    }
    if state.concussion_stacks > 0 {
        lines.push(trf(
            "combat.standard.concussion",
            &[("count", &state.concussion_stacks)],
        ));
    }
    if state.brittle {
        lines.push(tr("combat.standard.brittle").into());
    }
    if state.energy {
        lines.push(tr("combat.standard.energy").into());
    }
    if state.camouflaged {
        lines.push(tr("combat.standard.camouflaged").into());
    }
    if state.parrying {
        lines.push(tr("combat.standard.parrying").into());
    }
    if state.forged {
        lines.push(tr("combat.standard.forged").into());
    }
    if state.souls > 0 {
        lines.push(trf("combat.standard.souls", &[("count", &state.souls)]));
    }
    if state.shield_hp > 0.0 {
        lines.push(trf(
            "combat.standard.shield",
            &[("value", &format!("{:.0}", state.shield_hp))],
        ));
    }
    if state.root_remaining_secs > 0.0 {
        lines.push(tr("combat.standard.rooted").into());
    }
    if state.passive_remaining_secs > 0.0 {
        lines.push(trf(
            "combat.standard.acceleration",
            &[("seconds", &format!("{:.1}", state.passive_remaining_secs))],
        ));
    }
    for (i, slot) in state.slots.iter().enumerate().filter(|(_, s)| s.can_recast) {
        lines.push(trf(
            "combat.standard.recast",
            &[
                ("key", &["Q", "W", "E", "R"][i]),
                ("seconds", &format!("{:.1}", slot.recast_remaining_secs)),
            ],
        ));
    }
    text.0 = lines.join("\n");
}

fn point(p: Vec2, mode: PlayerVisualMode, map: Option<&crate::maps::MapLayout>) -> Vec3 {
    let world = Vec3::new(
        p.x,
        map.map_or(0.08, |m| m.terrain_height_3d(p.x, p.y) + 0.12),
        p.y,
    );
    if mode == PlayerVisualMode::Sprite2d {
        crate::world2d::simulation_xz_to_render_xy(world).extend(crate::world2d::layer::VFX)
    } else {
        world
    }
}

fn ground_line<G: GizmoConfigGroup>(
    gizmos: &mut Gizmos<G>,
    a: Vec2,
    b: Vec2,
    mode: PlayerVisualMode,
    map: Option<&crate::maps::MapLayout>,
    color: Color,
) {
    let steps = (a.distance(b) / 1.5).ceil().clamp(1.0, 192.0) as usize;
    gizmos.linestrip(
        (0..=steps).map(|i| point(a.lerp(b, i as f32 / steps as f32), mode, map)),
        color,
    );
}

fn ring<G: GizmoConfigGroup>(
    gizmos: &mut Gizmos<G>,
    p: Vec2,
    radius: f32,
    mode: PlayerVisualMode,
    map: Option<&crate::maps::MapLayout>,
    color: Color,
) {
    gizmos.linestrip(
        (0..=48).map(|i| {
            let angle = i as f32 * std::f32::consts::TAU / 48.0;
            point(p + Vec2::new(angle.cos(), angle.sin()) * radius, mode, map)
        }),
        color,
    );
}

/// Authored aim geometry while a key, touch drag or controller button is held.
pub(super) fn draw_aim(
    mut gizmos: Gizmos<SkillAimGizmos>,
    mut vector: ResMut<SkillAimVector>,
    local: Query<
        (
            &Transform,
            &NetworkHeroClass,
            &crate::net::PlayerProgression,
            &super::CombatStats,
        ),
        With<Player>,
    >,
    camera: Query<(&Camera, &GlobalTransform), With<crate::camera::MainCamera>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mode: Res<PlayerVisualMode>,
    map: Option<Res<crate::maps::MapLayout>>,
    context: Res<crate::input_context::GameplayInputContext>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    pad: Option<Res<crate::gamepad::GamepadControls>>,
) {
    vector.0 = None;
    if !context.gameplay_allowed() {
        return;
    }
    let Ok((pose, class, progression, stats)) = local.single() else {
        return;
    };
    if !stats.is_alive() {
        return;
    }
    let Some(preset) = shared::loadout::preset_for_class(class.0) else {
        return;
    };
    let touch = mobile
        .as_ref()
        .filter(|m| m.enabled)
        .and_then(|m| m.aimed_skill());
    let controller = pad.as_ref().filter(|p| p.active);
    let slot = touch
        .map(|t| t.slot)
        .or_else(|| controller.and_then(|p| p.aiming_slot))
        .or_else(|| {
            crate::input_bindings::SKILL_CAST_KEYS
                .iter()
                .position(|k| keyboard.pressed(*k))
        });
    let Some(slot) = slot.filter(|i| *i < 4) else {
        return;
    };
    let def = preset.skill(shared::SkillSlot::ALL[slot]);
    if def.ability.targeting == TargetingMode::SelfTarget {
        return;
    }
    let origin = pose.translation.xz();
    let range = shared::scaled_cast_range(&def.ability, progression.ranks[slot].max(1));
    let aim = if touch.is_some() || controller.is_some() {
        let screen = touch
            .and_then(|t| t.aim)
            .or_else(|| controller.and_then(|p| p.aim));
        let direction = screen
            .and_then(|screen| {
                camera.single().ok().map(|(_, camera)| {
                    crate::player::mobile_screen_direction(screen, camera, *mode).xz()
                })
            })
            .unwrap_or_else(|| pose.forward().xz())
            .normalize_or_zero();
        let extent = touch.map(|t| t.extent).unwrap_or_else(|| {
            controller
                .filter(|p| p.aim.is_some())
                .and_then(|p| p.raw)
                .map_or(1.0, |p| p.right.length().clamp(0.15, 1.0))
        });
        origin + direction * range * extent
    } else {
        let (Ok(window), Ok((camera, camera_pose))) = (windows.single(), camera.single()) else {
            return;
        };
        let Some(cursor) = window.cursor_position() else {
            return;
        };
        let Some(p) =
            crate::player::viewport_to_simulation_world(camera, camera_pose, cursor, *mode, 0.0)
        else {
            return;
        };
        p.xz()
    };
    let aim = bounded_aim(origin, aim, def.ability.targeting, range);
    let direction = (aim - origin).normalize_or_zero();
    let color = Color::linear_rgb(0.015, 0.8, 5.0);
    let map = map.as_deref();
    match def.effect {
        shared::loadout::SkillEffect::RecastZone { radius, .. } => {
            ring(&mut gizmos, aim, radius, *mode, map, color)
        }
        shared::loadout::SkillEffect::TrapLine {
            radius,
            count,
            spacing,
            ..
        } => {
            let side = Vec2::new(-direction.y, direction.x);
            for i in 0..count {
                ring(
                    &mut gizmos,
                    aim + side * (i as f32 - (count - 1) as f32 / 2.0) * spacing,
                    radius,
                    *mode,
                    map,
                    color,
                );
            }
        }
        _ => {
            let radius = match def.effect {
                shared::loadout::SkillEffect::Technique { radius, .. }
                | shared::loadout::SkillEffect::LinearProjectile { radius, .. }
                | shared::loadout::SkillEffect::ReturningShield { radius, .. }
                | shared::loadout::SkillEffect::ImpactRocket { radius, .. } => radius,
                shared::loadout::SkillEffect::Beam { width, .. } => width,
                _ => 0.2,
            };
            let side = Vec2::new(-direction.y, direction.x) * radius;
            for sign in [-1.0, 1.0] {
                ground_line(
                    &mut gizmos,
                    origin + side * sign,
                    origin + direction * range + side * sign,
                    *mode,
                    map,
                    color,
                );
            }
            if direction.length_squared() > 0.5 {
                if range >= 35.0 {
                    vector.0 = Some((origin, origin + direction * range));
                }
                let distance = range.min(14.0);
                let tip = origin + direction * distance;
                let wing = Vec2::new(-direction.y, direction.x) * radius.max(0.5);
                for sign in [-1.0, 1.0] {
                    ground_line(
                        &mut gizmos,
                        tip - direction * 1.5 + wing * sign,
                        tip,
                        *mode,
                        map,
                        color,
                    );
                }
            }
        }
    }
}

/// Bounded, snapshot-driven geometry. Effects do not depend on a visible owner.
pub(super) fn draw_effects(
    mut gizmos: Gizmos<SkillEffectGizmos>,
    game: Option<Res<GameStateSnapshot>>,
    mode: Res<PlayerVisualMode>,
    map: Option<Res<crate::maps::MapLayout>>,
    local: Query<&crate::team::Team, With<Player>>,
    actors: Query<(
        &Transform,
        &PlayerLoadout,
        Option<&NetworkHeroClass>,
        Option<&crate::net::NetworkPlayerId>,
        Option<&crate::team::Team>,
    )>,
) {
    let Some(game) = game else {
        return;
    };
    let map = map.as_deref();
    for e in game
        .skill_effects
        .iter()
        .take(shared::loadout::MAX_ACTIVE_EFFECTS)
    {
        if !e.position.into_iter().chain(e.end).all(f32::is_finite) || !e.radius.is_finite() {
            continue;
        }
        let p = Vec2::from_array(e.position);
        let end = Vec2::from_array(e.end);
        let friendly = local.single().is_ok_and(|t| *t == e.owner_team);
        let color = if e.kind == EffectVisualKind::Trap {
            if friendly {
                Color::linear_rgb(0.015, 0.8, 5.0)
            } else {
                Color::linear_rgb(5.0, 0.16, 0.015)
            }
        } else if friendly {
            Color::srgb(0.35, 0.95, 0.8)
        } else {
            Color::srgb(1.0, 0.3, 0.28)
        };
        let radius = e.radius.clamp(0.05, 256.0);
        match e.kind {
            EffectVisualKind::Cage => {
                for i in 0..5 {
                    if e.consumed_segments & (1 << i) != 0 {
                        continue;
                    }
                    let a = i as f32 * std::f32::consts::TAU / 5.0;
                    let b = (i + 1) as f32 * std::f32::consts::TAU / 5.0;
                    gizmos.line(
                        point(p + Vec2::new(a.cos(), a.sin()) * radius, *mode, map),
                        point(p + Vec2::new(b.cos(), b.sin()) * radius, *mode, map),
                        color,
                    );
                }
            }
            EffectVisualKind::ShieldWall => {
                let d = (end - p).normalize_or_zero();
                let center = p + d;
                let side = Vec2::new(-d.y, d.x) * radius;
                gizmos.line(
                    point(center - side, *mode, map),
                    point(center + side, *mode, map),
                    color,
                );
            }

            EffectVisualKind::Field
            | EffectVisualKind::Trap
            | EffectVisualKind::Healing
            | EffectVisualKind::Anchor
            | EffectVisualKind::Soul
            | EffectVisualKind::Orb
            | EffectVisualKind::Lantern => {
                ring(&mut gizmos, p, radius, *mode, map, color);
                ring(
                    &mut gizmos,
                    p,
                    radius * 0.86,
                    *mode,
                    map,
                    color.with_alpha(0.5),
                );
                if e.kind == EffectVisualKind::Trap {
                    let offset = radius * 0.7;
                    for sign in [-1.0, 1.0] {
                        gizmos.line(
                            point(p + Vec2::new(-offset, sign * offset), *mode, map),
                            point(p + Vec2::new(offset, -sign * offset), *mode, map),
                            if e.armed {
                                Color::WHITE
                            } else {
                                Color::srgb(1.0, 0.75, 0.15)
                            },
                        );
                    }
                }
            }
            EffectVisualKind::BeamWarning | EffectVisualKind::Beam => {
                let d = (end - p).normalize_or_zero();
                let side = Vec2::new(-d.y, d.x) * radius;
                for offset in [-1.0, 1.0] {
                    gizmos.line(
                        point(p + side * offset, *mode, map),
                        point(end + side * offset, *mode, map),
                        color,
                    );
                }
                if e.kind == EffectVisualKind::Beam {
                    for i in -3..=3 {
                        let side = side * (i as f32 / 3.0);
                        gizmos.line(
                            point(p + side, *mode, map),
                            point(end + side, *mode, map),
                            color,
                        );
                    }
                }
            }
            EffectVisualKind::Bolt | EffectVisualKind::Barrier | EffectVisualKind::Rocket => {
                ring(&mut gizmos, p, radius, *mode, map, color);
                let direction = (end - p).normalize_or_zero();
                gizmos.line(
                    point(p - direction * radius * 3.0, *mode, map),
                    point(p + direction * radius, *mode, map),
                    color,
                );
                if e.kind == EffectVisualKind::Barrier {
                    ring(
                        &mut gizmos,
                        p,
                        radius * 1.4,
                        *mode,
                        map,
                        Color::srgb(0.95, 0.85, 0.4),
                    );
                }
            }
        }
    }
    let duelist = actors
        .iter()
        .find(|(_, _, _, id, _)| id.is_some_and(|id| id.0 == game.your_id))
        .and_then(|(_, l, _, _, team)| {
            l.0.as_ref()
                .filter(|s| {
                    s.recipe
                        .as_ref()
                        .is_some_and(|r| r.passive == shared::loadout::PassiveId::Vitals)
                        || s.challenge_target.is_some()
                })
                .map(|s| (s, team))
        });
    for (pose, loadout, class, id, team) in &actors {
        if let (Some((duel, own_team)), Some(id)) = (duelist, id) {
            if team != own_team {
                let p = pose.translation.xz();
                let challenge = duel.challenge_target == Some(id.0);
                for side in 0..4 {
                    if challenge || side == (id.0 as u8).wrapping_add(duel.vital_rotation) % 4 {
                        let a = side as f32 * std::f32::consts::FRAC_PI_2;
                        let color = if challenge && duel.challenge_sides & (1 << side) != 0 {
                            Color::srgb(0.3, 0.35, 0.4)
                        } else {
                            Color::srgb(1.0, 0.75, 0.2)
                        };
                        ring(
                            &mut gizmos,
                            p + Vec2::new(a.cos(), a.sin()) * 1.4,
                            0.3,
                            *mode,
                            map,
                            color,
                        );
                    }
                }
            }
        }
        let Some(state) = &loadout.0 else {
            continue;
        };
        let p = pose.translation.xz();
        if let Some(orb) = state.orb_position {
            let orb = Vec2::from_array(orb);
            ring(
                &mut gizmos,
                orb,
                0.65,
                *mode,
                map,
                Color::srgb(0.9, 0.7, 1.0),
            );
            gizmos.line(
                point(p, *mode, map),
                point(orb, *mode, map),
                Color::srgba(0.8, 0.7, 1.0, 0.3),
            );
        }
        for i in 0..state.concussion_stacks.min(4) {
            ring(
                &mut gizmos,
                p + Vec2::new(-0.6 + i as f32 * 0.4, 1.4),
                0.12,
                *mode,
                map,
                Color::srgb(0.6, 0.9, 1.0),
            );
        }
        if state.brittle {
            ring(&mut gizmos, p, 1.2, *mode, map, Color::srgb(1.0, 0.6, 0.1));
        }
        if state.parrying {
            ring(&mut gizmos, p, 1.4, *mode, map, Color::WHITE);
        }
        if state.shield_hp > 0.0 {
            ring(&mut gizmos, p, 0.95, *mode, map, Color::srgb(0.5, 0.9, 1.0));
        }
        if state.root_remaining_secs > 0.0 {
            ring(&mut gizmos, p, 0.7, *mode, map, Color::srgb(1.0, 0.3, 0.55));
        }
        if state.mark_remaining_secs > 0.0 {
            ring(&mut gizmos, p, 1.15, *mode, map, Color::srgb(1.0, 0.9, 0.3));
        }
        if state.recipe.is_some()
            && let Some(class) = class
        {
            let corners = if class.0 == HeroClass::Dawnweaver {
                4
            } else {
                3
            };
            for i in 0..corners {
                let at = |n: i32| {
                    p + Vec2::from_angle(n as f32 * std::f32::consts::TAU / corners as f32) * 0.55
                };
                gizmos.line(
                    point(at(i), *mode, map),
                    point(at(i + 1), *mode, map),
                    Color::srgb(0.9, 0.8, 0.5),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn point_aim_clamps_but_direction_retains_its_ray() {
        assert_eq!(
            bounded_aim(Vec2::ZERO, Vec2::X * 50.0, TargetingMode::Point, 16.0),
            Vec2::X * 16.0
        );
        assert_eq!(
            bounded_aim(Vec2::ZERO, Vec2::X * 50.0, TargetingMode::Direction, 16.0),
            Vec2::X * 50.0
        );
        assert_eq!(
            bounded_aim(Vec2::ONE, Vec2::ZERO, TargetingMode::SelfTarget, 0.0),
            Vec2::ONE
        );
    }
}

#[derive(Component)]
pub(super) struct InteractButton;

/// One explicit action for keyboard, touch and the controller's left-stick click.
pub(super) fn interact(
    game: Option<Res<GameStateSnapshot>>,
    context: Res<crate::input_context::GameplayInputContext>,
    local: Query<(&Transform, &crate::team::Team, &super::CombatStats), With<Player>>,
    keys: Res<ButtonInput<KeyCode>>,
    pad: Res<crate::gamepad::GamepadControls>,
    mut held: Local<bool>,
    mut buttons: Query<(&mut Node, &Interaction, Ref<Interaction>), With<InteractButton>>,
    mut commands: MessageWriter<crate::net::NetworkCommand>,
) {
    let down = pad.active
        && pad
            .raw
            .is_some_and(|p| p.buttons & crate::gamepad::snapshot::L3 != 0);
    let pressed = down && !*held;
    *held = down;
    let nearest = game.as_ref().and_then(|game| {
        local
            .single()
            .ok()
            .filter(|(_, _, stats)| stats.is_alive() && context.gameplay_allowed())
            .and_then(|(pose, team, _)| {
                game.skill_effects
                    .iter()
                    .filter(|e| {
                        e.owner_team == *team
                            && e.owner_id != game.your_id
                            && matches!(
                                shared::loadout::skill(e.skill).effect,
                                shared::loadout::SkillEffect::Technique {
                                    action: shared::loadout::Technique::Lantern,
                                    ..
                                }
                            )
                            && Vec2::from_array(e.position).distance(pose.translation.xz()) <= 3.0
                    })
                    .min_by_key(|e| e.id)
                    .map(|e| e.id)
            })
    });
    let mut clicked = false;
    for (mut node, interaction, changed) in &mut buttons {
        node.display = if nearest.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        clicked |= changed.is_changed() && *interaction == Interaction::Pressed;
    }
    if let Some(object_id) =
        nearest.filter(|_| keys.just_pressed(KeyCode::KeyF) || pressed || clicked)
    {
        commands.write(crate::net::NetworkCommand::Interact { object_id });
    }
}

/// A pooled, clipped preview on the map shares the exact world aim vector.
#[derive(Component)]
pub(super) struct MinimapAimLine;
pub(super) fn draw_minimap_aim(
    mut commands: Commands,
    vector: Res<SkillAimVector>,
    layout: Res<crate::maps::MapLayout>,
    container: Query<Entity, With<crate::minimap::MinimapContainer>>,
    mut lines: Query<(&mut Node, &mut UiTransform), With<MinimapAimLine>>,
) {
    use crate::minimap::{clip_map_segment, line_node, map_point};
    let segment = vector.0.and_then(|(a, b)| {
        clip_map_segment(
            map_point(*layout, Vec3::new(a.x, 0.0, a.y)),
            map_point(*layout, Vec3::new(b.x, 0.0, b.y)),
        )
    });
    if let Ok((mut node, mut transform)) = lines.single_mut() {
        if let Some((a, b)) = segment {
            let (n, t) = line_node(a, b, 5.0);
            *node = n;
            *transform = t;
        } else {
            node.display = Display::None;
        }
    } else if let (Some((a, b)), Ok(parent)) = (segment, container.single()) {
        let (node, transform) = line_node(a, b, 5.0);
        commands.spawn((
            node,
            transform,
            BackgroundColor(Color::srgb(0.12, 0.65, 1.0)),
            ZIndex(30),
            MinimapAimLine,
            Name::new("MinimapSkillVector"),
            ChildOf(parent),
        ));
    }
}

pub(super) fn draw_minimap_traps(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    layout: Res<crate::maps::MapLayout>,
    container: Query<Entity, With<crate::minimap::MinimapContainer>>,
    local: Query<&crate::team::Team, With<Player>>,
    mut pool: Local<Vec<Entity>>,
) {
    let Ok(parent) = container.single() else {
        return;
    };
    let traps: Vec<_> = game
        .skill_effects
        .iter()
        .filter(|e| e.kind == EffectVisualKind::Trap && e.position.into_iter().all(f32::is_finite))
        .take(32)
        .collect();
    for (i, e) in traps.iter().enumerate() {
        let p = crate::minimap::map_point(*layout, Vec3::new(e.position[0], 0.0, e.position[1]));
        let color = if local.single().is_ok_and(|t| *t == e.owner_team) {
            Color::srgb(0.12, 0.65, 1.0)
        } else {
            Color::srgb(1.0, 0.22, 0.08)
        };
        let node = Node {
            position_type: PositionType::Absolute,
            left: Val::Px(p.x - 4.0),
            top: Val::Px(p.y - 4.0),
            width: Val::Px(8.0),
            height: Val::Px(8.0),
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::MAX,
            ..default()
        };
        let fill = BackgroundColor(if e.armed { color } else { Color::BLACK });
        if let Some(id) = pool.get(i) {
            commands
                .entity(*id)
                .insert((node, fill, BorderColor::all(color)));
        } else {
            pool.push(
                commands
                    .spawn((
                        node,
                        fill,
                        BorderColor::all(color),
                        ZIndex(12),
                        ChildOf(parent),
                        Name::new("MinimapTrap"),
                    ))
                    .id(),
            );
        }
    }
    for id in pool.iter().skip(traps.len()) {
        commands.entity(*id).insert(Node {
            display: Display::None,
            ..default()
        });
    }
}
