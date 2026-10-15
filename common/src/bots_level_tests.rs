//! Lower-level behaviour gates (docs/plans/bot-personas.md, S2).
//!
//! Each fixture builds a small world by hand and drives the real brain
//! (`simulate_bots`) on an explicit 50 ms clock. Controllers come from
//! `attach_with_profile`, so a level is tested without a practice host. The
//! passive enemies are human seats without a controller.
use super::*;
use crate::{
    combat_feedback::CombatLog,
    entities::Vec3f,
    game_world::GameWorld,
    match_rules::{MatchMode, MatchRules},
};
use shared::career::{MatchStats, ParticipantResult};
use shared::combat::{CombatEntity, CombatEntityKind, CombatEvent};
use shared::wire::{CharacterChoice, StructureKind};

const STEP: Duration = Duration::from_millis(50);

/// Counts heap allocations on the current thread while enabled, for the
/// no-new-allocation check of the level gates. Test builds only; every call
/// goes straight to the system allocator.
mod alloc_probe {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static COUNT: Cell<Option<u64>> = const { Cell::new(None) };
    }

    fn note() {
        let _ = COUNT.try_with(|count| {
            if let Some(n) = count.get() {
                count.set(Some(n + 1));
            }
        });
    }

    struct Counting;

    // SAFETY: every method forwards its arguments unchanged to `System`,
    // whose contract is the caller's.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            note();
            // SAFETY: as for `System::alloc`.
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            note();
            // SAFETY: as for `System::alloc_zeroed`.
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            note();
            // SAFETY: as for `System::realloc`.
            unsafe { System.realloc(ptr, layout, new_size) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: as for `System::dealloc`.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    /// The allocations `f` makes on this thread.
    pub(super) fn count(f: impl FnOnce()) -> u64 {
        COUNT.with(|count| count.set(Some(0)));
        f();
        COUNT.with(Cell::take).unwrap_or(0)
    }
}

struct Arena {
    world: GameWorld,
    bots: BotControllers,
    log: CombatLog,
    now: Instant,
}

impl Arena {
    fn new() -> Self {
        Self {
            world: GameWorld::empty(),
            bots: BotControllers::default(),
            log: CombatLog::default(),
            now: Instant::now(),
        }
    }

    /// A joined level-10 hero at `at` with 1000 HP, 500 mana, every slot
    /// ranked and no gold, so the brain never shops.
    fn hero(&mut self, addr: &str, team: Team, class: HeroClass, at: [f32; 2]) -> SocketAddr {
        let addr: SocketAddr = addr.parse().unwrap();
        self.world.ensure_connected(addr, self.now);
        crate::session::handle_join_request(
            self.world.players.get_mut(&addr).unwrap(),
            team,
            CharacterChoice::Ipfs,
            class,
            None,
            &self.world.map_layout,
            self.now,
        );
        let p = self.world.players.get_mut(&addr).unwrap();
        p.hero.x = at[0];
        p.hero.z = at[1];
        p.hero.hp = 1000.0;
        p.hero.max_hp = 1000.0;
        p.hero.mana = 500.0;
        p.hero.max_mana = 500.0;
        p.hero.progress.level = 10;
        p.hero.progress.ranks = [3; 4];
        p.hero.progress.skill_points = 0;
        p.economy.gold = 0;
        addr
    }

    /// A `hero` the brain drives at `profile`.
    fn bot(
        &mut self,
        addr: &str,
        team: Team,
        class: HeroClass,
        at: [f32; 2],
        profile: BotProfile,
    ) -> SocketAddr {
        let addr = self.hero(addr, team, class, at);
        self.bots
            .attach_with_profile(addr, class, profile, self.now);
        addr
    }

    fn set_hp(&mut self, addr: SocketAddr, fraction: f32) {
        let hero = &mut self.world.players.get_mut(&addr).unwrap().hero;
        hero.hp = hero.max_hp * fraction;
    }

    fn id(&self, addr: SocketAddr) -> u64 {
        self.world.players[&addr].hero.identity.id
    }

    fn hero_target(&self, addr: SocketAddr) -> Option<TargetId> {
        Some(TargetId {
            kind: TargetKind::Player,
            id: self.id(addr),
        })
    }

    fn controller(&self, addr: SocketAddr) -> &Controller {
        &self.bots.controllers[&addr]
    }

    fn target(&self, addr: SocketAddr) -> Option<TargetId> {
        self.controller(addr).target
    }

    fn targets(&self, addr: SocketAddr, kind: TargetKind) -> bool {
        self.target(addr).is_some_and(|target| target.kind == kind)
    }

    /// Whether the last tick was a retreat: timed, or a health retreat that
    /// `resume_hp` still holds (turret retreats aside).
    fn retreating(&self, addr: SocketAddr) -> bool {
        let controller = self.controller(addr);
        controller
            .retreat_until
            .is_some_and(|until| self.now < until)
            || controller.retreat_started.is_some()
    }

