use super::input::should_issue_ground_move;
use super::motion::resolve_player_collisions;
use crate::combat::{CombatStats, PendingCast, WorldPointerState};
use crate::minimap::MinimapNavigationState;
use crate::net::{GameState, GameStateSnapshot, NetworkStructure, StructureKind};
use crate::targeting::BasicAttackState;
use bevy::input::mouse::MouseButton;

#[test]
fn local_structure_sweep_blocks_crossing_and_allows_outward_recovery() {
    use super::*;
    let structures = [(Vec3::ZERO, StructureKind::Tower)];
    let blocked = resolve_player_collisions(Vec3::NEG_X * 5.0, Vec3::X * 5.0, &[], &structures);
    assert!(blocked.x < -crate::navigation::structure_collision_radius(StructureKind::Tower));
    let escaped = resolve_player_collisions(Vec3::X, Vec3::X * 3.0, &[], &structures);
    assert_eq!(escaped, Vec3::X * 3.0);
}

#[test]
fn mobile_structure_sliding_agrees_with_server_at_twenty_hz() {
    use super::*;
    // Reproduce direct thumb motion around a base, with three render frames
    // per network sample. Endpoints alone are not a safe swept trajectory.
    let center = Vec3::new(-79.549515, 0.5, -79.549515);
    let structures = [(center, StructureKind::BaseTower)];
    let discs = [shared::navigation::Disc {
        center: [center.x, center.z],
        radius: shared::navigation::BASE_COLLISION_RADIUS,
    }];
    let mut local = center + Vec3::new(4.5, 0.0, 0.0);
    let mut server = [local.x, local.z];
    for frame in 0..240 {
        let desired = local + Vec3::new(-0.6, 0.0, 0.8) * PLAYER_SPEED / 60.0;
        local = resolve_player_collisions(local, desired, &[], &structures);
        if frame % 3 == 2 {
            server = shared::navigation::clip_discs(server, [local.x, local.z], &discs);
            let error = (server[0] - local.x).hypot(server[1] - local.z);
            assert!(
                error < 0.02,
                "prediction diverged at frame {frame}: error={error}, local={local:?}, server={server:?}"
            );
        }
    }
    assert!(
        local.z > center.z + 10.0,
        "thumb movement must slide past the base"
    );
}

#[test]
fn mobile_joystick_uses_screen_axes_and_retains_analog_speed() {
    let camera = GlobalTransform::from(
        Transform::from_translation(crate::camera::locked_camera_offset(1.0))
            .looking_at(Vec3::ZERO, Vec3::Y),
    );
    let right = super::mobile_screen_direction(Vec2::X, &camera, PlayerVisualMode::Models3d);
    let up = super::mobile_screen_direction(Vec2::NEG_Y, &camera, PlayerVisualMode::Models3d);
    assert!(right.dot(Vec3::Z) > 0.99);
    assert!(up.dot(Vec3::X) > 0.99);
    let half = super::mobile_screen_direction(Vec2::X * 0.5, &camera, PlayerVisualMode::Models3d);
    assert!((half.length() - 0.5).abs() < 0.001);
    assert_eq!(
        super::mobile_screen_direction(Vec2::X, &camera, PlayerVisualMode::Sprite2d),
        Vec3::X
    );
    assert_eq!(
        super::mobile_screen_direction(Vec2::NEG_Y, &camera, PlayerVisualMode::Sprite2d),
        Vec3::Z
    );
}

use super::*;
#[test]
fn level_ten_route_prediction_moves_at_grown_speed_with_boots() {
    for class in shared::HeroClass::ALL {
        let (mut app, hero) = navigation_input_app();
        app.world_mut().entity_mut(hero).insert((
            crate::net::NetworkHeroClass(class),
            crate::net::PlayerProgression {
                level: 10,
                ..default()
            },
            crate::net::PlayerEquipment {
                item_bonuses: shared::shop::item_bonuses(&[shared::shop::ItemId::TrailBoots]),
                ..default()
            },
        ));
        minimap_order(&mut app, Vec3::new(8.0, 0.0, 0.0));
        app.update();
        let before = app.world().get::<Transform>(hero).unwrap().translation;
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(50));
        app.update();
        let after = app.world().get::<Transform>(hero).unwrap().translation;
        assert!((before.xz().distance(after.xz()) - 5.0 * 1.24 * 1.08 * 0.05).abs() < 0.001);
    }
}

