//! Opt-in native avatar evidence. The service metadata is a labeled loopback
//! fixture; model installation and input use the normal production systems.
use super::*;
use crate::frontend::preview::{PreviewCamera, PreviewStatus};
use bevy::input::touch::{TouchInput, TouchPhase};

pub(super) struct AvatarQaPlugin {
    pub directory: PathBuf,
}

impl Plugin for AvatarQaPlugin {
    fn build(&self, app: &mut App) {
        let dimension = |name: &str, fallback| {
            std::env::var(name)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(fallback)
        };
        app.insert_resource(AvatarQa {
            directory: self.directory.clone(),
            pixels: UVec2::new(
                dimension("OMOBA_QA_WIDTH", 1280),
                dimension("OMOBA_QA_HEIGHT", 720),
            ),
            slug: std::env::var("OMOBA_AVATAR_QA_SLUG").ok(),
            live_registry: std::env::var("OMOBA_AVATAR_QA_LIVE_REGISTRY").as_deref() == Ok("1"),
            stage: 0,
            frames: 0,
            started: Instant::now(),
            in_flight: false,
            readbacks: Vec::new(),
            captures: Vec::new(),
            finished: false,
            drag: None,
            sdk_pressed: false,
            ready_frames: 0,
        })
        .insert_resource(ScreenDriverPaused(true))
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(Startup, setup)
        .add_systems(PreUpdate, input.after(bevy::ui::UiSystems::Focus))
        .add_systems(
            PostUpdate,
            capture
                .after(bevy::ui::UiSystems::Layout)
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}

#[derive(Resource)]
struct AvatarQa {
    directory: PathBuf,
    pixels: UVec2,
    slug: Option<String>,
    live_registry: bool,
    stage: usize,
    frames: u32,
    started: Instant,
    in_flight: bool,
    readbacks: Vec<usize>,
    captures: Vec<serde_json::Value>,
    finished: bool,
    drag: Option<(Vec2, f32)>,
    sdk_pressed: bool,
    ready_frames: u32,
}

impl AvatarQa {
    fn filename(&self) -> &'static str {
        match self.stage {
            0 => "01-collection-default.png",
            1 => "02-collection-default-drag.png",
            2 if self.slug.is_none() => "03-collection-studio-status.png",
            2 => "03-collection-studio-loading.png",
            3 => "04-collection-studio-ready.png",
            _ => "05-collection-studio-drag.png",
        }
    }
}

#[derive(Component)]
struct AvatarShot(usize);

fn setup(mut commands: Commands, mut next: ResMut<NextState<AppScreen>>) {
    next.set(AppScreen::Collection);
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(2.0),
            right: Val::Px(8.0),
            ..default()
        },
        Text::new(
            if std::env::var("OMOBA_AVATAR_QA_LIVE_REGISTRY").as_deref() == Ok("1") {
                "QA · local Studio · automated input"
            } else {
                "QA · SDK service fixture · no external ownership claim"
            },
        ),
        TextFont {
            font_size: 8.0,
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::BLACK),
        FocusPolicy::Pass,
        ZIndex(300),
        Name::new("AvatarQaWatermark"),
    ));
}

