use super::cast::{PendingCastRequest, queue_cast_request, within_cast_range};
use super::hotbar::{DesktopSkillIcon, SKILL_SLOT_SIZE, SkillRankLabel, update_skill_tooltip};
use super::mobile::{mobile_assisted_target, mobile_target_score};
use super::selection::{
    BASE_TOWER_PICK_RADIUS_PX, MINION_PICK_RADIUS_PX, NEUTRAL_PICK_RADIUS_PX,
    PLAYER_PICK_RADIUS_PX, TOWER_PICK_RADIUS_PX, find_nearest_enemy_target,
    find_target_near_screen, screen_pick_distance,
};
use super::*;
use crate::camera::MainCamera;
use crate::input_context::GameplayInputContext;
use crate::net::{
    GameState, GameStateSnapshot, NetworkCommand, NetworkHeroClass, NetworkMinionId,
    NetworkPlayerId, NetworkStructure, NetworkStructureId, PlayerProgression, RemotePlayer,
    StructureKind, TargetId, TargetKind,
};
use crate::player::{MovementTarget, Player};
use crate::sprite::PlayerVisualMode;
use crate::team::{Team, TeamSelection};
use bevy::{input::mouse::MouseButton, window::PrimaryWindow};
use shared::{HeroClass, SkillSlot, ability_for_class_slot};

fn standard_cast_app(class: HeroClass, slot: usize, recast: bool) -> App {
    let mut app = App::new();
    app.init_resource::<TeamSelection>()
        .init_resource::<PendingCast>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<ActionFeedback>()
        .init_resource::<GameplayInputContext>()
        .add_message::<NetworkCommand>()
        .add_systems(Update, resolve_pending_cast_system);
    let mut state = shared::loadout::LoadoutState {
        recipe: shared::loadout::preset_for_class(class).map(|p| p.recipe()),
        ..default()
    };
    state.slots[slot].can_recast = recast;
    state.slots[slot].recast_remaining_secs = if recast { 3.0 } else { 0.0 };
    let mut stats = CombatStats::default();
    if recast {
        stats.mana = 0.0;
    }
    app.world_mut().spawn((
        Player,
        Transform::default(),
        stats,
        PlayerProgression {
            level: 10,
            ..default()
        },
        NetworkPlayerId(1),
        Team::Green,
        NetworkHeroClass(class),
        crate::net::PlayerLoadout(Some(state)),
    ));
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot,
        target_entity: None,
        target: None,
        approach_announced: false,
    });
    app.world_mut().resource_mut::<PendingCast>().aim = Some(Vec2::new(30.0, 0.0));
    if recast {
        app.world_mut()
            .resource_mut::<LocalCastCooldown>()
            .remaining_secs[slot] = 8.0;
    }
    app
}

#[test]
fn standard_skills_send_world_aim_without_a_selected_unit() {
    for class in [HeroClass::Dawnweaver, HeroClass::Wildspark] {
        for slot in 0..4 {
            let mut app = standard_cast_app(class, slot, false);
            app.update();
            let sent = app
                .world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .collect::<Vec<_>>();
            assert!(
                matches!(sent.as_slice(), [NetworkCommand::CastSkill { slot: sent_slot, aim }]
                if *sent_slot as usize == slot && aim.is_finite()),
                "{class:?} {slot}: {sent:?}"
            );
            assert!(!app.world().resource::<PendingCast>().is_pending());
        }
    }
}

#[test]
fn dawn_field_recast_bypasses_mana_recovery_and_running_base_cooldown() {
    let mut app = standard_cast_app(HeroClass::Dawnweaver, 2, true);
    app.world_mut()
        .resource_mut::<LocalCastCooldown>()
        .recovery_secs = 0.5;
    app.update();
    let sent = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect::<Vec<_>>();
    assert!(matches!(
        sent.as_slice(),
        [NetworkCommand::CastSkill { slot: 2, .. }]
    ));
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs[2],
        8.0
    );
}

#[test]
fn standard_keyboard_holds_before_cast_and_cancels_when_context_is_lost() {
    for canceled in [false, true] {
        let mut app = standard_cast_app(HeroClass::Dawnweaver, 0, false);
        app.world_mut().resource_mut::<PendingCast>().cancel();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<TargetState>()
            .add_systems(
                Update,
                cast_spell_system.before(resolve_pending_cast_system),
            );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyQ);
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count(),
            0,
            "holding only previews"
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        if canceled {
            app.world_mut()
                .resource_mut::<GameplayInputContext>()
                .modal_open = true;
            app.update();
            app.world_mut()
                .resource_mut::<GameplayInputContext>()
                .modal_open = false;
        }
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyQ);
        app.update();
        let sent = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect::<Vec<_>>();
        if canceled {
            assert!(
                sent.is_empty(),
                "a canceled hold cannot cast after the menu closes"
            );
        } else {
            assert!(matches!(
                sent.as_slice(),
                [NetworkCommand::CastSkill { slot: 0, .. }]
            ));
        }
    }
}

#[test]
fn standard_cast_predicts_recovery_and_preserves_the_next_aimed_input_until_ready() {
    let mut app = standard_cast_app(HeroClass::Dawnweaver, 0, false);
    app.init_resource::<Time>().add_systems(
        Update,
        tick_local_cast_cooldown.before(resolve_pending_cast_system),
    );
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        1
    );
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().recovery_secs,
        0.15
    );
    {
        let mut pending = app.world_mut().resource_mut::<PendingCast>();
        pending.request = Some(PendingCastRequest {
            slot: 1,
            target_entity: None,
            target: None,
            approach_announced: false,
        });
        pending.aim = Some(Vec2::new(4.0, 5.0));
    }
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(100));
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
    assert!(app.world().resource::<PendingCast>().is_pending());
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(60));
    app.update();
    let sent = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect::<Vec<_>>();
    assert!(
        matches!(sent.as_slice(), [NetworkCommand::CastSkill {slot:1,aim}] if *aim==Vec2::new(4.0,5.0))
    );
    assert!(!app.world().resource::<PendingCast>().is_pending());
}