    /// One 50 ms tick of the brain.
    fn step(&mut self) {
        self.now += STEP;
        CombatHost {
            world: &mut self.world,
            bots: &mut self.bots,
            combat_log: &mut self.log,
            rules: MatchRules::for_mode(MatchMode::Practice, 5),
            match_id: 1,
        }
        .simulate_bots(self.now, STEP.as_secs_f32());
    }

    fn run_until(&mut self, at: Instant) {
        while self.now < at {
            self.step();
        }
    }

    fn run(&mut self, duration: Duration) {
        self.run_until(self.now + duration);
    }

    /// Start the round ledger with every joined hero, so hits are recorded.
    fn start_ledger(&mut self) {
        let roster = self
            .world
            .players
            .values()
            .filter(|p| p.joined)
            .map(|p| ParticipantResult {
                is_bot: false,
                player_id: p.hero.identity.id,
                profile_id: None,
                nickname: format!("Hero {}", p.hero.identity.id),
                team: p.hero.identity.team,
                hero_class: p.hero.identity.hero_class,
                character: "ipfs".into(),
                avatar: None,
                sprite_character: None,
                stats: MatchStats::default(),
                disconnected: false,
                rating: None,
                progression_xp_gained: 0,
            })
            .collect();
        self.log.ledger.begin(roster).unwrap();
    }

    /// The authority reports `attacker` damaging `victim` now.
    fn hit(&mut self, attacker: SocketAddr, victim: SocketAddr) {
        let event = CombatEvent {
            source: CombatEntity {
                kind: CombatEntityKind::Player,
                id: self.id(attacker),
            },
            target: CombatEntity {
                kind: CombatEntityKind::Player,
                id: self.id(victim),
            },
            amount: 10.0,
            ..Default::default()
        };
        self.log.extend(self.now, [event]);
    }

    /// Living minions of `team` at `at`: a fresh wave cut to `count`.
    fn minions(&mut self, team: Team, count: usize, at: [f32; 2]) {
        crate::world::spawn_minion_wave_for_team_lane(
            &self.world.map_layout,
            &mut self.world.minions,
            &mut self.world.next_minion_id,
            team,
            Lane::Top,
        );
        let mut wave: Vec<_> = self
            .world
            .minions
            .iter()
            .filter(|(_, m)| m.state.team == team)
            .map(|(id, _)| *id)
            .collect();
        wave.sort_unstable();
        assert!(wave.len() >= count, "a wave has {} minions", wave.len());
        for (index, id) in wave.into_iter().enumerate() {
            let minion = self.world.minions.get_mut(&id).unwrap();
            minion.state.x = at[0];
            minion.state.z = at[1];
            if index >= count {
                minion.state.hp = 0.0;
            }
        }
    }

    /// A tower of `team` at `at` with an 8 m range.
    fn tower(&mut self, team: Team, lane: Lane, at: [f32; 2]) -> u64 {
        let mut next = self.world.structures.keys().max().map_or(1, |id| id + 1);
        let id = next;
        crate::world::add_structure(
            &mut self.world.structures,
            &mut next,
            StructureKind::Tower,
            StructureRole::LaneTower { lane },
            team,
            Vec3f::new(at[0], 3.0, at[1]),
        );
        self.world.structures.get_mut(&id).unwrap().attack_range = 8.0;
        id
    }
}

/// A terrain-clear stretch from 4 m behind `origin` to 12 m ahead of it, so
/// chases and lunges are not blocked.
fn clear_corridor() -> [f32; 2] {
    let nav = shared::navigation::world_navigation();
    (-40..=40)
        .step_by(5)
        .flat_map(|x| (-40..=40).step_by(5).map(move |z| [x as f32, z as f32]))
        .find(|p| nav.segment_clear([p[0] - 4.0, p[1]], [p[0] + 12.0, p[1]]))
        .expect("the map has a clear corridor")
}

fn levels() -> [(BotDifficulty, BotProfile); 4] {
    BotDifficulty::ALL.map(|level| (level, BotProfile::for_level(level)))
}

#[test]
fn only_the_lower_levels_build_think_gates() {
    assert!(
        !BotProfile::ELITE.has_gates(),
        "Elite keeps the original tick"
    );
    for profile in [
        BotProfile::RECRUIT,
        BotProfile::REGULAR,
        BotProfile::VETERAN,
    ] {
        assert!(profile.has_gates(), "{profile:?}");
    }
}

