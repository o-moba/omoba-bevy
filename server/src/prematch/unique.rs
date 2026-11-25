//! Hero class reservations and final roster assignment.
use super::*;
use shared::{HeroClass, map::Team, wire::CharacterChoice};

impl ServerRuntime {
    pub(crate) fn unique_classes_across_teams(&self) -> bool {
        self.match_service.worker().map_or_else(
            || self.career_queue_enabled(),
            |worker| worker.manifest.preference.rated(),
        )
    }

    pub(super) fn prematch_class_conflict(
        &self,
        addr: SocketAddr,
        action: &PrematchAction,
    ) -> Option<&'static str> {
        if self.prematch.phase != Some(PrematchPhase::Draft) {
            return None;
        }
        let own = self.world.players.get(&addr)?;
        let class = match action {
            PrematchAction::Select { hero_class, .. } if !own.draft.locked => *hero_class,
            PrematchAction::Lock { locked: true } => own.hero.identity.hero_class,
            _ => return None,
        };
        let match_wide = self.unique_classes_across_teams();
        self.world
            .players
            .iter()
            .any(|(other, p)| {
                *other != addr
                    && p.joined
                    && !p.hero.identity.is_bot
                    && p.draft.locked
                    && shared::prematch::hero_pick_conflicts(
                        match_wide,
                        own.hero.identity.team,
                        class,
                        p.hero.identity.team,
                        p.hero.identity.hero_class,
                    )
            })
            .then_some(class_taken_code(match_wide))
    }

    /// Valid human locks win; other humans keep their choice when available,
    /// then bots take the remaining kits. This also handles the expiry path.
    pub(super) fn finalize_unique_draft(&mut self, now: Instant) -> bool {
        if self.match_started_at.is_some() || self.career_roster_is_frozen() {
            return true;
        }
        let match_wide = self.unique_classes_across_teams();
        let mut players: Vec<_> = self
            .world
            .players
            .iter()
            .filter(|(_, p)| p.joined)
            .map(|(addr, p)| {
                (
                    (
                        if p.hero.identity.is_bot {
                            3
                        } else if p.draft.locked {
                            0
                        } else if match_wide && !supports_standard_classes(p) {
                            // Fill the restricted legacy slots before flexible picks.
                            1
                        } else {
                            2
                        },
                        p.hero.identity.id,
                    ),
                    *addr,
                )
            })
            .collect();
        players.sort_unstable_by_key(|(priority, _)| *priority);
        let mut used = [Vec::<HeroClass>::new(), Vec::new()];
        let mut assignments = Vec::with_capacity(players.len());
        for (_, addr) in players {
            let p = &self.world.players[&addr];
            let team = match p.hero.identity.team {
                Team::Green => 0,
                Team::Blue => 1,
            };
            let before = p.hero.identity.hero_class;
            let available = |class: &HeroClass| {
                !used[team].contains(class)
                    && (!match_wide || !used[1 - team].contains(class))
                    && (!class.is_standard() || supports_standard_classes(p))
            };
            let globally_available =
                |class: &HeroClass| available(class) && !used[1 - team].contains(class);
            let class = if p.hero.identity.is_bot {
                let practice_duty = self.rules.fills_with_bots
                    && !self.bots.sandbox
                    && self.bots.kind(addr) == Some(crate::bots::BotKind::Lane);
                if globally_available(&before)
                    && (!practice_duty || before.primary_role() == p.draft.role)
                {
                    Some(before)
                } else {
                    HeroClass::ALL
                        .into_iter()
                        .find(|class| {
                            globally_available(class) && class.primary_role() == p.draft.role
                        })
                        .or_else(|| globally_available(&before).then_some(before))
                        .or_else(|| HeroClass::ALL.into_iter().find(globally_available))
                        .or_else(|| available(&before).then_some(before))
                        .or_else(|| HeroClass::ALL.into_iter().find(available))
                }
            } else if available(&before) {
                Some(before)
            } else {
                HeroClass::ALL.into_iter().find(available)
            };
            let Some(class) = class else {
                self.world.players.get_mut(&addr).unwrap().draft.error =
                    Some(class_taken_code(match_wide).into());
                return false;
            };
            used[team].push(class);
            assignments.push((addr, class));
        }
        // A legacy roster can run out of compatible classes. Commit the picks
        // together so that a failed plan never rewrites accepted human choices.
        for (addr, class) in assignments {
            let p = self.world.players.get_mut(&addr).unwrap();
            let before = p.hero.identity.hero_class;
            if class != before {
                if !p.hero.identity.is_bot && p.draft.role == before.primary_role() {
                    p.draft.role = class.primary_role();
                }
                p.hero.identity.hero_class = class;
                // Automatic fallback selects free canonical equipment, never an
                // unverified avatar/weapon from another pending selection.
                p.hero.identity.character = CharacterChoice::Ipfs;
                p.hero.identity.avatar = Some(class.default_avatar_slug().to_owned());
                p.hero.identity.sprite_character =
                    Some(shared::normalize_sprite_character_id(None).to_owned());
                p.hero.identity.handheld = Default::default();
                let modifiers = p.modifiers;
                reset_player_round(p, &self.world.map_layout, now);
                p.modifiers = modifiers;
                p.hero.max_hp = common::hero_stats::max_hp(p);
                p.hero.hp = p.hero.max_hp;
                p.hero.max_mana = common::hero_stats::max_mana(p);
                p.hero.mana = p.hero.max_mana;
                p.draft.loaded = false;
                if !p.hero.identity.is_bot {
                    p.draft.error = Some("hero_auto_picked".into());
                }
            }
            p.draft.locked = true;
        }
        self.combat_host().refresh_practice_bot_duties(now);
        true
    }

    /// Career formation also covers clients that omit draft opt-in.
    /// Already persisted/played allocations are not reinterpreted on recovery.
    pub(crate) fn prepare_unique_loadouts(&mut self, now: Instant) -> bool {
        if (self.match_service.worker().is_none() && !self.career_queue_enabled())
            || self.career_roster_is_frozen()
            || self.match_started_at.is_some()
        {
            return true;
        }
        if !self.finalize_unique_draft(now) {
            return false;
        }
        self.prematch.phase != Some(PrematchPhase::Loading)
            || self
                .world
                .players
                .values()
                .filter(|p| p.joined && !p.hero.identity.is_bot && p.draft.capable)
                .all(|p| p.draft.loaded)
    }
}

fn supports_standard_classes(player: &ConnectedPlayer) -> bool {
    player.hero.identity.is_bot || (player.framed_snapshots && player.protocol_compatible)
}

fn class_taken_code(match_wide: bool) -> &'static str {
    if match_wide {
        "hero_taken_ranked"
    } else {
        "hero_taken"
    }
}
