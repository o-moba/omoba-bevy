//! Display-paced rendering and a small measured frame-rate readout.
//! The preference is a ceiling; thermal, low-power and GPU limits still apply.
// i18n-strict
use std::time::Duration;

use bevy::{
    prelude::*,
    winit::{UpdateMode, WinitSettings},
};
use serde::{Deserialize, Serialize};

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct RenderSettings {
    pub(crate) fps_limit: u16,
    pub(crate) show_fps: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            fps_limit: 60,
            show_fps: true,
        }
    }
}

impl RenderSettings {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            fps_limit: if self.fps_limit == 120 { 120 } else { 60 },
            show_fps: self.show_fps,
        }
    }
}

pub(crate) struct RenderSettingsPlugin;

impl Plugin for RenderSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderSettings>()
            .init_resource::<MeasuredFps>()
            .init_resource::<FramePacingDiagnostics>()
            .add_systems(Startup, spawn_fps_readout)
            .add_systems(
                Update,
                (measure_fps, collect_pacing_diagnostics, update_fps_readout).chain(),
            );
        #[cfg(target_os = "ios")]
        app.add_systems(Update, apply_ios_pacing);
        #[cfg(not(target_os = "ios"))]
        app.add_systems(Update, apply_timer_pacing);
    }
}

/// Input events are queued for the next frame; touch sampling must not bypass
/// the chosen cap. The display link owns iOS wake-ups; other platforms use
/// Winit's supported deadline loop. Neither path sleeps a render worker.
fn focused_pacing(settings: RenderSettings, native_display_link: bool) -> UpdateMode {
    UpdateMode::Reactive {
        wait: if native_display_link {
            // A one-second watchdog handles lifecycle transitions if the native
            // link is temporarily paused. Duration::MAX overflows Winit's
            // deadline arithmetic and can leave an old timer active.
            Duration::from_secs(1)
        } else {
            Duration::from_secs_f64(1.0 / f64::from(settings.sanitized().fps_limit))
        },
        react_to_device_events: false,
        react_to_window_events: false,
        react_to_user_events: native_display_link,
    }
}

#[cfg(not(target_os = "ios"))]
fn apply_timer_pacing(settings: Res<RenderSettings>, pacing: Option<ResMut<WinitSettings>>) {
    if !settings.is_changed() {
        return;
    }
    if let Some(mut pacing) = pacing {
        pacing.focused_mode = focused_pacing(*settings, false);
        pacing.unfocused_mode = UpdateMode::reactive_low_power(Duration::from_secs(1));
    }
}

#[cfg(any(target_os = "ios", test))]
static DISPLAY_PROXY: std::sync::OnceLock<
    winit::event_loop::EventLoopProxy<bevy::winit::WinitUserEvent>,
> = std::sync::OnceLock::new();

#[cfg(any(target_os = "ios", test))]
extern "C" fn display_tick() {
    if let Some(proxy) = DISPLAY_PROXY.get() {
        let _ = proxy.send_event(bevy::winit::WinitUserEvent::WakeUp);
    }
}

#[cfg(any(target_os = "ios", test))]
fn apply_ios_pacing(
    _main_thread: bevy::ecs::system::NonSendMarker,
    settings: Res<RenderSettings>,
    proxy: Option<Res<bevy::winit::EventLoopProxyWrapper>>,
    pacing: Option<ResMut<WinitSettings>>,
    mut applied: Local<Option<u16>>,
) {
    let limit = settings.sanitized().fps_limit;
    if *applied == Some(limit) {
        return;
    }
    let (Some(proxy), Some(mut pacing)) = (proxy, pacing) else {
        return;
    };
    DISPLAY_PROXY.get_or_init(|| (**proxy).clone());
    #[cfg(target_os = "ios")]
    unsafe extern "C" {
        fn omoba_frame_pacing_set(fps: i32, callback: extern "C" fn());
    }
    #[cfg(all(test, not(target_os = "ios")))]
    unsafe fn omoba_frame_pacing_set(_: i32, _: extern "C" fn()) {}
    // This system and the UIKit bridge both enforce main-thread execution.
    unsafe { omoba_frame_pacing_set(i32::from(limit), display_tick) };
    pacing.focused_mode = focused_pacing(*settings, true);
    pacing.unfocused_mode = UpdateMode::reactive_low_power(Duration::from_secs(1));
    *applied = Some(limit);
}

/// Diagnostics separate the requested ceiling, native callback delivery and
/// measured app updates. They do not claim to measure GPU completion/scanout.
#[derive(Resource, Default, Debug, Clone, Serialize)]
pub(crate) struct FramePacingDiagnostics {
    pub requested_limit: u16,
    pub measured_update_hz: Option<f64>,
    pub display_max_hz: Option<u16>,
    pub display_link_hz: Option<f64>,
}

