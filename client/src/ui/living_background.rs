//! Verdant Crown painted menu backgrounds (handoff F11).
//!
//! The component is deliberately ordinary Bevy UI: cover-cropped plate and
//! foreground images, tintable sprite effects and post-layout `UiTransform`
//! motion. No shader, render target or per-frame layout mutation is involved.

use std::collections::VecDeque;
use std::f32::consts::TAU;

use bevy::{
    prelude::*,
    ui::{BackgroundGradient, ColorStop, LinearGradient},
    window::PrimaryWindow,
};

use super::kit_assets::{Background, CoverImage, HighDensity, KitImage, LowDensity, Sprite};
use super::theme::{self, Form};
use super::tokens::{color, motion};

/// Persisted accessibility preference. Fades remain enabled; parallax and
/// decorative animation stop completely when this is true.
#[derive(Resource, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct MotionSettings {
    pub reduce: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LivingScene {
    Arena,
    Stage,
    Victory,
    Defeat,
}

impl LivingScene {
    fn plate(self) -> Background {
        match self {
            Self::Arena => Background::LivingArenaPlate,
            Self::Stage => Background::LivingStagePlate,
            Self::Victory => Background::LivingVictoryPlate,
            Self::Defeat => Background::LivingDefeatPlate,
        }
    }

    fn foreground(self) -> Background {
        match self {
            Self::Arena => Background::LivingArenaForeground,
            Self::Stage => Background::LivingStageForeground,
            Self::Victory => Background::LivingVictoryForeground,
            Self::Defeat => Background::LivingDefeatForeground,
        }
    }

    fn config(self) -> SceneConfig {
        match self {
            Self::Arena => SceneConfig {
                ray_origin: Vec2::new(0.18, -0.10),
                ray_count: 6,
                ray_offset: 0.55,
                ray_color: color::LIVING_ARENA_RAY,
                mote_count: 70,
                mote_speed: (0.03, 0.08),
                mote_color: color::LIVING_ARENA_MOTE,
                flake_count: 7,
                flake_sprite: Sprite::Leaf,
                flake_colors: &[
                    color::LIVING_LEAF,
                    color::LIVING_LEAF_LIGHT,
                    color::GOLD_500,
                ],
                fog: None,
                rune: None,
            },
            Self::Stage => SceneConfig {
                ray_origin: Vec2::new(0.50, -0.12),
                ray_count: 5,
                ray_offset: 0.0,
                ray_color: color::LIVING_STAGE_RAY,
                mote_count: 60,
                mote_speed: (0.03, 0.08),
                mote_color: color::LIVING_STAGE_MOTE,
                flake_count: 5,
                flake_sprite: Sprite::Leaf,
                flake_colors: &[color::LIVING_LEAF, color::LIVING_LEAF_LIGHT],
                fog: Some((0.82, color::LIVING_STAGE_FOG)),
                rune: Some(color::LIVING_STAGE_RUNE),
            },
            Self::Victory => SceneConfig {
                ray_origin: Vec2::new(0.47, 0.10),
                ray_count: 10,
                ray_offset: 0.0,
                ray_color: color::LIVING_VICTORY_RAY,
                mote_count: 50,
                mote_speed: (0.03, 0.08),
                mote_color: color::LIVING_VICTORY_MOTE,
                flake_count: 26,
                flake_sprite: Sprite::Petal,
                flake_colors: &[
                    color::LIVING_VICTORY_PETAL,
                    color::GOLD_400,
                    color::LIVING_VICTORY_PETAL_LIGHT,
                ],
                fog: None,
                rune: None,
            },
            Self::Defeat => SceneConfig {
                ray_origin: Vec2::new(0.60, 0.55),
                ray_count: 4,
                ray_offset: -1.60,
                ray_color: color::LIVING_DEFEAT_RAY,
                mote_count: 80,
                mote_speed: (0.06, 0.16),
                mote_color: color::LIVING_DEFEAT_EMBER,
                flake_count: 10,
                flake_sprite: Sprite::Petal,
                flake_colors: &[color::LIVING_DEFEAT_ASH, color::LIVING_DEFEAT_ASH_DARK],
                fog: Some((0.70, color::LIVING_DEFEAT_FOG)),
                rune: None,
            },
        }
    }
}

