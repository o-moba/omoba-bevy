//! Explicit local manifest/render proof. Does not invent an approved placement.
//! Set OMOBA_SPONSOR_QA_DIR and point the normal Registry override at a local
//! integration server. The ordinary production downloader remains in charge.
use crate::{camera::MainCamera, sponsorship::Sculpture};
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
/// Isolated art capture: actual map assets and production placement plugin,
/// without the gameplay shell, network handshake or unrelated preview cameras.
pub(crate) fn run(store_root: Option<PathBuf>) {
    let mut app = App::new();
    if let Some(root) = store_root {
        app.register_asset_source(
            omoba_passport::store::ASSET_SOURCE,
            bevy::asset::io::AssetSourceBuilder::platform_default(&root.to_string_lossy(), None),
        );
    }
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: omoba_passport::assets::client_asset_root()
                    .to_string_lossy()
                    .into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "OMOBA | local sculpture art proof".into(),
                    resolution: bevy::window::WindowResolution::new(1280, 720)
                        .with_scale_factor_override(1.),
                    ..default()
                }),
                ..default()
            }),
    )
    .insert_resource(crate::sprite::PlayerVisualMode::Models3d)
    .insert_resource(ClearColor(Color::srgb(0.12, 0.19, 0.20)))
    .insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 550.,
        ..default()
    })
    .add_plugins((
        crate::maps::MapsPlugin,
        crate::verdant3d::Verdant3dPlugin,
        crate::sponsorship::SponsorshipPlugin,
    ));
    app.add_plugins(SponsorshipQaPlugin);
    if let AppExit::Error(code) = app.run() {
        std::process::exit(i32::from(code.get()));
    }
}
pub(crate) struct SponsorshipQaPlugin;
#[derive(Resource)]
struct Audit {
    dir: PathBuf,
    start: Instant,
    ready: Option<Instant>,
    stage: u8,
    pending: bool,
    rotation: Option<Quat>,
    max_motion: f32,
    identity: Option<String>,
    retry_at: Option<Instant>,
    colored_readbacks: usize,
}
impl Plugin for SponsorshipQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(dir) = std::env::var_os("OMOBA_SPONSOR_QA_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        app.insert_resource(bevy::winit::WinitSettings::continuous())
            .insert_resource(Audit {
                dir,
                start: Instant::now(),
                ready: None,
                stage: 0,
                pending: false,
                rotation: None,
                max_motion: 0.,
                identity: None,
                retry_at: None,
                colored_readbacks: 0,
            })
            .add_systems(Startup, setup)
            .add_systems(PreUpdate, focus)
            .add_systems(PostUpdate, diagnostics)
            .add_systems(
                PostUpdate,
                capture.after(bevy::transform::TransformSystems::Propagate),
            );
    }
}
fn setup(mut commands: Commands) {
    let target = Vec3::from_array(shared::sponsorship::slot_position()) + Vec3::Y;
    commands.spawn((
        Camera3d::default(),
        MainCamera,
        Projection::Perspective(PerspectiveProjection {
            fov: 45_f32.to_radians(),
            ..default()
        }),
        Transform::from_translation(target + Vec3::new(5.5, 4.0, 7.5)).looking_at(target, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(-20., 40., 10.).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(20.),
            bottom: Val::Px(18.),
            ..default()
        },
        Text::new("LOCAL QA • Registry manifest • decorative community sculpture"),
        TextFont {
            font_size: 16.0.into(),
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0., 0., 0., 0.8)),
        ZIndex(999),
    ));
}
fn capture(
    mut commands: Commands,
    mut qa: ResMut<Audit>,
    roots: Query<&Sculpture>,
    nodes: Query<(&Name, &Transform)>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage >= 3 {
        return;
    }
    if qa.start.elapsed() > Duration::from_secs(90) {
        let _ = std::fs::create_dir_all(&qa.dir);
        let _=std::fs::write(qa.dir.join("result.json"),serde_json::json!({"pass":false,"error":"No authorized rendered waving cat before timeout","identity":qa.identity,"motion":qa.max_motion}).to_string());
        qa.stage = 3;
        exit.write(AppExit::error());
        return;
    }
    let Ok(root) = roots.single() else { return };
    qa.identity = Some(root.identity.clone());
    let Some((_, paw)) = nodes
        .iter()
        .find(|(name, _)| name.as_str() == "LuckyPawWave")
    else {
        return;
    };
    let first = *qa.rotation.get_or_insert(paw.rotation);
    qa.max_motion = qa.max_motion.max(first.angle_between(paw.rotation).abs());
    let ready = *qa.ready.get_or_insert_with(Instant::now);
    if qa.retry_at.is_some_and(|at| Instant::now() < at) {
        return;
    }
    if qa.pending || ready.elapsed() < Duration::from_secs(8) {
        return;
    }
    if qa.stage == 2 {
        let pass = qa.max_motion > 0.05 && qa.colored_readbacks == 2;
        let result = serde_json::json!({"pass":pass,"source":"normal Registry active manifest and verified HTTP GLB download","viewport":[1280,720],"language":"en","identity":qa.identity,"slot":shared::sponsorship::SLOT_ID,"position":shared::sponsorship::slot_position(),"pawRotationRadians":qa.max_motion,"gameplay":"Isolated art capture with real Verdant assets and production placement plugin; no UDP gameplay session","physicalMobileTested":false,"nonblackReadbacks":qa.colored_readbacks});
        let written = std::fs::write(
            qa.dir.join("result.json"),
            serde_json::to_vec_pretty(&result).unwrap(),
        );
        qa.stage = 3;
        exit.write(if pass && written.is_ok() {
            AppExit::Success
        } else {
            AppExit::error()
        });
        return;
    }
    if qa.stage == 1 && ready.elapsed() < Duration::from_millis(8750) {
        return;
    }
    if std::fs::create_dir_all(&qa.dir).is_err() {
        qa.stage = 3;
        exit.write(AppExit::error());
        return;
    }
    let file = if qa.stage == 0 {
        "01-sculpture.png"
    } else {
        "02-sculpture-wave.png"
    };
    qa.pending = true;
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(qa.dir.join(file)))
        .observe(|shot: On<ScreenshotCaptured>, mut qa: ResMut<Audit>| {
            qa.pending = false;
            let colored = shot.image.data.as_ref().map_or(0, |bytes| {
                bytes
                    .chunks_exact(4)
                    .filter(|rgba| rgba[..3].iter().any(|v| *v > 24))
                    .count()
            });
            if colored > 10000 {
                qa.colored_readbacks += 1;
                qa.stage += 1;
            } else {
                qa.retry_at = Some(Instant::now() + Duration::from_secs(1));
            }
        });
}

