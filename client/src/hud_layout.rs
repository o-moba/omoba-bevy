//! HUD geometry from the Verdant Crown handoff (`omoba-ui/handoff/screens/hud.md`,
//! redlines `hud-desktop.png` 1280 × 720 and `hud-phone.png` 844 × 390).
//!
//! One table of region rectangles in logical UI pixels: desktop regions are
//! anchored to the window edges (or centred) exactly as the redline measures
//! them, phone regions to `MobileControls.safe` and the compact match metrics.
//! Every HUD owner tags its root with a [`HudRegion`] and [`place_hud_regions`]
//! positions it, so the numbers live once and the unit tests below pin them
//! against the redline.
// i18n-strict
use bevy::{prelude::*, window::PrimaryWindow};

use crate::mobile_controls::{MobileControls, MobileSafeInsets};
use crate::ui::theme::Form;
use crate::ui::tokens::{size, space};

pub(crate) mod phone_rows;

/// Plate sizes the redline measures in px without a token of their own.
pub(crate) mod plate {
    use bevy::prelude::Vec2;

    pub const BUFF_CHIPS: Vec2 = Vec2::new(344.0, 52.0);
    pub const BRUSH_CHIP: Vec2 = Vec2::new(144.0, 24.0);
    pub const SCORE_STRIP_DESKTOP: Vec2 = Vec2::new(176.0, 40.0);
    pub const SCORE_STRIP_PHONE: Vec2 = Vec2::new(160.0, 24.0);
    pub const FEEDBACK_DESKTOP: Vec2 = Vec2::new(344.0, 52.0);
    pub const FEEDBACK_PHONE: Vec2 = Vec2::new(220.0, 40.0);
    pub const UPGRADE_CHIP: Vec2 = Vec2::new(148.0, 24.0);
    pub const ABILITY_BAR: Vec2 = Vec2::new(344.0, 108.0);
    pub const PLAYER_STATUS_DESKTOP: Vec2 = Vec2::new(288.0, 72.0);
    pub const EQUIPMENT: Vec2 = Vec2::new(200.0, 102.0);
    pub const TARGET_FRAME_H_DESKTOP: f32 = 56.0;
    /// Controller reminder strip, bottom-centre above the ability bar
    /// (hud.md § States, Controller active: y 520–572 at 720).
    pub const LEGEND_H: f32 = 52.0;
    /// Skill description tooltip / phone hold card (`skill-description.md`).
    pub const SKILL_CARD: Vec2 = Vec2::new(280.0, 148.0);
}

/// Desktop bottom offsets (distance from the region's top to the window
/// bottom at 720): the lower regions stay glued to the bottom edge.
const FEEDBACK_FROM_BOTTOM: f32 = 216.0;
const FEEDBACK_FROM_BOTTOM_PAD: f32 = 268.0;
const UPGRADE_FROM_BOTTOM: f32 = 156.0;
const ABILITY_FROM_BOTTOM: f32 = 124.0;
const STATUS_FROM_BOTTOM: f32 = 88.0;
const EQUIPMENT_FROM_BOTTOM: f32 = 118.0;
const LEGEND_FROM_BOTTOM: f32 = 200.0;
/// Left edge of the protected desktop centre (hud.md: x 384 of 1280).
const PROTECTED_LEFT: f32 = 0.30;
/// Desktop rows below the top plates.
const BRUSH_TOP: f32 = 80.0;
const BUFF_TOP: f32 = 200.0;
const KILL_FEED_TOP: f32 = 100.0;
/// Phone faces use less paint without reducing their independent tap areas.
pub(crate) const PHONE_QUICK_BUY_SLOT: f32 = phone_rows::BUY_SLOT;
pub(crate) const PHONE_QUICK_BUY_GAP: f32 = 0.0;
pub(crate) const PHONE_QUICK_BUY_FACE: f32 = 30.0;
/// Phone skill hold card: `safe top + 64` (skill-description.md).
const PHONE_CARD_TOP: f32 = 64.0;
/// Desktop skill tooltip bottom: `space.8` above the upgrade chip (564 at
/// 720) or, without the chip, above the ability bar plate (596).
const TOOLTIP_BOTTOM_WITH_CHIP: f32 = UPGRADE_FROM_BOTTOM + space::S8;
const TOOLTIP_BOTTOM_WITHOUT_CHIP: f32 = ABILITY_FROM_BOTTOM + space::S8;

/// Width of the service group: neighboring phone targets or the desktop row.
pub(crate) fn icon_row_width(form: Form) -> f32 {
    if form == Form::Phone {
        88.0
    } else {
        3.0 * size::ICON_BUTTON.at(form) + 2.0 * space::S8
    }
}

/// A point projected by a camera (`world_to_viewport`: logical window
/// pixels) in logical UI pixels, where a UI node is placed: ÷ `UiScale`
/// (F8.1). World-anchored overlays (boss nameplates, floating combat
/// numbers, chat bubbles, the target lock frame, the aim preview) place
/// their nodes with this, so the desktop match can use the R2.3a scale.
pub(crate) fn world_to_ui(point: Vec2, ui_scale: Option<&UiScale>) -> Vec2 {
    point / ui_scale.map_or(1.0, |scale| scale.0).max(0.1)
}

/// The logical UI viewport (window logical size ÷ `UiScale`).
pub(crate) fn ui_viewport(window: &Window, ui_scale: Option<&UiScale>) -> Vec2 {
    let scale = ui_scale.map_or(1.0, |scale| scale.0).max(0.1);
    Vec2::new(window.width(), window.height()) / scale
}

