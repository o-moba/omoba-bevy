use super::*;
use shared::combat::CombatEntity;

fn local() -> LocalState {
    LocalState {
        id: 7,
        position: Vec3::ZERO,
        alive: true,
        level: 1,
        team: Team::Green,
    }
}

fn event(id: u64, style: ProjectileStyle) -> CombatEvent {
    CombatEvent {
        id,
        source: CombatEntity {
            kind: CombatEntityKind::Player,
            id: 7,
        },
        target: CombatEntity {
            kind: CombatEntityKind::Player,
            id: 8,
        },
        amount: 10.0,
        style,
        ..Default::default()
    }
}

fn has(cues: &[Candidate], cue: AudioCue) -> bool {
    cues.iter().any(|candidate| candidate.cue == cue)
}

#[test]
fn vital_break_cue_is_confirmed_once_and_never_a_kill_cue() {
    let mut cursor = EventCursor::default();
    cursor.accept((1, 1), &GameState::Running, Some(local()), &[]);
    let mut receipt = event(1, ProjectileStyle::Standard);
    receipt.near_lethal = true;
    let cues = cursor
        .accept(
            (1, 1),
            &GameState::Running,
            Some(local()),
            &[receipt.clone()],
        )
        .1;
    assert!(has(&cues, AudioCue::VitalBreak));
    assert!(!has(&cues, AudioCue::Kill));
    assert!(
        cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                &[receipt.clone()]
            )
            .1
            .is_empty()
    );
    receipt.id = 2;
    receipt.amount = 0.0;
    assert!(!has(
        &cursor
            .accept((1, 1), &GameState::Running, Some(local()), &[receipt])
            .1,
        AudioCue::VitalBreak
    ));
}

#[test]
fn packaged_manifest_has_every_safe_stable_cue() {
    let catalog = CueCatalog::parse(include_str!("../../../assets/audio/manifest.json")).unwrap();
    assert_eq!(catalog.cues.len(), AudioCue::ALL.len());
    for cue in AudioCue::ALL {
        assert!(catalog.cues.contains_key(cue.id()));
    }
    assert_eq!(catalog.music.path, "audio/music/arena.ogg");
}

#[test]
fn catalog_rejects_paths_outside_the_packaged_audio_namespace() {
    for path in [
        "audio/sfx/../secret.ogg",
        "audio/sfx/a/b.ogg",
        "https://host/test.ogg",
        "/audio/sfx/a.ogg",
        "audio/sfx/a.ogg?url=x",
        "audio/sfx/a.wav",
        "audio/sfx/.ogg",
        "audio/sfx/a\\b.ogg",
    ] {
        assert!(
            validate_asset(
                &CueAsset {
                    path: path.into(),
                    gain: 1.0
                },
                "audio/sfx/"
            )
            .is_err(),
            "{path}"
        );
    }
    for gain in [f32::NAN, f32::INFINITY, -1.0, 1.1] {
        assert!(
            validate_asset(
                &CueAsset {
                    path: "audio/sfx/test.ogg".into(),
                    gain
                },
                "audio/sfx/"
            )
            .is_err()
        );
    }
}

#[test]
fn catalog_rejects_unknown_versions_and_incomplete_or_unknown_cues() {
    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("../../../assets/audio/manifest.json")).unwrap();
    json["version"] = 2.into();
    assert!(CueCatalog::parse(&json.to_string()).is_err());
    json["version"] = 1.into();
    let value = json["cues"]
        .as_object_mut()
        .unwrap()
        .remove("melee")
        .unwrap();
    assert!(CueCatalog::parse(&json.to_string()).is_err());
    json["cues"]["future_cue"] = value;
    assert!(CueCatalog::parse(&json.to_string()).is_err());
}

#[test]
fn repeated_snapshots_and_duplicate_receipts_play_once_and_baseline_reconnects() {
    let mut cursor = EventCursor::default();
    let first = event(1, ProjectileStyle::Arrow);
    assert!(
        cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                std::slice::from_ref(&first)
            )
            .1
            .is_empty()
    );
    let second = event(2, ProjectileStyle::Arrow);
    let cues = cursor
        .accept(
            (1, 1),
            &GameState::Running,
            Some(local()),
            &[first, second.clone(), second.clone()],
        )
        .1;
    assert_eq!(cues.len(), 1);
    assert!(has(&cues, AudioCue::Arrow));
    assert!(
        cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                std::slice::from_ref(&second)
            )
            .1
            .is_empty()
    );
    cursor.accept((1, 1), &GameState::Running, None, &[]);
    assert!(
        cursor
            .accept((1, 1), &GameState::Running, Some(local()), &[second])
            .1
            .is_empty()
    );
    let mut replacement = local();
    replacement.id = 99;
    assert!(
        cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(replacement),
                &[event(3, ProjectileStyle::Arcane)]
            )
            .1
            .is_empty()
    );
}

#[test]
fn every_authoritative_style_selects_its_palette_cue() {
    let mut cursor = EventCursor::default();
    cursor.accept((1, 1), &GameState::Running, Some(local()), &[]);
    for (index, (style, expected)) in [
        (ProjectileStyle::Standard, AudioCue::Melee),
        (ProjectileStyle::Crescent, AudioCue::Melee),
        (ProjectileStyle::Arrow, AudioCue::Arrow),
        (ProjectileStyle::Arcane, AudioCue::Arcane),
        (ProjectileStyle::Holy, AudioCue::Holy),
        (ProjectileStyle::CasterBolt, AudioCue::Caster),
        (ProjectileStyle::TowerBolt, AudioCue::Tower),
    ]
    .into_iter()
    .enumerate()
    {
        let cues = cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                &[event(index as u64 + 1, style)],
            )
            .1;
        assert_eq!(cues.len(), 1);
        assert!(has(&cues, expected));
    }
}

