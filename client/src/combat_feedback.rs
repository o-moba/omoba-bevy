//! Presentation consumes accepted server hits, never HP deltas or projectile disappearance.
// i18n-strict
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::ui::FocusPolicy;
use shared::combat::{CombatEntityKind, CombatEvent};
use shared::loadout::{LoadoutState, SkillEffectState};

use crate::{
    camera::MainCamera,
    combat_visuals::CombatVisualRegistry,
    game_vfx::{ClearCombatVfx, ConfirmedBurst, ImpactBurst, ParticleSpec, SkillBurst},
    maps::MapLayout,
    net::{
        GameStateSnapshot, NetworkAvatar, NetworkHeroClass, NetworkNeutralCampType,
        NetworkNeutralId, NetworkPlayerId, NetworkSpriteCharacter,
    },
    player::Player,
    skill_presentation::{
        SkillPresentation, accents,
        cast::CastKey,
        impacts::{Receipt, receipt_burst},
        vocab::PaletteSlot,
    },
    sprite::PlayerVisualMode,
    world2d::{layer, simulation_xz_to_render_xy},
};

const MAX_HITS: usize = 96;
const MAX_NUMBERS: usize = 48;
const NUMBER_LIFETIME: f32 = 0.95;

fn vital_break(event: &CombatEvent, target_visible: bool) -> bool {
    event.near_lethal
        && target_visible
        && event.target.kind == CombatEntityKind::Player
        && !event.killed
        && event.amount.is_finite()
        && event.amount > 0.0
}

pub struct CombatFeedbackPlugin;
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CollectCombatFeedback;
impl Plugin for CombatFeedbackPlugin {
    fn build(&self, app: &mut App) {
        crate::vfx_clock::ensure(app);
        #[cfg(feature = "qa")]
        app.init_resource::<ReceiptLooks>();
        app.init_resource::<CombatFeedback>()
            .add_message::<ConfirmedHit>()
            .add_systems(
                Update,
                collect_hits
                    .after(crate::net::ClientNetPipeline::ApplySnapshot)
                    .in_set(CollectCombatFeedback),
            )
            .add_systems(
                PostUpdate,
                (place_numbers, draw_impacts)
                    .after(bevy::camera::CameraUpdateSystems)
                    .before(bevy::ui::UiSystems::Layout),
            );
    }
}

#[derive(Default)]
struct HitCursor {
    round: Option<(u64, u64)>,
    high_water: u64,
}
impl HitCursor {
    fn accept(&mut self, round: (u64, u64), events: &[CombatEvent]) -> (bool, Vec<CombatEvent>) {
        let changed = self.round != Some(round);
        let latest = events.iter().map(|event| event.id).max().unwrap_or(0);
        if changed {
            self.round = Some(round);
            self.high_water = latest;
            // A new connection/round gets a baseline, not a replay of retained history.
            return (true, Vec::new());
        }
        let previous = self.high_water;
        self.high_water = self.high_water.max(latest);
        let mut accepted = Vec::new();
        for event in events.iter().take(MAX_HITS) {
            if event.id <= previous
                || accepted
                    .iter()
                    .any(|seen: &CombatEvent| seen.id == event.id)
            {
                continue;
            }
            if !event.amount.is_finite()
                || event.amount <= 0.0
                || !Vec3::new(event.x, event.y, event.z).is_finite()
                || event.target.kind == CombatEntityKind::Unknown
            {
                continue;
            }
            accepted.push(event.clone());
        }
        (false, accepted)
    }
}

/// A damage receipt the hit cursor accepted from a hero the client sees, for presentation
/// that may only follow a confirmed hit.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub(crate) struct ConfirmedHit {
    pub receipt: u64,
    /// Id of the hero that dealt it.
    pub source: u64,
    pub slot: u8,
    /// The ground under the receipt.
    pub position: Vec3,
}

/// The hero that dealt a receipt, when the client sees it. A hidden or unknown source has
/// none: its hit keeps the built-in burst of the wire style and points nowhere.
#[derive(Clone, Copy)]
struct Source<'a> {
    position: Vec3,
    class: shared::HeroClass,
    avatar: Option<&'a str>,
    sprite: Option<&'a str>,
    loadout: Option<&'a LoadoutState>,
}

impl Source<'_> {
    /// The row of the action that dealt the receipt. A basic attack of a repeater is the
    /// round its wire style names: the shot may land after the hero switched modes.
    fn key(&self, event: &CombatEvent) -> Option<CastKey> {
        CastKey::of(self.class, self.loadout, event.action_slot?)
            .map(|key| key.struck_as(event.style))
    }
}

/// Evidence of what drew the burst of one accepted receipt.
#[cfg(feature = "qa")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReceiptLook {
    pub receipt: u64,
    pub slot: Option<u8>,
    /// The kind of the recipe of the row that dealt the hit; `None` for a built-in burst.
    pub impact: Option<&'static str>,
    /// Particles the recipe asked for.
    pub particles: usize,
}

/// The newest receipts that were drawn, oldest first.
#[cfg(feature = "qa")]
#[derive(Resource, Default)]
pub(crate) struct ReceiptLooks(pub std::collections::VecDeque<ReceiptLook>);

#[cfg(feature = "qa")]
impl ReceiptLook {
    /// The look of a receipt whose burst is `themed`, the output of `themed_impact` for it.
    fn of(
        skills: Option<&SkillPresentation>,
        source: Option<&Source>,
        event: &CombatEvent,
        themed: Option<&[ParticleSpec]>,
    ) -> Self {
        Self {
            receipt: event.id,
            slot: event.action_slot,
            impact: themed.and_then(|_| {
                let look = skills?.look(source?.key(event)?)?;
                Some(look.impact?.kind.id())
            }),
            particles: themed.map_or(0, <[ParticleSpec]>::len),
        }
    }
}