#[test]
fn player_grounding_preserves_sprite_corners_and_tracks_verdant_walktops() {
    let layout = MapLayout::default();
    let c = layout.home_spawn;
    let sprite_y = ground_origin_y(
        &layout,
        PlayerVisualMode::Sprite2d,
        None,
        c.x + 24.0,
        c.z + 26.0,
    );
    let model_y = ground_origin_y(
        &layout,
        PlayerVisualMode::Models3d,
        None,
        c.x + 24.0,
        c.z + 26.0,
    );
    assert!((sprite_y - (0.7 * 5.0 / 6.0 + PLAYER_SIZE * 0.5)).abs() < 0.00001);
    assert!((model_y - (0.35 + PLAYER_SIZE * 0.5)).abs() < 0.00001);
    assert_eq!(
        ground_origin_y(&layout, PlayerVisualMode::Models3d, None, 0.0, 0.0),
        PLAYER_SIZE * 0.5
    );
}

#[test]
fn modal_ground_press_never_creates_movement_intent() {
    let mut app = App::new();
    app.init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<Touches>()
        .init_resource::<crate::input_context::GameplayInputContext>()
        .init_resource::<PlayerAnimationLibrary>()
        .init_resource::<PendingCast>()
        .init_resource::<BasicAttackState>()
        .init_resource::<WorldPointerState>()
        .insert_resource(PlayerVisualMode::Models3d)
        .add_systems(Update, handle_player_input);
    let player = app.world_mut().spawn((Player, CombatStats::default())).id();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.world_mut()
        .resource_mut::<crate::input_context::GameplayInputContext>()
        .modal_open = true;
    app.update();
    assert!(!app.world().entity(player).contains::<MovementTarget>());
    app.world_mut()
        .resource_mut::<crate::input_context::GameplayInputContext>()
        .modal_open = false;
    app.world_mut()
        .resource_mut::<crate::input_context::GameplayInputContext>()
        .debug_flight = true;
    app.update();
    assert!(!app.world().entity(player).contains::<MovementTarget>());
}

fn navigation_input_app() -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<Touches>()
        .init_resource::<crate::input_context::GameplayInputContext>()
        .init_resource::<PlayerAnimationLibrary>()
        .init_resource::<PendingCast>()
        .init_resource::<BasicAttackState>()
        .init_resource::<WorldPointerState>()
        .init_resource::<MinimapNavigationState>()
        .init_resource::<DebugSpeedBoost>()
        .init_resource::<MapLayout>()
        .init_resource::<Time>()
        .insert_resource(PlayerVisualMode::Models3d)
        .add_systems(
            Update,
            (handle_player_input, plan_movement_routes, move_player)
                .chain()
                .after(crate::input_context::InputContextSet::Resolve),
        );
    let player = app
        .world_mut()
        .spawn((
            Player,
            Transform::from_xyz(-8.0, 0.5, 0.0),
            CombatStats::default(),
        ))
        .id();
    (app, player)
}

fn minimap_order(app: &mut App, target: Vec3) {
    app.world_mut()
        .resource_mut::<MinimapNavigationState>()
        .movement_target = Some(target);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Right);
}

#[test]
fn right_minimap_order_cancels_pending_cast_and_replaces_route_without_camera_ray() {
    let (mut app, player) = navigation_input_app();
    app.insert_resource(PendingCast::queued_for_movement_test());
    // Minimap hover must allow its own move order even though it is UI.
    app.world_mut().spawn((Button, Interaction::Hovered));
    minimap_order(&mut app, Vec3::new(8.0, 0.0, 0.0));
    app.update();
    assert!(!app.world().resource::<PendingCast>().has_queued_request());
    assert_eq!(
        app.world()
            .get::<MovementRoute>(player)
            .unwrap()
            .destination
            .xz(),
        Vec2::new(8.0, 0.0)
    );
    minimap_order(&mut app, Vec3::new(-8.0, 0.0, 12.0));
    app.update();
    assert_eq!(
        app.world()
            .get::<MovementRoute>(player)
            .unwrap()
            .destination
            .xz(),
        Vec2::new(-8.0, 12.0)
    );
}

