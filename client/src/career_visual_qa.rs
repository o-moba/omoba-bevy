//! Opt-in UI fixture capture, never evidence of real matches or database writes.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    ui::FocusPolicy,
    window::PrimaryWindow,
};
use shared::{
    HeroClass,
    career::{
        CareerView, FriendPresence, FriendProfile, FriendsView, MatchOutcome, MatchResult,
        MatchStats, MatchSummary, ParticipantResult, ProfileSummary, RatingChange,
    },
    map::Team,
};

use crate::{
    career::{CareerClient, CareerModal, CareerUiSet},
    platform::UiProfile,
};

const SETTLE_FRAMES: u32 = 32;
const VIEWS: [(&str, CareerModal, f32); 6] = [
    ("01-profile.png", CareerModal::Profile, 0.0),
    ("02-friends-code.png", CareerModal::Friends, 0.0),
    ("03-friends-list.png", CareerModal::Friends, 350.0),
    ("04-history.png", CareerModal::History, 0.0),
    ("05-result.png", CareerModal::Result, 0.0),
    ("06-friend-profile.png", CareerModal::FriendProfile, 0.0),
];

pub(crate) struct CareerVisualQaPlugin;
impl Plugin for CareerVisualQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_CAREER_QA_OUTPUT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        let dimension = |name: &str, fallback| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(fallback)
                .clamp(200, 3840)
        };
        app.insert_resource(CareerQa {
            directory,
            started: Instant::now(),
            pixels: UVec2::new(
                dimension("OMOBA_QA_WIDTH", 1280),
                dimension("OMOBA_QA_HEIGHT", 720),
            ),
            stage: 0,
            applied_stage: None,
            settled: 0,
            in_flight: false,
            readbacks: Vec::new(),
            captures: Vec::new(),
            finished: false,
        })
        .add_systems(Startup, watermark)
        .add_systems(Update, prepare.after(CareerUiSet))
        .add_systems(
            PostUpdate,
            observe
                .after(bevy::ui::UiSystems::Layout)
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}

#[derive(Resource)]
struct CareerQa {
    directory: PathBuf,
    started: Instant,
    pixels: UVec2,
    stage: usize,
    applied_stage: Option<usize>,
    settled: u32,
    in_flight: bool,
    readbacks: Vec<usize>,
    captures: Vec<serde_json::Value>,
    finished: bool,
}

fn watermark(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(2.0),
            right: Val::Px(8.0),
            ..default()
        },
        Text::new("QA FIXTURE · no account or match data"),
        TextFont {
            font_size: 8.0,
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::BLACK),
        FocusPolicy::Pass,
        ZIndex(300),
        Name::new("CareerQaWatermark"),
    ));
}

fn profile(id: char, nickname: &str) -> ProfileSummary {
    ProfileSummary {
        profile_id: id.to_string().repeat(64),
        nickname: format!("{nickname}#{:04}", id as u32),
        rating: 1210,
        rated_matches: 8,
        matches_played: 12,
        wins: 7,
        losses: 5,
        progression_xp: 2450,
    }
}

