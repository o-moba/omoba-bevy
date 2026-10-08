use super::*;
use crate::combat::CombatStats;
use crate::skill_presentation::bodies;
use shared::loadout::{CoreId, EffectVisualKind};

const HEIGHT: f32 = crate::model_scale::DEFAULT_MODEL_TARGET_HEIGHT;

fn stance(now: f32) -> Stance {
    Stance {
        forward: Vec2::X,
        height: HEIGHT,
        facing: Stance::rest_facing(),
        now,
    }
}

/// The default flags with the one change that reports `state`.
fn flags_of(state: StateVisual) -> LoadoutState {
    let mut flags = LoadoutState::default();
    match state {
        // The server reports a stun as a root of the same length too.
        StateVisual::Stunned => {
            flags.stun_remaining_secs = 0.8;
            flags.root_remaining_secs = 0.8;
            flags.movement_multiplier = 0.0;
        }
        StateVisual::Rooted => {
            flags.root_remaining_secs = 1.2;
            flags.movement_multiplier = 0.0;
        }
        StateVisual::ParryStance => flags.parrying = true,
        StateVisual::Shielded => flags.shield_hp = 30.0,
        StateVisual::Marked => flags.mark_remaining_secs = 3.0,
        StateVisual::Brittle => flags.brittle = true,
        StateVisual::Concussed => flags.concussion_stacks = 2,
        StateVisual::Slowed => {
            flags.slow_multiplier = 0.6;
            flags.movement_multiplier = 0.6;
        }
        StateVisual::CamouflageVeil => flags.camouflaged = true,
        StateVisual::Forging => flags.forge_remaining_secs = 2.0,
    }
    flags
}

/// Every flag of the ten states at once.
fn every_flag() -> LoadoutState {
    LoadoutState {
        stun_remaining_secs: 0.8,
        root_remaining_secs: 1.5,
        parrying: true,
        shield_hp: 30.0,
        mark_remaining_secs: 3.0,
        brittle: true,
        concussion_stacks: 2,
        slow_multiplier: 0.6,
        movement_multiplier: 0.0,
        camouflaged: true,
        forge_remaining_secs: 2.0,
        ..default()
    }
}