#[test]
fn invalid_damage_and_far_away_hits_are_consumed_without_late_replay() {
    let mut cursor = EventCursor::default();
    cursor.accept((1, 1), &GameState::Running, Some(local()), &[]);
    let mut hit = event(1, ProjectileStyle::Arrow);
    hit.x = 100.0;
    assert!(
        cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                std::slice::from_ref(&hit)
            )
            .1
            .is_empty()
    );
    hit.x = 0.0;
    assert!(
        cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                std::slice::from_ref(&hit)
            )
            .1
            .is_empty()
    );
    for (index, amount) in [0.0, -1.0, f32::NAN, f32::INFINITY].into_iter().enumerate() {
        hit.id = index as u64 + 2;
        hit.amount = amount;
        assert!(
            cursor
                .accept(
                    (1, 1),
                    &GameState::Running,
                    Some(local()),
                    std::slice::from_ref(&hit)
                )
                .1
                .is_empty()
        );
    }
    hit.id = 99;
    hit.amount = 10.0;
    hit.target.kind = CombatEntityKind::Unknown;
    assert!(
        cursor
            .accept((1, 1), &GameState::Running, Some(local()), &[hit])
            .1
            .is_empty()
    );
}

#[test]
fn damage_position_uses_simulation_ground_plane_in_both_visual_modes() {
    assert_eq!(distance_gain(Vec3::ZERO, Vec3::new(0.0, 100.0, 0.0)), 1.0);
    assert_eq!(distance_gain(Vec3::ZERO, Vec3::new(36.0, 0.0, 0.0)), 0.0);
    assert_eq!(distance_gain(Vec3::ZERO, Vec3::new(0.0, 0.0, 36.0)), 0.0);
    assert!((distance_gain(Vec3::ZERO, Vec3::new(21.0, 0.0, 0.0)) - 0.25).abs() < 0.001);
    assert_eq!(distance_gain(Vec3::splat(f32::NAN), Vec3::ZERO), 0.0);
}

#[test]
fn death_receipt_and_health_transition_coalesce_then_respawn_and_level_emit_once() {
    let mut cursor = EventCursor::default();
    cursor.accept((1, 1), &GameState::Running, Some(local()), &[]);
    let mut killed = event(1, ProjectileStyle::CasterBolt);
    killed.source = CombatEntity {
        kind: CombatEntityKind::Minion,
        id: 7,
    };
    killed.target.id = 7;
    killed.killed = true;
    let dead = LocalState {
        alive: false,
        ..local()
    };
    let cues = cursor
        .accept(
            (1, 1),
            &GameState::Running,
            Some(dead),
            std::slice::from_ref(&killed),
        )
        .1;
    assert_eq!(
        cues.iter().filter(|cue| cue.cue == AudioCue::Death).count(),
        1
    );
    assert!(!has(&cues, AudioCue::Kill));
    assert!(
        cursor
            .accept((1, 1), &GameState::Running, Some(dead), &[killed])
            .1
            .is_empty()
    );
    let leveled = LocalState {
        level: 3,
        ..local()
    };
    let cues = cursor
        .accept((1, 1), &GameState::Running, Some(leveled), &[])
        .1;
    assert!(has(&cues, AudioCue::Respawn));
    assert!(has(&cues, AudioCue::LevelUp));
    assert!(
        cursor
            .accept((1, 1), &GameState::Running, Some(leveled), &[])
            .1
            .is_empty()
    );
}

#[test]
fn kill_confirmation_requires_typed_local_player_and_player_victim() {
    let mut cursor = EventCursor::default();
    cursor.accept((1, 1), &GameState::Running, Some(local()), &[]);
    let mut killed = event(1, ProjectileStyle::Arcane);
    killed.killed = true;
    assert!(has(
        &cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                std::slice::from_ref(&killed)
            )
            .1,
        AudioCue::Kill
    ));
    killed.id = 2;
    killed.source.kind = CombatEntityKind::Minion;
    assert!(!has(
        &cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                std::slice::from_ref(&killed)
            )
            .1,
        AudioCue::Kill
    ));
    killed.id = 3;
    killed.source.kind = CombatEntityKind::Player;
    killed.target.kind = CombatEntityKind::Neutral;
    assert!(!has(
        &cursor
            .accept((1, 1), &GameState::Running, Some(local()), &[killed])
            .1,
        AudioCue::Kill
    ));
}

#[test]
fn match_transitions_announce_once_without_new_epoch_or_round_history_replay() {
    let mut cursor = EventCursor::default();
    cursor.accept(
        (1, 1),
        &GameState::Starting { countdown_ms: 100 },
        None,
        &[],
    );
    assert!(has(
        &cursor
            .accept((1, 1), &GameState::Running, Some(local()), &[])
            .1,
        AudioCue::MatchStart
    ));
    let victory = GameState::Victory {
        winner: Team::Green.into(),
    };
    assert!(has(
        &cursor.accept((1, 1), &victory, Some(local()), &[]).1,
        AudioCue::Victory
    ));
    assert!(
        cursor
            .accept((1, 1), &victory, Some(local()), &[])
            .1
            .is_empty()
    );
    let cues = cursor
        .accept(
            (1, 2),
            &GameState::Running,
            Some(local()),
            &[event(1, ProjectileStyle::Arrow)],
        )
        .1;
    assert_eq!(cues.len(), 1);
    assert!(has(&cues, AudioCue::MatchStart));
    assert!(
        cursor
            .accept(
                (2, 1),
                &GameState::Running,
                Some(local()),
                &[event(1, ProjectileStyle::Arrow)]
            )
            .1
            .is_empty()
    );
    assert!(has(
        &cursor
            .accept(
                (2, 1),
                &GameState::Victory {
                    winner: Team::Blue.into()
                },
                Some(local()),
                &[]
            )
            .1,
        AudioCue::Defeat
    ));
}

#[test]
fn rate_budget_bounds_bursts_per_cue_per_frame_and_total_voices() {
    let mut budget = RateBudget::default();
    assert!(budget.allow(AudioCue::Arrow, 1.0, 0, 0));
    assert!(!budget.allow(AudioCue::Arrow, 1.01, 1, 1));
    for (frame, cue) in [AudioCue::Arcane, AudioCue::Holy, AudioCue::Melee]
        .into_iter()
        .enumerate()
    {
        assert!(budget.allow(cue, 1.01, frame + 1, frame + 1));
    }
    assert!(!budget.allow(AudioCue::Tower, 1.02, 4, 0));
    assert!(!budget.allow(AudioCue::Tower, 2.0, 0, MAX_FRAME_CUES));
    assert!(!budget.allow(AudioCue::Tower, 2.0, MAX_VOICES, 0));
    assert!(budget.allow(AudioCue::Tower, 2.0, 0, 0));
    assert!(budget.allow(AudioCue::Arrow, 2.0, 1, 1));
}