#[cfg(feature = "qa")]
impl ReceiptLooks {
    const KEPT: usize = 32;

    fn record(&mut self, look: ReceiptLook) {
        if self.0.len() == Self::KEPT {
            self.0.pop_front();
        }
        self.0.push_back(look);
    }
}

/// Ground direction of a hit: away from the source the client sees, else none in particular.
fn hit_direction(source: Option<Vec3>, position: Vec3) -> Vec2 {
    source
        .map(|source| (position - source).xz())
        .unwrap_or(Vec2::X)
        .normalize_or(Vec2::X)
}

/// The burst of an accepted receipt from the recipe of the row that dealt it. `None` keeps
/// the built-in burst: the source is not seen or its row carries no recipe. `reserved`
/// particles of the budget of the receipt are left to the engine's own add-on.
fn themed_impact(
    skills: Option<&SkillPresentation>,
    source: Option<&Source>,
    event: &CombatEvent,
    ground: f32,
    effects: &[SkillEffectState],
    reserved: usize,
) -> Option<Vec<ParticleSpec>> {
    let source = source?;
    receipt_burst(
        skills?,
        source.key(event)?,
        &Receipt {
            id: event.id,
            position: Vec3::new(event.x, event.y, event.z),
            ground,
            source: event.source.id,
            source_position: source.position,
            reserved,
        },
        effects,
    )
}

/// The camp hit: drops that rise from a neutral monster when a hero whose class deals
/// bonus damage to it lands a hit, and more of them when that hit kills an ordinary camp.
/// It follows the server's own condition for the bonus (`common/src/sim/neutrals.rs`): a
/// hero without a loadout whose class has a neutral damage multiplier above one. `boss` is
/// what the client last saw of the target; an unseen camp gets no kill flourish. Nothing
/// is drawn for a hero target, a source the client does not see, or a class without the
/// bonus, and nothing says how much was dealt.
fn camp_hit(
    skills: Option<&SkillPresentation>,
    source: Option<&Source>,
    event: &CombatEvent,
    boss: Option<bool>,
    at: Vec3,
) -> Vec<ParticleSpec> {
    let Some((source, theme)) = source.and_then(|source| {
        let theme = skills?.theme(source.class)?;
        Some((source, theme))
    }) else {
        return Vec::new();
    };
    let bonus = |boss| shared::jungle::neutral_damage_multiplier(source.class, boss) > 1.0;
    if event.target.kind != CombatEntityKind::Neutral
        || source.loadout.is_some()
        || !boss.map_or(bonus(false) && bonus(true), bonus)
    {
        return Vec::new();
    }
    accents::camp_hit(
        accents::Palette::of_class(theme).slot(PaletteSlot::Accent),
        at,
        event.killed && boss == Some(false),
        event.id,
    )
}

/// Receipts newer than `seen` of a trap that snapped without dealing damage (a shield took
/// it). The hit cursor passes them over, because they are not hits.
fn trap_snaps(seen: u64, events: &[CombatEvent]) -> Vec<&CombatEvent> {
    let mut snaps: Vec<&CombatEvent> = Vec::new();
    for event in events.iter().take(MAX_HITS) {
        if event.id > seen
            && event.trap_triggered
            && event.amount == 0.0
            && Vec3::new(event.x, event.y, event.z).is_finite()
            && snaps.iter().all(|snap| snap.id != event.id)
        {
            snaps.push(event);
        }
    }
    snaps
}

struct Impact {
    position: Vec3,
    age: f32,
    lifetime: f32,
    scale: f32,
    color: Color,
}
#[derive(Resource, Default)]
struct CombatFeedback {
    cursor: HitCursor,
    impacts: Vec<Impact>,
    /// Whether each neutral monster the client has seen this round is a boss. A killed
    /// monster leaves the snapshot with its last receipt, so its kind is kept from before.
    camps: std::collections::HashMap<u64, bool>,
}
#[derive(Component)]
pub(crate) struct DamageNumber {
    #[cfg(feature = "qa")]
    pub(crate) event_id: u64,
    position: Vec3,
    age: f32,
    lane: f32,
    color: Color,
}

/// Where the receipt collector reports to. It is the only writer of `ConfirmedBurst`.
#[derive(SystemParam)]
struct HitOutput<'w> {
    bursts: MessageWriter<'w, ImpactBurst>,
    confirmed: MessageWriter<'w, ConfirmedBurst>,
    cues: MessageWriter<'w, SkillBurst>,
    hits: MessageWriter<'w, ConfirmedHit>,
    resets: MessageWriter<'w, ClearCombatVfx>,
}