/// Drops the flag of `state`, as the server does when the state ends.
fn clear(flags: &mut LoadoutState, state: StateVisual) {
    match state {
        StateVisual::Stunned => flags.stun_remaining_secs = 0.0,
        StateVisual::Rooted => flags.root_remaining_secs = 0.0,
        StateVisual::ParryStance => flags.parrying = false,
        StateVisual::Shielded => flags.shield_hp = 0.0,
        StateVisual::Marked => flags.mark_remaining_secs = 0.0,
        StateVisual::Brittle => flags.brittle = false,
        StateVisual::Concussed => flags.concussion_stacks = 0,
        StateVisual::Slowed => flags.slow_multiplier = 1.0,
        StateVisual::CamouflageVeil => flags.camouflaged = false,
        StateVisual::Forging => flags.forge_remaining_secs = 0.0,
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .insert_resource(PlayerVisualMode::Models3d)
        .add_systems(Startup, bodies::setup_meshes)
        .add_plugins(StatusVisualsPlugin);
    app
}

const SPOT: Vec3 = Vec3::new(4.0, 0.3, -2.0);

fn hero(app: &mut App, id: u64, flags: LoadoutState) -> Entity {
    app.world_mut()
        .spawn((
            NetworkPlayerId(id),
            Transform::from_translation(SPOT),
            InheritedVisibility::VISIBLE,
            PlayerLoadout(Some(flags)),
            CombatStats::default(),
        ))
        .id()
}

fn set_flags(app: &mut App, hero: Entity, flags: LoadoutState) {
    app.world_mut().get_mut::<PlayerLoadout>(hero).unwrap().0 = Some(flags);
    app.update();
}

/// The visual of a hero: its root, its state and its part entities.
fn visual(app: &App, hero: Entity) -> Option<(Entity, StateVisual, Vec<Entity>)> {
    app.world()
        .resource::<StateVisuals>()
        .0
        .get(&hero)
        .map(|shown| (shown.root, shown.state, shown.parts.clone()))
}

fn meshes_drawn(app: &mut App) -> usize {
    app.world_mut().query::<&Mesh3d>().iter(app.world()).count()
}

/// AC6: each state is reported by its own replicated flag and by nothing else, and each
/// has a look of its own. No skill, action, receipt, effect or passing time shows, keeps or
/// ends a visual.
#[test]
fn status_visuals_follow_flags_only() {
    use Silhouette as Mesh;
    use StatePaint as Paint;
    use StateVisual as State;
    let looks = [
        (State::Stunned, vec![Mesh::Star; 3], Paint::Gold),
        (
            State::Rooted,
            vec![Mesh::Torus, Mesh::Cone, Mesh::Cone, Mesh::Cone],
            Paint::Rose,
        ),
        (State::ParryStance, vec![Mesh::Kite; 2], Paint::Violet),
        (State::Shielded, vec![Mesh::Torus, Mesh::Kite], Paint::Azure),
        (State::Marked, vec![Mesh::Diamond], Paint::Gold),
        (State::Brittle, vec![Mesh::Shard; 3], Paint::Ember),
        (State::Concussed, vec![Mesh::Diamond; 2], Paint::Azure),
        (State::Slowed, vec![Mesh::Chevron; 2], Paint::Azure),
        (
            State::CamouflageVeil,
            vec![Mesh::Torus, Mesh::Crescent, Mesh::Crescent],
            Paint::Shade,
        ),
        (State::Forging, vec![Mesh::Drop; 3], Paint::Ember),
    ];
    assert_eq!(looks.each_ref().map(|look| look.0), State::PRIORITY);
    for (state, meshes, paint) in &looks {
        // One flag, one state.
        let flags = flags_of(*state);
        assert_eq!(State::of(&flags).collect::<Vec<_>>(), [*state]);
        let parts = state_parts(*state, &flags, &stance(1.0));
        assert_eq!(
            parts.iter().map(|part| part.mesh).collect::<Vec<_>>(),
            *meshes,
            "{}",
            state.id()
        );
        assert!(
            parts.iter().all(|part| part.paint == *paint),
            "{}",
            state.id()
        );
    }
    // No two states share a look.
    for (i, a) in looks.iter().enumerate() {
        for b in &looks[i + 1..] {
            assert_ne!((&a.1, a.2), (&b.1, b.2), "{} {}", a.0.id(), b.0.id());
        }
    }

    // Everything else a hero replicates reports no state: its kit, a recast it may use, an
    // active slot, its passive and its resources.
    let mut kit = LoadoutState {
        recipe: Some(CoreId::Adventurer.preset()),
        vital_rotation: 3,
        challenge_target: Some(9),
        challenge_sides: 0b1111,
        forge_ready: true,
        energy: true,
        souls: 12,
        orb_position: Some([1.0, 2.0]),
        forged: true,
        weapon_mode: shared::loadout::WeaponMode::Rockets,
        // A speed buff is the speed read of the particle layer, not a state.
        movement_multiplier: 1.4,
        basic_attack_range: 9.0,
        basic_attack_mana_cost: 4.0,
        passive_stacks: 5,
        passive_remaining_secs: 4.0,
        cast_request_id: 77,
        ..default()
    };
    for slot in &mut kit.slots {
        slot.can_recast = true;
        slot.active = true;
        slot.recast_remaining_secs = 3.0;
    }
    assert_eq!(State::of(&kit).count(), 0);
    assert_eq!(State::of(&LoadoutState::default()).count(), 0);
    // Values that are not a number report nothing.
    let broken = LoadoutState {
        stun_remaining_secs: f32::NAN,
        root_remaining_secs: f32::NAN,
        shield_hp: f32::NAN,
        mark_remaining_secs: f32::NAN,
        slow_multiplier: f32::NAN,
        forge_remaining_secs: f32::NAN,
        ..default()
    };
    assert_eq!(State::of(&broken).count(), 0);

    // In the world: the victim of a stunning skill is not stunned until its own flag says
    // so. The caster's accepted Bluff, the receipt of its hit and a replicated effect of
    // the caster change nothing.
    let mut app = app();
    let caster = hero(&mut app, 7, kit.clone());
    app.world_mut()
        .entity_mut(caster)
        .insert(crate::net::PlayerCosmeticAction {
            sequence: 4,
            kind: Default::default(),
            slot: 1,
        });
    let victim = hero(&mut app, 8, LoadoutState::default());
    let mut game = crate::net::GameStateSnapshot::default();
    game.skill_effects.push(SkillEffectState {
        id: 1,
        owner_id: 7,
        owner_team: shared::map::Team::Green,
        skill: SkillId::WinterDivide,
        kind: EffectVisualKind::BeamWarning,
        position: [SPOT.x, SPOT.z],
        end: [SPOT.x + 4.0, SPOT.z],
        radius: 2.0,
        remaining_secs: 1.0,
        armed: true,
        consumed_segments: 0,
    });
    let player = |id| shared::combat::CombatEntity {
        kind: shared::combat::CombatEntityKind::Player,
        id,
    };
    game.combat_events.push(shared::combat::CombatEvent {
        id: 5,
        source: player(7),
        target: player(8),
        amount: 40.0,
        action_slot: Some(1),
        ..default()
    });
    app.insert_resource(game);
    for _ in 0..40 {
        app.update();
    }
    assert!(visual(&app, caster).is_none() && visual(&app, victim).is_none());
    assert_eq!(meshes_drawn(&mut app), 0);

    // The flag shows the visual in that frame and keeps it for as long as it is set.
    set_flags(&mut app, victim, flags_of(State::Stunned));
    let (root, state, parts) = visual(&app, victim).unwrap();
    assert_eq!((state, parts.len()), (State::Stunned, 3));
    for _ in 0..400 {
        app.update();
    }
    assert_eq!(visual(&app, victim).unwrap().0, root);
    // It leaves in the frame the flag does, with its parts.
    set_flags(&mut app, victim, LoadoutState::default());
    assert!(visual(&app, victim).is_none());
    assert!(app.world().get_entity(root).is_err());
    assert!(
        parts
            .iter()
            .all(|part| app.world().get_entity(*part).is_err())
    );
    assert_eq!(meshes_drawn(&mut app), 0);
}

/// The highest-ranking state is the one drawn, and a stun outranks the root that only
/// mirrors it.
#[test]
fn the_highest_ranking_state_is_the_one_drawn() {
    use StateVisual as State;
    let mut flags = every_flag();
    // A stunned hero is not also rooted, although the server reports a root as well.
    assert!(!State::Rooted.active(&flags));
    assert_eq!(State::of(&flags).count(), 9);
    let mut app = app();
    let hero = hero(&mut app, 3, flags.clone());
    app.update();
    for (rank, state) in State::PRIORITY.into_iter().enumerate() {
        assert_eq!(
            State::shown(PlayerVisualMode::Models3d, true, true, &flags),
            Some(state)
        );
        // Every lower state is still reported; the root joins them once the stun is over.
        let below: Vec<_> = State::PRIORITY[rank..]
            .iter()
            .copied()
            .filter(|lower| (state, *lower) != (State::Stunned, State::Rooted))
            .collect();
        assert_eq!(State::of(&flags).collect::<Vec<_>>(), below);
        let (root, drawn, _) = visual(&app, hero).unwrap();
        assert_eq!(drawn, state);
        clear(&mut flags, state);
        set_flags(&mut app, hero, flags.clone());
        // The next state is another visual: the old one is gone with its parts.
        assert!(app.world().get_entity(root).is_err());
    }
    assert!(visual(&app, hero).is_none());
    assert_eq!(meshes_drawn(&mut app), 0);

    // A root that outlasts the stun shows once the stun is over.
    let mut held = flags_of(State::Stunned);
    held.root_remaining_secs = 2.0;
    assert_eq!(State::of(&held).collect::<Vec<_>>(), [State::Stunned]);
    held.stun_remaining_secs = 0.0;
    assert_eq!(State::of(&held).collect::<Vec<_>>(), [State::Rooted]);
}

/// AC13: one visual per hero with at most four parts, in every state the flags can report
/// and at every moment, and no part ever leaves its hero.
#[test]
fn a_hero_has_one_visual_of_at_most_four_parts_that_stays_with_it() {
    use StateVisual as State;
    assert_eq!(MAX_PARTS, 4);
    let cameras = [
        Stance::rest_facing(),
        Quat::IDENTITY,
        Quat::from_rotation_y(2.1) * Quat::from_rotation_x(-0.4),
    ];
    for state in State::PRIORITY {
        let flags = flags_of(state);
        for step in 0..240 {
            let now = step as f32 * 0.37;
            for (turn, facing) in cameras.into_iter().enumerate() {
                let stance = Stance {
                    forward: Vec2::from_angle(step as f32 * 0.7 + turn as f32),
                    facing,
                    ..stance(now)
                };
                let parts = state_parts(state, &flags, &stance);
                assert!((1..=MAX_PARTS).contains(&parts.len()), "{}", state.id());
                for part in &parts {
                    let pose = part.pose;
                    assert!(pose.is_finite(), "{}", state.id());
                    assert!(pose.scale.min_element() > 0.05, "{}", state.id());
                    assert!(pose.scale.max_element() <= 2.0, "{}", state.id());
                    // Within arm's reach of the hero, between its feet and above its head.
                    assert!(pose.translation.xz().length() <= 1.25, "{}", state.id());
                    assert!(
                        (0.1..=HEIGHT + 1.0).contains(&pose.translation.y),
                        "{} {}",
                        state.id(),
                        pose.translation.y
                    );
                }
            }
        }
    }

    // Concussion: one pip per stack up to three, each in its own place from the left, so
    // a pip never moves when the next one arrives.
    let pips = |stacks: u8| {
        let flags = LoadoutState {
            concussion_stacks: stacks,
            ..default()
        };
        state_parts(State::Concussed, &flags, &stance(0.0))
    };
    assert_eq!(MAX_PIPS, 3);
    assert!(pips(0).is_empty());
    for stacks in 1..=u8::MAX {
        let shown = pips(stacks);
        assert_eq!(shown.len(), usize::from(stacks.min(3)));
        assert_eq!(shown[..], pips(3)[..shown.len()]);
    }
    let across = Stance::rest_facing() * Vec3::X;
    let places: Vec<f32> = pips(3)
        .iter()
        .map(|pip| pip.pose.translation.dot(across))
        .collect();
    assert!(places[0] < places[1] && places[1] < places[2]);
    assert!((places[0] + places[2]).abs() < 1e-5 && places[1].abs() < 1e-5);

    // The mark shrinks with the replicated time that is left and with nothing else: a
    // mark seen late has the size of a mark that has run down to that time.
    let mark = |left: f32, now: f32| {
        let flags = LoadoutState {
            mark_remaining_secs: left,
            ..default()
        };
        state_parts(State::Marked, &flags, &stance(now))[0]
            .pose
            .scale
            .x
    };
    assert_eq!(mark_secs(), 6.0);
    assert_eq!(mark(6.0, 0.0), MARK_SIZE.1);
    assert_eq!(mark(60.0, 0.0), MARK_SIZE.1);
    assert_eq!(mark(f32::INFINITY, 0.0), MARK_SIZE.1);
    assert!((mark(3.0, 0.0) - (MARK_SIZE.0 + MARK_SIZE.1) / 2.0).abs() < 1e-5);
    assert!(mark(0.01, 0.0) > MARK_SIZE.0 && mark(0.01, 0.0) < MARK_SIZE.0 + 0.01);
    for step in 0..60 {
        let left = step as f32 * 0.1;
        assert!(mark(left + 0.1, 0.0) > mark(left, 0.0));
        assert_eq!(mark(left, 0.0), mark(left, 123.4));
    }

    // In the world: a pip is added when the replicated stack count rises and the pips
    // leave together.
    let mut app = app();
    let struck = hero(&mut app, 9, LoadoutState::default());
    for stacks in [1, 2, 3, 3, 1, 0] {
        let flags = LoadoutState {
            concussion_stacks: stacks,
            ..default()
        };
        set_flags(&mut app, struck, flags);
        assert_eq!(meshes_drawn(&mut app), usize::from(stacks));
        assert_eq!(
            visual(&app, struck).map(|(_, state, parts)| (state, parts.len())),
            (stacks > 0).then_some((State::Concussed, usize::from(stacks)))
        );
    }

    // Ten heroes in every state spend at most forty parts.
    let mut app = self::app();
    let heroes: Vec<_> = (0..10)
        .map(|i| hero(&mut app, 20 + i, every_flag()))
        .collect();
    let mut flags = every_flag();
    for state in State::PRIORITY {
        for hero in &heroes {
            app.world_mut().get_mut::<PlayerLoadout>(*hero).unwrap().0 = Some(flags.clone());
        }
        app.update();
        let drawn = meshes_drawn(&mut app);
        assert!((10..=10 * MAX_PARTS).contains(&drawn), "{}", state.id());
        assert_eq!(app.world().resource::<StateVisuals>().0.len(), 10);
        clear(&mut flags, state);
    }
}

/// A visual is an entity of its own at the position of its hero: it follows the hero and
/// never joins its hierarchy. It is drawn from the shared meshes and from six materials
/// and takes no part in the shadow pass.
#[test]
fn a_visual_is_a_free_entity_of_shared_meshes_and_six_materials() {
    use StateVisual as State;
    assert_eq!(StatePaint::ALL.len(), 6);
    // The material rule of the parser counts these six with white and the two sides.
    assert_eq!(StatePaint::ALL.len() + 3, bodies::SHARED_MATERIALS);
    for (i, paint) in StatePaint::ALL.into_iter().enumerate() {
        assert_eq!(paint as usize, i);
        let material = paint.material();
        assert!(material.unlit && material.alpha_mode == AlphaMode::Opaque);
        for other in &StatePaint::ALL[i + 1..] {
            assert_ne!(paint.look().0, other.look().0);
        }
    }

    let mut app = app();
    app.update();
    let library = app.world().resource::<Assets<Mesh>>().len();
    let hero = hero(&mut app, 3, flags_of(State::Rooted));
    app.update();
    let (root, _, parts) = visual(&app, hero).unwrap();
    assert_eq!(parts.len(), 4);
    assert!(!app.world().entity(root).contains::<ChildOf>());
    assert!(!app.world().entity(hero).contains::<Children>());
    // The hero of this world is not measured: its feet are half a unit under its origin.
    let feet = Vec3::NEG_Y * 0.5;
    assert_eq!(
        app.world().get::<Transform>(root).unwrap().translation,
        SPOT + feet
    );
    let paints: Vec<_> = app.world().resource::<StatePaints>().0.to_vec();
    for part in &parts {
        let part = app.world().entity(*part);
        assert_eq!(part.get::<ChildOf>().unwrap().parent(), root);
        assert!(part.contains::<NotShadowCaster>() && part.contains::<NotShadowReceiver>());
        assert_eq!(
            part.get::<MeshMaterial3d<StandardMaterial>>().unwrap().0,
            paints[StatePaint::Rose as usize]
        );
    }
    // The same state in the next frames keeps its entities and follows the hero.
    let moved = SPOT + Vec3::new(1.5, 0.4, -3.0);
    app.world_mut()
        .get_mut::<Transform>(hero)
        .unwrap()
        .translation = moved;
    app.update();
    assert_eq!(visual(&app, hero).unwrap(), (root, State::Rooted, parts));
    assert_eq!(
        app.world().get::<Transform>(root).unwrap().translation,
        moved + feet
    );

    // The feet and the height come from the measured model, wherever its origin is.
    assert_eq!(standing(None, None), (-0.5, HEIGHT));
    assert_eq!(standing(Some(0.0), Some(2.1)), (0.0, 2.1));
    assert_eq!(standing(Some(-1.05), Some(1.05)), (-1.05, 2.1));
    assert_eq!(standing(Some(-0.9), Some(1.5)), (-0.9, 2.4));
    // A model that is measured only in part, or wrongly, still gives a hero's size.
    assert_eq!(standing(None, Some(1.6)), (-0.5, 2.1));
    assert_eq!(standing(Some(-0.2), None), (-0.2, HEIGHT));
    assert_eq!(standing(Some(0.0), Some(40.0)).1, 3.0);
    assert_eq!(standing(Some(0.0), Some(-3.0)).1, 0.3);
    assert_eq!(standing(Some(f32::NAN), Some(f32::NAN)), (-0.5, HEIGHT));
    assert_eq!(standing(Some(0.0), Some(f32::INFINITY)), (0.0, HEIGHT));

    // Every state of every hero is drawn from the library and the six materials.
    let mut flags = every_flag();
    for state in State::PRIORITY {
        set_flags(&mut app, hero, flags.clone());
        clear(&mut flags, state);
    }
    assert_eq!(app.world().resource::<Assets<Mesh>>().len(), library);
    assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 6);
    // A hero that leaves the world takes its visual along.
    set_flags(&mut app, hero, flags_of(State::Shielded));
    let (root, ..) = visual(&app, hero).unwrap();
    app.world_mut().entity_mut(hero).despawn();
    app.update();
    assert!(app.world().get_entity(root).is_err());
    assert!(app.world().resource::<StateVisuals>().0.is_empty());
}

