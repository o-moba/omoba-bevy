// i18n-strict
use crate::camera::MainCamera;
use crate::domain::CombatStats;
use crate::input_context::GameplayInputContext;
use crate::net::{NetworkCommand, NetworkHeroClass, PlayerProgression, TargetId, TargetKind};
use crate::player::Player;
use crate::sprite::PlayerVisualMode;
use crate::team::{Team, TeamSelection};
use bevy::prelude::*;
use shared::loadout::Technique;
use shared::{SkillSlot, TargetingMode, scaled_cast_range};

use super::cast::{PendingCast, queue_cast_request};
use super::feedback::ActionFeedback;
use super::selection::{TargetCandidates, TargetState};

/// Utility commands use the same network request IDs and match identity as
/// other combat actions. Cooldowns come only from authoritative snapshots.
pub(super) fn mobile_utility_system(
    mut mobile: Option<ResMut<crate::mobile_controls::MobileControls>>,
    context: Res<GameplayInputContext>,
    local: Query<(&Transform, &CombatStats, &crate::net::PlayerUtility), With<Player>>,
    camera: Query<&GlobalTransform, With<MainCamera>>,
    mode: Res<PlayerVisualMode>,
    mut commands: MessageWriter<NetworkCommand>,
) {
    let Some(mobile) = mobile.as_deref_mut().filter(|mobile| mobile.enabled) else {
        return;
    };
    let intents = std::mem::take(&mut mobile.utilities);
    if !context.gameplay_allowed() || !mobile.focused || !mobile.landscape {
        return;
    }
    let Ok((transform, stats, utility)) = local.single() else {
        return;
    };
    if !stats.is_alive() {
        return;
    }
    for (action, aim) in intents {
        use shared::utility::UtilityAction;
        let remaining = match action {
            UtilityAction::Dash => utility.state.dash_remaining_secs,
            UtilityAction::Haste => utility.state.haste_remaining_secs,
            UtilityAction::Recall | UtilityAction::CancelRecall => 0.0,
        };
        if remaining > 0.0 {
            continue;
        }
        let direction = if action == UtilityAction::Dash {
            let screen = aim
                .or_else(|| (mobile.movement.length_squared() > 0.001).then_some(mobile.movement));
            if let (Some(screen), Ok(camera)) = (screen, camera.single()) {
                crate::player::mobile_screen_direction(screen, camera, *mode)
                    .xz()
                    .normalize_or_zero()
            } else {
                transform.forward().xz().normalize_or_zero()
            }
        } else {
            Vec2::ZERO
        };
        commands.write(NetworkCommand::Utility { action, direction });
    }
}