#[test]
fn balanced_skill_recovery_buffers_next_slot_and_uses_level_cooldowns() {
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<TeamSelection>()
        .init_resource::<PendingCast>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<ActionFeedback>()
        .init_resource::<GameplayInputContext>()
        .add_message::<NetworkCommand>()
        .add_systems(
            Update,
            (tick_local_cast_cooldown, resolve_pending_cast_system).chain(),
        );
    app.world_mut().spawn((
        Player,
        Transform::default(),
        CombatStats::default(),
        PlayerProgression {
            level: 10,
            ..default()
        },
        NetworkPlayerId(1),
        Team::Green,
        NetworkHeroClass(HeroClass::Warrior),
    ));
    let enemy = app
        .world_mut()
        .spawn((
            Transform::from_xyz(2.0, 0.0, 0.0),
            CombatStats::default(),
            Team::Blue,
            NetworkPlayerId(2),
        ))
        .id();
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot: 0,
        target_entity: Some(enemy),
        target: Some(TargetId {
            kind: TargetKind::Player,
            id: 2,
        }),
        approach_announced: false,
    });
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        1
    );
    let cd = app.world().resource::<LocalCastCooldown>();
    assert!((cd.remaining_secs[0] - 1.25).abs() < 0.001); // 2s / 1.6 growth
    assert!((cd.recovery_secs - 0.3).abs() < 0.001);
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot: 1,
        target_entity: None,
        target: None,
        approach_announced: false,
    });
    app.update();
    assert!(app.world().resource::<PendingCast>().request.is_some());
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(310));
    app.update();
    assert!(app.world().resource::<PendingCast>().request.is_none());
    let sent: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect();
    assert!(matches!(
        sent.as_slice(),
        [NetworkCommand::Cast { slot: 1, .. }]
    ));
}

#[test]
fn authoritative_skill_deadlines_restore_after_reconnect_and_age_between_snapshots() {
    use crate::net::{PlayerEquipment, PlayerSkillCooldowns};
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<LocalCastCooldown>()
        .add_systems(
            Update,
            (
                tick_local_cast_cooldown,
                sync_authoritative_cooldown_durations,
            )
                .chain(),
        );
    let hero = app
        .world_mut()
        .spawn((
            Player,
            PlayerProgression {
                level: 10,
                ..default()
            },
            NetworkHeroClass(HeroClass::Warrior),
            PlayerEquipment::default(),
            PlayerSkillCooldowns {
                remaining_secs: [0.8, 2.0, 0.0, 10.0],
                recovery_secs: 0.2,
            },
        ))
        .id();
    app.update();
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs,
        [0.8, 2.0, 0.0, 10.0]
    );
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(100));
    app.update();
    assert!((app.world().resource::<LocalCastCooldown>().remaining_secs[0] - 0.7).abs() < 0.001);
    assert!((app.world().resource::<LocalCastCooldown>().recovery_secs - 0.1).abs() < 0.001);
    // A fresh reset/respawn snapshot removes old deadlines immediately.
    app.world_mut()
        .entity_mut(hero)
        .insert(PlayerSkillCooldowns::default());
    app.update();
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs,
        [0.0; 4]
    );
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().recovery_secs,
        0.0
    );
}

#[test]
fn sandbox_slow_attack_rate_matches_authoritative_q_duration() {
    let bonuses = shared::shop::ItemBonuses {
        attack_speed_multiplier: 0.25,
        ..default()
    };
    let normal = effective_cast_duration(HeroClass::Warrior, 1, 1, SkillSlot::Q, bonuses, false);
    assert_eq!(
        effective_cast_duration(HeroClass::Warrior, 1, 1, SkillSlot::Q, bonuses, true),
        normal * 4.0
    );
    assert_eq!(
        effective_cast_duration(HeroClass::Warrior, 1, 1, SkillSlot::W, bonuses, true),
        effective_cast_duration(HeroClass::Warrior, 1, 1, SkillSlot::W, bonuses, false)
    );
}

#[test]
fn utility_intents_obey_snapshot_cooldowns_and_clear_on_modal_death_or_focus_loss() {
    use shared::utility::{UtilityAction, UtilityState};
    for gate in 0..5 {
        let mut app = App::new();
        let mut mobile = crate::mobile_controls::MobileControls::default();
        mobile.enabled = true;
        mobile.utilities = vec![(UtilityAction::Dash, None), (UtilityAction::Haste, None)];
        if gate == 4 {
            mobile.focused = false;
        }
        app.insert_resource(mobile)
            .insert_resource(GameplayInputContext {
                modal_open: gate == 1,
                ..default()
            })
            .insert_resource(PlayerVisualMode::Models3d)
            .add_message::<NetworkCommand>()
            .add_systems(Update, mobile_utility_system);
        app.world_mut().spawn((
            Player,
            Transform::default(),
            CombatStats {
                hp: if gate == 2 { 0.0 } else { 100.0 },
                ..default()
            },
            crate::net::PlayerUtility {
                state: UtilityState {
                    dash_remaining_secs: if gate == 3 { 5.0 } else { 0.0 },
                    haste_remaining_secs: if gate == 3 { 8.0 } else { 0.0 },
                    ..default()
                },
            },
        ));
        app.update();
        let sent: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect();
        assert!(
            app.world()
                .resource::<crate::mobile_controls::MobileControls>()
                .utilities
                .is_empty()
        );
        if gate == 0 {
            assert!(
                matches!(sent.as_slice(), [NetworkCommand::Utility { action: UtilityAction::Dash, direction }, NetworkCommand::Utility { action: UtilityAction::Haste, .. }] if *direction == Vec2::NEG_Y)
            );
        } else {
            assert!(sent.is_empty());
        }
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count(),
            0
        );
    }
}

#[test]
fn hidden_and_protected_nearest_candidates_do_not_mask_visible_targets() {
    use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
    let mut app = App::new();
    let hidden = app
        .world_mut()
        .spawn((
            RemotePlayer,
            Transform::from_xyz(0.1, 0.0, 0.0),
            Team::Blue,
            NetworkPlayerId(2),
            CombatStats::default(),
            InheritedVisibility::HIDDEN,
        ))
        .id();
    app.world_mut().spawn((
        NetworkStructure,
        Transform::from_xyz(0.05, 0.0, 0.0),
        Team::Blue,
        NetworkStructureId(3),
        StructureKind::BaseTower,
        CombatStats::default(),
        crate::net::NetworkStructureProtected(true),
    ));
    let visible = app
        .world_mut()
        .spawn((
            RemotePlayer,
            Transform::from_xyz(0.12, 0.0, 0.0),
            Team::Blue,
            NetworkPlayerId(4),
            CombatStats::default(),
            InheritedVisibility::VISIBLE,
        ))
        .id();
    let camera = Camera {
        computed: ComputedCameraValues {
            clip_from_view: Mat4::IDENTITY,
            target_info: Some(RenderTargetInfo {
                physical_size: UVec2::new(800, 400),
                scale_factor: 1.0,
            }),
            ..default()
        },
        ..default()
    };
    let mut params = bevy::ecs::system::SystemState::<(
        TargetCandidates,
        crate::targeting::TargetValidity,
    )>::new(app.world_mut());
    let (candidates, validity) = params.get(app.world());
    let expected = Some((
        visible,
        TargetId {
            kind: TargetKind::Player,
            id: 4,
        },
    ));
    assert_eq!(
        find_nearest_enemy_target(
            Vec3::ZERO,
            Team::Green,
            &validity,
            &candidates.players,
            &candidates.minions,
            &candidates.neutrals,
            &candidates.structures
        ),
        expected
    );
    assert_eq!(
        find_target_near_screen(
            Vec2::new(440.0, 200.0),
            &camera,
            &GlobalTransform::IDENTITY,
            PlayerVisualMode::Models3d,
            Team::Green,
            &validity,
            &candidates.players,
            &candidates.minions,
            &candidates.neutrals,
            &candidates.structures
        ),
        expected
    );
    assert_eq!(
        mobile_assisted_target(
            Vec3::ZERO,
            Team::Green,
            10.0,
            None,
            &candidates,
            &validity,
            &camera,
            &GlobalTransform::IDENTITY,
            PlayerVisualMode::Models3d,
            Some(hidden)
        ),
        expected
    );
}

