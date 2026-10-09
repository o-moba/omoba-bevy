//! Directional dash intent; collision and accepted movement remain server-owned.
use crate::{
    camera::MainCamera, input_context::GameplayInputContext, player::Player,
    sprite::PlayerVisualMode,
};
use bevy::prelude::*;

pub(crate) struct DashPreviewPlugin;
impl Plugin for DashPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            draw.after(crate::net::NetworkGroundingSet)
                .before(bevy::transform::TransformSystems::Propagate),
        );
    }
}
fn draw(
    mut gizmos: Gizmos,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    context: Res<GameplayInputContext>,
    local: Query<
        (
            &Transform,
            &crate::domain::CombatStats,
            &crate::net::PlayerUtility,
        ),
        With<Player>,
    >,
    camera: Query<Entity, With<MainCamera>>,
    transforms: bevy::transform::helper::TransformHelper,
    mode: Res<PlayerVisualMode>,
    layout: Res<crate::maps::MapLayout>,
    game: Res<crate::net::GameStateSnapshot>,
    structures: Query<
        (
            &Transform,
            &crate::net::StructureKind,
            &crate::domain::CombatStats,
        ),
        With<crate::net::NetworkStructure>,
    >,
) {
    let Some(mobile) = mobile.filter(|m| m.enabled && m.focused && m.landscape) else {
        return;
    };
    let Some(aim) = mobile.dash_aim() else {
        return;
    };
    let (Ok((hero, stats, utility)), Ok(camera)) = (local.single(), camera.single()) else {
        return;
    };
    if !context.gameplay_allowed() || !stats.is_alive() || utility.state.dash_remaining_secs > 0.0 {
        return;
    }
    let Ok(camera) = transforms.compute_global_transform(camera) else {
        return;
    };
    let direction = crate::player::mobile_screen_direction(aim, &camera, *mode).normalize_or_zero();
    if direction.length_squared() < 0.01 {
        return;
    }
    let from = hero.translation;
    let requested = layout.clamp_position(from + direction * shared::utility::DASH_DISTANCE);
    let mut solids = crate::navigation::skill_terrain(Some(&game));
    solids.extend(
        structures
            .iter()
            .filter(|(_, _, stats)| stats.is_alive())
            .map(|(pose, kind, _)| shared::navigation::Disc {
                center: pose.translation.xz().to_array(),
                radius: crate::navigation::structure_collision_radius(*kind)
                    - shared::navigation::HERO_RADIUS,
            }),
    );
    let landing = shared::navigation::world_navigation().blink_landing(
        from.xz().to_array(),
        requested.xz().to_array(),
        &solids,
    );
    let end = Vec3::new(landing[0], from.y, landing[1]);
    let color = Color::srgba(0.28, 0.9, 1.0, 0.95);
    if *mode == PlayerVisualMode::Sprite2d {
        let a = crate::world2d::simulation_xz_to_render_xy(from);
        let b = crate::world2d::simulation_xz_to_render_xy(end);
        gizmos.arrow_2d(a, b, color);
        gizmos.circle_2d(b, 0.45, color);
    } else {
        let ground = |p: Vec3| Vec3::new(p.x, layout.terrain_height_3d(p.x, p.z) + 0.18, p.z);
        gizmos
            .arrow(ground(from), ground(end), color)
            .with_tip_length(0.5);
        gizmos.circle(
            Isometry3d::new(
                ground(end),
                Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            ),
            0.45,
            color,
        );
    }
}