/// Mobile abilities share the existing PendingCast/try_cast_slot path. The
/// assistance step changes only target choice, never range, mana or cooldowns.
pub(super) fn mobile_cast_system(
    mut mobile: Option<ResMut<crate::mobile_controls::MobileControls>>,
    context: Res<GameplayInputContext>,
    selection: Res<TeamSelection>,
    local: Query<
        (
            &Transform,
            &Team,
            &CombatStats,
            Option<&PlayerProgression>,
            Option<&NetworkHeroClass>,
            Option<&crate::net::PlayerLoadout>,
        ),
        With<Player>,
    >,
    candidates: TargetCandidates,
    validity: crate::targeting::TargetValidity,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    visual_mode: Res<PlayerVisualMode>,
    target: Res<TargetState>,
    basic: Res<crate::targeting::BasicAttackState>,
    mut pending: ResMut<PendingCast>,
    mut feedback: ResMut<ActionFeedback>,
    mut commands: MessageWriter<NetworkCommand>,
    world: super::aim_preview::AimWorld,
) {
    let Some(mobile) = mobile.as_deref_mut().filter(|mobile| mobile.enabled) else {
        return;
    };
    if !context.gameplay_allowed() {
        mobile.casts.clear();
        mobile.upgrades.clear();
        return;
    }
    let Ok((transform, team, stats, prog, class, loadout)) = local.single() else {
        return;
    };
    if !stats.is_alive() {
        mobile.casts.clear();
        mobile.upgrades.clear();
        return;
    }
    let prog = prog.copied().unwrap_or_default();
    let class = class.map(|class| class.0).unwrap_or(selection.hero_class);
    let Some(skills) = crate::equipped_skills::resolve(class, loadout) else {
        mobile.casts.clear();
        mobile.upgrades.clear();
        pending.cancel();
        return;
    };
    for slot in mobile.upgrades.drain(..) {
        if crate::equipped_skills::upgrade_eligible(&skills, &prog, slot) {
            commands.write(NetworkCommand::UpgradeSkill { slot: slot as u8 });
        }
    }
    let intent = mobile.casts.drain(..).next_back();
    let Some(intent) = intent else {
        return;
    };
    let Some(slot) = SkillSlot::from_index(intent.slot as u8) else {
        return;
    };
    let definition = skills.ability(slot);
    if skills.resolved().is_some() {
        let range = scaled_cast_range(definition, prog.ranks[intent.slot].max(1));
        let origin = transform.translation.xz();
        let manual = intent.aim.and_then(|screen| {
            camera.single().ok().map(|(_, pose)| {
                crate::player::mobile_screen_direction(screen, pose, *visual_mode).xz()
            })
        });
        let assisted = intent
            .aim
            .is_none()
            .then(|| {
                // A skill that is cast on an ally is aimed at one, never at an enemy.
                world
                    .ally_quick_cast(skills.skill(slot), origin, *team, &candidates)
                    .or_else(|| {
                        quick_cast_target(
                            origin,
                            *team,
                            range,
                            target.selected_entity,
                            basic.order.map(|order| order.entity),
                            &candidates,
                            &validity,
                        )
                    })
            })
            .flatten();
        queue_cast_request(intent.slot, &skills, &target, &mut pending, &mut feedback);
        pending.aim = Some(resolve_mobile_aim(
            origin,
            transform.forward().xz(),
            range,
            intent.extent,
            manual,
            assisted,
        ));
        return;
    }
    if definition.targeting == TargetingMode::UnitTarget {
        let Ok((camera, camera_transform)) = camera.single() else {
            return;
        };
        let range = scaled_cast_range(definition, prog.ranks[intent.slot].max(1));
        let pick = if intent.aim.is_none() && target.selected_entity.is_some() {
            // An explicit lock wins even when currently out of range. The skill
            // resolver reports that range error instead of hitting another foe.
            target.selected_entity.zip(target.selected_target)
        } else {
            mobile_assisted_target(
                transform.translation,
                *team,
                range,
                intent.aim,
                &candidates,
                &validity,
                camera,
                camera_transform,
                *visual_mode,
                target.selected_entity,
            )
        };
        let Some((entity, id)) = pick else {
            return;
        };
        let request_target = TargetState {
            selected_entity: Some(entity),
            selected_target: Some(id),
            ..default()
        };
        queue_cast_request(
            intent.slot,
            &skills,
            &request_target,
            &mut pending,
            &mut feedback,
        );
        return;
    }
    queue_cast_request(intent.slot, &skills, &target, &mut pending, &mut feedback);
}

/// Short taps prefer a valid lock/chase, then the closest visible hero. Manual
/// drags never pass through this chooser. Snapshot visibility remains authority.
pub(super) fn quick_cast_target(
    origin: Vec2,
    team: Team,
    range: f32,
    selected: Option<Entity>,
    chased: Option<Entity>,
    candidates: &TargetCandidates,
    validity: &crate::targeting::TargetValidity,
) -> Option<Vec2> {
    let range = range.min(shared::vision::HERO_SIGHT_RADIUS);
    let mut choices = Vec::new();
    let mut consider = |entity: Entity, id: TargetId, p: Vec3, stats: &CombatStats| {
        let distance = origin.distance(p.xz());
        if stats.is_alive()
            && p.is_finite()
            && distance <= range + validity.radius(entity, id)
            && validity.valid(entity, id, team)
        {
            let priority = if selected == Some(entity) {
                0
            } else if chased == Some(entity) {
                1
            } else if id.kind == TargetKind::Player {
                2
            } else {
                3
            };
            choices.push((priority, distance, id.id, p.xz()));
        }
    };
    for (entity, pose, id, stats, _) in &candidates.players {
        consider(
            entity,
            TargetId {
                kind: TargetKind::Player,
                id: id.0,
            },
            pose.translation,
            stats,
        );
    }
    for (entity, pose, id, stats, _) in &candidates.minions {
        consider(
            entity,
            TargetId {
                kind: TargetKind::Minion,
                id: id.0,
            },
            pose.translation,
            stats,
        );
    }
    for (entity, pose, id, stats) in &candidates.neutrals {
        consider(
            entity,
            TargetId {
                kind: TargetKind::Neutral,
                id: id.0,
            },
            pose.translation,
            stats,
        );
    }
    choices
        .into_iter()
        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)))
        .map(|choice| choice.3)
}

