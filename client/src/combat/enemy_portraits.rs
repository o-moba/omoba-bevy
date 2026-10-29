//! Nearby visible enemy locks and position-free public enemy death timers.
// i18n-strict
use bevy::{prelude::*, window::PrimaryWindow};

use crate::{
    camera::MainCamera,
    hud_layout::{HudLayout, ui_viewport, world_to_ui},
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

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EnemyPortraitAction {
    entity: Entity,
    target: TargetId,
}

/// Position-free public death information; never a targeting action.
#[derive(Component, Clone, Copy, Debug)]
pub(super) struct EnemyDeathPortrait {
    player_id: u64,
    side: f32,
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
    width: f32,
}

#[derive(Component)]
pub(super) struct EnemyTargetConnectorStroke {
    action: EnemyPortraitAction,
    index: usize,
    width: f32,
}

/// Actual portrait-to-world segment, clipped along its ray so an offscreen
/// enemy points to the correct edge without changing the direction.
fn target_connector(portrait: Rect, target: Vec2, viewport: Vec2) -> Option<[(Vec2, Vec2); 3]> {
    if !portrait.min.is_finite()
        || !portrait.max.is_finite()
        || !target.is_finite()
        || !viewport.is_finite()
        || portrait.width() <= 0.0
        || portrait.height() <= 0.0
        || viewport.min_element() <= 16.0
    {
        return None;
    }
    let center = portrait.center();
    let direction = (target - center).try_normalize()?;
    let start = center + direction * (portrait.size().max_element() * 0.5 + 2.0);
    if (target - center).length() <= (start - center).length() {
        return None;
    }
    let delta = target - start;
    let bounds = Rect::from_corners(Vec2::splat(8.0), viewport - Vec2::splat(8.0));
    let (mut enter, mut leave) = (0.0_f32, 1.0_f32);
    for (a, d, lo, hi) in [
        (start.x, delta.x, bounds.min.x, bounds.max.x),
        (start.y, delta.y, bounds.min.y, bounds.max.y),
    ] {
        if d.abs() < 0.00001 {
            if a < lo || a > hi {
                return None;
            }
        } else {
            let (t0, t1) = ((lo - a) / d, (hi - a) / d);
            enter = enter.max(t0.min(t1));
            leave = leave.min(t0.max(t1));
        }
    }
    if enter >= leave {
        return None;
    }
    let a = start + delta * enter;
    let b = start + delta * leave;
    if !a.is_finite() || !b.is_finite() || a.distance(b) < 2.0 {
        return None;
    }
    let head = 8.0_f32.min(a.distance(b) * 0.4);
    let normal = Vec2::new(-direction.y, direction.x) * (head * 0.5);
    let base = b - direction * head;
    Some([(a, b), (base + normal, b), (base - normal, b)])
}

fn spawn_target_connector(parent: &mut ChildSpawnerCommands, action: EnemyPortraitAction) {
    for (outline, width, paint) in [
        (true, 4.0, Color::srgba(0.01, 0.015, 0.025, 0.85)),
        (false, 2.0, color::GOLD_400),
    ] {
        for index in 0..3 {
            parent.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    display: Display::None,
                    ..default()
                },
                UiTransform::IDENTITY,
                BackgroundColor(paint),
                Pickable::IGNORE,
                GlobalZIndex(10),
                EnemyTargetConnectorStroke {
                    action,
                    index,
                    width,
                },
                Name::new(format!(
                    "EnemyTargetConnector{}-{}-{index}",
                    if outline { "Outline" } else { "" },
                    action.target.id
                )),
            ));
        }
    }
}

const DIRECTION_OVERLAY_SIDE: f32 = 40.0;
const DIRECTION_STROKE_WIDTH: f32 = 2.0;
const DIRECTION_OUTLINE_WIDTH: f32 = 4.0;

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
/// angle, including their 4px outline. They cannot paint over another HUD cell.
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
            for (outline, width, paint) in [
                (
                    true,
                    DIRECTION_OUTLINE_WIDTH,
                    Color::srgba(0.01, 0.015, 0.025, 0.95),
                ),
                (false, DIRECTION_STROKE_WIDTH, color::GOLD_400),
            ] {
                for (index, (a, b)) in direction_strokes(Vec2::X).into_iter().enumerate() {
                    let (node, transform) = crate::minimap::line_node(a, b, width);
                    arrow.spawn((
                        node,
                        transform,
                        BackgroundColor(paint),
                        Pickable::IGNORE,
                        ZIndex(if outline { 0 } else { 1 }),
                        EnemyPortraitDirectionStroke {
                            action,
                            index,
                            width,
                        },
                        Name::new(if outline {
                            format!("EnemyPortraitDirectionOutline-{}-{index}", action.target.id)
                        } else if index == 0 {
                            format!("EnemyPortraitDirection-{}", action.target.id)
                        } else {
                            format!("EnemyPortraitDirectionHead-{}-{index}", action.target.id)
                        }),
                    ));
                }
            }
        });
}

