//! Server-authorized cosmetics. Preview has a separate camera and never equips a
//! world actor; all live aura components come only from the server snapshot.
//!
//! Hero tiers (LR-67): ten styles share one 23-element effect. The four solid
//! tiers (Rose, Jade, Azure, Crimson) are palettes; the three Prism tiers cycle
//! their hue every frame on the style's shared materials, so every wearer of
//! one style shimmers in sync, faster and brighter per tier (Prism II adds
//! stronger sparkles, Prism III a crown ring in place of one spark). How a
//! player can buy access comes from [`payment_surface`].
// i18n-strict
use crate::ui::living_background::{self, LivingBands, LivingScene};
use crate::{
    career::{CareerClient, CareerModal},
    combat::CombatStats,
    i18n::{Locale, data, tr, trf},
    input_context::InputContextSet,
    net::{GameState, GameStateSnapshot, NetworkCommand, RemotePlayer},
    player::Player,
    sprite::PlayerVisualMode,
    ui::{
        Activated, ModalId, ModalRoot, Pressable, ScrollArea, TestId, UiAction, UiActionAppExt,
        UiSet, theme as ui,
    },
};
use bevy::{
    asset::RenderAssetUsages,
    camera::{RenderTarget, visibility::RenderLayers},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use shared::{
    career::CareerRequest,
    market::{PaymentProvider, sells, sells_tiers},
    supporter::{AuraStyle, SupporterStatus},
};
use std::collections::HashMap;

/// Every style has its own shared materials.
const STYLE_COUNT: usize = AuraStyle::ALL.len();

const PREVIEW_LAYER: usize = 29;
const AURA_RADIUS: f32 = 0.94;
const ORBIT_HEIGHT: f32 = 1.02;
const RIBBON_SEGMENTS: usize = 32;
const RIBBON_SWEEP: f32 = 2.85;
const ORBIT_COUNT: usize = 3;
const TRAIL_SAMPLES: usize = 6;
const GLINT_COUNT: usize = 8;
const TRAIL_STEP: f64 = 0.24;
const CROWN_HEIGHT: f32 = 2.02;
// Each 3D orbit is two shared ribbons plus two head billboards. Unlike the old
// 18 stretched spheres, ribbons cost 1,536 triangles total and no vertex uploads.
pub(crate) const HERO_AURA_ELEMENT_BUDGET: usize = 1 + 2 + ORBIT_COUNT * 4 + GLINT_COUNT;
const FLAT_AURA_ELEMENT_BUDGET: usize = 1 + 2 + ORBIT_COUNT * (2 + TRAIL_SAMPLES) + GLINT_COUNT;
fn aura_element_budget(flat: bool) -> usize {
    if flat {
        FLAT_AURA_ELEMENT_BUDGET
    } else {
        HERO_AURA_ELEMENT_BUDGET
    }
}

#[derive(Clone, Copy)]
enum AuraParticle {
    Mantle,
    Radiance,
    Core,
    Halo,
    Trail(usize),
    RibbonCore,
    RibbonGlow,
    Glint,
    /// Prism III: a ring above the head, taking one spark's place in the budget.
    Crown,
}

/// How this build lets a player buy Hero access (launch-readiness section 7,
/// "Purchase surface"; LR-68 adds the channel axis).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaymentSurface {
    /// Nothing is sold here (the panel still previews and equips).
    None,
    /// StoreKit in the iOS app (global Apple).
    StoreKit,
    /// «Поддержать на сайте»: the region portal's `/supporter` in the browser.
    Website,
}

/// The one accessor over the build's market for the Supporter UI (LR-68 R0
/// swaps its body for the bound region and channel). M0 builds are the
/// `direct` channel: an iOS build never links to a web payment.
pub(crate) fn payment_surface() -> PaymentSurface {
    payment_surface_for(
        crate::account_region::payment_providers(),
        cfg!(target_os = "ios"),
    )
}

/// The purchase surface for a provider list on iOS or another platform.
pub(crate) fn payment_surface_for(providers: &[PaymentProvider], ios: bool) -> PaymentSurface {
    if ios {
        if sells(providers, PaymentProvider::Apple) {
            PaymentSurface::StoreKit
        } else {
            PaymentSurface::None
        }
    } else if sells(providers, PaymentProvider::YooKassa) {
        PaymentSurface::Website
    } else {
        // Global desktop and Android stay as before: no in-game link to Solana.
        PaymentSurface::None
    }
}

/// What the panel offers, read once from [`payment_surface`] and the market.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PurchaseOffer {
    pub surface: PaymentSurface,
    /// Whether a provider here carries tier points (the tier styles and the
    /// tier line are shown); unlocked tier styles always show.
    pub tiers: bool,
}
impl Default for PurchaseOffer {
    fn default() -> Self {
        Self {
            surface: payment_surface(),
            tiers: sells_tiers(crate::account_region::payment_providers()),
        }
    }
}

/// Opens the region portal's Supporter page. Function pointers so tests can
/// observe the link without a browser.
#[derive(Resource, Clone, Copy)]
pub(crate) struct SupporterWebsite {
    pub url: fn() -> Result<String, String>,
    pub open: fn(&str) -> Result<(), String>,
}
impl Default for SupporterWebsite {
    fn default() -> Self {
        Self {
            url: || {
                omoba_passport::web_account::WebAccountApi::from_env()
                    .map(|api| omoba_passport::supporter_account::website_url(&api))
            },
            open: crate::platform::open_external_url,
        }
    }
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NetworkSupporterAura(pub Option<AuraStyle>);
#[derive(Resource)]
pub(crate) struct SupporterUiState {
    pub open: bool,
    pub selected: AuraStyle,
}
impl Default for SupporterUiState {
    fn default() -> Self {
        Self {
            // `OMOBA_QA_SUPPORTER=1` opens the panel for `qa::supporter` captures.
            open: cfg!(feature = "qa")
                && std::env::var("OMOBA_QA_SUPPORTER").is_ok_and(|value| value == "1"),
            selected: AuraStyle::Solar,
        }
    }
}
#[derive(Message, Clone, Copy, Debug)]
pub(crate) enum SupporterPlatformAction {
    Purchase,
    Restore,
}
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub(crate) struct SupporterPlatformState {
    pub available: bool,
    pub busy: bool,
    /// Unfinished StoreKit receipts must stay with this region until settled.
    pub pending_receipts: bool,
    pub price_label: Option<String>,
    pub message: Option<PlatformMessage>,
}
/// A store line: client copy by key (shown in the active language) or text
/// from StoreKit / the account service, shown as sent.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub(crate) enum PlatformMessage {
    Key(&'static str),
    Text(String),
}
impl PlatformMessage {
    pub(crate) fn text(&self) -> &str {
        match self {
            Self::Key(key) => tr(key),
            Self::Text(text) => text,
        }
    }
}
#[derive(Resource, Default)]
struct AuraRegistry(HashMap<Entity, (Entity, AuraStyle, PlayerVisualMode)>);
#[derive(Component)]
pub(crate) struct AuraOwner(pub(crate) Entity);
#[derive(Component)]
struct AuraAnchor;
#[derive(Component)]
struct AuraRing;
#[derive(Component)]
struct Orbit {
    index: f32,
    flat: bool,
    particle: AuraParticle,
    preview: bool,
    /// Spark brightness: 1 for most styles, more for Prism II and III.
    sparkle: f32,
}
#[derive(Component)]
struct PreviewCamera;
#[derive(Component)]
struct PreviewObject;
#[derive(Component)]
struct PreviewAura;
#[derive(Component)]
struct SupporterRoot;
#[derive(Component)]
struct SupporterScroll;
#[derive(Component)]
struct PreviewImage;
/// Supporter panel presses. The buttons keep their flat look (no hover
/// colour, as before): they carry `UiAction<Action>` but no `ButtonStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Close,
    Select(AuraStyle),
    Equip,
    Disable,
    Refresh,
    Purchase,
    Restore,
    Website,
}
type Styled<T> = [Handle<T>; STYLE_COUNT];
#[derive(Resource, Default)]
struct AuraAssets {
    ring: Handle<Mesh>,
    crown: Handle<Mesh>,
    ribbon_core: Handle<Mesh>,
    ribbon_glow: Handle<Mesh>,
    flat_orb: Handle<Mesh>,
    glow_quad: Handle<Mesh>,
    material: Styled<StandardMaterial>,
    flat: Styled<ColorMaterial>,
    cores: Styled<StandardMaterial>,
    halos: Styled<StandardMaterial>,
    trails: Styled<StandardMaterial>,
    ribbon_cores: Styled<StandardMaterial>,
    flat_cores: Styled<ColorMaterial>,
    flat_halos: Styled<ColorMaterial>,
    flat_trails: Styled<ColorMaterial>,
    mantles: Styled<StandardMaterial>,
    radiance: Styled<StandardMaterial>,
    flat_mantles: Styled<ColorMaterial>,
    flat_radiance: Styled<ColorMaterial>,
    preview: Handle<Image>,
}
fn style_index(style: AuraStyle) -> usize {
    AuraStyle::ALL
        .iter()
        .position(|candidate| *candidate == style)
        .expect("every style is listed")
}
/// A style's tint; the Prism tints at the start of their cycle.
fn color(style: AuraStyle) -> Color {
    match style {
        AuraStyle::Solar => Color::srgb(1., 0.67, 0.24),
        AuraStyle::Lunar => Color::srgb(0.43, 0.69, 1.),
        AuraStyle::Verdant => Color::srgb(0.25, 0.96, 0.65),
        AuraStyle::Rose => Color::srgb(1., 0.42, 0.72),
        AuraStyle::Jade => Color::srgb(0.10, 0.80, 0.45),
        AuraStyle::Azure => Color::srgb(0.15, 0.48, 1.),
        AuraStyle::Crimson => Color::srgb(0.95, 0.10, 0.18),
        AuraStyle::PrismI | AuraStyle::PrismII | AuraStyle::PrismIII => prism_tint(style, 0., 0.),
    }
}

/// Hue cycles per second, glow gain and spark size of a Prism tier; `None`
/// for a style with a fixed palette. Each tier shimmers faster and brighter.
fn prism(style: AuraStyle) -> Option<(f64, f32, f32)> {
    match style {
        AuraStyle::PrismI => Some((0.08, 1.0, 1.0)),
        AuraStyle::PrismII => Some((0.16, 1.15, 1.5)),
        AuraStyle::PrismIII => Some((0.30, 1.3, 1.8)),
        _ => None,
    }
}
fn prism_tint(style: AuraStyle, seconds: f64, offset_degrees: f32) -> Color {
    let speed = prism(style).map_or(0., |(speed, _, _)| speed);
    let hue = ((seconds * speed).rem_euclid(1.) * 360.) as f32 + offset_degrees;
    Color::hsl(hue.rem_euclid(360.), 0.9, 0.62)
}
fn sparkle(style: AuraStyle) -> f32 {
    prism(style).map_or(1., |(_, _, sparkle)| sparkle)
}