#[test]
fn invalidated_pending_skill_stops_chase_without_cast_or_cooldown() {
    for invalidation in ["hidden", "friendly", "identity"] {
        for distance in [2.0, 30.0] {
            let mut app = App::new();
            app.add_message::<NetworkCommand>()
                .init_resource::<TeamSelection>()
                .init_resource::<PendingCast>()
                .init_resource::<ActionFeedback>()
                .init_resource::<LocalCastCooldown>()
                .init_resource::<GameplayInputContext>()
                .add_systems(Update, resolve_pending_cast_system);
            let player = app
                .world_mut()
                .spawn((
                    Player,
                    Transform::default(),
                    Team::Green,
                    CombatStats::default(),
                    PlayerProgression::default(),
                    NetworkPlayerId(1),
                    NetworkHeroClass(HeroClass::Warrior),
                ))
                .id();
            let target = app
                .world_mut()
                .spawn((
                    Transform::from_xyz(30.0, 0.0, 0.0),
                    Team::Blue,
                    CombatStats::default(),
                    NetworkMinionId(77),
                    InheritedVisibility::VISIBLE,
                ))
                .id();
            app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
                slot: 0,
                target_entity: Some(target),
                target: Some(TargetId {
                    kind: TargetKind::Minion,
                    id: 77,
                }),
                approach_announced: false,
            });
            app.update();
            assert!(app.world().entity(player).contains::<MovementTarget>());
            app.world_mut()
                .entity_mut(player)
                .insert(crate::player::MovementRoute {
                    requested_target: Vec3::X * 30.0,
                    structure_revision: 1,
                    destination: Vec3::X * 30.0,
                    waypoints: vec![Vec3::X * 30.0],
                });
            app.world_mut()
                .entity_mut(target)
                .insert(Transform::from_xyz(distance, 0.0, 0.0));
            match invalidation {
                "hidden" => {
                    app.world_mut()
                        .entity_mut(target)
                        .insert(InheritedVisibility::HIDDEN);
                }
                "friendly" => {
                    app.world_mut().entity_mut(target).insert(Team::Green);
                }
                _ => {
                    app.world_mut()
                        .entity_mut(target)
                        .insert(NetworkMinionId(78));
                }
            }
            app.update();
            assert!(app.world().resource::<PendingCast>().request.is_none());
            assert!(!app.world().entity(player).contains::<MovementTarget>());
            assert!(
                !app.world()
                    .entity(player)
                    .contains::<crate::player::MovementRoute>()
            );
            assert_eq!(
                app.world().resource::<LocalCastCooldown>().remaining_secs,
                [0.0; 4]
            );
            assert!(
                app.world_mut()
                    .resource_mut::<Messages<NetworkCommand>>()
                    .drain()
                    .next()
                    .is_none(),
                "{invalidation} target at {distance} must not receive a queued skill"
            );
            assert!(
                app.world()
                    .resource::<ActionFeedback>()
                    .text
                    .contains("visible hostile")
            );
        }
    }
}

#[test]
fn desktop_mouse_select_attack_ground_and_ui_are_distinct() {
    use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
    let mut app = App::new();
    app.init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<GameplayInputContext>()
        .init_resource::<TargetState>()
        .init_resource::<PendingCast>()
        .init_resource::<WorldPointerState>()
        .init_resource::<BasicAttackState>()
        .insert_resource(PlayerVisualMode::Models3d)
        .add_systems(Update, select_target_system);
    let mut window = Window {
        resolution: bevy::window::WindowResolution::new(800, 400),
        ..default()
    };
    window.set_cursor_position(Some(Vec2::new(440.0, 200.0)));
    let window = app.world_mut().spawn((window, PrimaryWindow)).id();
    app.world_mut()
        .spawn((Player, Transform::default(), Team::Green));
    let enemy = app
        .world_mut()
        .spawn((
            RemotePlayer,
            Transform::from_xyz(0.1, 0.0, 0.0),
            Team::Blue,
            NetworkPlayerId(2),
            CombatStats::default(),
        ))
        .id();
    app.world_mut().spawn((
        MainCamera,
        GlobalTransform::IDENTITY,
        Camera {
            computed: ComputedCameraValues {
                clip_from_view: Mat4::IDENTITY,
                target_info: Some(RenderTargetInfo {
                    physical_size: UVec2::new(800, 400),
                    scale_factor: 1.0,
                }),
                ..default()
            },
            ..default()
        },
    ));
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    assert_eq!(
        app.world().resource::<TargetState>().selected_entity,
        Some(enemy)
    );
    assert!(app.world().resource::<BasicAttackState>().order.is_none());
    assert!(!app.world().resource::<PendingCast>().is_pending());
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Right);
    app.update();
    let attack = app.world().resource::<BasicAttackState>().order.unwrap();
    assert_eq!(attack.entity, enemy);
    assert_eq!(attack.target.id, 2);
    assert!(attack.repeat);
    assert!(
        app.world()
            .resource::<WorldPointerState>()
            .consumed_secondary_press
    );
    assert!(!app.world().resource::<PendingCast>().is_pending());
    app.world_mut().resource_mut::<BasicAttackState>().cancel();
    let ui = app.world_mut().spawn((Button, Interaction::Hovered)).id();
    app.update();
    assert!(app.world().resource::<BasicAttackState>().order.is_none());
    app.world_mut().despawn(ui);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::AltLeft);
    app.update();
    assert!(app.world().resource::<BasicAttackState>().order.is_none());
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(Vec2::new(700.0, 100.0)));
    app.update();
    assert!(
        !app.world()
            .resource::<WorldPointerState>()
            .consumed_secondary_press
    );
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    assert!(
        app.world()
            .resource::<TargetState>()
            .selected_entity
            .is_none()
    );
}

#[test]
fn directional_mobile_assist_respects_range_and_does_not_snap_behind_aim() {
    assert!(mobile_target_score(6.0, 5.0, Vec2::X, Some(Vec2::X), false).is_none());
    assert!(mobile_target_score(3.0, 5.0, Vec2::NEG_X, Some(Vec2::X), true).is_none());
    assert!(mobile_target_score(3.0, 5.0, Vec2::Y, Some(Vec2::X), false).is_none());
    let forward = mobile_target_score(3.0, 5.0, Vec2::X, Some(Vec2::X), false).unwrap();
    let edge = mobile_target_score(3.0, 5.0, Vec2::new(1.0, 0.5), Some(Vec2::X), false).unwrap();
    assert!(forward < edge);
    assert!(
        mobile_target_score(4.0, 5.0, Vec2::X, None, true)
            < mobile_target_score(1.0, 5.0, Vec2::X, None, false)
    );
}