fn nearby(origin: Vec3, point: Vec3) -> bool {
    origin.is_finite()
        && point.is_finite()
        && origin.xz().distance_squared(point.xz()) <= shared::vision::HERO_SIGHT_RADIUS.powi(2)
}

/// Live locks require replicated visible nearby entities. Public scoreboard death
/// timers have separate passive roots and never carry a world entity/position.
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
    mut buttons: Query<
        (Entity, &EnemyPortraitAction, &mut Node, &mut BorderColor),
        Without<EnemyDeathPortrait>,
    >,
    mut deaths: Query<(Entity, &EnemyDeathPortrait, &mut Node), Without<EnemyPortraitAction>>,
    mut portraits: Query<(&EnemyPortraitArt, &mut PortraitView)>,
    mut timers: Query<(&EnemyRespawnLabel, &mut Text)>,
) {
    let local = local.single().ok();
    let mut visible = Vec::new();
    let mut dead = Vec::new();
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
            ));
        }
        // A public death timer wins over a stale replicated live entity, but
        // provides only identity/art/time, never a world target or position.
        if let Some(board) = &game.scoreboard {
            for row in &board.players {
                if Team::from(row.team) == *team || row.respawn_remaining_ms == 0 {
                    continue;
                }
                visible.retain(|(action, _)| action.target.id != row.player_id);
                dead.push((
                    row.player_id,
                    PortraitView {
                        art: row
                            .avatar
                            .as_deref()
                            .and_then(omoba_passport::avatars::avatar_definition)
                            .and_then(crate::passport::thumbnail_asset_path),
                        fallback: game::class_icon(row.hero_class),
                        level: None,
                        xp: 0.0,
                        grey: true,
                        strong_rim: false,
                    },
                    row.respawn_remaining_ms.div_ceil(1000),
                ));
            }
        }
    }
    // Each phone class owns its own stable indices. A death appearing or
    // expiring cannot shift any of the live right-side lock buttons.
    visible.sort_by_key(|(action, _)| (action.target.id, action.entity.to_bits()));
    visible.dedup_by_key(|(action, _)| action.target.id);
    visible.truncate(5);
    dead.sort_by_key(|(id, _, _)| *id);
    dead.dedup_by_key(|(id, _, _)| *id);
    dead.truncate(5);
    let viewport = window.single().ok().map_or_else(
        || {
            mobile
                .as_ref()
                .map_or(Vec2::new(1280.0, 720.0), |mobile| mobile.viewport)
        },
        |window| ui_viewport(window, ui_scale.as_deref()),
    );
    let layout = HudLayout::resolve(viewport, mobile.as_deref(), false);
    let phone = layout.form == crate::ui::theme::Form::Phone;
    let live_slots = layout.enemy_portrait_slots();
    let death_slots = if phone {
        layout.enemy_death_portrait_slots()
    } else {
        live_slots
    };
    // Desktop retains the original shared ID-sorted right column. Its death
    // roots are still passive, even though their geometry remains unchanged.
    let mut desktop_ids: Vec<_> = visible
        .iter()
        .map(|(action, _)| action.target.id)
        .chain(dead.iter().map(|(id, _, _)| *id))
        .collect();
    desktop_ids.sort_unstable();
    desktop_ids.dedup();
    desktop_ids.truncate(5);
    if !phone {
        visible.retain(|(action, _)| desktop_ids.contains(&action.target.id));
        dead.retain(|(id, _, _)| desktop_ids.contains(id));
    }
    let slot_index = |id, independent| {
        if phone {
            independent
        } else {
            desktop_ids.binary_search(&id).unwrap()
        }
    };
    for (button, action, mut node, mut border) in &mut buttons {
        let Some(index) = visible.iter().position(|(entry, _)| entry == action) else {
            commands.entity(button).despawn();
            continue;
        };
        let slot = live_slots[slot_index(action.target.id, index)];
        node.left = Val::Px(slot.min.x);
        node.top = Val::Px(slot.min.y);
        *border = BorderColor::all(if selected.selected_entity == Some(action.entity) {
            color::GOLD_400
        } else {
            color::STATE_DANGER
        });
    }
    for (root, death, mut node) in &mut deaths {
        let Some(index) = dead.iter().position(|(id, _, _)| *id == death.player_id) else {
            commands.entity(root).despawn();
            continue;
        };
        let slot = death_slots[slot_index(death.player_id, index)];
        if death.side != slot.width() {
            // Recreate immutable portrait anatomy if the input form changes.
            commands.entity(root).despawn();
            continue;
        }
        node.left = Val::Px(slot.min.x);
        node.top = Val::Px(slot.min.y);
    }
    for (art, mut view) in &mut portraits {
        if let Some(next) = visible
            .iter()
            .find(|(entry, _)| entry.target.id == art.0)
            .map(|(_, view)| view)
            .or_else(|| {
                dead.iter()
                    .find(|(id, _, _)| *id == art.0)
                    .map(|(_, view, _)| view)
            })
        {
            view.set_if_neq(next.clone());
        }
    }
    for (label, mut text) in &mut timers {
        let value = dead
            .iter()
            .find(|(id, _, _)| *id == label.0)
            .map_or(0, |(_, _, secs)| *secs);
        text.0 = value.to_string();
    }
    for (index, (action, view)) in visible.into_iter().enumerate() {
        if buttons
            .iter()
            .any(|(_, existing, _, _)| *existing == action)
        {
            continue;
        }
        let slot = live_slots[slot_index(action.target.id, index)];
        // Each button owns one immutable target identity for its entire life.
        // A disappearing target never transfers its pending tap to a replacement.
        commands
            .spawn((
                Button,
                portrait_node(slot),
                BackgroundColor(color::SURFACE_1_OPAQUE),
                BorderColor::all(color::STATE_DANGER),
                ZIndex(18),
                UiAction(action),
                action,
                TestId::new(format!("EnemyPortrait-{}", action.target.id)),
                Name::new(format!("EnemyPortrait-{}", action.target.id)),
            ))
            .with_children(|button| {
                spawn_portrait_art(button, view, action.target.id, slot.width());
                spawn_portrait_direction(button, action);
                spawn_target_connector(button, action);
            });
    }
    for (index, (id, view, secs)) in dead.into_iter().enumerate() {
        let slot = death_slots[slot_index(id, index)];
        if deaths
            .iter()
            .any(|(_, existing, _)| existing.player_id == id && existing.side == slot.width())
        {
            continue;
        }
        commands
            .spawn((
                portrait_node(slot),
                BackgroundColor(color::SURFACE_1_OPAQUE),
                BorderColor::all(color::STATE_DANGER),
                Pickable::IGNORE,
                ZIndex(18),
                EnemyDeathPortrait {
                    player_id: id,
                    side: slot.width(),
                },
                TestId::new(format!("EnemyDeathPortrait-{id}")),
                Name::new(format!("EnemyDeathPortrait-{id}")),
            ))
            .with_children(|portrait| {
                spawn_portrait_art(portrait, view, id, slot.width());
                portrait.spawn((
                    Text::new(secs.to_string()),
                    TextFont {
                        font_size: (if phone { 14.0 } else { 19.0 }).into(),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    ZIndex(2),
                    Pickable::IGNORE,
                    EnemyRespawnLabel(id),
                    Name::new("Enemy respawn countdown"),
                ));
            });
    }
}