struct SceneConfig {
    ray_origin: Vec2,
    ray_count: usize,
    ray_offset: f32,
    ray_color: Color,
    mote_count: usize,
    mote_speed: (f32, f32),
    mote_color: Color,
    flake_count: usize,
    flake_sprite: Sprite,
    flake_colors: &'static [Color],
    fog: Option<(f32, Color)>,
    rune: Option<Color>,
}

/// Solid portions of the full-width legibility bands. The component adds the
/// handoff's 32 px desktop / 16 px phone fade beyond each value.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(crate) struct LivingBands {
    pub header: Option<f32>,
    pub footer: Option<f32>,
}

#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct LivingBackground {
    pub scene: LivingScene,
}

#[derive(Component, Clone, Copy)]
struct LivingOwner(Entity);

#[derive(Component, Clone, Copy)]
enum LivingVisual {
    Plate,
    Foreground,
    Vignette,
    Ray { index: usize },
    Fog { index: usize },
    Rune,
    Mote(Particle),
    Flake(Particle),
}

#[derive(Clone, Copy)]
struct Particle {
    base: Vec2,
    phase: f32,
    speed: f32,
    spin: f32,
    front: bool,
}

#[derive(Component, Clone, Copy)]
struct LivingTint(Color);

/// Only the two full-screen photographic layers use the handoff's 720/1080
/// breakpoint. Decorative sprites continue to follow the normal UI density.
#[derive(Component)]
struct LivingLayerImage;

#[derive(Component, Default)]
struct LivingFade(f32);

/// Session fallback. Once tripped it stays low-end until the client exits.
#[derive(Resource, Debug)]
pub(crate) struct LivingQuality {
    pub low_end: bool,
    elapsed: f32,
    samples: VecDeque<f32>,
}

impl Default for LivingQuality {
    fn default() -> Self {
        Self {
            low_end: std::env::var("OMOBA_UI_LOW_END").is_ok_and(|v| v == "1" || v == "true"),
            elapsed: 0.0,
            samples: VecDeque::with_capacity(240),
        }
    }
}

#[derive(Resource, Default)]
struct LivingClock {
    elapsed: f32,
    drift: Vec2,
    detail: f32,
}

pub(crate) struct LivingBackgroundPlugin;

impl Plugin for LivingBackgroundPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MotionSettings>()
            .init_resource::<LivingQuality>()
            .init_resource::<LivingClock>()
            .add_systems(
                Update,
                (
                    select_living_density,
                    sample_phone_performance,
                    animate_living_backgrounds,
                )
                    .chain()
                    .before(super::UiSet::Paint),
            );
    }
}