fn collect_hits(
    mut commands: Commands,
    clock: Res<crate::vfx_clock::VfxClock>,
    snapshot: Res<GameStateSnapshot>,
    registry: Res<CombatVisualRegistry>,
    skills: Option<Res<SkillPresentation>>,
    layout: Res<MapLayout>,
    mut feedback: ResMut<CombatFeedback>,
    mut out: HitOutput,
    numbers: Query<Entity, With<DamageNumber>>,
    heroes: Query<(
        &NetworkPlayerId,
        &NetworkHeroClass,
        Option<&NetworkAvatar>,
        Option<&NetworkSpriteCharacter>,
        Option<&crate::net::PlayerLoadout>,
    )>,
    local: Query<&NetworkPlayerId, With<Player>>,
    positions: Query<(&NetworkPlayerId, &Transform, &InheritedVisibility)>,
    neutrals: Query<(&NetworkNeutralId, &NetworkNeutralCampType)>,
    cameras: Query<(&Camera, &Transform), With<MainCamera>>,
    mode: Res<PlayerVisualMode>,
    #[cfg(feature = "qa")] mut shown: ResMut<ReceiptLooks>,
) {
    feedback.impacts.retain_mut(|impact| {
        impact.age += clock.delta;
        impact.age < impact.lifetime
    });
    for (id, camp) in &neutrals {
        feedback.camps.insert(id.0, camp.0.is_boss());
    }
    let seen = feedback.cursor.high_water;
    let (changed, events) = feedback.cursor.accept(
        (snapshot.meta.server_epoch, snapshot.meta.match_id),
        &snapshot.combat_events,
    );
    let mut number_count = numbers.iter().count();
    if changed {
        feedback.impacts.clear();
        feedback.camps.clear();
        out.resets.write(ClearCombatVfx);
        for entity in &numbers {
            commands.entity(entity).despawn();
        }
        number_count = 0;
    }
    let Ok(local_id) = local.single() else {
        return;
    };
    // Cull against the viewed battle, including free-camera/minimap focus.
    let on_screen = |position: Vec3| {
        let render = if *mode == PlayerVisualMode::Models3d {
            position + Vec3::Y * 2.0
        } else {
            simulation_xz_to_render_xy(position).extend(layer::OVERHEAD)
        };
        cameras.single().is_ok_and(|(camera, transform)| {
            let Some(size) = camera.logical_viewport_size() else {
                return false;
            };
            camera
                .world_to_viewport(&GlobalTransform::from(*transform), render)
                .is_ok_and(|p| p.x >= 0.0 && p.y >= 0.0 && p.x <= size.x && p.y <= size.y)
        })
    };
    // Resolve only a visible caster's accepted recipe. Hidden sources retain generic feedback.
    let source_of = |event: &CombatEvent| {
        let position = (event.source.kind == CombatEntityKind::Player)
            .then(|| {
                positions
                    .iter()
                    .find(|(id, _, visible)| id.0 == event.source.id && visible.get())
            })
            .flatten()
            .map(|(_, pose, _)| pose.translation)?;
        heroes.iter().find(|(id, ..)| id.0 == event.source.id).map(
            |(_, class, avatar, sprite, loadout)| Source {
                position,
                class: class.0,
                avatar: avatar.and_then(|v| v.0.as_deref()),
                sprite: sprite.and_then(|v| v.0.as_deref()),
                loadout: loadout.and_then(|l| l.0.as_ref()),
            },
        )
    };
    // Receipt positions are at aim height; ground rings and links need the floor below.
    let ground = |position: Vec3| {
        Vec3::new(
            position.x,
            layout.terrain_height_3d(position.x, position.z),
            position.z,
        )
    };
    if let Some(skills) = skills.as_deref().filter(|_| !changed) {
        for event in trap_snaps(seen, &snapshot.combat_events) {
            let position = Vec3::new(event.x, event.y, event.z);
            let Some(key) = source_of(event)
                .filter(|_| on_screen(position))
                .and_then(|source| source.key(event))
            else {
                continue;
            };
            let cue = accents::trap_snap(skills, key, ground(position), event.id);
            if !cue.is_empty() {
                out.cues.write(SkillBurst(cue));
            }
        }
    }
    for event in events {
        let position = Vec3::new(event.x, event.y, event.z);
        if !on_screen(position) {
            continue;
        }
        let source = source_of(&event);
        let profile = registry.resolve(
            source.map(|source| source.class),
            event.style,
            event.action_slot,
            source.and_then(|source| source.avatar),
            source.and_then(|source| source.sprite),
        );
        let impact_color = source
            .and_then(|source| {
                skills
                    .as_ref()?
                    .action_profile(source.class, source.loadout, event.action_slot?)
                    .map(|p| Color::srgb_from_array(p.color))
            })
            .unwrap_or_else(|| profile.impact.color());
        let vital = vital_break(
            &event,
            positions
                .iter()
                .any(|(id, _, visible)| id.0 == event.target.id && visible.get()),
        );
        // The camp hit shares the budget of its receipt: the recipe leaves it room.
        let drops = camp_hit(
            skills.as_deref(),
            source.as_ref(),
            &event,
            feedback.camps.get(&event.target.id).copied(),
            ground(position),
        );
        // The vital break is the engine's own burst; every other receipt is drawn from the
        // recipe of its row when it has one.
        let themed = (!vital)
            .then(|| {
                themed_impact(
                    skills.as_deref(),
                    source.as_ref(),
                    &event,
                    ground(position).y,
                    &snapshot.skill_effects,
                    drops.len(),
                )
            })
            .flatten();
        #[cfg(feature = "qa")]
        shown.record(ReceiptLook::of(
            skills.as_deref(),
            source.as_ref(),
            &event,
            themed.as_deref(),
        ));
        if themed.is_none() && !drops.is_empty() {
            out.confirmed.write(ConfirmedBurst(drops.clone()));
        }
        if let Some(mut burst) = themed {
            burst.extend(drops);
            out.confirmed.write(ConfirmedBurst(burst));
        } else {
            out.bursts.write(ImpactBurst {
                position,
                direction: hit_direction(source.map(|source| source.position), position),
                color: if vital {
                    Color::srgb(1.0, 0.13, 0.43)
                } else {
                    impact_color
                },
                scale: if vital { 1.25 } else { profile.impact.scale },
                lifetime: if vital { 0.72 } else { profile.impact.lifetime },
                kind: if vital {
                    crate::game_vfx::BurstKind::VitalBreak
                } else {
                    crate::game_vfx::BurstKind::for_style(event.style)
                },
                seed: event.id,
            });
            // The gizmo star belongs to the built-in burst; a recipe is its own mark.
            if feedback.impacts.len() == MAX_HITS {
                feedback.impacts.remove(0);
            }
            feedback.impacts.push(Impact {
                position,
                age: 0.0,
                lifetime: profile.impact.lifetime.clamp(0.08, 1.2),
                scale: profile.impact.scale.clamp(0.1, 2.0),
                color: impact_color,
            });
        }
        if let Some((_, slot)) = source.zip(event.action_slot) {
            out.hits.write(ConfirmedHit {
                receipt: event.id,
                source: event.source.id,
                slot,
                position: ground(position),
            });
        }
        if number_count >= MAX_NUMBERS {
            continue;
        }
        let incoming =
            event.target.kind == CombatEntityKind::Player && event.target.id == local_id.0;
        let outgoing =
            event.source.kind == CombatEntityKind::Player && event.source.id == local_id.0;
        let color = if incoming {
            Color::srgb(1.0, 0.28, 0.23)
        } else if outgoing {
            Color::srgb(1.0, 0.9, 0.45)
        } else {
            Color::srgb(0.85, 0.9, 1.0)
        };
        let text = if event.amount < 1.0 {
            format!("{:.1}", event.amount)
        } else {
            format!("{:.0}", event.amount)
        };
        commands.spawn((
            Name::new("ConfirmedDamageNumber"),
            DamageNumber {
                #[cfg(feature = "qa")]
                event_id: event.id,
                position,
                age: 0.0,
                lane: (event.id % 5) as f32 - 2.0,
                color,
            },
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                ..default()
            },
            Text::new(text),
            TextFont {
                font_size: (if outgoing || incoming { 23.0 } else { 16.0 }).into(),
                ..default()
            },
            TextColor(color),
            TextShadow {
                offset: Vec2::splat(1.0),
                color: Color::srgba(0.0, 0.0, 0.0, 0.8),
            },
            FocusPolicy::Pass,
            ZIndex(45),
        ));
        number_count += 1;
    }
}