fn focus(
    windows: Query<Entity, With<PrimaryWindow>>,
    mut retry: Local<(u8, Option<Instant>)>,
    _main: bevy::ecs::system::NonSendMarker,
) {
    if retry.0 >= 3
        || retry
            .1
            .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
    {
        return;
    }
    let Ok(entity) = windows.single() else { return };
    bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        if let Some(window) = windows.get_window(entity)
            && !window.has_focus()
        {
            window.focus_window();
            retry.0 += 1;
            retry.1 = Some(Instant::now());
        }
    });
}

/// Focused diagnostics for failed native readback; no production systems depend on this.
fn diagnostics(
    cameras: Query<(
        Entity,
        &Camera,
        &bevy::camera::RenderTarget,
        &GlobalTransform,
    )>,
    windows: Query<(Entity, &Window)>,
    mut recorded: Local<bool>,
    time: Res<Time>,
) {
    if *recorded || time.elapsed_secs() < 3. {
        return;
    }
    *recorded = true;
    info!("Sculpture QA camera count {}", cameras.iter().count());
    for (entity, camera, target, transform) in &cameras {
        info!(
            "Sculpture QA camera {entity:?}: active={}, target={target:?}, computed={:?}, transform={transform:?}",
            camera.is_active,
            camera.physical_target_size()
        );
    }
    for (entity, window) in &windows {
        info!(
            "Sculpture QA window {entity:?}: {}x{}, focused={}, visible={}",
            window.physical_width(),
            window.physical_height(),
            window.focused,
            window.visible
        );
    }
}
