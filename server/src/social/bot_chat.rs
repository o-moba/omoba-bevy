//! Short, authored replies to public match events from server-owned seats.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use common::{entities::ConnectedPlayer, match_stats::RoundLedger};
use shared::{
    bot_personas::BotPersonaId,
    map::Team,
    social::{BotChatTranslation, SocialChannel, SocialEventKind},
    wire::GameState,
};

use super::{ServerRuntime, social_team};

mod voice;
use voice::phrase;

const MIN_INTERVAL: Duration = Duration::from_secs(24);
const SPEAKER_INTERVAL: Duration = Duration::from_secs(65);
const MAX_RUNNING_LINES: u8 = 7;
const MAX_LATE: Duration = Duration::from_secs(3);
const KICKOFF_WINDOW: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(super) struct BotChatRuntime {
    latest_kill: u64,
    kickoff_seen: bool,
    end_seen: bool,
    last_line: Option<Instant>,
    last_speaker: HashMap<u64, Instant>,
    running_lines: u8,
    pending: Option<PendingLine>,
}

struct PendingLine {
    speaker_id: u64,
    persona: BotPersonaId,
    team: Team,
    at: Instant,
    ru: String,
    translations: BotChatTranslation,
    closing: bool,
}

struct Round<'a> {
    seed: u64,
    started: Instant,
    state: &'a GameState,
    ledger: &'a RoundLedger,
    players: &'a HashMap<SocketAddr, ConnectedPlayer>,
}

enum Cue<'a> {
    Kickoff,
    HumanKill { team: Team, nickname: &'a str },
    End { winner: Team },
}

impl ServerRuntime {
    pub(super) fn tick_bot_chat(&mut self, now: Instant) {
        let Some(started) = self.match_started_at else {
            return;
        };
        if self.match_service.worker().is_none() {
            return;
        }
        let round = Round {
            seed: self.server_epoch ^ self.match_id.rotate_left(17),
            started,
            state: &self.world.game_state,
            ledger: &self.combat_log.ledger,
            players: &self.world.players,
        };
        if let Some(line) = self.social.bot_chat.tick(round, now) {
            self.social.append_event(
                line.speaker_id,
                line.persona.nickname().into(),
                social_team(line.team),
                SocialEventKind::Chat {
                    channel: SocialChannel::Match,
                    text: line.ru,
                },
                Some(line.translations),
                now,
            );
        }
    }
}

impl BotChatRuntime {
    fn tick(&mut self, round: Round<'_>, now: Instant) -> Option<PendingLine> {
        let last_kill = self.latest_kill;
        if let Some(kill) = round.ledger.public_kills().last() {
            self.latest_kill = kill.event_id;
        }

        let closing = matches!(round.state, GameState::Victory { .. });
        if !matches!(round.state, GameState::Running | GameState::Victory { .. }) {
            self.pending = None;
            return None;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|line| line.closing != closing)
        {
            self.pending = None;
        }
        // Retained human seats controlled by AI have internal addresses. They
        // are neither an audience nor eligible for a congratulatory line.
        let connected = |addr: &SocketAddr, player: &ConnectedPlayer| {
            player.joined && !crate::bots::is_bot_address(*addr) && !player.hero.identity.is_bot
        };
        if !round
            .players
            .iter()
            .any(|(addr, player)| connected(addr, player))
        {
            self.pending = None;
            return None;
        }
        if self.pending.as_ref().is_some_and(|line| now >= line.at) {
            let line = self.pending.take().expect("a pending line was due");
            if now.saturating_duration_since(line.at) <= MAX_LATE
                && round.players.values().any(|player| {
                    player.joined
                        && player.hero.identity.is_bot
                        && player.hero.identity.id == line.speaker_id
                        && player.bot_persona == Some(line.persona)
                })
            {
                self.last_line = Some(now);
                self.last_speaker.insert(line.speaker_id, now);
                self.running_lines += u8::from(!line.closing);
                return Some(line);
            }
        }

        // Advance event cursors before the cooldown check. A busy fight can't
        // build up a queue of old kills to congratulate after it has ended.
        let cue = if let GameState::Victory { winner } = round.state {
            if std::mem::replace(&mut self.end_seen, true) {
                return None;
            }
            Some((0, Cue::End { winner: *winner }))
        } else {
            let kickoff_at = round.started + Duration::from_secs(7 + mix(round.seed) % 5);
            let kickoff_due = !self.kickoff_seen && now >= kickoff_at;
            self.kickoff_seen |= kickoff_due;
            if self.running_lines >= MAX_RUNNING_LINES {
                return None;
            }
            round
                .ledger
                .public_kills()
                .iter()
                .rev()
                .take_while(|kill| kill.event_id > last_kill)
                .find_map(|kill| {
                    let killer = round.ledger.participant(kill.killer_id)?;
                    let live_human = !killer.is_bot
                        && round.players.iter().any(|(addr, player)| {
                            player.hero.identity.id == kill.killer_id && connected(addr, player)
                        });
                    live_human.then_some((
                        kill.event_id,
                        Cue::HumanKill {
                            team: killer.team,
                            nickname: &killer.nickname,
                        },
                    ))
                })
                .or_else(|| {
                    (kickoff_due && now.saturating_duration_since(kickoff_at) <= KICKOFF_WINDOW)
                        .then_some((0, Cue::Kickoff))
                })
        };
        if self.pending.is_some()
            || self
                .last_line
                .is_some_and(|last| now.saturating_duration_since(last) < MIN_INTERVAL)
        {
            return None;
        }
        let (event_id, cue) = cue?;
        let seed = mix(round.seed ^ event_id.rotate_left(21) ^ u64::from(closing));
        let (speaker, persona) = round
            .players
            .values()
            .filter_map(|player| {
                let persona = player.bot_persona?;
                let identity = &player.hero.identity;
                let teammate = match cue {
                    Cue::HumanKill { team, .. } => identity.team == team,
                    _ => true,
                };
                let rested = self
                    .last_speaker
                    .get(&identity.id)
                    .is_none_or(|last| now.saturating_duration_since(*last) >= SPEAKER_INTERVAL);
                (player.joined && identity.is_bot && teammate && rested)
                    .then_some((identity, persona))
            })
            .min_by_key(|(identity, _)| {
                let prefer_winning_team =
                    matches!(cue, Cue::End { winner } if identity.team != winner);
                (prefer_winning_team, mix(seed ^ identity.id))
            })?;
        let (ru, translations) = phrase(persona, &cue, mix(seed ^ speaker.id))?;
        self.pending = Some(PendingLine {
            speaker_id: speaker.id,
            persona,
            team: speaker.team,
            at: now + Duration::from_millis(850 + seed % 1_400),
            ru,
            translations,
            closing,
        });
        None
    }
}

