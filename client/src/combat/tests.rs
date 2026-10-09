use super::cast::{PendingCastRequest, queue_cast_request, within_cast_range};
use super::hotbar::{DesktopSkillIcon, SKILL_SLOT_SIZE, SkillRankLabel, update_skill_tooltip};
use super::mobile::{mobile_assisted_target, mobile_target_score};
use super::selection::{
    BASE_TOWER_PICK_RADIUS_PX, MINION_PICK_RADIUS_PX, NEUTRAL_PICK_RADIUS_PX,
    PLAYER_PICK_RADIUS_PX, TOWER_PICK_RADIUS_PX, find_nearest_enemy_target,
    find_target_near_screen, screen_pick_distance,
};
use super::*;
use crate::camera::MainCamera;
use crate::input_context::GameplayInputContext;
use crate::net::{
    GameState, GameStateSnapshot, NetworkCommand, NetworkHeroClass, NetworkMinionId,
    NetworkPlayerId, NetworkStructure, NetworkStructureId, PlayerProgression, RemotePlayer,
    StructureKind, TargetId, TargetKind,
};
use crate::player::{MovementTarget, Player};
use crate::sprite::PlayerVisualMode;
use crate::team::{Team, TeamSelection};
use bevy::{input::mouse::MouseButton, window::PrimaryWindow};
use shared::{HeroClass, SkillSlot, ability_for_class_slot};

fn standard_cast_app(class: HeroClass, slot: usize, recast: bool) -> App {
    let mut app = App::new();
    app.init_resource::<TeamSelection>()
        .init_resource::<PendingCast>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<ActionFeedback>()
        .init_resource::<GameplayInputContext>()
        .add_message::<NetworkCommand>()
        .add_systems(Update, resolve_pending_cast_system);
    let mut state = shared::loadout::LoadoutState {
        recipe: shared::loadout::preset_for_class(class).map(|p| p.recipe()),
        ..default()
    };
    state.slots[slot].can_recast = recast;
    state.slots[slot].recast_remaining_secs = if recast { 3.0 } else { 0.0 };
    let mut stats = CombatStats::default();
    if recast {
        stats.mana = 0.0;
    }
    app.world_mut().spawn((
        Player,
        Transform::default(),
        stats,
        PlayerProgression {
            level: 10,
            ..default()
        },
        NetworkPlayerId(1),
        Team::Green,
        NetworkHeroClass(class),
        crate::net::PlayerLoadout(Some(state)),
    ));
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot,
        target_entity: None,
        target: None,
        approach_announced: false,
    });
    app.world_mut().resource_mut::<PendingCast>().aim = Some(Vec2::new(30.0, 0.0));
    if recast {
        app.world_mut()
            .resource_mut::<LocalCastCooldown>()
            .remaining_secs[slot] = 8.0;
    }
    app
}

#[test]
fn standard_skills_send_world_aim_without_a_selected_unit() {
    for class in [HeroClass::Dawnweaver, HeroClass::Wildspark] {
        for slot in 0..4 {
            let mut app = standard_cast_app(class, slot, false);
            app.update();
            let sent = app
                .world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .collect::<Vec<_>>();
            assert!(
                matches!(sent.as_slice(), [NetworkCommand::CastSkill { slot: sent_slot, aim }]
                if *sent_slot as usize == slot && aim.is_finite()),
                "{class:?} {slot}: {sent:?}"
            );
            assert!(!app.world().resource::<PendingCast>().is_pending());
        }
    }
}

#[test]
fn dawn_field_recast_bypasses_mana_recovery_and_running_base_cooldown() {
    let mut app = standard_cast_app(HeroClass::Dawnweaver, 2, true);
    app.world_mut()
        .resource_mut::<LocalCastCooldown>()
        .recovery_secs = 0.5;
    app.update();
    let sent = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect::<Vec<_>>();
    assert!(matches!(
        sent.as_slice(),
        [NetworkCommand::CastSkill { slot: 2, .. }]
    ));
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs[2],
        8.0
    );
}

/// A replicated Mountain Echo colossus of hero `owner`.
fn colossus(owner: u64, at: Vec2) -> shared::loadout::SkillEffectState {
    shared::loadout::SkillEffectState {
        id: 40,
        owner_id: owner,
        owner_team: shared::map::Team::Green,
        skill: shared::loadout::SkillId::MountainEcho,
        kind: shared::loadout::EffectVisualKind::Bolt,
        position: at.to_array(),
        end: [0.0; 2],
        radius: 2.0,
        remaining_secs: 4.0,
        armed: true,
        consumed_segments: 0,
    }
}

#[test]
fn colossus_recast_requires_own_effect_within_gate() {
    use super::cooldown::{recast_sendable, recast_usable};
    use shared::loadout::{SkillEffect, SkillId, SkillSlotState, skill};

    let echo = skill(SkillId::MountainEcho);
    let open = SkillSlotState {
        can_recast: true,
        recast_remaining_secs: 3.0,
        ..default()
    };
    let closed = SkillSlotState::default();
    let hero = Vec2::new(10.0, 5.0);
    let gate = crate::skill_presentation::geometry::RECAST_GATE_MOUNTAIN_ECHO;
    let away = |distance: f32| hero + Vec2::new(0.6, -0.8) * distance;
    let usable = |slot: &SkillSlotState, id: u64, effects: &[_]| {
        recast_usable(echo, slot, hero, id, effects)
    };

    // The open window alone is not a recast the server accepts: it needs the hero's own
    // colossus within the gate (`common/src/skills/advanced.rs`, 4.0 units).
    assert!(!usable(&open, 1, &[]));
    assert!(usable(&open, 1, &[colossus(1, away(gate - 0.01))]));
    assert!(!usable(&open, 1, &[colossus(1, away(gate + 0.01))]));
    assert!(usable(
        &open,
        1,
        &[colossus(1, away(20.0)), colossus(1, away(3.0))]
    ));
    // A closed window, another hero's colossus, one whose owner is hidden and another
    // effect of the hero open nothing.
    assert!(!usable(&closed, 1, &[colossus(1, away(1.0))]));
    assert!(!usable(&open, 1, &[colossus(2, away(1.0))]));
    assert!(!usable(&open, 0, &[colossus(0, away(1.0))]));
    let mut pillar = colossus(1, away(1.0));
    pillar.skill = SkillId::FaultLine;
    assert!(!usable(&open, 1, &[pillar]));

    // The press that is sent may lead the snapshot by what the colossus travels in a
    // tenth of a second, and by nothing else.
    let SkillEffect::Technique { speed, .. } = echo.effect else {
        panic!("Mountain Echo is a technique");
    };
    let lead = speed * 0.1;
    assert!((lead - 1.2).abs() < 1e-6);
    let sendable = |slot: &SkillSlotState, id: u64, effects: &[_]| {
        recast_sendable(echo, slot, hero, id, effects)
    };
    assert!(sendable(&open, 1, &[colossus(1, away(gate - 0.01))]));
    assert!(sendable(&open, 1, &[colossus(1, away(gate + lead - 0.01))]));
    assert!(!sendable(
        &open,
        1,
        &[colossus(1, away(gate + lead + 0.01))]
    ));
    assert!(!sendable(&open, 1, &[]));
    assert!(!sendable(&closed, 1, &[colossus(1, away(1.0))]));
    assert!(!sendable(&open, 1, &[colossus(2, away(1.0))]));
    assert!(!sendable(&open, 0, &[colossus(0, away(1.0))]));

    // Every other recast is what its flag says, wherever the hero stands.
    for id in SkillId::ALL {
        if id == SkillId::MountainEcho {
            continue;
        }
        let def = skill(id);
        assert!(recast_usable(def, &open, hero, 1, &[]), "{}", id.id());
        assert!(recast_sendable(def, &open, hero, 1, &[]), "{}", id.id());
        assert!(!recast_usable(def, &closed, hero, 1, &[]), "{}", id.id());
        assert!(!recast_sendable(def, &closed, hero, 1, &[]), "{}", id.id());
    }
}

#[test]
fn colossus_recast_is_offered_and_sent_only_inside_the_gate() {
    use super::standard::{StandardStatus, update_status};
    use crate::i18n::{tr, trf};
    use crate::net::{PlayerEquipment, PlayerSkillCooldowns};

    const R: usize = 3;
    let running = PlayerSkillCooldowns {
        remaining_secs: [0.0, 0.0, 0.0, 45.0],
        recovery_secs: 0.0,
    };
    let mut app = standard_cast_app(HeroClass::Cinderforge, R, true);
    let mut inspection = super::inspection::SkillInspection::default();
    inspection.slot = Some(R);
    app.insert_resource(inspection)
        .init_resource::<crate::mobile_controls::MobileControls>()
        .init_resource::<crate::gamepad::GamepadControls>()
        .insert_resource(GameStateSnapshot {
            your_id: 1,
            ..default()
        })
        .add_systems(
            Update,
            (
                sync_authoritative_cooldown_durations.before(resolve_pending_cast_system),
                update_status.after(resolve_pending_cast_system),
            ),
        );
    let label = app
        .world_mut()
        .spawn((Text::default(), Node::default(), StandardStatus))
        .id();
    let hero = app
        .world_mut()
        .query_filtered::<Entity, With<Player>>()
        .single(app.world())
        .unwrap();
    app.world_mut().entity_mut(hero).insert((
        PlayerEquipment::default(),
        running,
        MovementTarget { target: Vec3::X },
    ));
    app.world_mut().get_mut::<CombatStats>(hero).unwrap().mana = 100.0;
    *app.world_mut().resource_mut::<LocalCastCooldown>() = LocalCastCooldown::default();
    let recast_line = trf(
        "combat.standard.recast",
        &[("key", &"R"), ("seconds", &"3.0")],
    );

    // What one press does with the colossus `distance` away from the hero (none: gone).
    let press = |app: &mut App, distance: Option<f32>| {
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .skill_effects = distance
            .map(|distance| colossus(1, Vec2::new(0.0, distance)))
            .into_iter()
            .collect();
        app.world_mut()
            .resource_mut::<ActionFeedback>()
            .text
            .clear();
        app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
            slot: R,
            target_entity: None,
            target: None,
            approach_announced: false,
        });
        app.update();
        let sent = sent_commands(app);
        let cooldowns = app.world().resource::<LocalCastCooldown>();
        (
            sent,
            cooldowns.recast[R],
            cooldowns.remaining_secs[R],
            app.world().resource::<ActionFeedback>().text.clone(),
            app.world()
                .get::<Text>(label)
                .unwrap()
                .0
                .contains(&recast_line),
            app.world().entity(hero).contains::<MovementTarget>(),
        )
    };

    // The window is open for seconds, but the colossus is still far away: the slot keeps
    // its running cooldown, the status line offers nothing, and a press is refused aloud
    // without a command, a predicted cooldown or a lost move order.
    let (sent, offered, remaining, feedback, listed, walking) = press(&mut app, Some(20.0));
    assert!(sent.is_empty(), "{sent:?}");
    assert!(!offered && !listed);
    assert_eq!(remaining, 45.0);
    assert_eq!(feedback, tr("combat.standard.not_ready"));
    assert!(walking);
    assert!(!app.world().resource::<PendingCast>().is_pending());
    let cooldowns = app.world().resource::<LocalCastCooldown>();
    assert_eq!(cooldowns.recovery_secs, 0.0);
    assert!(cooldowns.pending_slot.is_none());

    // Just outside the gate nothing is offered either, but the press is sent: the
    // colossus closes that gap before the server reads the command.
    let (sent, offered, remaining, feedback, listed, _) = press(&mut app, Some(4.6));
    assert!(matches!(
        sent.as_slice(),
        [NetworkCommand::CastSkill { slot: 3, .. }]
    ));
    assert!(!offered && !listed);
    assert_eq!(remaining, 45.0);
    assert!(feedback.is_empty());

    // Inside the gate the slot is the recast and the press is sent as one: no cooldown is
    // predicted for it.
    app.world_mut()
        .entity_mut(hero)
        .insert(MovementTarget { target: Vec3::X });
    let (sent, offered, remaining, feedback, listed, walking) = press(&mut app, Some(3.9));
    assert!(matches!(
        sent.as_slice(),
        [NetworkCommand::CastSkill { slot: 3, .. }]
    ));
    assert!(offered && listed);
    assert_eq!(remaining, 0.0);
    assert!(feedback.is_empty());
    assert!(!walking, "a cast that is sent stops the hero as before");
    assert!(
        app.world()
            .resource::<LocalCastCooldown>()
            .pending_slot
            .is_none()
    );

    // The colossus is gone while the window is still open. Until the next snapshot the
    // mirror has no cooldown to show, and the press must still not go out as a first cast
    // with a predicted cooldown: the server would read it as the recast and drop it.
    let (sent, offered, remaining, feedback, listed, _) = press(&mut app, None);
    assert!(sent.is_empty(), "{sent:?}");
    assert!(!offered && !listed);
    assert_eq!(remaining, 0.0);
    assert_eq!(feedback, tr("combat.standard.not_ready"));
    assert!(
        app.world()
            .resource::<LocalCastCooldown>()
            .pending_slot
            .is_none()
    );

    // The next snapshot brings the real cooldown back and the press is refused again.
    app.world_mut()
        .entity_mut(hero)
        .insert(PlayerSkillCooldowns {
            remaining_secs: [0.0, 0.0, 0.0, 44.0],
            recovery_secs: 0.0,
        });
    let (sent, offered, remaining, feedback, listed, _) = press(&mut app, None);
    assert!(sent.is_empty(), "{sent:?}");
    assert!(!offered && !listed);
    assert_eq!(remaining, 44.0);
    assert_eq!(feedback, tr("combat.standard.not_ready"));
}