#[test]
fn recruit_retreats_at_45_percent_and_stays_until_85_percent_or_the_cap() {
    let mut arena = Arena::new();
    let elite = arena.bot(
        "[::]:34001",
        Team::Green,
        HeroClass::Warrior,
        [0.0, 0.0],
        BotProfile::ELITE,
    );
    arena.set_hp(elite, 0.45);
    arena.step();
    assert!(!arena.retreating(elite), "Elite holds at 45%");

    // Elite: six seconds, then back out even at 20%.
    let mut arena = Arena::new();
    let elite = arena.bot(
        "[::]:34001",
        Team::Green,
        HeroClass::Warrior,
        [0.0, 0.0],
        BotProfile::ELITE,
    );
    arena.set_hp(elite, 0.20);
    arena.step();
    let started = arena.now;
    assert!(arena.retreating(elite));
    arena.run_until(started + Duration::from_secs(6) - STEP);
    assert!(arena.retreating(elite));
    arena.step();
    assert!(!arena.retreating(elite), "Elite has no resume_hp");

    // Recruit: at 60% it is still on its way home after ten seconds, and it
    // comes back out as soon as it reaches 85%.
    let mut arena = Arena::new();
    let bot = arena.bot(
        "[::]:34002",
        Team::Green,
        HeroClass::Warrior,
        [0.0, 0.0],
        BotProfile::RECRUIT,
    );
    let spawn = spawn_position_for_team(&arena.world.map_layout, Team::Green);
    let home = |arena: &Arena| {
        let hero = &arena.world.players[&bot].hero;
        (hero.x - spawn.x).hypot(hero.z - spawn.z)
    };
    let far = home(&arena);
    arena.set_hp(bot, 0.45);
    arena.step();
    assert!(arena.retreating(bot), "Recruit retreats at 45%");
    arena.set_hp(bot, 0.60);
    arena.run(Duration::from_secs(10));
    assert!(arena.retreating(bot), "60% is short of resume_hp 0.85");
    assert_eq!(arena.target(bot), None);
    assert!(home(&arena) < far - 10.0, "it heads home");
    arena.set_hp(bot, 0.86);
    arena.step();
    assert!(!arena.retreating(bot), "resume_hp reached");

    // The cap ends the hold 30 s after the retreat began.
    let mut arena = Arena::new();
    let bot = arena.bot(
        "[::]:34002",
        Team::Green,
        HeroClass::Warrior,
        [0.0, 0.0],
        BotProfile::RECRUIT,
    );
    arena.set_hp(bot, 0.45);
    arena.step();
    let started = arena.now;
    arena.set_hp(bot, 0.60);
    arena.run_until(started + Duration::from_secs(30) - STEP);
    assert!(arena.retreating(bot));
    arena.step();
    assert!(!arena.retreating(bot), "retreat_cap_secs");
}

/// A Warrior on the top-lane approach to a Blue tower with `cover` Green
/// minions inside its range.
fn tower_fixture(profile: BotProfile, cover: usize) -> (Arena, SocketAddr) {
    let mut arena = Arena::new();
    let bot = arena.bot(
        "[::]:34101",
        Team::Green,
        HeroClass::Warrior,
        [-16.0, -8.0],
        profile,
    );
    arena.tower(Team::Blue, Lane::Top, [-4.0, -8.0]);
    arena.minions(Team::Green, cover, [-4.0, -6.0]);
    (arena, bot)
}

#[test]
fn recruit_waits_for_three_minions_of_cover_and_regular_for_two() {
    for (level, cover, pushes) in [
        (BotDifficulty::Elite, 1, true),
        (BotDifficulty::Veteran, 1, true),
        (BotDifficulty::Regular, 1, false),
        (BotDifficulty::Regular, 2, true),
        (BotDifficulty::Recruit, 2, false),
        (BotDifficulty::Recruit, 3, true),
    ] {
        let (mut arena, bot) = tower_fixture(BotProfile::for_level(level), cover);
        let mut closest = f32::INFINITY;
        for _ in 0..160 {
            arena.step();
            let hero = &arena.world.players[&bot].hero;
            closest = closest.min((hero.x + 4.0).hypot(hero.z + 8.0));
        }
        assert_eq!(
            closest < 8.0,
            pushes,
            "{level:?} with {cover} minions came within {closest} m"
        );
    }
}

#[test]
fn recruit_hits_a_covered_tower_only_at_60_percent_health_or_more() {
    for (level, hp, sieges) in [
        (BotDifficulty::Recruit, 0.55, false),
        (BotDifficulty::Recruit, 0.65, true),
        (BotDifficulty::Regular, 0.42, false),
        (BotDifficulty::Regular, 0.50, true),
        (BotDifficulty::Elite, 0.30, true),
    ] {
        let (mut arena, bot) = tower_fixture(BotProfile::for_level(level), 3);
        arena.set_hp(bot, hp);
        arena.step();
        assert!(!arena.retreating(bot), "{level:?} at {hp}");
        assert_eq!(
            arena.targets(bot, TargetKind::Structure),
            sieges,
            "{level:?} at {hp}"
        );
    }
}