#[allow(clippy::too_many_arguments)]
fn input(
    mut qa: ResMut<AvatarQa>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    screen: Res<State<AppScreen>>,
    mut next: ResMut<NextState<AppScreen>>,
    preview: Res<AvatarPreview>,
    nodes: Query<(&Name, &ComputedNode, &UiGlobalTransform)>,
    mut scrolls: Query<(&Name, &ComputedNode, &mut ScrollPosition)>,
    mut buttons: Query<(&Name, &mut Interaction), With<Button>>,
    mut touch: MessageWriter<TouchInput>,
) {
    if qa.finished || qa.in_flight {
        return;
    }
    let Ok((window_entity, mut window)) = windows.single_mut() else {
        return;
    };
    window.resolution.set_scale_factor_override(Some(1.0));
    if window.resolution.physical_width() != qa.pixels.x
        || window.resolution.physical_height() != qa.pixels.y
    {
        window
            .resolution
            .set_physical_resolution(qa.pixels.x, qa.pixels.y);
    }
    if *screen.get() != AppScreen::Collection {
        next.set(AppScreen::Collection);
        return;
    }
    qa.frames += 1;
    if qa.stage >= 2 {
        for (name, node, mut scroll) in &mut scrolls {
            if name.as_str() == "CollectionGrid" {
                scroll.y = ((node.content_size().y - node.size().y) * node.inverse_scale_factor())
                    .max(0.0);
            }
        }
    }
    if matches!(qa.stage, 1 | 4) {
        let Some((_, _, transform)) = nodes
            .iter()
            .find(|(name, _, _)| name.as_str() == "AvatarPreviewSurface")
        else {
            return;
        };
        let center = transform.translation / window.scale_factor();
        if qa.frames == 4 {
            qa.drag = Some((center, preview.yaw));
            touch.write(TouchInput {
                window: window_entity,
                id: 710,
                phase: TouchPhase::Started,
                position: center,
                force: None,
            });
        } else if qa.frames == 6 {
            let center = qa.drag.map_or(center, |(start, _)| start);
            touch.write(TouchInput {
                window: window_entity,
                id: 710,
                phase: TouchPhase::Moved,
                position: center + Vec2::new(64.0, 0.0),
                force: None,
            });
        } else if qa.frames == 8 {
            let center = qa.drag.map_or(center, |(start, _)| start);
            touch.write(TouchInput {
                window: window_entity,
                id: 710,
                phase: TouchPhase::Ended,
                position: center + Vec2::new(64.0, 0.0),
                force: None,
            });
        }
    }
    if qa.stage == 2 && !qa.sdk_pressed && qa.frames > 4 {
        let Some(slug) = qa.slug.clone() else {
            return;
        };
        for (name, mut interaction) in &mut buttons {
            if name.as_str() == format!("CollectionTile-{slug}") {
                *interaction = Interaction::Pressed;
                qa.sdk_pressed = true;
            }
        }
    }
}

fn abort(qa: &mut AvatarQa, reason: &str, exit: &mut MessageWriter<AppExit>) {
    qa.finished = true;
    let _ = std::fs::create_dir_all(&qa.directory);
    let _ = std::fs::write(
        qa.directory.join("qa-failure.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "reason":reason, "stage":qa.stage, "captures":qa.captures,
        }))
        .unwrap(),
    );
    error!("AVATAR_QA failed: {reason}");
    exit.write(AppExit::error());
}