#[test]
fn rift_step_onto_a_landing_the_server_refuses_is_not_sent() {
    use crate::i18n::tr;
    use shared::loadout::{EffectVisualKind, SkillEffectState, SkillId};

    const E: usize = 2;
    assert_eq!(
        shared::loadout::preset_for_class(HeroClass::Riftshot)
            .unwrap()
            .skills()[E],
        SkillId::RiftStep
    );
    let tower = Vec2::new(5.0, 0.0);
    let pillar = Vec2::new(0.0, 5.0);
    let map = shared::navigation::world_navigation();
    assert!(
        [Vec2::ZERO, tower, pillar, Vec2::X * 7.0]
            .into_iter()
            .all(|at| map.point_clear(at.to_array()))
    );
    let fault = |armed: bool| SkillEffectState {
        id: 41,
        owner_id: 2,
        owner_team: shared::map::Team::Blue,
        skill: SkillId::FaultLine,
        kind: EffectVisualKind::Trap,
        position: pillar.to_array(),
        end: pillar.to_array(),
        radius: 1.0,
        remaining_secs: 3.0,
        armed,
        consumed_segments: 0,
    };
    // One press of `slot` aimed at `aim`, next to a tower with `tower_hp` and the given
    // effects: the commands, the feedback, the predicted cooldown of the slot, whether a
    // prediction waits for the server and whether the hero still walks.
    let press = |slot: usize, aim: Vec2, tower_hp: f32, effects: Vec<SkillEffectState>| {
        let mut app = standard_cast_app(HeroClass::Riftshot, slot, false);
        app.insert_resource(GameStateSnapshot {
            your_id: 1,
            skill_effects: effects,
            ..default()
        });
        app.world_mut().spawn((
            NetworkStructure,
            NetworkStructureId(9),
            Transform::from_xyz(tower.x, 0.0, tower.y),
            CombatStats {
                hp: tower_hp,
                ..default()
            },
            Team::Blue,
            StructureKind::Tower,
        ));
        let hero = app
            .world_mut()
            .query_filtered::<Entity, With<Player>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(hero)
            .insert(MovementTarget { target: Vec3::Z });
        app.world_mut().resource_mut::<PendingCast>().aim = Some(aim);
        app.update();
        let sent = sent_commands(&mut app);
        let cooldowns = app.world().resource::<LocalCastCooldown>();
        (
            sent,
            app.world().resource::<ActionFeedback>().text.clone(),
            cooldowns.remaining_secs[slot],
            cooldowns.pending_slot.is_some() || cooldowns.recovery_secs > 0.0,
            app.world().entity(hero).contains::<MovementTarget>(),
            app.world().resource::<PendingCast>().is_pending(),
        )
    };
    let refused = |outcome: (Vec<NetworkCommand>, String, f32, bool, bool, bool), case: &str| {
        let (sent, feedback, cooldown, predicted, walking, pending) = outcome;
        assert!(sent.is_empty(), "{case}: {sent:?}");
        assert_eq!(feedback, tr("combat.standard.blocked_landing"), "{case}");
        assert_eq!(cooldown, 0.0, "{case}: no phantom cooldown");
        assert!(!predicted && walking && !pending, "{case}");
    };
    let landed =
        |outcome: (Vec<NetworkCommand>, String, f32, bool, bool, bool), aim: Vec2, case: &str| {
            let (sent, feedback, cooldown, predicted, walking, pending) = outcome;
            assert!(
                matches!(
                    sent.as_slice(),
                    [NetworkCommand::CastSkill { slot: 2, aim: sent_aim }]
                        if sent_aim.distance(aim) < 1e-4
                ),
                "{case}: {sent:?}"
            );
            assert!(feedback.is_empty(), "{case}: {feedback}");
            assert!(
                cooldown > 0.0 && predicted && !walking && !pending,
                "{case}"
            );
        };

    // A standing tower keeps the landing out of its disc of 1.3 units.
    refused(
        press(E, tower - Vec2::X * 1.29, 100.0, vec![]),
        "inside the tower",
    );
    let beside = tower - Vec2::X * 1.31;
    landed(press(E, beside, 100.0, vec![]), beside, "beside the tower");
    // A tower that fell does not.
    let rubble = tower - Vec2::X * 1.29;
    landed(press(E, rubble, 0.0, vec![]), rubble, "a fallen tower");
    // An armed pillar keeps a hero's radius more than its own; before it rises it does not.
    refused(
        press(E, pillar - Vec2::Y * 1.4, 100.0, vec![fault(true)]),
        "at the pillar",
    );
    let clear = pillar - Vec2::Y * 1.6;
    landed(
        press(E, clear, 100.0, vec![fault(true)]),
        clear,
        "clear of the pillar",
    );
    let rising = pillar - Vec2::Y * 1.4;
    landed(
        press(E, rising, 100.0, vec![fault(false)]),
        rising,
        "a pillar that has not risen",
    );
    // The aim is bounded by the cast range first: the landing that is judged is the one
    // that is sent.
    landed(
        press(E, Vec2::X * 30.0, 100.0, vec![]),
        Vec2::X * 7.0,
        "past the tower, bounded to the range",
    );
    // Only the blink has a landing: another skill aimed into the tower is sent.
    let (sent, feedback, ..) = press(0, tower, 100.0, vec![]);
    assert!(matches!(
        sent.as_slice(),
        [NetworkCommand::CastSkill { slot: 0, .. }]
    ));
    assert!(feedback.is_empty());
}

/// A hero of another player that the client shows.
fn remote_hero(app: &mut App, id: u64, team: Team, at: Vec2) -> Entity {
    app.world_mut()
        .spawn((
            RemotePlayer,
            NetworkPlayerId(id),
            Transform::from_xyz(at.x, 0.0, at.y),
            CombatStats::default(),
            team,
            InheritedVisibility::VISIBLE,
        ))
        .id()
}

/// A lane minion that the client shows.
fn lane_minion(app: &mut App, id: u64, team: Team, at: Vec2) -> Entity {
    app.world_mut()
        .spawn((
            crate::net::NetworkMinion,
            NetworkMinionId(id),
            Transform::from_xyz(at.x, 0.0, at.y),
            CombatStats::default(),
            team,
            InheritedVisibility::VISIBLE,
        ))
        .id()
}

/// What one press of a modular skill came to.
struct Press {
    sent: Vec<NetworkCommand>,
    feedback: String,
    cooldown: f32,
    /// A prediction waits for the server.
    predicted: bool,
    /// The hero still follows its move order.
    walking: bool,
    queued: bool,
}

impl Press {
    /// Nothing went out and nothing was predicted: one line, and the hero walks on.
    fn refused_for_want_of_an_ally(&self, case: &str) {
        assert!(self.sent.is_empty(), "{case}: {:?}", self.sent);
        assert_eq!(
            self.feedback,
            crate::i18n::tr("combat.cast.no_ally"),
            "{case}"
        );
        assert_eq!(self.cooldown, 0.0, "{case}: no phantom cooldown");
        assert!(!self.predicted && self.walking && !self.queued, "{case}");
    }

    /// One cast of `slot` went out, aimed at `aim`, and its cooldown is predicted.
    fn sent_to(&self, slot: usize, aim: Vec2, case: &str) {
        assert!(
            matches!(
                self.sent.as_slice(),
                [NetworkCommand::CastSkill { slot: sent, aim: at }]
                    if usize::from(*sent) == slot && at.distance(aim) < 1e-4
            ),
            "{case}: {:?}",
            self.sent
        );
        assert!(self.feedback.is_empty(), "{case}: {}", self.feedback);
        assert!(
            self.cooldown > 0.0 && self.predicted && !self.walking && !self.queued,
            "{case}"
        );
    }
}

/// One press of `slot` by a hero of `class` that walks at the origin, in the world that
/// `arrange` fills. Without an `aim` the press has neither a cursor nor a stick.
fn press_among(
    class: HeroClass,
    slot: usize,
    aim: Option<Vec2>,
    arrange: impl FnOnce(&mut App),
) -> Press {
    let mut app = standard_cast_app(class, slot, false);
    arrange(&mut app);
    let hero = app
        .world_mut()
        .query_filtered::<Entity, With<Player>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(hero)
        .insert(MovementTarget { target: Vec3::Z });
    app.world_mut().resource_mut::<PendingCast>().aim = aim;
    app.update();
    let sent = sent_commands(&mut app);
    let cooldowns = app.world().resource::<LocalCastCooldown>();
    Press {
        sent,
        feedback: app.world().resource::<ActionFeedback>().text.clone(),
        cooldown: cooldowns.remaining_secs[slot],
        predicted: cooldowns.pending_slot.is_some() || cooldowns.recovery_secs > 0.0,
        walking: app.world().entity(hero).contains::<MovementTarget>(),
        queued: app.world().resource::<PendingCast>().is_pending(),
    }
}

/// The slot a class casts `id` from.
fn preset_slot(class: HeroClass, id: shared::loadout::SkillId) -> usize {
    shared::loadout::preset_for_class(class)
        .unwrap()
        .skills()
        .iter()
        .position(|skill| *skill == id)
        .unwrap()
}

#[test]
fn orbital_guard_is_sent_to_the_allied_hero_at_its_aim_or_refused_with_a_line() {
    use shared::loadout::SkillId;

    let class = HeroClass::Orbitwright;
    let e = preset_slot(class, SkillId::OrbitalGuard);
    assert_eq!(e, 2);
    let range = shared::loadout::skill(SkillId::OrbitalGuard)
        .ability
        .cast_range;
    // How far from the aim the server still takes a hero.
    let reach = 2.0 + shared::PLAYER_TARGET_RADIUS;
    let ally = Vec2::new(6.0, 2.0);
    let beside = ally + Vec2::X;

    // Nobody to guard: nothing is sent, nothing is predicted, one line says why. So it is
    // for a press without a cursor, which aims ten units ahead.
    for aim in [Some(Vec2::new(30.0, 0.0)), Some(beside), None] {
        press_among(class, e, aim, |_| {}).refused_for_want_of_an_ally("no ally");
    }
    // An allied hero at the aim: the cast goes out, aimed at that hero.
    press_among(class, e, Some(beside), |app| {
        remote_hero(app, 9, Team::Green, ally);
    })
    .sent_to(e, ally, "an ally at the aim");
    // An enemy at the aim is nobody to guard.
    press_among(class, e, Some(beside), |app| {
        remote_hero(app, 9, Team::Blue, beside);
    })
    .refused_for_want_of_an_ally("an enemy at the aim");
    // The caster is a hero of his own team: aimed at himself, the orb comes home.
    press_among(class, e, Some(Vec2::X * 1.5), |_| {}).sent_to(e, Vec2::ZERO, "self aim");

    // The server takes heroes only and leaves the other kinds out before it looks for the
    // nearest: an allied minion nearer the aim does not shadow the hero, and alone it is
    // nobody to guard.
    press_among(class, e, Some(beside), |app| {
        remote_hero(app, 9, Team::Green, ally);
        lane_minion(app, 4, Team::Green, beside);
    })
    .sent_to(e, ally, "a minion nearer the aim");
    press_among(class, e, Some(beside), |app| {
        lane_minion(app, 4, Team::Green, beside);
    })
    .refused_for_want_of_an_ally("an allied minion alone");
    // A dead hero and one the client hides are not offered to the rule.
    press_among(class, e, Some(beside), |app| {
        let fallen = remote_hero(app, 9, Team::Green, ally);
        app.world_mut().entity_mut(fallen).insert(CombatStats {
            hp: 0.0,
            ..default()
        });
    })
    .refused_for_want_of_an_ally("a dead ally");
    press_among(class, e, Some(beside), |app| {
        let unseen = remote_hero(app, 9, Team::Green, ally);
        app.world_mut()
            .entity_mut(unseen)
            .insert(InheritedVisibility::HIDDEN);
    })
    .refused_for_want_of_an_ally("a hidden ally");

    // The pick reaches two units and the radius of the hero from the aim.
    press_among(class, e, Some(ally + Vec2::X * (reach - 0.03)), |app| {
        remote_hero(app, 9, Team::Green, ally);
    })
    .sent_to(e, ally, "inside the pick reach");
    press_among(class, e, Some(ally + Vec2::X * (reach + 0.03)), |app| {
        remote_hero(app, 9, Team::Green, ally);
    })
    .refused_for_want_of_an_ally("outside the pick reach");
    // A hero is taken up to its radius beyond the cast range, but a point aim beyond the
    // range is dropped: the bounded aim is sent as it is there.
    let edge = Vec2::X * (range + shared::PLAYER_TARGET_RADIUS);
    press_among(class, e, Some(Vec2::X * 30.0), |app| {
        remote_hero(app, 9, Team::Green, edge - Vec2::X * 0.03);
    })
    .sent_to(e, Vec2::X * range, "a hero at the edge of the range");
    press_among(class, e, Some(Vec2::X * 30.0), |app| {
        remote_hero(app, 9, Team::Green, edge + Vec2::X * 0.03);
    })
    .refused_for_want_of_an_ally("a hero beyond the range");
    // Of two heroes the nearer to the aim is taken, and the lower id at the same distance.
    press_among(class, e, Some(beside), |app| {
        remote_hero(app, 5, Team::Green, beside + Vec2::X);
        remote_hero(app, 9, Team::Green, beside - Vec2::X * 0.5);
    })
    .sent_to(e, beside - Vec2::X * 0.5, "the nearer of two");
    press_among(class, e, Some(beside), |app| {
        remote_hero(app, 9, Team::Green, beside + Vec2::X);
        remote_hero(app, 5, Team::Green, beside - Vec2::X);
    })
    .sent_to(e, beside - Vec2::X, "the lower id of two");
}

