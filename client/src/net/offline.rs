//! Socket-free practice, using the normal client snapshot/render/input pipeline.
//! This deliberately has no career backend, matchmaking, rewards, or persistence.
use super::*;
use shared::{SkillSlot, TargetingMode, hero_balance as balance, shop::ItemBonuses, utility::*};

pub(super) const ADDRESS: &str = "offline-practice";
const LOCAL_ID: u64 = 1;
const LEVEL: u32 = 6; // Every class slot is available for character testing.

pub(super) fn shipped_avatar(slug: Option<&str>) -> bool {
    slug.is_none_or(|s| {
        shared::avatar_roster()
            .iter()
            .any(|a| a.slug == s && a.passport.is_none())
    })
}

#[derive(Resource)]
pub(super) struct LocalPractice {
    commands: Receiver<ClientPacket>,
    snapshots: Sender<ServerPacket>,
    _signals: Sender<NetThreadSignal>,
    simulation: Simulation,
}
impl LocalPractice {
    pub(super) fn new(
        commands: Receiver<ClientPacket>,
        snapshots: Sender<ServerPacket>,
        signals: Sender<NetThreadSignal>,
    ) -> Self {
        Self {
            commands,
            snapshots,
            _signals: signals,
            simulation: Simulation::default(),
        }
    }
}

pub(super) fn step(practice: Option<ResMut<LocalPractice>>, time: Res<Time>) {
    let Some(mut practice) = practice else {
        return;
    };
    while let Ok(packet) = practice.commands.try_recv() {
        practice.simulation.command(packet);
    }
    practice.simulation.advance(time.delta_secs().min(0.1));
    let snapshot = practice.simulation.snapshot();
    let _ = practice.snapshots.try_send(snapshot);
}

struct Shot {
    state: ProjectileState,
    target: TargetId,
    damage: f32,
}
#[derive(Default)]
struct Simulation {
    players: Vec<PlayerState>,
    shots: Vec<Shot>,
    events: VecDeque<CombatEvent>,
    tick: u64,
    sequence: u64,
    elapsed: f32,
    respawn: HashMap<u64, f32>,
    error: Option<JoinRejection>,
}