#[test]
fn pending_sources_without_device_have_short_deadline_and_effects_absolute_limit() {
    assert!(!expired(0.29, false));
    assert!(expired(0.3, false));
    assert!(!expired(0.3, true));
    assert!(!expired(3.99, true));
    assert!(expired(4.0, true));
}

#[test]
fn trap_receipt_plays_once_only_after_confirmed_trigger() {
    let mut cursor = EventCursor::default();
    cursor.accept((1, 1), &GameState::Running, Some(local()), &[]);
    let mut trigger = event(1, ProjectileStyle::Arcane);
    trigger.trap_triggered = true;
    assert!(has(
        &cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                &[trigger.clone()]
            )
            .1,
        AudioCue::TrapTrigger
    ));
    assert!(!has(
        &cursor
            .accept((1, 1), &GameState::Running, Some(local()), &[trigger])
            .1,
        AudioCue::TrapTrigger
    ));
    assert!(!has(
        &cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                &[event(2, ProjectileStyle::Arcane)]
            )
            .1,
        AudioCue::TrapTrigger
    ));
}

#[test]
fn shielded_trap_is_audible_once_without_inventing_damage_or_remote_hits() {
    let mut cursor = EventCursor::default();
    cursor.accept((1, 1), &GameState::Running, Some(local()), &[]);
    let mut trigger = event(1, ProjectileStyle::Arcane);
    trigger.trap_triggered = true;
    trigger.amount = 0.0;
    let cues = cursor
        .accept(
            (1, 1),
            &GameState::Running,
            Some(local()),
            &[trigger.clone()],
        )
        .1;
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].cue, AudioCue::TrapTrigger);
    assert!(
        cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                &[trigger.clone()]
            )
            .1
            .is_empty()
    );
    trigger.id = 2;
    trigger.trap_triggered = false;
    assert!(
        cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                &[trigger.clone()]
            )
            .1
            .is_empty()
    );
    trigger.id = 3;
    trigger.trap_triggered = true;
    trigger.source.id = 9;
    trigger.x = 100.0;
    assert!(
        cursor
            .accept(
                (1, 1),
                &GameState::Running,
                Some(local()),
                &[trigger.clone()]
            )
            .1
            .is_empty()
    );
    trigger.id = 4;
    trigger.x = 0.0;
    trigger.amount = -1.0;
    assert!(
        cursor
            .accept((1, 1), &GameState::Running, Some(local()), &[trigger])
            .1
            .is_empty()
    );
}

#[test]
fn confirmed_enemy_attacks_sound_without_hp_receipts_and_never_replay() {
    let mut cursor = AttackCursor::default();
    let mut enemy = AttackObservation {
        id: 8,
        sequence: 5,
        attacking: true,
        visible: true,
        alive: true,
        team: Team::Blue,
        position: Vec3::X,
        style: ProjectileStyle::Crescent,
    };
    assert!(
        cursor
            .accept((1, 2), true, Some(local()), [enemy])
            .is_empty()
    );
    enemy.sequence += 1;
    assert!(has(
        &cursor.accept((1, 2), true, Some(local()), [enemy]),
        AudioCue::Melee
    ));
    assert!(
        cursor
            .accept((1, 2), true, Some(local()), [enemy])
            .is_empty()
    );
    enemy.visible = false;
    enemy.sequence += 1;
    assert!(
        cursor
            .accept((1, 2), true, Some(local()), [enemy])
            .is_empty()
    );
    enemy.visible = true;
    assert!(
        cursor
            .accept((1, 2), true, Some(local()), [enemy])
            .is_empty()
    );
    enemy.sequence += 1;
    assert!(
        cursor
            .accept((1, 3), true, Some(local()), [enemy])
            .is_empty()
    );
    enemy.sequence += 1;
    assert!(
        cursor
            .accept((1, 3), false, Some(local()), [enemy])
            .is_empty()
    );
}

// Voices from the rows of the skill registry.

use shared::loadout::{EffectVisualKind, SkillEffectState, SkillId};

const ENEMY: u64 = 8;
const BASIC: u8 = shared::BASIC_ATTACK_ACTION_SLOT;

fn target() -> SkillPresentation {
    SkillPresentation::target()
}

/// The slot the default kit of `class` binds the row `id` to.
fn slot_of(class: HeroClass, id: &str) -> u8 {
    (0..4)
        .find(|slot| CastKey::of(class, None, *slot).is_some_and(|key| row_id(key) == id))
        .unwrap_or_else(|| panic!("{id} is not in the kit of {}", class.id()))
}

fn heard_hero(id: u64, class: HeroClass, slot: u8) -> HeroHeard<'static> {
    HeroHeard {
        id,
        visible: true,
        class,
        loadout: None,
        slot,
    }
}

/// An accepted action of `actor` on `slot`, observed where the listener stands.
fn observed(actor: u64, class: HeroClass, slot: u8, sequence: u64) -> SkillCastObserved {
    SkillCastObserved {
        actor_id: actor,
        key: CastKey::of(class, None, slot).unwrap(),
        slot,
        sequence,
        recast: false,
        origin: Vec3::ZERO,
        position: Vec3::ZERO,
        yaw: None,
        forward: Vec3::NEG_Z,
        local: actor == local().id,
    }
}

/// A receipt of damage dealt by hero `source` with the action on `slot`.
fn dealt(id: u64, source: u64, slot: Option<u8>) -> CombatEvent {
    CombatEvent {
        source: CombatEntity {
            kind: CombatEntityKind::Player,
            id: source,
        },
        action_slot: slot,
        ..event(id, ProjectileStyle::Crescent)
    }
}

/// The candidates the receipt cursor gives for `events` after its baseline.
fn receipts(events: &[CombatEvent]) -> Vec<Candidate> {
    let mut cursor = EventCursor::default();
    cursor.accept((1, 1), &GameState::Running, Some(local()), &[]);
    cursor
        .accept((1, 1), &GameState::Running, Some(local()), events)
        .1
}

