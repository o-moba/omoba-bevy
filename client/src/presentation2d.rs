//! True-2D presentation proxies for non-player actors and bounded combat VFX.

use bevy::prelude::*;
use serde::Deserialize;
use shared::{
    PlayerActionKind,
    combat::{CombatEntityKind, MinionKind},
};
use std::collections::{HashMap, VecDeque};

use crate::bosses::BossVisual;
use crate::combat::CombatStats;
use crate::combat_visuals::{
    CombatVisualProfile, CombatVisualRegistry, ProjectilePresentationRoot, ProjectileShape,
};
use crate::maps::MapLayout;
use crate::net::{
    MinionBrainState, NetworkAvatar, NetworkHeroClass, NetworkMinion, NetworkMinionAction,
    NetworkMinionBrainState, NetworkMinionKind, NetworkNeutral, NetworkPlayerId, NetworkProjectile,
    NetworkSpriteCharacter, NetworkStructure, NeutralAiState, NeutralAiStateTag,
    PlayerCosmeticAction, RemotePlayer, StructureKind,
};
use crate::player::Player;
use crate::sprite::PlayerVisualMode;
use crate::team::Team;
use crate::world2d::{
    TRANSIENT_VFX_BUDGET, TRANSIENT_VFX_MAX_LIFETIME, layer, simulation_xz_to_render_xy, y_sorted_z,
};

const PRESENTATION_MANIFEST: &str = include_str!("../assets/presentation2d/manifest.json");

#[derive(Debug, Deserialize)]
struct PresentationManifest {
    schema_version: u32,
    actors_sheet: String,
    actors_grid: [u32; 2],
    effects_sheet: String,
    effects_grid: [u32; 2],
    frame_size: [u32; 2],
    #[allow(dead_code)]
    arena_texture: String,
    #[allow(dead_code)]
    ui_frame: String,
    #[allow(dead_code)]
    portraits: String,
    #[allow(dead_code)]
    portraits_grid: [u32; 2],
    #[allow(dead_code)]
    portrait_character_ids: Vec<String>,
    actors: HashMap<String, ActorDefinition>,
    effects: HashMap<String, EffectDefinition>,
}

#[derive(Clone, Debug, Deserialize)]
struct ActorDefinition {
    frame: usize,
    world_height: f32,
    pivot: [f32; 2],
    /// Alpha>=16 occupied bounds `[x, y, width, height]` inside the 256px cell.
    /// Runtime sizing and tests use this to reason about visible pixels rather
    /// than the transparent atlas rectangle.
    occupied_bounds: [u32; 4],
}

#[derive(Clone, Debug, Deserialize)]
struct EffectDefinition {
    start: usize,
    count: usize,
    fps: f32,
    world_height: f32,
}

