//! Opt-in presentation fixtures and production-action drivers for the edge HUD.
//! Synthetic actors/ledger never leave the client. Purchases/utilities require
//! authoritative receipts/acknowledgments, and are reported separately.
use super::{BetaUiQa, clears_playfield, measured_logical_rect, measured_rect};
use crate::{
    combat::{CombatStats, TargetState},
    input_context::{GameplayInputContext, InputContextSet},
    net::{
        GameState, GameStateSnapshot, NetworkHeroClass, NetworkMinionId, NetworkNeutralId,
        NetworkPlayerId, NetworkStructureId, PlayerEquipment, PlayerUtility, TargetId, TargetKind,
    },
    player::Player,
    team::{Team, TeamSelection},
};
use bevy::{
    input::touch::{TouchInput, TouchPhase},
    prelude::*,
    window::PrimaryWindow,
};
use shared::{
    live_score::{LiveScorePlayer, LiveScoreboard},
    shop::ItemId,
};

pub(super) const FILES: [&str; 7] = [
    "08-target-hero-fixture-720p.png",
    "09-target-minion-fixture-720p.png",
    "10-target-structure-fixture-720p.png",
    "11-target-neutral-fixture-720p.png",
    "12-scoreboard-fixture-720p.png",
    "13-scoreboard-closed-720p.png",
    "14-utilities-acknowledged-720p.png",
];
const TARGETS: [(TargetKind, f32, f32); 4] = [
    (TargetKind::Player, 1234.0, 2000.0),
    (TargetKind::Minion, 346.0, 700.0),
    (TargetKind::Structure, 1999.0, 3000.0),
    (TargetKind::Neutral, 520.0, 1200.0),
];
const FIXTURE_ID: u64 = u64::MAX - 100;

#[derive(Resource, Default)]
pub(super) struct EdgeQa {
    entities: [Option<Entity>; 4],
    pub purchase_item: Option<ItemId>,
    fixture_applied: bool,
    driver_stage: usize,
    driver_frame: u32,
    utility_requests: u32,
}
#[derive(Component)]
struct FixtureLabel;

pub(super) fn configure(app: &mut App) {
    app.init_resource::<EdgeQa>()
        .add_systems(PreUpdate, drive.after(super::prepare_controls))
        .add_systems(
            Update,
            fixture
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .before(InputContextSet::Modal),
        );
}

fn drive(
    qa: Res<BetaUiQa>,
    mut state: ResMut<EdgeQa>,
    shop: Res<crate::shop::ShopState>,
    scoreboard: Res<crate::edge_hud::ScoreboardState>,
    selection: Res<TeamSelection>,
    equipment: Query<&PlayerEquipment, With<Player>>,
    mut buttons: crate::qa::TestIdPresses,
    mobile: Res<crate::mobile_controls::MobileControls>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut touches: MessageWriter<TouchInput>,
) {
    if state.driver_stage != qa.stage {
        state.driver_stage = qa.stage;
        state.driver_frame = 0;
    }
    state.driver_frame += 1;
    if qa.stage == 4 && state.purchase_item.is_none() {
        state.purchase_item = equipment.single().ok().and_then(|equipment| {
            crate::shop::quick_offers(selection.hero_class, &equipment.inventory)[0]
        });
    }
    let purchased = purchase_verified(&state, equipment.single().ok());
    buttons.press_where(|name| match qa.stage {
        4 if !purchased && shop.open => name == "ShopCloseButton",
        4 if !purchased && !shop.open && !shop.purchase_pending() => name == "QuickBuy-0",
        4 if purchased && !shop.open => name == "GoldShopButton",
        11 if !scoreboard.open => name == "MatchScoreButton",
        12 if scoreboard.open => name == "ScoreboardCloseButton",
        _ => false,
    });
    if qa.skill_upgrades
        && matches!(qa.stage, 2 | 5)
        && mobile.enabled
        && let Ok(window) = windows.single()
    {
        let phase = match state.driver_frame {
            4 => Some(TouchPhase::Started),
            5 => Some(TouchPhase::Ended),
            _ => None,
        };
        if let Some(phase) = phase {
            touches.write(TouchInput {
                phase,
                position: mobile.layout().upgrade_center,
                window,
                force: None,
                id: 80_010,
            });
        }
    }
    // Each utility receives a real input-system touch start and release on
    // distinct frames. No direct NetworkCommand or local cooldown mutation.
    if qa.stage == 13
        && mobile.enabled
        && let Ok(window) = windows.single()
    {
        let pair = match state.driver_frame {
            4 => Some((0, TouchPhase::Started)),
            5 => Some((0, TouchPhase::Ended)),
            12 => Some((1, TouchPhase::Started)),
            13 => Some((1, TouchPhase::Ended)),
            _ => None,
        };
        if let Some((index, phase)) = pair {
            touches.write(TouchInput {
                phase,
                position: mobile.layout().utility_centers[index],
                window,
                force: None,
                id: 80_000 + index as u64,
            });
            if phase == TouchPhase::Ended {
                state.utility_requests += 1;
            }
        }
    }
}