/// Base colours of one style's shared materials, 3D and 2D. Prism styles get
/// three hues at once (orbits, heads, body glow) so the aura reads as a rainbow.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Palette {
    ring: Color,
    ring_emissive: LinearRgba,
    core: Color,
    halo: Color,
    trail: Color,
    ribbon_core: Color,
    mantle: Color,
    radiance: Color,
    flat_ring: Color,
    flat_core: Color,
    flat_halo: Color,
    flat_trail: Color,
    flat_mantle: Color,
    flat_radiance: Color,
}
fn palette(style: AuraStyle, seconds: f64) -> Palette {
    let glow = prism(style).map_or(1., |(_, glow, _)| glow);
    let tint = |offset: f32| {
        if prism(style).is_some() {
            prism_tint(style, seconds, offset)
        } else {
            color(style)
        }
    };
    let (orbit, head, body) = (tint(0.), tint(120.), tint(240.));
    let hdr = |tint: Color, white: f32, gain: f32| {
        crate::game_vfx::hdr_tint(tint.mix(&Color::WHITE, white), gain * glow)
    };
    Palette {
        ring: orbit.with_alpha(0.22),
        ring_emissive: LinearRgba::from(orbit) * 0.25 * glow,
        core: hdr(head, 0.85, 2.2).with_alpha(1.0),
        halo: hdr(head, 0.1, 2.2).with_alpha(0.85),
        trail: hdr(orbit, 0.10, 2.2).with_alpha(0.65),
        ribbon_core: hdr(orbit, 0.92, 4.0),
        // Daylit maps need a broad HDR halo, not only tiny orbit highlights.
        mantle: hdr(body, 0.25, 6.).with_alpha(0.23),
        radiance: hdr(body, 0.75, 6.).with_alpha(0.12),
        flat_ring: orbit.with_alpha(0.24),
        flat_core: head.mix(&Color::WHITE, 0.85),
        flat_halo: head.with_alpha(0.7),
        flat_trail: orbit.with_alpha(0.65),
        flat_mantle: body.with_alpha(0.48),
        flat_radiance: body.mix(&Color::WHITE, 0.72).with_alpha(0.25),
    }
}

// Shared materials and analytic paths keep the per-hero budget fixed. No emitter
// churn, dynamic lights, or bloom dependency on mobile renderers.
fn particle_material(base_color: Color) -> StandardMaterial {
    StandardMaterial {
        // Unlit materials draw base_color directly; emissive alone does not
        // brighten them. Use the same HDR convention as the game's other VFX.
        base_color,
        unlit: true,
        alpha_mode: AlphaMode::Add,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

fn glow_texture(star: bool) -> Image {
    glow_texture_with_falloff(star, 3.)
}

fn glow_texture_with_falloff(star: bool, falloff: f32) -> Image {
    const SIZE: u32 = 64;
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let u = (x as f32 + 0.5) / SIZE as f32 * 2. - 1.;
            let v = (y as f32 + 0.5) / SIZE as f32 * 2. - 1.;
            let radius = (u * u + v * v).sqrt();
            let edge = (1. - radius).max(0.);
            let mut alpha = edge.powf(falloff);
            if star {
                let rays = (-u.abs() * 45.).exp() * (1. - v.abs()).powi(2)
                    + (-v.abs() * 45.).exp() * (1. - u.abs()).powi(2);
                alpha = (alpha * 0.6 + rays * edge).min(1.);
            }
            data.extend_from_slice(&[255, 255, 255, (alpha * 255.) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// A circular ribbon rotated in its own plane: head and tail always use the
/// same frame. Reversing one orbit uses a proper 3D rotation (not negative scale),
/// so its trail still follows behind its head. Time reduction prevents long-session
/// precision loss; neither path nor brightness depends on frame rate.
fn orbit_rotation(index: f32, seconds: f64) -> Quat {
    let phase = (seconds * (1.15 + index as f64 * 0.17)
        + index as f64 * std::f64::consts::TAU / ORBIT_COUNT as f64)
        .rem_euclid(std::f64::consts::TAU) as f32;
    let precession = (seconds * 0.08).rem_euclid(std::f64::consts::TAU) as f32;
    let plane = Quat::from_rotation_y(index * std::f32::consts::TAU / 3. + precession)
        * Quat::from_rotation_z(0.32)
        * Quat::from_rotation_x(0.22);
    let reverse = if index == 1. {
        Quat::from_rotation_x(std::f32::consts::PI)
    } else {
        Quat::IDENTITY
    };
    plane * reverse * Quat::from_rotation_z(phase)
}
fn orbital_point(index: f32, seconds: f64) -> Vec3 {
    Vec3::Y * ORBIT_HEIGHT + orbit_rotation(index, seconds) * Vec3::X * AURA_RADIUS
}

/// Two crossed, feathered sheets prevent an orbit disappearing edge-on. Width
/// and vertex alpha both taper smoothly to zero. Built once and shared by every
/// Hero; the animation only transforms the mesh, never rebuilds it.
fn ribbon_mesh(half_width: f32) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    for sheet in 0..2 {
        let offset = positions.len() as u32;
        for step in 0..=RIBBON_SEGMENTS {
            let u = step as f32 / RIBBON_SEGMENTS as f32;
            let angle = -u * RIBBON_SWEEP;
            let radial = Vec3::new(angle.cos(), angle.sin(), 0.);
            let across = if sheet == 0 { radial } else { Vec3::Z };
            let taper = (1. - u).powf(0.75);
            let fade = (1. - u).powf(1.3);
            for side in [-1., 0., 1.] {
                positions
                    .push((radial * AURA_RADIUS + across * (side * half_width * taper)).to_array());
                normals.push(if sheet == 0 {
                    Vec3::Z.to_array()
                } else {
                    radial.to_array()
                });
                colors.push([1., 1., 1., if side == 0. { fade } else { 0. }]);
            }
        }
        for step in 0..RIBBON_SEGMENTS as u32 {
            for column in 0..2 {
                let a = offset + step * 3 + column;
                indices.extend_from_slice(&[a, a + 3, a + 1, a + 1, a + 3, a + 4]);
            }
        }
    }
    let vertices = positions.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5, 0.5]; vertices])
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

fn project_particle(point: Vec3, flat: bool) -> Vec3 {
    if flat {
        Vec3::new(point.x, point.y * 0.68 + point.z * 0.35, 0.)
    } else {
        point
    }
}

fn particle_pose(orbit: &Orbit, seconds: f64) -> Transform {
    let (position, scale, rotation) = match orbit.particle {
        AuraParticle::Mantle | AuraParticle::Radiance => {
            let breath = 1. + 0.04 * (seconds as f32 * 1.7).sin();
            let size = if matches!(orbit.particle, AuraParticle::Mantle) {
                Vec3::new(12., 19., 1.)
            } else {
                Vec3::new(7., 12., 1.)
            };
            (Vec3::Y * 1.05, size * breath, Quat::IDENTITY)
        }
        AuraParticle::Core | AuraParticle::Halo => {
            let head = orbital_point(orbit.index, seconds);
            let pulse = 1. + 0.10 * (seconds as f32 * 2.1 + orbit.index).sin();
            let size = if matches!(orbit.particle, AuraParticle::Halo) {
                4.6
            } else {
                3.2
            };
            (head, Vec3::splat(size * pulse), Quat::IDENTITY)
        }
        AuraParticle::RibbonCore | AuraParticle::RibbonGlow => (
            Vec3::Y * ORBIT_HEIGHT,
            Vec3::ONE,
            orbit_rotation(orbit.index, seconds),
        ),
        AuraParticle::Trail(sample) => {
            let start = orbital_point(orbit.index, seconds - sample as f64 * TRAIL_STEP);
            let end = orbital_point(orbit.index, seconds - (sample + 1) as f64 * TRAIL_STEP);
            let delta = project_particle(start, orbit.flat) - project_particle(end, orbit.flat);
            let taper = 1. - sample as f32 / TRAIL_SAMPLES as f32;
            let width = 0.48 * taper * taper + 0.025;
            (
                (start + end) * 0.5,
                Vec3::new(delta.length() / 0.13 + width, width, width),
                Quat::from_rotation_arc(Vec3::X, delta.normalize_or(Vec3::X)),
            )
        }
        AuraParticle::Glint => {
            let age =
                (seconds * 0.38 + orbit.index as f64 / GLINT_COUNT as f64).rem_euclid(1.) as f32;
            let phase = orbit.index * 2.399 + age * 1.8;
            let radius = AURA_RADIUS * (0.58 + 0.30 * age);
            let point = Vec3::new(
                phase.cos() * radius,
                0.06 + age * 1.90,
                phase.sin() * radius,
            );
            // Fade to zero at each wrap: rising sparks never pop at the feet.
            let sparkle = (std::f32::consts::PI * age).sin().powi(2) * 1.35 * orbit.sparkle;
            (point, Vec3::new(0.8, 1.65, 1.) * sparkle, Quat::IDENTITY)
        }
        AuraParticle::Crown => {
            // A slow, slightly tilting ring above the head (Prism III).
            let tilt = 0.10 * (seconds as f32 * 1.7).sin();
            let pulse = 1. + 0.06 * (seconds as f32 * 2.3).sin();
            let rotation = if orbit.flat {
                Quat::IDENTITY
            } else {
                Quat::from_rotation_y((seconds * 0.9).rem_euclid(std::f64::consts::TAU) as f32)
                    * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2 + tilt)
            };
            let squash = if orbit.flat { 0.42 } else { 1. };
            (
                Vec3::Y * CROWN_HEIGHT,
                Vec3::new(pulse, pulse * squash, pulse),
                rotation,
            )
        }
    };
    let mut projected = project_particle(position, orbit.flat);
    if orbit.flat {
        // Orbit behind the sprite on the far half and in front on the near half.
        projected.z = if matches!(
            orbit.particle,
            AuraParticle::Mantle | AuraParticle::Radiance
        ) {
            0.02
        } else if position.z >= 0. {
            crate::world2d::layer::VFX - crate::world2d::layer::MARKER
        } else {
            0.02
        };
    }
    Transform::from_translation(projected)
        .with_scale(scale)
        .with_rotation(rotation)
}

pub(crate) struct SupporterPlugin;
impl Plugin for SupporterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SupporterUiState>()
            .init_resource::<SupporterPlatformState>()
            .init_resource::<PurchaseOffer>()
            .init_resource::<SupporterWebsite>()
            .init_resource::<AuraRegistry>()
            .add_message::<SupporterPlatformAction>()
            .add_ui_action::<Action>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (actions, keyboard_close)
                    .chain()
                    .in_set(InputContextSet::Modal)
                    .after(UiSet::Dispatch)
                    .before(crate::pause_menu::toggle_pause_menu),
            )
            .add_systems(
                Update,
                (render_panel, sync_preview)
                    .chain()
                    .after(InputContextSet::Modal),
            )
            .add_systems(
                Update,
                (
                    sync_auras.after(crate::net::ClientNetPipeline::ApplySnapshot),
                    shimmer_prisms.after(sync_auras),
                ),
            )
            .add_systems(
                PostUpdate,
                (follow_actors, animate_orbits)
                    .chain()
                    .after(crate::net::NetworkGroundingSet)
                    .after(bevy::transform::TransformSystems::Propagate)
                    .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate)
                    .before(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
            );
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut flat: ResMut<Assets<ColorMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let image = images.add(Image::new_target_texture(
        320,
        224,
        TextureFormat::Rgba8Unorm,
        Some(TextureFormat::Rgba8UnormSrgb),
    ));
    let glow = images.add(glow_texture(false));
    let body_glow = images.add(glow_texture_with_falloff(false, 1.5));
    let star = images.add(glow_texture(true));
    let palettes = AuraStyle::ALL.map(|style| palette(style, 0.));
    let aura = AuraAssets {
        ring: meshes.add(Annulus::new(AURA_RADIUS - 0.025, AURA_RADIUS)),
        crown: meshes.add(Annulus::new(0.27, 0.33)),
        ribbon_core: meshes.add(ribbon_mesh(0.018)),
        ribbon_glow: meshes.add(ribbon_mesh(0.075)),
        flat_orb: meshes.add(Circle::new(0.065)),
        glow_quad: meshes.add(Rectangle::new(0.13, 0.13)),
        material: palettes.map(|p| {
            materials.add(StandardMaterial {
                emissive: p.ring_emissive,
                ..particle_material(p.ring)
            })
        }),
        flat: palettes.map(|p| flat.add(p.flat_ring)),
        cores: palettes.map(|p| {
            materials.add(StandardMaterial {
                base_color_texture: Some(star.clone()),
                ..particle_material(p.core)
            })
        }),
        halos: palettes.map(|p| {
            materials.add(StandardMaterial {
                base_color_texture: Some(glow.clone()),
                ..particle_material(p.halo)
            })
        }),
        trails: palettes.map(|p| materials.add(particle_material(p.trail))),
        ribbon_cores: palettes.map(|p| materials.add(particle_material(p.ribbon_core))),
        flat_cores: palettes.map(|p| {
            flat.add(ColorMaterial {
                color: p.flat_core,
                texture: Some(star.clone()),
                ..default()
            })
        }),
        flat_halos: palettes.map(|p| {
            flat.add(ColorMaterial {
                color: p.flat_halo,
                texture: Some(glow.clone()),
                ..default()
            })
        }),
        flat_trails: palettes.map(|p| flat.add(p.flat_trail)),
        mantles: palettes.map(|p| {
            materials.add(StandardMaterial {
                base_color_texture: Some(body_glow.clone()),
                ..particle_material(p.mantle)
            })
        }),
        radiance: palettes.map(|p| {
            materials.add(StandardMaterial {
                base_color_texture: Some(body_glow.clone()),
                ..particle_material(p.radiance)
            })
        }),
        flat_mantles: palettes.map(|p| {
            flat.add(ColorMaterial {
                color: p.flat_mantle,
                texture: Some(glow.clone()),
                ..default()
            })
        }),
        flat_radiance: palettes.map(|p| {
            flat.add(ColorMaterial {
                color: p.flat_radiance,
                texture: Some(glow.clone()),
                ..default()
            })
        }),
        preview: image.clone(),
    };
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -1,
            is_active: false,
            clear_color: Color::srgb(0.025, 0.06, 0.065).into(),
            ..default()
        },
        RenderTarget::Image(image.into()),
        // Frame the full orbit crown and foot ring in the 320×224 preview.
        Transform::from_xyz(1.9, 1.8, 2.8).looking_at(Vec3::Y * 0.95, Vec3::Y),
        RenderLayers::layer(PREVIEW_LAYER),
        PreviewCamera,
    ));
    // Neutral preview mannequin; no real player, account, combat or network ID.
    let body = materials.add(StandardMaterial {
        base_color: Color::srgb(0.58, 0.67, 0.64),
        unlit: true,
        ..default()
    });
    let limb = meshes.add(Capsule3d::new(0.065, 0.38));
    let leg = meshes.add(Capsule3d::new(0.08, 0.28));
    for (mesh, position, tilt) in [
        (meshes.add(Capsule3d::new(0.20, 0.40)), Vec3::Y * 0.84, 0.0),
        (meshes.add(Sphere::new(0.18)), Vec3::Y * 1.44, 0.0),
        (limb.clone(), Vec3::new(-0.29, 0.90, 0.0), -0.15),
        (limb, Vec3::new(0.29, 0.90, 0.0), 0.15),
        (leg.clone(), Vec3::new(-0.10, 0.23, 0.0), 0.0),
        (leg, Vec3::new(0.10, 0.23, 0.0), 0.0),
    ] {
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(body.clone()),
            Transform::from_translation(position).with_rotation(Quat::from_rotation_z(tilt)),
            Visibility::Inherited,
            RenderLayers::layer(PREVIEW_LAYER),
            PreviewObject,
        ));
    }
    commands.insert_resource(aura);
}

