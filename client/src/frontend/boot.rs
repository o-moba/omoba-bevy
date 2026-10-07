//! Boot splash: what the player sees from the first UI frame until the
//! startup assets have settled, instead of an empty Home.
//!
//! The splash is a modal (`ModalId::Boot`) drawn above every screen, so the
//! menu underneath is built and loaded but does not react. It reports real
//! progress of the tracked startup assets (interface fonts, then the Verdant
//! world), stays for at least [`MIN_VISIBLE`] seconds and never longer than
//! [`TIMEOUT`], then dips through the base colour into Home. The iOS launch
//! screen uses the same base colour (`mobile/ios/Assets.xcassets`).
//!
//! Automation that bypasses or drives the shell gets no splash: see
//! [`enabled_for`].
// i18n-strict

use bevy::asset::{LoadState, RecursiveDependencyLoadState, UntypedAssetId, UntypedHandle};
use bevy::prelude::*;

use crate::i18n::{Localized, tr, trf};
use crate::ui::living_background::{self, LivingBands, LivingScene};
use crate::ui::theme::{self, TextStyle};
use crate::ui::tokens::{Metric, TextRole, color};
use crate::ui::{ModalAppExt, ModalId, ModalRoot};

/// Shortest time the splash is on screen, so it never flashes by.
const MIN_VISIBLE: f32 = 1.4;
/// A stuck or missing asset must not keep the player out of the menu.
const TIMEOUT: f32 = 15.0;
/// The dip into the base colour and out of it.
const CLOSE: f32 = 0.25;
const REVEAL: f32 = 0.4;
/// Fastest the bar fills, in bar lengths per second: progress that arrives in
/// one frame still reads as loading.
const FILL_RATE: f32 = 1.2;
/// A long startup frame counts as this much splash time at most.
const MAX_STEP: f32 = 0.1;

pub(crate) struct BootSplashPlugin;

impl Plugin for BootSplashPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BootSplash>()
            .register_modal::<BootSplash>(ModalId::Boot, BootSplash::blocks_input)
            .add_systems(Startup, spawn_splash)
            .add_systems(PostStartup, track_startup_assets)
            .add_systems(Update, drive_splash);
    }
}

/// What a tracked asset belongs to; the status line names the first group
/// that is still loading.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BootGroup {
    Interface,
    World,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Phase {
    /// No splash: automation, or the splash has finished.
    #[default]
    Off,
    Loading,
    /// The curtain is covering the splash.
    Closing,
    /// The splash is gone; the curtain is uncovering the menu.
    Revealing,
}

#[derive(Resource, Default)]
pub(crate) struct BootSplash {
    phase: Phase,
    /// Splash time in the current phase and in total, in clamped real seconds.
    phase_age: f32,
    age: f32,
    /// Unclamped real seconds: the timeout must not stretch on a slow device.
    wall: f32,
    /// The bar's displayed fill, behind the real progress by [`FILL_RATE`].
    shown: f32,
    tracked: Vec<(BootGroup, UntypedHandle)>,
}

impl BootSplash {
    /// The menu below stays inert until the splash has fully left.
    pub(crate) fn blocks_input(&self) -> bool {
        self.phase != Phase::Off
    }

    /// Adds a startup asset the splash waits for (while it is loading).
    pub(crate) fn track(&mut self, group: BootGroup, handle: UntypedHandle) {
        if self.phase == Phase::Loading && !self.tracked.iter().any(|(_, known)| *known == handle) {
            self.tracked.push((group, handle));
        }
    }

    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.phase_age = 0.0;
    }

    /// A splash that is up, for tests of what it blocks.
    #[cfg(test)]
    pub(crate) fn loading_for_test() -> Self {
        Self {
            phase: Phase::Loading,
            ..default()
        }
    }
}

/// Player-facing switches. Any other `OMOBA_*` variable is a harness, a
/// capture or a debug launch that expects the shell without a splash.
const PLAYER_SWITCHES: [&str; 7] = [
    "OMOBA_LANGUAGE",
    "OMOBA_PLAYER_VISUAL_MODE",
    "OMOBA_CLIENT_CONFIG_DIR",
    "OMOBA_TOUCH_CONTROLS",
    "OMOBA_UI_LOW_END",
    "OMOBA_PASSPORT_CONNECT",
    BOOT_QA_DIR,
];