fn voiced(
    registry: &SkillPresentation,
    heroes: &[HeroHeard],
    casts: &[SkillCastObserved],
    stages: &[StageEvent],
    mut candidates: Vec<Candidate>,
) -> Vec<Candidate> {
    voice_rows(
        &Heard {
            registry,
            listener: local(),
            heroes,
            casts,
            stages,
        },
        &mut candidates,
    );
    candidates
}

/// The row and the moment a candidate was voiced for.
fn row_of(candidate: &Candidate) -> Option<(Moment, &'static str)> {
    match candidate.origin {
        Origin::Row { moment, row, .. } => Some((moment, row)),
        _ => None,
    }
}

fn played(cue: AudioCue, speed: f32, slice: AudioSlice) -> Variant {
    Variant {
        cue,
        step: (speed * 20.0).round() as u8,
        slice,
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn a_resolved_receipt_replaces_the_style_cue_and_an_unresolved_one_keeps_it() {
    let registry = target();
    let me = local().id;
    let bash = slot_of(HeroClass::Warrior, "shield_bash");
    let warrior = [heard_hero(me, HeroClass::Warrior, bash)];
    let styled = receipts(&[dealt(1, me, Some(bash))]);
    assert_eq!(styled.len(), 1);
    assert_eq!(styled[0].variant(), Variant::from(AudioCue::Melee));
    assert!(matches!(styled[0].origin, Origin::Receipt { id: 1, .. }));

    // The row of the action that dealt the receipt gives the hit its voice.
    let hit = voiced(&registry, &warrior, &[], &[], styled.clone());
    assert_eq!(hit.len(), 1);
    assert_eq!(
        hit[0].variant(),
        played(AudioCue::Melee, 0.75, AudioSlice::Tail)
    );
    assert!(close(hit[0].gain, 1.0));
    assert_eq!(
        hit[0].origin,
        Origin::Row {
            moment: Moment::Impact,
            row: "shield_bash",
            actor: me,
            id: 1
        }
    );
    // A basic attack is voiced by the cue of its class row, on the confirmed hit.
    let basic = voiced(
        &registry,
        &warrior,
        &[],
        &[],
        receipts(&[dealt(2, me, Some(BASIC))]),
    );
    assert_eq!(basic.len(), 1);
    assert_eq!(
        basic[0].variant(),
        played(AudioCue::Melee, 0.85, AudioSlice::Tick)
    );
    assert!(close(basic[0].gain, 0.55));
    assert_eq!(row_of(&basic[0]), Some((Moment::Impact, "warrior")));
    // The voice is heard as far away as the receipt is, at the gain of its row.
    let mut far = dealt(3, me, Some(BASIC));
    far.x = 21.0;
    let far = voiced(&registry, &warrior, &[], &[], receipts(&[far]));
    assert!(close(far[0].gain, 0.25 * 0.55));

    // Whatever no row answers for keeps the cue of the wire style, untouched.
    let kept = |registry: &SkillPresentation, heroes: &[HeroHeard], receipt: CombatEvent| {
        let styled = receipts(&[receipt]);
        let after = voiced(registry, heroes, &[], &[], styled.clone());
        assert_eq!(after.len(), 1);
        after[0].variant() == styled[0].variant()
            && after[0].gain == styled[0].gain
            && after[0].origin == styled[0].origin
            && after[0].notes == [None; MAX_NOTES]
            && after[0].detune() == 1.0
    };
    let packaged = SkillPresentation::packaged();
    assert!(
        kept(&packaged, &warrior, dealt(4, me, Some(bash))),
        "shipped"
    );
    assert!(
        kept(&packaged, &warrior, dealt(4, me, Some(BASIC))),
        "shipped"
    );
    let hidden = [HeroHeard {
        visible: false,
        ..warrior[0]
    }];
    assert!(kept(&registry, &hidden, dealt(5, me, Some(bash))), "hidden");
    assert!(kept(&registry, &[], dealt(6, me, Some(bash))), "unknown");
    let mut minion = dealt(7, me, Some(bash));
    minion.source.kind = CombatEntityKind::Minion;
    assert!(kept(&registry, &warrior, minion), "not a hero");
    assert!(kept(&registry, &warrior, dealt(8, me, None)), "no slot");
    assert!(kept(&registry, &warrior, dealt(9, me, Some(9))), "bad slot");
    // Battle Rally cannot deal damage, so its row has no hit voice.
    let rally = slot_of(HeroClass::Warrior, "battle_rally");
    assert!(
        kept(&registry, &warrior, dealt(10, me, Some(rally))),
        "no cue"
    );
    // The other cues of a receipt are never revoiced.
    let mut lethal = dealt(11, me, Some(bash));
    lethal.near_lethal = true;
    lethal.killed = false;
    let cues = voiced(&registry, &warrior, &[], &[], receipts(&[lethal]));
    assert!(has(&cues, AudioCue::VitalBreak));
    assert_eq!(cues.iter().filter(|cue| row_of(cue).is_some()).count(), 1);
}

#[test]
fn the_cooldown_is_per_variant() {
    let arrow = |speed, slice| Admission {
        variant: played(AudioCue::Arrow, speed, slice),
        notes: 1,
    };
    let mut budget = RateBudget::default();
    assert!(budget.allow(AudioCue::Arrow, 1.0, 0, 0));
    // The sample as recorded is one variant, however it is asked for.
    assert!(!budget.allow(AudioCue::Arrow, 1.01, 0, 0));
    assert!(!budget.allow(arrow(1.0, AudioSlice::Full), 1.01, 0, 0));
    // Another speed or another slice of the same sample has a cooldown of its own.
    assert!(budget.allow(arrow(1.1, AudioSlice::Full), 1.01, 0, 0));
    assert!(!budget.allow(arrow(1.1, AudioSlice::Full), 1.02, 0, 0));
    assert!(budget.allow(arrow(1.1, AudioSlice::Tick), 1.02, 0, 0));
    assert!(!budget.allow(arrow(1.1, AudioSlice::Tick), 1.03, 0, 0));
    // Each of them is free again when its own time is over.
    assert!(!budget.allow(AudioCue::Arrow, 1.119, 0, 0));
    assert!(budget.allow(AudioCue::Arrow, 1.12, 0, 0));
    assert!(!budget.allow(arrow(1.1, AudioSlice::Full), 1.12, 0, 0));
    assert!(budget.allow(arrow(1.1, AudioSlice::Full), 1.5, 0, 0));
    // The length of the cooldown is that of the sample: 0.2 s for the caster bolt.
    let caster = |speed| Admission {
        variant: played(AudioCue::Caster, speed, AudioSlice::Body),
        notes: 1,
    };
    let mut budget = RateBudget::default();
    assert!(budget.allow(caster(0.7), 5.0, 0, 0));
    assert!(budget.allow(caster(1.4), 5.0, 0, 1));
    assert!(!budget.allow(caster(0.7), 5.15, 0, 0));
    assert!(budget.allow(caster(0.7), 5.2, 0, 0));
}

#[test]
fn notes_are_admitted_together_or_not_at_all() {
    let voice = |speed, notes| Admission {
        variant: played(AudioCue::Melee, speed, AudioSlice::Body),
        notes,
    };
    // Tokens: three notes take three of the four, and two more do not fit. The refused
    // voice took nothing, so one note still does.
    let mut budget = RateBudget::default();
    assert!(budget.allow(voice(0.7, 3), 1.0, 0, 0));
    assert!(!budget.allow(voice(0.8, 2), 1.0, 0, 0));
    assert!(budget.allow(voice(0.9, 1), 1.0, 0, 0));
    assert!(!budget.allow(voice(1.0, 1), 1.0, 0, 0));
    // A refused voice starts no cooldown either: it is admitted as soon as it fits.
    assert!(!budget.allow(voice(0.8, 2), 1.1, 0, 0));
    assert!(budget.allow(voice(0.8, 2), 1.25, 0, 0));
    // The frame limit counts every note in the frame that asks.
    let mut budget = RateBudget::default();
    assert!(!budget.allow(voice(0.7, 3), 1.0, 0, MAX_FRAME_CUES - 2));
    assert!(budget.allow(voice(0.7, 3), 1.0, 0, MAX_FRAME_CUES - 3));
    // So does the voice limit, which also holds the notes that still wait.
    let mut budget = RateBudget::default();
    assert!(!budget.allow(voice(0.7, 3), 1.0, MAX_VOICES - 2, 0));
    assert!(budget.allow(voice(0.7, 3), 1.0, MAX_VOICES - 3, 0));

    // The local hero's Rampage asks for its three notes at once.
    let rampage = slot_of(HeroClass::Warrior, "rampage");
    let cast = voiced(
        &target(),
        &[],
        &[observed(local().id, HeroClass::Warrior, rampage, 4)],
        &[],
        Vec::new(),
    );
    assert_eq!(cast.len(), 1);
    assert_eq!(
        cast[0].admission(),
        Admission {
            variant: played(AudioCue::Melee, 0.7, AudioSlice::Body),
            notes: 3
        }
    );
    let mut budget = RateBudget::default();
    assert!(budget.allow(AudioCue::Kill, 1.0, 0, 0));
    assert!(budget.allow(AudioCue::Hit, 1.0, 1, 1));
    assert!(!budget.allow(cast[0].admission(), 1.0, 2, 2));
}

#[test]
fn non_local_casts_play_one_note() {
    let registry = target();
    let rampage = slot_of(HeroClass::Warrior, "rampage");
    let own = voiced(
        &registry,
        &[],
        &[observed(local().id, HeroClass::Warrior, rampage, 4)],
        &[],
        Vec::new(),
    );
    assert_eq!(own.len(), 1);
    assert_eq!(row_of(&own[0]), Some((Moment::Cast, "rampage")));
    assert_eq!(
        own[0].notes,
        [
            Some(Note {
                delay_secs: 0.09,
                step: 16,
                gain: 0.8
            }),
            Some(Note {
                delay_secs: 0.18,
                step: 18,
                gain: 0.9
            })
        ]
    );
    // Another hero's Rampage is the first note alone, however close it is.
    let other = voiced(
        &registry,
        &[],
        &[observed(ENEMY, HeroClass::Warrior, rampage, 4)],
        &[],
        Vec::new(),
    );
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].variant(), own[0].variant());
    assert_eq!(other[0].gain, own[0].gain);
    assert_eq!(other[0].notes, [None; MAX_NOTES]);
    assert_eq!(other[0].admission().notes, 1);

    // The same holds for the voice of a hit and of a release.
    let longshot = slot_of(HeroClass::Ranger, "longshot");
    for (source, notes) in [(local().id, 1), (ENEMY, 0)] {
        let hit = voiced(
            &registry,
            &[heard_hero(source, HeroClass::Ranger, longshot)],
            &[],
            &[],
            receipts(&[dealt(1, source, Some(longshot))]),
        );
        assert_eq!(row_of(&hit[0]), Some((Moment::Impact, "longshot")));
        assert_eq!(hit[0].notes.iter().flatten().count(), notes, "{source}");
    }
    // A later note is heard as far away as the first one.
    let mut far = dealt(2, local().id, Some(longshot));
    far.x = 21.0;
    let hit = voiced(
        &registry,
        &[heard_hero(local().id, HeroClass::Ranger, longshot)],
        &[],
        &[],
        receipts(&[far]),
    );
    assert!(close(hit[0].gain, 0.25));
    assert_eq!(
        hit[0].notes[0],
        Some(Note {
            delay_secs: 0.08,
            step: 20,
            gain: 0.25 * 0.5
        })
    );
    for (own, notes) in [(true, 2), (false, 0)] {
        let fired = voiced(
            &registry,
            &[],
            &[],
            &[stage(
                SkillId::DawnRay,
                EffectVisualKind::Beam,
                StageChange::Transition(Transition::KindFlipped),
                Some(own),
            )],
            Vec::new(),
        );
        assert_eq!(row_of(&fired[0]), Some((Moment::Release, "dawn_ray")));
        assert_eq!(fired[0].notes.iter().flatten().count(), notes, "{own}");
    }
}