#[test]
fn right_minimap_order_obeys_modal_flight_running_death_and_both_alt_keys() {
    for blocked in 0..6 {
        let (mut app, player) = navigation_input_app();
        minimap_order(&mut app, Vec3::new(8.0, 0.0, 0.0));
        match blocked {
            0 => {
                app.world_mut()
                    .resource_mut::<crate::input_context::GameplayInputContext>()
                    .modal_open = true
            }
            1 => {
                app.world_mut()
                    .resource_mut::<crate::input_context::GameplayInputContext>()
                    .debug_flight = true
            }
            2 => {
                app.world_mut()
                    .resource_mut::<crate::input_context::GameplayInputContext>()
                    .running = false
            }
            3 => app.world_mut().get_mut::<CombatStats>(player).unwrap().hp = 0.0,
            4 => app
                .world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::AltLeft),
            _ => app
                .world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::AltRight),
        }
        app.update();
        assert!(
            app.world().get::<MovementTarget>(player).is_none(),
            "gate {blocked}"
        );
        assert!(
            app.world().get::<MovementRoute>(player).is_none(),
            "gate {blocked}"
        );
    }
}

#[test]
fn persistent_route_reaches_destination_around_tower_at_normal_speed_after_release() {
    let (mut app, player) = navigation_input_app();
    app.world_mut().spawn((
        NetworkStructure,
        StructureKind::Tower,
        Transform::from_xyz(0.0, 0.5, 0.0),
    ));
    let target = Vec3::new(8.0, 0.5, 0.0);
    minimap_order(&mut app, target);
    app.update();
    assert!(
        app.world()
            .get::<MovementRoute>(player)
            .unwrap()
            .waypoints
            .len()
            > 1
    );
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<MinimapNavigationState>()
        .movement_target = None;
    let mut previous = app.world().get::<Transform>(player).unwrap().translation;
    let mut reached = false;
    for _ in 0..180 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(50));
        app.update();
        let current = app.world().get::<Transform>(player).unwrap().translation;
        assert!(previous.xz().distance(current.xz()) <= PLAYER_SPEED * 0.05 + 0.001);
        assert!(current.xz().length() >= 1.8 - 0.001);
        previous = current;
        if app.world().get::<MovementTarget>(player).is_none() {
            reached = true;
            break;
        }
    }
    assert!(reached, "route must terminate at arrival");
    assert!(previous.xz().distance(target.xz()) < 0.01);
    assert!(app.world().get::<MovementRoute>(player).is_none());
}

#[test]
fn a_destroyed_structure_replans_the_current_order_without_another_click() {
    let (mut app, player) = navigation_input_app();
    let tower = app
        .world_mut()
        .spawn((
            NetworkStructure,
            StructureKind::Tower,
            Transform::from_xyz(0.0, 0.5, 0.0),
            CombatStats::default(),
        ))
        .id();
    minimap_order(&mut app, Vec3::new(8.0, 0.5, 0.0));
    app.update();
    assert!(
        app.world()
            .get::<MovementRoute>(player)
            .unwrap()
            .waypoints
            .len()
            > 1
    );
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<MinimapNavigationState>()
        .movement_target = None;
    app.world_mut().get_mut::<CombatStats>(tower).unwrap().hp = 0.0;
    app.update();
    assert_eq!(
        app.world()
            .get::<MovementRoute>(player)
            .unwrap()
            .waypoints
            .len(),
        1
    );
}