/// Every HUD region of hud.md, in logical UI pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HudLayout {
    pub form: Form,
    pub viewport: Vec2,
    pub minimap: Rect,
    pub buff_chips: Rect,
    pub target_frame: Rect,
    pub allies: Rect,
    pub ally_cell_width: f32,
    pub ally_cell_height: f32,
    pub ally_cell_gap: f32,
    pub enemy_deaths: [Rect; 5],
    pub enemy_live: [Rect; 5],
    pub kill_feed: Rect,
    pub brush_chip: Rect,
    pub score_strip: Rect,
    pub icon_buttons: Rect,
    pub action_feedback: Rect,
    pub upgrade_chip: Rect,
    pub ability_bar: Rect,
    pub player_status: Rect,
    pub equipment: Rect,
    /// Phone quick-buy row (gold + two offers); on desktop the quick-buy
    /// column lives inside the equipment plate.
    pub quick_buy: Rect,
    pub legend: Rect,
    /// Desktop: the tooltip's width and bottom edge (x follows the hovered
    /// slot); phone: the hold card rectangle.
    pub skill_card: Rect,
}

fn rect(x: f32, y: f32, size: Vec2) -> Rect {
    Rect::from_corners(Vec2::new(x, y), Vec2::new(x + size.x, y + size.y))
}

fn phone_minimap_size(viewport: Vec2) -> f32 {
    size::MINIMAP.phone
        * if viewport.min_element() >= 600.0 {
            1.2
        } else {
            1.0
        }
}

/// The left information stack ends below quick-buy, independently of the
/// movement layout. Floating-stick bounds can reserve this rectangle without
/// calling `MobileControls::layout` recursively.
fn phone_plan(viewport: Vec2, safe: MobileSafeInsets) -> phone_rows::Layout {
    phone_rows::solve(phone_rows::Input {
        width: viewport.x,
        height: viewport.y,
        safe_left: safe.left,
        safe_right: safe.right,
        safe_top: safe.top,
        map_side: phone_minimap_size(viewport),
    })
}
fn from_phone_box(b: phone_rows::Box2) -> Rect {
    rect(b.left, b.top, Vec2::new(b.width, b.height))
}
pub(crate) fn phone_kill_feed_rect(viewport: Vec2, safe: MobileSafeInsets) -> Rect {
    from_phone_box(phone_plan(viewport, safe).kill_feed)
}

impl HudLayout {
    /// The layout family the HUD draws for (phone HUD = `MobileControls.enabled`).
    pub(crate) fn resolve(viewport: Vec2, mobile: Option<&MobileControls>, pad: bool) -> Self {
        match mobile.filter(|mobile| mobile.enabled) {
            Some(mobile) => Self::phone(mobile),
            None => Self::desktop(viewport, pad),
        }
    }

    /// Desktop at any logical viewport; `pad` = a controller owns input (the
    /// reminder strip takes y 520–572 and the action feedback moves up).
    pub(crate) fn desktop(viewport: Vec2, pad: bool) -> Self {
        let (w, h) = (viewport.x, viewport.y);
        let inset = space::S16;
        let form = Form::Desktop;
        let icons = icon_row_width(form);
        let icon_row_x = w - inset - icons;
        let score = plate::SCORE_STRIP_DESKTOP;
        let score_x = icon_row_x - space::S12 - score.x;
        let target_w = size::TARGET_FRAME_WIDTH.desktop;
        let centre = |width: f32| (w - width) * 0.5;
        // The ability bar, its feedback and the upgrade chip share one
        // centre line, never closer than the inset to the left edge.
        let bar_x = centre(plate::ABILITY_BAR.x).max(inset);
        let bar_centre = bar_x + plate::ABILITY_BAR.x * 0.5;
        let feedback_top = h - if pad {
            FEEDBACK_FROM_BOTTOM_PAD
        } else {
            FEEDBACK_FROM_BOTTOM
        };
        let card = plate::SKILL_CARD;
        Self {
            form,
            viewport,
            allies: rect(
                inset + size::MINIMAP.desktop + 8.0,
                inset + 50.0,
                Vec2::new(176.0, 44.0),
            ),
            ally_cell_width: 42.5,
            ally_cell_height: 44.0,
            ally_cell_gap: 2.0,
            enemy_deaths: [Rect::default(); 5],
            enemy_live: [Rect::default(); 5],
            kill_feed: rect(
                (w - inset - 280.0).max(space::S16),
                KILL_FEED_TOP,
                Vec2::new(280.0, 98.0),
            ),
            minimap: rect(inset, inset, Vec2::splat(size::MINIMAP.desktop)),
            // Max 344 wide; narrow windows cap it at the protected centre
            // (30 % of the width), where long chips clip.
            buff_chips: rect(
                inset,
                BUFF_TOP,
                Vec2::new(
                    plate::BUFF_CHIPS.x.min(w * PROTECTED_LEFT - inset),
                    plate::BUFF_CHIPS.y,
                ),
            ),
            target_frame: rect(
                centre(target_w),
                inset,
                Vec2::new(target_w, plate::TARGET_FRAME_H_DESKTOP),
            ),
            brush_chip: rect(centre(plate::BRUSH_CHIP.x), BRUSH_TOP, plate::BRUSH_CHIP),
            score_strip: rect(score_x, inset, score),
            icon_buttons: rect(
                icon_row_x,
                inset,
                Vec2::new(icons, size::ICON_BUTTON.desktop),
            ),
            action_feedback: rect(bar_x, feedback_top, plate::FEEDBACK_DESKTOP),
            upgrade_chip: rect(
                bar_centre - plate::UPGRADE_CHIP.x * 0.5,
                h - UPGRADE_FROM_BOTTOM,
                plate::UPGRADE_CHIP,
            ),
            ability_bar: rect(bar_x, h - ABILITY_FROM_BOTTOM, plate::ABILITY_BAR),
            player_status: rect(inset, h - STATUS_FROM_BOTTOM, plate::PLAYER_STATUS_DESKTOP),
            equipment: rect(
                w - inset - plate::EQUIPMENT.x,
                h - EQUIPMENT_FROM_BOTTOM,
                plate::EQUIPMENT,
            ),
            quick_buy: Rect::default(),
            legend: rect(
                bar_x,
                h - LEGEND_FROM_BOTTOM,
                Vec2::new(plate::ABILITY_BAR.x, plate::LEGEND_H),
            ),
            skill_card: rect(
                bar_centre - card.x * 0.5,
                h - TOOLTIP_BOTTOM_WITH_CHIP - card.y,
                card,
            ),
        }
    }