/// A hero that is hidden or dead has no visual, and the flat view draws none at all.
#[test]
fn a_hidden_or_dead_hero_has_no_visual_and_the_flat_view_draws_none() {
    use StateVisual as State;
    let flags = flags_of(State::Shielded);
    let shown = |mode, visible, alive| State::shown(mode, visible, alive, &flags);
    assert_eq!(
        shown(PlayerVisualMode::Models3d, true, true),
        Some(State::Shielded)
    );
    assert_eq!(shown(PlayerVisualMode::Models3d, false, true), None);
    assert_eq!(shown(PlayerVisualMode::Models3d, true, false), None);
    assert_eq!(shown(PlayerVisualMode::Sprite2d, true, true), None);

    let mut app = app();
    let hero = hero(&mut app, 3, flags.clone());
    app.update();
    let (root, ..) = visual(&app, hero).unwrap();
    assert_eq!(meshes_drawn(&mut app), 2);
    // Hidden: nothing stands where the hero is.
    app.world_mut()
        .entity_mut(hero)
        .insert(InheritedVisibility::HIDDEN);
    app.update();
    assert!(visual(&app, hero).is_none());
    assert!(app.world().get_entity(root).is_err());
    assert_eq!(meshes_drawn(&mut app), 0);
    app.world_mut()
        .entity_mut(hero)
        .insert(InheritedVisibility::VISIBLE);
    app.update();
    assert!(visual(&app, hero).is_some());
    // Dead.
    app.world_mut().get_mut::<CombatStats>(hero).unwrap().hp = 0.0;
    app.update();
    assert!(visual(&app, hero).is_none());
    app.world_mut().get_mut::<CombatStats>(hero).unwrap().hp = 50.0;
    app.update();
    assert!(visual(&app, hero).is_some());
    // No replicated state at all, or a position that is not a number.
    app.world_mut().get_mut::<PlayerLoadout>(hero).unwrap().0 = None;
    app.update();
    assert!(visual(&app, hero).is_none());
    set_flags(&mut app, hero, flags.clone());
    assert!(visual(&app, hero).is_some());
    app.world_mut()
        .get_mut::<Transform>(hero)
        .unwrap()
        .translation
        .x = f32::NAN;
    app.update();
    assert!(visual(&app, hero).is_none());
    app.world_mut()
        .get_mut::<Transform>(hero)
        .unwrap()
        .translation = SPOT;
    app.update();
    let (root, ..) = visual(&app, hero).unwrap();
    // The flat view: every visual is removed and none is made.
    *app.world_mut().resource_mut::<PlayerVisualMode>() = PlayerVisualMode::Sprite2d;
    app.update();
    assert!(app.world().get_entity(root).is_err());
    assert!(app.world().resource::<StateVisuals>().0.is_empty());
    for _ in 0..3 {
        app.update();
        assert_eq!(meshes_drawn(&mut app), 0);
    }
    *app.world_mut().resource_mut::<PlayerVisualMode>() = PlayerVisualMode::Models3d;
    app.update();
    assert!(visual(&app, hero).is_some());

    // A world that starts flat never spawns a visual.
    let mut flat = App::new();
    flat.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .insert_resource(PlayerVisualMode::Sprite2d)
        .add_systems(Startup, bodies::setup_meshes)
        .add_plugins(StatusVisualsPlugin);
    for id in 0..4 {
        self::hero(&mut flat, id, every_flag());
    }
    for _ in 0..3 {
        flat.update();
    }
    assert_eq!(meshes_drawn(&mut flat), 0);
}