/// Where a tap, or a controller press without the stick, aims a skill that is cast on an
/// ally: at one of the living allies the client shows within the cast range. The orb is
/// lent to the nearest allied hero. The leap goes to the allied hero that is lowest on
/// health (then the nearest, then the lowest id), else to the nearest allied minion.
/// Without any of them the aim is the caster, whom the server takes as an ally of his own
/// (`common/src/skills/advanced.rs:165-197`, `:497-500`). `None` for every other technique.
pub(crate) fn ally_quick_cast_target(
    action: Technique,
    origin: Vec2,
    team: Team,
    range: f32,
    candidates: &TargetCandidates,
    seen: impl Fn(Entity) -> bool,
) -> Option<Vec2> {
    if !matches!(action, Technique::BallGuard | Technique::AllyLeap) {
        return None;
    }
    // How far an ally the cast can reach stands, and where.
    let reachable = |entity: Entity, pose: &Transform, stats: &CombatStats, other: &Team| {
        let at = pose.translation.xz();
        let distance = origin.distance(at);
        (*other == team && stats.is_alive() && at.is_finite() && distance <= range && seen(entity))
            .then_some((distance, at))
    };
    let hero = candidates
        .players
        .iter()
        .filter_map(|(entity, pose, id, stats, other)| {
            let (distance, at) = reachable(entity, pose, stats, other)?;
            // Only the leap asks who needs it most.
            let health = if action == Technique::AllyLeap && stats.max_hp > 0.0 {
                stats.hp / stats.max_hp
            } else {
                0.0
            };
            Some((health, distance, id.0, at))
        })
        .min_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        })
        .map(|choice| choice.3);
    let minion = || {
        candidates
            .minions
            .iter()
            .filter(|_| action == Technique::AllyLeap)
            .filter_map(|(entity, pose, id, stats, other)| {
                let (distance, at) = reachable(entity, pose, stats, other)?;
                Some((distance, id.0, at))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|choice| choice.2)
    };
    Some(hero.or_else(minion).unwrap_or(origin))
}

pub(super) fn resolve_mobile_aim(
    origin: Vec2,
    facing: Vec2,
    range: f32,
    extent: f32,
    manual: Option<Vec2>,
    assisted: Option<Vec2>,
) -> Vec2 {
    if let Some(direction) = manual {
        origin + direction.normalize_or_zero() * range * extent.clamp(0.0, 1.0)
    } else if let Some(target) = assisted {
        target
    } else {
        origin + facing.normalize_or_zero() * range
    }
}

pub(crate) fn mobile_assisted_target(
    position: Vec3,
    team: Team,
    range: f32,
    aim: Option<Vec2>,
    candidates: &TargetCandidates,
    validity: &crate::targeting::TargetValidity,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    mode: PlayerVisualMode,
    selected: Option<Entity>,
) -> Option<(Entity, TargetId)> {
    let screen = |p: Vec3| {
        let render = if mode == PlayerVisualMode::Sprite2d {
            crate::world2d::simulation_xz_to_render_xy(p).extend(0.0)
        } else {
            p
        };
        camera.world_to_viewport(camera_transform, render).ok()
    };
    let origin = screen(position)?;
    let viewport = camera.logical_viewport_size()?;
    let mut best: Option<(Entity, TargetId, f32)> = None;
    let mut consider = |entity: Entity, id: TargetId, p: Vec3, stats: &CombatStats, enemy: bool| {
        if !enemy || !stats.is_alive() || !validity.valid(entity, id, team) {
            return;
        }
        let distance = position.xz().distance(p.xz());
        if distance > range {
            return;
        }
        let Some(projected) = screen(p) else {
            return;
        };
        if projected.x < 0.0
            || projected.y < 0.0
            || projected.x > viewport.x
            || projected.y > viewport.y
        {
            return;
        }
        let Some(score) = mobile_target_score(
            distance,
            range,
            projected - origin,
            aim,
            selected == Some(entity),
        ) else {
            return;
        };
        if best.is_none_or(|(_, _, previous)| score < previous) {
            best = Some((entity, id, score));
        }
    };
    for (e, t, id, s, target_team) in &candidates.players {
        consider(
            e,
            TargetId {
                kind: TargetKind::Player,
                id: id.0,
            },
            t.translation,
            s,
            *target_team != team,
        );
    }
    for (e, t, id, s, target_team) in &candidates.minions {
        consider(
            e,
            TargetId {
                kind: TargetKind::Minion,
                id: id.0,
            },
            t.translation,
            s,
            *target_team != team,
        );
    }
    for (e, t, id, s) in &candidates.neutrals {
        consider(
            e,
            TargetId {
                kind: TargetKind::Neutral,
                id: id.0,
            },
            t.translation,
            s,
            true,
        );
    }
    for (e, t, id, s, target_team, _) in &candidates.structures {
        consider(
            e,
            TargetId {
                kind: TargetKind::Structure,
                id: id.0,
            },
            t.translation,
            s,
            *target_team != team,
        );
    }
    best.map(|(entity, id, _)| (entity, id))
}