fn hero(
    id: u64,
    class: HeroClass,
    avatar: Option<String>,
    x: f32,
    z: f32,
    team: Team,
) -> PlayerState {
    // Keep protocol defaults in one place; explicitly set practice-only facts.
    serde_json::from_value(serde_json::json!({
        "id":id, "is_bot":id != LOCAL_ID, "x":x, "y":MapLayout::default().terrain_height_3d(x,z)+0.5, "z":z, "yaw":0.0,
        "team":team, "hero_class":class, "avatar":avatar, "level":LEVEL,
        "hp":balance::base_hp(class), "max_hp":balance::base_hp(class),
        "mana":100.0, "max_mana":100.0, "ranks":[1,1,1,1],
        "basic_attack_cooldown_secs":balance::basic_cooldown(class, LEVEL, ItemBonuses::NONE).as_secs_f32()
    })).expect("static practice player contract")
}
impl Simulation {
    fn command(&mut self, packet: ClientPacket) {
        match packet {
            ClientPacket::Join {
                character,
                hero_class,
                avatar,
                sprite_character,
                ..
            } => {
                if !self.players.is_empty() {
                    return;
                }
                if !shipped_avatar(avatar.as_deref()) {
                    self.error = Some(JoinRejection::AvatarNotAuthorized);
                    return;
                }
                self.error = None;
                let [x, z] = shared::map::geometry().home;
                let mut local = hero(LOCAL_ID, hero_class, avatar, x + 6.0, z + 6.0, Team::Green);
                local.character = character;
                local.sprite_character = sprite_character;
                self.players.push(local);
                for (i, class) in HeroClass::ALL.into_iter().enumerate() {
                    let avatar = shared::avatar_roster()
                        .iter()
                        .filter(|a| a.passport.is_none())
                        .nth(i)
                        .map(|a| a.slug.to_owned());
                    self.players.push(hero(
                        2 + i as u64,
                        class,
                        avatar,
                        x + 11.0 + i as f32 * 3.0,
                        z + 11.0,
                        Team::Blue,
                    ));
                }
            }
            ClientPacket::Leave => {
                self.players.clear();
                self.shots.clear();
                self.events.clear();
                self.respawn.clear();
            }
            ClientPacket::Transform {
                x,
                y,
                z,
                yaw,
                dash_sequence,
            } => {
                if let Some(p) = self.players.first_mut() {
                    if [x, y, z, yaw].iter().all(|v| v.is_finite())
                        && dash_sequence == p.utility.dash_sequence
                    {
                        p.x = x;
                        p.y = y;
                        p.z = z;
                        p.yaw = yaw;
                    }
                }
            }
            ClientPacket::BasicAttack {
                target, request_id, ..
            } => {
                let Some(p) = self.players.first_mut() else {
                    return;
                };
                if request_id <= p.basic_attack_request_id {
                    return;
                }
                p.basic_attack_request_id = request_id;
                if p.basic_attack_remaining_secs > 0.0 {
                    return;
                }
                let def = shared::basic_attack_for_class(p.hero_class);
                let damage = balance::basic_damage(p.hero_class, LEVEL, ItemBonuses::NONE);
                if self.valid_target(target, def.range) {
                    self.players[0].basic_attack_remaining_secs =
                        self.players[0].basic_attack_cooldown_secs;
                    self.attack(target, damage, None);
                }
            }
            ClientPacket::Cast { target, slot } => {
                let Some(index) = SkillSlot::from_index(slot) else {
                    return;
                };
                let Some(p) = self.players.first() else {
                    return;
                };
                let def = shared::ability_for_class_slot(p.hero_class, index);
                if p.skill_cooldown_remaining_secs[slot as usize] > 0.0
                    || p.skill_recovery_remaining_secs > 0.0
                    || p.mana < def.base_mana_cost
                {
                    return;
                }
                if def.targeting == TargetingMode::UnitTarget
                    && !self.valid_target(target, def.cast_range)
                {
                    return;
                }
                let p = &mut self.players[0];
                p.mana = (p.mana - def.base_mana_cost + def.self_mana_restore.unwrap_or(0.0))
                    .min(p.max_mana);
                p.hp = (p.hp + def.self_heal.unwrap_or(0.0)).min(p.max_hp);
                p.skill_cooldown_remaining_secs[slot as usize] =
                    balance::ability_cooldown(p.hero_class, LEVEL, 1, index, ItemBonuses::NONE)
                        .as_secs_f32();
                p.skill_recovery_remaining_secs = balance::skill_recovery_secs(LEVEL);
                p.action_sequence += 1;
                p.action_kind = PlayerActionKind::Cast;
                p.action_slot = slot;
                if let Some(damage) = def.projectile_damage {
                    let damage = damage * balance::ability_power_multiplier(p.hero_class, LEVEL);
                    self.attack(target, damage, Some(slot));
                }
            }
            ClientPacket::Utility {
                action,
                direction,
                request_id,
                ..
            } => {
                let Some(p) = self.players.first_mut() else {
                    return;
                };
                if request_id <= p.utility.last_request_id {
                    return;
                }
                p.utility.last_request_id = request_id;
                match action {
                    UtilityAction::Dash if p.utility.dash_remaining_secs <= 0.0 => {
                        let v = Vec2::from_array(direction);
                        if !v.is_finite() || v.length_squared() < 0.01 {
                            return;
                        }
                        // Use the same authored navigation as regular movement.
                        let target = Vec2::new(p.x, p.z) + v.normalize() * DASH_DISTANCE;
                        let target = shared::navigation::world_navigation()
                            .clip_movement([p.x, p.z], target.to_array());
                        p.x = target[0];
                        p.z = target[1];
                        p.utility.dash_sequence += 1;
                        p.utility.dash_remaining_secs = DASH_COOLDOWN_SECS;
                    }
                    UtilityAction::Haste if p.utility.haste_remaining_secs <= 0.0 => {
                        p.utility.haste_active_secs = HASTE_DURATION_SECS;
                        p.utility.haste_remaining_secs = HASTE_COOLDOWN_SECS;
                    }
                    _ => {}
                }
            }
            // No purchase, auth, signed career, network, or ranked result path exists here.
            _ => {}
        }
    }
    fn valid_target(&self, target: TargetId, range: f32) -> bool {
        target.kind == TargetKind::Player
            && self.players.iter().any(|p| {
                p.id == target.id
                    && p.id != LOCAL_ID
                    && p.hp > 0.0
                    && Vec2::new(p.x - self.players[0].x, p.z - self.players[0].z).length()
                        <= range + shared::PLAYER_TARGET_RADIUS
            })
    }
    fn attack(&mut self, target: TargetId, damage: f32, slot: Option<u8>) {
        let p = &mut self.players[0];
        if slot.is_none() {
            p.action_sequence += 1;
            p.action_kind = PlayerActionKind::Attack;
            p.action_slot = shared::BASIC_ATTACK_ACTION_SLOT;
        }
        self.sequence += 1;
        self.shots.push(Shot {
            state: ProjectileState {
                id: self.sequence,
                owner_id: LOCAL_ID,
                owner_team: Team::Green,
                source_kind: CombatEntityKind::Player,
                style: ProjectileStyle::for_class(p.hero_class),
                action_slot: slot,
                direction: [0.0; 3],
                x: p.x,
                y: p.y + 0.8,
                z: p.z,
            },
            target,
            damage,
        });
    }
    fn advance(&mut self, dt: f32) {
        self.elapsed += dt;
        for p in &mut self.players {
            for timer in [
                &mut p.basic_attack_remaining_secs,
                &mut p.skill_recovery_remaining_secs,
                &mut p.utility.dash_remaining_secs,
                &mut p.utility.haste_remaining_secs,
                &mut p.utility.haste_active_secs,
            ]
            .into_iter()
            .chain(p.skill_cooldown_remaining_secs.iter_mut())
            {
                *timer = (*timer - dt).max(0.0);
            }
            if p.id == LOCAL_ID {
                p.mana = (p.mana + 12.0 * dt).min(p.max_mana);
                continue;
            }
            if p.hp <= 0.0 {
                let timer = self.respawn.entry(p.id).or_insert(3.0);
                *timer -= dt;
                if *timer <= 0.0 {
                    p.hp = p.max_hp;
                    self.respawn.remove(&p.id);
                }
            } else {
                let [x, z] = shared::map::geometry().home;
                // The nearest target stands still for melee; others demonstrate running.
                if p.id > 2 {
                    let phase = self.elapsed * 0.9 + p.id as f32;
                    p.x = x + 11.0 + (p.id - 2) as f32 * 3.0 + phase.sin() * 2.0;
                    p.z = z + 11.0 + phase.cos() * 2.0;
                    p.yaw = phase.cos().atan2(-phase.sin());
                }
            }
        }
        let shots = std::mem::take(&mut self.shots);
        for mut shot in shots {
            let Some(p) = self
                .players
                .iter_mut()
                .find(|p| p.id == shot.target.id && p.hp > 0.0)
            else {
                continue;
            };
            let position = Vec3::new(shot.state.x, shot.state.y, shot.state.z);
            let target = Vec3::new(p.x, p.y + 0.8, p.z);
            let delta = target - position;
            if delta.length() <= 30.0 * dt + 0.3 {
                let amount = shot.damage.min(p.hp);
                p.hp -= amount;
                self.sequence += 1;
                self.events.push_back(CombatEvent {
                    id: self.sequence,
                    source: shared::combat::CombatEntity {
                        kind: CombatEntityKind::Player,
                        id: LOCAL_ID,
                    },
                    target: shared::combat::CombatEntity {
                        kind: CombatEntityKind::Player,
                        id: p.id,
                    },
                    amount,
                    x: p.x,
                    y: p.y + 0.8,
                    z: p.z,
                    style: shot.state.style,
                    action_slot: shot.state.action_slot,
                    killed: p.hp <= 0.0,
                });
                if self.events.len() > 32 {
                    self.events.pop_front();
                }
            } else {
                let direction = delta.normalize();
                let next = position + direction * 30.0 * dt;
                shot.state.x = next.x;
                shot.state.y = next.y;
                shot.state.z = next.z;
                shot.state.direction = direction.to_array();
                self.shots.push(shot);
            }
        }
    }
    fn snapshot(&mut self) -> ServerPacket {
        self.tick += 1;
        ServerPacket::Snapshot {
            meta: SnapshotMeta::new(u64::MAX, 1, self.tick),
            geometry_id: shared::map::GEOMETRY_ID.into(),
            map_profile: "verdant".into(),
            match_mode: "offline_practice".into(),
            join_error: self.error,
            your_id: LOCAL_ID,
            players: self.players.clone(),
            projectiles: self.shots.iter().map(|s| s.state.clone()).collect(),
            combat_events: self.events.iter().cloned().collect(),
            game_state: if self.players.is_empty() {
                GameState::Lobby
            } else {
                GameState::Running
            },
            sandbox: None,
            vision: None,
            forest_pickups: vec![],
            scoreboard: None,
            prematch: None,
            structures: vec![],
            minions: vec![],
            neutrals: vec![],
            team_buffs: vec![],
            rematch_in_secs: None,
            career: default(),
        }
    }
}

