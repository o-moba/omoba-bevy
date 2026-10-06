//! Distant matte painting below the authored island, in world space.
//! One static draw, no collision, navigation, replicated state or per-frame work.
use crate::maps::MapLayout;
use bevy::{
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    light::{NotShadowCaster, NotShadowReceiver},
    math::Affine2,
    prelude::*,
};

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
    // Keep forest details at world scale instead of stretching a 2k image
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
            uv_transform: Affine2::from_scale(Vec2::splat(4.0)),
            base_color: Color::srgb(0.78, 0.84, 0.82),
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(center.x, -10.0, center.y),
        NotShadowCaster,
        NotShadowReceiver,
    ));
}
