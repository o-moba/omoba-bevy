use super::*;

pub(crate) fn neutral_template(camp_type: NeutralCampType) -> NeutralTemplate {
    match camp_type {
        NeutralCampType::Skirmisher => NeutralTemplate {
            max_hp: SKIRMISHER_MAX_HP,
            attack_damage: SKIRMISHER_ATTACK_DAMAGE,
            attack_range: SKIRMISHER_ATTACK_RANGE,
            kill_gold: SKIRMISHER_KILL_GOLD,
            kill_xp: SKIRMISHER_KILL_XP,
        },
        NeutralCampType::Bruiser => NeutralTemplate {
            max_hp: BRUISER_MAX_HP,
            attack_damage: BRUISER_ATTACK_DAMAGE,
            attack_range: BRUISER_ATTACK_RANGE,
            kill_gold: BRUISER_KILL_GOLD,
            kill_xp: BRUISER_KILL_XP,
        },
        NeutralCampType::Spitter => NeutralTemplate {
            max_hp: SPITTER_MAX_HP,
            attack_damage: SPITTER_ATTACK_DAMAGE,
            attack_range: SPITTER_ATTACK_RANGE,
            kill_gold: SPITTER_KILL_GOLD,
            kill_xp: SPITTER_KILL_XP,
        },
        NeutralCampType::WendigoBoss => NeutralTemplate {
            max_hp: WENDIGO_MAX_HP,
            attack_damage: WENDIGO_ATTACK_DAMAGE,
            attack_range: WENDIGO_ATTACK_RANGE,
            kill_gold: WENDIGO_KILL_GOLD,
            kill_xp: WENDIGO_KILL_XP,
        },
        NeutralCampType::KingMutatioBoss => NeutralTemplate {
            max_hp: MUTATIO_MAX_HP,
            attack_damage: MUTATIO_ATTACK_DAMAGE,
            attack_range: MUTATIO_ATTACK_RANGE,
            kill_gold: MUTATIO_KILL_GOLD,
            kill_xp: MUTATIO_KILL_XP,
        },
    }
}

/// Time from match start (Lobby -> Running) until this neutral first spawns.
/// `None` = present from match start (regular camps).
pub(crate) fn boss_spawn_delay(camp_type: NeutralCampType) -> Option<Duration> {
    match camp_type {
        NeutralCampType::WendigoBoss => Some(BOTTOM_BOSS_SPAWN_DELAY),
        NeutralCampType::KingMutatioBoss => Some(TOP_BOSS_SPAWN_DELAY),
        _ => None,
    }
}

/// Per-type respawn cooldown after death (bosses respawn slower than camps).
pub(crate) fn neutral_respawn_cooldown(camp_type: NeutralCampType) -> Duration {
    if camp_type.is_boss() {
        BOSS_RESPAWN_COOLDOWN
    } else {
        NEUTRAL_RESPAWN_COOLDOWN
    }
}

/// Per-type aggro/leash radii; bosses hold a larger leash around their pit.
pub(crate) fn neutral_aggro_and_leash(camp_type: NeutralCampType) -> (f32, f32) {
    if camp_type.is_boss() {
        (BOSS_AGGRO_RADIUS, BOSS_LEASH_DISTANCE)
    } else {
        (NEUTRAL_AGGRO_RADIUS, NEUTRAL_LEASH_DISTANCE)
    }
}

/// Full map extent derived exactly like `world::build_map_layout` (see the
/// jungle ring formula used for camp anchors).
fn jungle_map_size() -> f32 {
    let inner_side = TARGET_BASE_DISTANCE / 2.0_f32.sqrt();
    let half_inner_side = inner_side * 0.5;
    let base_padding = BASE_PAD_SIZE * 0.5 + BASE_EDGE_MARGIN;
    let half_map_size = half_inner_side + base_padding;
    half_map_size * 2.0
}