#[test]
fn two_on_one_recruit_retreats_regular_holds_veteran_and_elite_fight() {
    for (level, profile) in levels() {
        let mut arena = Arena::new();
        let bot = arena.bot(
            "[::]:34201",
            Team::Green,
            HeroClass::Warrior,
            [0.0, 0.0],
            profile,
        );
        arena.hero(
            "127.0.0.1:34202",
            Team::Blue,
            HeroClass::Warrior,
            [5.0, 0.0],
        );
        arena.hero(
            "127.0.0.1:34203",
            Team::Blue,
            HeroClass::Warrior,
            [5.0, 1.5],
        );
        arena.step();
        let (retreats, fights) = match level {
            BotDifficulty::Recruit => (true, false),
            BotDifficulty::Regular => (false, false),
            _ => (false, true),
        };
        assert_eq!(arena.retreating(bot), retreats, "{level:?}");
        assert_eq!(arena.targets(bot, TargetKind::Player), fights, "{level:?}");
    }

    // Regular drops the hero it is fighting when a second one arrives, in
    // the next think tick, and still holds its ground.
    let mut arena = Arena::new();
    let bot = arena.bot(
        "[::]:34201",
        Team::Green,
        HeroClass::Warrior,
        [0.0, 0.0],
        BotProfile::REGULAR,
    );
    let first = arena.hero(
        "127.0.0.1:34202",
        Team::Blue,
        HeroClass::Warrior,
        [5.0, 0.0],
    );
    arena.step();
    assert_eq!(arena.target(bot), arena.hero_target(first));
    arena.hero(
        "127.0.0.1:34203",
        Team::Blue,
        HeroClass::Warrior,
        [5.0, 1.5],
    );
    arena.run(THINK_INTERVAL);
    assert_eq!(arena.target(bot), None);
    assert!(!arena.retreating(bot));
}

/// A Warden jungler next to its only camp with an enemy hero in sight.
fn camp_fixture(profile: BotProfile, hp: f32) -> (Arena, SocketAddr, SocketAddr) {
    let mut arena = Arena::new();
    let mut camps = crate::neutrals::build_neutral_camps(&mut 9_001);
    let id = camps
        .iter()
        .filter(|(_, n)| !n.state.camp_type.is_boss())
        .map(|(id, _)| *id)
        .min()
        .expect("an ordinary camp");
    let mut camp = camps.remove(&id).unwrap();
    camp.state.x = 4.0;
    camp.state.z = 0.0;
    camp.anchor.x = 4.0;
    camp.anchor.z = 0.0;
    arena.world.neutrals.insert(id, camp);
    let jungler = arena.bot(
        "[::]:34301",
        Team::Green,
        HeroClass::Warden,
        [0.0, 0.0],
        profile,
    );
    let enemy = arena.hero(
        "127.0.0.1:34302",
        Team::Blue,
        HeroClass::Warrior,
        [6.0, 2.0],
    );
    arena.set_hp(jungler, hp);
    (arena, jungler, enemy)
}

#[test]
fn recruit_jungler_never_ganks_and_regular_only_when_ahead() {
    for (level, hp, ganks) in [
        (BotDifficulty::Elite, 1.0, true),
        (BotDifficulty::Veteran, 1.0, true),
        (BotDifficulty::Regular, 1.0, true),
        (BotDifficulty::Regular, 0.5, false),
        (BotDifficulty::Recruit, 1.0, false),
    ] {
        let (mut arena, jungler, enemy) = camp_fixture(BotProfile::for_level(level), hp);
        arena.step();
        let expected = if ganks {
            arena.hero_target(enemy)
        } else {
            arena.world.neutrals.keys().next().map(|id| TargetId {
                kind: TargetKind::Neutral,
                id: *id,
            })
        };
        assert_eq!(arena.target(jungler), expected, "{level:?} at {hp}");
    }
}

/// An Edgeweaver whose only ready skill is its lunge, 3 m from an enemy hero
/// at `hp` of a large health pool.
fn lunge_fixture(profile: BotProfile, hp: f32) -> (Arena, SocketAddr) {
    let mut arena = Arena::new();
    let origin = clear_corridor();
    let bot = arena.bot(
        "[::]:34401",
        Team::Green,
        HeroClass::Edgeweaver,
        origin,
        profile,
    );
    let enemy = arena.hero(
        "127.0.0.1:34402",
        Team::Blue,
        HeroClass::Warrior,
        [origin[0] + 3.0, origin[1]],
    );
    arena.world.players.get_mut(&enemy).unwrap().hero.max_hp = 100_000.0;
    arena.set_hp(enemy, hp);
    let now = arena.now;
    arena
        .world
        .players
        .get_mut(&bot)
        .unwrap()
        .timers
        .last_cast_at = [None, Some(now), Some(now), Some(now)];
    (arena, bot)
}

fn lunged(arena: &Arena, bot: SocketAddr) -> bool {
    arena.world.players[&bot].timers.last_cast_at[0].is_some()
}