    /// The desktop tooltip's bottom edge for a slot row with or without the
    /// upgrade chip under it.
    pub(crate) fn tooltip_bottom(&self, chip: bool) -> f32 {
        self.viewport.y
            - if chip {
                TOOLTIP_BOTTOM_WITH_CHIP
            } else {
                TOOLTIP_BOTTOM_WITHOUT_CHIP
            }
    }

    /// Compact phone chrome anchored to `MobileControls.safe`: preserve the
    /// map's readable size, keep service targets alongside it and paint only
    /// a shallow score ribbon on the opposite edge.
    pub(crate) fn phone(mobile: &MobileControls) -> Self {
        let viewport = mobile.viewport;
        let safe = mobile.safe;
        let plan = phone_plan(viewport, safe);
        let ally_cells = plan.ally_cells();
        let safe_centre = (safe.left + viewport.x - safe.right) * 0.5;
        let card = plate::SKILL_CARD;
        let bar = rect(
            safe_centre - plate::ABILITY_BAR.x * 0.5,
            viewport.y - safe.bottom - plate::ABILITY_BAR.y,
            plate::ABILITY_BAR,
        );
        let chip = rect(
            safe_centre - plate::UPGRADE_CHIP.x * 0.5,
            bar.min.y - space::S8 - plate::UPGRADE_CHIP.y,
            plate::UPGRADE_CHIP,
        );
        Self {
            form: Form::Phone,
            viewport,
            minimap: from_phone_box(plan.minimap),
            allies: from_phone_box(plan.allies),
            ally_cell_width: ally_cells[0].width,
            ally_cell_height: ally_cells[0].height,
            ally_cell_gap: ally_cells[1].left - ally_cells[0].right(),
            enemy_deaths: plan.deaths.map(from_phone_box),
            enemy_live: plan.live_slots().map(from_phone_box),
            target_frame: from_phone_box(plan.target),
            score_strip: from_phone_box(plan.score),
            icon_buttons: rect(
                plan.menu.left,
                plan.menu.top,
                Vec2::new(plan.chat.right() - plan.menu.left, plan.menu.height),
            ),
            brush_chip: from_phone_box(plan.brush),
            action_feedback: from_phone_box(plan.feedback),
            kill_feed: from_phone_box(plan.kill_feed),
            quick_buy: from_phone_box(plan.quick_buy),
            player_status: from_phone_box(plan.player_status),
            buff_chips: Rect::default(),
            upgrade_chip: chip,
            ability_bar: bar,
            equipment: Rect::default(),
            legend: Rect::default(),
            skill_card: rect(safe_centre - card.x * 0.5, safe.top + PHONE_CARD_TOP, card),
        }
    }

    /// Passive public death timers live under allies on phones. Desktop
    /// retains its shared right-hand column through the portrait owner.
    pub(crate) fn enemy_death_portrait_slots(&self) -> [Rect; 5] {
        if self.form == Form::Phone {
            self.enemy_deaths
        } else {
            self.enemy_portrait_slots()
        }
    }

    /// Five hero targets fit the free right edge. On a phone two rows wrap
    /// leftward above the skill orbit; the fifth uses only the top row.
    pub(crate) fn enemy_portrait_slots(&self) -> [Rect; 5] {
        if self.form == Form::Phone {
            return self.enemy_live;
        }
        let side = 44.0;
        let step = side + space::S4;
        std::array::from_fn(|slot| {
            rect(
                self.icon_buttons.max.x - side,
                self.kill_feed.max.y + space::S12 + slot as f32 * step,
                Vec2::splat(side),
            )
        })
    }

    /// Compact phone notices follow the minimap and quick-buy on the left.
    pub(crate) fn kill_feed(&self) -> Rect {
        self.kill_feed
    }