#[test]
fn forest_rmb_route_survives_release_and_reaches_the_other_side() {
    let map = shared::navigation::world_navigation();
    let (start, end, center) = map
        .obstacles()
        .iter()
        .filter(|o| o.kind == "tree_trunk")
        .find_map(|o| {
            let center = o
                .vertices
                .iter()
                .fold(Vec2::ZERO, |a, p| a + Vec2::from_array(*p))
                / o.vertices.len() as f32;
            let a = center - Vec2::X * 4.0;
            let b = center + Vec2::X * 4.0;
            (map.point_clear(a.to_array())
                && map.point_clear(b.to_array())
                && !map.segment_clear(a.to_array(), b.to_array())
                && map.plan_route(a.to_array(), b.to_array(), &[]).is_some())
            .then_some((a, b, center))
        })
        .unwrap();
    let (mut app, player) = navigation_input_app();
    app.world_mut()
        .get_mut::<Transform>(player)
        .unwrap()
        .translation = Vec3::new(start.x, 0.5, start.y);
    minimap_order(&mut app, Vec3::new(end.x, 0.5, end.y));
    app.update();
    assert!(
        app.world()
            .get::<MovementRoute>(player)
            .unwrap()
            .waypoints
            .len()
            > 1
    );
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<MinimapNavigationState>()
        .movement_target = None;
    let next = app.world().get::<MovementRoute>(player).unwrap().waypoints[0].xz();
    let away = (center - next).normalize();
    let displaced = [3.0, 4.0, 5.0]
        .into_iter()
        .map(|r| center + away * r)
        .find(|p| {
            map.point_clear(p.to_array()) && !map.segment_clear(p.to_array(), next.to_array())
        })
        .unwrap();
    app.world_mut()
        .get_mut::<Transform>(player)
        .unwrap()
        .translation = Vec3::new(displaced.x, 0.5, displaced.y);
    app.update();
    let replanned = app.world().get::<MovementRoute>(player).unwrap().waypoints[0].xz();
    assert!(
        map.segment_clear(displaced.to_array(), replanned.to_array()),
        "off-route correction must replan"
    );
    let mut previous = displaced;
    for _ in 0..300 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(50));
        app.update();
        let current = app
            .world()
            .get::<Transform>(player)
            .unwrap()
            .translation
            .xz();
        assert!(map.segment_clear(previous.to_array(), current.to_array()));
        assert!(previous.distance(current) <= PLAYER_SPEED * 0.05 + 0.001);
        previous = current;
        if app.world().get::<MovementTarget>(player).is_none() {
            break;
        }
    }
    assert!(previous.distance(end) < 0.01);
    assert!(app.world().get::<MovementRoute>(player).is_none());
}

#[test]
fn admission_and_disconnect_gate_real_input_context_and_discard_stale_route() {
    let (mut app, player) = navigation_input_app();
    app.add_plugins(crate::input_context::InputContextPlugin)
        .insert_resource(GameStateSnapshot {
            state: GameState::Running,
            ..default()
        })
        .init_resource::<crate::net::ClientSession>();
    minimap_order(&mut app, Vec3::new(8.0, 0.5, 0.0));
    app.update();
    assert!(app.world().get::<MovementTarget>(player).is_none());
    app.insert_resource(crate::net::ClientSession::admitted_for_test());
    minimap_order(&mut app, Vec3::new(8.0, 0.5, 0.0));
    app.update();
    assert!(app.world().get::<MovementRoute>(player).is_some());
    app.world_mut()
        .resource_mut::<crate::net::ClientSession>()
        .state = crate::net::ClientConnectionState::Disconnected;
    minimap_order(&mut app, Vec3::new(12.0, 0.5, 0.0));
    app.update();
    assert!(app.world().get::<MovementTarget>(player).is_none());
    assert!(app.world().get::<MovementRoute>(player).is_none());
}

#[test]
fn removed_intent_and_death_discard_cached_route() {
    for dead in [false, true] {
        let (mut app, player) = navigation_input_app();
        minimap_order(&mut app, Vec3::new(8.0, 0.0, 0.0));
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<MinimapNavigationState>()
            .movement_target = None;
        if dead {
            app.world_mut().get_mut::<CombatStats>(player).unwrap().hp = 0.0;
        } else {
            app.world_mut()
                .entity_mut(player)
                .remove::<MovementTarget>();
        }
        app.update();
        assert!(app.world().get::<MovementRoute>(player).is_none());
        assert!(app.world().get::<MovementTarget>(player).is_none());
    }
}

#[test]
fn target_minimap_and_ui_presses_never_leak_ground_movement() {
    assert!(should_issue_ground_move(false, false, false));
    assert!(!should_issue_ground_move(true, false, false));
    assert!(!should_issue_ground_move(false, true, false));
    assert!(!should_issue_ground_move(false, false, true));
}
