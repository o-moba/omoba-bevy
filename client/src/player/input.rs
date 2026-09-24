use crate::camera::MainCamera;
use crate::combat::{CombatStats, PendingCast, WorldPointerState};
use crate::domain::{MovementRoute, MovementTarget, Player, PlayerBody};
use crate::maps::MapLayout;
use crate::minimap::MinimapNavigationState;
use crate::net::{
    GameState, GameStateSnapshot, NetworkAvatar, NetworkCharacterChoice, NetworkStructure,
    StructureKind,
};
use crate::sprite::PlayerVisualMode;
use crate::targeting::BasicAttackState;
use crate::team::CharacterChoice;
use crate::world2d::render_xy_to_simulation_xz;
use bevy::{
    input::mouse::MouseButton,
    math::{Dir3, primitives::InfinitePlane3d},
    prelude::*,
    window::PrimaryWindow,
};
use shared::hero_balance::{DEBUG_SPEED_MULTIPLIER, PLAYER_SPEED};

use super::animation::{PlayerAnimationLibrary, avatar_key};
use super::motion::{
    Jumping, clip_static_movement, hero_movement_multiplier, resolve_player_collisions,
    structure_revision,
};
use super::{DebugSpeedBoost, JUMP_DURATION};

pub(super) fn handle_player_input(
    mut commands: Commands,
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    touch_input: (
        Res<Touches>,
        Option<Res<crate::mobile_controls::MobileControls>>,
    ),
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    player_query: Query<
        (
            Entity,
            &CombatStats,
            Option<&NetworkCharacterChoice>,
            Option<&NetworkAvatar>,
        ),
        With<Player>,
    >,
    context: Res<crate::input_context::GameplayInputContext>,
    animation_library: Res<PlayerAnimationLibrary>,
    minimap_nav: Option<Res<MinimapNavigationState>>,
    map_layout: Option<Res<MapLayout>>,
    game_state: Option<Res<GameStateSnapshot>>,
    visual_mode: Res<PlayerVisualMode>,
    pointer_state: Res<WorldPointerState>,
    orders: (ResMut<PendingCast>, ResMut<BasicAttackState>),
    ui_interactions: Query<&Interaction, With<Button>>,
) {
    let (_touches, mobile) = touch_input;
    let (mut pending_cast, mut basic) = orders;
    if mobile.as_ref().is_some_and(|mobile| mobile.enabled) {
        return;
    }
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    if !context.gameplay_allowed() {
        return;
    }
    let Ok((player_entity, stats, character, avatar)) = player_query.single() else {
        return;
    };
    if !stats.is_alive() {
        return;
    }

    let secondary_move = secondary_move_pressed(&mouse_button_input, &keyboard);
    if !secondary_move || pointer_state.consumed_secondary_press {
        return;
    }
    let minimap_target = minimap_nav.as_ref().and_then(|nav| nav.movement_target);
    let target = if let Some(target) = minimap_target.filter(|_| secondary_move) {
        // The minimap is deliberately a UI surface. Its bounds and input gate
        // were checked before world picking; never raycast through it.
        Some(target)
    } else {
        let (Ok(window), Ok((camera, camera_transform))) =
            (window_query.single(), camera_query.single())
        else {
            return;
        };
        let position = window.focused.then(|| window.cursor_position()).flatten();
        let Some(position) = position else { return };
        if !should_issue_ground_move(
            pointer_state.consumed_secondary_press,
            minimap_nav
                .as_ref()
                .is_some_and(|nav| nav.consumed_primary_click),
            ui_interactions
                .iter()
                .any(|interaction| *interaction != Interaction::None),
        ) {
            return;
        }
        viewport_to_simulation_world(camera, camera_transform, position, *visual_mode, 0.0)
    };
    if let Some(mut target_pos) = target {
        pending_cast.cancel();
        basic.cancel_for_movement();
        if let Some(map_layout) = map_layout.as_ref() {
            target_pos = map_layout.clamp_position(target_pos);
        }
        commands
            .entity(player_entity)
            .insert(MovementTarget { target: target_pos });
        let character = character
            .map(|selected| selected.0)
            .unwrap_or(CharacterChoice::Ipfs);
        let key = avatar_key(character, avatar);
        if !animation_library.should_use_jump_fallback(&key) {
            commands.entity(player_entity).remove::<Jumping>();
        } else {
            commands.entity(player_entity).insert(Jumping {
                timer: Timer::from_seconds(JUMP_DURATION, TimerMode::Repeating),
            });
        }
    }
}

