//! Client-side raid-boss presentation (TASK-19).
//!
//! The server replicates bosses as neutrals with boss camp types; this module
//! renders them with their staged GLB models (`client/assets/bosses/`), scaled
//! up for raid presence, adds a floating nameplate, and drives the embedded
//! idle/walk clips from the replicated AI state. The HP bar comes for free via
//! the shared `CombatStats`/`CombatBars` pipeline.

use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::scene::SceneRoot;
use std::collections::HashMap;

use crate::camera::MainCamera;
use crate::net::{NetworkNeutral, NeutralAiState, NeutralAiStateTag, NeutralCampType};
use crate::model_scale::{ModelScaleSource, NormalizeModelScale};

/// Raid bosses render this many times taller than the normalized player model
/// (spec D7: 2.5-3.5x for raid presence).
pub const BOSS_MODEL_HEIGHT_SCALE: f32 = 3.0;

/// World-space clearance between the model top and the nameplate anchor.
const NAMEPLATE_CLEARANCE: f32 = 0.55;
/// Fallback anchor height while the boss model is still loading.
const NAMEPLATE_FALLBACK_HEIGHT: f32 = 1.2;
/// Horizontal centering offset for the projected nameplate text (px).
const NAMEPLATE_HALF_WIDTH: f32 = 56.0;
const NAMEPLATE_COLOR: Color = Color::srgba(1.0, 0.82, 0.35, 1.0);

/// Marks a networked neutral entity as a raid boss and names its model.
#[derive(Component, Clone, Copy, Debug)]
pub struct BossVisual {
    pub camp_type: NeutralCampType,
}

/// Screen-space nameplate following one boss entity.
#[derive(Component)]
struct BossNameplate {
    boss: Entity,
}

/// Display name shown on the boss nameplate.
pub fn boss_display_name(camp_type: NeutralCampType) -> &'static str {
    match camp_type {
        NeutralCampType::WendigoBoss => "Wendigo",
        NeutralCampType::KingMutatioBoss => "King Mutatio",
        _ => "Neutral",
    }
}

/// Asset path slug under `client/assets/bosses/` for a boss camp type.
fn boss_slug(camp_type: NeutralCampType) -> Option<&'static str> {
    match camp_type {
        NeutralCampType::WendigoBoss => Some("wendigo-hollow"),
        NeutralCampType::KingMutatioBoss => Some("king-mutatio"),
        _ => None,
    }
}

/// Preloaded handles for the two boss GLBs (scene for rendering, gltf for the
/// embedded animation clips). Analogous to the roster `AvatarAssetCache`, but
/// eager: there are exactly two known bosses.
#[derive(Resource, Default)]
pub struct BossAssetCache {
    handles: HashMap<NeutralCampType, (Handle<Scene>, Handle<Gltf>)>,
}

#[derive(Clone)]
struct BossAnimationSet {
    graph: Handle<AnimationGraph>,
    idle_node: AnimationNodeIndex,
    walk_node: AnimationNodeIndex,
}

/// Idle/walk animation graphs per boss camp type, built lazily once the boss
/// GLTF assets finish loading.
#[derive(Resource, Default)]
struct BossAnimationLibrary {
    sets: HashMap<NeutralCampType, BossAnimationSet>,
    evaluated: HashMap<NeutralCampType, Handle<Gltf>>,
}

/// Binds an `AnimationPlayer` inside a boss scene to its owning boss root.
#[derive(Component)]
struct BossAnimationBinding {
    owner: Entity,
    camp_type: NeutralCampType,
    state: NeutralAiState,
}

pub struct BossesPlugin;

impl Plugin for BossesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BossAnimationLibrary>()
            .add_systems(Startup, load_boss_assets)
            .add_systems(
                Update,
                (
                    attach_boss_models,
                    setup_boss_animation_library,
                    bind_boss_animation_players,
                    sync_boss_animation_state,
                    force_boss_models_double_sided,
                    update_boss_nameplates,
                ),
            );
    }
}