pub(crate) fn jungle_camp_blueprints() -> Vec<(Vec3f, NeutralCampType)> {
    let map_size = jungle_map_size();
    let jungle_outer = map_size * JUNGLE_MAP_OUTER_FRAC;
    let jungle_inner = map_size * JUNGLE_MAP_INNER_FRAC;
    let y = NEUTRAL_SPAWN_HEIGHT;
    vec![
        (
            Vec3f::new(-jungle_outer, y, jungle_inner),
            NeutralCampType::Skirmisher,
        ),
        (
            Vec3f::new(jungle_outer, y, -jungle_inner),
            NeutralCampType::Bruiser,
        ),
        (
            Vec3f::new(-jungle_inner, y, -jungle_outer),
            NeutralCampType::Spitter,
        ),
    ]
}

/// Boss pit anchors: 180-degree rotationally symmetric points (team-fair) in
/// the bottom-lane region (Wendigo) and the top-lane region (King Mutatio),
/// clear of the three camp slots, lanes, and towers.
pub(crate) fn boss_blueprints() -> Vec<(Vec3f, NeutralCampType)> {
    let map_size = jungle_map_size();
    let boss_outer = map_size * BOSS_PIT_OUTER_FRAC;
    let boss_inner = map_size * BOSS_PIT_INNER_FRAC;
    let y = NEUTRAL_SPAWN_HEIGHT;
    vec![
        (
            Vec3f::new(boss_inner, y, -boss_outer),
            NeutralCampType::WendigoBoss,
        ),
        (
            Vec3f::new(-boss_inner, y, boss_outer),
            NeutralCampType::KingMutatioBoss,
        ),
    ]
}

pub(crate) fn build_neutral_camps(next_id: &mut u64) -> HashMap<u64, Neutral> {
    let mut out = HashMap::new();
    for (anchor, camp_type) in jungle_camp_blueprints() {
        let template = neutral_template(camp_type);
        let id = *next_id;
        *next_id += 1;
        out.insert(
            id,
            Neutral {
                state: NeutralState {
                    id,
                    camp_type,
                    x: anchor.x,
                    y: anchor.y,
                    z: anchor.z,
                    yaw: 0.0,
                    hp: template.max_hp,
                    max_hp: template.max_hp,
                    ai_state: NeutralAiState::Idle,
                },
                anchor,
                target_player_id: None,
                last_attack_at: None,
                dead_until: None,
            },
        );
    }
    out
}

/// Builds the two raid bosses in a dormant state: `hp = 0` keeps them out of
/// snapshots (and inert in `simulate_neutrals`) until the match-start schedule
/// arms them via [`schedule_boss_spawns`].
pub(crate) fn build_boss_neutrals(next_id: &mut u64) -> HashMap<u64, Neutral> {
    let mut out = HashMap::new();
    for (anchor, camp_type) in boss_blueprints() {
        let template = neutral_template(camp_type);
        let id = *next_id;
        *next_id += 1;
        out.insert(
            id,
            Neutral {
                state: NeutralState {
                    id,
                    camp_type,
                    x: anchor.x,
                    y: anchor.y,
                    z: anchor.z,
                    yaw: 0.0,
                    hp: 0.0,
                    max_hp: template.max_hp,
                    ai_state: NeutralAiState::Idle,
                },
                anchor,
                target_player_id: None,
                last_attack_at: None,
                dead_until: None,
            },
        );
    }
    out
}

/// Arms the boss spawn schedule at match start (or rematch): every boss is
/// gated behind `dead_until = now + spawn_delay`, so the existing respawn
/// machinery brings it up at its pit with full HP exactly on schedule.
pub(crate) fn schedule_boss_spawns(neutrals: &mut HashMap<u64, Neutral>, now: Instant) {
    for neutral in neutrals.values_mut() {
        let Some(delay) = boss_spawn_delay(neutral.state.camp_type) else {
            continue;
        };
        neutral.dead_until = Some(now + delay);
        neutral.state.hp = 0.0;
        neutral.state.ai_state = NeutralAiState::Idle;
        neutral.target_player_id = None;
        neutral.last_attack_at = None;
        neutral.state.x = neutral.anchor.x;
        neutral.state.y = neutral.anchor.y;
        neutral.state.z = neutral.anchor.z;
        neutral.state.yaw = 0.0;
    }
}
