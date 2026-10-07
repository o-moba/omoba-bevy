//! What a hero's replicated flags report: one mesh visual per hero for the state that ranks
//! highest, and the recast marker of each slot the server offers again. The flags alone
//! select a visual and decide how long it stays. No skill, receipt, action or local timer
//! does, so a status the server did not report is never shown, and nothing is tinted by the
//! cause of a status, which is not replicated.
use super::bodies::{PartMesh, VfxMeshes};
use super::vocab::{RecastMarker, Silhouette};
use super::{SkillPresentation, category};
use crate::game_vfx::hdr_tint;
use crate::model_scale::{
    DEFAULT_MODEL_TARGET_HEIGHT, MAX_MODEL_TARGET_HEIGHT, MIN_MODEL_TARGET_HEIGHT,
    NormalizeModelScale,
};
use crate::net::{NetworkPlayerId, PlayerLoadout};
use crate::sprite::PlayerVisualMode;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use shared::loadout::{LoadoutState, PassiveEffect, PassiveId, SkillEffectState, SkillId};
use std::collections::{HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// Mesh parts the visual of one hero may have.
pub(crate) const MAX_PARTS: usize = 4;
/// Pips a concussion shows. The fourth stack stuns and clears the stacks in the same tick,
/// so it is never replicated.
pub(crate) const MAX_PIPS: u8 = 3;

/// Height above the head at which stars, the mark and the pips stand, clear of the bars.
const OVERHEAD: f32 = 0.85;
/// Three stars circle above a stunned hero.
const STUN_STAR: f32 = 0.78;
const STUN_ORBIT: f32 = 0.62;
const STUN_TURN: f32 = 3.4;
/// A ring with three teeth holds the feet of a rooted hero.
const ROOT_RING: f32 = 1.8;
const ROOT_TOOTH: Vec2 = Vec2::new(0.42, 0.8);
const ROOT_LIFT: f32 = 0.16;
/// Two plates turn ahead of a parrying hero, one to each side, where its body does not
/// hide them from a camera behind it.
const PARRY_KITE: f32 = 1.0;
const PARRY_AHEAD: f32 = 0.55;
const PARRY_APART: f32 = 0.8;
const PARRY_TURN: f32 = 5.0;
/// A band at the waist and a plate in front of a shielded hero.
const SHIELD_RING: f32 = 1.7;
const SHIELD_KITE: f32 = 0.95;
const SHIELD_AHEAD: f32 = 0.8;
/// The diamond of a mark at the end and at the start of the mark.
const MARK_SIZE: (f32, f32) = (0.45, 1.05);
/// Three shards circle the torso of a brittle hero.
const BRITTLE_SHARD: Vec3 = Vec3::new(0.3, 0.8, 0.3);
const BRITTLE_ORBIT: f32 = 0.8;
const BRITTLE_TURN: f32 = 1.8;
const BRITTLE_LEAN: f32 = 0.35;
/// The pips of a concussion and the distance between two of them.
const PIP: f32 = 0.6;
const PIP_STEP: f32 = 0.46;
/// Two chevrons sink beside the feet of a slowed hero, from the first height by the drop.
const SLOW_CHEVRON: f32 = 0.8;
const SLOW_APART: f32 = 0.62;
const SLOW_TOP: f32 = 0.95;
const SLOW_DROP: f32 = 0.45;
const SLOW_RATE: f32 = 0.9;
/// A low ring and two wisps turn at the feet of a camouflaged hero.
const VEIL_RING: f32 = 2.0;
const VEIL_WISP: f32 = 1.45;
const VEIL_LIFT: f32 = 0.14;
const VEIL_TURN: f32 = 1.3;
/// Three drops rise at the hands of a forging hero.
const FORGE_DROP: f32 = 0.6;
const FORGE_APART: f32 = 0.38;
const FORGE_AHEAD: f32 = 0.35;
const FORGE_RISE: f32 = 0.85;
const FORGE_RATE: f32 = 1.1;

/// A state of a hero that a replicated flag reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum StateVisual {
    Stunned,
    Rooted,
    ParryStance,
    Shielded,
    Marked,
    Brittle,
    Concussed,
    Slowed,
    CamouflageVeil,
    Forging,
}