fn mountain(id: u64, owner: u64, at: [f32; 2]) -> SkillEffectState {
    SkillEffectState {
        id,
        owner_id: owner,
        owner_team: shared::map::Team::Green,
        skill: SkillId::MountainEcho,
        kind: EffectVisualKind::Bolt,
        position: at,
        end: [at[0] + 1.0, at[1]],
        radius: 2.0,
        remaining_secs: 3.0,
        armed: true,
        consumed_segments: 0,
    }
}

/// A recast marker is shown for a slot only while the server reports its recast, in the
/// colour of the skill the accepted recipe has in that slot.
#[test]
fn recast_marker_needs_can_recast() {
    let registry = SkillPresentation::target();
    let at = Vec2::new(3.0, -1.0);
    let mut seen = Vec::new();
    for class in shared::HeroClass::ALL {
        let Some(kit) = shared::loadout::preset_for_class(class) else {
            continue;
        };
        let recipe = kit.recipe();
        let mut flags = LoadoutState {
            recipe: Some(recipe.clone()),
            ..default()
        };
        // Mountain Echo also needs its colossus near; it stands beside the hero here.
        let effects = [mountain(1, 7, at.to_array())];
        let markers =
            |flags: &LoadoutState| recast_markers(&registry, class, flags, 7, at, &effects);
        // Nothing is offered: no marker, whatever else the slots report.
        for slot in &mut flags.slots {
            slot.active = true;
            slot.recast_remaining_secs = 4.0;
        }
        assert!(markers(&flags).is_empty(), "{class:?}");
        for (slot, skill) in recipe.skills.into_iter().enumerate() {
            let profile = registry.profile(skill).unwrap();
            let marker = profile.cast.as_ref().and_then(|cast| cast.recast_marker);
            let mut offered = flags.clone();
            offered.slots[slot].can_recast = true;
            // Only the offered slot speaks, with the marker and the colour of its row.
            assert_eq!(
                markers(&offered),
                marker
                    .map(|marker| (marker, Color::srgb_from_array(profile.color)))
                    .into_iter()
                    .collect::<Vec<_>>(),
                "{}",
                skill.id()
            );
            if let Some(marker) = marker {
                seen.push((skill.id(), marker));
            }
        }
        // Every slot at once: one marker for each row that has one, in slot order.
        let mut all = flags.clone();
        for slot in &mut all.slots {
            slot.can_recast = true;
        }
        assert_eq!(
            markers(&all).len(),
            recipe
                .skills
                .iter()
                .filter(|skill| seen.iter().any(|(id, _)| *id == skill.id()))
                .count()
        );
    }
    seen.sort();
    assert_eq!(
        seen,
        [
            ("dawn_field", RecastMarker::RingPips),
            ("echo_strike", RecastMarker::OrbitMotes),
            ("flame_dance", RecastMarker::OrbitMotes),
            ("iron_hook", RecastMarker::RingPips),
            ("mountain_echo", RecastMarker::GroundArrows),
            ("thorn_volley", RecastMarker::OrbitMotes),
            ("thunder_pulse", RecastMarker::RingPips),
        ]
    );

    // The slot of the accepted recipe names the skill: a skill moved to another button
    // takes its marker along, and a recipe that does not resolve shows none.
    let mut recipe = CoreId::Wildspark.preset();
    recipe.skills[2] = SkillId::DawnField;
    let mut mixed = LoadoutState {
        recipe: Some(recipe),
        ..default()
    };
    mixed.slots[2].can_recast = true;
    let class = shared::HeroClass::Wildspark;
    assert_eq!(
        recast_markers(&registry, class, &mixed, 7, at, &[])
            .iter()
            .map(|(marker, _)| *marker)
            .collect::<Vec<_>>(),
        [RecastMarker::RingPips]
    );
    mixed.slots[2].can_recast = false;
    mixed.slots[0].can_recast = true;
    assert!(recast_markers(&registry, class, &mixed, 7, at, &[]).is_empty());
    mixed.slots[2].can_recast = true;
    mixed.recipe.as_mut().unwrap().skills[1] = SkillId::DawnField;
    assert!(recast_markers(&registry, class, &mixed, 7, at, &[]).is_empty());
    // The unmigrated rows have no marker yet.
    let unmigrated = SkillPresentation::unmigrated();
    mixed.recipe.as_mut().unwrap().skills = CoreId::Wildspark.preset().skills;
    mixed.recipe.as_mut().unwrap().skills[2] = SkillId::DawnField;
    assert!(recast_markers(&unmigrated, class, &mixed, 7, at, &[]).is_empty());
}

