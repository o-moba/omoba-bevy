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
    }
}

pub(crate) fn jungle_camp_blueprints() -> Vec<(Vec3f, NeutralCampType)> {
    let inner_side = TARGET_BASE_DISTANCE / 2.0_f32.sqrt();
    let half_inner_side = inner_side * 0.5;
    let base_padding = BASE_PAD_SIZE * 0.5 + BASE_EDGE_MARGIN;
    let half_map_size = half_inner_side + base_padding;
    let map_size = half_map_size * 2.0;
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