fn portrait_node(slot: Rect) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(slot.min.x),
        top: Val::Px(slot.min.y),
        width: Val::Px(slot.width()),
        height: Val::Px(slot.height()),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        border: UiRect::all(Val::Px(2.0)),
        border_radius: BorderRadius::MAX,
        ..default()
    }
}

fn spawn_portrait_art(
    parent: &mut ChildSpawnerCommands,
    view: PortraitView,
    id: u64,
    cell_side: f32,
) {
    let compact = cell_side < 44.0;
    game::live_portrait(
        parent,
        view,
        PortraitSpec {
            side: cell_side - 8.0,
            ring: false,
            disc: if compact { 12.0 } else { 14.0 },
            disc_at: Vec2::new(-space::S4, -space::S4),
            icon: if compact { 16.0 } else { 24.0 },
        },
        EnemyPortraitArt(id),
    );
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
    geometry: (
        Query<&Window, With<PrimaryWindow>>,
        Option<Res<UiScale>>,
        Query<
            (&EnemyPortraitAction, &Node),
            (
                Without<EnemyPortraitDirection>,
                Without<EnemyPortraitDirectionStroke>,
                Without<EnemyTargetConnectorStroke>,
            ),
        >,
    ),
    mut connectors: Query<
        (&EnemyTargetConnectorStroke, &mut Node, &mut UiTransform),
        (
            Without<EnemyPortraitDirection>,
            Without<EnemyPortraitDirectionStroke>,
        ),
    >,
    mut arrows: Query<
        (&EnemyPortraitDirection, &mut Node),
        (
            Without<EnemyTargetConnectorStroke>,
            Without<EnemyPortraitDirectionStroke>,
        ),
    >,
    mut strokes: Query<
        (&EnemyPortraitDirectionStroke, &mut Node, &mut UiTransform),
        (
            Without<EnemyPortraitDirection>,
            Without<EnemyTargetConnectorStroke>,
        ),
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
        let position = poses.compute_global_transform(entity).ok()?.translation();
        let render = if mode.as_deref().copied()? == PlayerVisualMode::Sprite2d {
            crate::world2d::simulation_xz_to_render_xy(position).extend(0.0)
        } else {
            position
        };
        let point = camera
            .world_to_viewport(&poses.compute_global_transform(camera_entity).ok()?, render)
            .ok()?;
        let viewport = geometry
            .0
            .single()
            .ok()
            .map(|w| ui_viewport(w, geometry.1.as_deref()))
            .or_else(|| {
                camera
                    .logical_viewport_size()
                    .map(|v| world_to_ui(v, geometry.1.as_deref()))
            })?;
        Some((
            EnemyPortraitAction { entity, target },
            direction,
            world_to_ui(point, geometry.1.as_deref()),
            viewport,
        ))
    })();
    let matches = |action: EnemyPortraitAction| {
        bearing.filter(|(selected, _, _, _)| {
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
    for (stroke, mut node, mut transform) in &mut connectors {
        let segment = matches(stroke.action).and_then(|(_, _, point, viewport)| {
            let (_, button) = geometry.2.iter().find(|(a, _)| **a == stroke.action)?;
            let (Val::Px(x), Val::Px(y), Val::Px(w), Val::Px(h)) =
                (button.left, button.top, button.width, button.height)
            else {
                return None;
            };
            let origin = Vec2::new(x, y);
            let endpoints = target_connector(
                Rect::from_corners(origin, origin + Vec2::new(w, h)),
                point,
                viewport,
            )?;
            Some((
                endpoints[stroke.index].0 - origin,
                endpoints[stroke.index].1 - origin,
            ))
        });
        if let Some((a, b)) = segment {
            let (next_node, next_transform) = crate::minimap::line_node(a, b, stroke.width);
            node.set_if_neq(next_node);
            transform.set_if_neq(next_transform);
        } else if node.display != Display::None {
            node.display = Display::None;
        }
    }
    for (stroke, mut node, mut transform) in &mut strokes {
        if let Some((_, direction, _, _)) = matches(stroke.action) {
            let (a, b) = direction_strokes(direction)[stroke.index];
            let (next_node, next_transform) = crate::minimap::line_node(a, b, stroke.width);
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
    mut buttons: Query<
        (Option<&EnemyPortraitAction>, &mut Node),
        Or<(With<EnemyPortraitAction>, With<EnemyDeathPortrait>)>,
    >,
) {
    // Resolve after modal toggles so portraits disappear on the same frame
    // as Settings/shop/scoreboard opens, then reappear when gameplay resumes.
    for (_, mut node) in &mut buttons {
        node.display = if context.gameplay_allowed() {
            Display::Flex
        } else {
            Display::None
        };
    }
    for Activated { source, action } in activated.read() {
        pointer.consumed_primary_press = true;
        pointer.consumed_secondary_press = true;
        if !context.gameplay_allowed() {
            continue;
        }
        let Ok((origin, team, stats)) = local.single() else {
            continue;
        };
        if !buttons
            .get(*source)
            .is_ok_and(|(current, _)| current == Some(action))
            || action.entity == Entity::PLACEHOLDER
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

    fn death_portraits(app: &mut App) -> Vec<(Entity, EnemyDeathPortrait)> {
        app.world_mut()
            .query::<(Entity, &EnemyDeathPortrait)>()
            .iter(app.world())
            .map(|(entity, death)| (entity, *death))
            .collect()
    }

    fn death_row(id: u64, milliseconds: u32) -> shared::live_score::LiveScorePlayer {
        shared::live_score::LiveScorePlayer {
            player_id: id,
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
            respawn_remaining_ms: milliseconds,
        }
    }

    fn set_deaths(app: &mut App, rows: Vec<shared::live_score::LiveScorePlayer>) {
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .scoreboard = Some(shared::live_score::LiveScoreboard {
            players: rows,
            ..default()
        });
    }

    fn live_positions(app: &mut App) -> Vec<(u64, Val, Val)> {
        let mut positions: Vec<_> = app
            .world_mut()
            .query::<(&EnemyPortraitAction, &Node)>()
            .iter(app.world())
            .map(|(action, node)| (action.target.id, node.left, node.top))
            .collect();
        positions.sort_by_key(|(id, _, _)| *id);
        positions
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
    fn connector_reaches_visible_world_point_and_clips_offscreen_rays() {
        let portrait = Rect::from_corners(Vec2::new(705.0, 52.0), Vec2::new(749.0, 96.0));
        let view = Vec2::new(791.0, 360.0);
        for target in [
            Vec2::new(395.0, 180.0),
            Vec2::new(395.0, -240.0),
            Vec2::new(-600.0, 190.0),
            Vec2::new(1400.0, 800.0),
        ] {
            let lines = target_connector(portrait, target, view).unwrap();
            let (a, b) = lines[0];
            assert!((a - portrait.center()).length() >= 24.0 - 0.001);
            assert!(
                (b - a)
                    .normalize()
                    .dot((target - portrait.center()).normalize())
                    > 0.99999
            );
            for p in [a, b] {
                assert!(p.x >= 7.99 && p.y >= 7.99 && p.x <= view.x - 7.99 && p.y <= view.y - 7.99);
            }
            if target.x > 8.0
                && target.y > 8.0
                && target.x < view.x - 8.0
                && target.y < view.y - 8.0
            {
                assert!(b.distance(target) < 0.001);
            } else {
                assert!(
                    (b.x - 8.0).abs() < 0.01
                        || (b.y - 8.0).abs() < 0.01
                        || (b.x - view.x + 8.0).abs() < 0.01
                        || (b.y - view.y + 8.0).abs() < 0.01
                );
            }
        }
        assert!(target_connector(portrait, portrait.center(), view).is_none());
        assert!(target_connector(portrait, Vec2::splat(f32::NAN), view).is_none());
    }

    #[test]
    fn connector_is_passive_and_reaches_the_edge_for_an_offscreen_visible_enemy() {
        let (mut app, _) = direction_app();
        let actor = enemy(&mut app, 17, 20.0);
        app.world_mut()
            .resource_mut::<TargetState>()
            .selected_entity = Some(actor);
        app.world_mut()
            .resource_mut::<TargetState>()
            .selected_target = Some(TargetId {
            kind: TargetKind::Player,
            id: 17,
        });
        app.update();
        let mut lines =
            app.world_mut()
                .query::<(&EnemyTargetConnectorStroke, &Node, &Pickable, &GlobalZIndex)>();
        assert_eq!(
            lines
                .iter(app.world())
                .filter(|(_, n, _, _)| n.display != Display::None)
                .count(),
            6
        );
        for (_, _, pickable, z) in lines.iter(app.world()) {
            assert_eq!(*pickable, Pickable::IGNORE);
            assert_eq!(z.0, 10);
        }
        app.world_mut().get_mut::<CombatStats>(actor).unwrap().hp = 0.0;
        app.update();
        assert!(
            !lines
                .iter(app.world())
                .any(|(_, n, _, _)| n.display != Display::None)
        );
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
        assert_eq!(app.world().resource::<StrokeChanges>().0, 6);
        let mut strokes = app
            .world_mut()
            .query::<(&EnemyPortraitDirectionStroke, &ZIndex, &Pickable)>();
        for (stroke, layer, pickable) in strokes.iter(app.world()) {
            assert_eq!(
                layer.0,
                if stroke.width == DIRECTION_OUTLINE_WIDTH {
                    0
                } else {
                    1
                }
            );
            assert!(!pickable.is_hoverable && !pickable.should_block_lower);
        }
        app.update();
        assert_eq!(app.world().resource::<StrokeChanges>().0, 0);
        *app.world_mut().get_mut::<Transform>(camera).unwrap() = camera_pose(Vec3::X);
        app.update();
        assert_eq!(app.world().resource::<StrokeChanges>().0, 6);
    }

    #[test]
    fn direction_strokes_fit_inside_the_portrait_at_every_angle() {
        let center = Vec2::splat(DIRECTION_OVERLAY_SIDE * 0.5);
        for step in 0..64 {
            let direction = Vec2::from_angle(step as f32 * std::f32::consts::TAU / 64.0);
            for (a, b) in direction_strokes(direction) {
                for point in [a, b] {
                    assert!(
                        point.distance(center) + DIRECTION_OUTLINE_WIDTH * 0.5
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
            assert!(
                !app.world_mut()
                    .query::<(&EnemyTargetConnectorStroke, &Node)>()
                    .iter(app.world())
                    .any(|(_, node)| node.display != Display::None),
                "ineligible connector remains for case {invalid}"
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
    fn public_death_portrait_is_position_free_passive_and_expires_at_respawn() {
        let mut app = app();
        set_deaths(&mut app, vec![death_row(77, 2400)]);
        app.update();
        let deaths = death_portraits(&mut app);
        assert_eq!(deaths.len(), 1);
        let (source, death) = deaths[0];
        assert_eq!(death.player_id, 77);
        assert!(portraits(&mut app).is_empty());
        assert!(app.world().get::<EnemyPortraitAction>(source).is_none());
        assert!(
            app.world()
                .get::<UiAction<EnemyPortraitAction>>(source)
                .is_none()
        );
        assert!(app.world().get::<Button>(source).is_none());
        assert!(app.world().get::<Transform>(source).is_none());
        assert!(app.world().get::<RemotePlayer>(source).is_none());
        let pickable = app.world().get::<Pickable>(source).unwrap();
        assert!(!pickable.is_hoverable && !pickable.should_block_lower);
        let node = app.world().get::<Node>(source).unwrap();
        assert_eq!((node.width, node.height), (Val::Px(32.0), Val::Px(32.0)));
        let hud = HudLayout::phone(app.world().resource::<MobileControls>());
        assert_eq!(node.left, Val::Px(hud.allies.min.x));
        assert!(matches!(node.top, Val::Px(y) if y >= hud.allies.max.y + space::S4));
        assert!(
            app.world_mut()
                .query::<&PortraitView>()
                .iter(app.world())
                .any(|view| view.grey)
        );
        assert!(
            app.world_mut()
                .query::<&EnemyPortraitDirection>()
                .iter(app.world())
                .next()
                .is_none(),
            "public death data cannot own a spatial bearing"
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
        set_deaths(&mut app, vec![death_row(77, 999)]);
        app.update();
        assert_eq!(countdown(&mut app), "1");
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .modal_open = true;
        app.update();
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
        set_deaths(&mut app, vec![death_row(77, 0)]);
        app.update();
        assert!(death_portraits(&mut app).is_empty());
        assert!(
            portraits(&mut app).is_empty(),
            "living hidden enemies stay absent"
        );
        let live = enemy(&mut app, 77, 3.0);
        app.update();
        let (new_source, action) = portraits(&mut app)[0];
        assert_eq!(action.entity, live);
        // Even a forged queued activation using the old passive root cannot
        // borrow the newly visible live entity's targeting action.
        app.world_mut().write_message(Activated { source, action });
        app.update();
        assert!(
            app.world()
                .resource::<TargetState>()
                .selected_target
                .is_none()
        );
        app.world_mut().write_message(Activated {
            source: new_source,
            action,
        });
        app.update();
        assert_eq!(
            app.world().resource::<TargetState>().selected_entity,
            Some(live)
        );
    }

    #[test]
    fn phone_death_row_does_not_consume_or_reorder_live_lock_slots() {
        let mut app = app();
        enemy(&mut app, 20, 3.0);
        enemy(&mut app, 40, 4.0);
        app.update();
        let before = live_positions(&mut app);
        set_deaths(&mut app, vec![death_row(10, 3000), death_row(30, 1500)]);
        app.update();
        assert_eq!(live_positions(&mut app), before);
        let mut dead: Vec<_> = death_portraits(&mut app)
            .into_iter()
            .map(|(entity, death)| {
                let node = app.world().get::<Node>(entity).unwrap();
                (death.player_id, node.left, node.top)
            })
            .collect();
        dead.sort_by_key(|(id, _, _)| *id);
        let slots =
            HudLayout::phone(app.world().resource::<MobileControls>()).enemy_death_portrait_slots();
        assert_eq!(
            dead[0],
            (10, Val::Px(slots[0].min.x), Val::Px(slots[0].min.y))
        );
        assert_eq!(
            dead[1],
            (30, Val::Px(slots[1].min.x), Val::Px(slots[1].min.y))
        );
        set_deaths(&mut app, vec![death_row(30, 1000)]);
        app.update();
        assert_eq!(live_positions(&mut app), before);
        assert_eq!(death_portraits(&mut app).len(), 1);
        set_deaths(&mut app, vec![]);
        app.update();
        assert_eq!(live_positions(&mut app), before);
        assert!(death_portraits(&mut app).is_empty());
    }

    #[test]
    fn public_death_replaces_a_stale_live_snapshot_and_rejects_its_queued_tap() {
        let (mut app, _) = direction_app();
        let live = enemy(&mut app, 17, 3.0);
        app.update();
        let (source, action) = portraits(&mut app)[0];
        // The world snapshot still says alive: public death information must
        // remove the live action before an already queued click is handled.
        assert!(app.world().get::<CombatStats>(live).unwrap().is_alive());
        set_deaths(&mut app, vec![death_row(17, 2500)]);
        app.world_mut().write_message(Activated { source, action });
        app.update();
        assert!(portraits(&mut app).is_empty());
        assert_eq!(death_portraits(&mut app).len(), 1);
        assert!(app.world().get_entity(source).is_err());
        assert!(
            app.world()
                .resource::<TargetState>()
                .selected_entity
                .is_none()
        );
        assert!(!direction_shown(&mut app, 17));
        set_deaths(&mut app, vec![]);
        app.update();
        let (replacement_source, replacement_action) = portraits(&mut app)[0];
        assert_ne!(replacement_source, source);
        assert_eq!(replacement_action, action);
        app.world_mut().write_message(Activated { source, action });
        app.update();
        assert!(
            app.world()
                .resource::<TargetState>()
                .selected_entity
                .is_none()
        );
    }

    #[test]
    fn desktop_keeps_shared_column_and_rebuilds_death_anatomy_on_form_change() {
        let mut app = app();
        app.world_mut().resource_mut::<MobileControls>().enabled = false;
        enemy(&mut app, 20, 3.0);
        enemy(&mut app, 40, 4.0);
        set_deaths(&mut app, vec![death_row(30, 3000)]);
        app.update();
        let slots = HudLayout::desktop(Vec2::new(852.0, 393.0), false).enemy_portrait_slots();
        let live = live_positions(&mut app);
        assert_eq!(
            live[0],
            (20, Val::Px(slots[0].min.x), Val::Px(slots[0].min.y))
        );
        assert_eq!(
            live[1],
            (40, Val::Px(slots[2].min.x), Val::Px(slots[2].min.y))
        );
        let (old_root, _) = death_portraits(&mut app)[0];
        let node = app.world().get::<Node>(old_root).unwrap();
        assert_eq!(
            (node.left, node.top),
            (Val::Px(slots[1].min.x), Val::Px(slots[1].min.y))
        );
        assert_eq!(node.width, Val::Px(44.0));
        app.world_mut().resource_mut::<MobileControls>().enabled = true;
        app.update();
        let deaths = death_portraits(&mut app);
        assert_eq!(deaths.len(), 1);
        let (new_root, _) = deaths[0];
        assert_ne!(old_root, new_root);
        assert!(app.world().get_entity(old_root).is_err());
        assert_eq!(
            app.world().get::<Node>(new_root).unwrap().width,
            Val::Px(32.0)
        );
        assert!(app.world().get::<Button>(new_root).is_none());
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
