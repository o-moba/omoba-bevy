//! Focused native raster proof, opt-in via OMOBA_FOG_COVERAGE_QA_DIR.
//! Uses the real fog entity; only its test texture/background and z-order are
//! overridden. Auto is the negative control, then its production mode is restored.
use super::*;
use bevy::{
    app::AppExit,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};
use std::{path::PathBuf, time::Instant};

#[derive(Resource)]
struct Capture {
    directory: PathBuf,
    started: Instant,
    frame: u32,
    production_mode: Option<NodeImageMode>,
}

pub(super) fn install(app: &mut App) {
    let Some(directory) = std::env::var_os("OMOBA_FOG_COVERAGE_QA_DIR").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&directory).expect("fog QA directory");
    app.insert_resource(Capture {
        directory,
        started: Instant::now(),
        frame: 0,
        production_mode: None,
    })
    .insert_resource(bevy::winit::WinitSettings::continuous())
    .add_systems(Update, show_fog.after(sync_visibility))
    .add_systems(Last, capture);
}

// Override the fixture display before UI layout; changing it in Last alone
// would leave the computed rectangle empty while the real session is in Home.
fn show_fog(mut fog: Query<&mut Node, With<FogOverlay>>) {
    for mut node in &mut fog {
        node.display = Display::Flex;
    }
}

fn capture(
    mut commands: Commands,
    mut qa: ResMut<Capture>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut fog: Query<(&mut Node, &mut ImageNode, &mut GlobalZIndex), With<FogOverlay>>,
    mut images: ResMut<Assets<Image>>,
    mut pacing: ResMut<bevy::winit::WinitSettings>,
    mut exit: MessageWriter<AppExit>,
) {
    *pacing = bevy::winit::WinitSettings::continuous();
    if qa.started.elapsed().as_secs() > 45 {
        panic!("fog raster capture timed out");
    }
    let Ok((mut node, mut image, mut z)) = fog.single_mut() else {
        return;
    };
    if qa.frame == 0 {
        windows.single_mut().unwrap().resolution.set(852.0, 393.0);
        qa.production_mode = Some(image.image_mode.clone());
        assert_eq!(image.image_mode, NodeImageMode::Stretch);
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.),
                height: Val::Percent(100.),
                ..default()
            },
            BackgroundColor(Color::WHITE),
            GlobalZIndex(99_999),
            Pickable::IGNORE,
        ));
    }
    node.display = Display::Flex;
    z.0 = 100_000;
    image.image_mode = if qa.frame < 60 {
        NodeImageMode::Auto
    } else {
        qa.production_mode.clone().unwrap()
    };
    let mut texture = images.get_mut(&image.image).unwrap();
    for pixel in texture.data.as_mut().unwrap().chunks_exact_mut(4) {
        pixel.copy_from_slice(&[8, 18, 24, 163]);
    }
    let name = match qa.frame {
        45 => Some("auto-control.png"),
        105 => Some("stretch-fixed.png"),
        _ => None,
    };
    if let Some(name) = name {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(qa.directory.join(name)));
    }
    qa.frame += 1;
    if qa.frame > 110 && qa.directory.join("stretch-fixed.png").is_file() {
        exit.write(AppExit::Success);
    }
}