#[test]
fn sheltering_leap_is_sent_to_the_ally_at_its_aim_or_refused_with_a_line() {
    use shared::loadout::SkillId;

    let class = HeroClass::Frostguard;
    let w = preset_slot(class, SkillId::ShelteringLeap);
    assert_eq!(w, 1);
    let aim = Vec2::new(5.0, -3.0);

    // The leap needs an ally to leap to.
    for aim in [Some(Vec2::new(30.0, 0.0)), Some(aim), None] {
        press_among(class, w, aim, |_| {}).refused_for_want_of_an_ally("no ally");
    }
    press_among(class, w, Some(aim), |app| {
        remote_hero(app, 9, Team::Blue, aim);
        lane_minion(app, 4, Team::Blue, aim);
    })
    .refused_for_want_of_an_ally("enemies at the aim");
    // An allied minion is one, and so is the caster.
    let minion = aim + Vec2::Y * 0.8;
    press_among(class, w, Some(aim), |app| {
        lane_minion(app, 4, Team::Green, minion);
    })
    .sent_to(w, minion, "an allied minion");
    press_among(class, w, Some(Vec2::X * 1.5), |_| {}).sent_to(w, Vec2::ZERO, "self aim");
    // The nearest ally to the aim is taken whatever its kind; a hero before a minion at
    // the same distance.
    let hero = aim - Vec2::Y;
    press_among(class, w, Some(aim), |app| {
        remote_hero(app, 9, Team::Green, hero);
        lane_minion(app, 4, Team::Green, minion);
    })
    .sent_to(w, minion, "the minion is nearer");
    press_among(class, w, Some(aim), |app| {
        remote_hero(app, 9, Team::Green, hero);
        lane_minion(app, 4, Team::Green, aim + Vec2::Y);
    })
    .sent_to(w, hero, "a hero first at the same distance");
}

/// Only a cast the server refuses without an ally is judged by its pick: every other
/// modular skill aimed at open ground is sent as before, the leap that may go without an
/// ally (Anchor Step) and the casts on an enemy among them.
#[test]
fn only_a_cast_that_needs_an_ally_is_refused_for_want_of_one() {
    use crate::skill_presentation::geometry::pick_rule;
    use shared::loadout::SkillId;

    let mut needing = Vec::new();
    for class in HeroClass::ALL {
        let Some(kit) = shared::loadout::preset_for_class(class) else {
            continue;
        };
        for (slot, id) in kit.skills().iter().copied().enumerate() {
            let press = press_among(class, slot, Some(Vec2::new(30.0, 0.0)), |_| {});
            if pick_rule(id).is_some_and(|rule| rule.ally && !rule.pick_optional) {
                press.refused_for_want_of_an_ally(&format!("{id:?}"));
                needing.push(id);
            } else {
                assert!(
                    matches!(
                        press.sent.as_slice(),
                        [NetworkCommand::CastSkill { slot: sent, .. }] if usize::from(*sent) == slot
                    ),
                    "{id:?}: {:?} {}",
                    press.sent,
                    press.feedback
                );
            }
        }
    }
    assert_eq!(needing, [SkillId::OrbitalGuard, SkillId::ShelteringLeap]);
}

/// One cast of `id` by the local hero of the practice authority, among the units
/// `arrange` places: the client world is built from the snapshot of that moment, the press
/// goes through the cast path, and what the client sends (or, when it sends nothing, the
/// aim it refused) is handed to the authority. Returns the aim the client sent, whether the
/// authority took the cast, the session a second later and the address of the dummy.
fn ally_cast_on_the_authority(
    class: HeroClass,
    id: shared::loadout::SkillId,
    aim: Vec2,
    arrange: impl FnOnce(&mut common::offline::PracticeSession, std::net::SocketAddr),
) -> (
    Option<Vec2>,
    bool,
    common::offline::PracticeSession,
    std::net::SocketAddr,
) {
    use common::offline::{EPOCH, LOCAL_ADDR, PracticeSession};
    use shared::practice::PracticeCommand;
    use shared::wire::{CharacterChoice, ClientPacket, ServerPacket};

    let slot = preset_slot(class, id);
    let mut session = PracticeSession::new(std::time::Instant::now());
    session.command(ClientPacket::Join {
        handheld: Default::default(),
        prematch: false,
        team: shared::map::Team::Green,
        character: CharacterChoice::Ipfs,
        hero_class: class,
        avatar: None,
        sprite_character: None,
        session_id: None,
        passport_ticket: None,
    });
    for command in [PracticeCommand::ClearBots, PracticeCommand::SpawnDummy] {
        session.command(ClientPacket::Practice { command });
    }
    session.bots = Default::default();
    session.world.structures.clear();
    session.world.minions.clear();
    session.world.neutrals.clear();
    let dummy = *session
        .world
        .players
        .iter()
        .find(|(_, player)| player.hero.identity.is_bot)
        .expect("the dummy")
        .0;
    {
        let caster = &mut session.world.players.get_mut(&LOCAL_ADDR).unwrap().hero;
        (caster.x, caster.z) = (0.0, 0.0);
    }
    arrange(&mut session, dummy);

    // What the client knows at this moment.
    let ServerPacket::Snapshot {
        your_id,
        players,
        minions,
        ..
    } = session.snapshot()
    else {
        panic!("practice publishes a snapshot");
    };
    let mut app = standard_cast_app(class, slot, false);
    let local = app
        .world_mut()
        .query_filtered::<Entity, With<Player>>()
        .single(app.world())
        .unwrap();
    for player in &players {
        let at = Vec2::new(player.x, player.z);
        let entity = if player.id == your_id {
            app.world_mut().entity_mut(local).insert((
                NetworkPlayerId(player.id),
                Transform::from_xyz(at.x, 0.0, at.y),
            ));
            local
        } else {
            remote_hero(&mut app, player.id, player.team.into(), at)
        };
        let mut stats = app.world_mut().get_mut::<CombatStats>(entity).unwrap();
        (stats.hp, stats.max_hp) = (player.hp, player.max_hp);
    }
    for minion in &minions {
        let at = Vec2::new(minion.x, minion.z);
        lane_minion(&mut app, minion.id, minion.team.into(), at);
    }
    app.world_mut().resource_mut::<PendingCast>().aim = Some(aim);
    app.update();
    let sent = match sent_commands(&mut app).as_slice() {
        [] => None,
        [NetworkCommand::CastSkill { slot: sent, aim }] if usize::from(*sent) == slot => Some(*aim),
        other => panic!("one cast of the slot at most: {other:?}"),
    };

    let definition = shared::loadout::skill(id);
    let refused = super::standard::bounded_aim(
        Vec2::ZERO,
        aim,
        definition.ability.targeting,
        definition.ability.cast_range,
    );
    session.command(ClientPacket::CastSkill {
        slot: slot as u8,
        aim: sent.unwrap_or(refused).to_array(),
        server_epoch: EPOCH,
        match_id: session.match_id,
        request_id: 1,
    });
    // An accepted technique starts the shared recovery.
    let taken = session.world.players[&LOCAL_ADDR]
        .hero
        .skills
        .recovery_until
        .is_some();
    for _ in 0..20 {
        session.advance(0.05);
    }
    (sent, taken, session, dummy)
}

/// Parity with the in-process authority for a cast on an ally: the client sends it exactly
/// when the authority takes it, and the unit the client aims it at is the unit the
/// authority gives it to. At the edge of the pick reach, at the edge of the cast range,
/// with a minion nearer the aim than the hero, on an enemy and on the caster.
#[test]
fn a_cast_on_an_ally_is_sent_exactly_when_the_authority_takes_it() {
    use common::offline::{LOCAL_ADDR, PracticeSession};
    use shared::loadout::SkillId;
    use std::net::SocketAddr;

    const MARGIN: f32 = 0.03;
    let reach = 2.0 + shared::PLAYER_TARGET_RADIUS;
    let green = shared::map::Team::Green;
    // The dummy as a hero of `team` at `at`.
    let stand = |session: &mut PracticeSession, dummy: SocketAddr, team, at: Vec2| {
        let hero = &mut session.world.players.get_mut(&dummy).unwrap().hero;
        hero.identity.team = team;
        (hero.x, hero.z) = (at.x, at.y);
    };
    // One allied lane minion at `at`.
    let march = |session: &mut PracticeSession, at: Vec2| {
        let world = &mut session.world;
        common::world::spawn_minion_wave_for_team_lane(
            &world.map_layout,
            &mut world.minions,
            &mut world.next_minion_id,
            green,
            shared::map::Lane::Mid,
        );
        let id = *world.minions.keys().min().expect("a wave has a minion");
        world.minions.retain(|key, _| *key == id);
        let minion = world.minions.get_mut(&id).unwrap();
        (minion.state.x, minion.state.z) = (at.x, at.y);
    };
    let shielded = |session: &PracticeSession, addr: SocketAddr| {
        !session.world.players[&addr].hero.skills.shields.is_empty()
    };
    let place = |session: &PracticeSession, addr: SocketAddr| {
        let hero = &session.world.players[&addr].hero;
        Vec2::new(hero.x, hero.z)
    };

    let guard = (HeroClass::Orbitwright, SkillId::OrbitalGuard);
    let leap = (HeroClass::Frostguard, SkillId::ShelteringLeap);
    let ally = Vec2::new(6.0, 2.0);
    let away = Vec2::new(-20.0, 0.0);
    for (gap, inside) in [(-MARGIN, true), (MARGIN, false)] {
        for (class, id) in [guard, leap] {
            let range = shared::loadout::skill(id).ability.cast_range;
            // An allied hero at the edge of the pick reach.
            let (sent, taken, session, dummy) = ally_cast_on_the_authority(
                class,
                id,
                ally + Vec2::X * (reach + gap),
                |s, dummy| stand(s, dummy, green, ally),
            );
            assert_eq!(
                (sent, taken),
                (inside.then_some(ally), inside),
                "{id:?} reach {gap}"
            );
            assert_eq!(shielded(&session, dummy), inside, "{id:?} reach {gap}");
            // An allied hero at the edge of the cast range, the aim far beyond it.
            let edge = Vec2::X * (range + shared::PLAYER_TARGET_RADIUS + gap);
            let (sent, taken, ..) =
                ally_cast_on_the_authority(class, id, Vec2::X * 40.0, |s, dummy| {
                    stand(s, dummy, green, edge)
                });
            assert_eq!(
                (sent, taken),
                (inside.then_some(Vec2::X * range), inside),
                "{id:?} range {gap}"
            );
            // The caster himself, the dummy out of the way.
            let (sent, taken, session, _) =
                ally_cast_on_the_authority(class, id, Vec2::Y * (reach + gap), |s, dummy| {
                    stand(s, dummy, green, away)
                });
            assert_eq!(
                (sent, taken),
                (inside.then_some(Vec2::ZERO), inside),
                "{id:?} self {gap}"
            );
            assert_eq!(shielded(&session, LOCAL_ADDR), inside, "{id:?} self {gap}");
        }
    }
    for (class, id) in [guard, leap] {
        // An enemy hero at the aim is no ally.
        let (sent, taken, ..) = ally_cast_on_the_authority(class, id, ally, |s, dummy| {
            stand(s, dummy, shared::map::Team::Blue, ally)
        });
        assert_eq!((sent, taken), (None, false), "{id:?} on an enemy");
    }

    // An allied minion nearer the aim than the allied hero. The orb takes heroes only and
    // goes to the hero; the leap takes the nearest ally and lands on the minion.
    let minion = ally + Vec2::X;
    let (sent, taken, session, dummy) =
        ally_cast_on_the_authority(guard.0, guard.1, minion, |s, dummy| {
            stand(s, dummy, green, ally);
            march(s, minion);
        });
    assert_eq!(
        (sent, taken),
        (Some(ally), true),
        "the orb passes the minion by"
    );
    assert!(shielded(&session, dummy));
    let orb = session.world.players[&LOCAL_ADDR]
        .hero
        .skills
        .advanced
        .orb
        .as_ref()
        .expect("the orb is out");
    assert!(
        Vec2::from_array(orb.pos).distance(ally) < 1e-3,
        "{:?}",
        orb.pos
    );
    // An allied minion alone is nobody to guard.
    let (sent, taken, ..) = ally_cast_on_the_authority(guard.0, guard.1, minion, |s, dummy| {
        stand(s, dummy, green, away);
        march(s, minion);
    });
    assert_eq!((sent, taken), (None, false), "a minion alone");

    let (sent, taken, session, _) =
        ally_cast_on_the_authority(leap.0, leap.1, minion, |s, dummy| {
            stand(s, dummy, green, ally);
            march(s, minion);
        });
    assert_eq!(
        (sent, taken),
        (Some(minion), true),
        "the leap takes the minion"
    );
    assert!(place(&session, LOCAL_ADDR).distance(minion) < 0.2);
    assert!(shielded(&session, LOCAL_ADDR));
}

