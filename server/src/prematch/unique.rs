//! Unique team picks at the existing draft/loadout boundary.
use super::*;
use shared::{HeroClass, map::Team, wire::CharacterChoice};

impl ServerRuntime {
    pub(super) fn prematch_class_taken(&self, addr: SocketAddr, action: &PrematchAction) -> bool {
        if self.prematch.phase != Some(PrematchPhase::Draft) {
            return false;
        }
        let Some(own) = self.world.players.get(&addr) else {
            return false;
        };
        let class = match action {
            PrematchAction::Select { hero_class, .. } if !own.draft.locked => *hero_class,
            PrematchAction::Lock { locked: true } => own.hero.identity.hero_class,
            _ => return false,
        };
        self.world.players.iter().any(|(other, p)| {
            *other != addr
                && p.joined
                && !p.hero.identity.is_bot
                && p.draft.locked
                && p.hero.identity.team == own.hero.identity.team
                && p.hero.identity.hero_class == class
        })
    }

    /// Valid human locks win; other humans keep their choice when available,
    /// then bots take the remaining kits. This also handles the expiry path.
    pub(super) fn finalize_unique_draft(&mut self, now: Instant) -> bool {
        if self.match_started_at.is_some() || self.career_roster_is_frozen() {
            return true;
        }
        let mut players: Vec<_> = self
            .world
            .players
            .iter()
            .filter(|(_, p)| p.joined)
            .map(|(addr, p)| {
                (
                    (
                        if p.hero.identity.is_bot {
                            2
                        } else if p.draft.locked {
                            0
                        } else {
                            1
                        },
                        p.hero.identity.id,
                    ),
                    *addr,
                )
            })
            .collect();
        players.sort_unstable_by_key(|(priority, _)| *priority);
        let mut used = [Vec::<HeroClass>::new(), Vec::new()];
        for (_, addr) in players {
            let p = self.world.players.get_mut(&addr).unwrap();
            let team = match p.hero.identity.team {
                Team::Green => 0,
                Team::Blue => 1,
            };
            let before = p.hero.identity.hero_class;
            let class = if !used[team].contains(&before) {
                Some(before)
            } else {
                HeroClass::ALL.into_iter().find(|class| {
                    !used[team].contains(class)
                        && (!class.is_standard()
                            || (p.framed_snapshots && p.protocol_compatible)
                            || p.hero.identity.is_bot)
                })
            };
            let Some(class) = class else {
                p.draft.error = Some("hero_taken".into());
                return false;
            };
            used[team].push(class);
            if class != before {
                if p.draft.role == before.primary_role() {
                    p.draft.role = class.primary_role();
                }
                p.hero.identity.hero_class = class;
                // Automatic fallback selects free canonical equipment, never an
                // unverified avatar/weapon from another pending selection.
                p.hero.identity.character = CharacterChoice::Ipfs;
                p.hero.identity.avatar = None;
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
        true
    }

    /// The public formation path also covers clients that omit draft opt-in.
    /// Already persisted/played allocations are not reinterpreted on recovery.
    pub(crate) fn prepare_public_unique_loadouts(&mut self, now: Instant) -> bool {
        if self.match_service.worker().is_none()
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