    /// The rectangle of a tagged region (sub-cells of the icon row included).
    pub(crate) fn region(&self, region: HudRegion) -> Rect {
        let button = if self.form == Form::Phone {
            44.0
        } else {
            size::ICON_BUTTON.at(self.form)
        };
        match region {
            HudRegion::BuffChips | HudRegion::PracticeBadge => self.buff_chips,
            HudRegion::TargetFrame => self.target_frame,
            HudRegion::BrushChip => self.brush_chip,
            HudRegion::ScoreStrip if self.form == Form::Phone => rect(
                self.score_strip.min.x,
                self.score_strip.min.y,
                Vec2::new(self.score_strip.width(), 44.0),
            ),
            HudRegion::ScoreStrip => self.score_strip,
            HudRegion::SocialEntry if self.form == Form::Phone => rect(
                self.icon_buttons.min.x + button,
                self.icon_buttons.min.y,
                Vec2::splat(button),
            ),
            HudRegion::SocialEntry => Rect::from_corners(
                self.icon_buttons.min,
                self.icon_buttons.min
                    + Vec2::new(
                        if self.form == Form::Phone {
                            button
                        } else {
                            2.0 * button + space::S8
                        },
                        button,
                    ),
            ),
            HudRegion::MenuButton if self.form == Form::Phone => rect(
                self.icon_buttons.min.x,
                self.icon_buttons.min.y,
                Vec2::splat(button),
            ),
            HudRegion::MenuButton => Rect::from_corners(
                self.icon_buttons.max - Vec2::splat(button),
                self.icon_buttons.max,
            ),
            HudRegion::ActionFeedback => self.action_feedback,
            HudRegion::UpgradeChip => self.upgrade_chip,
            HudRegion::AbilityBar => self.ability_bar,
            HudRegion::PlayerStatus => self.player_status,
            HudRegion::Equipment => self.equipment,
            HudRegion::QuickBuy => self.quick_buy,
            HudRegion::ContourBadge => self.contour_badge(),
        }
    }

    /// The contour badge: `space.8` under the score strip, left-aligned with
    /// it, one `size.badge.height` high (the free strip above the kill feed).
    /// The phone HUD has no free slot there (the enemy portraits wrap under
    /// the icon row), so it has none and `crate::contour_badge` spawns the
    /// badge only for the desktop HUD; dev-contour builds are desktop-only.
    fn contour_badge(&self) -> Rect {
        if self.form == Form::Phone {
            return Rect::default();
        }
        let top = self.score_strip.max.y + space::S8;
        Rect::from_corners(
            Vec2::new(self.score_strip.min.x, top),
            Vec2::new(self.score_strip.max.x, top + size::BADGE_HEIGHT),
        )
    }
}

/// A HUD root placed by [`place_hud_regions`] from the [`HudLayout`].
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HudRegion {
    BuffChips,
    /// The desktop offline-practice badge atop the buff-chip column (hugs
    /// its text).
    PracticeBadge,
    TargetFrame,
    BrushChip,
    ScoreStrip,
    /// Chat + reactions: the first two cells of the icon row.
    SocialEntry,
    /// The menu `≡`: the last cell of the icon row.
    MenuButton,
    ActionFeedback,
    UpgradeChip,
    AbilityBar,
    PlayerStatus,
    Equipment,
    QuickBuy,
    /// A contour build's `DEV` badge (`crate::contour_badge`) under the
    /// score strip (hugs its text).
    ContourBadge,
}

/// How a region's node takes its rectangle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Anchor {
    /// Left/top and the region's size.
    Box,
    /// Left/top; the content decides the size.
    TopLeft,
    /// Horizontally centred on the region, top edge.
    TopCentre,
    /// Horizontally centred on the region, bottom edge (grows upward).
    BottomCentre,
}

impl HudRegion {
    fn anchor(self, form: Form) -> Anchor {
        match self {
            HudRegion::QuickBuy | HudRegion::PracticeBadge | HudRegion::ContourBadge => {
                Anchor::TopLeft
            }
            HudRegion::BrushChip => Anchor::TopCentre,
            HudRegion::ActionFeedback if form == Form::Phone => Anchor::TopCentre,
            HudRegion::ActionFeedback => Anchor::BottomCentre,
            HudRegion::UpgradeChip => Anchor::TopCentre,
            _ => Anchor::Box,
        }
    }
}

/// Positions every [`HudRegion`] root from the current layout (logical UI
/// pixels: the window ÷ `UiScale` on desktop, `MobileControls` on phone).
#[allow(clippy::type_complexity)]
pub(crate) fn place_hud_regions(
    windows: Query<&Window, With<PrimaryWindow>>,
    ui_scale: Option<Res<UiScale>>,
    mobile: Option<Res<MobileControls>>,
    gamepad: Option<Res<crate::gamepad::GamepadControls>>,
    mut nodes: Query<(&HudRegion, &mut Node, Option<&mut UiTransform>)>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let pad = gamepad.as_ref().is_some_and(|pad| pad.active);
    let layout = HudLayout::resolve(
        ui_viewport(window, ui_scale.as_deref()),
        mobile.as_deref(),
        pad,
    );
    for (region, mut node, transform) in &mut nodes {
        let rect = layout.region(*region);
        let anchor = region.anchor(layout.form);
        let (left, right, top, bottom, width, height, shift) = match anchor {
            Anchor::Box => (
                Val::Px(rect.min.x),
                Val::Auto,
                Val::Px(rect.min.y),
                Val::Auto,
                Val::Px(rect.width()),
                Val::Px(rect.height()),
                0.0,
            ),
            Anchor::TopLeft => (
                Val::Px(rect.min.x),
                Val::Auto,
                Val::Px(rect.min.y),
                Val::Auto,
                node.width,
                node.height,
                0.0,
            ),
            Anchor::TopCentre => (
                Val::Px(rect.center().x),
                Val::Auto,
                Val::Px(rect.min.y),
                Val::Auto,
                node.width,
                node.height,
                -50.0,
            ),
            Anchor::BottomCentre => (
                Val::Px(rect.center().x),
                Val::Auto,
                Val::Auto,
                Val::Px(layout.viewport.y - rect.max.y),
                node.width,
                node.height,
                -50.0,
            ),
        };
        let next = (left, right, top, bottom, width, height);
        let current = (
            node.left,
            node.right,
            node.top,
            node.bottom,
            node.width,
            node.height,
        );
        if current != next {
            node.left = left;
            node.right = right;
            node.top = top;
            node.bottom = bottom;
            node.width = width;
            node.height = height;
        }
        if let Some(mut transform) = transform {
            let translation = Val2::new(Val::Percent(shift), Val::Px(0.0));
            if transform.translation != translation {
                transform.translation = translation;
            }
        }
    }
}

