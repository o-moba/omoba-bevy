//! Presentation regression: positions are sampled before UI layout and after
//! late movement/grounding, even while propagated globals still hold last frame.
use super::marker::TargetMarker;
use super::*;
use crate::camera::MainCamera;
use crate::net::{NetworkPlayerId, RemotePlayer, TargetId, TargetKind};
use crate::player::Player;
use crate::sprite::PlayerVisualMode;
use crate::targeting::{LockedTargetIndicator, LockedTargetLabel};
use crate::team::Team;
use bevy::camera::primitives::Aabb;
use bevy::camera::{ComputedCameraValues, RenderTargetInfo};

fn fixture(mode: PlayerVisualMode) -> (App, Entity, Entity, Entity, Entity) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin))
        .insert_resource(mode)
        .init_resource::<crate::maps::MapLayout>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>();
    configure_target_presentation(&mut app);
    app.world_mut().spawn((Player, Team::Green));
    let enemy = app
        .world_mut()
        .spawn((
            RemotePlayer,
            Team::Blue,
            NetworkPlayerId(9),
            CombatStats::default(),
            Transform::from_xyz(0.1, 0.0, 0.0),
            InheritedVisibility::VISIBLE,
        ))
        .id();
    let camera = app
        .world_mut()
        .spawn((
            MainCamera,
            Transform::IDENTITY,
            Camera {
                computed: ComputedCameraValues {
                    clip_from_view: Mat4::from_scale(Vec3::new(1.0, 1.0, 0.01)),
                    target_info: Some(RenderTargetInfo {
                        physical_size: UVec2::new(800, 400),
                        scale_factor: 1.0,
                    }),
                    ..default()
                },
                ..default()
            },
        ))
        .id();
    let marker = app
        .world_mut()
        .spawn((TargetMarker, Transform::IDENTITY, Visibility::Hidden))
        .id();
    let frame = app
        .world_mut()
        .spawn((LockedTargetIndicator, Node::default()))
        .id();
    app.world_mut().spawn((LockedTargetLabel, Text::default()));
    *app.world_mut().resource_mut::<TargetState>() = TargetState {
        selected_entity: Some(enemy),
        selected_target: Some(TargetId {
            kind: TargetKind::Player,
            id: 9,
        }),
        marker_entity: Some(marker),
    };
    (app, enemy, camera, marker, frame)
}

#[test]
fn target_presentation_uses_current_motion_and_camera_before_ui_layout() {
    for mode in [PlayerVisualMode::Models3d, PlayerVisualMode::Sprite2d] {
        let (mut app, enemy, camera, marker, frame) = fixture(mode);
        app.update();
        // Deliberately perform motion at the latest supported stage. A system
        // reading last-frame GlobalTransform or running in Update misses this.
        app.add_systems(
            PostUpdate,
            (move |mut transforms: Query<&mut Transform>| {
                transforms.get_mut(enemy).unwrap().translation.x = 0.4;
                transforms.get_mut(camera).unwrap().translation.x = 0.15;
            })
            .in_set(crate::net::NetworkGroundingSet),
        );
        app.add_systems(
            PostUpdate,
            (move |nodes: Query<&Node>| {
                // Identity projection: (0.4 - 0.15) maps to x=500 in an 800px viewport.
                assert_eq!(nodes.get(frame).unwrap().left, Val::Px(500.0 - 23.0));
            })
            .in_set(bevy::ui::UiSystems::Layout),
        );
        app.update();
        let pose = app.world().get::<GlobalTransform>(marker).unwrap();
        assert!((pose.translation().x - 0.4).abs() < 1e-6);
        assert_eq!(
            *app.world().get::<Visibility>(marker).unwrap(),
            Visibility::Visible
        );
    }
}

