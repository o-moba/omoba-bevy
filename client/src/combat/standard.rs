//! Presentation and input helpers for resolved skills. No hit or resource authority.
// i18n-strict
use bevy::prelude::*;
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
    local: Query<(&NetworkHeroClass, &PlayerLoadout), With<Player>>,
    mut label: Query<(&mut Text, &mut Node), With<StandardStatus>>,
    mobile: Res<crate::mobile_controls::MobileControls>,
    pad: Res<crate::gamepad::GamepadControls>,
) {
    let Ok((mut text, mut node)) = label.single_mut() else {
        return;
    };
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

fn ring(
    gizmos: &mut Gizmos,
    p: Vec2,
    radius: f32,
    mode: PlayerVisualMode,
    map: Option<&crate::maps::MapLayout>,
    color: Color,
) {
    let rotation = if mode == PlayerVisualMode::Sprite2d {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
    };
    gizmos.circle(
        Isometry3d::new(point(p, mode, map), rotation),
        radius,
        color,
    );
}

/// Authored aim geometry while a key, touch drag or controller button is held.
pub(super) fn draw_aim(
    mut gizmos: Gizmos,
    local: Query<
        (
            &Transform,
            &NetworkHeroClass,
            &crate::net::PlayerProgression,
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
    if !context.gameplay_allowed() {
        return;
    }
    let Ok((pose, class, progression)) = local.single() else {
        return;
    };
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
    let color = Color::srgb(0.94, 0.84, 0.43);
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
                shared::loadout::SkillEffect::LinearProjectile { radius, .. }
                | shared::loadout::SkillEffect::ReturningShield { radius, .. }
                | shared::loadout::SkillEffect::ImpactRocket { radius, .. } => radius,
                shared::loadout::SkillEffect::Beam { width, .. } => width,
                _ => 0.2,
            };
            let side = Vec2::new(-direction.y, direction.x) * radius;
            for sign in [-1.0, 1.0] {
                gizmos.line(
                    point(origin + side * sign, *mode, map),
                    point(origin + direction * range + side * sign, *mode, map),
                    color,
                );
            }
        }
    }
}

/// Bounded, snapshot-driven geometry. Effects do not depend on a visible owner.
pub(super) fn draw_effects(
    mut gizmos: Gizmos,
    game: Option<Res<GameStateSnapshot>>,
    mode: Res<PlayerVisualMode>,
    map: Option<Res<crate::maps::MapLayout>>,
    local: Query<&crate::team::Team, With<Player>>,
    actors: Query<(&Transform, &PlayerLoadout, Option<&NetworkHeroClass>)>,
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
        let color = if friendly {
            Color::srgb(0.35, 0.95, 0.8)
        } else {
            Color::srgb(1.0, 0.3, 0.28)
        };
        let radius = e.radius.clamp(0.05, 256.0);
        match e.kind {
            EffectVisualKind::Field | EffectVisualKind::Trap => {
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
                            color.with_alpha(if e.armed { 1.0 } else { 0.35 }),
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
    for (pose, loadout, class) in &actors {
        let Some(state) = &loadout.0 else {
            continue;
        };
        let p = pose.translation.xz();
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