impl StateVisual {
    /// Highest rank first. The first state a hero is in is the one its mesh visual shows.
    pub(crate) const PRIORITY: [Self; 10] = [
        Self::Stunned,
        Self::Rooted,
        Self::ParryStance,
        Self::Shielded,
        Self::Marked,
        Self::Brittle,
        Self::Concussed,
        Self::Slowed,
        Self::CamouflageVeil,
        Self::Forging,
    ];

    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Stunned => "stunned",
            Self::Rooted => "rooted",
            Self::ParryStance => "parry_stance",
            Self::Shielded => "shielded",
            Self::Marked => "marked",
            Self::Brittle => "brittle",
            Self::Concussed => "concussed",
            Self::Slowed => "slowed",
            Self::CamouflageVeil => "camouflage_veil",
            Self::Forging => "forging",
        }
    }

    /// Whether the flags of a hero report this state. The server reports a stun as a root
    /// of the same length too, so a stunned hero is not also `rooted`.
    pub(crate) fn active(self, flags: &LoadoutState) -> bool {
        match self {
            Self::Stunned => flags.stun_remaining_secs > 0.0,
            Self::Rooted => flags.root_remaining_secs > 0.0 && !Self::Stunned.active(flags),
            Self::ParryStance => flags.parrying,
            Self::Shielded => flags.shield_hp > 0.0,
            Self::Marked => flags.mark_remaining_secs > 0.0,
            Self::Brittle => flags.brittle,
            Self::Concussed => flags.concussion_stacks > 0,
            Self::Slowed => flags.slow_multiplier < 1.0,
            Self::CamouflageVeil => flags.camouflaged,
            Self::Forging => flags.forge_remaining_secs > 0.0,
        }
    }

    /// The states the flags report, highest rank first.
    pub(crate) fn of(flags: &LoadoutState) -> impl Iterator<Item = Self> + '_ {
        Self::PRIORITY
            .into_iter()
            .filter(|state| state.active(flags))
    }

    /// The state the mesh visual of a hero shows: the highest-ranking one of a living hero
    /// that is drawn with models. Every other state of the hero keeps its gizmo.
    pub(crate) fn shown(
        mode: PlayerVisualMode,
        visible: bool,
        alive: bool,
        flags: &LoadoutState,
    ) -> Option<Self> {
        (mode == PlayerVisualMode::Models3d && visible && alive)
            .then(|| Self::of(flags).next())
            .flatten()
    }
}

/// The six looks all state visuals share. None is the colour of a class or of a skill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum StatePaint {
    /// Stars and the mark.
    Gold,
    /// The root.
    Rose,
    /// The parry plates.
    Violet,
    /// The shield, the pips and the slow.
    Azure,
    /// The shards and the forge drops.
    Ember,
    /// The veil.
    Shade,
}

impl StatePaint {
    pub(crate) const ALL: [Self; 6] = [
        Self::Gold,
        Self::Rose,
        Self::Violet,
        Self::Azure,
        Self::Ember,
        Self::Shade,
    ];

    /// The colour and the HDR gain it is drawn with.
    const fn look(self) -> ([f32; 3], f32) {
        match self {
            Self::Gold => ([1.0, 0.74, 0.08], 4.5),
            Self::Rose => ([1.0, 0.14, 0.46], 4.0),
            Self::Violet => ([0.5, 0.3, 1.0], 4.5),
            Self::Azure => ([0.16, 0.72, 1.0], 4.0),
            Self::Ember => ([1.0, 0.4, 0.06], 4.5),
            Self::Shade => ([0.07, 0.04, 0.13], 1.0),
        }
    }

    /// A solid colour that hides what is behind it, so the outline of a part stays sharp.
    fn material(self) -> StandardMaterial {
        let (color, gain) = self.look();
        StandardMaterial {
            alpha_mode: AlphaMode::Opaque,
            ..super::effects::material(hdr_tint(Color::srgb_from_array(color), gain))
        }
    }
}