#[allow(clippy::too_many_arguments)]
fn capture(
    mut commands: Commands,
    mut qa: ResMut<AvatarQa>,
    preview: Res<AvatarPreview>,
    screen: Res<State<AppScreen>>,
    nodes: Query<(
        &Name,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&InheritedVisibility>,
    )>,
    scenes: Query<(&Name, &SceneRoot)>,
    cameras: Query<&Camera, With<PreviewCamera>>,
    thumbnails: Res<crate::team::AvatarThumbnails>,
    images: Res<Assets<Image>>,
    asset_server: Res<AssetServer>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.finished {
        return;
    }
    if qa.started.elapsed() > Duration::from_secs(120) {
        abort(&mut qa, "avatar capture timeout", &mut exit);
        return;
    }
    if qa.in_flight {
        if !qa.readbacks.contains(&qa.stage)
            || !qa
                .directory
                .join(qa.filename())
                .metadata()
                .is_ok_and(|file| file.len() > 32)
        {
            return;
        }
        qa.stage += 1;
        qa.frames = 0;
        qa.in_flight = false;
        qa.drag = None;
        qa.ready_frames = 0;
        if qa.stage == if qa.slug.is_some() { 5 } else { 3 } {
            let _ = std::fs::write(qa.directory.join("qa-summary.json"), serde_json::to_vec_pretty(&serde_json::json!({
                "status":"passed", "scenario":"avatar-collection", "version":env!("CARGO_PKG_VERSION"),
                "sdk_service_fixture":!qa.live_registry, "local_live_registry":qa.live_registry, "actual_model_download_and_validation":qa.slug.is_some(),
                "raw_touch_input_injected":true, "manual_or_physical_device_input":false, "captures":qa.captures,
            })).unwrap());
            qa.finished = true;
            if let Ok(window) = windows.single() {
                commands.entity(window).despawn();
            }
            exit.write(AppExit::Success);
        }
        return;
    }
    if *screen.get() != AppScreen::Collection || qa.frames < 24 {
        return;
    }
    let wanted_slug = qa.slug.as_deref();
    let is_sdk = wanted_slug.is_some() && qa.stage >= 2;
    if qa.stage == 0 && preview.status != PreviewStatus::Ready {
        return;
    }
    if is_sdk && preview.slug.as_deref() != wanted_slug {
        return;
    }
    let store_state = wanted_slug
        .filter(|_| is_sdk)
        .map(omoba_passport::store::model_state);
    if qa.stage == 2 && is_sdk && !qa.live_registry && preview.status != PreviewStatus::Loading {
        if preview.status == PreviewStatus::Ready {
            abort(
                &mut qa,
                "SDK fixture needs model delay to capture pending state",
                &mut exit,
            );
        }
        return;
    }
    if qa.stage >= 3
        && (preview.status != PreviewStatus::Ready
            || store_state != Some(omoba_passport::store::ModelState::Ready))
    {
        return;
    }
    if qa.stage == 3 {
        // The model can become Ready after Collection details updated in the
        // same frame. Wait for their next layouts before recording the image.
        qa.ready_frames += 1;
        if qa.ready_frames < 16 {
            return;
        }
    }
    if qa.stage == 2
        && !is_sdk
        && matches!(
            omoba_passport::store::catalogue_status(),
            omoba_passport::store::CatalogueStatus::Loading { .. }
        )
    {
        return;
    }
    let viewport = qa.pixels.as_vec2();
    let mut records = Vec::new();
    for (name, node, transform, visible) in &nodes {
        if name.as_str().starts_with("Collection") || name.as_str().starts_with("Avatar") {
            let size = node.size() * transform.to_scale_angle_translation().0.abs();
            let min = transform.translation - size * 0.5;
            records.push(serde_json::json!({"name":name.as_str(), "min":min.to_array(), "size":size.to_array(),
                "visible":visible.is_none_or(|value| value.get()), "fits_viewport":fits(min, size, viewport)}));
        }
    }
    let required = [
        "CollectionScreen",
        "CollectionBack",
        "AvatarPreviewSurface",
        "AvatarAutoSpin",
    ];
    if required.iter().any(|name| {
        !records.iter().any(|record| {
            record["name"] == *name && record["visible"] == true && record["fits_viewport"] == true
        })
    }) {
        abort(
            &mut qa,
            "Collection essential controls leave viewport",
            &mut exit,
        );
        return;
    }
    if qa.stage == 0 && ((preview.yaw - std::f32::consts::PI).abs() > 0.001 || preview.auto_spin) {
        abort(
            &mut qa,
            "default avatar is not stationary and front-facing",
            &mut exit,
        );
        return;
    }
    if matches!(qa.stage, 1 | 4)
        && !qa
            .drag
            .is_some_and(|(_, initial)| (preview.yaw - initial - 64.0 * 0.012).abs() < 0.001)
    {
        abort(
            &mut qa,
            "raw touch drag did not rotate naturally",
            &mut exit,
        );
        return;
    }
    let scene_path = scenes
        .iter()
        .find(|(name, _)| name.as_str().starts_with("AvatarPreviewModel-"))
        .and_then(|(_, scene)| asset_server.get_path(scene.0.id()))
        .map(|path| path.to_string());
    let thumb_ready = preview
        .slug
        .as_ref()
        .and_then(|slug| thumbnails.0.get(slug))
        .is_some_and(|handle| images.contains(handle.id()));
    if qa.stage >= 3
        && (!thumb_ready
            || !scene_path
                .as_ref()
                .is_some_and(|path| path.starts_with("ekza://")))
    {
        abort(
            &mut qa,
            "SDK preview/thumbnail did not resolve through the mounted validated asset source",
            &mut exit,
        );
        return;
    }
    let record = serde_json::json!({
        "file":qa.filename(), "pixels":qa.pixels.to_array(), "preview_slug":preview.slug,
        "preview_yaw":preview.yaw, "auto_spin":preview.auto_spin,
        "preview_status":format!("{:?}", preview.status), "store_model_state":format!("{store_state:?}"),
        "preview_scene_path":scene_path, "thumbnail_ready":thumb_ready,
        "preview_camera_active":cameras.iter().any(|camera| camera.is_active),
        "clips":preview.clips.iter().map(|clip| &clip.name).collect::<Vec<_>>(),
        "catalogue_status":format!("{:?}", omoba_passport::store::catalogue_status()),
        "nodes":records,
    });
    qa.captures.push(record);
    if std::fs::create_dir_all(&qa.directory).is_err() {
        abort(&mut qa, "cannot create capture output", &mut exit);
        return;
    }
    let stage = qa.stage;
    commands
        .spawn((Screenshot::primary_window(), AvatarShot(stage)))
        .observe(save_to_disk(qa.directory.join(qa.filename())))
        .observe(
            |event: On<ScreenshotCaptured>, shots: Query<&AvatarShot>, mut qa: ResMut<AvatarQa>| {
                if let Ok(shot) = shots.get(event.entity)
                    && event.image.width() == qa.pixels.x
                    && event.image.height() == qa.pixels.y
                {
                    qa.readbacks.push(shot.0);
                }
            },
        );
    qa.in_flight = true;
}
