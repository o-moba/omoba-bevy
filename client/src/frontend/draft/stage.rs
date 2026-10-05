//! Own-team presentation shared by the authoritative draft and loading screens.
// i18n-strict
use bevy::prelude::*;
use shared::prematch::{DraftPlayer, PrematchSnapshot};

use super::super::party_stage::{PartyStage, StageMember, StageSurface, slot_anchor};
use super::super::widgets;
use crate::i18n::{data, tr};
use crate::ui::theme;

/// Preserve server identity/order, moving only the local viewer to the centre.
/// The enemy team never appears on this stand, including when the viewer is Blue.
pub(crate) fn own_team(prematch: &PrematchSnapshot, your_id: u64) -> Vec<&DraftPlayer> {
    let Some(viewer) = prematch.players.iter().find(|p| p.player_id == your_id) else {
        return Vec::new();
    };
    let mut team: Vec<_> = prematch
        .players
        .iter()
        .filter(|p| p.team == viewer.team)
        .collect();
    team.sort_by_key(|p| p.player_id != your_id);
    team.truncate(shared::party::MAX_PARTY_SIZE);
    team
}

fn stage_members(prematch: &PrematchSnapshot, your_id: u64) -> Vec<StageMember> {
    own_team(prematch, your_id)
        .into_iter()
        .map(|player| StageMember {
            hero_class: player.hero_class,
            handheld: player.handheld.clone(),
            avatar: player.avatar.clone(),
            character: player.character,
            leader: false,
            // The viewer keeps a personal preview; a teammate's empty plinth
            // and PICKING plate remain until the server confirms their lock.
            revealed: player.locked || player.player_id == your_id,
        })
        .collect()
}

pub(crate) fn sync_members(stage: &mut PartyStage, prematch: &PrematchSnapshot, your_id: u64) {
    let members = stage_members(prematch, your_id);
    if stage.members != members {
        stage.members = members;
    }
}

/// Fit the rendered 16:9 image without stretching heroes at narrow viewports.
pub(crate) fn image_size(width: f32, height: f32) -> Vec2 {
    let width = width.min(height * 16.0 / 9.0).max(1.0);
    Vec2::new(width, width * 9.0 / 16.0)
}