fn fixture() -> CareerView {
    let own = profile('a', "QA Дмитрий");
    let names = [
        "QA Дмитрий",
        "QA_Long_Name_000002",
        "QA Мария",
        "QA 小明",
        "QA Alex",
    ];
    let classes = [HeroClass::Warrior, HeroClass::Mage, HeroClass::Ranger];
    let participants: Vec<_> = (0..10)
        .map(|index| ParticipantResult {
            is_bot: false,
            player_id: index + 1,
            profile_id: Some(if index == 0 {
                own.profile_id.clone()
            } else {
                format!("{:064x}", index + 100)
            }),
            nickname: names[index as usize % names.len()].into(),
            team: if index < 5 { Team::Green } else { Team::Blue },
            hero_class: classes[index as usize % classes.len()],
            character: "ipfs".into(),
            avatar: Some("agnes".into()),
            sprite_character: None,
            stats: MatchStats {
                kills: 2 + index as u32,
                deaths: 1 + index as u32 % 4,
                assists: 3 + index as u32,
                damage_to_heroes: 1200.0 + index as f64 * 99.0,
                damage_to_creeps: 2500.0 + index as f64 * 300.0,
                damage_to_structures: 630.0,
                damage_taken: 950.0,
                minion_last_hits: 34,
                jungle_last_hits: 5,
                structures_destroyed: 1,
                final_level: 12,
            },
            disconnected: index == 9,
            rating: Some(RatingChange {
                before: 1200,
                after: if index < 5 { 1216 } else { 1184 },
                delta: if index < 5 { 16 } else { -16 },
            }),
            progression_xp_gained: if index < 5 { 150 } else { 100 },
        })
        .collect();
    let result = MatchResult {
        result_id: "QA-FIXTURE-NOT-A-SAVED-MATCH".into(),
        server_epoch: 999,
        match_id: 1,
        started_at_ms: 1_789_340_400_000,
        ended_at_ms: 1_789_341_132_000,
        duration_ms: 732_000,
        map_profile: "verdant_default".into(),
        ruleset: "QA presentation fixture".into(),
        outcome: MatchOutcome::Completed,
        winner: Some(Team::Green),
        rated: true,
        unrated_reason: None,
        participants,
        saved: true,
    };
    let friends = FriendsView {
        incoming: vec![FriendProfile {
            profile: profile('b', "QA СверхдлинноеИмя"),
            presence: FriendPresence::Online,
        }],
        friends: vec![
            FriendProfile {
                profile: profile('c', "QA 小明"),
                presence: FriendPresence::Playing,
            },
            FriendProfile {
                profile: profile('d', "QA Мария"),
                presence: FriendPresence::Offline,
            },
        ],
        outgoing: vec![FriendProfile {
            profile: profile('e', "QA_Request_Pending"),
            presence: FriendPresence::Online,
        }],
    };
    let history = (0..5)
        .map(|index| MatchSummary {
            result_id: format!("QA-FIXTURE-{index}"),
            ended_at_ms: 1_789_341_132_000 - index * 1_000_000,
            duration_ms: 732_000 + index * 12_000,
            outcome: MatchOutcome::Completed,
            won: Some(index % 2 == 0),
            hero_class: classes[index as usize % classes.len()],
            avatar: Some("agnes".into()),
            sprite_character: None,
            kills: 5,
            deaths: 2,
            assists: 8,
            damage_to_heroes: 1456.0,
            rating: Some(RatingChange {
                before: 1200,
                after: if index % 2 == 0 { 1216 } else { 1184 },
                delta: if index % 2 == 0 { 16 } else { -16 },
            }),
        })
        .collect();
    CareerView {
        profile: Some(own),
        friends: Some(friends),
        visited_profile: Some(profile('b', "QA СверхдлинноеИмя")),
        history,
        history_loaded: true,
        history_next: Some(1_789_330_000_000),
        last_result: Some(result),
        ..default()
    }
}

fn prepare(
    mut qa: ResMut<CareerQa>,
    mut career: ResMut<CareerClient>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut scrolls: Query<(&Name, &mut ScrollPosition)>,
) {
    if qa.finished || qa.stage >= VIEWS.len() {
        return;
    }
    if let Ok(mut window) = windows.single_mut() {
        if window.resolution.physical_width() != qa.pixels.x
            || window.resolution.physical_height() != qa.pixels.y
        {
            window.resolution.set_scale_factor_override(Some(1.0));
            window
                .resolution
                .set_physical_resolution(qa.pixels.x, qa.pixels.y);
        }
    }
    if qa.applied_stage != Some(qa.stage) {
        career.present_visual_fixture(fixture(), VIEWS[qa.stage].1);
        qa.applied_stage = Some(qa.stage);
        qa.settled = 0;
    }
    for (name, mut scroll) in &mut scrolls {
        if name.as_str() == "CareerBody" {
            scroll.y = VIEWS[qa.stage].2;
        }
    }
}

#[derive(Component)]
struct Shot(usize);