#[test]
fn mobile_pending_cast_in_range_emits_and_out_of_range_never_starts_a_chase() {
    let mut app = App::new();
    let mut mobile = crate::mobile_controls::MobileControls::default();
    mobile.enabled = true;
    app.insert_resource(mobile)
        .add_message::<NetworkCommand>()
        .init_resource::<TeamSelection>()
        .init_resource::<PendingCast>()
        .init_resource::<ActionFeedback>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<GameplayInputContext>()
        .add_systems(Update, resolve_pending_cast_system);
    let player = app
        .world_mut()
        .spawn((
            Player,
            Transform::default(),
            CombatStats::default(),
            PlayerProgression::default(),
            NetworkHeroClass(HeroClass::Warrior),
            NetworkPlayerId(1),
            Team::Green,
        ))
        .id();
    let enemy = app
        .world_mut()
        .spawn((
            Transform::from_xyz(100.0, 0.0, 0.0),
            CombatStats::default(),
            Team::Blue,
            NetworkPlayerId(2),
        ))
        .id();
    let request = PendingCastRequest {
        slot: 0,
        target_entity: Some(enemy),
        target: Some(TargetId {
            kind: TargetKind::Player,
            id: 2,
        }),
        approach_announced: false,
    };
    app.world_mut().resource_mut::<PendingCast>().request = Some(request);
    app.update();
    assert!(app.world().resource::<PendingCast>().request.is_none());
    assert!(!app.world().entity(player).contains::<MovementTarget>());
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs,
        [0.0; 4]
    );
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
    app.world_mut()
        .entity_mut(enemy)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::X;
    app.world_mut().resource_mut::<PendingCast>().request = Some(request);
    app.update();
    let emitted: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect();
    assert_eq!(emitted.len(), 1);
    assert!(matches!(emitted[0], NetworkCommand::Cast { slot: 0, .. }));
    assert!(app.world().resource::<LocalCastCooldown>().remaining_secs[0] > 0.0);
    assert!(!app.world().entity(player).contains::<MovementTarget>());
}

#[test]
fn pointer_hit_areas_are_touch_sized_and_screen_bounded() {
    const {
        assert!(SKILL_SLOT_SIZE >= 48.0);
    }
    for radius in [
        PLAYER_PICK_RADIUS_PX,
        MINION_PICK_RADIUS_PX,
        NEUTRAL_PICK_RADIUS_PX,
        TOWER_PICK_RADIUS_PX,
        BASE_TOWER_PICK_RADIUS_PX,
    ] {
        assert!(radius >= 48.0);
        assert!(screen_pick_distance(Vec2::ZERO, Vec2::new(radius, 0.0), radius).is_some());
        assert!(screen_pick_distance(Vec2::ZERO, Vec2::new(radius + 0.1, 0.0), radius).is_none());
    }
}

#[test]
fn explicit_q_skill_request_preserves_the_exact_authoritative_target() {
    let entity = Entity::PLACEHOLDER;
    let target = TargetId {
        kind: TargetKind::Minion,
        id: 77,
    };
    let state = TargetState {
        selected_entity: Some(entity),
        selected_target: Some(target),
        marker_entity: None,
    };
    let mut pending = PendingCast::default();
    let mut feedback = ActionFeedback::default();
    queue_cast_request(
        SkillSlot::Q.index(),
        &crate::equipped_skills::resolve(HeroClass::Warrior, None).unwrap(),
        &state,
        &mut pending,
        &mut feedback,
    );
    assert_eq!(
        pending.request,
        Some(PendingCastRequest {
            slot: SkillSlot::Q.index(),
            target_entity: Some(entity),
            target: Some(target),
            approach_announced: false,
        })
    );
}

#[test]
fn cast_range_uses_horizontal_gameplay_distance() {
    assert!(within_cast_range(
        Vec3::ZERO,
        Vec3::new(3.0, 99.0, 4.0),
        5.0
    ));
    assert!(!within_cast_range(
        Vec3::ZERO,
        Vec3::new(3.01, 0.0, 4.0),
        5.0
    ));
}

#[test]
fn pending_unit_cast_approaches_then_emits_once_in_range() {
    let mut app = App::new();
    app.add_message::<NetworkCommand>()
        .insert_resource(TeamSelection::default())
        .insert_resource(PendingCast::default())
        .insert_resource(LocalCastCooldown::default())
        .insert_resource(ActionFeedback::default())
        .init_resource::<GameplayInputContext>()
        .add_systems(Update, resolve_pending_cast_system);

    let player = app
        .world_mut()
        .spawn((
            Player,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CombatStats::default(),
            PlayerProgression::default(),
            NetworkPlayerId(1),
            Team::Green,
            NetworkHeroClass(HeroClass::Warrior),
        ))
        .id();
    let target = app
        .world_mut()
        .spawn((
            Transform::from_xyz(30.0, 0.0, 0.0),
            CombatStats::default(),
            Team::Blue,
            NetworkMinionId(77),
        ))
        .id();
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot: SkillSlot::Q.index(),
        target_entity: Some(target),
        target: Some(TargetId {
            kind: TargetKind::Minion,
            id: 77,
        }),
        approach_announced: false,
    });

    app.update();
    assert!(app.world().entity(player).contains::<MovementTarget>());
    assert!(app.world().resource::<PendingCast>().request.is_some());
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs[0],
        0.0
    );
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );

    app.world_mut()
        .entity_mut(player)
        .insert(Transform::from_xyz(20.0, 0.0, 0.0));
    app.update();
    assert!(app.world().resource::<PendingCast>().request.is_none());
    assert!(app.world().resource::<LocalCastCooldown>().remaining_secs[0] > 0.0);
    let commands: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect();
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        commands[0],
        NetworkCommand::Cast {
            target: TargetId {
                kind: TargetKind::Minion,
                id: 77
            },
            slot: 0
        }
    ));
}

#[test]
fn insufficient_mana_rejects_without_phantom_cooldown() {
    let mut app = App::new();
    app.add_message::<NetworkCommand>()
        .insert_resource(TeamSelection::default())
        .insert_resource(PendingCast::default())
        .insert_resource(LocalCastCooldown::default())
        .insert_resource(ActionFeedback::default())
        .init_resource::<GameplayInputContext>()
        .add_systems(Update, resolve_pending_cast_system);

    let exhausted = CombatStats {
        mana: 0.0,
        ..default()
    };
    app.world_mut().spawn((
        Player,
        Transform::default(),
        exhausted,
        PlayerProgression::default(),
        NetworkPlayerId(1),
        Team::Green,
        NetworkHeroClass(HeroClass::Warrior),
    ));
    let target = app
        .world_mut()
        .spawn((
            Transform::from_xyz(2.0, 0.0, 0.0),
            CombatStats::default(),
            Team::Blue,
            NetworkMinionId(88),
        ))
        .id();
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot: SkillSlot::Q.index(),
        target_entity: Some(target),
        target: Some(TargetId {
            kind: TargetKind::Minion,
            id: 88,
        }),
        approach_announced: false,
    });

    app.update();
    assert!(app.world().resource::<PendingCast>().request.is_none());
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs[0],
        0.0
    );
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
}