fn place_numbers(
    mut commands: Commands,
    clock: Res<crate::vfx_clock::VfxClock>,
    mode: Res<PlayerVisualMode>,
    ui_scale: Option<Res<UiScale>>,
    camera: Query<(&Camera, &Transform), With<MainCamera>>,
    mut numbers: Query<(
        Entity,
        &mut DamageNumber,
        &mut Node,
        &mut TextColor,
        &mut TextShadow,
    )>,
) {
    let camera = camera.single().ok();
    for (entity, mut number, mut node, mut color, mut shadow) in &mut numbers {
        number.age += clock.delta;
        if number.age >= NUMBER_LIFETIME {
            commands.entity(entity).despawn();
            continue;
        }
        let progress = number.age / NUMBER_LIFETIME;
        let position = if *mode == PlayerVisualMode::Models3d {
            number.position + Vec3::Y * 2.0
        } else {
            simulation_xz_to_render_xy(number.position).extend(layer::OVERHEAD)
        };
        let screen = camera.and_then(|(camera, transform)| {
            // MainCamera is a root entity. Use its current local transform before UI
            // layout; Bevy propagates GlobalTransform only after that layout.
            let screen = camera
                .world_to_viewport(&GlobalTransform::from(*transform), position)
                .ok()?;
            let size = camera.logical_viewport_size()?;
            (screen.x > 8.0
                && screen.y > 8.0
                && screen.x < size.x - 30.0
                && screen.y < size.y - 30.0)
                .then_some(screen)
        });
        if let Some(screen) = screen {
            let screen = crate::hud_layout::world_to_ui(screen, ui_scale.as_deref());
            node.display = Display::Flex;
            node.left = Val::Px(screen.x - 12.0 + number.lane * (5.0 + 8.0 * progress));
            node.top = Val::Px(screen.y - 25.0 - 42.0 * progress);
            let opacity = ((1.0 - progress) * 3.0).min(1.0);
            color.0 = number.color.with_alpha(opacity);
            shadow.color = Color::srgba(0.0, 0.0, 0.0, 0.8 * opacity);
        } else {
            node.display = Display::None;
        }
    }
}