#[test]
fn recruit_dashes_onto_a_hero_only_at_or_below_35_percent() {
    let lunge = shared::loadout::skill(shared::loadout::SkillId::EdgeLunge);
    assert_eq!(
        mobility_technique(lunge),
        Some(shared::loadout::Technique::Lunge)
    );
    for (level, hp, dashes) in [
        (BotDifficulty::Elite, 1.0, true),
        (BotDifficulty::Veteran, 1.0, true),
        (BotDifficulty::Regular, 0.6, false),
        (BotDifficulty::Regular, 0.5, true),
        (BotDifficulty::Recruit, 0.5, false),
        (BotDifficulty::Recruit, 0.35, true),
    ] {
        let (mut arena, bot) = lunge_fixture(BotProfile::for_level(level), hp);
        arena.step();
        assert!(arena.targets(bot, TargetKind::Player), "{level:?} at {hp}");
        if dashes {
            assert!(
                lunged(&arena, bot),
                "{level:?} lunges in its first think tick"
            );
        } else {
            arena.run(Duration::from_secs(2));
            assert!(
                arena.targets(bot, TargetKind::Player),
                "{level:?} still fights"
            );
            assert!(
                !lunged(&arena, bot),
                "{level:?} withholds the lunge at {hp}"
            );
        }
    }

    // The gate is for heroes only: a minion gets the lunge at once.
    let mut arena = Arena::new();
    let origin = clear_corridor();
    let bot = arena.bot(
        "[::]:34401",
        Team::Green,
        HeroClass::Edgeweaver,
        origin,
        BotProfile::RECRUIT,
    );
    arena.minions(Team::Blue, 1, [origin[0] + 3.0, origin[1]]);
    let now = arena.now;
    arena
        .world
        .players
        .get_mut(&bot)
        .unwrap()
        .timers
        .last_cast_at = [None, Some(now), Some(now), Some(now)];
    arena.step();
    assert!(arena.targets(bot, TargetKind::Minion));
    assert!(lunged(&arena, bot));
}

#[test]
fn recruit_at_full_health_takes_a_fresh_lone_hero_within_11_m() {
    for (distance, hp, engages) in [(10.0, 1.0, true), (12.0, 1.0, false), (10.0, 0.9, false)] {
        let mut arena = Arena::new();
        let bot = arena.bot(
            "[::]:34501",
            Team::Green,
            HeroClass::Warrior,
            [0.0, 0.0],
            BotProfile::RECRUIT,
        );
        let enemy = arena.hero(
            "127.0.0.1:34502",
            Team::Blue,
            HeroClass::Warrior,
            [distance, 0.0],
        );
        arena.set_hp(bot, hp);
        arena.step();
        assert_eq!(
            arena.target(bot) == arena.hero_target(enemy),
            engages,
            "{distance} m at {hp}"
        );
    }
}

#[test]
fn recruit_minion_first_takes_a_hero_only_in_reach_or_with_no_minion_in_sight() {
    for (hero_at, minion, takes_hero) in [(5.0, false, true), (5.0, true, false), (1.5, true, true)]
    {
        let mut arena = Arena::new();
        let bot = arena.bot(
            "[::]:34511",
            Team::Green,
            HeroClass::Warrior,
            [0.0, 0.0],
            BotProfile::RECRUIT,
        );
        let enemy = arena.hero(
            "127.0.0.1:34512",
            Team::Blue,
            HeroClass::Warrior,
            [hero_at, 0.0],
        );
        if minion {
            arena.minions(Team::Blue, 1, [0.0, 8.0]);
        }
        arena.step();
        assert_eq!(
            arena.target(bot) == arena.hero_target(enemy),
            takes_hero,
            "hero at {hero_at} m, minion {minion}"
        );
    }
}

/// A Recruit at the origin and an enemy Ranger 13 m away, beyond its engage
/// range, with the round ledger running.
fn threat_fixture() -> (Arena, SocketAddr, SocketAddr) {
    let mut arena = Arena::new();
    let bot = arena.bot(
        "[::]:34521",
        Team::Green,
        HeroClass::Warrior,
        [0.0, 0.0],
        BotProfile::RECRUIT,
    );
    let enemy = arena.hero(
        "127.0.0.1:34522",
        Team::Blue,
        HeroClass::Ranger,
        [13.0, 0.0],
    );
    arena.start_ledger();
    (arena, bot, enemy)
}

#[test]
fn recruit_hit_from_13_m_answers_in_the_same_think_tick() {
    // Unprovoked, a hero at 13 m is out of a Recruit's range.
    let (mut arena, bot, _) = threat_fixture();
    arena.step();
    assert_eq!(arena.target(bot), None);
    assert!(!arena.retreating(bot));

    // Hit at full health: it retaliates in the first think tick.
    let (mut arena, bot, enemy) = threat_fixture();
    arena.hit(enemy, bot);
    arena.step();
    assert_eq!(arena.target(bot), arena.hero_target(enemy));

    // Hit at 70%: the trade gate refuses, so it backs off in that same tick
    // for `retreat_secs`, and the health re-arm is left alone.
    let (mut arena, bot, enemy) = threat_fixture();
    arena.set_hp(bot, 0.7);
    let rearm = arena.controller(bot).next_retreat;
    arena.hit(enemy, bot);
    arena.step();
    assert_eq!(arena.target(bot), None);
    assert_eq!(
        arena.controller(bot).retreat_until,
        Some(arena.now + Duration::from_secs(6))
    );
    assert_eq!(arena.controller(bot).next_retreat, rearm);

    // Busy with a minion, it still turns on the hero that hits it, in the
    // next think tick.
    let (mut arena, bot, enemy) = threat_fixture();
    arena.minions(Team::Blue, 1, [1.5, 0.0]);
    arena.step();
    assert!(arena.targets(bot, TargetKind::Minion));
    arena.hit(enemy, bot);
    arena.run(THINK_INTERVAL);
    assert_eq!(arena.target(bot), arena.hero_target(enemy));

    // A hit older than `threat_window_secs` is no threat any more.
    let (mut arena, bot, enemy) = threat_fixture();
    arena.hit(enemy, bot);
    arena.now += Duration::from_secs(2);
    arena.step();
    assert_eq!(arena.target(bot), None);
}

