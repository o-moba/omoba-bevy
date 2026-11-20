//! The same bone attachment path serves packaged and SDK-imported handhelds.
use crate::skill_presentation::cast::CastKey;
use crate::{
    humanoid::{HumanoidRuntimeLibrary, RuntimeHumanoidPlayer},
    net::{NetworkHeroClass, PlayerHandheld},
};
use bevy::prelude::*;
use omoba_passport::weapons::{Weapon, WildsparkSetHint};
use shared::{
    handheld::HandheldSelection,
    loadout::{LoadoutState, SkillId, WeaponMode},
};
#[derive(Component, Debug)]
pub(crate) struct HeldWeapon {
    pub owner: Entity,
    pub root: Entity,
    pub hand: Entity,
    pub id: String,
    pub model: String,
    /// The owner's shown gun mode; always the repeater for other classes. It
    /// changes in place, so a set model swaps variants without reloading.
    pub mode: WeaponMode,
    /// Muzzle tips embedded in an installed SDK set; packaged props have none.
    pub hint: Option<WildsparkSetHint>,
}
pub(crate) struct HeldWeaponsPlugin;
impl Plugin for HeldWeaponsPlugin {
    fn build(&self, app: &mut App) {
        crate::vfx_clock::ensure(app);
        app.init_resource::<WeaponActions>()
            .add_message::<crate::game_vfx::SkillBurst>()
            .add_systems(
                PostUpdate,
                (
                    track_actions,
                    sync_held_weapons,
                    bind_parts,
                    show_set_variant,
                    animate_parts,
                )
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
    mut attachments: Query<(Entity, &mut HeldWeapon)>,
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
            let mode = shown_mode(
                class.0,
                loadout.and_then(|l| l.0.as_ref()),
                actions.0.get(&owner),
            );
            let Some((def, class_default)) = resolve_held(
                &selection.0,
                class.0,
                mode,
                omoba_passport::weapon_store::definition,
            ) else {
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
            let model = shown_model(class.0, class_default, &def);
            wanted.insert(
                root,
                (owner, hand, def, frame.mul_transform(offset), model, mode),
            );
        }
    }
    for (entity, mut old) in &mut attachments {
        let kept = wanted
            .get(&old.root)
            .and_then(|(owner, hand, def, _, model, mode)| {
                (*owner == old.owner
                    && *hand == old.hand
                    && def.id == old.id
                    && *model == old.model)
                    .then_some(*mode)
            });
        if let Some(mode) = kept {
            if old.mode != mode {
                old.mode = mode;
            }
            wanted.remove(&old.root);
        } else {
            commands.entity(entity).despawn();
        }
    }
    for (root, (owner, hand, def, transform, model, mode)) in wanted {
        commands.spawn((
            HeldWeapon {
                owner,
                root,
                hand,
                id: def.id.clone(),
                model: model.clone(),
                mode,
                hint: omoba_passport::weapon_store::wildspark_set_hint(&def.id),
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

/// The model a held definition is drawn with: Wildspark's own repeater for its
/// class-default repeater, else the definition's model.
pub(crate) fn shown_model(class: shared::HeroClass, class_default: bool, def: &Weapon) -> String {
    if class == shared::HeroClass::Wildspark && class_default && def.id == "wild-repeater" {
        "weapons/wildspark-repeater.glb".to_owned()
    } else {
        def.model.clone()
    }
}

/// Only Wildspark switches guns. An accepted ultimate draws the launcher even
/// when Q is in repeater mode; the actual loadout, cadence and mana accounting
/// remain authoritative. Every peer derives this from replicated state.
fn shown_mode(
    class: shared::HeroClass,
    loadout: Option<&LoadoutState>,
    action: Option<&ActionTime>,
) -> WeaponMode {
    let casting = action
        .filter(|action| action.age < 1.1)
        .and_then(|action| action.skill);
    slot_mode(
        class,
        casting,
        loadout.map_or(WeaponMode::Repeater, |state| state.weapon_mode),
    )
}

/// The gun mode shown while `skill` (none for a basic attack) plays, with the
/// loadout in `mode`: Shockline and Last Spark draw the launcher.
pub(crate) fn slot_mode(
    class: shared::HeroClass,
    skill: Option<SkillId>,
    mode: WeaponMode,
) -> WeaponMode {
    if class != shared::HeroClass::Wildspark {
        return WeaponMode::Repeater;
    }
    if matches!(skill, Some(SkillId::WildRocket | SkillId::WildZap)) {
        return WeaponMode::Rockets;
    }
    mode
}

/// Seconds after an accepted action within which its barrel flash is emitted.
pub(crate) const MUZZLE_WINDOW_SECS: [f32; 2] = [0.08, 0.25];

/// Whether an accepted action flashes the barrel of a held gun: every basic
/// attack, and the shots of Shockline and Last Spark.
pub(crate) fn flashes_muzzle(slot: u8, skill: Option<SkillId>) -> bool {
    slot == shared::BASIC_ATTACK_ACTION_SLOT
        || matches!(skill, Some(SkillId::WildZap | SkillId::WildRocket))
}

/// An explicit item that is still downloading, has failed or is not yet in this
/// peer's catalogue shows the whole class-default pair instead of empty hands,
/// and switches once it is Ready. The flag reports a class-default definition.
fn resolve_held(
    selection: &HandheldSelection,
    class: shared::HeroClass,
    mode: WeaponMode,
    definition: impl Fn(&str) -> Option<Weapon>,
) -> Option<(Weapon, bool)> {
    let id = selection.resolve_mode(class, mode)?;
    if let Some(def) = definition(id) {
        return Some((def, selection.is_default()));
    }
    if !matches!(selection, HandheldSelection::Item(_)) {
        return None;
    }
    let id = HandheldSelection::ClassDefault.resolve_mode(class, mode)?;
    definition(id).map(|def| (def, true))
}

#[derive(Component)]
struct WeaponPose(Transform);

/// Semantic glTF nodes of a spawned handheld and their authored rest poses.
#[derive(Component, Default)]
struct WeaponParts {
    /// `WildsparkSetRepeater` and `WildsparkSetLauncher`, only when both exist.
    set: Option<[Entity; 2]>,
    rotors: Vec<Part>,
    slides: Vec<Part>,
    shown: Option<WeaponMode>,
}

/// A rotor or slide node. Inside a set it belongs to the variant above it and
/// moves only while that variant is shown; elsewhere it moves in every mode.
struct Part {
    node: Entity,
    rest: Transform,
    variant: Option<WeaponMode>,
}
impl Part {
    fn moves(&self, shown: WeaponMode) -> bool {
        self.variant.is_none_or(|variant| variant == shown)
    }
}

/// A scene instance spawns atomically, so the first frame with children has
/// every node. Rest poses are read before any motion. Mesh primitives may carry
/// their mesh's name and are skipped: only glTF nodes are semantic parts.
fn bind_parts(
    mut commands: Commands,
    weapons: Query<Entity, (With<HeldWeapon>, With<Children>, Without<WeaponParts>)>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    nodes: Query<(&Name, &Transform), Without<Mesh3d>>,
) {
    for entity in &weapons {
        let mut parts = WeaponParts::default();
        let (mut repeater, mut launcher) = (None, None);
        for node in children.iter_descendants(entity) {
            let Ok((name, rest)) = nodes.get(node) else {
                continue;
            };
            let part = Part {
                node,
                rest: *rest,
                variant: None,
            };
            match name.as_str() {
                "WildsparkSetRepeater" => repeater = repeater.or(Some(node)),
                "WildsparkSetLauncher" => launcher = launcher.or(Some(node)),
                "WildsparkRotor" => parts.rotors.push(part),
                "WildsparkSlide" => parts.slides.push(part),
                _ => {}
            }
        }
        parts.set = repeater.zip(launcher).map(|(r, l)| [r, l]);
        if let Some([repeater, launcher]) = parts.set {
            for part in parts.rotors.iter_mut().chain(&mut parts.slides) {
                part.variant = parents.iter_ancestors(part.node).find_map(|ancestor| {
                    (ancestor == repeater)
                        .then_some(WeaponMode::Repeater)
                        .or((ancestor == launcher).then_some(WeaponMode::Rockets))
                });
            }
        }
        commands.entity(entity).insert(parts);
    }
}

/// A Wildspark set GLB carries both guns; its launcher is authored at scale 0
/// so viewers unaware of the convention see only the repeater. Show the variant
/// for the owner's mode at full scale and hide the other one.
fn show_set_variant(
    mut weapons: Query<(&HeldWeapon, &mut WeaponParts)>,
    mut nodes: Query<(&mut Transform, &mut Visibility)>,
) {
    for (weapon, mut parts) in &mut weapons {
        let Some([repeater, launcher]) = parts.set else {
            continue;
        };
        if parts.shown == Some(weapon.mode) {
            continue;
        }
        parts.shown = Some(weapon.mode);
        let (shown, hidden) = match weapon.mode {
            WeaponMode::Repeater => (repeater, launcher),
            WeaponMode::Rockets => (launcher, repeater),
        };
        if let Ok((mut pose, mut visibility)) = nodes.get_mut(shown) {
            pose.scale = Vec3::ONE;
            *visibility = Visibility::Inherited;
        }
        if let Ok((_, mut visibility)) = nodes.get_mut(hidden) {
            *visibility = Visibility::Hidden;
        }
    }
}

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
        if !visible.get()
            || action.muzzle_sequence == action.sequence
            || action.age < MUZZLE_WINDOW_SECS[0]
        {
            continue;
        }
        action.muzzle_sequence = action.sequence;
        if action.age > MUZZLE_WINDOW_SECS[1] || !flashes_muzzle(action.slot, action.skill) {
            continue;
        }
        let Some(muzzle) = muzzle_point(&weapon.model, weapon.mode, weapon.hint) else {
            continue;
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

const REPEATER_MUZZLE: [f32; 3] = [0.0, 0.12, 0.735];
const LAUNCHER_MUZZLE: [f32; 3] = [0.0, 0.13, 0.81];

/// Packaged guns keep their measured tips and other packaged props emit none.
/// SDK models use their embedded set hint, else the baseline tip of the mode.
pub(crate) fn muzzle_point(
    model: &str,
    mode: WeaponMode,
    hint: Option<WildsparkSetHint>,
) -> Option<Vec3> {
    let tip = match model {
        "weapons/wildspark-repeater.glb" => REPEATER_MUZZLE,
        "weapons/wild-launcher.glb" => LAUNCHER_MUZZLE,
        packaged
            if packaged.starts_with("weapons/") && !packaged.starts_with("weapons/imported/") =>
        {
            return None;
        }
        _ => match mode {
            WeaponMode::Repeater => hint
                .and_then(|h| h.repeater_muzzle)
                .unwrap_or(REPEATER_MUZZLE),
            WeaponMode::Rockets => hint
                .and_then(|h| h.launcher_muzzle)
                .unwrap_or(LAUNCHER_MUZZLE),
        },
    };
    Some(Vec3::from_array(tip))
}

fn kick(age: f32) -> f32 {
    // Fast compression followed by a slower spring return; no motion while paused.
    (age / 0.035).clamp(0.0, 1.0) * (1.0 - age / 0.32).clamp(0.0, 1.0).powi(2)
}

/// Packaged Wildspark guns and any model with a named rotor or slide move. The
/// parts are posed relative to their authored rest transforms every frame. In a
/// set only the shown variant's mechanism moves: the repeater's rotor, or the
/// launcher's slide in rockets mode and while W or R draws it. The hidden
/// variant keeps its last pose.
fn animate_parts(
    actions: Res<WeaponActions>,
    attachments: Query<(Entity, &HeldWeapon, &WeaponPose, Option<&WeaponParts>)>,
    mut poses: Query<&mut Transform>,
) {
    for (entity, weapon, base, parts) in &attachments {
        let mechanical = parts.is_some_and(|p| !p.rotors.is_empty() || !p.slides.is_empty());
        if !mechanical && !matches!(weapon.id.as_str(), "wild-repeater" | "wild-launcher") {
            continue;
        }
        let Some(action) = actions.0.get(&weapon.owner) else {
            continue;
        };
        let firing = action.slot == shared::BASIC_ATTACK_ACTION_SLOT
            || matches!(action.skill, Some(SkillId::WildZap | SkillId::WildRocket));
        let recoil = if firing { kick(action.age) } else { 0.0 };
        if let Ok(mut pose) = poses.get_mut(entity) {
            *pose = base.0;
            pose.rotation *= Quat::from_rotation_x(-0.11 * recoil);
            // Lower and seat the newly selected weapon on Q.
            if action.skill == Some(SkillId::WildSwitch) && action.age < 0.3 {
                pose.rotation *= Quat::from_rotation_x(0.3 * (1.0 - action.age / 0.3));
            }
        }
        let Some(parts) = parts else {
            continue;
        };
        for part in parts.rotors.iter().filter(|part| part.moves(weapon.mode)) {
            if let Ok(mut pose) = poses.get_mut(part.node) {
                pose.rotation = part.rest.rotation * Quat::from_rotation_z(action.rotor);
            }
        }
        for part in parts.slides.iter().filter(|part| part.moves(weapon.mode)) {
            if let Ok(mut pose) = poses.get_mut(part.node) {
                pose.translation = part.rest.translation - Vec3::Z * (recoil * 0.095);
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

    const SDK_SET: &str = "ekza-0123456789abcdef0123456789abcdef";

    fn action(skill: Option<SkillId>, age: f32) -> ActionTime {
        ActionTime {
            sequence: 2,
            slot: shared::BASIC_ATTACK_ACTION_SLOT,
            skill,
            age,
            rotor: 1.25,
            muzzle_sequence: 1,
        }
    }

    #[test]
    fn only_wildspark_shows_the_launcher_and_ultimates_draw_it_briefly() {
        use shared::HeroClass;
        let rockets = LoadoutState {
            weapon_mode: WeaponMode::Rockets,
            ..default()
        };
        assert_eq!(
            shown_mode(HeroClass::Wildspark, Some(&rockets), None),
            WeaponMode::Rockets
        );
        assert_eq!(
            shown_mode(HeroClass::Wildspark, None, None),
            WeaponMode::Repeater
        );
        for skill in [SkillId::WildZap, SkillId::WildRocket] {
            let draw = action(Some(skill), 0.5);
            assert_eq!(
                shown_mode(HeroClass::Wildspark, None, Some(&draw)),
                WeaponMode::Rockets
            );
            let done = action(Some(skill), 1.2);
            assert_eq!(
                shown_mode(HeroClass::Wildspark, None, Some(&done)),
                WeaponMode::Repeater
            );
        }
        // Other owners of a set always hold its repeater variant.
        for class in [HeroClass::Riftshot, HeroClass::Warrior] {
            assert_eq!(
                shown_mode(
                    class,
                    Some(&rockets),
                    Some(&action(Some(SkillId::WildZap), 0.1))
                ),
                WeaponMode::Repeater
            );
        }
    }

    #[test]
    fn an_item_that_is_not_ready_shows_the_whole_class_default_pair() {
        use shared::HeroClass;
        let packaged = |id: &str| {
            omoba_passport::weapons::catalog()
                .items
                .iter()
                .find(|w| w.id == id)
                .cloned()
        };
        let item = HandheldSelection::Item(SDK_SET.into());
        let id = |held: Option<(Weapon, bool)>| held.map(|(def, default)| (def.id, default));
        for (mode, default) in [
            (WeaponMode::Repeater, "wild-repeater"),
            (WeaponMode::Rockets, "wild-launcher"),
        ] {
            // Downloading or failed: the real store has no runtime in unit tests.
            assert_eq!(
                id(resolve_held(
                    &item,
                    HeroClass::Wildspark,
                    mode,
                    omoba_passport::weapon_store::definition
                )),
                Some((default.to_owned(), true))
            );
            let ready = |id: &str| {
                if id == SDK_SET {
                    packaged("wild-repeater").map(|w| Weapon {
                        id: SDK_SET.into(),
                        model: format!("ekza://weapons/{SDK_SET}.glb"),
                        ..w
                    })
                } else {
                    packaged(id)
                }
            };
            assert_eq!(
                id(resolve_held(&item, HeroClass::Wildspark, mode, ready)),
                Some((SDK_SET.to_owned(), false))
            );
        }
        assert_eq!(
            id(resolve_held(
                &item,
                HeroClass::Warrior,
                WeaponMode::Repeater,
                packaged
            )),
            Some(("forge-sword".to_owned(), true))
        );
        assert_eq!(
            id(resolve_held(
                &HandheldSelection::Unequipped,
                HeroClass::Wildspark,
                WeaponMode::Rockets,
                packaged
            )),
            None
        );
        assert_eq!(
            id(resolve_held(
                &HandheldSelection::Item("dagger".into()),
                HeroClass::Wildspark,
                WeaponMode::Rockets,
                packaged
            )),
            Some(("dagger".to_owned(), false))
        );
    }

    /// Bevy's glTF layout: Scene0 → set root → variants → semantic parts. Each
    /// moving node also has a mesh primitive child that carries the same name.
    fn spawn_set(world: &mut World, owner: Entity, mode: WeaponMode, launcher: bool) -> Entity {
        let weapon = world
            .spawn((
                HeldWeapon {
                    owner,
                    root: owner,
                    hand: owner,
                    id: SDK_SET.into(),
                    model: format!("ekza://weapons/{SDK_SET}.glb"),
                    mode,
                    hint: None,
                },
                Transform::IDENTITY,
                WeaponPose(Transform::IDENTITY),
                Visibility::Inherited,
            ))
            .id();
        let scene = world
            .spawn((Name::new("Scene0"), Transform::IDENTITY, ChildOf(weapon)))
            .id();
        let set = world
            .spawn((
                Name::new("wildspark-set"),
                Transform::IDENTITY,
                ChildOf(scene),
            ))
            .id();
        let mut part = |name: &str, parent: Entity, rest: Transform| {
            let node = world
                .spawn((
                    Name::new(name.to_owned()),
                    rest,
                    Visibility::Inherited,
                    ChildOf(parent),
                ))
                .id();
            world.spawn((
                Name::new(name.to_owned()),
                Mesh3d(Handle::default()),
                Transform::IDENTITY,
                ChildOf(node),
            ));
            node
        };
        let repeater = part("WildsparkSetRepeater", set, Transform::IDENTITY);
        part(
            "WildsparkRotor",
            repeater,
            Transform::from_xyz(0.0, 0.12, 0.25).with_rotation(Quat::from_rotation_y(0.5)),
        );
        if launcher {
            let launcher = part(
                "WildsparkSetLauncher",
                set,
                Transform::IDENTITY.with_scale(Vec3::ZERO),
            );
            part(
                "WildsparkSlide",
                launcher,
                Transform::from_xyz(0.0, 0.13, 0.20),
            );
        }
        weapon
    }

    fn node(world: &mut World, name: &str) -> (Transform, Visibility) {
        let mut nodes = world.query_filtered::<(&Name, &Transform, &Visibility), Without<Mesh3d>>();
        let (_, pose, visibility) = nodes
            .iter(world)
            .find(|(n, ..)| n.as_str() == name)
            .unwrap();
        (*pose, *visibility)
    }

    #[test]
    fn a_set_shows_the_variant_for_the_owner_mode_and_hides_the_other() {
        let mut app = App::new();
        app.add_systems(Update, (bind_parts, show_set_variant).chain());
        let owner = app.world_mut().spawn_empty().id();
        let weapon = spawn_set(app.world_mut(), owner, WeaponMode::Repeater, true);
        app.update();
        let world = app.world_mut();
        assert_eq!(
            node(world, "WildsparkSetRepeater"),
            (Transform::IDENTITY, Visibility::Inherited)
        );
        assert_eq!(node(world, "WildsparkSetLauncher").1, Visibility::Hidden);
        world.get_mut::<HeldWeapon>(weapon).unwrap().mode = WeaponMode::Rockets;
        app.update();
        let world = app.world_mut();
        assert_eq!(
            node(world, "WildsparkSetLauncher"),
            (Transform::IDENTITY, Visibility::Inherited),
            "the launcher authored at scale 0 is restored to full size"
        );
        assert_eq!(node(world, "WildsparkSetRepeater").1, Visibility::Hidden);
        world.get_mut::<HeldWeapon>(weapon).unwrap().mode = WeaponMode::Repeater;
        app.update();
        let world = app.world_mut();
        assert_eq!(node(world, "WildsparkSetRepeater").1, Visibility::Inherited);
        assert_eq!(node(world, "WildsparkSetLauncher").1, Visibility::Hidden);
        let parts = world.get::<WeaponParts>(weapon).unwrap();
        assert_eq!((parts.rotors.len(), parts.slides.len()), (1, 1));

        // A single-gun skin with a rotor is not a set and keeps its own visibility.
        let world = app.world_mut();
        world.despawn(weapon);
        spawn_set(world, owner, WeaponMode::Rockets, false);
        app.update();
        let world = app.world_mut();
        assert_eq!(node(world, "WildsparkSetRepeater").1, Visibility::Inherited);
    }

    #[test]
    fn sdk_rotor_and_slide_move_relative_to_their_rest_pose() {
        let mut app = App::new();
        app.init_resource::<WeaponActions>()
            .add_systems(Update, (bind_parts, animate_parts).chain());
        let owner = app.world_mut().spawn_empty().id();
        let weapon = spawn_set(app.world_mut(), owner, WeaponMode::Repeater, true);
        let firing = action(None, 0.05);
        let (rotor, recoil) = (firing.rotor, kick(firing.age));
        app.world_mut()
            .resource_mut::<WeaponActions>()
            .0
            .insert(owner, firing);
        // Two frames: poses are recomputed from the rest, never accumulated.
        app.update();
        app.update();
        let world = app.world_mut();
        let spun = node(world, "WildsparkRotor").0;
        assert_eq!(spun.translation, Vec3::new(0.0, 0.12, 0.25));
        let spun_by = |angle: f32| Quat::from_rotation_y(0.5) * Quat::from_rotation_z(angle);
        assert!(spun.rotation.angle_between(spun_by(rotor)) < 1e-5);
        let rest_slide = Vec3::new(0.0, 0.13, 0.20);
        assert_eq!(
            node(world, "WildsparkSlide").0.translation,
            rest_slide,
            "the hidden launcher's slide does not recoil in repeater mode"
        );

        // Rockets mode, or a W/R draw: the launcher's slide recoils and the
        // hidden repeater's rotor keeps its last pose while the owner fires.
        world.get_mut::<HeldWeapon>(weapon).unwrap().mode = WeaponMode::Rockets;
        let mut launcher_shot = action(Some(SkillId::WildRocket), 0.05);
        launcher_shot.rotor = rotor + 1.0;
        world
            .resource_mut::<WeaponActions>()
            .0
            .insert(owner, launcher_shot);
        app.update();
        let world = app.world_mut();
        let slide = node(world, "WildsparkSlide").0;
        assert!(recoil > 0.0);
        assert!((slide.translation - (rest_slide - Vec3::Z * recoil * 0.095)).length() < 1e-6);
        let frozen = node(world, "WildsparkRotor").0;
        assert!(frozen.rotation.angle_between(spun_by(rotor)) < 1e-5);

        // An SDK prop without named parts stays static, even while its owner fires.
        let mut weapons = world.query_filtered::<Entity, With<HeldWeapon>>();
        let weapon = weapons.single(world).unwrap();
        world.despawn(weapon);
        let prop = world
            .spawn((
                HeldWeapon {
                    owner,
                    root: owner,
                    hand: owner,
                    id: SDK_SET.into(),
                    model: format!("ekza://weapons/{SDK_SET}.glb"),
                    mode: WeaponMode::Repeater,
                    hint: None,
                },
                Transform::from_xyz(0.0, 0.0, 0.1),
                WeaponPose(Transform::from_xyz(0.0, 0.0, 0.2)),
            ))
            .id();
        world.spawn((Name::new("Blade"), Transform::IDENTITY, ChildOf(prop)));
        app.update();
        assert_eq!(
            app.world().get::<Transform>(prop).unwrap().translation.z,
            0.1
        );
    }

    #[test]
    fn parts_outside_a_set_move_in_every_mode() {
        let mut app = App::new();
        app.init_resource::<WeaponActions>()
            .add_systems(Update, (bind_parts, animate_parts).chain());
        let owner = app.world_mut().spawn_empty().id();
        // A single gun with both a rotor and a slide, no set variants.
        let weapon = spawn_set(app.world_mut(), owner, WeaponMode::Rockets, false);
        let world = app.world_mut();
        let rotor = node(world, "WildsparkRotor").0;
        let mut nodes = world.query_filtered::<(Entity, &Name), Without<Mesh3d>>();
        let receiver = nodes
            .iter(world)
            .find(|(_, name)| name.as_str() == "WildsparkSetRepeater")
            .map(|(entity, _)| entity)
            .unwrap();
        world.spawn((
            Name::new("WildsparkSlide"),
            Transform::from_xyz(0.0, 0.13, 0.20),
            Visibility::Inherited,
            ChildOf(receiver),
        ));
        let firing = action(None, 0.05);
        let (angle, recoil) = (firing.rotor, kick(firing.age));
        world
            .resource_mut::<WeaponActions>()
            .0
            .insert(owner, firing);
        app.update();
        let world = app.world_mut();
        let parts = world.get::<WeaponParts>(weapon).unwrap();
        assert!(parts.set.is_none());
        assert!(
            parts
                .rotors
                .iter()
                .chain(&parts.slides)
                .all(|part| part.variant.is_none())
        );
        let spun = node(world, "WildsparkRotor").0;
        assert!(
            spun.rotation
                .angle_between(rotor.rotation * Quat::from_rotation_z(angle))
                < 1e-5
        );
        let slide = node(world, "WildsparkSlide").0;
        assert!((slide.translation - Vec3::new(0.0, 0.13, 0.20 - recoil * 0.095)).length() < 1e-6);
    }

    #[test]
    fn set_parts_belong_to_the_variant_above_them() {
        let mut app = App::new();
        app.add_systems(Update, bind_parts);
        let owner = app.world_mut().spawn_empty().id();
        let weapon = spawn_set(app.world_mut(), owner, WeaponMode::Repeater, true);
        app.update();
        let parts = app.world().get::<WeaponParts>(weapon).unwrap();
        let variants = |parts: &[Part]| parts.iter().map(|p| p.variant).collect::<Vec<_>>();
        assert_eq!(variants(&parts.rotors), [Some(WeaponMode::Repeater)]);
        assert_eq!(variants(&parts.slides), [Some(WeaponMode::Rockets)]);
        assert!(parts.rotors[0].moves(WeaponMode::Repeater));
        assert!(!parts.rotors[0].moves(WeaponMode::Rockets));
        assert!(parts.slides[0].moves(WeaponMode::Rockets));
        assert!(!parts.slides[0].moves(WeaponMode::Repeater));
    }

    #[test]
    fn sdk_muzzles_follow_the_shown_variant_and_the_embedded_hint() {
        let sdk = format!("ekza://weapons/{SDK_SET}.glb");
        let hint = WildsparkSetHint {
            repeater_muzzle: Some([0.0, 0.1, 0.9]),
            launcher_muzzle: None,
        };
        for mode in [WeaponMode::Repeater, WeaponMode::Rockets] {
            assert_eq!(
                muzzle_point("weapons/wildspark-repeater.glb", mode, Some(hint)),
                Some(Vec3::new(0.0, 0.12, 0.735))
            );
            assert_eq!(muzzle_point("weapons/forge-sword.glb", mode, None), None);
        }
        assert_eq!(
            muzzle_point(&sdk, WeaponMode::Repeater, None),
            Some(Vec3::new(0.0, 0.12, 0.735))
        );
        assert_eq!(
            muzzle_point(&sdk, WeaponMode::Rockets, None),
            Some(Vec3::new(0.0, 0.13, 0.81))
        );
        assert_eq!(
            muzzle_point(&sdk, WeaponMode::Repeater, Some(hint)),
            Some(Vec3::new(0.0, 0.1, 0.9))
        );
        assert_eq!(
            muzzle_point(&sdk, WeaponMode::Rockets, Some(hint)),
            Some(Vec3::new(0.0, 0.13, 0.81)),
            "a missing tip falls back to the baseline launcher"
        );
    }
}
