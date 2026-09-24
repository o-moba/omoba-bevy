//! Lifetime round accounting from accepted HP-loss receipts, independent of
//! connected endpoints and the short, repeated cosmetic event stream.
use shared::{
    career::{MAX_PARTICIPANTS, MatchStats, ParticipantResult},
    combat::{CombatEntityKind, CombatEvent},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const ASSIST_WINDOW: Duration = Duration::from_secs(10);

/// Only player identities enter the roster; another entity kind with the same
/// numeric wire ID cannot resolve to a participant through this key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PlayerId(u64);

struct FrozenParticipant {
    result: ParticipantResult,
    earned_gold: u32,
}

#[derive(Default)]
pub(crate) struct RoundLedger {
    participants: BTreeMap<PlayerId, FrozenParticipant>,
    /// Per-victim, per-attacker timestamps. At most 32*31 entries, regardless
    /// of match duration or hit frequency; cleared for each defeated victim.
    contributors: BTreeMap<PlayerId, BTreeMap<PlayerId, Instant>>,
    last_event_id: u64,
    started: bool,
    frozen: bool,
    /// Ids already logged as missing a row; bounded by the players seen.
    unregistered_reported: std::collections::BTreeSet<u64>,
}

impl RoundLedger {
    /// Start exactly once on a fresh CombatLog. A malformed/oversized roster
    /// leaves the original ledger unchanged; reconnects must use register.
    pub(crate) fn begin(&mut self, roster: Vec<ParticipantResult>) -> Result<(), &'static str> {
        if self.started || self.frozen {
            return Err("round statistics already started");
        }
        if roster.len() > MAX_PARTICIPANTS {
            return Err("round statistics exceed the 32 participant limit");
        }
        let mut next = Self {
            started: true,
            ..Default::default()
        };
        for participant in roster {
            if !next.register(participant)? {
                return Err("duplicate player in round statistics roster");
            }
        }
        *self = next;
        Ok(())
    }

    /// Late unranked admission is explicit. Re-registering an existing player
    /// never changes their match-time identity, loadout, team or accrued totals.
    pub(crate) fn register(
        &mut self,
        mut participant: ParticipantResult,
    ) -> Result<bool, &'static str> {
        if !self.started || self.frozen {
            return Err("round statistics are not accepting participants");
        }
        if participant.player_id == 0 {
            return Err("round statistics require a nonzero player id");
        }
        let id = PlayerId(participant.player_id);
        if self.participants.contains_key(&id) {
            return Ok(false);
        }
        if self.participants.len() >= MAX_PARTICIPANTS {
            return Err("round statistics exceed the 32 participant limit");
        }
        if let Some(profile_id) = &participant.profile_id
            && self
                .participants
                .values()
                .any(|other| other.result.profile_id.as_ref() == Some(profile_id))
        {
            return Err("a profile cannot occupy two round statistics seats");
        }
        participant.stats = MatchStats {
            final_level: participant.stats.final_level.max(1),
            ..Default::default()
        };
        // Progression and rating are awarded later by result finalization,
        // never inherited from an earlier result supplied as an identity DTO.
        participant.rating = None;
        participant.progression_xp_gained = 0;
        self.participants.insert(
            id,
            FrozenParticipant {
                result: participant,
                earned_gold: 0,
            },
        );
        Ok(true)
    }

    /// A hero hit involving a player without a row cannot be credited. Say so
    /// once per id in the match log instead of silently freezing the score.
    fn report_unregistered(&mut self, player_id: u64) {
        if self.unregistered_reported.insert(player_id) {
            println!(
                "MATCH_METRIC event=unregistered_participant player={player_id} rows={}",
                self.participants.len()
            );
        }
    }

    pub(crate) fn is_started(&self) -> bool {
        self.started
    }
    pub(crate) fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Connection and level are the only mutable roster fields during a round.
    /// A disconnected participant remains available for posthumous attribution.
    pub(crate) fn update_player(&mut self, player_id: u64, final_level: u32, disconnected: bool) {
        if self.frozen {
            return;
        }
        if let Some(participant) = self.participants.get_mut(&PlayerId(player_id)) {
            participant.result.stats.final_level =
                participant.result.stats.final_level.max(final_level);
            participant.result.disconnected = disconnected;
        }
    }

    pub(crate) fn update_earned_gold(&mut self, player_id: u64, earned_gold: u32) {
        if !self.frozen
            && let Some(participant) = self.participants.get_mut(&PlayerId(player_id))
        {
            participant.earned_gold = participant.earned_gold.max(earned_gold);
        }
    }

    pub(crate) fn live_scoreboard(&self) -> Option<shared::live_score::LiveScoreboard> {
        self.started.then(|| shared::live_score::LiveScoreboard {
            players: self
                .participants
                .values()
                .map(|participant| {
                    let p = &participant.result;
                    shared::live_score::LiveScorePlayer {
                        player_id: p.player_id,
                        nickname: p.nickname.clone(),
                        team: p.team,
                        hero_class: p.hero_class,
                        kills: p.stats.kills,
                        deaths: p.stats.deaths,
                        assists: p.stats.assists,
                        earned_gold: participant.earned_gold,
                        level: p.stats.final_level,
                        connected: !p.disconnected,
                    }
                })
                .collect(),
        })
    }

    pub(crate) fn snapshot(&self) -> Vec<ParticipantResult> {
        self.participants
            .values()
            .map(|participant| participant.result.clone())
            .collect()
    }

    /// Idempotent snapshot barrier. Hits, reconnects and level updates after
    /// this point cannot rewrite the result that is awaiting durable storage.
    pub(crate) fn freeze(&mut self) -> Vec<ParticipantResult> {
        self.frozen = true;
        self.contributors.clear();
        self.snapshot()
    }

    /// Only call with the server-assigned monotonically increasing receipt ID.
    /// Cosmetic retransmission/snapshot reads must never call this method.
    pub(crate) fn record(&mut self, now: Instant, event: &CombatEvent) {
        if !self.started || self.frozen || event.id == 0 || event.id <= self.last_event_id {
            return;
        }
        self.last_event_id = event.id;
        if !event.amount.is_finite() || event.amount <= 0.0 {
            return;
        }
        self.contributors.retain(|_, contributors| {
            contributors.retain(|_, at| now.saturating_duration_since(*at) <= ASSIST_WINDOW);
            !contributors.is_empty()
        });
        let source = (event.source.kind == CombatEntityKind::Player)
            .then_some(PlayerId(event.source.id))
            .filter(|id| self.participants.contains_key(id));
        let amount = f64::from(event.amount);
        match event.target.kind {
            CombatEntityKind::Player => {
                let victim = PlayerId(event.target.id);
                if event.source.kind == CombatEntityKind::Player && source.is_none() {
                    self.report_unregistered(event.source.id);
                }
                let Some(target) = self.participants.get(&victim) else {
                    self.report_unregistered(event.target.id);
                    return;
                };
                let victim_team = target.result.team;
                let hostile_source =
                    source.filter(|id| self.participants[id].result.team != victim_team);
                let target = self.participants.get_mut(&victim).expect("checked victim");
                target.result.stats.damage_taken += amount;
                if event.killed {
                    target.result.stats.deaths = target.result.stats.deaths.saturating_add(1);
                }
                if let Some(attacker) = hostile_source {
                    let stats = &mut self
                        .participants
                        .get_mut(&attacker)
                        .expect("checked source")
                        .result
                        .stats;
                    stats.damage_to_heroes += amount;
                    if event.killed {
                        stats.kills = stats.kills.saturating_add(1);
                    }
                    self.contributors
                        .entry(victim)
                        .or_default()
                        .insert(attacker, now);
                }
                // A minion/tower/neutral finishing blow earns no player
                // kill. Recent hostile contributors may still earn assists.
                if event.killed
                    && let Some(contributors) = self.contributors.remove(&victim)
                {
                    for (attacker, last_hit) in contributors {
                        if Some(attacker) != hostile_source
                            && now.saturating_duration_since(last_hit) <= ASSIST_WINDOW
                        {
                            let stats = &mut self
                                .participants
                                .get_mut(&attacker)
                                .expect("registered contributor")
                                .result
                                .stats;
                            stats.assists = stats.assists.saturating_add(1);
                        }
                    }
                }
            }
            CombatEntityKind::Minion | CombatEntityKind::Neutral | CombatEntityKind::Structure => {
                let Some(attacker) = source else {
                    return;
                };
                let stats = &mut self
                    .participants
                    .get_mut(&attacker)
                    .expect("checked source")
                    .result
                    .stats;
                match event.target.kind {
                    CombatEntityKind::Minion => {
                        stats.damage_to_creeps += amount;
                        if event.killed {
                            stats.minion_last_hits = stats.minion_last_hits.saturating_add(1);
                        }
                    }
                    CombatEntityKind::Neutral => {
                        stats.damage_to_creeps += amount;
                        if event.killed {
                            stats.jungle_last_hits = stats.jungle_last_hits.saturating_add(1);
                        }
                    }
                    CombatEntityKind::Structure => {
                        stats.damage_to_structures += amount;
                        if event.killed {
                            stats.structures_destroyed =
                                stats.structures_destroyed.saturating_add(1);
                        }
                    }
                    _ => unreachable!(),
                }
            }
            CombatEntityKind::Unknown => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{HeroClass, combat::CombatEntity, map::Team};

    fn participant(id: u64, team: Team) -> ParticipantResult {
        ParticipantResult {
            is_bot: false,
            player_id: id,
            profile_id: Some(format!("{id:064x}")),
            nickname: format!("Hero {id}"),
            team,
            hero_class: HeroClass::Mage,
            character: "cube".into(),
            avatar: None,
            sprite_character: None,
            stats: MatchStats::default(),
            disconnected: false,
            rating: None,
            progression_xp_gained: 0,
        }
    }
    fn roster() -> Vec<ParticipantResult> {
        vec![
            participant(1, Team::Green),
            participant(2, Team::Green),
            participant(3, Team::Blue),
        ]
    }
    fn ledger() -> RoundLedger {
        let mut ledger = RoundLedger::default();
        ledger.begin(roster()).unwrap();
        ledger
    }
    fn hit(
        id: u64,
        source_kind: CombatEntityKind,
        source: u64,
        target_kind: CombatEntityKind,
        target: u64,
        amount: f32,
        killed: bool,
    ) -> CombatEvent {
        CombatEvent {
            id,
            source: CombatEntity {
                kind: source_kind,
                id: source,
            },
            target: CombatEntity {
                kind: target_kind,
                id: target,
            },
            amount,
            killed,
            ..Default::default()
        }
    }
    fn stats(ledger: &RoundLedger, id: u64) -> MatchStats {
        ledger
            .snapshot()
            .into_iter()
            .find(|p| p.player_id == id)
            .unwrap()
            .stats
    }

    #[test]
    fn totals_survive_cosmetic_capacity_expiry_and_long_matches() {
        let now = Instant::now();
        let mut log = crate::combat_feedback::CombatLog::default();
        log.ledger.begin(roster()).unwrap();
        log.extend(
            now,
            (0..250).map(|_| {
                hit(
                    0,
                    CombatEntityKind::Player,
                    1,
                    CombatEntityKind::Minion,
                    9,
                    2.0,
                    false,
                )
            }),
        );
        assert_eq!(log.snapshot(now).len(), 96);
        assert_eq!(stats(&log.ledger, 1).damage_to_creeps, 500.0);
        assert!(log.snapshot(now + Duration::from_secs(2)).is_empty());
        log.extend(
            now + Duration::from_secs(3600),
            [hit(
                0,
                CombatEntityKind::Player,
                1,
                CombatEntityKind::Structure,
                1,
                7.0,
                true,
            )],
        );
        assert_eq!(stats(&log.ledger, 1).damage_to_creeps, 500.0);
        assert_eq!(stats(&log.ledger, 1).damage_to_structures, 7.0);
        assert_eq!(stats(&log.ledger, 1).structures_destroyed, 1);
        // Reading/repeating the cosmetic feed does not reapply any totals.
        let first = log.ledger.snapshot();
        for _ in 0..4 {
            assert_eq!(log.snapshot(now + Duration::from_secs(3600)).len(), 1);
        }
        assert_eq!(log.ledger.snapshot(), first);
    }

    #[test]
    fn typed_nonplayer_sources_cannot_steal_kills_or_last_hits() {
        let mut ledger = ledger();
        let now = Instant::now();
        for (index, kind) in [
            CombatEntityKind::Minion,
            CombatEntityKind::Structure,
            CombatEntityKind::Neutral,
            CombatEntityKind::Unknown,
        ]
        .into_iter()
        .enumerate()
        {
            ledger.record(
                now,
                &hit(
                    index as u64 + 1,
                    kind,
                    1,
                    CombatEntityKind::Player,
                    3,
                    20.0,
                    true,
                ),
            );
        }
        ledger.record(
            now,
            &hit(
                5,
                CombatEntityKind::Minion,
                1,
                CombatEntityKind::Minion,
                90,
                4.0,
                true,
            ),
        );
        ledger.record(
            now,
            &hit(
                6,
                CombatEntityKind::Structure,
                1,
                CombatEntityKind::Structure,
                90,
                4.0,
                true,
            ),
        );
        assert_eq!(
            stats(&ledger, 1),
            MatchStats {
                final_level: 1,
                ..Default::default()
            }
        );
        assert_eq!(stats(&ledger, 3).damage_taken, 80.0);
        assert_eq!(stats(&ledger, 3).deaths, 4);
    }

    #[test]
    fn accepted_overkill_amount_and_repeated_creep_ids_count_independent_lives() {
        let mut ledger = ledger();
        let now = Instant::now();
        ledger.record(
            now,
            &hit(
                1,
                CombatEntityKind::Player,
                1,
                CombatEntityKind::Player,
                3,
                3.5,
                true,
            ),
        );
        assert_eq!(stats(&ledger, 1).damage_to_heroes, 3.5);
        assert_eq!(stats(&ledger, 3).damage_taken, 3.5);
        assert_eq!(stats(&ledger, 1).kills, 1);
        for id in 2..=4 {
            ledger.record(
                now + Duration::from_secs(id * 40),
                &hit(
                    id,
                    CombatEntityKind::Player,
                    1,
                    CombatEntityKind::Neutral,
                    9001,
                    55.0,
                    true,
                ),
            );
        }
        ledger.record(
            now + Duration::from_secs(200),
            &hit(
                5,
                CombatEntityKind::Player,
                1,
                CombatEntityKind::Minion,
                9001,
                5.0,
                true,
            ),
        );
        assert_eq!(stats(&ledger, 1).jungle_last_hits, 3);
        assert_eq!(stats(&ledger, 1).minion_last_hits, 1);
        assert_eq!(stats(&ledger, 1).damage_to_creeps, 170.0);
    }

    #[test]
    fn assists_include_exact_ten_seconds_exclude_killer_and_clear_on_victim_death() {
        let now = Instant::now();
        let mut ledger = ledger();
        ledger.record(
            now,
            &hit(
                1,
                CombatEntityKind::Player,
                1,
                CombatEntityKind::Player,
                3,
                5.0,
                false,
            ),
        );
        ledger.record(
            now + ASSIST_WINDOW,
            &hit(
                2,
                CombatEntityKind::Player,
                2,
                CombatEntityKind::Player,
                3,
                5.0,
                true,
            ),
        );
        assert_eq!(stats(&ledger, 1).assists, 1);
        assert_eq!(stats(&ledger, 2).kills, 1);
        assert_eq!(stats(&ledger, 2).assists, 0);
        ledger.record(
            now + ASSIST_WINDOW,
            &hit(
                3,
                CombatEntityKind::Player,
                2,
                CombatEntityKind::Player,
                3,
                5.0,
                true,
            ),
        );
        assert_eq!(
            stats(&ledger, 1).assists,
            1,
            "old-life damage cannot grant another assist"
        );
        ledger.record(
            now + Duration::from_secs(20),
            &hit(
                4,
                CombatEntityKind::Player,
                1,
                CombatEntityKind::Player,
                3,
                5.0,
                false,
            ),
        );
        ledger.record(
            now + Duration::from_secs(30) + Duration::from_nanos(1),
            &hit(
                5,
                CombatEntityKind::Player,
                2,
                CombatEntityKind::Player,
                3,
                5.0,
                true,
            ),
        );
        assert_eq!(
            stats(&ledger, 1).assists,
            1,
            "older than ten seconds expires"
        );
    }

    #[test]
    fn environmental_finisher_grants_recent_hostile_assist_without_a_player_kill() {
        let now = Instant::now();
        let mut ledger = ledger();
        ledger.record(
            now,
            &hit(
                1,
                CombatEntityKind::Player,
                1,
                CombatEntityKind::Player,
                3,
                4.0,
                false,
            ),
        );
        ledger.record(
            now,
            &hit(
                2,
                CombatEntityKind::Structure,
                1,
                CombatEntityKind::Player,
                3,
                7.0,
                true,
            ),
        );
        assert_eq!(stats(&ledger, 1).assists, 1);
        assert_eq!(stats(&ledger, 1).kills, 0);
        assert_eq!(stats(&ledger, 3).deaths, 1);
        ledger.record(
            now,
            &hit(
                3,
                CombatEntityKind::Player,
                1,
                CombatEntityKind::Player,
                2,
                4.0,
                false,
            ),
        );
        ledger.record(
            now,
            &hit(
                4,
                CombatEntityKind::Neutral,
                1,
                CombatEntityKind::Player,
                2,
                7.0,
                true,
            ),
        );
        assert_eq!(
            stats(&ledger, 1).assists,
            1,
            "same-team hits cannot create assists"
        );
    }

    #[test]
    fn reconnect_preserves_match_identity_and_posthumous_damage_then_freeze_is_final() {
        let now = Instant::now();
        let mut ledger = ledger();
        ledger.record(
            now,
            &hit(
                1,
                CombatEntityKind::Player,
                1,
                CombatEntityKind::Player,
                3,
                4.0,
                false,
            ),
        );
        ledger.update_player(1, 5, true);
        let mut rejoin = participant(1, Team::Blue);
        rejoin.nickname = "Changed".into();
        rejoin.hero_class = HeroClass::Warrior;
        assert!(!ledger.register(rejoin).unwrap());
        ledger.record(
            now,
            &hit(
                2,
                CombatEntityKind::Player,
                1,
                CombatEntityKind::Minion,
                20,
                7.0,
                true,
            ),
        );
        let saved = ledger
            .snapshot()
            .into_iter()
            .find(|p| p.player_id == 1)
            .unwrap();
        assert_eq!(saved.nickname, "Hero 1");
        assert_eq!(saved.team, Team::Green);
        assert_eq!(saved.hero_class, HeroClass::Mage);
        assert_eq!(saved.stats.damage_to_heroes, 4.0);
        assert_eq!(saved.stats.minion_last_hits, 1);
        assert!(saved.disconnected);
        ledger.update_player(1, 1, false);
        assert_eq!(stats(&ledger, 1).final_level, 5);
        let frozen = ledger.freeze();
        assert!(ledger.is_frozen());
        ledger.record(
            now,
            &hit(
                3,
                CombatEntityKind::Player,
                1,
                CombatEntityKind::Player,
                3,
                99.0,
                true,
            ),
        );
        ledger.update_player(1, 20, true);
        assert!(ledger.register(participant(4, Team::Green)).is_err());
        assert_eq!(ledger.snapshot(), frozen);
        assert_eq!(ledger.freeze(), frozen);
        assert!(ledger.begin(roster()).is_err());
    }

    #[test]
    fn duplicate_receipts_invalid_damage_and_unregistered_sources_never_credit_players() {
        let now = Instant::now();
        let mut ledger = ledger();
        let accepted = hit(
            1,
            CombatEntityKind::Player,
            1,
            CombatEntityKind::Player,
            3,
            7.0,
            true,
        );
        ledger.record(now, &accepted);
        ledger.record(now, &accepted);
        for (index, amount) in [0.0, -1.0, f32::NAN, f32::INFINITY].into_iter().enumerate() {
            ledger.record(
                now,
                &hit(
                    index as u64 + 2,
                    CombatEntityKind::Player,
                    1,
                    CombatEntityKind::Player,
                    3,
                    amount,
                    true,
                ),
            );
        }
        ledger.record(
            now,
            &hit(
                6,
                CombatEntityKind::Player,
                99,
                CombatEntityKind::Player,
                3,
                3.0,
                true,
            ),
        );
        assert_eq!(stats(&ledger, 1).kills, 1);
        assert_eq!(stats(&ledger, 1).damage_to_heroes, 7.0);
        assert_eq!(stats(&ledger, 3).deaths, 2);
        assert_eq!(stats(&ledger, 3).damage_taken, 10.0);
    }

    #[test]
    fn roster_budget_and_duplicates_are_atomic_and_guest_seats_are_supported() {
        let mut ledger = RoundLedger::default();
        let too_many = (1..=33).map(|id| participant(id, Team::Green)).collect();
        assert!(ledger.begin(too_many).is_err());
        assert!(!ledger.is_started());
        assert!(ledger.snapshot().is_empty());
        assert!(
            ledger
                .begin(vec![
                    participant(1, Team::Green),
                    participant(1, Team::Green)
                ])
                .is_err()
        );
        let mut duplicate = participant(2, Team::Blue);
        duplicate.profile_id = participant(1, Team::Green).profile_id;
        assert!(
            ledger
                .begin(vec![participant(1, Team::Green), duplicate])
                .is_err()
        );
        assert!(!ledger.is_started());
        let guests = (1..=32)
            .map(|id| {
                let mut p = participant(id, Team::Green);
                p.profile_id = None;
                p
            })
            .collect();
        ledger.begin(guests).unwrap();
        assert_eq!(ledger.snapshot().len(), 32);
        assert!(ledger.register(participant(33, Team::Blue)).is_err());
        assert!(!ledger.register(participant(1, Team::Blue)).unwrap());
    }

    #[test]
    fn unstarted_log_ignores_pre_match_damage_and_participant_career_values_reset() {
        let now = Instant::now();
        let mut ledger = RoundLedger::default();
        ledger.record(
            now,
            &hit(
                1,
                CombatEntityKind::Player,
                1,
                CombatEntityKind::Player,
                3,
                10.0,
                true,
            ),
        );
        let mut prior = participant(1, Team::Green);
        prior.stats.kills = 99;
        prior.stats.final_level = 4;
        prior.progression_xp_gained = 999;
        ledger.begin(vec![prior]).unwrap();
        assert_eq!(stats(&ledger, 1).kills, 0);
        assert_eq!(stats(&ledger, 1).final_level, 4);
        assert_eq!(ledger.snapshot()[0].progression_xp_gained, 0);
    }
    #[test]
    fn live_score_uses_deduplicated_ledger_and_preserves_income_until_new_round() {
        let now = Instant::now();
        assert!(RoundLedger::default().live_scoreboard().is_none());
        let mut ledger = ledger();
        ledger.update_earned_gold(1, 250);
        ledger.update_earned_gold(1, 20);
        ledger.update_player(1, 8, true);
        let receipt = hit(
            1,
            CombatEntityKind::Player,
            1,
            CombatEntityKind::Player,
            3,
            100.0,
            true,
        );
        ledger.record(now, &receipt);
        ledger.record(now, &receipt);
        let rows = ledger.live_scoreboard().unwrap().players;
        assert_eq!(
            (
                rows[0].kills,
                rows[0].earned_gold,
                rows[0].level,
                rows[0].connected
            ),
            (1, 250, 8, false)
        );
        assert_eq!(rows[2].deaths, 1);
        ledger.update_player(1, 8, false);
        assert!(!ledger.register(participant(1, Team::Green)).unwrap());
        assert_eq!(
            ledger.live_scoreboard().unwrap().players[0].earned_gold,
            250
        );
        ledger.freeze();
        ledger.update_earned_gold(1, 999);
        assert_eq!(
            ledger.live_scoreboard().unwrap().players[0].earned_gold,
            250
        );
        ledger = RoundLedger::default();
        ledger.begin(roster()).unwrap();
        let p = &ledger.live_scoreboard().unwrap().players[0];
        assert_eq!(
            (p.kills, p.deaths, p.assists, p.earned_gold, p.level),
            (0, 0, 0, 0, 1)
        );
    }
}