/// Adds one background as the first child of a screen root.
pub(crate) fn spawn(
    parent: &mut ChildSpawnerCommands,
    scene: LivingScene,
    bands: LivingBands,
    form: Form,
) -> Entity {
    let config = scene.config();
    let fade_px = if form == Form::Phone { 16.0 } else { 32.0 };
    let mut root = parent.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(color::BG_BASE),
        // No explicit ZIndex: the screen inserts this as its first child, so
        // normal tree order keeps it above the parent's fill and below UI
        // siblings. A negative value hides it behind the parent; an explicit
        // zero creates a stacking layer that can cover nested gallery UI.
        Pickable::IGNORE,
        LivingBackground { scene },
        LivingFade::default(),
        Name::new(format!("LivingBackground::{scene:?}")),
    ));
    let owner = root.id();
    root.with_children(|canvas| {
        spawn_visual(
            canvas,
            owner,
            full_node(),
            KitImage::background(scene.plate()),
            LivingVisual::Plate,
            Color::WHITE,
            "LivingPlate",
        )
        .insert((CoverImage { anchor_y: 0.5 }, LivingLayerImage));

        for index in 0..config.ray_count {
            let spread = index as f32 / config.ray_count as f32 - 0.5;
            let angle = spread * 1.1 + config.ray_offset;
            let half = [0.05_f32, 0.075, 0.10][index % 3];
            let mut ray = spawn_visual(
                canvas,
                owner,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(config.ray_origin.x * 100.0 - half * 100.0),
                    top: Val::Percent(config.ray_origin.y * 100.0),
                    width: Val::Percent(half * 200.0),
                    height: Val::Percent(165.0),
                    ..default()
                },
                KitImage::sprite(Sprite::RayWedge, config.ray_color),
                LivingVisual::Ray { index },
                config.ray_color,
                "LivingRay",
            );
            ray.insert(UiTransform::from_rotation(Rot2::radians(angle)));
        }

        if let Some((line, tint)) = config.fog {
            for index in 0..3 {
                spawn_visual(
                    canvas,
                    owner,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Percent(-50.0 + index as f32 * 66.0),
                        top: Val::Percent(line * 100.0 - 30.0),
                        width: Val::Percent(120.0),
                        height: Val::Percent(60.0),
                        ..default()
                    },
                    KitImage::sprite(Sprite::DiscSoft, tint),
                    LivingVisual::Fog { index },
                    tint,
                    "LivingFog",
                );
            }
        }

        if let Some(tint) = config.rune {
            spawn_visual(
                canvas,
                owner,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(34.0),
                    top: Val::Percent(61.0),
                    width: Val::Percent(32.0),
                    height: Val::Percent(32.0),
                    ..default()
                },
                KitImage::sprite(Sprite::DiscSoft, tint),
                LivingVisual::Rune,
                tint,
                "LivingRune",
            );
        }

        spawn_particles(canvas, owner, &config, false);

        spawn_visual(
            canvas,
            owner,
            full_node(),
            KitImage::background(scene.foreground()),
            LivingVisual::Foreground,
            Color::WHITE,
            "LivingForeground",
        )
        .insert((CoverImage { anchor_y: 0.5 }, LivingLayerImage));

        spawn_particles(canvas, owner, &config, true);

        spawn_visual(
            canvas,
            owner,
            full_node(),
            KitImage::sprite(Sprite::Vignette, color::LIVING_VIGNETTE),
            LivingVisual::Vignette,
            color::LIVING_VIGNETTE,
            "LivingVignette",
        );

        if let Some(height) = bands.header {
            spawn_band(canvas, true, height, fade_px);
        }
        if let Some(height) = bands.footer {
            spawn_band(canvas, false, height, fade_px);
        }
    });
    owner
}

fn full_node() -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(0.0),
        right: Val::Px(0.0),
        top: Val::Px(0.0),
        bottom: Val::Px(0.0),
        ..default()
    }
}

fn spawn_visual<'a>(
    parent: &'a mut ChildSpawnerCommands,
    owner: Entity,
    node: Node,
    image: KitImage,
    visual: LivingVisual,
    tint: Color,
    name: &'static str,
) -> EntityCommands<'a> {
    parent.spawn((
        node,
        image,
        UiTransform::IDENTITY,
        Visibility::Inherited,
        LivingOwner(owner),
        visual,
        LivingTint(tint),
        Pickable::IGNORE,
        Name::new(name),
    ))
}