#[test]
fn the_68_target_cast_voices_are_distinct() {
    let registry = target();
    let mut voices = Vec::new();
    let mut rows = Vec::new();
    for class in HeroClass::ALL {
        for slot in 0..4 {
            let cast = voiced(
                &registry,
                &[],
                &[observed(local().id, class, slot, 9)],
                &[],
                Vec::new(),
            );
            assert_eq!(cast.len(), 1, "{} {slot}", class.id());
            let (moment, row) = row_of(&cast[0]).unwrap();
            assert_eq!(moment, Moment::Cast, "{row}");
            assert!(
                (14..=28).contains(&cast[0].step) && close(cast[0].detune(), detune(9)),
                "{row}"
            );
            voices.push((cast[0].variant(), row));
            rows.push(row);
        }
    }
    rows.sort_unstable();
    rows.dedup();
    assert_eq!(rows.len(), 68);
    // No two skills share a sample, speed and slice, so each has a cooldown of its own.
    voices.sort_unstable();
    for pair in voices.windows(2) {
        assert_ne!(pair[0].0, pair[1].0, "{} and {}", pair[0].1, pair[1].1);
    }
    // The Bluff sample is the voice of Bluff alone.
    let bluff: Vec<_> = voices
        .iter()
        .filter(|(variant, _)| variant.cue == AudioCue::Bluff)
        .collect();
    assert_eq!(
        bluff,
        [&(
            played(AudioCue::Bluff, 1.0, AudioSlice::Full),
            "dagger_bluff"
        )]
    );
    // A skill voice is built from the six general samples and that one.
    assert!(voices.iter().all(|(variant, _)| matches!(
        variant.cue,
        AudioCue::Melee
            | AudioCue::Arrow
            | AudioCue::Arcane
            | AudioCue::Holy
            | AudioCue::Caster
            | AudioCue::Tower
            | AudioCue::Bluff
    )));
    // The packaged rows name no voice yet: nothing is added to what plays today.
    let packaged = SkillPresentation::packaged();
    for class in HeroClass::ALL {
        for slot in [0, 1, 2, 3, BASIC] {
            let casts = [observed(local().id, class, slot, 9)];
            assert!(
                voiced(&packaged, &[], &casts, &[], Vec::new()).is_empty(),
                "{} {slot}",
                class.id()
            );
        }
    }
}

