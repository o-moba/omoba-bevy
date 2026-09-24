//! The client's plugin groups. `main` inserts `PlayerVisualMode`, then adds
//! `NetPlugins`, `UiPlugins`, `GameplayPlugins`, `PresentationPlugins` and,
//! with the `qa` feature, `qa::QaPlugins`, in that order.
//!
//! Only the order of `Plugin::build` calls follows from this file; system
//! ordering is declared inside the plugins and is unchanged. Build-time
//! dependencies (a `build` that reads a resource another `build` inserted):
//! - `UiKitPlugin` inserts `UiPlatform` (unless present), which
//!   `MobileControlsPlugin` reads to decide `MobileControls::enabled`, which
//!   `MobileUiPlugin` reads to decide whether to register at all. The three
//!   stay first in `UiPlugins`, in that order.
//! - `FrontendPlugin` (`UiPlugins`) initialises `ScreenDriverPaused`, which the
//!   frontend QA harnesses (`QaPlugins`, later) overwrite with `true`.
//! - The QA harnesses override `WinitSettings` with `continuous()`, after
//!   `platform::configure_app` and `DefaultPlugins`.
//!
//! Nothing else reads the world inside `build`. The debug tooling
//! (`SandboxPlugin`, `PracticeSandboxPlugin`, `GodModePlugin`,
//! `DebugConsolePlugin`) sits in `GameplayPlugins` until step 11 gives it its
//! own group.

use bevy::app::{PluginGroup, PluginGroupBuilder};

use crate::{
    battlefield_atmosphere::BattlefieldAtmospherePlugin, bosses::BossesPlugin,
    camera::CameraPlugin, career::CareerPlugin, career_identity::CareerIdentityPlugin,
    combat::CombatPlugin, combat_feedback::CombatFeedbackPlugin,
    combat_visuals::CombatVisualsPlugin, debug_console::DebugConsolePlugin, decor::DecorPlugin,
    edge_hud::EdgeHudPlugin, frontend::FrontendPlugin, game_audio::GameAudioPlugin,
    game_state::GameStateUiPlugin, game_vfx::GameVfxPlugin, god_mode::GodModePlugin,
    help_overlay::HelpOverlayPlugin, input_context::InputContextPlugin,
    jungle::JungleVisualsPlugin, map_visuals::MapVisualsPlugin, maps::MapsPlugin,
    match_hud::MatchHudPlugin, match_service::MatchServicePlugin, minimap::MinimapPlugin,
    minions::MinionVisualsPlugin, mobile_controls::MobileControlsPlugin, mobile_ui::MobileUiPlugin,
    model_scale::ModelScalePlugin, net::NetworkingPlugin, pause_menu::PauseMenuPlugin,
    persistence::ClientPersistencePlugin, player::PlayerPlugin,
    practice_sandbox::PracticeSandboxPlugin, presentation2d::Presentation2dPlugin,
    presentation3d::Presentation3dPlugin, projectile_visuals::ProjectileVisualsPlugin,
    reaction_visuals::ReactionVisualsPlugin, sandbox::SandboxPlugin, shop::ShopPlugin,
    social::SocialPlugin, sprite::SpriteVisualsPlugin, supporter::SupporterPlugin,
    supporter_storekit::SupporterStoreKitPlugin, team::TeamSelectPlugin,
    team_vision::TeamVisionPlugin, ui::UiKitPlugin, verdant3d::Verdant3dPlugin, world::SetupPlugin,
    world2d::World2dPlugin,
};

/// Preferences, the server session, the match service and the career identity.
pub(crate) struct NetPlugins;

impl PluginGroup for NetPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(ClientPersistencePlugin)
            .add(NetworkingPlugin)
            .add(MatchServicePlugin)
            .add(CareerIdentityPlugin)
    }
}

/// Map layout, input context, the local hero, combat input, and (until step
/// 11) the debug tooling.
pub(crate) struct GameplayPlugins;

impl PluginGroup for GameplayPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(MapsPlugin)
            .add(InputContextPlugin)
            .add(PlayerPlugin)
            .add(CombatPlugin)
            .add(SandboxPlugin)
            .add(PracticeSandboxPlugin)
            .add(GodModePlugin)
            .add(DebugConsolePlugin)
    }
}

/// Everything that draws or plays the world. Both render backends are always
/// added; their systems run under `sprite::in_sprite2d()`/`in_models3d()`
/// because 2D resources (`SpriteVisualAssets`, `MapVisualRegistry`) are read in
/// both modes.
pub(crate) struct PresentationPlugins;

impl PluginGroup for PresentationPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            // Shared by both backends.
            .add(CameraPlugin)
            .add(SetupPlugin)
            .add(ModelScalePlugin)
            .add(CombatVisualsPlugin)
            .add(CombatFeedbackPlugin)
            .add(GameVfxPlugin)
            .add(ReactionVisualsPlugin)
            .add(TeamVisionPlugin)
            .add(GameAudioPlugin)
            .add(MapVisualsPlugin)
            // 2D sprite backend.
            .add(SpriteVisualsPlugin)
            .add(Presentation2dPlugin)
            .add(World2dPlugin)
            // 3D model backend.
            .add(Presentation3dPlugin)
            .add(Verdant3dPlugin)
            .add(DecorPlugin)
            .add(JungleVisualsPlugin)
            .add(MinionVisualsPlugin)
            .add(BossesPlugin)
            .add(ProjectileVisualsPlugin)
            .add(BattlefieldAtmospherePlugin)
    }
}

/// Screens, HUD, menus and overlays. `UiKitPlugin` → `MobileControlsPlugin` →
/// `MobileUiPlugin` is a build-time chain (see the module docs).
pub(crate) struct UiPlugins;

impl PluginGroup for UiPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(UiKitPlugin)
            .add(MobileControlsPlugin)
            .add(MobileUiPlugin)
            .add(FrontendPlugin)
            .add(TeamSelectPlugin)
            .add(GameStateUiPlugin)
            .add(MatchHudPlugin)
            .add(EdgeHudPlugin)
            .add(MinimapPlugin)
            .add(ShopPlugin)
            .add(HelpOverlayPlugin)
            .add(PauseMenuPlugin)
            .add(SocialPlugin)
            .add(CareerPlugin)
            .add(SupporterPlugin)
            .add(SupporterStoreKitPlugin)
    }
}