fn fits(min: Vec2, size: Vec2, outer_min: Vec2, outer_size: Vec2) -> bool {
    size.x > 0.0
        && size.y > 0.0
        && min.cmpge(outer_min - Vec2::ONE).all()
        && (min + size).cmple(outer_min + outer_size + Vec2::ONE).all()
}

fn fail(qa: &mut CareerQa, reason: &str, exit: &mut MessageWriter<AppExit>) {
    qa.finished = true;
    let _ = std::fs::create_dir_all(&qa.directory);
    let value = serde_json::json!({"status":"failed", "reason":reason, "stage":qa.stage, "fixture":true, "captures":qa.captures});
    let _ = std::fs::write(
        qa.directory.join("qa-failure.json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    );
    error!("CAREER_QA failed: {reason}");
    exit.write(AppExit::error());
}

fn observe(
    mut commands: Commands,
    mut qa: ResMut<CareerQa>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    nodes: Query<(
        &Name,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&InheritedVisibility>,
    )>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.finished {
        return;
    }
    if qa.started.elapsed() > Duration::from_secs(120) {
        fail(&mut qa, "120-second native readback deadline", &mut exit);
        return;
    }
    if qa.in_flight {
        if !qa.readbacks.contains(&qa.stage)
            || !qa
                .directory
                .join(VIEWS[qa.stage].0)
                .metadata()
                .is_ok_and(|file| file.len() > 32)
        {
            return;
        }
        qa.stage += 1;
        qa.in_flight = false;
        if qa.stage == VIEWS.len() {
            let summary = serde_json::json!({"status":"passed", "scenario":"career-ui-fixture", "version":env!("CARGO_PKG_VERSION"), "method":"real Bevy primary_window ScreenshotCaptured + save_to_disk", "fixture":true, "live_account_or_database_evidence":false, "manual_input_verified":false, "physical_phone_verified":false, "settle_frames":SETTLE_FRAMES, "captures":qa.captures});
            let saved = std::fs::write(
                qa.directory.join("qa-summary.json"),
                serde_json::to_vec_pretty(&summary).unwrap(),
            );
            qa.finished = true;
            if let Ok((entity, _)) = windows.single() {
                commands.entity(entity).despawn();
            }
            exit.write(if saved.is_ok() {
                AppExit::Success
            } else {
                AppExit::error()
            });
        }
        return;
    }
    if qa.applied_stage != Some(qa.stage) {
        return;
    }
    qa.settled += 1;
    if qa.settled < SETTLE_FRAMES {
        return;
    }
    let Ok((_, window)) = windows.single() else {
        return;
    };
    let viewport = Vec2::new(
        window.resolution.physical_width() as f32,
        window.resolution.physical_height() as f32,
    );
    if viewport.as_uvec2() != qa.pixels {
        return;
    }
    let mut measured = Vec::new();
    for (name, node, transform, inherited) in &nodes {
        if !name.as_str().starts_with("Career") || name.as_str() == "CareerQaWatermark" {
            continue;
        }
        let size = node.size() * transform.to_scale_angle_translation().0.abs();
        let min = transform.translation - size * 0.5;
        measured.push((
            name.as_str().to_owned(),
            min,
            size,
            inherited.is_none_or(|visibility| visibility.get()),
        ));
    }
    let mobile = crate::platform::ui_profile() == UiProfile::Mobile;
    let root_name = if mobile {
        "CareerMobileRoot"
    } else {
        "CareerDesktopRoot"
    };
    let lookup = |wanted: &str| measured.iter().find(|(name, _, _, _)| name == wanted);
    let Some((_, root_min, root_size, root_visible)) = lookup(root_name) else {
        return;
    };
    let Some((_, panel_min, panel_size, panel_visible)) = lookup("CareerPanel") else {
        return;
    };
    let Some((_, body_min, body_size, body_visible)) = lookup("CareerBody") else {
        return;
    };
    let primary_fit = *root_visible
        && *panel_visible
        && *body_visible
        && fits(*root_min, *root_size, Vec2::ZERO, viewport)
        && fits(*panel_min, *panel_size, *root_min, *root_size)
        && fits(*body_min, *body_size, *panel_min, *panel_size);
    let header_names = [
        "CareerProfileTab",
        "CareerHistoryTab",
        "CareerFriendsTab",
        "CareerClose",
    ];
    let header_fit = header_names.iter().all(|name| {
        lookup(name).is_some_and(|(_, min, size, visible)| {
            *visible
                && fits(*min, *size, *panel_min, *panel_size)
                && min.y + size.y <= body_min.y + 1.0
        })
    });
    let header_rects: Vec<_> = header_names
        .iter()
        .filter_map(|name| lookup(name))
        .collect();
    let header_no_overlap = header_rects
        .iter()
        .enumerate()
        .all(|(i, (_, min, size, _))| {
            header_rects
                .iter()
                .skip(i + 1)
                .all(|(_, other_min, other_size, _)| {
                    let overlap =
                        (*min + *size).min(*other_min + *other_size) - min.max(*other_min);
                    overlap.x <= 1.0 || overlap.y <= 1.0
                })
        });
    let node_json: Vec<_> = measured.iter().map(|(name, min, size, visible)| serde_json::json!({"name":name,"min":min.to_array(),"size":size.to_array(),"visible":visible})).collect();
    let record = serde_json::json!({"file":VIEWS[qa.stage].0,"frame":qa.settled,"modal":format!("{:?}",VIEWS[qa.stage].1),"mobile":mobile,"ui_profile":if mobile {"Mobile"}else{"Desktop"},"pixels":viewport.to_array(),"logical_dimensions":[window.width(),window.height()],"scroll_y":VIEWS[qa.stage].2,"fixture":true,"primary_roots_fit":primary_fit,"header_buttons_fit":header_fit,"header_buttons_do_not_overlap":header_no_overlap,"nodes":node_json});
    qa.captures.push(record);
    if !primary_fit || !header_fit || !header_no_overlap {
        fail(
            &mut qa,
            "career root/panel/body/header geometry leaves its viewport or overlaps",
            &mut exit,
        );
        return;
    }
    if std::fs::create_dir_all(&qa.directory).is_err() {
        fail(&mut qa, "cannot create capture directory", &mut exit);
        return;
    }
    let stage = qa.stage;
    commands
        .spawn((Screenshot::primary_window(), Shot(stage)))
        .observe(save_to_disk(qa.directory.join(VIEWS[stage].0)))
        .observe(readback);
    qa.in_flight = true;
}

fn readback(captured: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<CareerQa>) {
    if let Ok(shot) = shots.get(captured.entity)
        && captured.image.width() == qa.pixels.x
        && captured.image.height() == qa.pixels.y
        && !qa.readbacks.contains(&shot.0)
    {
        qa.readbacks.push(shot.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_is_explicit_and_has_full_teams_and_relationship_states() {
        let view = fixture();
        let result = view.last_result.unwrap();
        assert!(result.result_id.starts_with("QA-FIXTURE"));
        assert_eq!(result.participants.len(), 10);
        for team in [Team::Green, Team::Blue] {
            assert_eq!(
                result
                    .participants
                    .iter()
                    .filter(|p| p.team == team)
                    .count(),
                5
            );
        }
        for participant in result.participants {
            assert!(shared::career::normalize_nickname(&participant.nickname).is_ok());
        }
        let friends = view.friends.unwrap();
        assert!(
            !friends.incoming.is_empty()
                && !friends.friends.is_empty()
                && !friends.outgoing.is_empty()
        );
    }
    #[test]
    fn bounds_reject_zero_and_clipped_panels() {
        assert!(fits(
            Vec2::ONE,
            Vec2::splat(8.0),
            Vec2::ZERO,
            Vec2::splat(10.0)
        ));
        assert!(!fits(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::splat(10.0)));
        assert!(!fits(
            Vec2::splat(3.0),
            Vec2::splat(10.0),
            Vec2::ZERO,
            Vec2::splat(10.0)
        ));
    }
}