fn spawn_aura(
    commands: &mut Commands,
    assets: &AuraAssets,
    style: AuraStyle,
    flat: bool,
    layer: usize,
) -> Entity {
    let index = style_index(style);
    let root = commands
        .spawn((
            AuraAnchor,
            Transform::default(),
            Visibility::Hidden,
            Name::new("Supporter cosmetic aura"),
        ))
        .id();
    let ring_transform = if flat {
        Transform::default()
    } else {
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
    };
    let ring = commands
        .spawn((
            AuraRing,
            ring_transform,
            Visibility::Inherited,
            RenderLayers::layer(layer),
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ))
        .id();
    if flat {
        commands.entity(ring).insert((
            Mesh2d(assets.ring.clone()),
            MeshMaterial2d(assets.flat[index].clone()),
        ));
    } else {
        commands.entity(ring).insert((
            Mesh3d(assets.ring.clone()),
            MeshMaterial3d(assets.material[index].clone()),
        ));
    }
    commands.entity(root).add_child(ring);
    let mut particles = vec![(0, AuraParticle::Mantle), (0, AuraParticle::Radiance)];
    for i in 0..ORBIT_COUNT {
        particles.extend([(i, AuraParticle::Core), (i, AuraParticle::Halo)]);
        if flat {
            // Small, independently sorted segments preserve sprite front/back
            // crossings; a single ribbon cannot have both 2D sorting depths.
            particles.extend((0..TRAIL_SAMPLES).map(|sample| (i, AuraParticle::Trail(sample))));
        } else {
            particles.extend([(i, AuraParticle::RibbonCore), (i, AuraParticle::RibbonGlow)]);
        }
    }
    // The Prism III crown takes the last spark's place: the budget is unchanged.
    let crowned = style == AuraStyle::PrismIII;
    let glints = GLINT_COUNT - usize::from(crowned);
    particles.extend((0..glints).map(|i| (i, AuraParticle::Glint)));
    if crowned {
        particles.push((0, AuraParticle::Crown));
    }
    let mut elements = 1; // Ground ring.
    for (index_orbit, particle) in particles {
        elements += 1;
        let orbit = Orbit {
            index: index_orbit as f32,
            flat,
            particle,
            preview: layer == PREVIEW_LAYER,
            sparkle: sparkle(style),
        };
        let (material, flat_material) = match particle {
            AuraParticle::Mantle => (&assets.mantles[index], &assets.flat_mantles[index]),
            AuraParticle::Radiance => (&assets.radiance[index], &assets.flat_radiance[index]),
            AuraParticle::Core | AuraParticle::Glint => {
                (&assets.cores[index], &assets.flat_cores[index])
            }
            AuraParticle::Halo => (&assets.halos[index], &assets.flat_halos[index]),
            AuraParticle::Trail(_) | AuraParticle::RibbonGlow => {
                (&assets.trails[index], &assets.flat_trails[index])
            }
            AuraParticle::RibbonCore => (&assets.ribbon_cores[index], &assets.flat_trails[index]),
            AuraParticle::Crown => (&assets.ribbon_cores[index], &assets.flat_cores[index]),
        };
        let orb = commands
            .spawn((
                particle_pose(&orbit, 0.),
                orbit,
                Visibility::Inherited,
                RenderLayers::layer(layer),
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
            ))
            .id();
        let billboard = !matches!(
            particle,
            AuraParticle::Trail(_)
                | AuraParticle::RibbonCore
                | AuraParticle::RibbonGlow
                | AuraParticle::Crown
        );
        if flat {
            commands.entity(orb).insert((
                Mesh2d(if matches!(particle, AuraParticle::Crown) {
                    assets.crown.clone()
                } else if billboard {
                    assets.glow_quad.clone()
                } else {
                    assets.flat_orb.clone()
                }),
                MeshMaterial2d(flat_material.clone()),
            ));
        } else {
            commands.entity(orb).insert((
                Mesh3d(match particle {
                    AuraParticle::RibbonCore => assets.ribbon_core.clone(),
                    AuraParticle::RibbonGlow => assets.ribbon_glow.clone(),
                    AuraParticle::Crown => assets.crown.clone(),
                    _ => assets.glow_quad.clone(),
                }),
                MeshMaterial3d(material.clone()),
            ));
        }
        commands.entity(root).add_child(orb);
    }
    debug_assert_eq!(elements, aura_element_budget(flat));
    root
}

fn sync_auras(
    mut commands: Commands,
    assets: Res<AuraAssets>,
    mode: Res<PlayerVisualMode>,
    actors: Query<(Entity, &NetworkSupporterAura), Or<(With<Player>, With<RemotePlayer>)>>,
    mut registry: ResMut<AuraRegistry>,
) {
    registry.0.retain(|owner, (root, style, old_mode)| {
        if actors
            .get(*owner)
            .is_ok_and(|(_, aura)| aura.0 == Some(*style))
            && *old_mode == *mode
        {
            true
        } else {
            commands.entity(*root).despawn();
            false
        }
    });
    for (actor, aura) in &actors {
        if registry.0.contains_key(&actor) {
            continue;
        }
        if let Some(style) = aura.0 {
            let root = spawn_aura(
                &mut commands,
                &assets,
                style,
                *mode == PlayerVisualMode::Sprite2d,
                0,
            );
            commands.entity(root).insert(AuraOwner(actor));
            registry.0.insert(actor, (root, style, *mode));
        }
    }
}

