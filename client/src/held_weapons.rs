//! The same bone attachment path serves packaged and SDK-imported handhelds.
use crate::{
    humanoid::{HumanoidRuntimeLibrary, RuntimeHumanoidPlayer},
    net::{NetworkHeroClass, PlayerHandheld},
};
use bevy::prelude::*;
#[derive(Component, Debug)]
pub(crate) struct HeldWeapon {
    pub owner: Entity,
    pub root: Entity,
    pub hand: Entity,
    pub id: String,
}
pub(crate) struct HeldWeaponsPlugin;
impl Plugin for HeldWeaponsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            sync_held_weapons
                .after(crate::humanoid::bind_runtime_humanoids)
                .before(bevy::transform::TransformSystems::Propagate),
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
            let mode = loadout
                .and_then(|l| l.0.as_ref())
                .map_or(shared::loadout::WeaponMode::Repeater, |state| {
                    state.weapon_mode
                });
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
            wanted.insert(root, (owner, hand, def, frame.mul_transform(offset)));
        }
    }
    for (entity, old) in &attachments {
        if wanted.get(&old.root).is_some_and(|(owner, hand, def, _)| {
            *owner == old.owner && *hand == old.hand && def.id == old.id
        }) {
            wanted.remove(&old.root);
        } else {
            commands.entity(entity).despawn();
        }
    }
    for (root, (owner, hand, def, transform)) in wanted {
        commands.spawn((
            HeldWeapon {
                owner,
                root,
                hand,
                id: def.id.clone(),
            },
            Name::new(format!("Handheld-{}", def.id)),
            SceneRoot(assets.load(format!("{}#Scene0", def.model))),
            transform,
            Visibility::Inherited,
            ChildOf(hand),
        ));
    }
}