fn collect_pacing_diagnostics(
    _main_thread: bevy::ecs::system::NonSendMarker,
    settings: Res<RenderSettings>,
    measured: Res<MeasuredFps>,
    mut diagnostics: ResMut<FramePacingDiagnostics>,
) {
    diagnostics.requested_limit = settings.sanitized().fps_limit;
    diagnostics.measured_update_hz = measured.value;
    #[cfg(target_os = "ios")]
    {
        unsafe extern "C" {
            fn omoba_frame_pacing_max_fps() -> i32;
            fn omoba_frame_pacing_hz() -> f64;
        }
        let maximum = unsafe { omoba_frame_pacing_max_fps() };
        let callbacks = unsafe { omoba_frame_pacing_hz() };
        diagnostics.display_max_hz = u16::try_from(maximum).ok().filter(|value| *value > 0);
        diagnostics.display_link_hz =
            (callbacks.is_finite() && callbacks > 0.0).then_some(callbacks);
    }
}

/// Smoothed observed app frame cadence, never the requested FPS preference.
/// VSync stays enabled so a normally paced app frame maps to a rendered frame;
/// this is not a GPU timestamp or a claim about physical display scanout.
#[derive(Resource, Default)]
struct MeasuredFps {
    seconds: f64,
    frames: u32,
    value: Option<f64>,
}

impl MeasuredFps {
    fn sample(&mut self, delta: f64) {
        if !delta.is_finite() || delta <= 0.0 || delta > 1.0 {
            self.seconds = 0.0;
            self.frames = 0;
            self.value = None;
            return;
        }
        self.seconds += delta;
        self.frames += 1;
        if self.seconds >= 0.5 {
            let observed = f64::from(self.frames) / self.seconds;
            self.value = Some(
                self.value
                    .map_or(observed, |old| old * 0.4 + observed * 0.6),
            );
            self.seconds = 0.0;
            self.frames = 0;
        }
    }
}

#[derive(Component)]
struct FpsReadout;