/// The Mountain Echo marker is shown only where the server accepts the recast: within
/// 4 units of one of the hero's own Mountain Echo effects.
#[test]
fn the_mountain_echo_marker_needs_the_hero_near_its_own_colossus() {
    let registry = SkillPresentation::target();
    let class = shared::HeroClass::Cinderforge;
    let recipe = CoreId::Cinderforge.preset();
    let slot = recipe
        .skills
        .iter()
        .position(|skill| *skill == SkillId::MountainEcho)
        .unwrap();
    let mut flags = LoadoutState {
        recipe: Some(recipe),
        ..default()
    };
    flags.slots[slot].can_recast = true;
    let at = Vec2::new(10.0, 5.0);
    let shown = |hero: u64, effects: &[SkillEffectState]| {
        recast_markers(&registry, class, &flags, hero, at, effects)
            .iter()
            .map(|(marker, _)| *marker)
            .collect::<Vec<_>>()
    };
    let gate = crate::skill_presentation::geometry::RECAST_GATE_MOUNTAIN_ECHO;
    assert_eq!(gate, 4.0);
    // The slot alone reports the recast for seconds; without the colossus in the
    // snapshot nothing is shown.
    assert!(shown(7, &[]).is_empty());
    let inside = mountain(1, 7, [at.x + gate - 0.01, at.y]);
    let outside = mountain(2, 7, [at.x, at.y + gate + 0.01]);
    assert_eq!(
        shown(7, std::slice::from_ref(&inside)),
        [RecastMarker::GroundArrows]
    );
    assert_eq!(shown(7, &[mountain(1, 7, [at.x + gate, at.y])]).len(), 1);
    assert!(shown(7, std::slice::from_ref(&outside)).is_empty());
    // Any of the hero's own effects of the skill opens the gate, as on the server.
    assert_eq!(shown(7, &[outside.clone(), inside.clone()]).len(), 1);
    // Another hero's colossus, one whose owner is hidden, and an effect of another skill
    // of the hero open nothing.
    assert!(shown(7, &[mountain(1, 8, at.to_array())]).is_empty());
    assert!(shown(7, &[mountain(1, 0, at.to_array())]).is_empty());
    assert!(shown(0, &[mountain(1, 0, at.to_array())]).is_empty());
    let mut other = inside.clone();
    other.skill = SkillId::FaultLine;
    assert!(shown(7, &[other]).is_empty());
    let mut broken = inside.clone();
    broken.position = [f32::NAN, at.y];
    assert!(shown(7, &[broken]).is_empty());

    // The gate belongs to Mountain Echo alone: every other recast is in reach anywhere.
    for skill in SkillId::ALL {
        assert_eq!(
            recast_in_reach(skill, 7, at, &[]),
            skill != SkillId::MountainEcho,
            "{}",
            skill.id()
        );
    }
    assert!(recast_in_reach(SkillId::MountainEcho, 7, at, &[inside]));
    assert!(!recast_in_reach(SkillId::MountainEcho, 7, at, &[outside]));
}
