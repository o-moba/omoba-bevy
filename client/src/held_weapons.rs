//! The same bone attachment path serves packaged and SDK-imported handhelds.
use crate::skill_presentation::cast::CastKey;
use crate::{
    humanoid::{HumanoidRuntimeLibrary, RuntimeHumanoidPlayer},
    net::{NetworkHeroClass, PlayerHandheld},
};
use bevy::prelude::*;
use shared::loadout::SkillId;
#[derive(Component, Debug)]
pub(crate) struct HeldWeapon {
    pub owner: Entity,
    pub root: Entity,
    pub hand: Entity,
    pub id: String,
    pub model: String,
}
pub(crate) struct HeldWeaponsPlugin;
impl Plugin for HeldWeaponsPlugin {
    fn build(&self, app: &mut App) {
        crate::vfx_clock::ensure(app);
        app.init_resource::<WeaponActions>()
            .add_message::<crate::game_vfx::SkillBurst>()
            .add_systems(
                PostUpdate,
                (track_actions, sync_held_weapons, animate_parts)
                    .chain()
                    .after(crate::humanoid::bind_runtime_humanoids)
                    .before(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(
                PostUpdate,
                emit_muzzle.after(bevy::transform::TransformSystems::Propagate),
            );
    }
}
fn sync_held_weapons(
    mut commands: Commands,
    mode: Res<crate::sprite::PlayerVisualMode>,
    assets: Res<AssetServer>,
    library: Res<HumanoidRuntimeLibrary>,
    roots: Query<(Entity, &RuntimeHumanoidPlayer)>,
    actors: Query<(
        &NetworkHeroClass,
        &PlayerHandheld,
        Option<&crate::net::PlayerLoadout>,
    )>,
    parents: Query<&ChildOf>,
    attachments: Query<(Entity, &HeldWeapon)>,
    actions: Res<WeaponActions>,
) {
    let mut wanted = std::collections::HashMap::new();
    if *mode == crate::sprite::PlayerVisualMode::Models3d {
        for (root, rig) in &roots {
            let mut owner = root;
            let mut actor = None;
            for _ in 0..128 {
                if let Ok(a) = actors.get(owner) {
                    actor = Some(a);
                    break;
                }
                let Ok(parent) = parents.get(owner) else {
                    break;
                };
                owner = parent.parent();
            }
            let Some((class, selection, loadout)) = actor else {
                continue;
            };
            let mut mode = loadout
                .and_then(|l| l.0.as_ref())
                .map_or(shared::loadout::WeaponMode::Repeater, |state| {
                    state.weapon_mode
                });
            // An accepted ultimate draws the launcher even when Q is in repeater mode.
            // The actual loadout, cadence and mana accounting remain authoritative.
            if class.0 == shared::HeroClass::Wildspark
                && actions.0.get(&owner).is_some_and(|action| {
                    matches!(action.skill, Some(SkillId::WildRocket | SkillId::WildZap))
                        && action.age < 1.1
                })
            {
                mode = shared::loadout::WeaponMode::Rockets;
            }
            let Some(id) = selection.0.resolve_mode(class.0, mode) else {
                continue;
            };
            let Some(def) = omoba_passport::weapon_store::definition(id) else {
                continue;
            };
            let Some((node, frame)) = library.hand_socket(rig.model, &def.grip.bone) else {
                continue;
            };
            let Some(hand) = rig.joint(node) else {
                continue;
            };
            let r = def.grip.rotation_degrees.map(f32::to_radians);
            let offset = Transform {
                translation: Vec3::from_array(def.grip.offset),
                rotation: Quat::from_euler(EulerRot::XYZ, r[0], r[1], r[2]),
                scale: Vec3::splat(def.grip.scale),
            };
            let model = if class.0 == shared::HeroClass::Wildspark
                && selection.0.is_default()
                && def.id == "wild-repeater"
            {
                "weapons/wildspark-repeater.glb".to_owned()
            } else {
                def.model.clone()
            };
            wanted.insert(root, (owner, hand, def, frame.mul_transform(offset), model));
        }
    }
    for (entity, old) in &attachments {
        if wanted
            .get(&old.root)
            .is_some_and(|(owner, hand, def, _, model)| {
                *owner == old.owner && *hand == old.hand && def.id == old.id && *model == old.model
            })
        {
            wanted.remove(&old.root);
        } else {
            commands.entity(entity).despawn();
        }
    }
    for (root, (owner, hand, def, transform, model)) in wanted {
        commands.spawn((
            HeldWeapon {
                owner,
                root,
                hand,
                id: def.id.clone(),
                model: model.clone(),
            },
            Name::new(format!("Handheld-{}", def.id)),
            WorldAssetRoot(assets.load(format!("{model}#Scene0"))),
            transform,
            WeaponPose(transform),
            Visibility::Inherited,
            ChildOf(hand),
        ));
    }
}

#[derive(Component)]
struct WeaponPose(Transform);

struct ActionTime {
    sequence: u64,
    slot: u8,
    skill: Option<SkillId>,
    age: f32,
    rotor: f32,
    muzzle_sequence: u64,
}
#[derive(Resource, Default)]
struct WeaponActions(std::collections::HashMap<Entity, ActionTime>);

fn track_actions(
    clock: Res<crate::vfx_clock::VfxClock>,
    actors: Query<(
        Entity,
        &NetworkHeroClass,
        &crate::net::PlayerCosmeticAction,
        Option<&crate::net::PlayerLoadout>,
    )>,
    mut actions: ResMut<WeaponActions>,
) {
    actions.0.retain(|entity, _| {
        actors
            .get(*entity)
            .is_ok_and(|(_, class, _, _)| class.0 == shared::HeroClass::Wildspark)
    });
    for (entity, class, action, loadout) in &actors {
        if class.0 != shared::HeroClass::Wildspark {
            continue;
        }
        let state = actions.0.entry(entity).or_insert(ActionTime {
            sequence: action.sequence,
            slot: u8::MAX,
            skill: None,
            age: 10.0,
            rotor: 0.0,
            muzzle_sequence: action.sequence,
        });
        state.age = (state.age + clock.delta).min(10.0);
        if action.sequence != state.sequence {
            state.sequence = action.sequence;
            if action.kind == shared::PlayerActionKind::None {
                state.slot = u8::MAX;
                state.skill = None;
                state.age = 10.0;
                state.muzzle_sequence = action.sequence;
                continue;
            }
            state.slot = action.slot;
            state.skill = CastKey::of(class.0, loadout.and_then(|l| l.0.as_ref()), action.slot)
                .and_then(CastKey::skill)
                .and_then(|key| key.modular());
            state.age = 0.0;
        }
        if state.slot == shared::BASIC_ATTACK_ACTION_SLOT {
            state.rotor = (state.rotor + clock.delta * 32.0 * (1.0 - state.age / 0.6).max(0.0))
                .rem_euclid(std::f32::consts::TAU);
        }
    }
}

/// The hand has reached the shot pose before the barrel flash is emitted. Its origin
/// is the actual socket transform, including the avatar's proportions and recoil.
fn emit_muzzle(
    attachments: Query<(&HeldWeapon, &GlobalTransform, &InheritedVisibility)>,
    mut actions: ResMut<WeaponActions>,
    mut bursts: MessageWriter<crate::game_vfx::SkillBurst>,
) {
    use crate::game_vfx::{Curve, ParticleSource, ParticleSpec, SkillBurst, Tint};
    for (weapon, pose, visible) in &attachments {
        let Some(action) = actions.0.get_mut(&weapon.owner) else {
            continue;
        };
        if !visible.get() || action.muzzle_sequence == action.sequence || action.age < 0.08 {
            continue;
        }
        action.muzzle_sequence = action.sequence;
        if action.age > 0.25
            || !(action.slot == shared::BASIC_ATTACK_ACTION_SLOT
                || matches!(action.skill, Some(SkillId::WildZap | SkillId::WildRocket)))
        {
            continue;
        }
        let muzzle = match weapon.model.as_str() {
            "weapons/wildspark-repeater.glb" => Vec3::new(0.0, 0.12, 0.735),
            "weapons/wild-launcher.glb" => Vec3::new(0.0, 0.13, 0.81),
            _ => continue,
        };
        let origin = pose.transform_point(muzzle);
        let forward = (origin - pose.transform_point(muzzle - Vec3::Z)).normalize_or_zero();
        let color = if action.skill == Some(SkillId::WildZap) {
            Color::srgb(0.12, 0.7, 1.0)
        } else {
            Color::srgb(1.0, 0.7, 0.16)
        };
        bursts.write(SkillBurst(vec![
            ParticleSpec {
                event_id: action.sequence,
                origin,
                lifetime: 0.09,
                size: 0.32,
                color: Tint { color, gain: 2.5 },
                source: ParticleSource::Accent,
                curve: Curve::Shrink,
                ..ParticleSpec::BASE
            },
            ParticleSpec {
                event_id: action.sequence,
                origin,
                velocity: forward * 3.0,
                lifetime: 0.12,
                size: 0.16,
                color: Tint { color, gain: 1.5 },
                source: ParticleSource::Accent,
                ..ParticleSpec::BASE
            },
        ]));
    }
}

fn kick(age: f32) -> f32 {
    // Fast compression followed by a slower spring return; no motion while paused.
    (age / 0.035).clamp(0.0, 1.0) * (1.0 - age / 0.32).clamp(0.0, 1.0).powi(2)
}

fn animate_parts(
    actions: Res<WeaponActions>,
    attachments: Query<(Entity, &HeldWeapon, &WeaponPose)>,
    children: Query<&Children>,
    mut parts: Query<(Option<&Name>, &mut Transform)>,
) {
    for (entity, weapon, base) in &attachments {
        if !matches!(weapon.id.as_str(), "wild-repeater" | "wild-launcher") {
            continue;
        }
        let Some(action) = actions.0.get(&weapon.owner) else {
            continue;
        };
        let firing = action.slot == shared::BASIC_ATTACK_ACTION_SLOT
            || matches!(action.skill, Some(SkillId::WildZap | SkillId::WildRocket));
        let recoil = if firing { kick(action.age) } else { 0.0 };
        if let Ok((_, mut pose)) = parts.get_mut(entity) {
            *pose = base.0;
            pose.rotation *= Quat::from_rotation_x(-0.11 * recoil);
            // Lower and seat the newly selected weapon on Q.
            if action.skill == Some(SkillId::WildSwitch) && action.age < 0.3 {
                pose.rotation *= Quat::from_rotation_x(0.3 * (1.0 - action.age / 0.3));
            }
        }
        for child in children.iter_descendants(entity) {
            let Ok((Some(name), mut pose)) = parts.get_mut(child) else {
                continue;
            };
            match name.as_str() {
                "WildsparkRotor" => pose.rotation = Quat::from_rotation_z(action.rotor),
                "WildsparkSlide" => pose.translation.z = 0.20 - recoil * 0.095,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mechanical_motion_starts_on_an_accepted_edge_and_freezes_with_the_sandbox() {
        let mut app = App::new();
        app.init_resource::<WeaponActions>()
            .insert_resource(crate::vfx_clock::VfxClock {
                now: 1.0,
                delta: 0.05,
            })
            .add_systems(Update, track_actions);
        let entity = app
            .world_mut()
            .spawn((
                NetworkHeroClass(shared::HeroClass::Wildspark),
                crate::net::PlayerCosmeticAction {
                    sequence: 7,
                    kind: shared::PlayerActionKind::Attack,
                    slot: shared::BASIC_ATTACK_ACTION_SLOT,
                },
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().resource::<WeaponActions>().0[&entity].rotor,
            0.0,
            "retained action must not replay on join"
        );
        app.world_mut()
            .get_mut::<crate::net::PlayerCosmeticAction>(entity)
            .unwrap()
            .sequence += 1;
        app.update();
        let turn = app.world().resource::<WeaponActions>().0[&entity].rotor;
        assert!(turn > 0.0);
        app.world_mut()
            .resource_mut::<crate::vfx_clock::VfxClock>()
            .delta = 0.0;
        app.update();
        assert_eq!(
            app.world().resource::<WeaponActions>().0[&entity].rotor,
            turn
        );
        app.world_mut()
            .resource_mut::<crate::vfx_clock::VfxClock>()
            .delta = 0.1;
        app.update();
        assert!(
            app.world().resource::<WeaponActions>().0[&entity].age >= 0.1,
            "repeated snapshots must not restart recoil"
        );
        app.world_mut().despawn(entity);
        app.update();
        assert!(app.world().resource::<WeaponActions>().0.is_empty());
    }
}