fn spawn_particles(
    canvas: &mut ChildSpawnerCommands,
    owner: Entity,
    config: &SceneConfig,
    front: bool,
) {
    for index in 0..config.mote_count {
        let particle = mote(index, config.mote_speed);
        if particle.front != front {
            continue;
        }
        let diameter = 4.8 + noise(index as u32 * 11 + 5) * 14.4;
        spawn_visual(
            canvas,
            owner,
            particle_node(particle.base, diameter),
            KitImage::sprite(Sprite::DiscSoft, config.mote_color),
            LivingVisual::Mote(particle),
            config.mote_color,
            "LivingMote",
        );
    }

    for index in 0..config.flake_count {
        let particle = flake(index);
        if particle.front != front {
            continue;
        }
        let half = 5.0 + noise(index as u32 * 19 + 7) * 7.0;
        let factor = if particle.front { 1.7 } else { 1.0 };
        let tint = config.flake_colors[index % config.flake_colors.len()];
        spawn_visual(
            canvas,
            owner,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(particle.base.x * 100.0),
                top: Val::Percent(particle.base.y * 100.0),
                width: Val::Px(half * 2.0 * factor),
                height: Val::Px(half * 0.9 * factor),
                ..default()
            },
            KitImage::sprite(config.flake_sprite, tint),
            LivingVisual::Flake(particle),
            tint,
            "LivingFlake",
        );
    }
}

fn particle_node(base: Vec2, diameter: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Percent(base.x * 100.0),
        top: Val::Percent(base.y * 100.0),
        width: Val::Px(diameter),
        height: Val::Px(diameter),
        ..default()
    }
}

fn spawn_band(parent: &mut ChildSpawnerCommands, header: bool, solid: f32, fade: f32) {
    let scrim = theme::perceptual(color::SCRIM_LIVING);
    let clear = scrim.with_alpha(0.0);
    let stops = if header {
        vec![
            ColorStop::px(scrim, 0.0),
            ColorStop::px(scrim, solid),
            ColorStop::px(clear, solid + fade),
        ]
    } else {
        vec![
            ColorStop::px(clear, 0.0),
            ColorStop::px(scrim, fade),
            ColorStop::px(scrim, solid + fade),
        ]
    };
    parent.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: header.then_some(Val::Px(0.0)).unwrap_or(Val::Auto),
            bottom: (!header).then_some(Val::Px(0.0)).unwrap_or(Val::Auto),
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            height: Val::Px(solid + fade),
            ..default()
        },
        BackgroundGradient::from(if header {
            LinearGradient::to_bottom(stops)
        } else {
            LinearGradient::to_top(stops)
        }),
        Pickable::IGNORE,
        Name::new(if header {
            "LivingHeaderBand"
        } else {
            "LivingFooterBand"
        }),
    ));
}

fn noise(seed: u32) -> f32 {
    let mut value = seed.wrapping_add(0x9E37_79B9);
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846C_A68B);
    value ^= value >> 16;
    value as f32 / u32::MAX as f32
}

fn mote(index: usize, speed: (f32, f32)) -> Particle {
    let i = index as u32;
    Particle {
        base: Vec2::new(noise(i * 7 + 1), noise(i * 7 + 2)),
        phase: noise(i * 7 + 3) * TAU,
        speed: speed.0 + (speed.1 - speed.0) * noise(i * 7 + 4),
        spin: 0.0,
        front: noise(i * 7 + 5) < 0.30,
    }
}

fn flake(index: usize) -> Particle {
    let i = index as u32;
    Particle {
        base: Vec2::new(noise(i * 13 + 1), -noise(i * 13 + 2)),
        phase: noise(i * 13 + 3) * TAU,
        speed: 0.08 + 0.08 * noise(i * 13 + 4),
        spin: -1.0 + 2.0 * noise(i * 13 + 5),
        front: noise(i * 13 + 6) < 0.50,
    }
}

fn select_living_density(
    mut commands: Commands,
    windows: Query<&Window, With<PrimaryWindow>>,
    platform: Res<super::UiPlatform>,
    mut images: Query<
        (Entity, &mut KitImage, Has<LowDensity>, Has<HighDensity>),
        With<LivingLayerImage>,
    >,
) {
    let physical_height = windows.single().map_or(720, Window::physical_height);
    let high = use_high_density(platform.is_mobile(), physical_height);
    for (entity, mut image, low, high_marker) in &mut images {
        if high {
            if !high_marker || low {
                commands
                    .entity(entity)
                    .try_remove::<LowDensity>()
                    .try_insert(HighDensity);
                image.set_changed();
            }
        } else {
            if !low || high_marker {
                commands
                    .entity(entity)
                    .try_remove::<HighDensity>()
                    .try_insert(LowDensity);
                image.set_changed();
            }
        }
    }
}