fn fixture(
    mut commands: Commands,
    qa: Res<BetaUiQa>,
    mut state: ResMut<EdgeQa>,
    mut game: ResMut<GameStateSnapshot>,
    mut target: ResMut<TargetState>,
    local: Query<&NetworkPlayerId, With<Player>>,
    mut labels: Query<&mut Node, With<FixtureLabel>>,
) {
    let active = (7..=11).contains(&qa.stage);
    if active && !state.fixture_applied {
        commands
            .spawn((
                FixtureLabel,
                Name::new("QaEdgeFixtureLabel"),
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(4.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                ZIndex(200),
            ))
            .with_child((
                Text::new("QA: synthetic target / scoreboard presentation"),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                TextColor(crate::ui::theme::GOLD),
                BackgroundColor(crate::ui::theme::PANEL),
            ));
        state.fixture_applied = true;
    }
    for mut node in &mut labels {
        node.display = if active { Display::Flex } else { Display::None };
    }
    if !active {
        if qa.stage >= 12 {
            target.selected_entity = None;
            target.selected_target = None;
        }
        return;
    }
    let local_id = local.single().map_or(0, |id| id.0);
    game.scoreboard = Some(LiveScoreboard {
        players: (0..10)
            .map(|index| LiveScorePlayer {
                player_id: if index == 0 {
                    local_id
                } else {
                    FIXTURE_ID + index as u64
                },
                nickname: if index == 1 {
                    "Long hero name stress fixture".into()
                } else {
                    format!("QA Player {}", index + 1)
                },
                team: if index < 5 {
                    shared::map::Team::Green
                } else {
                    shared::map::Team::Blue
                },
                hero_class: [
                    shared::HeroClass::Warrior,
                    shared::HeroClass::Mage,
                    shared::HeroClass::Ranger,
                    shared::HeroClass::Cleric,
                ][index % 4],
                kills: 10 + index as u32,
                deaths: index as u32,
                assists: 20 - index as u32,
                earned_gold: 12345 + index as u32 * 1234,
                level: 10 + index as u32,
                connected: index != 9,
            })
            .collect(),
    });
    if let Some(index) = qa
        .stage
        .checked_sub(7)
        .filter(|index| *index < TARGETS.len())
    {
        let (kind, hp, max_hp) = TARGETS[index];
        let id = FIXTURE_ID + 50 + index as u64;
        let entity = *state.entities[index].get_or_insert_with(|| {
            let mut entity = commands.spawn((
                CombatStats {
                    hp,
                    max_hp,
                    mana: 0.0,
                    max_mana: 0.0,
                },
                Team::Blue,
                Transform::from_xyz(0.0, 0.0, 0.0),
                Visibility::Visible,
            ));
            match kind {
                TargetKind::Player => {
                    entity.insert((
                        NetworkPlayerId(id),
                        NetworkHeroClass(shared::HeroClass::Cleric),
                    ));
                }
                TargetKind::Minion => {
                    entity.insert(NetworkMinionId(id));
                }
                TargetKind::Structure => {
                    entity.insert(NetworkStructureId(id));
                }
                TargetKind::Neutral => {
                    entity.insert(NetworkNeutralId(id));
                }
            }
            entity.id()
        });
        target.selected_entity = Some(entity);
        target.selected_target = Some(TargetId { kind, id });
    } else {
        target.selected_entity = None;
        target.selected_target = None;
    }
}

pub(super) fn purchase_verified(state: &EdgeQa, equipment: Option<&PlayerEquipment>) -> bool {
    state.purchase_item.is_some_and(|item| {
        equipment.is_some_and(|equipment| {
            equipment.inventory.contains(&item)
                && equipment
                    .last_purchase
                    .as_ref()
                    .is_some_and(|receipt| receipt.item_id == Some(item) && receipt.error.is_none())
        })
    })
}

pub(super) fn ready(
    stage: usize,
    game: &GameStateSnapshot,
    context: &GameplayInputContext,
    scoreboard: Option<&crate::edge_hud::ScoreboardState>,
    state: Option<&EdgeQa>,
    utility: Option<&PlayerUtility>,
) -> bool {
    matches!(game.state, GameState::Running)
        && match stage {
            7..=10 => {
                context.gameplay_allowed() && state.is_some_and(|s| s.entities[stage - 7].is_some())
            }
            11 => {
                scoreboard.is_some_and(|s| s.open)
                    && !context.gameplay_allowed()
                    && game
                        .scoreboard
                        .as_ref()
                        .is_some_and(|s| s.players.len() == 10)
            }
            12 => scoreboard.is_some_and(|s| !s.open) && context.gameplay_allowed(),
            13 => {
                context.gameplay_allowed()
                    && state.is_some_and(|s| s.utility_requests == 2)
                    && utility.is_some_and(|u| {
                        u.state.dash_sequence > 0
                            && u.state.dash_remaining_secs > 0.0
                            && u.state.haste_remaining_secs > 0.0
                            && u.state.haste_active_secs > 0.0
                    })
            }
            _ => false,
        }
}

pub(super) fn resting(stage: usize) -> bool {
    matches!(stage, 2 | 5 | 7..=10 | 12 | 13)
}
pub(super) fn circle(name: &str) -> bool {
    matches!(
        name,
        "MobileJoystick"
            | "MobileAttack"
            | "MobileMinionAttack"
            | "MobileTowerAttack"
            | "MobileDash"
            | "MobileHaste"
            | "MobileRankMode"
    ) || name.starts_with("MobileAbility-")
}
pub(super) fn tracked(name: &str) -> bool {
    name.starts_with("TargetHealth")
        || name.starts_with("Scoreboard")
        || name.starts_with("QuickBuy")
        || name.starts_with("MatchScore")
        || name.starts_with("MobileRank")
        || matches!(
            name,
            "GoldShopButton"
                | "MatchKdaText"
                | "MatchMenuButton"
                | "MobileMinionAttack"
                | "MobileTowerAttack"
                | "MobileDash"
                | "MobileHaste"
                | "MobileAttackCancel"
        )
}

/// Inspect actual rendered geometry, including art and expanded touch bounds.
/// This complements the model tests and catches draw/layout scale divergence.
pub(super) fn mobile_geometry_valid(
    nodes: &[serde_json::Value],
    mobile: &crate::mobile_controls::MobileControls,
) -> bool {
    let shown = |name: &str| {
        nodes
            .iter()
            .find(|node| node["name"] == name && node["visible"] == true)
            .and_then(measured_logical_rect)
    };
    let Some(attack) = shown("MobileAttack") else {
        return false;
    };
    let layout = mobile.layout();
    let scale = mobile.combat_scale();
    if attack.center().distance(layout.attack_center) > 1.0 {
        return false;
    }
    let mut circles = Vec::new();
    for name in [
        "MobileJoystick",
        "MobileAttack",
        "MobileAbility-0",
        "MobileAbility-1",
        "MobileAbility-2",
        "MobileAbility-3",
        "MobileMinionAttack",
        "MobileTowerAttack",
        "MobileDash",
        "MobileHaste",
        "MobileRankMode",
        "MobileAttackCancel",
    ] {
        let Some(rect) = shown(name) else {
            if matches!(name, "MobileRankMode" | "MobileAttackCancel") {
                continue;
            }
            return false;
        };
        if rect.width().min(rect.height()) < 43.5 || (rect.width() - rect.height()).abs() > 1.0 {
            return false;
        }
        let center = rect.center();
        let mut radius = rect.width() * 0.5;
        let satellite = name.starts_with("MobileAbility-")
            || matches!(name, "MobileMinionAttack" | "MobileTowerAttack");
        if satellite && (center.distance(attack.center()) - 92.0 * scale).abs() > 1.0 {
            return false;
        }
        if let Some(slot) = name.strip_prefix("MobileAbility-") {
            let Some(rank) = shown(&format!("MobileRankRing-{slot}")) else {
                return false;
            };
            if rank.center().distance(center) > 1.0
                || (rank.width() * 0.5 - radius - 3.0 * scale).abs() > 1.0
            {
                return false;
            }
            radius = rank.width() * 0.5;
        } else if name == "MobileJoystick" {
            radius *= 1.3;
        }
        if center.x - radius < mobile.safe.left - 1.0
            || center.y - radius < mobile.safe.top - 1.0
            || center.x + radius > mobile.viewport.x - mobile.safe.right + 1.0
            || center.y + radius > mobile.viewport.y - mobile.safe.bottom + 1.0
        {
            return false;
        }
        if !clears_playfield(
            Rect::from_center_size(center, Vec2::splat(radius * 2.0)),
            mobile.viewport,
            true,
        ) {
            return false;
        }
        circles.push((center, radius));
    }
    circles.iter().enumerate().all(|(index, (center, radius))| {
        circles[index + 1..].iter().all(|(other, other_radius)| {
            center.distance(*other) >= radius + other_radius + 3.5 * scale - 1.0
        })
    })
}

pub(super) fn validate(
    stage: usize,
    nodes: &[serde_json::Value],
    texts: &Query<(&Name, &Text)>,
    state: Option<&EdgeQa>,
    context: &GameplayInputContext,
    utility: Option<&PlayerUtility>,
) -> bool {
    let shown = |name: &str| {
        nodes
            .iter()
            .find(|node| node["name"] == name && node["visible"] == true)
            .and_then(measured_rect)
    };
    if (7..=10).contains(&stage) {
        let (_, hp, max_hp) = TARGETS[stage - 7];
        if shown("TargetHealthRoot").is_none()
            || !texts.iter().any(|(name, text)| {
                name.as_str() == "TargetHealthValue" && text.0 == format!("{hp:.0} / {max_hp:.0}")
            })
        {
            return false;
        }
    }
    if matches!(stage, 2 | 5 | 12) && shown("TargetHealthRoot").is_some() {
        return false;
    }
    if stage == 11 && (context.gameplay_allowed() || shown("ScoreboardCloseButton").is_none()) {
        return false;
    }
    if stage == 12 && shown("ScoreboardPanel").is_some() {
        return false;
    }
    if stage == 13
        && (state.is_none_or(|s| s.utility_requests != 2)
            || utility
                .is_none_or(|u| u.state.dash_sequence == 0 || u.state.haste_active_secs <= 0.0))
    {
        return false;
    }
    true
}
pub(super) fn record(
    stage: usize,
    state: Option<&EdgeQa>,
    game: &GameStateSnapshot,
    utility: Option<&PlayerUtility>,
    texts: &Query<(&Name, &Text)>,
) -> serde_json::Value {
    serde_json::json!({"presentation_fixture": (7..=11).contains(&stage), "fixture_target":stage.checked_sub(7).filter(|i|*i<TARGETS.len()).map(|i|format!("{:?}",TARGETS[i].0)),
        "scoreboard":game.scoreboard, "target_text":texts.iter().filter(|(name,_)|name.as_str().starts_with("TargetHealth")).map(|(name,text)|(name.as_str(),text.0.as_str())).collect::<std::collections::BTreeMap<_,_>>(),
        "quick_buy_item":state.and_then(|s|s.purchase_item),"utility_state":utility.map(|u|u.state),"utility_touch_releases":state.map_or(0,|s|s.utility_requests),
        "utility_state_source":"authoritative network snapshot", "manual_interaction_verified":false})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered_control_fixture(
        mobile: &crate::mobile_controls::MobileControls,
    ) -> Vec<serde_json::Value> {
        let layout = mobile.layout();
        let mut circles = vec![
            (
                "MobileAttack".to_owned(),
                layout.attack_center,
                layout.attack_radius,
            ),
            (
                "MobileJoystick".to_owned(),
                layout.joystick_center,
                layout.joystick_radius,
            ),
            (
                "MobileMinionAttack".to_owned(),
                layout.category_centers[0],
                layout.auxiliary_radius,
            ),
            (
                "MobileTowerAttack".to_owned(),
                layout.category_centers[1],
                layout.auxiliary_radius,
            ),
            (
                "MobileDash".to_owned(),
                layout.utility_centers[0],
                layout.auxiliary_radius,
            ),
            (
                "MobileHaste".to_owned(),
                layout.utility_centers[1],
                layout.auxiliary_radius,
            ),
            (
                "MobileRankMode".to_owned(),
                layout.upgrade_center,
                layout.upgrade_radius,
            ),
            (
                "MobileAttackCancel".to_owned(),
                layout.cancel_center,
                layout.cancel_radius,
            ),
        ];
        for slot in 0..4 {
            circles.push((
                format!("MobileAbility-{slot}"),
                layout.ability_centers[slot],
                layout.ability_radii[slot],
            ));
            circles.push((
                format!("MobileRankRing-{slot}"),
                layout.ability_centers[slot],
                layout.ability_radii[slot] + 3.0 * mobile.combat_scale(),
            ));
        }
        circles
            .into_iter()
            .map(|(name, center, radius)| {
                serde_json::json!({
                    "name":name, "visible":true,
                    "logical_min":[center.x - radius, center.y - radius],
                    "logical_size":[radius * 2.0, radius * 2.0],
                })
            })
            .collect()
    }

    #[test]
    fn rendered_radial_guard_checks_orbit_art_and_expanded_touch_bounds() {
        for viewport in [
            Vec2::new(693.0, 320.0),
            Vec2::new(844.0, 390.0),
            Vec2::new(932.0, 430.0),
        ] {
            let mut mobile = crate::mobile_controls::MobileControls::default();
            mobile.viewport = viewport;
            let nodes = rendered_control_fixture(&mobile);
            assert!(mobile_geometry_valid(&nodes, &mobile));
            for (name, axis, delta) in [("MobileTowerAttack", 0, 3.0), ("MobileJoystick", 1, 6.0)] {
                let mut displaced = nodes.clone();
                let node = displaced
                    .iter_mut()
                    .find(|node| node["name"] == name)
                    .unwrap();
                node["logical_min"][axis] =
                    serde_json::json!(node["logical_min"][axis].as_f64().unwrap() + delta);
                assert!(!mobile_geometry_valid(&displaced, &mobile));
            }
            let mut bad_ring = nodes.clone();
            let ring = bad_ring
                .iter_mut()
                .find(|node| node["name"] == "MobileRankRing-0")
                .unwrap();
            ring["logical_size"] = serde_json::json!([30.0, 30.0]);
            assert!(!mobile_geometry_valid(&bad_ring, &mobile));
        }
    }

    #[test]
    fn quick_buy_requires_matching_success_receipt_and_authoritative_inventory() {
        let state = EdgeQa {
            purchase_item: Some(ItemId::EmberBlade),
            ..default()
        };
        let mut equipment = PlayerEquipment {
            inventory: vec![ItemId::EmberBlade],
            ..default()
        };
        assert!(!purchase_verified(&state, Some(&equipment)));
        equipment.last_purchase = Some(shared::shop::PurchaseReceipt {
            request_id: 1,
            match_id: 1,
            item_id: Some(ItemId::TrailBoots),
            error: None,
        });
        assert!(!purchase_verified(&state, Some(&equipment)));
        equipment.last_purchase.as_mut().unwrap().item_id = Some(ItemId::EmberBlade);
        assert!(purchase_verified(&state, Some(&equipment)));
        equipment.inventory.clear();
        assert!(!purchase_verified(&state, Some(&equipment)));
    }
}