#[test]
fn target_presentation_ignores_animation_bounds_and_tracks_parent_motion() {
    let (mut app, enemy, _, marker, frame) = fixture(PlayerVisualMode::Models3d);
    let parent = app
        .world_mut()
        .spawn(Transform::from_xyz(0.1, 0.0, 0.0))
        .id();
    app.world_mut().entity_mut(parent).add_child(enemy);
    let animated = app
        .world_mut()
        .spawn((
            Transform::IDENTITY,
            Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0)),
        ))
        .id();
    app.world_mut().entity_mut(enemy).add_child(animated);
    app.update();
    let baseline = *app.world().get::<Transform>(marker).unwrap();
    let frame_top = app.world().get::<Node>(frame).unwrap().top;
    for i in 0..60 {
        *app.world_mut().get_mut::<Transform>(animated).unwrap() =
            Transform::from_xyz(0.0, i as f32, 0.0).with_rotation(Quat::from_rotation_z(i as f32));
        app.world_mut()
            .get_mut::<Transform>(enemy)
            .unwrap()
            .rotation = Quat::from_rotation_y(i as f32);
        app.update();
        assert_eq!(*app.world().get::<Transform>(marker).unwrap(), baseline);
        assert_eq!(app.world().get::<Node>(frame).unwrap().top, frame_top);
    }
    app.world_mut()
        .get_mut::<Transform>(parent)
        .unwrap()
        .translation
        .x += 0.2;
    app.update();
    assert!(
        (app.world()
            .get::<GlobalTransform>(marker)
            .unwrap()
            .translation()
            .x
            - 0.4)
            .abs()
            < 1e-6
    );
}

#[test]
fn target_presentation_clears_invalid_and_despawned_targets_without_stale_frame() {
    for mode in [PlayerVisualMode::Models3d, PlayerVisualMode::Sprite2d] {
        let (mut app, enemy, _, marker, frame) = fixture(mode);
        app.update();
        for invalid in 0..4 {
            app.world_mut().get_mut::<CombatStats>(enemy).unwrap().hp =
                if invalid == 0 { 0.0 } else { 100.0 };
            app.world_mut().entity_mut(enemy).insert(if invalid == 1 {
                InheritedVisibility::HIDDEN
            } else {
                InheritedVisibility::VISIBLE
            });
            app.world_mut().get_mut::<NetworkPlayerId>(enemy).unwrap().0 =
                if invalid == 2 { 99 } else { 9 };
            if invalid == 3 {
                app.world_mut().despawn(enemy);
            }
            app.update();
            assert_eq!(
                *app.world().get::<Visibility>(marker).unwrap(),
                Visibility::Hidden
            );
            assert_eq!(
                app.world().get::<Node>(frame).unwrap().display,
                Display::None
            );
        }
    }
}

#[test]
fn target_presentation_switches_and_clears_without_blending_between_enemies() {
    for mode in [PlayerVisualMode::Models3d, PlayerVisualMode::Sprite2d] {
        let (mut app, _, _, marker, frame) = fixture(mode);
        app.update();
        let other = app
            .world_mut()
            .spawn((
                RemotePlayer,
                Team::Blue,
                NetworkPlayerId(10),
                CombatStats::default(),
                Transform::from_xyz(-0.4, 0.0, 0.0),
                InheritedVisibility::VISIBLE,
            ))
            .id();
        {
            let mut target = app.world_mut().resource_mut::<TargetState>();
            target.selected_entity = Some(other);
            target.selected_target = Some(TargetId {
                kind: TargetKind::Player,
                id: 10,
            });
        }
        app.update();
        assert_eq!(
            app.world()
                .get::<GlobalTransform>(marker)
                .unwrap()
                .translation()
                .x,
            -0.4
        );
        assert!(
            (match app.world().get::<Node>(frame).unwrap().left {
                Val::Px(x) => x,
                _ => panic!("missing frame"),
            } - 217.0)
                .abs()
                < 1e-4
        );
        app.world_mut()
            .resource_mut::<TargetState>()
            .selected_entity = None;
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(marker).unwrap(),
            Visibility::Hidden
        );
        assert_eq!(
            app.world().get::<Node>(frame).unwrap().display,
            Display::None
        );
    }
}