fn use_high_density(phone: bool, physical_height: u32) -> bool {
    !phone && physical_height > 800
}

fn sample_phone_performance(
    time: Res<Time>,
    platform: Res<super::UiPlatform>,
    roots: Query<(), With<LivingBackground>>,
    mut quality: ResMut<LivingQuality>,
) {
    if !platform.is_mobile() || roots.is_empty() || quality.low_end {
        return;
    }
    let delta = time.delta_secs();
    if !(0.0..=0.25).contains(&delta) || delta == 0.0 {
        return;
    }
    quality.elapsed += delta;
    quality.samples.push_back(delta);
    while quality.samples.len() > 240 {
        quality.samples.pop_front();
    }
    if quality.elapsed < 3.0 || quality.samples.len() < 30 {
        return;
    }
    let mut samples: Vec<_> = quality.samples.iter().copied().collect();
    samples.sort_by(f32::total_cmp);
    let p90 = samples[((samples.len() - 1) as f32 * 0.9).round() as usize];
    if p90 > 0.025 {
        quality.low_end = true;
        info!(
            "Living background switched to low-end mode (menu p90 {:.1} ms)",
            p90 * 1000.0
        );
    }
    quality.elapsed = 0.0;
    quality.samples.clear();
}

#[allow(clippy::too_many_arguments)]
fn animate_living_backgrounds(
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    platform: Res<super::UiPlatform>,
    touches: Option<Res<Touches>>,
    settings: Res<MotionSettings>,
    quality: Res<LivingQuality>,
    mut clock: ResMut<LivingClock>,
    mut roots: Query<(Entity, &LivingBackground, &mut LivingFade)>,
    mut visuals: Query<(
        &LivingOwner,
        &LivingVisual,
        &LivingTint,
        &mut KitImage,
        &mut UiTransform,
        &mut Visibility,
    )>,
) {
    let Ok(window) = windows.single() else { return };
    let delta = if window.focused {
        time.delta_secs().min(0.05)
    } else {
        0.0
    };
    let fade_duration = motion::DURATION_SCREEN_FADE.as_secs_f32().max(f32::EPSILON);
    let mut root_state = Vec::new();
    for (entity, root, mut fade) in &mut roots {
        fade.0 = (fade.0 + delta / fade_duration).min(1.0);
        root_state.push((entity, root.scene, motion::EASING_STANDARD.ease(fade.0)));
    }
    if root_state.is_empty() {
        return;
    }
    let target_detail = if quality.low_end { 0.0 } else { 1.0 };
    let blend = (delta * 3.0).min(1.0);
    clock.detail += (target_detail - clock.detail) * blend;

    if delta > 0.0 && !settings.reduce {
        clock.elapsed += delta;
        let idle = Vec2::new(
            motion::LIVING_DRIFT_AMP_X
                * (TAU * clock.elapsed / motion::DURATION_LIVING_DRIFT_X.as_secs_f32()).sin(),
            motion::LIVING_DRIFT_AMP_Y
                * (TAU * clock.elapsed / motion::DURATION_LIVING_DRIFT_Y.as_secs_f32()
                    + motion::LIVING_DRIFT_PHASE_Y)
                    .sin(),
        );
        let position = if platform.is_mobile() {
            touches
                .as_ref()
                .and_then(|touches| touches.iter().next().map(|touch| touch.position()))
        } else {
            window.cursor_position()
        };
        let pointer = position.map_or(Vec2::ZERO, |cursor| {
            Vec2::new(
                cursor.x / window.width() * 2.0 - 1.0,
                cursor.y / window.height() * 2.0 - 1.0,
            )
            .clamp(Vec2::splat(-1.0), Vec2::ONE)
                * motion::LIVING_INPUT_GAIN
        });
        let blend = 1.0 - (1.0 - motion::LIVING_LERP).powf(60.0 * delta);
        let previous = clock.drift;
        clock.drift += (pointer + idle - previous) * blend;
    }

    let viewport = Vec2::new(window.width(), window.height());
    for (owner, visual, base, mut image, mut transform, mut visibility) in &mut visuals {
        let Some((_, scene, fade)) = root_state.iter().find(|(entity, _, _)| *entity == owner.0)
        else {
            continue;
        };
        let detailed = !visual_is_shown(visual, false, true);
        let shown =
            visual_is_shown(visual, settings.reduce, false) && (!detailed || clock.detail > 0.005);
        *visibility = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if !shown {
            continue;
        }

        let mut alpha_factor = *fade;
        if detailed {
            alpha_factor *= clock.detail;
        }
        let t = clock.elapsed;
        match *visual {
            LivingVisual::Plate => {
                transform.scale = Vec2::splat(motion::LIVING_SCALE_PLATE);
                transform.translation = if settings.reduce {
                    Val2::ZERO
                } else {
                    Val2::px(
                        -clock.drift.x * motion::LIVING_DEPTH_PLATE_X,
                        -clock.drift.y * motion::LIVING_DEPTH_PLATE_Y,
                    )
                };
            }
            LivingVisual::Foreground => {
                transform.scale = Vec2::splat(motion::LIVING_SCALE_FOREGROUND);
                transform.translation = if settings.reduce {
                    Val2::ZERO
                } else {
                    Val2::px(
                        -clock.drift.x * motion::LIVING_DEPTH_FOREGROUND_X,
                        -clock.drift.y * motion::LIVING_DEPTH_FOREGROUND_Y,
                    )
                };
            }
            LivingVisual::Vignette => *transform = UiTransform::IDENTITY,
            LivingVisual::Ray { index } => {
                let config = scene.config();
                let spread = index as f32 / config.ray_count as f32 - 0.5;
                transform.rotation = Rot2::radians(
                    spread * 1.1
                        + config.ray_offset
                        + 0.05 * (TAU * t / 41.89 + index as f32).sin(),
                );
                alpha_factor *= 0.55 + 0.45 * (TAU * t / 15.71 + 1.7 * index as f32).sin();
            }
            LivingVisual::Fog { index } => {
                let x = ((10.0 * (index + 1) as f32 * t) % (2.0 * viewport.x)) - 0.5 * viewport.x;
                let y = 10.0 * (TAU * t / 31.42 + 2.0 * index as f32).sin();
                transform.translation = Val2::px(x, y);
            }
            LivingVisual::Rune => {
                alpha_factor *= 0.59 + 0.41 * (TAU * t / 3.49).sin();
            }
            LivingVisual::Mote(particle) => {
                let travel = (particle.speed * t + particle.base.y + 0.05).rem_euclid(1.10) - 0.05;
                let y = (particle.base.y - travel) * viewport.y;
                let x = 16.0 * (TAU * t / 10.47 + particle.phase).sin()
                    - clock.drift.x
                        * if particle.front {
                            motion::LIVING_DEPTH_PARTICLES_FRONT
                        } else {
                            motion::LIVING_DEPTH_PARTICLES_BACK
                        };
                transform.translation = Val2::px(x, y);
                let scale = if particle.front { 1.8 } else { 1.0 };
                transform.scale = Vec2::splat(scale);
                alpha_factor *= 0.35 + 0.35 * (TAU * t / 2.09 + particle.phase).sin();
            }
            LivingVisual::Flake(particle) => {
                let travel = (particle.speed * t - particle.base.y).rem_euclid(1.20) - 0.10;
                let y = (travel - particle.base.y) * viewport.y;
                let depth = if particle.front {
                    motion::LIVING_DEPTH_PARTICLES_FRONT
                } else {
                    motion::LIVING_DEPTH_PARTICLES_BACK
                } * motion::LIVING_DEPTH_LEAF_FACTOR;
                let x = 20.0 * (TAU * t / 10.47 + particle.phase).sin() - clock.drift.x * depth;
                transform.translation = Val2::px(x, y);
                transform.rotation = Rot2::radians(particle.spin * t);
                let flutter = 0.45 + 0.55 * (TAU * t / 4.19 + particle.phase).sin().abs();
                transform.scale.y = flutter;
                alpha_factor *= if particle.front { 0.95 } else { 0.70 };
            }
        }
        let alpha = base.0.alpha() * alpha_factor.clamp(0.0, 1.0);
        image.tint = if alpha < 0.999 {
            theme::perceptual(base.0.with_alpha(alpha))
        } else {
            base.0
        };
    }
}

