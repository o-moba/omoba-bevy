//! Opt-in desktop QA render target. No OS window, desktop capture, focus or
//! lock/preference changes. Virtual window metrics retain production layout.
use bevy::{
    app::{PluginGroupBuilder, ScheduleRunnerPlugin},
    camera::{CameraUpdateSystems, RenderTarget},
    prelude::*,
    render::{render_resource::TextureFormat, view::screenshot::Screenshot},
    ui::IsDefaultUiCamera,
    window::{ExitCondition, PrimaryWindow, WindowRef},
    winit::WinitPlugin,
};
use std::time::Duration;

const ENV: &str = "OMOBA_QA_OFFSCREEN";

#[derive(Resource)]
struct Settings(UVec2);

#[derive(Resource)]
pub(super) struct Surface {
    window: Entity,
    pub image: Handle<Image>,
}

fn requested(
    flag: Option<&str>,
    supported_harness: bool,
    mobile: bool,
) -> Result<bool, &'static str> {
    let Some(flag) = flag else {
        return Ok(false);
    };
    if flag != "1" {
        return Err("OMOBA_QA_OFFSCREEN must be1 or unset");
    }
    if mobile || !supported_harness {
        return Err("Offscreen QA requires a desktop fair-play or Creator Loop harness");
    }
    Ok(true)
}

/// Called before DefaultPlugins so Winit never creates an OS window.
pub(crate) fn configure(app: &mut App, plugins: PluginGroupBuilder) -> PluginGroupBuilder {
    let set = |name| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    let flag = std::env::var(ENV).ok();
    let enabled = requested(
        flag.as_deref(),
        set("OMOBA_FAIR_PLAY_QA_OUTPUT") || set("OMOBA_CREATOR_LOOP_QA"),
        cfg!(any(target_os = "ios", target_os = "android")),
    )
    .expect("invalid offscreen QA configuration");
    if !enabled {
        return plugins;
    }
    let size = |name, default| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(default)
            .clamp(200, 3840)
    };
    app.insert_resource(Settings(UVec2::new(
        size("OMOBA_QA_WIDTH", 852),
        size("OMOBA_QA_HEIGHT", 393),
    )));
    plugins
        .set(WindowPlugin {
            primary_window: None,
            exit_condition: ExitCondition::DontExit,
            ..default()
        })
        .disable::<WinitPlugin>()
}

pub(super) struct OffscreenQaPlugin;
impl Plugin for OffscreenQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(settings) = app.world().get_resource::<Settings>() else {
            return;
        };
        let pixels = settings.0;
        let mut window = crate::platform::window_plugin()
            .primary_window
            .unwrap_or_default();
        window.visible = false;
        window.resolution.set_scale_factor_override(Some(1.));
        window
            .resolution
            .set_physical_resolution(pixels.x, pixels.y);
        let window = app
            .world_mut()
            .spawn((window, PrimaryWindow, Name::new("QaVirtualWindow")))
            .id();
        let image = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::new_target_texture(
                pixels.x,
                pixels.y,
                TextureFormat::Rgba8UnormSrgb,
                None,
            ));
        app.insert_resource(Surface { window, image })
            .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1. / 60.,
            )))
            .add_systems(PostStartup, redirect)
            .add_systems(
                PostUpdate,
                redirect
                    .before(CameraUpdateSystems)
                    .before(bevy::ui::UiSystems::Prepare),
            );
    }
}

fn primary_target(target: &RenderTarget, window: Entity) -> bool {
    matches!(target, RenderTarget::Window(WindowRef::Primary))
        || matches!(target,RenderTarget::Window(WindowRef::Entity(entity)) if *entity==window)
}

/// Preserve the production main camera as the default UI camera after its
/// target stops being WindowRef::Primary. Preview image cameras are untouched.
fn redirect(
    mut commands: Commands,
    surface: Res<Surface>,
    mut cameras: Query<(Entity, &mut RenderTarget, Has<crate::camera::MainCamera>), With<Camera>>,
) {
    for (entity, mut target, main) in &mut cameras {
        if primary_target(&target, surface.window) {
            *target = RenderTarget::Image(surface.image.clone().into());
            if main {
                commands.entity(entity).insert(IsDefaultUiCamera);
            }
        }
    }
}

pub(super) fn screenshot(image: Option<&Handle<Image>>) -> Screenshot {
    image.map_or_else(Screenshot::primary_window, |image| {
        Screenshot::image(image.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_supported_desktop_opt_in_is_required() {
        assert_eq!(requested(None, false, false), Ok(false));
        assert_eq!(requested(Some("1"), true, false), Ok(true));
        for (flag, harness, mobile) in [("0", true, false), ("1", false, false), ("1", true, true)]
        {
            assert!(requested(Some(flag), harness, mobile).is_err());
        }
    }

    #[test]
    fn redirects_main_window_without_stealing_preview_targets_and_reaches_late_cameras() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let mut images = Assets::<Image>::default();
        let image = images.add(Image::default());
        let preview_image = images.add(Image::default());
        app.insert_resource(Surface {
            window,
            image: image.clone(),
        })
        .add_systems(Update, redirect);
        let main = app
            .world_mut()
            .spawn((Camera2d, crate::camera::MainCamera))
            .id();
        let preview = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                RenderTarget::Image(preview_image.clone().into()),
            ))
            .id();
        let other = app.world_mut().spawn_empty().id();
        let other_camera = app
            .world_mut()
            .spawn((Camera2d, RenderTarget::Window(WindowRef::Entity(other))))
            .id();
        app.update();
        assert_eq!(
            app.world()
                .get::<RenderTarget>(main)
                .unwrap()
                .normalize(Some(window)),
            RenderTarget::Image(image.clone().into()).normalize(Some(window))
        );
        assert!(app.world().get::<IsDefaultUiCamera>(main).is_some());
        assert!(app.world().get::<IsDefaultUiCamera>(preview).is_none());
        assert_eq!(
            app.world()
                .get::<RenderTarget>(preview)
                .unwrap()
                .normalize(Some(window)),
            RenderTarget::Image(preview_image.into()).normalize(Some(window))
        );
        assert_eq!(
            app.world()
                .get::<RenderTarget>(other_camera)
                .unwrap()
                .normalize(Some(window)),
            RenderTarget::Window(WindowRef::Entity(other)).normalize(Some(window))
        );
        let late = app
            .world_mut()
            .spawn((Camera2d, RenderTarget::Window(WindowRef::Entity(window))))
            .id();
        app.update();
        assert_eq!(
            app.world()
                .get::<RenderTarget>(late)
                .unwrap()
                .normalize(Some(window)),
            RenderTarget::Image(image.into()).normalize(Some(window))
        );
        let mut defaults = app
            .world_mut()
            .query_filtered::<Entity, With<IsDefaultUiCamera>>();
        assert_eq!(defaults.iter(app.world()).collect::<Vec<_>>(), vec![main]);
        let mut default_camera =
            bevy::ecs::system::SystemState::<bevy::ui::DefaultUiCamera>::new(app.world_mut());
        assert_eq!(default_camera.get(app.world()).unwrap().get(), Some(main));
    }
}
