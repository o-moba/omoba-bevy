//! Live server-owned Hero aura on a roster character, never a client-granted preview.
use crate::{
    camera::CameraSettings,
    career::CareerClient,
    help_overlay::HelpOverlayVisible,
    net::{ClientSession, NetworkPlayerId},
    player::{MovementTarget, Player},
    supporter::{AuraOwner, NetworkSupporterAura},
};
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) struct SupporterWorldQaPlugin;
impl Plugin for SupporterWorldQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(dir) = std::env::var_os("OMOBA_HERO_WORLD_QA_DIR").map(PathBuf::from) else {
            return;
        };
        // A development evidence harness must never connect itself to a public server.
        let local = std::env::var("GAME_SERVER_ADDR")
            .ok()
            .and_then(|s| s.parse::<std::net::SocketAddr>().ok())
            .is_some_and(|a| a.ip().is_loopback());
        assert!(local, "Hero world QA requires an explicit loopback server");
        app.insert_resource(Qa {
            dir,
            started: Instant::now(),
            stage: 0,
            settled: 0,
            pending: false,
            origin: None,
            placed: false,
            reports: Vec::new(),
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
    stage: usize,
    settled: u32,
    pending: bool,
    origin: Option<Vec3>,
    placed: bool,
    reports: Vec<serde_json::Value>,
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
) {
    if qa.started.elapsed() > Duration::from_secs(150) {
        let _ = std::fs::write(
            qa.dir.join("failure.json"),
            format!(
                "{{\"stage\":{},\"reason\":\"live world timeout\"}}",
                qa.stage
            ),
        );
        exit.write(bevy::app::AppExit::error());
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
    if !session.join_confirmed() || qa.pending {
        return;
    }
    let Ok((entity, id, transform, aura)) = players.single() else {
        return;
    };
    let position = transform.translation();
    // Walk out of the base architecture through normal prediction/networking.
    // An unobstructed road is needed to judge the character's silhouette.
    if qa.stage == 0 {
        let destination = Vec3::new(-40., position.y, -46.);
        if !qa.placed {
            commands.entity(entity).insert(MovementTarget {
                target: destination,
            });
            qa.placed = true;
            return;
        }
        if position.distance(destination) > 1. {
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
        && roots[0].3.len() == 35;
    if qa.stage == 4 {
        let report = serde_json::json!({"pass":true,"scene":"live local server match with roster avatar",
            "seeded_payment_period":true,"real_payment":false,"client_aura_injected":false,
            "viewport":[1280,720],"locale":"en","captures":qa.reports});
        std::fs::write(
            qa.dir.join("world-summary.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        exit.write(bevy::app::AppExit::Success);
        return;
    }
    if qa.stage < 3 && !active {
        qa.settled = 0;
        return;
    }
    if qa.stage == 3 && (aura.0.is_some() || !roots.is_empty()) {
        qa.settled = 0;
        return;
    }
    if qa.stage == 1 && qa.origin.is_none_or(|p| p.distance(position) < 3.) {
        return;
    }
    qa.settled += 1;
    if qa.settled < if qa.stage == 0 { 180 } else { 30 } {
        return;
    }
    std::fs::create_dir_all(&qa.dir).unwrap();
    let names = [
        "01-hero-world.png",
        "02-hero-moving.png",
        "03-hero-close.png",
        "04-hero-revoked.png",
    ];
    let name = names[qa.stage];
    let record = serde_json::json!({"file":name,"stage":qa.stage,"player_id":id.0,
        "profile_id":career.public_profile_id,"position":position.to_array(),
        "replicated_aura":aura.0,"aura_roots":roots.len(),"aura_elements":roots.first().map(|r|r.3.len()).unwrap_or(0),
        "root_position":roots.first().map(|r|r.1.translation().to_array()),"camera_zoom":camera.zoom});
    std::fs::write(
        qa.dir.join(format!("stage-{}.json", qa.stage)),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
    qa.reports.push(record);
    if qa.stage == 0 {
        qa.origin = Some(position);
        let toward_center = Vec3::new(-position.x, 0., -position.z).normalize_or_zero();
        commands.entity(entity).insert(MovementTarget {
            target: position + toward_center * 12.,
        });
    }
    if qa.stage == 1 {
        camera.zoom = 0.55;
    }
    qa.pending = true;
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(qa.dir.join(name)))
        .observe(|_: On<ScreenshotCaptured>, mut qa: ResMut<Qa>| {
            qa.stage += 1;
            qa.settled = 0;
            qa.pending = false;
        });
}