/// Capture directory of the splash's own harness (`qa::boot_qa`).
// The name avoids the `_QA_DIR` suffix, which sends the shell into a match.
pub(crate) const BOOT_QA_DIR: &str = "OMOBA_BOOT_SPLASH_SHOTS";

/// Whether a launch with these (non-empty) environment keys shows the splash.
fn enabled_for(keys: impl Iterator<Item = String>) -> bool {
    keys.into_iter()
        .all(|key| !key.starts_with("OMOBA_") || PLAYER_SWITCHES.contains(&key.as_str()))
}

fn enabled() -> bool {
    !crate::sandbox::requested()
        && enabled_for(
            std::env::vars_os()
                .filter(|(_, value)| !value.is_empty())
                .map(|(key, _)| key.to_string_lossy().into_owned()),
        )
}

/// Settled assets out of the tracked ones; nothing tracked is complete.
fn progress(settled: usize, total: usize) -> f32 {
    if total == 0 {
        1.0
    } else {
        settled as f32 / total as f32
    }
}

/// The splash may leave once everything settled, the bar caught up and the
/// minimum (rendered) time passed, or unconditionally at the wall-clock
/// timeout.
fn may_close(age: f32, wall: f32, shown: f32, all_settled: bool) -> bool {
    wall >= TIMEOUT || (all_settled && shown >= 0.995 && age >= MIN_VISIBLE)
}

/// Loaded and failed assets are both settled: a missing file must not hold
/// the splash until the timeout.
fn settled(server: &AssetServer, id: UntypedAssetId) -> bool {
    matches!(server.load_state(id), LoadState::Failed(_))
        || matches!(
            server.recursive_dependency_load_state(id),
            RecursiveDependencyLoadState::Loaded | RecursiveDependencyLoadState::Failed(_)
        )
}

#[derive(Component)]
struct BootRoot;

/// Everything of the splash except the curtain; despawned under the curtain.
#[derive(Component)]
struct BootContent;

#[derive(Component)]
struct BootBarFill;

#[derive(Component)]
struct BootStatus;

#[derive(Component)]
struct BootCurtain;

fn full_layer() -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(0.0),
        right: Val::Px(0.0),
        top: Val::Px(0.0),
        bottom: Val::Px(0.0),
        ..default()
    }
}