#[test]
fn a_regular_hit_while_outnumbered_backs_off() {
    let mut arena = Arena::new();
    let bot = arena.bot(
        "[::]:34531",
        Team::Green,
        HeroClass::Warrior,
        [0.0, 0.0],
        BotProfile::REGULAR,
    );
    let enemy = arena.hero("127.0.0.1:34532", Team::Blue, HeroClass::Ranger, [6.0, 0.0]);
    arena.hero(
        "127.0.0.1:34533",
        Team::Blue,
        HeroClass::Warrior,
        [6.0, 2.0],
    );
    arena.start_ledger();
    arena.step();
    assert!(!arena.retreating(bot), "one short of outnumbered_retreat");
    arena.hit(enemy, bot);
    arena.run(THINK_INTERVAL);
    assert!(arena.retreating(bot), "the hold refuses the attacker");
    assert_eq!(arena.target(bot), None);
}

#[test]
fn a_recruit_defends_its_own_tower_through_every_gate() {
    // At 60% health, outnumbered and with an enemy minion in sight, a
    // Recruit backs off; under its own tower it takes on the divers.
    for own_tower in [false, true] {
        let mut arena = Arena::new();
        let bot = arena.bot(
            "[::]:34541",
            Team::Green,
            HeroClass::Warrior,
            [0.0, 0.0],
            BotProfile::RECRUIT,
        );
        arena.set_hp(bot, 0.6);
        arena.hero(
            "127.0.0.1:34542",
            Team::Blue,
            HeroClass::Warrior,
            [4.0, 0.0],
        );
        arena.hero(
            "127.0.0.1:34543",
            Team::Blue,
            HeroClass::Warrior,
            [4.0, 1.5],
        );
        arena.minions(Team::Blue, 1, [0.0, 6.0]);
        if own_tower {
            arena.tower(Team::Green, Lane::Mid, [2.0, 0.0]);
        }
        arena.step();
        assert_eq!(arena.retreating(bot), !own_tower, "own tower {own_tower}");
        assert_eq!(
            arena.targets(bot, TargetKind::Player),
            own_tower,
            "own tower {own_tower}"
        );
    }

    // The own base counts too, the fountain and shop zone.
    let mut arena = Arena::new();
    let home = arena.world.map_layout.home;
    let enemy = arena.hero(
        "127.0.0.1:34544",
        Team::Blue,
        HeroClass::Warrior,
        [home.x, home.z],
    );
    let hero = &arena.world.players[&enemy];
    assert!(defends(&arena.world, Team::Green, hero));
    assert!(!defends(&arena.world, Team::Blue, hero));
}

/// A Recruit at its spawn, past the timed part of a health retreat and held
/// home by `resume_hp` at 60%, with an enemy Ranger out of sight and the
/// round ledger running. `at(along, across)` is a point relative to the
/// spawn: `along` points away from the fountain, `across` sideways.
fn held_at_home() -> (Arena, SocketAddr, SocketAddr, impl Fn(f32, f32) -> [f32; 2]) {
    let mut arena = Arena::new();
    let spawn = spawn_position_for_team(&arena.world.map_layout, Team::Green);
    let home = arena.world.map_layout.home;
    let length = (spawn.x - home.x).hypot(spawn.z - home.z);
    let along = [(spawn.x - home.x) / length, (spawn.z - home.z) / length];
    let at = move |a: f32, c: f32| {
        [
            spawn.x + along[0] * a - along[1] * c,
            spawn.z + along[1] * a + along[0] * c,
        ]
    };
    let bot = arena.bot(
        "[::]:34551",
        Team::Green,
        HeroClass::Warrior,
        [spawn.x, spawn.z],
        BotProfile::RECRUIT,
    );
    let away = arena.world.map_layout.away;
    let enemy = arena.hero(
        "127.0.0.1:34552",
        Team::Blue,
        HeroClass::Ranger,
        [away.x, away.z],
    );
    arena.start_ledger();
    arena.set_hp(bot, 0.45);
    arena.step();
    let started = arena.now;
    arena.set_hp(bot, 0.6);
    arena.run_until(started + Duration::from_secs(7));
    let controller = arena.controller(bot);
    assert!(controller.retreat_started.is_some(), "the hold runs");
    assert!(
        controller
            .retreat_until
            .is_some_and(|until| arena.now >= until),
        "past retreat_secs"
    );
    assert_eq!(controller.target, None);
    (arena, bot, enemy, at)
}

