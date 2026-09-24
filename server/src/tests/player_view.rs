//! Characterization of the replicated `PlayerState` view.
//!
//! The stored `ConnectedPlayer::state` keeps the cooldown, utility-clock and
//! shop fields at their defaults; `owner_view` fills them from the hero
//! timers at the tick's `now`. These tests pin the exact numbers the old
//! per-tick refreshers wrote, byte for byte, through one hero lifetime and
//! through the sandbox's `apply_actor`.
use super::*;
use shared::hero_balance::{ability_cooldown, basic_cooldown, skill_recovery_secs};
use shared::shop::item_bonuses;
use shared::utility::{
    DASH_COOLDOWN_SECS, HASTE_COOLDOWN_SECS, HASTE_DURATION_SECS, UtilityAction,
};

const A: &str = "127.0.0.1:58301";
const B: &str = "127.0.0.1:58302";

fn fixture() -> (ServerRuntime, SocketAddr, SocketAddr, Instant) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
    rt.targeting_qa = false;
    let now = Instant::now();
    let a: SocketAddr = A.parse().unwrap();
    let b: SocketAddr = B.parse().unwrap();
    for (addr, team) in [(a, Team::Green), (b, Team::Blue)] {
        rt.handle_packet(
            addr,
            ClientPacket::Join {
                prematch: false,
                team,
                character: CharacterChoice::Ipfs,
                hero_class: HeroClass::Warrior,
                avatar: None,
                sprite_character: None,
                session_id: Some(format!("view-{}", addr.port())),
                passport_ticket: None,
            },
            now,
        );
    }
    assert_eq!(rt.world.game_state, GameState::Running);
    // Keep the lanes empty so nothing but the driven requests touches the heroes.
    rt.world.last_wave_spawn_at = now + Duration::from_secs(3600);
    (rt, a, b, now)
}

fn tick(rt: &mut ServerRuntime, now: Instant) {
    // Synthetic clock jumps must not read as a transport timeout.
    for player in rt.world.players.values_mut() {
        player.last_seen = now;
    }
    rt.tick(now, 1.0 / 60.0);
}

/// Seconds of `duration` left at `now` for a clock started at `since`.
fn left(duration: Duration, since: Instant, now: Instant) -> f32 {
    duration.saturating_sub(now - since).as_secs_f32()
}

fn secs(value: f32) -> Duration {
    Duration::from_secs_f32(value)
}

/// The view must be the stored state plus exactly the expected derived
/// fields, compared as the bytes that go on the wire.
fn assert_view(rt: &ServerRuntime, addr: SocketAddr, now: Instant, step: &str, expected: Expected) {
    let player = &rt.world.players[&addr];
    let mut want = player.state.clone();
    want.basic_attack_cooldown_secs = expected.basic_cooldown;
    want.basic_attack_remaining_secs = expected.basic_remaining;
    want.skill_cooldown_remaining_secs = expected.skill_remaining;
    want.skill_recovery_remaining_secs = expected.recovery;
    want.utility.dash_remaining_secs = expected.dash;
    want.utility.haste_remaining_secs = expected.haste_cooldown;
    want.utility.haste_active_secs = expected.haste_active;
    want.shop_available = expected.shop;
    let view = rt.player_view(addr, now);
    assert_eq!(
        serde_json::to_string(&view).unwrap(),
        serde_json::to_string(&want).unwrap(),
        "{step}: player {}",
        player.state.id
    );
    assert_eq!(
        serde_json::to_vec(&view).unwrap(),
        serde_json::to_vec(&player.public_view(now, &rt.world.map_layout, &rt.world.game_state))
            .unwrap(),
        "{step}: public view is the owner view until redaction lands"
    );
    // The stored struct never carries the derived fields.
    assert_eq!(player.state.basic_attack_cooldown_secs, 0.0, "{step}");
    assert_eq!(player.state.basic_attack_remaining_secs, 0.0, "{step}");
    assert_eq!(
        player.state.skill_cooldown_remaining_secs, [0.0; 4],
        "{step}"
    );
    assert_eq!(player.state.skill_recovery_remaining_secs, 0.0, "{step}");
    assert_eq!(player.state.utility.dash_remaining_secs, 0.0, "{step}");
    assert_eq!(player.state.utility.haste_remaining_secs, 0.0, "{step}");
    assert_eq!(player.state.utility.haste_active_secs, 0.0, "{step}");
    assert!(!player.state.shop_available, "{step}");
}