fn load_boss_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    let mut cache = BossAssetCache::default();
    for camp_type in [
        NeutralCampType::WendigoBoss,
        NeutralCampType::KingMutatioBoss,
    ] {
        let Some(slug) = boss_slug(camp_type) else {
            continue;
        };
        info!("Loading raid-boss model '{slug}'");
        cache.handles.insert(
            camp_type,
            (
                asset_server.load(format!("bosses/{slug}.glb#Scene0")),
                asset_server.load(format!("bosses/{slug}.glb")),
            ),
        );
    }
    commands.insert_resource(cache);
}

/// Attaches the staged GLB scene (scaled up) and a nameplate to every freshly
/// replicated boss entity.
fn attach_boss_models(
    mut commands: Commands,
    cache: Res<BossAssetCache>,
    new_bosses: Query<(Entity, &BossVisual), Added<BossVisual>>,
) {
    for (boss_entity, visual) in &new_bosses {
        let Some((scene, gltf)) = cache.handles.get(&visual.camp_type) else {
            warn!("No staged model for boss {:?}", visual.camp_type);
            continue;
        };
        commands
            .entity(boss_entity)
            .insert(NormalizeModelScale::scaled_by(BOSS_MODEL_HEIGHT_SCALE))
            .insert(ModelScaleSource {
                gltf: gltf.clone(),
                key: boss_slug(visual.camp_type)
                    .unwrap_or("unknown-boss")
                    .to_owned(),
            })
            .with_children(|parent| {
                parent.spawn((
                    SceneRoot(scene.clone()),
                    // Staged VRM-derived GLBs face -Z while the server yaw
                    // convention points +Z at the move/attack direction.
                    Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
                    Visibility::default(),
                    Name::new("BossModel"),
                ));
            });

        let display_name = boss_display_name(visual.camp_type);
        info!(
            "Boss spawned on client: {display_name} ({:?})",
            visual.camp_type
        );
        commands.spawn((
            Text::new(display_name),
            TextFont {
                font_size: 18.0,
                ..default()
            },
            TextColor(NAMEPLATE_COLOR),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(-1000.0),
                top: Val::Px(-1000.0),
                ..default()
            },
            ZIndex(9),
            BossNameplate { boss: boss_entity },
            Name::new(format!("BossNameplate-{display_name}")),
        ));
    }
}

/// Projects each boss nameplate above its model top in screen space; the
/// nameplate despawns with its boss.
fn update_boss_nameplates(
    mut commands: Commands,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    bosses: Query<(&GlobalTransform, &NormalizeModelScale), With<BossVisual>>,
    mut nameplates: Query<(Entity, &BossNameplate, &mut Node, &mut Visibility)>,
) {
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };
    for (plate_entity, plate, mut node, mut visibility) in &mut nameplates {
        let Ok((boss_transform, normalization)) = bosses.get(plate.boss) else {
            commands.entity(plate_entity).despawn();
            continue;
        };
        let head_height = normalization
            .head_local_y
            .unwrap_or(NAMEPLATE_FALLBACK_HEIGHT);
        let anchor =
            boss_transform.translation() + Vec3::Y * (head_height + NAMEPLATE_CLEARANCE);
        match camera.world_to_viewport(camera_transform, anchor) {
            Ok(screen) => {
                node.left = Val::Px(screen.x - NAMEPLATE_HALF_WIDTH);
                node.top = Val::Px(screen.y);
                *visibility = Visibility::Visible;
            }
            Err(_) => {
                *visibility = Visibility::Hidden;
            }
        }
    }
}