/// Thumb motion is a direct, analog step through the existing collision and map
/// clipping path. It never creates a long-lived route or an automatic chase.
pub(super) fn move_player_mobile(
    game: Option<Res<GameStateSnapshot>>,
    mut commands: Commands,
    time: Res<Time>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    context: Res<crate::input_context::GameplayInputContext>,
    mode: Res<PlayerVisualMode>,
    camera: Query<&GlobalTransform, With<MainCamera>>,
    mut transforms: ParamSet<(
        Query<
            (
                Entity,
                &mut Transform,
                &CombatStats,
                Option<&crate::net::PlayerEquipment>,
                Option<&crate::net::PlayerUtility>,
                Option<&crate::net::NetworkHeroClass>,
                Option<&crate::net::PlayerProgression>,
            ),
            With<Player>,
        >,
        Query<&Transform, (With<PlayerBody>, Without<Player>)>,
        Query<(&Transform, &StructureKind, Option<&CombatStats>), With<NetworkStructure>>,
    )>,
    map: Option<Res<MapLayout>>,
    boost: Res<DebugSpeedBoost>,
    mut pending: ResMut<PendingCast>,
    mut basic: ResMut<BasicAttackState>,
) {
    let Some(mobile) = mobile.filter(|mobile| mobile.enabled) else {
        return;
    };
    let other_players = transforms
        .p1()
        .iter()
        .map(|t| t.translation)
        .collect::<Vec<_>>();
    let structures = transforms
        .p2()
        .iter()
        .filter(|(_, _, stats)| stats.is_none_or(|s| s.is_alive()))
        .map(|(t, kind, _)| (t.translation, *kind))
        .collect::<Vec<_>>();
    let allowed = context.gameplay_allowed() && mobile.focused && mobile.landscape;
    let direction = camera
        .single()
        .ok()
        .map(|camera| mobile_screen_direction(mobile.movement, camera, *mode))
        .unwrap_or(Vec3::ZERO);
    for (entity, mut transform, stats, equipment, utility, class, progression) in
        &mut transforms.p0()
    {
        commands.entity(entity).remove::<Jumping>();
        if !allowed || !stats.is_alive() {
            commands
                .entity(entity)
                .remove::<(MovementTarget, MovementRoute)>();
            pending.cancel();
            basic.cancel_for_movement();
            continue;
        }
        if direction.length_squared() < 0.0001 {
            // Idle stick: a basic attack may be walking the hero to its target.
            continue;
        }
        // The stick takes over: drop any walk-to-target the attack started.
        commands
            .entity(entity)
            .remove::<(MovementTarget, MovementRoute)>();
        let current = transform.translation;
        let speed = crate::sandbox::movement_speed(
            game.as_deref(),
            PLAYER_SPEED
                * if boost.0 { DEBUG_SPEED_MULTIPLIER } else { 1.0 }
                * equipment.map_or(1.0, |e| e.item_bonuses.move_speed_multiplier)
                * hero_movement_multiplier(class, progression),
        ) * utility.map_or(1.0, |u| u.state.movement_multiplier());
        // Bound a resumed/hitched frame; the server movement envelope remains authoritative.
        let desired = current
            + direction
                * speed
                * time.delta_secs().min(0.1)
                * crate::sandbox::time_scale(game.as_deref());
        let mut desired = resolve_player_collisions(current, desired, &other_players, &structures);
        if let Some(map) = map.as_ref() {
            desired = map.clamp_position(desired);
        }
        desired = clip_static_movement(current, desired);
        transform.translation.x = desired.x;
        transform.translation.z = desired.z;
        let yaw = shared::math::hero_yaw_towards(direction.x, direction.z);
        transform.rotation = transform.rotation.slerp(
            Quat::from_rotation_y(yaw),
            (time.delta_secs() * 10.0).min(1.0),
        );
    }
}

pub(crate) fn mobile_screen_direction(
    screen: Vec2,
    camera: &GlobalTransform,
    mode: PlayerVisualMode,
) -> Vec3 {
    if mode == PlayerVisualMode::Sprite2d {
        return render_xy_to_simulation_xz(Vec2::new(screen.x, -screen.y), 0.0).normalize_or_zero()
            * screen.length().min(1.0);
    }
    let right = *camera.right();
    let up = *camera.up();
    let right = Vec3::new(right.x, 0.0, right.z).normalize_or_zero();
    let up = Vec3::new(up.x, 0.0, up.z).normalize_or_zero();
    (right * screen.x - up * screen.y).normalize_or_zero() * screen.length().min(1.0)
}

