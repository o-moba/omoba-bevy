//! Explicit local manifest/render proof. Does not invent an approved placement.
//! Set OMOBA_SPONSOR_QA_DIR and point the normal Registry override at a local
//! integration server. The ordinary production downloader remains in charge.
use crate::{camera::MainCamera, sponsorship::Sculpture};
use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
        mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel},
    },
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
    .add_plugins((
        crate::i18n::I18nPlugin::from_environment(),
        crate::input_context::InputContextPlugin,
        crate::ui::UiKitPlugin,
    ))
    .insert_resource(crate::net::GameStateSnapshot {
        state: crate::net::GameState::Running,
        ..default()
    })
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
    step_at: Instant,
    card_rotation: Option<Quat>,
    card_motion: f32,
    scrolled_to_end: bool,
    input_frames: usize,
    leaked_input_frames: usize,
    pointer_opened: bool,
    pointer_closed: bool,
    escape_closed: bool,
    description_scalars: usize,
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
                step_at: Instant::now(),
                card_rotation: None,
                card_motion: 0.,
                scrolled_to_end: false,
                input_frames: 0,
                leaked_input_frames: 0,
                pointer_opened: false,
                pointer_closed: false,
                escape_closed: false,
                description_scalars: 0,
            })
            .add_systems(Startup, setup)
            .add_systems(PreUpdate, focus)
            .add_systems(PostUpdate, diagnostics)
            .add_systems(
                Update,
                audit_input.in_set(crate::input_context::InputContextSet::Actions),
            )
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
        Transform::from_translation(target + Vec3::new(5.5, 4.0, 7.5))
            .looking_at(target + Vec3::new(7.5, 0., -5.5).normalize() * 3.4, Vec3::Y),
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
/// Low-level mouse/keyboard events flow through the same input and UI systems
/// as the native window. This audit is an input-policy proof, not a UDP match.
fn audit_input(
    mut qa: ResMut<Audit>,
    mouse: Res<ButtonInput<MouseButton>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    context: Res<crate::input_context::GameplayInputContext>,
) {
    if mouse.just_pressed(MouseButton::Left)
        || mouse.just_released(MouseButton::Left)
        || keyboard.just_pressed(KeyCode::Escape)
    {
        qa.input_frames += 1;
        if context.gameplay_allowed() || context.camera_allowed() {
            qa.leaked_input_frames += 1;
        }
    }
}
#[derive(bevy::ecs::system::SystemParam)]
struct QaInput<'w, 's> {
    windows: Query<'w, 's, (Entity, &'static mut Window), With<PrimaryWindow>>,
    mouse: MessageWriter<'w, MouseButtonInput>,
    wheel: MessageWriter<'w, MouseWheel>,
    keys: MessageWriter<'w, KeyboardInput>,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<MainCamera>>,
    controls: Query<'w, 's, (&'static crate::ui::TestId, &'static UiGlobalTransform)>,
}
impl QaInput<'_, '_> {
    fn pointer(&mut self, point: Option<Vec2>, state: ButtonState) {
        let Ok((entity, mut window)) = self.windows.single_mut() else {
            return;
        };
        if let Some(point) = point {
            window.set_cursor_position(Some(point));
        }
        self.mouse.write(MouseButtonInput {
            button: MouseButton::Left,
            state,
            window: entity,
        });
    }
    fn cat_point(&self) -> Option<Vec2> {
        let (camera, transform) = self.cameras.single().ok()?;
        camera
            .world_to_viewport(
                transform,
                Vec3::from_array(shared::sponsorship::slot_position()) + Vec3::Y,
            )
            .ok()
    }
}
fn capture(
    mut commands: Commands,
    mut qa: ResMut<Audit>,
    roots: Query<&Sculpture>,
    nodes: Query<(&Name, &Transform)>,
    info: Res<crate::sponsorship::info::SculptureInfo>,
    scrolls: Query<(&ComputedNode, &ScrollPosition), With<crate::sponsorship::info::InfoScroll>>,
    mut input: QaInput,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage >= 14 {
        return;
    }
    if qa.start.elapsed() > Duration::from_secs(90) {
        let _ = std::fs::create_dir_all(&qa.dir);
        let _ = std::fs::write(qa.dir.join("result.json"), serde_json::json!({"pass":false,"error":"Native sculpture info proof timed out","stage":qa.stage,"identity":qa.identity,"motion":qa.max_motion,"inputFrames":qa.input_frames,"leakedInputFrames":qa.leaked_input_frames}).to_string());
        qa.stage = 14;
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
    if info.open.is_some() {
        let first = *qa.card_rotation.get_or_insert(paw.rotation);
        qa.card_motion = qa.card_motion.max(first.angle_between(paw.rotation).abs());
    }
    let ready = *qa.ready.get_or_insert_with(Instant::now);
    if qa.retry_at.is_some_and(|at| Instant::now() < at)
        || qa.pending
        || ready.elapsed() < Duration::from_secs(8)
    {
        return;
    }
    if qa.stage == 2 {
        let point = input.cat_point();
        input.pointer(point, ButtonState::Pressed);
        qa.stage = 3;
        qa.step_at = Instant::now();
        return;
    }
    if qa.stage == 3 {
        if let Some(open) = &info.open {
            qa.pointer_opened = true;
            qa.description_scalars = open.description.chars().count();
            input.pointer(None, ButtonState::Released);
            qa.stage = 4;
            qa.step_at = Instant::now();
        }
        return;
    }
    if matches!(qa.stage, 1 | 4 | 5 | 7 | 10 | 12 | 13)
        && qa.step_at.elapsed() < Duration::from_millis(800)
    {
        return;
    }
    if qa.stage == 6 {
        if let Ok((entity, _)) = input.windows.single() {
            input.wheel.write(MouseWheel {
                unit: MouseScrollUnit::Line,
                phase: bevy::input::touch::TouchPhase::Moved,
                x: 0.,
                y: -10_000.,
                window: entity,
            });
        }
        qa.stage = 7;
        qa.step_at = Instant::now();
        return;
    }
    if qa.stage == 7 {
        if let Ok((node, scroll)) = scrolls.single() {
            let maximum =
                (node.content_size().y - node.size().y).max(0.) * node.inverse_scale_factor;
            qa.scrolled_to_end = scroll.y > 0. && (scroll.y - maximum).abs() < 1.;
        }
    }
    if qa.stage == 8 {
        let point = input
            .controls
            .iter()
            .find(|(id, _)| id.as_str() == "SculptureInfoClose")
            .map(|(_, transform)| transform.translation);
        input.pointer(point, ButtonState::Pressed);
        qa.stage = 9;
        qa.step_at = Instant::now();
        return;
    }
    if qa.stage == 9 {
        if info.open.is_none() {
            qa.pointer_closed = true;
            input.pointer(None, ButtonState::Released);
            qa.stage = 10;
            qa.step_at = Instant::now();
        }
        return;
    }
    if qa.stage == 10 {
        let point = input.cat_point();
        input.pointer(point, ButtonState::Pressed);
        qa.stage = 11;
        return;
    }
    if qa.stage == 11 {
        if info.open.is_some() {
            input.pointer(None, ButtonState::Released);
            qa.stage = 12;
            qa.step_at = Instant::now();
        }
        return;
    }
    if qa.stage == 12 {
        if let Ok((window, _)) = input.windows.single() {
            input.keys.write(KeyboardInput {
                window,
                key_code: KeyCode::Escape,
                logical_key: Key::Escape,
                text: None,
                state: ButtonState::Pressed,
                repeat: false,
            });
        }
        qa.stage = 13;
        qa.step_at = Instant::now();
        return;
    }
    if qa.stage == 13 {
        qa.escape_closed = info.open.is_none();
        let pass = qa.max_motion > 0.05
            && qa.card_motion > 0.05
            && qa.colored_readbacks == 5
            && qa.pointer_opened
            && qa.pointer_closed
            && qa.escape_closed
            && qa.scrolled_to_end
            && qa.description_scalars == 1000
            && qa.input_frames >= 6
            && qa.leaked_input_frames == 0;
        let result = serde_json::json!({"pass":pass,"source":"normal Registry active manifest and verified HTTP GLB download","viewport":[1280,720],"language":"en","identity":qa.identity,"slot":shared::sponsorship::SLOT_ID,"position":shared::sponsorship::slot_position(),"pawRotationRadians":qa.max_motion,"cardPawRotationRadians":qa.card_motion,"pointerOpened":qa.pointer_opened,"pointerClosed":qa.pointer_closed,"escapeClosed":qa.escape_closed,"scrolledToEnd":qa.scrolled_to_end,"descriptionScalars":qa.description_scalars,"inputFrames":qa.input_frames,"leakedInputFrames":qa.leaked_input_frames,"gameplay":"Isolated art capture with real Verdant assets and production input/modal/placement plugins; low-level pointer and keyboard messages; no UDP gameplay session","physicalMobileTested":false,"nonblackReadbacks":qa.colored_readbacks});
        let written = std::fs::write(
            qa.dir.join("result.json"),
            serde_json::to_vec_pretty(&result).unwrap(),
        );
        qa.stage = 14;
        exit.write(if pass && written.is_ok() {
            AppExit::Success
        } else {
            AppExit::error()
        });
        return;
    }
    if std::fs::create_dir_all(&qa.dir).is_err() {
        qa.stage = 14;
        exit.write(AppExit::error());
        return;
    }
    let file = match qa.stage {
        0 => "01-sculpture.png",
        1 => "02-sculpture-wave.png",
        4 => "03-info.png",
        5 => "04-info-wave.png",
        7 => "05-info-scrolled.png",
        _ => return,
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
                qa.step_at = Instant::now();
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
