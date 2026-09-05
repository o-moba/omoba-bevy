//! True-2D presentation proxies for non-player actors and bounded combat VFX.

use bevy::prelude::*;
use serde::Deserialize;
use shared::PlayerActionKind;
use std::collections::{HashMap, VecDeque};

use crate::bosses::BossVisual;
use crate::combat::CombatStats;
use crate::maps::MapLayout;
use crate::net::{
    MinionBrainState, NetworkMinion, NetworkMinionBrainState, NetworkNeutral, NetworkProjectile,
    NetworkStructure, NeutralAiState, NeutralAiStateTag, PlayerCosmeticAction, RemotePlayer,
    StructureKind,
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
    hp: f32,
    alive: bool,
    action_sequence: u64,
}

#[derive(Resource, Default)]
struct PreviousCombatStates(HashMap<Entity, PreviousCombatState>);

pub struct Presentation2dPlugin;

impl Plugin for Presentation2dPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Presentation2dAssets>()
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
        (Entity, &Transform, &Team, &NetworkMinionBrainState),
        (With<NetworkMinion>, Without<PresentationActorRoot>),
    >,
) {
    if *mode != PlayerVisualMode::Sprite2d {
        return;
    }
    for (entity, transform, team, state) in &roots {
        if let Some((visual, world_height)) = attach_actor(
            &mut commands,
            &assets,
            entity,
            transform.translation,
            PresentationActorKind::Minion,
            minion_key(*team, state.0),
        ) {
            spawn_minion_cue(&mut commands, visual, entity, *team, world_height);
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

fn attach_projectile_visuals(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    assets: Res<Presentation2dAssets>,
    roots: Query<(Entity, &Transform, &NetworkProjectile), Without<PresentationActorRoot>>,
) {
    if *mode != PlayerVisualMode::Sprite2d {
        return;
    }
    for (entity, transform, projectile) in &roots {
        commands
            .entity(entity)
            .remove::<Mesh3d>()
            .remove::<MeshMaterial3d<StandardMaterial>>();
        let key = match projectile.owner_team {
            Team::Green => "green_projectile",
            Team::Blue => "blue_projectile",
        };
        let _ = attach_actor(
            &mut commands,
            &assets,
            entity,
            transform.translation,
            PresentationActorKind::Projectile,
            key,
        );
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
            hp: stats.hp,
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
            } else if current.hp < old.hp {
                spawn_effect(
                    &mut commands,
                    &assets,
                    &mut live,
                    "hit",
                    *team,
                    transform.translation,
                );
            } else if current.hp > old.hp {
                spawn_effect(
                    &mut commands,
                    &assets,
                    &mut live,
                    "heal",
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
