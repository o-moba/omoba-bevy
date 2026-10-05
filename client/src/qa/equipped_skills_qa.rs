//! One English phone viewport, using synthetic accepted metadata and a real
//! held-touch gesture. This is HUD evidence, not server or physical-device proof.
use crate::{
    frontend::AppScreen,
    net::{NetworkHeroClass, PlayerLoadout, PlayerProgression},
    player::Player,
};
use bevy::{
    app::AppExit,
    input::touch::{TouchInput, TouchPhase},
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use shared::loadout::{CoreId, LoadoutState, SkillId};
use std::{path::PathBuf, time::Instant};

pub(crate) struct EquippedSkillsQaPlugin;
#[derive(Resource)]
struct Qa {
    directory: PathBuf,
    started: Instant,
    stage: usize,
    frames: u32,
    pending: bool,
    reports: Vec<serde_json::Value>,
}
const FILES: [&str; 2] = ["01-hybrid-hud.png", "02-hybrid-hold-card.png"];
impl Plugin for EquippedSkillsQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_EQUIPPED_SKILLS_QA_DIR").map(PathBuf::from)
        else {
            return;
        };
        std::fs::create_dir_all(&directory).expect("equipped skills QA directory");
        app.insert_resource(Qa {
            directory,
            started: Instant::now(),
            stage: 0,
            frames: 0,
            pending: false,
            reports: Vec::new(),
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(Startup, label)
        .add_systems(PreUpdate, super::combat_qa::focus_capture_window)
        .add_systems(
            Update,
            window.before(crate::mobile_controls::MobileControlsSet::Layout),
        )
        .add_systems(
            Update,
            prepare
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .before(crate::mobile_controls::MobileControlsSet::Input),
        )
        .add_systems(PostUpdate, capture.after(bevy::ui::UiSystems::PostLayout));
    }
}
fn label(mut commands: Commands) {
    commands.spawn((
        Text::new("QA · synthetic hybrid HUD · EN 852×393"),
        TextFont {
            font_size: (9.0).into(),
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::BLACK),
        Pickable::IGNORE,
        GlobalZIndex(5000),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(0.0),
            left: Val::Px(245.0),
            ..default()
        },
    ));
}
fn window(
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut mobile: ResMut<crate::mobile_controls::MobileControls>,
    mut locale: ResMut<crate::i18n::Locale>,
) {
    locale.set(crate::i18n::LocaleId::ENGLISH);
    mobile.enabled = true;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != 852 || window.physical_height() != 393 {
            window.resolution.set_physical_resolution(852, 393);
        }
        if std::env::var("OMOBA_QA_SYNTHETIC_FOCUS").as_deref() == Ok("1") {
            window.focused = true;
            mobile.focused = true;
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn prepare(
    mut qa: ResMut<Qa>,
    screen: Res<State<AppScreen>>,
    mut presses: super::TestIdPresses,
    mut selection: ResMut<crate::team::TeamSelection>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut hero: Query<
        (
            &mut NetworkHeroClass,
            &mut PlayerLoadout,
            &mut PlayerProgression,
            &mut crate::combat::CombatStats,
            Option<&mut crate::net::PlayerSkillCooldowns>,
        ),
        With<Player>,
    >,
    mobile: Res<crate::mobile_controls::MobileControls>,
    mut touch: MessageWriter<TouchInput>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.started.elapsed().as_secs() > 120 {
        let _ = std::fs::write(qa.directory.join("timeout.json"), serde_json::json!({"stage":qa.stage,"frames":qa.frames,"screen":format!("{:?}",screen.get())}).to_string());
        exit.write(AppExit::error());
        return;
    }
    presses.press("HelpDismissButton");
    if *screen.get() == AppScreen::Home {
        presses.press("HomeOfflinePractice");
        return;
    }
    if *screen.get() == AppScreen::HeroSelect {
        selection.hero_class = shared::HeroClass::Dawnweaver;
        selection.character = crate::team::CharacterChoice::Ipfs;
        selection.avatar = Some("agnes".into());
        presses.press("FindMatchButton");
        return;
    }
    let Ok((mut class, mut loadout, mut progression, mut stats, cooldowns)) = hero.single_mut()
    else {
        return;
    };
    qa.frames += 1;
    class.0 = shared::HeroClass::Dawnweaver;
    let mut recipe = CoreId::Dawnweaver.preset();
    recipe.skills = [
        SkillId::WildRocket,
        SkillId::DawnBarrier,
        SkillId::DawnField,
        SkillId::DawnBind,
    ];
    let mut state = LoadoutState {
        recipe: Some(recipe),
        ..default()
    };
    state.slots[2].can_recast = true;
    state.slots[2].recast_remaining_secs = 3.0;
    loadout.0 = Some(state);
    progression.level = 4;
    progression.ranks = [0, 1, 1, 1];
    progression.skill_points = 1;
    stats.hp = stats.max_hp;
    stats.mana = 75.0;
    if let Some(mut cooldowns) = cooldowns {
        cooldowns.remaining_secs = [0.0, 3.25, 0.0, 0.0];
    }
    if qa.stage == 1
        && qa.frames == 10
        && let Ok(window) = windows.single()
    {
        touch.write(TouchInput {
            window,
            phase: TouchPhase::Started,
            position: mobile.layout().ability_centers[0],
            force: None,
            id: 989,
        });
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn capture(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    mobile: Res<crate::mobile_controls::MobileControls>,
    context: Res<crate::input_context::GameplayInputContext>,
    faces: Query<&crate::ui::widgets::game::AbilityView>,
    cards: Query<(Entity, &crate::combat::skill_card::SkillCardView)>,
    children: Query<&Children>,
    texts: Query<(&crate::combat::skill_card::Part, &Text)>,
    geometry: Query<(
        &Node,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&InheritedVisibility>,
        Option<&bevy::ui::CalculatedClip>,
    )>,
    scenes: Query<(
        &WorldAssetRoot,
        Option<&bevy::world_serialization::WorldInstance>,
    )>,
    assets: Res<AssetServer>,
) {
    if qa.pending
        || qa.stage >= FILES.len()
        || qa.frames < 90
        || !context.gameplay_allowed()
        || !mobile.focused
    {
        return;
    }
    if scenes.iter().any(|(scene, instance)| {
        instance.is_none()
            || !matches!(
                assets.recursive_dependency_load_state(scene.0.id()),
                bevy::asset::RecursiveDependencyLoadState::Loaded
            )
    }) {
        return;
    }
    let expected = ["wild_rocket", "dawn_barrier", "dawn_field", "dawn_bind"];
    if !expected
        .iter()
        .all(|id| faces.iter().any(|view| view.ability == Some(*id)))
    {
        return;
    }
    let Some(rocket) = faces
        .iter()
        .find(|view| view.ability == Some("wild_rocket"))
    else {
        return;
    };
    if !rocket.locked || rocket.unlock_level != Some(6) {
        return;
    }
    // Reuse a valid HUD capture when only held-card verification changed.
    if qa.stage == 0 && std::env::var("OMOBA_EQUIPPED_SKILLS_QA_HELD_ONLY").as_deref() == Ok("1") {
        qa.stage = 1;
        qa.frames = 0;
        return;
    }
    let card = cards.iter().find(|(_, card)| card.visible && card.hint);
    // The view is written after this frame's card paint. Wait for the next
    // layout to prove actual rendered text, not just a newly-visible view model.
    let visible_rect = |entity| {
        let (node, computed, transform, inherited, clip) = geometry.get(entity).ok()?;
        if node.display == Display::None
            || computed.size().min_element() <= 1.0
            || inherited.is_some_and(|visibility| !visibility.get())
        {
            return None;
        }
        let rect = crate::ui::focus::node_rect(computed, transform);
        let viewport = Rect::from_corners(Vec2::ZERO, Vec2::new(852.0, 393.0));
        let shown = rect
            .intersect(viewport)
            .intersect(clip.map_or(viewport, |clip| clip.clip));
        (shown.width() >= rect.width() - 1.0 && shown.height() >= rect.height() - 1.0)
            .then_some([rect.min.x, rect.min.y, rect.max.x, rect.max.y])
    };
    let mut rendered = serde_json::Value::Null;
    if qa.stage == 1 {
        let Some((entity, card)) = card else { return };
        if mobile.inspected_skill() != Some(0)
            || !card.locked
            || card
                .skills
                .is_none_or(|skills| skills.ability(shared::SkillSlot::Q).id != "wild_rocket")
        {
            return;
        }
        let Some(rect) = visible_rect(entity) else {
            return;
        };
        if rect[2] - rect[0] < 279.0 || rect[3] - rect[1] < 147.0 {
            return;
        }
        let expected_name =
            crate::i18n::data::ability_name(&shared::loadout::skill(SkillId::WildRocket).ability);
        let expected_unlock = crate::i18n::trf("touch.skill.unlocks", &[("level", &6)]);
        let mut name = None;
        let mut unlock = None;
        for child in children.iter_descendants(entity) {
            let Ok((part, text)) = texts.get(child) else {
                continue;
            };
            let Some(text_rect) = visible_rect(child) else {
                continue;
            };
            if *part == crate::combat::skill_card::Part::Name && text.0 == expected_name {
                name = Some(serde_json::json!({"text":text.0,"rect":text_rect}));
            }
            if *part == crate::combat::skill_card::Part::Availability && text.0 == expected_unlock {
                unlock = Some(serde_json::json!({"text":text.0,"rect":text_rect}));
            }
        }
        let (Some(name), Some(unlock)) = (name, unlock) else {
            return;
        };
        rendered = serde_json::json!({"rect":rect,"name":name,"unlock":unlock});
    }
    let file = FILES[qa.stage];
    qa.reports.push(serde_json::json!({"file":file,"skill_ids":expected,"ultimate_binding":"Q","unlock_level":6,"held_card":card.map(|(_, card)| serde_json::json!({"skill":card.skills.unwrap().ability(shared::SkillSlot::Q).id,"mana":card.mana,"cooldown":card.cooldown,"locked":card.locked,"rendered":rendered}))}));
    qa.pending = true;
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(qa.directory.join(file)))
        .observe(readback);
}
fn readback(shot: On<ScreenshotCaptured>, mut qa: ResMut<Qa>, mut exit: MessageWriter<AppExit>) {
    if shot.image.width() != 852 || shot.image.height() != 393 {
        exit.write(AppExit::error());
        return;
    }
    qa.stage += 1;
    qa.frames = 0;
    qa.pending = false;
    if qa.stage == FILES.len() {
        std::fs::write(qa.directory.join("summary.json"), serde_json::to_vec_pretty(&serde_json::json!({"fixture":"synthetic accepted-loadout HUD","language":"en","viewport":[852,393],"physical_device_verified":false,"server_behavior_verified":false,"captures":qa.reports})).unwrap()).expect("QA summary");
        exit.write(AppExit::Success);
    }
}