fn visual_is_shown(visual: &LivingVisual, reduce_motion: bool, low_end: bool) -> bool {
    let effect = matches!(
        visual,
        LivingVisual::Ray { .. }
            | LivingVisual::Fog { .. }
            | LivingVisual::Rune
            | LivingVisual::Mote(_)
            | LivingVisual::Flake(_)
    );
    let foreground = matches!(visual, LivingVisual::Foreground);
    if low_end {
        !foreground && !effect
    } else if reduce_motion {
        !effect
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_stay_inside_the_handoff_node_budget() {
        for scene in [
            LivingScene::Arena,
            LivingScene::Stage,
            LivingScene::Victory,
            LivingScene::Defeat,
        ] {
            let c = scene.config();
            let nodes = 3
                + c.ray_count
                + c.mote_count
                + c.flake_count
                + usize::from(c.fog.is_some()) * 3
                + usize::from(c.rune.is_some())
                + 2; // worst-case header + footer bands
            assert!(nodes <= 124, "{scene:?}: {nodes} nodes");
        }
    }

    #[test]
    fn scene_assets_are_complete_at_both_resolutions() {
        for scene in [
            LivingScene::Arena,
            LivingScene::Stage,
            LivingScene::Victory,
            LivingScene::Defeat,
        ] {
            assert!(scene.plate().path(false).ends_with("plate-720.jpg"));
            assert!(scene.plate().path(true).ends_with("plate-1080.jpg"));
            assert!(
                scene
                    .foreground()
                    .path(false)
                    .ends_with("foreground-720.webp")
            );
            assert!(
                scene
                    .foreground()
                    .path(true)
                    .ends_with("foreground-1080.webp")
            );
        }
    }

    #[test]
    fn phones_always_use_720_and_large_desktops_use_1080() {
        assert!(!use_high_density(true, 1080));
        assert!(!use_high_density(false, 800));
        assert!(use_high_density(false, 801));
    }

    #[test]
    fn deterministic_particles_are_bounded() {
        for index in 0..80 {
            let mote = mote(index, (0.03, 0.08));
            assert!((0.0..=1.0).contains(&mote.base.x));
            assert!((0.0..=1.0).contains(&mote.base.y));
            assert!((0.03..=0.08).contains(&mote.speed));
            let flake = flake(index);
            assert!((0.0..=1.0).contains(&flake.base.x));
            assert!((-1.0..=0.0).contains(&flake.base.y));
            assert!((0.08..=0.16).contains(&flake.speed));
        }
    }

    #[test]
    fn motion_and_low_end_states_keep_exactly_the_specified_layers() {
        let layers = [
            LivingVisual::Plate,
            LivingVisual::Foreground,
            LivingVisual::Vignette,
            LivingVisual::Ray { index: 0 },
            LivingVisual::Fog { index: 0 },
            LivingVisual::Rune,
            LivingVisual::Mote(mote(0, (0.03, 0.08))),
            LivingVisual::Flake(flake(0)),
        ];
        assert!(
            layers
                .iter()
                .all(|visual| visual_is_shown(visual, false, false))
        );
        assert_eq!(
            layers
                .iter()
                .map(|visual| visual_is_shown(visual, true, false))
                .collect::<Vec<_>>(),
            [true, true, true, false, false, false, false, false]
        );
        assert_eq!(
            layers
                .iter()
                .map(|visual| visual_is_shown(visual, false, true))
                .collect::<Vec<_>>(),
            [true, false, true, false, false, false, false, false]
        );
    }
}
