//! Focused RU phone-preview proof of first binding and a cold region switch.
//! Opt in with OMOBA_ACCOUNT_REGION_QA=<output>, an isolated config directory,
//! and a dev-contour build. No production connection or real account is used.
use crate::{
    account_region::{self, RegionId},
    frontend::boot::BootSplash,
};
use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) struct AccountRegionQaPlugin;
impl Plugin for AccountRegionQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(output) = std::env::var_os("OMOBA_ACCOUNT_REGION_QA").filter(|s| !s.is_empty())
        else {
            return;
        };
        app.insert_resource(bevy::winit::WinitSettings::continuous())
            .insert_resource(Proof {
                output: output.into(),
                stage: 0,
                since: Instant::now(),
                started: Instant::now(),
                pending: false,
                checks: Vec::new(),
            })
            .add_systems(PostUpdate, step.after(bevy::ui::UiSystems::Layout));
    }
}
#[derive(Resource)]
struct Proof {
    output: PathBuf,
    stage: u8,
    since: Instant,
    started: Instant,
    pending: bool,
    checks: Vec<(&'static str, bool)>,
}
#[allow(clippy::too_many_arguments)]
fn step(
    mut commands: Commands,
    mut proof: ResMut<Proof>,
    splash: Res<BootSplash>,
    mut consent: ResMut<crate::legal::LegalConsent>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut presses: super::TestIdPresses,
    mut exit: MessageWriter<AppExit>,
) {
    if crate::session_config::BUILD_CONTOUR != Some("dev")
        || std::env::var_os("OMOBA_CLIENT_CONFIG_DIR").is_none()
        || proof.started.elapsed() > Duration::from_secs(60)
    {
        error!(
            "ACCOUNT_REGION_QA requires isolated dev settings and must finish within 60s; stage {}",
            proof.stage
        );
        exit.write(AppExit::error());
        return;
    }
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != 852 || window.physical_height() != 393 {
            window.resolution.set_physical_resolution(852, 393);
        }
    }
    if proof.pending || splash.blocks_input() {
        return;
    }
    let shot = match proof.stage {
        0 => {
            proof.checks.push((
                "first launch waits for account region",
                !account_region::bound(),
            ));
            proof.checks.push((
                "regional features are closed before confirmation",
                !account_region::features().ekza && !account_region::features().vk_sign_in,
            ));
            if !presses.press("AccountRegion-ru") {
                return;
            }
            proof.stage = 1;
            proof.since = Instant::now();
            return;
        }
        1 => {
            if proof.since.elapsed() < Duration::from_millis(750) {
                return;
            }
            "ru-account-region.png"
        }
        2 => {
            if !presses.press("LegalAccept") {
                return;
            }
            proof.stage = 3;
            return;
        }
        3 => {
            if !presses.press("LegalContinue") {
                return;
            }
            proof.stage = 4;
            proof.since = Instant::now();
            return;
        }
        4 => {
            if proof.since.elapsed() < Duration::from_millis(900) {
                return;
            }
            proof.checks.push((
                "first confirmation binds RU",
                shared::region::active().is_some_and(|r| r.id == RegionId::Ru)
                    && account_region::bound(),
            ));
            proof.checks.push((
                "RU uses its own dev lobby",
                crate::connection_profile::active().lobby == "lobby.dev.omoba.ru:4100",
            )); // i18n-allow
            proof.checks.push((
                "RU enables VK and disables Ekza",
                account_region::features().vk_sign_in && !account_region::features().ekza,
            ));
            proof.checks.push((
                "RU identity loaded after first confirmation",
                account_region::regional_directory(RegionId::Ru)
                    .is_some_and(|p| p.join("career_identity.json").is_file()),
            ));
            consent.open_region_choice();
            proof.stage = 5;
            return;
        }
        5 => {
            if !presses.press("AccountRegion-global") {
                return;
            }
            proof.stage = 6;
            return;
        }
        6 => {
            if !presses.press("LegalContinue") {
                return;
            }
            proof.stage = 7;
            proof.since = Instant::now();
            return;
        }
        7 => {
            if proof.since.elapsed() < Duration::from_millis(750) {
                return;
            }
            proof.checks.push((
                "switch closes regional traffic until restart",
                !account_region::bound()
                    && !account_region::features().ekza
                    && !account_region::features().vk_sign_in,
            ));
            proof.checks.push((
                "running process never rebinds",
                shared::region::active().is_some_and(|r| r.id == RegionId::Ru),
            ));
            proof.checks.push((
                "next region saved for cold restart",
                account_region::path()
                    .as_deref()
                    .and_then(account_region::read)
                    .is_some_and(|s| s.pending == Some(RegionId::Global)),
            ));
            "region-restart.png"
        }
        _ => {
            let pass = proof.checks.iter().all(|(_, pass)| *pass);
            let result = serde_json::json!({"pass":pass,"locale":"ru","viewport":[852,393],"checks":proof.checks.iter().map(|(check, pass)|serde_json::json!({"check":check,"pass":pass})).collect::<Vec<_>>()});
            let written = std::fs::write(
                proof.output.join("result.json"),
                serde_json::to_vec_pretty(&result).unwrap(),
            );
            info!("ACCOUNT_REGION_QA: {result}");
            exit.write(if pass && written.is_ok() {
                AppExit::Success
            } else {
                AppExit::error()
            });
            proof.pending = true;
            return;
        }
    };
    if std::fs::create_dir_all(&proof.output).is_err() {
        exit.write(AppExit::error());
        return;
    }
    proof.pending = true;
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(proof.output.join(shot)))
        .observe(|_: On<ScreenshotCaptured>, mut proof: ResMut<Proof>| {
            proof.pending = false;
            proof.stage += 1;
            proof.since = Instant::now();
        });
}