#[derive(Resource, Default)]
struct Presentation2dAssets {
    actor_image: Handle<Image>,
    actor_layout: Handle<TextureAtlasLayout>,
    effect_image: Handle<Image>,
    effect_layout: Handle<TextureAtlasLayout>,
    actors: HashMap<String, ActorDefinition>,
    effects: HashMap<String, EffectDefinition>,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum PresentationActorKind {
    Structure,
    Minion,
    Neutral,
    Boss,
    Projectile,
}

#[derive(Component)]
struct PresentationActorRoot;

#[derive(Resource, Default)]
struct ProjectileSpriteCache(HashMap<(u64, String), (Handle<Image>, Handle<TextureAtlasLayout>)>);

#[derive(Component)]
struct ProjectileSpriteVisual {
    profile: CombatVisualProfile,
    fallback: Entity,
    custom: Option<(Entity, Handle<Image>)>,
    elapsed: f32,
}

#[derive(Component)]
struct MinionRoleVisual {
    owner: Entity,
    kind: MinionKind,
    sequence: u64,
    remaining: f32,
    rest: Transform,
}

#[derive(Component, Clone, Copy, Debug)]
struct PresentationActorVisual {
    owner: Entity,
    kind: PresentationActorKind,
    world_height: f32,
    pivot: [f32; 2],
    previous_xy: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TowerLane {
    Top,
    Mid,
    Bot,
}

impl TowerLane {
    const fn label(self) -> &'static str {
        match self {
            Self::Top => "TOP",
            Self::Mid => "MID",
            Self::Bot => "BOT",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PresentationCueKind {
    TeamBadge(Team),
    LaneLabel(TowerLane),
    BaseLabel,
}

/// A fixed, render-only auxiliary. Towers have one team badge and one role
/// label; minions have one team badge. All are children of the primary proxy,
/// so owner cleanup remains bounded and recursive.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
struct PresentationActorCue {
    owner: Entity,
    kind: PresentationCueKind,
}

#[derive(Component)]
struct PresentationEffect {
    fps: f32,
    start: usize,
    count: usize,
    elapsed: f32,
}

#[derive(Resource, Default)]
struct LivePresentationEffects(VecDeque<Entity>);

fn evict_oldest_effect_if_full(live: &mut LivePresentationEffects) -> Option<Entity> {
    (live.0.len() >= TRANSIENT_VFX_BUDGET)
        .then(|| live.0.pop_front())
        .flatten()
}

#[derive(Clone, Copy)]
struct PreviousCombatState {
    alive: bool,
    action_sequence: u64,
}

#[derive(Resource, Default)]
struct PreviousCombatStates(HashMap<Entity, PreviousCombatState>);

pub struct Presentation2dPlugin;

impl Plugin for Presentation2dPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Presentation2dAssets>()
            .init_resource::<ProjectileSpriteCache>()
            .init_resource::<PreviousCombatStates>()
            .init_resource::<LivePresentationEffects>()
            .add_systems(
                Startup,
                load_presentation_assets.after(crate::persistence::load_persistent_client_settings),
            )
            .add_systems(Update, (emit_combat_effects, animate_effects).chain())
            // Snapshot owners and interpolation are produced in `Update`.
            // Reconcile presentation proxies afterwards, but before transform
            // propagation, so newly received actors are visible that frame.
            .add_systems(
                PostUpdate,
                (
                    attach_structure_visuals,
                    attach_minion_visuals,
                    attach_neutral_visuals,
                    attach_projectile_visuals,
                    animate_projectile_sprites,
                    animate_minion_role_cues,
                    update_actor_frames,
                    sync_actor_visuals,
                )
                    .chain()
                    .before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

fn load_presentation_assets(
    mode: Res<PlayerVisualMode>,
    mut assets: ResMut<Presentation2dAssets>,
    asset_server: Res<AssetServer>,
    mut atlas_layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    if *mode != PlayerVisualMode::Sprite2d {
        return;
    }
    let manifest = match serde_json::from_str::<PresentationManifest>(PRESENTATION_MANIFEST) {
        Ok(manifest) if manifest.schema_version == 1 => manifest,
        Ok(manifest) => {
            error!(
                "Unsupported 2D presentation schema {}",
                manifest.schema_version
            );
            return;
        }
        Err(error) => {
            error!("Invalid 2D presentation manifest: {error}");
            return;
        }
    };
    if manifest.actors.values().any(|definition| {
        let [x, y, width, height] = definition.occupied_bounds;
        width == 0
            || height == 0
            || x.saturating_add(width) > manifest.frame_size[0]
            || y.saturating_add(height) > manifest.frame_size[1]
    }) {
        error!("Invalid occupied bounds in 2D presentation manifest");
        return;
    }
    assets.actor_image = asset_server.load(format!("presentation2d/{}", manifest.actors_sheet));
    assets.effect_image = asset_server.load(format!("presentation2d/{}", manifest.effects_sheet));
    assets.actor_layout = atlas_layouts.add(TextureAtlasLayout::from_grid(
        UVec2::from_array(manifest.frame_size),
        manifest.actors_grid[0],
        manifest.actors_grid[1],
        None,
        None,
    ));
    assets.effect_layout = atlas_layouts.add(TextureAtlasLayout::from_grid(
        UVec2::from_array(manifest.frame_size),
        manifest.effects_grid[0],
        manifest.effects_grid[1],
        None,
        None,
    ));
    assets.actors = manifest.actors;
    assets.effects = manifest.effects;
}

fn actor_sprite(assets: &Presentation2dAssets, definition: &ActorDefinition) -> Sprite {
    let mut sprite = Sprite::from_atlas_image(
        assets.actor_image.clone(),
        TextureAtlas {
            layout: assets.actor_layout.clone(),
            index: definition.frame,
        },
    );
    sprite.custom_size = Some(Vec2::splat(definition.world_height));
    sprite
}

fn team_cue_color(team: Team) -> Color {
    match team {
        Team::Green => Color::srgb(0.34, 0.95, 0.42),
        Team::Blue => Color::srgb(0.38, 0.68, 1.0),
    }
}

fn spawn_team_badge(
    parent: &mut ChildSpawnerCommands,
    owner: Entity,
    team: Team,
    position: Vec2,
    size: f32,
) {
    let rotation = match team {
        Team::Green => Quat::IDENTITY,
        Team::Blue => Quat::from_rotation_z(std::f32::consts::FRAC_PI_4),
    };
    parent.spawn((
        Sprite::from_color(team_cue_color(team), Vec2::splat(size)),
        Transform::from_xyz(position.x, position.y, 0.2).with_rotation(rotation),
        PresentationActorCue {
            owner,
            kind: PresentationCueKind::TeamBadge(team),
        },
        Name::new(format!("Presentation2d-TeamBadge-{}", team.as_str())),
    ));
}

fn spawn_structure_cues(
    commands: &mut Commands,
    visual: Entity,
    owner: Entity,
    team: Team,
    lane: Option<TowerLane>,
    world_height: f32,
) {
    let cue_y = world_height * 0.54;
    commands.entity(visual).with_children(|parent| {
        spawn_team_badge(parent, owner, team, Vec2::new(-1.15, cue_y), 0.82);
        let (label, kind) = lane.map_or(("BASE", PresentationCueKind::BaseLabel), |lane| {
            (lane.label(), PresentationCueKind::LaneLabel(lane))
        });
        parent.spawn((
            Text2d::new(label),
            TextFont {
                font_size: 16.0,
                ..default()
            },
            TextColor(Color::WHITE),
            Transform::from_xyz(0.28, cue_y, 0.21).with_scale(Vec3::splat(0.105)),
            PresentationActorCue { owner, kind },
            Name::new(format!("Presentation2d-StructureCue-{label}")),
        ));
    });
}

fn spawn_minion_cue(
    commands: &mut Commands,
    visual: Entity,
    owner: Entity,
    team: Team,
    world_height: f32,
    role: MinionKind,
    sequence: u64,
) {
    commands.entity(visual).with_children(|parent| {
        spawn_team_badge(
            parent,
            owner,
            team,
            Vec2::new(0.0, world_height * 0.48),
            0.72,
        );
    });
    let rest = Transform::from_xyz(world_height * 0.37, -0.08, 0.25);
    commands
        .spawn((
            rest,
            MinionRoleVisual {
                owner,
                kind: role,
                sequence,
                remaining: 0.0,
                rest,
            },
            Visibility::default(),
            ChildOf(visual),
            Name::new(format!("Presentation2d-MinionRole-{role:?}")),
        ))
        .with_children(|parent| {
            let color = team_cue_color(team);
            let mut part = |size: Vec2, position: Vec2, angle: f32, color: Color| {
                parent.spawn((
                    Sprite::from_color(color, size),
                    Transform::from_xyz(position.x, position.y, 0.01)
                        .with_rotation(Quat::from_rotation_z(angle)),
                ));
            };
            match role {
                MinionKind::Caster => {
                    part(
                        Vec2::new(0.16, 1.35),
                        Vec2::ZERO,
                        0.0,
                        Color::srgb(0.95, 0.85, 0.60),
                    );
                    part(
                        Vec2::splat(0.49),
                        Vec2::new(0.0, 0.66),
                        std::f32::consts::FRAC_PI_4,
                        color,
                    );
                    part(
                        Vec2::splat(0.18),
                        Vec2::new(0.0, 0.66),
                        std::f32::consts::FRAC_PI_4,
                        Color::WHITE,
                    );
                }
                MinionKind::Melee => {
                    part(Vec2::new(0.48, 0.72), Vec2::new(0.0, -0.1), 0.0, color);
                    part(
                        Vec2::new(0.10, 0.57),
                        Vec2::new(0.0, -0.1),
                        0.0,
                        Color::WHITE,
                    );
                    part(
                        Vec2::new(0.23, 0.63),
                        Vec2::new(0.0, 0.48),
                        -0.22,
                        Color::srgb(0.95, 0.91, 0.75),
                    );
                }
            }
        });
}

fn animate_minion_role_cues(
    time: Option<Res<Time>>,
    actors: Query<(&NetworkMinionBrainState, Option<&NetworkMinionAction>)>,
    mut cues: Query<(&mut Transform, &mut MinionRoleVisual)>,
) {
    let delta = time.map_or(0.0, |time| time.delta_secs());
    for (mut transform, mut cue) in &mut cues {
        let Ok((_, action)) = actors.get(cue.owner) else {
            continue;
        };
        let sequence = action.map_or(0, |action| action.0);
        if sequence > cue.sequence {
            cue.remaining = 0.30;
        }
        cue.sequence = sequence;
        cue.remaining = (cue.remaining - delta).max(0.0);
        let pulse = (cue.remaining / 0.30 * std::f32::consts::PI).sin();
        *transform = cue.rest;
        match cue.kind {
            MinionKind::Caster => {
                transform.translation.y += pulse * 0.24;
                transform.scale *= 1.0 + pulse * 0.18;
            }
            MinionKind::Melee => {
                transform.rotation = Quat::from_rotation_z(-pulse * 0.9);
                transform.translation.x += pulse * 0.28;
            }
        }
    }
}

fn sample_polyline(points: &[Vec2], t: f32) -> Vec2 {
    let segment_lengths = points
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .collect::<Vec<_>>();
    let total_length = segment_lengths.iter().sum::<f32>();
    if total_length <= f32::EPSILON {
        return points.first().copied().unwrap_or(Vec2::ZERO);
    }
    let mut remaining = total_length * t.clamp(0.0, 1.0);
    for (index, length) in segment_lengths.into_iter().enumerate() {
        if remaining <= length {
            return points[index].lerp(points[index + 1], remaining / length.max(f32::EPSILON));
        }
        remaining -= length;
    }
    points.last().copied().unwrap_or(Vec2::ZERO)
}

fn classify_tower_lane(layout: &MapLayout, team: Team, position: Vec3) -> TowerLane {
    let sample = match team {
        Team::Green => 0.30,
        Team::Blue => 0.70,
    };
    let lanes = [TowerLane::Mid, TowerLane::Top, TowerLane::Bot];
    let position = Vec2::new(position.x, position.z);
    layout
        .lane_polylines()
        .iter()
        .zip(lanes)
        .min_by(|(left_points, _), (right_points, _)| {
            sample_polyline(left_points, sample)
                .distance_squared(position)
                .total_cmp(&sample_polyline(right_points, sample).distance_squared(position))
        })
        .map_or(TowerLane::Mid, |(_, lane)| lane)
}

fn attach_actor(
    commands: &mut Commands,
    assets: &Presentation2dAssets,
    owner: Entity,
    owner_position: Vec3,
    kind: PresentationActorKind,
    key: &str,
) -> Option<(Entity, f32)> {
    let Some(definition) = assets.actors.get(key) else {
        warn!("Missing 2D actor frame {key:?}");
        return None;
    };
    commands.entity(owner).insert(PresentationActorRoot);
    let xy = simulation_xz_to_render_xy(owner_position);
    let band = if kind == PresentationActorKind::Projectile {
        layer::PROJECTILE
    } else {
        layer::ACTOR
    };
    let anchor = Vec2::new(
        (0.5 - definition.pivot[0]) * definition.world_height,
        (0.5 - definition.pivot[1]) * definition.world_height,
    );
    let visual_entity = commands
        .spawn((
            actor_sprite(assets, definition),
            Transform::from_xyz(
                xy.x + anchor.x,
                xy.y + anchor.y,
                y_sorted_z(band, xy.y, owner),
            ),
            PresentationActorVisual {
                owner,
                kind,
                world_height: definition.world_height,
                pivot: definition.pivot,
                previous_xy: xy,
            },
            Name::new(format!("Presentation2d-{key}")),
        ))
        .id();
    if kind == PresentationActorKind::Boss {
        let label = if key.starts_with("wendigo") {
            "Wendigo"
        } else {
            "King Mutatio"
        };
        commands.entity(visual_entity).with_children(|parent| {
            parent.spawn((
                Text2d::new(label),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.86, 0.45)),
                Transform::from_xyz(0.0, definition.world_height * 0.62, 0.1)
                    .with_scale(Vec3::splat(0.15)),
                Name::new(format!("BossNameplate2d-{label}")),
            ));
        });
    }
    Some((visual_entity, definition.world_height))
}

fn structure_key(team: Team, kind: StructureKind) -> &'static str {
    match (team, kind) {
        (Team::Green, StructureKind::Tower) => "green_tower",
        (Team::Blue, StructureKind::Tower) => "blue_tower",
        (Team::Green, StructureKind::BaseTower) => "green_base_tower",
        (Team::Blue, StructureKind::BaseTower) => "blue_base_tower",
    }
}

fn minion_key(team: Team, state: MinionBrainState) -> &'static str {
    let marching = matches!(
        state,
        MinionBrainState::Marching | MinionBrainState::Chasing
    );
    match (team, marching) {
        (Team::Green, true) => "green_minion_march",
        (Team::Green, false) => "green_minion_idle",
        (Team::Blue, true) => "blue_minion_march",
        (Team::Blue, false) => "blue_minion_idle",
    }
}

fn boss_key(boss: &BossVisual, state: NeutralAiState) -> &'static str {
    let aggro = matches!(state, NeutralAiState::Aggro);
    match (boss.camp_type, aggro) {
        (crate::net::NeutralCampType::WendigoBoss, false) => "wendigo_idle",
        (crate::net::NeutralCampType::WendigoBoss, true) => "wendigo_aggro",
        (crate::net::NeutralCampType::KingMutatioBoss, false) => "king_mutatio_idle",
        (crate::net::NeutralCampType::KingMutatioBoss, true) => "king_mutatio_aggro",
        _ => "neutral",
    }
}