fn spawn_fps_readout(mut commands: Commands) {
    commands.spawn((
        FpsReadout,
        Name::new("MeasuredFps"),
        crate::ui::TestId::from("MeasuredFps"),
        Text::new(crate::i18n::trf("hud.fps", &[("value", &"—")])),
        TextFont {
            font_size: 9.0,
            ..default()
        },
        TextColor(Color::srgba(0.78, 0.85, 0.84, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(8.0),
            top: Val::Px(4.0),
            ..default()
        },
        GlobalZIndex(60),
        bevy::ui::FocusPolicy::Pass,
        Pickable::IGNORE,
    ));
}

fn measure_fps(time: Res<Time<Real>>, mut measured: ResMut<MeasuredFps>) {
    measured.sample(time.delta_secs_f64());
}

fn update_fps_readout(
    measured: Res<MeasuredFps>,
    settings: Res<RenderSettings>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut readout: Query<(&mut Text, &mut Node), With<FpsReadout>>,
) {
    for (mut text, mut node) in &mut readout {
        node.display = if settings.show_fps {
            Display::Flex
        } else {
            Display::None
        };
        let value = measured
            .value
            .map_or_else(|| "—".to_owned(), |value| format!("{value:.0}"));
        let label = crate::i18n::trf("hud.fps", &[("value", &value)]);
        if text.0 != label {
            text.0 = label;
        }
        let safe = mobile
            .as_ref()
            .filter(|mobile| mobile.enabled)
            .map(|mobile| mobile.safe);
        let right = Val::Px(safe.map_or(8.0, |safe| safe.right + 8.0));
        let top = Val::Px(if safe.is_some() { 2.0 } else { 4.0 });
        if node.right != right {
            node.right = right;
        }
        if node.top != top {
            node.top = top;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_bridge_waits_for_a_window_event_loop_before_switching_pacing() {
        let mut app = App::new();
        app.init_resource::<RenderSettings>()
            .insert_resource(WinitSettings::mobile())
            .add_systems(Update, apply_ios_pacing);
        app.update();
        // No proxy in a headless world: retain the existing loop, do not wait
        // forever for native callbacks that were never installed.
        assert_eq!(
            app.world().resource::<WinitSettings>().focused_mode,
            WinitSettings::mobile().focused_mode
        );
    }

    #[test]
    fn legacy_preferences_default_to_sixty_and_unknown_caps_are_safe() {
        let legacy: RenderSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.fps_limit, 60);
        for value in [0, 1, 59, 61, 144, u16::MAX] {
            assert_eq!(
                RenderSettings {
                    fps_limit: value,
                    ..default()
                }
                .sanitized(),
                legacy
            );
        }
        let settings = RenderSettings {
            fps_limit: 120,
            show_fps: false,
        };
        assert_eq!(
            serde_json::from_str::<RenderSettings>(&serde_json::to_string(&settings).unwrap())
                .unwrap(),
            settings
        );
    }

    #[test]
    fn touch_events_do_not_remove_cap_and_ios_is_display_driven() {
        let UpdateMode::Reactive {
            wait,
            react_to_device_events,
            react_to_window_events,
            react_to_user_events,
        } = focused_pacing(RenderSettings::default(), false)
        else {
            panic!()
        };
        assert_eq!(wait, Duration::from_secs_f64(1.0 / 60.0));
        assert!(!react_to_device_events && !react_to_window_events && !react_to_user_events);
        let UpdateMode::Reactive {
            wait,
            react_to_user_events,
            ..
        } = focused_pacing(
            RenderSettings {
                fps_limit: 120,
                ..default()
            },
            true,
        )
        else {
            panic!()
        };
        assert_eq!(wait, Duration::from_secs(1));
        assert!(react_to_user_events);
    }

    #[test]
    fn hiding_readout_keeps_measuring_and_preserves_the_selected_ceiling() {
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .init_resource::<MeasuredFps>()
            .init_resource::<FramePacingDiagnostics>()
            .insert_resource(RenderSettings {
                fps_limit: 120,
                show_fps: false,
            })
            .add_systems(
                Update,
                (measure_fps, collect_pacing_diagnostics, update_fps_readout).chain(),
            );
        let readout = app
            .world_mut()
            .spawn((Text::new(""), Node::default(), FpsReadout))
            .id();
        for _ in 0..60 {
            app.world_mut()
                .resource_mut::<Time<Real>>()
                .advance_by(Duration::from_secs_f64(1.0 / 30.0));
            app.update();
        }
        assert_eq!(
            app.world().get::<Node>(readout).unwrap().display,
            Display::None
        );
        let diagnostics = app.world().resource::<FramePacingDiagnostics>();
        assert_eq!(diagnostics.requested_limit, 120);
        assert!((diagnostics.measured_update_hz.unwrap() - 30.0).abs() < 0.1);
        assert_eq!(
            diagnostics.display_max_hz, None,
            "desktop cannot certify an iOS display"
        );
        app.world_mut().resource_mut::<RenderSettings>().show_fps = true;
        app.update();
        assert_eq!(
            app.world().get::<Node>(readout).unwrap().display,
            Display::Flex
        );
        assert_eq!(app.world().get::<Text>(readout).unwrap().0, "30 FPS");
        assert_eq!(app.world().resource::<RenderSettings>().fps_limit, 120);
    }

    #[test]
    fn readout_measures_cadence_and_discards_suspend_gap() {
        let mut measured = MeasuredFps::default();
        for _ in 0..60 {
            measured.sample(1.0 / 60.0);
        }
        assert!((measured.value.unwrap() - 60.0).abs() < 0.01);
        measured.sample(10.0);
        assert_eq!(measured.value, None);
        for _ in 0..120 {
            measured.sample(1.0 / 120.0);
        }
        assert!((measured.value.unwrap() - 120.0).abs() < 0.01);
    }
    #[test]
    fn readout_counts_real_frames_not_fixed_simulation_ticks_or_requested_cap() {
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .init_resource::<Time<Virtual>>()
            .init_resource::<MeasuredFps>()
            .insert_resource(RenderSettings {
                fps_limit: 120,
                ..default()
            })
            .add_systems(Update, measure_fps);
        for _ in 0..120 {
            app.world_mut()
                .resource_mut::<Time<Real>>()
                .advance_by(Duration::from_secs_f64(1.0 / 120.0));
            app.world_mut()
                .resource_mut::<Time<Virtual>>()
                .advance_by(Duration::from_secs_f64(1.0 / 30.0));
            app.update();
        }
        assert!((app.world().resource::<MeasuredFps>().value.unwrap() - 120.0).abs() < 0.1);
        // A genuine slow renderer must still report its measured rate, not 120.
        for _ in 0..150 {
            app.world_mut()
                .resource_mut::<Time<Real>>()
                .advance_by(Duration::from_secs_f64(1.0 / 30.0));
            app.update();
        }
        assert!((app.world().resource::<MeasuredFps>().value.unwrap() - 30.0).abs() < 0.1);
    }
}
