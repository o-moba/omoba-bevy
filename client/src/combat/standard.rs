//! Presentation and input helpers for resolved skills. No hit or resource authority.
// i18n-strict
use bevy::gizmos::config::GizmoConfigGroup;
use bevy::prelude::*;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct SkillAimGizmos;
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(super) struct SkillEffectGizmos;

#[derive(Resource, Default)]
pub(crate) struct SkillAimVector(pub Option<(Vec2, Vec2, f32)>);

use shared::loadout::{EffectVisualKind, LoadoutState, SkillEffectState, WeaponMode};
use shared::{HeroClass, TargetingMode};

use super::aim_preview;
use crate::i18n::{data, tr, trf};
use crate::net::{GameStateSnapshot, NetworkHeroClass, PlayerLoadout};
use crate::player::Player;
use crate::skill_presentation::SkillPresentation;
use crate::skill_presentation::geometry::{self, GeoShape};
use crate::skill_presentation::stage::{self, Stage};
use crate::skill_presentation::status::{self, StateVisual};
use crate::skill_presentation::vocab::RecastMarker;
use crate::sprite::PlayerVisualMode;

pub(crate) fn bounded_aim(origin: Vec2, aim: Vec2, targeting: TargetingMode, range: f32) -> Vec2 {
    match targeting {
        TargetingMode::Point => origin + (aim - origin).clamp_length_max(range),
        TargetingMode::SelfTarget => origin,
        _ => aim,
    }
}

pub(crate) fn attack_range(class: HeroClass, loadout: Option<&PlayerLoadout>) -> f32 {
    loadout
        .and_then(|s| s.0.as_ref())
        .filter(|s| s.basic_attack_range > 0.0)
        .map_or_else(
            || shared::basic_attack_for_class(class).range,
            |s| s.basic_attack_range,
        )
}

pub(crate) fn movement_factor(loadout: Option<&PlayerLoadout>) -> f32 {
    loadout.and_then(|s| s.0.as_ref()).map_or(1.0, |s| {
        if s.root_remaining_secs > 0.0 {
            0.0
        } else {
            s.movement_multiplier
        }
    })
}

#[derive(Component)]
pub(super) struct StandardStatus;

pub(super) fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands
        .spawn((
            Button,
            InteractButton,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(24.0),
                bottom: Val::Px(230.0),
                padding: UiRect::all(Val::Px(12.0)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(crate::ui::theme::BACKDROP),
            Name::new("LanternInteract"),
        ))
        .with_child((
            Text::new(tr("combat.standard.interact")),
            TextFont {
                font_size: (16.0).into(),
                ..default()
            },
            TextColor(Color::WHITE),
        ));
    commands.spawn((
        Text::default(),
        TextFont {
            font: assets.load("ui/Inter.ttf").into(),
            font_size: (14.0).into(),
            ..default()
        },
        TextColor(crate::ui::tokens::color::TEXT_PRIMARY),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(18.0),
            bottom: Val::Px(158.0),
            max_width: Val::Px(440.0),
            display: Display::None,
            ..default()
        },
        BackgroundColor(crate::ui::theme::BACKDROP),
        StandardStatus,
        Name::new("StandardKitStatus"),
    ));
}

pub(super) fn update_status(
    inspection: Res<super::inspection::SkillInspection>,
    local: Query<(&NetworkHeroClass, &PlayerLoadout), With<Player>>,
    mut label: Query<(&mut Text, &mut Node), With<StandardStatus>>,
    mobile: Res<crate::mobile_controls::MobileControls>,
    pad: Res<crate::gamepad::GamepadControls>,
) {
    let Ok((mut text, mut node)) = label.single_mut() else {
        return;
    };
    if inspection.slot.is_none() {
        node.display = Display::None;
        return;
    }
    let Ok((class, loadout)) = local.single() else {
        node.display = Display::None;
        return;
    };
    let Some(state) = loadout.0.as_ref().filter(|s| s.recipe.is_some()) else {
        node.display = Display::None;
        return;
    };
    node.display = Display::Flex;
    let passive = match state.recipe.as_ref().unwrap().passive {
        shared::loadout::PassiveId::Radiance => tr("combat.standard.radiance"),
        shared::loadout::PassiveId::Momentum => tr("combat.standard.momentum"),
        shared::loadout::PassiveId::Tempered => tr("combat.standard.tempered"),
        shared::loadout::PassiveId::Vitals => tr("combat.standard.vitals"),
        shared::loadout::PassiveId::Flow => tr("combat.standard.flow"),
        shared::loadout::PassiveId::Shroud => tr("combat.standard.shroud"),
        shared::loadout::PassiveId::Essence => tr("combat.standard.essence"),
        shared::loadout::PassiveId::Clockwork => tr("combat.standard.clockwork"),
        shared::loadout::PassiveId::Resonance => tr("combat.standard.resonance"),
        shared::loadout::PassiveId::Souls => tr("combat.standard.souls_passive"),
        shared::loadout::PassiveId::Concussion => tr("combat.standard.concussion_passive"),
        shared::loadout::PassiveId::DaggerMastery => tr("combat.standard.dagger_mastery_passive"),
    };
    let mut lines = vec![format!("{} · {}", data::hero_name(class.0), passive)];
    if !mobile.enabled && !pad.active {
        lines.push(tr("combat.standard.aim_hint").into());
    }
    if class.0 == HeroClass::Wildspark {
        lines.push(trf(
            "combat.standard.weapon",
            &[
                (
                    "mode",
                    &tr(match state.weapon_mode {
                        WeaponMode::Repeater => "combat.standard.repeater",
                        WeaponMode::Rockets => "combat.standard.rockets",
                    }),
                ),
                ("stacks", &state.passive_stacks),
                ("cost", &format!("{:.0}", state.basic_attack_mana_cost)),
            ],
        ));
    }
    if state.forge_remaining_secs > 0.0 {
        lines.push(trf(
            "combat.standard.forging",
            &[("seconds", &format!("{:.1}", state.forge_remaining_secs))],
        ));
    }
    if state.forge_ready {
        lines.push(tr("combat.standard.forge_ready").into());
    }
    if state.concussion_stacks > 0 {
        lines.push(trf(
            "combat.standard.concussion",
            &[("count", &state.concussion_stacks)],
        ));
    }
    if state.brittle {
        lines.push(tr("combat.standard.brittle").into());
    }
    if state.energy {
        lines.push(tr("combat.standard.energy").into());
    }
    if state.camouflaged {
        lines.push(tr("combat.standard.camouflaged").into());
    }
    if state.parrying {
        lines.push(tr("combat.standard.parrying").into());
    }
    if state.forged {
        lines.push(tr("combat.standard.forged").into());
    }
    if state.souls > 0 {
        lines.push(trf("combat.standard.souls", &[("count", &state.souls)]));
    }
    if state.shield_hp > 0.0 {
        lines.push(trf(
            "combat.standard.shield",
            &[("value", &format!("{:.0}", state.shield_hp))],
        ));
    }
    if state.root_remaining_secs > 0.0 {
        lines.push(tr("combat.standard.rooted").into());
    }
    if state.passive_remaining_secs > 0.0 {
        lines.push(trf(
            "combat.standard.acceleration",
            &[("seconds", &format!("{:.1}", state.passive_remaining_secs))],
        ));
    }
    for (i, slot) in state.slots.iter().enumerate().filter(|(_, s)| s.can_recast) {
        lines.push(trf(
            "combat.standard.recast",
            &[
                ("key", &["Q", "W", "E", "R"][i]),
                ("seconds", &format!("{:.1}", slot.recast_remaining_secs)),
            ],
        ));
    }
    text.0 = lines.join("\n");
}

pub(super) fn point(p: Vec2, mode: PlayerVisualMode, map: Option<&crate::maps::MapLayout>) -> Vec3 {
    let world = Vec3::new(
        p.x,
        map.map_or(0.08, |m| m.terrain_height_3d(p.x, p.y) + 0.12),
        p.y,
    );
    if mode == PlayerVisualMode::Sprite2d {
        crate::world2d::simulation_xz_to_render_xy(world).extend(crate::world2d::layer::VFX)
    } else {
        world
    }
}