fn attach_structure_visuals(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    assets: Res<Presentation2dAssets>,
    map_layout: Res<MapLayout>,
    roots: Query<
        (Entity, &Transform, &Team, &StructureKind),
        (With<NetworkStructure>, Without<PresentationActorRoot>),
    >,
) {
    if *mode != PlayerVisualMode::Sprite2d {
        return;
    }
    for (entity, transform, team, kind) in &roots {
        commands
            .entity(entity)
            .remove::<Mesh3d>()
            .remove::<MeshMaterial3d<StandardMaterial>>();
        let actor = attach_actor(
            &mut commands,
            &assets,
            entity,
            transform.translation,
            PresentationActorKind::Structure,
            structure_key(*team, *kind),
        );
        if let Some((visual, world_height)) = actor {
            let lane = (*kind == StructureKind::Tower)
                .then(|| classify_tower_lane(&map_layout, *team, transform.translation));
            spawn_structure_cues(&mut commands, visual, entity, *team, lane, world_height);
        }
    }
}

fn attach_minion_visuals(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    assets: Res<Presentation2dAssets>,
    roots: Query<
        (
            Entity,
            &Transform,
            &Team,
            &NetworkMinionBrainState,
            Option<&NetworkMinionKind>,
            Option<&NetworkMinionAction>,
        ),
        (With<NetworkMinion>, Without<PresentationActorRoot>),
    >,
) {
    if *mode != PlayerVisualMode::Sprite2d {
        return;
    }
    for (entity, transform, team, state, kind, action) in &roots {
        if let Some((visual, world_height)) = attach_actor(
            &mut commands,
            &assets,
            entity,
            transform.translation,
            PresentationActorKind::Minion,
            minion_key(*team, state.0),
        ) {
            spawn_minion_cue(
                &mut commands,
                visual,
                entity,
                *team,
                world_height,
                kind.map_or(MinionKind::Melee, |kind| kind.0),
                action.map_or(0, |action| action.0),
            );
        }
    }
}