/// One touch of the button of `slot` by a hero of `class` that walks at the origin, in the
/// world that `arrange` fills, through the touch cast and the cast path: a tap, or a drag
/// along a screen direction to a fraction of the range.
fn touch_among(
    class: HeroClass,
    slot: usize,
    drag: Option<(Vec2, f32)>,
    arrange: impl FnOnce(&mut App),
) -> Press {
    let mut app = standard_cast_app(class, slot, false);
    app.world_mut().resource_mut::<PendingCast>().cancel();
    let mut mobile = crate::mobile_controls::MobileControls::default();
    mobile.enabled = true;
    mobile.casts.push(crate::mobile_controls::MobileCastIntent {
        slot,
        extent: drag.map_or(0.15, |(_, extent)| extent),
        aim: drag.map(|(direction, _)| direction),
    });
    app.insert_resource(mobile)
        .init_resource::<TargetState>()
        .init_resource::<crate::targeting::BasicAttackState>()
        .insert_resource(PlayerVisualMode::Models3d)
        .add_systems(
            Update,
            super::mobile::mobile_cast_system.before(resolve_pending_cast_system),
        );
    app.world_mut()
        .spawn((MainCamera, Camera::default(), GlobalTransform::IDENTITY));
    arrange(&mut app);
    let hero = app
        .world_mut()
        .query_filtered::<Entity, With<Player>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(hero)
        .insert(MovementTarget { target: Vec3::Z });
    app.update();
    let sent = sent_commands(&mut app);
    let cooldowns = app.world().resource::<LocalCastCooldown>();
    Press {
        sent,
        feedback: app.world().resource::<ActionFeedback>().text.clone(),
        cooldown: cooldowns.remaining_secs[slot],
        predicted: cooldowns.pending_slot.is_some() || cooldowns.recovery_secs > 0.0,
        walking: app.world().entity(hero).contains::<MovementTarget>(),
        queued: app.world().resource::<PendingCast>().is_pending(),
    }
}

/// A tap has no aim of its own. For a skill that is cast on an ally it is aimed at an
/// ally and the cast goes out; it used to be aimed at the nearest enemy, which the server
/// refuses or turns into a cast on the caster.
#[test]
fn a_tap_on_a_skill_cast_on_an_ally_is_sent_to_an_ally_and_never_to_an_enemy() {
    use shared::loadout::SkillId;

    let enemy = Vec2::X * 2.0;
    let near = Vec2::X * 4.0;
    let hurt = Vec2::Y * 8.0;
    let hurt_hero = |app: &mut App| {
        let hero = remote_hero(app, 6, Team::Green, hurt);
        app.world_mut().get_mut::<CombatStats>(hero).unwrap().hp *= 0.3;
    };

    // Sheltering Leap: the allied hero lowest on health, then an allied minion, then the
    // caster. The enemy is the nearest unit every time.
    let class = HeroClass::Frostguard;
    let w = preset_slot(class, SkillId::ShelteringLeap);
    touch_among(class, w, None, |app| {
        remote_hero(app, 2, Team::Blue, enemy);
        remote_hero(app, 5, Team::Green, near);
        hurt_hero(app);
    })
    .sent_to(w, hurt, "the leap, two allied heroes");
    touch_among(class, w, None, |app| {
        remote_hero(app, 2, Team::Blue, enemy);
        lane_minion(app, 4, Team::Green, Vec2::Y * 6.0);
    })
    .sent_to(w, Vec2::Y * 6.0, "the leap, an allied minion");
    touch_among(class, w, None, |app| {
        remote_hero(app, 2, Team::Blue, enemy);
    })
    .sent_to(w, Vec2::ZERO, "the leap, nobody but the caster");

    // Orbital Guard: the nearest allied hero, else the caster.
    let class = HeroClass::Orbitwright;
    let e = preset_slot(class, SkillId::OrbitalGuard);
    touch_among(class, e, None, |app| {
        remote_hero(app, 2, Team::Blue, enemy);
        remote_hero(app, 5, Team::Green, near);
        hurt_hero(app);
        lane_minion(app, 4, Team::Green, Vec2::Y);
    })
    .sent_to(e, near, "the orb, two allied heroes");
    touch_among(class, e, None, |app| {
        remote_hero(app, 2, Team::Blue, enemy);
        lane_minion(app, 4, Team::Green, Vec2::Y);
    })
    .sent_to(e, Vec2::ZERO, "the orb, nobody but the caster");

    // A drag is the player's own aim and is not replaced: nobody stands at its end here,
    // so the cast is refused with the line, and an ally at its end is taken.
    let class = HeroClass::Frostguard;
    touch_among(class, w, Some((Vec2::X, 0.5)), |app| {
        hurt_hero(app);
    })
    .refused_for_want_of_an_ally("a drag past the ally");
    touch_among(class, w, Some((Vec2::X, 0.5)), |app| {
        hurt_hero(app);
        remote_hero(app, 5, Team::Green, Vec2::X * 5.5);
    })
    .sent_to(w, Vec2::X * 5.5, "a drag onto an ally");

    // Every other tap keeps its aim on the enemy: a lane, and the leap that may go
    // without an ally (Anchor Step).
    let q = preset_slot(class, SkillId::WinterShard);
    touch_among(class, q, None, |app| {
        remote_hero(app, 2, Team::Blue, enemy);
        remote_hero(app, 5, Team::Green, near);
    })
    .sent_to(q, enemy, "a lane");
    let class = HeroClass::Stormfist;
    let step = preset_slot(class, SkillId::AnchorStep);
    touch_among(class, step, None, |app| {
        remote_hero(app, 2, Team::Blue, enemy);
        remote_hero(app, 5, Team::Green, Vec2::Y * 6.0);
    })
    .sent_to(step, enemy, "Anchor Step");
}

#[test]
fn standard_keyboard_holds_before_cast_and_cancels_when_context_is_lost() {
    for canceled in [false, true] {
        let mut app = standard_cast_app(HeroClass::Dawnweaver, 0, false);
        app.world_mut().resource_mut::<PendingCast>().cancel();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<TargetState>()
            .add_systems(
                Update,
                cast_spell_system.before(resolve_pending_cast_system),
            );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyQ);
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count(),
            0,
            "holding only previews"
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        if canceled {
            app.world_mut()
                .resource_mut::<GameplayInputContext>()
                .modal_open = true;
            app.update();
            app.world_mut()
                .resource_mut::<GameplayInputContext>()
                .modal_open = false;
        }
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyQ);
        app.update();
        let sent = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect::<Vec<_>>();
        if canceled {
            assert!(
                sent.is_empty(),
                "a canceled hold cannot cast after the menu closes"
            );
        } else {
            assert!(matches!(
                sent.as_slice(),
                [NetworkCommand::CastSkill { slot: 0, .. }]
            ));
        }
    }
}

#[test]
fn standard_cast_predicts_recovery_and_preserves_the_next_aimed_input_until_ready() {
    let mut app = standard_cast_app(HeroClass::Dawnweaver, 0, false);
    app.init_resource::<Time>().add_systems(
        Update,
        tick_local_cast_cooldown.before(resolve_pending_cast_system),
    );
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        1
    );
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().recovery_secs,
        0.15
    );
    {
        let mut pending = app.world_mut().resource_mut::<PendingCast>();
        pending.request = Some(PendingCastRequest {
            slot: 1,
            target_entity: None,
            target: None,
            approach_announced: false,
        });
        pending.aim = Some(Vec2::new(4.0, 5.0));
    }
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(100));
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
    assert!(app.world().resource::<PendingCast>().is_pending());
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(60));
    app.update();
    let sent = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect::<Vec<_>>();
    assert!(
        matches!(sent.as_slice(), [NetworkCommand::CastSkill {slot:1,aim}] if *aim==Vec2::new(4.0,5.0))
    );
    assert!(!app.world().resource::<PendingCast>().is_pending());
}

#[test]
fn balanced_skill_recovery_buffers_next_slot_and_uses_level_cooldowns() {
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<TeamSelection>()
        .init_resource::<PendingCast>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<ActionFeedback>()
        .init_resource::<GameplayInputContext>()
        .add_message::<NetworkCommand>()
        .add_systems(
            Update,
            (tick_local_cast_cooldown, resolve_pending_cast_system).chain(),
        );
    app.world_mut().spawn((
        Player,
        Transform::default(),
        CombatStats::default(),
        PlayerProgression {
            level: 10,
            ..default()
        },
        NetworkPlayerId(1),
        Team::Green,
        NetworkHeroClass(HeroClass::Warrior),
    ));
    let enemy = app
        .world_mut()
        .spawn((
            Transform::from_xyz(2.0, 0.0, 0.0),
            CombatStats::default(),
            Team::Blue,
            NetworkPlayerId(2),
        ))
        .id();
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot: 0,
        target_entity: Some(enemy),
        target: Some(TargetId {
            kind: TargetKind::Player,
            id: 2,
        }),
        approach_announced: false,
    });
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        1
    );
    let cd = app.world().resource::<LocalCastCooldown>();
    assert!((cd.remaining_secs[0] - 1.25).abs() < 0.001); // 2s / 1.6 growth
    assert!((cd.recovery_secs - 0.3).abs() < 0.001);
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot: 1,
        target_entity: None,
        target: None,
        approach_announced: false,
    });
    app.update();
    assert!(app.world().resource::<PendingCast>().request.is_some());
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(310));
    app.update();
    assert!(app.world().resource::<PendingCast>().request.is_none());
    let sent: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect();
    assert!(matches!(
        sent.as_slice(),
        [NetworkCommand::Cast { slot: 1, .. }]
    ));
}

#[test]
fn authoritative_skill_deadlines_restore_after_reconnect_and_age_between_snapshots() {
    use crate::net::{PlayerEquipment, PlayerSkillCooldowns};
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<LocalCastCooldown>()
        .add_systems(
            Update,
            (
                tick_local_cast_cooldown,
                sync_authoritative_cooldown_durations,
            )
                .chain(),
        );
    let hero = app
        .world_mut()
        .spawn((
            Player,
            PlayerProgression {
                level: 10,
                ..default()
            },
            NetworkHeroClass(HeroClass::Warrior),
            PlayerEquipment::default(),
            PlayerSkillCooldowns {
                remaining_secs: [0.8, 2.0, 0.0, 10.0],
                recovery_secs: 0.2,
            },
        ))
        .id();
    app.update();
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs,
        [0.8, 2.0, 0.0, 10.0]
    );
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(100));
    app.update();
    assert!((app.world().resource::<LocalCastCooldown>().remaining_secs[0] - 0.7).abs() < 0.001);
    assert!((app.world().resource::<LocalCastCooldown>().recovery_secs - 0.1).abs() < 0.001);
    // A fresh reset/respawn snapshot removes old deadlines immediately.
    app.world_mut()
        .entity_mut(hero)
        .insert(PlayerSkillCooldowns::default());
    app.update();
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs,
        [0.0; 4]
    );
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().recovery_secs,
        0.0
    );
}

#[test]
fn sandbox_slow_attack_rate_matches_authoritative_q_duration() {
    let bonuses = shared::shop::ItemBonuses {
        attack_speed_multiplier: 0.25,
        ..default()
    };
    let normal = effective_cast_duration(HeroClass::Warrior, 1, 1, SkillSlot::Q, bonuses, false);
    assert_eq!(
        effective_cast_duration(HeroClass::Warrior, 1, 1, SkillSlot::Q, bonuses, true),
        normal * 4.0
    );
    assert_eq!(
        effective_cast_duration(HeroClass::Warrior, 1, 1, SkillSlot::W, bonuses, true),
        effective_cast_duration(HeroClass::Warrior, 1, 1, SkillSlot::W, bonuses, false)
    );
}