/// Prism styles cycle their hue on their shared materials: one update per
/// style and frame, whatever the number of wearers, so they shimmer in sync.
/// Styles nobody wears (and the closed preview) are left untouched.
fn shimmer_prisms(
    time: Res<Time>,
    assets: Res<AuraAssets>,
    registry: Res<AuraRegistry>,
    state: Res<SupporterUiState>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut flat: ResMut<Assets<ColorMaterial>>,
) {
    let seconds = time.elapsed_secs_f64();
    for style in [AuraStyle::PrismI, AuraStyle::PrismII, AuraStyle::PrismIII] {
        let worn = registry.0.values().any(|(_, worn, _)| *worn == style)
            || (state.open && state.selected == style);
        if worn {
            paint(
                &assets,
                style,
                &palette(style, seconds),
                &mut materials,
                &mut flat,
            );
        }
    }
}
fn paint(
    assets: &AuraAssets,
    style: AuraStyle,
    palette: &Palette,
    materials: &mut Assets<StandardMaterial>,
    flat: &mut Assets<ColorMaterial>,
) {
    let index = style_index(style);
    for (handle, color) in [
        (&assets.material[index], palette.ring),
        (&assets.cores[index], palette.core),
        (&assets.halos[index], palette.halo),
        (&assets.trails[index], palette.trail),
        (&assets.ribbon_cores[index], palette.ribbon_core),
        (&assets.mantles[index], palette.mantle),
        (&assets.radiance[index], palette.radiance),
    ] {
        if let Some(mut material) = materials.get_mut(handle) {
            material.base_color = color;
        }
    }
    if let Some(mut ring) = materials.get_mut(&assets.material[index]) {
        ring.emissive = palette.ring_emissive;
    }
    for (handle, color) in [
        (&assets.flat[index], palette.flat_ring),
        (&assets.flat_cores[index], palette.flat_core),
        (&assets.flat_halos[index], palette.flat_halo),
        (&assets.flat_trails[index], palette.flat_trail),
        (&assets.flat_mantles[index], palette.flat_mantle),
        (&assets.flat_radiance[index], palette.flat_radiance),
    ] {
        if let Some(mut material) = flat.get_mut(handle) {
            material.color = color;
        }
    }
}

fn actor_visible(
    stats: &CombatStats,
    visibility: &Visibility,
    inherited: &InheritedVisibility,
) -> bool {
    stats.is_alive() && *visibility != Visibility::Hidden && inherited.get()
}
fn follow_actors(
    actors: Query<
        (
            &GlobalTransform,
            &CombatStats,
            &Visibility,
            &InheritedVisibility,
        ),
        Without<AuraOwner>,
    >,
    mut roots: Query<
        (
            &AuraOwner,
            &mut Transform,
            &mut GlobalTransform,
            &mut Visibility,
            &mut InheritedVisibility,
        ),
        Without<CombatStats>,
    >,
    mode: Res<PlayerVisualMode>,
    map: Res<crate::maps::MapLayout>,
) {
    for (owner, mut transform, mut global, mut visibility, mut inherited_aura) in &mut roots {
        let Ok((pose, stats, own_visibility, inherited)) = actors.get(owner.0) else {
            *visibility = Visibility::Hidden;
            *inherited_aura = InheritedVisibility::HIDDEN;
            continue;
        };
        let position = pose.translation();
        *visibility = if actor_visible(stats, own_visibility, inherited) && position.is_finite() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        *inherited_aura = if *visibility == Visibility::Hidden {
            InheritedVisibility::HIDDEN
        } else {
            InheritedVisibility::VISIBLE
        };
        *transform = if *mode == PlayerVisualMode::Sprite2d {
            Transform::from_translation(
                crate::world2d::simulation_xz_to_render_xy(position)
                    .extend(crate::world2d::layer::MARKER - 0.01),
            )
        } else {
            Transform::from_xyz(
                position.x,
                map.terrain_height_3d(position.x, position.z) + 0.10,
                position.z,
            )
        };
        // Follow the interpolated ground pose in the same frame. Child poses are
        // updated below after propagation; no animated skeleton/scale is copied.
        *global = GlobalTransform::from(*transform);
    }
}

fn animate_orbits(
    time: Res<Time>,
    roots: Query<(&GlobalTransform, &InheritedVisibility), With<AuraAnchor>>,
    cameras: Query<
        (&Camera, &GlobalTransform, Option<&PreviewCamera>),
        (
            With<Camera3d>,
            Without<Orbit>,
            Without<AuraRing>,
            Without<AuraAnchor>,
        ),
    >,
    mut orbits: Query<
        (
            &Orbit,
            &ChildOf,
            &mut Transform,
            &mut GlobalTransform,
            &mut InheritedVisibility,
        ),
        Without<AuraAnchor>,
    >,
    mut rings: Query<
        (
            &ChildOf,
            &Transform,
            &mut GlobalTransform,
            &mut InheritedVisibility,
        ),
        (With<AuraRing>, Without<Orbit>, Without<AuraAnchor>),
    >,
) {
    for (orbit, parent, mut transform, mut global, mut inherited) in &mut orbits {
        *transform = particle_pose(orbit, time.elapsed_secs_f64());
        if !orbit.flat
            && !matches!(
                orbit.particle,
                AuraParticle::Trail(_)
                    | AuraParticle::RibbonCore
                    | AuraParticle::RibbonGlow
                    | AuraParticle::Crown
            )
        {
            if let Some((_, pose, _)) = cameras
                .iter()
                .find(|(camera, _, preview)| camera.is_active && preview.is_some() == orbit.preview)
            {
                transform.rotation = pose.to_scale_rotation_translation().1;
                if matches!(
                    orbit.particle,
                    AuraParticle::Mantle | AuraParticle::Radiance
                ) {
                    // Put the broad glow behind the model in camera space, so
                    // its own shading, costume and team colours stay readable.
                    let behind = transform.rotation * Vec3::NEG_Z * 0.42;
                    transform.translation += behind;
                }
            }
        }
        if let Ok((parent, visible)) = roots.get(parent.parent()) {
            *global = parent.mul_transform(*transform);
            *inherited = *visible;
        }
    }
    for (parent, transform, mut global, mut inherited) in &mut rings {
        if let Ok((parent, visible)) = roots.get(parent.parent()) {
            *global = parent.mul_transform(*transform);
            *inherited = *visible;
        }
    }
}

fn sync_preview(
    mut commands: Commands,
    state: Res<SupporterUiState>,
    assets: Res<AuraAssets>,
    mut cameras: Query<&mut Camera, With<PreviewCamera>>,
    roots: Query<Entity, With<PreviewAura>>,
    mut previous: Local<Option<(bool, AuraStyle)>>,
) {
    for mut camera in &mut cameras {
        camera.is_active = state.open;
    }
    let next = (state.open, state.selected);
    if *previous == Some(next) {
        return;
    }
    *previous = Some(next);
    for entity in &roots {
        commands.entity(entity).despawn();
    }
    if state.open {
        let entity = spawn_aura(&mut commands, &assets, state.selected, false, PREVIEW_LAYER);
        commands.entity(entity).insert((
            PreviewAura,
            Visibility::Visible,
            Transform::from_xyz(0., 0.1, 0.),
        ));
    }
}
fn keyboard_close(mut back: crate::ui::BackInput, mut state: ResMut<SupporterUiState>) {
    if state.open && back.just_pressed() {
        state.open = false;
        back.consume();
    }
}
fn actions(
    mut activated: MessageReader<Activated<Action>>,
    mut state: ResMut<SupporterUiState>,
    mut career: ResMut<CareerClient>,
    mut platform: ResMut<SupporterPlatformState>,
    offer: Res<PurchaseOffer>,
    website: Res<SupporterWebsite>,
    mut platform_actions: MessageWriter<SupporterPlatformAction>,
    mut requests: MessageWriter<NetworkCommand>,
    mut sequence: Local<u64>,
) {
    for Activated { action, .. } in activated.read() {
        *sequence = sequence.saturating_add(1);
        let id = 0x5000_0000_0000_0000u64.saturating_add(*sequence);
        match action {
            Action::Close => state.open = false,
            Action::Select(style) => state.selected = *style,
            Action::Refresh => {
                requests.write(NetworkCommand::Career(CareerRequest::SupporterStatus {
                    request_id: id,
                }));
            }
            Action::Equip if style_unlocked(career.view.supporter.as_ref(), state.selected) => {
                requests.write(NetworkCommand::Career(CareerRequest::EquipSupporterAura {
                    request_id: id,
                    aura: Some(state.selected),
                }));
            }
            Action::Disable => {
                requests.write(NetworkCommand::Career(CareerRequest::EquipSupporterAura {
                    request_id: id,
                    aura: None,
                }));
            }
            Action::Purchase
                if platform.available
                    && platform.price_label.is_some()
                    && !platform.busy
                    && purchase_allowed(&career) =>
            {
                platform_actions.write(SupporterPlatformAction::Purchase);
            }
            Action::Restore if platform.available && !platform.busy => {
                platform_actions.write(SupporterPlatformAction::Restore);
            }
            Action::Website if offer.surface == PaymentSurface::Website => {
                platform.message = Some(PlatformMessage::Key(
                    match (website.url)().and_then(|url| (website.open)(&url)) {
                        Ok(()) => "tier.website.opened",
                        Err(_) => "tier.website.unavailable",
                    },
                ));
            }
            _ => {}
        }
        // Only one modal owns input. The account is still the source of truth.
        career.modal = CareerModal::Closed;
    }
}

/// Whether the server would let this player wear `style` now (the shared
/// entitlement rule over the last status the server sent).
fn style_unlocked(status: Option<&SupporterStatus>, style: AuraStyle) -> bool {
    status.is_some_and(|status| style.unlocked(status.active, status.tier_points))
}

/// The styles the panel lists: the base styles, the tier styles where this
/// build sells tiers, and any tier style the player has unlocked anyway.
fn listed_styles(offer: &PurchaseOffer, status: Option<&SupporterStatus>) -> Vec<AuraStyle> {
    AuraStyle::ALL
        .into_iter()
        .filter(|style| style.tier().is_none() || offer.tiers || style_unlocked(status, *style))
        .collect()
}

/// The tier line: the current tier until when, then the step-down; or how
/// donations add up. `None` where this build sells no tiers and none is held.
fn tier_label(offer: &PurchaseOffer, status: Option<&SupporterStatus>) -> Option<String> {
    let tier = status.and_then(SupporterStatus::tier);
    match (tier, status) {
        (Some(tier), Some(status)) => {
            let name = data::aura(tier.style());
            let until = status
                .tier_until
                .map(utc_timestamp)
                .unwrap_or_else(|| "—".into());
            Some(match status.next_tier() {
                Some(next) => trf(
                    "tier.status.step_down",
                    &[
                        ("tier", &name),
                        ("until", &until),
                        ("next", &data::aura(next.style())),
                    ],
                ),
                None => trf("tier.status.until", &[("tier", &name), ("until", &until)]),
            })
        }
        _ if offer.tiers => Some(tr("tier.status.none").into()),
        _ => None,
    }
}

fn purchase_allowed(career: &CareerClient) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_secs().min(i64::MAX as u64) as i64);
    purchase_allowed_at(career, now)
}
fn purchase_allowed_at(career: &CareerClient, now: i64) -> bool {
    career.view.storage_enabled
        && career.view.auth_nonce.is_some()
        && career.view.profile.is_some()
        && career.view.error.is_none()
        && career.view.supporter.as_ref().is_some_and(|s| {
            !s.active
                && !s
                    .grants
                    .iter()
                    .any(|grant| !grant.revoked && grant.valid_until > now)
        })
}