#[derive(Component)]
pub(super) struct PracticeBanner;
pub(super) fn setup_banner(mut commands: Commands) {
    commands.spawn((
        Text::new("OFFLINE PRACTICE · Level 6 · No rewards"),
        TextFont {
            font_size: 13.0,
            ..default()
        },
        TextColor(crate::ui_theme::GOLD),
        BackgroundColor(crate::ui_theme::PANEL),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(68.0),
            left: Val::Percent(35.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
            display: Display::None,
            ..default()
        },
        ZIndex(22),
        PracticeBanner,
        Name::new("OfflinePracticeBanner"),
    ));
}
pub(super) fn sync_banner(
    session: Res<ClientSession>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    mut banners: Query<&mut Node, With<PracticeBanner>>,
) {
    let visible = session.is_offline()
        && screen.is_some_and(|s| *s.get() == crate::frontend::AppScreen::InMatch);
    for mut node in &mut banners {
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn joined(class: HeroClass) -> Simulation {
        let mut sim = Simulation::default();
        sim.command(ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: default(),
            hero_class: class,
            avatar: Some("agnes".into()),
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        });
        sim
    }
    fn target() -> TargetId {
        TargetId {
            kind: TargetKind::Player,
            id: 2,
        }
    }
    #[test]
    fn every_class_can_move_cast_attack_and_recover_targets_without_io() {
        for class in HeroClass::ALL {
            let mut sim = joined(class);
            assert_eq!(sim.players.len(), 5);
            assert_eq!(sim.players[0].avatar.as_deref(), Some("agnes"));
            let (x, y, z) = (sim.players[1].x, sim.players[1].y, sim.players[1].z - 2.0);
            sim.command(ClientPacket::Transform {
                x,
                y,
                z,
                yaw: 1.0,
                dash_sequence: 0,
            });
            assert_eq!(sim.players[0].z, z);
            sim.command(ClientPacket::BasicAttack {
                target: target(),
                server_epoch: u64::MAX,
                match_id: 1,
                request_id: 1,
            });
            assert_eq!(sim.shots.len(), 1);
            // Duplicate requests and cooldown spam must not create extra attacks.
            sim.command(ClientPacket::BasicAttack {
                target: target(),
                server_epoch: u64::MAX,
                match_id: 1,
                request_id: 1,
            });
            assert_eq!(sim.shots.len(), 1);
            for _ in 0..30 {
                sim.advance(0.05);
            }
            assert!(sim.players[1].hp < sim.players[1].max_hp);
            assert!(sim.events.iter().any(|e| e.amount > 0.0));
            for slot in 0..4 {
                sim.players[0].mana = 100.0;
                sim.players[0].hp = 80.0;
                let old = sim.players[0].action_sequence;
                sim.command(ClientPacket::Cast {
                    target: target(),
                    slot,
                });
                assert!(
                    sim.players[0].action_sequence > old,
                    "class {class:?} slot {slot}"
                );
                for _ in 0..12 {
                    sim.advance(0.05);
                }
            }
            sim.players[1].hp = 0.0;
            for _ in 0..80 {
                sim.advance(0.05);
            }
            assert_eq!(sim.players[1].hp, sim.players[1].max_hp);
            let ServerPacket::Snapshot {
                career,
                game_state,
                match_mode,
                scoreboard,
                ..
            } = sim.snapshot()
            else {
                panic!()
            };
            assert_eq!(career, shared::career::CareerView::default());
            assert_eq!(game_state, GameState::Running);
            assert_eq!(match_mode, "offline_practice");
            assert!(scoreboard.is_none());
            sim.command(ClientPacket::Leave);
            assert!(sim.players.is_empty());
            assert!(sim.shots.is_empty());
        }
    }
    #[test]
    fn utilities_are_local_and_late_pre_dash_transforms_cannot_undo_dash() {
        let mut sim = joined(HeroClass::Ranger);
        let old_x = sim.players[0].x;
        let old_z = sim.players[0].z;
        sim.command(ClientPacket::Utility {
            action: UtilityAction::Dash,
            direction: [1.0, 0.0],
            server_epoch: u64::MAX,
            match_id: 1,
            request_id: 1,
        });
        assert!(sim.players[0].x > old_x);
        assert_eq!(sim.players[0].utility.dash_sequence, 1);
        sim.command(ClientPacket::Transform {
            x: old_x,
            y: 0.5,
            z: old_z,
            yaw: 0.0,
            dash_sequence: 0,
        });
        assert!(sim.players[0].x > old_x);
        sim.command(ClientPacket::Utility {
            action: UtilityAction::Haste,
            direction: [0.0; 2],
            server_epoch: u64::MAX,
            match_id: 1,
            request_id: 2,
        });
        assert!(sim.players[0].utility.movement_multiplier() > 1.0);
        sim.advance(HASTE_DURATION_SECS + 0.1);
        assert_eq!(sim.players[0].utility.movement_multiplier(), 1.0);
    }
    #[test]
    fn offline_session_never_reuses_online_channels_or_overwrites_saved_address() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(ClientSession {
            offline_return_addr: Some("127.0.0.1:49999".into()),
            ephemeral_endpoint: true,
            ..default()
        });
        app.insert_resource(ResolvedServerAddressForPrefs("127.0.0.1:49999".into()));
        app.add_systems(
            Startup,
            |mut commands: Commands, mut session: ResMut<ClientSession>| {
                spawn_network_transport(&mut commands, &mut session, ADDRESS.into())
            },
        );
        app.add_systems(Update, step);
        app.update();
        assert!(app.world().contains_resource::<LocalPractice>());
        assert_eq!(
            app.world().resource::<ResolvedServerAddressForPrefs>().0,
            "127.0.0.1:49999"
        );
        let channels = app.world().resource::<NetworkChannels>();
        let snapshot = channels
            .incoming
            .try_recv()
            .expect("local snapshot without a listener or worker thread");
        assert!(matches!(
            snapshot,
            ServerPacket::Snapshot {
                game_state: GameState::Lobby,
                ..
            }
        ));
    }
    #[test]
    fn unknown_or_store_only_avatars_cannot_trigger_download_or_admission() {
        assert!(!shipped_avatar(Some("not-bundled")));
        assert!(shipped_avatar(None));
        let mut sim = Simulation::default();
        sim.command(ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: default(),
            hero_class: HeroClass::Mage,
            avatar: Some("not-bundled".into()),
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        });
        assert!(sim.players.is_empty());
        assert_eq!(sim.error, Some(JoinRejection::AvatarNotAuthorized));
    }
}