fn spawn_splash(
    mut commands: Commands,
    mut splash: ResMut<BootSplash>,
    platform: Res<crate::ui::UiPlatform>,
) {
    if !enabled() {
        return;
    }
    splash.enter(Phase::Loading);
    let form = theme::Form::of(platform.is_mobile());
    let shadow = TextShadow {
        offset: Vec2::splat(2.0),
        color: Color::srgba(0.0, 0.0, 0.0, 0.85),
    };
    let mut content = Vec::new();
    commands
        .spawn((
            full_layer(),
            BackgroundColor(color::BG_BASE),
            GlobalZIndex(ModalId::Boot.layer()),
            ModalRoot(ModalId::Boot),
            bevy::ui::FocusPolicy::Block,
            BootRoot,
            Name::new("BootSplash"),
        ))
        .with_children(|root| {
            content.push(living_background::spawn(
                root,
                LivingScene::Arena,
                LivingBands::default(),
                form,
            ));
            content.push(
                root.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        right: Val::Px(0.0),
                        bottom: Val::Px(0.0),
                        height: Val::Percent(62.0),
                        ..default()
                    },
                    BackgroundGradient::from(LinearGradient::to_top(vec![
                        ColorStop::percent(
                            theme::perceptual(color::SCRIM_LIVING).with_alpha(0.92),
                            0.0,
                        ),
                        ColorStop::percent(Color::NONE, 100.0),
                    ])),
                    Pickable::IGNORE,
                    Name::new("BootScrim"),
                ))
                .id(),
            );
            content.push(
                root.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::FlexEnd,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(8.0),
                        padding: UiRect::bottom(Val::Percent(5.0)),
                        ..full_layer()
                    },
                    Pickable::IGNORE,
                    Name::new("BootBody"),
                ))
                .with_children(|body| {
                    body.spawn((
                        Text::new("OMOBA"), // i18n-allow
                        theme::styled_text(
                            TextStyle::keep_case(TextRole::TitleXl).sized(Metric::new(88.0, 52.0)),
                        ),
                        TextColor(theme::IVORY),
                        shadow,
                        Name::new("BootTitle"),
                    ));
                    body.spawn((
                        Localized::new("home.tagline").into_text(),
                        theme::role_text(TextRole::Eyebrow),
                        TextColor(theme::GOLD),
                        shadow,
                        Name::new("BootTagline"),
                    ));
                    body.spawn((
                        Node {
                            width: Val::Px(360.0),
                            max_width: Val::Percent(70.0),
                            height: Val::Px(4.0),
                            margin: UiRect::top(Val::Px(14.0)),
                            border_radius: BorderRadius::all(Val::Px(2.0)),
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(color::SURFACE_3),
                        Name::new("BootBar"),
                    ))
                    .with_children(|bar| {
                        bar.spawn((
                            Node {
                                width: Val::Percent(0.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(color::GOLD_400),
                            BootBarFill,
                        ));
                    });
                    body.spawn((
                        Node {
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            ..default()
                        },
                        Name::new("BootStatusRow"),
                    ))
                    .with_children(|row| {
                        crate::ui::widgets::status::spinner(row, 18.0);
                        row.spawn((
                            Text::new(status_line(Some(BootGroup::Interface), 0.0)),
                            theme::styled_text(TextStyle::keep_case(TextRole::Caption)),
                            TextColor(theme::IVORY),
                            shadow,
                            BootStatus,
                            Name::new("BootStatus"),
                        ));
                    });
                    body.spawn((
                        Text::new(crate::build_info::label()),
                        theme::styled_text(TextStyle::keep_case(TextRole::Caption)),
                        TextColor(theme::MUTED),
                        shadow,
                        Name::new("BootBuildInfo"),
                    ));
                })
                .id(),
            );
            root.spawn((
                full_layer(),
                BackgroundColor(color::BG_BASE.with_alpha(0.0)),
                Pickable::IGNORE,
                BootCurtain,
                Name::new("BootCurtain"),
            ));
        });
    for entity in content {
        commands.entity(entity).insert(BootContent);
    }
}

/// The handles other startup systems created; `Startup` commands are applied
/// before `PostStartup`.
fn track_startup_assets(
    mut splash: ResMut<BootSplash>,
    fonts: Option<Res<theme::UiTheme>>,
    world: Option<Res<crate::verdant3d::VerdantAssets>>,
) {
    if splash.phase != Phase::Loading {
        return;
    }
    if let Some(fonts) = fonts {
        // The CJK body face is megabytes larger and only shapes CJK text:
        // wait for it only when the splash's own status line is written in it.
        let cjk = theme::needs_cjk_font(tr("boot.status.world"));
        for handle in std::iter::once(&fonts.font)
            .chain(fonts.families.iter().map(|(_, handle)| handle))
            .filter(|handle| cjk || **handle != fonts.cjk_font)
        {
            splash.track(BootGroup::Interface, handle.clone().untyped());
        }
    }
    if let Some(world) = world {
        for handle in world.handles() {
            splash.track(BootGroup::World, handle);
        }
    }
}