#[test]
fn utility_intents_obey_snapshot_cooldowns_and_clear_on_modal_death_or_focus_loss() {
    use shared::utility::{UtilityAction, UtilityState};
    for gate in 0..5 {
        let mut app = App::new();
        let mut mobile = crate::mobile_controls::MobileControls::default();
        mobile.enabled = true;
        mobile.utilities = vec![(UtilityAction::Dash, None), (UtilityAction::Haste, None)];
        if gate == 4 {
            mobile.focused = false;
        }
        app.insert_resource(mobile)
            .insert_resource(GameplayInputContext {
                modal_open: gate == 1,
                ..default()
            })
            .insert_resource(PlayerVisualMode::Models3d)
            .add_message::<NetworkCommand>()
            .add_systems(Update, mobile_utility_system);
        app.world_mut().spawn((
            Player,
            Transform::default(),
            CombatStats {
                hp: if gate == 2 { 0.0 } else { 100.0 },
                ..default()
            },
            crate::net::PlayerUtility {
                state: UtilityState {
                    dash_remaining_secs: if gate == 3 { 5.0 } else { 0.0 },
                    haste_remaining_secs: if gate == 3 { 8.0 } else { 0.0 },
                    ..default()
                },
            },
        ));
        app.update();
        let sent: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect();
        assert!(
            app.world()
                .resource::<crate::mobile_controls::MobileControls>()
                .utilities
                .is_empty()
        );
        if gate == 0 {
            assert!(
                matches!(sent.as_slice(), [NetworkCommand::Utility { action: UtilityAction::Dash, direction }, NetworkCommand::Utility { action: UtilityAction::Haste, .. }] if *direction == Vec2::NEG_Y)
            );
        } else {
            assert!(sent.is_empty());
        }
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count(),
            0
        );
    }
}

#[test]
fn hidden_and_protected_nearest_candidates_do_not_mask_visible_targets() {
    use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
    let mut app = App::new();
    let hidden = app
        .world_mut()
        .spawn((
            RemotePlayer,
            Transform::from_xyz(0.1, 0.0, 0.0),
            Team::Blue,
            NetworkPlayerId(2),
            CombatStats::default(),
            InheritedVisibility::HIDDEN,
        ))
        .id();
    app.world_mut().spawn((
        NetworkStructure,
        Transform::from_xyz(0.05, 0.0, 0.0),
        Team::Blue,
        NetworkStructureId(3),
        StructureKind::BaseTower,
        CombatStats::default(),
        crate::net::NetworkStructureProtected(true),
    ));
    let visible = app
        .world_mut()
        .spawn((
            RemotePlayer,
            Transform::from_xyz(0.12, 0.0, 0.0),
            Team::Blue,
            NetworkPlayerId(4),
            CombatStats::default(),
            InheritedVisibility::VISIBLE,
        ))
        .id();
    let camera = Camera {
        computed: ComputedCameraValues {
            clip_from_view: Mat4::IDENTITY,
            target_info: Some(RenderTargetInfo {
                physical_size: UVec2::new(800, 400),
                scale_factor: 1.0,
            }),
            ..default()
        },
        ..default()
    };
    let mut params = bevy::ecs::system::SystemState::<(
        TargetCandidates,
        crate::targeting::TargetValidity,
    )>::new(app.world_mut());
    let (candidates, validity) = params
        .get(app.world())
        .expect("targeting test resources exist");
    let expected = Some((
        visible,
        TargetId {
            kind: TargetKind::Player,
            id: 4,
        },
    ));
    assert_eq!(
        find_nearest_enemy_target(
            Vec3::ZERO,
            Team::Green,
            &validity,
            &candidates.players,
            &candidates.minions,
            &candidates.neutrals,
            &candidates.structures
        ),
        expected
    );
    assert_eq!(
        find_target_near_screen(
            Vec2::new(440.0, 200.0),
            &camera,
            &GlobalTransform::IDENTITY,
            PlayerVisualMode::Models3d,
            Team::Green,
            &validity,
            &candidates.players,
            &candidates.minions,
            &candidates.neutrals,
            &candidates.structures
        ),
        expected
    );
    assert_eq!(
        mobile_assisted_target(
            Vec3::ZERO,
            Team::Green,
            10.0,
            None,
            &candidates,
            &validity,
            &camera,
            &GlobalTransform::IDENTITY,
            PlayerVisualMode::Models3d,
            Some(hidden)
        ),
        expected
    );
}

#[test]
fn invalidated_pending_skill_stops_chase_without_cast_or_cooldown() {
    for invalidation in ["hidden", "friendly", "identity"] {
        for distance in [2.0, 30.0] {
            let mut app = App::new();
            app.add_message::<NetworkCommand>()
                .init_resource::<TeamSelection>()
                .init_resource::<PendingCast>()
                .init_resource::<ActionFeedback>()
                .init_resource::<LocalCastCooldown>()
                .init_resource::<GameplayInputContext>()
                .add_systems(Update, resolve_pending_cast_system);
            let player = app
                .world_mut()
                .spawn((
                    Player,
                    Transform::default(),
                    Team::Green,
                    CombatStats::default(),
                    PlayerProgression::default(),
                    NetworkPlayerId(1),
                    NetworkHeroClass(HeroClass::Warrior),
                ))
                .id();
            let target = app
                .world_mut()
                .spawn((
                    Transform::from_xyz(30.0, 0.0, 0.0),
                    Team::Blue,
                    CombatStats::default(),
                    NetworkMinionId(77),
                    InheritedVisibility::VISIBLE,
                ))
                .id();
            app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
                slot: 0,
                target_entity: Some(target),
                target: Some(TargetId {
                    kind: TargetKind::Minion,
                    id: 77,
                }),
                approach_announced: false,
            });
            app.update();
            assert!(app.world().entity(player).contains::<MovementTarget>());
            app.world_mut()
                .entity_mut(player)
                .insert(crate::player::MovementRoute {
                    requested_target: Vec3::X * 30.0,
                    structure_revision: 1,
                    destination: Vec3::X * 30.0,
                    waypoints: vec![Vec3::X * 30.0],
                });
            app.world_mut()
                .entity_mut(target)
                .insert(Transform::from_xyz(distance, 0.0, 0.0));
            match invalidation {
                "hidden" => {
                    app.world_mut()
                        .entity_mut(target)
                        .insert(InheritedVisibility::HIDDEN);
                }
                "friendly" => {
                    app.world_mut().entity_mut(target).insert(Team::Green);
                }
                _ => {
                    app.world_mut()
                        .entity_mut(target)
                        .insert(NetworkMinionId(78));
                }
            }
            app.update();
            assert!(app.world().resource::<PendingCast>().request.is_none());
            assert!(!app.world().entity(player).contains::<MovementTarget>());
            assert!(
                !app.world()
                    .entity(player)
                    .contains::<crate::player::MovementRoute>()
            );
            assert_eq!(
                app.world().resource::<LocalCastCooldown>().remaining_secs,
                [0.0; 4]
            );
            assert!(
                app.world_mut()
                    .resource_mut::<Messages<NetworkCommand>>()
                    .drain()
                    .next()
                    .is_none(),
                "{invalidation} target at {distance} must not receive a queued skill"
            );
            assert!(
                app.world()
                    .resource::<ActionFeedback>()
                    .text
                    .contains("visible hostile")
            );
        }
    }
}

#[test]
fn desktop_mouse_select_attack_ground_and_ui_are_distinct() {
    use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
    let mut app = App::new();
    app.init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<GameplayInputContext>()
        .init_resource::<TargetState>()
        .init_resource::<PendingCast>()
        .init_resource::<WorldPointerState>()
        .init_resource::<BasicAttackState>()
        .insert_resource(PlayerVisualMode::Models3d)
        .add_systems(Update, select_target_system);
    let mut window = Window {
        resolution: bevy::window::WindowResolution::new(800, 400),
        ..default()
    };
    window.set_cursor_position(Some(Vec2::new(440.0, 200.0)));
    let window = app.world_mut().spawn((window, PrimaryWindow)).id();
    app.world_mut()
        .spawn((Player, Transform::default(), Team::Green));
    let enemy = app
        .world_mut()
        .spawn((
            RemotePlayer,
            Transform::from_xyz(0.1, 0.0, 0.0),
            Team::Blue,
            NetworkPlayerId(2),
            CombatStats::default(),
        ))
        .id();
    app.world_mut().spawn((
        MainCamera,
        GlobalTransform::IDENTITY,
        Camera {
            computed: ComputedCameraValues {
                clip_from_view: Mat4::IDENTITY,
                target_info: Some(RenderTargetInfo {
                    physical_size: UVec2::new(800, 400),
                    scale_factor: 1.0,
                }),
                ..default()
            },
            ..default()
        },
    ));
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    assert_eq!(
        app.world().resource::<TargetState>().selected_entity,
        Some(enemy)
    );
    assert!(app.world().resource::<BasicAttackState>().order.is_none());
    assert!(!app.world().resource::<PendingCast>().is_pending());
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Right);
    app.update();
    let attack = app.world().resource::<BasicAttackState>().order.unwrap();
    assert_eq!(attack.entity, enemy);
    assert_eq!(attack.target.id, 2);
    assert!(attack.repeat);
    assert!(
        app.world()
            .resource::<WorldPointerState>()
            .consumed_secondary_press
    );
    assert!(!app.world().resource::<PendingCast>().is_pending());
    app.world_mut().resource_mut::<BasicAttackState>().cancel();
    let ui = app.world_mut().spawn((Button, Interaction::Hovered)).id();
    app.update();
    assert!(app.world().resource::<BasicAttackState>().order.is_none());
    app.world_mut().despawn(ui);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::AltLeft);
    app.update();
    assert!(app.world().resource::<BasicAttackState>().order.is_none());
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(Vec2::new(700.0, 100.0)));
    app.update();
    assert!(
        !app.world()
            .resource::<WorldPointerState>()
            .consumed_secondary_press
    );
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    assert!(
        app.world()
            .resource::<TargetState>()
            .selected_entity
            .is_none()
    );
}

#[test]
fn directional_mobile_assist_respects_range_and_does_not_snap_behind_aim() {
    assert!(mobile_target_score(6.0, 5.0, Vec2::X, Some(Vec2::X), false).is_none());
    assert!(mobile_target_score(3.0, 5.0, Vec2::NEG_X, Some(Vec2::X), true).is_none());
    assert!(mobile_target_score(3.0, 5.0, Vec2::Y, Some(Vec2::X), false).is_none());
    let forward = mobile_target_score(3.0, 5.0, Vec2::X, Some(Vec2::X), false).unwrap();
    let edge = mobile_target_score(3.0, 5.0, Vec2::new(1.0, 0.5), Some(Vec2::X), false).unwrap();
    assert!(forward < edge);
    assert!(
        mobile_target_score(4.0, 5.0, Vec2::X, None, true)
            < mobile_target_score(1.0, 5.0, Vec2::X, None, false)
    );
}

#[test]
fn mobile_pending_cast_in_range_emits_and_out_of_range_never_starts_a_chase() {
    let mut app = App::new();
    let mut mobile = crate::mobile_controls::MobileControls::default();
    mobile.enabled = true;
    app.insert_resource(mobile)
        .add_message::<NetworkCommand>()
        .init_resource::<TeamSelection>()
        .init_resource::<PendingCast>()
        .init_resource::<ActionFeedback>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<GameplayInputContext>()
        .add_systems(Update, resolve_pending_cast_system);
    let player = app
        .world_mut()
        .spawn((
            Player,
            Transform::default(),
            CombatStats::default(),
            PlayerProgression::default(),
            NetworkHeroClass(HeroClass::Warrior),
            NetworkPlayerId(1),
            Team::Green,
        ))
        .id();
    let enemy = app
        .world_mut()
        .spawn((
            Transform::from_xyz(100.0, 0.0, 0.0),
            CombatStats::default(),
            Team::Blue,
            NetworkPlayerId(2),
        ))
        .id();
    let request = PendingCastRequest {
        slot: 0,
        target_entity: Some(enemy),
        target: Some(TargetId {
            kind: TargetKind::Player,
            id: 2,
        }),
        approach_announced: false,
    };
    app.world_mut().resource_mut::<PendingCast>().request = Some(request);
    app.update();
    assert!(app.world().resource::<PendingCast>().request.is_none());
    assert!(!app.world().entity(player).contains::<MovementTarget>());
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs,
        [0.0; 4]
    );
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
    app.world_mut()
        .entity_mut(enemy)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::X;
    app.world_mut().resource_mut::<PendingCast>().request = Some(request);
    app.update();
    let emitted: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect();
    assert_eq!(emitted.len(), 1);
    assert!(matches!(emitted[0], NetworkCommand::Cast { slot: 0, .. }));
    assert!(app.world().resource::<LocalCastCooldown>().remaining_secs[0] > 0.0);
    assert!(!app.world().entity(player).contains::<MovementTarget>());
}

#[test]
fn pointer_hit_areas_are_touch_sized_and_screen_bounded() {
    const {
        assert!(SKILL_SLOT_SIZE >= 48.0);
    }
    for radius in [
        PLAYER_PICK_RADIUS_PX,
        MINION_PICK_RADIUS_PX,
        NEUTRAL_PICK_RADIUS_PX,
        TOWER_PICK_RADIUS_PX,
        BASE_TOWER_PICK_RADIUS_PX,
    ] {
        assert!(radius >= 48.0);
        assert!(screen_pick_distance(Vec2::ZERO, Vec2::new(radius, 0.0), radius).is_some());
        assert!(screen_pick_distance(Vec2::ZERO, Vec2::new(radius + 0.1, 0.0), radius).is_none());
    }
}