#[test]
fn self_target_hotbar_request_needs_no_selected_enemy() {
    let state = TargetState::default();
    let mut pending = PendingCast::default();
    let mut feedback = ActionFeedback::default();
    queue_cast_request(
        SkillSlot::W.index(),
        &crate::equipped_skills::resolve(HeroClass::Warrior, None).unwrap(),
        &state,
        &mut pending,
        &mut feedback,
    );
    assert_eq!(
        pending.request,
        Some(PendingCastRequest {
            slot: SkillSlot::W.index(),
            target_entity: None,
            target: None,
            approach_announced: false,
        })
    );
}

#[test]
fn actual_cast_and_upgrade_systems_obey_help_pause_and_debug_context() {
    let mut app = App::new();
    app.add_message::<NetworkCommand>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<TeamSelection>()
        .init_resource::<PendingCast>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>()
        .init_resource::<TargetAimPreview>()
        .init_resource::<ActionFeedback>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<crate::pause_menu::PauseMenuState>()
        .add_message::<crate::ui::Activated<super::hotbar::HotbarAction>>()
        .insert_resource(GameStateSnapshot {
            state: GameState::Running,
            ..default()
        })
        .add_plugins((
            crate::input_context::InputContextPlugin,
            crate::help_overlay::HelpOverlayPlugin,
        ))
        .add_systems(
            Update,
            (
                cast_spell_system,
                skill_upgrade_input_system,
                resolve_pending_cast_system,
            )
                .chain()
                .in_set(InputContextSet::Actions),
        );
    app.world_mut().spawn((
        Player,
        Transform::default(),
        CombatStats::default(),
        PlayerProgression {
            level: 6,
            skill_points: 2,
            ..default()
        },
        NetworkHeroClass(HeroClass::Cleric),
        NetworkPlayerId(1),
        Team::Green,
    ));
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyU);
    app.update(); // Automatic first-match help must suppress these same-frame keys.
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
    assert!(app.world().resource::<GameplayInputContext>().modal_open);
    app.world_mut()
        .resource_mut::<crate::help_overlay::HelpOverlayVisible>()
        .0 = false;
    app.world_mut()
        .resource_mut::<crate::pause_menu::PauseMenuState>()
        .open = true;
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
    app.world_mut()
        .resource_mut::<crate::pause_menu::PauseMenuState>()
        .open = false;
    let mut debug = crate::debug::DebugConsole::default();
    debug.ui_enabled = true;
    app.insert_resource(debug);
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .debug_flight = true;
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .debug_flight = false;
    app.update();
    let emitted: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect();
    assert!(
        emitted
            .iter()
            .any(|command| matches!(command, NetworkCommand::Cast { slot: 1, .. }))
    );
    assert!(
        emitted
            .iter()
            .any(|command| matches!(command, NetworkCommand::UpgradeSkill { .. }))
    );
}

#[test]
fn protected_base_rejection_is_visible_and_does_not_approach_or_start_cooldown() {
    let mut app = App::new();
    app.add_message::<NetworkCommand>()
        .init_resource::<TeamSelection>()
        .init_resource::<PendingCast>()
        .init_resource::<ActionFeedback>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<GameplayInputContext>()
        .add_systems(Update, resolve_pending_cast_system);
    let player = app
        .world_mut()
        .spawn((
            Player,
            Transform::default(),
            CombatStats::default(),
            PlayerProgression::default(),
            NetworkHeroClass(HeroClass::Warrior),
            NetworkPlayerId(1),
            Team::Green,
        ))
        .id();
    let base = app
        .world_mut()
        .spawn((
            Transform::from_xyz(100.0, 0.0, 0.0),
            CombatStats::default(),
            crate::net::NetworkStructureProtected(true),
            Team::Blue,
            NetworkStructureId(2),
        ))
        .id();
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot: 0,
        target_entity: Some(base),
        target: Some(TargetId {
            kind: TargetKind::Structure,
            id: 2,
        }),
        approach_announced: false,
    });
    app.update();
    assert!(
        app.world()
            .resource::<ActionFeedback>()
            .text
            .contains("Structure protected")
    );
    assert!(app.world().resource::<PendingCast>().request.is_none());
    assert!(!app.world().entity(player).contains::<MovementTarget>());
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs,
        [0.0; 4]
    );
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
}

#[test]
fn desktop_skill_art_tracks_class_loading_and_unavailable_state() {
    let mut app = App::new();
    app.init_resource::<TeamSelection>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<TargetState>()
        .init_resource::<PendingCast>()
        .init_resource::<Assets<Image>>()
        .add_systems(Update, update_skill_bar_system);
    let player = app
        .world_mut()
        .spawn((
            Player,
            Transform::default(),
            CombatStats::default(),
            PlayerProgression::default(),
            NetworkHeroClass(HeroClass::Mage),
        ))
        .id();
    let icon = app
        .world_mut()
        .spawn((
            DesktopSkillIcon { slot: 0 },
            ImageNode::default(),
            Node::default(),
        ))
        .id();
    let locked = app
        .world_mut()
        .spawn((
            DesktopSkillIcon { slot: 3 },
            ImageNode::default(),
            Node::default(),
        ))
        .id();
    app.update();
    assert_eq!(
        app.world().get::<Node>(icon).unwrap().display,
        Display::None
    );
    let atlas = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::default());
    for entity in [icon, locked] {
        app.world_mut().get_mut::<ImageNode>(entity).unwrap().image = atlas.clone();
    }
    app.update();
    let mage_rect = app.world().get::<ImageNode>(icon).unwrap().rect.unwrap();
    assert_eq!(
        app.world().get::<Node>(icon).unwrap().display,
        Display::Flex
    );
    assert_eq!(
        app.world().get::<ImageNode>(icon).unwrap().color,
        Color::WHITE
    );
    assert_ne!(
        app.world().get::<ImageNode>(locked).unwrap().color,
        Color::WHITE
    );
    app.world_mut()
        .get_mut::<NetworkHeroClass>(player)
        .unwrap()
        .0 = HeroClass::Ranger;
    app.update();
    assert_ne!(
        app.world().get::<ImageNode>(icon).unwrap().rect,
        Some(mage_rect)
    );
    app.world_mut()
        .resource_mut::<LocalCastCooldown>()
        .remaining_secs[0] = 2.0;
    app.update();
    assert_ne!(
        app.world().get::<ImageNode>(icon).unwrap().color,
        Color::WHITE
    );
    app.world_mut()
        .resource_mut::<LocalCastCooldown>()
        .remaining_secs[0] = 0.0;
    app.world_mut().get_mut::<CombatStats>(player).unwrap().mana = 0.0;
    app.update();
    assert_ne!(
        app.world().get::<ImageNode>(icon).unwrap().color,
        Color::WHITE
    );
    assert_eq!(
        app.world().get::<Node>(icon).unwrap().display,
        Display::Flex
    );
}