/// Builds idle/walk animation graphs from the boss GLTFs once they load
/// (same clip-name resolution as the player avatar pipeline).
fn setup_boss_animation_library(
    cache: Option<Res<BossAssetCache>>,
    mut library: ResMut<BossAnimationLibrary>,
    gltf_assets: Res<Assets<Gltf>>,
    mut animation_graphs: ResMut<Assets<AnimationGraph>>,
) {
    let Some(cache) = cache else {
        return;
    };
    for (camp_type, (_scene, gltf_handle)) in &cache.handles {
        if library.evaluated.get(camp_type) == Some(gltf_handle) {
            continue;
        }
        let Some(gltf) = gltf_assets.get(gltf_handle) else {
            continue;
        };
        library.evaluated.insert(*camp_type, gltf_handle.clone());

        let find_clip = |needle: &str| -> Option<Handle<AnimationClip>> {
            gltf.named_animations
                .iter()
                .find(|(name, _)| name.to_ascii_lowercase().contains(needle))
                .map(|(_, handle)| handle.clone())
        };
        let (Some(idle_clip), Some(walk_clip)) = (find_clip("idle"), find_clip("walk")) else {
            warn!("idle/walk clips not found for boss {:?}", camp_type);
            continue;
        };
        let (graph, nodes) = AnimationGraph::from_clips([idle_clip, walk_clip]);
        let (Some(idle_node), Some(walk_node)) = (nodes.first().copied(), nodes.get(1).copied())
        else {
            continue;
        };
        let graph_handle = animation_graphs.add(graph);
        library.sets.insert(
            *camp_type,
            BossAnimationSet {
                graph: graph_handle,
                idle_node,
                walk_node,
            },
        );
        info!("Boss animation set ready for {:?} (idle/walk)", camp_type);
    }
}

/// Binds boss scene `AnimationPlayer`s to their boss root and starts idle.
fn bind_boss_animation_players(
    mut commands: Commands,
    library: Res<BossAnimationLibrary>,
    boss_roots: Query<&BossVisual, With<NetworkNeutral>>,
    child_of_query: Query<&ChildOf>,
    mut animation_players: Query<
        (Entity, &mut AnimationPlayer),
        Without<BossAnimationBinding>,
    >,
) {
    if library.sets.is_empty() {
        return;
    }
    for (animation_entity, mut animation_player) in &mut animation_players {
        let mut current = animation_entity;
        let owner = loop {
            if let Ok(visual) = boss_roots.get(current) {
                break Some((current, visual.camp_type));
            }
            let Ok(child_of) = child_of_query.get(current) else {
                break None;
            };
            current = child_of.parent();
        };
        let Some((owner, camp_type)) = owner else {
            continue;
        };
        let Some(set) = library.sets.get(&camp_type) else {
            continue;
        };
        animation_player.stop_all();
        animation_player.play(set.idle_node).repeat();
        commands.entity(animation_entity).insert((
            AnimationGraphHandle(set.graph.clone()),
            BossAnimationBinding {
                owner,
                camp_type,
                state: NeutralAiState::Idle,
            },
        ));
    }
}

/// Switches bosses between idle and walk based on the replicated AI state.
fn sync_boss_animation_state(
    library: Res<BossAnimationLibrary>,
    ai_states: Query<&NeutralAiStateTag, With<BossVisual>>,
    mut animation_query: Query<(&mut AnimationPlayer, &mut BossAnimationBinding)>,
) {
    for (mut animation_player, mut binding) in &mut animation_query {
        let Ok(ai_state) = ai_states.get(binding.owner) else {
            continue;
        };
        let Some(set) = library.sets.get(&binding.camp_type) else {
            continue;
        };
        let desired = ai_state.0;
        let node = match desired {
            NeutralAiState::Idle => set.idle_node,
            NeutralAiState::Aggro => set.walk_node,
        };
        if desired != binding.state {
            animation_player.stop_all();
            animation_player.play(node).repeat();
            binding.state = desired;
        } else if !animation_player.is_playing_animation(node) {
            animation_player.stop_all();
            animation_player.play(node).repeat();
        }
    }
}

/// Boss models are VRM-staged GLBs like the roster avatars: force their
/// materials double-sided so the meshes do not cull inside-out (same fix as
/// `world::force_vrm_models_double_sided`, scoped to boss roots).
fn force_boss_models_double_sided(
    roots: Query<Entity, With<BossVisual>>,
    children_query: Query<&Children>,
    material_handles: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut patched: Local<std::collections::HashSet<bevy::asset::AssetId<StandardMaterial>>>,
) {
    for root in &roots {
        for descendant in children_query.iter_descendants(root) {
            let Ok(handle) = material_handles.get(descendant) else {
                continue;
            };
            let id = handle.0.id();
            if patched.contains(&id) {
                continue;
            }
            if let Some(material) = materials.get_mut(&handle.0) {
                material.double_sided = true;
                material.cull_mode = None;
                patched.insert(id);
            }
        }
    }
}
