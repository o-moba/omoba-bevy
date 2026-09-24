//! 3D presentation of server-owned team sight. This module never grants vision.
use crate::{
    camera::MainCamera,
    maps::MapLayout,
    net::{GameState, GameStateSnapshot},
    sprite::PlayerVisualMode,
};
use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    ui::FocusPolicy,
};
use shared::vision::{VisionSource, brush_at, brush_layout};

const MASK_WIDTH: u32 = 160;
const MASK_HEIGHT: u32 = 96;
const MAP_SIZE: u32 = 80;
const MAX_RENDER_SOURCES: usize = 256;
const UPDATE_SECS: f32 = 0.075;
const DARK_ALPHA: f32 = 0.64;

pub(crate) struct TeamVisionPlugin;
impl Plugin for TeamVisionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(
                Update,
                (sync_visibility, attach_minimap_mask, animate_brush),
            )
            .add_systems(
                PostUpdate,
                update_mask
                    .after(bevy::camera::CameraUpdateSystems)
                    .after(bevy::transform::TransformSystems::Propagate),
            );
    }
}
#[derive(Resource)]
pub(crate) struct VisionPresentation {
    pub active: bool,
    pub texture: Handle<Image>,
    pub updates: u64,
    pub darkest: u8,
    pub clearest: u8,
    minimap: Handle<Image>,
    elapsed: f32,
    identity: Option<(u64, u64)>,
}
#[derive(Component)]
struct FogOverlay;
#[derive(Component)]
struct MinimapFog;
#[derive(Component)]
struct BrushStatus;
#[derive(Component)]
struct BrushArt;
#[derive(Component)]
struct GrassTuft {
    phase: f32,
    yaw: f32,
}