fn draw_impacts(mut gizmos: Gizmos, mode: Res<PlayerVisualMode>, feedback: Res<CombatFeedback>) {
    for impact in &feedback.impacts {
        let progress = impact.age / impact.lifetime;
        let color = impact.color.with_alpha(1.0 - progress);
        let radius = (0.18 + progress * 0.75) * impact.scale;
        if *mode == PlayerVisualMode::Models3d {
            let center = impact.position + Vec3::Y * 0.75;
            for direction in [
                Vec3::X,
                Vec3::Y,
                Vec3::Z,
                Vec3::new(0.7, 0.7, 0.0),
                Vec3::new(-0.7, 0.7, 0.0),
            ] {
                gizmos.line(
                    center + direction * radius * 0.4,
                    center + direction * radius,
                    color,
                );
                gizmos.line(
                    center - direction * radius * 0.4,
                    center - direction * radius,
                    color,
                );
            }
        } else {
            let center = simulation_xz_to_render_xy(impact.position).extend(layer::VFX);
            gizmos.circle(Isometry3d::from_translation(center), radius, color);
            for direction in [Vec3::X, Vec3::Y] {
                gizmos.line(
                    center - direction * radius * 1.4,
                    center + direction * radius * 1.4,
                    color,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vital_break_requires_a_visible_living_hero_damage_receipt() {
        let mut event = hit(1, 40.0);
        event.near_lethal = true;
        assert!(!vital_break(&event, true)); // Minions never qualify.
        event.target.kind = CombatEntityKind::Player;
        assert!(vital_break(&event, true));
        assert!(!vital_break(&event, false));
        event.killed = true;
        assert!(!vital_break(&event, true));
        event.killed = false;
        event.amount = 0.0;
        assert!(!vital_break(&event, true));
    }
    fn hit(id: u64, amount: f32) -> CombatEvent {
        CombatEvent {
            id,
            amount,
            target: shared::combat::CombatEntity {
                kind: CombatEntityKind::Minion,
                id: 7,
            },
            ..default()
        }
    }
    /// A receipt a hero dealt to another hero with the action of `slot`.
    fn dealt(id: u64, amount: f32, source: u64, slot: u8) -> CombatEvent {
        CombatEvent {
            id,
            amount,
            source: shared::combat::CombatEntity {
                kind: CombatEntityKind::Player,
                id: source,
            },
            target: shared::combat::CombatEntity {
                kind: CombatEntityKind::Player,
                id: 9,
            },
            x: 4.0,
            y: 1.05,
            z: 0.0,
            action_slot: Some(slot),
            ..default()
        }
    }
    fn seen_hero(class: shared::HeroClass) -> Source<'static> {
        Source {
            position: Vec3::new(0.0, 0.5, 0.0),
            class,
            avatar: None,
            sprite: None,
            loadout: None,
        }
    }
    fn slot_of(class: shared::HeroClass, skill: shared::loadout::SkillId) -> u8 {
        shared::loadout::preset_for_class(class)
            .unwrap()
            .skills()
            .iter()
            .position(|id| *id == skill)
            .unwrap() as u8
    }
    const GROUND: Vec3 = Vec3::new(4.0, 0.0, 0.0);
    const FLOOR: f32 = 0.0;

    #[test]
    fn impacts_need_a_receipt() {
        use crate::game_vfx::ParticleSource;
        use crate::skill_presentation::cast::{CastObserver, Sighting};
        use shared::loadout::SkillId;
        let registry = SkillPresentation::target();
        let class = shared::HeroClass::Stormfist;
        let kick = slot_of(class, SkillId::ThunderKick);
        let source = seen_hero(class);

        // The accepted cast of a damaging skill draws its accent and waits: without a
        // receipt nothing it draws is an impact, and no link leaves the caster.
        let mut observer = CastObserver::default();
        let mut hero = Sighting {
            actor: World::new().spawn_empty().id(),
            actor_id: 7,
            local: false,
            visible: true,
            alive: true,
            position: source.position,
            forward: Vec3::X,
            class,
            loadout: None,
            action: default(),
            facing: default(),
            utility: default(),
        };
        observer.observe(Some((1, 1)), true, [hero]);
        hero.action.sequence = 1;
        hero.action.slot = kick;
        let casts = observer.observe(Some((1, 1)), true, [hero]).casts;
        let accent = accents::cast_burst(&registry, &casts[0], &[]);
        assert!(!accent.is_empty());
        assert!(
            accent
                .iter()
                .all(|spec| spec.source == ParticleSource::Accent)
        );
        let mut links = accents::LinkBook::default();
        links.turn(Some((1, 1)), 1);
        links.open(&registry, &casts[0]);

        let mut cursor = HitCursor::default();
        cursor.accept((1, 1), &[]);
        assert!(cursor.accept((1, 1), &[]).1.is_empty());
        // The receipt arrives: one burst from the recipe of the row, carrying its id.
        let hit = dealt(5, 40.0, 7, kick);
        let accepted = cursor.accept((1, 1), std::slice::from_ref(&hit)).1;
        assert_eq!(accepted.len(), 1);
        let burst =
            themed_impact(Some(&registry), Some(&source), &accepted[0], FLOOR, &[], 0).unwrap();
        assert!(!burst.is_empty() && burst.len() <= 12);
        assert!(
            burst
                .iter()
                .all(|spec| spec.source == ParticleSource::Impact && spec.event_id == 5)
        );
        // Only now does the link have somewhere to go.
        let confirmed = ConfirmedHit {
            receipt: hit.id,
            source: 7,
            slot: kick,
            position: GROUND,
        };
        let (link, _) = links.link(&confirmed).unwrap();
        assert!(link.iter().all(|spec| spec.source == ParticleSource::Link));
        // A repeated delivery of the same snapshot is not a second hit.
        assert!(
            cursor
                .accept((1, 1), std::slice::from_ref(&hit))
                .1
                .is_empty()
        );

        // The recipe needs a source the client sees, a registry and a row that names one;
        // every other receipt keeps the built-in burst.
        assert!(themed_impact(Some(&registry), None, &hit, FLOOR, &[], 0).is_none());
        assert!(themed_impact(None, Some(&source), &hit, FLOOR, &[], 0).is_none());
        let unmigrated = SkillPresentation::unmigrated();
        assert!(themed_impact(Some(&unmigrated), Some(&source), &hit, FLOOR, &[], 0).is_none());
        let mut unslotted = hit.clone();
        unslotted.action_slot = None;
        assert!(themed_impact(Some(&registry), Some(&source), &unslotted, FLOOR, &[], 0).is_none());
        // A basic attack is drawn from the row of its class.
        let basic = dealt(6, 12.0, 7, shared::BASIC_ATTACK_ACTION_SLOT);
        let burst = themed_impact(Some(&registry), Some(&source), &basic, FLOOR, &[], 0).unwrap();
        assert!(
            burst
                .iter()
                .all(|spec| spec.source == ParticleSource::Impact && spec.event_id == 6)
        );
        assert!(themed_impact(Some(&unmigrated), Some(&source), &basic, FLOOR, &[], 0).is_none());
    }

    /// A receipt on the neutral monster 50 from the hero 7.
    fn on_camp(id: u64, slot: u8, killed: bool) -> CombatEvent {
        let mut event = dealt(id, 30.0, 7, slot);
        event.target = shared::combat::CombatEntity {
            kind: CombatEntityKind::Neutral,
            id: 50,
        };
        event.killed = killed;
        event
    }

    /// Forest Tracker is seen on a neutral monster and nowhere else.
    #[test]
    fn camp_hit_needs_a_neutral_target_and_a_seen_hero_with_the_camp_bonus() {
        use crate::game_vfx::ParticleSource;
        use shared::HeroClass;
        let registry = SkillPresentation::target();
        let warden = seen_hero(HeroClass::Warden);
        let drops = |source: Option<&Source>, event: &CombatEvent, boss: Option<bool>| {
            camp_hit(Some(&registry), source, event, boss, GROUND)
        };
        // A hit on an ordinary camp: two drops in the spark colour of the class, above the
        // monster, carrying the receipt.
        let hit = drops(Some(&warden), &on_camp(5, 0, false), Some(false));
        assert_eq!(hit.len(), 2);
        let spark = accents::Palette::of_class(registry.theme(HeroClass::Warden).unwrap())
            .slot(PaletteSlot::Accent);
        for drop in &hit {
            assert_eq!((drop.source, drop.event_id), (ParticleSource::Cue, 5));
            assert_eq!(drop.color, spark);
            assert!(drop.origin.y > GROUND.y + 1.0 && drop.velocity.y > 0.0);
            assert!(drop.origin.xz().distance(GROUND.xz()) < 0.5);
        }
        // The kill of an ordinary camp is the one flourish. A boss gives the smaller bonus
        // and no flourish, and a camp the client never saw is not known to be ordinary.
        let kill = on_camp(6, 0, true);
        assert_eq!(drops(Some(&warden), &kill, Some(false)).len(), 4);
        assert_eq!(drops(Some(&warden), &kill, Some(true)).len(), 2);
        assert_eq!(drops(Some(&warden), &kill, None).len(), 2);
        assert_eq!(
            drops(Some(&warden), &on_camp(7, 0, false), Some(true)).len(),
            2
        );
        // Every action of the hero counts, the basic attack included.
        for slot in [0, 2, 3, shared::BASIC_ATTACK_ACTION_SLOT] {
            assert_eq!(
                drops(Some(&warden), &on_camp(8, slot, false), Some(false)).len(),
                2
            );
        }

        // Never on a hero, a minion or a structure, whatever dealt the hit.
        for kind in [
            CombatEntityKind::Player,
            CombatEntityKind::Minion,
            CombatEntityKind::Structure,
            CombatEntityKind::Unknown,
        ] {
            let mut other = on_camp(9, 0, true);
            other.target.kind = kind;
            assert!(
                drops(Some(&warden), &other, Some(false)).is_empty(),
                "{kind:?}"
            );
        }
        // A source the client does not see draws nothing, and neither does a missing registry.
        assert!(drops(None, &kill, Some(false)).is_empty());
        assert!(camp_hit(None, Some(&warden), &kill, Some(false), GROUND).is_empty());
        // Only a class the server gives the bonus to, and only without a loadout: the
        // server skips the multiplier for a hero that plays a recipe.
        for class in HeroClass::ALL {
            let hero = seen_hero(class);
            let bonus = shared::jungle::neutral_damage_multiplier(class, false) > 1.0;
            assert_eq!(bonus, class == HeroClass::Warden);
            assert_eq!(
                !drops(Some(&hero), &kill, Some(false)).is_empty(),
                bonus,
                "{}",
                class.id()
            );
        }
        let recipe = LoadoutState::default();
        let mixed = Source {
            loadout: Some(&recipe),
            ..warden
        };
        assert!(drops(Some(&mixed), &kill, Some(false)).is_empty());
    }

    /// The drops are part of the burst of their receipt: the recipe of the row is laid out
    /// with as many particles fewer, so the two together stay inside the impact budget.
    #[test]
    fn camp_hit_shares_the_budget_of_its_receipt() {
        use crate::game_vfx::ParticleSource;
        use crate::skill_presentation::impacts::IMPACT_MAX;
        use shared::HeroClass;
        let registry = SkillPresentation::target();
        let warden = seen_hero(HeroClass::Warden);
        let mut trimmed = 0;
        for slot in [0, 2, 3, shared::BASIC_ATTACK_ACTION_SLOT] {
            for (killed, count) in [(false, 2), (true, 4)] {
                let event = on_camp(11, slot, killed);
                let drops = camp_hit(Some(&registry), Some(&warden), &event, Some(false), GROUND);
                assert_eq!(drops.len(), count);
                let burst = |reserved: usize| {
                    themed_impact(Some(&registry), Some(&warden), &event, FLOOR, &[], reserved)
                        .unwrap()
                };
                let (alone, shared) = (burst(0), burst(count));
                assert!(shared.len() + drops.len() <= IMPACT_MAX, "slot {slot}");
                assert_eq!(shared.len(), alone.len().min(IMPACT_MAX - count));
                trimmed += usize::from(shared.len() < alone.len());
                // The recipe is laid out anew, not cut off: it keeps its first mark and
                // closes with the flash of the hit, and all of it is still the impact.
                for (kept, whole) in [
                    (&shared[0], &alone[0]),
                    (shared.last().unwrap(), alone.last().unwrap()),
                ] {
                    assert_eq!(
                        (kept.shape, kept.origin, kept.color, kept.dense),
                        (whole.shape, whole.origin, whole.color, whole.dense)
                    );
                    assert!((kept.size - whole.size).abs() < 1e-3);
                }
                assert!(shared.last().unwrap().dense);
                assert!(
                    shared
                        .iter()
                        .all(|spec| spec.source == ParticleSource::Impact)
                );
            }
        }
        // The three abilities of the Warden fill the budget on their own.
        assert!(trimmed >= 6, "{trimmed}");
    }

    /// The capture evidence of a receipt names the recipe that drew its burst, and no recipe
    /// for a burst the engine drew itself.
    #[cfg(feature = "qa")]
    #[test]
    fn the_evidence_of_a_receipt_names_what_drew_it() {
        use shared::loadout::SkillId;
        let registry = SkillPresentation::target();
        let class = shared::HeroClass::Stormfist;
        let source = seen_hero(class);
        for (skill, kind) in [
            (SkillId::ThunderKick, "thud_ring"),
            (SkillId::EchoStrike, "spark_fork"),
        ] {
            let hit = dealt(5, 40.0, 7, slot_of(class, skill));
            let burst = themed_impact(Some(&registry), Some(&source), &hit, FLOOR, &[], 0);
            let look = ReceiptLook::of(Some(&registry), Some(&source), &hit, burst.as_deref());
            assert_eq!(
                look,
                ReceiptLook {
                    receipt: 5,
                    slot: hit.action_slot,
                    impact: Some(kind),
                    particles: burst.unwrap().len(),
                }
            );
        }
        // A row without a recipe, and a source the client does not see, keep the built-in
        // burst: the evidence names no recipe and counts no particle of one.
        let hit = dealt(6, 40.0, 7, slot_of(class, SkillId::ThunderKick));
        let unmigrated = SkillPresentation::unmigrated();
        for (skills, source) in [(&unmigrated, Some(&source)), (&registry, None)] {
            let burst = themed_impact(Some(skills), source, &hit, FLOOR, &[], 0);
            assert!(burst.is_none());
            let look = ReceiptLook::of(Some(skills), source, &hit, burst.as_deref());
            assert_eq!((look.impact, look.particles), (None, 0));
        }
        // The log keeps the newest receipts only.
        let mut looks = ReceiptLooks::default();
        for id in 0..40 {
            let hit = dealt(id, 1.0, 7, 0);
            looks.record(ReceiptLook::of(None, None, &hit, None));
        }
        assert_eq!(looks.0.len(), ReceiptLooks::KEPT);
        assert_eq!((looks.0[0].receipt, looks.0[31].receipt), (8, 39));
    }

    /// The hit of a repeater's basic attack is drawn from the row of the round that
    /// struck. The wire style of the receipt names it; the mode the hero is in when the
    /// shot lands does not.
    #[test]
    fn rockets_variant_impact_follows_the_wire_style_of_the_receipt() {
        use crate::skill_presentation::impacts::{ImpactContext, impact_particles};
        use shared::BASIC_ATTACK_ACTION_SLOT;
        use shared::combat::ProjectileStyle;
        use shared::loadout::{CoreId, SkillId, WeaponMode};
        let registry = SkillPresentation::target();
        let class = shared::HeroClass::Wildspark;
        let row = registry.basic(class).unwrap();
        let nested = row.rockets.as_deref().unwrap();
        let palette = accents::Palette::of_class(registry.theme(class).unwrap());
        let armed = |core: CoreId, weapon_mode| LoadoutState {
            recipe: Some(core.preset()),
            weapon_mode,
            ..default()
        };
        let styled = |slot, style| CombatEvent {
            style,
            ..dealt(5, 40.0, 7, slot)
        };
        let drawn_from = |round: &crate::skill_presentation::BasicProfile| {
            impact_particles(
                round.impact.as_ref().unwrap(),
                &palette,
                &ImpactContext {
                    position: Vec3::new(4.0, 1.05, 0.0),
                    ground: FLOOR,
                    direction: Vec2::X * 4.0,
                    heading: None,
                    area_damage: false,
                    receipt: 5,
                    reserved: 0,
                },
            )
        };
        let (bullet, rocket) = (drawn_from(row), drawn_from(nested));
        assert_ne!(bullet, rocket);
        for mode in [WeaponMode::Repeater, WeaponMode::Rockets] {
            let state = armed(CoreId::Wildspark, mode);
            let source = Source {
                loadout: Some(&state),
                ..seen_hero(class)
            };
            let basic = |style| {
                let hit = styled(BASIC_ATTACK_ACTION_SLOT, style);
                themed_impact(Some(&registry), Some(&source), &hit, FLOOR, &[], 0).unwrap()
            };
            assert_eq!(basic(ProjectileStyle::Rocket), rocket, "{mode:?}");
            assert_eq!(basic(ProjectileStyle::Bullet), bullet, "{mode:?}");
            // The rocket's splash is not drawn as an area: every victim has its own
            // receipt, and its burst stays at the unit as any other basic hit does.
            assert!(
                rocket
                    .iter()
                    .all(|spec| spec.reach(Vec3::new(4.0, 1.8, 0.0)) <= 1.5 + 1e-3)
            );
            #[cfg(feature = "qa")]
            for (style, kind) in [
                (
                    ProjectileStyle::Rocket,
                    nested.impact.as_ref().unwrap().kind,
                ),
                (ProjectileStyle::Bullet, row.impact.as_ref().unwrap().kind),
            ] {
                let hit = styled(BASIC_ATTACK_ACTION_SLOT, style);
                let look = ReceiptLook::of(Some(&registry), Some(&source), &hit, Some(&[]));
                assert_eq!(look.impact, Some(kind.id()));
            }
            // A skill of the kit is its own row in either mode and whatever style it sends.
            let slot = slot_of(class, SkillId::WildZap);
            assert_eq!(
                themed_impact(
                    Some(&registry),
                    Some(&source),
                    &styled(slot, ProjectileStyle::Rocket),
                    FLOOR,
                    &[],
                    0
                ),
                themed_impact(
                    Some(&registry),
                    Some(&source),
                    &styled(slot, ProjectileStyle::Arcane),
                    FLOOR,
                    &[],
                    0
                )
            );
        }
        // A class without the entry has one hit, even for a receipt that says `Rocket`
        // while its state says rocket mode.
        let other = shared::HeroClass::Riftshot;
        let state = armed(CoreId::Riftshot, WeaponMode::Rockets);
        let source = Source {
            loadout: Some(&state),
            ..seen_hero(other)
        };
        let hit = |style| {
            let hit = styled(BASIC_ATTACK_ACTION_SLOT, style);
            themed_impact(Some(&registry), Some(&source), &hit, FLOOR, &[], 0).unwrap()
        };
        assert_eq!(hit(ProjectileStyle::Rocket), hit(ProjectileStyle::Standard));
        // A source the client does not see keeps the built-in burst of the wire style.
        let unseen = styled(BASIC_ATTACK_ACTION_SLOT, ProjectileStyle::Rocket);
        assert!(themed_impact(Some(&registry), None, &unseen, FLOOR, &[], 0).is_none());
    }

    #[test]
    fn a_trap_that_dealt_no_damage_draws_only_its_cue() {
        use crate::game_vfx::ParticleSource;
        use shared::loadout::SkillId;
        let registry = SkillPresentation::target();
        let class = shared::HeroClass::Wildspark;
        let traps = slot_of(class, SkillId::WildTraps);
        let source = seen_hero(class);
        let mut cursor = HitCursor::default();
        cursor.accept((1, 1), &[hit(3, 10.0)]);
        let seen = cursor.high_water;
        assert_eq!(seen, 3);

        let snap = |id: u64, amount: f32| CombatEvent {
            trap_triggered: true,
            ..dealt(id, amount, 7, traps)
        };
        let mut misplaced = snap(7, 0.0);
        misplaced.z = f32::NAN;
        let events = [
            snap(2, 0.0),
            snap(4, 0.0),
            snap(4, 0.0),
            snap(5, 12.0),
            dealt(6, 0.0, 7, traps),
            misplaced,
        ];
        // A shield took the damage of receipt 4: the cursor passes it over, it is no hit.
        let accepted = cursor.accept((1, 1), &events).1;
        assert_eq!(
            accepted.iter().map(|event| event.id).collect::<Vec<_>>(),
            [5]
        );
        let snaps = trap_snaps(seen, &events);
        assert_eq!(snaps.iter().map(|event| event.id).collect::<Vec<_>>(), [4]);
        let key = source.key(snaps[0]).unwrap();
        let cue = accents::trap_snap(&registry, key, GROUND, snaps[0].id);
        assert!(!cue.is_empty() && cue.len() <= accents::CUE_MAX);
        assert!(
            cue.iter()
                .all(|spec| spec.source == ParticleSource::Cue && spec.event_id == 4)
        );
        // It is not an impact, and a row without a `cast` block draws none.
        assert!(themed_impact(Some(&registry), Some(&source), snaps[0], FLOOR, &[], 0).is_some());
        assert!(accents::trap_snap(&SkillPresentation::unmigrated(), key, GROUND, 4).is_empty());
        assert!(accents::trap_snap(&registry, CastKey::Basic(class), GROUND, 4).is_empty());
        // The trap that did bite is an ordinary hit with the recipe of its row.
        let bite = themed_impact(Some(&registry), Some(&source), &accepted[0], FLOOR, &[], 0);
        assert!(
            bite.unwrap()
                .iter()
                .all(|spec| spec.source == ParticleSource::Impact)
        );
        // Nothing is owed after the snapshot was read once.
        assert!(trap_snaps(cursor.high_water, &events).is_empty());
    }

    #[test]
    fn a_hit_points_away_from_a_source_the_client_sees() {
        let at = Vec3::new(3.0, 1.05, 4.0);
        // Ground coordinates of the simulation in both render modes; heights do not turn it.
        assert_eq!(hit_direction(Some(Vec3::new(3.0, 9.0, 0.0)), at), Vec2::Y);
        assert_eq!(hit_direction(Some(Vec3::new(0.0, 0.5, 4.0)), at), Vec2::X);
        assert_eq!(
            hit_direction(Some(Vec3::new(3.0, 0.5, 6.0)), at),
            Vec2::NEG_Y
        );
        // A hidden source gives the hit no direction, and neither does a hit on itself.
        assert_eq!(hit_direction(None, at), Vec2::X);
        assert_eq!(hit_direction(Some(at), at), Vec2::X);
    }

    #[test]
    fn retained_history_is_not_replayed_and_duplicates_are_suppressed() {
        let mut cursor = HitCursor::default();
        assert!(cursor.accept((1, 1), &[hit(1, 10.0)]).1.is_empty());
        assert_eq!(
            cursor
                .accept(
                    (1, 1),
                    &[hit(1, 10.0), hit(3, 8.0), hit(2, 6.0), hit(3, 8.0)]
                )
                .1
                .len(),
            2
        );
        assert!(
            cursor
                .accept((1, 1), &[hit(2, 6.0), hit(3, 8.0)])
                .1
                .is_empty()
        );
        assert!(cursor.accept((1, 2), &[hit(1, 4.0)]).1.is_empty());
        assert_eq!(cursor.accept((1, 2), &[hit(2, 5.0)]).1.len(), 1);
        assert!(cursor.accept((2, 1), &[hit(1, 6.0)]).1.is_empty());
    }
    #[test]
    fn invalid_feedback_cannot_create_labels_and_feed_is_bounded() {
        let mut cursor = HitCursor::default();
        cursor.accept((1, 1), &[]);
        assert!(
            cursor
                .accept((1, 1), &[hit(1, f32::NAN), hit(2, -1.0), hit(3, 0.0)])
                .1
                .is_empty()
        );
        let mut bad = hit(4, 12.0);
        bad.x = f32::INFINITY;
        assert!(cursor.accept((1, 1), &[bad]).1.is_empty());
        let events = (5..205).map(|id| hit(id, 7.0)).collect::<Vec<_>>();
        assert_eq!(cursor.accept((1, 1), &events).1.len(), MAX_HITS);
        assert!(cursor.accept((1, 1), &events).1.is_empty());
    }
}