pub(super) fn mobile_target_score(
    distance: f32,
    range: f32,
    screen_delta: Vec2,
    aim: Option<Vec2>,
    selected: bool,
) -> Option<f32> {
    if !distance.is_finite() || distance > range || range <= 0.0 {
        return None;
    }
    if let Some(aim) = aim {
        let alignment = screen_delta
            .normalize_or_zero()
            .dot(aim.normalize_or_zero());
        // Directional assist uses a 45-degree half cone and has no fallback
        // behind the player when the requested direction contains no enemy.
        (alignment >= std::f32::consts::FRAC_1_SQRT_2)
            .then_some((1.0 - alignment) * 4.0 + distance / range)
    } else {
        Some(distance / range - if selected { 2.0 } else { 0.0 })
    }
}

#[cfg(test)]
mod quick_cast_tests {
    use super::*;
    #[test]
    fn quick_cast_uses_lock_then_chase_then_visible_hero_without_hidden_or_dead_targets() {
        use bevy::ecs::system::SystemState;
        let mut world = World::new();
        let mut hero = |id, x, team, visibility| {
            world
                .spawn((
                    crate::net::RemotePlayer,
                    crate::net::NetworkPlayerId(id),
                    Transform::from_xyz(x, 0.0, 0.0),
                    CombatStats::default(),
                    team,
                    visibility,
                ))
                .id()
        };
        let near = hero(2, 8.0, Team::Blue, InheritedVisibility::VISIBLE);
        let chase = hero(3, 16.0, Team::Blue, InheritedVisibility::VISIBLE);
        let hidden = hero(4, 2.0, Team::Blue, InheritedVisibility::HIDDEN);
        hero(5, 1.0, Team::Green, InheritedVisibility::VISIBLE);
        let far = hero(6, 60.0, Team::Blue, InheritedVisibility::VISIBLE);
        let pick = |world: &mut World, selected, chased| {
            let mut state =
                SystemState::<(TargetCandidates, crate::targeting::TargetValidity)>::new(world);
            let (candidates, validity) = state.get(world).expect("targeting test resources exist");
            quick_cast_target(
                Vec2::ZERO,
                Team::Green,
                100.0,
                selected,
                chased,
                &candidates,
                &validity,
            )
        };
        assert_eq!(pick(&mut world, None, None), Some(Vec2::X * 8.0));
        assert_eq!(pick(&mut world, None, Some(chase)), Some(Vec2::X * 16.0));
        assert_eq!(
            pick(&mut world, Some(near), Some(chase)),
            Some(Vec2::X * 8.0)
        );
        assert_eq!(
            pick(&mut world, Some(hidden), Some(chase)),
            Some(Vec2::X * 16.0)
        );
        assert_eq!(pick(&mut world, Some(far), None), Some(Vec2::X * 8.0));
        world.get_mut::<CombatStats>(near).unwrap().hp = 0.0;
        assert_eq!(pick(&mut world, Some(near), None), Some(Vec2::X * 16.0));
    }

    #[test]
    fn melee_tap_acquires_overlapping_and_edge_of_reach_targets() {
        use bevy::ecs::system::SystemState;
        for x in [0.0, 0.02, 2.6 + shared::PLAYER_TARGET_RADIUS - 0.05] {
            let mut world = World::new();
            world.spawn((
                crate::net::RemotePlayer,
                crate::net::NetworkPlayerId(2),
                Transform::from_xyz(x, 0.0, 0.0),
                CombatStats::default(),
                Team::Blue,
                InheritedVisibility::VISIBLE,
            ));
            let mut state =
                SystemState::<(TargetCandidates, crate::targeting::TargetValidity)>::new(
                    &mut world,
                );
            let (candidates, validity) = state.get(&world).expect("targeting test resources exist");
            assert_eq!(
                quick_cast_target(
                    Vec2::ZERO,
                    Team::Green,
                    2.6,
                    None,
                    None,
                    &candidates,
                    &validity
                ),
                Some(Vec2::X * x)
            );
        }
    }

