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
fn packaged_manifest_has_every_safe_stable_cue() {
    let catalog = CueCatalog::parse(include_str!("../../../assets/audio/manifest.json")).unwrap();
    assert_eq!(catalog.cues.len(), 16);
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
        winner: Team::Green,
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
                &GameState::Victory { winner: Team::Blue },
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
