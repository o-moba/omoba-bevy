//! Distant matte painting below the authored island, in world space.
//! One static draw, no collision, navigation, replicated state or per-frame work.
use crate::camera::{CAMERA_DISTANCE, CAMERA_HEIGHT};
use crate::maps::MapLayout;
use bevy::{
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    light::{NotShadowCaster, NotShadowReceiver},
    math::{Affine2, Mat2},
    prelude::*,
};

/// Painting tiles along the camera's depth axis (world X). Two tiles make
/// one upright tile long enough for everything the camera can reach.
const DEPTH_TILES: f32 = 2.0;
/// The upright tile is centred this far beyond the map centre: at the
/// largest zoom the camera sees about 80 m past the far edge of the island
/// and 25 m before the near one.
const DEPTH_BIAS: f32 = 28.0;

/// Maps the plane's UV (u along world X, v along world Z) into the painting.
///
/// The painting is an oblique view with "up" at its top edge, and the
/// gameplay camera looks along +X with +Z to its right. So the image's
/// vertical axis follows -X (top edge far from the camera) and its
/// horizontal axis follows +Z. Tiles are mirrored, which joins their edges;
/// a tile mirrored in depth would stand on its head, so the depth tile is
/// sized and placed to cover the whole reachable view by itself. Depth tiles
/// are longer than wide by the camera's pitch, which cancels the ground
/// plane's foreshortening on screen.
fn painting_uv(size: Vec2) -> Affine2 {
    let pitch = CAMERA_HEIGHT / CAMERA_HEIGHT.hypot(CAMERA_DISTANCE);
    let width_tiles = size.y / (size.x / DEPTH_TILES * pitch);
    Affine2::from_mat2_translation(
        Mat2::from_cols(Vec2::new(0.0, -DEPTH_TILES), Vec2::new(width_tiles, 0.0)),
        Vec2::new(
            // An unmirrored tile is centred on the map centre (v = 0.5).
            0.5 - width_tiles * 0.5,
            // Tile [0, 1] is unmirrored and centred on the biased depth.
            0.5 + DEPTH_TILES * (0.5 + DEPTH_BIAS / size.x),
        ),
    )
}

pub(super) fn spawn(
    mut commands: Commands,
    layout: Res<MapLayout>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // The authored island's lowest cliff layer reaches -8.95 m. Keep the
    // painting below it, with enough overscan for the supported mobile zoom.
    let size = layout.size() + Vec2::splat(480.0);
    let center = (layout.min + layout.max) * 0.5;
    // Keep forest details near world scale instead of stretching a 2k image
    // over the entire overscan. Mirrored wrapping joins the source edges.
    let painting = server
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| {
            settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::MirrorRepeat,
                address_mode_v: ImageAddressMode::MirrorRepeat,
                ..ImageSamplerDescriptor::linear()
            });
        })
        .load("verdant/surroundings.png");
    commands.spawn((
        Name::new("Verdant / distant forest surroundings"),
        Mesh3d(meshes.add(Plane3d::default().mesh().size(size.x, size.y))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(painting),
            uv_transform: painting_uv(size),
            base_color: Color::srgb(0.78, 0.84, 0.82),
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(center.x, -10.0, center.y),
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mirrored-repeat sampler coordinate folded into the image, 0..1.
    fn mirrored(coordinate: f32) -> f32 {
        let tile = coordinate.rem_euclid(2.0);
        if tile <= 1.0 { tile } else { 2.0 - tile }
    }

    /// Image position sampled `offset` metres (world X, Z) from the centre.
    fn sample(size: Vec2, offset: Vec2) -> Vec2 {
        let uv = Vec2::splat(0.5) + offset / size;
        let image = painting_uv(size).transform_point2(uv);
        Vec2::new(mirrored(image.x), mirrored(image.y))
    }

    #[test]
    fn painting_is_upright_for_the_camera_everywhere_it_can_look() {
        let size = MapLayout::default().size() + Vec2::splat(480.0);
        let half = MapLayout::default().size().x * 0.5;
        // Farther from the camera (+X) is higher in the image, with no
        // mirrored (upside-down) tile from 25 m before the near edge to 80 m
        // past the far one.
        let mut previous = f32::MAX;
        let mut depth = -half - 25.0;
        while depth <= half + 80.0 {
            let v = sample(size, Vec2::new(depth, 0.0)).y;
            assert!(v < previous, "depth {depth}: {v} after {previous}");
            previous = v;
            depth += 1.0;
        }
        // Screen right (+Z) is image right around the map centre.
        assert!(sample(size, Vec2::new(0.0, 10.0)).x > sample(size, Vec2::ZERO).x);
    }

    #[test]
    fn painting_is_not_stretched_on_screen() {
        let size = MapLayout::default().size() + Vec2::splat(480.0);
        let pitch = CAMERA_HEIGHT / CAMERA_HEIGHT.hypot(CAMERA_DISTANCE);
        let step = 10.0;
        let depth = (sample(size, Vec2::new(step, 0.0)) - sample(size, Vec2::ZERO)).length();
        let width = (sample(size, Vec2::new(0.0, step)) - sample(size, Vec2::ZERO)).length();
        // A metre of depth covers `pitch` of a metre of width on screen.
        assert!((depth / pitch - width).abs() < 1e-4, "{depth} {width}");
    }
}