#[test]
fn explicit_q_skill_request_preserves_the_exact_authoritative_target() {
    let entity = Entity::PLACEHOLDER;
    let target = TargetId {
        kind: TargetKind::Minion,
        id: 77,
    };
    let state = TargetState {
        selected_entity: Some(entity),
        selected_target: Some(target),
        marker_entity: None,
    };
    let mut pending = PendingCast::default();
    let mut feedback = ActionFeedback::default();
    queue_cast_request(
        SkillSlot::Q.index(),
        &crate::equipped_skills::resolve(HeroClass::Warrior, None).unwrap(),
        &state,
        &mut pending,
        &mut feedback,
    );
    assert_eq!(
        pending.request,
        Some(PendingCastRequest {
            slot: SkillSlot::Q.index(),
            target_entity: Some(entity),
            target: Some(target),
            approach_announced: false,
        })
    );
}

#[test]
fn cast_range_uses_horizontal_gameplay_distance() {
    assert!(within_cast_range(
        Vec3::ZERO,
        Vec3::new(3.0, 99.0, 4.0),
        5.0
    ));
    assert!(!within_cast_range(
        Vec3::ZERO,
        Vec3::new(3.01, 0.0, 4.0),
        5.0
    ));
}

#[test]
fn pending_unit_cast_approaches_then_emits_once_in_range() {
    let mut app = App::new();
    app.add_message::<NetworkCommand>()
        .insert_resource(TeamSelection::default())
        .insert_resource(PendingCast::default())
        .insert_resource(LocalCastCooldown::default())
        .insert_resource(ActionFeedback::default())
        .init_resource::<GameplayInputContext>()
        .add_systems(Update, resolve_pending_cast_system);

    let player = app
        .world_mut()
        .spawn((
            Player,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CombatStats::default(),
            PlayerProgression::default(),
            NetworkPlayerId(1),
            Team::Green,
            NetworkHeroClass(HeroClass::Warrior),
        ))
        .id();
    let target = app
        .world_mut()
        .spawn((
            Transform::from_xyz(30.0, 0.0, 0.0),
            CombatStats::default(),
            Team::Blue,
            NetworkMinionId(77),
        ))
        .id();
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot: SkillSlot::Q.index(),
        target_entity: Some(target),
        target: Some(TargetId {
            kind: TargetKind::Minion,
            id: 77,
        }),
        approach_announced: false,
    });

    app.update();
    assert!(app.world().entity(player).contains::<MovementTarget>());
    assert!(app.world().resource::<PendingCast>().request.is_some());
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs[0],
        0.0
    );
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );

    app.world_mut()
        .entity_mut(player)
        .insert(Transform::from_xyz(20.0, 0.0, 0.0));
    app.update();
    assert!(app.world().resource::<PendingCast>().request.is_none());
    assert!(app.world().resource::<LocalCastCooldown>().remaining_secs[0] > 0.0);
    let commands: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect();
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        commands[0],
        NetworkCommand::Cast {
            target: TargetId {
                kind: TargetKind::Minion,
                id: 77
            },
            slot: 0
        }
    ));
}

#[test]
fn insufficient_mana_rejects_without_phantom_cooldown() {
    let mut app = App::new();
    app.add_message::<NetworkCommand>()
        .insert_resource(TeamSelection::default())
        .insert_resource(PendingCast::default())
        .insert_resource(LocalCastCooldown::default())
        .insert_resource(ActionFeedback::default())
        .init_resource::<GameplayInputContext>()
        .add_systems(Update, resolve_pending_cast_system);

    let exhausted = CombatStats {
        mana: 0.0,
        ..default()
    };
    app.world_mut().spawn((
        Player,
        Transform::default(),
        exhausted,
        PlayerProgression::default(),
        NetworkPlayerId(1),
        Team::Green,
        NetworkHeroClass(HeroClass::Warrior),
    ));
    let target = app
        .world_mut()
        .spawn((
            Transform::from_xyz(2.0, 0.0, 0.0),
            CombatStats::default(),
            Team::Blue,
            NetworkMinionId(88),
        ))
        .id();
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot: SkillSlot::Q.index(),
        target_entity: Some(target),
        target: Some(TargetId {
            kind: TargetKind::Minion,
            id: 88,
        }),
        approach_announced: false,
    });

    app.update();
    assert!(app.world().resource::<PendingCast>().request.is_none());
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs[0],
        0.0
    );
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
}

#[test]
fn self_target_hotbar_request_needs_no_selected_enemy() {
    let state = TargetState::default();
    let mut pending = PendingCast::default();
    let mut feedback = ActionFeedback::default();
    queue_cast_request(
        SkillSlot::W.index(),
        &crate::equipped_skills::resolve(HeroClass::Warrior, None).unwrap(),
        &state,
        &mut pending,
        &mut feedback,
    );
    assert_eq!(
        pending.request,
        Some(PendingCastRequest {
            slot: SkillSlot::W.index(),
            target_entity: None,
            target: None,
            approach_announced: false,
        })
    );
}

/// A level-ten legacy hero at the origin that walks to `ORDER`, for the move-order rule.
fn legacy_hero_with_move_order(class: HeroClass) -> (App, Entity) {
    let mut app = App::new();
    app.add_message::<NetworkCommand>()
        .init_resource::<TeamSelection>()
        .init_resource::<PendingCast>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<ActionFeedback>()
        .init_resource::<GameplayInputContext>()
        .add_systems(Update, resolve_pending_cast_system);
    let hero = app
        .world_mut()
        .spawn((
            Player,
            Transform::default(),
            CombatStats::default(),
            PlayerProgression {
                level: 10,
                ..default()
            },
            NetworkPlayerId(1),
            Team::Green,
            NetworkHeroClass(class),
            MovementTarget { target: ORDER },
            crate::player::MovementRoute {
                requested_target: ORDER,
                structure_revision: 1,
                destination: ORDER,
                waypoints: vec![ORDER],
            },
        ))
        .id();
    (app, hero)
}

const ORDER: Vec3 = Vec3::new(-12.0, 0.0, 5.0);

fn queue_slot(app: &mut App, slot: usize, target: Option<(Entity, TargetId)>) {
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot,
        target_entity: target.map(|(entity, _)| entity),
        target: target.map(|(_, id)| id),
        approach_announced: false,
    });
}

fn sent_commands(app: &mut App) -> Vec<NetworkCommand> {
    app.world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect()
}

fn move_order(app: &App, hero: Entity) -> Option<Vec3> {
    app.world()
        .get::<MovementTarget>(hero)
        .map(|order| order.target)
}

#[test]
fn self_target_cast_keeps_the_move_order_and_only_ends_a_cast_approach() {
    use super::cast::cast_clears_move_order;
    use shared::TargetingMode::{SelfTarget, UnitTarget};

    let approach = Vec3::new(30.0, 0.0, 0.0);
    for (targeting, order, expected) in [
        (SelfTarget, Some(ORDER), false),
        (SelfTarget, Some(approach), true),
        (SelfTarget, None, false),
        (UnitTarget, Some(ORDER), true),
        (UnitTarget, Some(approach), true),
    ] {
        assert_eq!(
            cast_clears_move_order(targeting, order, Some(approach)),
            expected,
            "{targeting:?} {order:?}"
        );
    }
    assert!(!cast_clears_move_order(SelfTarget, Some(ORDER), None));

    // 1. Every legacy self cast is sent and leaves the player's order and route alone.
    let legacy = HeroClass::ALL
        .into_iter()
        .filter(|class| shared::loadout::preset_for_class(*class).is_none());
    let mut self_casts = 0;
    for class in legacy {
        for slot in SkillSlot::ALL {
            if ability_for_class_slot(class, slot).targeting != SelfTarget {
                continue;
            }
            self_casts += 1;
            let (mut app, hero) = legacy_hero_with_move_order(class);
            queue_slot(&mut app, slot.index(), None);
            app.update();
            let sent = sent_commands(&mut app);
            assert!(
                matches!(
                    sent.as_slice(),
                    [NetworkCommand::Cast {
                        target: TargetId {
                            kind: TargetKind::Player,
                            id: 1
                        },
                        slot: sent_slot
                    }] if usize::from(*sent_slot) == slot.index()
                ),
                "{class:?} {slot:?}: {sent:?}"
            );
            assert_eq!(move_order(&app, hero), Some(ORDER), "{class:?} {slot:?}");
            assert!(
                app.world()
                    .entity(hero)
                    .contains::<crate::player::MovementRoute>(),
                "{class:?} {slot:?}"
            );
        }
    }
    assert_eq!(self_casts, 7);

    // 2. A refused self cast says why and keeps the order as well.
    let (mut app, hero) = legacy_hero_with_move_order(HeroClass::Ranger);
    app.world_mut().get_mut::<CombatStats>(hero).unwrap().mana = 0.0;
    queue_slot(&mut app, SkillSlot::W.index(), None);
    app.update();
    assert!(sent_commands(&mut app).is_empty());
    assert!(!app.world().resource::<ActionFeedback>().text.is_empty());
    assert!(!app.world().resource::<PendingCast>().is_pending());
    assert_eq!(move_order(&app, hero), Some(ORDER));
    assert!(
        app.world()
            .entity(hero)
            .contains::<crate::player::MovementRoute>()
    );

    // 3. A walk into cast range is not an order of the player: the self cast that
    // replaces the queued shot ends it, and a later order of the player is kept again.
    let (mut app, hero) = legacy_hero_with_move_order(HeroClass::Ranger);
    let target = app
        .world_mut()
        .spawn((
            Transform::from_translation(approach),
            CombatStats::default(),
            Team::Blue,
            NetworkMinionId(77),
        ))
        .id();
    let id = TargetId {
        kind: TargetKind::Minion,
        id: 77,
    };
    queue_slot(&mut app, SkillSlot::Q.index(), Some((target, id)));
    app.update();
    assert_eq!(move_order(&app, hero), Some(approach));
    assert!(sent_commands(&mut app).is_empty());
    let skills = crate::equipped_skills::resolve(HeroClass::Ranger, None).unwrap();
    {
        let world = app.world_mut();
        let mut feedback = world.remove_resource::<ActionFeedback>().unwrap();
        queue_cast_request(
            SkillSlot::W.index(),
            &skills,
            &TargetState::default(),
            &mut world.resource_mut::<PendingCast>(),
            &mut feedback,
        );
        world.insert_resource(feedback);
    }
    app.update();
    assert!(matches!(
        sent_commands(&mut app).as_slice(),
        [NetworkCommand::Cast { slot: 1, .. }]
    ));
    assert_eq!(move_order(&app, hero), None);
    *app.world_mut().resource_mut::<LocalCastCooldown>() = LocalCastCooldown::default();
    app.world_mut()
        .entity_mut(hero)
        .insert(MovementTarget { target: ORDER });
    queue_slot(&mut app, SkillSlot::W.index(), None);
    app.update();
    assert!(matches!(
        sent_commands(&mut app).as_slice(),
        [NetworkCommand::Cast { slot: 1, .. }]
    ));
    assert_eq!(move_order(&app, hero), Some(ORDER));

    // 4. A cast on a unit stops the hero as before, whoever gave the order.
    let (mut app, hero) = legacy_hero_with_move_order(HeroClass::Ranger);
    let target = app
        .world_mut()
        .spawn((
            Transform::from_xyz(2.0, 0.0, 0.0),
            CombatStats::default(),
            Team::Blue,
            NetworkMinionId(77),
        ))
        .id();
    queue_slot(&mut app, SkillSlot::Q.index(), Some((target, id)));
    app.update();
    assert!(matches!(
        sent_commands(&mut app).as_slice(),
        [NetworkCommand::Cast { slot: 0, .. }]
    ));
    assert_eq!(move_order(&app, hero), None);
}

#[test]
fn actual_cast_and_upgrade_systems_obey_help_pause_and_debug_context() {
    let mut app = App::new();
    app.add_message::<NetworkCommand>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<TeamSelection>()
        .init_resource::<PendingCast>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>()
        .init_resource::<TargetAimPreview>()
        .init_resource::<ActionFeedback>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<crate::pause_menu::PauseMenuState>()
        .add_message::<crate::ui::Activated<super::hotbar::HotbarAction>>()
        .insert_resource(GameStateSnapshot {
            state: GameState::Running,
            ..default()
        })
        .add_plugins((
            crate::input_context::InputContextPlugin,
            crate::help_overlay::HelpOverlayPlugin,
        ))
        .add_systems(
            Update,
            (
                cast_spell_system,
                skill_upgrade_input_system,
                resolve_pending_cast_system,
            )
                .chain()
                .in_set(InputContextSet::Actions),
        );
    app.world_mut().spawn((
        Player,
        Transform::default(),
        CombatStats::default(),
        PlayerProgression {
            level: 6,
            skill_points: 2,
            ..default()
        },
        NetworkHeroClass(HeroClass::Cleric),
        NetworkPlayerId(1),
        Team::Green,
    ));
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyU);
    app.update(); // Automatic first-match help must suppress these same-frame keys.
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
    assert!(app.world().resource::<GameplayInputContext>().modal_open);
    app.world_mut()
        .resource_mut::<crate::help_overlay::HelpOverlayVisible>()
        .0 = false;
    app.world_mut()
        .resource_mut::<crate::pause_menu::PauseMenuState>()
        .open = true;
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
    app.world_mut()
        .resource_mut::<crate::pause_menu::PauseMenuState>()
        .open = false;
    let mut debug = crate::debug::DebugConsole::default();
    debug.ui_enabled = true;
    app.insert_resource(debug);
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .debug_flight = true;
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .debug_flight = false;
    app.update();
    let emitted: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect();
    assert!(
        emitted
            .iter()
            .any(|command| matches!(command, NetworkCommand::Cast { slot: 1, .. }))
    );
    assert!(
        emitted
            .iter()
            .any(|command| matches!(command, NetworkCommand::UpgradeSkill { .. }))
    );
}