/// The aim preview while a key, touch drag or controller button is held: the rule the
/// server would apply to the cast, as `geometry::preview_shape` derives it.
pub(crate) fn draw_aim(
    mut gizmos: Gizmos<SkillAimGizmos>,
    mut vector: ResMut<SkillAimVector>,
    local: Query<
        (
            &Transform,
            &NetworkHeroClass,
            &crate::net::PlayerProgression,
            &super::CombatStats,
            &crate::team::Team,
            Option<&crate::net::PlayerLoadout>,
            Option<&crate::net::NetworkPlayerId>,
        ),
        With<Player>,
    >,
    camera: Query<(&Camera, &GlobalTransform), With<crate::camera::MainCamera>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mode: Res<PlayerVisualMode>,
    map: Option<Res<crate::maps::MapLayout>>,
    context: Res<crate::input_context::GameplayInputContext>,
    (keyboard, mobile, pad): (
        Res<ButtonInput<KeyCode>>,
        Option<Res<crate::mobile_controls::MobileControls>>,
        Option<Res<crate::gamepad::GamepadControls>>,
    ),
    candidates: super::selection::TargetCandidates,
    validity: crate::targeting::TargetValidity,
    target: Res<super::selection::TargetState>,
    basic: Res<crate::targeting::BasicAttackState>,
    world: aim_preview::AimWorld,
    #[cfg(feature = "qa")] mut shown: ResMut<aim_preview::AimPreviewShown>,
) {
    vector.0 = None;
    #[cfg(feature = "qa")]
    {
        shown.0 = None;
    }
    if !context.gameplay_allowed() {
        return;
    }
    let Ok((pose, class, progression, stats, team, loadout, id)) = local.single() else {
        return;
    };
    if !stats.is_alive() {
        return;
    }
    let Some(skills) = crate::equipped_skills::resolve(class.0, loadout) else {
        return;
    };
    let touch = mobile
        .as_ref()
        .filter(|m| m.enabled)
        .and_then(|m| m.aimed_skill());
    let controller = pad.as_ref().filter(|p| p.active);
    let slot = touch
        .map(|t| t.slot)
        .or_else(|| controller.and_then(|p| p.aiming_slot))
        .or_else(|| {
            crate::input_bindings::SKILL_CAST_KEYS
                .iter()
                .position(|k| keyboard.pressed(*k))
        });
    let Some(slot) = slot.filter(|i| *i < 4) else {
        return;
    };
    let Some(def) = skills.skill(shared::SkillSlot::ALL[slot]) else {
        return;
    };
    let origin = pose.translation.xz();
    let range = shared::scaled_cast_range(&def.ability, progression.ranks[slot].max(1));
    let aim = if def.ability.targeting == TargetingMode::SelfTarget {
        // A self cast has no aim: its preview stands on the hero or on its orb.
        origin
    } else if touch.is_some() || controller.is_some() {
        let screen = touch
            .and_then(|t| t.aim)
            .or_else(|| controller.and_then(|p| p.aim));
        let manual = screen.and_then(|screen| {
            camera.single().ok().map(|(_, camera)| {
                crate::player::mobile_screen_direction(screen, camera, *mode).xz()
            })
        });
        let assisted = screen
            .is_none()
            .then(|| {
                super::mobile::quick_cast_target(
                    origin,
                    *team,
                    range,
                    target.selected_entity,
                    basic.order.map(|order| order.entity),
                    &candidates,
                    &validity,
                )
            })
            .flatten();
        let extent = touch.map(|t| t.extent).unwrap_or_else(|| {
            controller
                .filter(|p| p.aim.is_some())
                .and_then(|p| p.raw)
                .map_or(1.0, |p| p.right.length().clamp(0.15, 1.0))
        });
        super::mobile::resolve_mobile_aim(
            origin,
            pose.forward().xz(),
            range,
            extent,
            manual,
            assisted,
        )
    } else {
        let (Ok(window), Ok((camera, camera_pose))) = (windows.single(), camera.single()) else {
            return;
        };
        let Some(cursor) = window.cursor_position() else {
            return;
        };
        let Some(p) =
            crate::player::viewport_to_simulation_world(camera, camera_pose, cursor, *mode, 0.0)
        else {
            return;
        };
        p.xz()
    };
    let aim = bounded_aim(origin, aim, def.ability.targeting, range);
    let caster = aim_preview::Caster {
        position: origin,
        id: id.map(|id| id.0),
        team: *team,
        flags: loadout.and_then(|loadout| loadout.0.as_ref()),
        slot,
    };
    let preview = world.preview(def, &caster, aim, &candidates);
    vector.0 = aim_preview::minimap_vector(&preview);
    aim_preview::draw(&mut gizmos, &preview, *mode, map.as_deref());
    #[cfg(feature = "qa")]
    {
        shown.0 = Some((slot, preview));
    }
}

/// What a line of the fallback drawing of an effect is painted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ink {
    /// The colour of the skill: the exact boundary of the effect, or the mark of an object
    /// that has none.
    Skill,
    /// The team colour: reading aids that claim no area.
    Team,
    /// The team colour at half strength.
    TeamFaint,
    /// The cross of a trap that is armed.
    Armed,
    /// The cross of a trap that is not armed yet.
    Arming,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Stroke {
    pub points: Vec<Vec2>,
    pub ink: Ink,
}

/// The lines the fallback draws for one replicated effect, in simulation ground
/// coordinates: first its boundary exactly as `geometry::boundary_shape` derives it from
/// the received fields, then reading aids. The sides a cage has lost are left out.
pub(super) fn effect_strokes(e: &SkillEffectState) -> Vec<Stroke> {
    use EffectVisualKind as K;
    let geo = geometry::boundary_shape(e.skill, e.kind, e);
    let mut strokes: Vec<Stroke> = geo
        .outline()
        .into_iter()
        .enumerate()
        .filter(|(side, _)| e.kind != K::Cage || e.consumed_segments & (1 << side) == 0)
        .map(|(_, points)| Stroke {
            points,
            ink: Ink::Skill,
        })
        .collect();
    let p = Vec2::from_array(e.position);
    let end = Vec2::from_array(e.end);
    let radius = e.radius;
    let ring = |radius: f32, ink: Ink| Stroke {
        points: GeoShape::Ring { center: p, radius }.outline().remove(0),
        ink,
    };
    let line = |from: Vec2, to: Vec2, ink: Ink| Stroke {
        points: vec![from, to],
        ink,
    };
    match e.kind {
        K::Field | K::Healing | K::Anchor | K::Orb | K::Lantern => {
            strokes.push(ring(radius * 0.86, Ink::TeamFaint));
        }
        K::Trap => {
            strokes.push(ring(radius * 0.86, Ink::TeamFaint));
            let offset = radius * 0.7;
            let ink = if e.armed { Ink::Armed } else { Ink::Arming };
            for sign in [-1.0, 1.0] {
                strokes.push(line(
                    p + Vec2::new(-offset, sign * offset),
                    p + Vec2::new(offset, -sign * offset),
                    ink,
                ));
            }
        }
        K::BeamWarning => {
            // A circular warning fills toward its edge and is full exactly when the server
            // fires, as its marker does in 3D.
            let view = stage::view(e);
            let filled = radius * view.progress;
            if matches!(geo, GeoShape::Ring { .. })
                && view.stage == Stage::Telegraph
                && filled > 0.05
            {
                strokes.push(ring(filled, Ink::Team));
            }
        }
        K::Beam => {
            let side = (end - p).normalize_or_zero().perp() * radius;
            strokes.extend((-2..=2).map(|i| {
                let side = side * (i as f32 / 3.0);
                line(p + side, end + side, Ink::Team)
            }));
        }
        K::Bolt | K::Barrier | K::Rocket => {
            let direction = (end - p).normalize_or_zero();
            strokes.push(line(
                p - direction * radius * 3.0,
                p + direction * radius,
                Ink::Team,
            ));
        }
        K::Soul => {
            // No reach is replicated for a soul, so it gets a mark and no ring.
            let corners = [Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y, Vec2::X];
            strokes.push(Stroke {
                points: corners.map(|corner| p + corner * radius).to_vec(),
                ink: Ink::Skill,
            });
        }
        K::ShieldWall | K::Cage => {}
    }
    strokes
}

/// A gizmo line around a hero, in simulation ground coordinates.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct HeroMark {
    pub points: Vec<Vec2>,
    pub color: Color,
}

fn circle(center: Vec2, radius: f32) -> Vec<Vec2> {
    GeoShape::Ring { center, radius }.outline().remove(0)
}

/// Radius of the small ground ring of a slow.
const SLOW_RING: f32 = 0.42;

/// The gizmo of one state of the hero at `p`: the ring each state has had, and a small
/// ring for a slow. A stun is drawn as the root it is replicated with. Camouflage and
/// forging have no gizmo.
pub(super) fn state_marks(state: StateVisual, flags: &LoadoutState, p: Vec2) -> Vec<HeroMark> {
    let ring = |radius: f32, color: Color| {
        vec![HeroMark {
            points: circle(p, radius),
            color,
        }]
    };
    match state {
        StateVisual::Stunned | StateVisual::Rooted => ring(0.7, Color::srgb(1.0, 0.3, 0.55)),
        StateVisual::ParryStance => ring(1.4, Color::WHITE),
        StateVisual::Shielded => ring(0.95, Color::srgb(0.5, 0.9, 1.0)),
        StateVisual::Marked => ring(1.15, Color::srgb(1.0, 0.9, 0.3)),
        StateVisual::Brittle => ring(1.2, Color::srgb(1.0, 0.6, 0.1)),
        StateVisual::Concussed => (0..flags.concussion_stacks.min(4))
            .map(|i| HeroMark {
                points: circle(p + Vec2::new(-0.6 + f32::from(i) * 0.4, 1.4), 0.12),
                color: Color::srgb(0.6, 0.9, 1.0),
            })
            .collect(),
        StateVisual::Slowed => ring(SLOW_RING, Color::srgb(0.4, 0.65, 1.0)),
        StateVisual::CamouflageVeil | StateVisual::Forging => Vec::new(),
    }
}

