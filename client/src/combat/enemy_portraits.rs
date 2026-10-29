//! Nearby visible enemy locks and position-free public enemy death timers.
// i18n-strict
use bevy::{prelude::*, window::PrimaryWindow};

use crate::{
    camera::MainCamera,
    hud_layout::{HudLayout, ui_viewport},
    input_context::GameplayInputContext,
    mobile_controls::MobileControls,
    net::{
        GameState, GameStateSnapshot, NetworkAvatar, NetworkHeroClass, NetworkPlayerId,
        PlayerProgression, RemotePlayer, TargetId, TargetKind,
    },
    player::Player,
    sprite::PlayerVisualMode,
    targeting::{BasicAttackState, TargetValidity},
    team::Team,
    ui::{
        Activated, TestId, UiAction,
        tokens::{color, space},
        widgets::game::{self, PortraitSpec, PortraitView},
    },
};

use super::{CombatStats, PendingCast, TargetState, WorldPointerState};

#[derive(Component, Clone, Copy, Debug)]
pub(super) struct EnemyPortraitAction {
    entity: Entity,
    target: TargetId,
}

#[derive(Component)]
pub(super) struct EnemyPortraitArt(u64);

#[derive(Component)]
pub(super) struct EnemyRespawnLabel(u64);

#[derive(Component)]
pub(super) struct EnemyPortraitDirection(EnemyPortraitAction);

#[derive(Component)]
pub(super) struct EnemyPortraitDirectionStroke {
    action: EnemyPortraitAction,
    index: usize,
}

const DIRECTION_OVERLAY_SIDE: f32 = 40.0;
const DIRECTION_STROKE_WIDTH: f32 = 2.0;

/// Both endpoints are projected through the same current camera. Raw world XZ
/// would point the wrong way after an orbit or when the 2D renderer is selected.
fn portrait_screen_direction(
    camera: &Camera,
    camera_pose: &GlobalTransform,
    origin: Vec3,
    target: Vec3,
    mode: PlayerVisualMode,
) -> Option<Vec2> {
    let affine = camera_pose.affine();
    let determinant = affine.matrix3.determinant();
    if !camera.is_active
        || !origin.is_finite()
        || !target.is_finite()
        || !affine.is_finite()
        || !determinant.is_finite()
        || determinant.abs() < 0.000001
        || !camera.computed.clip_from_view.is_finite()
    {
        return None;
    }
    let screen = |position| {
        let render = if mode == PlayerVisualMode::Sprite2d {
            crate::world2d::simulation_xz_to_render_xy(position).extend(0.0)
        } else {
            position
        };
        camera.world_to_viewport(camera_pose, render).ok()
    };
    let delta = screen(target)? - screen(origin)?;
    if !delta.is_finite() {
        return None;
    }
    delta.try_normalize()
}

/// Three short strokes remain inside the portrait's 40px content box at every
/// angle, including their 2px width. They cannot paint over another HUD cell.
fn direction_strokes(direction: Vec2) -> [(Vec2, Vec2); 3] {
    let center = Vec2::splat(DIRECTION_OVERLAY_SIDE * 0.5);
    let tip = center + direction * 17.0;
    let base = tip - direction * 5.0;
    let normal = Vec2::new(-direction.y, direction.x) * 3.0;
    [
        (center + direction * 5.0, tip),
        (base + normal, tip),
        (base - normal, tip),
    ]
}

fn spawn_portrait_direction(parent: &mut ChildSpawnerCommands, action: EnemyPortraitAction) {
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Percent(50.0),
                width: Val::Px(DIRECTION_OVERLAY_SIDE),
                height: Val::Px(DIRECTION_OVERLAY_SIDE),
                display: Display::None,
                ..default()
            },
            UiTransform::from_translation(Val2::new(Val::Percent(-50.0), Val::Percent(-50.0))),
            Pickable::IGNORE,
            ZIndex(3),
            EnemyPortraitDirection(action),
            Name::new(format!(
                "EnemyPortraitDirectionOverlay-{}",
                action.target.id
            )),
        ))
        .with_children(|arrow| {
            for (index, (a, b)) in direction_strokes(Vec2::X).into_iter().enumerate() {
                let (node, transform) = crate::minimap::line_node(a, b, DIRECTION_STROKE_WIDTH);
                arrow.spawn((
                    node,
                    transform,
                    BackgroundColor(color::GOLD_400),
                    Pickable::IGNORE,
                    EnemyPortraitDirectionStroke { action, index },
                    Name::new(if index == 0 {
                        format!("EnemyPortraitDirection-{}", action.target.id)
                    } else {
                        format!("EnemyPortraitDirectionHead-{}-{index}", action.target.id)
                    }),
                ));
            }
        });
}