/// Keep each label centred on its own projected hero, including parties
/// with an empty right/outer seat. Limit width to the nearest occupied seat.
fn plate_bounds(index: usize, count: usize, image: Vec2, width: f32, compact: bool) -> (f32, f32) {
    let anchor = slot_anchor(index, count).x;
    let separation = (0..count)
        .filter(|&other| other != index)
        .map(|other| (anchor - slot_anchor(other, count).x).abs() * image.x)
        .fold(f32::INFINITY, f32::min);
    let card_width = (separation - 4.0).clamp(1.0, if compact { 112.0 } else { 166.0 });
    let left = (width - image.x) * 0.5 + anchor * image.x - card_width * 0.5;
    (left, card_width)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_team_stage(
    parent: &mut ChildSpawnerCommands,
    prematch: &PrematchSnapshot,
    your_id: u64,
    stage: Option<&PartyStage>,
    width: f32,
    height: f32,
    loading: bool,
    compact: bool,
) {
    let members = own_team(prematch, your_id);
    let image = image_size(width, height);
    parent
        .spawn((
            Node {
                width: Val::Px(width),
                height: Val::Px(height),
                min_width: Val::Px(0.0),
                overflow: Overflow::clip(),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.018, 0.045, 0.041, 0.65)),
            Name::new("PrematchTeamStage"),
        ))
        .with_children(|stand| {
            if let Some(stage) = stage {
                stand.spawn((
                    ImageNode::new(stage.image.clone()),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px((width - image.x) * 0.5),
                        top: Val::Px((height - image.y) * 0.35),
                        width: Val::Px(image.x),
                        height: Val::Px(image.y),
                        ..default()
                    },
                    StageSurface,
                    Name::new("PrematchStageImage"),
                ));
            }
            stand.spawn((
                widgets::label(
                    tr("draft.your_team"),
                    if compact { 10.0 } else { 12.0 },
                    theme::GOLD,
                ),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(10.0),
                    top: Val::Px(8.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            stand.spawn((
                widgets::label(
                    tr("draft.stage.rotate"),
                    if compact { 10.0 } else { 12.0 },
                    theme::MUTED,
                ),
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(10.0),
                    top: Val::Px(8.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            stand
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        right: Val::Px(0.0),
                        bottom: Val::Px(6.0),
                        height: Val::Px(if compact { 60.0 } else { 78.0 }),
                        ..default()
                    },
                    Name::new("PrematchTeamPlates"),
                ))
                .with_children(|plates| {
                    // The renderer's centre-front, left, right, outer-left, outer-right slots.
                    for index in [3, 1, 0, 2, 4] {
                        let Some(player) = members.get(index) else {
                            continue;
                        };
                        let mine = player.player_id == your_id;
                        let (left, plate_width) =
                            plate_bounds(index, members.len(), image, width, compact);
                        plates
                            .spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(left),
                                    bottom: Val::Px(0.0),
                                    width: Val::Px(plate_width),
                                    min_width: Val::Px(0.0),
                                    flex_direction: FlexDirection::Column,
                                    align_items: AlignItems::Center,
                                    padding: UiRect::all(Val::Px(if compact { 3.0 } else { 6.0 })),
                                    border: UiRect::top(Val::Px(if mine { 2.0 } else { 1.0 })),
                                    overflow: Overflow::clip(),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.022, 0.051, 0.044, 0.94)),
                                BorderColor::all(if mine {
                                    theme::GOLD
                                } else {
                                    theme::PANEL_EDGE
                                }),
                                Name::new(format!("PrematchStagePlayer-{}", player.player_id)),
                            ))
                            .with_children(|plate| {
                                let identity = if mine {
                                    format!("{}{}", player.nickname, tr("draft.roster.you"))
                                } else if player.is_bot {
                                    format!("{}{}", player.nickname, tr("draft.roster.bot"))
                                } else {
                                    player.nickname.clone()
                                };
                                for (text, ink, size) in [
                                    (
                                        identity.as_str(),
                                        theme::IVORY,
                                        if compact { 10.0 } else { 13.0 },
                                    ),
                                    (
                                        data::hero_name(player.hero_class),
                                        theme::GOLD,
                                        if compact { 10.0 } else { 12.0 },
                                    ),
                                    (
                                        data::role(player.role),
                                        theme::MUTED,
                                        if compact { 10.0 } else { 12.0 },
                                    ),
                                    (
                                        if loading {
                                            if player.loaded {
                                                tr("draft.roster.ready")
                                            } else {
                                                tr("draft.roster.loading")
                                            }
                                        } else if player.locked {
                                            tr("draft.roster.locked")
                                        } else {
                                            tr("draft.roster.picking")
                                        },
                                        if player.locked {
                                            theme::JADE
                                        } else {
                                            theme::MUTED
                                        },
                                        if compact { 9.0 } else { 10.0 },
                                    ),
                                ] {
                                    plate.spawn((
                                        widgets::label(text, size, ink),
                                        TextLayout::justify(Justify::Center)
                                            .with_linebreak(LineBreak::NoWrap),
                                        Pickable::IGNORE,
                                    ));
                                }
                            });
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{
        HeroClass,
        map::Team,
        prematch::{PrematchPhase, Role},
        wire::CharacterChoice,
    };

    #[test]
    fn stage_keeps_server_choices_and_centres_a_blue_viewer_among_five_teammates() {
        let players = (1..=10)
            .map(|id| DraftPlayer {
                handheld: Default::default(),
                player_id: id,
                nickname: format!("Player {id}"),
                team: if id <= 5 { Team::Green } else { Team::Blue },
                character: CharacterChoice::Ipfs,
                hero_class: HeroClass::Mage,
                avatar: Some(format!("avatar-{id}")),
                sprite_character: None,
                role: Role::Jungle,
                is_bot: false,
                locked: id % 2 == 0,
                loaded: false,
            })
            .collect();
        let mut prematch = PrematchSnapshot {
            generation: 1,
            phase: PrematchPhase::Draft,
            remaining_ms: 30_000,
            needed: 10,
            players,
            last_request_id: 0,
            error: None,
        };
        let team = own_team(&prematch, 8);
        assert_eq!(
            team.iter().map(|p| p.player_id).collect::<Vec<_>>(),
            [8, 6, 7, 9, 10]
        );
        assert!(team[0].locked);
        assert_eq!(team[0].avatar.as_deref(), Some("avatar-8"));
        assert_eq!(team[0].role, Role::Jungle);
        assert!(own_team(&prematch, 42).is_empty());

        // Unconfirmed teammates retain identity but no model; the local
        // player stays visible even before locking their personal preview.
        prematch
            .players
            .iter_mut()
            .find(|p| p.player_id == 8)
            .unwrap()
            .locked = false;
        let before = stage_members(&prematch, 8);
        assert_eq!(
            before.iter().map(|m| m.revealed).collect::<Vec<_>>(),
            [true, true, false, false, true]
        );
        assert_eq!(before[0].avatar.as_deref(), Some("avatar-8"));
        assert_eq!(before[2].avatar.as_deref(), Some("avatar-7"));
        prematch
            .players
            .iter_mut()
            .find(|p| p.player_id == 7)
            .unwrap()
            .locked = true;
        let after = stage_members(&prematch, 8);
        assert!(
            after[2].revealed,
            "the confirmed teammate appears in their original slot"
        );
        assert_eq!(
            after.iter().map(|m| &m.avatar).collect::<Vec<_>>(),
            before.iter().map(|m| &m.avatar).collect::<Vec<_>>()
        );
        assert!(
            after.iter().all(|m| !m.leader),
            "draft lock does not invent party leadership"
        );
    }

    #[test]
    fn identity_cards_follow_projection_and_do_not_overlap_at_any_team_size() {
        for (width, height, compact) in [(398.0, 254.0, true), (680.0, 568.0, false)] {
            let image = image_size(width, height);
            for count in 1..=5 {
                let mut last_right = 0.0;
                for index in [3, 1, 0, 2, 4].into_iter().filter(|&index| index < count) {
                    let (left, card_width) = plate_bounds(index, count, image, width, compact);
                    assert!(left >= last_right);
                    assert!(left + card_width <= width);
                    let expected = (width - image.x) * 0.5 + slot_anchor(index, count).x * image.x;
                    assert!((left + card_width * 0.5 - expected).abs() < 0.001);
                    last_right = left + card_width;
                }
            }
        }
    }

    #[test]
    fn stage_image_preserves_aspect_and_fits_desktop_phone_and_tablet_panels() {
        for size in [
            Vec2::new(680.0, 568.0),
            Vec2::new(398.0, 254.0),
            Vec2::new(629.0, 668.0),
        ] {
            let image = image_size(size.x, size.y);
            assert!(image.x <= size.x && image.y <= size.y);
            assert!((image.x / image.y - 16.0 / 9.0).abs() < 0.001);
        }
    }
}