#[test]
fn a_cast_takes_the_voice_of_its_row_for_that_edge() {
    let registry = target();
    let me = local().id;
    let field = slot_of(HeroClass::Dawnweaver, "dawn_field");
    let first = observed(me, HeroClass::Dawnweaver, field, 5);
    let cast = voiced(
        &registry,
        &[],
        std::slice::from_ref(&first),
        &[],
        Vec::new(),
    );
    assert_eq!(row_of(&cast[0]), Some((Moment::Cast, "dawn_field")));
    assert_eq!(
        cast[0].variant(),
        played(AudioCue::Holy, 1.2, AudioSlice::Full)
    );
    assert!(close(cast[0].gain, 0.8));
    // The recast edge has the recast voice of the row.
    let again = SkillCastObserved {
        recast: true,
        sequence: 6,
        ..first.clone()
    };
    let recast = voiced(
        &registry,
        &[],
        std::slice::from_ref(&again),
        &[],
        Vec::new(),
    );
    assert_eq!(row_of(&recast[0]), Some((Moment::Recast, "dawn_field")));
    assert_eq!(
        recast[0].variant(),
        played(AudioCue::Holy, 0.8, AudioSlice::Body)
    );
    assert_eq!(
        recast[0].origin,
        Origin::Row {
            moment: Moment::Recast,
            row: "dawn_field",
            actor: me,
            id: 6
        }
    );
    // A row without one is still heard when it is cast again.
    let plain = SkillPresentation::target_with(|config| {
        config["skills"]["dawn_field"]["sound"]
            .as_object_mut()
            .unwrap()
            .remove("recast");
    });
    let recast = voiced(&plain, &[], &[again], &[], Vec::new());
    assert_eq!(row_of(&recast[0]), Some((Moment::Cast, "dawn_field")));
    assert_eq!(recast[0].variant(), cast[0].variant());

    // A basic attack has no cast voice: its row is heard on the confirmed hit.
    let swing = observed(me, HeroClass::Dawnweaver, BASIC, 7);
    assert!(voiced(&registry, &[], &[swing], &[], Vec::new()).is_empty());

    // Another hero's cast is heard by its distance, and not at all beyond earshot.
    let mut near = observed(ENEMY, HeroClass::Dawnweaver, field, 5);
    near.position = Vec3::new(21.0, 0.0, 0.0);
    let heard = voiced(&registry, &[], &[near.clone()], &[], Vec::new());
    assert!(close(heard[0].gain, 0.25 * 0.8));
    near.position = Vec3::new(36.0, 0.0, 0.0);
    assert!(voiced(&registry, &[], &[near.clone()], &[], Vec::new()).is_empty());
    // The local hero's own cast is always heard in full and asks the budget first.
    let mut own = first;
    own.position = Vec3::new(36.0, 0.0, 0.0);
    near.position = Vec3::ZERO;
    let both = voiced(&registry, &[], &[near, own], &[], Vec::new());
    assert_eq!(both.len(), 2);
    assert!(matches!(both[0].origin, Origin::Row { actor, .. } if actor == me));
    assert!(close(both[0].gain, 0.8));

    // The Bluff sample comes from the row of Bluff.
    let bluff = slot_of(HeroClass::Adventurer, "dagger_bluff");
    let cast = voiced(
        &registry,
        &[],
        &[observed(me, HeroClass::Adventurer, bluff, 3)],
        &[],
        Vec::new(),
    );
    assert_eq!(cast[0].variant(), Variant::from(AudioCue::Bluff));
    assert!(close(cast[0].gain, 0.9));
}