#[test]
fn feedback_expires_in_place_and_hotbar_shows_server_rank_lock_mana_and_cooldown() {
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<ActionFeedback>()
        .init_resource::<TeamSelection>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>()
        .init_resource::<TargetAimPreview>()
        .init_resource::<PendingCast>()
        .add_systems(
            Startup,
            (setup_combat_ui, crate::targeting::setup_targeting_ui),
        )
        .add_systems(Update, (update_action_feedback, update_skill_bar_system));
    let player = app
        .world_mut()
        .spawn((
            Player,
            Transform::default(),
            CombatStats {
                mana: 0.0,
                ..default()
            },
            PlayerProgression {
                level: 2,
                ranks: [2, 1, 1, 1],
                ..default()
            },
            NetworkHeroClass(HeroClass::Cleric),
        ))
        .id();
    app.world_mut()
        .resource_mut::<LocalCastCooldown>()
        .remaining_secs[0] = 1.5;
    app.world_mut()
        .resource_mut::<ActionFeedback>()
        .push_line("Not enough mana.");
    app.update();
    let mut labels = app.world_mut().query::<(&SkillRankLabel, &Text)>();
    let text: Vec<_> = labels
        .iter(app.world())
        .map(|(slot, text)| (slot.slot, text.0.clone()))
        .collect();
    assert!(
        text.iter()
            .any(|(slot, text)| *slot == 0 && text.contains("R2") && text.contains("1.5s"))
    );
    assert!(
        text.iter()
            .any(|(slot, text)| *slot == 1 && text.contains("Need mana"))
    );
    assert!(
        text.iter()
            .any(|(slot, text)| *slot == 3 && text.contains("Locked Lv 6"))
    );
    let count = app.world().entities().len();
    for _ in 0..5 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(1));
        app.update();
    }
    assert!(app.world().resource::<ActionFeedback>().text.is_empty());
    assert_eq!(app.world().entities().len(), count);
    assert!(app.world().entity(player).contains::<CombatStats>());
}

#[test]
fn target_selection_keys_obey_modal_context_at_the_ecs_boundary() {
    let mut app = App::new();
    app.init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<Touches>()
        .init_resource::<GameplayInputContext>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>()
        .init_resource::<TargetAimPreview>()
        .init_resource::<PendingCast>()
        .init_resource::<WorldPointerState>()
        .insert_resource(PlayerVisualMode::Models3d)
        .add_systems(Update, select_target_system);
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    app.world_mut()
        .spawn((Player, Transform::default(), Team::Green));
    let enemy = app
        .world_mut()
        .spawn((
            RemotePlayer,
            Transform::from_xyz(2.0, 0.0, 0.0),
            Team::Blue,
            NetworkPlayerId(2),
            CombatStats::default(),
        ))
        .id();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Tab);
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .modal_open = true;
    app.update();
    assert!(
        app.world()
            .resource::<TargetState>()
            .selected_entity
            .is_none()
    );
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .modal_open = false;
    app.update();
    assert_eq!(
        app.world().resource::<TargetState>().selected_entity,
        Some(enemy)
    );
}

#[test]
fn round_change_event_clears_old_intents_cooldowns_and_queued_casts_but_reconnect_events_do_not() {
    use crate::domain::RoundId;
    use crate::net::SessionEvent;
    let mut app = App::new();
    app.add_message::<NetworkCommand>()
        .add_message::<SessionEvent>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>()
        .init_resource::<TargetAimPreview>()
        .init_resource::<PendingCast>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<ActionFeedback>()
        .add_systems(Update, reset_round_input_state);
    app.update();
    let actor = app
        .world_mut()
        .spawn(MovementTarget {
            target: Vec3::X * 50.0,
        })
        .id();
    app.world_mut()
        .resource_mut::<TargetState>()
        .selected_entity = Some(actor);
    app.world_mut()
        .resource_mut::<LocalCastCooldown>()
        .remaining_secs[3] = 40.0;
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot: 3,
        target_entity: Some(actor),
        target: None,
        approach_announced: true,
    });
    // A reconnect to the same round after a teardown: `net` announces no
    // `RoundChanged` for it (pinned in `net::apply`), so nothing resets.
    app.world_mut()
        .write_message(SessionEvent::TransportStarted {
            addr: "127.0.0.1:4000".into(),
            offline: false,
        });
    app.update();
    app.world_mut().write_message(SessionEvent::Connected);
    app.world_mut()
        .write_message(SessionEvent::Joined { your_id: 1 });
    app.update();
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs[3],
        40.0
    );
    assert!(app.world().entity(actor).contains::<MovementTarget>());
    assert!(app.world().resource::<PendingCast>().request.is_some());
    app.world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .write(NetworkCommand::Cast {
            target: TargetId {
                kind: TargetKind::Player,
                id: 2,
            },
            slot: 3,
        });
    app.world_mut().write_message(SessionEvent::RoundChanged {
        previous: RoundId {
            server_epoch: 10,
            match_id: 1,
        },
        current: RoundId {
            server_epoch: 10,
            match_id: 2,
        },
    });
    app.update();
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs,
        [0.0; 4]
    );
    assert!(app.world().resource::<PendingCast>().request.is_none());
    assert!(
        app.world()
            .resource::<TargetState>()
            .selected_entity
            .is_none()
    );
    assert!(!app.world().entity(actor).contains::<MovementTarget>());
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
}
#[test]
fn buying_haste_adjusts_active_deadlines_without_rescaling_elapsed_time() {
    use crate::net::PlayerEquipment;
    use shared::shop::{ItemId, item_bonuses, item_cooldown};
    let mut app = App::new();
    app.init_resource::<LocalCastCooldown>()
        .add_systems(Update, sync_authoritative_cooldown_durations);
    let hero = app
        .world_mut()
        .spawn((
            Player,
            PlayerProgression::default(),
            NetworkHeroClass(HeroClass::Mage),
            PlayerEquipment::default(),
        ))
        .id();
    app.update();
    let old = app.world().resource::<LocalCastCooldown>().total_secs;
    for (index, duration) in old.iter().enumerate() {
        app.world_mut()
            .resource_mut::<LocalCastCooldown>()
            .remaining_secs[index] = *duration * 0.5;
    }
    let bonuses = item_bonuses(&[ItemId::FocusCharm, ItemId::SwiftGrip]);
    app.world_mut().entity_mut(hero).insert(PlayerEquipment {
        item_bonuses: bonuses,
        ..default()
    });
    app.update();
    for slot in SkillSlot::ALL {
        let index = slot.index();
        let current = item_cooldown(
            ability_for_class_slot(HeroClass::Mage, slot),
            1,
            slot,
            bonuses,
        )
        .as_secs_f32();
        let expected = (old[index] * 0.5 + current - old[index]).max(0.0);
        assert!(
            (app.world().resource::<LocalCastCooldown>().remaining_secs[index] - expected).abs()
                < 0.0001
        );
        assert!(
            expected < current * 0.5,
            "elapsed time is preserved, not scaled"
        );
    }
    // A second identical authoritative snapshot cannot repeatedly shorten it.
    let once = app.world().resource::<LocalCastCooldown>().remaining_secs;
    app.update();
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs,
        once
    );
}