fn attach_neutral_visuals(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    assets: Res<Presentation2dAssets>,
    roots: Query<
        (Entity, &Transform, Option<&BossVisual>, &NeutralAiStateTag),
        (With<NetworkNeutral>, Without<PresentationActorRoot>),
    >,
) {
    if *mode != PlayerVisualMode::Sprite2d {
        return;
    }
    for (entity, transform, boss, state) in &roots {
        commands
            .entity(entity)
            .remove::<Mesh3d>()
            .remove::<MeshMaterial3d<StandardMaterial>>();
        let (kind, key) = boss.map_or((PresentationActorKind::Neutral, "neutral"), |boss| {
            (PresentationActorKind::Boss, boss_key(boss, state.0))
        });
        let _ = attach_actor(
            &mut commands,
            &assets,
            entity,
            transform.translation,
            kind,
            key,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn attach_projectile_visuals(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    registry: Option<Res<CombatVisualRegistry>>,
    server: Option<Res<AssetServer>>,
    mut layouts: Option<ResMut<Assets<TextureAtlasLayout>>>,
    mut cache: ResMut<ProjectileSpriteCache>,
    roots: Query<(Entity, &Transform, &NetworkProjectile), Without<PresentationActorRoot>>,
    owners: Query<(
        &NetworkPlayerId,
        Option<&NetworkHeroClass>,
        Option<&NetworkAvatar>,
        Option<&NetworkSpriteCharacter>,
    )>,
) {
    if *mode != PlayerVisualMode::Sprite2d {
        return;
    }
    let defaults = CombatVisualRegistry::default();
    let registry = registry.as_deref().unwrap_or(&defaults);
    for (owner, transform, projectile) in &roots {
        let identity = matches!(
            projectile.source_kind,
            CombatEntityKind::Player | CombatEntityKind::Unknown
        )
        .then(|| {
            owners
                .iter()
                .find(|(id, _, _, _)| id.0 == projectile.owner_id)
        })
        .flatten();
        let class = identity.and_then(|(_, class, _, _)| class.map(|class| class.0));
        let avatar =
            identity.and_then(|(_, _, avatar, _)| avatar.and_then(|avatar| avatar.0.as_deref()));
        let sprite_id =
            identity.and_then(|(_, _, _, sprite)| sprite.and_then(|sprite| sprite.0.as_deref()));
        let profile = registry
            .resolve(
                class,
                projectile.style,
                projectile.action_slot,
                avatar,
                sprite_id,
            )
            .clone();
        let xy = simulation_xz_to_render_xy(transform.translation);
        let direction = simulation_xz_to_render_xy(projectile.direction);
        let angle = if direction.is_finite() && direction.length_squared() > 0.000_001 {
            direction.y.atan2(direction.x)
        } else {
            0.0
        };
        commands.entity(owner).insert(PresentationActorRoot);
        let proxy = commands
            .spawn((
                Transform::from_xyz(xy.x, xy.y, y_sorted_z(layer::PROJECTILE, xy.y, owner))
                    .with_rotation(Quat::from_rotation_z(angle))
                    .with_scale(Vec3::splat(profile.scale)),
                Visibility::default(),
                ProjectilePresentationRoot { owner },
                PresentationActorVisual {
                    owner,
                    kind: PresentationActorKind::Projectile,
                    world_height: 0.0,
                    pivot: [0.5, 0.5],
                    previous_xy: xy,
                },
                // The transparent container allows the existing proxy reconciliation to stay shared.
                Sprite::from_color(Color::NONE, Vec2::ZERO),
                Name::new(format!("Presentation2d-Projectile-{}", profile.id)),
            ))
            .id();
        let fallback = commands
            .spawn((
                Transform::default(),
                Visibility::default(),
                ChildOf(proxy),
                Name::new("Projectile2d-Procedural"),
            ))
            .with_children(|parent| {
                spawn_projectile_shape_2d(parent, &profile, projectile.owner_team)
            })
            .id();
        let custom = profile.sprite.as_ref().and_then(|sprite| {
            let server = server.as_ref()?;
            let layouts = layouts.as_mut()?;
            let (image, layout) = cache
                .0
                .entry((registry.revision(), profile.id.clone()))
                .or_insert_with(|| {
                    (
                        server.load(sprite.path.clone()),
                        layouts.add(TextureAtlasLayout::from_grid(
                            UVec2::from_array(sprite.frame_size),
                            sprite.columns,
                            sprite.rows,
                            None,
                            None,
                        )),
                    )
                })
                .clone();
            let mut visual = Sprite::from_atlas_image(
                image.clone(),
                TextureAtlas {
                    layout,
                    index: sprite.first_frame,
                },
            );
            visual.custom_size = Some(Vec2::new(
                sprite.world_height * sprite.frame_size[0] as f32 / sprite.frame_size[1] as f32,
                sprite.world_height,
            ));
            let entity = commands
                .spawn((
                    visual,
                    Transform::default(),
                    Visibility::Hidden,
                    ChildOf(proxy),
                    Name::new("Projectile2d-PackagedSprite"),
                ))
                .id();
            Some((entity, image))
        });
        commands.entity(proxy).insert(ProjectileSpriteVisual {
            profile,
            fallback,
            custom,
            elapsed: 0.0,
        });
    }
}

fn spawn_projectile_shape_2d(
    parent: &mut ChildSpawnerCommands,
    profile: &CombatVisualProfile,
    team: Team,
) {
    let tint = profile.color();
    let mut part = |size: Vec2, center: Vec2, angle: f32, color: Color| {
        parent.spawn((
            Sprite::from_color(color, size),
            Transform::from_xyz(center.x, center.y, 0.01)
                .with_rotation(Quat::from_rotation_z(angle)),
        ));
    };
    match profile.shape {
        ProjectileShape::Arrow => {
            part(Vec2::new(1.5, 0.10), Vec2::ZERO, 0.0, Color::WHITE);
            part(Vec2::new(0.48, 0.10), Vec2::new(0.64, 0.14), -0.7, tint);
            part(Vec2::new(0.48, 0.10), Vec2::new(0.64, -0.14), 0.7, tint);
            for y in [-0.14, 0.14] {
                part(
                    Vec2::new(0.4, 0.13),
                    Vec2::new(-0.52, y),
                    y.signum() * 0.55,
                    tint,
                );
            }
        }
        ProjectileShape::Arcane => {
            part(
                Vec2::splat(0.65),
                Vec2::ZERO,
                std::f32::consts::FRAC_PI_4,
                tint,
            );
            part(Vec2::new(0.48, 0.10), Vec2::new(-0.6, 0.25), 0.0, tint);
            part(Vec2::new(0.48, 0.10), Vec2::new(-0.6, -0.25), 0.0, tint);
            part(
                Vec2::splat(0.23),
                Vec2::new(0.12, 0.0),
                std::f32::consts::FRAC_PI_4,
                Color::WHITE,
            );
        }
        ProjectileShape::Holy => {
            part(
                Vec2::splat(0.58),
                Vec2::ZERO,
                std::f32::consts::FRAC_PI_4,
                tint,
            );
            part(Vec2::new(0.12, 1.18), Vec2::ZERO, 0.0, Color::WHITE);
            part(Vec2::new(1.0, 0.12), Vec2::ZERO, 0.0, Color::WHITE);
        }
        ProjectileShape::Crescent => {
            for index in 0..9 {
                let angle = -1.2 + index as f32 * 2.4 / 8.0;
                part(
                    Vec2::new(0.13, 0.30),
                    Vec2::new(angle.cos() * 0.70 - 0.25, angle.sin() * 0.9),
                    angle,
                    tint,
                );
            }
        }
        ProjectileShape::Bolt => {
            part(Vec2::new(1.2, 0.25), Vec2::ZERO, 0.0, tint);
            part(
                Vec2::new(0.4, 0.12),
                Vec2::new(0.44, 0.0),
                0.0,
                Color::WHITE,
            );
        }
    }
    part(
        Vec2::splat(0.19),
        Vec2::new(-0.78, 0.0),
        std::f32::consts::FRAC_PI_4,
        team_cue_color(team),
    );
}

fn animate_projectile_sprites(
    time: Option<Res<Time>>,
    images: Option<Res<Assets<Image>>>,
    mut projectiles: Query<&mut ProjectileSpriteVisual>,
    mut sprites: Query<&mut Sprite>,
    mut visibility: Query<&mut Visibility>,
) {
    let delta = time.map_or(0.0, |time| time.delta_secs());
    for mut visual in &mut projectiles {
        visual.elapsed += delta;
        let Some((entity, image)) = &visual.custom else {
            continue;
        };
        let Some(config) = &visual.profile.sprite else {
            continue;
        };
        let ready = images.as_ref().is_some_and(|images| {
            images
                .get(image)
                .is_some_and(|image| config.matches_image(image))
        });
        if let Ok(mut state) = visibility.get_mut(*entity) {
            *state = if ready {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
        if let Ok(mut state) = visibility.get_mut(visual.fallback) {
            *state = if ready {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        }
        if ready {
            if let Ok(mut sprite) = sprites.get_mut(*entity) {
                if let Some(atlas) = sprite.texture_atlas.as_mut() {
                    atlas.index =
                        config.first_frame + (visual.elapsed * config.fps) as usize % config.frames;
                }
            }
        }
    }
}

fn update_actor_frames(
    assets: Res<Presentation2dAssets>,
    minions: Query<(&Team, &NetworkMinionBrainState), With<NetworkMinion>>,
    neutrals: Query<(Option<&BossVisual>, &NeutralAiStateTag), With<NetworkNeutral>>,
    mut visuals: Query<(&PresentationActorVisual, &mut Sprite)>,
) {
    for (visual, mut sprite) in &mut visuals {
        let key = match visual.kind {
            PresentationActorKind::Minion => minions
                .get(visual.owner)
                .ok()
                .map(|(team, state)| minion_key(*team, state.0)),
            PresentationActorKind::Boss => neutrals
                .get(visual.owner)
                .ok()
                .and_then(|(boss, state)| boss.map(|boss| boss_key(boss, state.0))),
            _ => None,
        };
        if let Some(frame) = key
            .and_then(|key| assets.actors.get(key))
            .map(|definition| definition.frame)
            && let Some(atlas) = sprite.texture_atlas.as_mut()
        {
            atlas.index = frame;
        }
    }
}

fn sync_actor_visuals(
    mut commands: Commands,
    owners: Query<&Transform, Without<PresentationActorVisual>>,
    mut visuals: Query<(
        Entity,
        &mut PresentationActorVisual,
        &mut Transform,
        &mut Sprite,
    )>,
) {
    for (entity, mut visual, mut transform, mut sprite) in &mut visuals {
        let Ok(owner) = owners.get(visual.owner) else {
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
            continue;
        };
        let xy = simulation_xz_to_render_xy(owner.translation);
        let anchor = Vec2::new(
            (0.5 - visual.pivot[0]) * visual.world_height,
            (0.5 - visual.pivot[1]) * visual.world_height,
        );
        transform.translation.x = xy.x + anchor.x;
        transform.translation.y = xy.y + anchor.y;
        let band = if visual.kind == PresentationActorKind::Projectile {
            layer::PROJECTILE
        } else {
            layer::ACTOR
        };
        transform.translation.z = y_sorted_z(band, xy.y, visual.owner);
        let delta = xy - visual.previous_xy;
        if visual.kind == PresentationActorKind::Projectile && delta.length_squared() > 0.000_001 {
            transform.rotation = Quat::from_rotation_z(delta.y.atan2(delta.x));
        } else if delta.x.abs() > 0.001 {
            sprite.flip_x = delta.x < 0.0;
        }
        visual.previous_xy = xy;
    }
}

fn spawn_effect(
    commands: &mut Commands,
    assets: &Presentation2dAssets,
    live: &mut LivePresentationEffects,
    effect_id: &str,
    team: Team,
    simulation_position: Vec3,
) {
    while live.0.len() >= TRANSIENT_VFX_BUDGET {
        if let Some(oldest) = evict_oldest_effect_if_full(live) {
            commands.entity(oldest).try_despawn();
        }
    }
    let resolved = match (effect_id, team) {
        ("cast", Team::Green) => "green_cast",
        ("cast", Team::Blue) => "blue_cast",
        ("hit", Team::Green) => "green_hit",
        ("hit", Team::Blue) => "blue_hit",
        _ => effect_id,
    };
    let Some(definition) = assets.effects.get(resolved) else {
        return;
    };
    let mut sprite = Sprite::from_atlas_image(
        assets.effect_image.clone(),
        TextureAtlas {
            layout: assets.effect_layout.clone(),
            index: definition.start,
        },
    );
    sprite.custom_size = Some(Vec2::splat(definition.world_height));
    let xy = simulation_xz_to_render_xy(simulation_position);
    let entity = commands
        .spawn((
            sprite,
            Transform::from_xyz(xy.x, xy.y, layer::VFX),
            PresentationEffect {
                fps: definition.fps,
                start: definition.start,
                count: definition.count,
                elapsed: 0.0,
            },
            Name::new(format!("PresentationEffect-{resolved}")),
        ))
        .id();
    live.0.push_back(entity);
}

fn emit_combat_effects(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    assets: Res<Presentation2dAssets>,
    mut live: ResMut<LivePresentationEffects>,
    mut previous: ResMut<PreviousCombatStates>,
    actors: Query<
        (
            Entity,
            &Transform,
            &CombatStats,
            &Team,
            &PlayerCosmeticAction,
        ),
        Or<(With<Player>, With<RemotePlayer>)>,
    >,
) {
    if *mode != PlayerVisualMode::Sprite2d {
        previous.0.clear();
        return;
    }
    previous.0.retain(|entity, _| actors.get(*entity).is_ok());
    for (entity, transform, stats, team, action) in &actors {
        let current = PreviousCombatState {
            alive: stats.is_alive(),
            action_sequence: action.sequence,
        };
        if let Some(old) = previous.0.insert(entity, current) {
            if old.alive && !current.alive {
                spawn_effect(
                    &mut commands,
                    &assets,
                    &mut live,
                    "death",
                    *team,
                    transform.translation,
                );
            }
            if action.sequence != old.action_sequence
                && matches!(
                    action.kind,
                    PlayerActionKind::Attack | PlayerActionKind::Cast
                )
            {
                spawn_effect(
                    &mut commands,
                    &assets,
                    &mut live,
                    "cast",
                    *team,
                    transform.translation,
                );
            }
        }
    }
}

fn animate_effects(
    mut commands: Commands,
    time: Res<Time>,
    mut live: ResMut<LivePresentationEffects>,
    mut effects: Query<(Entity, &mut PresentationEffect, &mut Sprite)>,
) {
    live.0.retain(|entity| effects.get(*entity).is_ok());
    for (entity, mut effect, mut sprite) in &mut effects {
        effect.elapsed += time.delta_secs().max(0.0);
        let frame = (effect.elapsed * effect.fps).floor() as usize;
        if frame >= effect.count || effect.elapsed >= TRANSIENT_VFX_MAX_LIFETIME {
            commands.entity(entity).despawn();
            continue;
        }
        if let Some(atlas) = sprite.texture_atlas.as_mut() {
            atlas.index = effect.start + frame;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::combat::ProjectileStyle;

    fn projectile_fixture(style: ProjectileStyle, id: u64) -> NetworkProjectile {
        NetworkProjectile {
            id,
            owner_id: 42,
            owner_team: Team::Green,
            source_kind: CombatEntityKind::Player,
            style,
            action_slot: None,
            direction: Vec3::X,
        }
    }

    #[test]
    fn procedural_projectile_classes_have_distinct_sprites_and_bounded_owner_cleanup() {
        let mut app = App::new();
        app.insert_resource(PlayerVisualMode::Sprite2d)
            .init_resource::<CombatVisualRegistry>()
            .init_resource::<ProjectileSpriteCache>()
            .add_systems(
                Update,
                (
                    attach_projectile_visuals,
                    animate_projectile_sprites,
                    sync_actor_visuals,
                )
                    .chain(),
            );
        let mut owners = Vec::new();
        for (id, style) in [
            ProjectileStyle::Arrow,
            ProjectileStyle::Arcane,
            ProjectileStyle::Holy,
            ProjectileStyle::Crescent,
        ]
        .into_iter()
        .enumerate()
        {
            owners.push(
                app.world_mut()
                    .spawn((
                        Transform::from_xyz(id as f32, 1.0, 3.0),
                        projectile_fixture(style, id as u64),
                    ))
                    .id(),
            );
        }
        app.update();
        app.update();
        let shapes: std::collections::HashSet<_> = app
            .world_mut()
            .query::<&ProjectileSpriteVisual>()
            .iter(app.world())
            .map(|visual| visual.profile.shape)
            .collect();
        assert_eq!(shapes.len(), 4);
        assert_eq!(
            app.world_mut()
                .query::<&ProjectilePresentationRoot>()
                .iter(app.world())
                .count(),
            4
        );
        let sprite_count = app.world_mut().query::<&Sprite>().iter(app.world()).count();
        assert!(sprite_count > 16 && sprite_count < 40);
        app.update();
        assert_eq!(
            app.world_mut().query::<&Sprite>().iter(app.world()).count(),
            sprite_count
        );
        for owner in owners {
            app.world_mut().entity_mut(owner).despawn();
        }
        app.update();
        assert_eq!(
            app.world_mut().query::<&Sprite>().iter(app.world()).count(),
            0
        );
    }

    #[test]
    fn configured_sprite_uses_cached_atlas_animates_and_falls_back_for_missing_or_wrong_image() {
        let registry = CombatVisualRegistry::from_json(
            r#"{"schema_version":1,
            "profiles":{"custom":{"shape":"arrow","color":[1,0.5,0.2,1],
              "sprite":{"path":"cosmetics/test.png","frame_size":[16,16],"columns":2,"rows":1,
                "frames":2,"fps":8,"world_height":1.5}}},
            "avatar_overrides":{"agnes":{"basic":"custom"}}}"#,
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::ZERO,
            ))
            .init_asset::<Image>()
            .init_asset::<TextureAtlasLayout>()
            .insert_resource(PlayerVisualMode::Sprite2d)
            .insert_resource(registry)
            .init_resource::<ProjectileSpriteCache>()
            .add_systems(
                Update,
                (
                    attach_projectile_visuals,
                    animate_projectile_sprites,
                    sync_actor_visuals,
                )
                    .chain(),
            );
        // Seed the real cache with a deliberately wrong-sized image. No disk/network
        // loader is required; attachment and animation still run their production path.
        let image = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        let layout = app
            .world_mut()
            .resource_mut::<Assets<TextureAtlasLayout>>()
            .add(TextureAtlasLayout::from_grid(
                UVec2::splat(16),
                2,
                1,
                None,
                None,
            ));
        app.world_mut()
            .resource_mut::<ProjectileSpriteCache>()
            .0
            .insert((0, "custom".into()), (image.clone(), layout.clone()));
        app.world_mut()
            .spawn((NetworkPlayerId(42), NetworkAvatar(Some("agnes".into()))));
        let owner = app
            .world_mut()
            .spawn((
                Transform::default(),
                projectile_fixture(ProjectileStyle::Arrow, 1),
            ))
            .id();
        app.update();
        let (proxy, fallback, custom) = app
            .world_mut()
            .query::<(Entity, &ProjectileSpriteVisual)>()
            .iter(app.world())
            .map(|(entity, visual)| (entity, visual.fallback, visual.custom.as_ref().unwrap().0))
            .next()
            .unwrap();
        assert_eq!(
            *app.world().get::<Visibility>(fallback).unwrap(),
            Visibility::Inherited
        );
        assert_eq!(
            *app.world().get::<Visibility>(custom).unwrap(),
            Visibility::Hidden
        );
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .get_mut(&image)
            .unwrap()
            .resize(bevy::render::render_resource::Extent3d {
                width: 32,
                height: 16,
                depth_or_array_layers: 1,
            });
        app.world_mut()
            .get_mut::<ProjectileSpriteVisual>(proxy)
            .unwrap()
            .elapsed = 0.16;
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(fallback).unwrap(),
            Visibility::Hidden
        );
        assert_eq!(
            *app.world().get::<Visibility>(custom).unwrap(),
            Visibility::Inherited
        );
        let sprite = app.world().get::<Sprite>(custom).unwrap();
        assert_eq!(sprite.texture_atlas.as_ref().unwrap().layout, layout);
        assert_eq!(sprite.texture_atlas.as_ref().unwrap().index, 1);
        assert_eq!(sprite.custom_size, Some(Vec2::splat(1.5)));
        let second = app
            .world_mut()
            .spawn((
                Transform::default(),
                projectile_fixture(ProjectileStyle::Arrow, 2),
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().resource::<Assets<TextureAtlasLayout>>().len(),
            1
        );
        assert_eq!(app.world().resource::<ProjectileSpriteCache>().0.len(), 1);
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .remove(image.id());
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(fallback).unwrap(),
            Visibility::Inherited
        );
        assert_eq!(
            *app.world().get::<Visibility>(custom).unwrap(),
            Visibility::Hidden
        );
        for entity in [owner, second] {
            app.world_mut().entity_mut(entity).despawn();
        }
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ProjectileSpriteVisual>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn minion_role_cues_are_distinct_release_from_sequence_and_clean_up_with_owner() {
        let mut app = proxy_test_app(PlayerVisualMode::Sprite2d);
        app.init_resource::<Time>()
            .add_systems(PostUpdate, animate_minion_role_cues);
        let owners: Vec<_> = [MinionKind::Melee, MinionKind::Caster]
            .into_iter()
            .map(|kind| {
                app.world_mut()
                    .spawn((
                        Transform::default(),
                        NetworkMinion,
                        Team::Green,
                        NetworkMinionBrainState(MinionBrainState::Attacking),
                        NetworkMinionKind(kind),
                        NetworkMinionAction(5),
                    ))
                    .id()
            })
            .collect();
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&MinionRoleVisual>()
                .iter(app.world())
                .count(),
            2
        );
        assert!(
            app.world_mut()
                .query::<&MinionRoleVisual>()
                .iter(app.world())
                .all(|cue| cue.remaining == 0.0)
        );
        for owner in &owners {
            app.world_mut()
                .get_mut::<NetworkMinionAction>(*owner)
                .unwrap()
                .0 = 6;
        }
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(0.1));
        app.update();
        for (transform, cue) in app
            .world_mut()
            .query::<(&Transform, &MinionRoleVisual)>()
            .iter(app.world())
        {
            match cue.kind {
                MinionKind::Melee => assert_ne!(transform.rotation, cue.rest.rotation),
                MinionKind::Caster => assert!(transform.scale.x > cue.rest.scale.x),
            }
        }
        for owner in owners {
            app.world_mut().entity_mut(owner).despawn();
        }
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&MinionRoleVisual>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn hp_changes_alone_do_not_create_inferred_hit_or_heal_effects() {
        let mut app = App::new();
        app.insert_resource(PlayerVisualMode::Sprite2d)
            .insert_resource(manifest_assets())
            .init_resource::<LivePresentationEffects>()
            .init_resource::<PreviousCombatStates>()
            .add_systems(Update, emit_combat_effects);
        let actor = app
            .world_mut()
            .spawn((
                Transform::default(),
                CombatStats::default(),
                Team::Green,
                PlayerCosmeticAction::default(),
                Player,
            ))
            .id();
        app.update();
        for hp in [80.0, 90.0, 100.0] {
            app.world_mut().get_mut::<CombatStats>(actor).unwrap().hp = hp;
            app.update();
        }
        assert!(
            app.world()
                .resource::<LivePresentationEffects>()
                .0
                .is_empty()
        );
        assert_eq!(
            app.world_mut()
                .query::<&PresentationEffect>()
                .iter(app.world())
                .count(),
            0
        );
    }
    #[test]
    fn models3d_startup_does_not_request_optional_2d_presentation_assets() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<Image>()
            .init_asset::<TextureAtlasLayout>()
            .insert_resource(PlayerVisualMode::Models3d)
            .init_resource::<Presentation2dAssets>()
            .add_systems(Startup, load_presentation_assets);
        app.update();
        let assets = app.world().resource::<Presentation2dAssets>();
        assert!(assets.actors.is_empty());
        assert!(assets.effects.is_empty());
        assert_eq!(assets.actor_image, Handle::default());
        assert_eq!(assets.effect_image, Handle::default());
        assert!(
            app.world()
                .resource::<Assets<TextureAtlasLayout>>()
                .is_empty()
        );
    }

    #[test]
    fn manifest_has_complete_actor_and_vfx_coverage() {
        let manifest: PresentationManifest = serde_json::from_str(PRESENTATION_MANIFEST).unwrap();
        assert_eq!(manifest.schema_version, 1);
        for key in [
            "green_tower",
            "blue_tower",
            "green_base_tower",
            "blue_base_tower",
            "green_minion_idle",
            "green_minion_march",
            "blue_minion_idle",
            "blue_minion_march",
            "neutral",
            "wendigo_idle",
            "wendigo_aggro",
            "king_mutatio_idle",
            "king_mutatio_aggro",
            "green_projectile",
            "blue_projectile",
        ] {
            assert!(manifest.actors.contains_key(key), "missing actor {key}");
        }
        for key in [
            "green_cast",
            "blue_cast",
            "green_hit",
            "blue_hit",
            "heal",
            "death",
        ] {
            assert!(manifest.effects.contains_key(key), "missing VFX {key}");
        }
        for effect in manifest.effects.values() {
            assert!(effect.count > 0 && effect.fps > 0.0);
            assert!(effect.count as f32 / effect.fps <= TRANSIENT_VFX_MAX_LIFETIME);
        }
    }

    #[test]
    fn team_and_state_keys_are_shape_distinct() {
        assert_ne!(
            structure_key(Team::Green, StructureKind::Tower),
            structure_key(Team::Blue, StructureKind::Tower)
        );
        assert_ne!(
            minion_key(Team::Green, MinionBrainState::Marching),
            minion_key(Team::Green, MinionBrainState::Attacking)
        );
    }

    fn manifest_assets() -> Presentation2dAssets {
        let manifest: PresentationManifest = serde_json::from_str(PRESENTATION_MANIFEST).unwrap();
        Presentation2dAssets {
            actors: manifest.actors,
            effects: manifest.effects,
            ..default()
        }
    }

    fn proxy_test_app(mode: PlayerVisualMode) -> App {
        let mut app = App::new();
        app.insert_resource(mode)
            .insert_resource(MapLayout::default())
            .insert_resource(manifest_assets())
            .add_systems(
                Update,
                (
                    attach_structure_visuals,
                    attach_minion_visuals,
                    update_actor_frames,
                    sync_actor_visuals,
                )
                    .chain(),
            );
        app
    }

    #[test]
    fn actor_scales_preserve_readability_at_gameplay_zoom() {
        const DEFAULT_CAMERA_SCALE: f32 = 0.08;
        const MAX_ZOOM_OUT_SCALE: f32 = 0.18;
        let manifest: PresentationManifest = serde_json::from_str(PRESENTATION_MANIFEST).unwrap();
        let visible_pixels = |key: &str, camera_scale: f32| {
            let actor = &manifest.actors[key];
            let occupied_height = actor.occupied_bounds[3] as f32 / 256.0;
            actor.world_height * occupied_height / camera_scale
        };

        for (key, expected_height) in [
            ("green_tower", 4.5),
            ("blue_tower", 4.5),
            ("green_base_tower", 6.0),
            ("blue_base_tower", 6.0),
            ("green_minion_idle", 2.2),
            ("green_minion_march", 2.2),
            ("blue_minion_idle", 2.2),
            ("blue_minion_march", 2.2),
        ] {
            assert_eq!(manifest.actors[key].world_height, expected_height);
            assert!(visible_pixels(key, DEFAULT_CAMERA_SCALE) >= 12.0, "{key}");
        }
        for key in ["green_tower", "blue_tower"] {
            assert!(visible_pixels(key, MAX_ZOOM_OUT_SCALE) >= 14.0, "{key}");
        }
        for key in [
            "green_minion_idle",
            "green_minion_march",
            "blue_minion_idle",
            "blue_minion_march",
        ] {
            assert!(visible_pixels(key, MAX_ZOOM_OUT_SCALE) >= 6.0, "{key}");
        }
        assert!(
            visible_pixels("green_base_tower", MAX_ZOOM_OUT_SCALE)
                > visible_pixels("green_tower", MAX_ZOOM_OUT_SCALE)
        );
        assert!(
            visible_pixels("green_tower", MAX_ZOOM_OUT_SCALE)
                > visible_pixels("green_minion_march", MAX_ZOOM_OUT_SCALE)
        );
    }

    #[test]
    fn lane_classification_uses_team_specific_authoritative_samples() {
        let layout = MapLayout::default();
        for (lane, points) in [TowerLane::Mid, TowerLane::Top, TowerLane::Bot]
            .into_iter()
            .zip(layout.lane_polylines())
        {
            for (team, fraction) in [(Team::Green, 0.30), (Team::Blue, 0.70)] {
                let anchor = sample_polyline(&points, fraction);
                let position = Vec3::new(anchor.x + 0.01, 3.0, anchor.y - 0.01);
                assert_eq!(classify_tower_lane(&layout, team, position), lane);
            }
        }
    }

    #[test]
    fn actor_proxy_attachment_is_idempotent_and_owner_cleanup_is_bounded() {
        let mut app = proxy_test_app(PlayerVisualMode::Sprite2d);
        let owner = app
            .world_mut()
            .spawn((
                Transform::from_xyz(4.0, 0.5, -7.0),
                NetworkStructure,
                Team::Green,
                StructureKind::Tower,
            ))
            .id();
        app.update();
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&PresentationActorVisual>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world_mut()
                .query::<&PresentationActorCue>()
                .iter(app.world())
                .count(),
            2
        );
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&PresentationActorVisual>()
                .iter(app.world())
                .count(),
            1
        );
        app.world_mut().entity_mut(owner).despawn();
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&PresentationActorVisual>()
                .iter(app.world())
                .count(),
            0
        );
        assert_eq!(
            app.world_mut()
                .query::<&PresentationActorCue>()
                .iter(app.world())
                .count(),
            0
        );
    }

    fn spawn_structure_and_wave_fixture(
        app: &mut App,
        wave_offset: f32,
    ) -> (Vec<Entity>, Vec<Entity>) {
        let layout = MapLayout::default();
        let mut structures = Vec::new();
        for points in layout.lane_polylines() {
            for (team, fraction) in [(Team::Green, 0.30), (Team::Blue, 0.70)] {
                let anchor = sample_polyline(&points, fraction);
                structures.push(
                    app.world_mut()
                        .spawn((
                            Transform::from_xyz(anchor.x, 3.0, anchor.y),
                            NetworkStructure,
                            team,
                            StructureKind::Tower,
                        ))
                        .id(),
                );
            }
        }
        for (team, position) in [
            (Team::Green, layout.home_spawn),
            (Team::Blue, layout.away_spawn),
        ] {
            structures.push(
                app.world_mut()
                    .spawn((
                        Transform::from_xyz(position.x, 3.0, position.z),
                        NetworkStructure,
                        team,
                        StructureKind::BaseTower,
                    ))
                    .id(),
            );
        }

        let mut minions = Vec::new();
        for lane in 0..3 {
            for team in [Team::Green, Team::Blue] {
                for member in 0..3 {
                    minions.push(
                        app.world_mut()
                            .spawn((
                                Transform::from_xyz(
                                    wave_offset + lane as f32 * 3.0,
                                    0.5,
                                    member as f32,
                                ),
                                NetworkMinion,
                                NetworkMinionBrainState(MinionBrainState::Marching),
                                team,
                            ))
                            .id(),
                    );
                }
            }
        }
        (structures, minions)
    }

    fn primary_and_cue_counts(app: &mut App) -> (usize, usize) {
        let primary = app
            .world_mut()
            .query::<&PresentationActorVisual>()
            .iter(app.world())
            .count();
        let cues = app
            .world_mut()
            .query::<&PresentationActorCue>()
            .iter(app.world())
            .count();
        (primary, cues)
    }

    fn despawn_owners(app: &mut App, owners: impl IntoIterator<Item = Entity>) {
        for owner in owners {
            app.world_mut().entity_mut(owner).despawn();
        }
    }

    #[test]
    fn six_towers_two_bases_and_initial_wave_stay_one_to_one_and_bounded() {
        let mut app = proxy_test_app(PlayerVisualMode::Sprite2d);
        let (structures, minions) = spawn_structure_and_wave_fixture(&mut app, 0.0);

        app.update();
        let mut owner_counts = HashMap::<Entity, usize>::new();
        let mut structure_count = 0;
        let mut lane_tower_count = 0;
        let mut base_count = 0;
        let mut minion_count = 0;
        for visual in app
            .world_mut()
            .query::<&PresentationActorVisual>()
            .iter(app.world())
        {
            *owner_counts.entry(visual.owner).or_default() += 1;
            match visual.kind {
                PresentationActorKind::Structure => {
                    structure_count += 1;
                    match app
                        .world()
                        .get::<StructureKind>(visual.owner)
                        .expect("structure proxy owner has a kind")
                    {
                        StructureKind::Tower => lane_tower_count += 1,
                        StructureKind::BaseTower => base_count += 1,
                    }
                }
                PresentationActorKind::Minion => minion_count += 1,
                _ => {}
            }
        }
        assert_eq!((structure_count, lane_tower_count, base_count), (8, 6, 2));
        assert_eq!(minion_count, 18);
        assert_eq!(lane_tower_count + minion_count, 24);
        assert_eq!(owner_counts.len(), 26);
        assert!(owner_counts.values().all(|count| *count == 1));
        let visual_positions = app
            .world_mut()
            .query::<(&PresentationActorVisual, &Transform)>()
            .iter(app.world())
            .map(|(visual, transform)| (*visual, *transform))
            .collect::<Vec<_>>();
        for (visual, transform) in visual_positions {
            let owner = app
                .world()
                .get::<Transform>(visual.owner)
                .expect("primary proxy owner must be live");
            let xy = simulation_xz_to_render_xy(owner.translation);
            let anchor = Vec2::new(
                (0.5 - visual.pivot[0]) * visual.world_height,
                (0.5 - visual.pivot[1]) * visual.world_height,
            );
            assert!(transform.translation.xy().distance(xy + anchor) < 0.001);
        }
        assert_eq!(
            app.world_mut()
                .query::<&PresentationActorCue>()
                .iter(app.world())
                .count(),
            34,
            "eight structures have two cues and 18 minions have one"
        );

        app.update();
        assert_eq!(primary_and_cue_counts(&mut app), (26, 34));
        app.world_mut().entity_mut(minions[0]).despawn();
        app.update();
        assert_eq!(primary_and_cue_counts(&mut app), (25, 33));

        // Snapshot omission/reconnect teardown removes every owner proxy and
        // fixed child cue before a fresh authoritative set is recreated.
        despawn_owners(
            &mut app,
            structures.into_iter().chain(minions.into_iter().skip(1)),
        );
        app.update();
        assert_eq!(primary_and_cue_counts(&mut app), (0, 0));

        let actor_definition_count = app.world().resource::<Presentation2dAssets>().actors.len();
        for wave in 1..=6 {
            let (structures, minions) =
                spawn_structure_and_wave_fixture(&mut app, wave as f32 * 0.25);
            app.update();
            assert_eq!(
                primary_and_cue_counts(&mut app),
                (26, 34),
                "wave {wave} must remain at the live-owner-derived bound"
            );
            assert_eq!(
                app.world().resource::<Presentation2dAssets>().actors.len(),
                actor_definition_count,
                "wave replay must not grow the presentation asset cache"
            );
            despawn_owners(&mut app, structures.into_iter().chain(minions));
            app.update();
            assert_eq!(
                primary_and_cue_counts(&mut app),
                (0, 0),
                "wave {wave} teardown must leave no orphan proxies or cues"
            );
        }

        let _recreated = spawn_structure_and_wave_fixture(&mut app, 2.0);
        app.update();
        assert_eq!(primary_and_cue_counts(&mut app), (26, 34));
    }

    #[test]
    fn minion_authoritative_state_and_movement_update_frame_facing_and_position() {
        let mut app = proxy_test_app(PlayerVisualMode::Sprite2d);
        let owner = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.5, 0.0),
                NetworkMinion,
                NetworkMinionBrainState(MinionBrainState::Marching),
                Team::Green,
            ))
            .id();
        app.update();

        let march_frame =
            app.world().resource::<Presentation2dAssets>().actors["green_minion_march"].frame;
        let idle_frame =
            app.world().resource::<Presentation2dAssets>().actors["green_minion_idle"].frame;
        let read_visual = |app: &mut App| {
            let (visual, transform, sprite) = app
                .world_mut()
                .query::<(&PresentationActorVisual, &Transform, &Sprite)>()
                .single(app.world())
                .expect("one minion primary proxy");
            (
                visual.previous_xy,
                transform.translation.xy(),
                sprite.flip_x,
                sprite
                    .texture_atlas
                    .as_ref()
                    .expect("minion uses the actor atlas")
                    .index,
            )
        };
        let (_, _, initial_flip, initial_frame) = read_visual(&mut app);
        assert!(!initial_flip);
        assert_eq!(initial_frame, march_frame);

        app.world_mut()
            .get_mut::<Transform>(owner)
            .unwrap()
            .translation
            .x = 3.0;
        app.update();
        let (previous_xy, visual_xy, moving_right_flip, moving_frame) = read_visual(&mut app);
        assert_eq!(previous_xy, Vec2::new(3.0, 0.0));
        assert!(visual_xy.x > 2.9);
        assert!(!moving_right_flip);
        assert_eq!(moving_frame, march_frame);

        app.world_mut()
            .get_mut::<Transform>(owner)
            .unwrap()
            .translation
            .x = -2.0;
        app.world_mut()
            .get_mut::<NetworkMinionBrainState>(owner)
            .unwrap()
            .0 = MinionBrainState::Attacking;
        app.update();
        let (previous_xy, visual_xy, moving_left_flip, attacking_frame) = read_visual(&mut app);
        assert_eq!(previous_xy, Vec2::new(-2.0, 0.0));
        assert!(visual_xy.x < -1.9);
        assert!(moving_left_flip);
        assert_eq!(attacking_frame, idle_frame);
        assert_eq!(primary_and_cue_counts(&mut app), (1, 1));
    }

    #[test]
    fn cues_encode_team_shape_and_all_three_tower_lanes() {
        let mut app = proxy_test_app(PlayerVisualMode::Sprite2d);
        let layout = MapLayout::default();
        for points in layout.lane_polylines() {
            for (team, fraction) in [(Team::Green, 0.30), (Team::Blue, 0.70)] {
                let anchor = sample_polyline(&points, fraction);
                app.world_mut().spawn((
                    Transform::from_xyz(anchor.x, 3.0, anchor.y),
                    NetworkStructure,
                    team,
                    StructureKind::Tower,
                ));
            }
        }
        app.update();

        let mut green_badges = 0;
        let mut blue_badges = 0;
        let mut lane_labels = HashMap::<TowerLane, usize>::new();
        for (cue, transform) in app
            .world_mut()
            .query::<(&PresentationActorCue, &Transform)>()
            .iter(app.world())
        {
            match cue.kind {
                PresentationCueKind::TeamBadge(Team::Green) => {
                    green_badges += 1;
                    assert_eq!(transform.rotation, Quat::IDENTITY);
                }
                PresentationCueKind::TeamBadge(Team::Blue) => {
                    blue_badges += 1;
                    assert_ne!(transform.rotation, Quat::IDENTITY);
                }
                PresentationCueKind::LaneLabel(lane) => {
                    *lane_labels.entry(lane).or_default() += 1;
                }
                PresentationCueKind::BaseLabel => panic!("lane towers must not use BASE cues"),
            }
        }
        assert_eq!((green_badges, blue_badges), (3, 3));
        assert_eq!(lane_labels.get(&TowerLane::Top), Some(&2));
        assert_eq!(lane_labels.get(&TowerLane::Mid), Some(&2));
        assert_eq!(lane_labels.get(&TowerLane::Bot), Some(&2));
    }

    #[test]
    fn minion_badges_use_team_color_and_different_shapes() {
        let mut app = proxy_test_app(PlayerVisualMode::Sprite2d);
        for team in [Team::Green, Team::Blue] {
            app.world_mut().spawn((
                Transform::default(),
                NetworkMinion,
                NetworkMinionBrainState(MinionBrainState::Marching),
                team,
            ));
        }
        app.update();

        let badges = app
            .world_mut()
            .query::<(&PresentationActorCue, &Transform, &Sprite)>()
            .iter(app.world())
            .map(|(cue, transform, sprite)| (cue.kind, transform.rotation, sprite.color))
            .collect::<Vec<_>>();
        assert_eq!(badges.len(), 2);
        let green = badges
            .iter()
            .find(|(kind, _, _)| *kind == PresentationCueKind::TeamBadge(Team::Green))
            .expect("green minion badge");
        let blue = badges
            .iter()
            .find(|(kind, _, _)| *kind == PresentationCueKind::TeamBadge(Team::Blue))
            .expect("blue minion badge");
        assert_eq!(green.1, Quat::IDENTITY, "Green uses an axis-aligned square");
        assert_ne!(blue.1, Quat::IDENTITY, "Blue uses a diamond");
        assert_ne!(green.2, blue.2, "team shape cue also retains team hue");
    }

    #[test]
    fn models3d_does_not_attach_2d_actor_proxies_or_cues() {
        let mut app = proxy_test_app(PlayerVisualMode::Models3d);
        app.world_mut().spawn((
            Transform::default(),
            NetworkStructure,
            Team::Green,
            StructureKind::Tower,
        ));
        app.world_mut().spawn((
            Transform::default(),
            NetworkMinion,
            NetworkMinionBrainState(MinionBrainState::Marching),
            Team::Blue,
        ));
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&PresentationActorVisual>()
                .iter(app.world())
                .count(),
            0
        );
        assert_eq!(
            app.world_mut()
                .query::<&PresentationActorCue>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn vfx_overflow_evicts_oldest_at_the_hard_cap() {
        let mut live = LivePresentationEffects(
            (1..=TRANSIENT_VFX_BUDGET)
                .map(|bits| Entity::from_bits(bits as u64))
                .collect(),
        );
        assert_eq!(
            evict_oldest_effect_if_full(&mut live),
            Some(Entity::from_bits(1))
        );
        assert_eq!(live.0.len(), TRANSIENT_VFX_BUDGET - 1);
        assert_eq!(evict_oldest_effect_if_full(&mut live), None);
    }
}