/// A stage event of an effect of `skill` at the listener. `owner` is whether the hero that
/// owns it is the local one; `None` is an effect whose owner the client does not see.
fn stage(
    skill: SkillId,
    kind: EffectVisualKind,
    change: StageChange,
    owner: Option<bool>,
) -> StageEvent {
    StageEvent {
        effect: SkillEffectState {
            id: 31,
            owner_id: match owner {
                Some(true) => local().id,
                Some(false) => ENEMY,
                None => 0,
            },
            owner_team: shared::map::Team::Green,
            skill,
            kind,
            position: [0.0, 0.0],
            end: [4.0, 0.0],
            radius: 1.0,
            remaining_secs: 0.3,
            armed: false,
            consumed_segments: 0,
        },
        change,
        owner: owner.map(|local| crate::skill_presentation::stage::OwnerSeen {
            visible: true,
            alive: true,
            parrying: Some(false),
            position: Vec3::ZERO,
            slot: Some(3),
            local,
        }),
    }
}

#[test]
fn a_telegraph_that_fired_plays_the_release_voice_of_its_row() {
    let registry = target();
    let flipped = StageChange::Transition(Transition::KindFlipped);
    let released = StageChange::Ended(EndKind::Released);
    let fired = |event: StageEvent| voiced(&registry, &[], &[], &[event], Vec::new());

    // A warning that became its beam.
    let ray = fired(stage(
        SkillId::DawnRay,
        EffectVisualKind::Beam,
        flipped,
        Some(true),
    ));
    assert_eq!(ray.len(), 1);
    assert_eq!(
        ray[0].variant(),
        played(AudioCue::Holy, 0.7, AudioSlice::Full)
    );
    assert!(close(ray[0].gain, 1.0));
    assert_eq!(
        ray[0].origin,
        Origin::Row {
            moment: Moment::Release,
            row: "dawn_ray",
            actor: local().id,
            id: 31
        }
    );
    // Its two later notes: one with the first note, one 40 ms on.
    assert_eq!(
        ray[0].notes.map(|note| note.unwrap().delay_secs),
        [0.0, 0.04]
    );
    // A warning that became the travelling wave, and a fuse that burned down.
    let wave = fired(stage(
        SkillId::HorizonWave,
        EffectVisualKind::Bolt,
        flipped,
        Some(false),
    ));
    assert_eq!(row_of(&wave[0]), Some((Moment::Release, "horizon_wave")));
    assert_eq!(
        wave[0].variant(),
        played(AudioCue::Arrow, 1.15, AudioSlice::Full)
    );
    let breath = fired(stage(
        SkillId::FurnaceBreath,
        EffectVisualKind::BeamWarning,
        released,
        Some(false),
    ));
    assert_eq!(
        row_of(&breath[0]),
        Some((Moment::Release, "furnace_breath"))
    );
    assert_eq!(
        breath[0].variant(),
        played(AudioCue::Tower, 0.75, AudioSlice::Full)
    );

    // Nothing else that happens to an effect is a release.
    for change in [
        StageChange::Transition(Transition::Armed),
        StageChange::Transition(Transition::Turned),
        StageChange::Transition(Transition::Renewed),
        StageChange::Transition(Transition::SegmentBroken(2)),
        StageChange::Ended(EndKind::TrueExpiry),
        StageChange::Ended(EndKind::Detonated),
    ] {
        let event = stage(
            SkillId::FurnaceBreath,
            EffectVisualKind::BeamWarning,
            change,
            Some(true),
        );
        assert!(fired(event).is_empty(), "{change:?}");
    }
    // A row without the voice plays none, and no packaged row has one.
    let field = stage(
        SkillId::DawnField,
        EffectVisualKind::Field,
        released,
        Some(true),
    );
    assert!(fired(field).is_empty());
    let ray = stage(
        SkillId::DawnRay,
        EffectVisualKind::Beam,
        flipped,
        Some(true),
    );
    assert!(voiced(&SkillPresentation::packaged(), &[], &[], &[ray], Vec::new()).is_empty());

    // The beam of a hero the client does not see sounds where it was received: by the
    // distance to the effect, with the first note alone.
    let mut unseen = stage(SkillId::DawnRay, EffectVisualKind::Beam, flipped, None);
    unseen.effect.position = [0.0, 21.0];
    let heard = fired(unseen.clone());
    assert!(close(heard[0].gain, 0.25));
    assert_eq!(heard[0].notes, [None; MAX_NOTES]);
    assert!(matches!(heard[0].origin, Origin::Row { actor: 0, .. }));
    unseen.effect.position = [0.0, 36.0];
    assert!(fired(unseen).is_empty());
}