/// DECISIONS R7.1: a protected enemy structure stays selected for inspection
/// (the target plate shows it with a lock) while every attack order on it is
/// dropped; a dead or unprotected-but-invalid target still clears.
#[test]
fn protected_structures_stay_selected_for_inspection_but_never_keep_an_attack_order() {
    let mut app = App::new();
    app.init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<GameplayInputContext>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>()
        .init_resource::<PendingCast>()
        .init_resource::<TargetAimPreview>()
        .add_systems(Update, crate::targeting::clear_invalid_selection);
    app.world_mut().spawn((
        Player,
        Transform::default(),
        Team::Green,
        CombatStats::default(),
    ));
    let tower = app
        .world_mut()
        .spawn((
            NetworkStructure,
            NetworkStructureId(3),
            StructureKind::Tower,
            Team::Blue,
            Transform::from_xyz(2.0, 0.0, 0.0),
            CombatStats::default(),
            crate::net::NetworkStructureProtected(true),
        ))
        .id();
    let id = TargetId {
        kind: TargetKind::Structure,
        id: 3,
    };
    {
        let mut target = app.world_mut().resource_mut::<TargetState>();
        target.selected_entity = Some(tower);
        target.selected_target = Some(id);
    }
    app.world_mut()
        .resource_mut::<BasicAttackState>()
        .start(tower, id, true);
    app.update();
    let target = app.world().resource::<TargetState>();
    assert_eq!(target.selected_entity, Some(tower), "kept for inspection");
    assert!(app.world().resource::<BasicAttackState>().order.is_none());
    // Protection drops: the selection is an ordinary target again.
    app.world_mut()
        .get_mut::<crate::net::NetworkStructureProtected>(tower)
        .unwrap()
        .0 = false;
    app.update();
    assert_eq!(
        app.world().resource::<TargetState>().selected_entity,
        Some(tower)
    );
    // Destroyed: cleared like any dead target.
    app.world_mut().get_mut::<CombatStats>(tower).unwrap().hp = 0.0;
    app.update();
    assert!(
        app.world()
            .resource::<TargetState>()
            .selected_entity
            .is_none()
    );
}

/// Skill descriptions require a deliberate hold, never hover or attack spam.
#[test]
fn skill_descriptions_require_continuous_hold_and_hide_on_release() {
    use super::skill_card::SkillCardView;
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<TeamSelection>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<TargetState>()
        .init_resource::<PendingCast>()
        .init_resource::<GameplayInputContext>()
        .init_resource::<crate::gamepad::GamepadControls>()
        .init_resource::<crate::mobile_controls::MobileControls>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<super::inspection::SkillInspection>()
        .add_systems(Startup, setup_combat_ui)
        .add_systems(
            Update,
            (
                super::inspection::update_inspection,
                update_skill_tooltip,
                super::standard::update_status,
            )
                .chain(),
        );
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    let status = app
        .world_mut()
        .spawn((
            super::standard::StandardStatus,
            Text::default(),
            Node::default(),
        ))
        .id();
    app.world_mut().spawn((
        Player,
        Transform::default(),
        CombatStats::default(),
        PlayerProgression {
            level: 4,
            ranks: [1, 1, 2, 0],
            ..default()
        },
        NetworkHeroClass(HeroClass::Dawnweaver),
        crate::net::PlayerLoadout(Some(shared::loadout::LoadoutState {
            recipe: shared::loadout::preset_for_class(HeroClass::Dawnweaver).map(|l| l.recipe()),
            ..default()
        })),
    ));
    app.update();
    // Slot E (index 2) laid out at the redline: 648..712 × 610..674.
    let slot = app
        .world_mut()
        .query::<(Entity, &crate::ui::TestId)>()
        .iter(app.world())
        .find(|(_, id)| id.0 == "SkillSlot-E")
        .map(|(entity, _)| entity)
        .unwrap();
    app.world_mut().entity_mut(slot).insert((
        ComputedNode {
            size: Vec2::splat(64.0),
            inverse_scale_factor: 1.0,
            ..default()
        },
        UiGlobalTransform::from_translation(Vec2::new(680.0, 642.0)),
        Interaction::Hovered,
    ));
    let card = |app: &mut App| {
        app.world_mut()
            .query_filtered::<(&SkillCardView, &Node), With<super::hotbar::SkillTooltip>>()
            .single(app.world())
            .map(|(view, node)| (view.clone(), node.left, node.top))
            .unwrap()
    };
    app.update();
    assert!(!card(&mut app).0.visible, "not before the delay");
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs(2));
    app.update();
    assert!(!card(&mut app).0.visible, "hover never opens descriptions");
    assert_eq!(
        app.world().get::<Node>(status).unwrap().display,
        Display::None
    );
    app.world_mut()
        .entity_mut(slot)
        .insert(Interaction::Pressed);
    app.update();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(1499));
    app.update();
    assert!(!card(&mut app).0.visible);
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(2));
    app.update();
    assert_eq!(
        app.world().get::<Node>(status).unwrap().display,
        Display::Flex
    );
    let (view, left, top) = card(&mut app);
    assert!(view.visible);
    assert_eq!((view.slot, view.rank, view.locked), (2, 2, false));
    assert_eq!(view.key.as_deref(), Some("E"));
    assert!(view.status.is_some(), "live status on desktop");
    // Centred on the slot (680 − 140) and bottom at 720 − 124 − 8 = 588
    // without the upgrade chip (no skill point).
    assert_eq!(left, Val::Px(540.0));
    assert_eq!(top, Val::Px(588.0 - super::skill_card::CARD.y));
    // Input ownership changes restart the delay, even for the same skill.
    app.world_mut()
        .resource_mut::<crate::gamepad::GamepadControls>()
        .active = true;
    app.update();
    assert!(!card(&mut app).0.visible);
    app.world_mut()
        .resource_mut::<crate::gamepad::GamepadControls>()
        .active = false;
    app.update();
    assert!(!card(&mut app).0.visible);
    app.world_mut().entity_mut(slot).insert(Interaction::None);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyE);
    app.update();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(1501));
    app.update();
    assert!(
        card(&mut app).0.visible,
        "keyboard hold describes the held skill"
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyE);
    app.update();
    assert!(!card(&mut app).0.visible);
    assert_eq!(
        app.world().get::<Node>(status).unwrap().display,
        Display::None
    );
    {
        let mut pad = app
            .world_mut()
            .resource_mut::<crate::gamepad::GamepadControls>();
        pad.active = true;
        pad.attack_held = true;
    }
    app.update();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs(3));
    app.update();
    assert!(
        !card(&mut app).0.visible,
        "basic attack holds never inspect skills"
    );
    app.world_mut()
        .resource_mut::<crate::gamepad::GamepadControls>()
        .aiming_slot = Some(2);
    app.update();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(1501));
    app.update();
    assert!(card(&mut app).0.visible, "controller skill hold inspects");
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .modal_open = true;
    app.update();
    assert!(!card(&mut app).0.visible);
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .modal_open = false;
    app.update();
    assert!(
        !card(&mut app).0.visible,
        "menu time cannot count toward a hold"
    );
}

