//! Minimap-only presentation of the shared collision map and local move order.
//! This module owns UI entities; geometry/search and movement stay elsewhere.
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

use crate::{
    combat::CombatStats,
    maps::MapLayout,
    minimap::{MINIMAP_INNER_SIZE, MinimapContainer, line_node, map_point},
    net::{ClientSession, GameState, GameStateSnapshot},
    player::{MovementRoute, MovementTarget, Player},
};

const ROUTE_COLOR: Color = Color::srgba(0.65, 1.0, 0.84, 0.98);
const ROUTE_WIDTH: f32 = 2.3;

pub(crate) struct MinimapRoutePlugin;
impl Plugin for MinimapRoutePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RouteUi>()
            .add_systems(Update, setup_collision_overlay)
            .add_systems(PostUpdate, update_route.before(bevy::ui::UiSystems::Layout));
    }
}

#[derive(Resource, Default)]
struct RouteUi {
    segments: Vec<Entity>,
    destination: Option<Entity>,
}

#[derive(Component)]
pub(crate) struct RouteSegment(pub(crate) usize);
#[derive(Component)]
pub(crate) struct RouteDestination;
#[derive(Component)]
struct CollisionOverlay;

fn setup_collision_overlay(
    mut commands: Commands,
    containers: Query<Entity, With<MinimapContainer>>,
    existing: Query<(), With<CollisionOverlay>>,
    layout: Res<MapLayout>,
    mut images: ResMut<Assets<Image>>,
) {
    if !existing.is_empty() {
        return;
    }
    let Ok(container) = containers.single() else {
        return;
    };
    // Rasterize once at the minimap's own logical resolution, avoiding thousands
    // of extra polygon UI entities. The mask is exactly the hero walkability
    // footprint used by collision, including trunk clearance, rather than canopy.
    let size = MINIMAP_INNER_SIZE as u32;
    let mut pixels = vec![0; (size * size * 4) as usize];
    let navigation = shared::navigation::world_navigation();
    for y in 0..size {
        for x in 0..size {
            let point = layout.min
                + Vec2::new(
                    1.0 - (y as f32 + 0.5) / size as f32,
                    (x as f32 + 0.5) / size as f32,
                ) * layout.size();
            if !navigation.point_clear(point.to_array()) {
                let offset = ((y * size + x) * 4) as usize;
                pixels[offset..offset + 4].copy_from_slice(&[19, 51, 31, 245]);
            }
        }
    }
    let image = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        ImageNode::new(images.add(image)),
        ZIndex(1),
        CollisionOverlay,
        ChildOf(container),
        Name::new("MinimapSolidForest"),
    ));
}

fn update_route(
    mut commands: Commands,
    mut ui: ResMut<RouteUi>,
    containers: Query<Entity, With<MinimapContainer>>,
    players: Query<
        (&Transform, &CombatStats, &MovementRoute),
        (With<Player>, With<MovementTarget>),
    >,
    layout: Res<MapLayout>,
    session: Option<Res<ClientSession>>,
    game: Option<Res<GameStateSnapshot>>,
) {
    let Ok(container) = containers.single() else {
        return;
    };
    let running = session.as_ref().is_none_or(|s| s.join_confirmed())
        && game
            .as_ref()
            .is_none_or(|g| matches!(g.state, GameState::Running));
    let route = players
        .single()
        .ok()
        .filter(|(_, stats, _)| running && stats.is_alive());
    let points: Vec<_> = route
        .map(|(transform, _, route)| {
            std::iter::once(transform.translation)
                .chain(route.waypoints.iter().copied())
                .map(|p| map_point(*layout, p))
                .collect()
        })
        .unwrap_or_default();
    // Reuse the largest route's pool; normal movement only changes node geometry
    // and hides consumed segments. Search already bounds the waypoint count.
    let required = points.len().saturating_sub(1);
    while ui.segments.len() < required {
        let index = ui.segments.len();
        ui.segments.push(
            commands
                .spawn((
                    Node::default(),
                    BackgroundColor(ROUTE_COLOR),
                    ZIndex(6),
                    RouteSegment(index),
                    ChildOf(container),
                    Name::new("MinimapMovementRoute"),
                ))
                .id(),
        );
    }
    for (index, entity) in ui.segments.iter().enumerate() {
        if index < required {
            let (node, transform) = line_node(points[index], points[index + 1], ROUTE_WIDTH);
            commands.entity(*entity).insert((node, transform));
        } else {
            commands.entity(*entity).insert(Node {
                display: Display::None,
                ..default()
            });
        }
    }
    let entity = *ui.destination.get_or_insert_with(|| {
        commands
            .spawn((
                Node::default(),
                BorderColor::all(ROUTE_COLOR),
                ZIndex(7),
                RouteDestination,
                ChildOf(container),
                Name::new("MinimapRouteDestination"),
            ))
            .id()
    });
    let node = if let Some((_, _, route)) = route.filter(|_| required > 0) {
        let point = map_point(*layout, route.destination);
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(point.x - 4.0),
            top: Val::Px(point.y - 4.0),
            width: Val::Px(8.0),
            height: Val::Px(8.0),
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::MAX,
            ..default()
        }
    } else {
        Node {
            display: Display::None,
            ..default()
        }
    };
    commands.entity(entity).insert(node);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_route_pool_tracks_progress_and_clears_on_cancel_or_death() {
        let mut app = App::new();
        app.init_resource::<RouteUi>()
            .init_resource::<MapLayout>()
            .add_systems(Update, update_route);
        app.world_mut().spawn(MinimapContainer);
        let start = Vec3::new(-8.0, 0.5, 0.0);
        let middle = Vec3::new(0.0, 0.5, 4.0);
        let end = Vec3::new(8.0, 0.5, 0.0);
        let hero = app
            .world_mut()
            .spawn((
                Player,
                Transform::from_translation(start),
                CombatStats::default(),
                MovementTarget { target: end },
                MovementRoute {
                    requested_target: end,
                    structure_revision: 0,
                    destination: end,
                    waypoints: vec![middle, end],
                },
            ))
            .id();
        app.update();
        let pool = app.world().resource::<RouteUi>().segments.clone();
        assert_eq!(pool.len(), 2);
        let expected = map_point(MapLayout::default(), start)
            .distance(map_point(MapLayout::default(), middle));
        assert_eq!(
            app.world().get::<Node>(pool[0]).unwrap().width,
            Val::Px(expected)
        );
        app.world_mut()
            .get_mut::<Transform>(hero)
            .unwrap()
            .translation = middle;
        app.world_mut()
            .get_mut::<MovementRoute>(hero)
            .unwrap()
            .waypoints
            .remove(0);
        app.update();
        assert_eq!(app.world().resource::<RouteUi>().segments, pool);
        assert_eq!(
            app.world().get::<Node>(pool[1]).unwrap().display,
            Display::None
        );
        app.world_mut().get_mut::<CombatStats>(hero).unwrap().hp = 0.0;
        app.update();
        assert!(
            pool.iter()
                .all(|e| app.world().get::<Node>(*e).unwrap().display == Display::None)
        );
        let destination = app.world().resource::<RouteUi>().destination.unwrap();
        assert_eq!(
            app.world().get::<Node>(destination).unwrap().display,
            Display::None
        );
        app.world_mut().get_mut::<CombatStats>(hero).unwrap().hp = 100.0;
        app.world_mut().entity_mut(hero).remove::<MovementTarget>();
        app.update();
        assert!(
            pool.iter()
                .all(|e| app.world().get::<Node>(*e).unwrap().display == Display::None)
        );
    }
}