fn status_label(status: Option<&SupporterStatus>) -> String {
    let Some(status) = status else {
        return tr("supporter.status.no_account").into();
    };
    if !status.active {
        return tr("supporter.status.inactive").into();
    }
    let source = status
        .grants
        .iter()
        .filter(|g| !g.revoked && Some(g.valid_until) == status.active_until)
        .map(|g| g.provider.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let until = status
        .active_until
        .map(utc_timestamp)
        .unwrap_or_else(|| "—".into());
    trf(
        "supporter.status.active",
        &[("source", &source), ("until", &until)],
    )
}
/// Gregorian civil date from an epoch day; avoids a platform locale dependency.
pub(crate) fn utc_timestamp(seconds: i64) -> String {
    let days = seconds.div_euclid(86400);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}", // i18n-allow: date format
        seconds.rem_euclid(86400) / 3600,
        seconds.rem_euclid(3600) / 60
    )
}

fn render_panel(
    mut commands: Commands,
    state: Res<SupporterUiState>,
    career: Res<CareerClient>,
    platform: Res<SupporterPlatformState>,
    offer: Res<PurchaseOffer>,
    assets: Res<AuraAssets>,
    roots: Query<Entity, With<SupporterRoot>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    locale: Option<Res<Locale>>,
    game: Option<Res<GameStateSnapshot>>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    ui_platform: Res<crate::ui::UiPlatform>,
    mut prior: Local<String>,
) {
    let compact = windows.single().is_ok_and(|w| w.height() < 540.);
    let in_match = game
        .as_ref()
        .is_some_and(|game| matches!(game.state, GameState::Running))
        && !screen.as_ref().is_some_and(|screen| screen.get().is_menu());
    let status = career.view.supporter.as_ref();
    let summary = status_label(status);
    let tier = tier_label(&offer, status);
    let key = format!(
        "{}|{:?}|{}|{:?}|{:?}|{:?}|{}|{:?}|{}|{}|{}",
        state.open,
        state.selected,
        summary,
        tier,
        status.map(|s| (s.equipped_aura, s.tier_points)),
        *platform,
        career.nickname,
        career.view.error,
        compact,
        locale.as_ref().map_or(0, |locale| locale.generation()),
        in_match,
    );
    if *prior == key {
        return;
    }
    *prior = key;
    for root in &roots {
        commands.entity(root).despawn();
    }
    if !state.open {
        return;
    }
    commands
        .spawn((
            SupporterRoot,
            ModalRoot(ModalId::Supporter),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.),
                height: Val::Percent(100.),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                padding: UiRect::all(Val::Px(10.)),
                ..default()
            },
            BackgroundColor(Color::srgba(0., 0., 0., 0.72)),
            GlobalZIndex(1300),
        ))
        .with_children(|root| {
            if !in_match {
                let form = ui::Form::of(ui_platform.is_mobile());
                living_background::spawn(
                    root,
                    LivingScene::Arena,
                    LivingBands {
                        header: Some(if form == ui::Form::Phone { 56.0 } else { 138.0 }),
                        footer: None,
                    },
                    form,
                );
            }
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(8.),
                    right: Val::Px(8.),
                    ..default()
                },
                GlobalZIndex(1301),
            ))
            .with_children(|corner| {
                button(corner, tr("supporter.close").into(), Action::Close, true);
            });
            root.spawn((
                SupporterScroll,
                panel_scroll(),
                Node {
                    width: Val::Px(if compact { 660. } else { 760. }),
                    max_width: Val::Percent(100.),
                    max_height: Val::Percent(96.),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(if compact { 5. } else { 9. }),
                    padding: UiRect::all(Val::Px(if compact { 8. } else { 12. })),
                    overflow: Overflow::scroll_y(),
                    ..ui::panel_node()
                },
                BackgroundColor(ui::PANEL.with_alpha(1.0)),
                BorderColor::all(ui::EDGE),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new(trf("supporter.title", &[("nickname", &career.nickname)])),
                    ui::text(if compact { 16. } else { 20. }),
                    TextColor(ui::GOLD),
                ));
                panel.spawn((
                    Text::new(summary),
                    ui::text(if compact { 12. } else { 14. }),
                    TextColor(ui::IVORY),
                ));
                if let Some(tier) = tier {
                    panel.spawn((
                        Text::new(tier),
                        ui::text(if compact { 11. } else { 13. }),
                        TextColor(ui::GOLD),
                        TestId::new("SupporterTier"),
                    ));
                }
                panel
                    .spawn((Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(12.),
                        flex_wrap: FlexWrap::Wrap,
                        ..default()
                    },))
                    .with_children(|row| {
                        row.spawn((
                            ImageNode::new(assets.preview.clone()),
                            PreviewImage,
                            Node {
                                width: Val::Px(if compact { 170. } else { 320. }),
                                height: Val::Px(if compact { 119. } else { 224. }),
                                ..default()
                            },
                        ));
                        row.spawn((Node {
                            width: Val::Px(330.),
                            flex_direction: FlexDirection::Row,
                            flex_wrap: FlexWrap::Wrap,
                            align_content: AlignContent::Center,
                            column_gap: Val::Px(6.),
                            row_gap: Val::Px(7.),
                            ..default()
                        },))
                            .with_children(|choices| {
                                // Every listed style previews for free; the
                                // server decides what can be worn.
                                for style in listed_styles(&offer, status) {
                                    button(
                                        choices,
                                        if state.selected == style {
                                            trf(
                                                "supporter.aura.preview",
                                                &[("aura", &data::aura(style))],
                                            )
                                        } else if status.is_some_and(|s| s.active)
                                            && !style_unlocked(status, style)
                                        {
                                            trf(
                                                "tier.style.locked",
                                                &[("aura", &data::aura(style))],
                                            )
                                        } else {
                                            data::aura(style).into()
                                        },
                                        Action::Select(style),
                                        true,
                                    );
                                }
                                let active = status.is_some_and(|s| s.active);
                                button(
                                    choices,
                                    tr("supporter.equip").into(),
                                    Action::Equip,
                                    style_unlocked(status, state.selected),
                                );
                                button(
                                    choices,
                                    tr("supporter.hide").into(),
                                    Action::Disable,
                                    active,
                                );
                            });
                    });
                panel.spawn((
                    Text::new(tr("supporter.notice")),
                    ui::text(if compact { 10. } else { 12. }),
                    TextColor(ui::MUTED),
                ));
                if let Some(error) = platform
                    .message
                    .as_ref()
                    .map(PlatformMessage::text)
                    .or(career.view.error.as_deref())
                {
                    panel.spawn((Text::new(error), ui::text(13.), TextColor(ui::GOLD)));
                }
                panel
                    .spawn((Node {
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(8.),
                        row_gap: Val::Px(6.),
                        ..default()
                    },))
                    .with_children(|row| {
                        if offer.surface == PaymentSurface::Website {
                            button(
                                row,
                                tr("tier.purchase.website").into(),
                                Action::Website,
                                true,
                            );
                        } else if offer.surface == PaymentSurface::StoreKit && platform.available {
                            button(
                                row,
                                platform
                                    .price_label
                                    .as_ref()
                                    .map(|price| {
                                        trf("supporter.purchase.price", &[("price", price)])
                                    })
                                    .unwrap_or_else(|| tr("supporter.purchase.apple").into()),
                                Action::Purchase,
                                platform.price_label.is_some()
                                    && !platform.busy
                                    && purchase_allowed(&career),
                            );
                            button(
                                row,
                                tr("supporter.purchase.restore").into(),
                                Action::Restore,
                                !platform.busy,
                            );
                        } else {
                            row.spawn((
                                Text::new(tr("supporter.purchase.unconfigured")),
                                ui::text(if compact { 10. } else { 12. }),
                                TextColor(ui::MUTED),
                            ));
                        }
                        button(row, tr("supporter.refresh").into(), Action::Refresh, true);
                    });
            });
        });
}
/// The supporter panel body: 24 px per wheel notch on every build, touch drag
/// from the first pixel.
fn panel_scroll() -> ScrollArea {
    ScrollArea::wheel(24.0).touch_drag(0.0)
}