fn equip_recipe(app: &mut App, recipe: shared::loadout::BuildRecipe, level: u32, mana: f32) {
    let mut query = app.world_mut().query_filtered::<(
        &mut crate::net::PlayerLoadout,
        &mut PlayerProgression,
        &mut CombatStats,
    ), With<Player>>();
    let (mut state, mut progression, mut stats) = query.single_mut(app.world_mut()).unwrap();
    state.0.as_mut().unwrap().recipe = Some(recipe);
    progression.level = level;
    stats.mana = mana;
}

#[test]
fn mixed_permuted_cast_prechecks_use_equipped_mana_range_unlock_and_cooldown() {
    use shared::loadout::{CoreId, SkillId};
    let mut recipe = CoreId::Dawnweaver.preset();
    recipe.skills = [
        SkillId::DawnRay,
        SkillId::DawnBarrier,
        SkillId::DawnBind,
        SkillId::WildTraps,
    ];
    let mut locked = standard_cast_app(HeroClass::Dawnweaver, 0, false);
    equip_recipe(&mut locked, recipe.clone(), 5, 1000.0);
    locked.update();
    assert_eq!(
        locked
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0,
        "ultimate on Q remains locked until level six"
    );

    let mut app = standard_cast_app(HeroClass::Dawnweaver, 3, false);
    equip_recipe(&mut app, recipe.clone(), 4, 1000.0);
    app.update();
    let definition = shared::loadout::skill(SkillId::WildTraps);
    let commands: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect();
    assert!(
        matches!(commands.as_slice(), [NetworkCommand::CastSkill { slot: 3, aim }] if (aim.length() - definition.ability.cast_range).abs() < 0.001)
    );
    let skills =
        shared::loadout::EquippedSkills::resolve(HeroClass::Dawnweaver, Some(&recipe)).unwrap();
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().total_secs[3],
        skills
            .cooldown(4, 1, SkillSlot::R, Default::default())
            .as_secs_f32()
    );

    let mut rejected = standard_cast_app(HeroClass::Dawnweaver, 3, false);
    equip_recipe(
        &mut rejected,
        recipe,
        10,
        shared::scaled_mana_cost(&definition.ability, 1) - 0.1,
    );
    rejected.update();
    assert_eq!(
        rejected
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
}

#[test]
fn malformed_equipped_recipe_cancels_cast_instead_of_substituting_preset() {
    let mut app = standard_cast_app(HeroClass::Dawnweaver, 0, false);
    let mut recipe = shared::loadout::CoreId::Dawnweaver.preset();
    recipe.skills[1] = recipe.skills[0];
    equip_recipe(&mut app, recipe, 10, 1000.0);
    app.update();
    assert!(!app.world().resource::<PendingCast>().is_pending());
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
}

#[test]
fn equipped_total_cooldown_keeps_authoritative_remaining_deadline() {
    use shared::loadout::{CoreId, LoadoutState, SkillId};
    let mut app = App::new();
    app.init_resource::<LocalCastCooldown>()
        .add_systems(Update, sync_authoritative_cooldown_durations);
    let mut recipe = CoreId::Dawnweaver.preset();
    recipe.skills.swap(0, 3);
    let equipped =
        shared::loadout::EquippedSkills::resolve(HeroClass::Dawnweaver, Some(&recipe)).unwrap();
    app.world_mut().spawn((
        Player,
        PlayerProgression {
            level: 10,
            ..default()
        },
        NetworkHeroClass(HeroClass::Dawnweaver),
        crate::net::PlayerEquipment::default(),
        crate::net::PlayerLoadout(Some(LoadoutState {
            recipe: Some(recipe),
            ..default()
        })),
        crate::net::PlayerSkillCooldowns {
            remaining_secs: [7.25, 1.0, 2.0, 3.0],
            recovery_secs: 0.2,
        },
    ));
    app.update();
    let cd = app.world().resource::<LocalCastCooldown>();
    assert_eq!(cd.remaining_secs, [7.25, 1.0, 2.0, 3.0]);
    assert_eq!(
        cd.total_secs[0],
        equipped
            .cooldown(10, 1, SkillSlot::Q, Default::default())
            .as_secs_f32()
    );
    assert_eq!(equipped.skill(SkillSlot::Q).unwrap().id, SkillId::DawnRay);
}

#[test]
fn borrowed_recast_requires_and_displays_its_own_mana_cost() {
    use shared::loadout::{CoreId, SkillId};
    for (mana, accepted) in [(24.0, false), (25.0, true)] {
        let mut app = standard_cast_app(HeroClass::Dawnweaver, 0, true);
        let mut recipe = CoreId::Dawnweaver.preset();
        recipe.skills[0] = SkillId::EchoStrike;
        let skills =
            shared::loadout::EquippedSkills::resolve(HeroClass::Dawnweaver, Some(&recipe)).unwrap();
        let card = super::skill_card::SkillCardView::of_equipped(
            &skills,
            &PlayerProgression {
                level: 10,
                ..default()
            },
            0,
            mana,
            0.0,
            true,
        );
        assert_eq!(card.mana, 25);
        assert_eq!(card.no_mana, !accepted);
        equip_recipe(&mut app, recipe, 10, mana);
        app.world_mut()
            .resource_mut::<LocalCastCooldown>()
            .recovery_secs = 0.5;
        app.update();
        let sent: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect();
        assert_eq!(sent.len(), usize::from(accepted), "mana {mana}: {sent:?}");
        if accepted {
            assert!(matches!(
                sent.as_slice(),
                [NetworkCommand::CastSkill { slot: 0, .. }]
            ));
        }
        assert_eq!(
            app.world().resource::<LocalCastCooldown>().remaining_secs[0],
            8.0,
            "follow-up does not reset its running base cooldown"
        );
    }
}