fn put(arena: &mut Arena, addr: SocketAddr, at: [f32; 2]) {
    let hero = &mut arena.world.players.get_mut(&addr).unwrap().hero;
    hero.x = at[0];
    hero.z = at[1];
}

#[test]
fn a_recruit_held_home_by_resume_hp_still_defends_its_base() {
    // The diver hits it from 13 m, inside the fountain and shop zone: an
    // answer within one think tick, though the hold goes on.
    let (mut arena, bot, enemy, at) = held_at_home();
    let inside = at(5.0, 12.0);
    let home = arena.world.map_layout.home;
    assert!((inside[0] - home.x).hypot(inside[1] - home.z) <= crate::balance::BASE_HEAL_RADIUS);
    let start = at(0.0, 0.0);
    assert!(
        shared::navigation::world_navigation().segment_clear(start, inside),
        "a clear approach"
    );
    put(&mut arena, enemy, inside);
    arena.hit(enemy, bot);
    arena.run(THINK_INTERVAL);
    assert_eq!(arena.target(bot), arena.hero_target(enemy));
    let distance = |arena: &Arena| {
        let (a, b) = (
            &arena.world.players[&bot].hero,
            &arena.world.players[&enemy].hero,
        );
        (a.x - b.x).hypot(a.z - b.z)
    };
    let before = distance(&arena);
    arena.run(Duration::from_secs(1));
    assert_eq!(arena.target(bot), arena.hero_target(enemy), "kept");
    assert!(
        distance(&arena) < before - 1.0,
        "it closes in, not walks home"
    );
    assert!(arena.controller(bot).retreat_started.is_some());

    // Once the diver leaves, the hold takes over again.
    let away = arena.world.map_layout.away;
    put(&mut arena, enemy, [away.x, away.z]);
    arena.run(THINK_INTERVAL);
    assert_eq!(arena.target(bot), None);
    assert!(
        arena.controller(bot).retreat_started.is_some(),
        "still held"
    );

    // Unprovoked, a diver inside the engage range is taken on too, and one
    // beyond it is left alone.
    for ((along, across), defended) in [((3.0, 4.0), true), ((5.0, 12.0), false)] {
        let (mut arena, bot, enemy, at) = held_at_home();
        put(&mut arena, enemy, at(along, across));
        arena.run(THINK_INTERVAL);
        let expected = if defended {
            arena.hero_target(enemy)
        } else {
            None
        };
        assert_eq!(arena.target(bot), expected, "at ({along}, {across})");
    }

    // An attacker outside the base and away from its towers does not end
    // the hold: walking home is the answer.
    let (mut arena, bot, enemy, at) = held_at_home();
    let outside = at(14.0, 0.0);
    assert!((outside[0] - home.x).hypot(outside[1] - home.z) > crate::balance::BASE_HEAL_RADIUS);
    put(&mut arena, enemy, outside);
    arena.hit(enemy, bot);
    arena.run(THINK_INTERVAL);
    assert_eq!(arena.target(bot), None);
    assert!(arena.controller(bot).retreat_started.is_some());
}

#[test]
fn every_level_heals_below_28_percent_and_only_there() {
    for (level, profile) in levels() {
        for (hp, heals) in [(0.30, false), (0.27, true)] {
            let mut arena = Arena::new();
            let bot = arena.bot(
                "[::]:34601",
                Team::Green,
                HeroClass::Warrior,
                [0.0, 0.0],
                profile,
            );
            arena.set_hp(bot, hp);
            arena.step();
            assert_eq!(
                arena.world.players[&bot].timers.last_cast_at[1].is_some(),
                heals,
                "{level:?} at {hp}"
            );
        }
    }
}

/// Every gate admits the fight: a lone hero 8 m away (inside 11 m), the bot
/// at 95% health and the hero at 30% (at most 95%, and under every dash
/// gate), no minions and no towers.
fn admitted_duel(profile: BotProfile) -> (Arena, SocketAddr) {
    let mut arena = Arena::new();
    let origin = clear_corridor();
    let bot = arena.bot(
        "[::]:34701",
        Team::Green,
        HeroClass::Edgeweaver,
        origin,
        profile,
    );
    let enemy = arena.hero(
        "127.0.0.1:34702",
        Team::Blue,
        HeroClass::Warrior,
        [origin[0] + 8.0, origin[1]],
    );
    arena.world.players.get_mut(&enemy).unwrap().hero.max_hp = 100_000.0;
    arena.set_hp(enemy, 0.30);
    arena.set_hp(bot, 0.95);
    (arena, bot)
}

/// Per tick: the commands in issue order (slot, aim point, target), then the
/// bot's position, facing, target and its think and route schedule.
fn duel_trace(profile: BotProfile) -> Vec<String> {
    let (mut arena, bot) = admitted_duel(profile);
    let start = arena.now;
    command_trace::start();
    let mut trace = Vec::new();
    for _ in 0..120 {
        arena.step();
        let controller = arena.controller(bot);
        let hero = &arena.world.players[&bot].hero;
        trace.push(format!(
            "{} | x={:?} z={:?} yaw={:?} target={:?} think={:?} route={:?}",
            command_trace::drain().join("; "),
            hero.x,
            hero.z,
            hero.yaw,
            controller.target,
            controller.next_think.saturating_duration_since(start),
            controller.next_route.saturating_duration_since(start),
        ));
    }
    command_trace::stop();
    trace
}