#[derive(Clone, Copy)]
struct Expected {
    basic_cooldown: f32,
    basic_remaining: f32,
    skill_remaining: [f32; 4],
    recovery: f32,
    dash: f32,
    haste_cooldown: f32,
    haste_active: f32,
    shop: bool,
}

impl Expected {
    fn idle(basic_cooldown: f32, shop: bool) -> Self {
        Self {
            basic_cooldown,
            basic_remaining: 0.0,
            skill_remaining: [0.0; 4],
            recovery: 0.0,
            dash: 0.0,
            haste_cooldown: 0.0,
            haste_active: 0.0,
            shop,
        }
    }
}

fn utility(rt: &mut ServerRuntime, addr: SocketAddr, action: UtilityAction, id: u64, now: Instant) {
    rt.handle_packet(
        addr,
        ClientPacket::Utility {
            action,
            direction: [1.0, 0.0],
            request_id: id,
            server_epoch: rt.server_epoch,
            match_id: rt.match_id,
        },
        now,
    );
}

#[test]
fn owner_view_reproduces_the_replicated_clocks_through_a_hero_lifetime() {
    let (mut rt, a, b, t0) = fixture();
    let target = TargetId {
        kind: TargetKind::Player,
        id: rt.world.players[&b].state.id,
    };
    let none = ItemBonuses::NONE;
    let warrior_l1 = basic_cooldown(HeroClass::Warrior, 1, none).as_secs_f32();
    let mut now = t0;
    tick(&mut rt, now);
    assert_view(&rt, a, now, "join", Expected::idle(warrior_l1, true));
    assert_view(&rt, b, now, "join", Expected::idle(warrior_l1, true));

    // Purchase inside the base, before the heroes walk out.
    rt.handle_packet(
        a,
        ClientPacket::BuyItem {
            item_id: ItemId::VitalityGem.id().to_owned(),
            request_id: 1,
            match_id: rt.match_id,
            server_epoch: rt.server_epoch,
        },
        now,
    );
    tick(&mut rt, now);
    assert_eq!(rt.world.players[&a].state.inventory, [ItemId::VitalityGem]);
    let gear = item_bonuses(&[ItemId::VitalityGem]);
    let geared_l1 = basic_cooldown(HeroClass::Warrior, 1, gear).as_secs_f32();
    assert_view(&rt, a, now, "purchase", Expected::idle(geared_l1, true));

    // Skill upgrade with a granted point: level 2, Q rank 2.
    apply_level_up(&mut rt.world.players.get_mut(&a).unwrap().state);
    rt.handle_packet(a, ClientPacket::UpgradeSkill { slot: 0 }, now);
    tick(&mut rt, now);
    assert_eq!(rt.world.players[&a].state.ranks[0], 2);
    let basic = basic_cooldown(HeroClass::Warrior, 2, gear);
    let q = ability_cooldown(HeroClass::Warrior, 2, 2, SkillSlot::Q, gear);
    let recovery = secs(skill_recovery_secs(2));
    assert_view(
        &rt,
        a,
        now,
        "skill upgrade",
        Expected::idle(basic.as_secs_f32(), true),
    );

    for (addr, x) in [(a, 0.0), (b, 2.0)] {
        let state = &mut rt.world.players.get_mut(&addr).unwrap().state;
        state.x = x;
        state.z = 0.0;
    }
    tick(&mut rt, now);
    assert_view(
        &rt,
        a,
        now,
        "left the base",
        Expected::idle(basic.as_secs_f32(), false),
    );

    // Cast Q, read the clocks 400 ms later: Q cools down, recovery runs.
    now += Duration::from_secs(1);
    rt.handle_packet(a, ClientPacket::Cast { target, slot: 0 }, now);
    assert_eq!(rt.world.projectiles.len(), 1);
    let cast_at = now;
    now += Duration::from_millis(400);
    tick(&mut rt, now);
    assert!(left(recovery, cast_at, now) > 0.0);
    assert_view(
        &rt,
        a,
        now,
        "cast",
        Expected {
            skill_remaining: [left(q, cast_at, now), 0.0, 0.0, 0.0],
            recovery: left(recovery, cast_at, now),
            ..Expected::idle(basic.as_secs_f32(), false)
        },
    );
    assert_view(
        &rt,
        b,
        now,
        "cast (target)",
        Expected::idle(warrior_l1, false),
    );

    // Basic attack once recovery has passed; read 250 ms later.
    now += Duration::from_secs(2);
    rt.handle_packet(
        a,
        ClientPacket::BasicAttack {
            target,
            server_epoch: rt.server_epoch,
            match_id: rt.match_id,
            request_id: 1,
        },
        now,
    );
    assert_eq!(rt.world.projectiles.len(), 2);
    let attack_at = now;
    now += Duration::from_millis(250);
    tick(&mut rt, now);
    assert_eq!(left(recovery, cast_at, now), 0.0);
    assert_view(
        &rt,
        a,
        now,
        "basic attack",
        Expected {
            basic_remaining: left(basic, attack_at, now),
            skill_remaining: [left(q, cast_at, now), 0.0, 0.0, 0.0],
            ..Expected::idle(basic.as_secs_f32(), false)
        },
    );

    // Dash, then haste 500 ms later, read 1.5 s after that.
    utility(&mut rt, a, UtilityAction::Dash, 1, now);
    assert_eq!(rt.world.players[&a].state.utility.dash_sequence, 1);
    let dash_at = now;
    now += Duration::from_millis(500);
    tick(&mut rt, now);
    assert_view(
        &rt,
        a,
        now,
        "dash",
        Expected {
            basic_remaining: left(basic, attack_at, now),
            skill_remaining: [left(q, cast_at, now), 0.0, 0.0, 0.0],
            dash: left(secs(DASH_COOLDOWN_SECS), dash_at, now),
            ..Expected::idle(basic.as_secs_f32(), false)
        },
    );
    utility(&mut rt, a, UtilityAction::Haste, 2, now);
    let haste_at = now;
    now += Duration::from_millis(1500);
    tick(&mut rt, now);
    assert_view(
        &rt,
        a,
        now,
        "haste",
        Expected {
            basic_remaining: left(basic, attack_at, now),
            skill_remaining: [left(q, cast_at, now), 0.0, 0.0, 0.0],
            dash: left(secs(DASH_COOLDOWN_SECS), dash_at, now),
            haste_cooldown: left(secs(HASTE_COOLDOWN_SECS), haste_at, now),
            haste_active: left(secs(HASTE_DURATION_SECS), haste_at, now),
            ..Expected::idle(basic.as_secs_f32(), false)
        },
    );
    assert_eq!(rt.player_view(a, now).utility.haste_active_secs, 1.5);

    // Death: combat clocks and haste read zero, utility cooldowns keep running.
    let id = rt.world.players[&a].state.id;
    assert!(apply_player_damage(&mut rt.world.players, id, 10_000.0, now).is_some());
    tick(&mut rt, now);
    assert_eq!(rt.world.players[&a].state.hp, 0.0);
    assert_view(
        &rt,
        a,
        now,
        "death",
        Expected {
            dash: left(secs(DASH_COOLDOWN_SECS), dash_at, now),
            haste_cooldown: left(secs(HASTE_COOLDOWN_SECS), haste_at, now),
            ..Expected::idle(basic.as_secs_f32(), false)
        },
    );

    // Respawn at the base: cast and strike clocks cleared, utilities still cooling.
    now += RESPAWN_DELAY + Duration::from_millis(1);
    tick(&mut rt, now);
    assert!(rt.world.players[&a].state.hp > 0.0);
    assert!(left(secs(DASH_COOLDOWN_SECS), dash_at, now) > 0.0);
    assert_view(
        &rt,
        a,
        now,
        "respawn",
        Expected {
            dash: left(secs(DASH_COOLDOWN_SECS), dash_at, now),
            haste_cooldown: left(secs(HASTE_COOLDOWN_SECS), haste_at, now),
            ..Expected::idle(basic.as_secs_f32(), true)
        },
    );
}