/// One mesh of a state visual, posed relative to the feet of its hero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StatePart {
    pub mesh: Silhouette,
    pub paint: StatePaint,
    pub pose: Transform,
}

/// What a visual is posed by besides the flags: how the hero stands and how it is seen.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Stance {
    /// The way the hero faces, on the ground.
    pub forward: Vec2,
    /// Standing height of the hero.
    pub height: f32,
    /// Rotation of the camera. Flat parts are turned to it, so a shape is never edge-on.
    pub facing: Quat,
    /// The presentation clock, in seconds.
    pub now: f32,
}

impl Stance {
    /// The rotation of the locked follow camera, which looks along +X from above.
    pub(crate) fn rest_facing() -> Quat {
        Transform::from_xyz(
            -crate::camera::CAMERA_DISTANCE,
            crate::camera::CAMERA_HEIGHT,
            0.0,
        )
        .looking_at(Vec3::ZERO, Vec3::Y)
        .rotation
    }
}

/// The longest a mark lasts (the Radiance passive); its diamond has its full size then.
fn mark_secs() -> f32 {
    let PassiveEffect::Radiance {
        mark_duration_secs, ..
    } = shared::loadout::passive(PassiveId::Radiance)
    else {
        return 0.0;
    };
    mark_duration_secs
}