#[test]
fn protected_base_rejection_is_visible_and_does_not_approach_or_start_cooldown() {
    let mut app = App::new();
    app.add_message::<NetworkCommand>()
        .init_resource::<TeamSelection>()
        .init_resource::<PendingCast>()
        .init_resource::<ActionFeedback>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<GameplayInputContext>()
        .add_systems(Update, resolve_pending_cast_system);
    let player = app
        .world_mut()
        .spawn((
            Player,
            Transform::default(),
            CombatStats::default(),
            PlayerProgression::default(),
            NetworkHeroClass(HeroClass::Warrior),
            NetworkPlayerId(1),
            Team::Green,
        ))
        .id();
    let base = app
        .world_mut()
        .spawn((
            Transform::from_xyz(100.0, 0.0, 0.0),
            CombatStats::default(),
            crate::net::NetworkStructureProtected(true),
            Team::Blue,
            NetworkStructureId(2),
        ))
        .id();
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot: 0,
        target_entity: Some(base),
        target: Some(TargetId {
            kind: TargetKind::Structure,
            id: 2,
        }),
        approach_announced: false,
    });
    app.update();
    assert!(
        app.world()
            .resource::<ActionFeedback>()
            .text
            .contains("Structure protected")
    );
    assert!(app.world().resource::<PendingCast>().request.is_none());
    assert!(!app.world().entity(player).contains::<MovementTarget>());
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs,
        [0.0; 4]
    );
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
}

#[test]
fn desktop_skill_art_tracks_class_loading_and_unavailable_state() {
    let mut app = App::new();
    app.init_resource::<TeamSelection>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<TargetState>()
        .init_resource::<PendingCast>()
        .init_resource::<Assets<Image>>()
        .add_systems(Update, update_skill_bar_system);
    let player = app
        .world_mut()
        .spawn((
            Player,
            Transform::default(),
            CombatStats::default(),
            PlayerProgression::default(),
            NetworkHeroClass(HeroClass::Mage),
        ))
        .id();
    let icon = app
        .world_mut()
        .spawn((
            DesktopSkillIcon { slot: 0 },
            ImageNode::default(),
            Node::default(),
        ))
        .id();
    let locked = app
        .world_mut()
        .spawn((
            DesktopSkillIcon { slot: 3 },
            ImageNode::default(),
            Node::default(),
        ))
        .id();
    app.update();
    assert_eq!(
        app.world().get::<Node>(icon).unwrap().display,
        Display::None
    );
    let atlas = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::default());
    for entity in [icon, locked] {
        app.world_mut().get_mut::<ImageNode>(entity).unwrap().image = atlas.clone();
    }
    app.update();
    let mage_rect = app.world().get::<ImageNode>(icon).unwrap().rect.unwrap();
    assert_eq!(
        app.world().get::<Node>(icon).unwrap().display,
        Display::Flex
    );
    assert_eq!(
        app.world().get::<ImageNode>(icon).unwrap().color,
        Color::WHITE
    );
    assert_ne!(
        app.world().get::<ImageNode>(locked).unwrap().color,
        Color::WHITE
    );
    app.world_mut()
        .get_mut::<NetworkHeroClass>(player)
        .unwrap()
        .0 = HeroClass::Ranger;
    app.update();
    assert_ne!(
        app.world().get::<ImageNode>(icon).unwrap().rect,
        Some(mage_rect)
    );
    app.world_mut()
        .resource_mut::<LocalCastCooldown>()
        .remaining_secs[0] = 2.0;
    app.update();
    assert_ne!(
        app.world().get::<ImageNode>(icon).unwrap().color,
        Color::WHITE
    );
    app.world_mut()
        .resource_mut::<LocalCastCooldown>()
        .remaining_secs[0] = 0.0;
    app.world_mut().get_mut::<CombatStats>(player).unwrap().mana = 0.0;
    app.update();
    assert_ne!(
        app.world().get::<ImageNode>(icon).unwrap().color,
        Color::WHITE
    );
    assert_eq!(
        app.world().get::<Node>(icon).unwrap().display,
        Display::Flex
    );
}

#[test]
fn feedback_expires_in_place_and_hotbar_shows_server_rank_lock_mana_and_cooldown() {
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<ActionFeedback>()
        .init_resource::<TeamSelection>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>()
        .init_resource::<TargetAimPreview>()
        .init_resource::<PendingCast>()
        .add_systems(
            Startup,
            (setup_combat_ui, crate::targeting::setup_targeting_ui),
        )
        .add_systems(Update, (update_action_feedback, update_skill_bar_system));
    let player = app
        .world_mut()
        .spawn((
            Player,
            Transform::default(),
            CombatStats {
                mana: 0.0,
                ..default()
            },
            PlayerProgression {
                level: 2,
                ranks: [2, 1, 1, 1],
                ..default()
            },
            NetworkHeroClass(HeroClass::Cleric),
        ))
        .id();
    app.world_mut()
        .resource_mut::<LocalCastCooldown>()
        .remaining_secs[0] = 1.5;
    app.world_mut()
        .resource_mut::<ActionFeedback>()
        .push_line("Not enough mana.");
    app.update();
    let mut labels = app.world_mut().query::<(&SkillRankLabel, &Text)>();
    let text: Vec<_> = labels
        .iter(app.world())
        .map(|(slot, text)| (slot.slot, text.0.clone()))
        .collect();
    assert!(
        text.iter()
            .any(|(slot, text)| *slot == 0 && text.contains("R2") && text.contains("1.5s"))
    );
    assert!(
        text.iter()
            .any(|(slot, text)| *slot == 1 && text.contains("Need mana"))
    );
    assert!(
        text.iter()
            .any(|(slot, text)| *slot == 3 && text.contains("Locked Lv 6"))
    );
    let count = app.world().entities().count_spawned();
    for _ in 0..5 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(1));
        app.update();
    }
    assert!(app.world().resource::<ActionFeedback>().text.is_empty());
    assert_eq!(app.world().entities().count_spawned(), count);
    assert!(app.world().entity(player).contains::<CombatStats>());
}

#[test]
fn target_selection_keys_obey_modal_context_at_the_ecs_boundary() {
    let mut app = App::new();
    app.init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<Touches>()
        .init_resource::<GameplayInputContext>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>()
        .init_resource::<TargetAimPreview>()
        .init_resource::<PendingCast>()
        .init_resource::<WorldPointerState>()
        .insert_resource(PlayerVisualMode::Models3d)
        .add_systems(Update, select_target_system);
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    app.world_mut()
        .spawn((Player, Transform::default(), Team::Green));
    let enemy = app
        .world_mut()
        .spawn((
            RemotePlayer,
            Transform::from_xyz(2.0, 0.0, 0.0),
            Team::Blue,
            NetworkPlayerId(2),
            CombatStats::default(),
        ))
        .id();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Tab);
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .modal_open = true;
    app.update();
    assert!(
        app.world()
            .resource::<TargetState>()
            .selected_entity
            .is_none()
    );
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .modal_open = false;
    app.update();
    assert_eq!(
        app.world().resource::<TargetState>().selected_entity,
        Some(enemy)
    );
}

#[test]
fn round_change_event_clears_old_intents_cooldowns_and_queued_casts_but_reconnect_events_do_not() {
    use crate::domain::RoundId;
    use crate::net::SessionEvent;
    let mut app = App::new();
    app.add_message::<NetworkCommand>()
        .add_message::<SessionEvent>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>()
        .init_resource::<TargetAimPreview>()
        .init_resource::<PendingCast>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<ActionFeedback>()
        .add_systems(Update, reset_round_input_state);
    app.update();
    let actor = app
        .world_mut()
        .spawn(MovementTarget {
            target: Vec3::X * 50.0,
        })
        .id();
    app.world_mut()
        .resource_mut::<TargetState>()
        .selected_entity = Some(actor);
    app.world_mut()
        .resource_mut::<LocalCastCooldown>()
        .remaining_secs[3] = 40.0;
    app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
        slot: 3,
        target_entity: Some(actor),
        target: None,
        approach_announced: true,
    });
    // A reconnect to the same round after a teardown: `net` announces no
    // `RoundChanged` for it (pinned in `net::apply`), so nothing resets.
    app.world_mut()
        .write_message(SessionEvent::TransportStarted {
            addr: "127.0.0.1:4000".into(),
            offline: false,
        });
    app.update();
    app.world_mut().write_message(SessionEvent::Connected);
    app.world_mut()
        .write_message(SessionEvent::Joined { your_id: 1 });
    app.update();
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs[3],
        40.0
    );
    assert!(app.world().entity(actor).contains::<MovementTarget>());
    assert!(app.world().resource::<PendingCast>().request.is_some());
    app.world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .write(NetworkCommand::Cast {
            target: TargetId {
                kind: TargetKind::Player,
                id: 2,
            },
            slot: 3,
        });
    // Messages retain delivered commands for two frames. Round reset must
    // discard the old rematch instead of writing it again with a fresh ID.
    app.world_mut()
        .write_message(NetworkCommand::RequestRematch);
    let mut sender = app
        .world()
        .resource::<Messages<NetworkCommand>>()
        .get_cursor();
    assert_eq!(
        sender
            .read(app.world().resource::<Messages<NetworkCommand>>())
            .count(),
        2
    );
    app.world_mut().write_message(SessionEvent::RoundChanged {
        previous: RoundId {
            server_epoch: 10,
            match_id: 1,
        },
        current: RoundId {
            server_epoch: 10,
            match_id: 2,
        },
    });
    app.update();
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs,
        [0.0; 4]
    );
    assert!(app.world().resource::<PendingCast>().request.is_none());
    assert!(
        app.world()
            .resource::<TargetState>()
            .selected_entity
            .is_none()
    );
    assert!(!app.world().entity(actor).contains::<MovementTarget>());
    assert_eq!(
        sender
            .read(app.world().resource::<Messages<NetworkCommand>>())
            .count(),
        0,
        "round change cannot replay a delivered rematch"
    );
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
}
#[test]
fn buying_haste_adjusts_active_deadlines_without_rescaling_elapsed_time() {
    use crate::net::PlayerEquipment;
    use shared::shop::{ItemId, item_bonuses, item_cooldown};
    let mut app = App::new();
    app.init_resource::<LocalCastCooldown>()
        .add_systems(Update, sync_authoritative_cooldown_durations);
    let hero = app
        .world_mut()
        .spawn((
            Player,
            PlayerProgression::default(),
            NetworkHeroClass(HeroClass::Mage),
            PlayerEquipment::default(),
        ))
        .id();
    app.update();
    let old = app.world().resource::<LocalCastCooldown>().total_secs;
    for (index, duration) in old.iter().enumerate() {
        app.world_mut()
            .resource_mut::<LocalCastCooldown>()
            .remaining_secs[index] = *duration * 0.5;
    }
    let bonuses = item_bonuses(&[ItemId::FocusCharm, ItemId::SwiftGrip]);
    app.world_mut().entity_mut(hero).insert(PlayerEquipment {
        item_bonuses: bonuses,
        ..default()
    });
    app.update();
    for slot in SkillSlot::ALL {
        let index = slot.index();
        let current = item_cooldown(
            ability_for_class_slot(HeroClass::Mage, slot),
            1,
            slot,
            bonuses,
        )
        .as_secs_f32();
        let expected = (old[index] * 0.5 + current - old[index]).max(0.0);
        assert!(
            (app.world().resource::<LocalCastCooldown>().remaining_secs[index] - expected).abs()
                < 0.0001
        );
        assert!(
            expected < current * 0.5,
            "elapsed time is preserved, not scaled"
        );
    }
    // A second identical authoritative snapshot cannot repeatedly shorten it.
    let once = app.world().resource::<LocalCastCooldown>().remaining_secs;
    app.update();
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().remaining_secs,
        once
    );
}