fn button(parent: &mut ChildSpawnerCommands, label: String, action: Action, enabled: bool) {
    parent
        .spawn((
            Button,
            Node {
                min_height: Val::Px(38.),
                padding: UiRect::axes(Val::Px(12.), Val::Px(7.)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::all(Val::Px(6.)),
                ..default()
            },
            BackgroundColor(if enabled { ui::TILE } else { ui::PANEL }),
            UiAction(action),
            // A disabled button is never hit or pressed (it used to carry no
            // action at all).
            Pressable {
                disabled: !enabled,
                ..default()
            },
            TestId::new(action.test_id()),
        ))
        .with_child((
            Text::new(label),
            ui::text(13.),
            TextColor(if enabled { ui::IVORY } else { ui::MUTED }),
        ));
}

impl Action {
    fn test_id(self) -> String {
        match self {
            Self::Close => "SupporterClose".into(),
            Self::Select(style) => format!("SupporterAura-{}", style.id()),
            Self::Equip => "SupporterEquip".into(),
            Self::Disable => "SupporterDisable".into(),
            Self::Refresh => "SupporterRefresh".into(),
            Self::Purchase => "SupporterPurchase".into(),
            Self::Restore => "SupporterRestore".into(),
            Self::Website => "SupporterWebsite".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supporter_panel_scrolls_by_wheel_and_drag_while_it_is_the_top_modal() {
        use crate::platform::UiProfile;
        use crate::ui::scroll::harness;
        for profile in [UiProfile::Desktop, UiProfile::Mobile] {
            let mut app = App::new();
            let window = harness::install(&mut app, profile);
            let mut stack = crate::ui::ModalStack::default();
            stack.push(ModalId::Supporter);
            app.insert_resource(stack);
            let root = app
                .world_mut()
                .spawn((SupporterRoot, ModalRoot(ModalId::Supporter)))
                .id();
            let panel = app
                .world_mut()
                .spawn((SupporterScroll, panel_scroll(), ChildOf(root)))
                .id();
            let center = Vec2::new(400.0, 300.0);
            harness::measure(&mut app, panel, center);
            harness::wheel_lines(&mut app, window, -1.0);
            assert_eq!(harness::offset(&app, panel), 24.0, "{profile:?}");
            harness::drag(&mut app, window, 1, center, 30.0);
            assert_eq!(harness::offset(&app, panel), 54.0, "{profile:?}");
            // Covered by another modal: the panel keeps its offset.
            app.world_mut()
                .resource_mut::<crate::ui::ModalStack>()
                .pop(ModalId::Supporter);
            app.world_mut()
                .resource_mut::<crate::ui::ModalStack>()
                .push(ModalId::Pause);
            harness::wheel_lines(&mut app, window, -1.0);
            assert_eq!(harness::offset(&app, panel), 54.0, "{profile:?}");
        }
    }
    #[test]
    fn orbital_paths_are_continuous_bounded_and_time_based() {
        for i in 0..ORBIT_COUNT {
            for tick in 0..1200 {
                let t = tick as f64 / 60.;
                let p = orbital_point(i as f32, t);
                assert!(p.is_finite());
                assert!(p.y > 0.08 && p.y < 1.97);
                assert!(p.x.abs() <= 1. && p.z.abs() <= 1.);
                assert!(p.distance(orbital_point(i as f32, t + 1. / 60.)) < 0.025);
            }
        }
        // Three distinct planes, not three lights on the same horizontal ring.
        assert!(orbital_point(0., 0.).distance(orbital_point(1., 0.)) > 0.4);
        let n = |i: f32| orbit_rotation(i, 0.) * Vec3::Z;
        assert!(n(0.).dot(n(1.)).abs() < 0.9);
    }

    #[test]
    fn trails_taper_and_flat_particles_cross_in_front_of_and_behind_actor() {
        for flat in [false, true] {
            let mut front = false;
            let mut back = false;
            for tick in 0..120 {
                let t = tick as f64 / 30.;
                for particle in [
                    AuraParticle::Mantle,
                    AuraParticle::Radiance,
                    AuraParticle::Core,
                    AuraParticle::Halo,
                    AuraParticle::Trail(0),
                    AuraParticle::Trail(TRAIL_SAMPLES - 1),
                    AuraParticle::Glint,
                ] {
                    let pose = particle_pose(
                        &Orbit {
                            index: 0.,
                            flat,
                            particle,
                            preview: false,
                            sparkle: 1.,
                        },
                        t,
                    );
                    assert!(
                        pose.translation.is_finite()
                            && pose.scale.is_finite()
                            && pose.rotation.is_finite()
                    );
                    assert!(pose.scale.min_element() >= 0.);
                    if flat {
                        front |= pose.translation.z > 10.;
                        back |= pose.translation.z < 1.;
                    }
                }
            }
            if flat {
                assert!(front && back);
            }
            let first = particle_pose(
                &Orbit {
                    index: 0.,
                    flat,
                    particle: AuraParticle::Trail(0),
                    preview: false,
                    sparkle: 1.,
                },
                2.,
            );
            let last = particle_pose(
                &Orbit {
                    index: 0.,
                    flat,
                    particle: AuraParticle::Trail(TRAIL_SAMPLES - 1),
                    preview: false,
                    sparkle: 1.,
                },
                2.,
            );
            assert!(last.scale.y < first.scale.y * 0.2);
        }
    }

    #[test]
    fn ribbons_join_their_heads_and_have_finite_tapered_geometry() {
        use bevy::mesh::VertexAttributeValues;
        for width in [0.018, 0.075] {
            let mesh = ribbon_mesh(width);
            let Some(VertexAttributeValues::Float32x3(points)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!("positions")
            };
            let Some(VertexAttributeValues::Float32x4(colors)) =
                mesh.attribute(Mesh::ATTRIBUTE_COLOR)
            else {
                panic!("colors")
            };
            assert_eq!(mesh.indices().unwrap().len() / 3, 256);
            assert_eq!(points.len(), 198);
            for (p, c) in points.iter().zip(colors) {
                assert!(Vec3::from(*p).is_finite());
                assert!(Vec3::from(*p).length() <= AURA_RADIUS + width + 0.001);
                assert!((0. ..=1.).contains(&c[3]));
            }
            for sheet in 0..2 {
                let base = sheet * (RIBBON_SEGMENTS + 1) * 3;
                for step in 0..RIBBON_SEGMENTS {
                    assert!(colors[base + step * 3 + 1][3] > colors[base + (step + 1) * 3 + 1][3]);
                    assert_eq!(colors[base + step * 3][3], 0.);
                    assert_eq!(colors[base + step * 3 + 2][3], 0.);
                }
                assert_eq!(colors[base + RIBBON_SEGMENTS * 3 + 1][3], 0.);
            }
            for i in 0..ORBIT_COUNT {
                for seconds in [0., 0.1, 2., 23., 1_000_000.] {
                    let orbit = Orbit {
                        index: i as f32,
                        flat: false,
                        particle: AuraParticle::RibbonCore,
                        preview: false,
                        sparkle: 1.,
                    };
                    let pose = particle_pose(&orbit, seconds);
                    let tip = pose.transform_point(Vec3::from(points[1]));
                    assert!(tip.distance(orbital_point(i as f32, seconds)) < 0.0001);
                }
            }
        }
    }

    #[test]
    fn sustained_orbits_share_assets_and_reduce_actual_mesh_work() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<ColorMaterial>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Time>()
            .init_resource::<AuraRegistry>()
            .insert_resource(PlayerVisualMode::Models3d)
            .add_systems(Startup, setup)
            .add_systems(Update, (sync_auras, animate_orbits).chain());
        for _ in 0..10 {
            app.world_mut()
                .spawn((RemotePlayer, NetworkSupporterAura(Some(AuraStyle::Solar))));
        }
        app.update();
        let counts = |world: &World| {
            (
                world.entities().len(),
                world.resource::<Assets<Mesh>>().len(),
                world.resource::<Assets<StandardMaterial>>().len(),
                world.resource::<Assets<ColorMaterial>>().len(),
                world.resource::<Assets<Image>>().len(),
            )
        };
        let before = counts(app.world());
        let meshes = app.world().resource::<Assets<Mesh>>();
        let assets = app.world().resource::<AuraAssets>();
        let triangles =
            |handle: &Handle<Mesh>| meshes.get(handle).unwrap().indices().unwrap().len() / 3;
        let old_sphere = Mesh::from(Sphere::new(0.065));
        let old_trails = old_sphere.indices().unwrap().len() / 3 * 18;
        let new_trails =
            (triangles(&assets.ribbon_core) + triangles(&assets.ribbon_glow)) * ORBIT_COUNT;
        let old_total = old_trails + triangles(&assets.ring) + 16 * 2;
        let new_total = new_trails + triangles(&assets.ring) + 16 * 2;
        assert_eq!(new_trails, 1536);
        assert!(new_total * 4 < old_total);
        assert_eq!(HERO_AURA_ELEMENT_BUDGET, 23);
        assert_eq!(FLAT_AURA_ELEMENT_BUDGET, 35);
        println!(
            "HERO_MESH_COST {{\"old_triangles_per_hero\":{old_total},\"new_triangles_per_hero\":{new_total},\"old_elements\":35,\"new_elements\":23,\"actors\":10,\"frames\":600,\"assets\":{before:?}}}"
        );
        for _ in 0..600 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs_f64(1. / 60.));
            app.update();
        }
        assert_eq!(
            counts(app.world()),
            before,
            "no steady-state assets or entities allocated"
        );
    }

    #[test]
    fn hidden_and_dead_actors_never_have_visible_aura() {
        let mut stats = CombatStats::default();
        assert!(actor_visible(
            &stats,
            &Visibility::Inherited,
            &InheritedVisibility::VISIBLE
        ));
        assert!(!actor_visible(
            &stats,
            &Visibility::Hidden,
            &InheritedVisibility::VISIBLE
        ));
        assert!(!actor_visible(
            &stats,
            &Visibility::Inherited,
            &InheritedVisibility::HIDDEN
        ));
        stats.hp = 0.;
        assert!(!actor_visible(
            &stats,
            &Visibility::Visible,
            &InheritedVisibility::VISIBLE
        ));
    }
    #[test]
    fn hero_glows_preserve_transparency_and_rising_sparks_fade_at_wrap() {
        for style in AuraStyle::ALL {
            for glow in [palette(style, 0.).mantle, palette(style, 7.5).radiance] {
                let material = particle_material(glow);
                assert!(material.unlit);
                assert_eq!(material.alpha_mode, AlphaMode::Add);
                let tint = material.base_color.to_linear();
                assert!(tint.red.max(tint.green).max(tint.blue) > 1., "{style:?}");
                assert!(material.base_color.alpha() < 0.5);
            }
        }
        for flat in [false, true] {
            for particle in [AuraParticle::Mantle, AuraParticle::Radiance] {
                for tick in 0..600 {
                    let pose = particle_pose(
                        &Orbit {
                            index: 0.,
                            flat,
                            particle,
                            preview: false,
                            sparkle: 1.,
                        },
                        tick as f64 / 60.,
                    );
                    // A soft backdrop, under three world units tall; never a
                    // changing model scale or an opaque layer over a 2D actor.
                    assert!(pose.scale.y * 0.13 < 3.);
                    if flat {
                        assert!(pose.translation.z < 1.);
                    }
                }
            }
            let spark = Orbit {
                index: 0.,
                flat,
                particle: AuraParticle::Glint,
                preview: false,
                sparkle: 1.,
            };
            let lifetime = 1. / 0.38;
            assert_eq!(particle_pose(&spark, 0.).scale, Vec3::ZERO);
            assert!(particle_pose(&spark, lifetime - 0.0001).scale.max_element() < 0.0001);
            assert!(particle_pose(&spark, lifetime + 0.0001).scale.max_element() < 0.0001);
            assert!(
                particle_pose(&spark, lifetime * 0.75).translation.y
                    > particle_pose(&spark, lifetime * 0.25).translation.y
            );
        }
    }
    #[test]
    fn aura_lifecycle_tracks_server_style_and_despawns_with_owner_in_both_modes() {
        for (mode, remote) in [PlayerVisualMode::Models3d, PlayerVisualMode::Sprite2d]
            .into_iter()
            .flat_map(|mode| [(mode, false), (mode, true)])
        {
            let mut app = App::new();
            app.insert_resource(mode)
                .init_resource::<AuraRegistry>()
                .init_resource::<AuraAssets>()
                .add_systems(Update, sync_auras);
            let actor = app
                .world_mut()
                .spawn((Player, NetworkSupporterAura(None)))
                .id();
            if remote {
                app.world_mut()
                    .entity_mut(actor)
                    .remove::<Player>()
                    .insert(RemotePlayer);
            }
            app.update();
            assert!(app.world().resource::<AuraRegistry>().0.is_empty());
            app.world_mut()
                .get_mut::<NetworkSupporterAura>(actor)
                .unwrap()
                .0 = Some(AuraStyle::Solar);
            app.update();
            let first = app.world().resource::<AuraRegistry>().0[&actor].0;
            let children = app
                .world()
                .get::<Children>(first)
                .unwrap()
                .iter()
                .collect::<Vec<_>>();
            let budget = aura_element_budget(mode == PlayerVisualMode::Sprite2d);
            assert_eq!(children.len(), budget);
            assert!(budget <= 35, "Hero stays within the previous aura budget");
            for _ in 0..120 {
                app.update();
                assert_eq!(app.world().resource::<AuraRegistry>().0[&actor].0, first);
                assert_eq!(app.world().get::<Children>(first).unwrap().len(), budget);
            }
            app.world_mut()
                .get_mut::<NetworkSupporterAura>(actor)
                .unwrap()
                .0 = Some(AuraStyle::Lunar);
            app.update();
            assert!(app.world().get_entity(first).is_err());
            let second = app.world().resource::<AuraRegistry>().0[&actor].0;
            // Expiry, revocation and unequip all replicate None. Every mesh must
            // disappear while the actor itself remains alive.
            app.world_mut()
                .get_mut::<NetworkSupporterAura>(actor)
                .unwrap()
                .0 = None;
            app.update();
            assert!(app.world().get_entity(second).is_err());
            assert!(app.world().resource::<AuraRegistry>().0.is_empty());
            assert_eq!(
                app.world_mut().query::<&Orbit>().iter(app.world()).count(),
                0
            );
            app.world_mut()
                .get_mut::<NetworkSupporterAura>(actor)
                .unwrap()
                .0 = Some(AuraStyle::Verdant);
            app.update();
            let third = app.world().resource::<AuraRegistry>().0[&actor].0;
            app.world_mut().despawn(actor);
            app.update();
            assert!(app.world().get_entity(third).is_err());
            assert!(app.world().resource::<AuraRegistry>().0.is_empty());
            for child in children {
                assert!(app.world().get_entity(child).is_err());
            }
        }
    }
    #[test]
    fn live_aura_follows_current_ground_pose_and_hides_children_without_a_frame_leak() {
        for (mode, remote) in [PlayerVisualMode::Models3d, PlayerVisualMode::Sprite2d]
            .into_iter()
            .flat_map(|mode| [(mode, false), (mode, true)])
        {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, TransformPlugin))
                .insert_resource(mode)
                .init_resource::<AuraRegistry>()
                .init_resource::<crate::maps::MapLayout>()
                .init_resource::<AuraAssets>()
                .add_systems(Update, sync_auras)
                .add_systems(
                    PostUpdate,
                    (follow_actors, animate_orbits)
                        .chain()
                        .after(bevy::transform::TransformSystems::Propagate),
                );
            let actor = app
                .world_mut()
                .spawn((
                    Player,
                    NetworkSupporterAura(Some(AuraStyle::Solar)),
                    CombatStats::default(),
                    Transform::from_xyz(3., 0., 4.),
                    Visibility::Inherited,
                    InheritedVisibility::VISIBLE,
                ))
                .id();
            if remote {
                app.world_mut()
                    .entity_mut(actor)
                    .remove::<Player>()
                    .insert(RemotePlayer);
            }
            app.update();
            let aura = app.world().resource::<AuraRegistry>().0[&actor].0;
            assert!(app.world().get::<InheritedVisibility>(aura).unwrap().get());
            let before = app
                .world()
                .get::<GlobalTransform>(aura)
                .unwrap()
                .translation();
            let mut pose = app.world_mut().get_mut::<Transform>(actor).unwrap();
            pose.translation.x += 2.;
            pose.rotation = Quat::from_rotation_y(2.);
            app.update();
            let after = app
                .world()
                .get::<GlobalTransform>(aura)
                .unwrap()
                .translation();
            assert!((after.x - before.x - 2.).abs() < 0.0001);
            assert_eq!(
                app.world().get::<Transform>(aura).unwrap().rotation,
                Quat::IDENTITY
            );
            *app.world_mut()
                .get_mut::<InheritedVisibility>(actor)
                .unwrap() = InheritedVisibility::HIDDEN;
            app.update();
            assert!(!app.world().get::<InheritedVisibility>(aura).unwrap().get());
            for child in app.world().get::<Children>(aura).unwrap().iter() {
                assert!(!app.world().get::<InheritedVisibility>(child).unwrap().get());
            }
            *app.world_mut()
                .get_mut::<InheritedVisibility>(actor)
                .unwrap() = InheritedVisibility::VISIBLE;
            app.world_mut().get_mut::<CombatStats>(actor).unwrap().hp = 0.;
            app.update();
            assert!(!app.world().get::<InheritedVisibility>(aura).unwrap().get());
        }
    }
    #[test]
    fn purchase_requires_known_inactive_authenticated_profile_and_never_stacks_memberships() {
        let mut career = CareerClient::default();
        assert!(!purchase_allowed(&career));
        career.view.profile = Some(shared::career::ProfileSummary::new(
            "a".repeat(64),
            "Tester".into(),
        ));
        career.view.auth_nonce = Some("b".repeat(64));
        career.view.storage_enabled = true;
        assert!(!purchase_allowed(&career));
        career.view.supporter = Some(SupporterStatus::default());
        assert!(purchase_allowed(&career));
        career.view.supporter.as_mut().unwrap().grants.push(
            shared::supporter::SupporterGrantSummary {
                provider: "solana".into(),
                valid_from: 200,
                valid_until: 300,
                revoked: false,
                renewal_enabled: Some(false),
                tier_points: 0,
            },
        );
        assert!(!purchase_allowed_at(&career, 100));
        assert!(purchase_allowed_at(&career, 300));
        career.view.supporter.as_mut().unwrap().grants.clear();
        career.view.supporter.as_mut().unwrap().active = true;
        assert!(!purchase_allowed(&career));
        career.view.supporter.as_mut().unwrap().active = false;
        career.view.error = Some("Unavailable".into());
        assert!(!purchase_allowed(&career));
    }
    #[test]
    fn timestamp_and_empty_status_are_explicit() {
        assert_eq!(utc_timestamp(0), "1970-01-01 00:00");
        assert_eq!(utc_timestamp(1789516800), "2026-09-16 00:00");
        assert!(status_label(Some(&SupporterStatus::default())).contains("only visible here"));
    }
    /// Client copy (status line, store messages, aura names) follows the
    /// language; StoreKit's own text is shown as sent.
    #[test]
    fn supporter_copy_follows_the_language() {
        if crate::i18n::testing::isolated("supporter::tests::supporter_copy_follows_the_language") {
            return;
        }
        use crate::i18n::{I18nPlugin, Locale, LocaleId};
        let mut app = App::new();
        app.add_plugins(I18nPlugin::default());
        let loading = PlatformMessage::Key("supporter.store.loading");
        let native = PlatformMessage::Text("Purchase pending.".into());
        assert_eq!(loading.text(), "Loading App Store pricing…");
        assert_eq!(
            status_label(None),
            "Connect your saved account to view Hero access."
        );
        app.world_mut()
            .resource_mut::<Locale>()
            .set(LocaleId::parse("zh-Hans").unwrap());
        assert_eq!(loading.text(), "正在加载 App Store 价格…");
        assert_eq!(native.text(), "Purchase pending.");
        assert_eq!(status_label(None), "连接已保存的账号即可查看 Hero 状态。");
        let active = SupporterStatus {
            active: true,
            active_until: Some(0),
            ..default()
        };
        assert_eq!(
            status_label(Some(&active)),
            "Hero 已激活 · \n外观有效期至 1970-01-01 00:00 UTC"
        );
        assert_ne!(data::aura(AuraStyle::Solar), AuraStyle::Solar.label());
        // The tier line and the website button in English and Russian.
        let grant = |until: i64, points: u32| shared::supporter::SupporterGrantSummary {
            provider: "yookassa".into(),
            valid_from: 0,
            valid_until: until,
            revoked: false,
            renewal_enabled: Some(false),
            tier_points: points,
        };
        let crimson = SupporterStatus::from_grants(
            vec![grant(86_400, 900), grant(172_800, 300)],
            100,
            1200,
            None,
        );
        let ru = PurchaseOffer {
            surface: PaymentSurface::Website,
            tiers: true,
        };
        app.world_mut()
            .resource_mut::<Locale>()
            .set(LocaleId::parse("en").unwrap());
        assert_eq!(
            tier_label(&ru, Some(&crimson)).unwrap(),
            "Hero tier Crimson until 1970-01-02 00:00 UTC, then Jade."
        );
        assert_eq!(tr("tier.purchase.website"), "Support on the website");
        app.world_mut()
            .resource_mut::<Locale>()
            .set(LocaleId::parse("ru").unwrap());
        assert_eq!(
            tier_label(&ru, Some(&crimson)).unwrap(),
            "Уровень Hero «Багрянец» до 1970-01-02 00:00 UTC, затем «Нефрит»."
        );
        assert_eq!(tr("tier.purchase.website"), "Поддержать на сайте");
        assert_eq!(data::aura(AuraStyle::PrismIII), "Призма III");
    }

    #[test]
    fn preview_cannot_grant_aura_or_change_combat_stats() {
        let mut app = App::new();
        app.init_resource::<SupporterUiState>()
            .init_resource::<AuraAssets>()
            .add_systems(Update, sync_preview);
        let actor = app
            .world_mut()
            .spawn((CombatStats::default(), NetworkSupporterAura(None)))
            .id();
        app.world_mut().resource_mut::<SupporterUiState>().open = true;
        app.world_mut().resource_mut::<SupporterUiState>().selected = AuraStyle::Verdant;
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&PreviewAura>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world_mut()
                .query::<&AuraOwner>()
                .iter(app.world())
                .count(),
            0
        );
        assert_eq!(
            app.world_mut().query::<&Orbit>().iter(app.world()).count(),
            HERO_AURA_ELEMENT_BUDGET - 1
        );
        assert_eq!(
            app.world().get::<NetworkSupporterAura>(actor).unwrap().0,
            None
        );
        assert_eq!(
            app.world().get::<CombatStats>(actor).unwrap().hp,
            CombatStats::default().hp
        );
        app.world_mut().resource_mut::<SupporterUiState>().open = false;
        app.update();
        assert_eq!(
            app.world_mut().query::<&Orbit>().iter(app.world()).count(),
            0
        );
    }

    #[test]
    fn supporter_presses_apply_once_and_disabled_buttons_do_nothing() {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.init_resource::<SupporterUiState>()
            .init_resource::<SupporterPlatformState>()
            .init_resource::<PurchaseOffer>()
            .init_resource::<SupporterWebsite>()
            .init_resource::<CareerClient>()
            .add_message::<SupporterPlatformAction>()
            .add_message::<NetworkCommand>()
            .add_ui_action::<Action>()
            .add_systems(Update, actions.after(UiSet::Dispatch));
        harness::spawn_ui(app.world_mut(), |row| {
            button(row, "Refresh".into(), Action::Refresh, true);
            button(row, "Hide my aura".into(), Action::Disable, false);
            button(row, "Close".into(), Action::Close, true);
        });
        app.world_mut().resource_mut::<SupporterUiState>().open = true;
        app.update();
        let requests = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count()
        };
        harness::press(app.world_mut(), "SupporterRefresh");
        app.update();
        assert_eq!(requests(&mut app), 1);
        app.update();
        assert_eq!(requests(&mut app), 0, "a press fires once");
        harness::drain_actions::<Action>(app.world_mut());
        // Rendered disabled (inactive membership): never pressed.
        harness::press(app.world_mut(), "SupporterDisable");
        app.update();
        assert_eq!(requests(&mut app), 0);
        assert!(harness::drain_actions::<Action>(app.world_mut()).is_empty());
        harness::press(app.world_mut(), "SupporterClose");
        app.update();
        assert!(!app.world().resource::<SupporterUiState>().open);
    }

    fn tier_status(points: u32, equipped: AuraStyle) -> SupporterStatus {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        SupporterStatus::from_grants(
            vec![shared::supporter::SupporterGrantSummary {
                provider: "yookassa".into(),
                valid_from: now - 60,
                valid_until: now + 86_400,
                revoked: false,
                renewal_enabled: Some(false),
                tier_points: points,
            }],
            now,
            points,
            Some(Some(equipped)),
        )
    }

    #[test]
    fn the_purchase_surface_follows_the_market_and_ios_never_links_to_the_web() {
        use shared::market::Market;
        let ru = Market::Ru.payment_providers();
        let global = Market::Global.payment_providers();
        assert_eq!(payment_surface_for(ru, false), PaymentSurface::Website);
        assert_eq!(payment_surface_for(ru, true), PaymentSurface::None);
        assert_eq!(payment_surface_for(global, true), PaymentSurface::StoreKit);
        // Global desktop and Android are unchanged: no in-game purchase link.
        assert_eq!(payment_surface_for(global, false), PaymentSurface::None);
        assert_eq!(payment_surface_for(&[], false), PaymentSurface::None);
        assert_eq!(payment_surface_for(&[], true), PaymentSurface::None);
        assert_eq!(
            payment_surface(),
            payment_surface_for(
                crate::account_region::payment_providers(),
                cfg!(target_os = "ios")
            )
        );
    }

    #[test]
    fn listed_styles_and_the_tier_line_follow_what_the_market_sells() {
        let ru = PurchaseOffer {
            surface: PaymentSurface::Website,
            tiers: true,
        };
        let global = PurchaseOffer {
            surface: PaymentSurface::None,
            tiers: false,
        };
        assert_eq!(listed_styles(&ru, None), AuraStyle::ALL.to_vec());
        assert_eq!(listed_styles(&global, None), AuraStyle::BASE.to_vec());
        assert!(tier_label(&global, None).is_none());
        assert!(tier_label(&ru, None).is_some());
        let crimson = tier_status(1000, AuraStyle::Crimson);
        // An unlocked tier style shows even where tiers are not sold.
        assert_eq!(
            listed_styles(&global, Some(&crimson)),
            [
                AuraStyle::Solar,
                AuraStyle::Lunar,
                AuraStyle::Verdant,
                AuraStyle::Rose,
                AuraStyle::Jade,
                AuraStyle::Azure,
                AuraStyle::Crimson
            ]
        );
        assert!(tier_label(&ru, Some(&crimson)).is_some());
        assert!(style_unlocked(Some(&crimson), AuraStyle::Crimson));
        assert!(!style_unlocked(Some(&crimson), AuraStyle::PrismI));
        assert!(style_unlocked(Some(&crimson), AuraStyle::Lunar));
        assert!(!style_unlocked(None, AuraStyle::Solar));
    }

    static OPENED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    fn panel_app(offer: PurchaseOffer, status: Option<SupporterStatus>) -> App {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.init_resource::<SupporterUiState>()
            .init_resource::<SupporterPlatformState>()
            .init_resource::<CareerClient>()
            .init_resource::<AuraAssets>()
            .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
            .insert_resource(offer)
            .insert_resource(SupporterWebsite {
                url: || Ok("https://portal.test/supporter".into()),
                open: |url| {
                    OPENED.lock().unwrap().push(url.into());
                    Ok(())
                },
            })
            .add_message::<SupporterPlatformAction>()
            .add_message::<NetworkCommand>()
            .add_ui_action::<Action>()
            .add_systems(
                Update,
                (actions.after(UiSet::Dispatch), render_panel.after(actions)),
            );
        app.world_mut()
            .resource_mut::<CareerClient>()
            .view
            .supporter = status;
        app.world_mut().resource_mut::<SupporterUiState>().open = true;
        app.update();
        app
    }

    /// RU UI tree: the panel reads the purchase surface (website, no StoreKit
    /// button), lists the tier styles, gates Equip by the tier and opens the
    /// portal's Supporter page.
    #[test]
    fn ru_panel_offers_the_website_and_equips_only_unlocked_tier_styles() {
        use crate::ui::test_id::harness;
        let mut app = panel_app(
            PurchaseOffer {
                surface: PaymentSurface::Website,
                tiers: true,
            },
            Some(tier_status(1000, AuraStyle::Crimson)),
        );
        let world = app.world_mut();
        for style in AuraStyle::ALL {
            assert!(
                harness::find(world, &format!("SupporterAura-{}", style.id())).is_some(),
                "{style:?}"
            );
        }
        assert!(harness::find(world, "SupporterWebsite").is_some());
        assert!(harness::find(world, "SupporterPurchase").is_none());
        assert!(harness::find(world, "SupporterRestore").is_none());
        assert!(harness::find(world, "SupporterTier").is_some());
        let requests = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .filter_map(|command| match command {
                    NetworkCommand::Career(CareerRequest::EquipSupporterAura { aura, .. }) => aura,
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        // Prism I previews but cannot be equipped with 1000 points.
        harness::press(app.world_mut(), "SupporterAura-prism_i");
        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<SupporterUiState>().selected,
            AuraStyle::PrismI
        );
        harness::press(app.world_mut(), "SupporterEquip");
        app.update();
        assert!(requests(&mut app).is_empty());
        harness::press(app.world_mut(), "SupporterAura-crimson");
        app.update();
        app.update();
        harness::press(app.world_mut(), "SupporterEquip");
        app.update();
        assert_eq!(requests(&mut app), [AuraStyle::Crimson]);
        OPENED.lock().unwrap().clear();
        harness::press(app.world_mut(), "SupporterWebsite");
        app.update();
        assert_eq!(*OPENED.lock().unwrap(), ["https://portal.test/supporter"]);
        assert_eq!(
            app.world().resource::<SupporterPlatformState>().message,
            Some(PlatformMessage::Key("tier.website.opened"))
        );
    }

    /// A build without a web surface (global desktop, or any iOS build without
    /// StoreKit) shows no purchase link and, global, only the base styles.
    #[test]
    fn panels_without_a_web_surface_show_no_link() {
        use crate::ui::test_id::harness;
        let mut app = panel_app(
            PurchaseOffer {
                surface: PaymentSurface::None,
                tiers: false,
            },
            Some(SupporterStatus::default()),
        );
        let world = app.world_mut();
        assert!(harness::find(world, "SupporterWebsite").is_none());
        assert!(harness::find(world, "SupporterPurchase").is_none());
        assert!(harness::find(world, "SupporterTier").is_none());
        assert!(harness::find(world, "SupporterAura-solar").is_some());
        assert!(harness::find(world, "SupporterAura-rose").is_none());
        // An iOS RU build: tiers are listed but nothing links to the web.
        let mut app = panel_app(
            PurchaseOffer {
                surface: PaymentSurface::None,
                tiers: true,
            },
            None,
        );
        let world = app.world_mut();
        assert!(harness::find(world, "SupporterWebsite").is_none());
        assert!(harness::find(world, "SupporterAura-prism_iii").is_some());
        harness::press(world, "SupporterRefresh");
    }

    /// Other players' tier auras render within the element budget (the Prism
    /// III crown replaces one spark), and every wearer of a Prism style shares
    /// its materials, which change colour over time while solid tiers do not.
    #[test]
    fn tier_auras_render_on_other_players_and_prisms_shimmer_in_sync() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<ColorMaterial>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Time>()
            .init_resource::<AuraRegistry>()
            .init_resource::<SupporterUiState>()
            .insert_resource(PlayerVisualMode::Models3d)
            .add_systems(Startup, setup)
            .add_systems(Update, (sync_auras, shimmer_prisms, animate_orbits).chain());
        let wearers: Vec<Entity> = [
            AuraStyle::PrismII,
            AuraStyle::PrismII,
            AuraStyle::PrismIII,
            AuraStyle::Crimson,
            AuraStyle::Rose,
        ]
        .into_iter()
        .map(|style| {
            app.world_mut()
                .spawn((RemotePlayer, NetworkSupporterAura(Some(style))))
                .id()
        })
        .collect();
        app.update();
        let roots: Vec<Entity> = wearers
            .iter()
            .map(|actor| app.world().resource::<AuraRegistry>().0[actor].0)
            .collect();
        for root in &roots {
            assert_eq!(
                app.world().get::<Children>(*root).unwrap().len(),
                HERO_AURA_ELEMENT_BUDGET
            );
        }
        let crowns = |app: &mut App| {
            app.world_mut()
                .query::<&Orbit>()
                .iter(app.world())
                .filter(|orbit| matches!(orbit.particle, AuraParticle::Crown))
                .count()
        };
        assert_eq!(crowns(&mut app), 1, "only Prism III wears the crown");
        let core = |app: &mut App, root: Entity| {
            let children: Vec<Entity> = app.world().get::<Children>(root).unwrap().to_vec();
            children
                .into_iter()
                .find(|child| {
                    app.world()
                        .get::<Orbit>(*child)
                        .is_some_and(|orbit| matches!(orbit.particle, AuraParticle::Core))
                })
                .and_then(|child| app.world().get::<MeshMaterial3d<StandardMaterial>>(child))
                .unwrap()
                .0
                .clone()
        };
        let shared = core(&mut app, roots[0]);
        assert_eq!(shared, core(&mut app, roots[1]), "one material per style");
        let crimson = core(&mut app, roots[3]);
        let colour = |app: &App, handle: &Handle<StandardMaterial>| {
            app.world()
                .resource::<Assets<StandardMaterial>>()
                .get(handle)
                .unwrap()
                .base_color
        };
        let before = (colour(&app, &shared), colour(&app, &crimson));
        let assets_before = app.world().resource::<Assets<StandardMaterial>>().len();
        for _ in 0..30 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs_f64(1. / 60.));
            app.update();
        }
        assert_ne!(colour(&app, &shared), before.0, "Prism II shimmers");
        assert_eq!(colour(&app, &crimson), before.1, "Crimson stays red");
        assert_eq!(
            app.world().resource::<Assets<StandardMaterial>>().len(),
            assets_before
        );
        // The 2D fallback has its own budget and keeps the crown inside it.
        *app.world_mut().resource_mut::<PlayerVisualMode>() = PlayerVisualMode::Sprite2d;
        app.update();
        let flat_root = app.world().resource::<AuraRegistry>().0[&wearers[2]].0;
        assert_eq!(
            app.world().get::<Children>(flat_root).unwrap().len(),
            FLAT_AURA_ELEMENT_BUDGET
        );
        assert_eq!(crowns(&mut app), 1);
    }

    #[test]
    fn every_style_has_a_distinct_palette_and_prism_tiers_grow_in_intensity() {
        let solid: Vec<Color> = AuraStyle::ALL
            .iter()
            .filter(|style| prism(**style).is_none())
            .map(|style| color(*style))
            .collect();
        for (i, a) in solid.iter().enumerate() {
            for b in &solid[i + 1..] {
                assert_ne!(a, b);
            }
        }
        let tiers = [AuraStyle::PrismI, AuraStyle::PrismII, AuraStyle::PrismIII];
        for pair in tiers.windows(2) {
            let (low, high) = (prism(pair[0]).unwrap(), prism(pair[1]).unwrap());
            assert!(high.0 > low.0 && high.1 > low.1 && high.2 >= low.2);
        }
        // Three hues at once: orbits, heads and the body glow differ.
        let p = palette(AuraStyle::PrismI, 3.);
        assert_ne!(p.flat_trail.with_alpha(1.), p.flat_halo.with_alpha(1.));
        assert_eq!(sparkle(AuraStyle::Solar), 1.);
        assert!(sparkle(AuraStyle::PrismIII) > sparkle(AuraStyle::PrismI));
    }
}