/// The share of a mark that is left, from the replicated time alone. A catalog without a
/// mark duration leaves the diamond at its full size.
fn mark_left(flags: &LoadoutState) -> f32 {
    let full = mark_secs();
    if full > 0.0 {
        (flags.mark_remaining_secs / full).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// The meshes of one state, at most `MAX_PARTS`. Every size, place and motion is a function
/// of the hero's own flags, its stance and the clock; a part never leaves its hero.
pub(crate) fn state_parts(
    state: StateVisual,
    flags: &LoadoutState,
    stance: &Stance,
) -> Vec<StatePart> {
    use Silhouette as Mesh;
    use StatePaint as Paint;
    let Stance {
        forward,
        height,
        facing,
        now,
    } = *stance;
    let ahead = Vec3::new(forward.x, 0.0, forward.y);
    let beside = Vec3::new(-forward.y, 0.0, forward.x);
    // To the right on the screen.
    let across = facing * Vec3::X;
    // A flat mesh turned to the viewer, pointing `turn` from the right of the screen.
    let flat = |turn: f32| facing * Quat::from_rotation_z(turn);
    // A flat mesh or a torus laid on the ground.
    let level = Quat::from_rotation_x(-FRAC_PI_2);
    let around = |turn: f32, radius: f32, lift: f32| {
        Vec3::new(turn.cos() * radius, lift, turn.sin() * radius)
    };
    let part = |mesh, paint, translation: Vec3, rotation: Quat, scale: Vec3| StatePart {
        mesh,
        paint,
        pose: Transform {
            translation,
            rotation,
            scale,
        },
    };
    let third = |i: usize| i as f32 * TAU / 3.0;
    let mut parts: Vec<StatePart> = match state {
        StateVisual::Stunned => (0..3)
            .map(|i| {
                let turn = now * STUN_TURN + third(i);
                part(
                    Mesh::Star,
                    Paint::Gold,
                    around(turn, STUN_ORBIT, height + OVERHEAD),
                    flat(turn),
                    Vec3::splat(STUN_STAR),
                )
            })
            .collect(),
        StateVisual::Rooted => {
            let ring = part(
                Mesh::Torus,
                Paint::Rose,
                Vec3::Y * ROOT_LIFT,
                level,
                Vec3::splat(ROOT_RING),
            );
            // The teeth stand on the middle of the tube.
            let reach = ROOT_RING * (0.5 - super::bodies::TORUS_TUBE / 2.0);
            let teeth = (0..3).map(|i| {
                part(
                    Mesh::Cone,
                    Paint::Rose,
                    around(third(i), reach, ROOT_LIFT + ROOT_TOOTH.y / 2.0),
                    Quat::IDENTITY,
                    Vec3::new(ROOT_TOOTH.x, ROOT_TOOTH.y, ROOT_TOOTH.x),
                )
            });
            std::iter::once(ring).chain(teeth).collect()
        }
        StateVisual::ParryStance => [-1.0, 1.0]
            .into_iter()
            .map(|side: f32| {
                // Upright with its point down, turning about its own axis. The two turn
                // against each other a quarter turn apart, so one of them always shows
                // its face.
                let turn = (now * PARRY_TURN + FRAC_PI_2 * side.max(0.0)) * side;
                part(
                    Mesh::Kite,
                    Paint::Violet,
                    ahead * PARRY_AHEAD + beside * side * PARRY_APART + Vec3::Y * height * 0.55,
                    Quat::from_rotation_y(turn) * Quat::from_rotation_z(FRAC_PI_2),
                    Vec3::splat(PARRY_KITE),
                )
            })
            .collect(),
        StateVisual::Shielded => vec![
            part(
                Mesh::Torus,
                Paint::Azure,
                Vec3::Y * height * 0.5,
                level,
                Vec3::new(SHIELD_RING, SHIELD_RING, SHIELD_RING * 0.5),
            ),
            part(
                Mesh::Kite,
                Paint::Azure,
                ahead * SHIELD_AHEAD + Vec3::Y * height * 0.55,
                flat(FRAC_PI_2),
                Vec3::splat(SHIELD_KITE),
            ),
        ],
        StateVisual::Marked => {
            let left = mark_left(flags);
            vec![part(
                Mesh::Diamond,
                Paint::Gold,
                Vec3::Y * (height + OVERHEAD),
                flat(FRAC_PI_2),
                Vec3::splat(MARK_SIZE.0 + (MARK_SIZE.1 - MARK_SIZE.0) * left),
            )]
        }
        StateVisual::Brittle => (0..3)
            .map(|i| {
                let turn = now * BRITTLE_TURN + third(i);
                part(
                    Mesh::Shard,
                    Paint::Ember,
                    around(turn, BRITTLE_ORBIT, height * 0.55),
                    Quat::from_rotation_y(-turn) * Quat::from_rotation_z(BRITTLE_LEAN),
                    BRITTLE_SHARD,
                )
            })
            .collect(),
        // Three places from left to right: three pips read as a full meter.
        StateVisual::Concussed => (0..flags.concussion_stacks.min(MAX_PIPS))
            .map(|i| {
                part(
                    Mesh::Diamond,
                    Paint::Azure,
                    across * (f32::from(i) - 1.0) * PIP_STEP + Vec3::Y * (height + OVERHEAD),
                    flat(FRAC_PI_2),
                    Vec3::splat(PIP),
                )
            })
            .collect(),
        StateVisual::Slowed => [-1.0, 1.0]
            .into_iter()
            .map(|side: f32| {
                let sunk = (now * SLOW_RATE + side * 0.25).rem_euclid(1.0);
                part(
                    Mesh::Chevron,
                    Paint::Azure,
                    across * side * SLOW_APART + Vec3::Y * (SLOW_TOP - sunk * SLOW_DROP),
                    flat(-FRAC_PI_2),
                    Vec3::splat(SLOW_CHEVRON),
                )
            })
            .collect(),
        StateVisual::CamouflageVeil => {
            let ring = part(
                Mesh::Torus,
                Paint::Shade,
                Vec3::Y * VEIL_LIFT,
                level,
                Vec3::new(VEIL_RING, VEIL_RING, VEIL_RING * 0.35),
            );
            // Each wisp is an arc around the feet; the two face each other and turn.
            let wisps = [0.0, PI].into_iter().map(|offset| {
                part(
                    Mesh::Crescent,
                    Paint::Shade,
                    Vec3::Y * (VEIL_LIFT + 0.12),
                    Quat::from_rotation_y(-(now * VEIL_TURN + offset)) * level,
                    Vec3::splat(VEIL_WISP),
                )
            });
            std::iter::once(ring).chain(wisps).collect()
        }
        StateVisual::Forging => (0..3)
            .map(|i| {
                let risen = (now * FORGE_RATE + i as f32 / 3.0).rem_euclid(1.0);
                part(
                    Mesh::Drop,
                    Paint::Ember,
                    ahead * FORGE_AHEAD
                        + beside * (i as f32 - 1.0) * FORGE_APART
                        + Vec3::Y * (height * 0.45 + risen * FORGE_RISE),
                    flat(FRAC_PI_2),
                    Vec3::splat(FORGE_DROP * (1.0 - 0.6 * risen)),
                )
            })
            .collect(),
    };
    parts.truncate(MAX_PARTS);
    parts
}

/// Whether the hero stands where the server accepts the recast of `skill`. Mountain Echo is
/// redirected only within reach of one of its own effects, although its slot reports the
/// recast for much longer (`common/src/skills/advanced.rs:503-512`). An effect of a hidden
/// owner carries no owner id and opens nothing.
pub(crate) fn recast_in_reach(
    skill: SkillId,
    hero: u64,
    at: Vec2,
    effects: &[SkillEffectState],
) -> bool {
    category::recast_gate(skill).is_none_or(|reach| {
        hero != 0
            && effects.iter().any(|effect| {
                effect.owner_id == hero
                    && effect.skill == skill
                    && Vec2::from_array(effect.position).distance(at) <= reach
            })
    })
}

/// The recast marker of every slot whose recast the server offers the hero now, with the
/// colour of the skill in that slot. The slot of the accepted recipe names the skill, so
/// this is the one state that carries a skill's colour.
pub(crate) fn recast_markers(
    registry: &SkillPresentation,
    class: shared::HeroClass,
    flags: &LoadoutState,
    hero: u64,
    at: Vec2,
    effects: &[SkillEffectState],
) -> Vec<(RecastMarker, Color)> {
    (0u8..4)
        .filter(|slot| flags.slots[usize::from(*slot)].can_recast)
        .filter_map(|slot| {
            let skill = super::equipped_skill(class, Some(flags), slot)?;
            let profile = registry.profile(skill)?;
            let marker = profile.cast.as_ref()?.recast_marker?;
            recast_in_reach(skill, hero, at, effects)
                .then(|| (marker, Color::srgb_from_array(profile.color)))
        })
        .collect()
}

/// Evidence of the mesh visual of a hero's state.
#[cfg(feature = "qa")]
#[derive(Component)]
pub(crate) struct StateVisualShown {
    pub hero: u64,
    pub state: StateVisual,
    pub parts: usize,
}

pub(super) struct StatusVisualsPlugin;
impl Plugin for StatusVisualsPlugin {
    fn build(&self, app: &mut App) {
        crate::vfx_clock::ensure(app);
        app.init_resource::<StateVisuals>()
            .add_systems(Startup, setup)
            .add_systems(
                PostUpdate,
                sync_status_visuals.before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

/// The materials of `StatePaint::ALL`, in that order.
#[derive(Resource)]
struct StatePaints([Handle<StandardMaterial>; StatePaint::ALL.len()]);

fn setup(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(StatePaints(
        StatePaint::ALL.map(|paint| materials.add(paint.material())),
    ));
}

/// Where the feet of a hero are relative to the origin of its entity, and how tall it
/// stands, from the measured bottom and top of its model. The origin of a model is
/// wherever its file has it. A hero that is not measured yet is the stand-in cube around
/// its origin.
fn standing(foot: Option<f32>, head: Option<f32>) -> (f32, f32) {
    let feet = foot
        .filter(|feet| feet.is_finite())
        .unwrap_or(-crate::player::PLAYER_SIZE * 0.5);
    let height = head
        .map(|head| head - feet)
        .filter(|height| height.is_finite())
        .map_or(DEFAULT_MODEL_TARGET_HEIGHT, |height| {
            height.clamp(MIN_MODEL_TARGET_HEIGHT, MAX_MODEL_TARGET_HEIGHT)
        });
    (feet, height)
}

struct Shown {
    root: Entity,
    state: StateVisual,
    parts: Vec<Entity>,
}

/// The visual of each hero that has one, by the entity of the hero.
#[derive(Resource, Default)]
struct StateVisuals(HashMap<Entity, Shown>);

/// Draws the highest-ranking state of every living hero the client sees, in 3D only: in the
/// flat view no hero has a state to show here, so every visual is removed. A visual stands
/// at the feet of its hero as an entity of its own, so it never joins or moves the hero's
/// hierarchy, and it leaves in the frame the flag does.
fn sync_status_visuals(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    clock: Res<crate::vfx_clock::VfxClock>,
    meshes: Res<VfxMeshes>,
    paints: Res<StatePaints>,
    camera: Query<&GlobalTransform, With<crate::camera::MainCamera>>,
    heroes: Query<(
        Entity,
        &NetworkPlayerId,
        &Transform,
        &InheritedVisibility,
        &PlayerLoadout,
        Option<&crate::combat::CombatStats>,
        Option<&NormalizeModelScale>,
    )>,
    mut poses: Query<&mut Transform, Without<NetworkPlayerId>>,
    mut visuals: ResMut<StateVisuals>,
) {
    let facing = camera
        .single()
        .map_or_else(|_| Stance::rest_facing(), GlobalTransform::rotation);
    let mut live = HashSet::new();
    for (hero, id, pose, visible, loadout, stats, model) in &heroes {
        let Some(flags) = loadout.0.as_ref().filter(|_| pose.translation.is_finite()) else {
            continue;
        };
        let alive = stats.is_none_or(|stats| stats.is_alive());
        let Some(state) = StateVisual::shown(*mode, visible.get(), alive, flags) else {
            continue;
        };
        let (feet, height) = standing(
            model.and_then(NormalizeModelScale::foot_local_y),
            model.and_then(|model| model.head_local_y),
        );
        let stance = Stance {
            forward: pose.forward().xz().normalize_or(Vec2::NEG_Y),
            height,
            facing,
            now: clock.now as f32,
        };
        let parts = state_parts(state, flags, &stance);
        let stands = Transform::from_translation(pose.translation + Vec3::Y * feet);
        live.insert(hero);
        if let Some(shown) = visuals
            .0
            .get(&hero)
            .filter(|shown| shown.state == state && shown.parts.len() == parts.len())
        {
            let moved = std::iter::once((shown.root, stands)).chain(
                shown
                    .parts
                    .iter()
                    .copied()
                    .zip(parts.iter().map(|part| part.pose)),
            );
            for (entity, pose) in moved {
                if let Ok(mut current) = poses.get_mut(entity) {
                    *current = pose;
                }
            }
            continue;
        }
        if let Some(previous) = visuals.0.remove(&hero) {
            commands.entity(previous.root).despawn();
        }
        let root = commands
            .spawn((
                stands,
                Visibility::Inherited,
                Name::new(format!("StateVisual-{}-{}", state.id(), id.0)),
                #[cfg(feature = "qa")]
                StateVisualShown {
                    hero: id.0,
                    state,
                    parts: parts.len(),
                },
            ))
            .id();
        let parts = parts
            .iter()
            .map(|part| {
                commands
                    .spawn((
                        Mesh3d(meshes.handle(PartMesh::Silhouette(part.mesh))),
                        MeshMaterial3d(paints.0[part.paint as usize].clone()),
                        part.pose,
                        NotShadowCaster,
                        NotShadowReceiver,
                        ChildOf(root),
                    ))
                    .id()
            })
            .collect();
        visuals.0.insert(hero, Shown { root, state, parts });
    }
    visuals.0.retain(|hero, shown| {
        if live.contains(hero) {
            true
        } else {
            commands.entity(shown.root).despawn();
            false
        }
    });
}

#[cfg(test)]
mod tests;