fn nearby(origin: Vec3, point: Vec3) -> bool {
    origin.is_finite()
        && point.is_finite()
        && origin.xz().distance_squared(point.xz()) <= shared::vision::HERO_SIGHT_RADIUS.powi(2)
}

/// Live locks require replicated visible nearby entities. Public scoreboard death
/// timers add disabled portraits without revealing hidden enemy positions.
#[allow(clippy::type_complexity)]
pub(super) fn sync_enemy_portraits(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mobile: Option<Res<MobileControls>>,
    ui_scale: Option<Res<UiScale>>,
    local: Query<(&Transform, &Team, &CombatStats), With<Player>>,
    enemies: Query<
        (
            Entity,
            &Transform,
            &NetworkPlayerId,
            Option<&NetworkHeroClass>,
            Option<&NetworkAvatar>,
            Option<&PlayerProgression>,
            Option<&Visibility>,
        ),
        (With<RemotePlayer>, Without<Player>),
    >,
    validity: TargetValidity,
    selected: Res<TargetState>,
    mut buttons: Query<(Entity, &EnemyPortraitAction, &mut Node, &mut BorderColor)>,
    mut portraits: Query<(&EnemyPortraitArt, &mut PortraitView)>,
    mut timers: Query<(&EnemyRespawnLabel, &mut Text)>,
) {
    let local = local.single().ok();
    let mut visible = Vec::new();
    if matches!(game.state, GameState::Running)
        && screen
            .as_ref()
            .is_none_or(|screen| *screen.get() == crate::frontend::AppScreen::InMatch)
        && let Some((origin, team, _)) = local
    {
        for (entity, transform, id, class, avatar, progression, visibility) in &enemies {
            let target = TargetId {
                kind: TargetKind::Player,
                id: id.0,
            };
            if visibility == Some(&Visibility::Hidden)
                || !validity.valid(entity, target, *team)
                || !nearby(origin.translation, transform.translation)
            {
                continue;
            }
            visible.push((
                EnemyPortraitAction { entity, target },
                PortraitView {
                    art: avatar
                        .and_then(|avatar| avatar.0.as_deref())
                        .and_then(omoba_passport::avatars::avatar_definition)
                        .and_then(crate::passport::thumbnail_asset_path),
                    fallback: game::class_icon(
                        class.map_or(shared::HeroClass::default(), |class| class.0),
                    ),
                    level: progression.map(|progression| progression.level),
                    xp: 0.0,
                    grey: false,
                    strong_rim: selected.selected_entity == Some(entity),
                },
                0_u32,
            ));
        }
        // Death timers are public match information; this carries no live enemy position.
        if let Some(board) = &game.scoreboard {
            for row in &board.players {
                if Team::from(row.team) == *team || row.respawn_remaining_ms == 0 {
                    continue;
                }
                visible.retain(|(action, _, _)| action.target.id != row.player_id);
                visible.push((
                    EnemyPortraitAction {
                        entity: Entity::PLACEHOLDER,
                        target: TargetId {
                            kind: TargetKind::Player,
                            id: row.player_id,
                        },
                    },
                    PortraitView {
                        art: row
                            .avatar
                            .as_deref()
                            .and_then(omoba_passport::avatars::avatar_definition)
                            .and_then(crate::passport::thumbnail_asset_path),
                        fallback: game::class_icon(row.hero_class),
                        level: Some(row.level),
                        xp: 0.0,
                        grey: true,
                        strong_rim: false,
                    },
                    row.respawn_remaining_ms.div_ceil(1000),
                ));
            }
        }
    }
    // Stable identities avoid reshuffling portraits as enemies move around.
    visible.sort_by_key(|(action, _, _)| action.target.id);
    visible.truncate(5);
    let viewport = window.single().ok().map_or_else(
        || {
            mobile
                .as_ref()
                .map_or(Vec2::new(1280.0, 720.0), |mobile| mobile.viewport)
        },
        |window| ui_viewport(window, ui_scale.as_deref()),
    );
    let slots = HudLayout::resolve(viewport, mobile.as_deref(), false).enemy_portrait_slots();
    for (button, action, mut node, mut border) in &mut buttons {
        let Some(slot) = visible.iter().position(|(entry, _, _)| {
            entry.target == action.target && entry.entity == action.entity
        }) else {
            commands.entity(button).despawn();
            continue;
        };
        node.left = Val::Px(slots[slot].min.x);
        node.top = Val::Px(slots[slot].min.y);
        *border = BorderColor::all(if selected.selected_entity == Some(action.entity) {
            color::GOLD_400
        } else {
            color::STATE_DANGER
        });
    }
    for (art, mut view) in &mut portraits {
        if let Some((_, next, _)) = visible
            .iter()
            .find(|(entry, _, _)| entry.target.id == art.0)
        {
            view.set_if_neq(next.clone());
        }
    }
    for (label, mut text) in &mut timers {
        let value = visible
            .iter()
            .find(|(action, _, _)| action.target.id == label.0)
            .map_or(0, |(_, _, secs)| *secs);
        text.0 = if value > 0 {
            value.to_string()
        } else {
            String::new()
        };
    }
    for (slot, (action, view, secs)) in visible.into_iter().enumerate() {
        if buttons.iter().any(|(_, existing, _, _)| {
            existing.target == action.target && existing.entity == action.entity
        }) {
            continue;
        }
        // Each button owns one immutable target identity for its entire life.
        // An enemy disappearing during a tap cannot turn that tap into a lock on its replacement.
        commands
            .spawn((
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(slots[slot].min.x),
                    top: Val::Px(slots[slot].min.y),
                    width: Val::Px(44.0),
                    height: Val::Px(44.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(color::SURFACE_1_OPAQUE),
                BorderColor::all(color::STATE_DANGER),
                ZIndex(18),
                UiAction(action),
                action,
                TestId::new(format!("EnemyPortrait-{}", action.target.id)),
                Name::new(format!("EnemyPortrait-{}", action.target.id)),
            ))
            .with_children(|button| {
                game::live_portrait(
                    button,
                    view,
                    PortraitSpec {
                        side: 36.0,
                        ring: false,
                        disc: 14.0,
                        disc_at: Vec2::new(-space::S4, -space::S4),
                        icon: 24.0,
                    },
                    EnemyPortraitArt(action.target.id),
                );
                button.spawn((
                    Text::new(if secs > 0 {
                        secs.to_string()
                    } else {
                        String::new()
                    }),
                    TextFont {
                        font_size: (19.0).into(),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    ZIndex(2),
                    Pickable::IGNORE,
                    EnemyRespawnLabel(action.target.id),
                    Name::new("Enemy respawn countdown"),
                ));
                spawn_portrait_direction(button, action);
            });
    }
}

/// Refresh after camera follow/grounding and before UI layout. Only a selected
/// portrait's still-valid visible world entity may supply a bearing; public
/// death timers never carry a position. Missing projection data fails closed.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn draw_enemy_portrait_directions(
    game: Option<Res<GameStateSnapshot>>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    context: Option<Res<GameplayInputContext>>,
    mode: Option<Res<PlayerVisualMode>>,
    selected: Res<TargetState>,
    local: Query<(Entity, &Transform, &Team, &CombatStats), With<Player>>,
    enemies: Query<(&Transform, Option<&Visibility>), (With<RemotePlayer>, Without<Player>)>,
    camera: Query<(Entity, &Camera), With<MainCamera>>,
    poses: bevy::transform::helper::TransformHelper,
    validity: TargetValidity,
    mut arrows: Query<(&EnemyPortraitDirection, &mut Node)>,
    mut strokes: Query<
        (&EnemyPortraitDirectionStroke, &mut Node, &mut UiTransform),
        Without<EnemyPortraitDirection>,
    >,
) {
    let bearing = (|| {
        if !game
            .as_ref()
            .is_some_and(|game| matches!(game.state, GameState::Running))
            || !context
                .as_ref()
                .is_some_and(|context| context.gameplay_allowed())
            || screen
                .as_ref()
                .is_some_and(|screen| *screen.get() != crate::frontend::AppScreen::InMatch)
        {
            return None;
        }
        let (entity, target) = selected.selected_entity.zip(selected.selected_target)?;
        if target.kind != TargetKind::Player || entity == Entity::PLACEHOLDER {
            return None;
        }
        let (hero, origin, team, stats) = local.single().ok()?;
        let (enemy, visibility) = enemies.get(entity).ok()?;
        if !stats.is_alive()
            || visibility == Some(&Visibility::Hidden)
            || !validity.valid(entity, target, *team)
            || !nearby(origin.translation, enemy.translation)
        {
            return None;
        }
        let (camera_entity, camera) = camera.single().ok()?;
        let direction = portrait_screen_direction(
            camera,
            &poses.compute_global_transform(camera_entity).ok()?,
            poses.compute_global_transform(hero).ok()?.translation(),
            poses.compute_global_transform(entity).ok()?.translation(),
            mode.as_deref().copied()?,
        )?;
        Some((EnemyPortraitAction { entity, target }, direction))
    })();
    let matches = |action: EnemyPortraitAction| {
        bearing.filter(|(selected, _)| {
            selected.entity == action.entity && selected.target == action.target
        })
    };
    for (arrow, mut node) in &mut arrows {
        let next = if matches(arrow.0).is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != next {
            node.display = next;
        }
    }
    for (stroke, mut node, mut transform) in &mut strokes {
        if let Some((_, direction)) = matches(stroke.action) {
            let (a, b) = direction_strokes(direction)[stroke.index];
            let (next_node, next_transform) =
                crate::minimap::line_node(a, b, DIRECTION_STROKE_WIDTH);
            node.set_if_neq(next_node);
            transform.set_if_neq(next_transform);
        } else if node.display != Display::None {
            node.display = Display::None;
        }
    }
}

pub(super) fn activate_enemy_portrait(
    mut activated: MessageReader<Activated<EnemyPortraitAction>>,
    context: Res<GameplayInputContext>,
    local: Query<(&Transform, &Team, &CombatStats), With<Player>>,
    validity: TargetValidity,
    visibility: Query<&Visibility>,
    mut selected: ResMut<TargetState>,
    mut pending: ResMut<PendingCast>,
    mut basic: ResMut<BasicAttackState>,
    mut pointer: ResMut<WorldPointerState>,
    mut buttons: Query<&mut Node, With<EnemyPortraitAction>>,
) {
    // Resolve after modal toggles so portraits disappear on the same frame
    // as Settings/shop/scoreboard opens, then reappear when gameplay resumes.
    for mut node in &mut buttons {
        node.display = if context.gameplay_allowed() {
            Display::Flex
        } else {
            Display::None
        };
    }
    for Activated { action, .. } in activated.read() {
        pointer.consumed_primary_press = true;
        pointer.consumed_secondary_press = true;
        if !context.gameplay_allowed() {
            continue;
        }
        let Ok((origin, team, stats)) = local.single() else {
            continue;
        };
        if action.entity == Entity::PLACEHOLDER
            || !stats.is_alive()
            || visibility
                .get(action.entity)
                .is_ok_and(|visibility| *visibility == Visibility::Hidden)
            || !validity.valid(action.entity, action.target, *team)
            || !validity
                .position(action.entity)
                .is_some_and(|point| nearby(origin.translation, point))
        {
            continue;
        }
        selected.selected_entity = Some(action.entity);
        selected.selected_target = Some(action.target);
        pending.cancel();
        basic.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        let mut mobile = MobileControls::default();
        mobile.enabled = true;
        mobile.viewport = Vec2::new(852.0, 393.0);
        app.insert_resource(mobile)
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .init_resource::<TargetState>()
            .init_resource::<GameplayInputContext>()
            .init_resource::<PendingCast>()
            .init_resource::<BasicAttackState>()
            .init_resource::<WorldPointerState>()
            .add_message::<Activated<EnemyPortraitAction>>()
            .add_systems(
                Update,
                (sync_enemy_portraits, activate_enemy_portrait).chain(),
            );
        app.world_mut().spawn((
            Player,
            Team::Green,
            Transform::default(),
            CombatStats::default(),
        ));
        app
    }

    fn enemy(app: &mut App, id: u64, x: f32) -> Entity {
        app.world_mut()
            .spawn((
                RemotePlayer,
                NetworkPlayerId(id),
                Team::Blue,
                Transform::from_xyz(x, 0.0, 0.0),
                CombatStats::default(),
                InheritedVisibility::VISIBLE,
            ))
            .id()
    }

    fn portraits(app: &mut App) -> Vec<(Entity, EnemyPortraitAction)> {
        app.world_mut()
            .query::<(Entity, &EnemyPortraitAction)>()
            .iter(app.world())
            .map(|(entity, action)| (entity, *action))
            .collect()
    }

    fn projection_camera() -> Camera {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        Camera {
            computed: ComputedCameraValues {
                clip_from_view: Mat4::from_scale(Vec3::new(0.1, 0.1, -0.01)),
                target_info: Some(RenderTargetInfo {
                    physical_size: UVec2::new(844, 384),
                    scale_factor: 1.0,
                }),
                ..default()
            },
            ..default()
        }
    }

    fn camera_pose(up: Vec3) -> Transform {
        Transform::from_xyz(0.0, 50.0, 0.0).looking_at(Vec3::ZERO, up)
    }

    fn direction_app() -> (App, Entity) {
        let mut app = app();
        app.insert_resource(PlayerVisualMode::Models3d)
            .add_systems(PostUpdate, draw_enemy_portrait_directions);
        let camera = app
            .world_mut()
            .spawn((MainCamera, camera_pose(Vec3::NEG_Z), projection_camera()))
            .id();
        (app, camera)
    }

    fn direction_shown(app: &mut App, id: u64) -> bool {
        app.world_mut()
            .query::<(&EnemyPortraitDirection, &Node)>()
            .iter(app.world())
            .any(|(arrow, node)| arrow.0.target.id == id && node.display != Display::None)
    }

    #[test]
    fn portrait_direction_uses_projected_screen_axes_and_camera_rotation() {
        let camera = projection_camera();
        let pose = GlobalTransform::from(camera_pose(Vec3::NEG_Z));
        for (world, expected) in [
            (Vec3::X * 4.0, Vec2::X),
            (Vec3::Z * 4.0, Vec2::Y),
            (Vec3::NEG_Z * 4.0, Vec2::NEG_Y),
        ] {
            let direction = portrait_screen_direction(
                &camera,
                &pose,
                Vec3::ZERO,
                world,
                PlayerVisualMode::Models3d,
            )
            .unwrap();
            assert!(direction.distance(expected) < 0.00001);
        }
        let rotated = GlobalTransform::from(camera_pose(Vec3::X));
        let direction = portrait_screen_direction(
            &camera,
            &rotated,
            Vec3::ZERO,
            Vec3::X * 4.0,
            PlayerVisualMode::Models3d,
        )
        .unwrap();
        assert!(
            direction.distance(Vec2::NEG_Y) < 0.00001,
            "world +X becomes screen up after rotating the camera"
        );
        let direction = portrait_screen_direction(
            &camera,
            &rotated,
            Vec3::ZERO,
            Vec3::Z * 4.0,
            PlayerVisualMode::Models3d,
        )
        .unwrap();
        assert!(direction.distance(Vec2::X) < 0.00001);
    }

    #[test]
    fn portrait_direction_honors_the_2d_projection_and_rejects_missing_geometry() {
        let mut camera = projection_camera();
        camera.computed.clip_from_view = Mat4::IDENTITY;
        let direction = portrait_screen_direction(
            &camera,
            &GlobalTransform::IDENTITY,
            Vec3::ZERO,
            Vec3::Z * 0.5,
            PlayerVisualMode::Sprite2d,
        )
        .unwrap();
        assert!(
            direction.distance(Vec2::NEG_Y) < 0.00001,
            "simulation +Z becomes render +Y, which is screen up"
        );
        for target in [Vec3::ZERO, Vec3::NAN] {
            assert!(
                portrait_screen_direction(
                    &camera,
                    &GlobalTransform::IDENTITY,
                    Vec3::ZERO,
                    target,
                    PlayerVisualMode::Sprite2d
                )
                .is_none()
            );
        }
        assert!(
            portrait_screen_direction(
                &Camera::default(),
                &GlobalTransform::IDENTITY,
                Vec3::ZERO,
                Vec3::X,
                PlayerVisualMode::Sprite2d
            )
            .is_none()
        );
        assert!(
            portrait_screen_direction(
                &camera,
                &GlobalTransform::from(Transform::from_scale(Vec3::ZERO)),
                Vec3::ZERO,
                Vec3::X,
                PlayerVisualMode::Sprite2d
            )
            .is_none()
        );
        assert!(
            portrait_screen_direction(
                &projection_camera(),
                &GlobalTransform::from(camera_pose(Vec3::NEG_Z)),
                Vec3::ZERO,
                Vec3::new(4.0, 100.0, 0.0),
                PlayerVisualMode::Models3d
            )
            .is_none(),
            "a point behind the camera has no trustworthy screen bearing"
        );
    }

    #[test]
    fn stationary_portrait_bearing_does_not_dirty_stroke_layout_each_frame() {
        #[derive(Resource, Default)]
        struct StrokeChanges(usize);
        let (mut app, camera) = direction_app();
        app.init_resource::<StrokeChanges>().add_systems(
            Last,
            |mut changes: ResMut<StrokeChanges>,
             nodes: Query<
                (),
                (
                    With<EnemyPortraitDirectionStroke>,
                    Or<(Changed<Node>, Changed<UiTransform>)>,
                ),
            >| {
                changes.0 = nodes.iter().count();
            },
        );
        let target = enemy(&mut app, 17, 4.0);
        *app.world_mut().resource_mut::<TargetState>() = TargetState::for_request(
            target,
            TargetId {
                kind: TargetKind::Player,
                id: 17,
            },
        );
        app.update();
        assert!(app.world().resource::<StrokeChanges>().0 > 0);
        app.update();
        assert_eq!(app.world().resource::<StrokeChanges>().0, 0);
        *app.world_mut().get_mut::<Transform>(camera).unwrap() = camera_pose(Vec3::X);
        app.update();
        assert!(app.world().resource::<StrokeChanges>().0 > 0);
    }

    #[test]
    fn direction_strokes_fit_inside_the_portrait_at_every_angle() {
        let center = Vec2::splat(DIRECTION_OVERLAY_SIDE * 0.5);
        for step in 0..64 {
            let direction = Vec2::from_angle(step as f32 * std::f32::consts::TAU / 64.0);
            for (a, b) in direction_strokes(direction) {
                for point in [a, b] {
                    assert!(
                        point.distance(center) + DIRECTION_STROKE_WIDTH * 0.5
                            < DIRECTION_OVERLAY_SIDE * 0.5
                    );
                }
            }
        }
    }

    #[test]
    fn selected_portrait_direction_expires_with_visibility_identity_life_and_projection() {
        for invalid in 0..13 {
            let (mut app, camera) = direction_app();
            let target = enemy(&mut app, 17, 4.0);
            *app.world_mut().resource_mut::<TargetState>() = TargetState::for_request(
                target,
                TargetId {
                    kind: TargetKind::Player,
                    id: 17,
                },
            );
            app.update();
            assert!(direction_shown(&mut app, 17));
            match invalid {
                0 => {
                    app.world_mut()
                        .resource_mut::<TargetState>()
                        .selected_target = None
                }
                1 => {
                    app.world_mut()
                        .resource_mut::<TargetState>()
                        .selected_target
                        .as_mut()
                        .unwrap()
                        .id = 999
                }
                2 => app.world_mut().get_mut::<CombatStats>(target).unwrap().hp = 0.0,
                3 => {
                    app.world_mut()
                        .entity_mut(target)
                        .insert(InheritedVisibility::HIDDEN);
                }
                4 => {
                    app.world_mut()
                        .entity_mut(target)
                        .insert(Visibility::Hidden);
                }
                5 => {
                    app.world_mut()
                        .get_mut::<Transform>(target)
                        .unwrap()
                        .translation
                        .x = shared::vision::HERO_SIGHT_RADIUS + 1.0
                }
                6 => {
                    let mut heroes = app
                        .world_mut()
                        .query_filtered::<&mut CombatStats, With<Player>>();
                    heroes.single_mut(app.world_mut()).unwrap().hp = 0.0;
                }
                7 => {
                    app.world_mut()
                        .resource_mut::<GameplayInputContext>()
                        .modal_open = true
                }
                8 => app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Lobby,
                9 => {
                    app.insert_resource(State::new(crate::frontend::AppScreen::Home));
                }
                10 => {
                    app.world_mut()
                        .get_mut::<Camera>(camera)
                        .unwrap()
                        .computed
                        .target_info = None
                }
                11 => {
                    app.world_mut().despawn(target);
                }
                _ => app.world_mut().get_mut::<Transform>(camera).unwrap().scale = Vec3::ZERO,
            }
            app.update();
            assert!(
                !direction_shown(&mut app, 17),
                "ineligible direction remains for case {invalid}"
            );
        }
    }

    #[test]
    fn portrait_direction_keeps_exact_selection_and_samples_current_camera_pose() {
        let (mut app, camera) = direction_app();
        let first = enemy(&mut app, 17, 4.0);
        let second = enemy(&mut app, 18, 3.0);
        app.update();
        let (source, action) = portraits(&mut app)
            .into_iter()
            .find(|(_, action)| action.entity == first)
            .unwrap();
        app.world_mut().write_message(Activated { source, action });
        app.update();
        assert_eq!(
            app.world().resource::<TargetState>().selected_entity,
            Some(first)
        );
        assert!(direction_shown(&mut app, 17));
        assert!(!direction_shown(&mut app, 18));
        // GlobalTransform remains stale here: the presentation helper must use
        // this frame's Transform, as the real camera-follow pipeline does.
        *app.world_mut().get_mut::<Transform>(camera).unwrap() = camera_pose(Vec3::X);
        app.update();
        let mut arrows = app.world_mut().query::<(&Name, &UiTransform, &Pickable)>();
        let (_, transform, pickable) = arrows
            .iter(app.world())
            .find(|(name, _, _)| name.as_str() == "EnemyPortraitDirection-17")
            .unwrap();
        assert!((transform.rotation * Vec2::X).distance(Vec2::NEG_Y) < 0.00001);
        assert!(!pickable.is_hoverable && !pickable.should_block_lower);
        app.world_mut()
            .resource_mut::<TargetState>()
            .selected_entity = Some(second);
        app.update();
        assert!(
            !direction_shown(&mut app, 17) && !direction_shown(&mut app, 18),
            "mismatched entity/identity cannot borrow a direction"
        );
        app.world_mut()
            .resource_mut::<TargetState>()
            .selected_target = Some(TargetId {
            kind: TargetKind::Player,
            id: 18,
        });
        app.update();
        assert!(direction_shown(&mut app, 18));
        app.world_mut().despawn(second);
        enemy(&mut app, 18, 3.0);
        app.update();
        assert!(
            !direction_shown(&mut app, 18),
            "a replacement entity cannot inherit the stale selection"
        );
    }

    #[test]
    fn portraits_require_live_visible_nearby_enemy_entities_and_expire_with_them() {
        let mut app = app();
        let live = enemy(&mut app, 1, 4.0);
        let ally = enemy(&mut app, 2, 3.0);
        *app.world_mut().get_mut::<Team>(ally).unwrap() = Team::Green;
        let dead = enemy(&mut app, 3, 5.0);
        app.world_mut().get_mut::<CombatStats>(dead).unwrap().hp = 0.0;
        let hidden = enemy(&mut app, 4, 6.0);
        app.world_mut()
            .entity_mut(hidden)
            .insert(InheritedVisibility::HIDDEN);
        enemy(&mut app, 5, shared::vision::HERO_SIGHT_RADIUS + 0.1);
        app.update();
        let entries = portraits(&mut app);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1.entity, live);
        app.world_mut()
            .entity_mut(live)
            .insert(InheritedVisibility::HIDDEN);
        app.update();
        assert!(portraits(&mut app).is_empty());
        app.world_mut()
            .entity_mut(live)
            .insert(InheritedVisibility::VISIBLE);
        app.update();
        assert_eq!(portraits(&mut app).len(), 1);
        app.world_mut().despawn(live);
        app.update();
        assert!(portraits(&mut app).is_empty());
    }

    #[test]
    fn public_death_portrait_has_no_position_cannot_lock_and_expires_at_respawn() {
        let mut app = app();
        let row = shared::live_score::LiveScorePlayer {
            player_id: 77,
            nickname: "Hidden opponent".into(),
            avatar: None,
            team: shared::map::Team::Blue,
            hero_class: shared::HeroClass::Ranger,
            kills: 0,
            deaths: 1,
            assists: 0,
            earned_gold: 0,
            level: 3,
            connected: true,
            respawn_remaining_ms: 2400,
        };
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .scoreboard = Some(shared::live_score::LiveScoreboard {
            players: vec![row],
            ..default()
        });
        app.update();
        let (source, action) = portraits(&mut app)[0];
        assert_eq!(
            action.entity,
            Entity::PLACEHOLDER,
            "no remote world entity or position is required"
        );
        let node = app.world().get::<Node>(source).unwrap();
        assert_eq!((node.width, node.height), (Val::Px(44.0), Val::Px(44.0)));
        assert!(
            app.world_mut()
                .query::<&PortraitView>()
                .iter(app.world())
                .any(|view| view.grey)
        );
        let countdown = |app: &mut App| {
            app.world_mut()
                .query::<(&EnemyRespawnLabel, &Text)>()
                .iter(app.world())
                .find(|(label, _)| label.0 == 77)
                .unwrap()
                .1
                .0
                .clone()
        };
        assert_eq!(countdown(&mut app), "3");
        app.world_mut().write_message(Activated { source, action });
        app.update();
        assert!(
            app.world()
                .resource::<TargetState>()
                .selected_target
                .is_none()
        );
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .scoreboard
            .as_mut()
            .unwrap()
            .players[0]
            .respawn_remaining_ms = 999;
        app.update();
        assert_eq!(countdown(&mut app), "1");
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .scoreboard
            .as_mut()
            .unwrap()
            .players[0]
            .respawn_remaining_ms = 0;
        app.update();
        assert!(
            portraits(&mut app).is_empty(),
            "a living hidden enemy must not remain on the public HUD"
        );
        let live = enemy(&mut app, 77, 3.0);
        app.update();
        assert_eq!(portraits(&mut app)[0].1.entity, live);
        app.world_mut().write_message(Activated { source, action });
        app.update();
        assert!(
            app.world()
                .resource::<TargetState>()
                .selected_target
                .is_none(),
            "a stale dead portrait cannot become a live lock"
        );
    }

    #[test]
    fn portrait_tap_locks_exact_enemy_consumes_pointer_and_never_retargets_stale_press() {
        let mut app = app();
        let first = enemy(&mut app, 17, 3.0);
        let second = enemy(&mut app, 18, 2.0);
        app.update();
        let entries = portraits(&mut app);
        let (source, action) = *entries
            .iter()
            .find(|(_, action)| action.entity == first)
            .unwrap();
        app.world_mut().write_message(Activated { source, action });
        app.update();
        assert_eq!(
            app.world().resource::<TargetState>().selected_entity,
            Some(first)
        );
        assert_eq!(
            app.world().resource::<TargetState>().selected_target,
            Some(action.target)
        );
        let pointer = app.world().resource::<WorldPointerState>();
        assert!(pointer.consumed_primary_press && pointer.consumed_secondary_press);
        assert!(app.world().resource::<BasicAttackState>().order.is_none());
        app.world_mut()
            .resource_mut::<TargetState>()
            .selected_entity = None;
        app.world_mut()
            .resource_mut::<TargetState>()
            .selected_target = None;
        app.world_mut().despawn(first);
        app.world_mut().write_message(Activated { source, action });
        app.update();
        assert!(
            app.world()
                .resource::<TargetState>()
                .selected_entity
                .is_none()
        );
        assert_eq!(portraits(&mut app)[0].1.entity, second);
        let (source, action) = portraits(&mut app)[0];
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .modal_open = true;
        app.world_mut().write_message(Activated { source, action });
        app.update();
        assert!(
            app.world()
                .resource::<TargetState>()
                .selected_entity
                .is_none()
        );
        assert_eq!(
            app.world().get::<Node>(source).unwrap().display,
            Display::None
        );
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .modal_open = false;
        app.update();
        assert_eq!(
            app.world().get::<Node>(source).unwrap().display,
            Display::Flex
        );
    }
}