#[test]
fn no_level_delays_an_admitted_action() {
    let elite = duel_trace(BotProfile::ELITE);
    assert!(
        elite[0].contains(" cast 0 "),
        "the lunge opens: {}",
        elite[0]
    );
    assert!(elite.iter().any(|tick| tick.contains(" attack ")));
    for (level, profile) in levels() {
        let trace = duel_trace(profile);
        for (tick, (elite, level_tick)) in elite.iter().zip(&trace).enumerate() {
            assert_eq!(
                level_tick, elite,
                "{level:?} differs from Elite at tick {tick}"
            );
        }
    }
}

/// A lone bot walking its lane for ten seconds, with an enemy hero 13 m
/// away when `enemy`: the instants it thought at, its think and route
/// schedule per tick, and whether its first think took the hero.
struct Cadence {
    thinks: Vec<Duration>,
    schedule: Vec<(Duration, Duration)>,
    took_hero: bool,
}

fn cadence(profile: BotProfile, enemy: bool) -> Cadence {
    let mut arena = Arena::new();
    let bot = arena.bot(
        "[::]:34801",
        Team::Green,
        HeroClass::Warrior,
        [0.0, 0.0],
        profile,
    );
    if enemy {
        let enemy = arena.hero(
            "127.0.0.1:34802",
            Team::Blue,
            HeroClass::Warrior,
            [13.0, 0.0],
        );
        arena.world.players.get_mut(&enemy).unwrap().hero.max_hp = 100_000.0;
        arena.set_hp(enemy, 1.0);
    }
    let start = arena.now;
    let mut thinks = Vec::new();
    let mut schedule = Vec::new();
    let mut took_hero = false;
    for tick in 0..200 {
        let before = arena.controller(bot).next_think;
        arena.step();
        if tick == 0 {
            took_hero = arena.targets(bot, TargetKind::Player);
        }
        let controller = arena.controller(bot);
        if controller.next_think != before {
            thinks.push(arena.now - start);
        }
        schedule.push((
            controller.next_think - start,
            controller.next_route.saturating_duration_since(start),
        ));
    }
    Cadence {
        thinks,
        schedule,
        took_hero,
    }
}

#[test]
fn think_and_route_cadence_never_depends_on_the_level() {
    let elite = cadence(BotProfile::ELITE, false);
    assert_eq!(elite.thinks[0], STEP, "the first tick thinks");
    for pair in elite.thinks.windows(2) {
        assert_eq!(pair[1] - pair[0], THINK_INTERVAL);
    }
    for (level, profile) in levels() {
        let walk = cadence(profile, false);
        assert_eq!(walk.thinks, elite.thinks, "{level:?}");
        assert_eq!(walk.schedule, elite.schedule, "{level:?}");
        // Where the levels decide differently, they still think on the
        // same ticks.
        let engaged = cadence(profile, true);
        assert_eq!(
            engaged.thinks, elite.thinks,
            "{level:?} with a hero in sight"
        );
        assert_eq!(
            engaged.took_hero,
            level != BotDifficulty::Recruit,
            "{level:?}: only Recruit leaves a hero at 13 m alone"
        );
    }
}

/// Per tick of an admitted duel: whether the bot thought, how many gate
/// scans ran and how many allocations the tick made.
fn tick_costs(profile: BotProfile) -> Vec<(bool, usize, u64)> {
    let (mut arena, bot) = admitted_duel(profile);
    (0..60)
        .map(|_| {
            let think = arena.controller(bot).next_think;
            let scans = gate_probe::scans();
            let allocations = alloc_probe::count(|| arena.step());
            (
                arena.controller(bot).next_think != think,
                gate_probe::scans() - scans,
                allocations,
            )
        })
        .collect()
}

#[test]
fn the_gate_scan_runs_once_per_think_tick_and_allocates_nothing() {
    // Build every lazily built table on this thread first.
    tick_costs(BotProfile::ELITE);
    let elite = tick_costs(BotProfile::ELITE);
    assert!(
        elite.iter().all(|(_, scans, _)| *scans == 0),
        "Elite scans nothing"
    );
    for profile in [
        BotProfile::RECRUIT,
        BotProfile::REGULAR,
        BotProfile::VETERAN,
    ] {
        let costs = tick_costs(profile);
        assert!(costs.iter().filter(|(think, ..)| *think).count() > 10);
        for (tick, ((think, scans, allocations), (_, _, elite_allocations))) in
            costs.iter().zip(&elite).enumerate()
        {
            assert_eq!(*scans, usize::from(*think), "tick {tick}");
            // The first think tick sizes the reused sight buffer.
            if tick > 0 {
                assert_eq!(
                    allocations, elite_allocations,
                    "{profile:?} allocates more than Elite at tick {tick}"
                );
            }
        }
    }
}