/// "Loading the world… · 62%", or the ready line once nothing is pending.
fn status_line(pending: Option<BootGroup>, shown: f32) -> String {
    let stage = match pending {
        Some(BootGroup::Interface) => tr("boot.status.interface"),
        Some(BootGroup::World) => tr("boot.status.world"),
        None => tr("boot.status.ready"),
    };
    let percent = (shown.clamp(0.0, 1.0) * 100.0).round() as u32;
    trf(
        "boot.status.line",
        &[("stage", &stage), ("percent", &percent)],
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive_splash(
    mut commands: Commands,
    time: Res<Time<Real>>,
    server: Option<Res<AssetServer>>,
    mut splash: ResMut<BootSplash>,
    mut root: Query<(Entity, &mut BackgroundColor), (With<BootRoot>, Without<BootCurtain>)>,
    content: Query<Entity, With<BootContent>>,
    mut fills: Query<&mut Node, With<BootBarFill>>,
    mut status: Query<&mut Text, With<BootStatus>>,
    mut curtain: Query<&mut BackgroundColor, (With<BootCurtain>, Without<BootRoot>)>,
) {
    if splash.phase == Phase::Off {
        return;
    }
    let step = time.delta_secs().min(MAX_STEP);
    splash.age += step;
    splash.phase_age += step;
    splash.wall += time.delta_secs();
    let mut curtain_alpha = None;
    match splash.phase {
        Phase::Off => {}
        Phase::Loading => {
            let total = splash.tracked.len();
            let mut done = 0;
            let mut pending = None;
            for (group, handle) in &splash.tracked {
                if server
                    .as_ref()
                    .is_none_or(|server| settled(server, handle.id()))
                {
                    done += 1;
                } else if pending.is_none() {
                    pending = Some(*group);
                }
            }
            let target = progress(done, total);
            splash.shown = (splash.shown + FILL_RATE * step).min(target);
            let width = Val::Percent(splash.shown * 100.0);
            for mut fill in &mut fills {
                if fill.width != width {
                    fill.width = width;
                }
            }
            // While the bar is still catching up, keep naming what it loaded.
            let stage = pending.or_else(|| {
                (splash.shown < 0.995)
                    .then(|| splash.tracked.last().map(|(group, _)| *group))
                    .flatten()
            });
            let line = status_line(stage, splash.shown);
            for mut text in &mut status {
                if text.0 != line {
                    text.0.clone_from(&line);
                }
            }
            if may_close(splash.age, splash.wall, splash.shown, done == total) {
                info!(
                    "Boot splash: {done} of {total} startup assets settled after {:.2} s",
                    splash.age
                );
                splash.enter(Phase::Closing);
            }
        }
        Phase::Closing => {
            let covered = (splash.phase_age / CLOSE).min(1.0);
            curtain_alpha = Some(covered);
            if covered >= 1.0 {
                for entity in &content {
                    commands.entity(entity).despawn();
                }
                for (_, mut fill) in &mut root {
                    fill.0 = Color::NONE;
                }
                splash.tracked.clear();
                splash.enter(Phase::Revealing);
            }
        }
        Phase::Revealing => {
            let left = 1.0 - (splash.phase_age / REVEAL).min(1.0);
            curtain_alpha = Some(left);
            if left <= 0.0 {
                for (entity, _) in &root {
                    commands.entity(entity).despawn();
                }
                splash.enter(Phase::Off);
            }
        }
    }
    if let Some(alpha) = curtain_alpha {
        for mut fill in &mut curtain {
            fill.0 = color::BG_BASE.with_alpha(alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(keys: &[&str]) -> impl Iterator<Item = String> {
        keys.iter()
            .map(|key| (*key).to_owned())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn players_get_the_splash_and_automation_does_not() {
        assert!(enabled_for(keys(&[])));
        assert!(enabled_for(keys(&[
            "HOME",
            "GAME_SERVER_ADDR",
            "OMOBA_LANGUAGE"
        ])));
        assert!(enabled_for(keys(&[BOOT_QA_DIR])));
        for harness in [
            "OMOBA_AUTOJOIN",
            "OMOBA_FRONTEND_QA_OUTPUT",
            "OMOBA_OFFLINE_SMOKE_DIR",
            "OMOBA_VISUAL_QA_DIR",
            "OMOBA_RECORD_DIR",
            "OMOBA_UI_GALLERY",
            "OMOBA_SOME_FUTURE_HARNESS",
        ] {
            assert!(
                !enabled_for(keys(&["OMOBA_LANGUAGE", harness])),
                "{harness}"
            );
        }
    }

    #[test]
    fn progress_counts_settled_assets_and_an_empty_barrier_is_complete() {
        assert_eq!(progress(0, 0), 1.0);
        assert_eq!(progress(0, 4), 0.0);
        assert_eq!(progress(3, 4), 0.75);
        assert_eq!(progress(4, 4), 1.0);
    }

    #[test]
    fn splash_holds_for_the_minimum_time_and_never_past_the_timeout() {
        // Everything loaded on the first frame: still shown, never a flash.
        assert!(!may_close(0.1, 0.1, 1.0, true));
        assert!(
            !may_close(MIN_VISIBLE, MIN_VISIBLE, 0.6, true),
            "the bar catches up first"
        );
        assert!(may_close(MIN_VISIBLE, MIN_VISIBLE, 1.0, true));
        // A pending asset holds it, but only until the timeout.
        assert!(!may_close(TIMEOUT - 0.1, TIMEOUT - 0.1, 0.5, false));
        assert!(may_close(TIMEOUT, TIMEOUT, 0.5, false));
        // Long frames stretch splash time, never the wall-clock timeout.
        assert!(may_close(2.0, TIMEOUT, 0.5, false));
    }

    #[test]
    fn only_a_loading_splash_tracks_and_input_stays_blocked_until_it_left() {
        let mut splash = BootSplash::default();
        assert!(!splash.blocks_input());
        splash.track(BootGroup::World, Handle::<Image>::default().untyped());
        assert!(
            splash.tracked.is_empty(),
            "automation launches track nothing"
        );
        splash.enter(Phase::Loading);
        splash.track(BootGroup::World, Handle::<Image>::default().untyped());
        splash.track(BootGroup::World, Handle::<Image>::default().untyped());
        assert_eq!(splash.tracked.len(), 1, "duplicates are ignored");
        for phase in [Phase::Loading, Phase::Closing, Phase::Revealing] {
            splash.enter(phase);
            assert!(splash.blocks_input(), "{phase:?}");
        }
        splash.enter(Phase::Off);
        assert!(!splash.blocks_input());
    }

    #[test]
    fn splash_runs_through_its_phases_and_removes_itself() {
        let mut app = App::new();
        app.insert_resource(BootSplash {
            phase: Phase::Loading,
            ..default()
        })
        .init_resource::<Time<Real>>()
        .add_systems(Update, drive_splash);
        let root = app
            .world_mut()
            .spawn((BootRoot, BackgroundColor(color::BG_BASE)))
            .id();
        let body = app.world_mut().spawn(BootContent).id();
        let curtain = app
            .world_mut()
            .spawn((BootCurtain, BackgroundColor(Color::NONE)))
            .id();
        let fill = app.world_mut().spawn((BootBarFill, Node::default())).id();
        let tick = |app: &mut App, seconds: f32| {
            let mut left = seconds;
            while left > 0.0 {
                let step = left.min(MAX_STEP);
                app.world_mut()
                    .resource_mut::<Time<Real>>()
                    .advance_by(std::time::Duration::from_secs_f32(step));
                app.update();
                left -= step;
            }
        };
        tick(&mut app, 0.5);
        assert_eq!(app.world().resource::<BootSplash>().phase, Phase::Loading);
        assert!(app.world().get::<Node>(fill).unwrap().width != Val::Percent(0.0));
        // Just past the minimum time: the curtain has started, not finished.
        tick(&mut app, MIN_VISIBLE - 0.5 + MAX_STEP);
        assert_eq!(app.world().resource::<BootSplash>().phase, Phase::Closing);
        tick(&mut app, CLOSE + MAX_STEP);
        assert_eq!(app.world().resource::<BootSplash>().phase, Phase::Revealing);
        assert!(app.world().get_entity(body).is_err(), "content is gone");
        assert_eq!(
            app.world().get::<BackgroundColor>(root).unwrap().0,
            Color::NONE,
            "the menu shows through the root"
        );
        assert!(
            app.world()
                .get::<BackgroundColor>(curtain)
                .unwrap()
                .0
                .alpha()
                > 0.0
        );
        tick(&mut app, REVEAL + MAX_STEP);
        assert_eq!(app.world().resource::<BootSplash>().phase, Phase::Off);
        assert!(app.world().get_entity(root).is_err());
        assert!(!app.world().resource::<BootSplash>().blocks_input());
    }
}