fn secondary_move_pressed(
    mouse: &ButtonInput<MouseButton>,
    keyboard: &ButtonInput<KeyCode>,
) -> bool {
    mouse.just_pressed(MouseButton::Right)
        && !keyboard.any_pressed([KeyCode::AltLeft, KeyCode::AltRight])
}

/// Plan once per new destination. Attack approach targets use the same route
/// machinery, while transient hero overlap remains the existing local resolver.
pub(super) fn plan_movement_routes(
    mut commands: Commands,
    players: Query<
        (
            Entity,
            &Transform,
            Option<&MovementTarget>,
            Option<&MovementRoute>,
            &CombatStats,
        ),
        With<Player>,
    >,
    structures: Query<(&Transform, &StructureKind, Option<&CombatStats>), With<NetworkStructure>>,
    layout: Option<Res<MapLayout>>,
    context: Option<Res<crate::input_context::GameplayInputContext>>,
    mut feedback: Option<ResMut<crate::combat::ActionFeedback>>,
) {
    let structures: Vec<_> = structures
        .iter()
        .filter(|(_, _, stats)| stats.is_none_or(|stats| stats.is_alive()))
        .map(|(transform, kind, _)| (transform.translation, *kind))
        .collect();
    let structure_revision = structure_revision(&structures);
    let layout = layout.as_deref().copied().unwrap_or_default();
    let running = context.as_ref().is_none_or(|context| context.running);
    for (entity, transform, target, route, stats) in &players {
        let Some(target) = target.filter(|_| stats.is_alive() && running) else {
            if route.is_some() {
                commands.entity(entity).remove::<MovementRoute>();
            }
            if !stats.is_alive() || !running {
                commands
                    .entity(entity)
                    .remove::<(MovementTarget, Jumping)>();
            }
            continue;
        };
        if route.is_some_and(|route| {
            route.structure_revision == structure_revision
                && route.waypoints.first().is_none_or(|next| {
                    let discs: Vec<_> = structures
                        .iter()
                        .map(|(p, kind)| shared::navigation::Disc {
                            center: [p.x, p.z],
                            radius: crate::navigation::structure_collision_radius(*kind)
                                - shared::navigation::HERO_RADIUS,
                        })
                        .collect();
                    shared::navigation::world_navigation().segment_clear_with_discs(
                        transform.translation.xz().to_array(),
                        next.xz().to_array(),
                        &discs,
                    )
                })
                && route
                    .requested_target
                    .xz()
                    .distance_squared(target.target.xz())
                    < 0.000001
        }) {
            continue;
        }
        match crate::navigation::plan_route(
            &layout,
            transform.translation,
            target.target,
            &structures,
        ) {
            Some(waypoints) if !waypoints.is_empty() => {
                commands.entity(entity).insert(MovementRoute {
                    requested_target: target.target,
                    structure_revision,
                    destination: *waypoints.last().unwrap(),
                    waypoints,
                });
            }
            result => {
                commands
                    .entity(entity)
                    .remove::<(MovementTarget, MovementRoute, Jumping)>();
                if result.is_none() {
                    if let Some(feedback) = feedback.as_deref_mut() {
                        feedback.push_line("No walkable route to that point.");
                    }
                }
            }
        }
    }
}

pub(super) const fn should_issue_ground_move(
    target_consumed: bool,
    minimap_consumed: bool,
    pointer_over_ui: bool,
) -> bool {
    !target_consumed && !minimap_consumed && !pointer_over_ui
}

/// Maps the active camera viewport into authoritative simulation XZ.
pub(crate) fn viewport_to_simulation_world(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    viewport_position: Vec2,
    mode: PlayerVisualMode,
    simulation_y: f32,
) -> Option<Vec3> {
    let ray = camera
        .viewport_to_world(camera_transform, viewport_position)
        .ok()?;
    if mode == PlayerVisualMode::Sprite2d {
        return Some(render_xy_to_simulation_xz(ray.origin.xy(), simulation_y));
    }
    let plane_normal = Dir3::new(Vec3::Y).ok()?;
    let distance = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(plane_normal))?;
    (distance >= 0.0).then(|| ray.get_point(distance))
}