/// The state gizmos of one hero: one for every state its flags report, except the state
/// its mesh visual shows. The flat view has no mesh visual, so there every state keeps its
/// gizmo; in 3D a slow that another state outranks keeps its small ring.
pub(super) fn hero_state_marks(
    mode: PlayerVisualMode,
    visible: bool,
    alive: bool,
    flags: &LoadoutState,
    p: Vec2,
) -> Vec<HeroMark> {
    let meshed = StateVisual::shown(mode, visible, alive, flags);
    StateVisual::of(flags)
        .filter(|state| Some(*state) != meshed)
        .flat_map(|state| state_marks(state, flags, p))
        .collect()
}

/// Distance of a recast marker from the feet of its hero.
const MARKER_RING: f32 = 0.85;
/// Radians per second at which the marks of `orbit_motes` circle.
const MARKER_TURN: f32 = 2.6;

/// The ground lines of a recast marker for the hero at `p` that faces `forward`.
pub(super) fn marker_strokes(
    marker: RecastMarker,
    p: Vec2,
    forward: Vec2,
    now: f32,
) -> Vec<Vec<Vec2>> {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    let pip = |center: Vec2, half: f32| {
        [Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y, Vec2::X]
            .map(|corner| center + corner * half)
            .to_vec()
    };
    match marker {
        RecastMarker::RingPips => std::iter::once(circle(p, MARKER_RING))
            .chain((0..4).map(|i| {
                let turn = FRAC_PI_4 + i as f32 * FRAC_PI_2;
                pip(p + Vec2::from_angle(turn) * MARKER_RING, 0.16)
            }))
            .collect(),
        RecastMarker::OrbitMotes => (0..2)
            .map(|i| {
                let turn = now * MARKER_TURN + i as f32 * PI;
                pip(p + Vec2::from_angle(turn) * MARKER_RING, 0.22)
            })
            .collect(),
        RecastMarker::GroundArrows => (0..3)
            .map(|i| {
                let tip = p + forward * (MARKER_RING + 0.35 + i as f32 * 0.45);
                let wing = forward.perp() * 0.36;
                vec![tip - forward * 0.3 + wing, tip, tip - forward * 0.3 - wing]
            })
            .collect(),
    }
}

/// The colour of the aid that leads a hero to its own orb in the flat view: that of the
/// skill its replicated orb belongs to.
fn orb_aid_color(
    profiles: Option<&SkillPresentation>,
    owner: Option<u64>,
    effects: &[SkillEffectState],
) -> Color {
    owner
        .and_then(|owner| {
            effects.iter().find(|effect| {
                effect.kind == EffectVisualKind::Orb && owner != 0 && effect.owner_id == owner
            })
        })
        .and_then(|orb| profiles?.profile(orb.skill))
        .map_or(Color::srgb(0.9, 0.7, 1.0), |profile| {
            Color::srgb_from_array(profile.color)
        })
}

/// One hero as its gizmos are drawn from it.
pub(super) struct HeroSight<'a> {
    pub mode: PlayerVisualMode,
    /// The hero is drawn: nothing hides its entity.
    pub visible: bool,
    pub alive: bool,
    pub class: Option<HeroClass>,
    pub id: Option<u64>,
    pub flags: &'a LoadoutState,
    pub p: Vec2,
    /// The way the hero faces, on the ground.
    pub forward: Vec2,
    pub now: f32,
    pub profiles: Option<&'a SkillPresentation>,
    pub effects: &'a [SkillEffectState],
}

/// Every gizmo line of one hero's replicated state: the aid to its own orb, the states its
/// mesh visual does not show, and the marker of each recast the server offers it.
pub(super) fn hero_marks(sight: &HeroSight) -> Vec<HeroMark> {
    let HeroSight {
        mode,
        visible,
        alive,
        flags,
        p,
        ..
    } = *sight;
    let mut marks = Vec::new();
    // The orb position is replicated to its owner alone. In 3D the orb is a body of its
    // own; the flat view leads the owner to it with a ring and a line.
    if mode == PlayerVisualMode::Sprite2d
        && let Some(orb) = flags.orb_position
    {
        let orb = Vec2::from_array(orb);
        let color = orb_aid_color(sight.profiles, sight.id, sight.effects);
        marks.push(HeroMark {
            points: circle(orb, 0.65),
            color,
        });
        marks.push(HeroMark {
            points: vec![p, orb],
            color: color.with_alpha(0.3),
        });
    }
    marks.extend(hero_state_marks(mode, visible, alive, flags, p));
    // A recast the server offers a living hero the client sees, in the colour of its skill.
    if visible
        && alive
        && let (Some(profiles), Some(class), Some(id)) = (sight.profiles, sight.class, sight.id)
    {
        for (marker, color) in status::recast_markers(profiles, class, flags, id, p, sight.effects)
        {
            marks.extend(
                marker_strokes(marker, p, sight.forward, sight.now)
                    .into_iter()
                    .map(|points| HeroMark { points, color }),
            );
        }
    }
    marks
}

/// Distance of a vital side from the hero it stands around.
const FACET_DISTANCE: f32 = 1.4;
/// Half the length of a facet, along the line out of the hero.
const FACET_LENGTH: f32 = 0.42;
/// Half the width of a facet.
const FACET_WIDTH: f32 = 0.2;
/// How far apart the two halves of a broken side lie.
const FACET_GAP: f32 = 0.15;

/// A received hero state as the vital sides it names are drawn.
pub(super) struct Duelist<'a> {
    pub flags: &'a LoadoutState,
    pub team: Option<crate::team::Team>,
    /// It is the state of the local hero: only that hero is shown the side of its passive.
    pub own: bool,
}

/// A hero as vital sides are drawn around it.
pub(super) struct FacetTarget {
    pub id: u64,
    pub team: Option<crate::team::Team>,
    /// The hero is drawn and alive.
    pub seen: bool,
    pub p: Vec2,
}

/// The vital sides a duelist has on a hostile hero the client sees: all four while that
/// hero is the one the duelist challenged, a struck side as two grey halves; otherwise,
/// for the local hero with the Vitals passive, the one side its next hit must come from.
/// The sides lie on the world axes in the order the server counts them (+x, +z, -x, -z).
/// Lines on the ground only: nothing here is a state of the target.
pub(super) fn duel_facets(duelist: &Duelist, target: &FacetTarget) -> Vec<HeroMark> {
    let duel = duelist.flags;
    if !target.seen || duelist.team == target.team {
        return Vec::new();
    }
    let challenged = duel.challenge_target == Some(target.id);
    let vitals = duelist.own
        && duel
            .recipe
            .as_ref()
            .is_some_and(|recipe| recipe.passive == shared::loadout::PassiveId::Vitals);
    let rotating = (target.id as u8).wrapping_add(duel.vital_rotation) % 4;
    let mut marks = Vec::new();
    for side in (0..4u8).filter(|side| challenged || (vitals && *side == rotating)) {
        let out = Vec2::from_angle(f32::from(side) * std::f32::consts::FRAC_PI_2);
        let center = target.p + out * FACET_DISTANCE;
        let (tip, wing) = (out * FACET_LENGTH, out.perp() * FACET_WIDTH);
        if challenged && duel.challenge_sides & (1 << side) != 0 {
            // A struck side: the diamond lies open, in two halves.
            let color = Color::srgb(0.3, 0.35, 0.4);
            for half in [1.0, -1.0] {
                let base = center + out * (FACET_GAP * 0.5) * half;
                marks.push(HeroMark {
                    points: vec![base + wing, center + tip * half, base - wing],
                    color,
                });
            }
        } else {
            // An open side: the whole diamond with the line of the hit through it.
            let color = Color::srgb(1.0, 0.75, 0.2);
            marks.push(HeroMark {
                points: vec![
                    center + tip,
                    center + wing,
                    center - tip,
                    center - wing,
                    center + tip,
                ],
                color,
            });
            marks.push(HeroMark {
                points: vec![center - tip, center + tip],
                color,
            });
        }
    }
    marks
}