#[test]
fn an_enemy_attack_has_one_voice() {
    let registry = target();
    let packaged = SkillPresentation::packaged();
    // The style cue of an accepted attack of enemy 8, action `sequence`.
    let attack = |sequence: u64| {
        let mut cursor = AttackCursor::default();
        let mut enemy = AttackObservation {
            id: ENEMY,
            sequence: sequence - 1,
            attacking: true,
            visible: true,
            alive: true,
            team: Team::Blue,
            position: Vec3::ZERO,
            style: ProjectileStyle::Crescent,
        };
        cursor.accept((1, 1), true, Some(local()), [enemy]);
        enemy.sequence = sequence;
        cursor.accept((1, 1), true, Some(local()), [enemy])
    };
    let styled = attack(6);
    assert_eq!(styled.len(), 1);
    assert_eq!(
        styled[0].origin,
        Origin::Attack {
            actor: ENEMY,
            sequence: 6
        }
    );

    // A basic attack sounds in the voice of its class row, and so does its hit: the two
    // share a cooldown, as the two style cues did.
    let swinging = [heard_hero(ENEMY, HeroClass::Warrior, BASIC)];
    let swing = voiced(&registry, &swinging, &[], &[], attack(6));
    assert_eq!(swing.len(), 1);
    assert_eq!(row_of(&swing[0]), Some((Moment::Attack, "warrior")));
    assert_eq!(
        swing[0].variant(),
        played(AudioCue::Melee, 0.85, AudioSlice::Tick)
    );
    assert!(close(swing[0].gain, 0.55));
    let hit = voiced(
        &registry,
        &swinging,
        &[],
        &[],
        receipts(&[dealt(1, ENEMY, Some(BASIC))]),
    );
    assert_eq!(hit[0].variant(), swing[0].variant());
    let mut budget = RateBudget::default();
    assert!(budget.allow(swing[0].admission(), 1.0, 0, 0));
    assert!(!budget.allow(hit[0].admission(), 1.05, 0, 0));

    // Later notes belong to the local hero: an enemy's attack never carries any.
    let chorded = SkillPresentation::target_with(|config| {
        config["basic_attacks"]["warrior"]["sound"]["notes"] =
            serde_json::json!([{ "delay_ms": 60, "speed": 1.0, "gain": 0.5 }]);
    });
    let swing = voiced(&chorded, &swinging, &[], &[], attack(6));
    assert_eq!(row_of(&swing[0]), Some((Moment::Attack, "warrior")));
    assert_eq!(swing[0].notes, [None; MAX_NOTES]);

    // A skill on the attack slot is heard through its cast voice alone.
    let bash = slot_of(HeroClass::Warrior, "shield_bash");
    let casting = [heard_hero(ENEMY, HeroClass::Warrior, bash)];
    let cast = observed(ENEMY, HeroClass::Warrior, bash, 6);
    let one = voiced(
        &registry,
        &casting,
        std::slice::from_ref(&cast),
        &[],
        attack(6),
    );
    assert_eq!(one.len(), 1);
    assert_eq!(row_of(&one[0]), Some((Moment::Cast, "shield_bash")));
    // The cast of another action, of another hero or on another slot says nothing about
    // this attack, and neither does a cast the client did not observe.
    let other_action = SkillCastObserved {
        sequence: 5,
        ..cast.clone()
    };
    let other_hero = SkillCastObserved {
        actor_id: 9,
        ..cast.clone()
    };
    let other_slot = observed(ENEMY, HeroClass::Warrior, bash + 1, 6);
    for casts in [
        vec![],
        vec![other_action],
        vec![other_hero],
        vec![other_slot],
    ] {
        let cues = voiced(&registry, &casting, &casts, &[], attack(6));
        let stand_in: Vec<_> = cues.iter().filter(|cue| row_of(cue).is_none()).collect();
        assert_eq!(stand_in.len(), 1);
        assert_eq!(stand_in[0].variant(), Variant::from(AudioCue::Melee));
        assert_eq!(stand_in[0].origin, styled[0].origin);
    }
    // With the packaged rows every attack keeps the cue of the class style.
    for heroes in [&swinging, &casting] {
        let cues = voiced(
            &packaged,
            heroes,
            std::slice::from_ref(&cast),
            &[],
            attack(6),
        );
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].variant(), Variant::from(AudioCue::Melee));
        assert_eq!(cues[0].origin, styled[0].origin);
    }
    // An attacker the frame does not hold keeps it as well.
    let cues = voiced(&registry, &[], &[], &[], attack(6));
    assert_eq!(cues[0].origin, styled[0].origin);
}

#[test]
fn a_voice_is_a_sample_a_speed_and_a_slice_detuned_by_three_percent_at_most() {
    // The four slices, in times of the sample.
    let ms = Duration::from_millis;
    let span = |slice| played(AudioCue::Holy, 1.0, slice).span();
    assert_eq!(span(AudioSlice::Full), (None, None));
    assert_eq!(span(AudioSlice::Tick), (None, Some(ms(120))));
    assert_eq!(span(AudioSlice::Body), (Some(ms(60)), Some(ms(340))));
    assert_eq!(span(AudioSlice::Tail), (Some(ms(200)), None));
    assert_eq!(AudioSlice::ALL.len(), 4);
    // The fifteen speed steps of a row, and the sample as recorded.
    for step in 14..=28u8 {
        let speed = f32::from(step) * 0.05;
        let cue = SoundCue {
            base: AudioBase::Tower,
            speed,
            slice: AudioSlice::Tick,
            gain: 0.5,
            notes: Vec::new(),
        };
        let voice = Candidate::voiced(&cue, 0.5, true, Origin::Other);
        assert_eq!(voice.step, step);
        assert!(close(voice.variant().speed(), speed));
        assert!(close(voice.gain, 0.25));
        assert_eq!(voice.cue, AudioCue::Tower);
    }
    assert_eq!(Variant::from(AudioCue::Hit).speed(), 1.0);
    assert_eq!(
        Candidate::local(AudioCue::Hit).variant(),
        AudioCue::Hit.into()
    );
    for (base, cue) in [
        (AudioBase::Melee, AudioCue::Melee),
        (AudioBase::Arrow, AudioCue::Arrow),
        (AudioBase::Arcane, AudioCue::Arcane),
        (AudioBase::Holy, AudioCue::Holy),
        (AudioBase::Caster, AudioCue::Caster),
        (AudioBase::Tower, AudioCue::Tower),
        (AudioBase::Bluff, AudioCue::Bluff),
    ] {
        assert_eq!(AudioCue::for_base(base), cue);
        assert_eq!(base.id(), cue.id());
    }
    assert_eq!(AudioBase::ALL.len(), 7);

    // The detune follows from the id alone, stays within 3 % and varies from id to id.
    let mut factors: Vec<f32> = (0..2000).map(detune).collect();
    assert!(factors.iter().all(|factor| (0.97..=1.03).contains(factor)));
    assert_eq!(factors, (0..2000).map(detune).collect::<Vec<_>>());
    factors.sort_by(f32::total_cmp);
    assert!(factors[0] < 0.975 && factors[1999] > 1.025);
    factors.dedup();
    assert!(factors.len() > 1000);
    // Only a voice of a row is detuned.
    let row = |id| Origin::Row {
        moment: Moment::Impact,
        row: "smite",
        actor: 1,
        id,
    };
    let voice = |origin| Candidate::plain(AudioCue::Holy, 1.0, origin);
    assert_eq!(voice(row(41)).detune(), detune(41));
    assert_ne!(voice(row(41)).detune(), voice(row(42)).detune());
    for origin in [
        Origin::Other,
        Origin::Attack {
            actor: 1,
            sequence: 41,
        },
        Origin::Receipt {
            id: 41,
            source: CombatEntity::default(),
            slot: None,
        },
    ] {
        assert_eq!(voice(origin).detune(), 1.0);
    }
}