    #[test]
    fn tap_faces_target_but_drag_and_no_enemy_fallback_preserve_intent() {
        let origin = Vec2::new(3.0, 4.0);
        let enemy = Vec2::new(-5.0, 8.0);
        assert_eq!(
            resolve_mobile_aim(origin, Vec2::X, 40.0, 1.0, None, Some(enemy)),
            enemy
        );
        assert_eq!(
            resolve_mobile_aim(origin, Vec2::X, 40.0, 0.5, Some(Vec2::Y), Some(enemy)),
            origin + Vec2::Y * 20.0
        );
        assert_eq!(
            resolve_mobile_aim(origin, Vec2::X, 40.0, 1.0, None, None),
            origin + Vec2::X * 40.0
        );
    }

    /// A hero of another player with `health` of 1000.
    fn hero(world: &mut World, id: u64, at: Vec2, team: Team, health: f32) -> Entity {
        world
            .spawn((
                crate::net::RemotePlayer,
                crate::net::NetworkPlayerId(id),
                Transform::from_xyz(at.x, 0.0, at.y),
                CombatStats {
                    hp: health,
                    max_hp: 1000.0,
                    ..default()
                },
                team,
                InheritedVisibility::VISIBLE,
            ))
            .id()
    }

    fn minion(world: &mut World, id: u64, at: Vec2, team: Team) -> Entity {
        world
            .spawn((
                crate::net::NetworkMinion,
                crate::net::NetworkMinionId(id),
                Transform::from_xyz(at.x, 0.0, at.y),
                CombatStats::default(),
                team,
                InheritedVisibility::VISIBLE,
            ))
            .id()
    }

    /// Where a tap of a green hero at the origin aims a technique of `range`.
    fn ally_tap(world: &mut World, action: Technique, range: f32) -> Option<Vec2> {
        use bevy::ecs::system::SystemState;
        let mut state = SystemState::<(TargetCandidates, Query<&InheritedVisibility>)>::new(world);
        let (candidates, visible) = state.get(world).expect("targeting test resources exist");
        ally_quick_cast_target(
            action,
            Vec2::ZERO,
            Team::Green,
            range,
            &candidates,
            |entity| visible.get(entity).map_or(true, |shown| shown.get()),
        )
    }

    #[test]
    fn ally_quick_cast_target_returns_the_nearest_living_ally_and_ignores_enemies_and_dead_allies()
    {
        let orb = Technique::BallGuard;
        let mut world = World::new();
        // Nobody to lend the orb to: it comes home to the caster.
        assert_eq!(ally_tap(&mut world, orb, 15.0), Some(Vec2::ZERO));
        let far = hero(&mut world, 2, Vec2::X * 8.0, Team::Green, 1000.0);
        let near = hero(&mut world, 3, Vec2::Y * 5.0, Team::Green, 1000.0);
        // Nearer than both: an enemy, a dead ally, an ally the client hides and an allied
        // minion, which the orb cannot guard. Weaker than both: an ally out of range.
        hero(&mut world, 4, Vec2::X * 2.0, Team::Blue, 1000.0);
        hero(&mut world, 5, Vec2::X, Team::Green, 0.0);
        let hidden = hero(&mut world, 6, Vec2::X * 3.0, Team::Green, 1000.0);
        world.entity_mut(hidden).insert(InheritedVisibility::HIDDEN);
        minion(&mut world, 7, Vec2::X * 0.5, Team::Green);
        hero(&mut world, 8, Vec2::X * 15.01, Team::Green, 100.0);
        assert_eq!(ally_tap(&mut world, orb, 15.0), Some(Vec2::Y * 5.0));
        // Not the weakest: the orb goes to the nearest.
        world.get_mut::<CombatStats>(far).unwrap().hp = 100.0;
        assert_eq!(ally_tap(&mut world, orb, 15.0), Some(Vec2::Y * 5.0));
        world.get_mut::<CombatStats>(near).unwrap().hp = 0.0;
        assert_eq!(ally_tap(&mut world, orb, 15.0), Some(Vec2::X * 8.0));
        // The cast range is the reach, to the unit itself.
        assert_eq!(ally_tap(&mut world, orb, 8.0), Some(Vec2::X * 8.0));
        assert_eq!(ally_tap(&mut world, orb, 7.99), Some(Vec2::ZERO));
        // At the same distance the lower id.
        hero(&mut world, 1, Vec2::NEG_X * 8.0, Team::Green, 1000.0);
        assert_eq!(ally_tap(&mut world, orb, 15.0), Some(Vec2::NEG_X * 8.0));
        // A skill that is not cast on an ally has no such aim: the leap that may go
        // without one, a cast on an enemy, a lane.
        for other in [Technique::GuardLeap, Technique::Lash, Technique::Hook] {
            assert_eq!(ally_tap(&mut world, other, 15.0), None, "{other:?}");
        }
    }