fn mix(input: u64) -> u64 {
    // SplitMix64 finalizer for repeatable speaker choice and timing. No shared
    // RNG state changes when a player connects or a UDP receipt is retried.
    let mixed = (input ^ (input >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^ (mixed >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::game_world::GameWorld;
    use shared::{
        career::{MatchStats, ParticipantResult},
        combat::{CombatEntity, CombatEntityKind, CombatEvent},
    };
    use std::net::{Ipv4Addr, Ipv6Addr};

    struct Fixture {
        now: Instant,
        world: GameWorld,
        ledger: RoundLedger,
    }

    impl Fixture {
        fn new() -> Self {
            let now = Instant::now();
            let mut world = GameWorld::new(Default::default(), now);
            let mut participants = Vec::new();
            for (port, team, persona, ai_controlled, name) in [
                (1, Team::Green, None, false, "Игрок#1234"),
                (2, Team::Blue, None, false, "ДругойИгрок"),
                (3, Team::Green, Some(BotPersonaId::Sever), true, "Север"),
                (4, Team::Blue, Some(BotPersonaId::Sova), true, "Сова"),
                (5, Team::Green, None, true, "Отключённый"),
            ] {
                let addr = if ai_controlled {
                    SocketAddr::from((Ipv6Addr::UNSPECIFIED, port))
                } else {
                    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
                };
                world.ensure_connected(addr, now);
                let player = world.players.get_mut(&addr).unwrap();
                player.joined = true;
                player.bot_persona = persona;
                player.hero.identity.is_bot = persona.is_some();
                player.hero.identity.team = team;
                participants.push(ParticipantResult {
                    input_kind: Default::default(),
                    participation: None,
                    player_id: player.hero.identity.id,
                    is_bot: persona.is_some(),
                    bot_persona: persona.map(|persona| persona.slug().into()),
                    profile_id: None,
                    nickname: name.into(),
                    team,
                    hero_class: Default::default(),
                    character: "ipfs".into(),
                    avatar: None,
                    sprite_character: None,
                    stats: MatchStats::default(),
                    disconnected: ai_controlled && persona.is_none(),
                    rating: None,
                    progression_xp_gained: 0,
                });
            }
            let mut ledger = RoundLedger::default();
            ledger.begin(participants).unwrap();
            world.game_state = GameState::Running;
            Self { now, world, ledger }
        }

        fn kill(&mut self, event_id: u64, killer_port: u16, victim_port: u16) {
            let id = |port| {
                self.world
                    .players
                    .iter()
                    .find(|(addr, _)| addr.port() == port)
                    .unwrap()
                    .1
                    .hero
                    .identity
                    .id
            };
            let player = |port| CombatEntity {
                kind: CombatEntityKind::Player,
                id: id(port),
            };
            self.ledger.record(
                self.now,
                &CombatEvent {
                    id: event_id,
                    source: player(killer_port),
                    target: player(victim_port),
                    amount: 10.0,
                    killed: true,
                    ..Default::default()
                },
            );
        }

        fn tick(&self, chat: &mut BotChatRuntime, offset: u64) -> Option<PendingLine> {
            chat.tick(
                Round {
                    seed: 23,
                    started: self.now,
                    state: &self.world.game_state,
                    ledger: &self.ledger,
                    players: &self.world.players,
                },
                self.now + Duration::from_secs(offset),
            )
        }
    }

    #[test]
    fn public_human_kill_uses_one_teammate_and_cooldown_drops_the_backlog() {
        let mut fixture = Fixture::new();
        let mut chat = BotChatRuntime::default();
        fixture.kill(1, 1, 2);
        assert!(fixture.tick(&mut chat, 12).is_none());
        assert!(fixture.tick(&mut chat, 12).is_none());
        let spoken = fixture.tick(&mut chat, 15).unwrap();
        assert_eq!(spoken.persona, BotPersonaId::Sever);
        assert_eq!(spoken.team, Team::Green);
        assert!(spoken.ru.contains("Игрок#1234"));
        assert!(spoken.translations.en.contains("Игрок#1234"));
        assert!(spoken.translations.zh_hans.contains("Игрок#1234"));
        assert!(!spoken.ru.contains("ДругойИгрок"));
        fixture.kill(2, 2, 1);
        assert!(fixture.tick(&mut chat, 16).is_none());
        assert!(fixture.tick(&mut chat, 50).is_none());
        assert!(chat.pending.is_none());
        fixture.kill(3, 2, 1);
        assert!(fixture.tick(&mut chat, 51).is_none());
        let spoken = fixture.tick(&mut chat, 54).unwrap();
        assert_eq!(spoken.persona, BotPersonaId::Sova);
        assert!(spoken.ru.contains("ДругойИгрок"));
    }

    #[test]
    fn kickoff_and_victory_are_single_lines_and_ai_kills_do_not_trigger_praise() {
        let mut fixture = Fixture::new();
        let mut chat = BotChatRuntime::default();
        assert!(fixture.tick(&mut chat, 7).is_none());
        assert!(!fixture.tick(&mut chat, 10).unwrap().closing);
        fixture.kill(1, 3, 2);
        fixture.kill(2, 5, 2);
        assert!(fixture.tick(&mut chat, 90).is_none());
        assert!(chat.pending.is_none());

        fixture.world.game_state = GameState::Victory { winner: Team::Blue };
        assert!(fixture.tick(&mut chat, 91).is_none());
        let spoken = fixture.tick(&mut chat, 94).unwrap();
        assert!(spoken.closing);
        assert_eq!(spoken.persona, BotPersonaId::Sova);
        assert!(fixture.tick(&mut chat, 200).is_none());
        assert!(chat.pending.is_none());
    }

    #[test]
    fn pending_kill_is_discarded_when_the_game_ends() {
        let mut fixture = Fixture::new();
        let mut chat = BotChatRuntime::default();
        fixture.kill(1, 1, 2);
        assert!(fixture.tick(&mut chat, 12).is_none());
        fixture.world.game_state = GameState::Victory { winner: Team::Blue };
        assert!(fixture.tick(&mut chat, 13).is_none());
        let spoken = fixture.tick(&mut chat, 16).unwrap();
        assert!(spoken.closing);
        assert_eq!(spoken.team, Team::Blue);
        assert!(!spoken.ru.contains("Игрок#1234"));
    }

    #[test]
    fn returning_player_does_not_trigger_an_old_kickoff() {
        let mut fixture = Fixture::new();
        let mut chat = BotChatRuntime::default();
        let green = SocketAddr::from((Ipv4Addr::LOCALHOST, 1));
        let blue = SocketAddr::from((Ipv4Addr::LOCALHOST, 2));
        let returning = fixture.world.players.remove(&green).unwrap();
        fixture.world.players.remove(&blue).unwrap();
        assert!(fixture.tick(&mut chat, 12).is_none());

        fixture.world.players.insert(green, returning);
        assert!(fixture.tick(&mut chat, 60).is_none());
        assert!(fixture.tick(&mut chat, 63).is_none());
        assert!(chat.kickoff_seen);
        assert!(chat.pending.is_none());
    }

    #[test]
    fn each_persona_has_valid_distinct_localized_voices() {
        let mut greetings = std::collections::HashSet::new();
        for persona in BotPersonaId::ALL {
            let (greeting, _) = phrase(persona, &Cue::Kickoff, 0).unwrap();
            assert!(greetings.insert(greeting));
            let kill = Cue::HumanKill {
                team: Team::Green,
                nickname: "Поддержка#1234",
            };
            let (first, _) = phrase(persona, &kill, 0).unwrap();
            let (second, _) = phrase(persona, &kill, 1).unwrap();
            assert_ne!(first, second);
            assert!(first.contains("Поддержка#1234"));
            assert!(
                phrase(
                    persona,
                    &Cue::End {
                        winner: Team::Green
                    },
                    0
                )
                .is_some()
            );
        }
    }
}