/// Whether the fallback lines of an effect are drawn. In the flat view they always are. In
/// 3D an effect whose row is known is drawn by that row, and a body draws its own boundary
/// in the colour of the team: lines of another colour over it would hide whose it is. Only
/// a trap whose row has no body yet keeps the tactical outline over its model.
pub(super) fn outlined(
    mode: PlayerVisualMode,
    kind: EffectVisualKind,
    known: bool,
    staged: bool,
) -> bool {
    mode != PlayerVisualMode::Models3d || !(staged || (known && kind != EffectVisualKind::Trap))
}

/// Bounded, snapshot-driven geometry. Effects do not depend on a visible owner.
pub(super) fn draw_effects(
    mut gizmos: Gizmos<SkillEffectGizmos>,
    profiles: Option<Res<SkillPresentation>>,
    game: Option<Res<GameStateSnapshot>>,
    mode: Res<PlayerVisualMode>,
    clock: Option<Res<crate::vfx_clock::VfxClock>>,
    map: Option<Res<crate::maps::MapLayout>>,
    local: Query<&crate::team::Team, With<Player>>,
    actors: Query<(
        &Transform,
        &PlayerLoadout,
        Option<&NetworkHeroClass>,
        Option<&crate::net::NetworkPlayerId>,
        Option<&crate::team::Team>,
        Option<&InheritedVisibility>,
        Option<&super::CombatStats>,
    )>,
) {
    let Some(game) = game else {
        return;
    };
    let map = map.as_deref();
    for e in game
        .skill_effects
        .iter()
        .take(shared::loadout::MAX_ACTIVE_EFFECTS)
    {
        if !e.position.into_iter().chain(e.end).all(f32::is_finite)
            || !(0.0..=256.0).contains(&e.radius)
        {
            continue;
        }
        let profile = profiles.as_ref().and_then(|r| r.profile(e.skill));
        let staged = profiles.as_ref().is_some_and(|r| r.body_for(e).is_some());
        if !outlined(*mode, e.kind, profile.is_some(), staged) {
            continue;
        }
        let friendly = local.single().is_ok_and(|t| *t == e.owner_team);
        let team = if e.kind == EffectVisualKind::Trap {
            if friendly {
                Color::linear_rgb(0.015, 0.8, 5.0)
            } else {
                Color::linear_rgb(5.0, 0.16, 0.015)
            }
        } else if friendly {
            Color::srgb(0.35, 0.95, 0.8)
        } else {
            Color::srgb(1.0, 0.3, 0.28)
        };
        // The flat view has no body for the effect, so its outline carries the colour of
        // the skill. Over a 3D model the outline stays a team marker.
        let skill = profile
            .filter(|_| *mode == PlayerVisualMode::Sprite2d)
            .map_or(team, |profile| Color::srgb_from_array(profile.color));
        for stroke in effect_strokes(e) {
            let color = match stroke.ink {
                Ink::Skill => skill,
                Ink::Team => team,
                Ink::TeamFaint => team.with_alpha(0.5),
                Ink::Armed => Color::WHITE,
                Ink::Arming => Color::srgb(1.0, 0.75, 0.15),
            };
            // Long edges are split so that they follow the ground in 3D.
            let points = stroke.points.windows(2).flat_map(|edge| {
                let steps = (edge[0].distance(edge[1]) / 1.5).ceil().clamp(1.0, 192.0) as usize;
                (0..steps).map(move |i| edge[0].lerp(edge[1], i as f32 / steps as f32))
            });
            gizmos.linestrip(
                points
                    .chain(stroke.points.last().copied())
                    .map(|at| point(at, *mode, map)),
                color,
            );
        }
    }
    // Every received state that can name a vital side: the local hero's own, and any other
    // whose challenge the server sends (today it sends that of the local hero alone).
    let duelists: Vec<Duelist> = actors
        .iter()
        .filter_map(|(_, loadout, _, id, team, ..)| {
            Some(Duelist {
                flags: loadout.0.as_ref()?,
                team: team.copied(),
                own: id.is_some_and(|id| id.0 == game.your_id),
            })
        })
        .collect();
    let now = clock.map_or(0.0, |clock| clock.now as f32);
    for (pose, loadout, class, id, team, visible, stats) in &actors {
        let p = pose.translation.xz();
        let visible = visible.is_some_and(|visible| visible.get());
        let alive = stats.is_none_or(|stats| stats.is_alive());
        if let Some(id) = id {
            let target = FacetTarget {
                id: id.0,
                team: team.copied(),
                seen: visible && alive,
                p,
            };
            for mark in duelists
                .iter()
                .flat_map(|duelist| duel_facets(duelist, &target))
            {
                gizmos.linestrip(
                    mark.points.iter().map(|at| point(*at, *mode, map)),
                    mark.color,
                );
            }
        }
        let Some(state) = &loadout.0 else {
            continue;
        };
        let sight = HeroSight {
            mode: *mode,
            visible,
            alive,
            class: class.map(|class| class.0),
            id: id.map(|id| id.0),
            flags: state,
            p,
            forward: pose.forward().xz().normalize_or(Vec2::NEG_Y),
            now,
            profiles: profiles.as_deref(),
            effects: &game.skill_effects,
        };
        for mark in hero_marks(&sight) {
            gizmos.linestrip(
                mark.points.iter().map(|at| point(*at, *mode, map)),
                mark.color,
            );
        }
        if state.recipe.is_some()
            && let Some(class) = class
        {
            let corners = if class.0 == HeroClass::Dawnweaver {
                4
            } else {
                3
            };
            for i in 0..corners {
                let at = |n: i32| {
                    p + Vec2::from_angle(n as f32 * std::f32::consts::TAU / corners as f32) * 0.55
                };
                gizmos.line(
                    point(at(i), *mode, map),
                    point(at(i + 1), *mode, map),
                    Color::srgb(0.9, 0.8, 0.5),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::loadout::SkillId;

    fn effect(skill: SkillId, kind: EffectVisualKind) -> SkillEffectState {
        SkillEffectState {
            id: 1,
            owner_id: 7,
            owner_team: shared::map::Team::Green,
            skill,
            kind,
            position: [3.0, -2.0],
            end: [3.0, 5.0],
            radius: 1.5,
            remaining_secs: 1.0,
            armed: false,
            consumed_segments: 0,
        }
    }

    /// The strokes painted in the colour of the skill.
    fn boundary(e: &SkillEffectState) -> Vec<Vec<Vec2>> {
        effect_strokes(e)
            .into_iter()
            .filter(|stroke| stroke.ink == Ink::Skill)
            .map(|stroke| stroke.points)
            .collect()
    }

    #[test]
    fn every_effect_kind_has_a_flat_outline_that_equals_its_boundary() {
        use EffectVisualKind as K;
        // One skill for every kind the server can replicate.
        let cases = [
            (SkillId::OrbitalCommand, K::Orb),
            (SkillId::IronHook, K::Soul),
            (SkillId::AnchorStep, K::Anchor),
            (SkillId::FourfoldDuel, K::Healing),
            (SkillId::Northwall, K::ShieldWall),
            (SkillId::IronBoundary, K::Cage),
            (SkillId::GuidingLantern, K::Lantern),
            (SkillId::WinterShard, K::Bolt),
            (SkillId::DawnBarrier, K::Barrier),
            (SkillId::DawnField, K::Field),
            (SkillId::DawnRay, K::BeamWarning),
            (SkillId::DawnRay, K::Beam),
            (SkillId::WildTraps, K::Trap),
            (SkillId::WildRocket, K::Rocket),
        ];
        let mut kinds = Vec::new();
        for (skill, kind) in cases {
            let e = effect(skill, kind);
            let strokes = effect_strokes(&e);
            assert!(!strokes.is_empty(), "{kind:?}");
            for stroke in &strokes {
                assert!(stroke.points.len() >= 2, "{kind:?}");
                assert!(stroke.points.iter().all(|at| at.is_finite()), "{kind:?}");
            }
            // The outline is the boundary of the received fields and nothing else.
            let geo = geometry::boundary_shape(skill, kind, &e);
            if kind == K::Soul {
                // No reach is replicated for a soul: a mark, and no ring.
                assert_eq!(geo, GeoShape::None);
                assert_eq!(boundary(&e).len(), 1);
                assert_eq!(boundary(&e)[0].len(), 5);
            } else {
                assert_eq!(boundary(&e), geo.outline(), "{kind:?}");
            }
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        // A new kind does not compile in `effect_strokes` until it has an arm; this list
        // keeps the table above complete.
        assert_eq!(kinds.len(), 14);
    }

    #[test]
    fn a_staged_body_replaces_the_fallback_lines_of_its_effect_in_3d() {
        use EffectVisualKind as K;
        use PlayerVisualMode::{Models3d, Sprite2d};
        for kind in [K::Trap, K::Field, K::Bolt, K::Cage] {
            // The flat view has no body: every effect keeps its lines.
            for (known, staged) in [(false, false), (true, false), (true, true)] {
                assert!(outlined(Sprite2d, kind, known, staged), "{kind:?}");
            }
            // An effect of no known row is drawn by its lines alone.
            assert!(outlined(Models3d, kind, false, false), "{kind:?}");
            // A body carries the boundary and the team colour of its effect, a trap's too.
            assert!(!outlined(Models3d, kind, true, true), "{kind:?}");
            // A row without a body still draws its effect, except for a trap: its model
            // has no boundary, so the tactical outline stays.
            assert_eq!(outlined(Models3d, kind, true, false), kind == K::Trap);
        }
    }

    #[test]
    fn the_fissure_is_a_capsule_the_cone_a_sector_and_the_collapse_a_ring() {
        use EffectVisualKind as K;
        let fissure = effect(SkillId::WinterDivide, K::BeamWarning);
        assert_eq!(
            boundary(&fissure),
            GeoShape::Capsule {
                from: Vec2::new(3.0, -2.0),
                to: Vec2::new(3.0, 5.0),
                radius: 1.5
            }
            .outline()
        );
        // Both round ends are part of the outline.
        let tips = [Vec2::new(3.0, 6.5), Vec2::new(3.0, -3.5)];
        for tip in tips {
            assert!(
                boundary(&fissure)[0]
                    .iter()
                    .any(|at| at.distance(tip) < 1e-3)
            );
        }
        // It has no timed telegraph: no read-out is drawn inside it.
        assert_eq!(effect_strokes(&fissure).len(), 1);

        // The cone at its full range is a sector with the server's half-angle; a cone the
        // fog cut short is the received line and nothing wider.
        let range = shared::loadout::skill(SkillId::FurnaceBreath)
            .ability
            .cast_range;
        let mut cone = effect(SkillId::FurnaceBreath, K::BeamWarning);
        cone.end = [3.0, -2.0 + range];
        let outline = boundary(&cone);
        let widest = outline[0]
            .iter()
            .map(|at| (*at - Vec2::new(3.0, -2.0)).angle_to(Vec2::Y).abs())
            .fold(0.0, f32::max);
        assert!((widest.cos() - geometry::FURNACE_CONE_COS).abs() < 1e-4);
        cone.end = [3.0, -2.0 + range - 1.0];
        assert_eq!(
            boundary(&cone),
            [vec![Vec2::new(3.0, -2.0), Vec2::new(3.0, -3.0 + range)]]
        );

        // The collapse is a ring of the replicated radius. Its inner ring is full exactly
        // when the server fires, also when the warning is first seen late.
        let mut collapse = effect(SkillId::OrbitalCollapse, K::BeamWarning);
        collapse.end = collapse.position;
        collapse.radius = 5.0;
        let mut inner = |remaining: f32| {
            collapse.remaining_secs = remaining;
            let strokes = effect_strokes(&collapse);
            assert_eq!(
                strokes[0].points,
                GeoShape::Ring {
                    center: Vec2::new(3.0, -2.0),
                    radius: 5.0
                }
                .outline()[0]
            );
            strokes
                .get(1)
                .map(|stroke| (stroke.ink, stroke.points[0].distance(Vec2::new(3.0, -2.0))))
        };
        assert_eq!(inner(0.9), None);
        let (ink, half) = inner(0.55).unwrap();
        assert_eq!(ink, Ink::Team);
        assert!((half - 2.5).abs() < 1e-4);
        assert!((inner(0.2).unwrap().1 - 5.0).abs() < 1e-4);
        // Below the tail the ring stays on the edge and never leaves it.
        assert!((inner(0.05).unwrap().1 - 5.0).abs() < 1e-4);
    }

    #[test]
    fn a_cage_draws_only_the_sides_it_still_has() {
        let mut cage = effect(SkillId::IronBoundary, EffectVisualKind::Cage);
        cage.radius = 6.0;
        let sides = GeoShape::Pentagon {
            center: Vec2::new(3.0, -2.0),
            radius: 6.0,
        }
        .outline();
        assert_eq!(boundary(&cage), sides);
        cage.consumed_segments = 0b01010;
        assert_eq!(
            boundary(&cage),
            [sides[0].clone(), sides[2].clone(), sides[4].clone()]
        );
        cage.consumed_segments = 0b11111;
        assert!(effect_strokes(&cage).is_empty());
    }

    #[test]
    fn reading_aids_stay_with_the_received_geometry() {
        use EffectVisualKind as K;
        let centre = Vec2::new(3.0, -2.0);
        // The inner ring of a round effect and the cross of a trap lie inside its radius.
        let mut trap = effect(SkillId::WildTraps, K::Trap);
        let aids = |e: &SkillEffectState| -> Vec<Stroke> {
            effect_strokes(e)
                .into_iter()
                .filter(|stroke| stroke.ink != Ink::Skill)
                .collect()
        };
        for stroke in aids(&trap) {
            assert!(stroke.points.iter().all(|at| at.distance(centre) <= 1.5));
        }
        assert_eq!(
            aids(&trap).iter().map(|s| s.ink).collect::<Vec<_>>(),
            [Ink::TeamFaint, Ink::Arming, Ink::Arming]
        );
        trap.armed = true;
        assert_eq!(aids(&trap)[1].ink, Ink::Armed);
        // A fired beam is hatched between its two edges.
        let beam = effect(SkillId::DawnRay, K::Beam);
        let hatch = aids(&beam);
        assert_eq!(hatch.len(), 5);
        for stroke in hatch {
            assert!(stroke.points.iter().all(|at| (at.x - 3.0).abs() < 1.5));
            assert_eq!((stroke.points[0].y, stroke.points[1].y), (-2.0, 5.0));
        }
        // A travelling body shows its replicated heading and no second ring.
        for (skill, kind) in [
            (SkillId::WinterShard, K::Bolt),
            (SkillId::DawnBarrier, K::Barrier),
            (SkillId::WildRocket, K::Rocket),
        ] {
            let mut body = effect(skill, kind);
            body.end = [3.0, -1.0];
            assert_eq!(
                aids(&body),
                [Stroke {
                    points: vec![Vec2::new(3.0, -6.5), Vec2::new(3.0, -0.5)],
                    ink: Ink::Team
                }],
                "{kind:?}"
            );
        }
    }

    /// A ring of `state_marks` as its distance from `p` and its colour.
    fn rings(marks: &[HeroMark], p: Vec2) -> Vec<(f32, Color)> {
        marks
            .iter()
            .map(|mark| {
                let radius = mark.points[0].distance(p);
                assert!(
                    mark.points
                        .iter()
                        .all(|at| (at.distance(p) - radius).abs() < 1e-4)
                );
                ((radius * 100.0).round() / 100.0, mark.color)
            })
            .collect()
    }

    #[test]
    fn a_state_keeps_its_gizmo_unless_the_mesh_visual_shows_it() {
        use PlayerVisualMode::{Models3d, Sprite2d};
        let p = Vec2::new(3.0, -2.0);
        let pink = Color::srgb(1.0, 0.3, 0.55);
        let cyan = Color::srgb(0.5, 0.9, 1.0);
        let yellow = Color::srgb(1.0, 0.9, 0.3);
        let slow = (SLOW_RING, Color::srgb(0.4, 0.65, 1.0));
        let marks = |mode, visible, alive, flags: &LoadoutState| {
            rings(&hero_state_marks(mode, visible, alive, flags, p), p)
        };

        // Shield, mark and slow together. In 3D the shield is the mesh visual, so its
        // ring is left out; the mark keeps its ring and the slow its small one.
        let flags = LoadoutState {
            shield_hp: 20.0,
            mark_remaining_secs: 2.0,
            slow_multiplier: 0.6,
            ..default()
        };
        assert_eq!(marks(Models3d, true, true, &flags), [(1.15, yellow), slow]);
        // The flat view has no mesh visual: every state keeps its gizmo.
        let all = [(0.95, cyan), (1.15, yellow), slow];
        assert_eq!(marks(Sprite2d, true, true, &flags), all);
        // A hero without a mesh visual in 3D (not drawn, or dead) keeps them as well.
        assert_eq!(marks(Models3d, false, true, &flags), all);
        assert_eq!(marks(Models3d, true, false, &flags), all);

        // A slow alone is the mesh visual in 3D and the small ring in the flat view.
        let slowed = LoadoutState {
            slow_multiplier: 0.5,
            ..default()
        };
        assert!(marks(Models3d, true, true, &slowed).is_empty());
        // The smallest ring a hero can have, inside the corners of its kit mark.
        assert_eq!(marks(Sprite2d, true, true, &slowed), [slow]);
        assert_eq!(slow.0, 0.42);

        // A stun is replicated with a root of the same length: one ring in the flat view,
        // as before, and none under the stars in 3D.
        let stunned = LoadoutState {
            stun_remaining_secs: 1.0,
            root_remaining_secs: 1.0,
            ..default()
        };
        assert_eq!(marks(Sprite2d, true, true, &stunned), [(0.7, pink)]);
        assert!(marks(Models3d, true, true, &stunned).is_empty());
        let rooted = LoadoutState {
            root_remaining_secs: 1.0,
            ..default()
        };
        assert_eq!(marks(Sprite2d, true, true, &rooted), [(0.7, pink)]);
        assert!(marks(Models3d, true, true, &rooted).is_empty());

        // The rings every state has had, by state; camouflage and forging have none.
        let every = LoadoutState {
            stun_remaining_secs: 1.0,
            root_remaining_secs: 1.0,
            parrying: true,
            shield_hp: 20.0,
            mark_remaining_secs: 2.0,
            brittle: true,
            concussion_stacks: 0,
            slow_multiplier: 0.6,
            camouflaged: true,
            forge_remaining_secs: 1.0,
            ..default()
        };
        assert_eq!(
            marks(Sprite2d, true, true, &every),
            [
                (0.7, pink),
                (1.4, Color::WHITE),
                (0.95, cyan),
                (1.15, yellow),
                (1.2, Color::srgb(1.0, 0.6, 0.1)),
                slow,
            ]
        );
        // In 3D the stars stand for the stun; everything below keeps its gizmo.
        assert_eq!(
            marks(Models3d, true, true, &every),
            marks(Sprite2d, true, true, &every)[1..]
        );
        // Concussion pips stay small rings beside the hero, one per stack.
        for stacks in 0..=6u8 {
            let flags = LoadoutState {
                concussion_stacks: stacks,
                ..default()
            };
            let pips = hero_state_marks(Sprite2d, true, true, &flags, p);
            assert_eq!(pips.len(), usize::from(stacks.min(4)));
            for (i, pip) in pips.iter().enumerate() {
                let center = p + Vec2::new(-0.6 + i as f32 * 0.4, 1.4);
                assert!(
                    pip.points
                        .iter()
                        .all(|at| (at.distance(center) - 0.12).abs() < 1e-4)
                );
            }
            // In 3D the pips are the mesh visual.
            assert!(hero_state_marks(Models3d, true, true, &flags, p).is_empty());
        }
        // No flag, no gizmo.
        assert!(hero_state_marks(Sprite2d, true, true, &LoadoutState::default(), p).is_empty());
    }

    #[test]
    fn recast_markers_are_ground_lines_at_the_feet_of_the_hero() {
        let p = Vec2::new(3.0, -2.0);
        for marker in RecastMarker::ALL {
            for step in 0..24 {
                let forward = Vec2::from_angle(step as f32 * 0.4);
                let now = step as f32 * 0.31;
                let strokes = marker_strokes(*marker, p, forward, now);
                assert!(!strokes.is_empty(), "{}", marker.id());
                for stroke in &strokes {
                    assert!(stroke.len() >= 2, "{}", marker.id());
                    for at in stroke {
                        assert!(at.is_finite(), "{}", marker.id());
                        // Around the feet, clear of the body and within two and a half
                        // units: a marker claims no area.
                        let reach = at.distance(p);
                        assert!((0.6..=2.5).contains(&reach), "{} {reach}", marker.id());
                    }
                }
            }
        }

        // `ring_pips`: a ring and four pips on it, the same at every moment and facing.
        let pips = marker_strokes(RecastMarker::RingPips, p, Vec2::X, 0.0);
        assert_eq!(pips.len(), 5);
        assert_eq!(pips[0], circle(p, MARKER_RING));
        for pip in &pips[1..] {
            let center = pip[..4].iter().sum::<Vec2>() / 4.0;
            assert!((center.distance(p) - MARKER_RING).abs() < 1e-4);
        }
        assert_eq!(
            pips,
            marker_strokes(RecastMarker::RingPips, p, Vec2::Y, 9.0)
        );

        // `orbit_motes`: two marks opposite each other that circle the feet.
        let center = |stroke: &Vec<Vec2>| stroke[..4].iter().sum::<Vec2>() / 4.0;
        let motes = |now: f32| -> Vec<Vec2> {
            marker_strokes(RecastMarker::OrbitMotes, p, Vec2::X, now)
                .iter()
                .map(center)
                .collect()
        };
        let (early, late) = (motes(0.0), motes(0.5));
        assert_eq!(early.len(), 2);
        for at in early.iter().chain(&late) {
            assert!((at.distance(p) - MARKER_RING).abs() < 1e-4);
        }
        assert!((early[0] + early[1] - p * 2.0).length() < 1e-4);
        let turned = (early[0] - p).angle_to(late[0] - p);
        assert!((turned - 0.5 * MARKER_TURN).abs() < 1e-4);

        // `ground_arrows`: three arrows ahead of the hero that point the way it faces.
        for forward in [Vec2::X, Vec2::NEG_Y, Vec2::new(0.6, 0.8)] {
            let arrows = marker_strokes(RecastMarker::GroundArrows, p, forward, 3.0);
            assert_eq!(arrows.len(), 3);
            let mut last = 0.0;
            for arrow in &arrows {
                assert_eq!(arrow.len(), 3);
                let tip = arrow[1];
                // The tip lies on the facing line, ahead of both wings and of the last tip.
                assert!((tip - p).perp_dot(forward).abs() < 1e-4);
                let ahead = (tip - p).dot(forward);
                assert!(ahead > last);
                last = ahead;
                for wing in [arrow[0], arrow[2]] {
                    assert!((wing - p).dot(forward) < ahead);
                    assert!((wing - p).dot(forward) > 0.0);
                }
                assert!(
                    ((arrow[0] - tip) + (arrow[2] - tip))
                        .perp_dot(forward)
                        .abs()
                        < 1e-4
                );
            }
        }
    }

    #[test]
    fn the_orb_aid_takes_the_colour_of_the_skill_of_the_replicated_orb() {
        // The final rows give every skill of the kit a colour of its own.
        let registry = SkillPresentation::target();
        let lilac = Color::srgb(0.9, 0.7, 1.0);
        let mut orb = effect(SkillId::OrbitalCommand, EffectVisualKind::Orb);
        let skill = |id: SkillId| Color::srgb_from_array(registry.profile(id).unwrap().color);
        assert_eq!(
            orb_aid_color(Some(&registry), Some(7), std::slice::from_ref(&orb)),
            skill(SkillId::OrbitalCommand)
        );
        // The orb is re-ordered by another skill of the kit: the aid follows it.
        orb.skill = SkillId::OrbitalGuard;
        assert_eq!(
            orb_aid_color(Some(&registry), Some(7), std::slice::from_ref(&orb)),
            skill(SkillId::OrbitalGuard)
        );
        assert_ne!(skill(SkillId::OrbitalGuard), skill(SkillId::OrbitalCommand));
        // Another hero's orb, a hidden owner, another kind of effect or no registry give
        // the neutral colour.
        assert_eq!(
            orb_aid_color(Some(&registry), Some(8), std::slice::from_ref(&orb)),
            lilac
        );
        assert_eq!(
            orb_aid_color(Some(&registry), None, std::slice::from_ref(&orb)),
            lilac
        );
        assert_eq!(
            orb_aid_color(None, Some(7), std::slice::from_ref(&orb)),
            lilac
        );
        let mut hidden = orb.clone();
        hidden.owner_id = 0;
        assert_eq!(orb_aid_color(Some(&registry), Some(0), &[hidden]), lilac);
        orb.kind = EffectVisualKind::Field;
        assert_eq!(orb_aid_color(Some(&registry), Some(7), &[orb]), lilac);
    }

    #[test]
    fn hero_gizmos_are_the_orb_aid_of_the_flat_view_the_states_and_the_offered_recasts() {
        use PlayerVisualMode::{Models3d, Sprite2d};
        use shared::loadout::CoreId;
        const P: Vec2 = Vec2::new(3.0, -2.0);
        fn sight<'a>(
            registry: &'a SkillPresentation,
            mode: PlayerVisualMode,
            class: HeroClass,
            flags: &'a LoadoutState,
            effects: &'a [SkillEffectState],
        ) -> HeroSight<'a> {
            HeroSight {
                mode,
                visible: true,
                alive: true,
                class: Some(class),
                id: Some(7),
                flags,
                p: P,
                forward: Vec2::X,
                now: 0.0,
                profiles: Some(registry),
                effects,
            }
        }
        let registry = SkillPresentation::target();
        let (registry, p) = (&registry, P);

        // The owner's orb: a ring around it and a faint line to it, in the colour of the
        // skill the replicated orb belongs to. Only the flat view draws the aid.
        let at = Vec2::new(6.0, 1.0);
        let orbiting = LoadoutState {
            recipe: Some(CoreId::Orbitwright.preset()),
            orb_position: Some(at.to_array()),
            ..default()
        };
        let orbs = [effect(SkillId::OrbitalCommand, EffectVisualKind::Orb)];
        let colour =
            Color::srgb_from_array(registry.profile(SkillId::OrbitalCommand).unwrap().color);
        assert_eq!(
            hero_marks(&sight(
                registry,
                Sprite2d,
                HeroClass::Orbitwright,
                &orbiting,
                &orbs
            )),
            [
                HeroMark {
                    points: circle(at, 0.65),
                    color: colour
                },
                HeroMark {
                    points: vec![p, at],
                    color: colour.with_alpha(0.3)
                },
            ]
        );
        assert!(
            hero_marks(&sight(
                registry,
                Models3d,
                HeroClass::Orbitwright,
                &orbiting,
                &orbs
            ))
            .is_empty()
        );

        // A recast the server offers: the marker of the row, in the colour of its skill,
        // in both views. Dawn Field is the E of the Dawnweaver kit.
        let mut casting = LoadoutState {
            recipe: Some(CoreId::Dawnweaver.preset()),
            ..default()
        };
        let slot = casting
            .recipe
            .as_ref()
            .unwrap()
            .skills
            .iter()
            .position(|skill| *skill == SkillId::DawnField)
            .unwrap();
        for mode in [Models3d, Sprite2d] {
            assert!(
                hero_marks(&sight(registry, mode, HeroClass::Dawnweaver, &casting, &[])).is_empty()
            );
        }
        casting.slots[slot].can_recast = true;
        let field = Color::srgb_from_array(registry.profile(SkillId::DawnField).unwrap().color);
        let pips: Vec<_> = marker_strokes(RecastMarker::RingPips, p, Vec2::X, 0.0)
            .into_iter()
            .map(|points| HeroMark {
                points,
                color: field,
            })
            .collect();
        for mode in [Models3d, Sprite2d] {
            let offered = sight(registry, mode, HeroClass::Dawnweaver, &casting, &[]);
            assert_eq!(hero_marks(&offered), pips);
            // Not for a hero that is hidden or dead, nor without its class, its id or
            // the registry.
            for unseen in [
                HeroSight {
                    visible: false,
                    ..sight(registry, mode, HeroClass::Dawnweaver, &casting, &[])
                },
                HeroSight {
                    alive: false,
                    ..sight(registry, mode, HeroClass::Dawnweaver, &casting, &[])
                },
                HeroSight {
                    class: None,
                    ..sight(registry, mode, HeroClass::Dawnweaver, &casting, &[])
                },
                HeroSight {
                    id: None,
                    ..sight(registry, mode, HeroClass::Dawnweaver, &casting, &[])
                },
                HeroSight {
                    profiles: None,
                    ..sight(registry, mode, HeroClass::Dawnweaver, &casting, &[])
                },
            ] {
                assert!(hero_marks(&unseen).is_empty());
            }
        }
        // The marker joins the state gizmos of the hero; it replaces none of them.
        casting.mark_remaining_secs = 2.0;
        casting.shield_hp = 10.0;
        let both = hero_marks(&sight(
            registry,
            Models3d,
            HeroClass::Dawnweaver,
            &casting,
            &[],
        ));
        assert_eq!(both.len(), 1 + pips.len());
        assert_eq!(both[1..], pips[..]);
        assert_eq!(
            both[..1],
            hero_state_marks(Models3d, true, true, &casting, p)[..]
        );
    }

    /// The four sides in the order the server counts them: the attacker stands to +x, +z,
    /// -x or -z of the hero it strikes.
    const SIDES: [Vec2; 4] = [Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y];
    const GOLD: Color = Color::srgb(1.0, 0.75, 0.2);
    const GREY: Color = Color::srgb(0.3, 0.35, 0.4);

    fn duelist(flags: &LoadoutState, own: bool) -> Duelist<'_> {
        Duelist {
            flags,
            team: Some(crate::team::Team::Green),
            own,
        }
    }

    fn hostile(id: u64, p: Vec2) -> FacetTarget {
        FacetTarget {
            id,
            team: Some(crate::team::Team::Blue),
            seen: true,
            p,
        }
    }

    /// The sides `marks` draw around `p`, each with whether it is struck. Every mark must
    /// be one of the two glyphs: a closed gold diamond with the line of the hit through it,
    /// or two grey halves that do not meet.
    fn facets(marks: &[HeroMark], p: Vec2) -> Vec<(usize, bool)> {
        let near = |a: Vec2, b: Vec2| a.distance(b) < 1e-4;
        marks
            .chunks(2)
            .map(|glyph| {
                let [first, second] = glyph else {
                    panic!("a side is two lines: {glyph:?}");
                };
                let middle = first.points[..first.points.len().min(4)]
                    .iter()
                    .chain(&second.points)
                    .fold(Vec2::ZERO, |sum, at| sum + *at)
                    / (first.points.len().min(4) + second.points.len()) as f32;
                let side = SIDES
                    .iter()
                    .position(|out| near(p + *out * 1.4, middle))
                    .unwrap_or_else(|| panic!("no side at {middle}"));
                let (out, center) = (SIDES[side], p + SIDES[side] * 1.4);
                let (tip, wing) = (out * 0.42, out.perp() * 0.2);
                let struck = first.color == GREY;
                if struck {
                    assert_eq!(second.color, GREY);
                    for (half, sign) in [(first, 1.0), (second, -1.0)] {
                        let base = center + out * 0.075 * sign;
                        let expected = [base + wing, center + tip * sign, base - wing];
                        assert_eq!(half.points.len(), 3);
                        assert!(
                            half.points
                                .iter()
                                .zip(expected)
                                .all(|(at, expected)| near(*at, expected)),
                            "{half:?}"
                        );
                    }
                    // The halves are 0.15 apart and stay inside the diamond they were.
                    assert!(near(first.points[0] - second.points[0], out * FACET_GAP));
                } else {
                    assert_eq!((first.color, second.color), (GOLD, GOLD));
                    let outline = [
                        center + tip,
                        center + wing,
                        center - tip,
                        center - wing,
                        center + tip,
                    ];
                    assert_eq!(first.points.len(), 5);
                    assert!(
                        first
                            .points
                            .iter()
                            .zip(outline)
                            .all(|(at, expected)| near(*at, expected)),
                        "{first:?}"
                    );
                    assert_eq!(second.points.len(), 2);
                    assert!(
                        near(second.points[0], center - tip)
                            && near(second.points[1], center + tip)
                    );
                }
                (side, struck)
            })
            .collect()
    }

    /// The sides of a duel are drawn for no one but a duelist: the rotating side needs the
    /// Vitals passive of the local hero, the four sides need the challenge, and a recipe
    /// that has the challenge without the passive shows nothing on the other enemies.
    #[test]
    fn duel_facets_need_the_vitals_passive_or_a_challenge() {
        use shared::loadout::{CoreId, PassiveId};
        const P: Vec2 = Vec2::new(3.0, -2.0);
        let edgeweaver = CoreId::Edgeweaver.preset();
        assert_eq!(edgeweaver.passive, PassiveId::Vitals);
        let mixed = shared::loadout::BuildRecipe {
            passive: PassiveId::Tempered,
            ..edgeweaver.clone()
        };
        let sides = |flags: &LoadoutState, own: bool, id: u64| {
            facets(&duel_facets(&duelist(flags, own), &hostile(id, P)), P)
        };

        // Neither the passive nor a challenge: nothing, whoever the state belongs to.
        for recipe in [
            None,
            Some(mixed.clone()),
            Some(CoreId::Cinderforge.preset()),
        ] {
            for rotation in 0..4 {
                let flags = LoadoutState {
                    recipe: recipe.clone(),
                    vital_rotation: rotation,
                    challenge_sides: 0b0101,
                    ..default()
                };
                for (own, id) in [(true, 9), (false, 9), (true, 10)] {
                    assert!(sides(&flags, own, id).is_empty(), "{recipe:?}");
                }
            }
        }

        // The passive alone: the one side the server expects next, `(id + rotation) % 4`
        // (`common/src/skills/advanced.rs:1689`), on every hostile hero, for its owner only.
        for rotation in 0..4u8 {
            for id in [0u64, 1, 2, 3, 9, 10, 255, 256, 1027] {
                let flags = LoadoutState {
                    recipe: Some(edgeweaver.clone()),
                    vital_rotation: rotation,
                    ..default()
                };
                let expected = usize::from((id as u8).wrapping_add(rotation) % 4);
                assert_eq!(
                    sides(&flags, true, id),
                    [(expected, false)],
                    "{id} {rotation}"
                );
                assert!(sides(&flags, false, id).is_empty());
            }
        }

        // A challenge without the passive: four sides on the challenged hero and nothing on
        // any other. The state may be anyone's: the server names the target.
        let challenge = LoadoutState {
            recipe: Some(mixed),
            vital_rotation: 2,
            challenge_target: Some(9),
            ..default()
        };
        let all = [(0, false), (1, false), (2, false), (3, false)];
        for own in [true, false] {
            assert_eq!(sides(&challenge, own, 9), all);
            assert!(sides(&challenge, own, 10).is_empty());
        }

        // Both: four on the challenged hero, the rotating side on the others.
        let both = LoadoutState {
            recipe: Some(edgeweaver),
            ..challenge.clone()
        };
        assert_eq!(sides(&both, true, 9), all);
        assert_eq!(sides(&both, true, 10), [(0, false)]);
        assert_eq!(sides(&both, false, 9), all);
        assert!(sides(&both, false, 10).is_empty());

        // Never around an ally, the duelist itself, a hero that is not drawn or a dead one.
        let own = duelist(&both, true);
        for target in [
            FacetTarget {
                team: own.team,
                ..hostile(9, P)
            },
            FacetTarget {
                seen: false,
                ..hostile(9, P)
            },
            FacetTarget {
                seen: false,
                ..hostile(10, P)
            },
        ] {
            assert!(duel_facets(&own, &target).is_empty());
        }
    }

    /// A side of a challenge is whole and gold until the server reports it struck, then
    /// two grey halves: the drawing follows `challenge_sides` bit for bit and stays a
    /// ground glyph 1.4 units from the hero.
    #[test]
    fn duel_facets_break_with_the_replicated_challenge_sides() {
        const P: Vec2 = Vec2::new(-4.0, 7.5);
        for mask in 0..16u8 {
            let flags = LoadoutState {
                challenge_target: Some(9),
                challenge_sides: mask,
                ..default()
            };
            let marks = duel_facets(&duelist(&flags, true), &hostile(9, P));
            assert_eq!(marks.len(), 8, "{mask}");
            assert_eq!(
                facets(&marks, P),
                (0..4)
                    .map(|side| (side, mask & (1 << side) != 0))
                    .collect::<Vec<_>>(),
                "{mask}"
            );
            // Around the hero, never on it: outside its bars and its state rings.
            for at in marks.iter().flat_map(|mark| &mark.points) {
                let distance = at.distance(P);
                assert!(
                    (FACET_DISTANCE - FACET_LENGTH - 1e-4..=FACET_DISTANCE + FACET_LENGTH + 1e-4)
                        .contains(&distance),
                    "{mask} {distance}"
                );
            }
        }
        // Bits above the four sides are not sides.
        let flags = LoadoutState {
            challenge_target: Some(9),
            challenge_sides: 0b1111_0000,
            ..default()
        };
        let marks = duel_facets(&duelist(&flags, true), &hostile(9, P));
        assert!(facets(&marks, P).iter().all(|(_, struck)| !struck));
    }

    #[test]
    fn point_aim_clamps_but_direction_retains_its_ray() {
        assert_eq!(
            bounded_aim(Vec2::ZERO, Vec2::X * 50.0, TargetingMode::Point, 16.0),
            Vec2::X * 16.0
        );
        assert_eq!(
            bounded_aim(Vec2::ZERO, Vec2::X * 50.0, TargetingMode::Direction, 16.0),
            Vec2::X * 50.0
        );
        assert_eq!(
            bounded_aim(Vec2::ONE, Vec2::ZERO, TargetingMode::SelfTarget, 0.0),
            Vec2::ONE
        );
    }
}

#[derive(Component)]
pub(super) struct InteractButton;

/// One explicit action for keyboard, touch and the controller's left-stick click.
pub(super) fn interact(
    game: Option<Res<GameStateSnapshot>>,
    context: Res<crate::input_context::GameplayInputContext>,
    local: Query<(&Transform, &crate::team::Team, &super::CombatStats), With<Player>>,
    keys: Res<ButtonInput<KeyCode>>,
    pad: Res<crate::gamepad::GamepadControls>,
    mut held: Local<bool>,
    mut buttons: Query<(&mut Node, &Interaction, Ref<Interaction>), With<InteractButton>>,
    mut commands: MessageWriter<crate::net::NetworkCommand>,
) {
    let down = pad.active
        && pad
            .raw
            .is_some_and(|p| p.buttons & crate::gamepad::snapshot::L3 != 0);
    let pressed = down && !*held;
    *held = down;
    let nearest = game.as_ref().and_then(|game| {
        local
            .single()
            .ok()
            .filter(|(_, _, stats)| stats.is_alive() && context.gameplay_allowed())
            .and_then(|(pose, team, _)| {
                game.skill_effects
                    .iter()
                    .filter(|e| {
                        e.owner_team == *team
                            && e.owner_id != game.your_id
                            && matches!(
                                shared::loadout::skill(e.skill).effect,
                                shared::loadout::SkillEffect::Technique {
                                    action: shared::loadout::Technique::Lantern,
                                    ..
                                }
                            )
                            && Vec2::from_array(e.position).distance(pose.translation.xz()) <= 3.0
                    })
                    .min_by_key(|e| e.id)
                    .map(|e| e.id)
            })
    });
    let mut clicked = false;
    for (mut node, interaction, changed) in &mut buttons {
        node.display = if nearest.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        clicked |= changed.is_changed() && *interaction == Interaction::Pressed;
    }
    if let Some(object_id) =
        nearest.filter(|_| keys.just_pressed(KeyCode::KeyF) || pressed || clicked)
    {
        commands.write(crate::net::NetworkCommand::Interact { object_id });
    }
}

/// A pooled, clipped preview on the map shares the exact world aim vector.
#[derive(Component)]
pub(super) struct MinimapAimLine;
pub(super) fn draw_minimap_aim(
    mut commands: Commands,
    vector: Res<SkillAimVector>,
    layout: Res<crate::maps::MapLayout>,
    container: Query<Entity, With<crate::minimap::MinimapContainer>>,
    mut lines: Query<(&mut Node, &mut UiTransform), With<MinimapAimLine>>,
) {
    use crate::minimap::{clip_map_segment, line_node, map_point};
    let segment = vector.0.and_then(|(a, b, _)| {
        clip_map_segment(
            map_point(*layout, Vec3::new(a.x, 0.0, a.y)),
            map_point(*layout, Vec3::new(b.x, 0.0, b.y)),
        )
    });
    if let Ok((mut node, mut transform)) = lines.single_mut() {
        if let Some((a, b)) = segment {
            let (n, t) = line_node(a, b, 5.0);
            *node = n;
            *transform = t;
        } else {
            node.display = Display::None;
        }
    } else if let (Some((a, b)), Ok(parent)) = (segment, container.single()) {
        let (node, transform) = line_node(a, b, 5.0);
        commands.spawn((
            node,
            transform,
            BackgroundColor(Color::srgb(0.12, 0.65, 1.0)),
            ZIndex(30),
            MinimapAimLine,
            Name::new("MinimapSkillVector"),
            ChildOf(parent),
        ));
    }
}

pub(super) fn draw_minimap_traps(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    layout: Res<crate::maps::MapLayout>,
    container: Query<Entity, With<crate::minimap::MinimapContainer>>,
    local: Query<&crate::team::Team, With<Player>>,
    mut pool: Local<Vec<Entity>>,
) {
    let Ok(parent) = container.single() else {
        return;
    };
    let traps: Vec<_> = game
        .skill_effects
        .iter()
        .filter(|e| e.kind == EffectVisualKind::Trap && e.position.into_iter().all(f32::is_finite))
        .take(32)
        .collect();
    for (i, e) in traps.iter().enumerate() {
        let p = crate::minimap::map_point(*layout, Vec3::new(e.position[0], 0.0, e.position[1]));
        let color = if local.single().is_ok_and(|t| *t == e.owner_team) {
            Color::srgb(0.12, 0.65, 1.0)
        } else {
            Color::srgb(1.0, 0.22, 0.08)
        };
        let node = Node {
            position_type: PositionType::Absolute,
            left: Val::Px(p.x - 4.0),
            top: Val::Px(p.y - 4.0),
            width: Val::Px(8.0),
            height: Val::Px(8.0),
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::MAX,
            ..default()
        };
        let fill = BackgroundColor(if e.armed { color } else { Color::BLACK });
        if let Some(id) = pool.get(i) {
            commands
                .entity(*id)
                .insert((node, fill, BorderColor::all(color)));
        } else {
            pool.push(
                commands
                    .spawn((
                        node,
                        fill,
                        BorderColor::all(color),
                        ZIndex(12),
                        ChildOf(parent),
                        Name::new("MinimapTrap"),
                    ))
                    .id(),
            );
        }
    }
    for id in pool.iter().skip(traps.len()) {
        commands.entity(*id).insert(Node {
            display: Display::None,
            ..default()
        });
    }
}