#[test]
fn owner_view_reproduces_the_replicated_clocks_for_sandbox_actors() {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
    let wall = Instant::now();
    rt.sandbox = Some(sandbox::SandboxRuntime::new(wall));
    rt.targeting_qa = true;
    let a: SocketAddr = "127.0.0.1:58303".parse().unwrap();
    rt.handle_packet(
        a,
        ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: CharacterChoice::Cube,
            hero_class: HeroClass::Mage,
            avatar: None,
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        },
        wall,
    );
    let advance = |rt: &mut ServerRuntime| {
        let (now, dt) = rt.sandbox.as_mut().unwrap().advance(0.1);
        rt.tick(now, dt);
        now
    };
    let request = |rt: &mut ServerRuntime, request_id, command| {
        rt.handle_sandbox(
            a,
            shared::sandbox::SandboxRequest {
                server_epoch: rt.server_epoch,
                match_id: rt.match_id,
                request_id,
                command,
            },
        );
    };
    let cast = shared::sandbox::SandboxCommand::ForceCast {
        actor: shared::sandbox::SandboxActor::Player,
        slot: 0,
        target_id: None,
    };
    let now = advance(&mut rt);
    let actor = |rt: &ServerRuntime| rt.world.players[&a].sandbox.clone().unwrap();
    // The sandbox folds the actor's attack speed into its gear bonuses.
    let bonuses = |c: &shared::sandbox::ActorConfig| {
        let mut bonuses = item_bonuses(&c.inventory);
        bonuses.attack_speed_multiplier *= c.attack_speed;
        bonuses
    };
    let basic = |rt: &ServerRuntime| {
        let c = actor(rt);
        basic_cooldown(c.hero, c.level, bonuses(&c)).as_secs_f32()
    };
    let inside_shop = shop_is_available(
        &rt.world.players[&a].state,
        &rt.world.map_layout,
        &rt.world.game_state,
    );
    assert_view(
        &rt,
        a,
        now,
        "sandbox init",
        Expected::idle(basic(&rt), inside_shop),
    );

    let mut config = rt.sandbox.as_ref().unwrap().config.clone();
    config.enemy.enabled = true;
    config.player.no_cooldowns = true;
    config.player.unlock_all = true;
    config.player.infinite_resource = true;
    config.player.attack_speed = 2.0;
    request(
        &mut rt,
        1,
        shared::sandbox::SandboxCommand::ApplyConfig {
            config: config.clone(),
        },
    );
    let now = advance(&mut rt);
    assert_view(
        &rt,
        a,
        now,
        "sandbox no_cooldowns",
        Expected::idle(basic(&rt), inside_shop),
    );
    request(&mut rt, 2, cast.clone());
    assert!(rt.world.players[&a].timers.last_cast_at[0].is_some());
    let now = advance(&mut rt);
    assert!(rt.world.players[&a].timers.last_cast_at[0].is_none());
    assert_view(
        &rt,
        a,
        now,
        "sandbox forced cast without cooldowns",
        Expected::idle(basic(&rt), inside_shop),
    );

    config.player.no_cooldowns = false;
    request(
        &mut rt,
        3,
        shared::sandbox::SandboxCommand::ApplyConfig { config },
    );
    let now = advance(&mut rt);
    assert_view(
        &rt,
        a,
        now,
        "sandbox cooldowns restored",
        Expected::idle(basic(&rt), inside_shop),
    );
    request(&mut rt, 4, cast);
    let cast_at = rt.world.players[&a].timers.last_cast_at[0].unwrap();
    let now = advance(&mut rt);
    let c = actor(&rt);
    let q = ability_cooldown(c.hero, c.level, c.ranks[0], SkillSlot::Q, bonuses(&c));
    assert!(left(q, cast_at, now) > 0.0);
    assert_view(
        &rt,
        a,
        now,
        "sandbox forced cast with cooldowns",
        Expected {
            skill_remaining: [left(q, cast_at, now), 0.0, 0.0, 0.0],
            recovery: left(secs(skill_recovery_secs(c.level)), cast_at, now),
            ..Expected::idle(basic(&rt), inside_shop)
        },
    );
}