    #[test]
    fn ally_leap_tap_prefers_weakest_allied_hero_then_minion_then_self() {
        let leap = Technique::AllyLeap;
        let mut world = World::new();
        // Nobody to leap to: the caster shields himself.
        assert_eq!(ally_tap(&mut world, leap, 10.0), Some(Vec2::ZERO));
        // Enemies are never leapt to, however weak or near.
        hero(&mut world, 20, Vec2::X, Team::Blue, 10.0);
        minion(&mut world, 21, Vec2::Y, Team::Blue);
        assert_eq!(ally_tap(&mut world, leap, 10.0), Some(Vec2::ZERO));
        // An allied minion is an ally to leap to, the nearest living one in range.
        minion(&mut world, 30, Vec2::X * 6.0, Team::Green);
        let near = minion(&mut world, 31, Vec2::Y * 4.0, Team::Green);
        minion(&mut world, 32, Vec2::X * 10.5, Team::Green);
        assert_eq!(ally_tap(&mut world, leap, 10.0), Some(Vec2::Y * 4.0));
        world.get_mut::<CombatStats>(near).unwrap().hp = 0.0;
        assert_eq!(ally_tap(&mut world, leap, 10.0), Some(Vec2::X * 6.0));
        // An allied hero comes before every minion, even at full health and further away.
        let hale = hero(&mut world, 2, Vec2::NEG_X * 9.0, Team::Green, 1000.0);
        assert_eq!(ally_tap(&mut world, leap, 10.0), Some(Vec2::NEG_X * 9.0));
        // Of the heroes the one lowest on health, as a fraction of its own maximum.
        let hurt = hero(&mut world, 3, Vec2::NEG_Y * 8.0, Team::Green, 400.0);
        assert_eq!(ally_tap(&mut world, leap, 10.0), Some(Vec2::NEG_Y * 8.0));
        let sturdy = hero(&mut world, 4, Vec2::new(3.0, 3.0), Team::Green, 700.0);
        world.get_mut::<CombatStats>(sturdy).unwrap().max_hp = 2000.0;
        assert_eq!(ally_tap(&mut world, leap, 10.0), Some(Vec2::new(3.0, 3.0)));
        // Not one out of range, dead or hidden.
        hero(&mut world, 5, Vec2::X * 10.01, Team::Green, 50.0);
        hero(&mut world, 6, Vec2::X * 2.0, Team::Green, 0.0);
        let hidden = hero(&mut world, 7, Vec2::X * 2.5, Team::Green, 50.0);
        world.entity_mut(hidden).insert(InheritedVisibility::HIDDEN);
        assert_eq!(ally_tap(&mut world, leap, 10.0), Some(Vec2::new(3.0, 3.0)));
        // At the same health the nearest, then the lower id.
        world.get_mut::<CombatStats>(sturdy).unwrap().hp = 2000.0;
        world.get_mut::<CombatStats>(hale).unwrap().hp = 400.0;
        assert_eq!(ally_tap(&mut world, leap, 10.0), Some(Vec2::NEG_Y * 8.0));
        world.get_mut::<Transform>(hale).unwrap().translation = Vec3::new(8.0, 0.0, 0.0);
        assert_eq!(ally_tap(&mut world, leap, 10.0), Some(Vec2::X * 8.0));
        // At full health all round, the nearest hero.
        world.get_mut::<CombatStats>(hurt).unwrap().hp = 1000.0;
        world.get_mut::<CombatStats>(hale).unwrap().hp = 1000.0;
        assert_eq!(ally_tap(&mut world, leap, 10.0), Some(Vec2::new(3.0, 3.0)));
    }
}
