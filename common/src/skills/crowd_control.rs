//! Admission and application shared by skill roots, advanced stuns and Bluff.
//! Policies preserve existing exceptions: unstoppable blocks Bluff and forced
//! displacement, but does not grant immunity to ordinary roots/advanced stuns.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Root,
    Stun,
    Bluff,
    // Charm keeps its own movement target and historical player-mark policy.
    Charm,
}

/// Returns true only when a living, susceptible target received this control.
/// Rejected control cannot consume Brittle or interrupt recall.
pub(super) fn apply(
    w: &mut GameWorld,
    c: Candidate,
    id: u64,
    team: Team,
    root: f32,
    slow: f32,
    secs: f32,
    reveal: f32,
    now: Instant,
    kind: Kind,
) -> bool {
    if c.target.kind == TargetKind::Structure {
        return false;
    }
    if c.target.kind == TargetKind::Player {
        let Some(p) = w.players.values_mut().find(|p| {
            p.joined
                && p.hero.identity.id == c.target.id
                && p.hero.hp > 0.0
                && !p.modifiers.god_mode
        }) else {
            return false;
        };
        if remaining(p.hero.skills.advanced.parry_until, now) > 0.0 {
            p.hero.skills.advanced.parried_control |= root > 0.0;
            return false;
        }
        if p.hero.skills.advanced.immune(now)
            || (matches!(kind, Kind::Bluff)
                && remaining(p.hero.skills.advanced.unstoppable_until, now) > 0.0)
        {
            return false;
        }
        if root > 0.0 {
            // Recall's tick already cancels immobilized channels. Apply the same
            // rule at admission so no order within the tick can complete one.
            crate::recall::cancel(p);
        }
        if matches!(kind, Kind::Bluff) {
            p.hero.skills.control.stun_until = Some(
                p.hero
                    .skills
                    .control
                    .stun_until
                    .unwrap_or(now)
                    .max(now + duration(root)),
            );
        } else if matches!(kind, Kind::Charm) {
            p.hero.skills.control.root_until = Some(now + duration(root));
        } else {
            p.hero
                .skills
                .control
                .apply(id, root, slow, secs, reveal, now);
            if matches!(kind, Kind::Stun) && root > 0.0 {
                p.hero.skills.control.stun_until = Some(now + duration(root));
            }
        }
    } else {
        if !candidates(w).iter().any(|live| live.target == c.target) {
            return false;
        }
        let status = w
            .skill_runtime
            .npc_controls
            .entry(key(c.target))
            .or_default();
        status.apply(id, root, slow, secs, reveal, now);
        if matches!(kind, Kind::Stun | Kind::Bluff | Kind::Charm) && root > 0.0 {
            status.stun_until = Some(now + duration(root));
        }
        if reveal > 0.0 {
            let until = &mut status.revealed_to[usize::from(team == Team::Blue)];
            *until = Some(until.unwrap_or(now).max(now + duration(reveal)));
        }
    }
    // Player charm historically does not consume the mark; NPC charm does.
    // Bluff now follows the admitted immobilization path and consumes it once.
    if root > 0.0 && (!matches!(kind, Kind::Charm) || c.target.kind != TargetKind::Player) {
        advanced::consume_brittle(w, c, now);
    }
    true
}