fn mask_image(width: u32, height: u32) -> Image {
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; (width * height * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::linear();
    image
}
fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mode: Res<PlayerVisualMode>,
    layout: Res<MapLayout>,
) {
    let texture = images.add(mask_image(MASK_WIDTH, MASK_HEIGHT));
    let minimap = images.add(mask_image(MAP_SIZE, MAP_SIZE));
    commands.insert_resource(VisionPresentation {
        active: false,
        texture: texture.clone(),
        minimap,
        updates: 0,
        darkest: 0,
        clearest: 0,
        elapsed: UPDATE_SECS,
        identity: None,
    });
    commands.spawn((
        Name::new("Team fog of war"),
        FogOverlay,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.),
            top: Val::Px(0.),
            width: Val::Percent(100.),
            height: Val::Percent(100.),
            display: Display::None,
            ..default()
        },
        ImageNode::new(texture),
        GlobalZIndex(-90),
        FocusPolicy::Pass,
        Pickable::IGNORE,
    ));
    commands.spawn((
        Name::new("BrushStatus"),
        BrushStatus,
        Text::new(""),
        crate::ui_theme::text(12.),
        TextColor(Color::srgb(0.7, 1., 0.78)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(74.),
            left: Val::Percent(50.),
            padding: UiRect::axes(Val::Px(10.), Val::Px(4.)),
            border_radius: BorderRadius::all(Val::Px(5.)),
            display: Display::None,
            ..default()
        },
        UiTransform::from_translation(Val2::new(Val::Percent(-50.), Val::Px(0.))),
        BackgroundColor(Color::srgba(0.035, 0.075, 0.07, 0.90)),
        GlobalZIndex(25),
        FocusPolicy::Pass,
        Pickable::IGNORE,
    ));
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    let tuft = meshes.add(grass_mesh());
    let tints = [
        Color::srgb(0.22, 0.40, 0.22),
        Color::srgb(0.32, 0.49, 0.27),
        Color::srgb(0.43, 0.56, 0.30),
    ];
    let grass = tints.map(|base_color| {
        materials.add(StandardMaterial {
            base_color,
            perceptual_roughness: 1.,
            cull_mode: None,
            double_sided: true,
            ..default()
        })
    });
    let floor = materials.add(StandardMaterial {
        base_color: Color::srgba(0.12, 0.24, 0.14, 0.42),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        cull_mode: None,
        ..default()
    });
    let disc = meshes.add(Circle::new(1.));
    for zone in brush_layout() {
        let y = layout.terrain_height_3d(zone.center[0], zone.center[1]);
        commands
            .spawn((
                Name::new(format!("Gameplay brush {}", zone.id)),
                BrushArt,
                Transform::from_xyz(zone.center[0], y, zone.center[1]),
                Visibility::Visible,
            ))
            .with_children(|parent| {
                parent.spawn((
                    Mesh3d(disc.clone()),
                    MeshMaterial3d(floor.clone()),
                    Transform::from_xyz(0., 0.035, 0.)
                        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                        .with_scale(Vec3::splat(zone.radius)),
                ));
                for i in 0..31 {
                    let phase = i as f32 * 2.39996 + zone.id as f32;
                    let distance = (i as f32 / 30.).sqrt() * (zone.radius - 0.45);
                    let yaw = phase * 1.7;
                    parent.spawn((
                        Mesh3d(tuft.clone()),
                        MeshMaterial3d(grass[i % 3].clone()),
                        Transform::from_xyz(phase.cos() * distance, 0., phase.sin() * distance)
                            .with_rotation(Quat::from_rotation_y(yaw))
                            .with_scale(Vec3::splat(0.85 + (i % 4) as f32 * 0.06)),
                        GrassTuft { phase, yaw },
                    ));
                }
            });
    }
}
fn grass_mesh() -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for leaf in 0..11 {
        let angle = leaf as f32 * 2.39996;
        let side = Vec3::new(angle.cos(), 0., angle.sin());
        let lean = Vec3::new(-angle.sin(), 0., angle.cos());
        let height = 0.85 + (leaf % 4) as f32 * 0.10;
        let base = positions.len() as u32;
        for p in [
            -side * 0.06,
            side * 0.06,
            Vec3::Y * height * 0.6 + lean * 0.2 - side * 0.10,
            Vec3::Y * height * 0.6 + lean * 0.2 + side * 0.10,
            Vec3::Y * height + lean * 0.48,
        ] {
            positions.push(p.to_array());
        }
        indices.extend([
            base,
            base + 1,
            base + 2,
            base + 1,
            base + 3,
            base + 2,
            base + 2,
            base + 3,
            base + 4,
        ]);
    }
    let count = positions.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 1., 0.]; count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0., 0.]; count])
    .with_inserted_indices(Indices::U32(indices))
}
fn animate_brush(time: Res<Time>, mut tufts: Query<(&GrassTuft, &mut Transform)>) {
    for (tuft, mut pose) in &mut tufts {
        pose.rotation = Quat::from_rotation_y(tuft.yaw)
            * Quat::from_rotation_z((time.elapsed_secs() * 1.7 + tuft.phase).sin() * 0.06);
    }
}
fn attach_minimap_mask(
    mut commands: Commands,
    art: Res<VisionPresentation>,
    container: Query<Entity, With<crate::minimap::MinimapContainer>>,
    existing: Query<(), With<MinimapFog>>,
) {
    if !existing.is_empty() {
        return;
    }
    let Ok(container) = container.single() else {
        return;
    };
    commands.spawn((
        Name::new("Minimap team fog"),
        MinimapFog,
        ChildOf(container),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.),
            top: Val::Px(0.),
            width: Val::Percent(100.),
            height: Val::Percent(100.),
            ..default()
        },
        ImageNode::new(art.minimap.clone()),
        ZIndex(1),
        FocusPolicy::Pass,
        Pickable::IGNORE,
    ));
}
fn sync_visibility(
    game: Option<Res<GameStateSnapshot>>,
    mode: Res<PlayerVisualMode>,
    mut art: ResMut<VisionPresentation>,
    mut overlay: Query<
        &mut Node,
        (
            Or<(With<FogOverlay>, With<MinimapFog>)>,
            Without<BrushStatus>,
        ),
    >,
    mut status: Query<(&mut Text, &mut Node, &mut TextColor), With<BrushStatus>>,
    mut brush: Query<&mut Visibility, With<BrushArt>>,
) {
    art.active = *mode == PlayerVisualMode::Models3d
        && game
            .as_ref()
            .is_some_and(|g| matches!(g.state, GameState::Running) && g.vision.is_some());
    for mut node in &mut overlay {
        node.display = if art.active {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut visible in &mut brush {
        *visible = if *mode == PlayerVisualMode::Models3d {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for (mut text, mut node, mut color) in &mut status {
        let vision = game.as_ref().and_then(|g| g.vision.as_ref());
        let show = art.active && vision.is_some_and(|v| v.local_brush.is_some());
        node.display = if show { Display::Flex } else { Display::None };
        if let Some(vision) = vision.filter(|_| show) {
            let line = if vision.local_hidden {
                "BRUSH · CONCEALED"
            } else {
                "BRUSH · REVEALED"
            };
            if text.0 != line {
                text.0 = line.into();
            }
            color.0 = if vision.local_hidden {
                Color::srgb(0.65, 1., 0.73)
            } else {
                Color::srgb(1., 0.79, 0.40)
            };
        }
    }
    if !art.active {
        art.identity = None;
        art.elapsed = UPDATE_SECS;
    }
}
/// Only presentation feathers the hard authoritative boundary; hidden enemies
/// have already been removed from the recipient snapshot.
fn fog_alpha(sources: &[VisionSource], point: [f32; 2]) -> u8 {
    if !point.iter().all(|x| x.is_finite()) {
        return (DARK_ALPHA * 255.) as u8;
    }
    let brush = brush_at(point);
    let mut light = 0.0_f32;
    for source in sources.iter().take(MAX_RENDER_SOURCES) {
        if !source.radius.is_finite()
            || source.radius <= 0.
            || !source.position.iter().all(|x| x.is_finite())
        {
            continue;
        }
        let distance = (point[0] - source.position[0]).hypot(point[1] - source.position[1]);
        let t = ((source.radius + 1.5 - distance) / 3.).clamp(0., 1.);
        let mut strength = t * t * (3. - 2. * t);
        if brush.is_some() && brush_at(source.position) != brush {
            strength *= 0.35;
        }
        light = light.max(strength);
        if light >= 1. {
            break;
        }
    }
    ((1. - light) * DARK_ALPHA * 255.).round() as u8
}
fn ground_point(view_from_clip: Mat4, world_from_view: Mat4, uv: Vec2) -> Option<[f32; 2]> {
    let ndc = Vec2::new(uv.x * 2. - 1., 1. - uv.y * 2.);
    let near = view_from_clip.project_point3(ndc.extend(1.));
    let far = view_from_clip.project_point3(ndc.extend(f32::EPSILON));
    let origin = world_from_view.transform_point3(near);
    let direction = world_from_view.transform_vector3(far - near);
    if !origin.is_finite() || !direction.is_finite() || direction.y.abs() < 0.0001 {
        return None;
    }
    let t = -origin.y / direction.y;
    (t >= 0.).then(|| (origin + direction * t).xz().to_array())
}
fn update_mask(
    time: Res<Time>,
    game: Option<Res<GameStateSnapshot>>,
    mut art: ResMut<VisionPresentation>,
    cameras: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    layout: Res<MapLayout>,
    mut images: ResMut<Assets<Image>>,
) {
    if !art.active {
        return;
    }
    let Some(game) = game else {
        return;
    };
    let Some(vision) = game.vision.as_ref() else {
        return;
    };
    art.elapsed += time.delta_secs();
    let identity = (game.meta.server_epoch, game.meta.match_id);
    if art.elapsed < UPDATE_SECS && art.identity == Some(identity) {
        return;
    }
    let Ok((camera, pose)) = cameras.single() else {
        return;
    };
    if camera.logical_viewport_size().is_none() {
        return;
    }
    let view = camera.clip_from_view().inverse();
    let world = pose.to_matrix();
    if !view.is_finite() || !world.is_finite() {
        return;
    }
    art.elapsed = 0.;
    art.identity = Some(identity);
    let mut darkest = 0;
    let mut clearest = 255;
    if let Some(image) = images.get_mut(&art.texture)
        && let Some(data) = image.data.as_mut()
    {
        for y in 0..MASK_HEIGHT {
            for x in 0..MASK_WIDTH {
                let uv = Vec2::new(
                    (x as f32 + 0.5) / MASK_WIDTH as f32,
                    (y as f32 + 0.5) / MASK_HEIGHT as f32,
                );
                let alpha = ground_point(view, world, uv)
                    .map_or((DARK_ALPHA * 255.) as u8, |p| fog_alpha(&vision.sources, p));
                let i = ((y * MASK_WIDTH + x) * 4) as usize;
                data[i..i + 4].copy_from_slice(&[8, 18, 24, alpha]);
                darkest = darkest.max(alpha);
                clearest = clearest.min(alpha);
            }
        }
    }
    if let Some(image) = images.get_mut(&art.minimap)
        && let Some(data) = image.data.as_mut()
    {
        for y in 0..MAP_SIZE {
            for x in 0..MAP_SIZE {
                let uv = Vec2::new(
                    (x as f32 + 0.5) / MAP_SIZE as f32,
                    (y as f32 + 0.5) / MAP_SIZE as f32,
                );
                let p = layout.min + Vec2::new(1. - uv.y, uv.x) * layout.size();
                let i = ((y * MAP_SIZE + x) * 4) as usize;
                data[i..i + 4].copy_from_slice(&[
                    4,
                    10,
                    16,
                    fog_alpha(&vision.sources, p.to_array()),
                ]);
            }
        }
    }
    art.darkest = darkest;
    art.clearest = clearest;
    art.updates += 1;
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feathered_fog_tracks_radial_sight_and_brush_without_invalid_pixels() {
        let brush = brush_layout()[0];
        let outside = [brush.center[0] + brush.radius + 2., brush.center[1]];
        let source = VisionSource {
            position: outside,
            radius: 32.,
        };
        assert!(fog_alpha(&[source.clone()], brush.center) > 80);
        assert_eq!(fog_alpha(&[source], outside), 0);
        let inside = VisionSource {
            position: brush.center,
            radius: 32.,
        };
        assert_eq!(fog_alpha(&[inside], brush.center), 0);
        assert!(fog_alpha(&[], outside) > 150);
        assert!(fog_alpha(&[], [f32::NAN, 0.]) > 150);
        let source = VisionSource {
            position: [0., 0.],
            radius: 32.,
        };
        let alphas: Vec<_> = (30..=35)
            .map(|x| fog_alpha(&[source.clone()], [x as f32, 0.]))
            .collect();
        assert!(alphas.windows(2).all(|p| p[0] <= p[1]));
    }
    #[test]
    fn projected_ground_roundtrip_matches_camera_ray_and_orbit() {
        let projection = Mat4::perspective_infinite_reverse_rh(0.8, 1.6, 0.1);
        for eye in [Vec3::new(0., 40., 40.), Vec3::new(40., 50., 0.)] {
            let camera = Transform::from_translation(eye)
                .looking_at(Vec3::ZERO, Vec3::Y)
                .to_matrix();
            let p = ground_point(projection.inverse(), camera, Vec2::splat(0.5)).unwrap();
            assert!(Vec2::from_array(p).length() < 0.001);
        }
    }
    #[test]
    fn brush_and_fog_assets_are_fixed_noninteractive_and_reset_in_lobby() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<MapLayout>()
            .insert_resource(PlayerVisualMode::Models3d)
            .init_resource::<GameStateSnapshot>()
            .add_plugins(TeamVisionPlugin);
        app.update();
        let count = app.world().entities().len();
        let images = app.world().resource::<Assets<Image>>().len();
        assert_eq!(images, 2);
        assert_eq!(
            app.world_mut()
                .query::<&GrassTuft>()
                .iter(app.world())
                .count(),
            brush_layout().len() * 31
        );
        let mut game = app.world_mut().resource_mut::<GameStateSnapshot>();
        game.state = GameState::Running;
        game.vision = Some(shared::vision::TeamVision {
            sources: vec![],
            local_brush: Some(1),
            local_hidden: true,
        });
        for _ in 0..30 {
            app.update();
        }
        assert!(app.world().resource::<VisionPresentation>().active);
        let (text, node) = app
            .world_mut()
            .query_filtered::<(&Text, &Node), With<BrushStatus>>()
            .single(app.world())
            .unwrap();
        assert_eq!(text.0, "BRUSH · CONCEALED");
        assert_eq!(node.display, Display::Flex);
        let pick = app
            .world_mut()
            .query_filtered::<&Pickable, With<FogOverlay>>()
            .single(app.world())
            .unwrap();
        assert_eq!(*pick, Pickable::IGNORE);
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Lobby;
        app.update();
        assert!(!app.world().resource::<VisionPresentation>().active);
        assert_eq!(app.world().entities().len(), count);
        assert_eq!(app.world().resource::<Assets<Image>>().len(), images);
    }
}
