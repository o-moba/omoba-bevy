//! QA-feature-only, loopback-only evidence of a server-owned Hero aura on a
//! roster character. Movement uses the normal prediction/networking path; this
//! harness never inserts an aura or changes an account grant.
use crate::{
    camera::CameraSettings,
    career::CareerClient,
    help_overlay::HelpOverlayVisible,
    i18n::Locale,
    net::{ClientSession, GameState, GameStateSnapshot, NetworkAvatar, NetworkPlayerId},
    player::{MovementTarget, Player},
    sprite::PlayerVisualMode,
    supporter::{AuraOwner, HERO_AURA_ELEMENT_BUDGET, NetworkSupporterAura},
};
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const MOTION_FRAMES: usize = 24;
const TIMING_FRAMES: usize = 180;
const WARMUP_FRAMES: u32 = 60;
const ROAD_START: Vec2 = Vec2::new(-40., -46.);
const ROAD_END: Vec2 = Vec2::new(-47., -40.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Normal,
    Moving,
    Close,
    Motion,
    WarmActive,
    SampleActive,
    WaitRevocation,
    WarmInactive,
    SampleInactive,
    Revoked,
    Complete,
}

pub(crate) struct SupporterWorldQaPlugin;
impl Plugin for SupporterWorldQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(dir) = std::env::var_os("OMOBA_HERO_WORLD_QA_DIR").map(PathBuf::from) else {
            return;
        };
        let local = std::env::var("GAME_SERVER_ADDR")
            .ok()
            .and_then(|s| s.parse::<std::net::SocketAddr>().ok())
            .is_some_and(|a| a.ip().is_loopback());
        assert!(local, "Hero world QA requires an explicit loopback server");
        std::fs::create_dir_all(&dir).expect("create Hero QA evidence directory");
        app.insert_resource(Qa {
            dir,
            started: Instant::now(),
            stage: Stage::Normal,
            settled: 0,
            pending: false,
            origin: None,
            placed: false,
            reports: Vec::new(),
            motion_frame: 0,
            last_capture: 0.,
            timing_position: None,
            timing_started: 0.,
            samples: Vec::new(),
            active_timing: None,
            inactive_timing: None,
            max_drift: 0.,
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(
            PostUpdate,
            capture
                .after(bevy::ui::UiSystems::Layout)
                .after(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
        );
    }
}

#[derive(Resource)]
struct Qa {
    dir: PathBuf,
    started: Instant,
    stage: Stage,
    settled: u32,
    pending: bool,
    origin: Option<Vec3>,
    placed: bool,
    reports: Vec<serde_json::Value>,
    motion_frame: usize,
    last_capture: f64,
    timing_position: Option<Vec3>,
    timing_started: f64,
    samples: Vec<f64>,
    active_timing: Option<serde_json::Value>,
    inactive_timing: Option<serde_json::Value>,
    max_drift: f32,
}

fn write_json(qa: &Qa, name: &str, value: &serde_json::Value) {
    std::fs::write(qa.dir.join(name), serde_json::to_vec_pretty(value).unwrap())
        .expect("write Hero QA evidence");
}

fn fail(qa: &Qa, exit: &mut MessageWriter<bevy::app::AppExit>, reason: &str) {
    write_json(
        qa,
        "failure.json",
        &serde_json::json!({
            "stage":format!("{:?}",qa.stage), "reason":reason,
            "elapsed_seconds":qa.started.elapsed().as_secs_f64(),
        }),
    );
    exit.write(bevy::app::AppExit::error());
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    sorted[((sorted.len() as f64 * fraction).ceil() as usize).saturating_sub(1)]
}

fn timing_report(
    qa: &Qa,
    active: bool,
    elapsed: f64,
    identity: &serde_json::Value,
) -> serde_json::Value {
    let mut sorted = qa.samples.clone();
    sorted.sort_by(f64::total_cmp);
    serde_json::json!({
        "measurement":"Time<Real> delta; local end-to-end frame timing, not GPU timing",
        "aura_enabled":active, "warmup_frames":WARMUP_FRAMES, "sample_count":qa.samples.len(),
        "p50_ms":percentile(&sorted,0.50), "p95_ms":percentile(&sorted,0.95),
        "min_ms":sorted[0], "max_ms":sorted[sorted.len()-1], "samples_ms":qa.samples,
        "sample_elapsed_seconds":elapsed-qa.timing_started,
        "sum_frame_seconds":qa.samples.iter().sum::<f64>()/1000.,
        "position":qa.timing_position.map(|p|p.to_array()), "max_position_drift":qa.max_drift,
        "identity":identity, "viewport":[1280,720], "camera_zoom":0.55,
        "screenshots_during_sampling":false, "startup_and_capture_warmup_excluded":true,
        "physical_device_verified":false, "limitations":"One local desktop run; frame pacing and other scene work are included. No GPU or mobile performance claim.",
    })
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn capture(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    session: Res<ClientSession>,
    career: Res<CareerClient>,
    help: Res<HelpOverlayVisible>,
    mut buttons: super::TestIdPresses,
    players: Query<
        (
            Entity,
            &NetworkPlayerId,
            &GlobalTransform,
            &NetworkSupporterAura,
            Option<&NetworkAvatar>,
        ),
        With<Player>,
    >,
    auras: Query<(
        &AuraOwner,
        &GlobalTransform,
        &InheritedVisibility,
        &Children,
    )>,
    mut camera: ResMut<CameraSettings>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
    mut exit: MessageWriter<bevy::app::AppExit>,
    real: Res<Time<Real>>,
    locale: Res<Locale>,
    game: Res<GameStateSnapshot>,
    mode: Res<PlayerVisualMode>,
) {
    if qa.started.elapsed() > Duration::from_secs(180) {
        fail(&qa, &mut exit, "live world timeout");
        return;
    }
    if *mode != PlayerVisualMode::Models3d || locale.id().code() != "en" {
        fail(
            &qa,
            &mut exit,
            "Hero world evidence requires English Models3d",
        );
        return;
    }
    if let Ok(mut w) = windows.single_mut() {
        w.resolution.set_scale_factor_override(Some(1.));
        if w.physical_width() != 1280 || w.physical_height() != 720 {
            w.resolution.set_physical_resolution(1280, 720);
        }
    }
    if help.0 {
        buttons.press("HelpDismissButton");
        return;
    }
    if !session.join_confirmed() || qa.pending || game.state != GameState::Running {
        return;
    }
    let Ok((entity, id, transform, aura, avatar)) = players.single() else {
        return;
    };
    let position = transform.translation();
    if qa.stage == Stage::Normal {
        camera.zoom = 1.;
        if !qa.placed {
            commands.entity(entity).insert(MovementTarget {
                target: Vec3::new(ROAD_START.x, position.y, ROAD_START.y),
            });
            qa.placed = true;
            return;
        }
        if position.xz().distance(ROAD_START) > 0.5 {
            return;
        }
    }
    let roots: Vec<_> = auras
        .iter()
        .filter(|(owner, _, _, _)| owner.0 == entity)
        .collect();
    let active = aura.0 == Some(shared::supporter::AuraStyle::Solar)
        && roots.len() == 1
        && roots[0].2.get()
        && roots[0].3.len() == HERO_AURA_ELEMENT_BUDGET;
    let inactive = aura.0.is_none() && roots.is_empty();
    let identity = serde_json::json!({
        "player_id":id.0, "profile_id":career.public_profile_id,
        "avatar":avatar.and_then(|a|a.0.as_deref()), "match_mode":game.match_mode,
        "geometry_id":game.geometry_id, "map_profile":game.map_profile,
        "client_version":env!("CARGO_PKG_VERSION"), "visual_mode":format!("{:?}",*mode),
        "locale":locale.id().code(),
    });
    let elapsed = real.elapsed_secs_f64();
    if matches!(qa.stage, Stage::WarmActive | Stage::SampleActive) && !active {
        fail(
            &qa,
            &mut exit,
            "aura disappeared before active timing completed",
        );
        return;
    }
    if matches!(
        qa.stage,
        Stage::WarmInactive | Stage::SampleInactive | Stage::Revoked
    ) && !inactive
    {
        fail(&qa, &mut exit, "replicated aura returned after revocation");
        return;
    }
    if matches!(
        qa.stage,
        Stage::Normal | Stage::Moving | Stage::Close | Stage::Motion
    ) && !active
    {
        qa.settled = 0;
        return;
    }
    if let Some(at) = qa.timing_position {
        qa.max_drift = qa.max_drift.max(at.distance(position));
        if at.distance(position) > 0.15 || (camera.zoom - 0.55).abs() > 0.001 {
            fail(
                &qa,
                &mut exit,
                "position or camera changed during comparison",
            );
            return;
        }
    }
    match qa.stage {
        Stage::WaitRevocation => {
            if inactive {
                qa.stage = Stage::WarmInactive;
                qa.settled = 0;
            }
            return;
        }
        Stage::WarmActive | Stage::WarmInactive => {
            qa.settled += 1;
            if qa.settled >= WARMUP_FRAMES {
                qa.stage = if qa.stage == Stage::WarmActive {
                    Stage::SampleActive
                } else {
                    Stage::SampleInactive
                };
                qa.samples.clear();
                qa.timing_started = elapsed;
            }
            return;
        }
        Stage::SampleActive | Stage::SampleInactive => {
            let milliseconds = real.delta_secs_f64() * 1000.;
            if !milliseconds.is_finite() || milliseconds <= 0. {
                fail(&qa, &mut exit, "invalid real frame delta");
                return;
            }
            qa.samples.push(milliseconds);
            if qa.samples.len() < TIMING_FRAMES {
                return;
            }
            let enabled = qa.stage == Stage::SampleActive;
            let report = timing_report(&qa, enabled, elapsed, &identity);
            if enabled {
                write_json(&qa, "active-timing.json", &report);
                qa.active_timing = Some(report);
                qa.stage = Stage::WaitRevocation;
                write_json(
                    &qa,
                    "ready-to-revoke.json",
                    &serde_json::json!({
                        "active_samples_complete":true, "identity":identity,
                        "instruction":"Coordinator may now revoke the seeded local grant; the client only observes the server result.",
                    }),
                );
            } else {
                write_json(&qa, "inactive-timing.json", &report);
                qa.inactive_timing = Some(report);
                qa.stage = Stage::Revoked;
                qa.settled = 0;
            }
            return;
        }
        Stage::Complete => {
            write_json(
                &qa,
                "world-summary.json",
                &serde_json::json!({
                    "pass":true, "scene":"live local server match with roster avatar",
                    "seeded_payment_period":true, "real_payment":false, "client_aura_injected":false,
                    "viewport":[1280,720], "locale":locale.id().code(), "identity":identity, "captures":qa.reports,
                    "motion_frames":qa.motion_frame, "active_timing":qa.active_timing,
                    "inactive_timing":qa.inactive_timing, "revoked_through_server":inactive,
                    "physical_device_verified":false,
                    "performance_scope":"Local end-to-end frame-time sanity comparison; not isolated GPU cost or mobile certification.",
                }),
            );
            exit.write(bevy::app::AppExit::Success);
            return;
        }
        _ => {}
    }
    if qa.stage == Stage::Moving
        && qa
            .origin
            .is_none_or(|p| p.xz().distance(position.xz()) < 3.)
    {
        return;
    }
    if qa.stage == Stage::Close && position.xz().distance(ROAD_END) > 0.35 {
        return;
    }
    qa.settled += 1;
    if qa.stage == Stage::Motion {
        if elapsed - qa.last_capture < 0.125 {
            return;
        }
    } else if qa.settled
        < match qa.stage {
            Stage::Normal => 180,
            Stage::Moving => 12,
            _ => 60,
        }
    {
        return;
    }
    let (name, index) = match qa.stage {
        Stage::Normal => ("01-hero-world.png".to_string(), 0),
        Stage::Moving => ("02-hero-moving.png".to_string(), 1),
        Stage::Close => ("03-hero-close.png".to_string(), 2),
        Stage::Motion => (
            format!("hero-orbit-motion-{:03}.png", qa.motion_frame),
            4 + qa.motion_frame,
        ),
        Stage::Revoked => ("04-hero-revoked.png".to_string(), 3),
        _ => unreachable!(),
    };
    let record = serde_json::json!({
        "file":name, "stage":format!("{:?}",qa.stage), "identity":identity,
        "player_id":id.0, "profile_id":career.public_profile_id,
        "position":position.to_array(), "replicated_aura":aura.0,
        "aura_roots":roots.len(), "aura_elements":roots.first().map(|r|r.3.len()).unwrap_or(0),
        "root_position":roots.first().map(|r|r.1.translation().to_array()),
        "camera_zoom":camera.zoom, "real_elapsed_seconds":elapsed,
    });
    write_json(&qa, &format!("stage-{index}.json"), &record);
    qa.reports.push(record);
    qa.last_capture = elapsed;
    if qa.stage == Stage::Normal {
        qa.origin = Some(position);
        // This road runs away from the base tower, unlike the old centre-bound path.
        commands.entity(entity).insert(MovementTarget {
            target: Vec3::new(ROAD_END.x, position.y, ROAD_END.y),
        });
    }
    if qa.stage == Stage::Moving {
        camera.zoom = 0.55;
    }
    if qa.stage == Stage::Close {
        qa.timing_position = Some(position);
    }
    qa.pending = true;
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(qa.dir.join(name)))
        .observe(|_: On<ScreenshotCaptured>, mut qa: ResMut<Qa>| {
            qa.stage = match qa.stage {
                Stage::Normal => Stage::Moving,
                Stage::Moving => Stage::Close,
                Stage::Close => Stage::Motion,
                Stage::Motion => {
                    qa.motion_frame += 1;
                    if qa.motion_frame == MOTION_FRAMES {
                        Stage::WarmActive
                    } else {
                        Stage::Motion
                    }
                }
                Stage::Revoked => Stage::Complete,
                _ => unreachable!(),
            };
            qa.settled = 0;
            qa.pending = false;
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timing_percentiles_keep_spikes_and_use_all_samples() {
        let mut samples = vec![10.; TIMING_FRAMES];
        samples[0] = 200.;
        samples[1] = 20.;
        samples.sort_by(f64::total_cmp);
        assert_eq!(percentile(&samples, 0.5), 10.);
        assert_eq!(percentile(&samples, 0.95), 10.);
        assert_eq!(samples[samples.len() - 1], 200.);
        assert_eq!(samples.len(), 180);
    }
}