/// DECISIONS R7.1: a protected enemy structure stays selected for inspection
/// (the target plate shows it with a lock) while every attack order on it is
/// dropped; a dead or unprotected-but-invalid target still clears.
#[test]
fn protected_structures_stay_selected_for_inspection_but_never_keep_an_attack_order() {
    let mut app = App::new();
    app.init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<GameplayInputContext>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>()
        .init_resource::<PendingCast>()
        .init_resource::<TargetAimPreview>()
        .add_systems(Update, crate::targeting::clear_invalid_selection);
    app.world_mut().spawn((
        Player,
        Transform::default(),
        Team::Green,
        CombatStats::default(),
    ));
    let tower = app
        .world_mut()
        .spawn((
            NetworkStructure,
            NetworkStructureId(3),
            StructureKind::Tower,
            Team::Blue,
            Transform::from_xyz(2.0, 0.0, 0.0),
            CombatStats::default(),
            crate::net::NetworkStructureProtected(true),
        ))
        .id();
    let id = TargetId {
        kind: TargetKind::Structure,
        id: 3,
    };
    {
        let mut target = app.world_mut().resource_mut::<TargetState>();
        target.selected_entity = Some(tower);
        target.selected_target = Some(id);
    }
    app.world_mut()
        .resource_mut::<BasicAttackState>()
        .start(tower, id, true);
    app.update();
    let target = app.world().resource::<TargetState>();
    assert_eq!(target.selected_entity, Some(tower), "kept for inspection");
    assert!(app.world().resource::<BasicAttackState>().order.is_none());
    // Protection drops: the selection is an ordinary target again.
    app.world_mut()
        .get_mut::<crate::net::NetworkStructureProtected>(tower)
        .unwrap()
        .0 = false;
    app.update();
    assert_eq!(
        app.world().resource::<TargetState>().selected_entity,
        Some(tower)
    );
    // Destroyed: cleared like any dead target.
    app.world_mut().get_mut::<CombatStats>(tower).unwrap().hp = 0.0;
    app.update();
    assert!(
        app.world()
            .resource::<TargetState>()
            .selected_entity
            .is_none()
    );
}

/// Skill descriptions require a deliberate hold, never hover or attack spam.
#[test]
fn skill_descriptions_require_continuous_hold_and_hide_on_release() {
    use super::skill_card::SkillCardView;
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<TeamSelection>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<TargetState>()
        .init_resource::<PendingCast>()
        .init_resource::<GameplayInputContext>()
        .init_resource::<crate::gamepad::GamepadControls>()
        .init_resource::<crate::mobile_controls::MobileControls>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<super::inspection::SkillInspection>()
        .add_systems(Startup, setup_combat_ui)
        .add_systems(
            Update,
            (
                super::inspection::update_inspection,
                update_skill_tooltip,
                super::standard::update_status,
            )
                .chain(),
        );
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    let status = app
        .world_mut()
        .spawn((
            super::standard::StandardStatus,
            Text::default(),
            Node::default(),
        ))
        .id();
    app.world_mut().spawn((
        Player,
        Transform::default(),
        CombatStats::default(),
        PlayerProgression {
            level: 4,
            ranks: [1, 1, 2, 0],
            ..default()
        },
        NetworkHeroClass(HeroClass::Dawnweaver),
        crate::net::PlayerLoadout(Some(shared::loadout::LoadoutState {
            recipe: shared::loadout::preset_for_class(HeroClass::Dawnweaver).map(|l| l.recipe()),
            ..default()
        })),
    ));
    app.update();
    // Slot E (index 2) laid out at the redline: 648..712 × 610..674.
    let slot = app
        .world_mut()
        .query::<(Entity, &crate::ui::TestId)>()
        .iter(app.world())
        .find(|(_, id)| id.0 == "SkillSlot-E")
        .map(|(entity, _)| entity)
        .unwrap();
    app.world_mut().entity_mut(slot).insert((
        ComputedNode {
            size: Vec2::splat(64.0),
            inverse_scale_factor: 1.0,
            ..default()
        },
        UiGlobalTransform::from_translation(Vec2::new(680.0, 642.0)),
        Interaction::Hovered,
    ));
    let card = |app: &mut App| {
        app.world_mut()
            .query_filtered::<(&SkillCardView, &Node), With<super::hotbar::SkillTooltip>>()
            .single(app.world())
            .map(|(view, node)| (view.clone(), node.left, node.top))
            .unwrap()
    };
    app.update();
    assert!(!card(&mut app).0.visible, "not before the delay");
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs(2));
    app.update();
    assert!(!card(&mut app).0.visible, "hover never opens descriptions");
    assert_eq!(
        app.world().get::<Node>(status).unwrap().display,
        Display::None
    );
    app.world_mut()
        .entity_mut(slot)
        .insert(Interaction::Pressed);
    app.update();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(1499));
    app.update();
    assert!(!card(&mut app).0.visible);
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(2));
    app.update();
    assert_eq!(
        app.world().get::<Node>(status).unwrap().display,
        Display::Flex
    );
    let (view, left, top) = card(&mut app);
    assert!(view.visible);
    assert_eq!((view.slot, view.rank, view.locked), (2, 2, false));
    assert_eq!(view.key.as_deref(), Some("E"));
    assert!(view.status.is_some(), "live status on desktop");
    // Centred on the slot (680 − 140) and bottom at 720 − 124 − 8 = 588
    // without the upgrade chip (no skill point).
    assert_eq!(left, Val::Px(540.0));
    assert_eq!(top, Val::Px(588.0 - super::skill_card::CARD.y));
    // Input ownership changes restart the delay, even for the same skill.
    app.world_mut()
        .resource_mut::<crate::gamepad::GamepadControls>()
        .active = true;
    app.update();
    assert!(!card(&mut app).0.visible);
    app.world_mut()
        .resource_mut::<crate::gamepad::GamepadControls>()
        .active = false;
    app.update();
    assert!(!card(&mut app).0.visible);
    app.world_mut().entity_mut(slot).insert(Interaction::None);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyE);
    app.update();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(1501));
    app.update();
    assert!(
        card(&mut app).0.visible,
        "keyboard hold describes the held skill"
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyE);
    app.update();
    assert!(!card(&mut app).0.visible);
    assert_eq!(
        app.world().get::<Node>(status).unwrap().display,
        Display::None
    );
    {
        let mut pad = app
            .world_mut()
            .resource_mut::<crate::gamepad::GamepadControls>();
        pad.active = true;
        pad.attack_held = true;
    }
    app.update();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs(3));
    app.update();
    assert!(
        !card(&mut app).0.visible,
        "basic attack holds never inspect skills"
    );
    app.world_mut()
        .resource_mut::<crate::gamepad::GamepadControls>()
        .aiming_slot = Some(2);
    app.update();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(1501));
    app.update();
    assert!(card(&mut app).0.visible, "controller skill hold inspects");
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .modal_open = true;
    app.update();
    assert!(!card(&mut app).0.visible);
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .modal_open = false;
    app.update();
    assert!(
        !card(&mut app).0.visible,
        "menu time cannot count toward a hold"
    );
}

fn equip_recipe(app: &mut App, recipe: shared::loadout::BuildRecipe, level: u32, mana: f32) {
    let mut query = app.world_mut().query_filtered::<(
        &mut crate::net::PlayerLoadout,
        &mut PlayerProgression,
        &mut CombatStats,
    ), With<Player>>();
    let (mut state, mut progression, mut stats) = query.single_mut(app.world_mut()).unwrap();
    state.0.as_mut().unwrap().recipe = Some(recipe);
    progression.level = level;
    stats.mana = mana;
}

#[test]
fn mixed_permuted_cast_prechecks_use_equipped_mana_range_unlock_and_cooldown() {
    use shared::loadout::{CoreId, SkillId};
    let mut recipe = CoreId::Dawnweaver.preset();
    recipe.skills = [
        SkillId::DawnRay,
        SkillId::DawnBarrier,
        SkillId::DawnBind,
        SkillId::WildTraps,
    ];
    let mut locked = standard_cast_app(HeroClass::Dawnweaver, 0, false);
    equip_recipe(&mut locked, recipe.clone(), 5, 1000.0);
    locked.update();
    assert_eq!(
        locked
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0,
        "ultimate on Q remains locked until level six"
    );

    let mut app = standard_cast_app(HeroClass::Dawnweaver, 3, false);
    equip_recipe(&mut app, recipe.clone(), 4, 1000.0);
    app.update();
    let definition = shared::loadout::skill(SkillId::WildTraps);
    let commands: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect();
    assert!(
        matches!(commands.as_slice(), [NetworkCommand::CastSkill { slot: 3, aim }] if (aim.length() - definition.ability.cast_range).abs() < 0.001)
    );
    let skills =
        shared::loadout::EquippedSkills::resolve(HeroClass::Dawnweaver, Some(&recipe)).unwrap();
    assert_eq!(
        app.world().resource::<LocalCastCooldown>().total_secs[3],
        skills
            .cooldown(4, 1, SkillSlot::R, Default::default())
            .as_secs_f32()
    );

    let mut rejected = standard_cast_app(HeroClass::Dawnweaver, 3, false);
    equip_recipe(
        &mut rejected,
        recipe,
        10,
        shared::scaled_mana_cost(&definition.ability, 1) - 0.1,
    );
    rejected.update();
    assert_eq!(
        rejected
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
}

#[test]
fn malformed_equipped_recipe_cancels_cast_instead_of_substituting_preset() {
    let mut app = standard_cast_app(HeroClass::Dawnweaver, 0, false);
    let mut recipe = shared::loadout::CoreId::Dawnweaver.preset();
    recipe.skills[1] = recipe.skills[0];
    equip_recipe(&mut app, recipe, 10, 1000.0);
    app.update();
    assert!(!app.world().resource::<PendingCast>().is_pending());
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .count(),
        0
    );
}

#[test]
fn equipped_total_cooldown_keeps_authoritative_remaining_deadline() {
    use shared::loadout::{CoreId, LoadoutState, SkillId};
    let mut app = App::new();
    app.init_resource::<LocalCastCooldown>()
        .add_systems(Update, sync_authoritative_cooldown_durations);
    let mut recipe = CoreId::Dawnweaver.preset();
    recipe.skills.swap(0, 3);
    let equipped =
        shared::loadout::EquippedSkills::resolve(HeroClass::Dawnweaver, Some(&recipe)).unwrap();
    app.world_mut().spawn((
        Player,
        PlayerProgression {
            level: 10,
            ..default()
        },
        NetworkHeroClass(HeroClass::Dawnweaver),
        crate::net::PlayerEquipment::default(),
        crate::net::PlayerLoadout(Some(LoadoutState {
            recipe: Some(recipe),
            ..default()
        })),
        crate::net::PlayerSkillCooldowns {
            remaining_secs: [7.25, 1.0, 2.0, 3.0],
            recovery_secs: 0.2,
        },
    ));
    app.update();
    let cd = app.world().resource::<LocalCastCooldown>();
    assert_eq!(cd.remaining_secs, [7.25, 1.0, 2.0, 3.0]);
    assert_eq!(
        cd.total_secs[0],
        equipped
            .cooldown(10, 1, SkillSlot::Q, Default::default())
            .as_secs_f32()
    );
    assert_eq!(equipped.skill(SkillSlot::Q).unwrap().id, SkillId::DawnRay);
}

#[test]
fn borrowed_recast_requires_and_displays_its_own_mana_cost() {
    use shared::loadout::{CoreId, SkillId};
    for (mana, accepted) in [(24.0, false), (25.0, true)] {
        let mut app = standard_cast_app(HeroClass::Dawnweaver, 0, true);
        let mut recipe = CoreId::Dawnweaver.preset();
        recipe.skills[0] = SkillId::EchoStrike;
        let skills =
            shared::loadout::EquippedSkills::resolve(HeroClass::Dawnweaver, Some(&recipe)).unwrap();
        let card = super::skill_card::SkillCardView::of_equipped(
            &skills,
            &PlayerProgression {
                level: 10,
                ..default()
            },
            0,
            mana,
            0.0,
            true,
        );
        assert_eq!(card.mana, 25);
        assert_eq!(card.no_mana, !accepted);
        equip_recipe(&mut app, recipe, 10, mana);
        app.world_mut()
            .resource_mut::<LocalCastCooldown>()
            .recovery_secs = 0.5;
        app.update();
        let sent: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect();
        assert_eq!(sent.len(), usize::from(accepted), "mana {mana}: {sent:?}");
        if accepted {
            assert!(matches!(
                sent.as_slice(),
                [NetworkCommand::CastSkill { slot: 0, .. }]
            ));
        }
        assert_eq!(
            app.world().resource::<LocalCastCooldown>().remaining_secs[0],
            8.0,
            "follow-up does not reset its running base cooldown"
        );
    }
}

#[test]
fn rejected_skill_clears_prediction_after_grace_even_with_unchanged_zero_snapshot() {
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<LocalCastCooldown>()
        .add_systems(
            Update,
            (
                tick_local_cast_cooldown,
                sync_authoritative_cooldown_durations,
            )
                .chain(),
        );
    app.world_mut().spawn((
        Player,
        PlayerProgression::default(),
        NetworkHeroClass(HeroClass::Adventurer),
        crate::net::PlayerEquipment::default(),
        crate::net::PlayerSkillCooldowns::default(),
    ));
    app.update();
    {
        let mut cd = app.world_mut().resource_mut::<LocalCastCooldown>();
        cd.pending_slot = Some(3);
        cd.prediction_grace_secs = 0.3;
        cd.remaining_secs[3] = 22.0;
    }
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(100));
    app.update();
    assert!(app.world().resource::<LocalCastCooldown>().remaining_secs[3] > 20.0);
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(250));
    app.update();
    let cd = app.world().resource::<LocalCastCooldown>();
    assert_eq!(cd.remaining_secs[3], 0.0);
    assert!(cd.pending_slot.is_none());
}