pub(crate) struct HudLayoutPlugin;

impl Plugin for HudLayoutPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            place_hud_regions.before(bevy::ui::UiSystems::Layout),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_kill_notices_follow_left_quick_buy_and_reserve_room_for_the_stick() {
        for viewport in [
            Vec2::new(844.0, 384.0),
            Vec2::new(693.0, 320.0),
            Vec2::new(568.0, 320.0),
        ] {
            for inset in [32.0, 47.0] {
                let mut mobile = MobileControls::default();
                mobile.viewport = viewport;
                mobile.safe.left = inset;
                mobile.safe.right = inset;
                mobile.safe.top = 4.0;
                let hud = HudLayout::phone(&mobile);
                let feed = hud.kill_feed();
                assert_eq!(feed, phone_kill_feed_rect(viewport, mobile.safe));
                assert_eq!(feed.min.x, hud.minimap.min.x);
                assert_eq!(feed.min.y, hud.quick_buy.max.y + phone_rows::GAP);
                assert_eq!(feed.width(), 180.0);
                let short = viewport.y <= 340.0;
                assert_eq!(
                    feed.height(),
                    if short {
                        phone_rows::FEED_SHORT_HEIGHT
                    } else {
                        phone_rows::FEED_HEIGHT
                    }
                );
                let stick_diameter = mobile.layout().joystick_radius * 2.0;
                assert!(stick_diameter >= 44.0);
                assert!(feed.max.y + space::S4 + stick_diameter <= viewport.y - mobile.safe.bottom);
                for occupied in [
                    hud.minimap,
                    hud.quick_buy,
                    hud.player_status,
                    hud.region(HudRegion::MenuButton),
                    hud.region(HudRegion::SocialEntry),
                    hud.allies,
                    hud.target_frame,
                    hud.region(HudRegion::ScoreStrip),
                ]
                .into_iter()
                .chain(hud.enemy_portrait_slots())
                {
                    let overlap = feed.intersect(occupied);
                    assert!(
                        overlap.width() <= 0.0 || overlap.height() <= 0.0,
                        "{viewport:?}/{inset}: feed {feed:?} covers {occupied:?}"
                    );
                }
                let controls = mobile.layout();
                assert!(
                    controls.joystick_center.y - controls.joystick_radius >= feed.max.y + space::S4,
                    "{viewport:?}/{inset}: stick base must stay below the left feed"
                );
                let utility_hit = controls.auxiliary_radius.max(22.0);
                assert!(feed.max.x < controls.attack_center.x - controls.attack_radius);
                for centre in controls
                    .utility_centers
                    .into_iter()
                    .chain([controls.recall_center])
                {
                    assert!(
                        feed.max.y <= centre.y - utility_hit - space::S4 + 0.001,
                        "{viewport:?}/{inset}: feed {feed:?} covers utility {centre:?}"
                    );
                    assert!(centre.distance(centre.clamp(feed.min, feed.max)) >= utility_hit);
                }
            }
        }
    }

    #[test]
    fn left_feed_helper_tracks_safe_insets_and_tablet_map_size() {
        let mut mobile = MobileControls::default();
        mobile.safe.left = 47.0;
        mobile.safe.right = 47.0;
        mobile.safe.top = 12.0;
        for (viewport, expected) in [
            (Vec2::new(844.0, 384.0), (57.0, 184.0, 180.0, 30.0)),
            (Vec2::new(568.0, 320.0), (57.0, 216.0, 180.0, 16.0)),
            (Vec2::new(1180.0, 820.0), (57.0, 208.0, 180.0, 30.0)),
        ] {
            mobile.viewport = viewport;
            let feed = phone_kill_feed_rect(viewport, mobile.safe);
            assert_eq!(at(feed), expected);
            assert_eq!(HudLayout::phone(&mobile).kill_feed(), feed);
            assert!(feed.max.x <= viewport.x - mobile.safe.right);
            let stick = mobile.layout();
            assert!(stick.joystick_radius >= 22.0);
            assert!(
                feed.max.y + space::S4 + stick.joystick_radius * 2.0
                    <= viewport.y - mobile.safe.bottom
            );
        }
    }

    #[test]
    fn phone_allies_and_target_share_top_edge_at_every_supported_width() {
        for width in [568.0, 693.0, 791.0, 795.0, 796.0, 797.0, 844.0] {
            for inset in [32.0, 47.0] {
                let mut mobile = MobileControls::default();
                mobile.viewport = Vec2::new(width, 320.0);
                mobile.safe.left = inset;
                mobile.safe.right = inset;
                mobile.safe.top = 4.0;
                let hud = HudLayout::phone(&mobile);
                let target = hud.target_frame;
                assert_eq!(target.height(), phone_rows::TARGET_HEIGHT);
                assert!(target.width() >= 126.0 && target.width() <= 144.0);
                assert_eq!(target.min.y, mobile.safe.top);
                assert_eq!(target.center().x, width * 0.5);
                assert_eq!(hud.allies.width(), 134.0);
                assert_eq!(hud.allies.min.y, mobile.safe.top);
                assert_eq!(hud.ally_cell_width, 32.0);
                assert_eq!(hud.ally_cell_height, 44.0);
                assert_eq!(hud.ally_cell_gap, 2.0);
                assert_eq!(
                    hud.enemy_death_portrait_slots()[0].min.y,
                    hud.allies.max.y + 4.0
                );
                let occupied = [
                    hud.minimap,
                    hud.allies,
                    hud.region(HudRegion::MenuButton),
                    hud.region(HudRegion::SocialEntry),
                    hud.region(HudRegion::ScoreStrip),
                    hud.brush_chip,
                    hud.action_feedback,
                ];
                for occupied in occupied.into_iter().chain(hud.enemy_portrait_slots()) {
                    let overlap = target.intersect(occupied);
                    assert!(
                        overlap.width() <= 0.0 || overlap.height() <= 0.0,
                        "{width}/{inset}: target {target:?} covers {occupied:?}"
                    );
                }
                let secondary = [hud.allies, hud.brush_chip, hud.action_feedback];
                let obstacles = [
                    hud.minimap,
                    hud.quick_buy,
                    hud.kill_feed,
                    hud.player_status,
                    hud.region(HudRegion::MenuButton),
                    hud.region(HudRegion::SocialEntry),
                    hud.region(HudRegion::ScoreStrip),
                ];
                for (i, area) in secondary.into_iter().enumerate() {
                    for occupied in obstacles
                        .into_iter()
                        .chain(hud.enemy_portrait_slots())
                        .chain(secondary.into_iter().skip(i + 1))
                    {
                        let overlap = area.intersect(occupied);
                        assert!(
                            overlap.width() <= 0.0 || overlap.height() <= 0.0,
                            "{width}/{inset}: secondary {area:?} covers {occupied:?}"
                        );
                    }
                }
                let controls = mobile.layout();
                let discs = controls
                    .ability_centers
                    .into_iter()
                    .zip(controls.ability_radii)
                    .chain([
                        (controls.upgrade_center, controls.upgrade_radius),
                        (controls.cancel_center, controls.cancel_radius),
                    ]);
                for (centre, radius) in discs {
                    assert!(
                        centre.distance(centre.clamp(target.min, target.max)) >= radius,
                        "{width}/{inset}: target {target:?} covers combat control {centre:?}"
                    );
                }
            }
        }
        let mut mobile = MobileControls::default();
        mobile.viewport = Vec2::new(844.0, 384.0);
        mobile.safe.top = 4.0;
        let hud = HudLayout::phone(&mobile);
        assert_eq!(hud.target_frame.min, Vec2::new(350.0, 4.0));
        assert_eq!(hud.target_frame.size(), Vec2::new(144.0, 40.0));
    }

    #[test]
    fn phone_target_stays_centred_with_asymmetric_safe_insets() {
        let mut mobile = MobileControls::default();
        mobile.viewport = Vec2::new(791.0, 360.0);
        mobile.safe.left = 47.0;
        mobile.safe.right = 32.0;
        for top in [0.0, 4.0, 12.0] {
            mobile.safe.top = top;
            let hud = HudLayout::phone(&mobile);
            assert_eq!(hud.target_frame.min.y, top.max(4.0));
            assert_eq!(hud.target_frame.center().x, 403.0);
            assert_eq!(hud.target_frame.width(), 144.0);
            assert_eq!(hud.allies.min.y, top.max(4.0));
        }
    }

    #[test]
    fn phone_chrome_paints_small_but_keeps_disjoint_tap_regions() {
        let mut mobile = MobileControls::default();
        mobile.viewport = Vec2::new(844.0, 384.0);
        mobile.safe.top = 4.0;
        let layout = HudLayout::phone(&mobile);
        let menu = layout.region(HudRegion::MenuButton);
        let chat = layout.region(HudRegion::SocialEntry);
        assert_eq!(menu.size(), Vec2::splat(44.0));
        assert_eq!(chat.size(), Vec2::splat(44.0));
        assert_eq!(
            menu.min.y,
            layout.enemy_death_portrait_slots()[0].max.y + 4.0
        );
        assert_eq!(chat.min.y, menu.min.y);
        assert_eq!(chat.min.x, menu.max.x);
        assert!(menu.min.x >= layout.minimap.max.x);
        assert!(chat.min.x > layout.quick_buy.max.x);
        assert!(menu.min.y >= layout.allies.max.y);
        assert_eq!(layout.score_strip.height(), 24.0);
        assert_eq!(layout.region(HudRegion::ScoreStrip).height(), 44.0);
        assert!(layout.score_strip.min.x > mobile.viewport.x * 0.65);
        for target in layout.enemy_portrait_slots() {
            assert!(target.min.x > mobile.viewport.x * 0.5);
            assert!(target.max.x <= layout.score_strip.max.x);
            assert!(target.min.y >= layout.region(HudRegion::ScoreStrip).max.y);
        }
    }

    #[test]
    fn phone_kill_feed_and_top_chrome_avoid_control_targets() {
        let mut mobile = MobileControls::default();
        mobile.viewport = Vec2::new(852.0, 393.0);
        let layout = HudLayout::phone(&mobile);
        let controls = mobile.layout();
        let feed = layout.kill_feed();
        assert_eq!(feed.min.x, layout.minimap.min.x);
        assert_eq!(feed.min.y, layout.quick_buy.max.y + phone_rows::GAP);
        assert!(feed.max.x < controls.utility_centers[0].x - controls.auxiliary_radius);
        assert!(feed.max.y <= mobile.viewport.y - mobile.safe.bottom);
        assert_eq!(layout.target_frame.center().x, mobile.viewport.x * 0.5);
        assert_eq!(layout.allies.min.y, layout.minimap.min.y);
        assert!(layout.allies.max.x < layout.target_frame.min.x);
        assert!(layout.target_frame.max.x < layout.score_strip.min.x);
    }

    #[test]
    fn phone_shop_follows_minimap_and_enemy_portraits_avoid_the_skill_orbit() {
        let mut mobile = MobileControls::default();
        mobile.viewport = Vec2::new(852.0, 393.0);
        let hud = HudLayout::phone(&mobile);
        assert_eq!(hud.quick_buy.min.x, mobile.safe.left);
        assert_eq!(hud.quick_buy.min.y, hud.minimap.max.y + phone_rows::GAP);
        assert_eq!(
            hud.player_status.min.y,
            hud.kill_feed.max.y + phone_rows::GAP
        );
        let controls = mobile.layout();
        let circles: Vec<_> = controls
            .ability_centers
            .into_iter()
            .zip(controls.ability_radii)
            .chain(
                controls
                    .category_centers
                    .into_iter()
                    .map(|p| (p, controls.category_radius)),
            )
            .chain(
                controls
                    .utility_centers
                    .into_iter()
                    .map(|p| (p, controls.auxiliary_radius)),
            )
            .chain([
                (controls.attack_center, controls.attack_radius),
                (controls.recall_center, controls.auxiliary_radius),
                (controls.upgrade_center, controls.upgrade_radius),
            ])
            .collect();
        for rect in hud.enemy_portrait_slots() {
            assert!(rect.max.x <= mobile.viewport.x - mobile.safe.right);
            assert!(rect.min.y >= hud.score_strip.max.y + space::S8);
            assert!(rect.min.x > mobile.viewport.x * 0.5);
            for (center, radius) in &circles {
                let nearest = center.clamp(rect.min, rect.max);
                assert!(
                    nearest.distance(*center) >= *radius,
                    "portrait {rect:?} overlaps control at {center:?}"
                );
            }
        }
    }

    fn at(rect: Rect) -> (f32, f32, f32, f32) {
        (rect.min.x, rect.min.y, rect.width(), rect.height())
    }

    /// Every desktop region at 1280 × 720 is the hud.md table (x, y, w, h).
    #[test]
    fn desktop_regions_match_the_redline() {
        let l = HudLayout::desktop(Vec2::new(1280.0, 720.0), false);
        assert_eq!(at(l.minimap), (16.0, 16.0, 176.0, 176.0));
        assert_eq!(at(l.buff_chips), (16.0, 200.0, 344.0, 52.0));
        assert_eq!(at(l.target_frame), (500.0, 16.0, 280.0, 56.0));
        assert_eq!(at(l.brush_chip), (568.0, 80.0, 144.0, 24.0));
        assert_eq!(at(l.score_strip), (940.0, 16.0, 176.0, 40.0));
        assert_eq!(at(l.icon_buttons), (1128.0, 16.0, 136.0, 40.0));
        assert_eq!(at(l.action_feedback), (468.0, 504.0, 344.0, 52.0));
        assert_eq!(at(l.upgrade_chip), (566.0, 564.0, 148.0, 24.0));
        assert_eq!(at(l.ability_bar), (468.0, 596.0, 344.0, 108.0));
        assert_eq!(at(l.player_status), (16.0, 632.0, 288.0, 72.0));
        assert_eq!(at(l.equipment), (1064.0, 602.0, 200.0, 102.0));
        // skill-description.md: tooltip 540,408 for slot E; bottom 556.
        assert_eq!(l.skill_card.max.y, 556.0);
        assert_eq!(l.tooltip_bottom(true), 556.0);
        assert_eq!(l.tooltip_bottom(false), 588.0);
        // Menu = the last icon cell; chat + reactions the first two.
        assert_eq!(
            at(l.region(HudRegion::MenuButton)),
            (1224.0, 16.0, 40.0, 40.0)
        );
        assert_eq!(
            at(l.region(HudRegion::SocialEntry)),
            (1128.0, 16.0, 88.0, 40.0)
        );
        // Controller: legend y 520–572, feedback moves up to 452.
        let pad = HudLayout::desktop(Vec2::new(1280.0, 720.0), true);
        assert_eq!((pad.legend.min.y, pad.legend.max.y), (520.0, 572.0));
        assert_eq!(pad.action_feedback.min.y, 452.0);
    }

    /// The protected centre (x 384–896, y 144–468 at 1280 × 720) holds no
    /// resting plate at the reference or a scaled logical viewport.
    #[test]
    fn desktop_resting_plates_stay_out_of_the_protected_centre() {
        for viewport in [
            Vec2::new(1280.0, 720.0),
            Vec2::new(1024.0, 640.0),
            Vec2::new(1440.0, 720.0),
        ] {
            let l = HudLayout::desktop(viewport, false);
            let protected = Rect::from_corners(
                Vec2::new(0.30, 0.20) * viewport,
                Vec2::new(0.70, 0.65) * viewport,
            );
            for plate in [
                l.minimap,
                l.buff_chips,
                l.target_frame,
                l.score_strip,
                l.icon_buttons,
                l.ability_bar,
                l.player_status,
                l.equipment,
            ] {
                let overlap = plate.intersect(protected);
                assert!(
                    overlap.width() <= 0.5 || overlap.height() <= 0.5,
                    "{plate:?} enters the centre at {viewport:?}"
                );
            }
            // Plates never overlap each other.
            let plates = [
                l.minimap,
                l.target_frame,
                l.score_strip,
                l.icon_buttons,
                l.ability_bar,
                l.player_status,
                l.equipment,
            ];
            for (i, a) in plates.iter().enumerate() {
                for b in &plates[i + 1..] {
                    let overlap = a.intersect(*b);
                    assert!(
                        overlap.width() <= 0.0 || overlap.height() <= 0.0,
                        "{a:?} overlaps {b:?} at {viewport:?}"
                    );
                }
            }
        }
    }

    /// The compact phone profile at the reference safe area (47/47/0/21).
    #[test]
    fn compact_phone_regions_keep_the_map_and_free_the_top_right() {
        let mut mobile = MobileControls::default();
        mobile.enabled = true;
        mobile.viewport = Vec2::new(844.0, 390.0);
        mobile.safe.left = 47.0;
        mobile.safe.right = 47.0;
        mobile.safe.top = 0.0;
        mobile.safe.bottom = 21.0;
        let l = HudLayout::phone(&mobile);
        assert_eq!(at(l.minimap), (57.0, 4.0, 120.0, 120.0));
        assert_eq!(at(l.player_status), (57.0, 210.0, 200.0, 48.0));
        assert_eq!(at(l.quick_buy), (47.0, 128.0, 132.0, 44.0));
        assert_eq!(at(l.target_frame), (350.0, 4.0, 144.0, 40.0));
        assert_eq!(at(l.brush_chip), (350.0, 92.0, 144.0, 24.0));
        assert_eq!(at(l.action_feedback), (312.0, 120.0, 220.0, 40.0));
        assert_eq!(at(l.score_strip), (627.0, 4.0, 160.0, 24.0));
        assert_eq!(at(l.icon_buttons), (181.0, 88.0, 88.0, 44.0));
        assert_eq!(at(l.skill_card), (282.0, 64.0, 280.0, 148.0));
        // Quick-buy ends above the protected passage (y 242), also at the
        // runtime's conservative insets (32/32/12/20).
        assert!(l.quick_buy.max.y < 242.0);
        mobile.safe.top = 12.0;
        assert!(HudLayout::phone(&mobile).quick_buy.max.y < 242.0);
    }

    /// A dev build's contour badge sits `space.8` under the score strip and
    /// clear of every resting plate around it; the phone HUD has none.
    #[test]
    fn the_contour_badge_sits_under_the_score_strip_clear_of_other_plates() {
        let reference = HudLayout::desktop(Vec2::new(1280.0, 720.0), false);
        assert_eq!(
            at(reference.region(HudRegion::ContourBadge)),
            (940.0, 64.0, 176.0, size::BADGE_HEIGHT)
        );
        assert_eq!(
            HudRegion::ContourBadge.anchor(Form::Desktop),
            Anchor::TopLeft
        );
        let mut mobile = MobileControls::default();
        mobile.enabled = true;
        mobile.viewport = Vec2::new(844.0, 390.0);
        assert_eq!(
            HudLayout::phone(&mobile).region(HudRegion::ContourBadge),
            Rect::default()
        );
        for viewport in [
            Vec2::new(1280.0, 720.0),
            Vec2::new(1024.0, 640.0),
            Vec2::new(1440.0, 720.0),
        ] {
            let l = HudLayout::desktop(viewport, false);
            let badge = l.region(HudRegion::ContourBadge);
            assert_eq!(badge.min.x, l.score_strip.min.x);
            assert_eq!(badge.min.y, l.score_strip.max.y + space::S8);
            let plates = [
                l.minimap,
                l.allies,
                l.target_frame,
                l.brush_chip,
                l.score_strip,
                l.icon_buttons,
                l.kill_feed,
                l.action_feedback,
            ];
            for plate in plates.into_iter().chain(l.enemy_portrait_slots()) {
                let overlap = badge.intersect(plate);
                assert!(
                    overlap.width() <= 0.0 || overlap.height() <= 0.0,
                    "{badge:?} overlaps {plate:?} at {:?}",
                    l.viewport
                );
            }
        }
    }

    #[test]
    fn world_points_divide_by_the_ui_scale() {
        let scale = UiScale(1.5);
        assert_eq!(
            world_to_ui(Vec2::new(960.0, 540.0), Some(&scale)),
            Vec2::new(640.0, 360.0)
        );
        assert_eq!(
            world_to_ui(Vec2::new(12.0, 8.0), None),
            Vec2::new(12.0, 8.0)
        );
    }

    #[test]
    fn the_ui_viewport_divides_the_window_by_the_ui_scale() {
        let mut window = Window::default();
        window.resolution.set(1920.0, 1080.0);
        assert_eq!(
            ui_viewport(&window, Some(&UiScale(1.5))),
            Vec2::new(1280.0, 720.0)
        );
        assert_eq!(ui_viewport(&window, None), Vec2::new(1920.0, 1080.0));
    }
    #[test]
    fn tablet_map_is_twenty_percent_larger_without_changing_phone_map() {
        let mut mobile = MobileControls::default();
        mobile.viewport = Vec2::new(852.0, 393.0);
        let phone = HudLayout::phone(&mobile);
        mobile.viewport = Vec2::new(1180.0, 820.0);
        let tablet = HudLayout::phone(&mobile);
        assert!((tablet.minimap.width() / phone.minimap.width() - 1.2).abs() < 0.001);
        for layout in [phone, tablet] {
            assert_eq!(layout.quick_buy.min.x + 10.0, layout.minimap.min.x);
            assert_eq!(
                layout.quick_buy.min.y,
                layout.minimap.max.y + phone_rows::GAP
            );
            assert_eq!(
                layout.player_status.min.y,
                layout.kill_feed.max.y + phone_rows::GAP
            );
        }
    }
}
